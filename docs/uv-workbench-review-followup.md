# UV workbench review follow-up

This note tracks the finalized CodeRabbit review findings for PR #8805 and the
focused validation run for their fixes. It complements
[`uv-workbench-validation.md`](uv-workbench-validation.md), which records the
broader merge validation and its previously accepted pixel-test failures.
The review is available at [PR #8805](https://github.com/storytold/photocraft/pull/8805).

## Review findings

| # | Finding | Resolution |
|---|---|---|
| 1 | Unbounded mask refresh could enumerate oversized or invalid persisted Fill bounds. | Bound unscoped Fill expansion by checked area and tile budgets; canvas consumers refresh only the canvas; reject malformed bounds during load. |
| 2 | Cache-only mask edits could diverge from the editable mask-paint stack. | Validate edits on a clone and reject cache-only stack mutations atomically; reject unsupported stack transforms before mutation. |
| 3 | Removing a stack layer below the active layer left the active index pointing at the wrong layer. | Adjust the active index when deleting a preceding layer and cover it with an engine regression. |
| 4 | A 64-megapixel Fill could allocate multiple full-resolution RGBA buffers. | Keep the supported size while rendering in bounded horizontal slabs and writing each completed region. |
| 5 | The live-fill snapshot example assumed a developer-specific drive path. | Accept an explicit path and use a portable relative default. |
| 6 | The mask-paint snapshot example assumed developer-specific input/output paths. | Accept explicit paths and use portable relative defaults. |
| 7 | The standalone UV renderer did not observe GPU device loss. | Attach a device-health watcher when no shared application GPU health handle is available. |
| 8 | Deferred GPU redraws could drop materialized CPU mask edits or repeat full document cloning. | Preserve materialized masks in deferred work; materialize only when a CPU fallback is actually required and use canvas-bounded refresh. |
| 9 | Paint picking ignored sticky modifier state. | Include sticky modifiers in the picker-suppression gesture check. |
| 10 | A stale opacity preview error prevented recovery writes for other dirty documents. | Retain the preview error while continuing canonical recovery saves for other dirty documents. |
| 11 | Alt-clicking a Gray mask view could immediately re-enable the view being toggled off. | Match the active Gray, unfiltered view to the exact layer before suppressing re-enable; add a layer-identity regression. |
| 12 | Control-right brush resizing leaked from the UV workspace into ordinary 2D painting. | Scope the normalized Control-right gesture to UV workspace; retain Alt-right behavior elsewhere. |
| 13 | Transient fit/fill pending flags made a clean UV project appear dirty. | Ignore those transient flags only in the dirty-state comparison; retain them in serialization and save/restore. |
| 14 | Validation documentation described intermediate merge state and linked local logs. | Align the validation note with the finalized merge state and portable references. |
| 15 | Invalid geometry construction permanently discarded the UV renderer. | Keep the existing renderer available so a later valid model can recover. |
| 16 | Several UV workspace labels were hard-coded and missing Russian localization. | Route the labels through translation keys and add Russian catalog entries. |
| 17 | Transform rejection included unlinked descendant masks that a group pixel transform leaves fixed. | Reject editable mask-stack transforms only when the chosen target actually moves those masks; cover unlinked group success and linked group atomic rejection. |
| 18 | A synchronous CPU fallback materialized every editable mask stack even for a small damage rect. | Materialize only damage plus each pixel mask's feather support for rect rendering; retain complete snapshots for full and deferred redraws. |
| 19 | Transforming a linked group mask also transforms its linked descendant masks, but the mask-target preflight checked only the group's own Paint stack. | Before starting the warp, reject when the selected group mask is linked and any linked descendant mask has an editable Paint stack; keep unlinked group-mask transforms allowed. |

## Focused validation

The focused checks passed:

- `cargo test -p photocraft-engine --features uv-workbench --test uv_mask_paint` — 8 passed.
- `cargo test -p photocraft-compose --lib unbounded_refresh_skips_pathological_persisted_fill_bounds` — 1 passed.
- `cargo test -p photocraft-format loader_rejects_inverted_fill_and_bounds_on_non_fill_layers` — 1 passed.
- `cargo test -p photocraft-ui-egui --features uv-workbench stale_opacity_preview_fails_recovery_without_losing_pending_value` — 1 passed.
- `cargo test -p photocraft-ui-egui --features uv-workbench uv_container_save_quit_and_workspace_scope_preserve_normal_canvas` — 1 passed.
- `cargo test -p photocraft-ui-egui --features uv-workbench alt_mask_view_toggle_only_suppresses_reenable_for_the_same_gray_layer_mask` — 1 passed.
- `cargo test -p photocraft-ui-egui --features uv-workbench mac_control_right_brush_resize_is_scoped_to_the_uv_workspace` — 1 passed.
- `cargo clippy -p photocraft-compose -p photocraft-engine -p photocraft-format -p photocraft-ui-egui --all-targets --features photocraft-ui-egui/uv-workbench -- -D warnings` — passed.
- `cargo check -p photocraft-ui-egui --all-targets` — passed.
- `cargo test -p photocraft-engine --test panic_hunt -- --ignored` — adversarial command panic hunt passed (1 test, 15.36 s).
- `cargo xtask layers` — passed, 32 crates with no layering violations.
- `cargo xtask wasm` — passed for 25 crates and the codecs `heif` feature check.

These focused checks do not replace the broader validation documented in
[`uv-workbench-validation.md`](uv-workbench-validation.md). The six previously
recorded pixel assertions remain deferred.

The #19 linked group-mask regression passed:

- `cargo test -p photocraft-engine --features uv-workbench --lib targeted_group_mask_rejects_linked_descendant_paint_masks_before_transform` — 1 passed; covers a linked group mask with linked descendant Paint (specific early Free Transform error and atomic document), an unlinked group mask with linked descendant Paint (allowed), and a linked group mask with an unlinked descendant Paint (allowed).
- `cargo clippy -p photocraft-engine --all-targets --features photocraft-engine/uv-workbench -- -D warnings` — passed.
- `cargo test -p photocraft-engine --test panic_hunt -- --ignored` — adversarial command panic hunt passed (1 test, 14.78 s).
- `cargo xtask layers` — passed, 32 crates with no layering violations.
- `cargo xtask wasm` — passed for 25 crates and `photocraft-codecs --features heif`.

## 2026-10-27 latest review follow-up

The latest two CodeRabbit findings are addressed in the working tree. Focused
regressions passed:

- `cargo test -p photocraft-engine --features uv-workbench --lib group_transform_allows_unlinked_descendant_paint_masks_and_rejects_linked_ones_atomically` — 1 passed.
- `cargo test -p photocraft-ui-egui --features uv-workbench --lib partial_cpu_materialization_matches_full_mask_values_with_feather_and_stays_damage_bounded` — 1 passed; compares the rendered rect with a full snapshot and verifies a tiny-damage snapshot on a 100,000 × 100,000 canvas touches at most one mask tile.

Additional scoped gates passed:

- `cargo clippy -p photocraft-engine -p photocraft-compose -p photocraft-ui-egui --all-targets --features photocraft-ui-egui/uv-workbench -- -D warnings` — passed.
- `cargo check -p photocraft-ui-egui --all-targets` — passed.
- `cargo xtask layers` — passed, 32 crates with no violations.
- `cargo xtask wasm` — passed for 25 crates and `photocraft-codecs --features heif`.

## 2026-10-27 refreshed upstream-main check

The PR branch was refreshed to upstream `main` at
`07bd794686ea4db0b557c978d58173a62760e4ca`; the shared `craft-libs` pin is now
`92d392cff132eaf53e13afff77f38b5c11b73aa0`. The upstream Clippy policy also
replaces the two UV toolbar `egui::ComboBox` calls with `craft_ui::ComboBox`.
The UV-feature native check, default native check, UV save/workspace regression,
font-picker regression, popup layering regression, strict UI Clippy, layering,
and wasm checks passed. Exact commands and the scope of this validation are
recorded in [`uv-workbench-validation.md`](uv-workbench-validation.md).
