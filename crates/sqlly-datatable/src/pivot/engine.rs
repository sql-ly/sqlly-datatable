//! Pure pivot computation: source rows in, [`PivotResult`] out.
//!
//! No GPUI types and **no mutation of the source data** — the engine borrows
//! the rows, buckets them into hierarchical row/column group trees, and
//! streams every value through [`Accumulator`]s at every rollup level:
//! leaf × leaf intersections, group subtotals, axis totals, and the grand
//! total. Aggregating once per level (instead of aggregating aggregates)
//! keeps `Avg`/`Count` correct at every level.

use crate::config::ResolvedColumnFormat;
use crate::data::{compare_cells, CellValue, Column};
use crate::format::format_cell;
use crate::pivot::aggregation::Accumulator;
use crate::pivot::config::PivotConfig;
use std::collections::HashMap;

/// Sentinel node key meaning "the total across this whole axis". Used as a
/// key into [`PivotResult::values`] alongside real node ids.
pub const TOTAL_KEY: usize = usize::MAX;

/// One group node on the row or column axis.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotNode {
    /// Formatted grouping value (or the configured blank label).
    pub label: String,
    /// The raw grouping value, kept for ordering with
    /// [`crate::data::compare_cells`].
    pub sort_key: CellValue,
    /// 0 = outermost field on this axis.
    pub depth: usize,
    /// Parent node id; `None` for roots.
    pub parent: Option<usize>,
    /// Child node ids (empty at the innermost depth).
    pub children: Vec<usize>,
    /// This node's subtotal across the entire opposite axis.
    pub total: CellValue,
}

impl PivotNode {
    /// `true` when this node is at the innermost depth of its axis.
    #[must_use]
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

/// The complete computed pivot. Cheap to share behind an `Arc`; the paint
/// path never touches the source rows again.
#[derive(Clone, Debug, Default)]
pub struct PivotResult {
    /// Arena of row-axis nodes; tree edges via `parent`/`children`.
    pub row_nodes: Vec<PivotNode>,
    /// Arena of column-axis nodes.
    pub col_nodes: Vec<PivotNode>,
    /// Row-axis roots (depth 0), in canonical ascending order.
    pub row_roots: Vec<usize>,
    /// Column-axis roots (depth 0), in canonical ascending order.
    pub col_roots: Vec<usize>,
    /// Aggregated value for every `(row key, col key)` pair where a key is a
    /// node id or [`TOTAL_KEY`]. Contains every rollup level, so collapsed
    /// groups and subtotal rows/columns read straight from this map.
    pub values: HashMap<(usize, usize), CellValue>,
    /// The value across all rows and columns.
    pub grand_total: CellValue,
    /// Number of row fields (row tree depth).
    pub row_depth: usize,
    /// Number of column fields (column tree depth).
    pub col_depth: usize,
    /// Display names of the row fields, outermost first.
    pub row_field_names: Vec<String>,
    /// Display names of the column fields, outermost first.
    pub col_field_names: Vec<String>,
    /// Header caption for the value area, e.g. `"Sum of Amount"`.
    pub value_caption: String,
    /// How many source rows were included (after source filters).
    pub source_row_count: usize,
}

impl PivotResult {
    /// Aggregated value for `(row_key, col_key)` where either key may be
    /// [`TOTAL_KEY`]. Missing intersections (no source rows) are `None`.
    #[must_use]
    pub fn value(&self, row_key: usize, col_key: usize) -> &CellValue {
        self.values
            .get(&(row_key, col_key))
            .unwrap_or(&CellValue::None)
    }

    /// Ids of row-axis leaves in canonical depth-first order.
    #[must_use]
    pub fn row_leaves(&self) -> Vec<usize> {
        collect_leaves(&self.row_nodes, &self.row_roots)
    }

    /// Ids of column-axis leaves in canonical depth-first order.
    #[must_use]
    pub fn col_leaves(&self) -> Vec<usize> {
        collect_leaves(&self.col_nodes, &self.col_roots)
    }
}

fn collect_leaves(nodes: &[PivotNode], roots: &[usize]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack: Vec<usize> = roots.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let node = &nodes[id];
        if node.is_leaf() {
            out.push(id);
        } else {
            for &child in node.children.iter().rev() {
                stack.push(child);
            }
        }
    }
    out
}

/// Incrementally builds one axis' group tree while source rows stream by.
struct AxisBuilder {
    nodes: Vec<PivotNode>,
    roots: Vec<usize>,
    /// `(parent id or TOTAL_KEY for roots, label) -> node id`.
    index: HashMap<(usize, String), usize>,
}

impl AxisBuilder {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            roots: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// Resolve the node path for one source row down this axis. Returns the
    /// node id at every depth (outermost first). Empty when the axis has no
    /// fields.
    fn path_for_row(
        &mut self,
        row: &[CellValue],
        fields: &[usize],
        formats: &[ResolvedColumnFormat],
        blank_label: &str,
    ) -> Vec<usize> {
        let mut path = Vec::with_capacity(fields.len());
        let mut parent_key = TOTAL_KEY;
        for (depth, &field) in fields.iter().enumerate() {
            let cell = row.get(field).unwrap_or(&CellValue::None);
            let label = if matches!(cell, CellValue::None) {
                blank_label.to_owned()
            } else {
                format_cell(cell, &formats[field]).0
            };
            let id = match self.index.get(&(parent_key, label.clone())) {
                Some(&id) => id,
                None => {
                    let id = self.nodes.len();
                    self.nodes.push(PivotNode {
                        label: label.clone(),
                        sort_key: cell.clone(),
                        depth,
                        parent: (parent_key != TOTAL_KEY).then_some(parent_key),
                        children: Vec::new(),
                        total: CellValue::None,
                    });
                    self.index.insert((parent_key, label), id);
                    if parent_key == TOTAL_KEY {
                        self.roots.push(id);
                    } else {
                        self.nodes[parent_key].children.push(id);
                    }
                    id
                }
            };
            path.push(id);
            parent_key = id;
        }
        path
    }

    /// Sort every sibling list by the raw grouping value (ascending) so the
    /// initial presentation is deterministic and natural.
    fn sort_canonical(&mut self) {
        let keys: Vec<CellValue> = self.nodes.iter().map(|n| n.sort_key.clone()).collect();
        let by_key = |a: &usize, b: &usize| compare_cells(&keys[*a], &keys[*b]);
        self.roots.sort_by(by_key);
        for node in &mut self.nodes {
            node.children.sort_by(by_key);
        }
    }
}

/// Compute a pivot over `rows[source_rows...]`.
///
/// * `columns` / `formats` describe the source schema (formats drive group
///   labels, so date grouping follows the column's display format).
/// * `source_rows` selects which rows participate — the caller applies any
///   source filters first. Pass `0..rows.len()` for everything.
///
/// The source slices are only read; nothing is cloned except the small
/// grouping values that become node labels/sort keys.
#[must_use]
pub fn compute_pivot(
    columns: &[Column],
    rows: &[Vec<CellValue>],
    source_rows: &[usize],
    config: &PivotConfig,
    formats: &[ResolvedColumnFormat],
) -> PivotResult {
    let mut row_axis = AxisBuilder::new();
    let mut col_axis = AxisBuilder::new();
    let mut accs: HashMap<(usize, usize), Accumulator> = HashMap::new();

    let value_field = config.value_field;
    let agg = config.aggregation;

    for &row_idx in source_rows {
        let Some(row) = rows.get(row_idx) else {
            continue;
        };
        let row_path = row_axis.path_for_row(row, &config.row_fields, formats, &config.blank_label);
        let col_path =
            col_axis.path_for_row(row, &config.column_fields, formats, &config.blank_label);
        let Some(vf) = value_field else {
            continue;
        };
        let value = row.get(vf).unwrap_or(&CellValue::None);
        // Ingest at every rollup level: each ancestor (incl. leaf) on the row
        // path plus the axis total, crossed with the same on the column path.
        for &rk in row_path.iter().chain(std::iter::once(&TOTAL_KEY)) {
            for &ck in col_path.iter().chain(std::iter::once(&TOTAL_KEY)) {
                accs.entry((rk, ck))
                    .or_insert_with(|| Accumulator::new(agg))
                    .ingest(value);
            }
        }
    }

    row_axis.sort_canonical();
    col_axis.sort_canonical();

    let values: HashMap<(usize, usize), CellValue> =
        accs.iter().map(|(k, acc)| (*k, acc.finish())).collect();

    let mut row_nodes = row_axis.nodes;
    let mut col_nodes = col_axis.nodes;
    for (id, node) in row_nodes.iter_mut().enumerate() {
        node.total = values
            .get(&(id, TOTAL_KEY))
            .cloned()
            .unwrap_or(CellValue::None);
    }
    for (id, node) in col_nodes.iter_mut().enumerate() {
        node.total = values
            .get(&(TOTAL_KEY, id))
            .cloned()
            .unwrap_or(CellValue::None);
    }
    let grand_total = values
        .get(&(TOTAL_KEY, TOTAL_KEY))
        .cloned()
        .unwrap_or(CellValue::None);

    let field_name = |idx: usize| {
        columns
            .get(idx)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("column {idx}"))
    };
    let value_caption = match value_field {
        Some(vf) => agg.caption(&field_name(vf)),
        None => "Values".to_owned(),
    };

    PivotResult {
        row_roots: row_axis.roots,
        col_roots: col_axis.roots,
        row_nodes,
        col_nodes,
        values,
        grand_total,
        row_depth: config.row_fields.len(),
        col_depth: config.column_fields.len(),
        row_field_names: config.row_fields.iter().map(|&f| field_name(f)).collect(),
        col_field_names: config
            .column_fields
            .iter()
            .map(|&f| field_name(f))
            .collect(),
        value_caption,
        source_row_count: source_rows.len(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::GridConfig;
    use crate::data::ColumnKind;
    use crate::pivot::aggregation::AggregationFn;
    use CellValue::{Decimal, Integer, None as Null, Text};

    /// region / product / year / amount ledger used across tests.
    fn fixture() -> (Vec<Column>, Vec<Vec<CellValue>>, Vec<ResolvedColumnFormat>) {
        let columns = vec![
            Column::new("region", ColumnKind::Text, 100.0),
            Column::new("product", ColumnKind::Text, 100.0),
            Column::new("year", ColumnKind::Integer, 80.0),
            Column::new("amount", ColumnKind::Decimal, 100.0),
        ];
        let r = |region: &str, product: &str, year: i64, amount: f64| {
            vec![
                Text(region.into()),
                Text(product.into()),
                Integer(year),
                Decimal(amount),
            ]
        };
        let rows = vec![
            r("Europe", "Widget", 2023, 10.0),
            r("Europe", "Widget", 2024, 20.0),
            r("Europe", "Gadget", 2023, 5.0),
            r("Asia", "Widget", 2023, 7.0),
            r("Asia", "Gadget", 2024, 3.0),
            r("Asia", "Gadget", 2024, 4.0),
        ];
        let formats = GridConfig::default().resolve_all(&columns);
        (columns, rows, formats)
    }

    fn all_rows(rows: &[Vec<CellValue>]) -> Vec<usize> {
        (0..rows.len()).collect()
    }

    fn config(rows: &[usize], cols: &[usize], value: usize, agg: AggregationFn) -> PivotConfig {
        PivotConfig {
            row_fields: rows.to_vec(),
            column_fields: cols.to_vec(),
            value_field: Some(value),
            aggregation: agg,
            ..PivotConfig::default()
        }
    }

    fn node_by_label<'a>(result: &'a PivotResult, label: &str) -> (usize, &'a PivotNode) {
        result
            .row_nodes
            .iter()
            .enumerate()
            .find(|(_, n)| n.label == label)
            .unwrap()
    }

    #[test]
    fn single_row_and_column_field_sum() {
        let (columns, rows, formats) = fixture();
        let cfg = config(&[0], &[2], 3, AggregationFn::Sum);
        let result = compute_pivot(&columns, &rows, &all_rows(&rows), &cfg, &formats);

        assert_eq!(result.row_roots.len(), 2); // Asia, Europe
        assert_eq!(result.col_roots.len(), 2); // 2023, 2024
                                               // Canonical order is ascending by raw value.
        assert_eq!(result.row_nodes[result.row_roots[0]].label, "Asia");
        assert_eq!(result.row_nodes[result.row_roots[1]].label, "Europe");

        let (europe, europe_node) = node_by_label(&result, "Europe");
        let y2023 = result.col_roots[0];
        let y2024 = result.col_roots[1];
        assert_eq!(result.value(europe, y2023), &Decimal(15.0));
        assert_eq!(result.value(europe, y2024), &Decimal(20.0));
        assert_eq!(europe_node.total, Decimal(35.0));

        let (asia, asia_node) = node_by_label(&result, "Asia");
        assert_eq!(result.value(asia, y2023), &Decimal(7.0));
        assert_eq!(result.value(asia, y2024), &Decimal(7.0));
        assert_eq!(asia_node.total, Decimal(14.0));

        // Column totals and grand total.
        assert_eq!(result.col_nodes[y2023].total, Decimal(22.0));
        assert_eq!(result.col_nodes[y2024].total, Decimal(27.0));
        assert_eq!(result.grand_total, Decimal(49.0));
    }

    #[test]
    fn two_row_fields_build_two_level_tree_with_subtotals() {
        let (columns, rows, formats) = fixture();
        let cfg = config(&[0, 1], &[], 3, AggregationFn::Sum);
        let result = compute_pivot(&columns, &rows, &all_rows(&rows), &cfg, &formats);

        assert_eq!(result.row_depth, 2);
        let (europe, europe_node) = node_by_label(&result, "Europe");
        assert_eq!(europe_node.depth, 0);
        assert_eq!(europe_node.children.len(), 2); // Gadget, Widget
        let child_labels: Vec<&str> = europe_node
            .children
            .iter()
            .map(|&c| result.row_nodes[c].label.as_str())
            .collect();
        assert_eq!(child_labels, vec!["Gadget", "Widget"]);
        // Group subtotal at the parent level.
        assert_eq!(europe_node.total, Decimal(35.0));
        // Leaf value: Europe->Widget across (no column fields => TOTAL col).
        let widget = europe_node
            .children
            .iter()
            .copied()
            .find(|&c| result.row_nodes[c].label == "Widget")
            .unwrap();
        assert_eq!(result.value(widget, TOTAL_KEY), &Decimal(30.0));
        assert_eq!(result.row_nodes[widget].parent, Some(europe));
        // Leaves enumerate depth-first: Asia(Gadget,Widget), Europe(Gadget,Widget).
        let leaves = result.row_leaves();
        let leaf_labels: Vec<&str> = leaves
            .iter()
            .map(|&l| result.row_nodes[l].label.as_str())
            .collect();
        assert_eq!(leaf_labels, vec!["Gadget", "Widget", "Gadget", "Widget"]);
    }

    #[test]
    fn count_and_avg_levels_are_computed_from_source_not_from_child_aggregates() {
        let (columns, rows, formats) = fixture();
        let cfg = config(&[0], &[], 3, AggregationFn::Avg);
        let result = compute_pivot(&columns, &rows, &all_rows(&rows), &cfg, &formats);
        let (_, europe) = node_by_label(&result, "Europe");
        // Europe rows: 10, 20, 5 -> avg 35/3, NOT avg of per-product avgs.
        match &europe.total {
            Decimal(v) => assert!((v - 35.0 / 3.0).abs() < 1e-9),
            other => panic!("expected Decimal, got {other:?}"),
        }
        match &result.grand_total {
            Decimal(v) => assert!((v - 49.0 / 6.0).abs() < 1e-9),
            other => panic!("expected Decimal, got {other:?}"),
        }
    }

    #[test]
    fn count_aggregation_reports_integers() {
        let (columns, rows, formats) = fixture();
        let cfg = config(&[0], &[2], 3, AggregationFn::Count);
        let result = compute_pivot(&columns, &rows, &all_rows(&rows), &cfg, &formats);
        let (asia, _) = node_by_label(&result, "Asia");
        let y2024 = result.col_roots[1];
        assert_eq!(result.value(asia, y2024), &Integer(2));
        assert_eq!(result.grand_total, Integer(6));
    }

    #[test]
    fn empty_source_produces_empty_result_without_panic() {
        let (columns, _, formats) = fixture();
        let rows: Vec<Vec<CellValue>> = vec![];
        let cfg = config(&[0], &[2], 3, AggregationFn::Sum);
        let result = compute_pivot(&columns, &rows, &[], &cfg, &formats);
        assert!(result.row_roots.is_empty());
        assert!(result.col_roots.is_empty());
        assert_eq!(result.grand_total, Null);
        assert_eq!(result.source_row_count, 0);
    }

    #[test]
    fn null_grouping_values_bucket_under_blank_label() {
        let (columns, mut rows, formats) = fixture();
        rows.push(vec![
            Null,
            Text("Widget".into()),
            Integer(2023),
            Decimal(2.0),
        ]);
        rows.push(vec![
            Null,
            Text("Gadget".into()),
            Integer(2023),
            Decimal(3.0),
        ]);
        let cfg = config(&[0], &[], 3, AggregationFn::Sum);
        let result = compute_pivot(&columns, &rows, &all_rows(&rows), &cfg, &formats);
        let (blank, blank_node) = node_by_label(&result, "(blank)");
        assert_eq!(blank_node.sort_key, Null);
        assert_eq!(result.value(blank, TOTAL_KEY), &Decimal(5.0));
        // Null sorts first, so (blank) is the first root.
        assert_eq!(result.row_roots[0], blank);
    }

    #[test]
    fn source_row_subset_excludes_filtered_rows() {
        let (columns, rows, formats) = fixture();
        let cfg = config(&[0], &[], 3, AggregationFn::Sum);
        // Only the Asia rows (indices 3..6).
        let result = compute_pivot(&columns, &rows, &[3, 4, 5], &cfg, &formats);
        assert_eq!(result.row_roots.len(), 1);
        assert_eq!(result.row_nodes[result.row_roots[0]].label, "Asia");
        assert_eq!(result.grand_total, Decimal(14.0));
        assert_eq!(result.source_row_count, 3);
    }

    #[test]
    fn missing_intersections_read_as_none() {
        let (columns, rows, formats) = fixture();
        let cfg = config(&[0, 1], &[2], 3, AggregationFn::Sum);
        let result = compute_pivot(&columns, &rows, &all_rows(&rows), &cfg, &formats);
        // Asia -> Widget has no 2024 rows.
        let (asia, asia_node) = node_by_label(&result, "Asia");
        let widget = asia_node
            .children
            .iter()
            .copied()
            .find(|&c| result.row_nodes[c].label == "Widget")
            .unwrap();
        let y2024 = result.col_roots[1];
        assert_eq!(result.value(widget, y2024), &Null);
        assert_eq!(result.row_nodes[widget].parent, Some(asia));
    }

    #[test]
    fn value_caption_reflects_aggregation_and_field() {
        let (columns, rows, formats) = fixture();
        let cfg = config(&[0], &[], 3, AggregationFn::Sum);
        let result = compute_pivot(&columns, &rows, &all_rows(&rows), &cfg, &formats);
        assert_eq!(result.value_caption, "Sum of amount");
        assert_eq!(result.row_field_names, vec!["region".to_owned()]);
    }

    #[test]
    fn source_rows_are_not_mutated() {
        let (columns, rows, formats) = fixture();
        let snapshot = rows.clone();
        let cfg = config(&[0, 1], &[2], 3, AggregationFn::Avg);
        let _ = compute_pivot(&columns, &rows, &all_rows(&rows), &cfg, &formats);
        assert_eq!(rows, snapshot);
    }

    #[test]
    fn boolean_and_date_grouping_fields_work() {
        let columns = vec![
            Column::new("flag", ColumnKind::Boolean, 80.0),
            Column::new("when", ColumnKind::Date, 120.0),
            Column::new("n", ColumnKind::Integer, 80.0),
        ];
        let rows = vec![
            vec![CellValue::Boolean(true), CellValue::Date(0), Integer(1)],
            vec![CellValue::Boolean(false), CellValue::Date(0), Integer(2)],
            vec![
                CellValue::Boolean(true),
                CellValue::Date(86_400),
                Integer(3),
            ],
        ];
        let formats = GridConfig::default().resolve_all(&columns);
        let cfg = config(&[0], &[1], 2, AggregationFn::Sum);
        let result = compute_pivot(&columns, &rows, &all_rows(&rows), &cfg, &formats);
        assert_eq!(result.row_roots.len(), 2);
        assert_eq!(result.col_roots.len(), 2);
        // Labels come from the resolved formats (default date is %Y-%m-%d).
        assert_eq!(result.col_nodes[result.col_roots[0]].label, "1970-01-01");
        let t = result
            .row_roots
            .iter()
            .copied()
            .find(|&r| result.row_nodes[r].label == "true")
            .unwrap();
        assert_eq!(result.row_nodes[t].total, Integer(4));
    }
}
