# Remove Subject

> **Last reviewed:** 2026-10-21 · **Last updated:** 2026-10-21 · **Change:** major (new engine command)

`edit.removeSubject` (Edit › Remove Subject) finds the subject and fills its place from the surrounding image. It runs as a cancellable edit with progress and one undo step.

```json
{"layer": 2, "sampleAllLayers": false, "expand": 8, "keepMask": false, "fill": "auto", "newLayer": false, "engine": "auto"}
```

- `layer` is an optional layer id or name; the active layer is the default. The target must be an unlocked pixel layer.
- `sampleAllLayers` samples the visible composite for both subject detection and filling. Otherwise it samples the target's own pixels.
- `expand` grows the subject mask by 0–200 pixels (default 8) before filling, to include its edge and halo.
- `keepMask` keeps the expanded removal mask, intersected with the previous selection, as the document selection. Otherwise the previous selection is preserved, including the absence of a selection.
- `fill` is `auto`, `lama`, or `patch`. `auto` tries the locally installed LaMa model, falling back to patch completion when the model is missing or unsupported. `lama` requires the model and reports an error without it. Other inference failures are reported; cancellation never starts a fallback fill.
- `newLayer` puts the fill on a transparent pixel layer immediately above the target and makes it active. The target stays untouched. Otherwise the fill changes the target's pixels.
- `engine` is `auto`, `model`, or `classical`. `auto` uses the local subject model when it is installed and the classical subject finder otherwise; `model` requires the model. `classical` bypasses inference.

The existing selection limits the fill, including fractional coverage at feathered edges. Pixels outside the removal mask remain unchanged. Filled pixels are opaque at full coverage; a new output layer is transparent elsewhere. The native patch path retains the document's channel model and depth; model input/output uses the existing RGB conversion and surface APIs.

The result contains `layer`, `bounds` (the filled mask's `[x, y, width, height]` in document pixels), `subjectFound`, `filledWith` (`lama` or `patch`), and `subjectEngine` (`model` or `classical`). No subject, an empty intersection with the selection, or no surrounding source pixels produces an error without an edit.

The subject comes from `smartselect_cmds::find_subject_region`, the same helper Select Subject and Remove Background use.

Dense input is limited to 64 Mi pixels, each side at most 32,768 pixels; the fill window is limited to 16 Mi pixels. Each limit is checked before allocating or sampling its corresponding dense buffer. An existing layer's transparency lock is respected.

## Validation

Tests were written before implementation; the first build failed on the missing `fill_window` implementation.

| Gate | Result |
|---|---|
| `cargo test -p photocraft-engine remove_subject` | 14 passed, covering 8/16/32-bit removal, undo/redo, selection limits and feathering, mask retention, output layers, composite sampling, cancellation, invalid params, size limits, expansion, background jobs, and injected model results/errors |
| `cargo test -p photocraft-engine` | 2,073 passed; 19 ignored |
| `cargo clippy -p photocraft-engine --all-targets -- -D warnings` | Passed after the ML owner fixed dependency lints |
| `cargo test -p photocraft-engine --test panic_hunt -- --ignored` | Passed; the test discovers this command through the registry |
| `cargo xtask layers` | Passed: 32 crates, no violations |
| `cargo xtask wasm` | Passed: 25 crates and the HEIF codec feature check |
| `cargo xtask parity` | Regenerated `docs/parity-checklist.md`: 632/632 live, Edit 76/76 |
| `cargo xtask scorecard` | Regenerated `docs/scorecard.md` to reflect the menu count |
| Formatting | New module passed rustfmt checking |
| Offscreen UI | Inspected a 1440×900 PNG: Remove Subject is enabled for an unlocked pixel layer, directly below Content-Aware Fill |

The full suite and panic hunt used an empty `PHOTOCRAFT_MODELS_DIR` to exercise the CI fallback path. An earlier full-suite run began expensive inference against installed weights and was stopped before completion. Model outputs are covered with injected test stubs; real-model removal quality remains unverified.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-21 | major | Remove Subject command contract and implementation notes |
