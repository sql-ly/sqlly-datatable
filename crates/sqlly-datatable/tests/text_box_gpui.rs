//! Integration-style smoke test. The integration binary name
//! (`text_box_gpui`) anchors the CI pipeline step `cargo test --test
//! text_box_gpui` to a real test target instead of relying solely on the
//! library unit tests. The contents are intentionally cheap — they verify the
//! public API compiles and basic invariants hold without ever opening a
//! window, so the suite stays portable across macOS CI hosts.

use sqlly_datatable::{
    CellValue, Column, ColumnKind, GridConfig, GridData, NumberFormat, ResolvedColumnFormat,
    StringFormat, TextAlignment, compare_cells,
};

#[test]
fn public_api_links_and_compiles() {
    let _data = GridData::new(
        vec![
            Column::new("a", ColumnKind::Integer, 80.0),
            Column::new("b", ColumnKind::Text, 120.0),
        ],
        vec![
            vec![CellValue::Integer(1), CellValue::Text("hello".into())],
            vec![CellValue::Integer(2), CellValue::Text("world".into())],
        ],
    )
    .expect("rectangular");
    let _config = GridConfig::default();
}

#[test]
fn compare_cells_is_deterministic_for_sample_data() {
    use std::cmp::Ordering;
    assert_eq!(
        compare_cells(&CellValue::Integer(1), &CellValue::Integer(2)),
        Ordering::Less
    );
    assert_eq!(
        compare_cells(&CellValue::None, &CellValue::Integer(0)),
        Ordering::Less
    );
    assert_eq!(
        compare_cells(&CellValue::None, &CellValue::None),
        Ordering::Equal
    );
}

#[test]
fn default_format_options_yield_safe_resolved_format() {
    let rcf = ResolvedColumnFormat {
        kind: ColumnKind::Decimal,
        number: NumberFormat::default(),
        date: sqlly_datatable::DateFormat::default(),
        boolean: sqlly_datatable::BooleanFormat::default(),
        string: StringFormat::default(),
        replacements: vec![],
        replacement_timing: sqlly_datatable::ReplacementTiming::default(),
    };
    assert_eq!(rcf.alignment(), TextAlignment::Right);
}
