use gpui::{
    div, prelude::*, px, size, App, Bounds, ClipboardItem, Context, Entity, Window, WindowBounds,
    WindowOptions,
};
use sqlly_datatable::{
    AggregationFn, Column, ColumnKind, ColumnOverride, ContextMenuItem, ContextMenuProvider,
    ContextMenuRequest, GridConfig, GridData, GridState, GridThemePair, NumberFormat, PivotConfig,
    PivotContextMenuProvider, PivotContextMenuRequest, PivotMenuItem, PivotState, SqllyDataTable,
};

fn main() {
    let data = sample_data();
    let config = sample_config(&data);

    let application = gpui::Application::new();
    application.run(move |cx: &mut App| {
        cx.activate(true);

        let view = SqllyDataTable::builder(data)
            .config(config)
            .theme_family(ThemeChoice::Signature.pair())
            .context_menu_provider(SampleMenuProvider)
            .pivot(sample_pivot_config())
            .pivot_context_menu_provider(SamplePivotMenuProvider)
            .pivot_save_config(|config, _cx| {
                println!("save pivot config: {config:?}");
            })
            .build(cx);
        let focus = view.state.read(cx).focus_handle.clone();

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1200.0), px(700.0)),
                cx,
            ))),
            titlebar: Some(Default::default()),
            is_movable: true,
            is_resizable: true,
            window_min_size: Some(size(px(600.0), px(400.0))),
            ..Default::default()
        };

        // Move the built widget (grid + pivot tab) into the window intact —
        // re-wrapping only the GridState via `SqllyDataTable::new` would
        // drop the pivot parts created by the builder.
        match cx.open_window(options, move |_window, cx| {
            let table = cx.new(|_cx| view);
            cx.new(|_cx| RootView {
                table,
                theme_choice: ThemeChoice::Signature,
            })
        }) {
            Ok(window) => {
                let _ = window.update(cx, |_view, window, _cx| {
                    window.focus(&focus);
                    window.on_window_should_close(_cx, |_window, cx| {
                        cx.quit();
                        true
                    });
                });
            }
            Err(err) => {
                eprintln!("failed to open window: {err}");
                cx.quit();
            }
        }
    });
}

/// The two theme families shipped with the crate, as offered by the sample
/// app's switcher.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ThemeChoice {
    Neutral,
    Signature,
}

impl ThemeChoice {
    fn pair(self) -> GridThemePair {
        match self {
            Self::Neutral => GridThemePair::neutral(),
            Self::Signature => GridThemePair::signature(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Neutral => "Neutral",
            Self::Signature => "Signature",
        }
    }
}

/// Root view: a themed toolbar (title + theme-family switcher) above the
/// grid. The toolbar paints from the grid's current `GridTheme`, so it
/// follows both the family switcher and the OS light/dark appearance.
struct RootView {
    table: Entity<SqllyDataTable>,
    theme_choice: ThemeChoice,
}

impl RootView {
    fn segment(
        &self,
        choice: ThemeChoice,
        theme: &sqlly_datatable::GridTheme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.theme_choice == choice;
        let hover_bg = theme.menu_hover_bg;
        let hover_fg = theme.header_fg;
        div()
            .id(choice.label())
            .px(px(12.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .text_size(px(12.0))
            .when(active, |seg| {
                seg.bg(theme.selection_bg).text_color(theme.header_fg)
            })
            .when(!active, |seg| {
                seg.text_color(theme.muted_text)
                    .hover(move |seg| seg.bg(hover_bg).text_color(hover_fg))
            })
            .on_click(cx.listener(move |this, _event, window, cx| {
                if this.theme_choice == choice {
                    return;
                }
                this.theme_choice = choice;
                this.table.update(cx, |table, cx| {
                    table.set_theme_family(choice.pair(), window, cx);
                });
                cx.notify();
            }))
            .child(choice.label())
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.table.read(cx).state.read(cx).theme.clone();
        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(44.0))
            .px(px(12.0))
            .bg(theme.header_bg)
            .border_b_1()
            .border_color(theme.grid_line)
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.header_fg)
                    .child("sqlly-datatable"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.muted_text)
                            .child("Theme"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.0))
                            .p(px(2.0))
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(theme.grid_line)
                            .bg(theme.bg)
                            .child(self.segment(ThemeChoice::Neutral, &theme, cx))
                            .child(self.segment(ThemeChoice::Signature, &theme, cx)),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg)
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(self.table.clone()))
    }
}

/// Preconfigured pivot layout demonstrating programmatic setup: narrative
/// down the side, currency across the top, summed amounts at the
/// intersections, and `trans_part` available as a source filter. The user
/// can rearrange everything from the sidebar at runtime.
fn sample_pivot_config() -> PivotConfig {
    PivotConfig {
        row_fields: vec![2],    // narrative
        column_fields: vec![1], // currency_id
        value_field: Some(0),   // amount
        aggregation: AggregationFn::Sum,
        filter_fields: vec![3], // trans_part
        ..PivotConfig::default()
    }
}

fn sample_config(data: &GridData) -> GridConfig {
    let mut config = GridConfig::default();
    let mut overrides = vec![ColumnOverride::default(); data.columns.len()];

    for (i, col) in data.columns.iter().enumerate() {
        match col.kind {
            ColumnKind::Integer => {
                overrides[i] = ColumnOverride {
                    number: Some(NumberFormat {
                        decimals: 0,
                        ..NumberFormat::default()
                    }),
                    ..Default::default()
                };
            }
            ColumnKind::Decimal => {
                overrides[i] = ColumnOverride {
                    number: Some(NumberFormat {
                        decimals: 4,
                        ..NumberFormat::default()
                    }),
                    ..Default::default()
                };
            }
            _ => {}
        }
    }
    config.column_overrides = overrides;
    config
}

fn sample_data() -> GridData {
    use sqlly_datatable::CellValue::*;

    // 40 columns. The first four keep their original names because the
    // sample context-menu provider looks them up by name (`amount`, `narrative`).
    // The remaining 36 cycle through the supported kinds to exercise every
    // default formatter.
    let mut columns = Vec::with_capacity(40);
    columns.push(Column::new("amount", ColumnKind::Decimal, 140.0));
    columns.push(Column::new("currency_id", ColumnKind::Integer, 130.0));
    columns.push(Column::new("narrative", ColumnKind::Text, 260.0));
    columns.push(Column::new("trans_part", ColumnKind::Boolean, 120.0));

    let extra_kinds = [
        ColumnKind::Text,
        ColumnKind::Integer,
        ColumnKind::Decimal,
        ColumnKind::Boolean,
        ColumnKind::Date,
    ];
    for i in 4..40 {
        let kind = extra_kinds[i % extra_kinds.len()];
        let width = match kind {
            ColumnKind::Text => 200.0,
            ColumnKind::Integer => 120.0,
            ColumnKind::Decimal => 140.0,
            ColumnKind::Boolean => 120.0,
            ColumnKind::Date => 150.0,
            ColumnKind::None => 120.0,
        };
        columns.push(Column::new(format!("field_{i:02}"), kind, width));
    }

    // Deterministic pseudo-random generator — enough variety across 100k rows
    // without pulling in the `rand` crate.
    let mut rng = Lcg::new(0x0123_4567_89AB_CDEF);
    let narratives = [
        "saldo de apertura",
        "cargo",
        "abono",
        "transferencia",
        "comisión",
        "interés",
    ];

    let mut rows = Vec::with_capacity(100_000);
    for r in 0..100_000 {
        let mut row = Vec::with_capacity(40);
        // Skewed positive with ~25% negatives so the pivot's red negative
        // styling shows up in totals and cells.
        row.push(Decimal((rng.next_f64() - 0.25) * 20_000.0));
        row.push(Integer((r % 5) as i64 + 1));
        row.push(Text(narratives[r % narratives.len()].into()));
        row.push(Boolean(r % 2 == 0));
        for (i, col) in columns.iter().enumerate().skip(4) {
            let cell = match col.kind {
                ColumnKind::Text => Text(format!("row {r} field {i:02}")),
                ColumnKind::Integer => Integer((r as i64).wrapping_mul((i as i64) + 7)),
                ColumnKind::Decimal => Decimal(rng.next_f64() * 1_000.0),
                ColumnKind::Boolean => Boolean((r + i) % 3 == 0),
                ColumnKind::Date => Date(1_700_000_000 + ((r as i64) + (i as i64)) * 86400),
                ColumnKind::None => None,
            };
            row.push(cell);
        }
        rows.push(row);
    }

    GridData::new(columns, rows).expect("rectangular sample data")
    // (allowed in a sample binary, not in the library)
}

/// Minimal linear-congruential generator — deterministic demo data without a
/// dependency. Uses the Numerical Recipes 64-bit constants.
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }

    fn next_f64(&mut self) -> f64 {
        // Map the high 53 bits of the state into [0, 1).
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Sample context-menu provider that demonstrates cell/row right-click menus.
struct SampleMenuProvider;

impl ContextMenuProvider for SampleMenuProvider {
    fn menu_items(&self, request: &ContextMenuRequest) -> Vec<ContextMenuItem> {
        let mut items = Vec::new();

        if let Some(row) = request.clicked_row() {
            if let Some(narrative) = row.value_by_name("narrative") {
                items.push(ContextMenuItem::action(
                    "copy-narrative",
                    format!("Copy narrative: {narrative:?}"),
                ));
            }
            if let Some(amount) = row.value_by_name("amount") {
                items.push(ContextMenuItem::action(
                    "copy-amount",
                    format!("Copy amount: {amount:?}"),
                ));
            }
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::action(
                "copy-row",
                format!("Copy full row ({} cells)", row.values.len()),
            ));
            items.push(ContextMenuItem::action(
                "copy-row-csv",
                format!("Copy row as CSV ({} fields)", row.columns.len()),
            ));
            items.push(ContextMenuItem::action("copy-row-json", "Copy row as JSON"));
            items.push(ContextMenuItem::action(
                "row-cell-count",
                format!(
                    "Row {} has {} cells",
                    row.display_row_index,
                    row.values.len()
                ),
            ));
            items.push(ContextMenuItem::action(
                "row-source-index",
                format!("Source row index: {}", row.source_row_index),
            ));
            // --- 10 additional row items ---
            items.push(ContextMenuItem::action("copy-row-tsv", "Copy row as TSV"));
            items.push(ContextMenuItem::action(
                "copy-row-keys",
                format!("Copy column names ({})", row.columns.len()),
            ));
            items.push(ContextMenuItem::action(
                "copy-row-values",
                "Copy values only",
            ));
            items.push(ContextMenuItem::action(
                "copy-row-markdown",
                "Copy row as Markdown table",
            ));
            items.push(ContextMenuItem::action(
                "copy-row-sql",
                "Copy row as SQL INSERT",
            ));
            items.push(ContextMenuItem::action(
                "row-non-empty-count",
                format!(
                    "Non-empty cells: {}",
                    row.values
                        .iter()
                        .filter(|v| !matches!(v, sqlly_datatable::CellValue::None))
                        .count()
                ),
            ));
            items.push(ContextMenuItem::action(
                "row-numeric-sum",
                "Sum numeric cells",
            ));
            items.push(ContextMenuItem::action(
                "copy-row-pipe",
                "Copy row (pipe-delimited)",
            ));
            items.push(ContextMenuItem::action(
                "copy-row-display-index",
                format!("Copy display index: {}", row.display_row_index),
            ));
            items.push(ContextMenuItem::action(
                "row-column-kinds",
                "Copy column kinds",
            ));
        }

        if let Some(cell) = request.clicked_cell() {
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::action(
                "copy-cell",
                format!("Copy cell ({}): {:?}", cell.column_name, cell.value),
            ));
            items.push(ContextMenuItem::action(
                "cell-location",
                format!(
                    "Cell at col {} ({}) row {}",
                    cell.column_index, cell.column_name, cell.display_row_index
                ),
            ));
            // --- 10 additional cell items ---
            items.push(ContextMenuItem::action(
                "copy-cell-name-value",
                format!("Copy \"{}={:?}\"", cell.column_name, cell.value),
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-column-name",
                format!("Copy column name: {}", cell.column_name),
            ));
            items.push(ContextMenuItem::action(
                "cell-value-kind",
                format!("Value kind: {}", cell_value_kind(&cell.value)),
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-source-index",
                format!("Copy source row: {}", cell.source_row_index),
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-coord",
                format!(
                    "Copy coord (r{}, c{})",
                    cell.display_row_index, cell.column_index
                ),
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-json",
                "Copy cell as JSON",
            ));
            items.push(ContextMenuItem::action(
                "cell-is-empty",
                format!(
                    "Cell empty? {}",
                    matches!(cell.value, sqlly_datatable::CellValue::None)
                ),
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-upper",
                "Copy value (UPPERCASE)",
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-lower",
                "Copy value (lowercase)",
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-len",
                "Copy value length",
            ));
        }

        if request.selected_cell_count() > 0 {
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::action(
                "copy-selection",
                format!("Copy {} selected cell(s)", request.selected_cell_count()),
            ));
            let rows = request.selected_row_count();
            let cells = request.selected_cell_count();
            // `selected_row_count` is 0 for column-oriented selections, so
            // the column count is only derivable when rows are known.
            let summary = if rows > 0 {
                format!("Selection spans {} row(s) × {} col(s)", rows, cells / rows)
            } else {
                format!("Selection spans {cells} cell(s) in whole column(s)")
            };
            items.push(ContextMenuItem::action("selection-summary", summary));
            // --- 10 additional selection items ---
            items.push(ContextMenuItem::action(
                "copy-selection-values",
                "Copy selection values only",
            ));
            items.push(ContextMenuItem::action(
                "copy-selection-tsv",
                "Copy selection as TSV",
            ));
            items.push(ContextMenuItem::action(
                "copy-selection-json",
                "Copy selection as JSON",
            ));
            items.push(ContextMenuItem::action(
                "copy-selection-csv",
                "Copy selection as CSV",
            ));
            items.push(ContextMenuItem::action(
                "selection-numeric-sum",
                "Sum numeric selection",
            ));
            items.push(ContextMenuItem::action(
                "selection-numeric-avg",
                "Average numeric selection",
            ));
            items.push(ContextMenuItem::action(
                "selection-cell-count",
                format!("Selected cells: {}", request.selected_cell_count()),
            ));
            items.push(ContextMenuItem::action(
                "selection-row-count",
                format!("Selected rows: {}", request.selected_row_count()),
            ));
            items.push(ContextMenuItem::action(
                "selection-distinct-columns",
                "Copy distinct column names",
            ));
            items.push(ContextMenuItem::action(
                "selection-empty-count",
                "Count empty cells in selection",
            ));
        }

        // Compose built-in column-header actions when right-clicking headers.
        if matches!(
            request.target,
            sqlly_datatable::ContextMenuTarget::ColumnHeader { .. }
                | sqlly_datatable::ContextMenuTarget::SortButton { .. }
        ) {
            items.push(ContextMenuItem::separator());
            items.extend(ContextMenuItem::standard_column_header_items());
        }

        if items.is_empty() {
            items.push(ContextMenuItem::action("noop", "No action for this target"));
        }

        items
    }

    fn on_action(
        &self,
        action_id: &str,
        request: &ContextMenuRequest,
        state: &mut GridState,
        cx: &mut App,
    ) {
        // Heavy export runs off the main thread so the UI stays responsive and
        // a loading indicator is shown while the CSV is assembled.
        if action_id == "copy-selection-csv" {
            let req = request.clone();
            state.spawn_background(
                cx,
                "Exporting selection to CSV…",
                move || {
                    use std::fmt::Write as _;
                    let mut out = String::new();
                    req.for_each_selected_cell(|c| {
                        let _ = writeln!(out, "{},{:?}", c.column_name, c.value);
                    });
                    out
                },
                |csv, _s, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(csv));
                },
            );
            return;
        }

        let text = match action_id {
            "copy-narrative" => request
                .clicked_row()
                .and_then(|r| r.value_by_name("narrative").map(|v| format!("{v:?}")))
                .unwrap_or_default(),
            "copy-amount" => request
                .clicked_row()
                .and_then(|r| r.value_by_name("amount").map(|v| format!("{v:?}")))
                .unwrap_or_default(),
            "copy-row" => request
                .clicked_row()
                .map(|r| {
                    r.named_values()
                        .map(|(name, val)| format!("{name}={val:?}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
            "copy-row-csv" => request
                .clicked_row()
                .map(|r| {
                    let header = r
                        .columns
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(",");
                    let values = r
                        .values
                        .iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("{header}\n{values}")
                })
                .unwrap_or_default(),
            "copy-row-json" => request
                .clicked_row()
                .map(|r| {
                    let pairs = r
                        .named_values()
                        .map(|(name, val)| format!("\"{name}\": {val:?}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{{ {pairs} }}")
                })
                .unwrap_or_default(),
            "row-cell-count" | "row-source-index" => {
                // Diagnostic items: reflect the clicked row's metadata.
                request
                    .clicked_row()
                    .map(|r| {
                        format!(
                            "row={} source={} cells={}",
                            r.display_row_index,
                            r.source_row_index,
                            r.values.len()
                        )
                    })
                    .unwrap_or_default()
            }
            "copy-cell" => request
                .clicked_cell()
                .map(|c| format!("{:?}", c.value))
                .unwrap_or_default(),
            "cell-location" => request
                .clicked_cell()
                .map(|c| {
                    format!(
                        "col={} ({}) row={}",
                        c.column_index, c.column_name, c.display_row_index
                    )
                })
                .unwrap_or_default(),
            "copy-selection" => request
                .selected_cells()
                .iter()
                .map(|c| format!("{}={:?}", c.column_name, c.value))
                .collect::<Vec<_>>()
                .join("\n"),
            "selection-summary" => format!(
                "{} rows × {} cells",
                request.selected_rows().len(),
                request.selected_cells().len()
            ),
            // --- additional row actions ---
            "copy-row-tsv" => request
                .clicked_row()
                .map(|r| {
                    let header = r
                        .columns
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join("\t");
                    let values = r
                        .values
                        .iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join("\t");
                    format!("{header}\n{values}")
                })
                .unwrap_or_default(),
            "copy-row-keys" => request
                .clicked_row()
                .map(|r| {
                    r.columns
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default(),
            "copy-row-values" => request
                .clicked_row()
                .map(|r| {
                    r.values
                        .iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
            "copy-row-markdown" => request
                .clicked_row()
                .map(|r| {
                    let header = r
                        .columns
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" | ");
                    let sep = r
                        .columns
                        .iter()
                        .map(|_| "---")
                        .collect::<Vec<_>>()
                        .join(" | ");
                    let values = r
                        .values
                        .iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join(" | ");
                    format!("| {header} |\n| {sep} |\n| {values} |")
                })
                .unwrap_or_default(),
            "copy-row-sql" => request
                .clicked_row()
                .map(|r| {
                    let cols = r
                        .columns
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let vals = r
                        .values
                        .iter()
                        .map(sql_literal)
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("INSERT INTO sample_data ({cols}) VALUES ({vals});")
                })
                .unwrap_or_default(),
            "row-non-empty-count" => request
                .clicked_row()
                .map(|r| {
                    r.values
                        .iter()
                        .filter(|v| !matches!(v, sqlly_datatable::CellValue::None))
                        .count()
                        .to_string()
                })
                .unwrap_or_default(),
            "row-numeric-sum" => request
                .clicked_row()
                .map(|r| sum_numeric(r.values.iter()).to_string())
                .unwrap_or_default(),
            "copy-row-pipe" => request
                .clicked_row()
                .map(|r| {
                    r.values
                        .iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .unwrap_or_default(),
            "copy-row-display-index" => request
                .clicked_row()
                .map(|r| r.display_row_index.to_string())
                .unwrap_or_default(),
            "row-column-kinds" => request
                .clicked_row()
                .map(|r| {
                    r.columns
                        .iter()
                        .map(|c| format!("{}: {:?}", c.name, c.kind))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
            // --- additional cell actions ---
            "copy-cell-name-value" => request
                .clicked_cell()
                .map(|c| format!("{}={:?}", c.column_name, c.value))
                .unwrap_or_default(),
            "copy-cell-column-name" => request
                .clicked_cell()
                .map(|c| c.column_name.clone())
                .unwrap_or_default(),
            "cell-value-kind" => request
                .clicked_cell()
                .map(|c| cell_value_kind(&c.value).to_string())
                .unwrap_or_default(),
            "copy-cell-source-index" => request
                .clicked_cell()
                .map(|c| c.source_row_index.to_string())
                .unwrap_or_default(),
            "copy-cell-coord" => request
                .clicked_cell()
                .map(|c| format!("(r{}, c{})", c.display_row_index, c.column_index))
                .unwrap_or_default(),
            "copy-cell-json" => request
                .clicked_cell()
                .map(|c| format!("{{ \"{}\": {:?} }}", c.column_name, c.value))
                .unwrap_or_default(),
            "cell-is-empty" => request
                .clicked_cell()
                .map(|c| matches!(c.value, sqlly_datatable::CellValue::None).to_string())
                .unwrap_or_default(),
            "copy-cell-upper" => request
                .clicked_cell()
                .map(|c| cell_display(&c.value).to_uppercase())
                .unwrap_or_default(),
            "copy-cell-lower" => request
                .clicked_cell()
                .map(|c| cell_display(&c.value).to_lowercase())
                .unwrap_or_default(),
            "copy-cell-len" => request
                .clicked_cell()
                .map(|c| cell_display(&c.value).chars().count().to_string())
                .unwrap_or_default(),
            // --- additional selection actions ---
            "copy-selection-values" => request
                .selected_cells()
                .iter()
                .map(|c| format!("{:?}", c.value))
                .collect::<Vec<_>>()
                .join("\n"),
            "copy-selection-tsv" => request
                .selected_cells()
                .iter()
                .map(|c| format!("{}\t{:?}", c.column_name, c.value))
                .collect::<Vec<_>>()
                .join("\n"),
            "copy-selection-json" => {
                let pairs = request
                    .selected_cells()
                    .iter()
                    .map(|c| format!("{{ \"{}\": {:?} }}", c.column_name, c.value))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[ {pairs} ]")
            }
            "selection-numeric-sum" => {
                sum_numeric(request.selected_cells().iter().map(|c| &c.value)).to_string()
            }
            "selection-numeric-avg" => {
                let nums: Vec<f64> = request
                    .selected_cells()
                    .iter()
                    .filter_map(|c| numeric_value(&c.value))
                    .collect();
                if nums.is_empty() {
                    "0".to_string()
                } else {
                    (nums.iter().sum::<f64>() / nums.len() as f64).to_string()
                }
            }
            "selection-cell-count" => request.selected_cells().len().to_string(),
            "selection-row-count" => request.selected_rows().len().to_string(),
            "selection-distinct-columns" => {
                let cells = request.selected_cells();
                let mut names: Vec<&str> = cells.iter().map(|c| c.column_name.as_str()).collect();
                names.sort_unstable();
                names.dedup();
                names.join(", ")
            }
            "selection-empty-count" => request
                .selected_cells()
                .iter()
                .filter(|c| matches!(c.value, sqlly_datatable::CellValue::None))
                .count()
                .to_string(),
            _ => return,
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

/// Sample pivot right-click provider: composes the built-in items (drill
/// through, copy value, copy CSV) with custom actions that demonstrate the
/// context passed up — the clicked cell's grouping path, aggregation, and
/// the source rows driving it.
struct SamplePivotMenuProvider;

/// `"region=Europe, product=Widget"`-style description of a clicked cell.
fn pivot_cell_path(request: &PivotContextMenuRequest) -> Option<String> {
    let cell = request.clicked_cell()?;
    let parts: Vec<String> = cell
        .row_path
        .iter()
        .chain(&cell.col_path)
        .map(|c| format!("{}={}", c.field_name, c.label))
        .collect();
    Some(if parts.is_empty() {
        "grand total".to_owned()
    } else {
        parts.join(", ")
    })
}

impl PivotContextMenuProvider for SamplePivotMenuProvider {
    fn menu_items(&self, request: &PivotContextMenuRequest) -> Vec<PivotMenuItem> {
        let mut items = PivotMenuItem::standard_items(&request.target);
        items.push(PivotMenuItem::separator());
        if let Some(cell) = request.clicked_cell() {
            items.push(PivotMenuItem::action(
                "copy-cell-path",
                format!(
                    "Copy cell path ({})",
                    pivot_cell_path(request).unwrap_or_default()
                ),
            ));
            items.push(PivotMenuItem::action(
                "copy-cell-summary",
                format!(
                    "{}: {}",
                    request.value_caption,
                    if cell.formatted_value.is_empty() {
                        "(empty)"
                    } else {
                        &cell.formatted_value
                    }
                ),
            ));
        }
        items.push(PivotMenuItem::action(
            "copy-driving-count",
            format!("Driving source rows: {}", request.source_row_count()),
        ));
        items
    }

    fn on_action(
        &self,
        action_id: &str,
        request: &PivotContextMenuRequest,
        _state: &mut PivotState,
        cx: &mut App,
    ) {
        let text = match action_id {
            "copy-cell-path" => pivot_cell_path(request).unwrap_or_default(),
            "copy-cell-summary" => request
                .clicked_cell()
                .map(|c| format!("{} = {}", request.value_caption, c.formatted_value))
                .unwrap_or_default(),
            "copy-driving-count" => request.source_row_count().to_string(),
            _ => return,
        };
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
}

/// Short human-readable name for a cell value's variant.
fn cell_value_kind(value: &sqlly_datatable::CellValue) -> &'static str {
    use sqlly_datatable::CellValue;
    match value {
        CellValue::Text(_) => "Text",
        CellValue::Integer(_) => "Integer",
        CellValue::Decimal(_) => "Decimal",
        CellValue::Date(_) => "Date",
        CellValue::Boolean(_) => "Boolean",
        CellValue::None => "None",
    }
}

/// Plain string rendering of a cell value (no `Debug` quoting).
fn cell_display(value: &sqlly_datatable::CellValue) -> String {
    use sqlly_datatable::CellValue;
    match value {
        CellValue::Text(s) => s.clone(),
        CellValue::Integer(i) => i.to_string(),
        CellValue::Decimal(d) => d.to_string(),
        CellValue::Date(d) => d.to_string(),
        CellValue::Boolean(b) => b.to_string(),
        CellValue::None => String::new(),
    }
}

/// SQL literal rendering: numbers bare, everything else single-quoted, `None` -> NULL.
fn sql_literal(value: &sqlly_datatable::CellValue) -> String {
    use sqlly_datatable::CellValue;
    match value {
        CellValue::Integer(i) => i.to_string(),
        CellValue::Decimal(d) => d.to_string(),
        CellValue::Boolean(b) => b.to_string(),
        CellValue::None => "NULL".to_string(),
        CellValue::Date(d) => d.to_string(),
        CellValue::Text(s) => format!("'{}'", s.replace('\'', "''")),
    }
}

/// Numeric projection of a cell value; `None` for non-numeric variants.
fn numeric_value(value: &sqlly_datatable::CellValue) -> Option<f64> {
    use sqlly_datatable::CellValue;
    match value {
        CellValue::Integer(i) => Some(*i as f64),
        CellValue::Decimal(d) => Some(*d),
        _ => None,
    }
}

/// Sum the numeric cells in an iterator of values.
fn sum_numeric<'a>(values: impl Iterator<Item = &'a sqlly_datatable::CellValue>) -> f64 {
    values.filter_map(numeric_value).sum()
}
