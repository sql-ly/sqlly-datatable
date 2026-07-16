# Product

## Register

product

## Platform

desktop (native GPUI — macOS, Linux, Windows; not web/ios/android)

## Users

Two audiences matter equally. First, end users of [sqlly.app](https://sqlly.app) — people who live in SQL query results for hours at a time and need to scan, select, sort, filter, and pivot large datasets without friction or eye strain. Second, GPUI developers evaluating the crate on crates.io or GitHub — they judge it in seconds from screenshots and the sample app, and they need to believe the default look is production-grade before they'll read a line of API docs.

## Product Purpose

A configurable, virtualized data grid and pivot table component for GPUI, extracted from sqlly.app and shared as a crate. It fills a real gap: GPUI ships no production data grid, and every serious data tool needs one. Success means the visual quality erases the README's own "AI slop" worry — the grid should be polished enough that GPUI contributors take its patterns seriously, and sqlly.app can ship it to paying users on all three OSes without embarrassment.

## Positioning

Excel-class in Rust: spreadsheet-grade interactions — pivot tables, drag selection, rich per-column formatting — in a native, virtualized Rust component.

## Brand Personality

Fast, sharp, professional. The grid should feel engineered: crisp edges, instant feedback, keyboard-first energy. It is an instrument, not a decoration — the data is the interface.

References: Linear (subtle hovers, tight type scale, immaculate spacing) and Excel / Apple Numbers (spreadsheet conventions users already know — selection marquee, header highlight tied to selection, the feel of frozen headers).

## Theming strategy

The crate ships **two complete out-of-box themes**, each with a light and dark variant that follows the OS window appearance:

1. **Refined neutral** — restrained palette with one accent, tuned for long data-reading sessions; blends into a host app.
2. **Signature** — a committed, recognizable look for screenshots and the sample app.

Both are demonstrated (switchable) in the sample app. Beyond the shipped pair, `GridTheme` must remain **fully themable**: every color a consumer-settable field, no hardcoded colors in paint code, so a host app can derive the grid's palette from its own theme.

## Anti-references

- Legacy enterprise grids: chunky 3D borders, beveled headers, Windows-Forms-era chrome.
- Web-app aesthetics: rounded cards, drop shadows everywhere, bouncy animation — anything that betrays it isn't native.
- Terminal/hacker styling: green-on-black, monospace-everything as a costume.

## Design Principles

1. **The data is the interface.** Chrome recedes; values, selection state, and structure (totals, groups) carry the visual hierarchy.
2. **Earned familiarity.** Use spreadsheet conventions people already know rather than inventing affordances. The tool should disappear into the task.
3. **State is instant and legible.** Hover, selection, sort, filter-active, and drag states each read at a glance, in both light and dark, on all three OSes.
4. **Themable to the bone.** Two beautiful defaults, zero hardcoded colors — a host app can make the grid its own.
5. **Density without fatigue.** SQL results are dense by nature; tune contrast, rhythm, and alternating rows for hours of reading, not seconds of demo.

## Accessibility & Inclusion

WCAG AA contrast (≥4.5:1 body text) for all text roles in both themes, light and dark variants alike. Negative/positive number styling must not rely on color alone (color-blind-safe — keep the parentheses/format channel). No motion that cannot be disabled; any animation respects a reduced-motion preference.
