//! Grid-wide value search: a case-insensitive find across every column of the
//! currently visible rows, with match highlighting and next/previous
//! navigation. The host supplies the find-bar UI (an input plus a match
//! counter and next/prev buttons) and drives it through the public methods on
//! [`GridState`]; the paint layer highlights the matches and the focused match.
//!
//! Matches are computed over the rows resident in memory (respecting the active
//! filter and grouping), using each cell's *displayed* text — so what the user
//! searches is what they see. In windowed/spill-backed mode only resident rows
//! are searched, and the scan is capped at [`MAX_SEARCH_MATCHES`] so a
//! one-character query over a huge grid can never allocate without bound.

use crate::format::format_cell;
use crate::grid::state::GridState;

/// Hard cap on collected matches. Navigation and highlighting degrade
/// gracefully past this (the counter shows a trailing `+`).
pub const MAX_SEARCH_MATCHES: usize = 100_000;

impl GridState {
    /// Set (or clear) the grid-wide search query and recompute matches. An
    /// empty/blank query turns search off. The focused match is the first match
    /// at or after the current selection, wrapping to the top; it is scrolled
    /// into view.
    pub fn set_search_query(&mut self, query: &str) {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            self.clear_search();
            return;
        }
        self.search.query = query.to_string();
        self.recompute_search_matches();
        // Focus the first match at or after the current selection so opening
        // the find bar jumps forward from where the user is, not back to row 0.
        let anchor = self.search_anchor();
        self.search.active = self.search.matches.iter().position(|&m| m >= anchor).or(
            if self.search.matches.is_empty() {
                None
            } else {
                Some(0)
            },
        );
        self.scroll_active_match_into_view();
    }

    /// Turn search off and drop all matches.
    pub fn clear_search(&mut self) {
        self.search.query.clear();
        self.search.matches.clear();
        self.search.active = None;
        self.search.truncated = false;
    }

    /// Whether a search query is currently active.
    #[must_use]
    pub fn search_is_active(&self) -> bool {
        !self.search.query.is_empty()
    }

    /// Number of matching cells (capped at [`MAX_SEARCH_MATCHES`]).
    #[must_use]
    pub fn search_match_count(&self) -> usize {
        self.search.matches.len()
    }

    /// True when the scan stopped at the match cap.
    #[must_use]
    pub fn search_truncated(&self) -> bool {
        self.search.truncated
    }

    /// 1-based ordinal of the focused match, for a "3 of 42" counter.
    #[must_use]
    pub fn search_active_ordinal(&self) -> Option<usize> {
        self.search.active.map(|i| i + 1)
    }

    /// A ready-made "N of M" (or "No matches") counter label for the find bar.
    #[must_use]
    pub fn search_count_label(&self) -> String {
        let total = self.search.matches.len();
        if total == 0 {
            return "No matches".to_string();
        }
        let n = self.search_active_ordinal().unwrap_or(1);
        if self.search.truncated {
            format!("{n} of {total}+")
        } else {
            format!("{n} of {total}")
        }
    }

    /// Advance the focused match to the next one (wrapping) and scroll it into
    /// view. No-op when there are no matches.
    pub fn search_next(&mut self) {
        let len = self.search.matches.len();
        if len == 0 {
            return;
        }
        self.search.active = Some(match self.search.active {
            Some(i) => (i + 1) % len,
            None => 0,
        });
        self.scroll_active_match_into_view();
    }

    /// Move the focused match to the previous one (wrapping) and scroll it into
    /// view. No-op when there are no matches.
    pub fn search_prev(&mut self) {
        let len = self.search.matches.len();
        if len == 0 {
            return;
        }
        self.search.active = Some(match self.search.active {
            Some(i) => (i + len - 1) % len,
            None => len - 1,
        });
        self.scroll_active_match_into_view();
    }

    /// The `(display_row, col)` of the focused match, if any. Read by the paint
    /// layer to draw the stronger active-match highlight.
    #[must_use]
    pub fn search_active_cell(&self) -> Option<(usize, usize)> {
        self.search
            .active
            .and_then(|i| self.search.matches.get(i))
            .copied()
    }

    /// Read-only view of every matching `(display_row, col)` cell.
    #[must_use]
    pub fn search_matches(&self) -> &[(usize, usize)] {
        &self.search.matches
    }

    /// Recompute matches after a query, data, filter, or sort change. Public so
    /// the host can refresh highlights when the visible rows change under an
    /// active query.
    pub fn recompute_search_matches(&mut self) {
        self.search.matches.clear();
        self.search.match_set = std::sync::Arc::new(std::collections::HashSet::new());
        self.search.truncated = false;
        if self.search.query.is_empty() {
            self.search.active = None;
            return;
        }
        let needle = self.search.query.to_lowercase();
        let ncols = self.data.columns.len();
        // In windowed mode only the resident span can match; scanning the
        // full virtual range would burn O(total_rows) per recompute on a huge
        // spill-backed grid just to skip non-resident rows.
        let (start, end) = match self.window {
            Some(w) => (
                w.offset,
                w.offset
                    .saturating_add(self.data.rows.len())
                    .min(w.total_rows),
            ),
            None => (0, self.display_row_count()),
        };
        'outer: for dr in start..end {
            let Some(row_idx) = self.resident_row_for_display(dr) else {
                continue;
            };
            let Some(row) = self.data.rows.get(row_idx) else {
                continue;
            };
            for col in 0..ncols {
                let Some(cell) = row.get(col) else { continue };
                let fmt = &self.resolved_formats[col];
                // Mirror the paint layer: null cells render their placeholder
                // text (e.g. "NULL"), so that is what the search sees too.
                let text = if matches!(cell, crate::data::CellValue::None) {
                    fmt.null.text.clone()
                } else {
                    format_cell(cell, fmt).0
                };
                if text.to_lowercase().contains(&needle) {
                    self.search.matches.push((dr, col));
                    if self.search.matches.len() >= MAX_SEARCH_MATCHES {
                        self.search.truncated = true;
                        break 'outer;
                    }
                }
            }
        }
        // Keep the focused match in range after a recompute: clamp an
        // out-of-range index, and adopt the first match when none was focused
        // so the counter and the active highlight always agree.
        self.search.active = match self.search.active {
            _ if self.search.matches.is_empty() => None,
            Some(active) if active < self.search.matches.len() => Some(active),
            Some(_) => Some(self.search.matches.len() - 1),
            None => Some(0),
        };
        self.search.match_set = std::sync::Arc::new(self.search.matches.iter().copied().collect());
    }

    /// `(display_row, col)` used as the "start from here" point when focusing
    /// the first match: the current cell selection, else the top-left.
    fn search_anchor(&self) -> (usize, usize) {
        use crate::grid::selection::Selection;
        match &self.selection {
            Selection::Cell(r, c) => (*r, *c),
            Selection::Row(r) => (*r, 0),
            Selection::Column(c) => (0, *c),
            // Range/multi selections anchor at their first (top-left) cell so
            // "first match at or after the selection" holds for them too.
            Selection::CellRange(r1, c1, _, _) => (*r1, *c1),
            Selection::RowRange(r1, _) => (*r1, 0),
            Selection::Rows(rows) => (rows.first().copied().unwrap_or(0), 0),
            Selection::Columns(cols) => (0, cols.first().copied().unwrap_or(0)),
            Selection::Cells(cells) => cells.first().copied().unwrap_or((0, 0)),
            Selection::None => (0, 0),
        }
    }

    fn scroll_active_match_into_view(&mut self) {
        if let Some((row, col)) = self.search_active_cell() {
            self.ensure_visible(Some(row), Some(col));
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use crate::config::GridConfig;
    use crate::data::{CellValue, Column, ColumnKind, GridData};
    use crate::grid::state::GridState;

    fn grid() -> GridState {
        // Two columns; "alpha" appears in col 0 twice and col 1 once, so a
        // search for it must find matches across columns and rows in row-major
        // order.
        let data = GridData::new(
            vec![
                Column::new("a", ColumnKind::Text, 100.0),
                Column::new("b", ColumnKind::Text, 100.0),
            ],
            vec![
                vec![CellValue::Text("alpha".into()), CellValue::Text("x".into())],
                vec![
                    CellValue::Text("beta".into()),
                    CellValue::Text("Alpha!".into()),
                ],
                vec![
                    CellValue::Text("ALPHAbet".into()),
                    CellValue::Text("y".into()),
                ],
            ],
        )
        .expect("rectangular");
        gpui::TestAppContext::single()
            .update(|cx| GridState::new(data, GridConfig::default(), cx.focus_handle()))
    }

    #[test]
    fn sort_refreshes_matches_under_an_active_search() {
        // Regression: the refresh used to be unreachable for ungrouped grids
        // (dead code after rebuild_display_rows' early return), so sorting
        // left match positions keyed to the pre-sort display rows.
        let mut g = grid();
        g.set_search_query("beta");
        assert_eq!(g.search_matches(), &[(1, 0)]);
        // Sort col 0 descending: "beta" (row order: beta, alpha, ALPHAbet
        // under descending text compare of alpha/beta/ALPHAbet).
        g.sort = Some((0, crate::grid::selection::SortDirection::Descending));
        g.recompute();
        let sorted_pos = g
            .display_indices
            .iter()
            .position(|&src| src == 1)
            .expect("row 'beta' still displayed");
        assert_eq!(
            g.search_matches(),
            &[(sorted_pos, 0)],
            "match must re-key to the post-sort display row"
        );
    }

    #[test]
    fn filter_refreshes_matches_under_an_active_search() {
        use crate::filter::{ColumnFilter, FilterPredicate, TextOp};
        let mut g = grid();
        g.set_search_query("alpha");
        assert_eq!(g.search_match_count(), 3);
        // Filter col 0 to rows containing "beta" — only source row 1 remains,
        // whose col-1 value "Alpha!" still matches, now at display row 0.
        g.filters[0] = ColumnFilter {
            predicate: FilterPredicate::Text {
                op: TextOp::Contains,
                operand: "beta".into(),
            },
            ..ColumnFilter::default()
        };
        g.recompute();
        assert_eq!(g.search_matches(), &[(0, 1)]);
    }

    #[test]
    fn windowed_paging_rekeys_matches_to_virtual_rows_and_scans_only_residents() {
        let mut g = grid();
        // 100 virtual rows, resident window of 2 at offset 10.
        g.set_row_window(
            100,
            10,
            vec![
                vec![CellValue::Text("alpha".into()), CellValue::Text("x".into())],
                vec![CellValue::Text("y".into()), CellValue::Text("alpha".into())],
            ],
        )
        .expect("rectangular");
        g.set_search_query("alpha");
        // Matches are keyed by VIRTUAL display row (offset applied).
        assert_eq!(g.search_matches(), &[(10, 0), (11, 1)]);
        // Paging to a different window refreshes the matches for it.
        g.set_row_window(
            100,
            50,
            vec![vec![
                CellValue::Text("nothing".into()),
                CellValue::Text("alpha".into()),
            ]],
        )
        .expect("rectangular");
        assert_eq!(g.search_matches(), &[(50, 1)]);
    }

    #[test]
    fn append_rows_extends_matches_under_an_active_search() {
        let mut g = grid();
        g.set_search_query("alpha");
        assert_eq!(g.search_match_count(), 3);
        g.append_rows(vec![vec![
            CellValue::Text("alphine".into()),
            CellValue::Text("alpha".into()),
        ]])
        .expect("rectangular");
        assert_eq!(g.search_match_count(), 4);
        assert!(g.search_matches().contains(&(3, 1)));
    }

    #[test]
    fn null_placeholder_text_is_searchable() {
        // The paint layer renders `fmt.null.text` for null cells; the search
        // must see the same text the user sees.
        let data = GridData::new(
            vec![Column::new("a", ColumnKind::Text, 100.0)],
            vec![vec![CellValue::None], vec![CellValue::Text("x".into())]],
        )
        .expect("rectangular");
        let mut g = gpui::TestAppContext::single()
            .update(|cx| GridState::new(data, GridConfig::default(), cx.focus_handle()));
        let placeholder = g.resolved_formats[0].null.text.clone();
        assert!(!placeholder.is_empty(), "default null placeholder exists");
        g.set_search_query(&placeholder);
        assert_eq!(g.search_matches(), &[(0, 0)]);
    }

    #[test]
    fn range_selection_anchors_the_first_match() {
        let mut g = grid();
        // A range whose top-left sits past the first match: the initial focus
        // must land at-or-after it, not back at the top.
        g.selection = crate::grid::selection::Selection::CellRange(1, 0, 2, 1);
        g.set_search_query("alpha");
        assert_eq!(g.search_active_cell(), Some((1, 1)));
    }

    #[test]
    fn search_finds_case_insensitive_matches_across_columns_row_major() {
        let mut g = grid();
        g.set_search_query("alpha");
        // (0,0) alpha, (1,1) Alpha!, (2,0) ALPHAbet — row-major order.
        assert_eq!(g.search_matches(), &[(0, 0), (1, 1), (2, 0)]);
        assert_eq!(g.search_match_count(), 3);
        assert!(g.search_is_active());
        assert_eq!(g.search_active_ordinal(), Some(1));
        assert_eq!(g.search_active_cell(), Some((0, 0)));
    }

    #[test]
    fn next_and_prev_wrap_around() {
        let mut g = grid();
        g.set_search_query("alpha");
        assert_eq!(g.search_active_cell(), Some((0, 0)));
        g.search_next();
        assert_eq!(g.search_active_cell(), Some((1, 1)));
        g.search_next();
        assert_eq!(g.search_active_cell(), Some((2, 0)));
        g.search_next(); // wraps
        assert_eq!(g.search_active_cell(), Some((0, 0)));
        g.search_prev(); // wraps back
        assert_eq!(g.search_active_cell(), Some((2, 0)));
    }

    #[test]
    fn count_label_and_no_matches() {
        let mut g = grid();
        g.set_search_query("alpha");
        assert_eq!(g.search_count_label(), "1 of 3");
        g.set_search_query("nothing-here");
        assert_eq!(g.search_count_label(), "No matches");
        assert_eq!(g.search_match_count(), 0);
        assert_eq!(g.search_active_cell(), None);
    }

    #[test]
    fn blank_query_clears_search() {
        let mut g = grid();
        g.set_search_query("alpha");
        assert!(g.search_is_active());
        g.set_search_query("   ");
        assert!(!g.search_is_active());
        assert_eq!(g.search_match_count(), 0);
        assert!(g.search_matches().is_empty());
    }

    #[test]
    fn match_set_mirrors_matches_for_paint_lookup() {
        let mut g = grid();
        g.set_search_query("alpha");
        for m in g.search_matches() {
            assert!(g.search.match_set.contains(m));
        }
        assert_eq!(g.search.match_set.len(), g.search_match_count());
    }
}
