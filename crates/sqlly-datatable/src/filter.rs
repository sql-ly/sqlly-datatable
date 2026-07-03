//! Rich per-column filter model plus the pure matching pipeline.
//!
//! A [`ColumnFilter`] combines two independent, AND-composed mechanisms that
//! mirror the Numbers-style filter popover:
//!
//! * an optional operator **predicate** (the "Choose One" rule) — a text
//!   "like" operation (contains / begins with / regex …) for string columns,
//!   or a numeric/date comparison (greater than / between …) for numeric and
//!   date columns; and
//! * an optional **value set** (the searchable checkbox list) — the exact set
//!   of *formatted* values that are allowed through.
//!
//! Either half is inert when unset ([`FilterPredicate::None`] and
//! `values == None` respectively), so an empty [`ColumnFilter`] passes every
//! row. This module is intentionally GPUI-free: it operates on
//! [`CellValue`]/[`ResolvedColumnFormat`] and is reusable from exports, tests,
//! and server-side previews.

use std::cmp::Ordering;
use std::collections::HashSet;

use crate::config::ResolvedColumnFormat;
use crate::data::{CellValue, ColumnKind};
use crate::format::format_cell;

/// Text ("like") operators offered for string-like columns
/// ([`ColumnKind::Text`], [`ColumnKind::Boolean`], [`ColumnKind::None`]).
///
/// All comparisons except [`TextOp::Matches`] are plain case-insensitive
/// string operations against the *formatted* cell value. [`TextOp::Matches`]
/// compiles the operand as a case-insensitive regular expression; an invalid
/// pattern simply matches nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextOp {
    /// Formatted value contains the operand.
    Contains,
    /// Formatted value does not contain the operand.
    DoesNotContain,
    /// Formatted value starts with the operand.
    BeginsWith,
    /// Formatted value ends with the operand.
    EndsWith,
    /// Formatted value equals the operand exactly.
    Is,
    /// Formatted value differs from the operand.
    IsNot,
    /// Formatted value matches the operand interpreted as a regex.
    Matches,
}

/// Numeric / date comparison operators offered for [`ColumnKind::Integer`],
/// [`ColumnKind::Decimal`], and [`ColumnKind::Date`] columns.
///
/// Operands are stored as `f64`; date operands are the Unix-seconds value of
/// the parsed calendar date. Comparisons use [`f64::total_cmp`] so `NaN` is
/// ordered deterministically and never triggers a float-equality lint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberOp {
    /// `value == a`
    Eq,
    /// `value != a`
    Ne,
    /// `value > a`
    Gt,
    /// `value >= a`
    Ge,
    /// `value < a`
    Lt,
    /// `value <= a`
    Le,
    /// `min(a,b) <= value <= max(a,b)`
    Between,
    /// `value < min(a,b) || value > max(a,b)`
    NotBetween,
}

/// The operator-rule half of a [`ColumnFilter`].
#[derive(Clone, Debug, Default, PartialEq)]
pub enum FilterPredicate {
    /// No operator rule; this half is inert.
    #[default]
    None,
    /// A text "like" rule against the formatted value.
    Text {
        /// Which text operation to apply.
        op: TextOp,
        /// The right-hand operand (a substring, exact value, or regex).
        operand: String,
    },
    /// A numeric/date comparison. `b` is only used by
    /// [`NumberOp::Between`]/[`NumberOp::NotBetween`].
    Number {
        /// Which comparison to apply.
        op: NumberOp,
        /// Primary operand.
        a: f64,
        /// Secondary operand (range upper bound); ignored by single-operand ops.
        b: f64,
    },
}

/// A single column's committed filter: an optional operator predicate ANDed
/// with an optional allow-list of formatted values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColumnFilter {
    /// Operator rule ("Choose One"). [`FilterPredicate::None`] => inert.
    pub predicate: FilterPredicate,
    /// Allowed *formatted* values (checkbox list). `None` => every value
    /// passes; `Some(set)` => only values whose formatted text is in `set`.
    pub values: Option<HashSet<String>>,
}

impl ColumnFilter {
    /// `true` when either half constrains rows (used to paint the
    /// filtered-column marker and to decide whether "Clear filter" applies).
    #[must_use]
    pub fn is_active(&self) -> bool {
        !matches!(self.predicate, FilterPredicate::None) || self.values.is_some()
    }
}

/// `true` when a column of `kind` uses numeric/date operators rather than the
/// text "like" operator set.
#[must_use]
pub fn uses_number_ops(kind: ColumnKind) -> bool {
    matches!(
        kind,
        ColumnKind::Integer | ColumnKind::Decimal | ColumnKind::Date
    )
}

/// Evaluate a cell against a column filter. Empty/inert filters pass.
#[must_use]
pub fn cell_passes_filter(
    value: &CellValue,
    fmt: &ResolvedColumnFormat,
    filter: &ColumnFilter,
) -> bool {
    if !predicate_matches(value, fmt, &filter.predicate) {
        return false;
    }
    if let Some(allowed) = &filter.values {
        let (formatted, _) = format_cell(value, fmt);
        if !allowed.contains(&formatted) {
            return false;
        }
    }
    true
}

fn predicate_matches(
    value: &CellValue,
    fmt: &ResolvedColumnFormat,
    predicate: &FilterPredicate,
) -> bool {
    match predicate {
        FilterPredicate::None => true,
        FilterPredicate::Text { op, operand } => text_matches(value, fmt, *op, operand),
        FilterPredicate::Number { op, a, b } => number_matches(value, *op, *a, *b),
    }
}

fn text_matches(value: &CellValue, fmt: &ResolvedColumnFormat, op: TextOp, operand: &str) -> bool {
    let (formatted, _) = format_cell(value, fmt);
    if op == TextOp::Matches {
        return regex_matches(&formatted, operand);
    }
    let hay = formatted.to_lowercase();
    let needle = operand.to_lowercase();
    match op {
        TextOp::Contains => hay.contains(&needle),
        TextOp::DoesNotContain => !hay.contains(&needle),
        TextOp::BeginsWith => hay.starts_with(&needle),
        TextOp::EndsWith => hay.ends_with(&needle),
        TextOp::Is => hay == needle,
        TextOp::IsNot => hay != needle,
        TextOp::Matches => unreachable!("handled above"),
    }
}

fn regex_matches(hay: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    match regex::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
    {
        Ok(re) => re.is_match(hay),
        // An invalid pattern matches nothing rather than erroring the grid.
        Err(_) => false,
    }
}

/// Numeric projection used by number/date predicates. Non-numeric cells
/// (`Text`, `Boolean`, `None`) have no numeric value and never satisfy a
/// numeric predicate.
fn cell_number(value: &CellValue) -> Option<f64> {
    match value {
        CellValue::Integer(i) => Some(*i as f64),
        CellValue::Decimal(d) => Some(*d),
        CellValue::Date(t) => Some(*t as f64),
        CellValue::Text(_) | CellValue::Boolean(_) | CellValue::None => None,
    }
}

fn number_matches(value: &CellValue, op: NumberOp, a: f64, b: f64) -> bool {
    let Some(v) = cell_number(value) else {
        return false;
    };
    // `total_cmp` keeps the comparison total (NaN-safe) and sidesteps the
    // float-equality lint entirely.
    let ord = v.total_cmp(&a);
    match op {
        NumberOp::Eq => ord == Ordering::Equal,
        NumberOp::Ne => ord != Ordering::Equal,
        NumberOp::Gt => ord == Ordering::Greater,
        NumberOp::Ge => ord != Ordering::Less,
        NumberOp::Lt => ord == Ordering::Less,
        NumberOp::Le => ord != Ordering::Greater,
        NumberOp::Between => in_range(v, a, b),
        NumberOp::NotBetween => !in_range(v, a, b),
    }
}

fn in_range(v: f64, a: f64, b: f64) -> bool {
    let (lo, hi) = if a.total_cmp(&b) == Ordering::Greater {
        (b, a)
    } else {
        (a, b)
    };
    v.total_cmp(&lo) != Ordering::Less && v.total_cmp(&hi) != Ordering::Greater
}

/// Parse a `YYYY-MM-DD` calendar date into Unix seconds (UTC midnight).
/// Returns `None` for malformed input. Used to interpret date operands typed
/// into the filter panel's range fields.
#[must_use]
pub fn parse_ymd_to_unix(s: &str) -> Option<i64> {
    let t = s.trim();
    let mut parts = t.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d) * 86_400)
}

/// Howard Hinnant's `days_from_civil`: days since the Unix epoch for a proleptic
/// Gregorian calendar date. Inverse of `format::days_to_ymd`.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::config::{BooleanFormat, DateFormat, NumberFormat, ReplacementTiming, StringFormat};

    fn resolved(kind: ColumnKind) -> ResolvedColumnFormat {
        ResolvedColumnFormat {
            kind,
            number: NumberFormat::default(),
            date: DateFormat::default(),
            boolean: BooleanFormat::default(),
            string: StringFormat::default(),
            replacements: vec![],
            replacement_timing: ReplacementTiming::AfterFormat,
        }
    }

    fn text_filter(op: TextOp, operand: &str) -> ColumnFilter {
        ColumnFilter {
            predicate: FilterPredicate::Text {
                op,
                operand: operand.to_owned(),
            },
            values: None,
        }
    }

    fn number_filter(op: NumberOp, a: f64, b: f64) -> ColumnFilter {
        ColumnFilter {
            predicate: FilterPredicate::Number { op, a, b },
            values: None,
        }
    }

    #[test]
    fn empty_filter_passes_everything() {
        let f = ColumnFilter::default();
        assert!(!f.is_active());
        assert!(cell_passes_filter(
            &CellValue::Text("anything".into()),
            &resolved(ColumnKind::Text),
            &f
        ));
    }

    #[test]
    fn text_ops_are_case_insensitive() {
        let fmt = resolved(ColumnKind::Text);
        let v = CellValue::Text("Hello World".into());
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &text_filter(TextOp::Contains, "LO W")
        ));
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &text_filter(TextOp::BeginsWith, "hell")
        ));
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &text_filter(TextOp::EndsWith, "RLD")
        ));
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &text_filter(TextOp::Is, "hello world")
        ));
        assert!(!cell_passes_filter(
            &v,
            &fmt,
            &text_filter(TextOp::IsNot, "hello world")
        ));
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &text_filter(TextOp::DoesNotContain, "zzz")
        ));
    }

    #[test]
    fn text_matches_regex_and_bad_regex_matches_nothing() {
        let fmt = resolved(ColumnKind::Text);
        let v = CellValue::Text("abc123".into());
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &text_filter(TextOp::Matches, r"^abc\d+$")
        ));
        assert!(!cell_passes_filter(
            &v,
            &fmt,
            &text_filter(TextOp::Matches, r"^\d+$")
        ));
        // Unbalanced group => invalid regex => matches nothing.
        assert!(!cell_passes_filter(
            &v,
            &fmt,
            &text_filter(TextOp::Matches, "(")
        ));
    }

    #[test]
    fn number_ops_cover_comparisons_and_ranges() {
        let fmt = resolved(ColumnKind::Integer);
        let v = CellValue::Integer(50);
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &number_filter(NumberOp::Eq, 50.0, 0.0)
        ));
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &number_filter(NumberOp::Ne, 51.0, 0.0)
        ));
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &number_filter(NumberOp::Gt, 49.0, 0.0)
        ));
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &number_filter(NumberOp::Ge, 50.0, 0.0)
        ));
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &number_filter(NumberOp::Lt, 51.0, 0.0)
        ));
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &number_filter(NumberOp::Le, 50.0, 0.0)
        ));
        // Between is order-insensitive on its bounds.
        assert!(cell_passes_filter(
            &v,
            &fmt,
            &number_filter(NumberOp::Between, 100.0, 10.0)
        ));
        assert!(!cell_passes_filter(
            &v,
            &fmt,
            &number_filter(NumberOp::NotBetween, 10.0, 100.0)
        ));
    }

    #[test]
    fn number_predicate_rejects_non_numeric_cells() {
        let fmt = resolved(ColumnKind::Integer);
        assert!(!cell_passes_filter(
            &CellValue::None,
            &fmt,
            &number_filter(NumberOp::Ge, 0.0, 0.0)
        ));
    }

    #[test]
    fn value_set_allow_list_filters() {
        let fmt = resolved(ColumnKind::Text);
        let mut allowed = HashSet::new();
        allowed.insert("keep".to_owned());
        let f = ColumnFilter {
            predicate: FilterPredicate::None,
            values: Some(allowed),
        };
        assert!(f.is_active());
        assert!(cell_passes_filter(
            &CellValue::Text("keep".into()),
            &fmt,
            &f
        ));
        assert!(!cell_passes_filter(
            &CellValue::Text("drop".into()),
            &fmt,
            &f
        ));
    }

    #[test]
    fn predicate_and_value_set_compose_with_and() {
        let fmt = resolved(ColumnKind::Text);
        let mut allowed = HashSet::new();
        allowed.insert("alpha".to_owned());
        allowed.insert("apex".to_owned());
        let f = ColumnFilter {
            predicate: FilterPredicate::Text {
                op: TextOp::BeginsWith,
                operand: "al".into(),
            },
            values: Some(allowed),
        };
        // In the allow-list AND matches the predicate.
        assert!(cell_passes_filter(
            &CellValue::Text("alpha".into()),
            &fmt,
            &f
        ));
        // In the allow-list but fails the predicate.
        assert!(!cell_passes_filter(
            &CellValue::Text("apex".into()),
            &fmt,
            &f
        ));
    }

    #[test]
    fn date_range_via_parsed_operands() {
        let fmt = resolved(ColumnKind::Date);
        // 2024-01-01 UTC == 1_704_067_200.
        let jan1 = parse_ymd_to_unix("2024-01-01").expect("valid date");
        assert_eq!(jan1, 1_704_067_200);
        let feb1 = parse_ymd_to_unix("2024-02-01").expect("valid date");
        let v = CellValue::Date(1_706_000_000); // late Jan 2024
        let f = number_filter(NumberOp::Between, jan1 as f64, feb1 as f64);
        assert!(cell_passes_filter(&v, &fmt, &f));
    }

    #[test]
    fn parse_ymd_rejects_garbage() {
        assert!(parse_ymd_to_unix("not-a-date").is_none());
        assert!(parse_ymd_to_unix("2024-13-01").is_none());
        assert!(parse_ymd_to_unix("2024-01-32").is_none());
        assert!(parse_ymd_to_unix("2024-01-01-01").is_none());
    }

    #[test]
    fn uses_number_ops_matches_numeric_kinds() {
        assert!(uses_number_ops(ColumnKind::Integer));
        assert!(uses_number_ops(ColumnKind::Decimal));
        assert!(uses_number_ops(ColumnKind::Date));
        assert!(!uses_number_ops(ColumnKind::Text));
        assert!(!uses_number_ops(ColumnKind::Boolean));
        assert!(!uses_number_ops(ColumnKind::None));
    }
}
