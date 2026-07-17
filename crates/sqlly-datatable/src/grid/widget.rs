//! The `SqllyDataTable` GPUI widget and its builder. Owns one
//! `Entity<GridState>` and wires GPUI's mouse / keyboard / scroll events to
//! its methods. A bunch of `state.clone()` clones exist because each closure
//! needs its own owned reference to the GPUI entity handle.

use crate::config::GridConfig;
use crate::data::GridData;
use crate::filter::{ColumnFilter, FilterPredicate};
use crate::grid::context_menu::{
    ContextMenuProvider, ContextMenuProviderHandle, PendingCustomContextMenuAction,
};
use crate::grid::paint::{paint_grid, paint_status_bar, PaintData, StatusBarData};
use crate::grid::state::state_inner;
use crate::grid::state::{FilterInput, GridState, EDGE_SCROLL_TICK_MS};
use crate::grid::theme::{GridTheme, GridThemePair};
use crate::grid::{menu, HitResult, MenuItem, SortDirection};
use crate::pivot::config::PivotConfig;
use crate::pivot::context_menu::{PivotContextMenuProvider, PivotContextMenuProviderHandle};
use crate::pivot::sidebar::PivotSidebar;
use crate::pivot::state::{
    PivotSaveConfigHandler, PivotState, DEFAULT_PIVOT_COLUMN_WIDTH, DEFAULT_PIVOT_ROW_HEIGHT,
    DEFAULT_PIVOT_SIDEBAR_WIDTH,
};
use crate::pivot::widget::PivotGrid;

use gpui::prelude::FluentBuilder;
use gpui::{
    anchored, canvas, deferred, div, point, pulsating_between, px, relative, Anchor, Animation,
    AnimationExt, App, AppContext, Context, Div, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Render, ScrollWheelEvent, StatefulInteractiveElement, Styled,
    Window,
};
use gpui_component::resizable::{h_resizable, resizable_panel, ResizableState};
use gpui_component::{Icon, IconName};
use std::sync::Arc;

/// Draw order for the context-menu overlay. Deliberately far above any
/// ordinary application UI so the menu — and, crucially, its event hitbox —
/// sits on top of everything, even content painted outside the grid widget's
/// own layout bounds (e.g. a host header above the grid). Deferred draws
/// register their hitbox in a later pass, so this also fixes hover/click
/// routing for menu items that visually overflow the grid area.
const CONTEXT_MENU_PRIORITY: usize = 1_000_000;
const MIN_PIVOT_SIDEBAR_WIDTH: f32 = 180.0;
const MAX_PIVOT_SIDEBAR_WIDTH: f32 = 480.0;

/// Which view of the data is active when the pivot tab is enabled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridTab {
    /// The flat data grid.
    #[default]
    Grid,
    /// The pivot view (accordion controls + pivot grid).
    Pivot,
}

/// Side of the pivot grid where the control panel is rendered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PivotSidebarPosition {
    /// Render the controls before the pivot grid.
    #[default]
    Left,
    /// Render the controls after the pivot grid.
    Right,
}

/// The entities backing the pivot tab. Created when pivoting is enabled.
pub(crate) struct PivotParts {
    pub(crate) state: Entity<PivotState>,
    grid: Entity<PivotGrid>,
    sidebar: Entity<PivotSidebar>,
}

/// Top-level GPUI widget.
pub struct SqllyDataTable {
    pub state: Entity<GridState>,
    /// Present when the pivot tab is enabled (via
    /// [`SqllyDataTableBuilder::pivot`] or [`SqllyDataTable::enable_pivot`]).
    pub(crate) pivot: Option<PivotParts>,
    /// The active tab. Only meaningful while the pivot tab is enabled.
    active_tab: GridTab,
    /// Prevents opening the pivot while a host is still loading its source
    /// rows. The tab remains visible and may display `pivot_status`.
    pivot_locked: bool,
    pivot_status: Option<String>,
    pivot_sidebar_position: PivotSidebarPosition,
    pivot_sidebar_collapsed: bool,
    pivot_sidebar_width: f32,
    /// Split state owned by `gpui-component`'s resizable panel group. Created
    /// lazily on the first pivot render; dropped (and thus re-seeded from
    /// `pivot_sidebar_width`) when the sidebar switches sides, because panel
    /// sizes in the group state are positional.
    pivot_sidebar_resize: Option<Entity<ResizableState>>,
    /// Set when `set_pivot_sidebar_width` is called so the next render pushes
    /// the programmatic width into the resizable group state (which is
    /// otherwise authoritative once the user has dragged the divider).
    pivot_sidebar_width_dirty: bool,
    /// Human-readable description of the column filters applied by the most
    /// recent pivot drill-through (e.g. `region = East, txn_type = Sale`).
    /// While set, the Grid tab shows a banner naming the filter with a
    /// one-click clear; cleared by [`SqllyDataTable::clear_drill_filter`] or
    /// replaced by the next drill.
    drill_filter: Option<String>,
    /// When `true`, the grid swaps between the built-in light/dark
    /// [`GridTheme`] palettes to follow the OS window appearance. Disabled
    /// automatically when the caller supplies an explicit theme override.
    follow_system_appearance: bool,
    /// Retained appearance-observer subscription. Registered lazily on the
    /// first render (that is where a `Window` is available); dropping it would
    /// unregister the observer, so it is stored for the widget's lifetime.
    appearance_subscription: Option<gpui::Subscription>,
}

impl SqllyDataTable {
    /// Wrap an existing `Entity<GridState>`.
    #[must_use]
    pub fn new(state: Entity<GridState>) -> Self {
        Self {
            state,
            pivot: None,
            active_tab: GridTab::Grid,
            pivot_locked: false,
            pivot_status: None,
            pivot_sidebar_position: PivotSidebarPosition::Left,
            pivot_sidebar_collapsed: false,
            pivot_sidebar_width: DEFAULT_PIVOT_SIDEBAR_WIDTH,
            pivot_sidebar_resize: None,
            pivot_sidebar_width_dirty: false,
            drill_filter: None,
            follow_system_appearance: true,
            appearance_subscription: None,
        }
    }

    /// Construct from `GridData` using the default [`GridConfig`].
    #[must_use]
    pub fn builder(data: GridData) -> SqllyDataTableBuilder {
        SqllyDataTableBuilder {
            data,
            config: GridConfig::default(),
            context_menu_provider: None,
            theme: None,
            theme_family: None,
            debug_bar: false,
            grouped_column: None,
            pivot: None,
            pivot_context_menu_provider: None,
            pivot_save_config_handler: None,
            pivot_sidebar_position: PivotSidebarPosition::Left,
            pivot_sidebar_collapsed: false,
            pivot_sidebar_width: DEFAULT_PIVOT_SIDEBAR_WIDTH,
            pivot_row_height: DEFAULT_PIVOT_ROW_HEIGHT,
            pivot_column_width: DEFAULT_PIVOT_COLUMN_WIDTH,
        }
    }

    /// The pivot state entity, when the pivot tab is enabled. Read it for the
    /// current [`PivotConfig`], row height, or column width; update it to
    /// reconfigure the pivot programmatically.
    #[must_use]
    pub fn pivot_state(&self) -> Option<&Entity<PivotState>> {
        self.pivot.as_ref().map(|p| &p.state)
    }

    /// The currently active tab.
    #[must_use]
    pub fn active_tab(&self) -> GridTab {
        self.active_tab
    }

    /// Swap the light/dark theme family at runtime while keeping automatic
    /// OS light/dark following. The pair's variant matching the current
    /// window appearance is applied immediately; subsequent appearance
    /// changes resolve against the new family. Re-enables appearance
    /// following if it was disabled by an explicit
    /// [`SqllyDataTableBuilder::theme`] override.
    pub fn set_theme_family(
        &mut self,
        family: GridThemePair,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.follow_system_appearance = true;
        let appearance = window.appearance();
        self.state.update(cx, |s, cx| {
            s.theme = family.for_appearance(appearance);
            s.theme_family = family;
            cx.notify();
        });
        cx.notify();
    }

    /// Whether the visible pivot tab currently rejects activation.
    #[must_use]
    pub fn pivot_locked(&self) -> bool {
        self.pivot_locked
    }

    /// Side of the pivot grid where the control panel is rendered.
    #[must_use]
    pub fn pivot_sidebar_position(&self) -> PivotSidebarPosition {
        self.pivot_sidebar_position
    }

    /// Move the pivot control panel to the left or right of the pivot grid.
    pub fn set_pivot_sidebar_position(&mut self, position: PivotSidebarPosition) {
        if self.pivot_sidebar_position != position {
            // Panel sizes in the resizable group state are positional, so a
            // side switch must re-seed the group from `pivot_sidebar_width`.
            self.pivot_sidebar_resize = None;
        }
        self.pivot_sidebar_position = position;
    }

    /// Whether the pivot control panel is collapsed into its divider.
    #[must_use]
    pub fn pivot_sidebar_collapsed(&self) -> bool {
        self.pivot_sidebar_collapsed
    }

    /// Collapse or expand the pivot control panel.
    pub fn set_pivot_sidebar_collapsed(&mut self, collapsed: bool) {
        self.pivot_sidebar_collapsed = collapsed;
    }

    /// Current width of the expanded pivot control panel, in pixels.
    #[must_use]
    pub fn pivot_sidebar_width(&self) -> f32 {
        self.pivot_sidebar_width
    }

    /// Resize the pivot control panel, clamped to its supported range.
    pub fn set_pivot_sidebar_width(&mut self, width: f32) {
        self.pivot_sidebar_width = width.clamp(MIN_PIVOT_SIDEBAR_WIDTH, MAX_PIVOT_SIDEBAR_WIDTH);
        self.pivot_sidebar_width_dirty = true;
    }

    /// Human-readable description of the column filters applied by the most
    /// recent pivot drill-through, while they are still marked as such. The
    /// Grid tab shows this in a banner with a one-click clear.
    #[must_use]
    pub fn drill_filter(&self) -> Option<&str> {
        self.drill_filter.as_deref()
    }

    /// Clear the filters applied by a pivot drill-through: every column
    /// filter is reset and the Grid tab's drill banner is dismissed.
    pub fn clear_drill_filter(&mut self, cx: &mut Context<Self>) {
        if self.drill_filter.take().is_none() {
            return;
        }
        self.state.update(cx, |g, cx| {
            for filter in &mut g.filters {
                *filter = ColumnFilter::default();
            }
            g.recompute();
            cx.notify();
        });
        cx.notify();
    }

    /// Lock or unlock the pivot tab while keeping it visible. `status` is
    /// rendered beside the Pivot title while locked, for example `Loading`.
    /// Locking an active pivot returns immediately to the flat grid so a host
    /// can continue streaming rows without recomputing the pivot snapshot.
    pub fn set_pivot_locked(&mut self, locked: bool, status: Option<String>) {
        self.pivot_locked = locked;
        self.pivot_status = locked.then_some(status).flatten();
        if locked {
            self.active_tab = GridTab::Grid;
        }
    }

    /// Switch between the flat grid and the pivot view. Switching to the
    /// pivot re-syncs its source snapshot if the grid's data changed (e.g.
    /// rows were appended). No-op when the pivot tab is not enabled and
    /// `Pivot` is requested.
    pub fn set_active_tab(&mut self, tab: GridTab, cx: &mut App) {
        if tab == GridTab::Pivot {
            if self.pivot.is_none() || self.pivot_locked {
                return;
            }
            self.sync_pivot_source(cx);
        }
        self.active_tab = tab;
    }

    /// Enable the pivot tab at runtime with the given configuration. If
    /// already enabled, the existing pivot state is reconfigured instead
    /// (collapse/sort/filter state is preserved).
    pub fn enable_pivot(&mut self, config: PivotConfig, cx: &mut App) {
        if let Some(parts) = &self.pivot {
            parts.state.update(cx, |s, cx| {
                s.config = config;
                s.recompute();
                cx.notify();
            });
            return;
        }
        self.pivot = Some(build_pivot_parts(
            &self.state,
            config,
            None,
            None,
            DEFAULT_PIVOT_ROW_HEIGHT,
            DEFAULT_PIVOT_COLUMN_WIDTH,
            cx,
        ));
    }

    /// Remove the pivot tab and return to the flat grid.
    pub fn disable_pivot(&mut self) {
        self.pivot = None;
        self.active_tab = GridTab::Grid;
    }

    /// Register (or replace) the pivot's right-click menu provider at
    /// runtime. No-op when the pivot tab is not enabled — enable it first
    /// (or register via
    /// [`SqllyDataTableBuilder::pivot_context_menu_provider`]).
    pub fn set_pivot_context_menu_provider(
        &mut self,
        provider: impl PivotContextMenuProvider + 'static,
        cx: &mut App,
    ) {
        if let Some(parts) = &self.pivot {
            parts.state.update(cx, |s, _cx| {
                s.set_context_menu_provider(provider);
            });
        }
    }

    /// Register (or replace) the pivot's save-configuration action at
    /// runtime. While registered, the pivot controls sidebar shows a save
    /// button next to the Layout section that invokes `handler` with the
    /// live [`PivotConfig`]. No-op when the pivot tab is not enabled — enable
    /// it first (or register via
    /// [`SqllyDataTableBuilder::pivot_save_config`]).
    pub fn set_pivot_save_config(
        &mut self,
        handler: impl Fn(&PivotConfig, &mut App) + 'static,
        cx: &mut App,
    ) {
        if let Some(parts) = &self.pivot {
            parts.state.update(cx, |s, cx| {
                s.on_save_config(handler);
                cx.notify();
            });
        }
    }

    /// Remove the pivot's save-configuration action; the sidebar's save
    /// button disappears. No-op when the pivot tab is not enabled.
    pub fn clear_pivot_save_config(&mut self, cx: &mut App) {
        if let Some(parts) = &self.pivot {
            parts.state.update(cx, |s, cx| {
                s.clear_save_config_handler();
                cx.notify();
            });
        }
    }

    /// Push the grid's current data snapshot into the pivot state when it
    /// changed (O(1) compare via Arc identity).
    fn sync_pivot_source(&self, cx: &mut App) {
        let Some(parts) = &self.pivot else {
            return;
        };
        let (columns, rows) = {
            let s = self.state.read(cx);
            (s.data.columns.clone(), Arc::clone(&s.data_rows))
        };
        parts.state.update(cx, |ps, cx| {
            if ps.source_differs(&rows) {
                ps.set_source(columns, rows);
                cx.notify();
            }
        });
    }
}

/// Create the pivot entities over the grid's current data snapshot.
fn build_pivot_parts(
    grid_state: &Entity<GridState>,
    config: PivotConfig,
    menu_provider: Option<PivotContextMenuProviderHandle>,
    save_config_handler: Option<PivotSaveConfigHandler>,
    row_height: f32,
    column_width: f32,
    cx: &mut App,
) -> PivotParts {
    let (columns, rows, formats, key_bindings, theme, animations) = {
        let s = grid_state.read(cx);
        (
            s.data.columns.clone(),
            Arc::clone(&s.data_rows),
            s.resolved_formats.as_ref().clone(),
            s.config.key_bindings.clone(),
            s.theme.clone(),
            s.config.animations,
        )
    };
    let focus = cx.focus_handle();
    let state = cx.new(|_| {
        let mut ps = PivotState::new(columns, rows, formats, config, key_bindings, focus);
        ps.theme = theme;
        ps.animations = animations;
        ps.context_menu_provider = menu_provider;
        ps.save_config_handler = save_config_handler;
        ps.set_row_height(row_height);
        ps.set_column_width(column_width);
        ps
    });
    let grid = cx.new(|_| PivotGrid::new(state.clone()));
    let sidebar = cx.new(|_| PivotSidebar::new(state.clone()));
    PivotParts {
        state,
        grid,
        sidebar,
    }
}

/// Builder for `SqllyDataTable`.
pub struct SqllyDataTableBuilder {
    data: GridData,
    config: GridConfig,
    context_menu_provider: Option<ContextMenuProviderHandle>,
    theme: Option<GridTheme>,
    theme_family: Option<GridThemePair>,
    debug_bar: bool,
    grouped_column: Option<usize>,
    pivot: Option<PivotConfig>,
    pivot_context_menu_provider: Option<PivotContextMenuProviderHandle>,
    pivot_save_config_handler: Option<PivotSaveConfigHandler>,
    pivot_sidebar_position: PivotSidebarPosition,
    pivot_sidebar_collapsed: bool,
    pivot_sidebar_width: f32,
    pivot_row_height: f32,
    pivot_column_width: f32,
}

impl SqllyDataTableBuilder {
    /// Override the entire [`GridConfig`].
    #[must_use]
    pub fn config(mut self, config: GridConfig) -> Self {
        self.config = config;
        self
    }

    /// Override the [`GridTheme`]. Supplying an explicit theme opts out of the
    /// automatic OS light/dark following; the grid uses exactly this theme.
    #[must_use]
    pub fn theme(mut self, theme: GridTheme) -> Self {
        self.theme = Some(theme);
        self
    }

    /// Choose the light/dark theme family used while following the OS window
    /// appearance (default: [`GridThemePair::neutral`]). Unlike
    /// [`SqllyDataTableBuilder::theme`], this keeps automatic light/dark
    /// following — the pair's matching variant is applied whenever the
    /// system appearance changes. Ignored when an explicit theme override is
    /// also supplied.
    #[must_use]
    pub fn theme_family(mut self, family: GridThemePair) -> Self {
        self.theme_family = Some(family);
        self
    }

    /// Register a custom right-click menu provider. When registered, the
    /// provider fully controls the right-click menu for all targets (cells,
    /// row headers, column headers). The built-in column-header menu is
    /// suppressed; use
    /// [`crate::grid::context_menu::ContextMenuItem::standard_column_header_items`]
    /// to compose built-in actions.
    #[must_use]
    pub fn context_menu_provider(mut self, provider: impl ContextMenuProvider + 'static) -> Self {
        self.context_menu_provider = Some(ContextMenuProviderHandle::new(provider));
        self
    }

    /// Enable or disable the debug status bar. When enabled, a bar is painted
    /// at the bottom of the grid showing click position, scroll offset, and
    /// hovered cell coordinates. Off by default.
    #[must_use]
    pub fn debug_bar(mut self, enabled: bool) -> Self {
        self.debug_bar = enabled;
        self
    }

    /// Group the initial flat-grid rows into expandable sections using the
    /// formatted values in `column`. Invalid indices are ignored at build time.
    #[must_use]
    pub fn group_by_column(mut self, column: usize) -> Self {
        self.grouped_column = Some(column);
        self
    }

    /// Enable the pivot tab, preconfigured with `config`. The widget renders
    /// a "Grid" / "Pivot" tab bar; the pivot tab shows resizable accordion
    /// controls next to the pivot grid. Pass
    /// [`PivotConfig::default()`] for an unconfigured pivot the user builds
    /// interactively.
    #[must_use]
    pub fn pivot(mut self, config: PivotConfig) -> Self {
        self.pivot = Some(config);
        self
    }

    /// Place the pivot control panel on the left or right side of the grid.
    #[must_use]
    pub fn pivot_sidebar_position(mut self, position: PivotSidebarPosition) -> Self {
        self.pivot_sidebar_position = position;
        self
    }

    /// Build the pivot control panel initially collapsed.
    #[must_use]
    pub fn pivot_sidebar_collapsed(mut self, collapsed: bool) -> Self {
        self.pivot_sidebar_collapsed = collapsed;
        self
    }

    /// Set the initial width of the expanded pivot control panel.
    #[must_use]
    pub fn pivot_sidebar_width(mut self, width: f32) -> Self {
        self.pivot_sidebar_width = width.clamp(MIN_PIVOT_SIDEBAR_WIDTH, MAX_PIVOT_SIDEBAR_WIDTH);
        self
    }

    /// Set the initial height of every row in the pivot view.
    #[must_use]
    pub fn pivot_row_height(mut self, height: f32) -> Self {
        if height.is_finite() {
            self.pivot_row_height = height.max(crate::pivot::state::MIN_PIVOT_ROW_HEIGHT);
        }
        self
    }

    /// Set the initial width of every value column in the pivot view.
    #[must_use]
    pub fn pivot_column_width(mut self, width: f32) -> Self {
        if width.is_finite() {
            self.pivot_column_width = width.max(crate::pivot::state::MIN_PIVOT_COLUMN_WIDTH);
        }
        self
    }

    /// Register a custom right-click menu provider for the pivot view. When
    /// registered, the provider fully controls the pivot's context menu (the
    /// built-in `pivot.*` action ids remain handled by the pivot; compose
    /// them via [`crate::pivot::PivotMenuItem::standard_items`]). Only takes
    /// effect together with [`SqllyDataTableBuilder::pivot`].
    #[must_use]
    pub fn pivot_context_menu_provider(
        mut self,
        provider: impl PivotContextMenuProvider + 'static,
    ) -> Self {
        self.pivot_context_menu_provider = Some(PivotContextMenuProviderHandle::new(provider));
        self
    }

    /// Register a save-configuration action for the pivot view. While
    /// registered, the pivot controls sidebar shows a save button next to
    /// the Layout section that invokes `handler` with the live
    /// [`PivotConfig`] (persist it and pass it back to
    /// [`SqllyDataTableBuilder::pivot`] on the next launch). Without a
    /// handler the button is not rendered. Only takes effect together with
    /// [`SqllyDataTableBuilder::pivot`].
    #[must_use]
    pub fn pivot_save_config(mut self, handler: impl Fn(&PivotConfig, &mut App) + 'static) -> Self {
        self.pivot_save_config_handler = Some(std::rc::Rc::new(handler));
        self
    }

    /// Build the widget inside the supplied [`gpui::App`].
    pub fn build(self, cx: &mut App) -> SqllyDataTable {
        let focus = cx.focus_handle();
        let provider = self.context_menu_provider;
        let theme_override = self.theme;
        let theme_family = self.theme_family;
        let debug_bar = self.debug_bar;
        let grouped_column = self.grouped_column;
        let pivot_config = self.pivot;
        let pivot_sidebar_position = self.pivot_sidebar_position;
        let pivot_sidebar_collapsed = self.pivot_sidebar_collapsed;
        let pivot_sidebar_width = self.pivot_sidebar_width;
        let pivot_row_height = self.pivot_row_height;
        let pivot_column_width = self.pivot_column_width;
        let follow_system_appearance = theme_override.is_none();
        let state = cx.new(|cx| {
            let mut s = GridState::new(self.data, self.config, focus.clone());
            s.context_menu_provider = provider;
            s.debug_bar_enabled = debug_bar;
            s.set_grouped_column(grouped_column);
            s.self_weak = Some(cx.weak_entity());
            if let Some(family) = theme_family {
                s.theme = family.light.clone();
                s.theme_family = family;
            }
            if let Some(theme) = theme_override {
                s.theme = theme;
            }
            s
        });
        let pivot_menu_provider = self.pivot_context_menu_provider;
        let pivot_save_config_handler = self.pivot_save_config_handler;
        let pivot = pivot_config.map(|cfg| {
            build_pivot_parts(
                &state,
                cfg,
                pivot_menu_provider,
                pivot_save_config_handler,
                pivot_row_height,
                pivot_column_width,
                cx,
            )
        });
        SqllyDataTable {
            state,
            pivot,
            active_tab: GridTab::Grid,
            pivot_locked: false,
            pivot_status: None,
            pivot_sidebar_position,
            pivot_sidebar_collapsed,
            pivot_sidebar_width,
            pivot_sidebar_resize: None,
            pivot_sidebar_width_dirty: false,
            drill_filter: None,
            follow_system_appearance,
            appearance_subscription: None,
        }
    }
}

impl Focusable for SqllyDataTable {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.state.read(cx).focus_handle.clone()
    }
}

impl Render for SqllyDataTable {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        // Follow the OS light/dark appearance. An observer swaps the theme
        // whenever the system appearance changes, but we also reconcile the
        // resolved theme against `window.appearance()` on every render: the
        // appearance read on the very first frame can be stale (reported
        // before the window has settled onto the OS appearance), which would
        // otherwise strand the grid in the light variant on a dark-mode OS
        // until the next appearance *change* event. Re-resolving each render
        // self-heals that — it clones + compares one theme and only writes
        // (and notifies) when the variant actually differs, so a steady state
        // is a cheap no-op. Skipped entirely when the caller supplied an
        // explicit theme override.
        if self.follow_system_appearance {
            let appearance = window.appearance();
            let resolved = self.state.read(cx).theme_family.for_appearance(appearance);
            if self.state.read(cx).theme != resolved {
                self.state.update(cx, |s, cx| {
                    s.theme = resolved;
                    cx.notify();
                });
            }
            if self.appearance_subscription.is_none() {
                let state_appearance = self.state.clone();
                self.appearance_subscription =
                    Some(window.observe_window_appearance(move |window, cx| {
                        let appearance = window.appearance();
                        state_appearance.update(cx, |s, cx| {
                            let resolved = s.theme_family.for_appearance(appearance);
                            if s.theme != resolved {
                                s.theme = resolved;
                                cx.notify();
                            }
                        });
                    }));
            }
        }

        // Keep the pivot's theme in lockstep with the grid theme (which may
        // have just changed via the appearance observer), and its sidebar
        // width in sync with the resizable panel (the sidebar needs it to
        // decide when chip labels are truncated).
        if let Some(parts) = &self.pivot {
            let grid_theme = self.state.read(cx).theme.clone();
            let sidebar_width = self.pivot_sidebar_width;
            parts.state.update(cx, |s, cx| {
                let mut dirty = false;
                if s.theme != grid_theme {
                    s.theme = grid_theme;
                    dirty = true;
                }
                if (s.sidebar_width - sidebar_width).abs() > 0.5 {
                    s.sidebar_width = sidebar_width;
                    dirty = true;
                }
                if dirty {
                    cx.notify();
                }
            });
        }

        // Drill-through: a double-click on a pivot cell (or the built-in
        // "Show source rows" menu action) queued per-column value filters.
        // Apply them to the flat grid and switch to the Grid tab so the user
        // lands on exactly the rows that drive the clicked cell.
        if let Some(parts) = &self.pivot {
            let drill = parts.state.update(cx, |s, _cx| s.take_pending_drill_down());
            if let Some(filter_sets) = drill {
                let mut applied = false;
                self.state.update(cx, |g, cx| {
                    // Filters are unsupported in windowed-row mode; the
                    // drill-through is skipped rather than presenting a
                    // filter that silently covers only resident rows.
                    if g.window.is_none() {
                        for filter in &mut g.filters {
                            *filter = ColumnFilter::default();
                        }
                        for (field, values) in &filter_sets {
                            if let Some(slot) = g.filters.get_mut(*field) {
                                *slot = ColumnFilter {
                                    predicate: FilterPredicate::None,
                                    values: Some(values.clone()),
                                };
                            }
                        }
                        g.recompute();
                        applied = true;
                    }
                    cx.notify();
                });
                if applied {
                    self.drill_filter =
                        Some(drill_filter_label(&self.state.read(cx).data, &filter_sets));
                }
                self.active_tab = GridTab::Grid;
                let focus = self.state.read(cx).focus_handle.clone();
                window.focus(&focus, cx);
                cx.notify();
            }
        }

        let grid_view = self.render_grid_view(cx);

        let Some(parts) = &self.pivot else {
            return div().size_full().child(grid_view);
        };

        let theme = self.state.read(cx).theme.clone();
        let tab = |label: String,
                   this_tab: GridTab,
                   active: bool,
                   locked: bool,
                   cx: &mut Context<Self>| {
            div()
                .px(px(14.0))
                .py(px(5.0))
                .text_size(px(13.0))
                .when(!locked, |tab| tab.cursor_pointer())
                .when(locked, |tab| tab.opacity(0.55))
                .bg(if active { theme.bg } else { theme.header_bg })
                .text_color(if active {
                    theme.header_fg
                } else {
                    theme.muted_text
                })
                .border_b_2()
                .border_color(if active {
                    theme.sort_indicator
                } else {
                    theme.header_bg
                })
                .child(label)
                .when(!locked, |tab| {
                    tab.on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, window, cx| {
                            if this.active_tab == this_tab {
                                return;
                            }
                            this.set_active_tab(this_tab, cx);
                            // Route keyboard input to the now-visible view.
                            match this_tab {
                                GridTab::Grid => {
                                    let focus = this.state.read(cx).focus_handle.clone();
                                    window.focus(&focus, cx);
                                }
                                GridTab::Pivot => {
                                    if let Some(p) = &this.pivot {
                                        let focus = p.state.read(cx).focus_handle.clone();
                                        window.focus(&focus, cx);
                                    }
                                }
                            }
                            cx.notify();
                        }),
                    )
                })
        };

        let pivot_label = self
            .pivot_status
            .as_deref()
            .map_or_else(|| "Pivot".to_string(), |status| format!("Pivot  {status}"));

        let tab_bar = div()
            .flex()
            .flex_row()
            .flex_none()
            .bg(theme.header_bg)
            .border_b_1()
            .border_color(theme.grid_line)
            .child(tab(
                "Grid".to_string(),
                GridTab::Grid,
                self.active_tab == GridTab::Grid,
                false,
                cx,
            ))
            .child(tab(
                pivot_label,
                GridTab::Pivot,
                self.active_tab == GridTab::Pivot,
                self.pivot_locked,
                cx,
            ));

        let content: gpui::AnyElement = match self.active_tab {
            GridTab::Grid => {
                // While a pivot drill-through's filters are active, a banner
                // names them and offers a one-click clear — otherwise the
                // filtered grid is indistinguishable from the full dataset.
                if let Some(label) = self.drill_filter.clone() {
                    let table_clear = cx.entity().clone();
                    let hover_bg = theme.menu_hover_bg;
                    let banner = div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .px(px(10.0))
                        .py(px(4.0))
                        .bg(theme.filter_active_bg)
                        .border_b_1()
                        .border_color(theme.grid_line)
                        .text_size(px(12.0))
                        .text_color(theme.header_fg)
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .truncate()
                                .child(format!("Filtered from pivot: {label}")),
                        )
                        .child(
                            div()
                                .id("clear-drill-filter")
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap(px(4.0))
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .hover(move |style| style.bg(hover_bg))
                                .child(Icon::new(IconName::CircleX))
                                .child("Clear filter")
                                .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                                    table_clear.update(cx, |table, cx| {
                                        table.clear_drill_filter(cx);
                                    });
                                }),
                        );
                    div()
                        .flex()
                        .flex_col()
                        .size_full()
                        .child(banner)
                        .child(div().flex_1().min_h_0().child(grid_view))
                        .into_any_element()
                } else {
                    grid_view.into_any_element()
                }
            }
            GridTab::Pivot => {
                let position = self.pivot_sidebar_position;
                let collapsed = self.pivot_sidebar_collapsed;

                if collapsed {
                    // Collapsed: a slim labelled rail where the sidebar was;
                    // clicking it expands the sidebar again.
                    let rail = pivot_collapsed_rail(&theme, position, cx.entity().clone());
                    let pivot_grid = div().flex_1().min_w_0().child(parts.grid.clone());
                    let pivot_view = div().flex().flex_row().size_full();
                    let pivot_view = match position {
                        PivotSidebarPosition::Left => pivot_view.child(rail).child(pivot_grid),
                        PivotSidebarPosition::Right => pivot_view.child(pivot_grid).child(rail),
                    };
                    pivot_view.into_any_element()
                } else {
                    // Expanded: a `gpui-component` resizable split. The group
                    // state owns the live panel sizes; drag-to-resize is
                    // handled entirely by the library's resize handle.
                    let resize_state = match self.pivot_sidebar_resize.clone() {
                        Some(state) => state,
                        None => {
                            let state = cx.new(|_| ResizableState::default());
                            self.pivot_sidebar_resize = Some(state.clone());
                            state
                        }
                    };
                    let sidebar_ix = match position {
                        PivotSidebarPosition::Left => 0,
                        PivotSidebarPosition::Right => 1,
                    };
                    // Push a programmatic `set_pivot_sidebar_width` into the
                    // group state (a no-op on the very first render, where the
                    // panel seeds itself from `.size()` below).
                    if self.pivot_sidebar_width_dirty {
                        self.pivot_sidebar_width_dirty = false;
                        let width = px(self.pivot_sidebar_width);
                        resize_state.update(cx, |state, cx| {
                            state.resize_panel(sidebar_ix, width, window, cx);
                        });
                    }

                    let toggle_strip = pivot_toggle_strip(&theme, position, cx.entity().clone());
                    let sidebar_body = div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(parts.sidebar.clone());
                    let sidebar_content = div().flex().flex_row().size_full();
                    let sidebar_content = match position {
                        PivotSidebarPosition::Left => {
                            sidebar_content.child(sidebar_body).child(toggle_strip)
                        }
                        PivotSidebarPosition::Right => {
                            sidebar_content.child(toggle_strip).child(sidebar_body)
                        }
                    };
                    let sidebar_panel = resizable_panel()
                        .size(px(self.pivot_sidebar_width))
                        .size_range(px(MIN_PIVOT_SIDEBAR_WIDTH)..px(MAX_PIVOT_SIDEBAR_WIDTH))
                        .child(sidebar_content);
                    let grid_panel = resizable_panel()
                        .child(div().size_full().min_w_0().child(parts.grid.clone()));

                    let table_resize = cx.entity().clone();
                    let group = h_resizable("pivot-sidebar-split")
                        .with_state(&resize_state)
                        .on_resize(move |state, _window, cx| {
                            let Some(width) = state.read(cx).sizes().get(sidebar_ix).copied()
                            else {
                                return;
                            };
                            table_resize.update(cx, |table, cx| {
                                // Direct field write: `set_pivot_sidebar_width`
                                // would mark the width dirty and push it right
                                // back into the group state next render.
                                table.pivot_sidebar_width = f32::from(width)
                                    .clamp(MIN_PIVOT_SIDEBAR_WIDTH, MAX_PIVOT_SIDEBAR_WIDTH);
                                cx.notify();
                            });
                        });
                    let group = match position {
                        PivotSidebarPosition::Left => group.child(sidebar_panel).child(grid_panel),
                        PivotSidebarPosition::Right => group.child(grid_panel).child(sidebar_panel),
                    };
                    group.into_any_element()
                }
            }
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(tab_bar)
            .child(div().flex_1().min_h_0().child(content))
    }
}

/// Human-readable summary of a drill-through's per-column value filters,
/// e.g. `region = East, location = Boston, txn_type = Sale`. Multi-value
/// sets list up to three values; longer sets are elided with a count.
fn drill_filter_label(
    data: &crate::data::GridData,
    filter_sets: &[(usize, std::collections::HashSet<String>)],
) -> String {
    let mut parts = Vec::with_capacity(filter_sets.len());
    for (field, values) in filter_sets {
        let name = data.columns.get(*field).map_or("?", |c| c.name.as_str());
        let mut sorted: Vec<&str> = values.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        let shown = match sorted.len() {
            0 => continue,
            1..=3 => sorted.join(" | "),
            n => format!("{} | … ({n} values)", sorted[..2].join(" | ")),
        };
        parts.push(format!("{name} = {shown}"));
    }
    parts.join(", ")
}

/// The slim labelled rail shown in place of the pivot sidebar while it is
/// collapsed. Ports the old `gpui-ui-kit` pane-divider collapsed strip:
/// expand arrows above and below a vertically stacked label; any click
/// expands the sidebar.
fn pivot_collapsed_rail(
    theme: &GridTheme,
    position: PivotSidebarPosition,
    table: Entity<SqllyDataTable>,
) -> impl IntoElement {
    // Lucide panel-open icons pointing toward where the sidebar reappears.
    let icon_name = match position {
        PivotSidebarPosition::Left => IconName::PanelLeftOpen,
        PivotSidebarPosition::Right => IconName::PanelRightOpen,
    };
    let label_chars = "Pivot".chars().map({
        let fg = theme.muted_text;
        move |c| {
            div()
                .text_color(fg)
                .text_size(px(11.0))
                .child(c.to_string())
        }
    });
    let hover_bg = theme.menu_hover_bg;
    let arrow_glyph = move |fg: gpui::Hsla| {
        div()
            .text_color(fg)
            .text_size(px(13.0))
            .child(Icon::new(icon_name.clone()))
    };

    div()
        .id("pivot-sidebar-rail")
        .w(px(24.0))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(4.0))
        .bg(theme.menu_bg)
        .border_x_1()
        .border_color(theme.grid_line)
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .child(arrow_glyph(theme.muted_text))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.0))
                .py_2()
                .children(label_chars),
        )
        .child(arrow_glyph(theme.muted_text))
        .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
            table.update(cx, |table, cx| {
                table.set_pivot_sidebar_collapsed(false);
                cx.notify();
            });
        })
}

/// The slim collapse strip on the pivot sidebar's inner edge while expanded.
/// Clicking it collapses the sidebar into [`pivot_collapsed_rail`]. Sits
/// beside the resizable group's drag handle, replacing the old pane-divider
/// double-click-to-collapse affordance with a single click.
fn pivot_toggle_strip(
    theme: &GridTheme,
    position: PivotSidebarPosition,
    table: Entity<SqllyDataTable>,
) -> impl IntoElement {
    // Lucide panel-close icon pointing toward where the sidebar collapses.
    let icon_name = match position {
        PivotSidebarPosition::Left => IconName::PanelLeftClose,
        PivotSidebarPosition::Right => IconName::PanelRightClose,
    };
    let hover_bg = theme.menu_hover_bg;

    div()
        .id("pivot-sidebar-toggle")
        .w(px(16.0))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .bg(theme.header_bg)
        .border_x_1()
        .border_color(theme.grid_line)
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .child(
            div()
                .text_color(theme.muted_text)
                .text_size(px(13.0))
                .child(Icon::new(icon_name)),
        )
        .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
            table.update(cx, |table, cx| {
                table.set_pivot_sidebar_collapsed(true);
                cx.notify();
            });
        })
}

impl SqllyDataTable {
    /// The flat-grid element tree (canvas, overlays, input handlers). Shared
    /// by the tabless render path and the "Grid" tab.
    fn render_grid_view(&mut self, cx: &mut Context<Self>) -> Div {
        let state_canvas = self.state.clone();
        let state_status = self.state.clone();
        let state_mouse = self.state.clone();
        let state_move = self.state.clone();
        let state_up = self.state.clone();
        let state_scroll = self.state.clone();
        let state_key = self.state.clone();
        let state_right = self.state.clone();
        let bg = self.state.read(cx).theme.bg;
        let focus_handle = self.state.read(cx).focus_handle.clone();
        let focus_left = focus_handle.clone();
        let focus_right = focus_handle.clone();
        let debug_bar = self.state.read(cx).debug_bar_enabled;
        let status_h = self.state.read(cx).status_bar_height;

        // Process any pending menu action from a previous mouse-down on a
        // menu item (needs App access for clipboard).
        if let Some((action, col)) = self.state.read(cx).pending_action {
            self.state.update(cx, |s, cx| {
                s.execute_action(action, col, cx);
                s.pending_action = None;
            });
        }

        // Process any pending custom context-menu action.
        if let Some(pending) = self
            .state
            .read(cx)
            .pending_custom_context_menu_action
            .clone()
        {
            self.state.update(cx, |s, cx| {
                s.pending_custom_context_menu_action = None;
                s.execute_custom_context_menu_action(pending, cx);
            });
        }

        // Spawn an edge-scroll timer **only while a drag is in progress**, and
        // **only one at a time**. Without the `edge_scroll_active` guard,
        // `render` would spawn a fresh 16 ms loop on every frame/notify during
        // a drag — each successful tick calls `cx.notify()`, which re-renders
        // and spawned yet another task, stacking concurrent loops that each
        // apply a scroll delta per tick and multiply the effective speed
        // without bound. The task clears the flag when it exits.
        if self.state.read(cx).is_dragging && !self.state.read(cx).edge_scroll_active {
            self.state.update(cx, |s, _cx| s.edge_scroll_active = true);
            let state_edge = self.state.clone();
            cx.spawn(async move |_weak, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(EDGE_SCROLL_TICK_MS))
                        .await;
                    let scrolled =
                        cx.update(|cx| state_edge.update(cx, |s, _cx| s.apply_edge_scroll()));
                    if scrolled {
                        state_edge.update(cx, |_s, cx| cx.notify());
                    }
                    let dragging = cx.update(|cx| state_edge.read(cx).is_dragging);
                    if !dragging {
                        break;
                    }
                }
                cx.update(|cx| state_edge.update(cx, |s, _cx| s.edge_scroll_active = false));
            })
            .detach();
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .relative()
            .track_focus(&focus_handle)
            .bg(bg)
            .child(
                canvas(
                    move |bounds, window, cx| -> PaintData {
                        let viewport = window.viewport_size();
                        state_canvas.update(cx, |s, cx| {
                            let mut dirty = false;
                            if s.bounds != bounds {
                                s.bounds = bounds;
                                s.clamp_scroll_to_bounds();
                                dirty = true;
                            }
                            if s.window_viewport != viewport {
                                s.window_viewport = viewport;
                            }
                            if dirty {
                                cx.notify();
                            }
                        });
                        let s = state_canvas.read(cx);
                        let mut data = PaintData::from_state(s);
                        data.focused = s.focus_handle.is_focused(window);
                        data
                    },
                    move |bounds, data, window, cx| {
                        paint_grid(&data, window, cx, bounds);
                    },
                )
                .flex_1(),
            )
            .children(debug_bar.then(|| {
                canvas(
                    move |_bounds, _window, cx| -> StatusBarData {
                        let s = state_status.read(cx);
                        StatusBarData::from_state(s)
                    },
                    move |bounds, data, window, cx| {
                        paint_status_bar(&data, window, cx, bounds);
                    },
                )
                .h(px(status_h))
            }))
            .children(render_context_menu_overlay(&self.state, cx))
            .children(render_filter_panel_overlay(&self.state, cx))
            .children(render_busy_overlay(&self.state, cx))
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, window, cx| {
                    window.focus(&focus_left, cx);
                    state_mouse.update(cx, |s, cx| {
                        // Ignore grid input while a background task is running;
                        // the busy overlay is shown and occludes interaction.
                        if s.busy.is_some() {
                            return;
                        }
                        // Normalize the absolute window pointer into the grid's
                        // own frame. Menu hit-testing is handled by the deferred
                        // overlay's own item handlers, so a left-click that
                        // reaches the grid means the pointer was NOT on the menu;
                        // dismiss any open menu and proceed with grid selection.
                        let rel = state_inner::to_grid_relative(event.position, s.bounds.origin);
                        if s.context_menu.is_some() || s.filter_panel.is_some() {
                            s.context_menu = None;
                            s.filter_panel = None;
                        }
                        s.handle_mouse_down_with_modifiers(
                            rel,
                            event.modifiers.shift,
                            event.modifiers.platform,
                        );
                        cx.notify();
                    });
                },
            )
            .on_mouse_down(
                MouseButton::Right,
                move |event: &MouseDownEvent, window, cx| {
                    window.focus(&focus_right, cx);
                    state_right.update(cx, |s, cx| {
                        if s.busy.is_some() {
                            return;
                        }
                        let pos = state_inner::to_grid_relative(event.position, s.bounds.origin);
                        let hit = s.hit_test(pos);

                        // No provider — existing built-in behavior.
                        if s.context_menu_provider.is_none() {
                            match hit {
                                HitResult::ColumnHeader(col) | HitResult::SortButton(col) => {
                                    s.open_context_menu(col, pos);
                                }
                                _ => {
                                    s.context_menu = None;
                                    s.filter_panel = None;
                                }
                            }
                            cx.notify();
                            return;
                        }

                        // Provider exists — build custom menu.
                        let Some(target) = s.context_menu_target_from_hit(hit) else {
                            s.context_menu = None;
                            s.filter_panel = None;
                            cx.notify();
                            return;
                        };

                        let effective = s.effective_selection_for_context_target(&target);
                        if effective != s.selection {
                            s.selection = effective.clone();
                        }

                        let request = s.build_context_menu_request(target, &effective);
                        let col = request.target.column_index().unwrap_or(0);

                        let Some(provider) = s.context_menu_provider.clone() else {
                            return;
                        };
                        let public_items = provider.menu_items(&request);
                        let items = GridState::convert_context_menu_items(public_items);

                        if items.is_empty() {
                            s.context_menu = None;
                        } else {
                            s.context_menu =
                                Some(menu::ContextMenu::custom(col, pos, items, request));
                        }
                        s.filter_panel = None;
                        cx.notify();
                    });
                },
            )
            .on_mouse_move(move |event: &MouseMoveEvent, _window, cx| {
                state_move.update(cx, |s, cx| {
                    let rel = state_inner::to_grid_relative(event.position, s.bounds.origin);
                    s.handle_mouse_move(rel, event.pressed_button);
                    cx.notify();
                });
            })
            .on_mouse_up(
                MouseButton::Left,
                move |_event: &MouseUpEvent, _window, cx| {
                    state_up.update(cx, |s, cx| {
                        s.handle_mouse_up();
                        cx.notify();
                    });
                },
            )
            .on_scroll_wheel(move |event: &ScrollWheelEvent, _window, cx| {
                state_scroll.update(cx, |s, cx| {
                    let line_h = px(s.row_height);
                    let delta = event.delta.pixel_delta(line_h);
                    let scroll = s.scroll_handle.offset();
                    let (mx, my) = s.max_scroll();
                    let new_y = (f32::from(scroll.y) - f32::from(delta.y)).clamp(0.0, my);
                    let new_x = (f32::from(scroll.x) - f32::from(delta.x)).clamp(0.0, mx);
                    s.scroll_handle.set_offset(point(px(new_x), px(new_y)));
                    if s.drag_start.is_some() {
                        s.handle_scroll_drag();
                    }
                    cx.notify();
                });
            })
            .on_key_down(move |event: &KeyDownEvent, _window, cx| {
                let ks = &event.keystroke;
                if ks.modifiers.platform && ks.key == "q" {
                    cx.quit();
                    return;
                }
                state_key.update(cx, |s, cx| {
                    let kb = &s.config.key_bindings;
                    if kb.select_all.matches(ks) {
                        s.select_all();
                    } else if kb.copy.matches(ks) {
                        s.copy_selection(false, cx);
                    } else if kb.copy_with_headers.matches(ks) {
                        s.copy_selection(true, cx);
                    } else if kb.page_up.matches(ks) {
                        s.page_up();
                    } else if kb.page_down.matches(ks) {
                        s.page_down();
                    } else {
                        s.handle_key(ks);
                    }
                    cx.notify();
                });
            })
    }
}

/// Build the context-menu overlay as a `deferred` + `anchored` element so it
/// paints — and receives mouse events — on top of everything, including
/// regions outside the grid widget's own layout bounds. Returns `None` when no
/// menu is open.
///
/// Positioning reuses [`menu::ContextMenu::resolved_position`] (window-viewport
/// aware: flips up when there's no room below, shifts left at the right edge),
/// then converts to absolute window coordinates for `anchored().position(..)`.
/// Each selectable row carries its own `on_mouse_down` (dispatch) and
/// `on_mouse_move` (hover highlight) handlers; a full-screen backdrop behind
/// the menu dismisses it on any outside click.
fn render_context_menu_overlay(
    state: &Entity<GridState>,
    cx: &mut Context<SqllyDataTable>,
) -> Option<impl IntoElement> {
    let s = state.read(cx);
    let menu = s.context_menu.clone()?;
    let theme = s.theme.clone();
    let animations = s.config.animations;
    let cw = s.char_width;
    let grid_ox = f32::from(s.bounds.origin.x);
    let grid_oy = f32::from(s.bounds.origin.y);
    let viewport = s.window_viewport;
    let vw = f32::from(viewport.width);
    let vh = f32::from(viewport.height);

    let resolved = menu.resolved_position(grid_ox, grid_oy, vw, vh, cw);
    let abs_x = grid_ox + f32::from(resolved.x);
    let abs_y = grid_oy + f32::from(resolved.y);
    let menu_w = menu.width_for(cw);

    // Build one row per item. `selectable_idx` counts only Action/Custom items
    // so it matches the `hovered` index convention used elsewhere.
    let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(menu.items.len());
    let mut selectable_idx = 0usize;
    for item in &menu.items {
        match item {
            MenuItem::Separator => {
                rows.push(
                    div()
                        .h(px(menu::MENU_ITEM_HEIGHT))
                        .flex()
                        .items_center()
                        .child(div().mx(px(4.0)).h(px(1.0)).w_full().bg(theme.grid_line))
                        .into_any_element(),
                );
            }
            MenuItem::Action(_) | MenuItem::Custom { .. } => {
                let this_idx = selectable_idx;
                selectable_idx += 1;
                let label = item.label().unwrap_or("").to_owned();
                let hovered = menu.hovered == Some(this_idx);

                // Dispatch: set the pending action and close the menu. The
                // pending fields are drained at the top of `render` (they need
                // App access for clipboard).
                let action = match item {
                    MenuItem::Action(a) => MenuDispatch::Builtin(*a, menu.col),
                    MenuItem::Custom { id, .. } => {
                        MenuDispatch::Custom(id.clone(), menu.request.clone())
                    }
                    MenuItem::Separator => unreachable!(),
                };

                let state_click = state.clone();
                let state_hover = state.clone();
                let mut row = div()
                    .h(px(menu::MENU_ITEM_HEIGHT))
                    .px(px(menu::MENU_PADDING_X))
                    .flex()
                    .items_center()
                    .text_color(theme.menu_fg)
                    .text_size(px(menu::MENU_FONT_SIZE))
                    .child(label)
                    .on_mouse_move(move |_e: &MouseMoveEvent, _window, cx| {
                        state_hover.update(cx, |s, cx| {
                            if let Some(m) = s.context_menu.as_mut() {
                                if m.hovered != Some(this_idx) {
                                    m.hovered = Some(this_idx);
                                    cx.notify();
                                }
                            }
                        });
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        move |_e: &MouseDownEvent, _window, cx| {
                            state_click.update(cx, |s, cx| {
                                match &action {
                                    MenuDispatch::Builtin(a, col) => {
                                        s.pending_action = Some((*a, *col));
                                    }
                                    MenuDispatch::Custom(id, request) => {
                                        if let Some(request) = request {
                                            s.pending_custom_context_menu_action =
                                                Some(PendingCustomContextMenuAction {
                                                    id: id.clone(),
                                                    request: request.clone(),
                                                });
                                        }
                                    }
                                }
                                s.context_menu = None;
                                cx.notify();
                            });
                        },
                    );
                if hovered {
                    row = row.bg(theme.menu_hover_bg);
                }
                rows.push(row.into_any_element());
            }
        }
    }

    let menu_body = div()
        .flex()
        .flex_col()
        .w(px(menu_w))
        .py(px(menu::MENU_INNER_PAD))
        .bg(theme.menu_bg)
        .border_1()
        .border_color(theme.grid_line)
        .children(rows);

    // Full-window transparent backdrop: catches clicks outside the menu to
    // dismiss it. Placed behind the menu within the same anchored overlay.
    let state_backdrop = state.clone();
    let overlay = deferred(
        anchored().position(point(px(abs_x), px(abs_y))).child(
            div()
                .occlude()
                .child(crate::grid::motion::pop_in(
                    menu_body,
                    "grid-context-menu",
                    animations,
                ))
                .on_mouse_down_out(move |_e: &MouseDownEvent, _window, cx| {
                    state_backdrop.update(cx, |s, cx| {
                        if s.context_menu.is_some() {
                            s.context_menu = None;
                            s.filter_panel = None;
                            cx.notify();
                        }
                    });
                }),
        ),
    )
    .with_priority(CONTEXT_MENU_PRIORITY);

    Some(overlay)
}

/// Fixed width of the filter popover, in pixels.
const FILTER_PANEL_WIDTH: f32 = 300.0;
/// Max number of distinct value rows rendered at once (search narrows the set).
const FILTER_PANEL_MAX_ROWS: usize = 200;

/// Build the Numbers-style per-column filter popover as a `deferred` +
/// `anchored` overlay, using the exact mechanism as
/// [`render_context_menu_overlay`] so it paints and receives events outside the
/// grid's own layout bounds. Returns `None` when no panel is open.
#[allow(clippy::too_many_lines)]
fn render_filter_panel_overlay(
    state: &Entity<GridState>,
    cx: &mut Context<SqllyDataTable>,
) -> Option<impl IntoElement> {
    let s = state.read(cx);
    let panel = s.filter_panel.clone()?;
    let theme = s.theme.clone();
    let animations = s.config.animations;
    let col = panel.col;
    let current_sort = s.sort;
    let filter_active = s.filters.get(col).is_some_and(|f| f.is_active());
    let grid_ox = f32::from(s.bounds.origin.x);
    let grid_oy = f32::from(s.bounds.origin.y);

    // Anchor (grid-relative) -> absolute window coords. The default
    // `SwitchAnchor` fit mode on `anchored()` handles viewport-edge flipping
    // automatically using the actual rendered height, so we don't need a
    // manual estimate or flip calculation here.
    let abs_x = grid_ox + f32::from(panel.anchor.x);
    let abs_y = grid_oy + f32::from(panel.anchor.y);

    // Palette (all `Hsla` are `Copy`, so they move freely into closures).
    let c_bg = theme.menu_bg;
    let c_line = theme.grid_line;
    let c_fg = theme.menu_fg;
    let c_accent = theme.sort_indicator;
    let c_hover = theme.menu_hover_bg;
    let c_muted = theme.muted_text;

    let checkbox = {
        let theme = theme.clone();
        move |checked: bool| crate::grid::checkbox(checked, &theme)
    };

    // --- Sort row -----------------------------------------------------------
    let (asc_active, desc_active) = match current_sort {
        Some((c, SortDirection::Ascending)) if c == col => (true, false),
        Some((c, SortDirection::Descending)) if c == col => (false, true),
        _ => (false, false),
    };
    let st_asc = state.clone();
    let st_desc = state.clone();
    let sort_row = div()
        .flex()
        .gap(px(6.0))
        .child(
            div()
                .flex_1()
                .h(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(c_line)
                .bg(if asc_active { c_accent } else { c_hover })
                .cursor_pointer()
                .child("Ascending")
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    st_asc.update(cx, |s, cx| {
                        s.set_panel_sort(SortDirection::Ascending);
                        cx.notify();
                    });
                }),
        )
        .child(
            div()
                .flex_1()
                .h(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(c_line)
                .bg(if desc_active { c_accent } else { c_hover })
                .cursor_pointer()
                .child("Descending")
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    st_desc.update(cx, |s, cx| {
                        s.set_panel_sort(SortDirection::Descending);
                        cx.notify();
                    });
                }),
        );

    // --- Operator dropdown --------------------------------------------------
    let st_op_toggle = state.clone();
    let op_button = div()
        .h(px(26.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .border_1()
        .border_color(c_line)
        .bg(c_bg)
        .cursor_pointer()
        .child(panel.current_op_label())
        .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
            st_op_toggle.update(cx, |s, cx| {
                s.toggle_filter_op_menu();
                cx.notify();
            });
        });

    let op_menu = panel.op_menu_open.then(|| {
        let mut items: Vec<gpui::AnyElement> = Vec::new();
        for (i, label) in panel.op_labels().iter().enumerate() {
            let selected = i == panel.op_index;
            let st_pick = state.clone();
            items.push(
                div()
                    .h(px(24.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .bg(if selected { c_accent } else { c_bg })
                    .cursor_pointer()
                    .child(*label)
                    .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                        st_pick.update(cx, |s, cx| {
                            s.set_filter_operator(i);
                            cx.notify();
                        });
                    })
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(c_line)
            .bg(c_bg)
            .children(items)
    });

    // --- Operand field(s) ---------------------------------------------------
    let operand_field = |value: &str, focused: bool, placeholder: &str, input: FilterInput| {
        let st_focus = state.clone();
        let (text, is_placeholder) = if value.is_empty() {
            (placeholder.to_owned(), true)
        } else {
            (value.to_owned(), false)
        };
        div()
            .h(px(26.0))
            .px(px(6.0))
            .flex()
            .items_center()
            .gap(px(2.0))
            .border_1()
            .border_color(if focused { c_accent } else { c_line })
            .bg(c_bg)
            .cursor_pointer()
            .child(
                div()
                    .text_color(if is_placeholder { c_muted } else { c_fg })
                    .child(text),
            )
            .children(focused.then(|| div().w(px(1.0)).h(px(14.0)).bg(c_accent)))
            .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                st_focus.update(cx, |s, cx| {
                    s.set_filter_focus(input);
                    cx.notify();
                });
            })
    };

    let operand_placeholder = if panel.kind == crate::data::ColumnKind::Date {
        "YYYY-MM-DD"
    } else if crate::filter::uses_number_ops(panel.kind) {
        "value"
    } else if panel.op_index == 7 {
        // Text "matches" operator.
        "regex"
    } else {
        "value"
    };
    let operands = panel.needs_operand().then(|| {
        let mut row = div().flex().flex_col().gap(px(4.0)).child(operand_field(
            &panel.operand_a.value,
            panel.focus == FilterInput::OperandA,
            operand_placeholder,
            FilterInput::OperandA,
        ));
        if panel.needs_second_operand() {
            row = row
                .child(div().text_color(c_muted).text_size(px(11.0)).child("and"))
                .child(operand_field(
                    &panel.operand_b.value,
                    panel.focus == FilterInput::OperandB,
                    operand_placeholder,
                    FilterInput::OperandB,
                ));
        }
        row
    });

    // --- Search box ---------------------------------------------------------
    let st_search = state.clone();
    let search_focused = panel.focus == FilterInput::Search;
    let (search_text, search_is_ph) = if panel.search.value.is_empty() {
        ("Search".to_owned(), true)
    } else {
        (panel.search.value.clone(), false)
    };
    let search_box = div()
        .h(px(26.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .gap(px(2.0))
        .border_1()
        .border_color(if search_focused { c_accent } else { c_line })
        .bg(c_bg)
        .cursor_pointer()
        .child(
            div()
                .text_color(if search_is_ph { c_muted } else { c_fg })
                .child(search_text),
        )
        .children(search_focused.then(|| div().w(px(1.0)).h(px(14.0)).bg(c_accent)))
        .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
            st_search.update(cx, |s, cx| {
                s.set_filter_focus(FilterInput::Search);
                cx.notify();
            });
        });

    // --- (Select All) + value checklist ------------------------------------
    let st_all = state.clone();
    let select_all_row = div()
        .id("filter-select-all")
        .h(px(24.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(4.0))
        .rounded(px(4.0))
        .cursor_pointer()
        .hover(move |style| style.bg(c_hover))
        .child(checkbox(panel.all_checked()))
        .child("(Select All)")
        .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
            st_all.update(cx, |s, cx| {
                s.toggle_filter_select_all();
                cx.notify();
            });
        });

    let visible = panel.visible_indices();
    let mut value_rows: Vec<gpui::AnyElement> = Vec::new();
    for &idx in visible.iter().take(FILTER_PANEL_MAX_ROWS) {
        let row = &panel.distinct[idx];
        let st_val = state.clone();
        value_rows.push(
            div()
                .id(("filter-value", idx))
                .h(px(22.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(4.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .hover(move |style| style.bg(c_hover))
                .child(checkbox(row.checked))
                .child(div().text_color(c_fg).child(row.label.clone()))
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    st_val.update(cx, |s, cx| {
                        s.toggle_filter_value(idx);
                        cx.notify();
                    });
                })
                .into_any_element(),
        );
    }
    let truncated = visible.len() > FILTER_PANEL_MAX_ROWS;
    let value_list = div()
        .id("filter-value-list")
        .flex()
        .flex_col()
        .max_h(px(180.0))
        .overflow_y_scroll()
        .children(value_rows)
        .children(truncated.then(|| {
            div()
                .text_color(c_muted)
                .text_size(px(11.0))
                .child("Refine search to see more…")
        }));

    // --- Clear (left, disabled when no active filter) + Close (right) -----
    let st_clear = state.clone();
    let st_close = state.clone();
    let clear_bg = if filter_active { c_hover } else { c_bg };
    let clear_fg = if filter_active { c_fg } else { c_muted };
    let clear_border = if filter_active { c_line } else { c_muted };
    let buttons_row = div()
        .flex()
        .gap(px(6.0))
        .child(
            div()
                .flex_1()
                .h(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(clear_border)
                .bg(clear_bg)
                .text_color(clear_fg)
                .cursor_pointer()
                .child("Clear Filter")
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    if !filter_active {
                        return;
                    }
                    st_clear.update(cx, |s, cx| {
                        s.clear_filter_panel();
                        cx.notify();
                    });
                }),
        )
        .child(
            div()
                .flex_1()
                .h(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(c_line)
                .bg(c_hover)
                .cursor_pointer()
                .child("Close")
                .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, _w, cx| {
                    st_close.update(cx, |s, cx| {
                        s.filter_panel = None;
                        cx.notify();
                    });
                }),
        );

    let panel_body = div()
        .flex()
        .flex_col()
        .w(px(FILTER_PANEL_WIDTH))
        .p(px(10.0))
        .gap(px(8.0))
        .bg(c_bg)
        .border_1()
        .border_color(c_line)
        .text_color(c_fg)
        .text_size(px(13.0))
        .child(div().text_color(c_muted).text_size(px(11.0)).child("Sort"))
        .child(sort_row)
        .child(
            div()
                .text_color(c_muted)
                .text_size(px(11.0))
                .child("Filter"),
        )
        .child(op_button)
        .children(op_menu)
        .children(operands)
        .child(search_box)
        .child(select_all_row)
        .child(value_list)
        .child(buttons_row);

    let st_backdrop = state.clone();
    let overlay = deferred(
        anchored()
            .anchor(Anchor::BottomLeft)
            .position(point(px(abs_x), px(abs_y)))
            .child(
                div()
                    .occlude()
                    .child(crate::grid::motion::pop_in(
                        panel_body,
                        "grid-filter-panel",
                        animations,
                    ))
                    .on_mouse_down_out(move |_e: &MouseDownEvent, _window, cx| {
                        st_backdrop.update(cx, |s, cx| {
                            if s.filter_panel.is_some() {
                                s.filter_panel = None;
                                cx.notify();
                            }
                        });
                    }),
            ),
    )
    .with_priority(CONTEXT_MENU_PRIORITY);

    Some(overlay)
}

/// Renders the loading overlay while a background task runs. Returns `None`
/// when the grid is not busy. Painted as an absolute, input-occluding scrim
/// over the whole grid area with a centered card showing the task label and a
/// progress bar (determinate when progress is known, otherwise an animated
/// indeterminate bar).
fn render_busy_overlay(
    state: &Entity<GridState>,
    cx: &mut Context<SqllyDataTable>,
) -> Option<impl IntoElement> {
    let s = state.read(cx);
    let busy = s.busy.clone()?;
    let theme = s.theme.clone();
    let animations = s.config.animations;
    let track = theme.grid_line;
    let accent = theme.sort_indicator;

    let bar: gpui::AnyElement = if let Some(p) = busy.progress {
        let p = p.clamp(0.0, 1.0);
        div()
            .h_full()
            .w(relative(p))
            .rounded(px(3.0))
            .bg(accent)
            .into_any_element()
    } else {
        div()
            .h_full()
            .w(relative(0.3))
            .rounded(px(3.0))
            .bg(accent)
            .with_animation(
                "busy-indeterminate",
                Animation::new(std::time::Duration::from_millis(900))
                    .repeat()
                    .with_easing(pulsating_between(0.15, 0.85)),
                |el, delta| el.w(relative(delta)),
            )
            .into_any_element()
    };

    let card = div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .p(px(16.0))
        .min_w(px(220.0))
        .rounded(px(8.0))
        .bg(theme.menu_bg)
        .border_1()
        .border_color(theme.grid_line)
        .child(
            div()
                .text_color(theme.menu_fg)
                .text_size(px(14.0))
                .child(busy.label.clone()),
        )
        .child(
            div()
                .w_full()
                .h(px(6.0))
                .rounded(px(3.0))
                .bg(track)
                .child(bar),
        );

    let overlay = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.overlay_scrim)
        .child(card);

    Some(crate::grid::motion::fade_in(
        overlay,
        "grid-busy-overlay",
        crate::grid::motion::SCRIM_ENTER_MS,
        animations,
    ))
}

/// What a menu row dispatches when clicked. Captured per-row so the click
/// handler owns its data without borrowing the menu snapshot.
enum MenuDispatch {
    Builtin(menu::MenuAction, usize),
    Custom(
        String,
        Option<crate::grid::context_menu::ContextMenuRequest>,
    ),
}
