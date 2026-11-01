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
