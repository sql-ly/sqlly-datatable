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
use crate::grid::state::{FilterInput, GridState, EDGE_SCROLL_TICK_MS};
use crate::grid::theme::GridTheme;
use crate::grid::{menu, HitResult, MenuItem, SortDirection};

use gpui::{
    anchored, canvas, deferred, div, point, px, App, AppContext, Context, Corner, Entity,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Render, ScrollWheelEvent,
    StatefulInteractiveElement, Styled, Window,
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
    /// When `true`, the grid swaps between the built-in light/dark
    /// [`GridTheme`] palettes to follow the OS window appearance. Disabled
    /// automatically when the caller supplies an explicit theme override.
    follow_system_appearance: bool,
    /// Retained appearance-observer subscription. Registered lazily on the
    /// first render (that is where a `Window` is available); dropping it would
    /// unregister the observer, so it is stored for the widget's lifetime.
    appearance_subscription: Option<gpui::Subscription>,
}

impl SqllyDataTable {
    /// Wrap an existing `Entity<GridState>`.
    #[must_use]
    pub fn new(state: Entity<GridState>) -> Self {
        Self {
            state,
            follow_system_appearance: true,
            appearance_subscription: None,
        }
    }

    /// Construct from `GridData` using the default [`GridConfig`].
    #[must_use]
    pub fn builder(data: GridData) -> SqllyDataTableBuilder {
        SqllyDataTableBuilder {
            data,
            config: GridConfig::default(),
            context_menu_provider: None,
            theme: None,
            debug_bar: false,
        }
    }
}

/// Builder for `SqllyDataTable`.
pub struct SqllyDataTableBuilder {
    data: GridData,
    config: GridConfig,
    context_menu_provider: Option<ContextMenuProviderHandle>,
    theme: Option<GridTheme>,
    debug_bar: bool,
}

impl SqllyDataTableBuilder {
    /// Override the entire [`GridConfig`].
    #[must_use]
    pub fn config(mut self, config: GridConfig) -> Self {
        self.config = config;
        self
    }

    /// Override the [`GridTheme`]. Supplying an explicit theme opts out of the
    /// automatic OS light/dark following; the grid uses exactly this theme.
    #[must_use]
    pub fn theme(mut self, theme: GridTheme) -> Self {
        self.theme = Some(theme);
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

    /// Enable or disable the debug status bar. When enabled, a bar is painted
    /// at the bottom of the grid showing click position, scroll offset, and
    /// hovered cell coordinates. Off by default.
    #[must_use]
    pub fn debug_bar(mut self, enabled: bool) -> Self {
        self.debug_bar = enabled;
        self
    }

    /// Build the widget inside the supplied [`gpui::App`].
    pub fn build(self, cx: &mut App) -> SqllyDataTable {
        let focus = cx.focus_handle();
        let provider = self.context_menu_provider;
        let theme_override = self.theme;
        let debug_bar = self.debug_bar;
        let follow_system_appearance = theme_override.is_none();
        let state = cx.new(|_cx| {
            let mut s = GridState::new(self.data, self.config, focus.clone());
            s.context_menu_provider = provider;
            s.debug_bar_enabled = debug_bar;
            if let Some(theme) = theme_override {
                s.theme = theme;
            }
            s
        });
        SqllyDataTable {
            state,
            follow_system_appearance,
            appearance_subscription: None,
        }
    }
}

impl Focusable for SqllyDataTable {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.state.read(cx).focus_handle.clone()
    }
}

impl Render for SqllyDataTable {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        // Follow the OS light/dark appearance: set the initial theme from the
        // current window appearance and register a one-time observer that
        // swaps the grid theme whenever the system appearance changes. Skipped
        // when the caller supplied an explicit theme override.
        if self.follow_system_appearance && self.appearance_subscription.is_none() {
            let initial = GridTheme::for_appearance(window.appearance());
            self.state.update(cx, |s, _cx| s.theme = initial);
            let state_appearance = self.state.clone();
            self.appearance_subscription =
                Some(window.observe_window_appearance(move |window, cx| {
                    let theme = GridTheme::for_appearance(window.appearance());
                    state_appearance.update(cx, |s, cx| {
                        s.theme = theme;
                        cx.notify();
                    });
                }));
        }

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
        let debug_bar = self.state.read(cx).debug_bar_enabled;
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
            .children(debug_bar.then(|| {
                canvas(
                    move |_bounds, _window, cx| -> StatusBarData {
                        let s = state_status.read(cx);
                        StatusBarData::from_state(s)
                    },
                    move |bounds, data, window, cx| {
                        paint_status_bar(&data, window, cx, bounds);
                    },
                )
                .h(px(status_h))
            }))
            .children(render_context_menu_overlay(&self.state, cx))
            .children(render_filter_panel_overlay(&self.state, cx))
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
                        if s.context_menu.is_some() || s.filter_panel.is_some() {
                            s.context_menu = None;
                            s.filter_panel = None;
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
                                    s.filter_panel = None;
                                }
                            }
                            cx.notify();
                            return;
                        }

                        // Provider exists — build custom menu.
                        let Some(target) = s.context_menu_target_from_hit(hit) else {
                            s.context_menu = None;
                            s.filter_panel = None;
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
                        s.filter_panel = None;
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
                        s.filter_panel = None;
                        cx.notify();
                    }
                });
            },
        ),
    ))
    .with_priority(CONTEXT_MENU_PRIORITY);

    Some(overlay)
}

/// Fixed width of the filter popover, in pixels.
const FILTER_PANEL_WIDTH: f32 = 300.0;
/// Max number of distinct value rows rendered at once (search narrows the set).
const FILTER_PANEL_MAX_ROWS: usize = 200;

/// Build the Numbers-style per-column filter popover as a `deferred` +
/// `anchored` overlay, using the exact mechanism as
/// [`render_context_menu_overlay`] so it paints and receives events outside the
/// grid's own layout bounds. Returns `None` when no panel is open.
#[allow(clippy::too_many_lines)]
fn render_filter_panel_overlay(
    state: &Entity<GridState>,
    cx: &mut Context<SqllyDataTable>,
) -> Option<impl IntoElement> {
    let s = state.read(cx);
    let panel = s.filter_panel.clone()?;
    let theme = s.theme.clone();
    let col = panel.col;
    let current_sort = s.sort;
    let filter_active = s.filters.get(col).is_some_and(|f| f.is_active());
    let grid_ox = f32::from(s.bounds.origin.x);
    let grid_oy = f32::from(s.bounds.origin.y);

    // Anchor (grid-relative) -> absolute window coords. The default
    // `SwitchAnchor` fit mode on `anchored()` handles viewport-edge flipping
    // automatically using the actual rendered height, so we don't need a
    // manual estimate or flip calculation here.
    let abs_x = grid_ox + f32::from(panel.anchor.x);
    let abs_y = grid_oy + f32::from(panel.anchor.y);

    // Palette (all `Hsla` are `Copy`, so they move freely into closures).
    let c_bg = theme.menu_bg;
    let c_line = theme.grid_line;
    let c_fg = theme.menu_fg;
    let c_accent = theme.sort_indicator;
    let c_hover = theme.menu_hover_bg;
    let c_muted = theme.muted_text;

    let checkbox = move |checked: bool| {
        let mut b = div()
            .w(px(14.0))
            .h(px(14.0))
            .border_1()
            .border_color(c_line)
            .bg(c_bg)
            .flex()
            .items_center()
            .justify_center();
        if checked {
            b = b.child(div().w(px(8.0)).h(px(8.0)).bg(c_accent));
        }
        b
    };

    // --- Sort row -----------------------------------------------------------
    let (asc_active, desc_active) = match current_sort {
        Some((c, SortDirection::Ascending)) if c == col => (true, false),
        Some((c, SortDirection::Descending)) if c == col => (false, true),
        _ => (false, false),
    };
    let st_asc = state.clone();
    let st_desc = state.clone();
    let sort_row = div()
        .flex()
        .gap(px(6.0))
        .child(
            div()
                .flex_1()
                .h(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(c_line)
                .bg(if asc_active { c_accent } else { c_hover })
                .cursor_pointer()
                .child("Ascending")
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    st_asc.update(cx, |s, cx| {
                        s.set_panel_sort(SortDirection::Ascending);
                        cx.notify();
                    });
                }),
        )
        .child(
            div()
                .flex_1()
                .h(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(c_line)
                .bg(if desc_active { c_accent } else { c_hover })
                .cursor_pointer()
                .child("Descending")
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    st_desc.update(cx, |s, cx| {
                        s.set_panel_sort(SortDirection::Descending);
                        cx.notify();
                    });
                }),
        );

    // --- Operator dropdown --------------------------------------------------
    let st_op_toggle = state.clone();
    let op_button = div()
        .h(px(26.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .border_1()
        .border_color(c_line)
        .bg(c_bg)
        .cursor_pointer()
        .child(panel.current_op_label())
        .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
            st_op_toggle.update(cx, |s, cx| {
                s.toggle_filter_op_menu();
                cx.notify();
            });
        });

    let op_menu = panel.op_menu_open.then(|| {
        let mut items: Vec<gpui::AnyElement> = Vec::new();
        for (i, label) in panel.op_labels().iter().enumerate() {
            let selected = i == panel.op_index;
            let st_pick = state.clone();
            items.push(
                div()
                    .h(px(24.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .bg(if selected { c_accent } else { c_bg })
                    .cursor_pointer()
                    .child(*label)
                    .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                        st_pick.update(cx, |s, cx| {
                            s.set_filter_operator(i);
                            cx.notify();
                        });
                    })
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(c_line)
            .bg(c_bg)
            .children(items)
    });

    // --- Operand field(s) ---------------------------------------------------
    let operand_field = |value: &str, focused: bool, placeholder: &str, input: FilterInput| {
        let st_focus = state.clone();
        let (text, is_placeholder) = if value.is_empty() {
            (placeholder.to_owned(), true)
        } else {
            (value.to_owned(), false)
        };
        div()
            .h(px(26.0))
            .px(px(6.0))
            .flex()
            .items_center()
            .gap(px(2.0))
            .border_1()
            .border_color(if focused { c_accent } else { c_line })
            .bg(c_bg)
            .cursor_pointer()
            .child(
                div()
                    .text_color(if is_placeholder { c_muted } else { c_fg })
                    .child(text),
            )
            .children(focused.then(|| div().w(px(1.0)).h(px(14.0)).bg(c_accent)))
            .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                st_focus.update(cx, |s, cx| {
                    s.set_filter_focus(input);
                    cx.notify();
                });
            })
    };

    let operand_placeholder = if panel.kind == crate::data::ColumnKind::Date {
        "YYYY-MM-DD"
    } else if crate::filter::uses_number_ops(panel.kind) {
        "value"
    } else if panel.op_index == 7 {
        // Text "matches" operator.
        "regex"
    } else {
        "value"
    };
    let operands = panel.needs_operand().then(|| {
        let mut row = div().flex().flex_col().gap(px(4.0)).child(operand_field(
            &panel.operand_a.value,
            panel.focus == FilterInput::OperandA,
            operand_placeholder,
            FilterInput::OperandA,
        ));
        if panel.needs_second_operand() {
            row = row
                .child(div().text_color(c_muted).text_size(px(11.0)).child("and"))
                .child(operand_field(
                    &panel.operand_b.value,
                    panel.focus == FilterInput::OperandB,
                    operand_placeholder,
                    FilterInput::OperandB,
                ));
        }
        row
    });

    // --- Search box ---------------------------------------------------------
    let st_search = state.clone();
    let search_focused = panel.focus == FilterInput::Search;
    let (search_text, search_is_ph) = if panel.search.value.is_empty() {
        ("Search".to_owned(), true)
    } else {
        (panel.search.value.clone(), false)
    };
    let search_box = div()
        .h(px(26.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .gap(px(2.0))
        .border_1()
        .border_color(if search_focused { c_accent } else { c_line })
        .bg(c_bg)
        .cursor_pointer()
        .child(
            div()
                .text_color(if search_is_ph { c_muted } else { c_fg })
                .child(search_text),
        )
        .children(search_focused.then(|| div().w(px(1.0)).h(px(14.0)).bg(c_accent)))
        .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
            st_search.update(cx, |s, cx| {
                s.set_filter_focus(FilterInput::Search);
                cx.notify();
            });
        });

    // --- (Select All) + value checklist ------------------------------------
    let st_all = state.clone();
    let select_all_row = div()
        .h(px(24.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .cursor_pointer()
        .child(checkbox(panel.all_checked()))
        .child("(Select All)")
        .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
            st_all.update(cx, |s, cx| {
                s.toggle_filter_select_all();
                cx.notify();
            });
        });

    let visible = panel.visible_indices();
    let mut value_rows: Vec<gpui::AnyElement> = Vec::new();
    for &idx in visible.iter().take(FILTER_PANEL_MAX_ROWS) {
        let row = &panel.distinct[idx];
        let st_val = state.clone();
        value_rows.push(
            div()
                .h(px(22.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .cursor_pointer()
                .child(checkbox(row.checked))
                .child(div().text_color(c_fg).child(row.label.clone()))
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    st_val.update(cx, |s, cx| {
                        s.toggle_filter_value(idx);
                        cx.notify();
                    });
                })
                .into_any_element(),
        );
    }
    let truncated = visible.len() > FILTER_PANEL_MAX_ROWS;
    let value_list = div()
        .id("filter-value-list")
        .flex()
        .flex_col()
        .max_h(px(180.0))
        .overflow_y_scroll()
        .children(value_rows)
        .children(truncated.then(|| {
            div()
                .text_color(c_muted)
                .text_size(px(11.0))
                .child("Refine search to see more…")
        }));

    // --- Clear (left, disabled when no active filter) + Close (right) -----
    let st_clear = state.clone();
    let st_close = state.clone();
    let clear_bg = if filter_active { c_hover } else { c_bg };
    let clear_fg = if filter_active { c_fg } else { c_muted };
    let clear_border = if filter_active { c_line } else { c_muted };
    let buttons_row = div()
        .flex()
        .gap(px(6.0))
        .child(
            div()
                .flex_1()
                .h(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(clear_border)
                .bg(clear_bg)
                .text_color(clear_fg)
                .cursor_pointer()
                .child("Clear Filter")
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    if !filter_active {
                        return;
                    }
                    st_clear.update(cx, |s, cx| {
                        s.clear_filter_panel();
                        cx.notify();
                    });
                }),
        )
        .child(
            div()
                .flex_1()
                .h(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(c_line)
                .bg(c_hover)
                .cursor_pointer()
                .child("Close")
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    st_close.update(cx, |s, cx| {
                        s.filter_panel = None;
                        cx.notify();
                    });
                }),
        );

    let panel_body = div()
        .flex()
        .flex_col()
        .w(px(FILTER_PANEL_WIDTH))
        .p(px(10.0))
        .gap(px(8.0))
        .bg(c_bg)
        .border_1()
        .border_color(c_line)
        .text_color(c_fg)
        .text_size(px(13.0))
        .child(div().text_color(c_muted).text_size(px(11.0)).child("Sort"))
        .child(sort_row)
        .child(
            div()
                .text_color(c_muted)
                .text_size(px(11.0))
                .child("Filter"),
        )
        .child(op_button)
        .children(op_menu)
        .children(operands)
        .child(search_box)
        .child(select_all_row)
        .child(value_list)
        .child(buttons_row);

    let st_backdrop = state.clone();
    let overlay = deferred(
        anchored()
            .anchor(Corner::BottomLeft)
            .position(point(px(abs_x), px(abs_y)))
            .child(div().occlude().child(panel_body).on_mouse_down_out(
                move |_e: &MouseDownEvent, _window, cx| {
                    st_backdrop.update(cx, |s, cx| {
                        if s.filter_panel.is_some() {
                            s.filter_panel = None;
                            cx.notify();
                        }
                    });
                },
            )),
    )
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
