# Selection Brush Tool

> **Target:** Adobe Photoshop 2026 parity · **Tool:** `select.brush` (`Tool::SelectionBrush`)

The Selection Brush Tool allows painting directly on the canvas to build or subtract from the active selection with the feel and hardness profile of standard painting brushes.

## Overview

In Photoshop 2026, the Selection Brush lets artists markup regions directly on the canvas:
1. **Live Brush Hardness Preview:** While dragging, the stroke displays real-time brush hardness and soft falloff matching the active brush settings (hardness slider, size, spacing).
2. **Closed-Loop Auto-Fill:** Drawing an outline and connecting the start and end points automatically fills the enclosed interior region as part of the selection.
3. **Seamless Outer Falloff:** The interior is filled solidly (alpha 1.0) while the outer perimeter retains the exact mathematical brush hardness falloff curve without seams or blank lines.
4. **Options Bar Controls:** Overlay color swatch and overlay opacity percentage slider, allowing customized visual feedback.
5. **Markup Overlay vs Marching Ants:** While the Selection Brush tool is active, selections are rendered as a translucent colored overlay over marked regions. Switching to another tool displays the traditional marching ants boundary.

## Interaction & Behavior

- **Modes:**
  - **New / Replace:** Replaces the current selection with the painted stroke/region.
  - **Add (Shift):** Adds to the active selection (`f32::max` combination).
  - **Subtract (Option / Alt):** Erases from the active selection.
  - **Intersect (Shift + Option):** Intersects with the active selection.
- **Closed Loop Auto-Detection:**
  - When the user draws an enclosing stroke where the distance between the starting dab and finishing dab is within `min(radius, 24.0)` document pixels, a closed loop is detected.
  - The closed path is triangulated/rasterized (`photocraft_algo::selection::polygon_surface`).
  - The interior raster is combined with the stroke via `combine_sparse(f32::max)`. Because the interior boundary connects at the stroke's centerline, the outside falloff is unaffected, preserving 100% of the authentic brush hardness.
- **Live Preview:**
  - While dragging, `trail.feed_dabs(&d.points, brush)` computes live dabs with the user-selected hardness curve so the on-canvas preview accurately depicts soft or hard edges before release.
- **Overlay Customization:**
  - Located on the top options bar when `Tool::SelectionBrush` is selected.
  - Preset overlay color picker (default red `#FF0000`).
  - Overlay opacity slider (default 50%, adjustable from 1% to 100%).

## Implementation Architecture

- **`crates/ui-egui/src/canvas/paint.rs`:** Directs live pointer motion to `trail.feed_dabs(&d.points, brush)` for `Tool::SelectionBrush`.
- **`crates/engine/src/cmd/smartselect_cmds.rs`:** Implements `select.brush` command, handling stroke dab rasterization, closed loop polygon surface interior fill, and selection channel combination modes.
- **`crates/ui-egui/src/canvas.rs`:** Handles canvas overlay rendering and suppresses marching ants while on `SelectionBrush`.
- **`crates/ui-egui/src/panels/options_bar.rs` & `tools/retouch_ui.rs`:** Options bar controls for overlay color and opacity.
- **`crates/ui-egui/src/state/tool_options.rs`:** Storage and persistence for Selection Brush options.

## Automated Tests

- `selection_brush_closed_loop_auto_fill`: Validates closed loop detection and interior filling.
- `selection_brush_closed_loop_soft_hardness_preserves_outer_falloff`: Validates that soft outer falloff is identical between open and closed strokes.
- `selection_brush_hardness_falloff`: Validates soft brush falloff at 0% hardness vs 100% hardness.
- `selection_brush_add_subtract_and_replace`: Tests selection combine modes.
- `selection_brush_trail_shows_brush_hardness_live`: Validates live trail soft falloff rendering during pointer drag.

