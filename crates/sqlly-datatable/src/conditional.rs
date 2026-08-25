//! Conditional formatting: rule-driven per-cell colors, color scales, and
//! data bars.
//!
//! Rules are declared on [`crate::config::GridConfig::conditional_rules`] as a
//! list of [`ConditionalRule`]s, each targeting a column by name
//! (case-insensitive). The grid resolves them once per data/config change into
//! a [`ResolvedConditionals`] — per-column rule lists plus the precomputed
//! numeric min/max each [`ConditionalKind::ColorScale`] /
//! [`ConditionalKind::DataBar`] needs — so the per-visible-cell work during
//! paint is a cheap [`ColumnConditionals::evaluate`] call. When no rules are
//! configured, resolution yields an empty [`ResolvedConditionals`] and paint
//! skips the whole feature behind a single `is_empty` check.
//!
//! Everything in this module is intentionally GPUI-free apart from the final
//! [`RgbaColor`] → [`gpui::Hsla`] conversion, so rule evaluation is usable
//! (and tested) without a window. All declaration types serialize with serde,
//! letting hosts persist rule sets alongside their own settings.
//!
//! **Numeric semantics.** Only *typed* numeric cells participate in numeric
//! conditions, color scales, and data bars: [`CellValue::Integer`] and finite
//! [`CellValue::Decimal`]. Numeric-looking text is never parsed, and
//! non-finite decimals (`NaN`, ±∞) are excluded from both the per-column
//! min/max statistics and per-cell evaluation. Text conditions
//! ([`ConditionalCondition::EqualsText`] /
//! [`ConditionalCondition::ContainsText`]) match [`CellValue::Text`] cells
//! only, case-insensitively.

use crate::data::{CellValue, Column};
use gpui::Hsla;
use serde::{Deserialize, Serialize};

/// A serde-friendly RGBA color with `0.0..=1.0` channels.
///
/// The theme layer uses [`gpui::Hsla`] directly, but `Hsla` is a poor
/// persistence format for host-authored rules (authors think in RGB hex).
/// This is the declaration-side color type; [`RgbaColor::to_hsla`] (or the
/// [`From`] impl) converts to the paint layer's color space via
/// [`gpui::Rgba`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RgbaColor {
    /// Red channel, `0.0..=1.0`.
    pub r: f32,
    /// Green channel, `0.0..=1.0`.
    pub g: f32,
    /// Blue channel, `0.0..=1.0`.
    pub b: f32,
    /// Alpha channel, `0.0..=1.0`.
    pub a: f32,
}

impl RgbaColor {
    /// Convenience constructor.
    #[must_use]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Convert to the paint layer's [`gpui::Hsla`].
    #[must_use]
    pub fn to_hsla(self) -> Hsla {
        gpui::Rgba {
            r: self.r,
            g: self.g,
            b: self.b,
            a: self.a,
        }
        .into()
    }
}

impl From<RgbaColor> for Hsla {
    fn from(color: RgbaColor) -> Self {
        color.to_hsla()
    }
}

/// Predicate of a [`ConditionalKind::Rule`], tested per cell.
///
/// Numeric conditions apply to typed numeric cells only (see the module docs);
/// text conditions apply to [`CellValue::Text`] cells only and compare
/// case-insensitively. [`ConditionalCondition::Between`] is inclusive on both
/// ends and normalizes reversed bounds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConditionalCondition {
    /// Matches [`CellValue::None`].
    IsNull,
    /// Matches every cell except [`CellValue::None`].
    IsNotNull,
    /// Numeric value strictly greater than the operand.
    GreaterThan(f64),
    /// Numeric value strictly less than the operand.
    LessThan(f64),
    /// Numeric value within the inclusive range (bounds in either order).
    Between(f64, f64),
    /// Text cell equal to the operand, case-insensitively.
    EqualsText(String),
    /// Text cell containing the operand, case-insensitively.
    ContainsText(String),
}

/// Style applied by a matching [`ConditionalKind::Rule`]. Unset channels leave
/// the cell's default rendering untouched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConditionalStyle {
    /// Override the cell text color.
    pub foreground: Option<RgbaColor>,
    /// Fill the cell background.
    pub background: Option<RgbaColor>,
    /// Render the cell text in the bold face.
    pub bold: bool,
}

/// One conditional-formatting behavior for a column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConditionalKind {
    /// Apply `style` to cells matching `condition`. When several `Rule`s
    /// target the same column, the first matching one wins.
    Rule {
        /// Per-cell predicate.
        condition: ConditionalCondition,
        /// Style applied when the predicate matches.
        style: ConditionalStyle,
    },
    /// Numeric columns: fill each cell's background with `min`→`max` lerped
    /// (per RGBA channel) by where the value sits in the column's actual
    /// range. A column whose values are all equal paints the midpoint color.
    /// An explicit [`ConditionalStyle::background`] from a matching `Rule`
    /// wins over the scale color.
    ColorScale {
        /// Color painted at the column minimum.
        min: RgbaColor,
        /// Color painted at the column maximum.
        max: RgbaColor,
    },
    /// Numeric columns: paint a proportional bar from the cell's left edge,
    /// behind the text, of width `(value - min) / (max - min)` × the cell
    /// width (clamped to `0..=1`). A column whose values are all equal paints
    /// full bars.
    DataBar {
        /// Bar fill color.
        color: RgbaColor,
    },
}

/// A conditional-formatting rule targeting one column by name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConditionalRule {
    /// Column name, matched against [`Column::name`] case-insensitively. If
    /// several columns share the name, all of them receive the rule; a name
    /// matching no column is ignored.
    pub column: String,
    /// The behavior to apply.
    pub kind: ConditionalKind,
}

/// The visual effect [`ColumnConditionals::evaluate`] computed for one cell.
/// Unset channels leave the default rendering untouched.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellEffect {
    /// Text color override.
    pub fg: Option<Hsla>,
    /// Background fill (rule background or lerped color-scale color).
    pub bg: Option<Hsla>,
    /// Data-bar fill fraction of the cell width, `0.0..=1.0`.
    pub bar_fraction: Option<f32>,
    /// Data-bar fill color. `Some` exactly when `bar_fraction` is.
    pub bar_color: Option<Hsla>,
    /// Render the cell text in the bold face.
    pub bold: bool,
}

/// The rules resolved for one column, with the numeric statistics its color
/// scale / data bar need.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColumnConditionals {
    /// Rule behaviors targeting this column, in declaration order.
    pub kinds: Vec<ConditionalKind>,
    /// `(min, max)` over the column's finite numeric cells. `Some` only when
    /// a [`ConditionalKind::ColorScale`] or [`ConditionalKind::DataBar`]
    /// targets this column *and* at least one finite numeric value exists.
    pub stats: Option<(f64, f64)>,
}

impl ColumnConditionals {
    /// Evaluate this column's rules against one cell.
    ///
    /// Returns `None` when nothing applies. Composition:
    ///
    /// * The **first matching [`ConditionalKind::Rule`]** supplies
    ///   foreground / background / bold; later matching rules are ignored.
    /// * The first [`ConditionalKind::ColorScale`] supplies the background
    ///   for numeric cells — unless the matching rule already set an explicit
    ///   background, which wins regardless of declaration order.
    /// * The first [`ConditionalKind::DataBar`] supplies the bar fraction and
    ///   color for numeric cells.
    #[must_use]
    pub fn evaluate(&self, cell: &CellValue) -> Option<CellEffect> {
        if self.kinds.is_empty() {
            return None;
        }
        let mut effect = CellEffect::default();
        let mut any = false;
        let mut rule_done = false;
        let mut rule_bg = false;
        let mut scale_done = false;
        let mut bar_done = false;
        for kind in &self.kinds {
            match kind {
                ConditionalKind::Rule { condition, style } => {
                    if rule_done || !condition_matches(condition, cell) {
                        continue;
                    }
                    rule_done = true;
                    any = true;
                    effect.fg = style.foreground.map(RgbaColor::to_hsla);
                    if let Some(bg) = style.background {
                        effect.bg = Some(bg.to_hsla());
                        rule_bg = true;
                    }
                    effect.bold = style.bold;
                }
                ConditionalKind::ColorScale { min, max } => {
                    if scale_done {
                        continue;
                    }
                    let Some((lo, hi)) = self.stats else { continue };
                    let Some(v) = numeric_value(cell) else {
                        continue;
                    };
                    scale_done = true;
                    any = true;
                    if !rule_bg {
                        // All-equal columns paint the midpoint of the scale.
                        let t = if hi > lo {
                            ((v - lo) / (hi - lo)) as f32
                        } else {
                            0.5
                        };
                        effect.bg = Some(lerp_color(*min, *max, t.clamp(0.0, 1.0)).to_hsla());
                    }
                }
                ConditionalKind::DataBar { color } => {
                    if bar_done {
                        continue;
                    }
                    let Some((lo, hi)) = self.stats else { continue };
                    let Some(v) = numeric_value(cell) else {
                        continue;
                    };
                    bar_done = true;
                    any = true;
                    // All-equal columns paint full bars.
                    let f = if hi > lo {
                        ((v - lo) / (hi - lo)) as f32
                    } else {
                        1.0
                    };
                    effect.bar_fraction = Some(f.clamp(0.0, 1.0));
                    effect.bar_color = Some(color.to_hsla());
                }
            }
        }
        any.then_some(effect)
    }
}

/// Per-column conditional formatting, resolved from
/// [`crate::config::GridConfig::conditional_rules`] against a concrete column
/// list and row set. Rebuilt by the grid whenever the config, the columns, or
/// the resident rows change (append, row-window paging), so the min/max
/// statistics always describe the rows currently in memory.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedConditionals {
    /// One entry per column when any rule matched; empty when conditional
    /// formatting is entirely inactive (the paint fast path).
    columns: Vec<ColumnConditionals>,
}

impl ResolvedConditionals {
    /// Resolve `rules` against `columns`, precomputing numeric min/max over
    /// `rows` for every column targeted by a color scale or data bar.
    ///
    /// Column matching is case-insensitive; rules naming no column are
    /// ignored. When `rules` is empty — or nothing matches — the result
    /// [`is_empty`](Self::is_empty), which paint treats as "feature off".
    #[must_use]
    pub fn resolve(rules: &[ConditionalRule], columns: &[Column], rows: &[Vec<CellValue>]) -> Self {
        if rules.is_empty() {
            return Self::default();
        }
        let mut cols: Vec<ColumnConditionals> = columns
            .iter()
            .map(|_| ColumnConditionals::default())
            .collect();
        for rule in rules {
            let target = rule.column.to_lowercase();
            for (ci, col) in columns.iter().enumerate() {
                if col.name.to_lowercase() == target {
                    cols[ci].kinds.push(rule.kind.clone());
                }
            }
        }
        if cols.iter().all(|c| c.kinds.is_empty()) {
            return Self::default();
        }
        for (ci, cc) in cols.iter_mut().enumerate() {
            let needs_stats = cc.kinds.iter().any(|k| {
                matches!(
                    k,
                    ConditionalKind::ColorScale { .. } | ConditionalKind::DataBar { .. }
                )
            });
            if !needs_stats {
                continue;
            }
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            for row in rows {
                if let Some(v) = row.get(ci).and_then(numeric_value) {
                    min = min.min(v);
                    max = max.max(v);
                }
            }
            if min <= max {
                cc.stats = Some((min, max));
            }
        }
        Self { columns: cols }
    }

    /// `true` when conditional formatting is entirely inactive. Paint gates
    /// all per-cell work behind this single check.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// The resolved conditionals for column `index`, if any rule targets it.
    #[must_use]
    pub fn column(&self, index: usize) -> Option<&ColumnConditionals> {
        self.columns.get(index).filter(|c| !c.kinds.is_empty())
    }
}

/// The numeric reading of a cell for conditions, scales, and bars: typed
/// [`CellValue::Integer`] / finite [`CellValue::Decimal`] only. Text is never
/// parsed; dates and booleans do not participate.
fn numeric_value(cell: &CellValue) -> Option<f64> {
    match cell {
        CellValue::Integer(v) => Some(*v as f64),
        CellValue::Decimal(v) if v.is_finite() => Some(*v),
        _ => None,
    }
}

fn condition_matches(condition: &ConditionalCondition, cell: &CellValue) -> bool {
    match condition {
        ConditionalCondition::IsNull => matches!(cell, CellValue::None),
        ConditionalCondition::IsNotNull => !matches!(cell, CellValue::None),
        ConditionalCondition::GreaterThan(t) => numeric_value(cell).is_some_and(|v| v > *t),
        ConditionalCondition::LessThan(t) => numeric_value(cell).is_some_and(|v| v < *t),
        ConditionalCondition::Between(a, b) => numeric_value(cell).is_some_and(|v| {
            let (lo, hi) = (a.min(*b), a.max(*b));
            v >= lo && v <= hi
        }),
        ConditionalCondition::EqualsText(s) => {
            matches!(cell, CellValue::Text(t) if t.to_lowercase() == s.to_lowercase())
        }
        ConditionalCondition::ContainsText(s) => {
            matches!(cell, CellValue::Text(t) if t.to_lowercase().contains(&s.to_lowercase()))
        }
    }
}

fn lerp_color(min: RgbaColor, max: RgbaColor, t: f32) -> RgbaColor {
    let ch = |a: f32, b: f32| a + (b - a) * t;
    RgbaColor {
        r: ch(min.r, max.r),
        g: ch(min.g, max.g),
        b: ch(min.b, max.b),
        a: ch(min.a, max.a),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::data::ColumnKind;

    const RED: RgbaColor = RgbaColor::new(1.0, 0.0, 0.0, 1.0);
    const GREEN: RgbaColor = RgbaColor::new(0.0, 1.0, 0.0, 1.0);
    const BLUE: RgbaColor = RgbaColor::new(0.0, 0.0, 1.0, 1.0);

    fn cols(names: &[&str]) -> Vec<Column> {
        names
            .iter()
            .map(|n| Column::new(*n, ColumnKind::Decimal, 80.0))
            .collect()
    }

    fn rule(
        column: &str,
        condition: ConditionalCondition,
        style: ConditionalStyle,
    ) -> ConditionalRule {
        ConditionalRule {
            column: column.into(),
            kind: ConditionalKind::Rule { condition, style },
        }
    }

    fn bg_style(color: RgbaColor) -> ConditionalStyle {
        ConditionalStyle {
            background: Some(color),
            ..Default::default()
        }
    }

    fn resolve_one(rules: Vec<ConditionalRule>, rows: Vec<Vec<CellValue>>) -> ResolvedConditionals {
        ResolvedConditionals::resolve(&rules, &cols(&["amount"]), &rows)
    }

    #[test]
    fn resolve_matches_column_name_case_insensitively() {
        let resolved = ResolvedConditionals::resolve(
            &[rule("AMOUNT", ConditionalCondition::IsNull, bg_style(RED))],
            &cols(&["id", "Amount"]),
            &[],
        );
        assert!(!resolved.is_empty());
        assert!(resolved.column(0).is_none());
        assert_eq!(resolved.column(1).unwrap().kinds.len(), 1);
    }

    #[test]
    fn resolve_ignores_unknown_column() {
        let resolved = ResolvedConditionals::resolve(
            &[rule("missing", ConditionalCondition::IsNull, bg_style(RED))],
            &cols(&["amount"]),
            &[],
        );
        // Nothing matched: fully inactive, so paint takes the fast path.
        assert!(resolved.is_empty());
        assert!(resolved.column(0).is_none());
    }

    #[test]
    fn resolve_with_no_rules_is_empty() {
        let resolved = ResolvedConditionals::resolve(&[], &cols(&["amount"]), &[]);
        assert!(resolved.is_empty());
    }

    #[test]
    fn is_null_and_is_not_null_conditions() {
        let resolved = resolve_one(
            vec![rule("amount", ConditionalCondition::IsNull, bg_style(RED))],
            vec![],
        );
        let cc = resolved.column(0).unwrap();
        assert!(cc.evaluate(&CellValue::None).is_some());
        assert!(cc.evaluate(&CellValue::Integer(1)).is_none());

        let resolved = resolve_one(
            vec![rule(
                "amount",
                ConditionalCondition::IsNotNull,
                bg_style(RED),
            )],
            vec![],
        );
        let cc = resolved.column(0).unwrap();
        assert!(cc.evaluate(&CellValue::None).is_none());
        assert!(cc.evaluate(&CellValue::Text("x".into())).is_some());
        assert!(cc.evaluate(&CellValue::Integer(1)).is_some());
    }

    #[test]
    fn numeric_conditions_match_typed_numbers_only() {
        let resolved = resolve_one(
            vec![rule(
                "amount",
                ConditionalCondition::GreaterThan(10.0),
                bg_style(RED),
            )],
            vec![],
        );
        let cc = resolved.column(0).unwrap();
        assert!(cc.evaluate(&CellValue::Integer(11)).is_some());
        assert!(cc.evaluate(&CellValue::Decimal(10.5)).is_some());
        assert!(cc.evaluate(&CellValue::Integer(10)).is_none(), "strict >");
        // Numeric-looking text is deliberately NOT parsed.
        assert!(cc.evaluate(&CellValue::Text("99".into())).is_none());
        // Non-finite decimals never match numeric conditions.
        assert!(cc.evaluate(&CellValue::Decimal(f64::NAN)).is_none());
    }

    #[test]
    fn less_than_condition() {
        let resolved = resolve_one(
            vec![rule(
                "amount",
                ConditionalCondition::LessThan(0.0),
                bg_style(RED),
            )],
            vec![],
        );
        let cc = resolved.column(0).unwrap();
        assert!(cc.evaluate(&CellValue::Decimal(-0.1)).is_some());
        assert!(cc.evaluate(&CellValue::Decimal(0.0)).is_none(), "strict <");
        assert!(cc.evaluate(&CellValue::Integer(3)).is_none());
    }

    #[test]
    fn between_condition_is_inclusive_and_normalizes_reversed_bounds() {
        for bounds in [(1.0, 5.0), (5.0, 1.0)] {
            let resolved = resolve_one(
                vec![rule(
                    "amount",
                    ConditionalCondition::Between(bounds.0, bounds.1),
                    bg_style(RED),
                )],
                vec![],
            );
            let cc = resolved.column(0).unwrap();
            assert!(cc.evaluate(&CellValue::Integer(1)).is_some());
            assert!(cc.evaluate(&CellValue::Integer(3)).is_some());
            assert!(cc.evaluate(&CellValue::Integer(5)).is_some());
            assert!(cc.evaluate(&CellValue::Integer(0)).is_none());
            assert!(cc.evaluate(&CellValue::Integer(6)).is_none());
        }
    }

    #[test]
    fn text_conditions_match_case_insensitively_on_text_cells_only() {
        let resolved = resolve_one(
            vec![rule(
                "amount",
                ConditionalCondition::EqualsText("Error".into()),
                bg_style(RED),
            )],
            vec![],
        );
        let cc = resolved.column(0).unwrap();
        assert!(cc.evaluate(&CellValue::Text("ERROR".into())).is_some());
        assert!(cc.evaluate(&CellValue::Text("error!".into())).is_none());
        assert!(cc.evaluate(&CellValue::Integer(1)).is_none());

        let resolved = resolve_one(
            vec![rule(
                "amount",
                ConditionalCondition::ContainsText("err".into()),
                bg_style(RED),
            )],
            vec![],
        );
        let cc = resolved.column(0).unwrap();
        assert!(cc
            .evaluate(&CellValue::Text("An ERRor here".into()))
            .is_some());
        assert!(cc.evaluate(&CellValue::Text("fine".into())).is_none());
        // A boolean is not a text cell even though it formats to text.
        assert!(cc.evaluate(&CellValue::Boolean(true)).is_none());
    }

    #[test]
    fn first_matching_rule_wins() {
        let resolved = resolve_one(
            vec![
                rule(
                    "amount",
                    ConditionalCondition::GreaterThan(0.0),
                    bg_style(RED),
                ),
                rule(
                    "amount",
                    ConditionalCondition::GreaterThan(10.0),
                    bg_style(GREEN),
                ),
            ],
            vec![],
        );
        let cc = resolved.column(0).unwrap();
        // Matches both; the first declared rule supplies the style.
        let effect = cc.evaluate(&CellValue::Integer(100)).unwrap();
        assert_eq!(effect.bg, Some(RED.to_hsla()));
        // Matches only the first.
        let effect = cc.evaluate(&CellValue::Integer(5)).unwrap();
        assert_eq!(effect.bg, Some(RED.to_hsla()));
    }

    #[test]
    fn rule_style_channels_apply() {
        let resolved = resolve_one(
            vec![rule(
                "amount",
                ConditionalCondition::IsNotNull,
                ConditionalStyle {
                    foreground: Some(BLUE),
                    background: Some(RED),
                    bold: true,
                },
            )],
            vec![],
        );
        let effect = resolved
            .column(0)
            .unwrap()
            .evaluate(&CellValue::Integer(1))
            .unwrap();
        assert_eq!(effect.fg, Some(BLUE.to_hsla()));
        assert_eq!(effect.bg, Some(RED.to_hsla()));
        assert!(effect.bold);
        assert_eq!(effect.bar_fraction, None);
    }

    #[test]
    fn stats_precomputed_over_typed_numeric_cells_only() {
        let rows = vec![
            vec![CellValue::Integer(10)],
            vec![CellValue::Decimal(-2.5)],
            vec![CellValue::Text("999".into())], // never parsed
            vec![CellValue::Decimal(f64::NAN)],  // non-finite excluded
            vec![CellValue::Decimal(f64::INFINITY)], // non-finite excluded
            vec![CellValue::None],
        ];
        let resolved = resolve_one(
            vec![ConditionalRule {
                column: "amount".into(),
                kind: ConditionalKind::DataBar { color: BLUE },
            }],
            rows,
        );
        assert_eq!(resolved.column(0).unwrap().stats, Some((-2.5, 10.0)));
    }

    #[test]
    fn stats_absent_without_scale_or_bar_and_without_numeric_values() {
        // A plain rule never pays for statistics.
        let resolved = resolve_one(
            vec![rule("amount", ConditionalCondition::IsNull, bg_style(RED))],
            vec![vec![CellValue::Integer(1)]],
        );
        assert_eq!(resolved.column(0).unwrap().stats, None);
        // A scale over a column with no finite numeric values gets no stats
        // and paints nothing.
        let resolved = resolve_one(
            vec![ConditionalRule {
                column: "amount".into(),
                kind: ConditionalKind::ColorScale {
                    min: RED,
                    max: GREEN,
                },
            }],
            vec![vec![CellValue::Text("x".into())], vec![CellValue::None]],
        );
        let cc = resolved.column(0).unwrap();
        assert_eq!(cc.stats, None);
        assert!(cc.evaluate(&CellValue::Integer(5)).is_none());
    }

    #[test]
    fn color_scale_lerps_over_column_range() {
        let rows = vec![
            vec![CellValue::Decimal(0.0)],
            vec![CellValue::Decimal(10.0)],
        ];
        let resolved = resolve_one(
            vec![ConditionalRule {
                column: "amount".into(),
                kind: ConditionalKind::ColorScale {
                    min: RgbaColor::new(0.0, 0.0, 0.0, 1.0),
                    max: RgbaColor::new(1.0, 1.0, 1.0, 1.0),
                },
            }],
            rows,
        );
        let cc = resolved.column(0).unwrap();
        let at = |v: f64| cc.evaluate(&CellValue::Decimal(v)).unwrap().bg.unwrap();
        assert_eq!(at(0.0), RgbaColor::new(0.0, 0.0, 0.0, 1.0).to_hsla());
        assert_eq!(at(10.0), RgbaColor::new(1.0, 1.0, 1.0, 1.0).to_hsla());
        assert_eq!(at(5.0), RgbaColor::new(0.5, 0.5, 0.5, 1.0).to_hsla());
        // Values outside the precomputed range clamp to the endpoints.
        assert_eq!(at(-100.0), at(0.0));
        assert_eq!(at(100.0), at(10.0));
        // Non-finite and non-numeric cells paint no scale color.
        assert!(cc.evaluate(&CellValue::Decimal(f64::NAN)).is_none());
        assert!(cc.evaluate(&CellValue::Text("5".into())).is_none());
    }

    #[test]
    fn color_scale_with_equal_min_and_max_paints_midpoint() {
        let rows = vec![vec![CellValue::Integer(7)], vec![CellValue::Integer(7)]];
        let resolved = resolve_one(
            vec![ConditionalRule {
                column: "amount".into(),
                kind: ConditionalKind::ColorScale {
                    min: RgbaColor::new(0.0, 0.0, 0.0, 1.0),
                    max: RgbaColor::new(1.0, 1.0, 1.0, 1.0),
                },
            }],
            rows,
        );
        let effect = resolved
            .column(0)
            .unwrap()
            .evaluate(&CellValue::Integer(7))
            .unwrap();
        assert_eq!(
            effect.bg,
            Some(RgbaColor::new(0.5, 0.5, 0.5, 1.0).to_hsla())
        );
    }

    #[test]
    fn data_bar_fraction_spans_range_and_clamps() {
        let rows = vec![vec![CellValue::Integer(-10)], vec![CellValue::Integer(30)]];
        let resolved = resolve_one(
            vec![ConditionalRule {
                column: "amount".into(),
                kind: ConditionalKind::DataBar { color: BLUE },
            }],
            rows,
        );
        let cc = resolved.column(0).unwrap();
        let frac = |v: i64| {
            cc.evaluate(&CellValue::Integer(v))
                .unwrap()
                .bar_fraction
                .unwrap()
        };
        assert_eq!(frac(-10), 0.0);
        assert_eq!(frac(30), 1.0);
        assert_eq!(frac(10), 0.5);
        // Outside the precomputed range: clamped to 0..=1.
        assert_eq!(frac(-999), 0.0);
        assert_eq!(frac(999), 1.0);
        let effect = cc.evaluate(&CellValue::Integer(10)).unwrap();
        assert_eq!(effect.bar_color, Some(BLUE.to_hsla()));
    }

    #[test]
    fn data_bar_with_equal_min_and_max_paints_full_bars() {
        let rows = vec![vec![CellValue::Integer(4)], vec![CellValue::Integer(4)]];
        let resolved = resolve_one(
            vec![ConditionalRule {
                column: "amount".into(),
                kind: ConditionalKind::DataBar { color: BLUE },
            }],
            rows,
        );
        let effect = resolved
            .column(0)
            .unwrap()
            .evaluate(&CellValue::Integer(4))
            .unwrap();
        assert_eq!(effect.bar_fraction, Some(1.0));
    }

    #[test]
    fn scale_and_bar_compose_with_rule_fg_and_bold() {
        let rows = vec![vec![CellValue::Integer(0)], vec![CellValue::Integer(10)]];
        let resolved = resolve_one(
            vec![
                rule(
                    "amount",
                    ConditionalCondition::GreaterThan(5.0),
                    ConditionalStyle {
                        foreground: Some(RED),
                        background: None,
                        bold: true,
                    },
                ),
                ConditionalRule {
                    column: "amount".into(),
                    kind: ConditionalKind::ColorScale {
                        min: GREEN,
                        max: BLUE,
                    },
                },
                ConditionalRule {
                    column: "amount".into(),
                    kind: ConditionalKind::DataBar { color: BLUE },
                },
            ],
            rows,
        );
        let effect = resolved
            .column(0)
            .unwrap()
            .evaluate(&CellValue::Integer(10))
            .unwrap();
        assert_eq!(effect.fg, Some(RED.to_hsla()), "rule fg composes");
        assert!(effect.bold, "rule bold composes");
        assert_eq!(effect.bg, Some(BLUE.to_hsla()), "scale supplies bg");
        assert_eq!(effect.bar_fraction, Some(1.0));
    }

    #[test]
    fn explicit_rule_background_wins_over_color_scale() {
        let rows = vec![vec![CellValue::Integer(0)], vec![CellValue::Integer(10)]];
        for reorder in [false, true] {
            let mut rules = vec![
                rule(
                    "amount",
                    ConditionalCondition::GreaterThan(5.0),
                    bg_style(RED),
                ),
                ConditionalRule {
                    column: "amount".into(),
                    kind: ConditionalKind::ColorScale {
                        min: GREEN,
                        max: BLUE,
                    },
                },
            ];
            if reorder {
                rules.reverse();
            }
            let resolved = resolve_one(rules, rows.clone());
            let effect = resolved
                .column(0)
                .unwrap()
                .evaluate(&CellValue::Integer(10))
                .unwrap();
            assert_eq!(effect.bg, Some(RED.to_hsla()), "reorder={reorder}");
        }
    }

    #[test]
    fn color_lerp_is_channelwise() {
        let a = RgbaColor::new(0.0, 1.0, 0.2, 0.0);
        let b = RgbaColor::new(1.0, 0.0, 0.2, 1.0);
        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 1.0), b);
        assert_eq!(lerp_color(a, b, 0.5), RgbaColor::new(0.5, 0.5, 0.2, 0.5));
    }

    #[test]
    fn serde_round_trips_rules() {
        let rules = vec![
            rule(
                "amount",
                ConditionalCondition::Between(1.0, 5.0),
                ConditionalStyle {
                    foreground: Some(RED),
                    background: Some(GREEN),
                    bold: true,
                },
            ),
            rule(
                "note",
                ConditionalCondition::ContainsText("err".into()),
                ConditionalStyle::default(),
            ),
            rule("id", ConditionalCondition::IsNull, bg_style(BLUE)),
            ConditionalRule {
                column: "score".into(),
                kind: ConditionalKind::ColorScale {
                    min: RED,
                    max: GREEN,
                },
            },
            ConditionalRule {
                column: "score".into(),
                kind: ConditionalKind::DataBar { color: BLUE },
            },
        ];
        let json = serde_json::to_string(&rules).expect("serialize");
        let back: Vec<ConditionalRule> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, rules);
    }
}
