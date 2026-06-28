use crate::data::{CellValue, ColumnType};

pub fn format_cell(value: &CellValue, col_type: &ColumnType) -> (String, bool) {
    match (value, col_type) {
        (CellValue::Text(s), _) => (s.clone(), false),
        (CellValue::Integer(v), ColumnType::Integer { decimals }) => {
            (format_number(*v as f64, *decimals), *v < 0)
        }
        (CellValue::Decimal(v), ColumnType::Decimal { decimals }) => {
            (format_number(*v, *decimals), *v < 0.0)
        }
        (CellValue::Date(ts), ColumnType::Date { format }) => {
            (format_date(*ts, format), false)
        }
        (CellValue::Integer(v), _) => (v.to_string(), *v < 0),
        (CellValue::Decimal(v), _) => (v.to_string(), *v < 0.0),
        (CellValue::Date(ts), _) => (format_date(*ts, "%Y-%m-%d"), false),
        (CellValue::Boolean(b), ColumnType::Boolean) => (if *b { "true".into() } else { "false".into() }, false),
        (CellValue::Boolean(b), _) => (if *b { "true".into() } else { "false".into() }, false),
        (CellValue::None, _) => (String::new(), false),
    }
}

pub fn format_number(value: f64, decimals: usize) -> String {
    let abs = value.abs();
    let num_str = if decimals == 0 {
        format!("{:.0}", abs)
    } else {
        format!("{:.*}", decimals, abs)
    };
    let with_sep = add_thousands_separator(&num_str);
    if value < 0.0 {
        format!("-{}", with_sep)
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

pub fn format_date(ts: i64, format: &str) -> String {
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

pub fn cell_matches_filter(value: &CellValue, col_type: &ColumnType, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let (formatted, _) = format_cell(value, col_type);
    formatted.to_lowercase().contains(&filter.to_lowercase())
}
