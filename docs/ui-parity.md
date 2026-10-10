# UI parity: tools, handles, modifiers and feel

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** major (first version: tools, transform, nudging, snapping, modifiers, navigation, shortcuts, panels audited from source and open issues) · **Target:** Adobe Photoshop 2026

How PhotoCraft's direct manipulation compares with Photoshop 2026 (27.11.0): the toolbox, the
handles and modifier keys of each gesture, nudging, snapping, numeric precision, navigation,
shortcuts and panels. Painting feel has its own checklist in [`brush-parity.md`](brush-parity.md)
and right-click menus in [`context-menu-parity.md`](context-menu-parity.md). The design system
(themes, tokens, widgets) is [`ui-design.md`](ui-design.md).

Status: **done** (behaves as in Photoshop, with a test), **partial** (exists, differs or untested),
**missing**. Evidence is a file under `crates/ui-egui/src/` unless another path is given, a
scorecard id, or an issue. Modifier names are macOS first (⌘ ⌥ ⇧); Windows/Linux use Ctrl and Alt.

**Summary (estimated, weights = how often the gesture is used):** ~40% ready. Tools 53/68
present (78%, measured). Free Transform modifiers and Crop are close to Photoshop; the gaps are
in the long tail of modifiers, in-transform UI, spring-loaded tools, panels and preferences.
Remaining: 60–110 h for transforms and modifiers (gap G5), 60–100 h for tools (G6), 150–250 h
for panels and workspace (G9).

## Tools

Measured: the `Tool` enum (`state.rs:64`) and `TOOL_SECTIONS` (`panels.rs:17`, 22 flyout slots)
against the 67 `<tool>` entries of Photoshop's `Default Keyboard Shortcuts.kys` plus Remove,
Triangle and Curvature Pen (minus legacy Rounded Rectangle and Targeted Adjustment) = 68.

| Group | Present | Missing |
|---|---|---|
| Move, artboard | Move | Artboard (commands `layer.new.artboard*` exist) |
| Marquee | Rectangular, Elliptical | Single Row, Single Column |
| Lasso | Lasso, Polygonal, Magnetic | |
| Selection | Object Selection, Quick Selection, Magic Wand | |
| Crop and slice | Crop, Slice, Slice Select | Perspective Crop |
| Frame | | Frame (`layer.new.frameFromLayers` exists) |
| Eyedropper | Eyedropper, Ruler, Note, Count | Color Sampler (#1046) |
| Retouch | Remove, Spot Healing, Healing, Patch, Content-Aware Move, Red Eye | |
| Brush | Brush, Pencil, Mixer Brush | Color Replacement (`paint.colorReplacement` exists) |
| Stamp | Clone Stamp, Pattern Stamp | |
| History | History Brush | Art History Brush |
| Eraser | Eraser, Background Eraser, Magic Eraser | |
| Fill | Gradient, Paint Bucket | |
| Focus, toning | Blur, Sharpen, Smudge, Dodge, Burn, Sponge | |
| Pen | Pen | Freeform Pen, Curvature Pen (#1054), Add Anchor Point, Delete Anchor Point, Convert Point (⌥ with Pen only) |
| Type | Horizontal, Vertical | Horizontal Type Mask, Vertical Type Mask |
| Path selection | Path Selection, Direct Selection | |
| Shapes | Rectangle, Ellipse, Triangle, Polygon, Line, Custom Shape | (Star is a Polygon option in Photoshop) |
| View | Hand, Rotate View, Zoom | |
| Toolbar controls | Edit Toolbar, colour chips, Quick Mask, Screen Mode | |

| Toolbar behaviour | Status | Evidence |
|---|---|---|
| Letter shortcuts, ⇧+letter cycles the group, "Use Shift Key for Tool Switch" | done | `shortcuts.rs:413` |
| Group remembers the last tool used | partial | L always resets to Lasso (#2608) |
| Press-and-hold or click-drag opens the flyout | missing | right-click only (#2301) |
| Edit Toolbar customization | done | `edit.toolbar` |

## Free Transform (⌘T) and transform handles

`transform_tool.rs` (3,158 lines), behaviour summary in [`ui-design.md`](ui-design.md#interaction-models-match-photoshop-cc).

| Behaviour | Status | Evidence |
|---|---|---|
| Eight handles, drag inside moves, outside rotates with rotation cursor and live angle | done | #1948 |
| Corners and edges proportional by default; ⇧ frees them (edge stretches one axis) | done | #1873 |
| ⌥ scales about the reference point; ⌥-click places it; draggable reference point | done | |
| ⌘-corner distort, ⌘-edge skew, ⌘⌥⇧ perspective, ⇧ rotate in 15° steps | done | |
| Warp: presets, bend, split crosswise/vertical/horizontal, 3×3–5×5 and custom grids | done | |
| Options bar X/Y, W/H % with link, angle | done | |
| Options bar H/V skew fields | missing | |
| Interpolation in the options bar | partial | Bicubic/Bilinear/Nearest; no Bicubic Smoother/Sharper/Automatic, Preserve Details |
| Commit/cancel buttons in the options bar | missing | #2442 |
| Right-click menu during transform (Scale, Rotate, Skew, Distort, Perspective, Warp, flips, rotates) | missing | TOOL-214-5 |
| Arrow keys nudge the box 1 px, ⇧ 10 px | done | `move_mods.rs` |
| Preview keeps layer order, blend mode and mask | partial | mask yes (BUG-205-2); order/blend not (#2439) |
| Preview clipped to the canvas, follows edits while open | done | #2201, #2099 |
| Hand (space) mid-transform doesn't interrupt it | partial | #2302 |
| Blend mode / opacity edits while transforming | missing | #1384 |
| Locked layer refused before the drag | missing | fails only at commit (#2585) |
| Transform a copy (⌥⌘T), step and repeat (⌥⇧⌘T) | done | 2026-10-07 |
| Transform a targeted alpha channel | missing | BUG-207-4 |
| Smart object perspective (non-affine) | missing | FILE-215-6, #579 |
| Type-layer transform while editing (⌘ held), no ⌘-corner distort on type | done | `ui-design.md` |
| Puppet Warp mesh follows the outline | missing | block grid (TOOL-214-6) |

## Move tool and nudging

| Behaviour | Status | Evidence |
|---|---|---|
| Arrow 1 px, ⇧-arrow 10 px for layers | done | `move_mods.rs` |
| Arrow nudge of a selection outline (selection tools) | done | #1502 |
| Arrow nudge of the crop box, X swaps orientation | done | TOOL-214-19 |
| Arrow nudge of path points with Direct Selection | missing | no arrow handling in `direct_select.rs` |
| ⌘-arrow nudges the layer from any tool | missing | #2474 |
| ⌥-drag duplicates, ⇧ constrains to 45° | done | `move_mods.rs` |
| Auto-Select with hover outline, box-select layers, double-click fit | missing | TOOL-214-4 |
| Auto-Select bounding box and snapping follow the picked layer | partial | follow the previous layer (#2666) |
| ⌘-drag temporary Move shows the box and snaps | partial | #2681 |
| ⌘-right-click layer list under the pointer | done | `layer_pick_ui.rs` |
| Moving a selection on a mask moves the mask, not the pixels | partial | #2468 |

## Selection modifiers

| Behaviour | Status | Evidence |
|---|---|---|
| ⇧ square/circle, ⌥ from centre, live W × H readout (marquee) | done | `marquee_tests.rs` |
| ⇧ add, ⌥ subtract, ⇧⌥ intersect from modifiers at press | partial | works; only a badge test (BUG-208-4) |
| Space repositions the marquee while dragging | done | `hold_keys.rs` |
| Polygonal Lasso ⇧ snaps to 45° | missing | #2352 |
| Autoscroll at the window edge while selecting | missing | #2231 |
| Single-pixel rows and columns kept on release | partial | #2391 |
| Esc deselects / ⌘Z undoes the last selection | partial | #2674, #1432 |
| Select › Modify live preview | missing | #1752 |

## Pen, paths and shapes

| Behaviour | Status | Evidence |
|---|---|---|
| Click corner, drag smooth, ⌥ breaks a handle, ⌘ direct-selects, ⇧ 45° | partial | TOOL-214-10 |
| Handle on the last point, close-drag handle | partial | #1482 |
| Path operations in the options bar (combine, subtract, intersect, exclude) | partial | `layer.combineShapes.*`; Pen options #1397 |
| On-canvas corner-radius handles on live shapes | missing | TOOL-214-12 |
| Shape stroke controls (dash, caps, joins, align) | done | TOOL-1053-1, [`shape-strokes.md`](shape-strokes.md) |
| Rotated shape selection hints correct | partial | #2512 |
| Curved shape bounds exact | partial | one pixel too wide (#2537) |

## Crop

| Behaviour | Status | Evidence |
|---|---|---|
| Drag outside rotates (⇧ 15°), angle readout; ↵ rotates and crops in one undo step | done | TOOL-214-18 |
| Straighten button and ⌘-drag line | done | TOOL-214-21 |
| Overlays (thirds, grid, diagonal, triangle, golden ratio/spiral), O / ⇧O | done | TOOL-214-20 |
| Crop shield, Show Cropped Area (H) | done | TOOL-214-22 |
| W × H × Resolution with units, saved presets | partial | TOOL-214-9, #2443 |
| Content-Aware crop fill, Delete Cropped Pixels off | partial | |

## Snapping, guides, rulers, precision

| Behaviour | Status | Evidence |
|---|---|---|
| Snap to guides, grid, layers, document bounds and centre, selection, slices; ⌘ disables while dragging | done | `engine/src/snap.rs`, `snap_ui.rs` |
| Snap vector tools and transforms to the pixel grid | done | `snap_ui.rs:196` |
| Smart guides (magenta alignment lines) | done | |
| Smart guide distance and spacing labels | missing | |
| Ruler units px/in/cm/mm/pt/pica/% | done | |
| Status bar follows the ruler unit | partial | #2602 |
| Ruler corner and gaps drawn as Photoshop | partial | #2600, #2601, #2551 |
| Guides stored as f64 | missing | `Vec<f32>` (UI-217-20) |
| Geometry precision | partial | `geom::Point` f64, paths f64; paint internals mostly f32 |
| Numeric fields: arithmetic, scrubby labels | partial | units in expressions and double-click reset missing (UI-217-13) |

## Navigation

| Behaviour | Status | Evidence |
|---|---|---|
| Space = Hand, ⌘Space / ⌘⌥Space = zoom in/out, spring-loaded while held | done | `hold_keys.rs` |
| Hold any tool letter for a temporary tool | missing | #2672 |
| Scrubby zoom | done | `zoom_tool.rs` |
| Flick panning | missing | `tools.enable_flick_panning` unread |
| Animated zoom | missing | unread pref; #566 |
| Double-click Zoom tool = 100%, double-click Hand = fit | partial | Zoom missing (#2508) |
| ⌥-wheel zoom in gentle steps | missing | UI-217-18 |
| Wheel, smooth scrolling and pinch as in Photoshop | partial | #2485, #2463, #1989 |
| Trackpad rotate drives Rotate View | done | `rotate_view.rs:136` |
| Zoom levels to 3200% and beyond | partial | #1813 |
| Physical 100% at any display scale | done | #1943 |
| ⌃Tab / ⌃⇧Tab switch document tabs | missing | #2340 |

## Keyboard shortcuts

| Measure | Value | Kind |
|---|---|---|
| Photoshop default shortcuts (`.kys`) | 58 tool keys, 107 menu commands with 115 shortcuts, 25 tool-area commands | measured |
| PhotoCraft | ~199 shortcut assignments, 113 distinct modifier/F-key combos, 20 tool letters, 3 temporary-tool bindings | measured (source) |
| Shortcut audit | 214 → 0 failures (2026-10-07) | measured then; not re-run here |
| `.kys` import | done (`engine/src/kys.rs`) | |
| Shortcut glyphs per platform | missing (UI-217-19) | |

## Panels and workspace

30 of 35 Window-menu panels exist (measured). Missing: Libraries, Comments, Contextual Task Bar,
Discover, Generative variations. Dock tabs: Layers, Channels, Paths, History, Actions, Layer
Comps, Properties, Adjustments, Navigator, Histogram, Info, Color, Swatches, Character,
Paragraph; the rest open as in-app floating windows.

| Behaviour | Status | Evidence |
|---|---|---|
| Panel groups float and tear off into OS windows | missing | UI-217-5, #1828, #1213 |
| Drag panels to reorder and regroup | partial | #2272, #2305 |
| Docked resize pushes the neighbour | partial | #2573 |
| Color panel wheel and RGB/HSB/CMYK/Lab sliders | missing | UI-217-17, #2569 |
| Window size and dock width remembered | done | UI-217-2 |
| Command palette with recents | done | UI-217-7 |
| Every preference takes effect | partial | 46 of 149 do nothing (scorecard) |
| UI scale presets | missing | UI-217-9, #2492 |
| Themes: Photoshop's four greys | partial | five themes; #2661 |
| Document tabs: overflow, context menu, reorder | done | UI-217-6; middle-click close #2570 |
| Layers panel: wheel steps blend modes, drop on New/Trash | partial | UI-217-14 |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | major | First version, from a source audit against Photoshop 2026 27.11.0's shortcuts file and the open issues |
