## Understanding

The goal is to add a **pivot table** (cross-tabulation) capability to the `sqlly-datatable` GPUI widget library. A pivot table reorganizes flat row data into a two-dimensional summary where:

- **Row fields** — columns whose distinct values become left-axis row headers (grouped hierarchically when multiple).
- **Column fields** — columns whose distinct values become top-axis column headers.
- **Value field** — the single measure aggregated at each intersection.
- **Aggregation function** — how values are combined: count, sum, avg, min, max.

The user wants:
1. Programmatic preconfiguration of the pivot layout.
2. Graphical (drag-and-drop) configuration — like Excel's PivotTable Field List, delivered as a sidebar.
3. High performance with large datasets (virtualized rendering, incremental recompute).
4. Visually pleasing (consistent with the existing `GridTheme`; alternating row/group shading).
5. Expand/collapse for row and column groups (hierarchical drill-down).
6. Per-value custom cell formatting (reuse the existing `NumberFormat`, `DateFormat`, etc.).
7. Ability to read back the current pivot configuration (row/column/value field assignments, aggregation function).
8. Aggregation functions: min, max, count, sum, avg.
9. Drag-and-drop to assign fields to row/column/value zones.
10. Sort/filter on the pivot result itself.

**Architecture decision (from user): Tabbed views + sidebar.** The original flat grid and the pivot result are tabs; a persistent sidebar shows the drag-and-drop field list. This keeps the existing grid completely intact and adds a new `PivotGrid` widget alongside it.

**Suggested killer features:**
- **Grand totals and subtotals** — summary rows/columns at each group level.
- **Top-N / Bottom-N filtering** — "show only the top 10 accounts by sum".
- **Calculated field** — user-defined formula over the value field (e.g. "% of total", "YoY change").
- **Export pivot to CSV/JSON** — with the hierarchical structure preserved.
- **Cell-level conditional formatting** — color scales, data bars, icon sets.
- **Pivot chart** — bar/line chart generated from the pivot result (stretch goal for a later plan).
- **Undo/redo for configuration changes** — since drag-and-drop is inherently trial-and-error.

## Existing Patterns To Preserve

### Pure computation / rendering split
The codebase strictly separates pure data operations (no GPUI dependency) from GPUI widget rendering. `data.rs`, `filter.rs`, and `format.rs` are GPUI-free. `grid/state.rs`, `grid/widget.rs`, and `grid/paint.rs` are the GPUI layer. The pivot engine must follow this: `pivot/engine.rs` computes the pivot result; `pivot/widget.rs` and `pivot/paint.rs` render it.

### Arc-wrapped snapshots for paint
`PaintData::from_state` clones only lightweight references (Arcs, selections, scroll offsets). The pivot paint path must do the same: the pivot result is Arc-wrapped so paint clones are O(1).

### Config → ResolvedColumnFormat pipeline
`GridConfig` / `ColumnOverride` / `ResolvedColumnFormat` cascade is reused for the pivot result cells. A `PivotConfig` struct holds the field assignments and aggregation choice; formatting is resolved per column from the existing config system.

### Deferred + anchored overlays for popovers
The filter panel and context menu use GPUI's `deferred` + `anchored` overlay pattern for positioning outside the grid bounds. The pivot sidebar and any pivot-specific context menus follow the same pattern.

### Entity<GridState> ownership model
`SqllyDataTable` owns `Entity<GridState>`. The pivot should introduce a parallel `Entity<PivotState>` (or a `PivotView` wrapper) so the two views can coexist in the same application without cross-contaminating state.

### Context menu provider extensibility
The provider pattern in `context_menu.rs` is reused for the pivot sidebar's field list right-click actions (e.g. "Move to rows", "Remove field").

### Theme consistency
All new UI surfaces use `GridTheme` fields. New theme fields are added only for pivot-specific concepts (group expand/collapse icons, drop zone highlights).

### Clippy / rustfmt discipline
`cargo fmt --all` and `cargo clippy -p sqlly-datatable --all-targets` must be clean before any commit (per AGENTS.md). All new code follows the workspace-level lint rules (no unsafe, warn on unwrap/expect/todo/dbg).

## Files, Pages, And Modules

### New modules (all inside `crates/sqlly-datatable/src/`)

| File | Purpose |
|------|---------|
| `pivot/mod.rs` | Module root; re-exports public types. |
| `pivot/config.rs` | `PivotConfig` struct — field assignments, aggregation, formatting. |
| `pivot/engine.rs` | Pure computation: given `GridData` + `PivotConfig`, produce `PivotResult`. No GPUI. |
| `pivot/state.rs` | `PivotState` — runtime pivot state (expand/collapse, sort, filter on pivot result). |
| `pivot/widget.rs` | `PivotGrid` GPUI widget + `PivotGridBuilder`. Owns `Entity<PivotState>`. |
| `pivot/paint.rs` | Canvas paint functions for the pivot grid's specialized row/column header hierarchy. |
| `pivot/sidebar.rs` | Drag-and-drop pivot configuration sidebar widget. Renders field zones (rows, columns, values, filters). |
| `pivot/drag_drop.rs` | Drag-and-drop hit testing, drop-zone resolution, ghost rendering during drag. |
| `pivot/aggregation.rs` | Aggregation function enum + trait for applying min/max/count/sum/avg to `CellValue` sets. |

### Modified existing files

| File | Change |
|------|--------|
| `lib.rs` | Add `pub mod pivot;` and re-export `PivotConfig`, `PivotState`, `PivotGrid`, `PivotGridBuilder`, `AggregationFn`. |
| `config.rs` | Add optional `pivot: PivotConfig` field to `GridConfig` for preconfiguration. |
| `data.rs` | No changes needed — `GridData` is the input to pivoting. |
| `grid/theme.rs` | Add pivot-specific theme fields: `pivot_group_bg`, `pivot_expand_icon`, `pivot_collapse_icon`, `pivot_drop_zone_bg`, `pivot_drop_zone_active`, `pivot_drop_zone_border`, `pivot_subtotal_bg`, `pivot_grand_total_bg`, `pivot_total_fg`, `pivot_value_bar_bg`. |
| `grid/context_menu.rs` | No structural changes; pivot-specific actions use the existing `ContextMenuProvider` pattern. |

### Sample app changes

The `sqlly-datatable-sample` crate gains a pivot demo mode. A tab selector switches between "Flat Grid" and "Pivot View" tabs. The sidebar is shown when the Pivot tab is active.

## Step-By-Step Plan

### Phase 1: Pure pivot engine (no UI)

1. **Create `pivot/aggregation.rs`** — Define `AggregationFn` enum (`Count`, `Sum`, `Avg`, `Min`, `Max`) and a function `aggregate(values: &[CellValue]) -> Option<CellValue>` that applies the chosen function to a slice of cells. Handle `CellValue::None` (skip for `Sum/Avg/Min/Max`, count as-is for `Count`). Handle mixed numeric types by promoting `Integer` to `f64`. Return `None` for empty input sets.

2. **Create `pivot/config.rs`** — Define `PivotConfig`:
   ```rust
   pub struct PivotConfig {
       pub row_fields: Vec<usize>,       // column indices for row headers
       pub column_fields: Vec<usize>,    // column indices for column headers
       pub value_field: usize,           // column index to aggregate
       pub aggregation: AggregationFn,
       pub show_row_totals: bool,
       pub show_column_totals: bool,
       pub show_grand_total: bool,       // lower-right corner
       pub column_format_overrides: Vec<ColumnOverride>, // indexed by pivot column index
   }
   ```
   Add `PivotConfig::get()` and `PivotConfig::set_field_assignments()` public API for reading/writing the configuration programmatically.

3. **Create `pivot/engine.rs`** — Pure function `compute_pivot(data: &GridData, config: &PivotConfig, fmt: &[ResolvedColumnFormat]) -> PivotResult`. Algorithm:
   - Group source rows by the Cartesian product of their row-field values → each unique tuple of row-field values is a "row group".
   - Group row groups by the Cartesian product of their column-field values → each unique tuple of column-field values is a "column group".
   - The intersection cell aggregates all source rows matching both the row-group and column-group.
   - Return a hierarchical tree structure (rows/columns as trees, not flat grids) to support expand/collapse natively.

   **Data structure for `PivotResult`:**
   ```rust
   pub struct PivotResult {
       pub row_tree: Vec<PivotRowGroup>,    // hierarchical row headers
       pub col_tree: Vec<PivotColGroup>,    // hierarchical column headers
       pub cells: Vec<Vec<CellValue>>,      // [row_leaf_idx][col_leaf_idx]
       pub row_field_count: usize,
       pub col_field_count: usize,
       pub row_field_names: Vec<String>,
       pub col_field_names: Vec<String>,
       pub value_field_name: String,
       pub row_totals: Vec<CellValue>,      // per-row-leaf totals
       pub col_totals: Vec<CellValue>,      // per-col-leaf totals
       pub grand_total: CellValue,
       pub row_leaf_count: usize,
       pub col_leaf_count: usize,
   }

   pub struct PivotRowGroup {
       pub label: String,
       pub value: CellValue,        // the original grouping value
       pub depth: usize,            // 0 = outermost row field
       pub expanded: bool,
       pub children: Vec<PivotRowGroup>,
       pub leaf_indices: Vec<usize>, // indices into cells rows (for leaf groups)
       pub subtotal: Option<CellValue>,
   }
   // PivotColGroup is structurally identical
   ```

   **Performance note:** Use `HashMap<(composite_key), Vec<source_row_idx>>` for grouping. For single field pivots this is O(n). For two row fields, build the outer group first, then sub-group. This avoids the Cartesian explosion of building all combinations upfront.

4. **Unit tests for the engine** — Test with small known datasets: single row/col field + sum, two row fields + count, empty groups, all-`None` value fields, date/boolean grouping fields.

### Phase 2: Pivot state and rendering

5. **Create `pivot/state.rs`** — `PivotState` struct:
   ```rust
   pub struct PivotState {
       pub config: PivotConfig,
       pub result: Arc<PivotResult>,      // Arc for cheap paint snapshots
       pub source_data: Arc<GridData>,    // original data for recalc
       pub resolved_formats: Vec<ResolvedColumnFormat>,
       pub scroll_handle: ScrollHandle,
       pub focus_handle: FocusHandle,
       pub bounds: Bounds<Pixels>,
       pub theme: GridTheme,
       pub row_height: f32,
       pub header_height: f32,
       pub col_header_height: f32,        // taller — multi-level column headers
       pub row_header_width: f32,         // wider — indented for depth
       pub font_size: f32,
       pub char_width: f32,
       pub selection: PivotSelection,
       pub hover_hit: Option<PivotHitResult>,
       pub sort: Option<(PivotSortTarget, SortDirection)>,
       pub filters: Vec<ColumnFilter>,
       pub self_weak: Option<gpui::WeakEntity<PivotState>>,
       // ... scrollbar geometry, drag state, etc.
   }
   ```
   Implement `toggle_group(row_or_col, index)` for expand/collapse. Implement `recompute()` that re-runs the engine when config changes. Implement `sort_pivot()` that sorts leaf rows/columns by their subtotal values.

6. **Create `pivot/paint.rs`** — `PivotPaintData` snapshot (like `PaintData`) and `paint_pivot_grid()`. The paint function handles:
   - **Row header area with indentation** — each depth level adds a fixed indent (20px). Group rows show expand/collapse chevrons. Leaf rows show no indent beyond their ancestors' depth.
   - **Column header area** — stacked vertically for multiple column fields. Each field gets its own header row with group labels and expand/collapse chevrons.
   - **Data cells** — standard cell painting reused from `paint.rs` utility functions.
   - **Subtotal rows/columns** — distinct background color, bold text.
   - **Grand total cell** — bottom-right corner.
   - **Alternating row shading** — but reset at each outer group boundary.
   - **Scroll synchronization** — row headers scroll vertically with data; column headers scroll horizontally with data.

   Extract shared paint primitives from the existing `paint.rs` into a `paint_shared.rs` or `grid/paint_helpers.rs` file: cell text painting, selection highlight, grid lines, header backgrounds. Both `grid/paint.rs` and `pivot/paint.rs` import from this shared module. This prevents code duplication.

7. **Create `pivot/widget.rs`** — `PivotGrid` widget and `PivotGridBuilder`. Follows the exact same pattern as `SqllyDataTable`:
   - Owns `Entity<PivotState>`
   - Builder accepts `GridData`, `GridConfig`, optional `PivotConfig`
   - Canvas-based rendering using `paint_pivot_grid`
   - Mouse/keyboard events wired to `PivotState` methods
   - Edge scroll during drag, scrollbar drag, column resize (for value columns)
   - Right-click context menu (using existing provider pattern)

### Phase 3: Drag-and-drop sidebar

8. **Create `pivot/sidebar.rs`** — The pivot configuration sidebar widget:
   - Renders the list of available source columns as a scrollable list.
   - Four drop zones: "Rows", "Columns", "Values", "Filters".
   - Each zone shows the currently assigned fields as draggable chips.
   - Value zone chip shows the aggregation function picker (dropdown).
   - Filter zone chip shows a mini filter indicator (like the grid's filter marker).
   - "Values" zone accepts exactly one field; dropping a new one replaces it.
   - Empty zones show placeholder text ("Drag fields here").

9. **Create `pivot/drag_drop.rs`** — Drag-and-drop mechanics:
   - `on_mouse_down` on a field chip initiates a drag — capture the chip identity.
   - `on_mouse_move` during drag paints a ghost chip at the cursor position.
   - Drop-zone hit testing via `hit_test_drop_zone(pos)` → returns which zone the cursor is over.
   - Visual feedback: active drop zone gets a highlighted border/background.
   - `on_mouse_up` finalizes the drop — moves the field assignment in `PivotConfig`, triggers `recompute()`.
   - GPUI's `canvas` approach: the ghost chip during drag is painted in the same canvas pass as the sidebar, overlaid on top. Or use a `deferred` overlay for true cross-widget ghost rendering.

   **Gotcha:** GPUI's drag-and-drop across separate widgets is complex because each widget only sees events within its bounds. The simplest reliable approach: paint the sidebar and the drag ghost as a single `canvas` element. The ghost follows the mouse position tracked via `on_mouse_move`. This works within the sidebar widget's bounds. For dragging from the sidebar to the pivot drop zones, the sidebar is a single tall panel with all zones stacked vertically — the mouse never leaves the sidebar during drag.

### Phase 4: Sort/filter on pivot result

10. **Pivot result sorting** — Add to `PivotState`:
    - Sort by row label (alphabetical/numerical on the outermost row field).
    - Sort by a specific value column's data (ascending/descending).
    - Sort by subtotal.
    - Implemented by reordering the `row_tree` / `col_tree` children and regenerating `cells`.

11. **Pivot result filtering** — Add to `PivotState`:
    - Value filter: keep only rows where a column's value satisfies a predicate (e.g. "sum > 1000").
    - Label filter: keep only rows whose label matches a text predicate.
    - Top-N filter: keep top/bottom N rows by a value column.
    - Reuses the existing `filter.rs` `ColumnFilter` / `FilterPredicate` types with a pivot-specific wrapper.

### Phase 5: Tabbed view integration

12. **Sample app integration** — In `sqlly-datatable-sample`, add:
    - A tab bar at the top: "Flat Grid" / "Pivot View".
    - When "Pivot View" is active, show the sidebar on the left and the pivot grid on the right.
    - The sidebar and pivot grid communicate via shared `Entity<PivotState>` (the sidebar mutates the config, the pivot grid reads it and repaints).
    - Demonstrates preconfiguration: sample app seeds a default pivot config (e.g. "Currency" as row, "Year" as column, "Amount" as value, sum).

### Phase 6: Cell formatting and value bars

13. **Cell formatting reuse** — The pivot result cells are `CellValue` instances formatted via the existing `format_cell()`. `PivotConfig::column_format_overrides` provides per-pivot-column overrides. The value column uses the `PivotConfig::value_field_format` override. Row/column header labels use `StringFormat`. Subtotals and grand totals also use `NumberFormat` with bold styling.

14. **Conditional formatting (killer feature)** — Optional `ConditionalFormat` rules in `PivotConfig`:
    - Color scale: map value range to a color gradient.
    - Data bar: fill a portion of the cell background proportional to value.
    - Icon set: show ▲/▼/● based on thresholds.
    - Applied during `paint_pivot_grid()` data cell rendering.

### Phase 7: Export and serialization

15. **PivotConfig serialization** — Implement `serde::Serialize` / `serde::Deserialize` on `PivotConfig` (behind an optional `serde` feature flag, since the crate currently has no serde dependency). This enables saving/loading pivot layouts.

16. **Pivot export** — Add export methods to `PivotState`:
    - `to_csv()`: flattened pivot with hierarchical row labels.
    - `to_json()`: nested structure matching the tree.
    - Reuse existing copy-to-clipboard patterns.

## Pseudocode

### Core engine: `compute_pivot()`

```rust
fn compute_pivot(data: &GridData, config: &PivotConfig, fmt: &[ResolvedColumnFormat]) -> PivotResult {
    // Step 1: Map each source row to its composite row key
    //   row_key = tuple of formatted values for each row_field column
    //   row_groups: HashMap<row_key, Vec<source_row_idx>>

    // Step 2: Build row tree
    //   For single row field: one level of groups, each is a leaf.
    //   For N row fields: build N-level tree. Level 0 groups by field 0,
    //     each child groups by field 1 within its parent's rows, etc.

    // Step 3: Map each source row to its composite column key
    //   col_key = tuple of formatted values for each col_field column
    //   row_groups[row_key] has rows → sub-group by col_key

    // Step 4: Build column tree (same structure as row tree)

    // Step 5: For each (row_leaf, col_leaf) intersection:
    //   cells[row_leaf][col_leaf] = aggregate(
    //       source rows matching both row_leaf and col_leaf,
    //       value_field column
    //   )

    // Step 6: Compute subtotals and grand total

    PivotResult { row_tree, col_tree, cells, ... }
}
```

### Row tree for two row fields (Region → Product)

```text
row_tree:
  "Europe" (expanded=true)
    ├── child: "Widget"  → leaf_indices: [0]
    ├── child: "Gadget"  → leaf_indices: [1]
    subtotal: sum(Europe rows)
  "Asia" (expanded=true)
    ├── child: "Widget"  → leaf_indices: [2]
    ├── child: "Gadget"  → leaf_indices: [3]
    subtotal: sum(Asia rows)
```

### Pivot paint: row header area

```rust
fn paint_pivot_row_headers(data: &PivotPaintData, window: &mut Window, cx: &mut App, bounds: Bounds<Pixels>) {
    let mut y = header_height;
    let visible_start = (scroll_y / row_height) as usize;
    let mut flat_idx = 0; // counts leaf rows only

    for group in &data.row_tree {
        paint_group_row(group, depth: 0, &mut y, &mut flat_idx, visible_start, data, window, cx, bounds);
    }

    if data.show_grand_total {
        paint_grand_total_row(&mut y, data, window, cx, bounds);
    }
}

fn paint_group_row(group: &PivotRowGroup, depth: usize, y: &mut f32, flat_idx: &mut usize,
                    visible_start: usize, data: &PivotPaintData, ...) {
    let indent = depth as f32 * 20.0;

    // Paint expand/collapse chevron (► or ▼)
    let chevron = if group.expanded { "▼" } else { "►" };
    // Paint group label (formatted value)
    // Paint background (alternating per outer group)

    if group.expanded {
        for child in &group.children {
            paint_group_row(child, depth + 1, y, flat_idx, visible_start, data, ...);
        }
    }

    if group.children.is_empty() || group.expanded {
        for &leaf_idx in &group.leaf_indices {
            // Each leaf gets a row in the data area
            *flat_idx += 1;
        }
    }

    if data.show_subtotals && group.subtotal.is_some() {
        paint_subtotal_row(group, depth, y, data, ...);
    }
}
```

### Sidebar drag-and-drop

```rust
struct PivotSidebar {
    state: Entity<PivotState>,
    dragging: Option<DragState>,
    drag_ghost_pos: Option<Point<Pixels>>,
    drop_zone_hover: Option<DropZone>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DropZone { Rows, Columns, Values, Filters }

struct DragState {
    field_index: usize,      // source column index being dragged
    source_zone: DropZone,   // where it was picked up from (or None for unassigned list)
}

impl PivotSidebar {
    fn hit_test_drop_zone(&self, pos: Point<Pixels>) -> Option<DropZone> {
        // Check if pos falls within any of the four zone rectangles.
        // Zone rectangles are computed from the sidebar layout.
    }

    fn handle_mouse_down(&mut self, pos: Point<Pixels>) {
        // If click is on a field chip, start drag
        if let Some(chip) = self.chip_at(pos) {
            self.dragging = Some(DragState { field_index: chip.field, source_zone: chip.zone });
        }
    }

    fn handle_mouse_move(&mut self, pos: Point<Pixels>) {
        if let Some(_drag) = &self.dragging {
            self.drag_ghost_pos = Some(pos);
            self.drop_zone_hover = self.hit_test_drop_zone(pos);
        }
    }

    fn handle_mouse_up(&mut self, pos: Point<Pixels>) {
        if let Some(drag) = self.dragging.take() {
            self.drag_ghost_pos = None;
            if let Some(target_zone) = self.hit_test_drop_zone(pos) {
                self.state.update(cx, |s, cx| {
                    s.move_field(drag.field_index, drag.source_zone, target_zone);
                    s.recompute();
                    cx.notify();
                });
            }
            self.drop_zone_hover = None;
        }
    }
}
```

## Performance Implications

### Pivot computation cost
- Grouping by N fields: O(rows × fields) to build HashMaps. For 100k rows with 2 row fields + 1 column field: ~300k hash lookups — < 100ms in Rust.
- Recomputation is triggered only when PivotConfig changes (field assignments, aggregation function), not on every scroll/paint. So it's infrequent.
- The pivot result (cells matrix) is O(row_leaf_count × col_leaf_count). With high-cardinality fields this can explode. Mitigations:
  - **Lazy compute** — don't fill all cells until they scroll into view. For the initial implementation, compute the full matrix because typical pivot tables have < 1,000 row leaves and < 100 column leaves. Add lazy compute as a follow-up if needed.
  - **Clamp limit** — warn in docs that pivoting a high-cardinality text column as a row field with > 10,000 distinct values will be slow/expensive.

### Paint virtualization
- The pivot grid MUST virtualize: only paint row groups and cells that intersect the visible viewport.
- The `flat_idx` counter in `paint_pivot_row_headers` is the key: skip painting groups whose leaf range doesn't overlap `[visible_start, visible_start + visible_rows]`.
- This is identical in principle to the existing grid's virtualization — we just need to compute the visible leaf range from the tree, not from a flat array.

### Arc-wrapped result
- `PivotResult` is `Arc`-wrapped on `PivotState` so `PivotPaintData::from_state()` does O(1) shallow clone (just bumps the Arc refcount).
- The `cells` matrix in `PivotResult` is `Vec<Vec<CellValue>>` — same type the existing grid already handles efficiently.

### Recomputing on expand/collapse
- Expand/collapse does NOT recompute the engine — it only toggles the `expanded` flag on a tree node. The paint pass then skips children of collapsed nodes. This is instant.
- Only field assignment changes or aggregation changes trigger a full recompute.

### Drag-and-drop overhead
- During drag, the sidebar repaints on every mouse move. This is cheap because the sidebar layout is static (zones don't resize). Only the ghost chip position and drop-zone highlight change.

## Reusability Opportunities

### Shared paint primitives
Extract from `grid/paint.rs` (currently ~600 lines):
- `paint_cell_text()` — formatted cell text with alignment and color.
- `paint_selection_rect()` — selection highlight rectangle.
- `paint_grid_lines()` — horizontal and vertical grid lines.
- `paint_header_bg()` — column/row header background fill.
- Put these in a new `grid/paint_helpers.rs` (pub(crate)) imported by both `grid/paint.rs` and `pivot/paint.rs`.

### Aggregation trait
`AggregationFn` is a pure enum + function usable outside the pivot context:
- Export pipelines can use it to summarize column data.
- The sample app's context menu provider already has "Sum numeric selection" — this can be rewritten to use `aggregate()`.

### PivotConfig as a standalone concept
`PivotConfig` is serializable/deserializable and carries no GPUI dependency. External tools could generate pivot configs and pass them to the grid. Analytics platforms could store pivot layouts in a database.

## Risks And Missed Details

### High-cardinality field explosion
A text column like "customer_name" with 50,000 distinct values as a row field produces 50,000 row leaves. The paint must virtualize rows (skip non-visible row groups). The `PivotState` should track which row groups are visible and only expand/collapse paint logic walks the visible portion of the tree.

### Multi-level column header rendering
Existing grid `PaintData` has a single `header_height`. The pivot needs variable column header height: `col_field_count * col_header_row_height`. Paint logic must handle column header rows stacked vertically, with merged cells for group labels spanning sub-columns. This is the trickiest rendering challenge.

### Scroll synchronization
The pivot has TWO header areas that scroll:
- Row headers scroll with vertical scroll but are fixed horizontally.
- Column headers scroll with horizontal scroll but are fixed vertically.
- The corner (top-left) is fixed in both axes.

The existing grid already does this correctly (`ContentMask` approach in `paint.rs`). The pivot paint must reuse this pattern but with two-dimensional hierarchical headers.

### Drag-and-drop within GPUI's single-widget event model
GPUI routes mouse events to the widget under the cursor. If the user drags a chip from the sidebar and the cursor moves OUTSIDE the sidebar widget (e.g. over the pivot grid), the sidebar stops receiving `on_mouse_move` events. This means we cannot complete a drag that crosses widget boundaries.

**Mitigation:** The sidebar and pivot grid are rendered inside a single parent container widget that tracks global mouse position. The drag state lives on a shared entity visible to both. Or, simpler: the drop zones are all WITHIN the sidebar itself. The user drags chips between zones inside the sidebar only. This avoids cross-widget drag entirely and matches Excel's PivotTable Field List behavior (drag within the panel, not onto the grid).

### Number formatting for aggregation results
When the value field is Integer but aggregation is `Avg`, the result is always `Decimal`. The `PivotConfig` should auto-detect this: if aggregation is `Count` → `CellValue::Integer`, if `Avg` → `CellValue::Decimal`, if `Sum` → same kind as source. But `Min`/ `Max` preserve the source kind. This prevents mixing Integer and Decimal cells in the result, which would confuse `compare_cells()` during pivot sorting.

### Null handling in groups
What happens when a row field value is `CellValue::None`? Options:
- Group all nulls together with label "(blank)".
- Exclude null-group rows from the pivot entirely (configurable).
- Default: group nulls under "(blank)" label, same as Excel.

### Sort/filter interaction with expand/collapse
When the user sorts the pivot by a value column, then collapses a group — should the collapsed group still contribute to the display order? Yes: the sort pre-computes the row order; collapse only hides children, it doesn't reorder. When the user re-expands, children appear in the sorted order.

### GPUI version lock
The crate depends on `gpui = "0.2"`. Any GPUI API used in the pivot code must exist in 0.2. Check that `canvas()`, `deferred()`, `anchored()`, scroll handles, and drag primitives are stable in 0.2 before using them.

### No serde dependency yet
The crate has no `serde` dependency. Adding it for `PivotConfig` serialization requires a feature flag or a new optional dependency. Since the user asked for "ability to get the configuration", a simple manual `PivotConfig::to_json_value()` + `PivotConfig::from_json_value()` using string formatting avoids the serde dependency entirely. Or we make serde optional behind a `serde` feature.

## Verification Plan

### Unit tests (no GPUI required)

1. `pivot/aggregation.rs` — test all five aggregation functions with known inputs, empty inputs, mixed numeric types, all-`None` inputs.
2. `pivot/config.rs` — test `PivotConfig` defaults, field assignment setters, getters.
3. `pivot/engine.rs` — test with small fixed datasets:
   - Single row field + single column field + sum.
   - Two row fields → correct two-level tree.
   - Row totals, column totals, grand total verified.
   - Empty data → empty pivot result (no panic).
   - All null values → correct grouping under "(blank)".
   - High-cardinality column fields → all combinations present.
4. `pivot/state.rs` — test expand/collapse toggles, sort by subtotal, sort by label.
5. `pivot/drag_drop.rs` — test drop-zone hit testing, field move between zones, value zone single-field enforcement.

### Integration tests (require GPUI Application)

6. `tests/pivot_render.rs` — render a small pivot grid, verify paint output.
7. `tests/pivot_sidebar.rs` — render sidebar, simulate mouse events, verify zone state.

### Manual verification via sample app

8. Launch the sample app with pivot demo: 100k rows, pivot by currency (row) and year (column), sum of amount.
9. Drag fields between zones in the sidebar.
10. Expand/collapse row and column groups.
11. Sort pivot by values.
12. Apply top-10 filter.
13. Copy pivot to clipboard as CSV.

### Clippy / fmt compliance

14. After all phases: `cargo fmt --all --check` exits 0.
15. `cargo clippy -p sqlly-datatable --all-targets` is clean (only the allowed `block v0.1.6` warning).
16. `cargo clippy -p sqlly-datatable-sample --all-targets` is clean.

## Open Questions

1. **Should `PivotConfig` be embedded in `GridConfig` or passed separately?** Embedding in `GridConfig` is simpler for preconfiguration (single builder call). But `GridConfig` is already large. Separate is cleaner for separation of concerns. Plan assumes embedded as an `Option<PivotConfig>` field for preconfiguration, with the sidebar providing runtime configuration.

2. **Conditional formatting scope:** The user said "custom cell formatting" — should this cover conditional formatting (data bars, color scales) or only static per-column formatting? Plan includes both. Static formatting reuses the existing `ColumnOverride` system. Conditional formatting is a new optional feature.

3. **Should pivot results support windowed-row mode?** The source data may already be windowed. The pivot engine reads what's in `GridData.rows`. For windowed data, the user should page in all needed rows before pivoting (the pivot needs the full dataset to compute groups correctly). Pivot results themselves are always fully computed (typically small: hundreds of leaf rows × dozens of leaf columns). Windowed mode for pivot results is a stretch goal.

4. **Serde dependency:** The crate currently has zero serde usage. Should we add `serde` as an optional dependency for `PivotConfig` serialization, or implement manual JSON conversion?

5. **Pivot chart (stretch):** Should the plan include a pivot chart visualization, or defer to a separate plan? Defer to a separate plan.

6. **Should the sidebar be part of the crate's public API or just a sample-app component?** The sidebar is complex GPUI UI code. Making it part of the crate's public API gives consumers a turnkey pivot UX. But it also locks the sidebar design for all consumers. Recommendation: make it a public widget (`PivotSidebar`) with customization hooks (drop zone styling via theme), but document that consumers can build their own sidebar by reading/writing `PivotConfig`.

7. **Memory overhead for the pivot tree:** Each `PivotRowGroup` stores a `Vec<usize>` of leaf indices and source row indices. For 100k rows with 500 distinct row groups: each leaf index vector is small, but the total overhead is ~500 × (avg group size × 8 bytes). Acceptable.
