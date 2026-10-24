# Levels/Curves grayscale target follow-up

Completed 2026-10-17 against upstream main `2a952f164c0b4cf936b1c6497db10dd095341c00`. Follow-up to PR #5448 for issue #5335.

Levels now lists only the selected target by its actual name, using literal custom names and translated built-in names like Curves. Its controls and Auto use the matching parameter key and histogram for individual RGB/CMYK/Lab components. It uses grayscale controls and the edited plane's histogram for raster masks, alpha/spot channels and Quick Mask in every document colour mode. Curves/Levels Action dialogs resolve their recorded explicit target before choosing defaults. Their channel context survives Reset, pencil editing and sampling. Grayscale black/white eyedroppers sample the committed plane directly at native depth, including default samples, and update the visible neutral curve. Explicit scalar Action previews use the same CPU preview path as a normally selected scalar channel.

## Validation (2026-10-17)

- Full engine/UI `cargo test`: **4,988 passed, 0 failed, 32 normally ignored**, including doctest builds and all new tests.
- The ignored full-registry `panic_hunt` test passed separately.
- Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and `cargo xtask wasm` passed. Wasm reports three pre-existing cfg-specific unused-constant warnings (`PAR_PIXELS`, `MAX_SUBFOLDER_DEPTH`, `MAX_SUBFOLDER_FILES`); native strict Clippy is clean.
- `cargo xtask perf --quick` passed with `failures: []`. Quick mode uses small documents and does not apply full-size budgets. Its repository-baseline comparison was skipped because that baseline is from an M4 Pro; the separate comparison below uses the same machine for both builds.
- Full-app offscreen before/after screenshots were inspected for Lab Levels gamma, a Lab alpha white-point picker with a real canvas click, an alpha-targeted Action played while Red is selected, and Levels on a selected Green component. Updated candidate captures show `Saved sky` and `Green` in the Levels dropdown.
- Multiple local review/fix passes completed; the final review found no further actionable functional bugs within the reviewed scope.

The initial 18 regression functions exercise RGB, CMYK, Lab and grayscale documents across native 8/16/32-bit samples; real Levels field editing; exact target names, literal spot names, translated built-in names, every RGB/CMYK/Lab component, Levels Auto histogram routing; black/white pickers on masks, alpha/spot and Quick Mask; serialized Action defaults and explicit target precedence; reset, pencil and sampling context; explicit pixel targets; preview/commit agreement; native-depth edge averages and unstored mask defaults; feathered selections; preservation of colour pixels, transparency and other planes; undo/redo, cancel and malformed inputs.

## Review follow-up (2026-10-20)

The destructive picker now passes the actual Levels/Curves command name to target injection. Added one permanent regression (19 regression functions total in this PR) for a sparse mask smaller than the canvas, shifted through actual linked/unlinked layer-mask moves. It checks 1,152 sampling cases across RGB/Gray/Lab/CMYK, U8/U16/F32, normal and partly off-canvas positions, point and 3×3 averages, stored pixels and unstored defaults. Implicit and explicit mask targets must produce the same adjustment; expected native-depth averages are calculated independently. No sampling algorithm changed.

Repeated on Linux: full engine tests **1,906 passed, 0 failed, 19 normally ignored**; all 6 targeted picker tests passed; strict engine all-target Clippy, formatting, layering and wasm passed. The ignored full-registry adversarial test passed separately. The earlier engine/UI result above remains dated to 2026-10-17; the UI was unchanged by this follow-up. No new UI-latency or performance measurement is claimed for this routing-name cleanup.

## CodeRabbit and local review follow-up (2026-10-23)

Ctrl/Cmd-click point insertion now uses the shared context-preservation helper, like Reset and sampling. The existing explicit-target regression already passed before this cleanup: public target parameters were restored from the parameter map. Added documentation for introduced helpers, regressions and the benchmark in response to CodeRabbit's documentation-coverage warning.

A further review found that reopening a whole-image Levels/Curves Smart Filter could inherit the current channel's isolation, or a Layers-panel mask target and grayscale defaults. Reopening now resolves the captured Smart Filter before seeding its colour controls. A new regression covers RGB/Lab/CMYK, both tone dialogs, and five current targets (alpha, Channels-panel mask, individual component, Layers-panel mask and Quick Mask): 30 reopening/preview/commit cases preserving recorded non-neutral curves, masks and saved channels. Action private-field sanitization also checks that a recorded step cannot forge the Smart Filter marker.

Validation of `7b89bc637`: full engine/UI `cargo test` **6,221 passed, 0 failed, 36 normally ignored**; the ignored full-registry adversarial command test passed separately. Strict engine/UI all-target Clippy, formatting, layering and wasm passed. Offscreen Smart Filter captures are retained with the review logs. The last general quick-perf attempt (2026-10-22) stopped at an upstream `layout_bench` fixture that converts already imported Smart Objects a second time; four earlier suites passed and the layout benchmark's `--direct` path completed. This dialog-setup follow-up does not change the historical CPU benchmark workload below.

## Further parameter review (2026-10-23)

CodeRabbit's next review identified retained non-neutral RGB channel records in scalar parameters. The shared gray-target filter now drops RGB as well as CMYK/Lab component records. A regression first failed on the previous implementation, then passed across 432 cases: RGB/Gray/Lab/CMYK, U8/U16/F32, masks/alpha/spot/Quick Mask, Levels/point Curves/pencil Curves, direct/sampled edits and implicit/explicit targets. Whole target surfaces match an independent clean-parameter command; colour pixels, other planes and spot metadata remain unchanged. Action tests now carry non-neutral RGB records and check neutral hidden controls for scalar targets and preserved records for pixel targets.

A further local review reproduced a recorded `gray` alias seeding bug: Levels could display Gamma 1 for recorded Gamma 2 and let the hidden alias override later edits, while default Curves points could conceal the recorded curve. Scalar dialogs now resolve valid aliases through the checked parser/serializer before merging defaults. Invalid aliases remain preview/commit errors and leave document data unchanged. Offscreen Lab alpha Action captures show the recorded Gamma 2 and inverted curve under `Saved sky` while Lightness is selected.

Validation of this follow-up: full engine/UI `cargo test` **6,224 passed, 0 failed, 36 normally ignored**; the ignored adversarial command test passed separately. Strict engine/UI all-target Clippy, formatting, layering and wasm passed. A third local review found no further actionable issues in scope. No proxy or pixel-processing algorithm changed; no fresh timing comparison is claimed for parameter filtering and dialog initialization.

## Main integration and review (2026-10-24)

Current main integrated: `1c3db2e5c` (PhotoCraft 0.15.1). The overlap with Alt+adjustment last-used settings exposed additional target/context issues. Recall now keeps the current mask target, drops hidden colour records for scalar planes and grayscale layer pixels, and falls back to neutral for an incompatible tone space. Saved preferences retain settings without document/layer/channel addresses. Compatible colour-channel settings remain intact. The Levels localization test now derives the built-in label from the current translator, as upstream's Curves test does, while preserving literal custom-name assertions.

Three new regression functions first reproduced the problems and then passed after fixes. They cover 44 recall scenarios across RGB/Gray/CMYK/Lab, Levels/Curves, alpha/mask/Quick Mask and grayscale pixels, compatible/incompatible colour settings, malformed preferences, commit/cancel and preservation of other pixels. This PR now introduces 27 regression functions plus expanded Action metadata checks. Three local review/fix passes completed; the latest found no further actionable bugs within the reviewed scope.

Validation on Linux with `PHOTOCRAFT_FULL_SWEEP=1`:

- All engine/UI targets in the workspace sweep passed: **6,420 passed, 0 failed, 38 normally ignored** (engine 2,472; UI 3,948), including doctest builds. The 24 tone-target integration regressions passed separately too.
- The ignored full-registry adversarial command test passed separately. Strict engine/UI all-target Clippy (`-D warnings`), formatting, layering and wasm passed. `cargo xtask test-corpus --changed` correctly skipped: this PR changes none of the corpus-critical paths relative to the integrated main.
- Fresh offscreen Action and Alt+shortcut captures were inspected. Recalling a Lab colour dialog's settings on `Saved sky` shows Gamma 2 or the inverted neutral curve, with the expected grayscale preview; the resized Curves controls remain inside the dialog.
- **The unfiltered workspace run is not fully green on this machine.** Two unchanged GPU tests comparing measured execution time with wall-clock waiting time failed on RTX 3090/Vulkan: `a_refresh_that_submits_several_times_is_one_measurement` and, on a later run, `gpu_time_tracks_execution_not_submission`. The first also failed in three isolated runs; the second passed in the initial run. The exact GPU integration binary passed **89 tests** with only those two timing assertions excluded. GPU code and its lower-layer dependencies match upstream main.
- The remaining workspace sweep (`cargo test --workspace --exclude photocraft-gpu --no-fail-fast`) completed with **11,353 passed, 1 failed, 91 ignored**. The sole failure is the unchanged IPP test `error_statuses_and_broken_replies_are_errors`: this environment resolves `printer.invalid` and returns HTTP 400, while the test expects an unresolvable host. No printer or GPU code was altered to conceal these failures.
- `cargo xtask perf --quick` completed five suites (`perf_scenarios`, `bevel_bench`, `camera_raw_bench`, `type_bench`, `bench_filters2`) and failed only in the unchanged `layout_bench` fixture at line 175, which converts already imported Smart Objects a second time. The repository-baseline comparison was skipped because it belongs to a different machine. These results are not a fresh before/after latency comparison; the historical 24 MP measurements below remain explicitly dated.

Logs, failure reproductions, review notes and current captures are retained locally in `plan/merge-main-20261024/` (gitignored). Windows and macOS were not tested locally.

## 24 MP release comparison

Dense synthetic 6000 × 4000 RGB8 raster layer, raster mask, saved alpha channel and Quick Mask, each filled with 25% gray. The central selection is a 3000 × 2000 rectangle at 0.5 coverage. Proxy factor 4 (1500 × 1000). AMD Ryzen 7 9700X, 16 logical CPUs, Ubuntu 26.04.1, Rust 1.99.0, default optimized release profile (thin LTO).

Identical benchmark source at main `2a952f164c0b4cf936b1c6497db10dd095341c00` and the candidate. Three process runs per build, nine warmed samples per case per run: medians of 27 samples. Order: main/fix/fix/main/main/fix, with no competing compiler, test or screenshot workloads. Cold samples are retained separately and excluded. The RGB and grayscale values match in this fixture, so the white-point row produces equivalent edits in both builds.

Timing includes CPU proxy construction, command execution and flattening. It excludes histogram/dialog setup, GPU upload, UI drawing, worker scheduling and pointer-to-display latency. It measures the CPU fallback workload rather than the composite adjustment-layer GPU fast path.

| Target | Selection | Main (ms) | Fix (ms) | Change |
|---|---|---:|---:|---:|
| Composite | None | 39.92 | 37.35 | -6.4% |
| Composite | Central 50% coverage | 40.43 | 39.76 | -1.7% |
| Alpha | None | 75.95 | 72.82 | -4.1% |
| Alpha | Central 50% coverage | 41.95 | 40.38 | -3.7% |
| Raster mask | None | 71.89 | 70.36 | -2.1% |
| Raster mask | Central 50% coverage | 41.01 | 38.51 | -6.1% |
| Quick Mask | None | 73.89 | 73.42 | -0.6% |
| Quick Mask | Central 50% coverage | 41.74 | 40.48 | -3.0% |
| Alpha + white-point sample | None | 74.72 | 74.21 | -0.7% |
| Alpha + white-point sample | Central 50% coverage | 42.58 | 40.49 | -4.9% |

Timings were captured on the initial functional fix (`96dcb10f`); the later Levels dropdown naming/control update leaves this CPU benchmark workload unchanged.

No candidate median was slower in these cases. Small reductions are observations, not a claimed algorithmic speedup. Existing pixel-processing and proxy algorithms are reused; metadata resolution and a bounded 101 × 101 picker sample add no full-canvas scan to a drag frame. These measurements do not establish a 16 ms interactive budget.

Reproduce:

```sh
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo +1.99.0 run --release \
  -p photocraft-ui-egui --example tone_targets_bench -- \
  --reps 9 --out /tmp/tone-targets-bench.json
```

Raw samples, build logs, binary SHA-256 hashes, screenshot scripts and captures are retained locally in `plan/curves-followup-evidence/` (gitignored). The benchmark source and regression tests are included in the change.
