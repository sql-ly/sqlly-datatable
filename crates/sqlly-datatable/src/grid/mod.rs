//! `GridState` plus the GPUI widget, paint functions, and helpers that swing
//! between them. The flat public re-exports below keep existing consumers of
//! the 0.1.x API working; new code should prefer importing from the canonical
//! `crate::grid::*` paths.

pub mod context_menu;
pub mod menu;
pub(crate) mod motion;
pub mod paint;
pub mod selection;
pub mod state;
pub mod theme;
pub mod widget;

// Flat re-exports so external code can write `use sqlly_datatable::GridState`
// without mapping the internal split.
pub use context_menu::{
    ColumnContext, ContextMenuItem, ContextMenuProvider, ContextMenuRequest, ContextMenuSelection,
    ContextMenuTarget, SelectedCellContext, SelectedRowContext,
};
pub use menu::{ContextMenu, MenuAction, MenuItem};
pub use selection::{HitResult, ScrollbarAxis, Selection, SortDirection};
pub use state::{
    BusyState, FilterInput, FilterPanel, FilterValueRow, GridState, RowGroup, RowWindow,
};
pub use theme::{GridTheme, GridThemePair};
pub use widget::{GridTab, PivotSidebarPosition, SqllyDataTable, SqllyDataTableBuilder};

// Inline a couple of constants that callers used to read from the `grid` mod.
pub use state::SCROLLBAR_SIZE;

use gpui::{div, prelude::*, px, Div, FontWeight};
use gpui_component::{Icon, IconName};

/// The single checkbox used across every filter panel, the pivot filter
/// popover, the pivot layout options, and the per-field format dialog. One
/// size, one style — so the same affordance never ships at three sizes again
/// (it previously shipped at 12 / 14 / 16 px in four hand-rolled copies).
/// Accent-filled with a knockout check when on; an outlined empty box when off.
pub(crate) fn checkbox(checked: bool, theme: &GridTheme) -> Div {
    let mut b = div()
        .w(px(14.0))
        .h(px(14.0))
        .flex_none()
        .rounded(px(3.0))
        .border_1()
        .border_color(if checked {
            theme.sort_indicator
        } else {
            theme.grid_line
        })
        .bg(if checked {
            theme.sort_indicator
        } else {
            theme.menu_bg
        })
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.bg);
    if checked {
        // Lucide check, sized by the cascaded 11px text size and colored by
        // the knockout `text_color` above (SVG icons render everywhere the
        // same — no reliance on font glyph coverage, which the web build's
        // embedded fonts lack for "✓").
        b = b.child(Icon::new(IconName::Check));
    }
    b
}
