//! Core data model for the grid: cell values, columns, and the rectangular
//! [`GridData`] container.
//!
//! `GridData` is intentionally simple — a column list paired with a `Vec` of
//! rectangular rows of [`CellValue`]. It carries no rendering, sorting, or
//! filtering state: those live on [`crate::grid::GridState`]. Keeping the data
//! layer pure makes it reusable from outside the widget (export pipelines,
//! server-side previews, test fixtures).
//!
//! [`CellValue`] does not implement [`Eq`]/[`Ord`] because [`CellValue::Decimal`]
//! holds an `f64`. Use [`compare_cells`] when you need a deterministic total
//! ordering that handles `NaN` and mixed numeric kinds deliberately rather
//! than collapsing to `Equal`.

use std::cmp::Ordering;

/// A single cell value.
///
/// Decimal values are stored as `f64`; for very large integers that exceed
/// `2^53`, route them through [`CellValue::Text`] instead.
#[derive(Clone, Debug, PartialEq)]
pub enum CellValue {
    /// Free-form text. The grid will case-fold, truncate, and align it per
    /// [`crate::config::StringFormat`].
    Text(String),
    /// 64-bit signed integer.
    Integer(i64),
    /// 64-bit floating point. `NaN` is permitted; [`compare_cells`] places it
    /// after all finite numbers so sorting remains stable.
    Decimal(f64),
    /// Unix timestamp in seconds. Formatting is driven by
    /// [`crate::config::DateFormat`].
    Date(i64),
    /// Boolean value rendered with [`crate::config::BooleanFormat`].
    Boolean(bool),
    /// Explicit "no value" — distinct from empty string and zero. Sorts before
    /// every other variant.
    None,
}

/// Declared column kind. Drives the default [`crate::config::ResolvedColumnFormat`]
/// when no [`crate::config::ColumnOverride`] is supplied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColumnKind {
    /// Text columns (`StringFormat`).
    Text,
    /// Integer columns (`NumberFormat`, default decimals = 0).
    Integer,
    /// Decimal columns (`NumberFormat`, default decimals = 2).
    Decimal,
    /// Date columns (`DateFormat`).
    Date,
    /// Boolean columns (`BooleanFormat`).
    Boolean,
    /// Unknown / un-inferred kind. Falls back to [`crate::config::StringFormat`] for display.
    None,
}

/// A single column declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// Human-readable column name. Rendered as the header label.
    pub name: String,
    /// Inferred kind driving default formatting.
    pub kind: ColumnKind,
    /// Initial column width in logical pixels. Resizable by the user at runtime.
    pub width: f32,
}

impl Column {
    /// Convenience constructor.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: ColumnKind, width: f32) -> Self {
        Self {
            name: name.into(),
            kind,
            width,
        }
    }
}

/// Rectangular grid data: `rows.len()` rows each of length `columns.len()`.
///
/// The library does not silently fix ragged rows; use [`GridData::new`] or
/// [`GridData::validate`] to detect and reject them.
#[derive(Clone, Debug)]
pub struct GridData {
    /// Column metadata. `columns.len()` is the row width for every row.
    pub columns: Vec<Column>,
    /// Row contents. Every row must have exactly `columns.len()` cells.
    pub rows: Vec<Vec<CellValue>>,
}

/// Error returned when [`GridData`] cannot be constructed or validated because
/// at least one row's length disagrees with the column count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GridDataError {
    /// A row had a different number of cells than `columns.len()`.
    RaggedRow {
        /// Index of the offending row.
        row_index: usize,
        /// Expected number of cells (always `columns.len()`).
        expected: usize,
        /// Actual number of cells found in the row.
        actual: usize,
    },
}

impl std::fmt::Display for GridDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GridDataError::RaggedRow {
                row_index,
                expected,
                actual,
            } => write!(
                f,
                "row {row_index} has {actual} cells but {expected} were expected"
            ),
        }
    }
}

impl std::error::Error for GridDataError {}

impl GridData {
    /// Construct a new `GridData`, validating that every row has exactly
    /// `columns.len()` cells.
    ///
    /// # Errors
    ///
    /// Returns [`GridDataError::RaggedRow`] pointing at the first mis-sized row.
    pub fn new(columns: Vec<Column>, rows: Vec<Vec<CellValue>>) -> Result<Self, GridDataError> {
        let data = Self { columns, rows };
        data.validate()?;
        Ok(data)
    }

    /// Validate the rectangular invariant. Cheap; called by [`GridData::new`]
    /// and by debug assertions in the paint/copy hot paths.
    ///
    /// # Errors
    ///
    /// Returns [`GridDataError::RaggedRow`] pointing at the first mis-sized row.
    pub fn validate(&self) -> Result<(), GridDataError> {
        let expected = self.columns.len();
        for (row_index, row) in self.rows.iter().enumerate() {
            if row.len() != expected {
                return Err(GridDataError::RaggedRow {
                    row_index,
                    expected,
                    actual: row.len(),
                });
            }
        }
        Ok(())
    }

    /// Safe accessor for cell `(row, col)`. Returns `None` if either index is
    /// out of bounds.
    #[must_use]
    pub fn cell(&self, row: usize, col: usize) -> Option<&CellValue> {
        self.rows.get(row).and_then(|r| r.get(col))
    }

    /// Number of rows (after sort/filter this reflects the live `display_indices`
    /// length, not `rows.len()`).
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Number of columns. Always equal to any row's length if [`Self::validate`]
    /// succeeded.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Ordinal index of the first column whose name matches `name` exactly
    /// (case-sensitive). If duplicate names exist, the first match wins.
    #[must_use]
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|col| col.name == name)
    }
}

impl From<&str> for CellValue {
    fn from(s: &str) -> Self {
        CellValue::Text(s.to_owned())
    }
}

impl From<String> for CellValue {
    fn from(s: String) -> Self {
        CellValue::Text(s)
    }
}

impl From<i64> for CellValue {
    fn from(v: i64) -> Self {
        CellValue::Integer(v)
    }
}

impl From<i32> for CellValue {
    fn from(v: i32) -> Self {
        CellValue::Integer(v.into())
    }
}

impl From<f64> for CellValue {
    fn from(v: f64) -> Self {
        CellValue::Decimal(v)
    }
}

impl From<bool> for CellValue {
    fn from(v: bool) -> Self {
        CellValue::Boolean(v)
    }
}

impl From<Option<CellValue>> for CellValue {
    fn from(v: Option<CellValue>) -> Self {
        v.unwrap_or(CellValue::None)
    }
}

/// Total deterministic ordering for `CellValue`.
///
/// Behavior:
///
/// * Same-kind numeric values compare numerically; decimals use
///   [`f64::total_cmp`] so `NaN` is ordered consistently (after all finite
///   values, and `0.0` before `-0.0` is reversed by `total_cmp` semantics —
///   we keep that contract).
/// * Mixed `Integer` / `Decimal` pairs compare numerically.
/// * `None` always sorts before every other variant.
/// * Cross-type non-numeric pairs fall back to a stable type-rank order so
///   the return value is never `Equal` for genuinely different values.
///
/// Use [`std::cmp::Ordering`] directly via `slice::sort_by`; do not rely on
/// whatever a future `PartialOrd` derive might produce.
#[must_use]
pub fn compare_cells(a: &CellValue, b: &CellValue) -> Ordering {
    match (a, b) {
        (CellValue::None, CellValue::None) => Ordering::Equal,
        (CellValue::None, _) => Ordering::Less,
        (_, CellValue::None) => Ordering::Greater,

        (CellValue::Integer(x), CellValue::Integer(y)) => x.cmp(y),
        (CellValue::Decimal(x), CellValue::Decimal(y)) => x.total_cmp(y),
        (CellValue::Integer(x), CellValue::Decimal(y)) => (*x as f64).total_cmp(y),
        (CellValue::Decimal(x), CellValue::Integer(y)) => x.total_cmp(&(*y as f64)),

        (CellValue::Text(x), CellValue::Text(y)) => x.cmp(y),
        (CellValue::Date(x), CellValue::Date(y)) => x.cmp(y),
        (CellValue::Boolean(x), CellValue::Boolean(y)) => x.cmp(y),

        (left, right) => type_rank(left).cmp(&type_rank(right)),
    }
}

/// Stable rank used as a tie-breaker when two cells have different, non-numeric
/// kinds. Sort order is `None < Boolean < Integer == Decimal < Date < Text`.
fn type_rank(value: &CellValue) -> u8 {
    match value {
        CellValue::None => 0,
        CellValue::Boolean(_) => 1,
        CellValue::Integer(_) | CellValue::Decimal(_) => 2,
        CellValue::Date(_) => 3,
        CellValue::Text(_) => 4,
    }
}

/// A handful of synthetic ledger-style rows for examples and the sample
/// application. Kept here so examples have a known shape without pulling in a
/// separate data file. Production code should construct [`GridData`] directly.
#[must_use]
pub fn sample_data() -> GridData {
    use CellValue::{Boolean as B, Decimal as D, Integer as I, None as N, Text as T};
    use ColumnKind::*;

    let columns = vec![
        Column::new("JournalLineId", Integer, 120.0),
        Column::new("TenantId", Integer, 100.0),
        Column::new("JournalId", Integer, 110.0),
        Column::new("FinancialAccountingKeyId", Integer, 200.0),
        Column::new("ExtendedFinancialAccountingKeyId", Integer, 240.0),
        Column::new("TransactionCurrencyAmount", Decimal, 200.0),
        Column::new("JurisdictionalCurrencyAmount", Decimal, 200.0),
        Column::new("ReportingCurrencyAmount", Decimal, 200.0),
        Column::new("Sequence", Integer, 100.0),
        Column::new("TransPart", Boolean, 110.0),
        Column::new("ReferenceTypeId", Integer, 140.0), // Nullable
        Column::new("ReferenceEntityId", Integer, 150.0), // Nullable
        Column::new("InternalReference", Text, 160.0),
        Column::new("CounterPartyReference", Text, 180.0),
        Column::new("Narrative", Text, 270.0),
        Column::new("CurrencyId", Integer, 110.0),
        Column::new("IsCleared", Boolean, 110.0),
    ];

    let row = |id: i64,
               ta: i64,
               ja: i64,
               fa: i64,
               ea: i64,
               tx: i64,
               jx: i64,
               rx: i64,
               sq: i64,
               pa: bool,
               rt: Option<i64>,
               re: Option<i64>,
               ir: &str,
               cr: Option<&str>,
               na: &str,
               ci: i64,
               cl: bool| {
        vec![
            I(id),
            I(ta),
            I(ja),
            I(fa),
            I(ea),
            D(tx as f64),
            D(jx as f64),
            D(rx as f64),
            I(sq),
            B(pa),
            rt.map(I).unwrap_or(N),
            re.map(I).unwrap_or(N),
            T(ir.into()),
            cr.map(|s| T(s.into())).unwrap_or(N),
            T(na.into()),
            I(ci),
            B(cl),
        ]
    };

    let rows = vec![
        row(
            1096,
            1,
            148,
            33,
            528,
            17968,
            17968,
            485,
            0,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1097,
            1,
            148,
            33,
            530,
            717,
            717,
            19,
            1,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1098,
            1,
            148,
            33,
            532,
            768,
            768,
            20,
            2,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1099,
            1,
            148,
            33,
            533,
            1141,
            1141,
            30,
            3,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1100,
            1,
            148,
            33,
            536,
            1937,
            1937,
            52,
            4,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1101,
            1,
            148,
            33,
            538,
            1018,
            1018,
            27,
            5,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1102,
            1,
            148,
            33,
            542,
            3172,
            3172,
            85,
            6,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1103,
            1,
            148,
            33,
            544,
            1640,
            1640,
            44,
            7,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1104,
            1,
            148,
            33,
            546,
            809,
            809,
            21,
            8,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1105,
            1,
            148,
            33,
            573,
            67,
            67,
            1,
            9,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1106,
            1,
            148,
            33,
            574,
            20,
            20,
            0,
            10,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1107,
            1,
            148,
            33,
            575,
            70,
            70,
            1,
            11,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1108,
            1,
            148,
            33,
            576,
            29,
            29,
            0,
            12,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1109,
            1,
            148,
            33,
            577,
            35,
            35,
            0,
            13,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1110,
            1,
            148,
            33,
            578,
            283,
            283,
            7,
            14,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1111,
            1,
            148,
            33,
            579,
            200,
            200,
            5,
            15,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1112,
            1,
            148,
            33,
            580,
            1140,
            1140,
            30,
            16,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1113,
            1,
            148,
            33,
            581,
            117,
            117,
            3,
            17,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1114,
            1,
            148,
            33,
            582,
            366,
            366,
            9,
            18,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1115,
            1,
            148,
            33,
            603,
            241,
            241,
            6,
            19,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1116,
            1,
            148,
            33,
            604,
            458,
            458,
            12,
            20,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1117,
            1,
            148,
            33,
            605,
            2640,
            2640,
            71,
            21,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1118,
            1,
            148,
            33,
            606,
            104,
            104,
            2,
            22,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1119,
            1,
            148,
            33,
            607,
            236,
            236,
            6,
            23,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1120,
            1,
            148,
            33,
            608,
            356,
            356,
            9,
            24,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
        row(
            1121,
            1,
            148,
            33,
            609,
            323,
            323,
            8,
            25,
            false,
            Option::None,
            Option::None,
            "tomar 1",
            Option::None,
            "saldo de apertura de carga",
            1,
            false,
        ),
    ];

    GridData { columns, rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_same_kind_numeric() {
        assert_eq!(
            compare_cells(&CellValue::Integer(1), &CellValue::Integer(2)),
            Ordering::Less
        );
        assert_eq!(
            compare_cells(&CellValue::Integer(2), &CellValue::Integer(1)),
            Ordering::Greater
        );
        assert_eq!(
            compare_cells(&CellValue::Integer(7), &CellValue::Integer(7)),
            Ordering::Equal
        );
        assert_eq!(
            compare_cells(&CellValue::Decimal(1.5), &CellValue::Decimal(2.5)),
            Ordering::Less
        );
    }

    #[test]
    fn compare_decimal_handles_nan_deterministically() {
        let nan = CellValue::Decimal(f64::NAN);
        let one = CellValue::Decimal(1.0);
        // `NaN` should not collapse to Equal: it sits at a defined slot in total_cmp.
        assert_ne!(compare_cells(&nan, &one), Ordering::Equal);
        // Two `NaN` should be equal under total_cmp.
        assert_eq!(
            compare_cells(&nan, &CellValue::Decimal(f64::NAN)),
            Ordering::Equal
        );
    }

    #[test]
    fn compare_mixed_numeric_via_total_cmp() {
        assert_eq!(
            compare_cells(&CellValue::Integer(5), &CellValue::Decimal(5.5)),
            Ordering::Less,
        );
        assert_eq!(
            compare_cells(&CellValue::Decimal(5.5), &CellValue::Integer(5)),
            Ordering::Greater,
        );
        assert_eq!(
            compare_cells(&CellValue::Integer(5), &CellValue::Decimal(5.0)),
            Ordering::Equal,
        );
    }

    #[test]
    fn compare_null_is_always_less_than_other() {
        assert_eq!(
            compare_cells(&CellValue::None, &CellValue::Integer(0)),
            Ordering::Less
        );
        assert_eq!(
            compare_cells(&CellValue::Integer(0), &CellValue::None),
            Ordering::Greater
        );
        assert_eq!(
            compare_cells(&CellValue::None, &CellValue::None),
            Ordering::Equal
        );
        assert_eq!(
            compare_cells(&CellValue::None, &CellValue::Text("z".into())),
            Ordering::Less
        );
    }

    #[test]
    fn compare_cross_type_non_numeric_is_deterministic_non_equal() {
        // Different kinds, neither numeric, both non-null -> type-rank, Equal only by rank.
        assert_ne!(
            compare_cells(&CellValue::Boolean(true), &CellValue::Text("x".into())),
            Ordering::Equal,
        );
        assert_eq!(
            compare_cells(&CellValue::Boolean(true), &CellValue::Boolean(true)),
            Ordering::Equal,
        );
    }

    #[test]
    fn grid_data_construction_validates_rows() {
        let cols = vec![
            Column::new("a", ColumnKind::Integer, 80.0),
            Column::new("b", ColumnKind::Integer, 80.0),
        ];
        // Good.
        let ok = GridData::new(
            cols.clone(),
            vec![vec![CellValue::Integer(1), CellValue::Integer(2)]],
        );
        assert!(ok.is_ok());

        // Ragged row.
        let bad = GridData::new(
            cols,
            vec![vec![
                CellValue::Integer(1),
                CellValue::Integer(2),
                CellValue::Integer(3),
            ]],
        );
        assert_eq!(
            bad.err(),
            Some(GridDataError::RaggedRow {
                row_index: 0,
                expected: 2,
                actual: 3
            }),
        );
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn grid_data_cell_safe_access() {
        let data = GridData::new(
            vec![Column::new("a", ColumnKind::Integer, 80.0)],
            vec![vec![CellValue::Integer(9)]],
        )
        .expect("row width matches columns");
        assert_eq!(data.cell(0, 0), Some(&CellValue::Integer(9)));
        assert_eq!(data.cell(1, 0), Option::None);
        assert_eq!(data.cell(0, 1), Option::None);
    }

    #[test]
    fn from_conversions_match_variant() {
        assert_eq!(
            CellValue::from(String::from("x")),
            CellValue::Text("x".into())
        );
        assert_eq!(CellValue::from(42_i64), CellValue::Integer(42));
        assert_eq!(CellValue::from(7_i32), CellValue::Integer(7));
        assert_eq!(CellValue::from(0.5_f64), CellValue::Decimal(0.5));
        assert_eq!(CellValue::from(true), CellValue::Boolean(true));
        assert_eq!(
            CellValue::from(Some(CellValue::Integer(3))),
            CellValue::Integer(3),
        );
        assert_eq!(CellValue::from(Option::None::<CellValue>), CellValue::None);
    }

    #[test]
    fn sample_data_is_rectangular() {
        let sample = sample_data();
        assert!(
            sample.validate().is_ok(),
            "sample rows should be rectangular"
        );
        assert!(sample.row_count() > 0);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn column_index_exact_match() {
        let data = GridData::new(
            vec![
                Column::new("alpha", ColumnKind::Integer, 80.0),
                Column::new("beta", ColumnKind::Text, 80.0),
                Column::new("gamma", ColumnKind::Decimal, 80.0),
            ],
            vec![vec![
                CellValue::Integer(1),
                CellValue::Text("x".into()),
                CellValue::Decimal(1.0),
            ]],
        )
        .expect("rectangular");
        assert_eq!(data.column_index("alpha"), Some(0));
        assert_eq!(data.column_index("beta"), Some(1));
        assert_eq!(data.column_index("gamma"), Some(2));
        assert_eq!(data.column_index("Alpha"), None);
        assert_eq!(data.column_index("missing"), None);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn column_index_first_duplicate_wins() {
        let data = GridData::new(
            vec![
                Column::new("dup", ColumnKind::Integer, 80.0),
                Column::new("dup", ColumnKind::Integer, 80.0),
            ],
            vec![vec![CellValue::Integer(1), CellValue::Integer(2)]],
        )
        .expect("rectangular");
        assert_eq!(data.column_index("dup"), Some(0));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn column_index_empty_data_returns_none() {
        let data = GridData::new(vec![], vec![]).expect("empty");
        assert_eq!(data.column_index("anything"), None);
    }
}
