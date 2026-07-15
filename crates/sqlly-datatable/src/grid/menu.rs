//! Context menu — column-header right-click interaction. Layout, hover
//! resolution, and action labels live here so paint code only consumes the
//! menu snapshot.

use gpui::{px, Hsla, Pixels, Point};

use crate::grid::context_menu::ContextMenuRequest;

/// Height, padding, and minimum width used to lay the menu out. Public so the
/// state module's hit-testing math can stay in sync with paint.
pub const MENU_FONT_SIZE: f32 = 14.0;
pub const MENU_ITEM_HEIGHT: f32 = MENU_FONT_SIZE + 8.0;
pub const MENU_PADDING_X: f32 = 12.0;
pub const MENU_MIN_WIDTH: f32 = 180.0;
pub const MENU_BORDER: f32 = 1.0;
pub const MENU_INNER_PAD: f32 = 4.0;
/// Gap kept between the menu and the window edge when it must be nudged
/// on-screen. Small so the menu still visually hugs the pointer.
pub const MENU_SCREEN_MARGIN: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    SelectColumn,
    CopyColumn,
    CopyColumnWithHeaders,
    SortAscending,
    SortDescending,
    ClearSort,
    GroupBy,
    ClearGrouping,
    FilterPrompt,
    ClearFilter,
}

#[derive(Clone, Debug)]
pub enum MenuItem {
    Action(MenuAction),
    Custom { id: String, label: String },
    Separator,
}

impl MenuItem {
    /// Display label for the item, or `None` for separators.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        match self {
            Self::Action(a) => Some(label(*a)),
            Self::Custom { label, .. } => Some(label.as_str()),
            Self::Separator => None,
        }
    }

    /// `true` for action/custom items that participate in hover/click.
    #[must_use]
    pub fn is_selectable(&self) -> bool {
        !matches!(self, Self::Separator)
    }
}

#[derive(Clone, Debug)]
pub struct ContextMenu {
    pub col: usize,
    pub anchor: Point<Pixels>,
    pub items: Vec<MenuItem>,
    pub hovered: Option<usize>,
    pub request: Option<ContextMenuRequest>,
}

impl ContextMenu {
    /// Standard column-header menu. Constructed by state when the user
    /// right-clicks a column header or sort button.
    #[must_use]
    pub fn standard(col: usize, anchor: Point<Pixels>) -> Self {
        Self {
            col,
            anchor,
            items: vec![
                MenuItem::Action(MenuAction::SelectColumn),
                MenuItem::Action(MenuAction::CopyColumn),
                MenuItem::Action(MenuAction::CopyColumnWithHeaders),
                MenuItem::Separator,
                MenuItem::Action(MenuAction::SortAscending),
                MenuItem::Action(MenuAction::SortDescending),
                MenuItem::Action(MenuAction::ClearSort),
                MenuItem::Separator,
                MenuItem::Action(MenuAction::GroupBy),
                MenuItem::Action(MenuAction::ClearGrouping),
                MenuItem::Separator,
                MenuItem::Action(MenuAction::FilterPrompt),
                MenuItem::Action(MenuAction::ClearFilter),
            ],
            hovered: None,
            request: None,
        }
    }

    /// Construct a custom menu from provider-supplied items plus the
    /// captured request snapshot. `col` is used for built-in action
    /// dispatch when the provider composes `BuiltIn` items.
    #[must_use]
    pub fn custom(
        col: usize,
        anchor: Point<Pixels>,
        items: Vec<MenuItem>,
        request: ContextMenuRequest,
    ) -> Self {
        Self {
            col,
            anchor,
            items,
            hovered: None,
            request: Some(request),
        }
    }

    /// Width needed to fit the longest label, with padding, bounded below by
    /// [`MENU_MIN_WIDTH`].
    #[must_use]
    pub fn width_for(&self, char_width: f32) -> f32 {
        let mut max_label_w = 0.0_f32;
        for item in &self.items {
            if let Some(text) = item.label() {
                max_label_w = max_label_w.max(text.chars().count() as f32 * char_width);
            }
        }
        MENU_MIN_WIDTH.max(max_label_w + MENU_PADDING_X * 2.0)
    }

    /// Total height including inner padding.
    #[must_use]
    pub fn total_height(&self) -> f32 {
        self.items.len() as f32 * MENU_ITEM_HEIGHT + MENU_INNER_PAD * 2.0
    }

    /// Resolve the menu's final top-left corner in **grid-relative** space,
    /// given the grid origin in window space (`grid_ox`, `grid_oy`) and the
    /// window viewport size (`vw`, `vh`).
    ///
    /// The menu must never be clipped by the grid area — only the window edge
    /// constrains it. Vertically it always opens *downward* from the anchor
    /// unless the full menu would not fit below the anchor within the window,
    /// in which case it flips to open *upward* (anchored so its bottom sits at
    /// the anchor). Horizontally it shifts left to stay on-screen.
    ///
    /// Returned in grid-relative coordinates so it composes directly with
    /// [`hover_at`] and the widget's grid-relative pointer math. Paint adds the
    /// grid origin back to reach absolute window space.
    #[must_use]
    pub fn resolved_position(
        &self,
        grid_ox: f32,
        grid_oy: f32,
        vw: f32,
        vh: f32,
        char_width: f32,
    ) -> Point<Pixels> {
        let menu_w = self.width_for(char_width);
        let menu_h = self.total_height();
        // Desired top-left in absolute window space.
        let ax = grid_ox + f32::from(self.anchor.x);
        let ay = grid_oy + f32::from(self.anchor.y);

        // Horizontal: keep the whole menu inside the window, never clipped by
        // the grid. Prefer the anchor x; shift left if the right edge spills
        // past the window; never let the left edge go off-screen.
        let mut mx = ax;
        if mx + menu_w > vw {
            mx = vw - menu_w - MENU_SCREEN_MARGIN;
        }
        if mx < MENU_SCREEN_MARGIN {
            mx = MENU_SCREEN_MARGIN;
        }

        // Vertical: open down by default. Flip up only when there is literally
        // no room for the full menu below the anchor within the window.
        let opens_down = ay + menu_h + MENU_SCREEN_MARGIN <= vh;
        let mut my = if opens_down {
            ay
        } else {
            // Open upward: anchor the menu's bottom at the click point.
            ay - menu_h
        };
        // Final safety: never let the menu run off the top of the window. This
        // can only trigger for a menu taller than the whole viewport.
        if my < MENU_SCREEN_MARGIN {
            my = MENU_SCREEN_MARGIN;
        }

        // Convert back to grid-relative space for downstream consumers.
        Point {
            x: px(mx - grid_ox),
            y: px(my - grid_oy),
        }
    }
}

/// Maps an action to its user-facing label. Used by hit-testing, paint, and
/// any overlay that needs to show the same string the menu shows.
#[must_use]
pub fn label(action: MenuAction) -> &'static str {
    match action {
        MenuAction::SelectColumn => "Select column",
        MenuAction::CopyColumn => "Copy column",
        MenuAction::CopyColumnWithHeaders => "Copy column with headers",
        MenuAction::SortAscending => "Sort Ascending",
        MenuAction::SortDescending => "Sort Descending",
        MenuAction::ClearSort => "Clear sort",
        MenuAction::GroupBy => "Group by this column",
        MenuAction::ClearGrouping => "Clear grouping",
        MenuAction::FilterPrompt => "Filter...",
        MenuAction::ClearFilter => "Clear filter",
    }
}

/// Index of the hovered action under `x` (content-space) given the
/// caller's full `y`. The caller supplies `y` because the menu overlay is
/// drawn outside the bounds; we don't double-correct it here.
///
/// Uses the menu's stored anchor. When the menu has been repositioned to stay
/// on-screen (flip up / shift left), callers must use [`hover_at_anchor`] with
/// the resolved top-left so hit-testing matches paint.
#[must_use]
pub fn hover_at(menu: &ContextMenu, x: f32, y: f32, char_width: f32) -> Option<usize> {
    hover_at_anchor(menu, menu.anchor, x, y, char_width)
}

/// Like [`hover_at`] but tests against an explicit resolved top-left `anchor`
/// (grid-relative) rather than the menu's stored anchor. Keeps hover/click
/// hit-testing aligned with the on-screen position produced by
/// [`ContextMenu::resolved_position`].
#[must_use]
pub fn hover_at_anchor(
    menu: &ContextMenu,
    anchor: Point<Pixels>,
    x: f32,
    y: f32,
    char_width: f32,
) -> Option<usize> {
    let w = menu.width_for(char_width);
    let ax: f32 = anchor.x.into();
    let ay: f32 = anchor.y.into();
    if x < ax || x > ax + w || y < ay {
        return None;
    }
    let rel_y = y - ay - MENU_INNER_PAD;
    if rel_y < 0.0 {
        return None;
    }
    let idx = (rel_y / MENU_ITEM_HEIGHT) as usize;
    if idx >= menu.items.len() {
        return None;
    }
    for (cur_row, item) in menu.items.iter().enumerate() {
        if cur_row == idx {
            return match item {
                MenuItem::Action(_) | MenuItem::Custom { .. } => action_index(&menu.items, idx),
                MenuItem::Separator => None,
            };
        }
    }
    None
}

fn action_index(items: &[MenuItem], row: usize) -> Option<usize> {
    let mut action_idx = 0;
    for (i, item) in items.iter().enumerate() {
        if item.is_selectable() {
            if i == row {
                return Some(action_idx);
            }
            action_idx += 1;
        }
    }
    None
}

/// Stable palette for menu chrome.
#[must_use]
pub fn background() -> Hsla {
    Hsla {
        h: 0.0,
        s: 0.0,
        l: 1.0,
        a: 1.0,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;

    fn menu_at(x: f32, y: f32) -> ContextMenu {
        ContextMenu::standard(7, point_from(x, y))
    }

    fn point_from(x: f32, y: f32) -> Point<Pixels> {
        Point { x: px(x), y: px(y) }
    }

    fn anchor_y(m: &ContextMenu) -> f32 {
        f32::from(m.anchor.y)
    }

    #[test]
    fn standard_menu_item_sequence_is_stable() {
        let m = ContextMenu::standard(0, point_from(0.0, 0.0));
        let kinds: Vec<&'static str> = m
            .items
            .iter()
            .map(|i| match i {
                MenuItem::Action(MenuAction::SelectColumn) => "SelectColumn",
                MenuItem::Action(MenuAction::CopyColumn) => "CopyColumn",
                MenuItem::Action(MenuAction::CopyColumnWithHeaders) => "CopyColumnWithHeaders",
                MenuItem::Separator => "Separator",
                MenuItem::Action(MenuAction::SortAscending) => "SortAscending",
                MenuItem::Action(MenuAction::SortDescending) => "SortDescending",
                MenuItem::Action(MenuAction::ClearSort) => "ClearSort",
                MenuItem::Action(MenuAction::GroupBy) => "GroupBy",
                MenuItem::Action(MenuAction::ClearGrouping) => "ClearGrouping",
                MenuItem::Action(MenuAction::FilterPrompt) => "FilterPrompt",
                MenuItem::Action(MenuAction::ClearFilter) => "ClearFilter",
                MenuItem::Custom { .. } => "Custom",
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "SelectColumn",
                "CopyColumn",
                "CopyColumnWithHeaders",
                "Separator",
                "SortAscending",
                "SortDescending",
                "ClearSort",
                "Separator",
                "GroupBy",
                "ClearGrouping",
                "Separator",
                "FilterPrompt",
                "ClearFilter",
            ],
        );
    }

    #[test]
    fn separators_break_menu_into_action_groups() {
        let m = ContextMenu::standard(0, point_from(0.0, 0.0));
        let separators = m
            .items
            .iter()
            .filter(|i| matches!(i, MenuItem::Separator))
            .count();
        assert_eq!(separators, 3);
    }

    #[test]
    fn every_menu_action_has_non_empty_label() {
        for a in [
            MenuAction::SelectColumn,
            MenuAction::CopyColumn,
            MenuAction::CopyColumnWithHeaders,
            MenuAction::SortAscending,
            MenuAction::SortDescending,
            MenuAction::ClearSort,
            MenuAction::GroupBy,
            MenuAction::ClearGrouping,
            MenuAction::FilterPrompt,
            MenuAction::ClearFilter,
        ] {
            assert!(!label(a).is_empty(), "{a:?} has empty label");
        }
    }

    #[test]
    fn width_respects_min_width() {
        let m = menu_at(0.0, 0.0);
        assert!(m.width_for(1.0) >= MENU_MIN_WIDTH);
    }

    #[test]
    fn width_grows_with_longest_label() {
        let m = menu_at(0.0, 0.0);
        let narrow = m.width_for(1.0);
        let wide = m.width_for(20.0);
        assert!(wide > narrow);
    }

    #[test]
    fn total_height_matches_items_and_padding() {
        let m = menu_at(0.0, 0.0);
        let expected = m.items.len() as f32 * MENU_ITEM_HEIGHT + MENU_INNER_PAD * 2.0;
        assert_eq!(m.total_height(), expected);
    }

    #[test]
    fn hover_returns_none_outside_x_bounds() {
        let m = menu_at(100.0, 100.0);
        let right = m.width_for(8.0);
        assert_eq!(hover_at(&m, 99.0, 110.0, 8.0), None);
        assert_eq!(hover_at(&m, 100.0 + right + 1.0, 110.0, 8.0), None);
    }

    #[test]
    fn hover_returns_none_above_anchor() {
        let m = menu_at(100.0, 100.0);
        assert_eq!(hover_at(&m, 110.0, 99.0, 8.0), None);
    }

    #[test]
    fn hover_on_first_action_returns_action_index_zero() {
        let m = menu_at(100.0, 100.0);
        let y: f32 = anchor_y(&m) + MENU_INNER_PAD;
        assert_eq!(hover_at(&m, 110.0, y, 8.0), Some(0));
    }

    #[test]
    fn hover_on_separator_returns_none() {
        let m = menu_at(100.0, 100.0);
        let y: f32 = anchor_y(&m) + MENU_INNER_PAD + 3.0 * MENU_ITEM_HEIGHT;
        assert_eq!(hover_at(&m, 110.0, y, 8.0), None);
    }

    #[test]
    fn hover_below_last_item_is_none() {
        let m = menu_at(100.0, 100.0);
        let y: f32 = anchor_y(&m) + 1000.0;
        assert_eq!(hover_at(&m, 110.0, y, 8.0), None);
    }

    fn custom_menu_with_items(x: f32, y: f32, items: Vec<MenuItem>) -> ContextMenu {
        ContextMenu {
            col: 0,
            anchor: point_from(x, y),
            items,
            hovered: None,
            request: None,
        }
    }

    #[test]
    fn custom_item_contributes_to_width() {
        let long_label = "A very long custom menu item label";
        let items = vec![
            MenuItem::Custom {
                id: "a".into(),
                label: long_label.into(),
            },
            MenuItem::Separator,
        ];
        let m = custom_menu_with_items(0.0, 0.0, items);
        let w = m.width_for(8.0);
        let expected = long_label.chars().count() as f32 * 8.0 + MENU_PADDING_X * 2.0;
        assert_eq!(w, expected);
    }

    #[test]
    fn custom_item_is_selectable_and_hoverable() {
        let items = vec![
            MenuItem::Custom {
                id: "first".into(),
                label: "First".into(),
            },
            MenuItem::Separator,
            MenuItem::Custom {
                id: "third".into(),
                label: "Third".into(),
            },
        ];
        let m = custom_menu_with_items(100.0, 100.0, items);
        // First custom item at index 0.
        let y: f32 = anchor_y(&m) + MENU_INNER_PAD;
        assert_eq!(hover_at(&m, 110.0, y, 8.0), Some(0));
        // Separator at row 1 returns None.
        let y: f32 = anchor_y(&m) + MENU_INNER_PAD + 1.0 * MENU_ITEM_HEIGHT;
        assert_eq!(hover_at(&m, 110.0, y, 8.0), None);
        // Third item (second custom) at row 2 -> action index 1.
        let y: f32 = anchor_y(&m) + MENU_INNER_PAD + 2.0 * MENU_ITEM_HEIGHT;
        assert_eq!(hover_at(&m, 110.0, y, 8.0), Some(1));
    }

    #[test]
    fn menu_item_label_helper() {
        assert_eq!(
            MenuItem::Action(MenuAction::SortAscending).label(),
            Some("Sort Ascending")
        );
        assert_eq!(
            MenuItem::Custom {
                id: "x".into(),
                label: "Hello".into()
            }
            .label(),
            Some("Hello")
        );
        assert_eq!(MenuItem::Separator.label(), None);
    }

    #[test]
    fn menu_item_is_selectable() {
        assert!(MenuItem::Action(MenuAction::ClearFilter).is_selectable());
        assert!(MenuItem::Custom {
            id: "x".into(),
            label: "y".into()
        }
        .is_selectable());
        assert!(!MenuItem::Separator.is_selectable());
    }

    // --- resolved_position: never-clip + up/down flip ------------------------

    /// With a large window and the anchor near the top, the menu opens straight
    /// down from the anchor (position unchanged).
    #[test]
    fn resolved_opens_down_when_room_below() {
        let m = menu_at(50.0, 30.0);
        // grid origin at window (0,0), big window.
        let p = m.resolved_position(0.0, 0.0, 2000.0, 2000.0, 8.0);
        assert_eq!(f32::from(p.x), 50.0);
        assert_eq!(f32::from(p.y), 30.0);
    }

    /// When the full menu would not fit below the anchor within the window, it
    /// flips up: its bottom sits at the anchor (top = anchor_y - height).
    #[test]
    fn resolved_flips_up_when_no_room_below() {
        let m = menu_at(50.0, 590.0);
        let h = m.total_height();
        // Window only 600 tall; anchor at y=590 leaves no room for the menu.
        let p = m.resolved_position(0.0, 0.0, 2000.0, 600.0, 8.0);
        assert_eq!(f32::from(p.y), 590.0 - h);
    }

    /// The menu is clamped to the *window* width, not the grid width — proving
    /// it is never clipped by a grid area smaller than the window. Grid is only
    /// 300 wide but the window is 2000 wide, so a menu near the grid's right
    /// edge still extends past the grid without being pulled in.
    #[test]
    fn resolved_not_clipped_by_grid_only_by_window() {
        let m = menu_at(280.0, 30.0);
        let w = m.width_for(8.0);
        // Grid origin at window (0,0); grid is 300 wide (sw), window 2000 wide.
        let p = m.resolved_position(0.0, 0.0, 2000.0, 2000.0, 8.0);
        // Stays at the anchor x — not shifted to fit inside the 300px grid.
        assert_eq!(f32::from(p.x), 280.0);
        // And its right edge is allowed to exceed the grid width (300).
        assert!(f32::from(p.x) + w > 300.0);
    }

    /// When the anchor is close to the window's right edge, the menu shifts
    /// left so its right edge stays inside the window.
    #[test]
    fn resolved_shifts_left_at_window_right_edge() {
        let m = menu_at(1950.0, 30.0);
        let w = m.width_for(8.0);
        let vw = 2000.0;
        let p = m.resolved_position(0.0, 0.0, vw, 2000.0, 8.0);
        let right = f32::from(p.x) + w;
        assert!(right <= vw, "menu right edge {right} must stay within {vw}");
        assert_eq!(f32::from(p.x), vw - w - MENU_SCREEN_MARGIN);
    }

    /// The grid origin offset is honored: a grid placed at a window offset
    /// shifts the absolute placement but the returned value stays grid-relative.
    #[test]
    fn resolved_accounts_for_grid_origin() {
        let m = menu_at(10.0, 10.0);
        // Grid origin at (100, 200) in the window; plenty of room below.
        let p = m.resolved_position(100.0, 200.0, 2000.0, 2000.0, 8.0);
        // Grid-relative result is unchanged because absolute (110,210) fits.
        assert_eq!(f32::from(p.x), 10.0);
        assert_eq!(f32::from(p.y), 10.0);
    }
}
