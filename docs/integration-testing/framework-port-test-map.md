# Tests affected by the shared UI port

Tracking issue [#15466](https://github.com/storytold/photocraft/issues/15466), part of the [UI rework](https://github.com/storytold/photocraft/issues/15476).

This inventory compares the PhotoCraft parent [`01b79958a7`](https://github.com/storytold/photocraft/commit/01b79958a7b73c3d840b76492069e9635f9f7394) with the framework port [`ed14e1c413`](https://github.com/storytold/photocraft/commit/ed14e1c413ebea61dcb54abda3d388b33a29a4af). Successors were checked in PhotoCraft [`42c853396d`](https://github.com/storytold/photocraft/commit/42c853396d147a554f540c9723b90e1179080a79) and its pinned craft-libs [`683fe06031`](https://github.com/storytold/craft-libs/commit/683fe06031224725b913c8ad3697d8e8bfec6c90).

The scan found **226 whole-file removals** with **1,659 recognized test declarations**. 215 paths belonged to `ui-egui`. The scan includes inline tests in implementation files and counts whole paths, so its file total uses a different scope from the original issue’s test-file count. Declarations removed from files that stayed in place are outside this inventory.

The source scan recognized Rust functions marked `#[test]`, namespaced `#[…::test]` and `#[trial]`. 121 declarations have at least one same-name candidate. The inventory also records bodies with identical Rust tokens after dropping comments and whitespace. Names and body matches identify places to inspect. The behavior notes below come from reading assertions. No tests were run for this audit.

## Reviewed successors

| Removed PhotoCraft source | Current test | Assertions retained and scope |
| --- | --- | --- |
| [`crates/ui-egui/src/canvas/rectangle_click_in_shape_tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/canvas/rectangle_click_in_shape_tests.rs#L41) | [PhotoCraft rectangle input](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/tools/vector_ui/rectangle_click_in_shape_tests.rs#L41) | Inside-click and one-pixel motion leave the live shape alone. Rounded corners use the outline, and path and pixel modes retain their distinct behavior. |
| [`crates/ui-egui/src/canvas/canvas_autoscroll.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/canvas/canvas_autoscroll.rs#L140) | [Autoscroll edges](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-canvas/tests/it/canvas/navigate.rs#L734) | Checks edge clamping, fitting and overscrolled images, a rotated view with pixel aspect, and an oblique screen axis. The old HiDPI, flip and negative quarter-turn cases need separate review. |
| [`crates/ui-egui/src/canvas/rotate_view/tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/canvas/rotate_view/tests.rs#L163) | [Rotate View drag](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-canvas/tests/it/canvas/tools.rs#L312) | Checks pointer movement in both flip states and 15-degree Shift steps. PhotoCraft history and key routing are outside this fixture. |
| [`crates/ui-egui/src/canvas/zoom_hold.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/canvas/zoom_hold.rs#L139) | [Zoom hold and scrub](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-canvas/tests/it/canvas/tools.rs#L381) | Checks the anchor, 30 and 120 Hz holds, an animation-off step, quick clicks and drag scrubbing. Nonfinite input, limits and history still need review. |
| [`crates/ui-egui/src/canvas/pixel_grid.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/canvas/pixel_grid.rs#L194) | [Grid line colour](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-canvas/tests/it/canvas_controls/grid.rs#L76), [Canvas grid](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/tests/it/gpu_tests/checker_canvas.rs#L169) | Checks transparency, adaptive contrast and the 500% threshold in the app. Exact 501% behavior and panning-cache bounds remain open. |
| [`crates/ui-egui/tests/it/crop_shift_corner.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/tests/it/crop_shift_corner.rs#L100) | [Crop corner diagonal](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-canvas/tests/it/canvas/crop.rs#L756) | Checks the ratio, reachable guide snap and distant-guide rejection in pending geometry. PhotoCraft’s committed result is a separate assertion. |
| [`crates/ui-egui/tests/it/crop_turn_keeps_frame_inside.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/tests/it/crop_turn_keeps_frame_inside.rs#L152) | [Turned crop fit](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-canvas/tests/it/canvas/crop.rs#L667) | Checks fit, shrink, restore, corner movement and the Command override. Committed pixels and a modifier changed during a turn remain open. |
| [`crates/ui-egui/src/tools/crop_mode/tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/tools/crop_mode/tests.rs#L519) | [Dropped crop on document switch](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/tools/crop_mode_tests.rs#L92) | Switching away drops the pending crop and returns the original view upright. The old test repeated the scenario in both document orders. |
| [`crates/ui-egui/src/input/control_click.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/input/control_click.rs#L45) | [macOS Control-click](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-contract/tests/it/runtime/control_click.rs#L39), [PhotoCraft menus](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/canvas_menu_tests.rs#L88) | Checks secondary click through release, other modifiers and platforms, re-entry and the next primary click. PhotoCraft also checks that the right menus open. |
| [`crates/ui-egui/src/input/scrub_wrap.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/input/scrub_wrap.rs#L75) | [Scrub wrap](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-widgets/tests/it/label/mod.rs#L44) | Checks that a warp landing and the next move preserve a continuous value. Movement coalesced with the landing event and both logical edges remain open. |
| [`crates/ui-egui/src/input/page_keys.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/input/page_keys.rs#L116) | [Canvas page navigation](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-canvas/tests/it/canvas/navigate.rs#L62) | Checks vertical Page Down and horizontal Cmd or Ctrl Page Down. The legacy zoom-scaled amount, Shift step and Home or End corner assertions still need review. |
| [`crates/ui-egui/src/menus/menu_keys/harness_tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/menus/menu_keys/harness_tests.rs#L397) | [Command shortcut focus](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-widgets/tests/it/menu/pick_focus.rs#L131) | A command dispatches once, closes the menu and restores its prior focus. |
| [`crates/ui-egui/src/menus/menu_nav/tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/menus/menu_nav/tests.rs#L453) | [Popup clearance](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-widgets/tests/it/menu/overflow.rs#L169) | The menu and submenu clear the bar and stay in the window. The original ran across more display and menu configurations. |
| [`crates/ui-egui/src/menus/menu_nav/tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/menus/menu_nav/tests.rs#L832) | [Submenu pointer path](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-widgets/tests/it/menu/submenus.rs#L42) | Moving toward the open submenu over a competing row keeps it open, then a rested pointer switches it. The legacy path matrix was broader. |
| [`crates/ui-egui/src/widgets/type_ahead.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/widgets/type_ahead.rs#L129) | [Localized type-ahead](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-widgets/tests/it/pickers/select_list.rs#L118) | Checks Azerbaijani dotted and dotless I, combining dot and Greek. The old split composing sequence needs separate review. |
| [`crates/ui-egui/src/widgets/tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/widgets/tests.rs#L629) | [Text event limits](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-contract/tests/it/text/input_cap.rs#L110) | Checks Unicode character limits for text, paste and IME and removes an out-of-bounds preedit range. |
| [`crates/ui-egui/src/widgets/tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/widgets/tests.rs#L916) | [Numeric expression stepping](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-widgets/tests/it/text/number.rs#L614) | Arrow keys evaluate `5+5`, reach 11 then 10, and keep focus in the field. |
| [`crates/ui-egui/src/widgets/task_bar.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/widgets/task_bar.rs#L203) | [Task bar placement](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-canvas/tests/it/canvas_controls/task_bar.rs#L185) | Checks anchoring, manual placement, pin/reset, clamping and nonfinite input. Exact spacing and a fixed left edge during content growth need separate review. |
| [`crates/ui-egui/src/panels/layer_row_ui/edit_tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/panels/layer_row_ui/edit_tests.rs#L161) | [Invalid type edits](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/layers_thumbnail_tests.rs#L146) | A rejected type edit keeps the active tool and leaves text editing inactive. |
| [`crates/ui-egui/src/panels/layer_row_ui/tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/panels/layer_row_ui/tests.rs#L804) | [Masked artboard retargeting](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/layers_thumbnail_tests.rs#L164) | Clicking an artboard thumbnail exits mask view and retargets the artboard. |
| [`crates/ui-egui/src/panels/props_layout/tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/panels/props_layout/tests.rs#L338) | [Background plus another layer](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/panels/properties.rs#L165) | A single Background hides alignment actions. A multiple selection keeps them and horizontal alignment moves only x. |
| [`crates/ui-egui/src/panels/titlebar/tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/panels/titlebar/tests.rs#L278) | [Dirty window close](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/panels/title_bar/tests.rs#L50) | Checks the unsaved-work prompt for a drawable window and direct closing after device loss. The former no-document case needs separate review. |
| [`crates/ui-egui/src/panels/layer_reveal/tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/panels/layer_reveal/tests.rs#L69) | [Layer reveal and group behavior](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/layers/tests/it/panel_reveal.rs#L136), [Per-document scroll](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/layers/tests/it/panel_reveal.rs#L188) | Checks group expansion, row visibility, stable visible rows, optional group preservation and retained scroll. PhotoCraft’s undo-triggered reveal still needs a fixture. |
| [`crates/ui-egui/src/dialogs/export_dialog/enter_key_tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/dialogs/export_dialog/enter_key_tests.rs#L51) | [Explicit export button](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-patterns/tests/it/form_dialogs/hosted.rs#L265), [Scale field Enter](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-patterns/tests/it/form_dialogs/hosted.rs#L348), [Escape cancellation](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-patterns/tests/it/form_dialogs/hosted.rs#L104) | Bare Enter keeps Export As open. Enter in Scale commits the field, only the button requests a path, and Escape reaches the host as Cancel. |
| [`crates/ui-egui/src/dialogs/image_size_readout_tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/dialogs/image_size_readout_tests.rs#L43) | [Flat image size](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/dialogs/sizing/tests.rs#L159), [Bitmap row padding and previous size](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-patterns/tests/it/form_dialogs/hosted.rs#L385) | Checks channel and depth calculations, whole-byte bitmap rows and when the previous size appears. Localized formatting and same-byte changes remain open. |
| [`crates/ui-egui/src/dialogs/image_size_resample_shortcuts_tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/dialogs/image_size_resample_shortcuts_tests.rs#L74) | [Alt-digit resampling](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-patterns/tests/it/form_dialogs/hosted.rs#L404) | Checks two picks, ignored chords, typing in the field and unavailable choices. Full key position coverage, same-frame ordering and applying the choice remain open. |
| [`crates/ui-egui/src/dialogs/camera_raw_preview_ui.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/dialogs/camera_raw_preview_ui.rs#L299) | [Leaving Fit](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-canvas/tests/it/full_window_parts.rs#L27), [Pointer anchor](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-canvas/tests/it/full_window_parts.rs#L107), [Shift+Alt zoom ladder](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/ui-canvas/tests/it/canvas/navigate.rs#L555) | Shared canvas tests cover the core navigation. The Camera Raw input wiring and its pan and snap limits still need app-level coverage. |

The Magnetic Lasso cursor-preference matrix returned in [PhotoCraft #15805](https://github.com/storytold/photocraft/pull/15805). The removed Alt+Tab case has a draft in [craft-libs #2805](https://github.com/storytold/craft-libs/pull/2805). Customized-workspace Properties coverage is tracked in [#15791](https://github.com/storytold/photocraft/issues/15791).

## Coverage still to review

These assertions have no equivalent identified in the reviewed successors. The gaps concern test coverage based on these snapshots.

- Selection outline paint order above guides across zoom and DPI and drawing in a secondary view. [Original assertion](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/canvas/guide_ants_tests.rs#L49)
- Escape and Enter routing while the crop ratio list owns the keyboard, with the crop remaining pending. [Original assertion](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/tests/it/crop_popup_keys.rs#L72)
- The pointer-driven third and fourth corner and hover grid. The current app tests cover committed quad input. [Original assertion](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/tools/perspective_crop_click_preview_tests.rs#L31)
- Perspective Crop hover hit zones and keeping the drag cursor while moving. [Original assertion](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/canvas/perspective_crop_cursor_tests.rs#L45)
- Native pinch and Ctrl-wheel in the same frame coalesce into one zoom. [Original assertion](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/canvas/wheel_nav/tests.rs#L770)
- Non-US physical bracket fallback, the Dvorak exception and leaving the German plus key alone. [Original assertion](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/input/layout_keys.rs#L142)
- Off-field diagonal selection follows x on single-line fields and preserves multiline movement. [Original assertion](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/input/single_line_drag.rs#L157)
- Field-local rejection before parsing, including Unicode, IME, undo and same-frame Tab. [Original assertion](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/widgets/numeric_input_tests.rs#L52)
- Fit Image validation and undo across field editing and programmatic input. [Original assertion](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/dialogs/fit_image_range_tests.rs#L59)
- Camera Raw-specific clamped panning, snap limits and fitting after tiny or nonfinite resize. [Original assertion](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/dialogs/camera_raw_preview_ui.rs#L299)
- Dialog-specific undo isolation for Filter Gallery and Vanishing Point. [Original assertion](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/dialogs/edit_history.rs#L197)

## Identical body matches

Each row links the first matched declaration at a destination and counts matching bodies at that path. The inventory counts unique legacy declarations, so a test at more than one destination is counted once there.

| Removed source | Candidate location | Matched bodies |
| --- | --- | ---: |
| [`apps/photocraft/src/icon_theme.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/apps/photocraft/src/icon_theme.rs#L280) | [`crates/window/src/desktop/icon_theme.rs`](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/window/src/desktop/icon_theme.rs#L253) | 4 |
| [`apps/photocraft/src/portal_save/tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/apps/photocraft/src/portal_save/tests.rs#L5) | [`crates/dialogs/src/portal/tests.rs`](https://github.com/storytold/craft-libs/blob/683fe06031224725b913c8ad3697d8e8bfec6c90/crates/dialogs/src/portal/tests.rs#L23) | 1 |
| [`crates/ui-egui/src/canvas/rectangle_click_in_shape_tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/canvas/rectangle_click_in_shape_tests.rs#L54) | [`crates/ui-egui/src/tools/vector_ui/rectangle_click_in_shape_tests.rs`](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/tools/vector_ui/rectangle_click_in_shape_tests.rs#L54) | 2 |
| [`crates/ui-egui/src/dialogs/adjust_editors/curves.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/dialogs/adjust_editors/curves.rs#L545) | [`crates/ui-egui/src/dialogs/adjust_editors/curves.rs`](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/dialogs/adjust_editors/curves.rs#L546) | 13 |
| [`crates/ui-egui/src/shape_bounds_box_tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/shape_bounds_box_tests.rs#L86) | [`crates/ui-egui/src/tools/shape_controls/box_tests.rs`](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/tools/shape_controls/box_tests.rs#L71) | 2 |
| [`crates/ui-egui/src/tools/analysis_ui/tests.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/tools/analysis_ui/tests.rs#L147) | [`crates/ui-egui/src/tools/analysis/tests.rs`](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/tools/analysis/tests.rs#L134) | 2 |
| [`crates/ui-egui/src/widgets/point_curve.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/widgets/point_curve.rs#L229) | [`crates/ui-egui/src/widgets/point_curve.rs`](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/widgets/point_curve.rs#L262) | 6 |
| [`crates/ui-egui/src/widgets/tone.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/src/widgets/tone.rs#L582) | [`crates/ui-egui/src/tone.rs`](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/tone.rs#L533) | 13 |
| [`crates/ui-egui/tests/it/custom_print_paper.rs`](https://github.com/storytold/photocraft/blob/01b79958a7b73c3d840b76492069e9635f9f7394/crates/ui-egui/tests/it/custom_print_paper.rs#L234) | [`crates/ui-egui/src/dialogs/file_ui/custom_paper_tests.rs`](https://github.com/storytold/photocraft/blob/42c853396d147a554f540c9723b90e1179080a79/crates/ui-egui/src/dialogs/file_ui/custom_paper_tests.rs#L200) | 1 |

## Whole-file inventory

A dash means no same-name candidate or identical body surfaced in the scanned snapshots. Current tests can cover a behavior under a different name. The assertion comparisons above record reviewed examples, including renamed tests.

| Removed path | Cases | Same-name leads | Identical bodies |
| --- | ---: | ---: | ---: |
| `apps/photocraft/src/agent_link.rs` | 6 | 1 | — |
| `apps/photocraft/src/appearance.rs` | 3 | — | — |
| `apps/photocraft/src/caption_style.rs` | 7 | — | — |
| `apps/photocraft/src/control_server.rs` | 8 | 1 | — |
| `apps/photocraft/src/file_dialog_probe.rs` | 7 | 2 | — |
| `apps/photocraft/src/icon_theme.rs` | 9 | 8 | 4 |
| `apps/photocraft/src/mac_menu.rs` | 2 | — | — |
| `apps/photocraft/src/portal_save/tests.rs` | 11 | 1 | 1 |
| `apps/photocraft/src/screen_color.rs` | 6 | — | — |
| `crates/engine/src/cmd/updates_cmds.rs` | 4 | — | — |
| `crates/engine/src/kys.rs` | 10 | 1 | — |
| `crates/ui-egui/examples/cursor_bench/args.rs` | 1 | — | — |
| `crates/ui-egui/src/background_eraser_alt_click_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/background_eraser_bar_angle_tests.rs` | 13 | 1 | — |
| `crates/ui-egui/src/background_eraser_brush_popup_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/background_eraser_digit_keys_tests.rs` | 7 | — | — |
| `crates/ui-egui/src/blend_dropdown_groups_tests.rs` | 2 | — | — |
| `crates/ui-egui/src/blend_dropdown_keys_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/blend_dropdown_wheel_tests.rs` | 9 | — | — |
| `crates/ui-egui/src/blur_strength_alert_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/brush_alt_digit_smoothing_tests.rs` | 7 | — | — |
| `crates/ui-egui/src/brush_hud_rate_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/brush_opacity_minimum_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/brush_options_tooltips_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/brush_picker_angle_widget_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/bucket_bitmap_alert_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/burn_bar_tooltips_tests.rs` | 10 | — | — |
| `crates/ui-egui/src/burn_default_tip_tests.rs` | 3 | 2 | — |
| `crates/ui-egui/src/burn_exposure_field_tests.rs` | 10 | — | — |
| `crates/ui-egui/src/cam_options_bar_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/cam_reset_tool_mode_tests.rs` | 3 | — | — |
| `crates/ui-egui/src/canvas/ants.rs` | 7 | — | — |
| `crates/ui-egui/src/canvas/canvas_autoscroll.rs` | 10 | — | — |
| `crates/ui-egui/src/canvas/canvas_popup.rs` | 2 | — | — |
| `crates/ui-egui/src/canvas/canvas_tool_menu/hand_menu_entries_tests.rs` | 3 | — | — |
| `crates/ui-egui/src/canvas/canvas_tool_menu/zoom_menu_anchor_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/canvas/guide_ants_tests.rs` | 2 | — | — |
| `crates/ui-egui/src/canvas/paint.rs` | 5 | — | — |
| `crates/ui-egui/src/canvas/pattern_preview.rs` | 6 | — | — |
| `crates/ui-egui/src/canvas/perspective_crop_cursor_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/canvas/pixel_grid.rs` | 16 | — | — |
| `crates/ui-egui/src/canvas/rectangle_click_in_shape_tests.rs` | 6 | 6 | 2 |
| `crates/ui-egui/src/canvas/remove_cursor_tests.rs` | 3 | — | — |
| `crates/ui-egui/src/canvas/rotate_view/tests.rs` | 21 | — | — |
| `crates/ui-egui/src/canvas/sampler_cursor_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/canvas/scrollbars.rs` | 11 | — | — |
| `crates/ui-egui/src/canvas/tool_cursor/add_anchor_cursor_tests.rs` | 8 | — | — |
| `crates/ui-egui/src/canvas/tool_cursor/count_tests.rs` | 5 | 1 | — |
| `crates/ui-egui/src/canvas/tool_cursor/ruler_tests.rs` | 9 | — | — |
| `crates/ui-egui/src/canvas/tool_cursor/tests.rs` | 50 | — | — |
| `crates/ui-egui/src/canvas/wheel_nav/scroll_step_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/canvas/wheel_nav/tests.rs` | 52 | — | — |
| `crates/ui-egui/src/canvas/zoom_field.rs` | 6 | — | — |
| `crates/ui-egui/src/canvas/zoom_hold.rs` | 9 | — | — |
| `crates/ui-egui/src/canvas/zoom_levels.rs` | 6 | — | — |
| `crates/ui-egui/src/canvas/zoom_window_fit.rs` | 14 | — | — |
| `crates/ui-egui/src/clone_stamp_reset_tip_tests.rs` | 4 | 2 | — |
| `crates/ui-egui/src/color_replacement_alt_click_tests.rs` | 3 | — | — |
| `crates/ui-egui/src/color_replacement_blend_keys_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/color_replacement_tolerance_field_tests.rs` | 8 | — | — |
| `crates/ui-egui/src/dialogs/adjust_editors/curves.rs` | 13 | 13 | 13 |
| `crates/ui-egui/src/dialogs/camera_raw_preview_ui.rs` | 6 | — | — |
| `crates/ui-egui/src/dialogs/camera_raw_ui/controls.rs` | 2 | — | — |
| `crates/ui-egui/src/dialogs/credits/ui_tests.rs` | 12 | — | — |
| `crates/ui-egui/src/dialogs/dialog_alt_reset.rs` | 3 | — | — |
| `crates/ui-egui/src/dialogs/edit_history.rs` | 4 | — | — |
| `crates/ui-egui/src/dialogs/export_dialog/enter_key_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/dialogs/fit_image_range_tests.rs` | 7 | — | — |
| `crates/ui-egui/src/dialogs/image_size_readout_tests.rs` | 13 | — | — |
| `crates/ui-egui/src/dialogs/image_size_resample_shortcuts_tests.rs` | 9 | — | — |
| `crates/ui-egui/src/dialogs/label_column_tests.rs` | 1 | — | — |
| `crates/ui-egui/src/dialogs/layer_style/bevel_modes_tests.rs` | 1 | — | — |
| `crates/ui-egui/src/dialogs/rgb_histogram.rs` | 7 | — | — |
| `crates/ui-egui/src/dialogs/select_mask_ui/cycle_tool_mode_tests.rs` | 9 | — | — |
| `crates/ui-egui/src/dialogs/shape_dialog/create_triangle_range_alert_tests.rs` | 7 | — | — |
| `crates/ui-egui/src/dodge_default_tip_tests.rs` | 3 | 2 | — |
| `crates/ui-egui/src/dropdown_press_tests.rs` | 3 | — | — |
| `crates/ui-egui/src/empty_workspace_tests.rs` | 2 | — | — |
| `crates/ui-egui/src/eraser_range_alert_tests.rs` | 8 | — | — |
| `crates/ui-egui/src/i18n/casing.rs` | 1 | — | — |
| `crates/ui-egui/src/input/control_click.rs` | 4 | — | — |
| `crates/ui-egui/src/input/layout_keys.rs` | 3 | — | — |
| `crates/ui-egui/src/input/page_keys.rs` | 2 | — | — |
| `crates/ui-egui/src/input/scrub_wrap.rs` | 2 | — | — |
| `crates/ui-egui/src/input/single_line_drag.rs` | 4 | — | — |
| `crates/ui-egui/src/layer_shortcut_tests.rs` | 10 | — | — |
| `crates/ui-egui/src/layer_via_label_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/layers_empty_area_menu_tests.rs` | 10 | — | — |
| `crates/ui-egui/src/marquee_press_mods_tests.rs` | 3 | — | — |
| `crates/ui-egui/src/menu_item_labels_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/menus/menu_keys.rs` | 4 | — | — |
| `crates/ui-egui/src/menus/menu_keys/harness_tests.rs` | 23 | 1 | — |
| `crates/ui-egui/src/menus/menu_nav/tests.rs` | 24 | — | — |
| `crates/ui-egui/src/mixer_cursor_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/new_doc_pixel_aspect_tests.rs` | 7 | — | — |
| `crates/ui-egui/src/new_doc_save_preset_icon_tests.rs` | 7 | — | — |
| `crates/ui-egui/src/new_doc_web_artboards_tests.rs` | 8 | — | — |
| `crates/ui-egui/src/new_group_button_tests.rs` | 8 | — | — |
| `crates/ui-egui/src/options_hit_area_tests.rs` | 11 | — | — |
| `crates/ui-egui/src/panels/brushes_tab/tests.rs` | 12 | — | — |
| `crates/ui-egui/src/panels/compact_visual_regressions.rs` | 12 | — | — |
| `crates/ui-egui/src/panels/layer_filter_ui/effect_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/panels/layer_filter_ui/group_tests.rs` | 7 | — | — |
| `crates/ui-egui/src/panels/layer_menu_ui/color_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/panels/layer_pct_slider_tests.rs` | 7 | — | — |
| `crates/ui-egui/src/panels/layer_reveal/tests.rs` | 3 | — | — |
| `crates/ui-egui/src/panels/layer_row_ui/edit_tests.rs` | 7 | 1 | — |
| `crates/ui-egui/src/panels/layer_row_ui/rename_tests.rs` | 8 | — | — |
| `crates/ui-egui/src/panels/layer_row_ui/tests.rs` | 25 | 1 | — |
| `crates/ui-egui/src/panels/layer_tree_ui/tests.rs` | 7 | — | — |
| `crates/ui-egui/src/panels/options_bar/hand_tooltips_tests.rs` | 3 | — | — |
| `crates/ui-egui/src/panels/options_bar/zoom_buttons_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/panels/panel_state.rs` | 1 | — | — |
| `crates/ui-egui/src/panels/props_layout/tests.rs` | 13 | 1 | — |
| `crates/ui-egui/src/panels/titlebar/tests.rs` | 25 | — | — |
| `crates/ui-egui/src/pen_path_visual_tests.rs` | 1 | — | — |
| `crates/ui-egui/src/pencil_auto_erase_tooltip_tests.rs` | 1 | — | — |
| `crates/ui-egui/src/pencil_shift_digit_opacity_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/polygon_lasso_step_snap_tests.rs` | 7 | — | — |
| `crates/ui-egui/src/prefs/flick_pan.rs` | 4 | — | — |
| `crates/ui-egui/src/prefs/prefs_ui/shortcut_set_switch_tests.rs` | 10 | — | — |
| `crates/ui-egui/src/profile_prompt_tests.rs` | 2 | — | — |
| `crates/ui-egui/src/shape_bounds_box_tests.rs` | 18 | 6 | 2 |
| `crates/ui-egui/src/sharpen_refusal_alert_tests.rs` | 7 | 4 | — |
| `crates/ui-egui/src/sharpen_strength_alert_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/smoothing_options_greyed_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/smudge_default_tip_tests.rs` | 4 | 2 | — |
| `crates/ui-egui/src/smudge_mode_bar_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/smudge_strength_alert_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/sponge_default_tip_tests.rs` | 4 | 2 | — |
| `crates/ui-egui/src/sponge_flow_field_tests.rs` | 9 | — | — |
| `crates/ui-egui/src/sponge_refusal_alert_tests.rs` | 6 | 5 | — |
| `crates/ui-egui/src/state/type_default_size_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/status_bar_tests.rs` | 8 | — | — |
| `crates/ui-egui/src/text/bidi_text.rs` | 1 | — | — |
| `crates/ui-egui/src/text/egui_fonts.rs` | 3 | — | — |
| `crates/ui-egui/src/text/font_filters.rs` | 3 | — | — |
| `crates/ui-egui/src/text/font_preview.rs` | 5 | — | — |
| `crates/ui-egui/src/tool_options_theme_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/toolbar_more_slot_flyout_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/tools/analysis_ruler_click_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/tools/analysis_ruler_escape_tests.rs` | 2 | — | — |
| `crates/ui-egui/src/tools/analysis_ruler_move_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/tools/analysis_ruler_shift_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/tools/analysis_ruler_visibility_tests.rs` | 3 | — | — |
| `crates/ui-egui/src/tools/analysis_ui/sampler_drag_tests.rs` | 11 | — | — |
| `crates/ui-egui/src/tools/analysis_ui/sampler_extras_tests.rs` | 2 | — | — |
| `crates/ui-egui/src/tools/analysis_ui/sampler_menu_tests.rs` | 8 | 2 | — |
| `crates/ui-egui/src/tools/analysis_ui/tests.rs` | 14 | 9 | 2 |
| `crates/ui-egui/src/tools/anchor_tools/delete_anchor_shown_anchors_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/tools/corner_radius/tests.rs` | 13 | 5 | — |
| `crates/ui-egui/src/tools/crop_mode/tests.rs` | 26 | — | — |
| `crates/ui-egui/src/tools/crop_options_labels_tests.rs` | 10 | — | — |
| `crates/ui-egui/src/tools/crop_overlay.rs` | 13 | — | — |
| `crates/ui-egui/src/tools/crop_shield.rs` | 14 | 1 | — |
| `crates/ui-egui/src/tools/crop_size.rs` | 18 | — | — |
| `crates/ui-egui/src/tools/crop_straighten.rs` | 9 | — | — |
| `crates/ui-egui/src/tools/direct_select/alt_drag_duplicate_tests.rs` | 7 | — | — |
| `crates/ui-egui/src/tools/direct_select/convert_point_tolerance_tests.rs` | 9 | — | — |
| `crates/ui-egui/src/tools/direct_select/shift_angle_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/tools/ellipse_center_mark_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/tools/gradient_refusal_messages_tests.rs` | 4 | 2 | — |
| `crates/ui-egui/src/tools/lasso_ui/tests.rs` | 21 | 6 | — |
| `crates/ui-egui/src/tools/line_live_handles_tests.rs` | 8 | — | — |
| `crates/ui-egui/src/tools/magnetic_lasso_ui/caps_lock_cursor_tests.rs` | 3 | — | — |
| `crates/ui-egui/src/tools/perspective_crop_click_preview_tests.rs` | 9 | — | — |
| `crates/ui-egui/src/tools/perspective_crop_modifiers_tests.rs` | 11 | — | — |
| `crates/ui-egui/src/tools/perspective_crop_shield_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/tools/remove_default_size_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/tools/remove_locked_alert_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/tools/shape_stroke_ui/shortcut_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/tools/transform_tex.rs` | 5 | 1 | — |
| `crates/ui-egui/src/tools/transform_tool/warp_handles_visible_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/tools/transform_tool/warp_options_bar_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/tools/transform_tool/warp_split_one_shot_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/tools/type_tool/fonts_loading_tests.rs` | 1 | — | — |
| `crates/ui-egui/src/tools/type_tool/options_tooltips_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/tools/type_tool/size_presets_tests.rs` | 13 | — | — |
| `crates/ui-egui/src/tools/vector_ui/add_anchor_options_bar_tests.rs` | 1 | — | — |
| `crates/ui-egui/src/tools/vector_ui/delete_anchor_options_bar_tests.rs` | 2 | — | — |
| `crates/ui-egui/src/tools/vector_ui/pen_alt_drag_segment_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/tools/vector_ui/pen_mode_menu_tests.rs` | 5 | — | — |
| `crates/ui-egui/src/tools/vector_ui/rectangle_path_make_tests.rs` | 1 | — | — |
| `crates/ui-egui/src/tools/warp_preview.rs` | 4 | — | — |
| `crates/ui-egui/src/tools/zoom_tool/tests.rs` | 12 | 1 | — |
| `crates/ui-egui/src/tools/zoom_tool/zoom_out_mode_tests.rs` | 8 | — | — |
| `crates/ui-egui/src/transform_corner_projection_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/transform_reference_point_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/transform_relative_position_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/transform_rotate_snap_tests.rs` | 3 | — | — |
| `crates/ui-egui/src/widgets/angle_dial.rs` | 4 | — | — |
| `crates/ui-egui/src/widgets/color_button_tests.rs` | 4 | — | — |
| `crates/ui-egui/src/widgets/color_count.rs` | 3 | — | — |
| `crates/ui-egui/src/widgets/field_tab.rs` | 4 | — | — |
| `crates/ui-egui/src/widgets/field_wheel.rs` | 20 | — | — |
| `crates/ui-egui/src/widgets/numeric_expression.rs` | 5 | — | — |
| `crates/ui-egui/src/widgets/numeric_input_tests.rs` | 6 | — | — |
| `crates/ui-egui/src/widgets/point_curve.rs` | 6 | 6 | 6 |
| `crates/ui-egui/src/widgets/press_menu.rs` | 9 | — | — |
| `crates/ui-egui/src/widgets/swap_arrow.rs` | 3 | — | — |
| `crates/ui-egui/src/widgets/tab_strip.rs` | 5 | — | — |
| `crates/ui-egui/src/widgets/task_bar.rs` | 4 | — | — |
| `crates/ui-egui/src/widgets/tests.rs` | 82 | — | — |
| `crates/ui-egui/src/widgets/text_field_menu.rs` | 2 | — | — |
| `crates/ui-egui/src/widgets/tone.rs` | 15 | 13 | 13 |
| `crates/ui-egui/src/widgets/type_ahead.rs` | 8 | — | — |
| `crates/ui-egui/tests/it/actions_record_again.rs` | 4 | 4 | — |
| `crates/ui-egui/tests/it/background_eraser_reset_tip.rs` | 4 | 2 | — |
| `crates/ui-egui/tests/it/camera_raw_localization.rs` | 1 | — | — |
| `crates/ui-egui/tests/it/camera_raw_zoom.rs` | 6 | — | — |
| `crates/ui-egui/tests/it/crop_beyond_canvas.rs` | 1 | — | — |
| `crates/ui-egui/tests/it/crop_default_mode.rs` | 6 | — | — |
| `crates/ui-egui/tests/it/crop_frame_look.rs` | 10 | — | — |
| `crates/ui-egui/tests/it/crop_overlay_icons.rs` | 5 | — | — |
| `crates/ui-egui/tests/it/crop_popup_keys.rs` | 2 | — | — |
| `crates/ui-egui/tests/it/crop_shift_corner.rs` | 9 | — | — |
| `crates/ui-egui/tests/it/crop_turn_grid.rs` | 10 | — | — |
| `crates/ui-egui/tests/it/crop_turn_keeps_frame_inside.rs` | 8 | — | — |
| `crates/ui-egui/tests/it/custom_print_paper.rs` | 6 | 5 | 1 |
| `crates/ui-egui/tests/it/export_as_scaling.rs` | 2 | — | — |
| `crates/ui-egui/tests/it/eyedropper_ring.rs` | 2 | — | — |
| `crates/ui-egui/tests/it/marching_ants_pixels.rs` | 2 | — | — |
| `crates/ui-egui/tests/it/marquee_autoscroll.rs` | 3 | — | — |
| `crates/ui-egui/tests/it/transform_readout_units.rs` | 1 | — | — |
| `crates/ui-egui/tests/magnetic_lasso_spacing.rs` | 11 | — | — |
| `crates/ui-egui/tests/magnetic_lasso_width_keys.rs` | 5 | — | — |
