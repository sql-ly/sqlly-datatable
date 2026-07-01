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
    canvas, div, point, px, App, AppContext, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Render, ScrollWheelEvent, Styled, Window,
};

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

        // Spawn an edge-scroll timer **only while a drag is in progress**.
        // The task self-detaches when `wants_edge_scroll_tick` is false so it
        // is no longer a 60 fps loop.
        if self.state.read(cx).is_dragging {
            let state_edge = self.state.clone();
            cx.spawn(async move |_weak, cx| loop {
                gpui::Timer::after(std::time::Duration::from_millis(EDGE_SCROLL_TICK_MS)).await;
                let res = cx.update(|cx| state_edge.update(cx, |s, _cx| s.apply_edge_scroll()));
                if let Ok(true) = res {
                    let _ = state_edge.update(cx, |_s, cx| cx.notify());
                }
                let dragging_res = cx.update(|cx| state_edge.read(cx).is_dragging);
                if !matches!(dragging_res, Ok(true)) {
                    break;
                }
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
                    move |bounds, _window, cx| -> PaintData {
                        state_canvas.update(cx, |s, cx| {
                            if s.bounds != bounds {
                                s.bounds = bounds;
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
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, window, cx| {
                    window.focus(&focus_left);
                    state_mouse.update(cx, |s, cx| {
                        if let Some(menu) = s.context_menu.clone() {
                            let cw = s.char_width;
                            let (mx_rel, my_rel) = state_inner::screen_to_content(
                                event.position,
                                s.bounds.origin,
                                s.scroll_handle.offset(),
                            );
                            let w = menu.width_for(cw);
                            let total_h = menu.total_height();
                            let ax = f32::from(menu.anchor.x);
                            let ay = f32::from(menu.anchor.y);
                            if mx_rel >= ax
                                && mx_rel <= ax + w
                                && my_rel >= ay
                                && my_rel <= ay + total_h
                            {
                                if let Some(action_idx) = menu::hover_at(&menu, mx_rel, my_rel, cw)
                                {
                                    let mut cur = 0;
                                    for item in &menu.items {
                                        match item {
                                            MenuItem::Action(a) => {
                                                if cur == action_idx {
                                                    s.pending_action = Some((*a, menu.col));
                                                    s.context_menu = None;
                                                    cx.notify();
                                                    return;
                                                }
                                                cur += 1;
                                            }
                                            MenuItem::Custom { id, .. } => {
                                                if cur == action_idx {
                                                    if let Some(request) = &menu.request {
                                                        s.pending_custom_context_menu_action =
                                                            Some(PendingCustomContextMenuAction {
                                                                id: id.clone(),
                                                                request: request.clone(),
                                                            });
                                                    }
                                                    s.context_menu = None;
                                                    cx.notify();
                                                    return;
                                                }
                                                cur += 1;
                                            }
                                            MenuItem::Separator => {}
                                        }
                                    }
                                }
                            } else {
                                s.context_menu = None;
                                s.filter_prompt = None;
                            }
                        }
                        s.handle_mouse_down(event.position, event.modifiers.shift);
                        cx.notify();
                    });
                },
            )
            .on_mouse_down(
                MouseButton::Right,
                move |event: &MouseDownEvent, window, cx| {
                    window.focus(&focus_right);
                    state_right.update(cx, |s, cx| {
                        let pos = event.position;
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
                    s.handle_mouse_move(event.position, event.pressed_button);
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
