//! [`PivotConfig`] — the complete, GPUI-free description of a pivot layout.
//!
//! The struct is plain data: reading it back *is* the "get current
//! configuration" API, and constructing/mutating one is the programmatic
//! preconfiguration API. The sidebar mutates the same struct through
//! [`PivotConfig::move_field`] / [`PivotConfig::remove_field`] so drag-and-drop
//! and code paths cannot diverge.

use crate::config::NumberFormat;
use crate::pivot::aggregation::AggregationFn;

/// The four sidebar drop zones a source column can be assigned to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PivotZone {
    /// Distinct values become the left-axis row groups.
    Rows,
    /// Distinct values become the top-axis column groups.
    Columns,
    /// The single measure aggregated at each intersection.
    Values,
    /// Source-row filters applied before pivoting.
    Filters,
}

/// Field assignments plus display options for a pivot view.
///
/// All field references are **source column indices** into the grid's
/// `GridData::columns`.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotConfig {
    /// Columns whose distinct values form the row axis, outermost first.
    pub row_fields: Vec<usize>,
    /// Columns whose distinct values form the column axis, outermost first.
    pub column_fields: Vec<usize>,
    /// The measure column. `None` renders an empty pivot prompting for
    /// configuration.
    pub value_field: Option<usize>,
    /// How intersection values are combined.
    pub aggregation: AggregationFn,
    /// Columns available as source-row filters (the Filters zone). The
    /// actual predicate state lives on the pivot state, not the config.
    pub filter_fields: Vec<usize>,
    /// Show subtotal values on expanded row group headers.
    pub show_row_subtotals: bool,
    /// Append a "Total" column after each expanded column group.
    pub show_column_subtotals: bool,
    /// Show the grand-total row at the bottom.
    pub show_row_grand_total: bool,
    /// Show the grand-total column at the right.
    pub show_column_grand_total: bool,
    /// Label used when a grouping value is `CellValue::None`.
    pub blank_label: String,
    /// Optional number-format override for value cells. `None` falls back
    /// to the value column's resolved format.
    pub value_format: Option<NumberFormat>,
}

impl Default for PivotConfig {
    fn default() -> Self {
        Self {
            row_fields: vec![],
            column_fields: vec![],
            value_field: None,
            aggregation: AggregationFn::default(),
            filter_fields: vec![],
            show_row_subtotals: true,
            show_column_subtotals: false,
            show_row_grand_total: true,
            show_column_grand_total: true,
            blank_label: "(blank)".into(),
            value_format: None,
        }
    }
}

impl PivotConfig {
    /// `true` when there is enough configuration to compute a pivot: a value
    /// field plus at least one axis field.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.value_field.is_some()
            && (!self.row_fields.is_empty() || !self.column_fields.is_empty())
    }

    /// The zone `field` is currently assigned to, if any.
    #[must_use]
    pub fn zone_of(&self, field: usize) -> Option<PivotZone> {
        if self.row_fields.contains(&field) {
            Some(PivotZone::Rows)
        } else if self.column_fields.contains(&field) {
            Some(PivotZone::Columns)
        } else if self.value_field == Some(field) {
            Some(PivotZone::Values)
        } else if self.filter_fields.contains(&field) {
            Some(PivotZone::Filters)
        } else {
            None
        }
    }

    /// Fields currently assigned to `zone`, in display order.
    #[must_use]
    pub fn fields_in(&self, zone: PivotZone) -> Vec<usize> {
        match zone {
            PivotZone::Rows => self.row_fields.clone(),
            PivotZone::Columns => self.column_fields.clone(),
            PivotZone::Values => self.value_field.into_iter().collect(),
            PivotZone::Filters => self.filter_fields.clone(),
        }
    }

    /// Assign `field` to `zone` at `index` (clamped; `None` appends).
    ///
    /// A field lives in at most one zone, so any previous assignment is
    /// removed first. Dropping onto [`PivotZone::Values`] replaces the
    /// current value field — the zone holds exactly one.
    pub fn move_field(&mut self, field: usize, zone: PivotZone, index: Option<usize>) {
        self.remove_field(field);
        match zone {
            PivotZone::Rows => insert_clamped(&mut self.row_fields, field, index),
            PivotZone::Columns => insert_clamped(&mut self.column_fields, field, index),
            PivotZone::Values => self.value_field = Some(field),
            PivotZone::Filters => insert_clamped(&mut self.filter_fields, field, index),
        }
    }

    /// Remove `field` from whatever zone holds it. No-op when unassigned.
    pub fn remove_field(&mut self, field: usize) {
        self.row_fields.retain(|&f| f != field);
        self.column_fields.retain(|&f| f != field);
        self.filter_fields.retain(|&f| f != field);
        if self.value_field == Some(field) {
            self.value_field = None;
        }
    }

    /// Drop every assignment that refers to a column index outside
    /// `0..column_count`. Call after the source schema changes.
    pub fn clamp_to_columns(&mut self, column_count: usize) {
        self.row_fields.retain(|&f| f < column_count);
        self.column_fields.retain(|&f| f < column_count);
        self.filter_fields.retain(|&f| f < column_count);
        if self.value_field.is_some_and(|f| f >= column_count) {
            self.value_field = None;
        }
    }
}

fn insert_clamped(list: &mut Vec<usize>, field: usize, index: Option<usize>) {
    let at = index.unwrap_or(list.len()).min(list.len());
    list.insert(at, field);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn default_is_not_ready() {
        let cfg = PivotConfig::default();
        assert!(!cfg.is_ready());
        assert_eq!(cfg.zone_of(0), None);
    }

    #[test]
    fn ready_needs_value_and_one_axis() {
        let mut cfg = PivotConfig::default();
        cfg.value_field = Some(2);
        assert!(!cfg.is_ready(), "value alone is not enough");
        cfg.row_fields = vec![0];
        assert!(cfg.is_ready());
        cfg.row_fields.clear();
        cfg.column_fields = vec![1];
        assert!(cfg.is_ready());
    }

    #[test]
    fn move_field_between_zones_is_exclusive() {
        let mut cfg = PivotConfig::default();
        cfg.move_field(3, PivotZone::Rows, None);
        assert_eq!(cfg.zone_of(3), Some(PivotZone::Rows));
        cfg.move_field(3, PivotZone::Columns, None);
        assert_eq!(cfg.zone_of(3), Some(PivotZone::Columns));
        assert!(cfg.row_fields.is_empty());
        cfg.move_field(3, PivotZone::Filters, None);
        assert_eq!(cfg.zone_of(3), Some(PivotZone::Filters));
        assert!(cfg.column_fields.is_empty());
    }

    #[test]
    fn values_zone_holds_exactly_one() {
        let mut cfg = PivotConfig::default();
        cfg.move_field(1, PivotZone::Values, None);
        cfg.move_field(2, PivotZone::Values, None);
        assert_eq!(cfg.value_field, Some(2));
        // The displaced field is unassigned, not relocated.
        assert_eq!(cfg.zone_of(1), None);
    }

    #[test]
    fn move_field_respects_insertion_index() {
        let mut cfg = PivotConfig::default();
        cfg.move_field(1, PivotZone::Rows, None);
        cfg.move_field(2, PivotZone::Rows, None);
        cfg.move_field(3, PivotZone::Rows, Some(0));
        assert_eq!(cfg.row_fields, vec![3, 1, 2]);
        // Out-of-range index clamps to append.
        cfg.move_field(4, PivotZone::Rows, Some(99));
        assert_eq!(cfg.row_fields, vec![3, 1, 2, 4]);
    }

    #[test]
    fn reorder_within_same_zone() {
        let mut cfg = PivotConfig::default();
        cfg.row_fields = vec![1, 2, 3];
        cfg.move_field(3, PivotZone::Rows, Some(0));
        assert_eq!(cfg.row_fields, vec![3, 1, 2]);
    }

    #[test]
    fn remove_field_clears_every_zone() {
        let mut cfg = PivotConfig::default();
        cfg.row_fields = vec![1];
        cfg.value_field = Some(2);
        cfg.remove_field(1);
        cfg.remove_field(2);
        assert!(cfg.row_fields.is_empty());
        assert_eq!(cfg.value_field, None);
    }

    #[test]
    fn clamp_to_columns_drops_out_of_range() {
        let mut cfg = PivotConfig::default();
        cfg.row_fields = vec![0, 5];
        cfg.column_fields = vec![9];
        cfg.filter_fields = vec![2, 7];
        cfg.value_field = Some(6);
        cfg.clamp_to_columns(4);
        assert_eq!(cfg.row_fields, vec![0]);
        assert!(cfg.column_fields.is_empty());
        assert_eq!(cfg.filter_fields, vec![2]);
        assert_eq!(cfg.value_field, None);
    }

    #[test]
    fn fields_in_reports_zone_contents() {
        let mut cfg = PivotConfig::default();
        cfg.row_fields = vec![4, 1];
        cfg.value_field = Some(0);
        assert_eq!(cfg.fields_in(PivotZone::Rows), vec![4, 1]);
        assert_eq!(cfg.fields_in(PivotZone::Values), vec![0]);
        assert!(cfg.fields_in(PivotZone::Columns).is_empty());
    }
}
