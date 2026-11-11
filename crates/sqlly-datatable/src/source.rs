//! Row sources too large to hold in memory, for the pivot and chart tabs.
//!
//! A grid in windowed-row mode (see [`crate::RowWindow`]) only holds the rows
//! around the viewport, so the pivot and chart cannot aggregate its resident
//! rows. A host that keeps the full set elsewhere (a file on disk, say)
//! implements [`RowSource`] and hands it to
//! [`crate::SqllyDataTable::set_row_source`]. The pivot and chart then read
//! every row once per layout change on the background executor, holding
//! only their running totals, and show progress while they do.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::data::CellValue;

/// Why a [`RowSource`] could not finish a scan. The text is a diagnostic for
/// the host's logs; the pivot and chart show their own localized message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowSourceError(pub String);

impl std::fmt::Display for RowSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RowSourceError {}

/// A complete result set that can be read start to finish, any number of
/// times, from any thread.
///
/// Rows must have the grid's column layout (the same columns, in the same
/// order, converted the same way as the rows the grid pages in), because
/// pivot and chart configs refer to grid column indices.
pub trait RowSource: Send + Sync + 'static {
    /// Number of rows a full scan visits. Drives progress and sizing (the
    /// chart's line downsampling buckets rows by it).
    fn row_count(&self) -> usize;

    /// Visit every row in order. `visit` returns `false` to stop early (the
    /// scan was cancelled or needs no more rows); stopping that way is not
    /// an error.
    ///
    /// # Errors
    ///
    /// Returns [`RowSourceError`] when the rows cannot be read.
    fn scan(&self, visit: &mut dyn FnMut(&[CellValue]) -> bool) -> Result<(), RowSourceError>;
}

/// The rows behind a pivot cell or chart mark, handed to the host when the
/// grid holds only a window of the result and cannot filter itself (see
/// [`crate::SqllyDataTable::set_row_source_drill_handler`]). The host reads
/// its [`RowSource`] and keeps the rows [`Self::matches`] accepts; the test
/// reproduces exactly how the pivot or chart grouped them (formatted labels,
/// blank groups, Filters-zone choices, histogram bins).
#[derive(Clone)]
pub struct RowSourceDrill {
    label: String,
    predicate: RowPredicate,
}

/// A thread-safe test over one full row.
type RowPredicate = Arc<dyn Fn(&[CellValue]) -> bool + Send + Sync>;

impl RowSourceDrill {
    /// A drill described by `label` (banner text such as
    /// `region = East, year = 2024`) that keeps the rows `predicate` accepts.
    pub fn new(
        label: impl Into<String>,
        predicate: impl Fn(&[CellValue]) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            predicate: Arc::new(predicate),
        }
    }

    /// Banner text naming what the drill keeps.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Whether `row` (in the grid's column layout) is one of the drilled rows.
    #[must_use]
    pub fn matches(&self, row: &[CellValue]) -> bool {
        (self.predicate)(row)
    }
}

impl std::fmt::Debug for RowSourceDrill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowSourceDrill")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Host callback receiving a [`RowSourceDrill`]. Called outside the grid's
/// own update, so it may update the grid entity.
pub type RowSourceDrillHandler = std::rc::Rc<dyn Fn(RowSourceDrill, &mut gpui::App)>;

/// Host callback run when the user clears a host-shown drill banner (see
/// [`crate::SqllyDataTable::show_row_source_drill`]).
pub type RowSourceDrillClearHandler = std::rc::Rc<dyn Fn(&mut gpui::App)>;

/// Shared between a background scan and its owner: rows read so far, a
/// cancel request, and completion.
#[derive(Debug, Default)]
pub(crate) struct ScanControl {
    scanned: AtomicUsize,
    cancelled: AtomicBool,
    done: AtomicBool,
}

impl ScanControl {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub(crate) fn finish(&self) {
        self.done.store(true, Ordering::Release);
    }

    pub(crate) fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    pub(crate) fn scanned(&self) -> usize {
        self.scanned.load(Ordering::Relaxed)
    }

    fn set_scanned(&self, rows: usize) {
        self.scanned.store(rows, Ordering::Relaxed);
    }
}

/// Rows between progress updates and cancel checks.
const PROGRESS_STRIDE: usize = 4096;

/// One pass over `source` on behalf of `control`. `passes`/`pass` spread the
/// progress over several passes (the histogram reads the rows twice), so the
/// shown count rises once from zero to the total. Returns `Ok(false)` when
/// cancelled.
pub(crate) fn scan_with_progress(
    source: &dyn RowSource,
    control: &ScanControl,
    pass: usize,
    passes: usize,
    visit: &mut dyn FnMut(&[CellValue]),
) -> Result<bool, RowSourceError> {
    let total = source.row_count();
    let passes = passes.max(1);
    let base = total.saturating_mul(pass) / passes;
    let mut seen = 0usize;
    let mut stopped = false;
    source.scan(&mut |row| {
        if seen.is_multiple_of(PROGRESS_STRIDE) {
            if control.is_cancelled() {
                stopped = true;
                return false;
            }
            control.set_scanned(base + seen / passes);
        }
        seen += 1;
        visit(row);
        true
    })?;
    Ok(!stopped && !control.is_cancelled())
}

/// In-memory rows as a [`RowSource`]; used by tests and by hosts whose rows
/// already live in memory but want the background path anyway.
impl RowSource for Vec<Vec<CellValue>> {
    fn row_count(&self) -> usize {
        self.len()
    }

    fn scan(&self, visit: &mut dyn FnMut(&[CellValue]) -> bool) -> Result<(), RowSourceError> {
        for row in self {
            if !visit(row) {
                break;
            }
        }
        Ok(())
    }
}

/// The progress label shown while a background scan runs: `scanned` of
/// `total` rows as a whole percent.
pub(crate) fn progress_percent(scanned: usize, total: usize) -> usize {
    if total == 0 {
        return 0;
    }
    (scanned.saturating_mul(100) / total).min(99)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(n: usize) -> Vec<Vec<CellValue>> {
        (0..n)
            .map(|i| vec![CellValue::Integer(i64::try_from(i).unwrap_or_default())])
            .collect()
    }

    #[test]
    fn scan_visits_every_row_and_reports_completion() {
        let source = rows(10_000);
        let control = ScanControl::new();
        let mut count = 0;
        let complete = scan_with_progress(&source, &control, 0, 1, &mut |_| count += 1);
        assert_eq!(complete, Ok(true));
        assert_eq!(count, 10_000);
    }

    #[test]
    fn cancelled_scan_stops_early_and_says_so() {
        let source = rows(100_000);
        let control = ScanControl::new();
        let mut count = 0;
        let complete = scan_with_progress(&source, &control, 0, 1, &mut |_| {
            count += 1;
            if count == 10 {
                control.cancel();
            }
        });
        assert_eq!(complete, Ok(false));
        assert!(count < 100_000, "stopped at the next progress stride");
    }

    #[test]
    fn progress_spans_passes_without_going_backwards() {
        let source = rows(8192);
        let control = ScanControl::new();
        let _ = scan_with_progress(&source, &control, 0, 2, &mut |_| {});
        let after_first = control.scanned();
        let _ = scan_with_progress(&source, &control, 1, 2, &mut |_| {});
        assert!(control.scanned() >= after_first);
        assert!(control.scanned() <= 8192);
    }

    #[test]
    fn percent_never_claims_done_early() {
        assert_eq!(progress_percent(0, 0), 0);
        assert_eq!(progress_percent(50, 100), 50);
        assert_eq!(progress_percent(100, 100), 99);
    }
}
