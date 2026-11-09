//! Shaped cell text retained across frames.
//!
//! The painter shapes every visible cell on every frame. gpui's own line
//! cache makes a repeat shape cheap, but not free: the text is re-hashed
//! and re-compared, a `SharedString` is allocated per call, and a cell that
//! overflows its column shapes three times (full, ellipsis, fitted). Keeping
//! the finished [`ShapedLine`] per cell turns a steady scroll into one hash
//! and one map lookup per cell; only rows entering the viewport shape.
//!
//! Entries are validated by a fingerprint of everything the shape depends
//! on (text, color, face, size, box width, syntax spans), so no explicit
//! invalidation is needed when data, formats, theme, or widths change: the
//! fingerprint misses and the cell reshapes.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use gpui::{Hsla, ShapedLine};

/// Upper bound on retained cells; the map is cleared when it is reached, so
/// a long scroll through a wide result set cannot grow memory without limit.
const MAX_ENTRIES: usize = 50_000;

struct Entry {
    fingerprint: u64,
    shaped: Option<Arc<ShapedLine>>,
}

#[derive(Default)]
pub(crate) struct ShapeCache {
    entries: Mutex<HashMap<(usize, usize), Entry>>,
}

impl std::fmt::Debug for ShapeCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.entries.lock().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("ShapeCache").field("entries", &len).finish()
    }
}

impl ShapeCache {
    /// The shaped line for the cell at `key` (source row, column), shaping
    /// with `shape` when the cached entry is missing or its fingerprint
    /// differs.
    pub(crate) fn get_or_shape(
        &self,
        key: (usize, usize),
        fingerprint: u64,
        shape: impl FnOnce() -> Option<ShapedLine>,
    ) -> Option<Arc<ShapedLine>> {
        if let Ok(map) = self.entries.lock() {
            if let Some(entry) = map.get(&key) {
                if entry.fingerprint == fingerprint {
                    return entry.shaped.clone();
                }
            }
        }
        let shaped = shape().map(Arc::new);
        if let Ok(mut map) = self.entries.lock() {
            if map.len() >= MAX_ENTRIES {
                map.clear();
            }
            map.insert(
                key,
                Entry {
                    fingerprint,
                    shaped: shaped.clone(),
                },
            );
        }
        shaped
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.lock().map(|m| m.len()).unwrap_or(0)
    }
}

/// Fingerprint of every input the shaped line depends on. `spans` are
/// identified by address and length: the span cache hands out the same
/// `Arc` while a cell's visible prefix is unchanged, and a recomputed
/// (equal) span set merely costs one reshape.
pub(crate) fn fingerprint(
    text: &str,
    color: Hsla,
    max_w: f32,
    italic: bool,
    bold: bool,
    font_size: f32,
    spans: Option<&[crate::text_style::TextSpan]>,
) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    color.h.to_bits().hash(&mut h);
    color.s.to_bits().hash(&mut h);
    color.l.to_bits().hash(&mut h);
    color.a.to_bits().hash(&mut h);
    max_w.to_bits().hash(&mut h);
    italic.hash(&mut h);
    bold.hash(&mut h);
    font_size.to_bits().hash(&mut h);
    match spans {
        Some(s) => {
            (s.as_ptr() as usize).hash(&mut h);
            s.len().hash(&mut h);
        }
        None => 0usize.hash(&mut h),
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_lookup_with_same_fingerprint_does_not_reshape() {
        let cache = ShapeCache::default();
        let mut shapes = 0;
        let first = cache.get_or_shape((3, 1), 42, || {
            shapes += 1;
            Some(ShapedLine::default())
        });
        let second = cache.get_or_shape((3, 1), 42, || {
            shapes += 1;
            Some(ShapedLine::default())
        });
        assert_eq!(shapes, 1);
        match (first, second) {
            (Some(a), Some(b)) => assert!(Arc::ptr_eq(&a, &b)),
            other => panic!("expected two cached lines, got {other:?}"),
        }
    }

    #[test]
    fn changed_fingerprint_reshapes_and_replaces() {
        let cache = ShapeCache::default();
        let mut shapes = 0;
        cache.get_or_shape((0, 0), 1, || {
            shapes += 1;
            Some(ShapedLine::default())
        });
        cache.get_or_shape((0, 0), 2, || {
            shapes += 1;
            Some(ShapedLine::default())
        });
        assert_eq!(shapes, 2);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn none_results_are_cached_too() {
        let cache = ShapeCache::default();
        let mut shapes = 0;
        for _ in 0..2 {
            let got = cache.get_or_shape((0, 0), 7, || {
                shapes += 1;
                None
            });
            assert!(got.is_none());
        }
        assert_eq!(shapes, 1);
    }

    #[test]
    fn cache_is_cleared_at_the_entry_cap() {
        let cache = ShapeCache::default();
        for i in 0..MAX_ENTRIES {
            cache.get_or_shape((i, 0), 0, || Some(ShapedLine::default()));
        }
        assert_eq!(cache.len(), MAX_ENTRIES);
        cache.get_or_shape((MAX_ENTRIES, 0), 0, || Some(ShapedLine::default()));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn fingerprint_tracks_every_shaping_input() {
        let base = fingerprint("a", gpui::black(), 10.0, false, false, 12.0, None);
        assert_eq!(
            base,
            fingerprint("a", gpui::black(), 10.0, false, false, 12.0, None)
        );
        assert_ne!(
            base,
            fingerprint("b", gpui::black(), 10.0, false, false, 12.0, None)
        );
        assert_ne!(
            base,
            fingerprint("a", gpui::white(), 10.0, false, false, 12.0, None)
        );
        assert_ne!(
            base,
            fingerprint("a", gpui::black(), 11.0, false, false, 12.0, None)
        );
        assert_ne!(
            base,
            fingerprint("a", gpui::black(), 10.0, true, false, 12.0, None)
        );
        assert_ne!(
            base,
            fingerprint("a", gpui::black(), 10.0, false, true, 12.0, None)
        );
        assert_ne!(
            base,
            fingerprint("a", gpui::black(), 10.0, false, false, 13.0, None)
        );
    }
}
