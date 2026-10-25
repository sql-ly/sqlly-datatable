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
/// How far a submenu overlaps its parent's edge, so the pointer crosses
/// straight from the parent row into the child with no gap.
pub const SUBMENU_OVERLAP: f32 = 2.0;
/// Room kept at the right of a menu that has submenus, for the `▸` mark.
pub const SUBMENU_ARROW_WIDTH: f32 = 18.0;
/// How long the pointer must rest on another row before an open submenu
/// switches or closes. Long enough to cross a sibling row on the way into
/// the open child without it collapsing; short enough to feel immediate.
pub const SUBMENU_HOVER_DELAY_MS: u64 = 200;
/// The mark painted at the end of a submenu row.
pub const SUBMENU_ARROW: &str = "\u{25B8}";

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
    FreezeToHere,
    UnfreezeColumns,
    HideColumn,
    ShowAllColumns,
    MoveColumnLeft,
    MoveColumnRight,
    FilterPrompt,
    ClearFilter,
}

#[derive(Clone, Debug)]
pub enum MenuItem {
    Action(MenuAction),
    Custom {
        id: String,
        label: String,
    },
    Separator,
    /// A row that opens a nested menu beside it.
    Submenu {
        label: String,
        items: Vec<MenuItem>,
    },
}

impl MenuItem {
    /// Display label for the item, or `None` for separators.
    #[must_use]
    pub fn label(&self) -> Option<String> {
        match self {
            Self::Action(a) => Some(label(*a)),
            Self::Custom { label, .. } | Self::Submenu { label, .. } => Some(label.clone()),
            Self::Separator => None,
        }
    }

    /// `true` for action/custom/submenu items that participate in hover/click.
    #[must_use]
    pub fn is_selectable(&self) -> bool {
        !matches!(self, Self::Separator)
    }

    /// `true` for a row that opens a nested menu.
    #[must_use]
    pub fn is_submenu(&self) -> bool {
        matches!(self, Self::Submenu { .. })
    }

    /// The nested items of a submenu row, or `None` for any other item.
    #[must_use]
    pub fn submenu_items(&self) -> Option<&[MenuItem]> {
        match self {
            Self::Submenu { items, .. } => Some(items),
            _ => None,
        }
    }
}

/// Tidy a menu level for display, recursively:
/// - every submenu is tidied first, and a submenu left with nothing to choose
///   is dropped (an empty submenu is never shown);
/// - separators never lead, trail, or sit next to another separator.
#[must_use]
pub fn normalize_items(items: Vec<MenuItem>) -> Vec<MenuItem> {
    let mut out: Vec<MenuItem> = Vec::with_capacity(items.len());
    for item in items {
        match item {
            MenuItem::Separator => {
                if out.last().is_some_and(MenuItem::is_selectable) {
                    out.push(MenuItem::Separator);
                }
            }
            MenuItem::Submenu { label, items } => {
                let items = normalize_items(items);
                if items.iter().any(MenuItem::is_selectable) {
                    out.push(MenuItem::Submenu { label, items });
                }
            }
            other => out.push(other),
        }
    }
    while matches!(out.last(), Some(MenuItem::Separator)) {
        out.pop();
    }
    out
}

/// Width needed to fit the longest label of one menu level (plus the `▸`
/// mark when the level holds a submenu), bounded below by [`MENU_MIN_WIDTH`].
#[must_use]
pub fn level_width(items: &[MenuItem], char_width: f32) -> f32 {
    let mut max_label_w = 0.0_f32;
    let mut has_submenu = false;
    for item in items {
        has_submenu |= item.is_submenu();
        if let Some(text) = item.label() {
            max_label_w = max_label_w.max(text.chars().count() as f32 * char_width);
        }
    }
    let arrow = if has_submenu {
        SUBMENU_ARROW_WIDTH
    } else {
        0.0
    };
    MENU_MIN_WIDTH.max(max_label_w + arrow + MENU_PADDING_X * 2.0)
}

/// Height of one menu level, including inner padding. Separators take a full
/// row, as they are painted.
#[must_use]
pub fn level_height(items: &[MenuItem]) -> f32 {
    items.len() as f32 * MENU_ITEM_HEIGHT + MENU_INNER_PAD * 2.0
}

/// A submenu that is open, recorded per level below the top one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenSubmenu {
    /// Row (item index, separators included) of the submenu item in its
    /// parent level.
    pub row: usize,
    /// Highlighted row inside this submenu, if any.
    pub hovered: Option<usize>,
}

/// A menu level's on-screen rectangle, in absolute window coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl MenuRect {
    /// `true` when the window point `(x, y)` falls inside this rectangle.
    #[must_use]
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}

/// What choosing a menu row (by click or Enter) asks the grid to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuActivation {
    Builtin(MenuAction),
    Custom(String),
}

/// The result of a key press while the menu is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuKeyOutcome {
    /// The key moved the highlight or opened/closed a submenu.
    Handled,
    /// Escape with no submenu open: close the whole menu.
    Close,
    /// Enter on an action row: run it and close the menu.
    Activate(MenuActivation),
    /// The key means nothing to the menu.
    Ignored,
}

#[derive(Clone, Debug)]
pub struct ContextMenu {
    pub col: usize,
    pub anchor: Point<Pixels>,
    pub items: Vec<MenuItem>,
    /// Highlighted top-level item, counted among selectable items only
    /// (separators skipped), as [`hover_at`] reports it.
    pub hovered: Option<usize>,
    pub request: Option<ContextMenuRequest>,
    /// Open submenus, outermost first. Empty when only the top level shows.
    pub open: Vec<OpenSubmenu>,
    /// Bumped on every hover change, so a delayed submenu switch scheduled
    /// for an earlier hover can tell it is stale.
    pub hover_gen: u64,
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
                MenuItem::Action(MenuAction::FreezeToHere),
                MenuItem::Action(MenuAction::UnfreezeColumns),
                MenuItem::Action(MenuAction::HideColumn),
                MenuItem::Action(MenuAction::ShowAllColumns),
                MenuItem::Action(MenuAction::MoveColumnLeft),
                MenuItem::Action(MenuAction::MoveColumnRight),
                MenuItem::Separator,
                MenuItem::Action(MenuAction::FilterPrompt),
                MenuItem::Action(MenuAction::ClearFilter),
            ],
            hovered: None,
            request: None,
            open: Vec::new(),
            hover_gen: 0,
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
            items: normalize_items(items),
            hovered: None,
            request: Some(request),
            open: Vec::new(),
            hover_gen: 0,
        }
    }

    /// Width needed to fit the longest top-level label, with padding,
    /// bounded below by [`MENU_MIN_WIDTH`].
    #[must_use]
    pub fn width_for(&self, char_width: f32) -> f32 {
        level_width(&self.items, char_width)
    }

    /// Total height of the top level including inner padding.
    #[must_use]
    pub fn total_height(&self) -> f32 {
        level_height(&self.items)
    }

    /// The items shown at `level`: 0 is the top menu, 1 the first open
    /// submenu, and so on. `None` past the deepest open submenu.
    #[must_use]
    pub fn level_items(&self, level: usize) -> Option<&[MenuItem]> {
        let mut items: &[MenuItem] = &self.items;
        for open in self.open.iter().take(level) {
            items = items.get(open.row)?.submenu_items()?;
        }
        if level > self.open.len() {
            return None;
        }
        Some(items)
    }

    /// Number of levels on screen (the top menu plus every open submenu).
    #[must_use]
    pub fn level_count(&self) -> usize {
        self.open.len() + 1
    }

    /// The highlighted row (item index, separators included) at `level`.
    #[must_use]
    pub fn highlighted_row(&self, level: usize) -> Option<usize> {
        if level == 0 {
            let selectable = self.hovered?;
            return self
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.is_selectable())
                .nth(selectable)
                .map(|(row, _)| row);
        }
        self.open.get(level - 1)?.hovered
    }

    /// Highlight `row` at `level`. Ignored for separators and rows that don't
    /// exist.
    pub fn set_highlight(&mut self, level: usize, row: usize) {
        let Some(items) = self.level_items(level) else {
            return;
        };
        if !items.get(row).is_some_and(MenuItem::is_selectable) {
            return;
        }
        if level == 0 {
            self.hovered = Some(action_index(items, row).unwrap_or(0));
        } else if let Some(open) = self.open.get_mut(level - 1) {
            open.hovered = Some(row);
        }
    }

    /// Open the submenu at `row` of `level`, closing anything deeper first.
    /// Highlights the row; does nothing more when it is not a submenu.
    pub fn open_submenu(&mut self, level: usize, row: usize) {
        if level > self.open.len() {
            return;
        }
        self.open.truncate(level);
        self.set_highlight(level, row);
        let is_submenu = self
            .level_items(level)
            .and_then(|items| items.get(row))
            .is_some_and(MenuItem::is_submenu);
        if is_submenu {
            self.open.push(OpenSubmenu { row, hovered: None });
        }
    }

    /// Close every submenu deeper than `level`.
    pub fn close_below(&mut self, level: usize) {
        self.open.truncate(level);
    }

    /// What choosing `row` at `level` does: an activation for an action row,
    /// `None` for a submenu row (which opens instead) or a separator.
    #[must_use]
    pub fn activation_at(&self, level: usize, row: usize) -> Option<MenuActivation> {
        match self.level_items(level)?.get(row)? {
            MenuItem::Action(action) => Some(MenuActivation::Builtin(*action)),
            MenuItem::Custom { id, .. } => Some(MenuActivation::Custom(id.clone())),
            MenuItem::Separator | MenuItem::Submenu { .. } => None,
        }
    }

    /// Keyboard navigation. Keys act on the deepest open level: Up/Down move
    /// the highlight among choosable rows (wrapping), Right or Enter on a
    /// submenu row opens it with its first row highlighted, Enter on an action
    /// row activates it, Left closes the deepest submenu, and Escape closes
    /// the deepest submenu or, with none open, the whole menu.
    pub fn key(&mut self, key: &str) -> MenuKeyOutcome {
        let level = self.open.len();
        match key {
            "up" | "down" => {
                let Some(items) = self.level_items(level) else {
                    return MenuKeyOutcome::Ignored;
                };
                let rows: Vec<usize> = items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.is_selectable())
                    .map(|(row, _)| row)
                    .collect();
                if rows.is_empty() {
                    return MenuKeyOutcome::Handled;
                }
                let current = self
                    .highlighted_row(level)
                    .and_then(|row| rows.iter().position(|r| *r == row));
                let next = match (key, current) {
                    ("down", Some(i)) => rows[(i + 1) % rows.len()],
                    ("up", Some(i)) => rows[(i + rows.len() - 1) % rows.len()],
                    ("down", None) => rows[0],
                    _ => rows[rows.len() - 1],
                };
                self.set_highlight(level, next);
                self.hover_gen += 1;
                MenuKeyOutcome::Handled
            }
            "right" | "enter" => {
                let Some(row) = self.highlighted_row(level) else {
                    return if key == "enter" {
                        MenuKeyOutcome::Handled
                    } else {
                        MenuKeyOutcome::Ignored
                    };
                };
                if let Some(activation) = self.activation_at(level, row) {
                    return if key == "enter" {
                        MenuKeyOutcome::Activate(activation)
                    } else {
                        MenuKeyOutcome::Ignored
                    };
                }
                self.open_submenu(level, row);
                if self.open.len() > level {
                    let first = self
                        .level_items(level + 1)
                        .and_then(|items| items.iter().position(MenuItem::is_selectable));
                    if let Some(first) = first {
                        self.set_highlight(level + 1, first);
                    }
                }
                self.hover_gen += 1;
                MenuKeyOutcome::Handled
            }
            "left" => {
                if self.open.pop().is_some() {
                    self.hover_gen += 1;
                    MenuKeyOutcome::Handled
                } else {
                    MenuKeyOutcome::Ignored
                }
            }
            "escape" => {
                if self.open.pop().is_some() {
                    self.hover_gen += 1;
                    MenuKeyOutcome::Handled
                } else {
                    MenuKeyOutcome::Close
                }
            }
            _ => MenuKeyOutcome::Ignored,
        }
    }

    /// Every level's on-screen rectangle, in absolute window coordinates,
    /// top level first. A submenu opens beside its row, to the right, and
    /// flips to the left of its parent when it would run past the window's
    /// right edge; it lines its first row up with the parent row and moves
    /// up when it would run past the bottom.
    #[must_use]
    pub fn level_rects(
        &self,
        grid_ox: f32,
        grid_oy: f32,
        vw: f32,
        vh: f32,
        char_width: f32,
    ) -> Vec<MenuRect> {
        let top = self.resolved_position(grid_ox, grid_oy, vw, vh, char_width);
        let mut rects = vec![MenuRect {
            x: grid_ox + f32::from(top.x),
            y: grid_oy + f32::from(top.y),
            w: self.width_for(char_width),
            h: self.total_height(),
        }];
        for (i, open) in self.open.iter().enumerate() {
            let Some(items) = self.level_items(i + 1) else {
                break;
            };
            let parent = rects[i];
            let w = level_width(items, char_width);
            let h = level_height(items);
            let mut x = parent.x + parent.w - SUBMENU_OVERLAP;
            if x + w > vw - MENU_SCREEN_MARGIN {
                x = parent.x - w + SUBMENU_OVERLAP;
            }
            x = x.max(MENU_SCREEN_MARGIN);
            let mut y = parent.y + open.row as f32 * MENU_ITEM_HEIGHT;
            if y + h > vh - MENU_SCREEN_MARGIN {
                y = vh - h - MENU_SCREEN_MARGIN;
            }
            y = y.max(MENU_SCREEN_MARGIN);
            rects.push(MenuRect { x, y, w, h });
        }
        rects
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
pub fn label(action: MenuAction) -> String {
    let key = match action {
        MenuAction::SelectColumn => "datatable.grid.context_menu.select_column",
        MenuAction::CopyColumn => "datatable.grid.context_menu.copy_column",
        MenuAction::CopyColumnWithHeaders => "datatable.grid.context_menu.copy_column_with_headers",
        MenuAction::SortAscending => "datatable.grid.context_menu.sort_ascending",
        MenuAction::SortDescending => "datatable.grid.context_menu.sort_descending",
        MenuAction::ClearSort => "datatable.grid.context_menu.clear_sort",
        MenuAction::GroupBy => "datatable.grid.context_menu.group_by_column",
        MenuAction::ClearGrouping => "datatable.grid.context_menu.clear_grouping",
        MenuAction::FreezeToHere => "datatable.grid.context_menu.freeze_columns_up_to_here",
        MenuAction::UnfreezeColumns => "datatable.grid.context_menu.unfreeze_columns",
        MenuAction::HideColumn => "datatable.grid.context_menu.hide_column",
        MenuAction::ShowAllColumns => "datatable.grid.context_menu.show_all_columns",
        MenuAction::MoveColumnLeft => "datatable.grid.context_menu.move_column_left",
        MenuAction::MoveColumnRight => "datatable.grid.context_menu.move_column_right",
        MenuAction::FilterPrompt => "datatable.grid.context_menu.filter_prompt",
        MenuAction::ClearFilter => "datatable.grid.context_menu.clear_filter",
    };
    crate::localization::text(key)
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
                MenuItem::Action(_) | MenuItem::Custom { .. } | MenuItem::Submenu { .. } => {
                    action_index(&menu.items, idx)
                }
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
#[deprecated(
    since = "3.0.0",
    note = "menu chrome is themed; use `GridTheme::menu_bg` instead"
)]
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
                MenuItem::Action(MenuAction::FreezeToHere) => "FreezeToHere",
                MenuItem::Action(MenuAction::UnfreezeColumns) => "UnfreezeColumns",
                MenuItem::Action(MenuAction::HideColumn) => "HideColumn",
                MenuItem::Action(MenuAction::ShowAllColumns) => "ShowAllColumns",
                MenuItem::Action(MenuAction::MoveColumnLeft) => "MoveColumnLeft",
                MenuItem::Action(MenuAction::MoveColumnRight) => "MoveColumnRight",
                MenuItem::Action(MenuAction::FilterPrompt) => "FilterPrompt",
                MenuItem::Action(MenuAction::ClearFilter) => "ClearFilter",
                MenuItem::Custom { .. } => "Custom",
                MenuItem::Submenu { .. } => "Submenu",
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
                "FreezeToHere",
                "UnfreezeColumns",
                "HideColumn",
                "ShowAllColumns",
                "MoveColumnLeft",
                "MoveColumnRight",
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
        assert_eq!(separators, 4);
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
            MenuAction::FreezeToHere,
            MenuAction::UnfreezeColumns,
            MenuAction::HideColumn,
            MenuAction::ShowAllColumns,
            MenuAction::MoveColumnLeft,
            MenuAction::MoveColumnRight,
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
            open: Vec::new(),
            hover_gen: 0,
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
            Some("Sort Ascending".to_owned())
        );
        assert_eq!(
            MenuItem::Custom {
                id: "x".into(),
                label: "Hello".into()
            }
            .label(),
            Some("Hello".to_owned())
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

    // --- submenus ------------------------------------------------------------

    fn custom(id: &str) -> MenuItem {
        MenuItem::Custom {
            id: id.into(),
            label: id.into(),
        }
    }

    fn sub(label: &str, items: Vec<MenuItem>) -> MenuItem {
        MenuItem::Submenu {
            label: label.into(),
            items,
        }
    }

    fn nested_menu() -> ContextMenu {
        // 0 a | 1 sep | 2 More ▸ [0 b | 1 sep | 2 Deeper ▸ [0 c | 1 d]] | 3 e
        custom_menu_with_items(
            100.0,
            100.0,
            vec![
                custom("a"),
                MenuItem::Separator,
                sub(
                    "More",
                    vec![
                        custom("b"),
                        MenuItem::Separator,
                        sub("Deeper", vec![custom("c"), custom("d")]),
                    ],
                ),
                custom("e"),
            ],
        )
    }

    #[test]
    fn normalize_drops_empty_submenus_and_stray_separators() {
        let tidy = normalize_items(vec![
            MenuItem::Separator,
            custom("a"),
            MenuItem::Separator,
            MenuItem::Separator,
            sub(
                "Empty",
                vec![MenuItem::Separator, sub("Also empty", vec![])],
            ),
            MenuItem::Separator,
            sub(
                "Kept",
                vec![MenuItem::Separator, custom("b"), MenuItem::Separator],
            ),
            MenuItem::Separator,
        ]);
        let shape: Vec<String> = tidy
            .iter()
            .map(|i| match i {
                MenuItem::Separator => "-".to_string(),
                MenuItem::Submenu { label, items } => format!("{label}:{}", items.len()),
                other => other.label().unwrap_or_default().to_string(),
            })
            .collect();
        assert_eq!(shape, ["a", "-", "Kept:1"]);
    }

    #[test]
    fn submenu_rows_are_hoverable_and_widen_the_level() {
        let m = nested_menu();
        // Row 2 ("More") is the second choosable row.
        let y = anchor_y(&m) + MENU_INNER_PAD + 2.0 * MENU_ITEM_HEIGHT;
        assert_eq!(hover_at(&m, 110.0, y, 8.0), Some(1));
        // Wide enough characters that the labels, not the minimum width,
        // decide the width.
        let plain = level_width(&[custom("More")], 60.0);
        assert_eq!(level_width(&m.items, 60.0), plain + SUBMENU_ARROW_WIDTH);
    }

    #[test]
    fn open_submenu_walks_levels_and_closes_deeper_ones() {
        let mut m = nested_menu();
        m.open_submenu(0, 2);
        assert_eq!(m.level_count(), 2);
        assert_eq!(m.level_items(1).map(<[MenuItem]>::len), Some(3));
        m.open_submenu(1, 2);
        assert_eq!(m.level_count(), 3);
        assert_eq!(m.level_items(2).map(<[MenuItem]>::len), Some(2));
        // Re-opening at the top closes both and reopens one level.
        m.open_submenu(0, 2);
        assert_eq!(m.level_count(), 2);
        // An action row closes everything below its level.
        m.open_submenu(0, 0);
        assert_eq!(m.level_count(), 1);
        assert_eq!(m.highlighted_row(0), Some(0));
        assert!(m.level_items(1).is_none());
    }

    #[test]
    fn activation_is_the_same_at_every_level() {
        let mut m = nested_menu();
        m.open_submenu(0, 2);
        m.open_submenu(1, 2);
        assert_eq!(
            m.activation_at(2, 1),
            Some(MenuActivation::Custom("d".into()))
        );
        assert_eq!(
            m.activation_at(0, 3),
            Some(MenuActivation::Custom("e".into()))
        );
        assert_eq!(m.activation_at(0, 2), None, "a submenu row opens instead");
        assert_eq!(m.activation_at(0, 1), None, "a separator does nothing");
    }

    #[test]
    fn keyboard_walks_into_and_out_of_submenus() {
        let mut m = nested_menu();
        assert_eq!(m.key("down"), MenuKeyOutcome::Handled);
        assert_eq!(m.highlighted_row(0), Some(0));
        m.key("down");
        assert_eq!(m.highlighted_row(0), Some(2), "skips the separator");
        assert_eq!(m.key("right"), MenuKeyOutcome::Handled);
        assert_eq!(m.level_count(), 2);
        assert_eq!(m.highlighted_row(1), Some(0), "first row of the child");
        m.key("up");
        assert_eq!(m.highlighted_row(1), Some(2), "wraps, skipping separators");
        assert_eq!(m.key("enter"), MenuKeyOutcome::Handled);
        assert_eq!(m.level_count(), 3);
        m.key("down");
        assert_eq!(
            m.key("enter"),
            MenuKeyOutcome::Activate(MenuActivation::Custom("d".into()))
        );
        assert_eq!(m.key("left"), MenuKeyOutcome::Handled);
        assert_eq!(m.level_count(), 2);
        assert_eq!(m.key("escape"), MenuKeyOutcome::Handled);
        assert_eq!(m.level_count(), 1);
        assert_eq!(
            m.highlighted_row(0),
            Some(2),
            "parent row stays highlighted"
        );
        assert_eq!(m.key("left"), MenuKeyOutcome::Ignored);
        assert_eq!(m.key("right"), MenuKeyOutcome::Handled, "reopens");
        m.key("escape");
        assert_eq!(m.key("escape"), MenuKeyOutcome::Close);
        assert_eq!(m.key("x"), MenuKeyOutcome::Ignored);
    }

    #[test]
    fn right_on_an_action_row_does_nothing() {
        let mut m = nested_menu();
        m.key("down");
        assert_eq!(m.key("right"), MenuKeyOutcome::Ignored);
        assert_eq!(m.level_count(), 1);
    }

    #[test]
    fn submenu_opens_beside_its_row_and_flips_at_window_edges() {
        let mut m = nested_menu();
        m.open_submenu(0, 2);
        let rects = m.level_rects(0.0, 0.0, 2000.0, 2000.0, 8.0);
        assert_eq!(rects.len(), 2);
        let (parent, child) = (rects[0], rects[1]);
        assert_eq!(child.x, parent.x + parent.w - SUBMENU_OVERLAP);
        assert_eq!(child.y, parent.y + 2.0 * MENU_ITEM_HEIGHT);

        // Near the right edge the child flips to the parent's left.
        let mut m = custom_menu_with_items(1700.0, 100.0, nested_menu().items);
        m.open_submenu(0, 2);
        let vw = 1700.0 + m.width_for(8.0) + 20.0;
        let rects = m.level_rects(0.0, 0.0, vw, 2000.0, 8.0);
        assert_eq!(rects[1].x, rects[0].x - rects[1].w + SUBMENU_OVERLAP);

        // Near the bottom the child moves up to stay inside the window.
        let mut m = custom_menu_with_items(100.0, 100.0, nested_menu().items);
        m.open_submenu(0, 2);
        let vh = 100.0 + m.total_height() + 10.0;
        let rects = m.level_rects(0.0, 0.0, 2000.0, vh, 8.0);
        assert!(rects[1].y + rects[1].h <= vh - MENU_SCREEN_MARGIN + 0.01);
        assert!(rects.iter().all(|r| r.contains(r.x + 1.0, r.y + 1.0)));
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
