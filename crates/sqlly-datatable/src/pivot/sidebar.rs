//! The pivot configuration sidebar: a field list plus the Rows / Columns /
//! Values / Filters drop zones, driven by GPUI's native drag-and-drop
//! (`on_drag` / `drag_over` / `on_drop`).
//!
//! The sidebar and the pivot grid share one [`Entity<PivotState>`]: every
//! drop mutates [`crate::pivot::PivotConfig`] through the same
//! `move_field` API available to programmatic callers, then triggers
//! `recompute()`.

use crate::pivot::aggregation::AggregationFn;
use crate::pivot::config::PivotZone;
use crate::pivot::state::PivotState;

use gpui::{
    anchored, deferred, div, point, px, App, AppContext as _, Context, Corner, Entity,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window,
};

/// Fixed sidebar width in pixels.
pub const SIDEBAR_WIDTH: f32 = 260.0;
/// Draw priority for the filter popover overlay (matches the grid's menus).
const POPOVER_PRIORITY: usize = 1_000_000;
/// Max checklist rows rendered in the filter popover.
const FILTER_POPOVER_MAX_ROWS: usize = 200;

/// Payload carried by a field-chip drag.
struct DraggedField {
    field: usize,
}

/// The ghost chip rendered under the cursor during a drag.
struct DragGhost {
    label: String,
    bg: gpui::Hsla,
    fg: gpui::Hsla,
    border: gpui::Hsla,
}

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .bg(self.bg)
            .border_1()
            .border_color(self.border)
            .text_color(self.fg)
            .text_size(px(12.0))
            .child(self.label.clone())
    }
}

/// Drag-and-drop pivot configuration sidebar. Owns nothing but a handle to
/// the shared [`PivotState`].
pub struct PivotSidebar {
    /// The shared pivot state.
    pub state: Entity<PivotState>,
}

impl PivotSidebar {
    /// Wrap an existing pivot state entity.
    #[must_use]
    pub fn new(state: Entity<PivotState>) -> Self {
        Self { state }
    }

    /// A draggable field chip. `zone` is `None` for the source field list.
    #[allow(clippy::too_many_arguments)]
    fn chip(
        state: &Entity<PivotState>,
        id: (&'static str, usize),
        field: usize,
        label: String,
        zone: Option<PivotZone>,
        removable: bool,
        marker: Option<&'static str>,
        cx: &App,
    ) -> gpui::AnyElement {
        let s = state.read(cx);
        let theme = s.theme.clone();
        let ghost_label = label.clone();
        let ghost_bg = theme.pivot_chip_bg;
        let ghost_fg = theme.pivot_chip_fg;
        let ghost_border = theme.grid_line;

        let state_drop = state.clone();
        let state_remove = state.clone();

        let mut chip = div()
            .id(id)
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .bg(theme.pivot_chip_bg)
            .border_1()
            .border_color(theme.grid_line)
            .text_color(theme.pivot_chip_fg)
            .text_size(px(12.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .cursor_pointer()
            .on_drag(
                DraggedField { field },
                move |_drag, _offset, _window, cx| {
                    cx.new(|_| DragGhost {
                        label: ghost_label.clone(),
                        bg: ghost_bg,
                        fg: ghost_fg,
                        border: ghost_border,
                    })
                },
            )
            .child(div().flex_1().child(label));

        if let Some(marker) = marker {
            chip = chip.child(
                div()
                    .text_color(theme.sort_indicator)
                    .text_size(px(11.0))
                    .child(marker),
            );
        }

        if removable {
            chip = chip.child(
                div()
                    .text_color(theme.muted_text)
                    .cursor_pointer()
                    .child("✕")
                    .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                        cx.stop_propagation();
                        state_remove.update(cx, |s, cx| {
                            s.config.remove_field(field);
                            s.recompute();
                            cx.notify();
                        });
                    }),
            );
        }

        // Dropping onto a chip inside a zone inserts the dragged field at
        // this chip's position (reorder support).
        if let Some(zone) = zone {
            chip = chip.on_drop::<DraggedField>(move |dragged, _window, cx| {
                cx.stop_propagation();
                let dragged_field = dragged.field;
                state_drop.update(cx, |s, cx| {
                    let index = s.config.fields_in(zone).iter().position(|&f| f == field);
                    s.config.move_field(dragged_field, zone, index);
                    s.recompute();
                    cx.notify();
                });
            });
        }

        chip.into_any_element()
    }

    /// One drop zone with its label, hint, and assigned chips.
    fn zone(
        state: &Entity<PivotState>,
        zone: PivotZone,
        title: &'static str,
        cx: &App,
    ) -> gpui::AnyElement {
        let s = state.read(cx);
        let theme = s.theme.clone();
        let fields = s.config.fields_in(zone);
        let columns = s.source_columns().to_vec();
        let agg = s.config.aggregation;
        let agg_menu_open = s.agg_menu_open;

        let state_drop = state.clone();
        let active_bg = theme.pivot_drop_zone_active_bg;

        let mut chips: Vec<gpui::AnyElement> = Vec::new();
        for (i, &field) in fields.iter().enumerate() {
            let name = columns
                .get(field)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| format!("column {field}"));
            match zone {
                PivotZone::Values => {
                    chips.push(Self::values_chip(
                        state,
                        field,
                        agg,
                        agg_menu_open,
                        name,
                        cx,
                    ));
                }
                PivotZone::Filters => {
                    let filter_on = s.filter_active(field);
                    let state_open = state.clone();
                    let chip = div()
                        .id(("pivot-filter-chip", i))
                        .on_mouse_down(MouseButton::Left, {
                            let state_open = state_open.clone();
                            move |e: &MouseDownEvent, _w, cx| {
                                state_open.update(cx, |s, cx| {
                                    s.open_filter_popover(field, e.position);
                                    cx.notify();
                                });
                            }
                        })
                        .child(Self::chip(
                            state,
                            ("pivot-filter-chip-inner", i),
                            field,
                            name,
                            Some(zone),
                            true,
                            filter_on.then_some("●"),
                            cx,
                        ));
                    chips.push(chip.into_any_element());
                }
                PivotZone::Rows | PivotZone::Columns => {
                    let id = match zone {
                        PivotZone::Rows => ("pivot-row-chip", i),
                        _ => ("pivot-col-chip", i),
                    };
                    chips.push(Self::chip(
                        state,
                        id,
                        field,
                        name,
                        Some(zone),
                        true,
                        None,
                        cx,
                    ));
                }
            }
        }

        let hint = if chips.is_empty() {
            Some(
                div()
                    .text_color(theme.muted_text)
                    .text_size(px(11.0))
                    .child("Drag fields here"),
            )
        } else {
            None
        };

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_color(theme.muted_text)
                    .text_size(px(11.0))
                    .child(title),
            )
            .child(
                div()
                    .min_h(px(40.0))
                    .p(px(6.0))
                    .rounded(px(4.0))
                    .bg(theme.pivot_drop_zone_bg)
                    .border_1()
                    .border_color(theme.grid_line)
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .drag_over::<DraggedField>(move |style, _, _, _| style.bg(active_bg))
                    .on_drop::<DraggedField>(move |dragged, _window, cx| {
                        let dragged_field = dragged.field;
                        state_drop.update(cx, |s, cx| {
                            s.config.move_field(dragged_field, zone, None);
                            s.recompute();
                            cx.notify();
                        });
                    })
                    .children(chips)
                    .children(hint),
            )
            .into_any_element()
    }

    /// The Values-zone chip: caption plus the aggregation picker.
    fn values_chip(
        state: &Entity<PivotState>,
        field: usize,
        agg: AggregationFn,
        menu_open: bool,
        name: String,
        cx: &App,
    ) -> gpui::AnyElement {
        let s = state.read(cx);
        let theme = s.theme.clone();
        let state_toggle = state.clone();
        let state_remove = state.clone();

        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let ghost_label = agg.caption(&name);
        let ghost_bg = theme.pivot_chip_bg;
        let ghost_fg = theme.pivot_chip_fg;
        let ghost_border = theme.grid_line;
        rows.push(
            div()
                .id("pivot-values-chip")
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(4.0))
                .bg(theme.pivot_chip_bg)
                .border_1()
                .border_color(theme.grid_line)
                .text_color(theme.pivot_chip_fg)
                .text_size(px(12.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .cursor_pointer()
                .on_drag(
                    DraggedField { field },
                    move |_drag, _offset, _window, cx| {
                        cx.new(|_| DragGhost {
                            label: ghost_label.clone(),
                            bg: ghost_bg,
                            fg: ghost_fg,
                            border: ghost_border,
                        })
                    },
                )
                .child(div().flex_1().child(agg.caption(&name)))
                .child(
                    div()
                        .cursor_pointer()
                        .text_color(theme.sort_indicator)
                        .child(if menu_open { "▲" } else { "▼" })
                        .on_mouse_down(MouseButton::Left, {
                            let state_toggle = state_toggle.clone();
                            move |_e: &MouseDownEvent, _w, cx| {
                                cx.stop_propagation();
                                state_toggle.update(cx, |s, cx| {
                                    s.agg_menu_open = !s.agg_menu_open;
                                    cx.notify();
                                });
                            }
                        }),
                )
                .child(
                    div()
                        .text_color(theme.muted_text)
                        .cursor_pointer()
                        .child("✕")
                        .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                            cx.stop_propagation();
                            state_remove.update(cx, |s, cx| {
                                s.config.remove_field(field);
                                s.recompute();
                                cx.notify();
                            });
                        }),
                )
                .into_any_element(),
        );

        if menu_open {
            for func in AggregationFn::all() {
                let selected = func == agg;
                let state_pick = state.clone();
                rows.push(
                    div()
                        .px(px(8.0))
                        .py(px(2.0))
                        .rounded(px(3.0))
                        .bg(if selected {
                            theme.menu_hover_bg
                        } else {
                            theme.menu_bg
                        })
                        .text_color(theme.menu_fg)
                        .text_size(px(12.0))
                        .cursor_pointer()
                        .child(func.label())
                        .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                            state_pick.update(cx, |s, cx| {
                                s.config.aggregation = func;
                                s.agg_menu_open = false;
                                s.recompute();
                                cx.notify();
                            });
                        })
                        .into_any_element(),
                );
            }
        }

        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(rows)
            .into_any_element()
    }

    /// A labelled checkbox row bound to one totals option.
    fn option_row(
        state: &Entity<PivotState>,
        label: &'static str,
        checked: bool,
        apply: impl Fn(&mut PivotState) + 'static,
        cx: &App,
    ) -> gpui::AnyElement {
        let theme = state.read(cx).theme.clone();
        let state_toggle = state.clone();
        let boxed = div()
            .w(px(12.0))
            .h(px(12.0))
            .border_1()
            .border_color(theme.grid_line)
            .bg(theme.menu_bg)
            .flex()
            .items_center()
            .justify_center()
            .children(checked.then(|| div().w(px(6.0)).h(px(6.0)).bg(theme.sort_indicator)));
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .cursor_pointer()
            .text_size(px(12.0))
            .text_color(theme.menu_fg)
            .child(boxed)
            .child(label)
            .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                state_toggle.update(cx, |s, cx| {
                    apply(s);
                    s.recompute();
                    cx.notify();
                });
            })
            .into_any_element()
    }

    /// Small inline text button.
    fn text_button(
        state: &Entity<PivotState>,
        label: &'static str,
        apply: impl Fn(&mut PivotState, &mut App) + 'static,
        cx: &App,
    ) -> gpui::AnyElement {
        let theme = state.read(cx).theme.clone();
        let state_btn = state.clone();
        div()
            .px(px(6.0))
            .py(px(2.0))
            .rounded(px(3.0))
            .border_1()
            .border_color(theme.grid_line)
            .bg(theme.pivot_drop_zone_bg)
            .text_color(theme.menu_fg)
            .text_size(px(11.0))
            .cursor_pointer()
            .child(label)
            .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                state_btn.update(cx, |s, cx| {
                    apply(s, cx);
                    cx.notify();
                });
            })
            .into_any_element()
    }

    /// The Filters-zone checklist popover overlay, if open.
    fn render_filter_popover(
        state: &Entity<PivotState>,
        cx: &App,
    ) -> Option<impl IntoElement + use<>> {
        let s = state.read(cx);
        let popover = s.filter_popover()?.clone();
        let theme = s.theme.clone();
        let field_name = s
            .source_columns()
            .get(popover.field)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        let checkbox = |checked: bool| {
            let mut b = div()
                .w(px(12.0))
                .h(px(12.0))
                .border_1()
                .border_color(theme.grid_line)
                .bg(theme.menu_bg)
                .flex()
                .items_center()
                .justify_center();
            if checked {
                b = b.child(div().w(px(6.0)).h(px(6.0)).bg(theme.sort_indicator));
            }
            b
        };

        let state_all = state.clone();
        let select_all = div()
            .h(px(22.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .cursor_pointer()
            .child(checkbox(popover.all_checked()))
            .child("(Select All)")
            .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                state_all.update(cx, |s, cx| {
                    s.toggle_filter_popover_select_all();
                    cx.notify();
                });
            });

        let mut value_rows: Vec<gpui::AnyElement> = Vec::new();
        for (i, row) in popover
            .rows
            .iter()
            .enumerate()
            .take(FILTER_POPOVER_MAX_ROWS)
        {
            let state_val = state.clone();
            value_rows.push(
                div()
                    .h(px(20.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .cursor_pointer()
                    .child(checkbox(row.checked))
                    .child(div().text_color(theme.menu_fg).child(row.label.clone()))
                    .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                        state_val.update(cx, |s, cx| {
                            s.toggle_filter_popover_value(i);
                            cx.notify();
                        });
                    })
                    .into_any_element(),
            );
        }
        let truncated = popover.rows.len() > FILTER_POPOVER_MAX_ROWS;

        let state_clear = state.clone();
        let state_close = state.clone();
        let clear_field = popover.field;
        let buttons = div()
            .flex()
            .gap(px(6.0))
            .child(
                div()
                    .flex_1()
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_color(theme.grid_line)
                    .bg(theme.menu_hover_bg)
                    .cursor_pointer()
                    .child("Clear")
                    .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                        state_clear.update(cx, |s, cx| {
                            s.clear_filter(clear_field);
                            cx.notify();
                        });
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_color(theme.grid_line)
                    .bg(theme.menu_hover_bg)
                    .cursor_pointer()
                    .child("Close")
                    .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                        state_close.update(cx, |s, cx| {
                            s.close_filter_popover();
                            cx.notify();
                        });
                    }),
            );

        let body = div()
            .flex()
            .flex_col()
            .w(px(240.0))
            .p(px(8.0))
            .gap(px(6.0))
            .bg(theme.menu_bg)
            .border_1()
            .border_color(theme.grid_line)
            .text_color(theme.menu_fg)
            .text_size(px(12.0))
            .child(
                div()
                    .text_color(theme.muted_text)
                    .text_size(px(11.0))
                    .child(format!("Filter: {field_name}")),
            )
            .child(select_all)
            .child(
                div()
                    .id("pivot-filter-values")
                    .flex()
                    .flex_col()
                    .max_h(px(200.0))
                    .overflow_y_scroll()
                    .children(value_rows)
                    .children(truncated.then(|| {
                        div()
                            .text_color(theme.muted_text)
                            .text_size(px(11.0))
                            .child(format!("Showing first {FILTER_POPOVER_MAX_ROWS} values…"))
                    })),
            )
            .child(buttons);

        let state_backdrop = state.clone();
        let overlay = deferred(
            anchored()
                .anchor(Corner::TopLeft)
                .position(point(popover.anchor.x, popover.anchor.y))
                .child(div().occlude().child(body).on_mouse_down_out(
                    move |_e: &MouseDownEvent, _window, cx| {
                        state_backdrop.update(cx, |s, cx| {
                            if s.filter_popover().is_some() {
                                s.close_filter_popover();
                                cx.notify();
                            }
                        });
                    },
                )),
        )
        .with_priority(POPOVER_PRIORITY);
        Some(overlay)
    }
}

impl Render for PivotSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (theme, columns, config) = {
            let s = self.state.read(cx);
            (
                s.theme.clone(),
                s.source_columns().to_vec(),
                s.config.clone(),
            )
        };

        // Source field list with zone badges.
        let mut field_chips: Vec<gpui::AnyElement> = Vec::new();
        for (i, col) in columns.iter().enumerate() {
            let badge = match config.zone_of(i) {
                Some(PivotZone::Rows) => Some("R"),
                Some(PivotZone::Columns) => Some("C"),
                Some(PivotZone::Values) => Some("V"),
                Some(PivotZone::Filters) => Some("F"),
                None => None,
            };
            field_chips.push(Self::chip(
                &self.state,
                ("pivot-source-field", i),
                i,
                col.name.clone(),
                None,
                false,
                badge,
                cx,
            ));
        }

        let options = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_color(theme.muted_text)
                    .text_size(px(11.0))
                    .child("Totals"),
            )
            .child(Self::option_row(
                &self.state,
                "Row subtotals",
                config.show_row_subtotals,
                |s| s.config.show_row_subtotals = !s.config.show_row_subtotals,
                cx,
            ))
            .child(Self::option_row(
                &self.state,
                "Column subtotal columns",
                config.show_column_subtotals,
                |s| s.config.show_column_subtotals = !s.config.show_column_subtotals,
                cx,
            ))
            .child(Self::option_row(
                &self.state,
                "Grand total row",
                config.show_row_grand_total,
                |s| s.config.show_row_grand_total = !s.config.show_row_grand_total,
                cx,
            ))
            .child(Self::option_row(
                &self.state,
                "Grand total column",
                config.show_column_grand_total,
                |s| s.config.show_column_grand_total = !s.config.show_column_grand_total,
                cx,
            ));

        let tools = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_color(theme.muted_text)
                    .text_size(px(11.0))
                    .child("Groups"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(4.0))
                    .child(Self::text_button(
                        &self.state,
                        "Expand rows",
                        |s, _| s.expand_all_rows(),
                        cx,
                    ))
                    .child(Self::text_button(
                        &self.state,
                        "Collapse rows",
                        |s, _| s.collapse_all_rows(),
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(px(4.0))
                    .child(Self::text_button(
                        &self.state,
                        "Expand cols",
                        |s, _| s.expand_all_cols(),
                        cx,
                    ))
                    .child(Self::text_button(
                        &self.state,
                        "Collapse cols",
                        |s, _| s.collapse_all_cols(),
                        cx,
                    )),
            )
            .child(
                div()
                    .text_color(theme.muted_text)
                    .text_size(px(11.0))
                    .child("Export"),
            )
            .child(Self::text_button(
                &self.state,
                "Copy pivot as CSV",
                |s, cx| {
                    let csv = s.to_csv();
                    if !csv.is_empty() {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(csv));
                    }
                },
                cx,
            ));

        div()
            .id("pivot-sidebar")
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(8.0))
            .bg(theme.menu_bg)
            .border_r_1()
            .border_color(theme.grid_line)
            .text_color(theme.menu_fg)
            .text_size(px(12.0))
            .overflow_y_scroll()
            .child(
                div()
                    .text_color(theme.header_fg)
                    .text_size(px(13.0))
                    .child("Pivot Fields"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .text_color(theme.muted_text)
                            .text_size(px(11.0))
                            .child("Fields (drag into a zone)"),
                    )
                    .child(
                        div()
                            .id("pivot-field-list")
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .max_h(px(220.0))
                            .overflow_y_scroll()
                            .children(field_chips),
                    ),
            )
            .child(Self::zone(&self.state, PivotZone::Filters, "Filters", cx))
            .child(Self::zone(&self.state, PivotZone::Columns, "Columns", cx))
            .child(Self::zone(&self.state, PivotZone::Rows, "Rows", cx))
            .child(Self::zone(&self.state, PivotZone::Values, "Values", cx))
            .child(options)
            .child(tools)
            .children(Self::render_filter_popover(&self.state, cx))
    }
}
