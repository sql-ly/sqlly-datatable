use std::collections::HashSet;

use sqlly_datatable::format::format_relative_date;
use sqlly_datatable::grid::menu::{label as menu_label, MenuAction};
use sqlly_datatable::localization::{
    clear_translator, english_default, format, install_translator, text, ENGLISH_DEFAULTS,
};
use sqlly_datatable::{
    AggregationFn, CellValue, ChartKind, Column, ColumnKind, GridData, GridDataError,
    RelativeDateFormat, RelativeUnit,
};

#[test]
fn injection_formatting_fallback_and_literal_coverage() {
    clear_translator();
    assert_eq!(menu_label(MenuAction::SelectColumn), "Select column");
    assert_eq!(ChartKind::Bar.label(), "Bar");
    assert_eq!(AggregationFn::Sum.caption("Amount"), "Sum of Amount");
    assert_eq!(
        format(
            "datatable.grid.search.position",
            &[("total", "42"), ("current", "3")],
        ),
        "3 of 42"
    );

    install_translator(|key: &str, _: &[(&str, &str)]| match key {
        "datatable.grid.context_menu.select_column" => Some("Choose this column".to_owned()),
        "datatable.chart.kind.bar" => Some("English bars".to_owned()),
        _ => None,
    });
    assert_eq!(menu_label(MenuAction::SelectColumn), "Choose this column");
    assert_eq!(ChartKind::Bar.label(), "English bars");
    assert_eq!(ChartKind::Line.label(), "Line");

    install_translator(|key: &str, args: &[(&str, &str)]| {
        let arg = |name| {
            args.iter()
                .find_map(|(candidate, value)| (*candidate == name).then_some(*value))
                .unwrap_or_default()
        };
        match key {
            "datatable.grid.context_menu.select_column" => Some("Seleccionar columna".to_owned()),
            "datatable.chart.kind.bar" => Some("Barras".to_owned()),
            "datatable.pivot.aggregation.sum_caption" => {
                Some(format!("Suma de {}", arg("field_name")))
            }
            "datatable.grid.data_error.ragged_row" => Some(format!(
                "la fila {} tiene {} celdas; se esperaban {}",
                arg("row_index"),
                arg("actual"),
                arg("expected")
            )),
            "datatable.format.relative.future" => Some(format!("dentro de {}", arg("duration"))),
            "datatable.format.relative.day_singular" => Some("día".to_owned()),
            "datatable.grid.search.position" => {
                Some(format!("{} de {}", arg("current"), arg("total")))
            }
            _ => None,
        }
    });

    assert_eq!(menu_label(MenuAction::SelectColumn), "Seleccionar columna");
    assert_eq!(ChartKind::Bar.label(), "Barras");
    assert_eq!(ChartKind::Line.label(), "Line");
    assert_eq!(AggregationFn::Sum.caption("Importe"), "Suma de Importe");
    assert_eq!(
        format(
            "datatable.grid.search.position",
            &[("total", "42"), ("current", "3")],
        ),
        "3 de 42"
    );
    assert_eq!(
        format("datatable.chart.sidebar.top_n", &[("count", "25")],),
        "Top 25"
    );

    let error = GridData::new(
        vec![Column::new("id", ColumnKind::Integer, 80.0)],
        vec![vec![CellValue::Integer(1), CellValue::Integer(2)]],
    );
    assert_eq!(
        error.err(),
        Some(GridDataError::RaggedRow {
            row_index: 0,
            expected: 1,
            actual: 2,
        })
    );
    assert_eq!(
        GridDataError::RaggedRow {
            row_index: 0,
            expected: 1,
            actual: 2,
        }
        .to_string(),
        "la fila 0 tiene 2 celdas; se esperaban 1"
    );

    let relative = RelativeDateFormat {
        units: vec![RelativeUnit::Day],
        max_components: 1,
    };
    assert_eq!(
        format_relative_date(86_400, 0, &relative),
        "dentro de 1 día"
    );

    install_translator(|key: &str, _: &[(&str, &str)]| Some(format!("es:{key}")));
    for (key, _) in ENGLISH_DEFAULTS {
        assert_eq!(text(key), format!("es:{key}"), "untranslated key: {key}");
    }

    clear_translator();
    assert_eq!(menu_label(MenuAction::SelectColumn), "Select column");

    let mut unique = HashSet::new();
    let implementation = [
        include_str!("../src/data.rs"),
        include_str!("../src/format.rs"),
        include_str!("../src/grid/menu.rs"),
        include_str!("../src/grid/paint.rs"),
        include_str!("../src/grid/search.rs"),
        include_str!("../src/grid/state.rs"),
        include_str!("../src/grid/widget.rs"),
        include_str!("../src/chart/config.rs"),
        include_str!("../src/chart/model.rs"),
        include_str!("../src/chart/sidebar.rs"),
        include_str!("../src/chart/state.rs"),
        include_str!("../src/chart/widget.rs"),
        include_str!("../src/pivot/aggregation.rs"),
        include_str!("../src/pivot/context_menu.rs"),
        include_str!("../src/pivot/engine.rs"),
        include_str!("../src/pivot/paint.rs"),
        include_str!("../src/pivot/sidebar.rs"),
        include_str!("../src/pivot/state.rs"),
    ]
    .join("\n");
    for (key, default) in ENGLISH_DEFAULTS {
        assert!(unique.insert(*key), "duplicate key: {key}");
        assert!(
            key.starts_with("datatable."),
            "missing crate namespace: {key}"
        );
        assert!(
            key.split('.').all(|segment| {
                !segment.is_empty()
                    && segment
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            }),
            "non-contextual snake_case key: {key}"
        );
        assert!(!default.is_empty(), "empty English default: {key}");
        assert_eq!(english_default(key), Some(*default));
        let dynamically_composed = key.starts_with("datatable.format.date.month_")
            || key.starts_with("datatable.format.date.day_")
            || key.starts_with("datatable.format.relative.year_")
            || key.starts_with("datatable.format.relative.month_")
            || key.starts_with("datatable.format.relative.week_")
            || key.starts_with("datatable.format.relative.day_")
            || key.starts_with("datatable.format.relative.hour_")
            || key.starts_with("datatable.format.relative.minute_")
            || key.starts_with("datatable.format.relative.second_");
        assert!(
            dynamically_composed || implementation.contains(key),
            "catalog key is not used by any visible-text implementation: {key}"
        );
    }
}
