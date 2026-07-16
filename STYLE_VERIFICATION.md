# Cross-platform style verification

Manual visual checklist for the theming and styling work (theme families,
sort affordances, icons, typography, hardening). Run the sample app on each
OS and tick what you see working.

```sh
./run-sample.sh                          # normal run (100k rows)
SQLLY_SAMPLE_ROWS=0 ./run-sample.sh      # empty-result state
```

On Windows (PowerShell — `run-sample.sh` needs a POSIX shell, e.g. Git Bash):

```powershell
cargo run -p sqlly-datatable-sample                            # normal run
$env:SQLLY_SAMPLE_ROWS = "0"; cargo run -p sqlly-datatable-sample   # empty-result state
```

The sample data includes purpose-built fixtures for this checklist: hostile
narrative *values* (emoji ZWJ / CJK / RTL / over-long), one hostile column
*name* ("interés compuesto acumulado 💸 手数料 عمولة sobre el saldo" — for
chip/header truncation and tooltips), and scattered NULL cells in the
`field_*` columns (for the italic placeholder over `null_bg`).

Legend: `[x]` verified · `[ ]` not yet verified. All macOS `[x]` marks were
verified by screenshot during development on macOS (Menlo, retina, dark-mode
host OS). **Every macOS `[ ]` below is explicitly NOT yet verified — check
those first.** Windows was verified 2026-07-16 by driving the sample app and
inspecting screenshots (Consolas, 96 DPI, Windows 11); notes from that pass
are inline below. Linux is entirely unverified; the font stack
(DejaVu Sans Mono), the `char_width` advance ratios, and OS appearance
following all have per-platform code paths worth real eyes.

## 1. OS appearance mode × app theme

Every combination of OS mode and app theme family, on each view. High-contrast
modes: macOS *System Settings → Accessibility → Display → Increase contrast*;
Windows *Settings → Accessibility → Contrast themes* (Aquatic = dark,
Desert = light); Linux the desktop's HighContrast theme (e.g. GNOME a11y
setting). Note: the crate follows only the OS light/dark signal —
high-contrast modes resolve to the matching light/dark palette, so these
checks are about **legibility under the mode**, not a dedicated high-contrast
palette (which does not exist yet).

| OS mode × app theme | macOS | Linux | Windows |
|---|---|---|---|
| OS Light × Neutral — grid tab (white canvas, azure accent) | [x] | [ ] | [x] |
| OS Light × Neutral — pivot tab | [x] | [ ] | [x] |
| OS Light × Signature — grid tab (teal-tinted chrome) | [x] | [ ] | [x] |
| OS Light × Signature — pivot tab | [x] | [ ] | [x] |
| OS Dark × Neutral — grid tab | [x] | [ ] | [x] |
| OS Dark × Neutral — pivot tab | [x] | [ ] | [x] |
| OS Dark × Signature — grid tab (teal accents, warm-red negatives) | [x] | [ ] | [x] |
| OS Dark × Signature — pivot tab (teal chips, grand-total hierarchy) | [x] | [ ] | [x] |
| OS High-contrast Light × Neutral — grid + pivot legible | [ ] | [ ] | [ ] |
| OS High-contrast Light × Signature — grid + pivot legible | [ ] | [ ] | [ ] |
| OS High-contrast Dark × Neutral — grid + pivot legible | [ ] | [ ] | [x] |
| OS High-contrast Dark × Signature — grid + pivot legible | [ ] | [ ] | [x] |
| macOS "vibrant" appearances (translucent host window, `VibrantLight`/`VibrantDark`) resolve to the right variant | [ ] | — | — |

Windows high-contrast notes: verified with the system's dark contrast theme
enabled while the app was running (spot-checked Signature/pivot and
Neutral/grid; the app resolves to the same dark palettes as the normal dark
rows). The light contrast theme (Desert) still needs a manual pass — switch
in *Settings → Accessibility → Contrast themes*, or toggle the current
contrast theme with the left Alt + left Shift + Print Screen hotkey.

## 1b. Appearance switching & the theme switcher

| Check | macOS | Linux | Windows |
|---|---|---|---|
| OS light→dark switch rethemes live, without restart | [x] | [ ] | [x] |
| OS dark→light switch rethemes live | [x] | [ ] | [x] |
| Toggling OS high-contrast while running doesn't break theming (falls back to light/dark) | [ ] | [ ] | [x] |
| Toolbar switcher: Signature → Neutral swaps all surfaces (toolbar, grid, pivot, sidebar, menus) | [x] | [ ] | [x] |
| Toolbar switcher: Neutral → Signature swaps back | [x] | [ ] | [x] |
| Theme switched at runtime + OS mode switched afterwards: new family's variant applies (family sticks) | [ ] | [ ] | [x] |
| Toolbar switcher: hover state on the inactive segment | [ ] | [ ] | [x] |

Windows note: OS light/dark can be flipped programmatically for testing —
set `AppsUseLightTheme`/`SystemUsesLightTheme` under
`HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize` and
broadcast `WM_SETTINGCHANGE` with `lParam = "ImmersiveColorSet"`. The app
rethemed live in both directions and the selected family stuck.

## 2. Typography & fonts

| Check | macOS | Linux | Windows |
|---|---|---|---|
| Cell text renders in a real monospace face (Menlo / DejaVu / Consolas), not a fallback | [x] | [ ] | [x] |
| Column-header labels render **bold** (visibly heavier than cell text) | [x] | [ ] | [x] |
| Pivot Grand Total row/column renders **bold** | [x] | [ ] | [x] |
| Flat-grid group-header rows render **bold** (group any column via right-click → "Group by this column") | [ ] | [ ] | [x] |
| NULL cells render *italic* with the amber null background (the sample seeds NULLs into the `field_*` columns — scroll right a little) | [ ] | [ ] | [x] |
| Right-aligned numbers align cleanly at the cell edge (char-width ratio is per-OS) | [x] | [ ] | [x] |
| Header labels ellipsize (don't overflow) on narrow columns after resize-dragging a column very small | [ ] | [ ] | [x] |

Windows note: on a *very* narrow column with an active filter, the funnel
icon paints over the tail of the clipped header label (cosmetic; nothing
overflows into the neighbouring column).

## 3. Sort affordances & icons (grid)

| Check | macOS | Linux | Windows |
|---|---|---|---|
| Header row is quiet at rest (no boxes/glyphs on unsorted, unhovered columns) | [x] | [ ] | [x] |
| Hovering a column header reveals the outlined button with `-` hint | [x] | [ ] | [x] |
| Sorted column shows bold accent `↑` (asc) / `↓` (desc), 33% larger than cell text | [x] | [ ] | [x] |
| Sort glyph swaps `↑`→`↓`→off when cycling the sort button | [ ] | [ ] | [x] |
| Filter funnel icon (right-click header → Filter…, apply one) paints at the larger size next to the sort button | [ ] | [ ] | [x] |
| Drag-selection marquee shows a 1px accent outline while dragging | [ ] | [ ] | [x] |
| Grouped column shows the 3px accent underline in its header | [ ] | [ ] | [x] |

## 4. Sort affordances & icons (pivot)

| Check | macOS | Linux | Windows |
|---|---|---|---|
| Hovering an innermost column header (e.g. "1".."5") reveals the `-` hint | [x] | [ ] | [x] |
| Clicking it sorts rows by that column; bold accent `↑`/`↓` appears | [x] | [ ] | [x] |
| Corner (top-left, "narrative") hover shows `-`; click cycles row-label sort with `↑`/`↓` | [ ] | [ ] | [x] |
| Grand Total column header: hover hint + sort glyph when sorting by subtotals | [ ] | [ ] | [x] |
| Subtotal "Total" columns (enable "Column subtotal columns" in sidebar; add a second Columns field to see them) : hover hint + sort glyph | [ ] | [ ] | [x] |
| Single-value layout (remove the Columns field): value caption column shows hover hint + sort glyph | [ ] | [ ] | [x] |
| Column-label sort: right-click any pivot column header → "Sort column labels (cycle)" (sample-app menu item wrapping `PivotState::cycle_col_label_sort`; there is no built-in affordance): `↑`/`↓` next to "currency_id" caption | [ ] | [ ] | [x] |
| Sorted-state glyph doesn't collide with right-aligned header labels (numeric innermost headers are right-aligned by default; per-field alignment lives in the chip format dialog) | [ ] | [ ] | [x] |

## 5. Pivot sidebar

| Check | macOS | Linux | Windows |
|---|---|---|---|
| Field chips: uniform height and perfectly even gaps (measured 24px chips / 28px pitch on macOS) | [x] | [ ] | [x] |
| Chip gaps stay even while scrolling the field list | [ ] | [ ] | [x] |
| Long/hostile chip labels (the "interés compuesto acumulado 💸 手数料 عمولة…" field) ellipsize with hover tooltip | [ ] | [ ] | [x] |
| Drop-zone hover highlight while dragging a chip over each zone | [ ] | [ ] | [x] |
| Drag ghost matches chip styling (24px, themed) | [ ] | [ ] | [x] |
| Per-field format dialog (double-click a zone chip): themed popover anchored at the chip, dismissed by clicking outside (no scrim — `overlay_scrim` only backs the grid's busy overlay) | [ ] | [ ] | [x] |

## 6. Menus & popups

| Check | macOS | Linux | Windows |
|---|---|---|---|
| Right-click context menu: themed surface, readable labels, separators | [x] | [ ] | [x] |
| Menu item hover highlight (`menu_hover_bg`) | [ ] | [ ] | [x] |
| Filter popup: themed, cursor visible, emoji/CJK input doesn't misplace the caret badly | [ ] | [ ] | [ ] |
| Menus escape the grid bounds near window edges without clipping | [ ] | [ ] | [x] |

Filter-popup note (all platforms): the emoji/CJK check needs a real IME —
synthetic key injection (SendKeys etc.) can't produce CJK, and the filter
inputs don't implement paste (`keystroke_to_char` ignores ctrl/cmd-modified
keys), so this must be typed by hand with an IME enabled. Themed surface,
visible caret, and live list narrowing were verified on Windows with ASCII.

## 7. Hardening / hostile data

| Check | macOS | Linux | Windows |
|---|---|---|---|
| Emoji ZWJ + CJK + RTL narrative values render in grid cells without panic | [x] | [ ] | [x] |
| Same values as pivot row labels, truncated cleanly | [x] | [ ] | [x] |
| Over-long narrative truncates at the cell edge (no overflow into next column) | [x] | [ ] | [x] |
| `SQLLY_SAMPLE_ROWS=0`: grid shows centered muted "No rows" hint (position/weight is a best guess — judge it) | [ ] | [ ] | [x] |
| `SQLLY_SAMPLE_ROWS=0`: pivot tab shows its hints, no panic | [ ] | [ ] | [x] |
| Window resized to 600×400 minimum with sidebar open: nothing clips or overlaps | [ ] | [ ] | [x] |
| Scrollbar thumbs visible against tracks in both themes (vertical + horizontal) | [x] | [ ] | [x] |

## 8. Platform-specific code paths (Linux / Windows only)

| Check | Linux | Windows |
|---|---|---|
| Font resolves to DejaVu Sans Mono (Linux) / Consolas (Windows) — bold + italic actually render | [ ] | [x] |
| `char_width` ratio (0.6022 / 0.55) — right-aligned numbers and menu widths look correct | [ ] | [x] |
| OS appearance following works (`WindowAppearance` reporting on that platform) | [ ] | [x] |
| WCAG contrast test suite passes: `cargo test -p sqlly-datatable wcag` | [ ] | [x] |
