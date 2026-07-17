//! Pure cell formatting: numbers, dates, strings, booleans, filter matching.
//!
//! All functions here are intentionally GPUI-free so they can be reused for
//! exports, server-side previews, and tests. The widget layer calls
//! [`format_cell`] on every visible cell during paint and on every
//! clipboard-copy; tests below document the public behavior.

use crate::config::{
    DateFormat, NumberFormat, RelativeDateFormat, RelativeUnit, ReplacementRule, ReplacementTiming,
    ResolvedColumnFormat, StringFormat, TextAlignment, TextCase, TruncationBehavior,
};
use crate::data::{CellValue, ColumnKind};

// `std::time::SystemTime::now()` is unimplemented on wasm32-unknown-unknown
// (it panics), so the web build reads the clock through `web-time`, which has
// an identical API backed by the JS `Date`.
#[cfg(not(target_family = "wasm"))]
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(target_family = "wasm")]
use web_time::{SystemTime, UNIX_EPOCH};

/// Format any cell into the user-visible text plus a "is negative" flag that
/// lets paint code color it red without re-parsing the text.
#[must_use]
pub fn format_cell(value: &CellValue, fmt: &ResolvedColumnFormat) -> (String, bool) {
    let (text, is_neg) = match (value, &fmt.kind) {
        (CellValue::Text(s), ColumnKind::Text) => {
            let s = if fmt.replacement_timing == ReplacementTiming::BeforeFormat {
                apply_replacements(s, &fmt.replacements)
            } else {
                s.clone()
            };
            (format_string(&s, &fmt.string), false)
        }
        (CellValue::Integer(v), ColumnKind::Integer) => (format_integer(*v, &fmt.number), *v < 0),
        (CellValue::Decimal(v), ColumnKind::Decimal) => (format_number(*v, &fmt.number), *v < 0.0),
        (CellValue::Integer(v), ColumnKind::Decimal) => {
            (format_integer_as_decimal(*v, &fmt.number), *v < 0)
        }
        (CellValue::Decimal(v), ColumnKind::Integer) => (format_number(*v, &fmt.number), *v < 0.0),
        (CellValue::Date(ts), ColumnKind::Date) => (format_date(*ts, &fmt.date), false),
        (CellValue::Boolean(b), ColumnKind::Boolean) => (format_boolean(*b, &fmt.boolean), false),
        (CellValue::None, _) => (String::new(), false),
        (CellValue::Text(s), _) => (s.clone(), false),
        (CellValue::Integer(v), _) => (v.to_string(), *v < 0),
        (CellValue::Decimal(v), _) => (v.to_string(), *v < 0.0),
        (CellValue::Date(ts), _) => (format_date(*ts, &fmt.date), false),
        (CellValue::Boolean(b), _) => (format_boolean(*b, &fmt.boolean), false),
    };

    let text = if fmt.replacement_timing == ReplacementTiming::AfterFormat {
        apply_replacements(&text, &fmt.replacements)
    } else {
        text
    };

    (text, is_neg)
}

/// Format a `CellValue::Integer` against a [`NumberFormat`] without first
/// casting through `f64`. This preserves full `i64` precision for values
/// larger than `2^53`.
#[must_use]
pub fn format_integer(value: i64, fmt: &NumberFormat) -> String {
    if fmt.decimals == 0 {
        let raw = value.unsigned_abs().to_string();
        let with_sep = if fmt.thousands_separator {
            add_thousands_separator(&raw)
        } else {
            raw
        };
        if value < 0 {
            if fmt.negative_parentheses {
                format!("({with_sep})")
            } else {
                format!("-{with_sep}")
            }
        } else {
            with_sep
        }
    } else {
        // Decimals require a fractional part; route through `format_number`.
        // We accept the f64 round-trip because the user explicitly asked for
        // fractional display.
        format_number(value as f64, fmt)
    }
}

fn format_integer_as_decimal(value: i64, fmt: &NumberFormat) -> String {
    if fmt.decimals == 0 {
        format_integer(value, fmt)
    } else {
        format_number(value as f64, fmt)
    }
}

/// Format a `f64` against a [`NumberFormat`]. Negative formatting
/// (parentheses vs leading minus) and thousands separators are driven by the
/// format options.
#[must_use]
pub fn format_number(value: f64, fmt: &NumberFormat) -> String {
    let abs = value.abs();
    let num_str = format!("{abs:.*}", fmt.decimals);
    let with_sep = if fmt.thousands_separator {
        add_thousands_separator(&num_str)
    } else {
        num_str
    };
    if value < 0.0 {
        if fmt.negative_parentheses {
            format!("({with_sep})")
        } else {
            format!("-{with_sep}")
        }
    } else {
        with_sep
    }
}

fn add_thousands_separator(s: &str) -> String {
    let (int_part, dec_part) = match s.split_once('.') {
        Some((i, d)) => (i, format!(".{d}")),
        None => (s, String::new()),
    };
    let chars: Vec<char> = int_part.chars().collect();
    let mut result = String::new();
    let len = chars.len();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(*c);
    }
    format!("{result}{dec_part}")
}

/// Format a Unix timestamp (seconds). When `fmt.relative` is set, the result
/// is a "2 days ago" / "in 3 weeks" string relative to `SystemTime::now()`;
/// use [`format_date_at`] to inject a frozen clock for tests.
#[must_use]
pub fn format_date(ts: i64, fmt: &DateFormat) -> String {
    let now = current_unix_seconds();
    format_date_at(ts, now, fmt)
}

/// Same as [`format_date`] but with an explicit `now` timestamp so tests can
/// pin the relative-date output to a known clock.
#[must_use]
pub fn format_date_at(ts: i64, now: i64, fmt: &DateFormat) -> String {
    let adjusted_ts = ts + i64::from(fmt.timezone_offset_minutes) * 60;
    if let Some(relative) = &fmt.relative {
        let adjusted_now = now + i64::from(fmt.timezone_offset_minutes) * 60;
        return format_relative_date(adjusted_ts, adjusted_now, relative);
    }
    format_date_str(adjusted_ts, &fmt.format)
}

fn current_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn format_date_str(ts: i64, format: &str) -> String {
    let (year, month, day, hour, min, sec) = timestamp_to_components(ts);
    format
        .replace("%Y", &format!("{year:04}"))
        .replace("%m", &format!("{month:02}"))
        .replace("%d", &format!("{day:02}"))
        .replace("%H", &format!("{hour:02}"))
        .replace("%M", &format!("{min:02}"))
        .replace("%S", &format!("{sec:02}"))
        .replace("%y", &format!("{:02}", year.rem_euclid(100)))
        .replace("%B", &month_name(month))
        .replace("%b", &month_name(month)[..3.min(month_name(month).len())])
        .replace("%A", &day_name(ts))
        .replace("%a", &day_name(ts)[..3.min(day_name(ts).len())])
}

#[must_use]
pub fn format_relative_date(ts: i64, now: i64, relative: &RelativeDateFormat) -> String {
    let diff = ts - now;
    if diff == 0 {
        return "now".into();
    }
    let abs_diff = diff.unsigned_abs();
    let components = break_down_duration(abs_diff, &relative.units);
    let parts: Vec<String> = components
        .iter()
        .take(relative.max_components)
        .map(|(unit, count)| format!("{} {}", count, unit_name(unit, *count)))
        .collect();
    if parts.is_empty() {
        return "now".into();
    }
    let joined = parts.join(" and ");
    if diff > 0 {
        format!("in {joined}")
    } else {
        format!("{joined} ago")
    }
}

fn break_down_duration(seconds: u64, units: &[RelativeUnit]) -> Vec<(RelativeUnit, u64)> {
    let mut remaining = seconds;
    let mut result = vec![];
    let ordered = order_units_desc(units);
    for unit in ordered {
        let size = unit_seconds(unit);
        if size > 0 && remaining >= size {
            let count = remaining / size;
            remaining %= size;
            result.push((unit, count));
        }
    }
    result
}

fn order_units_desc(units: &[RelativeUnit]) -> Vec<RelativeUnit> {
    let all = [
        RelativeUnit::Year,
        RelativeUnit::Month,
        RelativeUnit::Week,
        RelativeUnit::Day,
        RelativeUnit::Hour,
        RelativeUnit::Minute,
        RelativeUnit::Second,
    ];
    all.iter().copied().filter(|u| units.contains(u)).collect()
}

fn unit_seconds(unit: RelativeUnit) -> u64 {
    match unit {
        RelativeUnit::Year => 31_557_600,
        RelativeUnit::Month => 2_630_016,
        RelativeUnit::Week => 604_800,
        RelativeUnit::Day => 86_400,
        RelativeUnit::Hour => 3_600,
        RelativeUnit::Minute => 60,
        RelativeUnit::Second => 1,
    }
}

fn unit_name(unit: &RelativeUnit, count: u64) -> &'static str {
    match unit {
        RelativeUnit::Year => {
            if count == 1 {
                "year"
            } else {
                "years"
            }
        }
        RelativeUnit::Month => {
            if count == 1 {
                "month"
            } else {
                "months"
            }
        }
        RelativeUnit::Week => {
            if count == 1 {
                "week"
            } else {
                "weeks"
            }
        }
        RelativeUnit::Day => {
            if count == 1 {
                "day"
            } else {
                "days"
            }
        }
        RelativeUnit::Hour => {
            if count == 1 {
                "hour"
            } else {
                "hours"
            }
        }
        RelativeUnit::Minute => {
            if count == 1 {
                "minute"
            } else {
                "minutes"
            }
        }
        RelativeUnit::Second => {
            if count == 1 {
                "second"
            } else {
                "seconds"
            }
        }
    }
}

fn format_boolean(b: bool, fmt: &crate::config::BooleanFormat) -> String {
    if b {
        fmt.true_text.clone()
    } else {
        fmt.false_text.clone()
    }
}

/// Format text according to a [`StringFormat`]: case, length, truncation.
#[must_use]
pub fn format_string(s: &str, fmt: &StringFormat) -> String {
    let cased = match fmt.case {
        TextCase::Upper => s.to_uppercase(),
        TextCase::Lower => s.to_lowercase(),
        TextCase::Title => title_case(s),
        TextCase::None => s.to_owned(),
    };
    match fmt.max_length {
        Some(max) if cased.chars().count() > max => truncate_chars(&cased, max, fmt.truncation),
        _ => cased,
    }
}

fn truncate_chars(s: &str, max: usize, mode: TruncationBehavior) -> String {
    let truncated: String = s.chars().take(max).collect();
    match mode {
        TruncationBehavior::Ellipsis if max >= 3 => {
            let mut t: String = s.chars().take(max - 3).collect();
            t.push_str("...");
            t
        }
        TruncationBehavior::Ellipsis => truncated,
        TruncationBehavior::CutOff | TruncationBehavior::Wrap => truncated,
    }
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn apply_replacements(s: &str, rules: &[ReplacementRule]) -> String {
    let mut result = s.to_owned();
    for rule in rules {
        result = result.replace(&rule.find, &rule.replace);
    }
    result
}

fn timestamp_to_components(ts: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400) as u32;
    let hour = secs / 3600;
    let min = (secs % 3600) / 60;
    let sec = secs % 60;
    let (year, month, day) = days_to_ymd(days);
    (year, month, day, hour, min, sec)
}

fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + (era as i32) * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

fn month_name(m: u32) -> String {
    match m {
        1 => "January".into(),
        2 => "February".into(),
        3 => "March".into(),
        4 => "April".into(),
        5 => "May".into(),
        6 => "June".into(),
        7 => "July".into(),
        8 => "August".into(),
        9 => "September".into(),
        10 => "October".into(),
        11 => "November".into(),
        12 => "December".into(),
        _ => "Unknown".into(),
    }
}

fn day_name(ts: i64) -> String {
    let day_of_week = (ts.div_euclid(86_400) + 4).rem_euclid(7) as u32;
    match day_of_week {
        0 => "Sunday".into(),
        1 => "Monday".into(),
        2 => "Tuesday".into(),
        3 => "Wednesday".into(),
        4 => "Thursday".into(),
        5 => "Friday".into(),
        6 => "Saturday".into(),
        _ => "Unknown".into(),
    }
}

/// Case-insensitive substring filter against the user-visible rendered text.
/// Empty filter always matches.
#[must_use]
pub fn cell_matches_filter(value: &CellValue, fmt: &ResolvedColumnFormat, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let (formatted, _) = format_cell(value, fmt);
    formatted.to_lowercase().contains(&filter.to_lowercase())
}

#[must_use]
pub fn alignment_for(fmt: &ResolvedColumnFormat) -> TextAlignment {
    fmt.alignment()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BooleanFormat, StringFormat};
    use crate::data::{Column, ColumnKind};
    use std::cell::Cell;

    fn plain_resolved(kind: ColumnKind) -> ResolvedColumnFormat {
        ResolvedColumnFormat {
            kind,
            number: NumberFormat::default(),
            date: DateFormat::default(),
            boolean: BooleanFormat::default(),
            string: StringFormat::default(),
            null: crate::config::NullFormat::default(),
            replacements: vec![],
            replacement_timing: ReplacementTiming::AfterFormat,
        }
    }

    #[test]
    fn format_integer_preserves_precision_above_2_pow_53() {
        // Value that loses exactness through f64; format_integer must keep it.
        let big = 9_007_199_254_740_993_i64;
        let fmt = NumberFormat {
            decimals: 0,
            thousands_separator: false,
            ..NumberFormat::default()
        };
        let s = format_integer(big, &fmt);
        assert_eq!(s, "9007199254740993");
    }

    #[test]
    fn format_integer_with_separators() {
        let fmt = NumberFormat {
            decimals: 0,
            thousands_separator: true,
            ..NumberFormat::default()
        };
        assert_eq!(format_integer(1_234_567, &fmt), "1,234,567");
        assert_eq!(format_integer(-1_234_567, &fmt), "-1,234,567");
    }

    #[test]
    fn format_integer_with_parentheses() {
        let fmt = NumberFormat {
            decimals: 0,
            negative_parentheses: true,
            ..NumberFormat::default()
        };
        assert_eq!(format_integer(-42, &fmt), "(42)");
    }

    /// Color-blind safety invariant: a negative value must always carry a
    /// non-color sign channel in its text (leading `-` or `( … )`), whether or
    /// not the decorative red fill is enabled. A red/green-color-blind reader
    /// relies entirely on this glyph, so it can never be gated behind
    /// `show_negative_red`. Guards WCAG 1.4.1 (use of color) for every
    /// negative-styling combination, across integer and decimal paths.
    #[test]
    fn negatives_always_carry_a_non_color_sign_channel() {
        for red in [false, true] {
            for parens in [false, true] {
                let fmt = NumberFormat {
                    decimals: 2,
                    show_negative_red: red,
                    negative_parentheses: parens,
                    ..NumberFormat::default()
                };
                let signed =
                    |s: &str| s.starts_with('-') || (s.starts_with('(') && s.ends_with(')'));
                for dec in [format_number(-1_493.17, &fmt), format_number(-0.01, &fmt)] {
                    assert!(
                        signed(&dec),
                        "decimal negative lacks sign channel: {dec:?} (red={red}, parens={parens})"
                    );
                }
                let int = format_integer(
                    -42,
                    &NumberFormat {
                        decimals: 0,
                        show_negative_red: red,
                        negative_parentheses: parens,
                        ..NumberFormat::default()
                    },
                );
                assert!(
                    signed(&int),
                    "integer negative lacks sign channel: {int:?} (red={red}, parens={parens})"
                );
            }
        }
    }

    #[test]
    fn format_number_negative_zero_path_does_not_panic() {
        let fmt = NumberFormat::default();
        assert_eq!(format_number(-0.0, &fmt), "0.00");
    }

    /// Hostile numeric input must never panic the formatter — `NaN` and the
    /// infinities flow straight from SQL sources (0.0/0.0 aggregates, etc.).
    #[test]
    fn format_number_nan_and_infinity_do_not_panic() {
        for sep in [false, true] {
            for parens in [false, true] {
                let fmt = NumberFormat {
                    decimals: 2,
                    thousands_separator: sep,
                    negative_parentheses: parens,
                    ..NumberFormat::default()
                };
                assert_eq!(format_number(f64::NAN, &fmt), "NaN");
                assert_eq!(format_number(f64::INFINITY, &fmt), "inf");
                let neg_inf = format_number(f64::NEG_INFINITY, &fmt);
                assert!(
                    neg_inf == "-inf" || neg_inf == "(inf)",
                    "negative infinity renders with its sign channel: {neg_inf}"
                );
            }
        }
    }

    #[test]
    fn format_number_thousands_separator_with_decimals() {
        let fmt = NumberFormat {
            decimals: 2,
            thousands_separator: true,
            ..NumberFormat::default()
        };
        assert_eq!(format_number(1_234_567.89, &fmt), "1,234,567.89");
    }

    #[test]
    fn format_string_truncates_on_chars_not_bytes() {
        // Emoji is 4 bytes but 1 char. Truncation at char boundary must not panic.
        let fmt = StringFormat {
            max_length: Some(3),
            truncation: TruncationBehavior::Ellipsis,
            ..StringFormat::default()
        };
        // Six emoji => 6 chars; truncation must keep the budget.
        let out = format_string(
            "\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}",
            &fmt,
        );
        assert_eq!(out, "...");
        assert_eq!(out.chars().count(), 3);

        // Longer budget keeps content + ellipsis.
        let fmt = StringFormat {
            max_length: Some(5),
            truncation: TruncationBehavior::Ellipsis,
            ..StringFormat::default()
        };
        let out = format_string(
            "\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}\u{1F600}",
            &fmt,
        );
        assert_eq!(out, "\u{1F600}\u{1F600}...");
    }

    #[test]
    fn format_string_truncation_modes() {
        let cases = [
            (TruncationBehavior::Ellipsis, "ab..."),
            (TruncationBehavior::CutOff, "abcde"),
            (TruncationBehavior::Wrap, "abcde"),
        ];
        for (mode, expected) in cases {
            let fmt = StringFormat {
                max_length: Some(5),
                truncation: mode,
                ..StringFormat::default()
            };
            assert_eq!(format_string("abcdefgh", &fmt), expected);
        }
    }

    #[test]
    fn format_string_case() {
        let fmt = StringFormat {
            case: TextCase::Upper,
            ..StringFormat::default()
        };
        assert_eq!(format_string("hello", &fmt), "HELLO");
        let fmt = StringFormat {
            case: TextCase::Lower,
            ..StringFormat::default()
        };
        assert_eq!(format_string("HELLO", &fmt), "hello");
        let fmt = StringFormat {
            case: TextCase::Title,
            ..StringFormat::default()
        };
        assert_eq!(format_string("hello world", &fmt), "Hello World");
    }

    #[test]
    fn format_relative_date_with_frozen_clock() {
        thread_local!(static NOW: Cell<i64> = const { Cell::new(0) });
    }

    #[test]
    fn format_relative_date_past_and_future() {
        let relative = RelativeDateFormat {
            units: vec![RelativeUnit::Day, RelativeUnit::Hour, RelativeUnit::Second],
            max_components: 2,
        };
        let now = 1_700_000_000;
        assert_eq!(
            format_relative_date(now - 86_400, now, &relative),
            "1 day ago",
        );
        assert_eq!(
            format_relative_date(now - (86_400 + 3600), now, &relative),
            "1 day and 1 hour ago",
        );
        assert_eq!(format_relative_date(now, now, &relative), "now");
        assert_eq!(
            format_relative_date(now + 86_400, now, &relative),
            "in 1 day",
        );
    }

    #[test]
    fn format_date_supports_all_documented_tokens() {
        let fmt = DateFormat {
            format: "%Y-%m-%d %H:%M:%S %y %B %b %A %a".into(),
            ..DateFormat::default()
        };
        // 2024-01-01 00:00:00 UTC == 1_704_067_200
        let out = format_date_at(1_704_067_200, 1_704_067_200, &fmt);
        assert!(out.contains("2024"), "{out}");
        assert!(out.contains("January"), "{out}");
        assert!(out.contains("Jan"), "{out}");
        assert!(out.contains("Monday"), "{out}");
        assert!(out.contains("Mon"), "{out}");
    }

    #[test]
    fn format_date_2_digit_year_handles_centuries() {
        let fmt = DateFormat {
            format: "%y".into(),
            ..DateFormat::default()
        };
        assert_eq!(format_date_at(1_704_067_200, 0, &fmt), "24");
    }

    #[test]
    fn cell_matches_filter_is_case_insensitive() {
        let fmt = plain_resolved(ColumnKind::Text);
        assert!(cell_matches_filter(
            &CellValue::Text("Hello".into()),
            &fmt,
            "ELL"
        ));
        assert!(cell_matches_filter(
            &CellValue::Text("Hello".into()),
            &fmt,
            ""
        ));
        assert!(!cell_matches_filter(
            &CellValue::Text("Hello".into()),
            &fmt,
            "zzz"
        ));
    }

    #[test]
    fn cell_matches_filter_uses_formatted_value_for_numbers() {
        let fmt = plain_resolved(ColumnKind::Decimal);
        assert!(cell_matches_filter(
            &CellValue::Decimal(1234.5),
            &fmt,
            "1,234"
        ));
        // Default decimal formatting emits a leading minus, not parentheses.
        assert!(cell_matches_filter(
            &CellValue::Decimal(-5.0),
            &fmt,
            "-5.00"
        ));
    }

    #[test]
    fn resolve_resolves_for_columns() {
        let cols = vec![
            Column::new("a", ColumnKind::Text, 80.0),
            Column::new("b", ColumnKind::Decimal, 100.0),
        ];
        let cfg = crate::config::GridConfig::default();
        let resolved = cfg.resolve_all(&cols);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].kind, ColumnKind::Text);
        assert_eq!(resolved[1].kind, ColumnKind::Decimal);
    }
}
