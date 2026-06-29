use crate::config::{
    DateFormat, NumberFormat, RelativeDateFormat, RelativeUnit, ReplacementTiming,
    ResolvedColumnFormat, StringFormat, TextCase, TextAlignment, TruncationBehavior,
    ReplacementRule,
};
use crate::data::{ColumnKind, CellValue};
use std::time::{SystemTime, UNIX_EPOCH};

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
        (CellValue::Integer(v), ColumnKind::Integer) => (format_number(*v as f64, &fmt.number), *v < 0),
        (CellValue::Decimal(v), ColumnKind::Decimal) => (format_number(*v, &fmt.number), *v < 0.0),
        (CellValue::Integer(v), ColumnKind::Decimal) => (format_number(*v as f64, &fmt.number), *v < 0),
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

pub fn format_number(value: f64, fmt: &NumberFormat) -> String {
    let abs = value.abs();
    let num_str = format!("{:.*}", fmt.decimals, abs);
    let with_sep = if fmt.thousands_separator {
        add_thousands_separator(&num_str)
    } else {
        num_str
    };
    if value < 0.0 {
        if fmt.negative_parentheses {
            format!("({})", with_sep)
        } else {
            format!("-{}", with_sep)
        }
    } else {
        with_sep
    }
}

fn add_thousands_separator(s: &str) -> String {
    let parts: Vec<&str> = s.split('.').collect();
    let int_part = parts[0];
    let dec_part = if parts.len() > 1 { format!(".{}", parts[1]) } else { String::new() };
    let chars: Vec<char> = int_part.chars().collect();
    let mut result = String::new();
    let len = chars.len();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(*c);
    }
    format!("{}{}", result, dec_part)
}

pub fn format_date(ts: i64, fmt: &DateFormat) -> String {
    let adjusted_ts = ts + (fmt.timezone_offset_minutes as i64) * 60;
    if let Some(relative) = &fmt.relative {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let adjusted_now = now + (fmt.timezone_offset_minutes as i64) * 60;
        return format_relative_date(adjusted_ts, adjusted_now, relative);
    }
    format_date_str(adjusted_ts, &fmt.format)
}

fn format_date_str(ts: i64, format: &str) -> String {
    let (year, month, day, hour, min, sec) = timestamp_to_components(ts);
    format
        .replace("%Y", &format!("{:04}", year))
        .replace("%m", &format!("{:02}", month))
        .replace("%d", &format!("{:02}", day))
        .replace("%H", &format!("{:02}", hour))
        .replace("%M", &format!("{:02}", min))
        .replace("%S", &format!("{:02}", sec))
        .replace("%y", &format!("{:02}", year % 100))
        .replace("%B", &month_name(month))
        .replace("%b", &month_name(month)[..3])
        .replace("%A", &day_name(ts))
        .replace("%a", &day_name(ts)[..3])
}

fn format_relative_date(ts: i64, now: i64, relative: &RelativeDateFormat) -> String {
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
        format!("in {}", joined)
    } else {
        format!("{} ago", joined)
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
    all.iter()
        .filter(|u| units.contains(u))
        .cloned()
        .collect()
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
        RelativeUnit::Year => if count == 1 { "year" } else { "years" },
        RelativeUnit::Month => if count == 1 { "month" } else { "months" },
        RelativeUnit::Week => if count == 1 { "week" } else { "weeks" },
        RelativeUnit::Day => if count == 1 { "day" } else { "days" },
        RelativeUnit::Hour => if count == 1 { "hour" } else { "hours" },
        RelativeUnit::Minute => if count == 1 { "minute" } else { "minutes" },
        RelativeUnit::Second => if count == 1 { "second" } else { "seconds" },
    }
}

fn format_boolean(b: bool, fmt: &crate::config::BooleanFormat) -> String {
    if b {
        fmt.true_text.clone()
    } else {
        fmt.false_text.clone()
    }
}

fn format_string(s: &str, fmt: &StringFormat) -> String {
    let cased = match fmt.case {
        TextCase::Upper => s.to_uppercase(),
        TextCase::Lower => s.to_lowercase(),
        TextCase::Title => title_case(s),
        TextCase::None => s.to_string(),
    };
    let result = match fmt.max_length {
        Some(max) if cased.chars().count() > max => {
            let truncated: String = cased.chars().take(max).collect();
            match fmt.truncation {
                TruncationBehavior::Ellipsis => {
                    if max >= 3 {
                        let mut t: String = cased.chars().take(max - 3).collect();
                        t.push_str("...");
                        t
                    } else {
                        truncated
                    }
                }
                TruncationBehavior::CutOff => truncated,
                TruncationBehavior::Wrap => truncated,
            }
        }
        _ => cased,
    };
    result
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
    let mut result = s.to_string();
    for rule in rules {
        result = result.replace(&rule.find, &rule.replace);
    }
    result
}

fn timestamp_to_components(ts: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = ts.div_euclid(86400);
    let secs = ts.rem_euclid(86400) as u32;
    let hour = secs / 3600;
    let min = (secs % 3600) / 60;
    let sec = secs % 60;
    let (year, month, day) = days_to_ymd(days);
    (year, month, day, hour, min, sec)
}

fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
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
        1 => "January".into(), 2 => "February".into(), 3 => "March".into(),
        4 => "April".into(), 5 => "May".into(), 6 => "June".into(),
        7 => "July".into(), 8 => "August".into(), 9 => "September".into(),
        10 => "October".into(), 11 => "November".into(), 12 => "December".into(),
        _ => "Unknown".into(),
    }
}

fn day_name(ts: i64) -> String {
    let day_of_week = (((ts.div_euclid(86400) + 4i64) % 7) as u32) % 7;
    match day_of_week {
        0 => "Sunday".into(), 1 => "Monday".into(), 2 => "Tuesday".into(),
        3 => "Wednesday".into(), 4 => "Thursday".into(), 5 => "Friday".into(),
        6 => "Saturday".into(), _ => "Unknown".into(),
    }
}

pub fn cell_matches_filter(value: &CellValue, fmt: &ResolvedColumnFormat, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let (formatted, _) = format_cell(value, fmt);
    formatted.to_lowercase().contains(&filter.to_lowercase())
}

pub fn alignment_for(fmt: &ResolvedColumnFormat) -> TextAlignment {
    fmt.alignment()
}
