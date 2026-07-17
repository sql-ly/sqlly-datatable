//! The pivot configuration sidebar: a field list plus the Rows / Columns /
//! Values / Filters drop zones, driven by GPUI's native drag-and-drop
//! (`on_drag` / `drag_over` / `on_drop`).
//!
//! The sidebar and the pivot grid share one [`Entity<PivotState>`]: every
//! drop mutates [`crate::pivot::PivotConfig`] through the same
//! `move_field` API available to programmatic callers, then triggers
//! `recompute()`.

use crate::grid::theme::GridTheme;
use crate::pivot::aggregation::AggregationFn;
use crate::pivot::config::PivotZone;
use crate::pivot::state::PivotState;

use gpui::{
    anchored, deferred, div, point, px, App, AppContext as _, Context, Corner, Entity, FontWeight,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window,
};

/// Draw priority for the filter popover overlay (matches the grid's menus).
const POPOVER_PRIORITY: usize = 1_000_000;
/// Max checklist rows rendered in the filter popover.
const FILTER_POPOVER_MAX_ROWS: usize = 200;

/// Horizontal chrome between the sidebar edge and accordion content: sidebar
/// padding (8×2), accordion border (1×2), accordion content padding (16×2).
const SIDEBAR_CONTENT_CHROME: f32 = 50.0;
/// Horizontal chrome of a drop-zone box: padding (6×2) + border (1×2).
const ZONE_CHROME: f32 = 14.0;
/// Horizontal chrome of a chip: padding (8×2) + border (1×2).
const CHIP_CHROME: f32 = 18.0;
/// Flex gap between a chip's label and its trailing glyphs (marker, picker
/// arrow, remove button).
const CHIP_GAP: f32 = 6.0;

/// Fixed outer height of every field chip (source list, zone chips, drag
/// ghost). An explicit integer height keeps the stacked-chip rhythm exact:
/// with padding-derived heights the fractional text line height made the
/// 3px list gaps render as anything from 2px to 4px.
const CHIP_HEIGHT: f32 = 24.0;
/// Chip label font size.
const CHIP_FONT_SIZE: f32 = 12.0;
/// Zone-marker / badge font size.
const MARKER_FONT_SIZE: f32 = 11.0;

/// Measured pixel width of `text` at `size` in the window's default UI font.
fn measure_text(window: &Window, text: &str, size: f32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let run = gpui::TextRun {
        len: text.len(),
        color: gpui::Hsla::default(),
        font: window.text_style().font(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(
        SharedString::from(text.to_owned()),
        px(size),
        &[run],
        None,
    );
    f32::from(line.width)
}

/// Hover tooltip showing a chip's full label when its text is chopped.
struct ChipTooltip {
    label: SharedString,
    bg: gpui::Hsla,
    fg: gpui::Hsla,
    border: gpui::Hsla,
}

impl Render for ChipTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(4.0))
            .bg(self.bg)
            .border_1()
            .border_color(self.border)
            .text_color(self.fg)
            .text_size(px(CHIP_FONT_SIZE))
            .child(self.label.clone())
    }
}

/// A little floppy-disk glyph drawn with divs (no icon font dependency).
fn disk_icon(color: gpui::Hsla) -> gpui::Div {
    div()
        .w(px(13.0))
        .h(px(13.0))
        .rounded(px(2.0))
        .border_1()
        .border_color(color)
        .relative()
        .child(
            div()
                .absolute()
                .top(px(0.0))
                .left(px(3.0))
                .w(px(5.0))
                .h(px(4.0))
                .border_1()
                .border_color(color),
        )
        .child(
            div()
                .absolute()
                .bottom(px(1.0))
                .left(px(2.0))
                .w(px(7.0))
                .h(px(4.0))
                .bg(color),
        )
}

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
            .h(px(CHIP_HEIGHT))
            .flex()
            .items_center()
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
    expanded_sections: Vec<SharedString>,
}

impl PivotSidebar {
    /// Wrap an existing pivot state entity.
    #[must_use]
    pub fn new(state: Entity<PivotState>) -> Self {
        Self {
            state,
            expanded_sections: vec!["fields".into(), "layout".into(), "display".into()],
        }
    }

    fn set_section_expanded(&mut self, id: &SharedString, expanded: bool) {
        if expanded {
            if !self.expanded_sections.contains(id) {
                self.expanded_sections.push(id.clone());
            }
        } else {
            self.expanded_sections.retain(|section| section != id);
        }
    }

    /// One collapsible sidebar section: a clickable header row (title, an
    /// optional extra control next to the title, expand indicator) over an
    /// optional content body. Hand-rolled — instead of the `gpui-ui-kit`
    /// accordion, whose header only accepts a title string — so the Layout
    /// header can host the save-configuration button.
    #[allow(clippy::too_many_arguments)]
    fn section(
        sidebar: &Entity<PivotSidebar>,
        theme: &GridTheme,
        id: &'static str,
        title: &'static str,
        header_extra: Option<gpui::AnyElement>,
        expanded: bool,
        first: bool,
        content: gpui::AnyElement,
    ) -> gpui::AnyElement {
        let sidebar = sidebar.clone();
        let section_id: SharedString = id.into();
        let hover_bg = theme.menu_hover_bg;

        let mut header = div()
            .id(SharedString::from(format!("pivot-section-{id}")))
            .flex()
            .items_center()
            .justify_between()
            .px(px(16.0))
            .py(px(12.0))
            .bg(theme.header_bg)
            .cursor_pointer()
            .hover(move |style| style.bg(hover_bg))
            .on_mouse_up(MouseButton::Left, move |_e: &MouseUpEvent, _w, cx| {
                sidebar.update(cx, |this, cx| {
                    this.set_section_expanded(&section_id, !expanded);
                    cx.notify();
                });
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.header_fg)
                            .child(title),
                    )
                    .children(header_extra),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme.muted_text)
                    .child(if expanded { "▼" } else { "▶" }),
            );
        if !first {
            header = header.border_t_1().border_color(theme.grid_line);
        }

        let mut wrapper = div().flex().flex_col().child(header);
        if expanded {
            wrapper = wrapper.child(
                div()
                    .px(px(16.0))
                    .py(px(12.0))
                    .bg(theme.menu_bg)
                    .border_t_1()
                    .border_color(theme.grid_line)
                    .child(content),
            );
        }
        wrapper.into_any_element()
    }

    /// A draggable field chip. `zone` is `None` for the source field list.
    /// `label_budget` is the chip's inner content width; when the label (plus
    /// its trailing glyphs) can't fit, the text is ellipsized and a hover
    /// tooltip shows the full name.
    #[allow(clippy::too_many_arguments)]
    fn chip(
        state: &Entity<PivotState>,
        id: (&'static str, usize),
        field: usize,
        label: String,
        zone: Option<PivotZone>,
        removable: bool,
        marker: Option<&'static str>,
        label_budget: f32,
        window: &Window,
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

        let mut available = label_budget;
        if let Some(marker) = marker {
            available -= CHIP_GAP + measure_text(window, marker, MARKER_FONT_SIZE);
        }
        if removable {
            available -= CHIP_GAP + measure_text(window, "✕", CHIP_FONT_SIZE);
        }
        let chopped = measure_text(window, &label, CHIP_FONT_SIZE) > available + 0.5;

        let mut chip = div()
            .id(id)
            .px(px(8.0))
            .h(px(CHIP_HEIGHT))
            .flex_none()
            .rounded(px(4.0))
            .bg(theme.pivot_chip_bg)
            .border_1()
            .border_color(theme.grid_line)
            .text_color(theme.pivot_chip_fg)
            .text_size(px(CHIP_FONT_SIZE))
            .flex()
            .items_center()
            .gap(px(CHIP_GAP))
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
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .child(label.clone()),
            );

        if chopped {
            let tip_label: SharedString = label.into();
            let tip_bg = theme.menu_bg;
            let tip_fg = theme.menu_fg;
            let tip_border = theme.grid_line;
            chip = chip.tooltip(move |_window, cx| {
                cx.new(|_| ChipTooltip {
                    label: tip_label.clone(),
                    bg: tip_bg,
                    fg: tip_fg,
                    border: tip_border,
                })
                .into()
            });
        }

        if let Some(marker) = marker {
            chip = chip.child(
                div()
                    .flex_none()
                    .text_color(theme.sort_indicator)
                    .text_size(px(MARKER_FONT_SIZE))
                    .child(marker),
            );
        }

        if removable {
            chip = chip.child(
                div()
                    .flex_none()
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
        // this chip's position (reorder support). Double-clicking a zone chip
        // opens the per-field format dialog.
        if let Some(zone) = zone {
            let state_dialog = state.clone();
            chip = chip
                .on_drop::<DraggedField>(move |dragged, _window, cx| {
                    cx.stop_propagation();
                    let dragged_field = dragged.field;
                    state_drop.update(cx, |s, cx| {
                        let index = s.config.fields_in(zone).iter().position(|&f| f == field);
                        s.config.move_field(dragged_field, zone, index);
                        s.recompute();
                        cx.notify();
                    });
                })
                .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _w, cx| {
                    if e.click_count == 2 {
                        cx.stop_propagation();
                        state_dialog.update(cx, |s, cx| {
                            s.close_filter_popover();
                            s.open_format_dialog(field, zone, e.position);
                            cx.notify();
                        });
                    }
                });
        }

        chip.into_any_element()
    }

    /// One drop zone with its label, hint, and assigned chips. `content_w` is
    /// the accordion content width available to the zone box.
    fn zone(
        state: &Entity<PivotState>,
        zone: PivotZone,
        title: &'static str,
        content_w: f32,
        window: &Window,
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
        let label_budget = content_w - ZONE_CHROME - CHIP_CHROME;

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
                        label_budget,
                        window,
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
                                // Double-clicks open the format dialog (see
                                // the inner chip); only single clicks open
                                // the checklist. The popover is anchored a
                                // little below the cursor so the second
                                // click of a double-click still reaches the
                                // chip instead of the freshly opened popover.
                                if e.click_count != 1 {
                                    return;
                                }
                                let anchor = point(e.position.x, e.position.y + px(20.0));
                                state_open.update(cx, |s, cx| {
                                    s.open_filter_popover(field, anchor);
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
                            filter_on.then_some("🔽"),
                            label_budget,
                            window,
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
                        label_budget,
                        window,
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

    /// The Values-zone chip: caption plus the aggregation picker. The caption
    /// is ellipsized so both the picker arrow and the remove button always
    /// fit; a hover tooltip shows the full caption when chopped.
    #[allow(clippy::too_many_arguments)]
    fn values_chip(
        state: &Entity<PivotState>,
        field: usize,
        agg: AggregationFn,
        menu_open: bool,
        name: String,
        label_budget: f32,
        window: &Window,
        cx: &App,
    ) -> gpui::AnyElement {
        let s = state.read(cx);
        let theme = s.theme.clone();
        let state_toggle = state.clone();
        let state_remove = state.clone();

        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let caption = agg.caption(&name);
        let ghost_label = caption.clone();
        let ghost_bg = theme.pivot_chip_bg;
        let ghost_fg = theme.pivot_chip_fg;
        let ghost_border = theme.grid_line;

        let arrow = if menu_open { "▲" } else { "▼" };
        let available = label_budget
            - (CHIP_GAP + measure_text(window, arrow, CHIP_FONT_SIZE))
            - (CHIP_GAP + measure_text(window, "✕", CHIP_FONT_SIZE));
        let chopped = measure_text(window, &caption, CHIP_FONT_SIZE) > available + 0.5;

        let mut chip = div()
            .id("pivot-values-chip")
            .px(px(8.0))
            .h(px(CHIP_HEIGHT))
            .flex_none()
            .rounded(px(4.0))
            .bg(theme.pivot_chip_bg)
            .border_1()
            .border_color(theme.grid_line)
            .text_color(theme.pivot_chip_fg)
            .text_size(px(CHIP_FONT_SIZE))
            .flex()
            .items_center()
            .gap(px(CHIP_GAP))
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
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .child(caption.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .cursor_pointer()
                    .text_color(theme.sort_indicator)
                    .child(arrow)
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
                    .flex_none()
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

        if chopped {
            let tip_label: SharedString = caption.into();
            let tip_bg = theme.menu_bg;
            let tip_fg = theme.menu_fg;
            let tip_border = theme.grid_line;
            chip = chip.tooltip(move |_window, cx| {
                cx.new(|_| ChipTooltip {
                    label: tip_label.clone(),
                    bg: tip_bg,
                    fg: tip_fg,
                    border: tip_border,
                })
                .into()
            });
        }

        let state_dialog = state.clone();
        chip = chip.on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _w, cx| {
            if e.click_count == 2 {
                cx.stop_propagation();
                state_dialog.update(cx, |s, cx| {
                    s.open_format_dialog(field, PivotZone::Values, e.position);
                    cx.notify();
                });
            }
        });

        rows.push(chip.into_any_element());

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
        let hover_bg = theme.menu_hover_bg;
        // Checked: accent-filled box with a knockout check (the theme's `bg`
        // reads against the accent in both light and dark). Unchecked: an
        // outlined empty box.
        let boxed = div()
            .w(px(16.0))
            .h(px(16.0))
            .flex_none()
            .rounded(px(3.0))
            .border_1()
            .border_color(if checked {
                theme.sort_indicator
            } else {
                theme.grid_line
            })
            .bg(if checked {
                theme.sort_indicator
            } else {
                theme.menu_bg
            })
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.bg)
            .children(checked.then_some("✓"));
        div()
            .id(SharedString::from(format!("pivot-option-{label}")))
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(4.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .hover(move |style| style.bg(hover_bg))
            .text_size(px(13.0))
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
        let hover_bg = theme.menu_hover_bg;
        div()
            .id(SharedString::from(format!("pivot-button-{label}")))
            .px(px(10.0))
            .py(px(4.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(theme.grid_line)
            .bg(theme.pivot_drop_zone_bg)
            .hover(move |style| style.bg(hover_bg))
            .text_color(theme.menu_fg)
            .text_size(px(12.0))
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
                    // Own the wheel in the popover's value list so it doesn't
                    // also scroll the sidebar behind it (see the field list).
                    .occlude()
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

    /// The per-field format dialog overlay (chip double-click), if open.
    /// Every control applies immediately; "Reset" drops the override.
    #[allow(clippy::too_many_lines)]
    fn render_format_dialog(
        state: &Entity<PivotState>,
        cx: &App,
    ) -> Option<impl IntoElement + use<>> {
        use crate::config::{NumberFormat, TextAlignment};

        let s = state.read(cx);
        let dialog = *s.format_dialog()?;
        let fmt = s.format_dialog_format()?;
        let theme = s.theme.clone();
        let field_name = s
            .source_columns()
            .get(dialog.field)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        let c_bg = theme.menu_bg;
        let c_line = theme.grid_line;
        let c_fg = theme.menu_fg;
        let c_muted = theme.muted_text;
        let c_accent = theme.sort_indicator;
        let c_hover = theme.menu_hover_bg;

        let checkbox = move |checked: bool| {
            let mut b = div()
                .w(px(12.0))
                .h(px(12.0))
                .border_1()
                .border_color(c_line)
                .bg(c_bg)
                .flex()
                .items_center()
                .justify_center();
            if checked {
                b = b.child(div().w(px(6.0)).h(px(6.0)).bg(c_accent));
            }
            b
        };

        let check_row = |label: &'static str, checked: bool, apply: fn(&mut NumberFormat)| {
            let st = state.clone();
            div()
                .h(px(22.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .cursor_pointer()
                .child(checkbox(checked))
                .child(label)
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    st.update(cx, |s, cx| {
                        s.update_format_dialog(apply);
                        cx.notify();
                    });
                })
        };

        // Segmented pair choosing how negatives are written.
        let neg_btn = |label: &'static str, parens: bool| {
            let st = state.clone();
            let selected = fmt.negative_parentheses == parens;
            div()
                .flex_1()
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(c_line)
                .bg(if selected { c_accent } else { c_hover })
                .cursor_pointer()
                .child(label)
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    st.update(cx, |s, cx| {
                        s.update_format_dialog(|f| f.negative_parentheses = parens);
                        cx.notify();
                    });
                })
        };

        let dec_btn = |label: &'static str, apply: fn(&mut NumberFormat)| {
            let st = state.clone();
            div()
                .w(px(20.0))
                .h(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(c_line)
                .bg(c_hover)
                .cursor_pointer()
                .child(label)
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    st.update(cx, |s, cx| {
                        s.update_format_dialog(apply);
                        cx.notify();
                    });
                })
        };

        let align_btn = |label: &'static str, value: TextAlignment| {
            let st = state.clone();
            let selected = fmt.alignment == value;
            div()
                .flex_1()
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(c_line)
                .bg(if selected { c_accent } else { c_hover })
                .cursor_pointer()
                .child(label)
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    st.update(cx, |s, cx| {
                        s.update_format_dialog(move |f| f.alignment = value);
                        cx.notify();
                    });
                })
        };

        let st_reset = state.clone();
        let st_close = state.clone();
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
                    .border_color(c_line)
                    .bg(c_hover)
                    .cursor_pointer()
                    .child("Reset")
                    .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                        st_reset.update(cx, |s, cx| {
                            s.reset_format_dialog();
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
                    .border_color(c_line)
                    .bg(c_hover)
                    .cursor_pointer()
                    .child("Close")
                    .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                        st_close.update(cx, |s, cx| {
                            s.close_format_dialog();
                            cx.notify();
                        });
                    }),
            );

        let body = div()
            .flex()
            .flex_col()
            .w(px(230.0))
            .p(px(8.0))
            .gap(px(6.0))
            .bg(c_bg)
            .border_1()
            .border_color(c_line)
            .text_color(c_fg)
            .text_size(px(12.0))
            .child(
                div()
                    .text_color(c_muted)
                    .text_size(px(11.0))
                    .child(format!("Format: {field_name}")),
            )
            .child(check_row(
                "Negative numbers in red",
                fmt.show_negative_red,
                |f| f.show_negative_red = !f.show_negative_red,
            ))
            .child(check_row(
                "Thousands separator",
                fmt.thousands_separator,
                |f| f.thousands_separator = !f.thousands_separator,
            ))
            .child(
                div()
                    .text_color(c_muted)
                    .text_size(px(11.0))
                    .child("Negatives"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(neg_btn("-1,234", false))
                    .child(neg_btn("(1,234)", true)),
            )
            .child(
                div()
                    .h(px(22.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_color(c_muted)
                            .text_size(px(11.0))
                            .child("Decimals"),
                    )
                    .child(dec_btn("-", |f| f.decimals = f.decimals.saturating_sub(1)))
                    .child(
                        div()
                            .w(px(18.0))
                            .flex()
                            .justify_center()
                            .child(fmt.decimals.to_string()),
                    )
                    .child(dec_btn("+", |f| f.decimals = (f.decimals + 1).min(8))),
            )
            .child(
                div()
                    .text_color(c_muted)
                    .text_size(px(11.0))
                    .child("Alignment"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(align_btn("Left", TextAlignment::Left))
                    .child(align_btn("Center", TextAlignment::Center))
                    .child(align_btn("Right", TextAlignment::Right)),
            )
            .child(buttons);

        let state_backdrop = state.clone();
        let overlay = deferred(
            anchored()
                .anchor(Corner::TopLeft)
                .position(point(dialog.anchor.x, dialog.anchor.y))
                .child(div().occlude().child(body).on_mouse_down_out(
                    move |_e: &MouseDownEvent, _window, cx| {
                        state_backdrop.update(cx, |s, cx| {
                            if s.format_dialog().is_some() {
                                s.close_format_dialog();
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (theme, columns, config, sidebar_width, save_handler) = {
            let s = self.state.read(cx);
            (
                s.theme.clone(),
                s.source_columns().to_vec(),
                s.config.clone(),
                s.sidebar_width,
                s.save_config_handler.clone(),
            )
        };
        let content_w = sidebar_width - SIDEBAR_CONTENT_CHROME;

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
                content_w - CHIP_CHROME,
                window,
                cx,
            ));
        }

        let options = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .text_color(theme.muted_text)
                    .text_size(px(12.0))
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
            .gap(px(6.0))
            .child(
                div()
                    .text_color(theme.muted_text)
                    .text_size(px(12.0))
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
                    .text_size(px(12.0))
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

        let fields = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .text_color(theme.muted_text)
                    .text_size(px(11.0))
                    .child("Drag a field into a layout zone"),
            )
            .child(
                div()
                    .id("pivot-field-list")
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .max_h(px(220.0))
                    .overflow_y_scroll()
                    // Own the wheel within the list's own area. Without this
                    // the outer `#pivot-sidebar` scroll container (also under
                    // the cursor) receives the same wheel event — GPUI's
                    // overflow-scroll handler never stops propagation — so the
                    // whole control panel scrolled along with the list. As a
                    // `BlockMouse` hitbox the list truncates the scroll
                    // hit-test, leaving the sidebar out of it.
                    .occlude()
                    .children(field_chips),
            );

        // Save-configuration button, rendered in the Layout section header
        // next to its title. Only present while a host wired up the action
        // (via `PivotState::on_save_config` or the widget builder).
        let save_button = save_handler.map(|handler| {
            let state_save = self.state.clone();
            let icon_color = theme.muted_text;
            let hover_bg = theme.pivot_drop_zone_active_bg;
            let tip_bg = theme.menu_bg;
            let tip_fg = theme.menu_fg;
            let tip_border = theme.grid_line;
            div()
                .id("pivot-save-config")
                .p(px(2.0))
                .rounded(px(3.0))
                .cursor_pointer()
                .hover(move |style| style.bg(hover_bg))
                .child(disk_icon(icon_color))
                .tooltip(move |_window, cx| {
                    cx.new(|_| ChipTooltip {
                        label: "Save configuration".into(),
                        bg: tip_bg,
                        fg: tip_fg,
                        border: tip_border,
                    })
                    .into()
                })
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    cx.stop_propagation();
                    let config = state_save.read(cx).config.clone();
                    handler(&config, cx);
                })
                // The section header toggles on mouse-up; swallow it so a
                // click on the save button doesn't also collapse the section.
                .on_mouse_up(MouseButton::Left, |_e: &MouseUpEvent, _w, cx| {
                    cx.stop_propagation();
                })
                .into_any_element()
        });

        let layout = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(Self::zone(
                &self.state,
                PivotZone::Filters,
                "Filters",
                content_w,
                window,
                cx,
            ))
            .child(Self::zone(
                &self.state,
                PivotZone::Columns,
                "Columns",
                content_w,
                window,
                cx,
            ))
            .child(Self::zone(
                &self.state,
                PivotZone::Rows,
                "Rows",
                content_w,
                window,
                cx,
            ))
            .child(Self::zone(
                &self.state,
                PivotZone::Values,
                "Values",
                content_w,
                window,
                cx,
            ));

        let display = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(options)
            .child(tools);

        let sidebar = cx.entity().clone();
        let is_expanded = |id: &str| self.expanded_sections.iter().any(|section| section == id);
        let sections = div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(theme.grid_line)
            .rounded_lg()
            .child(Self::section(
                &sidebar,
                &theme,
                "fields",
                "Fields",
                None,
                is_expanded("fields"),
                true,
                fields.into_any_element(),
            ))
            .child(Self::section(
                &sidebar,
                &theme,
                "layout",
                "Layout",
                save_button,
                is_expanded("layout"),
                false,
                layout.into_any_element(),
            ))
            .child(Self::section(
                &sidebar,
                &theme,
                "display",
                "Display and export",
                None,
                is_expanded("display"),
                false,
                display.into_any_element(),
            ));

        div()
            .id("pivot-sidebar")
            .h_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(8.0))
            .bg(theme.menu_bg)
            .text_color(theme.menu_fg)
            .text_size(px(12.0))
            .overflow_y_scroll()
            .child(
                div()
                    .text_color(theme.header_fg)
                    .text_size(px(13.0))
                    .child("Pivot Controls"),
            )
            .child(sections)
            .children(Self::render_filter_popover(&self.state, cx))
            .children(Self::render_format_dialog(&self.state, cx))
    }
}
