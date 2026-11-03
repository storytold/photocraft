# Historical 3D / UV validation — v0.16.0 source, 2026-10-27

Historical source baseline: official PhotoCraft v0.16.0,
`b1794c6e0608ccfc887213dcf58398977c5cb75e`, plus the local 3D/UV changes.
Checks ran on the Windows source checkout used for the working 0.16.0 build.
The publication checkout was compared with it; source differences are rustfmt and
the snapshot example's portable default output filename.

## Checks

| Check | Result |
|---|---|
| Native application, `cargo check -p photocraft --features uv-workbench --locked` | Passed |
| UV library unit tests | 6 passed, including FBX preflight rejection cases |
| Engine `uv_container` integration tests | 9 passed |
| CPU compositor `mask_paint` unit tests | 12 passed |
| UI library `uv_` test filter, with `uv-workbench` | 83 passed; 6 manual diagnostics ignored |
| All-target Clippy, `-D warnings`, UV/compose/GPU/engine/UI crates | Passed |
| Native UI check without the UV feature | Passed |
| UI check with UV for `wasm32-unknown-unknown` | Passed; six dead-code warnings in unchanged upstream files (five engine, one plugins) |
| `cargo xtask layers` | 32 crates, no violations |
| `cargo xtask scorecard --check` | Passed after regeneration for the added crate |
| Explicit rustfmt checks on changed Rust files | 113 files passed |
| Whitespace, presentation links and screenshot integrity | Passed; all four PNG SHA-256 hashes match the supplied originals |

The UI tests cover DX12/FXC startup, compositor fallback, original-resolution maps,
mask preview/commit, Fill transforms, image-shelf placement, mask-stack expansion,
painting/Undo, wireframe, navigation, workspace isolation and localization.
The existing offscreen mask-Levels/checker example also passed its interaction
assertions; its Properties render was inspected visually.

The upload preparation updates existing test calls for the UI context parameter,
uses the current U/V margin labels, and tests explicit mask-stack expansion and
collapse. Assertions remain enabled. Strict-lint cleanup names result/cache types,
groups viewport parameters and simplifies equivalent conditionals.

The full workspace/corpus sweep and performance benchmarks were not rerun for
this upload. These results do not claim that every workspace or integration test
is green.

## Merge status

The source tree now includes upstream `main` at
`7ac7b8b106acccf6826d45ed5c9e481dfe341035`; no unresolved merge entries remain.
The table above is historical v0.16.0 evidence. Current-main results and deferred
checks are listed below.

## 2026-10-27 current-main merge validation

Source is the finalized PR #8805 merge commit `2d7b67ed2f13b78de2b340b8289abd3ee55198b6`, which includes upstream main at `7ac7b8b106acccf6826d45ed5c9e481dfe341035`. This evidence is separate from the historical v0.16.0 table above.

Completed on the merged source:

| Check | Result |
|---|---|
| UV library suite, `cargo test -p photocraft-ui-egui --features uv-workbench -- --test-threads=4` with `CRAFT_FONTS_DIR` and `RUST_MIN_STACK=16777216` | 4,188 passed, 0 failed, 24 ignored (195.42 s) |
| UV UI integration suite, same command | 127 passed, 6 failed, 1 ignored (667.85 s); see failure details below |
| Default UI all-target check | Passed |
| Native application check, `cargo check -p photocraft --features uv-workbench` | Passed |
| Engine panic hunt, `cargo test -p photocraft-engine --test panic_hunt -- --ignored` | Passed, 1 test |
| `cargo xtask layers` | Passed, 32 crates |
| `cargo xtask wasm` | Passed, 25 crates plus `photocraft-codecs --features heif` |

Integration failures on the current merge are recorded without changing their assertions:

- `checker_canvas::cpu_pixel_grid_is_not_drawn_over_empty_checker`: empty checker pixel differed at zoom 8 (`[204,204,204,255]` vs `[255,255,255,255]`).
- `full_res_preview::adjustment_dialog_zoomed_out_skips_the_reduced_copy_when_low_res_is_off`: reports zoomed-out on reduced copy.
- `full_res_preview::filter_dialog_previews_at_full_resolution_when_low_res_is_off`: measured preview 128 vs 229.84.
- `full_res_preview::adjustment_layer_drag_previews_at_full_resolution_when_low_res_is_off`: expected 1 px columns at 100% not present.
- `live_stroke_canvas::the_canvas_shows_the_committed_stroke_while_it_is_drawn`: 81/255 canvas change at release in 4x flipped case.
- `pattern_preview_canvas::pattern_preview_repeats_the_document_on_gpu_and_cpu_canvases`: right-copy GPU sample was `[40,40,40]`.

The failing integration test files and `canvas/pixel_grid.rs` are identical to the upstream merge parent. UV overlay paths require workspace 3D and active Fill state; these tests do not set those conditions. This does not establish that the failures are pre-existing; default-feature exact reproductions are deferred.

With the UV feature enabled, the crop-ratio integration test overflowed its Windows test-thread stack at the standard 2 MiB Rust test-thread size. It passed at 16 MiB; the same default-feature exact passed at the standard size. This indicates higher stack use in that feature-enabled test path, but does not establish a production startup problem; a native `--help` smoke was deferred.

Detailed gate logs were kept outside the repository checkout and are not part of the PR. Default UI check, native application check, panic hunt, layer check and default wasm check passed. Initial strict Clippy and core test attempts found a missing `PixelFormat` test import and `type_complexity` in the touched GPU cache path; narrow fixes are present, but strict Clippy and core tests were not rerun. Corpus assets were fetched and verified, but the release corpus test build was stopped before tests ran. UV-feature wasm, exact default-feature reproductions, perf, and native `--help` checks were deferred at the user's direction to publish the resolved merge without extending validation further.

## 2026-10-27 refreshed upstream-main validation

The follow-up merge candidate includes upstream `main` at
`07bd794686ea4db0b557c978d58173a62760e4ca`. The only text conflict was the root
workspace manifest: the UV workspace dependencies were retained and the shared
`craft-libs` dependencies were advanced to upstream's `92d392cff132eaf53e13afff77f38b5c11b73aa0` pin.

Scoped checks on this candidate:

| Check | Result |
|---|---|
| `cargo check -p photocraft --features uv-workbench --locked` | Passed |
| `cargo check -p photocraft --locked` | Passed |
| UV container save/quit and workspace-scope regression | 1 passed |
| UV 2D channel selection and preference round-trip regression | 1 passed |
| Font-picker open/search focus regression | 1 passed |
| Trackpad popup layering and selection regression | 1 passed |
| `cargo clippy -p photocraft-ui-egui --all-targets --features uv-workbench -- -D warnings` | Passed |
| `cargo xtask layers` | Passed, 32 crates |
| `cargo xtask wasm` | Passed, 25 crates plus `photocraft-codecs --features heif` |

The updated upstream Clippy rule disallows `egui::ComboBox`; the two UV toolbar
channel selectors now use the workspace's pinned `craft_ui::ComboBox`. This
refresh does not rerun the full UI integration suite. The six pixel assertions
listed above remain deferred, and the prior results remain evidence only for
their recorded source revision.

## 2026-10-31 upstream-main validation (v0.18.0)

This follow-up worktree is based on upstream `main` at
`422ca7beda90d927643bcd074646ebbd37706d75`. These checks cover the UV brush-mode
menu integration and the focused merge-conflict/localization paths; they do not
replace the historical validation sections above.

| Check | Result |
|---|---|
| `cargo check -p photocraft --features uv-workbench --locked` | Passed |
| `cargo check -p photocraft --locked` | Passed |
| `cargo test -p photocraft-ui-egui --lib --features uv-workbench uv_ui::tests::shift_right_click_opens_brush_mode_menu_in_3d_and_2d_uv_views -- --exact` | Passed, 1 test |
| `cargo test -p photocraft-ui-egui --lib --features uv-workbench brush_mode_menu::tests::menu_is_owned_by_its_detached_viewport_and_dismisses_there -- --exact` | Passed, 1 test |
| `cargo test -p photocraft-ui-egui --lib i18n::tests::bundled_catalogs_are_consistent -- --exact` | Passed, 1 test |
| `cargo test -p photocraft-ui-egui --lib --no-run` | Passed (default-feature test compilation) |
| `cargo fmt --all -- --check` | Passed |
| `git diff --check` on the touched source, attribution and localization files | Passed |

The detached-viewport menu ownership and dismissal behavior was exercised with
egui's synthetic viewport test harness; a native multi-window application smoke
test and the full UI suite were not run for this follow-up. The default-feature
test binary compiled but was not executed as a suite.

## 2026-11-02 exact comparison of the six deferred UI failures

Reviewed working head: `ecfb9b83e` plus the workspace-close, Fill proxy, Crop tool-state and raster-budget corrections. Baseline: upstream merge base `fab871dadb85e7422d0b7c872497ff485ae92da9`, in a detached worktree. The four integration-test source files are byte-identical between baseline and head. No assertions were changed for this comparison.

All runs used the same Windows host, shared dependency versions, GPU, `RUST_MIN_STACK=16777216`, and `--test-threads=1`, with no `CRAFT_FONTS_DIR` override. Baseline has no `uv-workbench` feature; the valid comparison is baseline default versus head default, with head UV also checked. Commands: `cargo test --offline -p photocraft-ui-egui --test it -- <filter> --test-threads=1`, adding `--features uv-workbench` for the head UV run.

| Test/filter | Baseline default | Head default | Head UV |
|---|---|---|---|
| `checker_canvas::cpu_pixel_grid_is_not_drawn_over_empty_checker` | Failed: empty-checker pixel 252 vs 255 at zoom 16 | Failed: 255 vs 252 at zoom 8 | Failed: 255 vs 252 at zoom 16 |
| `full_res_preview::adjustment_dialog_zoomed_out_skips_the_reduced_copy_when_low_res_is_off` | Failed, same assertion | Failed | Failed |
| `full_res_preview::adjustment_layer_drag_previews_at_full_resolution_when_low_res_is_off` | Failed, same assertion | Failed | Failed |
| `full_res_preview::filter_dialog_previews_at_full_resolution_when_low_res_is_off` | Failed: 128 vs 229.84 | Failed: 128 vs 229.84 | Failed: 128 vs 229.84 |
| `live_stroke_canvas::the_canvas_shows_the_committed_stroke_part_0` | Initial full filter passed; isolated repeat passed once and failed once at 97/255 | Failed: 97/255; two isolated repeats also failed | Failed: 97/255 |
| `pattern_preview_canvas::pattern_preview_repeats_the_document_on_gpu_and_cpu_canvases` | Failed: right-copy GPU sample `[40,40,40]` | Failed: same sample | Failed: same sample |

The former live-stroke test is now split into four parts; parts 1–3 passed in all three configurations. The shared part-0 failure uses zoom 12, smoothing 0.5, 16-bit depth, pressure and flipped canvas. A single passing baseline run was therefore insufficient: repeated baseline execution reproduced the exact head assertion and 97/255 difference.

These results establish baseline reproductions of all six previously deferred failures on this host, including an unstable live-stroke case. They do not make the tests green or establish correctness on every renderer. Their existing assertions remain in place; fixing these shared canvas/display issues is separate from the UV-feature changes. Detailed logs are in `D:/PhotoCraft-UV-build/Temp/{baseline,head-default,head-uv,repeat-baseline,repeat-head}-*.txt` locally and are not committed.

## 2026-11-02 PR #8805 CI and import-budget follow-up

These checks cover the final CI lint cleanup and bounded automatic UV texture import. They are scoped results, not a claim that the full workspace or corpus suite is green.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed after the import-budget changes |
| `cargo xtask layers` | Passed, 32 crates |
| `cargo xtask wasm` | Passed, 25 crates and `photocraft-codecs --features heif` |
| `cargo clippy --workspace --all-targets --features heif -- -D warnings` | Passed on the CI-fix source before the final UV import-budget changes |
| `cargo clippy -p photocraft-gpu --all-targets -- -D warnings` | Passed |
| `cargo test -p photocraft-compose --lib --tests` | 440 passed, 2 ignored; 3 artboard integration tests passed |
| `cargo test -p photocraft-engine --lib cmd::brush_cmds::tests` | 77 passed, 2 ignored |
| `cargo test -p photocraft-engine --features uv-workbench uv_cmds::import_budget_tests --lib --locked -j 6` | 6 passed, including duplicate-channel and 8K-budget boundaries |
| GPU mask-stack integration tests | 13 passed on Vulkan with an RTX 5080 adapter |
| Offscreen UI snapshot | Passed after rebuilding current sources; 256×256 white canvas visible, Eyedropper selected, and panels intact |

The `uv-workbench` engine Clippy run still reports cognitive complexity in the existing `crates/engine/src/uv_mask_paint_cmds.rs:357`; the default workspace strict-Clippy run above passed before the final UV import-budget edits.

`cargo xtask test-corpus` did not pass. Its release `photocraft-io --lib` test executable aborted with Windows `STATUS_STACK_OVERFLOW` in `svg::tests::bounded_embedded_svg_gets_only_the_nesting_depth_the_drawing_left`. A direct release-binary isolation reproduced that failure. The no-HEIF integration binary also aborts in `svg::an_embedded_svg_gets_only_the_nesting_depth_the_drawing_left` on its explicit 1 MiB worker stack, even with `RUST_MIN_STACK=16777216` (the test sets its own stack size). The failing SVG source and test files are byte-identical to `fab871d`; the baseline binary was not rerun, so this is not recorded as a runtime baseline reproduction. The pinned `usvg` 0.45.1 converter recursively visits nested groups, which is consistent with the failure at the current 512-element XML-depth limit. No SVG source or assertion was changed for this follow-up.

The fresh snapshot was rendered with `cargo run --offline -p photocraft-ui-egui --example snapshot --locked -- --out D:/PhotoCraft-UV-build/Temp/ci-canvas-smoke.png --size 1100x700 --scale 1 --script '[["engine.execute",{"command":"file.new","params":{"width":256,"height":256,"background":"white"}}],["ui.set",{"tool":"eyedropper"}]]'`. The image was inspected visually. Validation logs and the snapshot remain under `D:/PhotoCraft-UV-build/Temp/` and are not committed.

## 2026-11-03 PR #8805 review follow-up on updated main

The PR branch includes upstream main at `c6af3dcb98b17ceeb38dbf9fd200e6a684a088f1` through merge commit `0a454c9f`.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` and `git diff --check` | Passed after the final local changes |
| `cargo xtask layers` | Passed, 32 crates |
| `cargo xtask wasm` | Passed, 25 crates plus `photocraft-codecs --features heif`; existing unused-code warnings remain |
| `cargo clippy --workspace --all-targets --locked --features heif -- -D warnings` | Passed |
| GPU readback boundary tests | 3 passed; real GPU brush readback integration passed; strict GPU Clippy passed |
| `cargo test -p photocraft-engine --features uv-workbench --lib uv_cmds::import_budget_tests --locked -j6` | 16 passed, 0 failed, including source growth and embedded-cache regressions |
| Focused UI brush-preview test | 1 passed, 0 failed, including the updated-main `Buffer::into_px` compatibility changes |

The separate UV-feature strict Clippy checks remain blocked by existing diagnostics outside these review fixes: engine cognitive complexity at `crates/engine/src/uv_mask_paint_cmds.rs:357` (38/30), plus existing UV/UI lint findings. The workspace strict-Clippy command above passes for the default feature set with HEIF.

This follow-up did not rerun `cargo xtask test-corpus`; the 2026-11-02 SVG stack-overflow result and its scope limitation remain as recorded above.
