//! The `SqllyDataTable` GPUI widget and its builder. Owns one
//! `Entity<GridState>` and wires GPUI's mouse / keyboard / scroll events to
//! its methods. A bunch of `state.clone()` clones exist because each closure
//! needs its own owned reference to the GPUI entity handle.

use crate::config::GridConfig;
use crate::data::GridData;
use crate::grid::context_menu::{
    ContextMenuProvider, ContextMenuProviderHandle, PendingCustomContextMenuAction,
};
use crate::grid::paint::{paint_grid, paint_status_bar, PaintData, StatusBarData};
use crate::grid::state::state_inner;
use crate::grid::state::{GridState, EDGE_SCROLL_TICK_MS};
use crate::grid::theme::GridTheme;
use crate::grid::{menu, HitResult, MenuItem};

use gpui::{
    anchored, canvas, deferred, div, point, px, App, AppContext, Context, Entity, FocusHandle,
    Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Render, ScrollWheelEvent, Styled, Window,
};

/// Draw order for the context-menu overlay. Deliberately far above any
/// ordinary application UI so the menu — and, crucially, its event hitbox —
/// sits on top of everything, even content painted outside the grid widget's
/// own layout bounds (e.g. a host header above the grid). Deferred draws
/// register their hitbox in a later pass, so this also fixes hover/click
/// routing for menu items that visually overflow the grid area.
const CONTEXT_MENU_PRIORITY: usize = 1_000_000;

/// Top-level GPUI widget.
pub struct SqllyDataTable {
    pub state: Entity<GridState>,
}

impl SqllyDataTable {
    /// Wrap an existing `Entity<GridState>`.
    #[must_use]
    pub fn new(state: Entity<GridState>) -> Self {
        Self { state }
    }

    /// Construct from `GridData` using the default [`GridConfig`].
    #[must_use]
    pub fn builder(data: GridData) -> SqllyDataTableBuilder {
        SqllyDataTableBuilder {
            data,
            config: GridConfig::default(),
            context_menu_provider: None,
        }
    }
}

/// Builder for `SqllyDataTable`.
pub struct SqllyDataTableBuilder {
    data: GridData,
    config: GridConfig,
    context_menu_provider: Option<ContextMenuProviderHandle>,
}

impl SqllyDataTableBuilder {
    /// Override the entire [`GridConfig`].
    #[must_use]
    pub fn config(mut self, config: GridConfig) -> Self {
        self.config = config;
        self
    }

    /// Override only the [`GridTheme`]. No-op for now; kept for symmetry.
    #[must_use]
    pub fn theme(self, theme: GridTheme) -> Self {
        let _ = theme;
        self
    }

    /// Register a custom right-click menu provider. When registered, the
    /// provider fully controls the right-click menu for all targets (cells,
    /// row headers, column headers). The built-in column-header menu is
    /// suppressed; use
    /// [`crate::grid::context_menu::ContextMenuItem::standard_column_header_items`]
    /// to compose built-in actions.
    #[must_use]
    pub fn context_menu_provider(mut self, provider: impl ContextMenuProvider + 'static) -> Self {
        self.context_menu_provider = Some(ContextMenuProviderHandle::new(provider));
        self
    }

    /// Build the widget inside the supplied [`gpui::App`].
    pub fn build(self, cx: &mut App) -> SqllyDataTable {
        let focus = cx.focus_handle();
        let provider = self.context_menu_provider;
        let state = cx.new(|_cx| {
            let mut s = GridState::new(self.data, self.config, focus.clone());
            s.context_menu_provider = provider;
            s
        });
        SqllyDataTable { state }
    }
}

impl Focusable for SqllyDataTable {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.state.read(cx).focus_handle.clone()
    }
}

impl Render for SqllyDataTable {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let state_canvas = self.state.clone();
        let state_status = self.state.clone();
        let state_mouse = self.state.clone();
        let state_move = self.state.clone();
        let state_up = self.state.clone();
        let state_scroll = self.state.clone();
        let state_key = self.state.clone();
        let state_right = self.state.clone();
        let bg = self.state.read(cx).theme.bg;
        let focus_handle = self.state.read(cx).focus_handle.clone();
        let focus_left = focus_handle.clone();
        let focus_right = focus_handle.clone();
        let status_h = self.state.read(cx).status_bar_height;

        // Process any pending menu action from a previous mouse-down on a
        // menu item (needs App access for clipboard).
        if let Some((action, col)) = self.state.read(cx).pending_action {
            self.state.update(cx, |s, cx| {
                s.execute_action(action, col, cx);
                s.pending_action = None;
            });
        }

        // Process any pending custom context-menu action.
        if let Some(pending) = self
            .state
            .read(cx)
            .pending_custom_context_menu_action
            .clone()
        {
            self.state.update(cx, |s, cx| {
                s.pending_custom_context_menu_action = None;
                s.execute_custom_context_menu_action(pending, cx);
            });
        }

        // Spawn an edge-scroll timer **only while a drag is in progress**, and
        // **only one at a time**. Without the `edge_scroll_active` guard,
        // `render` would spawn a fresh 16 ms loop on every frame/notify during
        // a drag — each successful tick calls `cx.notify()`, which re-renders
        // and spawned yet another task, stacking concurrent loops that each
        // apply a scroll delta per tick and multiply the effective speed
        // without bound. The task clears the flag when it exits.
        if self.state.read(cx).is_dragging && !self.state.read(cx).edge_scroll_active {
            self.state.update(cx, |s, _cx| s.edge_scroll_active = true);
            let state_edge = self.state.clone();
            cx.spawn(async move |_weak, cx| {
                loop {
                    gpui::Timer::after(std::time::Duration::from_millis(EDGE_SCROLL_TICK_MS)).await;
                    let res = cx.update(|cx| state_edge.update(cx, |s, _cx| s.apply_edge_scroll()));
                    if let Ok(true) = res {
                        let _ = state_edge.update(cx, |_s, cx| cx.notify());
                    }
                    let dragging_res = cx.update(|cx| state_edge.read(cx).is_dragging);
                    if !matches!(dragging_res, Ok(true)) {
                        break;
                    }
                }
                let _ =
                    cx.update(|cx| state_edge.update(cx, |s, _cx| s.edge_scroll_active = false));
            })
            .detach();
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .track_focus(&focus_handle)
            .bg(bg)
            .child(
                canvas(
                    move |bounds, window, cx| -> PaintData {
                        let viewport = window.viewport_size();
                        state_canvas.update(cx, |s, cx| {
                            let mut dirty = false;
                            if s.bounds != bounds {
                                s.bounds = bounds;
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
                        PaintData::from_state(s)
                    },
                    move |bounds, data, window, cx| {
                        paint_grid(&data, window, cx, bounds);
                    },
                )
                .flex_1(),
            )
            .child(
                canvas(
                    move |_bounds, _window, cx| -> StatusBarData {
                        let s = state_status.read(cx);
                        StatusBarData::from_state(s)
                    },
                    move |bounds, data, window, cx| {
                        paint_status_bar(&data, window, cx, bounds);
                    },
                )
                .h(px(status_h)),
            )
            .children(render_context_menu_overlay(&self.state, cx))
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, window, cx| {
                    window.focus(&focus_left);
                    state_mouse.update(cx, |s, cx| {
                        // Normalize the absolute window pointer into the grid's
                        // own frame. Menu hit-testing is handled by the deferred
                        // overlay's own item handlers, so a left-click that
                        // reaches the grid means the pointer was NOT on the menu;
                        // dismiss any open menu and proceed with grid selection.
                        let rel = state_inner::to_grid_relative(event.position, s.bounds.origin);
                        if s.context_menu.is_some() {
                            s.context_menu = None;
                            s.filter_prompt = None;
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
                        let pos = state_inner::to_grid_relative(event.position, s.bounds.origin);
                        let hit = s.hit_test(pos);

                        // No provider — existing built-in behavior.
                        if s.context_menu_provider.is_none() {
                            match hit {
                                HitResult::ColumnHeader(col) | HitResult::SortButton(col) => {
                                    s.open_context_menu(col, pos);
                                }
                                _ => {
                                    s.context_menu = None;
                                    s.filter_prompt = None;
                                }
                            }
                            cx.notify();
                            return;
                        }

                        // Provider exists — build custom menu.
                        let Some(target) = s.context_menu_target_from_hit(hit) else {
                            s.context_menu = None;
                            s.filter_prompt = None;
                            cx.notify();
                            return;
                        };

                        let effective = s.effective_selection_for_context_target(&target);
                        if effective != s.selection {
                            s.selection = effective.clone();
                        }

                        let request = s.build_context_menu_request(target, &effective);
                        let col = request.target.column_index().unwrap_or(0);

                        let Some(provider) = s.context_menu_provider.clone() else {
                            return;
                        };
                        let public_items = provider.menu_items(&request);
                        let items = GridState::convert_context_menu_items(public_items);

                        if items.is_empty() {
                            s.context_menu = None;
                        } else {
                            s.context_menu =
                                Some(menu::ContextMenu::custom(col, pos, items, request));
                        }
                        s.filter_prompt = None;
                        cx.notify();
                    });
                },
            )
            .on_mouse_move(move |event: &MouseMoveEvent, _window, cx| {
                state_move.update(cx, |s, cx| {
                    let rel = state_inner::to_grid_relative(event.position, s.bounds.origin);
                    s.handle_mouse_move(rel, event.pressed_button);
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
                    let scroll = s.scroll_handle.offset();
                    let (mx, my) = s.max_scroll();
                    let new_y = (f32::from(scroll.y) - f32::from(delta.y)).clamp(0.0, my);
                    let new_x = (f32::from(scroll.x) - f32::from(delta.x)).clamp(0.0, mx);
                    s.scroll_handle.set_offset(point(px(new_x), px(new_y)));
                    if s.drag_start.is_some() {
                        s.handle_scroll_drag();
                    }
                    cx.notify();
                });
            })
            .on_key_down(move |event: &KeyDownEvent, _window, cx| {
                let ks = &event.keystroke;
                if ks.modifiers.platform && ks.key == "q" {
                    cx.quit();
                    return;
                }
                state_key.update(cx, |s, cx| {
                    let kb = &s.config.key_bindings;
                    if kb.select_all.matches(ks) {
                        s.select_all();
                    } else if kb.copy.matches(ks) {
                        s.copy_selection(false, cx);
                    } else if kb.copy_with_headers.matches(ks) {
                        s.copy_selection(true, cx);
                    } else if kb.page_up.matches(ks) {
                        s.page_up();
                    } else if kb.page_down.matches(ks) {
                        s.page_down();
                    } else {
                        s.handle_key(ks);
                    }
                    cx.notify();
                });
            })
    }
}

/// Build the context-menu overlay as a `deferred` + `anchored` element so it
/// paints — and receives mouse events — on top of everything, including
/// regions outside the grid widget's own layout bounds. Returns `None` when no
/// menu is open.
///
/// Positioning reuses [`menu::ContextMenu::resolved_position`] (window-viewport
/// aware: flips up when there's no room below, shifts left at the right edge),
/// then converts to absolute window coordinates for `anchored().position(..)`.
/// Each selectable row carries its own `on_mouse_down` (dispatch) and
/// `on_mouse_move` (hover highlight) handlers; a full-screen backdrop behind
/// the menu dismisses it on any outside click.
fn render_context_menu_overlay(
    state: &Entity<GridState>,
    cx: &mut Context<SqllyDataTable>,
) -> Option<impl IntoElement> {
    let s = state.read(cx);
    let menu = s.context_menu.clone()?;
    let theme = s.theme.clone();
    let cw = s.char_width;
    let grid_ox = f32::from(s.bounds.origin.x);
    let grid_oy = f32::from(s.bounds.origin.y);
    let viewport = s.window_viewport;
    let vw = f32::from(viewport.width);
    let vh = f32::from(viewport.height);

    let resolved = menu.resolved_position(grid_ox, grid_oy, vw, vh, cw);
    let abs_x = grid_ox + f32::from(resolved.x);
    let abs_y = grid_oy + f32::from(resolved.y);
    let menu_w = menu.width_for(cw);

    // Build one row per item. `selectable_idx` counts only Action/Custom items
    // so it matches the `hovered` index convention used elsewhere.
    let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(menu.items.len());
    let mut selectable_idx = 0usize;
    for item in &menu.items {
        match item {
            MenuItem::Separator => {
                rows.push(
                    div()
                        .h(px(menu::MENU_ITEM_HEIGHT))
                        .flex()
                        .items_center()
                        .child(div().mx(px(4.0)).h(px(1.0)).w_full().bg(theme.grid_line))
                        .into_any_element(),
                );
            }
            MenuItem::Action(_) | MenuItem::Custom { .. } => {
                let this_idx = selectable_idx;
                selectable_idx += 1;
                let label = item.label().unwrap_or("").to_owned();
                let hovered = menu.hovered == Some(this_idx);

                // Dispatch: set the pending action and close the menu. The
                // pending fields are drained at the top of `render` (they need
                // App access for clipboard).
                let action = match item {
                    MenuItem::Action(a) => MenuDispatch::Builtin(*a, menu.col),
                    MenuItem::Custom { id, .. } => {
                        MenuDispatch::Custom(id.clone(), menu.request.clone())
                    }
                    MenuItem::Separator => unreachable!(),
                };

                let state_click = state.clone();
                let state_hover = state.clone();
                let mut row = div()
                    .h(px(menu::MENU_ITEM_HEIGHT))
                    .px(px(menu::MENU_PADDING_X))
                    .flex()
                    .items_center()
                    .text_color(theme.menu_fg)
                    .text_size(px(menu::MENU_FONT_SIZE))
                    .child(label)
                    .on_mouse_move(move |_e: &MouseMoveEvent, _window, cx| {
                        state_hover.update(cx, |s, cx| {
                            if let Some(m) = s.context_menu.as_mut() {
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
                                match &action {
                                    MenuDispatch::Builtin(a, col) => {
                                        s.pending_action = Some((*a, *col));
                                    }
                                    MenuDispatch::Custom(id, request) => {
                                        if let Some(request) = request {
                                            s.pending_custom_context_menu_action =
                                                Some(PendingCustomContextMenuAction {
                                                    id: id.clone(),
                                                    request: request.clone(),
                                                });
                                        }
                                    }
                                }
                                s.context_menu = None;
                                cx.notify();
                            });
                        },
                    );
                if hovered {
                    row = row.bg(theme.menu_hover_bg);
                }
                rows.push(row.into_any_element());
            }
        }
    }

    let menu_body = div()
        .absolute()
        .flex()
        .flex_col()
        .w(px(menu_w))
        .py(px(menu::MENU_INNER_PAD))
        .bg(theme.menu_bg)
        .border_1()
        .border_color(theme.grid_line)
        .children(rows);

    // Full-window transparent backdrop: catches clicks outside the menu to
    // dismiss it. Placed behind the menu within the same anchored overlay.
    let state_backdrop = state.clone();
    let overlay = deferred(anchored().position(point(px(abs_x), px(abs_y))).child(
        div().occlude().child(menu_body).on_mouse_down_out(
            move |_e: &MouseDownEvent, _window, cx| {
                state_backdrop.update(cx, |s, cx| {
                    if s.context_menu.is_some() {
                        s.context_menu = None;
                        s.filter_prompt = None;
                        cx.notify();
                    }
                });
            },
        ),
    ))
    .with_priority(CONTEXT_MENU_PRIORITY);

    Some(overlay)
}

/// What a menu row dispatches when clicked. Captured per-row so the click
/// handler owns its data without borrowing the menu snapshot.
enum MenuDispatch {
    Builtin(menu::MenuAction, usize),
    Custom(
        String,
        Option<crate::grid::context_menu::ContextMenuRequest>,
    ),
}
