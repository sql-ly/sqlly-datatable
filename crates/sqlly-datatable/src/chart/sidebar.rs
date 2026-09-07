//! The chart configuration sidebar: chart type, category/value/aggregate/
//! order/limit pickers, the click-to-navigate toggle, and SVG export —
//! styled like the pivot sidebar's accordion sections.
//!
//! The sidebar and the chart canvas share one [`Entity<ChartState>`]: every
//! picker mutates [`crate::chart::ChartConfig`] through `set_config` (which
//! recomputes the extraction), the same API available to programmatic
//! callers.

use crate::grid::theme::GridTheme;

use gpui::{
    div, px, Context, Entity, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseUpEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window,
};
use gpui_component::checkbox::Checkbox;
use gpui_component::{Icon, IconName};

use super::config::{ChartAggregate, ChartConfig, ChartKind, ChartOrder};
use super::model::is_numeric_column;
use super::state::ChartState;

/// Row height of every selectable picker row.
const ROW_HEIGHT: f32 = 26.0;
/// Font size of section titles and picker rows.
const ROW_FONT_SIZE: f32 = 12.0;
/// Font size of group labels ("Category", "Values", …).
const GROUP_FONT_SIZE: f32 = 11.0;

/// The chart configuration sidebar. Owns nothing but a handle to the shared
/// [`ChartState`].
pub struct ChartSidebar {
    /// The shared chart state.
    pub state: Entity<ChartState>,
    expanded_sections: Vec<SharedString>,
}

impl ChartSidebar {
    /// Wrap an existing chart state entity.
    #[must_use]
    pub fn new(state: Entity<ChartState>) -> Self {
        Self {
            state,
            expanded_sections: vec![
                "type".into(),
                "data".into(),
                "behavior".into(),
                "export".into(),
            ],
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

    /// One collapsible sidebar section, hand-rolled like the pivot's (a
    /// clickable header with an optional extra control over an optional
    /// body).
    #[allow(clippy::too_many_arguments)]
    fn section(
        sidebar: &Entity<ChartSidebar>,
        theme: &GridTheme,
        id: &'static str,
        title: &'static str,
        header_extra: Option<gpui::AnyElement>,
        expanded: bool,
        first: bool,
        animations: bool,
        content: gpui::AnyElement,
    ) -> gpui::AnyElement {
        let sidebar = sidebar.clone();
        let section_id: SharedString = id.into();
        let hover_bg = theme.menu_hover_bg;

        let mut header = div()
            .id(SharedString::from(format!("chart-section-{id}")))
            .flex()
            .items_center()
            .justify_between()
            .px(px(12.0))
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
                    .child(Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })),
            );
        if !first {
            header = header.border_t_1().border_color(theme.grid_line);
        }

        let mut wrapper = div().flex().flex_col().child(header);
        if expanded {
            let body = div()
                .px(px(12.0))
                .py(px(12.0))
                .bg(theme.menu_bg)
                .border_t_1()
                .border_color(theme.grid_line)
                .child(content);
            // Same reveal treatment as the pivot sections: the chevron flips
            // instantly, the body fades in (no height tween — that janks).
            wrapper = wrapper.child(crate::grid::motion::fade_in(
                body,
                SharedString::from(format!("chart-section-body-{id}")),
                crate::grid::motion::SECTION_ENTER_MS,
                animations,
            ));
        }
        wrapper.into_any_element()
    }

    /// A small group label ("Category", "Values", …) inside a section body.
    fn group_label(theme: &GridTheme, text: &'static str) -> gpui::AnyElement {
        div()
            .text_color(theme.muted_text)
            .text_size(px(GROUP_FONT_SIZE))
            .child(text)
            .into_any_element()
    }

    /// One selectable picker row: label on the left, a check mark when
    /// `selected`. `on_pick` runs on click.
    fn pick_row(
        theme: &GridTheme,
        id: SharedString,
        label: impl Into<SharedString>,
        selected: bool,
        on_pick: impl Fn(&mut ChartState, &mut Context<ChartState>) + 'static,
        state: &Entity<ChartState>,
    ) -> gpui::AnyElement {
        let state = state.clone();
        let hover_bg = theme.menu_hover_bg;
        let mut row = div()
            .id(id)
            .flex()
            .items_center()
            .justify_between()
            .h(px(ROW_HEIGHT))
            .px(px(6.0))
            .rounded(px(4.0))
            .text_size(px(ROW_FONT_SIZE))
            .text_color(if selected {
                theme.header_fg
            } else {
                theme.menu_fg
            })
            .cursor_pointer()
            .hover(move |style| style.bg(hover_bg))
            .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                state.update(cx, |s, cx| on_pick(s, cx));
            })
            .child(div().child(label.into()));
        if selected {
            row = row.child(
                div()
                    .text_color(theme.sort_indicator)
                    .child(Icon::new(IconName::Check)),
            );
        }
        row.into_any_element()
    }

    /// A plain action row (no selection state), e.g. "Copy SVG".
    fn action_row(
        theme: &GridTheme,
        id: &'static str,
        label: &'static str,
        on_click: impl Fn(&mut ChartState, &mut Context<ChartState>) + 'static,
        state: &Entity<ChartState>,
    ) -> gpui::AnyElement {
        let state = state.clone();
        let hover_bg = theme.menu_hover_bg;
        div()
            .id(id)
            .flex()
            .items_center()
            .h(px(ROW_HEIGHT))
            .px(px(6.0))
            .rounded(px(4.0))
            .text_size(px(ROW_FONT_SIZE))
            .text_color(theme.menu_fg)
            .cursor_pointer()
            .hover(move |style| style.bg(hover_bg))
            .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                state.update(cx, |s, cx| on_click(s, cx));
            })
            .child(div().child(label))
            .into_any_element()
    }

    /// Replace the config through the state's recompute path.
    fn configure(state: &mut ChartState, cx: &mut Context<ChartState>, config: ChartConfig) {
        state.set_config(config);
        cx.notify();
    }
}

impl Render for ChartSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (theme, columns, config, rows_len, save_handler, animations, numeric_columns) = {
            let s = self.state.read(cx);
            let numeric = (0..s.source_columns.len())
                .filter(|&i| is_numeric_column(i, &s.source_rows))
                .collect::<Vec<_>>();
            (
                s.theme.clone(),
                s.source_columns.clone(),
                s.config.clone(),
                s.source_rows.len(),
                s.save_config_handler.clone(),
                s.animations,
                numeric,
            )
        };
        let histogram = config.kind == ChartKind::Histogram;

        // -- Type section: one button per chart kind ----------------------
        let mut kind_buttons = Vec::new();
        for kind in ChartKind::all() {
            let state_kind = self.state.clone();
            let active = config.kind == *kind;
            let border = if active {
                theme.sort_indicator
            } else {
                theme.grid_line
            };
            let bg = if active {
                theme.filter_active_bg
            } else {
                theme.menu_bg
            };
            let label: SharedString = kind.label().into();
            kind_buttons.push(
                div()
                    .id(SharedString::from(format!("chart-kind-{}", kind.label())))
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(border)
                    .bg(bg)
                    .text_size(px(ROW_FONT_SIZE))
                    .text_color(if active {
                        theme.header_fg
                    } else {
                        theme.menu_fg
                    })
                    .cursor_pointer()
                    .child(label)
                    .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                        let kind = *kind;
                        state_kind.update(cx, |s, cx| {
                            s.set_kind(kind);
                            cx.notify();
                        });
                    }),
            );
        }

        // Save-configuration button in the Type section header — same shape
        // as the pivot sidebar's.
        let save_button = save_handler.map(|handler| {
            let state_save = self.state.clone();
            let icon_color = theme.muted_text;
            let hover_bg = theme.pivot_drop_zone_active_bg;
            div()
                .id("chart-save-config")
                .p(px(2.0))
                .rounded(px(3.0))
                .cursor_pointer()
                .hover(move |style| style.bg(hover_bg))
                .child(disk_icon(icon_color))
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

        // -- Data section -------------------------------------------------
        let mut data_children: Vec<gpui::AnyElement> = Vec::new();

        if !histogram {
            data_children.push(Self::group_label(&theme, "Category"));
            data_children.push(Self::pick_row(
                &theme,
                SharedString::from("chart-category-auto"),
                "Auto",
                config.label_column.is_none(),
                |s, cx| {
                    let mut config = s.config.clone();
                    config.label_column = None;
                    Self::configure(s, cx, config);
                },
                &self.state,
            ));
            for (index, column) in columns.iter().enumerate() {
                let selected = config.label_column == Some(index);
                let label: SharedString = column.name.clone().into();
                data_children.push(Self::pick_row(
                    &theme,
                    SharedString::from(format!("chart-category-{index}")),
                    label,
                    selected,
                    move |s, cx| {
                        let mut config = s.config.clone();
                        config.label_column = Some(index);
                        Self::configure(s, cx, config);
                    },
                    &self.state,
                ));
            }
        }

        data_children.push(Self::group_label(&theme, "Values"));
        if numeric_columns.is_empty() {
            data_children.push(
                div()
                    .text_size(px(ROW_FONT_SIZE))
                    .text_color(theme.muted_text)
                    .child("No numeric columns")
                    .into_any_element(),
            );
        } else {
            for &index in &numeric_columns {
                let checked = config.value_columns.contains(&index);
                let label: SharedString = columns[index].name.clone().into();
                data_children.push(Self::pick_row(
                    &theme,
                    SharedString::from(format!("chart-value-{index}")),
                    label,
                    checked,
                    move |s, cx| {
                        let mut config = s.config.clone();
                        if checked {
                            config.value_columns.retain(|&c| c != index);
                        } else {
                            config.value_columns.push(index);
                            config.value_columns.sort_unstable();
                        }
                        Self::configure(s, cx, config);
                    },
                    &self.state,
                ));
            }
        }

        if !histogram {
            data_children.push(Self::group_label(&theme, "Aggregate"));
            for aggregate in ChartAggregate::all() {
                let selected = config.aggregate == *aggregate;
                let label: SharedString = aggregate.label().into();
                data_children.push(Self::pick_row(
                    &theme,
                    SharedString::from(format!("chart-aggregate-{}", aggregate.label())),
                    label,
                    selected,
                    move |s, cx| {
                        let mut config = s.config.clone();
                        config.aggregate = *aggregate;
                        Self::configure(s, cx, config);
                    },
                    &self.state,
                ));
            }

            data_children.push(Self::group_label(&theme, "Order"));
            for order in ChartOrder::all() {
                let selected = config.order == *order;
                let label: SharedString = order.label().into();
                data_children.push(Self::pick_row(
                    &theme,
                    SharedString::from(format!("chart-order-{}", order.label())),
                    label,
                    selected,
                    move |s, cx| {
                        let mut config = s.config.clone();
                        config.order = *order;
                        Self::configure(s, cx, config);
                    },
                    &self.state,
                ));
            }

            data_children.push(Self::group_label(&theme, "Limit"));
            let top_label: SharedString = format!("Top {}", config.top_n).into();
            data_children.push(Self::pick_row(
                &theme,
                SharedString::from("chart-top-n"),
                top_label,
                false,
                |s, cx| {
                    let mut config = s.config.clone();
                    config.top_n = config.next_top_n();
                    Self::configure(s, cx, config);
                },
                &self.state,
            ));
        }

        // -- Behavior section ---------------------------------------------
        let state_navigate = self.state.clone();
        let behavior = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                Checkbox::new("chart-navigate-on-click")
                    .checked(config.navigate_on_click)
                    .label("Click marks to show source rows")
                    .on_click(move |checked: &bool, _window, app| {
                        state_navigate.update(app, |s, cx| {
                            let mut config = s.config.clone();
                            config.navigate_on_click = *checked;
                            Self::configure(s, cx, config);
                        });
                    }),
            )
            .child(
                div()
                    .text_size(px(GROUP_FONT_SIZE))
                    .text_color(theme.muted_text)
                    .child(
                        "Clicking a bar, point, or slice selects the result rows \
                         behind it on the Grid tab.",
                    ),
            );

        // -- Export section -----------------------------------------------
        let state_copy = self.state.clone();
        let copy_clipboard = theme.menu_bg;
        let _ = copy_clipboard;
        let state_save_svg = self.state.clone();
        let export = div().flex().flex_col().gap(px(4.0)).child(Self::action_row(
            &theme,
            "chart-copy-svg",
            "Copy SVG",
            move |_s, cx| {
                if let Some(svg) = state_copy.read(cx).svg() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(svg));
                }
            },
            &self.state,
        ));
        #[cfg(not(target_arch = "wasm32"))]
        let export = export.child(Self::action_row(
            &theme,
            "chart-save-svg",
            "Save SVG…",
            move |_s, cx| {
                if let Some(svg) = state_save_svg.read(cx).svg() {
                    let rx = cx.prompt_for_new_path(std::path::Path::new("."), Some("chart.svg"));
                    cx.spawn(async move |_this, _cx| {
                        if let Ok(Ok(Some(path))) = rx.await {
                            // Persisting to disk can fail (permissions, a
                            // mid-write cancel); the clipboard export above
                            // remains the always-available fallback.
                            let _ = std::fs::write(&path, svg.as_bytes());
                        }
                    })
                    .detach();
                }
            },
            &self.state,
        ));
        #[cfg(target_arch = "wasm32")]
        let export = export.child(
            div()
                .text_size(px(GROUP_FONT_SIZE))
                .text_color(theme.muted_text)
                .child("Save SVG is unavailable in the browser preview; use Copy SVG."),
        );

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
                "type",
                "Type",
                save_button,
                is_expanded("type"),
                true,
                animations,
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.0))
                    .children(kind_buttons)
                    .into_any_element(),
            ))
            .child(Self::section(
                &sidebar,
                &theme,
                "data",
                "Data",
                None,
                is_expanded("data"),
                false,
                animations,
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .children(data_children)
                    .into_any_element(),
            ))
            .child(Self::section(
                &sidebar,
                &theme,
                "behavior",
                "Behavior",
                None,
                is_expanded("behavior"),
                false,
                animations,
                behavior.into_any_element(),
            ))
            .child(Self::section(
                &sidebar,
                &theme,
                "export",
                "Export",
                None,
                is_expanded("export"),
                false,
                animations,
                export.into_any_element(),
            ));

        let _ = rows_len;
        div()
            .id("chart-sidebar")
            .h_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(8.0))
            .bg(theme.menu_bg)
            .text_color(theme.menu_fg)
            .text_size(px(ROW_FONT_SIZE))
            .overflow_y_scroll()
            .child(
                div()
                    .text_color(theme.header_fg)
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Chart Controls"),
            )
            .child(sections)
    }
}

/// A little floppy-disk glyph drawn with divs (no icon font dependency) —
/// same glyph as the pivot sidebar's save button.
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
