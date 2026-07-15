//! Aggregation functions applied at every pivot intersection.
//!
//! GPUI-free on purpose: the same [`AggregationFn`] / [`Accumulator`] pair is
//! usable from export pipelines and tests. All five functions are streaming —
//! the engine feeds source cells one at a time and never materializes the
//! per-group value lists.

use crate::data::{compare_cells, CellValue};
use std::cmp::Ordering;

/// How the value field is combined at each pivot intersection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AggregationFn {
    /// Number of non-null values.
    Count,
    /// Numeric sum. Integers stay integers until a decimal (or `i64`
    /// overflow) forces promotion to `f64`.
    #[default]
    Sum,
    /// Numeric mean; always produces [`CellValue::Decimal`].
    Avg,
    /// Smallest value under [`compare_cells`]; preserves the source kind.
    Min,
    /// Largest value under [`compare_cells`]; preserves the source kind.
    Max,
}

impl AggregationFn {
    /// All functions, in the order pickers should present them.
    #[must_use]
    pub fn all() -> [AggregationFn; 5] {
        [Self::Count, Self::Sum, Self::Avg, Self::Min, Self::Max]
    }

    /// Short label ("Count", "Sum", …).
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Count => "Count",
            Self::Sum => "Sum",
            Self::Avg => "Avg",
            Self::Min => "Min",
            Self::Max => "Max",
        }
    }

    /// Header caption for the value area, e.g. `"Sum of Amount"`.
    #[must_use]
    pub fn caption(&self, field_name: &str) -> String {
        format!("{} of {}", self.label(), field_name)
    }
}

/// Streaming accumulator for one pivot intersection. Feed cells with
/// [`Accumulator::ingest`], read the result with [`Accumulator::finish`].
#[derive(Clone, Debug)]
pub struct Accumulator {
    func: AggregationFn,
    count: u64,
    int_sum: i64,
    float_sum: f64,
    /// Set once an ingested value was `Decimal`, or the integer sum
    /// overflowed and had to be promoted.
    promoted: bool,
    extreme: Option<CellValue>,
}

impl Accumulator {
    /// A fresh accumulator for `func` with no values ingested.
    #[must_use]
    pub fn new(func: AggregationFn) -> Self {
        Self {
            func,
            count: 0,
            int_sum: 0,
            float_sum: 0.0,
            promoted: false,
            extreme: None,
        }
    }

    /// Feed one source cell. `CellValue::None` is skipped by every function
    /// (Excel semantics: blanks do not count and do not sum). Non-numeric
    /// values contribute to `Count`/`Min`/`Max` but are ignored by
    /// `Sum`/`Avg`.
    pub fn ingest(&mut self, value: &CellValue) {
        if matches!(value, CellValue::None) {
            return;
        }
        self.count += 1;
        match self.func {
            AggregationFn::Count => {}
            AggregationFn::Sum | AggregationFn::Avg => match value {
                CellValue::Integer(v) => {
                    if self.promoted {
                        self.float_sum += *v as f64;
                    } else {
                        match self.int_sum.checked_add(*v) {
                            Some(next) => self.int_sum = next,
                            None => {
                                self.promote();
                                self.float_sum += *v as f64;
                            }
                        }
                    }
                }
                CellValue::Decimal(v) => {
                    if !self.promoted {
                        self.promote();
                    }
                    self.float_sum += *v;
                }
                // Text/Date/Boolean cells do not participate in numeric sums;
                // they still count toward the row having values so an
                // Avg over a purely non-numeric group finishes as None below.
                _ => self.count -= 1,
            },
            AggregationFn::Min => {
                let replace = match &self.extreme {
                    Some(current) => compare_cells(value, current) == Ordering::Less,
                    None => true,
                };
                if replace {
                    self.extreme = Some(value.clone());
                }
            }
            AggregationFn::Max => {
                let replace = match &self.extreme {
                    Some(current) => compare_cells(value, current) == Ordering::Greater,
                    None => true,
                };
                if replace {
                    self.extreme = Some(value.clone());
                }
            }
        }
    }

    fn promote(&mut self) {
        self.float_sum += self.int_sum as f64;
        self.int_sum = 0;
        self.promoted = true;
    }

    /// Final aggregated cell. Empty input finishes as [`CellValue::None`]
    /// (the paint layer renders those as blank, not `0`).
    #[must_use]
    pub fn finish(&self) -> CellValue {
        if self.count == 0 && !matches!(self.func, AggregationFn::Count) {
            return CellValue::None;
        }
        match self.func {
            AggregationFn::Count => CellValue::Integer(self.count as i64),
            AggregationFn::Sum => {
                if self.promoted {
                    CellValue::Decimal(self.float_sum)
                } else {
                    CellValue::Integer(self.int_sum)
                }
            }
            AggregationFn::Avg => {
                let total = if self.promoted {
                    self.float_sum
                } else {
                    self.int_sum as f64
                };
                CellValue::Decimal(total / self.count as f64)
            }
            AggregationFn::Min | AggregationFn::Max => {
                self.extreme.clone().unwrap_or(CellValue::None)
            }
        }
    }
}

/// One-shot convenience over [`Accumulator`] for slices; used by tests and
/// small export paths.
#[must_use]
pub fn aggregate(values: &[CellValue], func: AggregationFn) -> CellValue {
    let mut acc = Accumulator::new(func);
    for v in values {
        acc.ingest(v);
    }
    acc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use CellValue::{Boolean, Decimal, Integer, None as Null, Text};

    #[test]
    fn count_skips_nulls_and_counts_everything_else() {
        let vals = vec![
            Integer(1),
            Null,
            Text("x".into()),
            Boolean(true),
            Decimal(2.5),
            Null,
        ];
        assert_eq!(aggregate(&vals, AggregationFn::Count), Integer(4));
    }

    #[test]
    fn count_of_empty_is_zero() {
        assert_eq!(aggregate(&[], AggregationFn::Count), Integer(0));
        assert_eq!(aggregate(&[Null, Null], AggregationFn::Count), Integer(0));
    }

    #[test]
    fn sum_of_integers_stays_integer() {
        let vals = vec![Integer(1), Integer(2), Integer(3)];
        assert_eq!(aggregate(&vals, AggregationFn::Sum), Integer(6));
    }

    #[test]
    fn sum_of_pure_decimals_is_decimal() {
        let vals = vec![Decimal(1.25), Decimal(2.5), Decimal(-0.75)];
        assert_eq!(aggregate(&vals, AggregationFn::Sum), Decimal(3.0));
    }

    #[test]
    fn sum_promotes_on_mixed_numeric() {
        let vals = vec![Integer(1), Decimal(0.5)];
        assert_eq!(aggregate(&vals, AggregationFn::Sum), Decimal(1.5));
    }

    #[test]
    fn sum_promotes_on_i64_overflow() {
        let vals = vec![Integer(i64::MAX), Integer(1)];
        match aggregate(&vals, AggregationFn::Sum) {
            Decimal(v) => {
                let expected = i64::MAX as f64 + 1.0;
                assert!((v - expected).abs() < 1e3, "got {v}");
            }
            other => panic!("expected Decimal, got {other:?}"),
        }
    }

    #[test]
    fn sum_ignores_non_numeric_and_null() {
        let vals = vec![Integer(4), Text("x".into()), Null, Boolean(true)];
        assert_eq!(aggregate(&vals, AggregationFn::Sum), Integer(4));
    }

    #[test]
    fn sum_of_empty_is_none() {
        assert_eq!(aggregate(&[], AggregationFn::Sum), Null);
        assert_eq!(aggregate(&[Null], AggregationFn::Sum), Null);
    }

    #[test]
    fn avg_is_always_decimal() {
        let vals = vec![Integer(1), Integer(2)];
        assert_eq!(aggregate(&vals, AggregationFn::Avg), Decimal(1.5));
        let vals = vec![Integer(2), Integer(2)];
        assert_eq!(aggregate(&vals, AggregationFn::Avg), Decimal(2.0));
    }

    #[test]
    fn avg_of_empty_is_none() {
        assert_eq!(aggregate(&[], AggregationFn::Avg), Null);
    }

    #[test]
    fn min_max_preserve_kind_and_skip_null() {
        let vals = vec![Null, Integer(5), Integer(2), Integer(9)];
        assert_eq!(aggregate(&vals, AggregationFn::Min), Integer(2));
        assert_eq!(aggregate(&vals, AggregationFn::Max), Integer(9));
        let dates = vec![CellValue::Date(200), CellValue::Date(100)];
        assert_eq!(aggregate(&dates, AggregationFn::Min), CellValue::Date(100));
        let texts = vec![Text("beta".into()), Text("alpha".into())];
        assert_eq!(aggregate(&texts, AggregationFn::Max), Text("beta".into()));
    }

    #[test]
    fn min_max_on_decimals() {
        let vals = vec![Decimal(2.5), Decimal(-1.25), Decimal(9.75), Null];
        assert_eq!(aggregate(&vals, AggregationFn::Min), Decimal(-1.25));
        assert_eq!(aggregate(&vals, AggregationFn::Max), Decimal(9.75));
    }

    #[test]
    fn min_max_on_mixed_integer_and_decimal_compare_numerically() {
        let vals = vec![Integer(2), Decimal(1.5), Decimal(2.5), Integer(3)];
        assert_eq!(aggregate(&vals, AggregationFn::Min), Decimal(1.5));
        assert_eq!(aggregate(&vals, AggregationFn::Max), Integer(3));
    }

    #[test]
    fn min_max_of_empty_is_none() {
        assert_eq!(aggregate(&[], AggregationFn::Min), Null);
        assert_eq!(aggregate(&[Null], AggregationFn::Max), Null);
    }

    #[test]
    fn caption_formats_label_and_field() {
        assert_eq!(AggregationFn::Sum.caption("Amount"), "Sum of Amount");
        assert_eq!(AggregationFn::Count.caption("Id"), "Count of Id");
    }

    #[test]
    fn all_lists_five_distinct_functions() {
        let all = AggregationFn::all();
        assert_eq!(all.len(), 5);
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
