//! Host-injected localization with standalone English defaults.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

/// Translates one complete message, including any named placeholder values.
///
/// Return `None` when the key is unavailable to use the built-in English
/// default. Arguments are ordered `(name, value)` pairs; placeholders in
/// catalog messages use `{name}` syntax.
pub trait Translator: Send + Sync + 'static {
    fn translate(&self, key: &str, args: &[(&str, &str)]) -> Option<String>;
}

impl<F> Translator for F
where
    F: Fn(&str, &[(&str, &str)]) -> Option<String> + Send + Sync + 'static,
{
    fn translate(&self, key: &str, args: &[(&str, &str)]) -> Option<String> {
        self(key, args)
    }
}

static TRANSLATOR: RwLock<Option<Arc<dyn Translator>>> = RwLock::new(None);

/// Install the process-wide translator used by every grid, chart, and pivot.
/// Call during application startup and again whenever the language changes.
pub fn install_translator(translator: impl Translator) {
    *TRANSLATOR
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(translator));
}

/// Restore the built-in English messages for all subsequently rendered text.
pub fn clear_translator() {
    *TRANSLATOR
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// Translate a message without placeholder arguments.
#[must_use]
pub fn text(key: &str) -> String {
    format(key, &[])
}

/// Translate and format a complete message with named placeholder arguments.
/// Unknown keys remain visible as the key itself, making incomplete catalog
/// integration diagnosable rather than silently blank.
#[must_use]
pub fn format(key: &str, args: &[(&str, &str)]) -> String {
    let translator = TRANSLATOR
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some(translated) = translator.and_then(|translator| translator.translate(key, args)) {
        return translated;
    }
    let template = english_default(key).unwrap_or(key).to_owned();
    replace_placeholders(template, args)
}

/// The English fallback for a known key, before placeholder substitution.
#[must_use]
pub fn english_default(key: &str) -> Option<&'static str> {
    static BY_KEY: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    BY_KEY
        .get_or_init(|| ENGLISH_DEFAULTS.iter().copied().collect())
        .get(key)
        .copied()
}

fn replace_placeholders(mut template: String, args: &[(&str, &str)]) -> String {
    for (name, value) in args {
        template = template.replace(&format!("{{{name}}}"), value);
    }
    template
}

/// Every crate-owned message and its standalone English default.
pub const ENGLISH_DEFAULTS: &[(&str, &str)] = &[
    ("datatable.grid.context_menu.select_column", "Select column"),
    ("datatable.grid.context_menu.copy_column", "Copy column"),
    (
        "datatable.grid.context_menu.copy_column_with_headers",
        "Copy column with headers",
    ),
    (
        "datatable.grid.context_menu.sort_ascending",
        "Sort Ascending",
    ),
    (
        "datatable.grid.context_menu.sort_descending",
        "Sort Descending",
    ),
    ("datatable.grid.context_menu.clear_sort", "Clear sort"),
    (
        "datatable.grid.context_menu.group_by_column",
        "Group by this column",
    ),
    (
        "datatable.grid.context_menu.clear_grouping",
        "Clear grouping",
    ),
    (
        "datatable.grid.context_menu.freeze_columns_up_to_here",
        "Freeze Columns Up to Here",
    ),
    (
        "datatable.grid.context_menu.unfreeze_columns",
        "Unfreeze Columns",
    ),
    ("datatable.grid.context_menu.hide_column", "Hide Column"),
    (
        "datatable.grid.context_menu.show_all_columns",
        "Show All Columns",
    ),
    (
        "datatable.grid.context_menu.move_column_left",
        "Move Column Left",
    ),
    (
        "datatable.grid.context_menu.move_column_right",
        "Move Column Right",
    ),
    ("datatable.grid.context_menu.filter_prompt", "Filter..."),
    ("datatable.grid.context_menu.clear_filter", "Clear filter"),
    ("datatable.grid.filter_panel.sort_ascending", "Ascending"),
    ("datatable.grid.filter_panel.sort_descending", "Descending"),
    (
        "datatable.grid.filter_panel.text_operator_choose_one",
        "Choose One",
    ),
    (
        "datatable.grid.filter_panel.text_operator_contains",
        "contains",
    ),
    (
        "datatable.grid.filter_panel.text_operator_does_not_contain",
        "does not contain",
    ),
    (
        "datatable.grid.filter_panel.text_operator_begins_with",
        "begins with",
    ),
    (
        "datatable.grid.filter_panel.text_operator_ends_with",
        "ends with",
    ),
    ("datatable.grid.filter_panel.text_operator_is", "is"),
    ("datatable.grid.filter_panel.text_operator_is_not", "is not"),
    (
        "datatable.grid.filter_panel.text_operator_matches_regex",
        "matches (regex)",
    ),
    (
        "datatable.grid.filter_panel.number_operator_choose_one",
        "Choose One",
    ),
    (
        "datatable.grid.filter_panel.number_operator_equal_to",
        "equal to",
    ),
    (
        "datatable.grid.filter_panel.number_operator_not_equal_to",
        "not equal to",
    ),
    (
        "datatable.grid.filter_panel.number_operator_greater_than",
        "greater than",
    ),
    (
        "datatable.grid.filter_panel.number_operator_greater_than_or_equal_to",
        "greater than or equal to",
    ),
    (
        "datatable.grid.filter_panel.number_operator_less_than",
        "less than",
    ),
    (
        "datatable.grid.filter_panel.number_operator_less_than_or_equal_to",
        "less than or equal to",
    ),
    (
        "datatable.grid.filter_panel.number_operator_between",
        "between",
    ),
    (
        "datatable.grid.filter_panel.number_operator_not_between",
        "not between",
    ),
    (
        "datatable.grid.filter_panel.date_operand_placeholder",
        "YYYY-MM-DD",
    ),
    (
        "datatable.grid.filter_panel.number_operand_placeholder",
        "value",
    ),
    (
        "datatable.grid.filter_panel.regex_operand_placeholder",
        "regex",
    ),
    (
        "datatable.grid.filter_panel.text_operand_placeholder",
        "value",
    ),
    ("datatable.grid.filter_panel.second_operand_and", "and"),
    ("datatable.grid.filter_panel.search_placeholder", "Search"),
    ("datatable.grid.filter_panel.select_all", "(Select All)"),
    (
        "datatable.grid.filter_panel.refine_search",
        "Refine search to see more…",
    ),
    ("datatable.grid.filter_panel.clear_filter", "Clear Filter"),
    ("datatable.grid.filter_panel.close", "Close"),
    ("datatable.grid.filter_panel.sort_heading", "Sort"),
    ("datatable.grid.filter_panel.filter_heading", "Filter"),
    ("datatable.grid.search.no_matches", "No matches"),
    ("datatable.grid.search.position", "{current} of {total}"),
    (
        "datatable.grid.search.position_truncated",
        "{current} of {total}+",
    ),
    ("datatable.grid.empty_state.no_rows", "No rows"),
    (
        "datatable.grid.group_header.row_singular",
        "{marker}  {group_label}  ({row_count} row)",
    ),
    (
        "datatable.grid.group_header.row_plural",
        "{marker}  {group_label}  ({row_count} rows)",
    ),
    ("datatable.grid.tabs.grid", "Grid"),
    ("datatable.grid.tabs.pivot", "Pivot"),
    ("datatable.grid.tabs.pivot_with_status", "Pivot  {status}"),
    ("datatable.grid.tabs.chart", "Chart"),
    ("datatable.grid.tabs.chart_with_status", "Chart  {status}"),
    (
        "datatable.grid.drill_filter.banner",
        "Filtered from pivot: {filter_label}",
    ),
    ("datatable.grid.drill_filter.clear_filter", "Clear filter"),
    (
        "datatable.grid.drill_filter.values_omitted",
        "{values} | … ({value_count} values)",
    ),
    (
        "datatable.grid.drill_filter.field_equals",
        "{field_name} = {values}",
    ),
    ("datatable.grid.sidebar.collapsed_pivot", "Pivot"),
    ("datatable.grid.sidebar.collapsed_chart", "Chart"),
    (
        "datatable.grid.data_error.ragged_row",
        "row {row_index} has {actual} cells but {expected} were expected",
    ),
    ("datatable.format.boolean.true", "true"),
    ("datatable.format.boolean.false", "false"),
    ("datatable.format.metric.thousands", "{value}K"),
    ("datatable.format.metric.millions", "{value}M"),
    ("datatable.format.metric.billions", "{value}G"),
    ("datatable.format.metric.trillions", "{value}T"),
    ("datatable.format.metric.quadrillions", "{value}P"),
    ("datatable.format.date.month_january", "January"),
    ("datatable.format.date.month_february", "February"),
    ("datatable.format.date.month_march", "March"),
    ("datatable.format.date.month_april", "April"),
    ("datatable.format.date.month_may", "May"),
    ("datatable.format.date.month_june", "June"),
    ("datatable.format.date.month_july", "July"),
    ("datatable.format.date.month_august", "August"),
    ("datatable.format.date.month_september", "September"),
    ("datatable.format.date.month_october", "October"),
    ("datatable.format.date.month_november", "November"),
    ("datatable.format.date.month_december", "December"),
    ("datatable.format.date.month_january_abbreviated", "Jan"),
    ("datatable.format.date.month_february_abbreviated", "Feb"),
    ("datatable.format.date.month_march_abbreviated", "Mar"),
    ("datatable.format.date.month_april_abbreviated", "Apr"),
    ("datatable.format.date.month_may_abbreviated", "May"),
    ("datatable.format.date.month_june_abbreviated", "Jun"),
    ("datatable.format.date.month_july_abbreviated", "Jul"),
    ("datatable.format.date.month_august_abbreviated", "Aug"),
    ("datatable.format.date.month_september_abbreviated", "Sep"),
    ("datatable.format.date.month_october_abbreviated", "Oct"),
    ("datatable.format.date.month_november_abbreviated", "Nov"),
    ("datatable.format.date.month_december_abbreviated", "Dec"),
    ("datatable.format.date.day_sunday", "Sunday"),
    ("datatable.format.date.day_monday", "Monday"),
    ("datatable.format.date.day_tuesday", "Tuesday"),
    ("datatable.format.date.day_wednesday", "Wednesday"),
    ("datatable.format.date.day_thursday", "Thursday"),
    ("datatable.format.date.day_friday", "Friday"),
    ("datatable.format.date.day_saturday", "Saturday"),
    ("datatable.format.date.day_sunday_abbreviated", "Sun"),
    ("datatable.format.date.day_monday_abbreviated", "Mon"),
    ("datatable.format.date.day_tuesday_abbreviated", "Tue"),
    ("datatable.format.date.day_wednesday_abbreviated", "Wed"),
    ("datatable.format.date.day_thursday_abbreviated", "Thu"),
    ("datatable.format.date.day_friday_abbreviated", "Fri"),
    ("datatable.format.date.day_saturday_abbreviated", "Sat"),
    ("datatable.format.date.unknown_month", "Unknown"),
    ("datatable.format.date.unknown_day", "Unknown"),
    ("datatable.format.relative.now", "now"),
    ("datatable.format.relative.component_separator", " and "),
    ("datatable.format.relative.future", "in {duration}"),
    ("datatable.format.relative.past", "{duration} ago"),
    ("datatable.format.relative.year_singular", "year"),
    ("datatable.format.relative.year_plural", "years"),
    ("datatable.format.relative.month_singular", "month"),
    ("datatable.format.relative.month_plural", "months"),
    ("datatable.format.relative.week_singular", "week"),
    ("datatable.format.relative.week_plural", "weeks"),
    ("datatable.format.relative.day_singular", "day"),
    ("datatable.format.relative.day_plural", "days"),
    ("datatable.format.relative.hour_singular", "hour"),
    ("datatable.format.relative.hour_plural", "hours"),
    ("datatable.format.relative.minute_singular", "minute"),
    ("datatable.format.relative.minute_plural", "minutes"),
    ("datatable.format.relative.second_singular", "second"),
    ("datatable.format.relative.second_plural", "seconds"),
    ("datatable.chart.error.no_rows", "No rows to chart"),
    (
        "datatable.chart.error.no_numeric_columns",
        "No numeric columns to chart",
    ),
    (
        "datatable.chart.error.no_numeric_column_to_histogram",
        "No numeric column to histogram",
    ),
    (
        "datatable.chart.error.no_numeric_values_to_histogram",
        "No numeric values to histogram",
    ),
    (
        "datatable.chart.error.radial_requires_one_series",
        "Pie and donut charts require exactly one numeric series",
    ),
    (
        "datatable.chart.error.radial_category_limit",
        "Pie and donut charts support at most {max_categories} categories",
    ),
    (
        "datatable.chart.error.radial_negative_values",
        "Pie and donut charts cannot represent negative values",
    ),
    (
        "datatable.chart.error.radial_requires_positive_value",
        "Pie and donut charts need at least one positive value",
    ),
    (
        "datatable.chart.series.fallback_name",
        "column {column_number}",
    ),
    ("datatable.chart.axis.thousands", "{value}K"),
    ("datatable.chart.axis.millions", "{value}M"),
    ("datatable.chart.axis.billions", "{value}B"),
    ("datatable.chart.category.true", "true"),
    ("datatable.chart.category.false", "false"),
    ("datatable.chart.kind.bar", "Bar"),
    ("datatable.chart.kind.line", "Line"),
    ("datatable.chart.kind.area", "Area"),
    ("datatable.chart.kind.scatter", "Scatter"),
    ("datatable.chart.kind.histogram", "Histogram"),
    ("datatable.chart.kind.pie", "Pie"),
    ("datatable.chart.kind.donut", "Donut"),
    ("datatable.chart.aggregate.none", "None"),
    ("datatable.chart.aggregate.sum", "Sum"),
    ("datatable.chart.aggregate.average", "Average"),
    ("datatable.chart.aggregate.minimum", "Minimum"),
    ("datatable.chart.aggregate.maximum", "Maximum"),
    ("datatable.chart.aggregate.count", "Count"),
    ("datatable.chart.order.source", "Source"),
    ("datatable.chart.order.category_ascending", "Category A–Z"),
    ("datatable.chart.order.value_descending", "Value high–low"),
    ("datatable.chart.sidebar.category_heading", "Category"),
    ("datatable.chart.sidebar.category_auto", "Auto"),
    ("datatable.chart.sidebar.values_heading", "Values"),
    (
        "datatable.chart.sidebar.no_numeric_columns",
        "No numeric columns",
    ),
    ("datatable.chart.sidebar.aggregate_heading", "Aggregate"),
    ("datatable.chart.sidebar.order_heading", "Order"),
    ("datatable.chart.sidebar.limit_heading", "Limit"),
    ("datatable.chart.sidebar.top_n", "Top {count}"),
    (
        "datatable.chart.sidebar.navigate_on_click",
        "Click marks to show source rows",
    ),
    (
        "datatable.chart.sidebar.navigate_explanation",
        "Clicking a bar, point, or slice selects the result rows behind it on the Grid tab.",
    ),
    ("datatable.chart.sidebar.copy_svg", "Copy SVG"),
    ("datatable.chart.sidebar.save_svg", "Save SVG…"),
    (
        "datatable.chart.sidebar.browser_save_svg_unavailable",
        "Save SVG is unavailable in the browser preview; use Copy SVG.",
    ),
    ("datatable.chart.sidebar.type_section", "Type"),
    ("datatable.chart.sidebar.data_section", "Data"),
    ("datatable.chart.sidebar.behavior_section", "Behavior"),
    ("datatable.chart.sidebar.export_section", "Export"),
    ("datatable.chart.sidebar.controls_title", "Chart Controls"),
    ("datatable.chart.legend.by_label", "by {label}"),
    (
        "datatable.chart.truncation.top_categories",
        "top {count} categories",
    ),
    (
        "datatable.chart.truncation.first_numeric_columns",
        "first {count} numeric columns",
    ),
    (
        "datatable.chart.truncation.first_source_rows",
        "first {count} source rows",
    ),
    ("datatable.chart.truncation.component_separator", " · "),
    ("datatable.chart.truncation.showing", "Showing {details}"),
    ("datatable.pivot.aggregation.count", "Count"),
    ("datatable.pivot.aggregation.sum", "Sum"),
    ("datatable.pivot.aggregation.average", "Avg"),
    ("datatable.pivot.aggregation.minimum", "Min"),
    ("datatable.pivot.aggregation.maximum", "Max"),
    ("datatable.pivot.aggregation.first", "First"),
    (
        "datatable.pivot.aggregation.count_caption",
        "Count of {field_name}",
    ),
    (
        "datatable.pivot.aggregation.sum_caption",
        "Sum of {field_name}",
    ),
    (
        "datatable.pivot.aggregation.average_caption",
        "Avg of {field_name}",
    ),
    (
        "datatable.pivot.aggregation.minimum_caption",
        "Min of {field_name}",
    ),
    (
        "datatable.pivot.aggregation.maximum_caption",
        "Max of {field_name}",
    ),
    (
        "datatable.pivot.aggregation.first_caption",
        "First of {field_name}",
    ),
    ("datatable.pivot.values.caption", "Values"),
    (
        "datatable.pivot.values.fallback_field_name",
        "column {column_index}",
    ),
    ("datatable.pivot.totals.grand_total", "Grand Total"),
    ("datatable.pivot.totals.total", "Total"),
    ("datatable.pivot.totals.group_total", "{group_label} Total"),
    ("datatable.pivot.group.blank", "(blank)"),
    (
        "datatable.pivot.empty_state.no_matching_data",
        "No data matches the current pivot filters.",
    ),
    (
        "datatable.pivot.empty_state.not_configured",
        "Drag fields into Rows, Columns, and Values to build the pivot.",
    ),
    (
        "datatable.pivot.context_menu.show_source_rows",
        "Show source rows in Grid",
    ),
    ("datatable.pivot.context_menu.copy_value", "Copy value"),
    ("datatable.pivot.context_menu.copy_csv", "Copy pivot as CSV"),
    (
        "datatable.pivot.sidebar.fallback_field_name",
        "column {column_index}",
    ),
    (
        "datatable.pivot.sidebar.drag_fields_here",
        "Drag fields here",
    ),
    ("datatable.pivot.sidebar.filter_select_all", "(Select All)"),
    ("datatable.pivot.sidebar.filter_clear", "Clear"),
    ("datatable.pivot.sidebar.filter_close", "Close"),
    (
        "datatable.pivot.sidebar.filter_title",
        "Filter: {field_name}",
    ),
    (
        "datatable.pivot.sidebar.filter_first_values",
        "Showing first {count} values…",
    ),
    ("datatable.pivot.sidebar.format_reset", "Reset"),
    ("datatable.pivot.sidebar.format_close", "Close"),
    (
        "datatable.pivot.sidebar.format_title",
        "Format: {field_name}",
    ),
    (
        "datatable.pivot.sidebar.negative_numbers_in_red",
        "Negative numbers in red",
    ),
    (
        "datatable.pivot.sidebar.thousands_separator",
        "Thousands separator",
    ),
    ("datatable.pivot.sidebar.negatives_heading", "Negatives"),
    ("datatable.pivot.sidebar.negative_example_minus", "-1,234"),
    (
        "datatable.pivot.sidebar.negative_example_parentheses",
        "(1,234)",
    ),
    ("datatable.pivot.sidebar.decimals_heading", "Decimals"),
    ("datatable.pivot.sidebar.alignment_heading", "Alignment"),
    ("datatable.pivot.sidebar.align_left", "Left"),
    ("datatable.pivot.sidebar.align_center", "Center"),
    ("datatable.pivot.sidebar.align_right", "Right"),
    ("datatable.pivot.sidebar.layout_heading", "Layout"),
    (
        "datatable.pivot.sidebar.flat_rows",
        "Flat rows (no hierarchy)",
    ),
    ("datatable.pivot.sidebar.totals_heading", "Totals"),
    ("datatable.pivot.sidebar.row_subtotals", "Row subtotals"),
    (
        "datatable.pivot.sidebar.column_subtotals",
        "Column subtotal columns",
    ),
    ("datatable.pivot.sidebar.grand_total_row", "Grand total row"),
    (
        "datatable.pivot.sidebar.grand_total_column",
        "Grand total column",
    ),
    ("datatable.pivot.sidebar.groups_heading", "Groups"),
    ("datatable.pivot.sidebar.expand_rows", "Expand rows"),
    ("datatable.pivot.sidebar.collapse_rows", "Collapse rows"),
    ("datatable.pivot.sidebar.expand_columns", "Expand cols"),
    ("datatable.pivot.sidebar.collapse_columns", "Collapse cols"),
    ("datatable.pivot.sidebar.export_heading", "Export"),
    ("datatable.pivot.sidebar.copy_csv", "Copy pivot as CSV"),
    (
        "datatable.pivot.sidebar.unconfigured_hint",
        "Drag a field into a layout zone",
    ),
    (
        "datatable.pivot.sidebar.save_configuration",
        "Save configuration",
    ),
    ("datatable.pivot.sidebar.filters_zone", "Filters"),
    ("datatable.pivot.sidebar.columns_zone", "Columns"),
    ("datatable.pivot.sidebar.rows_zone", "Rows"),
    ("datatable.pivot.sidebar.values_zone", "Values"),
    ("datatable.pivot.sidebar.rows_zone_badge", "R"),
    ("datatable.pivot.sidebar.columns_zone_badge", "C"),
    ("datatable.pivot.sidebar.values_zone_badge", "V"),
    ("datatable.pivot.sidebar.filters_zone_badge", "F"),
    ("datatable.pivot.sidebar.fields_section", "Fields"),
    ("datatable.pivot.sidebar.layout_section", "Layout"),
    (
        "datatable.pivot.sidebar.display_section",
        "Display and export",
    ),
    ("datatable.pivot.sidebar.controls_title", "Pivot Controls"),
];
