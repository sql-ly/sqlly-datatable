# Cross-platform style verification

Manual visual checklist for the theming and styling work (theme families,
sort affordances, icons, typography, hardening). Run the sample app on each
OS and tick what you see working.

```sh
./run-sample.sh                          # normal run (100k rows)
SQLLY_SAMPLE_ROWS=0 ./run-sample.sh      # empty-result state
```

Legend: `[x]` verified · `[ ]` not yet verified. All macOS `[x]` marks were
verified by screenshot during development on macOS (Menlo, retina, dark-mode
host OS). **Every macOS `[ ]` below is explicitly NOT yet verified — check
those first.** Linux and Windows are entirely unverified; the font stack
(DejaVu Sans Mono / Consolas), the `char_width` advance ratios, and OS
appearance following all have per-platform code paths worth real eyes.

## 1. OS appearance mode × app theme

Every combination of OS mode and app theme family, on each view. High-contrast
modes: macOS *System Settings → Accessibility → Display → Increase contrast*;
Windows *Contrast themes* (Aquatic = dark, Desert = light); Linux the desktop's
HighContrast theme (e.g. GNOME a11y setting). Note: the crate follows only the
OS light/dark signal — high-contrast modes resolve to the matching light/dark
palette, so these checks are about **legibility under the mode**, not a
dedicated high-contrast palette (which does not exist yet).

| OS mode × app theme | macOS | Linux | Windows |
|---|---|---|---|
| OS Light × Neutral — grid tab (white canvas, azure accent) | [x] | [ ] | [ ] |
| OS Light × Neutral — pivot tab | [x] | [ ] | [ ] |
| OS Light × Signature — grid tab (teal-tinted chrome) | [x] | [ ] | [ ] |
| OS Light × Signature — pivot tab | [x] | [ ] | [ ] |
| OS Dark × Neutral — grid tab | [x] | [ ] | [ ] |
| OS Dark × Neutral — pivot tab | [x] | [ ] | [ ] |
| OS Dark × Signature — grid tab (teal accents, warm-red negatives) | [x] | [ ] | [ ] |
| OS Dark × Signature — pivot tab (teal chips, grand-total hierarchy) | [x] | [ ] | [ ] |
| OS High-contrast Light × Neutral — grid + pivot legible | [ ] | [ ] | [ ] |
| OS High-contrast Light × Signature — grid + pivot legible | [ ] | [ ] | [ ] |
| OS High-contrast Dark × Neutral — grid + pivot legible | [ ] | [ ] | [ ] |
| OS High-contrast Dark × Signature — grid + pivot legible | [ ] | [ ] | [ ] |
| macOS "vibrant" appearances (translucent host window, `VibrantLight`/`VibrantDark`) resolve to the right variant | [ ] | — | — |

## 1b. Appearance switching & the theme switcher

| Check | macOS | Linux | Windows |
|---|---|---|---|
| OS light→dark switch rethemes live, without restart | [x] | [ ] | [ ] |
| OS dark→light switch rethemes live | [x] | [ ] | [ ] |
| Toggling OS high-contrast while running doesn't break theming (falls back to light/dark) | [ ] | [ ] | [ ] |
| Toolbar switcher: Signature → Neutral swaps all surfaces (toolbar, grid, pivot, sidebar, menus) | [x] | [ ] | [ ] |
| Toolbar switcher: Neutral → Signature swaps back | [x] | [ ] | [ ] |
| Theme switched at runtime + OS mode switched afterwards: new family's variant applies (family sticks) | [ ] | [ ] | [ ] |
| Toolbar switcher: hover state on the inactive segment | [ ] | [ ] | [ ] |

## 2. Typography & fonts

| Check | macOS | Linux | Windows |
|---|---|---|---|
| Cell text renders in a real monospace face (Menlo / DejaVu / Consolas), not a fallback | [x] | [ ] | [ ] |
| Column-header labels render **bold** (visibly heavier than cell text) | [x] | [ ] | [ ] |
| Pivot Grand Total row/column renders **bold** | [x] | [ ] | [ ] |
| Flat-grid group-header rows render **bold** (group any column via right-click → "Group by this column") | [ ] | [ ] | [ ] |
| NULL cells render *italic* with the amber null background (needs null data; not present in sample) | [ ] | [ ] | [ ] |
| Right-aligned numbers align cleanly at the cell edge (char-width ratio is per-OS) | [x] | [ ] | [ ] |
| Header labels ellipsize (don't overflow) on narrow columns after resize-dragging a column very small | [ ] | [ ] | [ ] |

## 3. Sort affordances & icons (grid)

| Check | macOS | Linux | Windows |
|---|---|---|---|
| Header row is quiet at rest (no boxes/glyphs on unsorted, unhovered columns) | [x] | [ ] | [ ] |
| Hovering a column header reveals the outlined button with `-` hint | [x] | [ ] | [ ] |
| Sorted column shows bold accent `↑` (asc) / `↓` (desc), 33% larger than cell text | [x] | [ ] | [ ] |
| Sort glyph swaps `↑`→`↓`→off when cycling the sort button | [ ] | [ ] | [ ] |
| Active-filter 🔽 emoji (right-click header → Filter…, apply one) paints next to the sort button | [ ] | [ ] | [ ] |
| Drag-selection marquee shows a 1px accent outline while dragging | [ ] | [ ] | [ ] |
| Grouped column shows the 3px accent underline in its header | [ ] | [ ] | [ ] |

## 4. Sort affordances & icons (pivot)

| Check | macOS | Linux | Windows |
|---|---|---|---|
| Hovering an innermost column header (e.g. "1".."5") reveals the `-` hint | [x] | [ ] | [ ] |
| Clicking it sorts rows by that column; bold accent `↑`/`↓` appears | [x] | [ ] | [ ] |
| Corner (top-left, "narrative") hover shows `-`; click cycles row-label sort with `↑`/`↓` | [ ] | [ ] | [ ] |
| Grand Total column header: hover hint + sort glyph when sorting by subtotals | [ ] | [ ] | [ ] |
| Subtotal "Total" columns (enable "Column subtotal columns" in sidebar): hover hint + sort glyph | [ ] | [ ] | [ ] |
| Single-value layout (remove the Columns field): value caption column shows hover hint + sort glyph | [ ] | [ ] | [ ] |
| Column-label sort (context menu on a column header → sort labels): `↑`/`↓` next to "currency_id" caption | [ ] | [ ] | [ ] |
| Sorted-state glyph doesn't collide with right-aligned header labels (set a right-aligned per-field format first) | [ ] | [ ] | [ ] |

## 5. Pivot sidebar

| Check | macOS | Linux | Windows |
|---|---|---|---|
| Field chips: uniform height and perfectly even gaps (measured 24px chips / 28px pitch on macOS) | [x] | [ ] | [ ] |
| Chip gaps stay even while scrolling the field list | [ ] | [ ] | [ ] |
| Long/hostile chip labels ("interés compuesto…", emoji/CJK/Arabic) ellipsize with hover tooltip | [ ] | [ ] | [ ] |
| Filters-zone chip shows 🔽 while its filter is active (open the trans_part checklist, uncheck a value) | [ ] | [ ] | [ ] |
| Drop-zone hover highlight while dragging a chip over each zone | [ ] | [ ] | [ ] |
| Drag ghost matches chip styling (24px, themed) | [ ] | [ ] | [ ] |
| Per-field format dialog (double-click a zone chip): themed dialog over the darkened scrim (`overlay_scrim`) | [ ] | [ ] | [ ] |
| Display and export: 16px checkboxes, accent ✓ when checked, readable labels/buttons, hover states | [ ] | [ ] | [ ] |

## 6. Menus & popups

| Check | macOS | Linux | Windows |
|---|---|---|---|
| Right-click context menu: themed surface, readable labels, separators | [x] | [ ] | [ ] |
| Menu item hover highlight (`menu_hover_bg`) | [ ] | [ ] | [ ] |
| Filter popup: themed, cursor visible, emoji/CJK input doesn't misplace the caret badly | [ ] | [ ] | [ ] |
| Menus escape the grid bounds near window edges without clipping | [ ] | [ ] | [ ] |

## 7. Hardening / hostile data

| Check | macOS | Linux | Windows |
|---|---|---|---|
| Emoji ZWJ + CJK + RTL narrative values render in grid cells without panic | [x] | [ ] | [ ] |
| Same values as pivot row labels, truncated cleanly | [x] | [ ] | [ ] |
| Over-long narrative truncates at the cell edge (no overflow into next column) | [x] | [ ] | [ ] |
| `SQLLY_SAMPLE_ROWS=0`: grid shows centered muted "No rows" hint (position/weight is a best guess — judge it) | [ ] | [ ] | [ ] |
| `SQLLY_SAMPLE_ROWS=0`: pivot tab shows its hints, no panic | [ ] | [ ] | [ ] |
| Window resized to 600×400 minimum with sidebar open: nothing clips or overlaps | [ ] | [ ] | [ ] |
| Scrollbar thumbs visible against tracks in both themes (vertical + horizontal) | [x] | [ ] | [ ] |

## 8. Platform-specific code paths (Linux / Windows only)

| Check | Linux | Windows |
|---|---|---|
| Font resolves to DejaVu Sans Mono (Linux) / Consolas (Windows) — bold + italic actually render | [ ] | [ ] |
| `char_width` ratio (0.6022 / 0.55) — right-aligned numbers and menu widths look correct | [ ] | [ ] |
| OS appearance following works (`WindowAppearance` reporting on that platform) | [ ] | [ ] |
| WCAG contrast test suite passes: `cargo test -p sqlly-datatable wcag` | [ ] | [ ] |
