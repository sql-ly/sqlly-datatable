//! The sqlly-datatable sample application, structured as a library entered
//! through [`init_and_open`]: the `sqlly-datatable-sample` binary
//! (`src/main.rs`) calls it inside `gpui::Application::new().run(...)`.
//!
//! (The 4.0.x experimental web/wasm entry point is gone with the move back
//! to registry `gpui` — the web backend exists only on zed's git `main`.)

use gpui::{
    div, prelude::*, px, size, App, Bounds, ClipboardItem, Context, Entity, Window, WindowBounds,
    WindowOptions,
};
use sqlly_datatable::{
    AggregationFn, Column, ColumnKind, ColumnOverride, ContextMenuItem, ContextMenuProvider,
    ContextMenuRequest, GridConfig, GridData, GridState, GridThemePair, NumberFormat, PivotConfig,
    PivotContextMenuProvider, PivotContextMenuRequest, PivotMenuItem, PivotState, SqllyDataTable,
};

/// Everything the sample does once a GPUI `App` exists: initialize the
/// toolkit, build the grid + pivot widget, and open the demo window. Shared
/// verbatim by the native binary and the wasm entry point (on the web the
/// "window" is the browser canvas; sizing/movability options are ignored
/// there, and the close handler simply never fires).
pub fn init_and_open(cx: &mut App) {
    let data = sample_data();
    let config = sample_config(&data);

    sqlly_datatable::init(cx);
    cx.activate(true);

    let view = SqllyDataTable::builder(data)
        .config(config)
        .theme_family(ThemeChoice::Component.pair())
        .context_menu_provider(SampleMenuProvider)
        .pivot(sample_pivot_config())
        .pivot_context_menu_provider(SamplePivotMenuProvider)
        .pivot_save_config(|config, _cx| {
            println!("save pivot config: {config:?}");
        })
        .build(cx);
    let focus = view.state.read(cx).focus_handle.clone();

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1200.0), px(700.0)),
            cx,
        ))),
        titlebar: Some(Default::default()),
        is_movable: true,
        is_resizable: true,
        window_min_size: Some(size(px(600.0), px(400.0))),
        ..Default::default()
    };

    // Move the built widget (grid + pivot tab) into the window intact —
    // re-wrapping only the GridState via `SqllyDataTable::new` would
    // drop the pivot parts created by the builder.
    match cx.open_window(options, move |_window, cx| {
        let table = cx.new(|_cx| view);
        cx.new(|_cx| RootView {
            table,
            theme_choice: ThemeChoice::Component,
        })
    }) {
        Ok(window) => {
            let _ = window.update(cx, |_view, window, cx| {
                window.focus(&focus);
                window.on_window_should_close(cx, |_window, cx| {
                    cx.quit();
                    true
                });
            });
        }
        Err(err) => {
            eprintln!("failed to open window: {err}");
            cx.quit();
        }
    }
}

/// The theme families offered by the sample app's switcher: the two shipped
/// with the crate, plus a family derived from `gpui-component`'s built-in
/// palettes via the `GridTheme::from_component_colors` bridge.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ThemeChoice {
    Neutral,
    Signature,
    Component,
}

impl ThemeChoice {
    fn pair(self) -> GridThemePair {
        match self {
            Self::Neutral => GridThemePair::neutral(),
            Self::Signature => GridThemePair::signature(),
            Self::Component => GridThemePair::component(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Neutral => "Neutral",
            Self::Signature => "Signature",
            Self::Component => "Component",
        }
    }
}

/// Root view: a themed toolbar (title + theme-family switcher) above the
/// grid. The toolbar paints from the grid's current `GridTheme`, so it
/// follows both the family switcher and the OS light/dark appearance.
struct RootView {
    table: Entity<SqllyDataTable>,
    theme_choice: ThemeChoice,
}

impl RootView {
    fn segment(
        &self,
        choice: ThemeChoice,
        theme: &sqlly_datatable::GridTheme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.theme_choice == choice;
        let hover_bg = theme.menu_hover_bg;
        let hover_fg = theme.header_fg;
        div()
            .id(choice.label())
            .px(px(12.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .text_size(px(12.0))
            .when(active, |seg| {
                seg.bg(theme.selection_bg).text_color(theme.header_fg)
            })
            .when(!active, |seg| {
                seg.text_color(theme.muted_text)
                    .hover(move |seg| seg.bg(hover_bg).text_color(hover_fg))
            })
            .on_click(cx.listener(move |this, _event, window, cx| {
                if this.theme_choice == choice {
                    return;
                }
                this.theme_choice = choice;
                this.table.update(cx, |table, cx| {
                    table.set_theme_family(choice.pair(), window, cx);
                });
                cx.notify();
            }))
            .child(choice.label())
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.table.read(cx).state.read(cx).theme.clone();
        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(44.0))
            .px(px(12.0))
            .bg(theme.header_bg)
            .border_b_1()
            .border_color(theme.grid_line)
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.header_fg)
                    .child("sqlly-datatable"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.muted_text)
                            .child("Theme"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.0))
                            .p(px(2.0))
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(theme.grid_line)
                            .bg(theme.bg)
                            .child(self.segment(ThemeChoice::Neutral, &theme, cx))
                            .child(self.segment(ThemeChoice::Signature, &theme, cx))
                            .child(self.segment(ThemeChoice::Component, &theme, cx)),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg)
            .child(toolbar)
            .child(div().flex_1().min_h_0().child(self.table.clone()))
    }
}

/// Preconfigured pivot layout demonstrating programmatic setup: the P&L at
/// a glance — region/location down the side, transaction type across the
/// top, summed amounts at the intersections, with payment method and cost
/// center available as source filters. The user can rearrange everything
/// from the sidebar at runtime.
fn sample_pivot_config() -> PivotConfig {
    PivotConfig {
        row_fields: vec![4, 5], // region, location
        column_fields: vec![1], // txn_type
        value_field: Some(2),   // amount
        aggregation: AggregationFn::Sum,
        filter_fields: vec![8, 6], // payment_method, cost_center
        ..PivotConfig::default()
    }
}

fn sample_config(data: &GridData) -> GridConfig {
    let mut config = GridConfig::default();
    let mut overrides = vec![ColumnOverride::default(); data.columns.len()];

    for (i, col) in data.columns.iter().enumerate() {
        match col.kind {
            ColumnKind::Integer => {
                overrides[i] = ColumnOverride {
                    number: Some(NumberFormat {
                        decimals: 0,
                        ..NumberFormat::default()
                    }),
                    ..Default::default()
                };
            }
            ColumnKind::Decimal => {
                // Money columns: cents.
                overrides[i] = ColumnOverride {
                    number: Some(NumberFormat {
                        decimals: 2,
                        ..NumberFormat::default()
                    }),
                    ..Default::default()
                };
            }
            _ => {}
        }
    }
    config.column_overrides = overrides;
    config
}

/// Default record count (native and web; small enough for wasm32's 32-bit
/// address space even at ~124 columns per row).
const DEFAULT_SAMPLE_ROWS: usize = 60_000;

/// Marker columns separating the burger-chain ledger from the randomized
/// wide block: header and every cell value are the same word, so it is
/// obvious mid-scroll where the ledger ends and the stress-test columns
/// begin.
const MARKER_COLUMNS: [&str; 6] = [
    "extra",
    "columns",
    "to",
    "show",
    "horizontal",
    "virturalizion",
];

/// Number of randomized columns after the markers, cycling datetime / text /
/// decimal / integer / boolean (20 of each). Together with the ledger and
/// markers this puts the grid at 124 columns — enough to exercise
/// horizontal virtualization.
const WIDE_COLUMNS: usize = 100;

/// Six months of daily business: 2026-01-01 through 2026-06-30.
const WINDOW_DAYS: i64 = 181;

/// Days from 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date from days since 1970-01-01 (inverse of [`days_from_civil`]).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Round to cents.
fn cents(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Six months of the general ledger of "Patty Cachè 🍔", a fictitious
/// US burger chain: 3 regions × 10 locations, 500 staff recording sales,
/// purchases, and payroll. Sales rows carry the receipt fields (receipt
/// number, tax / sub-total / coupon / total, customer id); purchase rows
/// carry vendor / invoice / purchase-order; payroll rows carry only an
/// amount — so the dataset exercises NULL-heavy columns, pivoting income
/// vs. expense, and per-group receipt numbering.
fn sample_data() -> GridData {
    use sqlly_datatable::CellValue;
    use sqlly_datatable::CellValue::{Boolean, Date, Decimal, Integer, Text};

    /// Value pool for the randomized wide text columns.
    const WIDE_WORDS: [&str; 16] = [
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
        "juliett", "kilo", "lima", "mike", "november", "oscar", "papa",
    ];

    let mut columns = vec![
        Column::new("txn_date", ColumnKind::Date, 120.0),
        Column::new("txn_type", ColumnKind::Text, 110.0),
        Column::new("amount", ColumnKind::Decimal, 120.0),
        Column::new("recorded_by", ColumnKind::Text, 170.0),
        Column::new("region", ColumnKind::Text, 110.0),
        Column::new("location", ColumnKind::Text, 145.0),
        Column::new("cost_center", ColumnKind::Text, 150.0),
        Column::new("item", ColumnKind::Text, 190.0),
        Column::new("payment_method", ColumnKind::Text, 140.0),
        Column::new("receipt_number", ColumnKind::Text, 235.0),
        Column::new("tax_amount", ColumnKind::Decimal, 110.0),
        Column::new("sub_total", ColumnKind::Decimal, 110.0),
        Column::new("coupon_value", ColumnKind::Decimal, 125.0),
        Column::new("total", ColumnKind::Decimal, 110.0),
        Column::new("customer_id", ColumnKind::Text, 130.0),
        Column::new("vendor", ColumnKind::Text, 190.0),
        Column::new("invoice_number", ColumnKind::Text, 140.0),
        Column::new("purchase_order", ColumnKind::Text, 145.0),
    ];
    // Marker columns: header == value in every row.
    for name in MARKER_COLUMNS {
        columns.push(Column::new(name, ColumnKind::Text, 130.0));
    }
    // The randomized wide block, cycling the five cell kinds.
    const WIDE_KINDS: [ColumnKind; 5] = [
        ColumnKind::Date,
        ColumnKind::Text,
        ColumnKind::Decimal,
        ColumnKind::Integer,
        ColumnKind::Boolean,
    ];
    for i in 0..WIDE_COLUMNS {
        let kind = WIDE_KINDS[i % WIDE_KINDS.len()];
        let width = match kind {
            ColumnKind::Date => 150.0,
            ColumnKind::Text => 140.0,
            _ => 115.0,
        };
        columns.push(Column::new(format!("wide_{:03}", i + 1), kind, width));
    }

    // --- dimension tables -------------------------------------------------
    let regions: [(&str, &str, f64); 3] = [
        // (name, 4-char receipt code, sales-tax rate)
        ("West", "WEST", 0.0900),
        ("Central", "CNTR", 0.0825),
        ("East", "EAST", 0.0700),
    ];
    let locations: [[(&str, &str); 10]; 3] = [
        [
            ("Seattle", "SEAT"),
            ("Portland", "PORT"),
            ("San Francisco", "SNFR"),
            ("Los Angeles", "LOSA"),
            ("San Diego", "SNDG"),
            ("Las Vegas", "LSVG"),
            ("Phoenix", "PHNX"),
            ("Denver", "DENV"),
            ("Salt Lake City", "SLCY"),
            ("Sacramento", "SACR"),
        ],
        [
            ("Dallas", "DALL"),
            ("Austin", "AUST"),
            ("Houston", "HOUS"),
            ("Chicago", "CHIC"),
            ("Minneapolis", "MINN"),
            ("St. Louis", "STLS"),
            ("Kansas City", "KSCY"),
            ("Oklahoma City", "OKCY"),
            ("Nashville", "NASH"),
            ("New Orleans", "NWOR"),
        ],
        [
            ("New York", "NYRK"),
            ("Boston", "BOST"),
            ("Philadelphia", "PHIL"),
            ("Miami", "MIAM"),
            ("Atlanta", "ATLA"),
            ("Charlotte", "CHAR"),
            ("Washington", "WASH"),
            ("Baltimore", "BALT"),
            ("Pittsburgh", "PITT"),
            ("Orlando", "ORLA"),
        ],
    ];

    // 500 distinct staff names: 25 first × 20 last.
    let first_names = [
        "Ava", "Ben", "Carla", "Diego", "Elena", "Frank", "Grace", "Hector", "Imani", "Jonas",
        "Keiko", "Liam", "Maria", "Noah", "Olivia", "Priya", "Quinn", "Rosa", "Sam", "Tara",
        "Umar", "Vera", "Wes", "Ximena", "Yusuf",
    ];
    let last_names = [
        "Adams", "Baker", "Chen", "Dawson", "Ellis", "Flores", "García", "Hughes", "Ito",
        "Jackson", "Kim", "López", "Meyer", "Nguyen", "O'Brien", "Patel", "Quintero", "Reyes",
        "Silva", "Torres",
    ];

    let sale_cost_centers = [
        "Food Sales",
        "Beverage Sales",
        "Merchandise",
        "Catering",
        "Delivery Orders",
    ];
    let menu_items = [
        "Classic Burger",
        "Double Cheeseburger",
        "Bacon Smash",
        "Mushroom Swiss",
        "Veggie Burger",
        "Jalapeño Popper Burger",
        "Crispy Chicken Sandwich",
        "Kids Meal",
        "Fries",
        "Onion Rings",
        "Side Salad",
        "Chocolate Shake",
        "Vanilla Shake",
        "Fountain Drink",
        "Combo #1",
        "Combo #2",
        "Combo #3",
        "Patty Cachè Tee",
    ];

    let expense_cost_centers = [
        "Food Supplies",
        "Packaging",
        "Rent",
        "Utilities",
        "Marketing",
        "Equipment",
        "Maintenance",
        "Insurance",
        "Waste Disposal",
    ];
    let supply_items = [
        "Ground Beef 80/20",
        "Burger Buns",
        "American Cheese",
        "Käse Bühler 🧀 Swiss",
        "Lettuce",
        "Tomatoes",
        "Red Onions",
        "Pickles",
        "Fry Oil",
        "Potatoes",
        "Napkins",
        "Cups 16oz",
        "To-Go Bags",
        "Cleaning Supplies",
    ];
    let vendors = [
        "Lone Star Beef Co.",
        "Golden Bun Bakery",
        "FreshFarm Produce",
        "Pacific Paper Goods",
        "Metro Restaurant Supply",
        "Cascade Dairy Collective",
        "Sunbelt Property Group",
        "GridPower Utilities",
        "AdWave Media",
        "KitchenTech Equipment",
        "SparkleClean Services",
        "SafeGuard Insurance",
        "GreenHaul Waste",
        "Bühler Käse Imports",
        "Riverline Logistics",
        "Bayou Spice Trading",
    ];
    let payment_methods = [
        "Amex",
        "Visa",
        "Master Card",
        "Crypto",
        "Cash",
        "Apple Pay",
        "Google Pay",
    ];

    let window_start_days = days_from_civil(2026, 1, 1);

    // Row count override for testing edge cases (e.g. SQLLY_SAMPLE_ROWS=0
    // shows the empty-result state). The wasm build has no environment.
    #[cfg(not(target_family = "wasm"))]
    let row_count: usize = std::env::var("SQLLY_SAMPLE_ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_SAMPLE_ROWS);
    #[cfg(target_family = "wasm")]
    let row_count: usize = DEFAULT_SAMPLE_ROWS;

    // Deterministic pseudo-random generator — enough variety across 500k
    // rows without pulling in the `rand` crate.
    let mut rng = Lcg::new(0x0123_4567_89AB_CDEF);
    // Per-(day, location) sequential receipt counter, per the receipt format
    // {region:4}{location:4}{yyyyMMdd}{seq:08}.
    let mut receipt_seq = vec![0u32; (WINDOW_DAYS as usize) * 30];

    let mut rows = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        let day = (rng.next_u64() % WINDOW_DAYS as u64) as i64;
        let region_ix = (rng.next_u64() % 3) as usize;
        let location_ix = (rng.next_u64() % 10) as usize;
        let (region_name, region_code, tax_rate) = regions[region_ix];
        let (location_name, location_code) = locations[region_ix][location_ix];

        let (y, m, d) = civil_from_days(window_start_days + day);
        // Business hours: 06:00–23:00 local.
        let time_of_day = 6 * 3600 + (rng.next_u64() % (17 * 3600)) as i64;
        let txn_date = Date((window_start_days + day) * 86_400 + time_of_day);

        let user_ix = (rng.next_u64() % 500) as usize;
        let recorded_by = Text(format!(
            "{} {}",
            first_names[user_ix % 25],
            last_names[user_ix / 25]
        ));

        let mut row = Vec::with_capacity(columns.len());
        row.push(txn_date);

        let kind = rng.next_u64() % 1000;
        if kind < 940 {
            // ---- sale ----------------------------------------------------
            let seq = &mut receipt_seq[(day as usize) * 30 + region_ix * 10 + location_ix];
            *seq += 1;
            let receipt = format!("{region_code}{location_code}{y:04}{m:02}{d:02}{seq:08}");

            let sub_total = cents(4.0 + rng.next_f64() * 51.0);
            let coupon = if rng.next_u64() % 100 < 20 {
                cents((1.0 + rng.next_f64() * 7.0).min(sub_total * 0.5))
            } else {
                0.0
            };
            let tax = cents((sub_total - coupon) * tax_rate);
            let total = cents(sub_total - coupon + tax);

            row.push(Text("Sale".into()));
            row.push(Decimal(total)); // amount: income
            row.push(recorded_by);
            row.push(Text(region_name.into()));
            row.push(Text(location_name.into()));
            row.push(Text(
                sale_cost_centers[(rng.next_u64() % 5) as usize].into(),
            ));
            // "Not every record has an item": a small share of sales are
            // recorded without one.
            if rng.next_u64() % 100 < 93 {
                row.push(Text(menu_items[(rng.next_u64() % 18) as usize].into()));
            } else {
                row.push(CellValue::None);
            }
            row.push(Text(payment_methods[(rng.next_u64() % 7) as usize].into()));
            row.push(Text(receipt));
            row.push(Decimal(tax));
            row.push(Decimal(sub_total));
            row.push(Decimal(coupon));
            row.push(Decimal(total));
            row.push(Text(format!("CUST-{:06}", rng.next_u64() % 250_000)));
            row.push(CellValue::None); // vendor
            row.push(CellValue::None); // invoice_number
            row.push(CellValue::None); // purchase_order
        } else if kind < 985 {
            // ---- purchase ------------------------------------------------
            let cost_center = expense_cost_centers[(rng.next_u64() % 9) as usize];
            let amount = -cents(25.0 + rng.next_f64() * 445.0);

            row.push(Text("Purchase".into()));
            row.push(Decimal(amount)); // amount: expense
            row.push(recorded_by);
            row.push(Text(region_name.into()));
            row.push(Text(location_name.into()));
            row.push(Text(cost_center.into()));
            // Only goods-like cost centers name a specific item.
            if matches!(cost_center, "Food Supplies" | "Packaging") {
                row.push(Text(supply_items[(rng.next_u64() % 14) as usize].into()));
            } else {
                row.push(CellValue::None);
            }
            row.push(CellValue::None); // payment_method (invoiced, not card)
            row.push(CellValue::None); // receipt_number
            row.push(CellValue::None); // tax_amount
            row.push(CellValue::None); // sub_total
            row.push(CellValue::None); // coupon_value
            row.push(CellValue::None); // total
            row.push(CellValue::None); // customer_id
            row.push(Text(vendors[(rng.next_u64() % 16) as usize].into()));
            row.push(Text(format!("INV-{:07}", rng.next_u64() % 10_000_000)));
            row.push(Text(format!("PO-{:07}", rng.next_u64() % 10_000_000)));
        } else {
            // ---- payroll -------------------------------------------------
            let amount = -cents(500.0 + rng.next_f64() * 600.0);

            row.push(Text("Payroll".into()));
            row.push(Decimal(amount)); // amount: expense
            row.push(recorded_by);
            row.push(Text(region_name.into()));
            row.push(Text(location_name.into()));
            row.push(Text("Payroll".into()));
            row.push(CellValue::None); // item
            row.push(CellValue::None); // payment_method
            row.push(CellValue::None); // receipt_number
            row.push(CellValue::None); // tax_amount
            row.push(CellValue::None); // sub_total
            row.push(CellValue::None); // coupon_value
            row.push(CellValue::None); // total
            row.push(CellValue::None); // customer_id
            row.push(CellValue::None); // vendor
            row.push(CellValue::None); // invoice_number
            row.push(CellValue::None); // purchase_order
        }

        // Marker cells: header == value, all the way down.
        for name in MARKER_COLUMNS {
            row.push(Text(name.into()));
        }
        // The randomized wide block, kinds matching the column declarations.
        for col in &columns[18 + MARKER_COLUMNS.len()..] {
            let cell = match col.kind {
                ColumnKind::Date => {
                    Date(window_start_days * 86_400 + (rng.next_u64() % (365 * 86_400)) as i64)
                }
                ColumnKind::Text => {
                    Text(WIDE_WORDS[(rng.next_u64() % WIDE_WORDS.len() as u64) as usize].into())
                }
                ColumnKind::Decimal => Decimal(cents(rng.next_f64() * 10_000.0)),
                ColumnKind::Integer => Integer((rng.next_u64() % 1_000_000) as i64),
                ColumnKind::Boolean => Boolean(rng.next_u64() & 1 == 0),
                ColumnKind::None => CellValue::None,
            };
            row.push(cell);
        }
        rows.push(row);
    }

    GridData::new(columns, rows).expect("rectangular sample data")
    // (allowed in a sample binary, not in the library)
}

/// Minimal linear-congruential generator — deterministic demo data without a
/// dependency. Uses the Numerical Recipes 64-bit constants.
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }

    fn next_f64(&mut self) -> f64 {
        // Map the high 53 bits of the state into [0, 1).
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Sample context-menu provider that demonstrates cell/row right-click menus.
struct SampleMenuProvider;

impl ContextMenuProvider for SampleMenuProvider {
    fn menu_items(&self, request: &ContextMenuRequest) -> Vec<ContextMenuItem> {
        let mut items = Vec::new();

        if let Some(row) = request.clicked_row() {
            if let Some(cost_center) = row.value_by_name("cost_center") {
                items.push(ContextMenuItem::action(
                    "copy-narrative",
                    format!("Copy cost center: {cost_center:?}"),
                ));
            }
            if let Some(amount) = row.value_by_name("amount") {
                items.push(ContextMenuItem::action(
                    "copy-amount",
                    format!("Copy amount: {amount:?}"),
                ));
            }
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::action(
                "copy-row",
                format!("Copy full row ({} cells)", row.values.len()),
            ));
            items.push(ContextMenuItem::action(
                "copy-row-csv",
                format!("Copy row as CSV ({} fields)", row.columns.len()),
            ));
            items.push(ContextMenuItem::action("copy-row-json", "Copy row as JSON"));
            items.push(ContextMenuItem::action(
                "row-cell-count",
                format!(
                    "Row {} has {} cells",
                    row.display_row_index,
                    row.values.len()
                ),
            ));
            items.push(ContextMenuItem::action(
                "row-source-index",
                format!("Source row index: {}", row.source_row_index),
            ));
            // --- 10 additional row items ---
            items.push(ContextMenuItem::action("copy-row-tsv", "Copy row as TSV"));
            items.push(ContextMenuItem::action(
                "copy-row-keys",
                format!("Copy column names ({})", row.columns.len()),
            ));
            items.push(ContextMenuItem::action(
                "copy-row-values",
                "Copy values only",
            ));
            items.push(ContextMenuItem::action(
                "copy-row-markdown",
                "Copy row as Markdown table",
            ));
            items.push(ContextMenuItem::action(
                "copy-row-sql",
                "Copy row as SQL INSERT",
            ));
            items.push(ContextMenuItem::action(
                "row-non-empty-count",
                format!(
                    "Non-empty cells: {}",
                    row.values
                        .iter()
                        .filter(|v| !matches!(v, sqlly_datatable::CellValue::None))
                        .count()
                ),
            ));
            items.push(ContextMenuItem::action(
                "row-numeric-sum",
                "Sum numeric cells",
            ));
            items.push(ContextMenuItem::action(
                "copy-row-pipe",
                "Copy row (pipe-delimited)",
            ));
            items.push(ContextMenuItem::action(
                "copy-row-display-index",
                format!("Copy display index: {}", row.display_row_index),
            ));
            items.push(ContextMenuItem::action(
                "row-column-kinds",
                "Copy column kinds",
            ));
        }

        if let Some(cell) = request.clicked_cell() {
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::action(
                "copy-cell",
                format!("Copy cell ({}): {:?}", cell.column_name, cell.value),
            ));
            items.push(ContextMenuItem::action(
                "cell-location",
                format!(
                    "Cell at col {} ({}) row {}",
                    cell.column_index, cell.column_name, cell.display_row_index
                ),
            ));
            // --- 10 additional cell items ---
            items.push(ContextMenuItem::action(
                "copy-cell-name-value",
                format!("Copy \"{}={:?}\"", cell.column_name, cell.value),
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-column-name",
                format!("Copy column name: {}", cell.column_name),
            ));
            items.push(ContextMenuItem::action(
                "cell-value-kind",
                format!("Value kind: {}", cell_value_kind(&cell.value)),
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-source-index",
                format!("Copy source row: {}", cell.source_row_index),
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-coord",
                format!(
                    "Copy coord (r{}, c{})",
                    cell.display_row_index, cell.column_index
                ),
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-json",
                "Copy cell as JSON",
            ));
            items.push(ContextMenuItem::action(
                "cell-is-empty",
                format!(
                    "Cell empty? {}",
                    matches!(cell.value, sqlly_datatable::CellValue::None)
                ),
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-upper",
                "Copy value (UPPERCASE)",
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-lower",
                "Copy value (lowercase)",
            ));
            items.push(ContextMenuItem::action(
                "copy-cell-len",
                "Copy value length",
            ));
        }

        if request.selected_cell_count() > 0 {
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::action(
                "copy-selection",
                format!("Copy {} selected cell(s)", request.selected_cell_count()),
            ));
            let rows = request.selected_row_count();
            let cells = request.selected_cell_count();
            // `selected_row_count` is 0 for column-oriented selections, so
            // the column count is only derivable when rows are known.
            let summary = if rows > 0 {
                format!("Selection spans {} row(s) × {} col(s)", rows, cells / rows)
            } else {
                format!("Selection spans {cells} cell(s) in whole column(s)")
            };
            items.push(ContextMenuItem::action("selection-summary", summary));
            // --- 10 additional selection items ---
            items.push(ContextMenuItem::action(
                "copy-selection-values",
                "Copy selection values only",
            ));
            items.push(ContextMenuItem::action(
                "copy-selection-tsv",
                "Copy selection as TSV",
            ));
            items.push(ContextMenuItem::action(
                "copy-selection-json",
                "Copy selection as JSON",
            ));
            items.push(ContextMenuItem::action(
                "copy-selection-csv",
                "Copy selection as CSV",
            ));
            items.push(ContextMenuItem::action(
                "selection-numeric-sum",
                "Sum numeric selection",
            ));
            items.push(ContextMenuItem::action(
                "selection-numeric-avg",
                "Average numeric selection",
            ));
            items.push(ContextMenuItem::action(
                "selection-cell-count",
                format!("Selected cells: {}", request.selected_cell_count()),
            ));
            items.push(ContextMenuItem::action(
                "selection-row-count",
                format!("Selected rows: {}", request.selected_row_count()),
            ));
            items.push(ContextMenuItem::action(
                "selection-distinct-columns",
                "Copy distinct column names",
            ));
            items.push(ContextMenuItem::action(
                "selection-empty-count",
                "Count empty cells in selection",
            ));
        }

        // Compose built-in column-header actions when right-clicking headers.
        if matches!(
            request.target,
            sqlly_datatable::ContextMenuTarget::ColumnHeader { .. }
                | sqlly_datatable::ContextMenuTarget::SortButton { .. }
        ) {
            items.push(ContextMenuItem::separator());
            items.extend(ContextMenuItem::standard_column_header_items());
        }

        if items.is_empty() {
            items.push(ContextMenuItem::action("noop", "No action for this target"));
        }

        items
    }

    fn on_action(
        &self,
        action_id: &str,
        request: &ContextMenuRequest,
        state: &mut GridState,
        cx: &mut App,
    ) {
        // Heavy export runs off the main thread so the UI stays responsive and
        // a loading indicator is shown while the CSV is assembled.
        if action_id == "copy-selection-csv" {
            let req = request.clone();
            state.spawn_background(
                cx,
                "Exporting selection to CSV…",
                move || {
                    use std::fmt::Write as _;
                    let mut out = String::new();
                    req.for_each_selected_cell(|c| {
                        let _ = writeln!(out, "{},{:?}", c.column_name, c.value);
                    });
                    out
                },
                |csv, _s, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(csv));
                },
            );
            return;
        }

        let text = match action_id {
            "copy-narrative" => request
                .clicked_row()
                .and_then(|r| r.value_by_name("cost_center").map(|v| format!("{v:?}")))
                .unwrap_or_default(),
            "copy-amount" => request
                .clicked_row()
                .and_then(|r| r.value_by_name("amount").map(|v| format!("{v:?}")))
                .unwrap_or_default(),
            "copy-row" => request
                .clicked_row()
                .map(|r| {
                    r.named_values()
                        .map(|(name, val)| format!("{name}={val:?}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
            "copy-row-csv" => request
                .clicked_row()
                .map(|r| {
                    let header = r
                        .columns
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(",");
                    let values = r
                        .values
                        .iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("{header}\n{values}")
                })
                .unwrap_or_default(),
            "copy-row-json" => request
                .clicked_row()
                .map(|r| {
                    let pairs = r
                        .named_values()
                        .map(|(name, val)| format!("\"{name}\": {val:?}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{{ {pairs} }}")
                })
                .unwrap_or_default(),
            "row-cell-count" | "row-source-index" => {
                // Diagnostic items: reflect the clicked row's metadata.
                request
                    .clicked_row()
                    .map(|r| {
                        format!(
                            "row={} source={} cells={}",
                            r.display_row_index,
                            r.source_row_index,
                            r.values.len()
                        )
                    })
                    .unwrap_or_default()
            }
            "copy-cell" => request
                .clicked_cell()
                .map(|c| format!("{:?}", c.value))
                .unwrap_or_default(),
            "cell-location" => request
                .clicked_cell()
                .map(|c| {
                    format!(
                        "col={} ({}) row={}",
                        c.column_index, c.column_name, c.display_row_index
                    )
                })
                .unwrap_or_default(),
            "copy-selection" => request
                .selected_cells()
                .iter()
                .map(|c| format!("{}={:?}", c.column_name, c.value))
                .collect::<Vec<_>>()
                .join("\n"),
            "selection-summary" => format!(
                "{} rows × {} cells",
                request.selected_rows().len(),
                request.selected_cells().len()
            ),
            // --- additional row actions ---
            "copy-row-tsv" => request
                .clicked_row()
                .map(|r| {
                    let header = r
                        .columns
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join("\t");
                    let values = r
                        .values
                        .iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join("\t");
                    format!("{header}\n{values}")
                })
                .unwrap_or_default(),
            "copy-row-keys" => request
                .clicked_row()
                .map(|r| {
                    r.columns
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default(),
            "copy-row-values" => request
                .clicked_row()
                .map(|r| {
                    r.values
                        .iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
            "copy-row-markdown" => request
                .clicked_row()
                .map(|r| {
                    let header = r
                        .columns
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" | ");
                    let sep = r
                        .columns
                        .iter()
                        .map(|_| "---")
                        .collect::<Vec<_>>()
                        .join(" | ");
                    let values = r
                        .values
                        .iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join(" | ");
                    format!("| {header} |\n| {sep} |\n| {values} |")
                })
                .unwrap_or_default(),
            "copy-row-sql" => request
                .clicked_row()
                .map(|r| {
                    let cols = r
                        .columns
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let vals = r
                        .values
                        .iter()
                        .map(sql_literal)
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("INSERT INTO sample_data ({cols}) VALUES ({vals});")
                })
                .unwrap_or_default(),
            "row-non-empty-count" => request
                .clicked_row()
                .map(|r| {
                    r.values
                        .iter()
                        .filter(|v| !matches!(v, sqlly_datatable::CellValue::None))
                        .count()
                        .to_string()
                })
                .unwrap_or_default(),
            "row-numeric-sum" => request
                .clicked_row()
                .map(|r| sum_numeric(r.values.iter()).to_string())
                .unwrap_or_default(),
            "copy-row-pipe" => request
                .clicked_row()
                .map(|r| {
                    r.values
                        .iter()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .unwrap_or_default(),
            "copy-row-display-index" => request
                .clicked_row()
                .map(|r| r.display_row_index.to_string())
                .unwrap_or_default(),
            "row-column-kinds" => request
                .clicked_row()
                .map(|r| {
                    r.columns
                        .iter()
                        .map(|c| format!("{}: {:?}", c.name, c.kind))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
            // --- additional cell actions ---
            "copy-cell-name-value" => request
                .clicked_cell()
                .map(|c| format!("{}={:?}", c.column_name, c.value))
                .unwrap_or_default(),
            "copy-cell-column-name" => request
                .clicked_cell()
                .map(|c| c.column_name.clone())
                .unwrap_or_default(),
            "cell-value-kind" => request
                .clicked_cell()
                .map(|c| cell_value_kind(&c.value).to_string())
                .unwrap_or_default(),
            "copy-cell-source-index" => request
                .clicked_cell()
                .map(|c| c.source_row_index.to_string())
                .unwrap_or_default(),
            "copy-cell-coord" => request
                .clicked_cell()
                .map(|c| format!("(r{}, c{})", c.display_row_index, c.column_index))
                .unwrap_or_default(),
            "copy-cell-json" => request
                .clicked_cell()
                .map(|c| format!("{{ \"{}\": {:?} }}", c.column_name, c.value))
                .unwrap_or_default(),
            "cell-is-empty" => request
                .clicked_cell()
                .map(|c| matches!(c.value, sqlly_datatable::CellValue::None).to_string())
                .unwrap_or_default(),
            "copy-cell-upper" => request
                .clicked_cell()
                .map(|c| cell_display(&c.value).to_uppercase())
                .unwrap_or_default(),
            "copy-cell-lower" => request
                .clicked_cell()
                .map(|c| cell_display(&c.value).to_lowercase())
                .unwrap_or_default(),
            "copy-cell-len" => request
                .clicked_cell()
                .map(|c| cell_display(&c.value).chars().count().to_string())
                .unwrap_or_default(),
            // --- additional selection actions ---
            "copy-selection-values" => request
                .selected_cells()
                .iter()
                .map(|c| format!("{:?}", c.value))
                .collect::<Vec<_>>()
                .join("\n"),
            "copy-selection-tsv" => request
                .selected_cells()
                .iter()
                .map(|c| format!("{}\t{:?}", c.column_name, c.value))
                .collect::<Vec<_>>()
                .join("\n"),
            "copy-selection-json" => {
                let pairs = request
                    .selected_cells()
                    .iter()
                    .map(|c| format!("{{ \"{}\": {:?} }}", c.column_name, c.value))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[ {pairs} ]")
            }
            "selection-numeric-sum" => {
                sum_numeric(request.selected_cells().iter().map(|c| &c.value)).to_string()
            }
            "selection-numeric-avg" => {
                let nums: Vec<f64> = request
                    .selected_cells()
                    .iter()
                    .filter_map(|c| numeric_value(&c.value))
                    .collect();
                if nums.is_empty() {
                    "0".to_string()
                } else {
                    (nums.iter().sum::<f64>() / nums.len() as f64).to_string()
                }
            }
            "selection-cell-count" => request.selected_cells().len().to_string(),
            "selection-row-count" => request.selected_rows().len().to_string(),
            "selection-distinct-columns" => {
                let cells = request.selected_cells();
                let mut names: Vec<&str> = cells.iter().map(|c| c.column_name.as_str()).collect();
                names.sort_unstable();
                names.dedup();
                names.join(", ")
            }
            "selection-empty-count" => request
                .selected_cells()
                .iter()
                .filter(|c| matches!(c.value, sqlly_datatable::CellValue::None))
                .count()
                .to_string(),
            _ => return,
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

/// Sample pivot right-click provider: composes the built-in items (drill
/// through, copy value, copy CSV) with custom actions that demonstrate the
/// context passed up — the clicked cell's grouping path, aggregation, and
/// the source rows driving it.
struct SamplePivotMenuProvider;

/// `"region=Europe, product=Widget"`-style description of a clicked cell.
fn pivot_cell_path(request: &PivotContextMenuRequest) -> Option<String> {
    let cell = request.clicked_cell()?;
    let parts: Vec<String> = cell
        .row_path
        .iter()
        .chain(&cell.col_path)
        .map(|c| format!("{}={}", c.field_name, c.label))
        .collect();
    Some(if parts.is_empty() {
        "grand total".to_owned()
    } else {
        parts.join(", ")
    })
}

impl PivotContextMenuProvider for SamplePivotMenuProvider {
    fn menu_items(&self, request: &PivotContextMenuRequest) -> Vec<PivotMenuItem> {
        let mut items = PivotMenuItem::standard_items(&request.target);
        items.push(PivotMenuItem::separator());
        if let Some(cell) = request.clicked_cell() {
            items.push(PivotMenuItem::action(
                "copy-cell-path",
                format!(
                    "Copy cell path ({})",
                    pivot_cell_path(request).unwrap_or_default()
                ),
            ));
            items.push(PivotMenuItem::action(
                "copy-cell-summary",
                format!(
                    "{}: {}",
                    request.value_caption,
                    if cell.formatted_value.is_empty() {
                        "(empty)"
                    } else {
                        &cell.formatted_value
                    }
                ),
            ));
        }
        items.push(PivotMenuItem::action(
            "copy-driving-count",
            format!("Driving source rows: {}", request.source_row_count()),
        ));
        // Column-label sort has no built-in affordance; expose the API here
        // so the style checklist can exercise the header sort glyph.
        items.push(PivotMenuItem::separator());
        items.push(PivotMenuItem::action(
            "sort-column-labels",
            "Sort column labels (cycle)",
        ));
        items
    }

    fn on_action(
        &self,
        action_id: &str,
        request: &PivotContextMenuRequest,
        state: &mut PivotState,
        cx: &mut App,
    ) {
        if action_id == "sort-column-labels" {
            state.cycle_col_label_sort();
            return;
        }
        let text = match action_id {
            "copy-cell-path" => pivot_cell_path(request).unwrap_or_default(),
            "copy-cell-summary" => request
                .clicked_cell()
                .map(|c| format!("{} = {}", request.value_caption, c.formatted_value))
                .unwrap_or_default(),
            "copy-driving-count" => request.source_row_count().to_string(),
            _ => return,
        };
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
}

/// Short human-readable name for a cell value's variant.
fn cell_value_kind(value: &sqlly_datatable::CellValue) -> &'static str {
    use sqlly_datatable::CellValue;
    match value {
        CellValue::Text(_) => "Text",
        CellValue::Integer(_) => "Integer",
        CellValue::Decimal(_) => "Decimal",
        CellValue::Date(_) => "Date",
        CellValue::Boolean(_) => "Boolean",
        CellValue::None => "None",
    }
}

/// Plain string rendering of a cell value (no `Debug` quoting).
fn cell_display(value: &sqlly_datatable::CellValue) -> String {
    use sqlly_datatable::CellValue;
    match value {
        CellValue::Text(s) => s.clone(),
        CellValue::Integer(i) => i.to_string(),
        CellValue::Decimal(d) => d.to_string(),
        CellValue::Date(d) => d.to_string(),
        CellValue::Boolean(b) => b.to_string(),
        CellValue::None => String::new(),
    }
}

/// SQL literal rendering: numbers bare, everything else single-quoted, `None` -> NULL.
fn sql_literal(value: &sqlly_datatable::CellValue) -> String {
    use sqlly_datatable::CellValue;
    match value {
        CellValue::Integer(i) => i.to_string(),
        CellValue::Decimal(d) => d.to_string(),
        CellValue::Boolean(b) => b.to_string(),
        CellValue::None => "NULL".to_string(),
        CellValue::Date(d) => d.to_string(),
        CellValue::Text(s) => format!("'{}'", s.replace('\'', "''")),
    }
}

/// Numeric projection of a cell value; `None` for non-numeric variants.
fn numeric_value(value: &sqlly_datatable::CellValue) -> Option<f64> {
    use sqlly_datatable::CellValue;
    match value {
        CellValue::Integer(i) => Some(*i as f64),
        CellValue::Decimal(d) => Some(*d),
        _ => None,
    }
}

/// Sum the numeric cells in an iterator of values.
fn sum_numeric<'a>(values: impl Iterator<Item = &'a sqlly_datatable::CellValue>) -> f64 {
    values.filter_map(numeric_value).sum()
}
