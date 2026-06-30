use gpui::{prelude::*, px, size, App, Bounds, ClipboardItem, WindowBounds, WindowOptions};
use sqlly_datatable::{
    Column, ColumnKind, ColumnOverride, ContextMenuItem, ContextMenuProvider, ContextMenuRequest,
    GridConfig, GridData, GridState, NumberFormat, SqllyDataTable,
};

fn main() {
    let data = sample_data();
    let config = sample_config(&data);

    let application = gpui::Application::new();
    application.run(move |cx: &mut App| {
        cx.activate(true);

        let view = SqllyDataTable::builder(data)
            .config(config)
            .context_menu_provider(SampleMenuProvider)
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

        let state = view.state.clone();
        match cx.open_window(options, move |_window, cx| {
            cx.new(|_cx| SqllyDataTable::new(state.clone()))
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
    let columns = vec![
        Column::new("amount", ColumnKind::Decimal, 200.0),
        Column::new("currency_id", ColumnKind::Integer, 110.0),
        Column::new("narrative", ColumnKind::Text, 270.0),
        Column::new("trans_part", ColumnKind::Boolean, 110.0),
    ];

    let rows = vec![
        vec![
            Decimal(17968.20),
            Integer(1),
            Text("saldo de apertura".into()),
            Boolean(false),
        ],
        vec![
            Decimal(717.84),
            Integer(1),
            Text("saldo de apertura".into()),
            Boolean(false),
        ],
        vec![
            Decimal(768.41),
            Integer(1),
            Text("saldo de apertura".into()),
            Boolean(false),
        ],
        vec![
            Decimal(1141.10),
            Integer(1),
            Text("cargo".into()),
            Boolean(true),
        ],
        vec![
            Decimal(1937.50),
            Integer(1),
            Text("cargo".into()),
            Boolean(true),
        ],
        vec![
            Decimal(1018.81),
            Integer(1),
            Text("cargo".into()),
            Boolean(true),
        ],
        vec![
            Decimal(3172.81),
            Integer(1),
            Text("abono".into()),
            Boolean(false),
        ],
        vec![
            Decimal(1640.00),
            Integer(2),
            Text("abono".into()),
            Boolean(false),
        ],
    ];

    GridData::new(columns, rows).expect("rectangular sample data")
    // (allowed in a sample binary, not in the library)
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
        }

        if !request.selected_cells().is_empty() {
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::action(
                "copy-selection",
                format!("Copy {} selected cell(s)", request.selected_cells().len()),
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
        _state: &mut GridState,
        cx: &mut App,
    ) {
        let text = match action_id {
            "copy-narrative" => request
                .clicked_row()
                .and_then(|r| r.value_by_name("narrative"))
                .map(|v| format!("{v:?}"))
                .unwrap_or_default(),
            "copy-amount" => request
                .clicked_row()
                .and_then(|r| r.value_by_name("amount"))
                .map(|v| format!("{v:?}"))
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
            "copy-selection" => request
                .selected_cells()
                .iter()
                .map(|c| format!("{}={:?}", c.column_name, c.value))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => return,
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}
