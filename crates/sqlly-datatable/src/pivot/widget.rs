//! The `PivotGrid` GPUI widget: a canvas-rendered pivot table wired to an
//! [`Entity<PivotState>`]. Mirrors `SqllyDataTable`'s structure — the widget
//! owns no data of its own, it only routes events into the state and paints
//! a `PivotPaintData` snapshot.

use crate::grid::menu::{MENU_FONT_SIZE, MENU_INNER_PAD, MENU_ITEM_HEIGHT, MENU_PADDING_X};
use crate::grid::selection::to_grid_relative;
use crate::pivot::context_menu::PivotMenuItem;
use crate::pivot::paint::{paint_pivot_grid, PivotPaintData};
use crate::pivot::state::{PivotHitResult, PivotState};

use gpui::{
    anchored, canvas, deferred, div, point, px, App, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Render, ScrollWheelEvent, Styled, Window,
};

/// Draw order for the pivot's context-menu overlay; matches the flat grid's
/// menu priority so it always paints and receives events on top.
const PIVOT_MENU_PRIORITY: usize = 1_000_000;

/// Canvas widget rendering one [`PivotState`].
pub struct PivotGrid {
    /// The shared pivot state. The sidebar mutates the same entity.
    pub state: Entity<PivotState>,
}

impl PivotGrid {
    /// Wrap an existing pivot state entity.
    #[must_use]
    pub fn new(state: Entity<PivotState>) -> Self {
        Self { state }
    }
}

impl Focusable for PivotGrid {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.state.read(cx).focus_handle.clone()
    }
}

impl Render for PivotGrid {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let state_canvas = self.state.clone();
        let state_down = self.state.clone();
        let state_move = self.state.clone();
        let state_up = self.state.clone();
        let state_scroll = self.state.clone();
        let state_key = self.state.clone();
        let state_right = self.state.clone();
        let bg = self.state.read(cx).theme.bg;
        let focus_handle = self.state.read(cx).focus_handle.clone();
        let focus_down = focus_handle.clone();
        let focus_right = focus_handle.clone();

        div()
            .size_full()
            .relative()
            .track_focus(&focus_handle)
            .bg(bg)
            .child(
                canvas(
                    move |bounds, window, cx| -> PivotPaintData {
                        let viewport = window.viewport_size();
                        state_canvas.update(cx, |s, cx| {
                            let mut dirty = false;
                            if s.bounds != bounds {
                                s.bounds = bounds;
                                s.clamp_scroll_to_bounds();
                                dirty = true;
                            }
                            if s.window_viewport != viewport {
                                s.window_viewport = viewport;
                            }
                            if dirty {
                                cx.notify();
                            }
                        });
                        let s = state_canvas.read(cx);
                        PivotPaintData::from_state(s)
                    },
                    move |bounds, data, window, cx| {
                        paint_pivot_grid(&data, window, cx, bounds);
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, window, cx| {
                    window.focus(&focus_down);
                    state_down.update(cx, |s, cx| {
                        // A left-click reaching the grid means the pointer
                        // was not on the menu overlay; dismiss it.
                        s.menu = None;
                        let rel = to_grid_relative(event.position, s.bounds.origin);
                        // Double-click on a value cell drills through to the
                        // flat grid, filtered to the cell's driving rows.
                        if event.click_count >= 2 {
                            if let PivotHitResult::Cell { row, col } = s.hit_test(rel) {
                                s.request_drill_down(row, col);
                                cx.notify();
                                return;
                            }
                        }
                        s.handle_mouse_down(rel, event.modifiers.shift);
                        cx.notify();
                    });
                },
            )
            .on_mouse_down(
                MouseButton::Right,
                move |event: &MouseDownEvent, window, cx| {
                    window.focus(&focus_right);
                    state_right.update(cx, |s, cx| {
                        let rel = to_grid_relative(event.position, s.bounds.origin);
                        let hit = s.hit_test(rel);
                        s.open_context_menu(hit, rel);
                        cx.notify();
                    });
                },
            )
            .on_mouse_move(move |event: &MouseMoveEvent, _window, cx| {
                state_move.update(cx, |s, cx| {
                    let rel = to_grid_relative(event.position, s.bounds.origin);
                    s.handle_mouse_move(rel, event.pressed_button == Some(MouseButton::Left));
                    cx.notify();
                });
            })
            .on_mouse_up(
                MouseButton::Left,
                move |_event: &MouseUpEvent, _window, cx| {
                    state_up.update(cx, |s, cx| {
                        s.handle_mouse_up();
                        cx.notify();
                    });
                },
            )
            .on_scroll_wheel(move |event: &ScrollWheelEvent, _window, cx| {
                state_scroll.update(cx, |s, cx| {
                    let line_h = px(s.row_height);
                    let delta = event.delta.pixel_delta(line_h);
                    s.apply_scroll_delta(f32::from(delta.x), f32::from(delta.y));
                    cx.notify();
                });
            })
            .on_key_down(move |event: &KeyDownEvent, _window, cx| {
                state_key.update(cx, |s, cx| {
                    s.handle_key(&event.keystroke, cx);
                    cx.notify();
                });
            })
            .children(render_pivot_menu_overlay(&self.state, cx))
    }
}

/// Build the pivot's right-click menu as a `deferred` + `anchored` overlay
/// (same mechanism as the flat grid's menu, so it paints and receives events
/// on top of everything). Returns `None` when no menu is open.
fn render_pivot_menu_overlay(
    state: &Entity<PivotState>,
    cx: &mut Context<PivotGrid>,
) -> Option<impl IntoElement + use<>> {
    let s = state.read(cx);
    let menu = s.menu.as_ref()?;
    let theme = s.theme.clone();
    let animations = s.animations;
    let items = menu.items.clone();
    let hovered = menu.hovered;
    let cw = s.char_width;
    let abs_x = f32::from(s.bounds.origin.x) + f32::from(menu.anchor.x);
    let abs_y = f32::from(s.bounds.origin.y) + f32::from(menu.anchor.y);

    let max_label_chars = items
        .iter()
        .filter_map(|item| match item {
            PivotMenuItem::Action { label, .. } => Some(label.chars().count()),
            PivotMenuItem::Separator => None,
        })
        .max()
        .unwrap_or(0);
    let menu_w = (max_label_chars as f32 * cw + MENU_PADDING_X * 2.0).max(160.0);

    let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(items.len());
    let mut action_idx = 0usize;
    for item in &items {
        match item {
            PivotMenuItem::Separator => {
                rows.push(
                    div()
                        .h(px(MENU_ITEM_HEIGHT))
                        .flex()
                        .items_center()
                        .child(div().mx(px(4.0)).h(px(1.0)).w_full().bg(theme.grid_line))
                        .into_any_element(),
                );
            }
            PivotMenuItem::Action { id, label } => {
                let this_idx = action_idx;
                action_idx += 1;
                let id = id.clone();
                let state_click = state.clone();
                let state_hover = state.clone();
                let mut row = div()
                    .h(px(MENU_ITEM_HEIGHT))
                    .px(px(MENU_PADDING_X))
                    .flex()
                    .items_center()
                    .text_color(theme.menu_fg)
                    .text_size(px(MENU_FONT_SIZE))
                    .child(label.clone())
                    .on_mouse_move(move |_e: &MouseMoveEvent, _window, cx| {
                        state_hover.update(cx, |s, cx| {
                            if let Some(m) = s.menu.as_mut() {
                                if m.hovered != Some(this_idx) {
                                    m.hovered = Some(this_idx);
                                    cx.notify();
                                }
                            }
                        });
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        move |_e: &MouseDownEvent, _window, cx| {
                            state_click.update(cx, |s, cx| {
                                if let Some(menu) = s.menu.take() {
                                    s.execute_menu_action(&id, menu, cx);
                                }
                                cx.notify();
                            });
                        },
                    );
                if hovered == Some(this_idx) {
                    row = row.bg(theme.menu_hover_bg);
                }
                rows.push(row.into_any_element());
            }
        }
    }

    let menu_body = div()
        .flex()
        .flex_col()
        .w(px(menu_w))
        .py(px(MENU_INNER_PAD))
        .bg(theme.menu_bg)
        .border_1()
        .border_color(theme.grid_line)
        .children(rows);

    let state_backdrop = state.clone();
    let overlay = deferred(
        anchored().position(point(px(abs_x), px(abs_y))).child(
            div()
                .occlude()
                .child(crate::grid::motion::pop_in(
                    menu_body,
                    "pivot-context-menu",
                    animations,
                ))
                .on_mouse_down_out(move |_e: &MouseDownEvent, _window, cx| {
                    state_backdrop.update(cx, |s, cx| {
                        if s.menu.is_some() {
                            s.menu = None;
                            cx.notify();
                        }
                    });
                }),
        ),
    )
    .with_priority(PIVOT_MENU_PRIORITY);
    Some(overlay)
}
