# Sparse brush path reconstruction

> Integration notes updated 2026-11-06. Original investigation and implementation: 2026-10-25–26.

Fast freehand curves could become a chain of straight chords in PhotoCraft. The investigation
recorded in “Compare Krita and Photocraft brushes” found that position smoothing and dense dab
spacing did not reconstruct the missing curved geometry. Krita's freehand path reconstruction
and macOS input handling informed the architectural comparison; this change is an original Rust
implementation, with no Krita code or brush assets imported.

The original comparison distinguished Krita's Basic/Weighted curve paths from its No Smoothing
polyline path. PhotoCraft already consumed every pointer move delivered within a frame; simply
adding more dabs or raising smoothing could not recover geometry or events discarded earlier.
The native coalescing change was an input-preservation investigation target, not a measured
sample-rate or latency improvement. Position and sensor timing were still separate streams.

## Original contribution and discussion

[PR #9587](https://github.com/storytold/photocraft/pull/9587) contains the original implementation
(`3ccc7a5d4`) and Mixer Brush compatibility follow-up (`f1b5abbe3`). It was still open when checked
on 2026-11-06. The original branch is `fatjester:codex/smooth-fast-brush-curves`.

The [issue-comment index](https://github.com/storytold/photocraft/pull/9587#issuecomment-6419323016)
records the reporter follow-ups posted on October 26:

- [#2138](https://github.com/storytold/photocraft/issues/2138) is the interpolation tracker;
  [#9187](https://github.com/storytold/photocraft/issues/9187) describes the Windows mouse case.
- [#4731](https://github.com/storytold/photocraft/issues/4731),
  [#5040](https://github.com/storytold/photocraft/issues/5040),
  [#6237](https://github.com/storytold/photocraft/issues/6237) and
  [#316](https://github.com/storytold/photocraft/issues/316) contain related fast-curve reports.
  Duplicate closure does not demonstrate that the painting behavior is fixed.
- [#7993](https://github.com/storytold/photocraft/issues/7993) overlaps through a report of
  straight segments at 1% spacing; disconnected dabs and pressure concerns are broader.
- [#2057](https://github.com/storytold/photocraft/issues/2057) concerns lag. Curve reconstruction
  does not establish an input-to-display latency fix or resolve its Eyedropper/Pen follow-up.
- [PR #317](https://github.com/storytold/photocraft/pull/317) supplies earlier input-event recovery.
  [PR #6131](https://github.com/storytold/photocraft/pull/6131) is parallel interpolation work;
  check overlap before proposing another upstream contribution.

## Pipeline and invariants

The regular painting pipeline is now:

`pointer samples → position smoothing → cubic reconstruction → distance/time spacing → dabs → coverage → pixels`

- [`StrokeInterpolator`](../crates/paint/src/interpolate.rs) uses one sample of lookahead,
  adaptive subdivision at 0.125 document-pixel flatness, maximum depth 10, and a 16,384-pixel
  span guard. Reconstruction runs with smoothing at 0% too.
- The latest segment remains provisional. Finishing a cloned generator renders the live tail;
  the next sample resolves that segment, and release commits it once. Batch boundaries,
  preview requests, seeded jitter and dual brushes must not change the final dabs.
- Taps, straight lines, reversals, turns of 90 degrees or sharper and aliased Pencil geometry
  are preserved. Pressure, tilt, wheel and time interpolate with the path; barrel rotation
  follows its shorter arc.
- Dense retouch previews combine provisional coverage with older coverage in tiles absent
  from the tail. Treating those holes as empty erased previous Clone Stamp coverage.
- Sequential effects (Mixer Brush, Smudge, Dodge/Burn and other `LiveDab` tools) explicitly
  use `DabGenerator::new_polyline` for both live and replay paths. Curving them requires
  replaceable effect-state snapshots as well as replaceable pixels.
- The macOS monitor disables AppKit mouse coalescing while monitoring. A shared lease restores
  the original setting only after the last monitor drops, regardless of drop order. Other
  platforms receive the shared Rust geometry change, without this AppKit-specific adjustment.

## November 6 base refresh

The local `main` was fast-forwarded to upstream `b49c6c97c` (PhotoCraft 0.20.1).
The two original commits were reapplied on `codex/brush-update-20261106`. The original PR branch
was retained. Integration adjustments preserve upstream changes:

- Engine modules moved beneath `crates/engine/src/cmd/`; retouch logic is now split between
  `retouch_cmds/clone.rs` and `retouch_cmds/dab.rs`.
- `LiveDab` now uses its own generator instead of recording an accumulating renderer. It and
  its test reference explicitly select the sequential path, as does Mixer Brush.
- Coverage tiles now use packed keys. The dense tail lookup uses `tile_key` accordingly,
  retaining upstream brush rasterization, parallelism and mipmap-cache improvements.
- The brush/gap documents retain the newer upstream entries and their existing estimates.
- The [overlapping-monitor review finding](https://github.com/storytold/photocraft/pull/9587#discussion_r4373557106)
  was reproduced with a native regression, then addressed with shared coalescing ownership.
  The regression checks both drop orders, both original settings and continued event delivery.

## Validation and remaining work

The original October 25 comparison used the same seven samples on a radius-140 px arc at 0%
smoothing: maximum radial dab error fell from 4.7704 px to 0.1522 px. Three alternating release
benchmark runs on a 6016×6016 image with 200 px brushes showed median throughput changes from
−0.9% to +1.9% across RGBA8/16/32F and CMYKA8. These are historical measurements, not results
from the November base refresh and not latency measurements.

The original rebased PR recorded 6,855 passing tests, 27 missing-font UI failures, and unrelated
strict-lint blockers. Those counts describe the October base only.

Validation on the November 6 base (Rust 1.95, without the optional craft-fonts build input):

| Check | Result |
|---|---|
| Paint | 163 tests passed |
| Tablet | 16 unit tests and the native monitor lifecycle regression passed; the new overlap regression failed before the lease fix |
| Engine | 3,806 unit/integration tests passed |
| Interface library | 5,301 passed, 23 failed, 22 ignored; all 20 canvas stroke tests passed |
| Adversarial commands | The freshly built `panic_hunt` executable passed `no_command_panics_or_hangs_on_adversarial_params` with `--ignored` |
| Dependency layers | 31 crates, no violations |
| WebAssembly | 24 crates and optional codec HEIF support passed |
| Strict paint/tablet Clippy | Passed, all targets |
| Quick performance harness | Passed, 85 scenarios; contended run, no matching machine baseline, full-size budgets not applied |
| Formatting and diff whitespace | Passed |
| Broader strict Clippy | Blocked by unchanged upstream lints described below; all four changed packages pass all-target checks with `--no-deps` and those three lint categories allowed |

There were 9,288 distinct passing tests across those test runs. The interface failures comprise
19 missing-glyph panics, three Type-tool font-selection mismatches, and one recent-thumbnail
cache-interval assertion. The three font-selection mismatches and thumbnail assertion repeat
when run in isolation. These failures are in unchanged components; they are not claimed fixed.
The combined test command stopped after the failing interface unit binary, so interface
integration binaries were not run in that command. Native Windows/Linux testing and a new
packaged desktop application are outside this source refresh.

Unrestricted strict Clippy stops on `psd/src/codepage.rs:243` (`nonminimal_bool`). Checking the
selected packages with `--no-deps` also exposes existing `collapsible_match` warnings in
`engine/src/cmd/atn_cmds/steps_layer_style.rs:90` and `engine/src/cmd.rs:323`,
`manual_range_contains` in `ui-egui/src/canvas/tool_cursor/pen.rs:95`, and
`nonminimal_bool` in `ui-egui/src/menus/menu_keys.rs:376` and `menu_nav.rs:347`.
The final affected-package check passed with only `collapsible_match`, `nonminimal_bool` and
`manual_range_contains` allowed; no source-level lint suppressions were added.

The quick performance harness completed successfully and wrote `target/perf/results.json`
and `target/perf/summary.md`. Its high-load marker and different baseline machine class mean
these timings are smoke-check evidence only, not a before/after speed claim or proof that
full-size performance budgets are met. A fresh current-renderer arc/S-curve comparison was also
rendered and visually checked: the same sparse input follows curves with reconstruction and
straight chords with the explicit polyline generator, both at 0% smoothing.

Current regression locations:

- [`interpolation_tests.rs`](../crates/paint/src/interpolation_tests.rs): sparse arcs, corners,
  sensors, batching, dual brushes, all three depths, dense tail holes and sequential paths.
- [`monitor_macos.rs`](../crates/tablet/tests/monitor_macos.rs): native events and monitor lifecycle.
- [`mixer_live_tests.rs`](../crates/engine/src/cmd/brush_cmds/mixer_live_tests.rs) and
  [`retouch_cmds/tests.rs`](../crates/engine/src/cmd/retouch_cmds/tests.rs): live/reference/replay.
- [`stroke_timing_tests.rs`](../crates/ui-egui/src/stroke_timing_tests.rs): actual canvas input,
  sparse curves and preview/commit behavior.

Still open: real-tablet and Photoshop comparisons; native Windows/Linux verification; paired
native position/timestamp capture; replaceable sequential-effect state; large-brush responsiveness.
The original follow-up priorities were to measure native samples against delivered stroke points,
then consider an ordered queue carrying position, time, pressure, tilt, rotation and contact state
together. If current profiling still shows synchronous painting blocking input, evaluate a paint
worker consuming that queue. Recheck the newer upstream parallel rasterizer before deciding on
that scheduling work; improved responsiveness and reconstructed geometry are separate concerns.
This work does not claim full Photoshop or Krita brush parity. See [brush parity](brush-parity.md)
and [gap G4](gaps.md#g4-brush-feel) for the broader scope.
