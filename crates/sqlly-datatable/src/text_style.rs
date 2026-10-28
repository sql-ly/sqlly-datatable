//! Host-supplied colored text runs inside cells (syntax coloring).
//!
//! A host that wants, say, JSON keys and strings in different colors sets
//! [`crate::GridConfig::cell_text_styler`] to a [`CellTextStyler`]. The grid
//! asks it once per column whether the column is styled, then — for each
//! visible cell of a styled column — for [`TextSpan`]s over the cell's
//! display text. To keep painting cheap:
//!
//! * the styler only ever sees the **visible prefix** of a cell (a few
//!   characters more than fit the column), never a whole multi-megabyte
//!   value;
//! * spans are **cached per cell** and reused frame to frame while the
//!   cell's visible prefix is unchanged, so scrolling and repainting do not
//!   re-tokenize.
//!
//! A conditional-formatting text color beats the spans (the user's rule
//! wins), and NULL placeholders are never styled.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, Mutex};

use gpui::Hsla;

use crate::conditional::RgbaColor;

/// One colored run of a cell's display text: a byte range (on `char`
/// boundaries) and its color. Text outside every span paints in the cell's
/// normal color.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpan {
    pub range: Range<usize>,
    pub color: RgbaColor,
}

impl TextSpan {
    #[must_use]
    pub fn new(range: Range<usize>, color: RgbaColor) -> Self {
        Self { range, color }
    }
}

/// Computes colored runs for cell text. Implemented by the host.
pub trait CellTextStyler: Send + Sync {
    /// Whether cells of the column named `column` may carry spans. Called
    /// once per column when the config or data changes, not per frame.
    fn styles_column(&self, column: &str) -> bool;

    /// Spans over `text` — the visible prefix of one cell's display text in
    /// the column named `column`. Return an empty vector for "no styling".
    /// Spans may be unsorted or overlap; the grid sorts them, drops
    /// overlaps, and clips them to the text.
    fn spans(&self, column: &str, text: &str) -> Vec<TextSpan>;
}

/// A shareable handle to a [`CellTextStyler`]. Two handles are equal only
/// when they point at the same styler (so a rebuilt config with the same
/// styler compares equal).
#[derive(Clone)]
pub struct CellStyler(pub Arc<dyn CellTextStyler>);

impl CellStyler {
    pub fn new(styler: impl CellTextStyler + 'static) -> Self {
        Self(Arc::new(styler))
    }
}

impl PartialEq for CellStyler {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for CellStyler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CellStyler(..)")
    }
}

/// How many characters past the column's visible width the styler sees, so
/// a token cut at the cell edge still gets its color.
const PREFIX_SLACK_CHARS: usize = 8;

/// Never hand the styler more than this many characters, however wide the
/// column.
const MAX_PREFIX_CHARS: usize = 4096;

/// Cached cells before the cache starts over (bounded memory).
const MAX_CACHED_CELLS: usize = 16_384;

/// The visible prefix of `text` for a column `width` px wide at `char_width`
/// px per character: enough characters to fill it plus a little slack.
#[must_use]
pub fn visible_prefix(text: &str, width: f32, char_width: f32) -> &str {
    let fit = if char_width > 0.0 && width.is_finite() && width > 0.0 {
        (width / char_width).ceil() as usize
    } else {
        0
    };
    let limit = (fit + PREFIX_SLACK_CHARS).min(MAX_PREFIX_CHARS);
    match text.char_indices().nth(limit) {
        Some((byte, _)) => &text[..byte],
        None => text,
    }
}

/// The styled columns and the per-cell span cache for one grid.
#[derive(Default)]
pub(crate) struct TextStyling {
    styler: Option<CellStyler>,
    /// Indexed by data column; `true` when the styler colors that column.
    columns: Vec<bool>,
    cache: Mutex<HashMap<(usize, usize), CachedSpans>>,
}

impl fmt::Debug for TextStyling {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextStyling")
            .field("styler", &self.styler)
            .field("columns", &self.columns)
            .finish_non_exhaustive()
    }
}

struct CachedSpans {
    fingerprint: u64,
    spans: Arc<[TextSpan]>,
}

fn fingerprint(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

impl TextStyling {
    pub(crate) fn resolve(styler: Option<&CellStyler>, columns: &[crate::data::Column]) -> Self {
        let styler = styler.cloned();
        let columns = match &styler {
            Some(s) => columns.iter().map(|c| s.0.styles_column(&c.name)).collect(),
            None => Vec::new(),
        };
        Self {
            styler,
            columns,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn styler(&self) -> Option<&CellStyler> {
        self.styler.as_ref()
    }

    pub(crate) fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Whether any column is styled (one cheap check per cell otherwise).
    pub(crate) fn is_active(&self) -> bool {
        self.columns.iter().any(|on| *on)
    }

    /// Spans for the cell at (`row`, `col`) whose visible prefix is
    /// `prefix`, from the cache when the prefix is unchanged.
    pub(crate) fn spans_for(
        &self,
        row: usize,
        col: usize,
        column_name: &str,
        prefix: &str,
    ) -> Option<Arc<[TextSpan]>> {
        if !self.columns.get(col).copied().unwrap_or(false) {
            return None;
        }
        let styler = self.styler.as_ref()?;
        let fp = fingerprint(prefix);
        if let Ok(cache) = self.cache.lock() {
            if let Some(hit) = cache.get(&(row, col)).filter(|c| c.fingerprint == fp) {
                return Some(Arc::clone(&hit.spans));
            }
        }
        let spans: Arc<[TextSpan]> =
            normalize_spans(styler.0.spans(column_name, prefix), prefix.len()).into();
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() >= MAX_CACHED_CELLS {
                cache.clear();
            }
            cache.insert(
                (row, col),
                CachedSpans {
                    fingerprint: fp,
                    spans: Arc::clone(&spans),
                },
            );
        }
        Some(spans)
    }

    #[cfg(test)]
    fn cached_cells(&self) -> usize {
        self.cache.lock().map(|c| c.len()).unwrap_or(0)
    }
}

/// Sort spans, clip them to `len`, and drop empty or overlapping ones (the
/// earlier-starting span keeps the overlap).
#[must_use]
pub fn normalize_spans(mut spans: Vec<TextSpan>, len: usize) -> Vec<TextSpan> {
    spans.sort_by_key(|s| (s.range.start, s.range.end));
    let mut out: Vec<TextSpan> = Vec::with_capacity(spans.len());
    let mut cursor = 0;
    for mut span in spans {
        span.range.start = span.range.start.max(cursor);
        span.range.end = span.range.end.min(len);
        if span.range.start >= span.range.end {
            continue;
        }
        cursor = span.range.end;
        out.push(span);
    }
    out
}

/// Split the first `len` bytes of `text` into `(byte_len, color)` runs that
/// cover it exactly: span colors where spans lie, `default` elsewhere. Span
/// ends that fall inside a character are moved back to its start so every
/// run boundary is a `char` boundary.
#[must_use]
pub fn color_runs(text: &str, len: usize, spans: &[TextSpan], default: Hsla) -> Vec<(usize, Hsla)> {
    let len = crate::grid::paint::floor_char_boundary(text, len.min(text.len()));
    let mut runs: Vec<(usize, Hsla)> = Vec::new();
    let mut push = |n: usize, color: Hsla| {
        if n == 0 {
            return;
        }
        match runs.last_mut() {
            Some((last_n, last_color)) if *last_color == color => *last_n += n,
            _ => runs.push((n, color)),
        }
    };
    let mut cursor = 0;
    for span in spans {
        let start = crate::grid::paint::floor_char_boundary(text, span.range.start.min(len));
        let end = crate::grid::paint::floor_char_boundary(text, span.range.end.min(len));
        if start < cursor || start >= end {
            continue;
        }
        push(start - cursor, default);
        push(end - start, span.color.to_hsla());
        cursor = end;
    }
    push(len - cursor, default);
    runs
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::data::{Column, ColumnKind};

    const RED: RgbaColor = RgbaColor::new(1.0, 0.0, 0.0, 1.0);
    const BLUE: RgbaColor = RgbaColor::new(0.0, 0.0, 1.0, 1.0);

    fn default_color() -> Hsla {
        gpui::hsla(0.0, 0.0, 0.5, 1.0)
    }

    struct CountingStyler {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    fn counting() -> (CellStyler, Arc<std::sync::atomic::AtomicUsize>) {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let styler = CellStyler::new(CountingStyler {
            calls: Arc::clone(&calls),
        });
        (styler, calls)
    }

    impl CellTextStyler for CountingStyler {
        fn styles_column(&self, column: &str) -> bool {
            column == "doc"
        }
        fn spans(&self, _column: &str, text: &str) -> Vec<TextSpan> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            vec![TextSpan::new(0..text.len().min(1), RED)]
        }
    }

    fn column(name: &str) -> Column {
        Column {
            name: name.into(),
            kind: ColumnKind::Text,
            width: 100.0,
        }
    }

    #[test]
    fn runs_cover_the_text_exactly_in_order() {
        let text = r#"{"a": 1}"#;
        let spans = normalize_spans(
            vec![TextSpan::new(6..7, BLUE), TextSpan::new(1..4, RED)],
            text.len(),
        );
        let runs = color_runs(text, text.len(), &spans, default_color());
        let total: usize = runs.iter().map(|(n, _)| n).sum();
        assert_eq!(total, text.len());
        assert_eq!(
            runs,
            vec![
                (1, default_color()),
                (3, RED.to_hsla()),
                (2, default_color()),
                (1, BLUE.to_hsla()),
                (1, default_color()),
            ]
        );
    }

    #[test]
    fn runs_clip_to_a_truncated_length_on_char_boundaries() {
        let text = "\"é€\" tail";
        let spans = vec![TextSpan::new(0..text.len(), RED)];
        // Cut in the middle of `€` (bytes 3..6): the run stops before it.
        let runs = color_runs(text, 4, &spans, default_color());
        assert_eq!(runs, vec![(3, RED.to_hsla())]);
        assert!(color_runs("", 0, &spans, default_color()).is_empty());
    }

    #[test]
    fn overlapping_and_out_of_range_spans_are_tamed() {
        let spans = normalize_spans(
            vec![
                TextSpan::new(2..8, BLUE),
                TextSpan::new(0..4, RED),
                TextSpan::new(9..20, RED),
                TextSpan::new(5..5, BLUE),
            ],
            10,
        );
        assert_eq!(
            spans,
            vec![
                TextSpan::new(0..4, RED),
                TextSpan::new(4..8, BLUE),
                TextSpan::new(9..10, RED),
            ]
        );
    }

    #[test]
    fn visible_prefix_is_bounded_by_the_column_width() {
        let long = "x".repeat(100_000);
        assert_eq!(
            visible_prefix(&long, 80.0, 8.0).len(),
            10 + PREFIX_SLACK_CHARS
        );
        assert_eq!(visible_prefix(&long, 1e9, 1.0).len(), MAX_PREFIX_CHARS);
        assert_eq!(visible_prefix("short", 800.0, 8.0), "short");
        // Multibyte text is cut on a char boundary.
        let wide = "é".repeat(50);
        assert_eq!(
            visible_prefix(&wide, 16.0, 8.0).chars().count(),
            2 + PREFIX_SLACK_CHARS
        );
    }

    #[test]
    fn spans_are_cached_per_cell_until_the_text_changes() {
        let (styler, calls) = counting();
        let calls = || calls.load(std::sync::atomic::Ordering::SeqCst);
        let styling = TextStyling::resolve(Some(&styler), &[column("id"), column("doc")]);
        assert!(styling.is_active());
        assert!(
            styling.spans_for(0, 0, "id", "1").is_none(),
            "unstyled column"
        );
        assert_eq!(calls(), 0);
        let first = styling.spans_for(0, 1, "doc", "{}").expect("styled");
        let again = styling.spans_for(0, 1, "doc", "{}").expect("styled");
        assert!(Arc::ptr_eq(&first, &again), "second paint reuses the cache");
        assert_eq!(calls(), 1);
        let changed = styling.spans_for(0, 1, "doc", "[]").expect("styled");
        assert!(!Arc::ptr_eq(&first, &changed), "new text re-tokenizes");
        assert_eq!(calls(), 2);
        assert_eq!(styling.cached_cells(), 1);
        let none = TextStyling::resolve(None, &[column("doc")]);
        assert!(!none.is_active());
        assert!(none.spans_for(0, 0, "doc", "{}").is_none());
    }

    #[test]
    fn grid_state_follows_the_configured_styler() {
        use crate::{CellValue, GridConfig, GridData, GridState};
        let data = GridData::new(
            vec![column("id"), column("doc")],
            vec![vec![CellValue::Integer(1), CellValue::Text("{}".into())]],
        )
        .expect("rectangular");
        let (styler, _) = counting();
        let mut state = gpui::TestAppContext::single()
            .update(|cx| GridState::new(data, GridConfig::default(), cx.focus_handle()));
        assert!(!state.text_styling.is_active());
        state.set_config(GridConfig {
            cell_text_styler: Some(styler.clone()),
            ..GridConfig::default()
        });
        assert!(state.text_styling.is_active());
        let cached = Arc::clone(&state.text_styling);
        assert!(cached.spans_for(0, 1, "doc", "{}").is_some());
        // Re-applying the same styler keeps the warm cache.
        state.set_config(GridConfig {
            cell_text_styler: Some(styler),
            ..GridConfig::default()
        });
        assert!(Arc::ptr_eq(&cached, &state.text_styling));
        state.set_config(GridConfig::default());
        assert!(!state.text_styling.is_active());
    }

    #[test]
    fn styler_handles_compare_by_identity() {
        let (a, _) = counting();
        let (b, _) = counting();
        assert_eq!(a, a.clone());
        assert_ne!(a, b);
    }
}
