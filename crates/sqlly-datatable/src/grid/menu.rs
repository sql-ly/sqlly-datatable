//! Context menu — column-header right-click interaction. Layout, hover
//! resolution, and action labels live here so paint code only consumes the
//! menu snapshot.

use gpui::{Hsla, Pixels, Point};

/// Height, padding, and minimum width used to lay the menu out. Public so the
/// state module's hit-testing math can stay in sync with paint.
pub const MENU_FONT_SIZE: f32 = 14.0;
pub const MENU_ITEM_HEIGHT: f32 = MENU_FONT_SIZE + 8.0;
pub const MENU_PADDING_X: f32 = 12.0;
pub const MENU_MIN_WIDTH: f32 = 180.0;
pub const MENU_BORDER: f32 = 1.0;
pub const MENU_INNER_PAD: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    SelectColumn,
    CopyColumn,
    CopyColumnWithHeaders,
    SortAscending,
    SortDescending,
    ClearSort,
    FilterPrompt,
    ClearFilter,
}

#[derive(Clone, Debug)]
pub enum MenuItem {
    Action(MenuAction),
    Separator,
}

#[derive(Clone, Debug)]
pub struct ContextMenu {
    pub col: usize,
    pub anchor: Point<Pixels>,
    pub items: Vec<MenuItem>,
    pub hovered: Option<usize>,
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
                MenuItem::Action(MenuAction::FilterPrompt),
                MenuItem::Action(MenuAction::ClearFilter),
            ],
            hovered: None,
        }
    }

    /// Width needed to fit the longest label, with padding, bounded below by
    /// [`MENU_MIN_WIDTH`].
    #[must_use]
    pub fn width_for(&self, char_width: f32) -> f32 {
        let mut max_label_w = 0.0_f32;
        for item in &self.items {
            if let MenuItem::Action(a) = item {
                max_label_w = max_label_w.max(label(*a).len() as f32 * char_width);
            }
        }
        MENU_MIN_WIDTH.max(max_label_w + MENU_PADDING_X * 2.0)
    }

    /// Total height including inner padding.
    #[must_use]
    pub fn total_height(&self) -> f32 {
        self.items.len() as f32 * MENU_ITEM_HEIGHT + MENU_INNER_PAD * 2.0
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
        MenuAction::FilterPrompt => "Filter...",
        MenuAction::ClearFilter => "Clear filter",
    }
}

/// Index of the hovered action under `x` (content-space) given the
/// caller's full `y`. The caller supplies `y` because the menu overlay is
/// drawn outside the bounds; we don't double-correct it here.
#[must_use]
pub fn hover_at(menu: &ContextMenu, x: f32, y: f32, char_width: f32) -> Option<usize> {
    let w = menu.width_for(char_width);
    let ax: f32 = menu.anchor.x.into();
    let ay: f32 = menu.anchor.y.into();
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
                MenuItem::Action(_) => action_index(&menu.items, idx),
                MenuItem::Separator => None,
            };
        }
    }
    None
}

fn action_index(items: &[MenuItem], row: usize) -> Option<usize> {
    let mut action_idx = 0;
    for (i, item) in items.iter().enumerate() {
        if matches!(item, MenuItem::Action(_)) {
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
