# Panel docking repair validation

## Review submission (2026-10-31)

Validated code: `88da5a9142c9c55bbb9fb93abe8ddc5192f83929`, integrating main
`422ca7beda90d927643bcd074646ebbd37706d75`. The conflict in Cargo.lock is resolved
with the explicit local craft-ui override retained. Craft-libs dependencies and
snapshot provenance now use `c5a1cbfb5f9049f9270955caffb5bee5997aebf7`; egui's
immutable revision is unchanged. The four upstream UI files updated since the old
pin are copied unchanged. A source audit confirms 76 vendored source files match
that revision; the six documented docking patch files are the only differences.

The contributor built and tested the repaired v0.18.0 Windows EXE before this last
upstream merge and reported no remaining major panel issues. The final upstream
integration is covered by the automated results below; it is not a claim that the
contributor tested the later updater-dialog/shared-widget changes. The agent did
not build or launch the product EXE. Physical monitor/mixed-DPI, focused native IME
checks and GPU/input latency have not been independently certified.

Fresh checks on the integrated code:

| Check | Result |
|---|---|
| Panel regressions | 164 passed; two explicit benchmark tests ignored |
| Shared UI library | 579 passed, including all 346 docking tests |
| Workspace integration | 9 passed |
| Desktop all-targets compile check | passed |
| fmt, layering | passed; 31 crates |
| wasm | passed; 24 crates plus codecs/heif |
| Complete UI library suite | **4494 passed, 95 failed, 23 ignored** |

All 95 failure names exactly match the pre-merge repaired v0.18.0 snapshot; none
were added or removed. This comparison does not establish an independently rerun
clean-main baseline, and the complete UI suite is **not green**.

Strict Clippy is also **not green**. The standard UI command stops at unchanged
main code `text/src/psd_path.rs:157`; the `--no-deps` run reports the same five
locations at `canvas/strokes.rs:495,500`, `dialogs/vp_ui.rs:989`,
`widgets/text_field.rs:29` and `dialogs/color_picker_ui/tests.rs:130`. The standalone
shared-library all-targets check reports two `nonminimal_bool` diagnostics at
`docking/render/preview.rs:120`, which is byte-identical to the pinned upstream
source. No lint suppressions, CI changes or unrelated fixes were introduced.

Fresh captures comprise 444 PNGs: 120 native-follow-up states, 72 desktop-review
states, 120 seam states and 132 join/collapse transitions, across three Pro themes
and 100/125/150/200%. All four capture tests and their physical-pixel seam assertions
pass. Compared with the contributor-reviewed repair, 443 PNGs are byte-identical;
one drag-preview frame was reviewed side by side with no observed geometry change.
Both documented after PNGs and the three new GIFs come from this integrated code.
See [demonstrations and reproduction](panel-docking-demo.md).

A new 600-sample-per-phase CPU kittest run measured p50 idle/hover/drag at
2.273/2.268/2.473 ms, with drag p95 2.982 ms. This is test UI-frame CPU cost, not
native FPS or GPU/input latency. No new broad `perf --quick`, engine-suite or corpus
measurements are claimed for this integration. Previous counts below are historical.

Raw logs, source audits, failure-name comparisons, capture hashes and frames are
preserved separately in the fork's `codex/panel-layout-handoff-2026-10-31` branch,
under `plan/panel-repair/pr-ready-portable-2026-10-31`. Earlier portable snapshots
remain historical. The two original CodeRabbit documentation findings are fixed:
committed test fixtures have their own attribution section, and the timing/demo
links describe their actual targets.

## Historical integration with upstream b1cfff9d3 (2026-10-30)

Integrated upstream `b1cfff9d30298eea4f446a8f6a5e4285cb43c6e7`, resolving all five
conflicts. Both host test-module additions and upstream screen-mode availability
checks remain, with the whole-Tools footer and shared Workspace adapter. The
reviewed craft-ui/egui pins and core panel implementation did not change.

Fresh results on this merged source:

- 136 panel tests passed; two opt-in benchmarks ignored. Dependency-contract,
  dock, workspace, Tools, type/screen-mode flyout, visibility and overflow filters passed.
- 2762 engine tests passed (22 ignored); adversarial `panic_hunt` passed separately.
- 346 shared docking tests and 9 workspace integration tests passed.
- fmt, 31-crate layers, 24-crate wasm plus codecs/heif, parity/scorecard generation,
  UI/engine doctest checks (zero examples), and shared-library Clippy passed.
- 570 new offscreen matrix PNGs, 72 physical-pixel seam assertions, 515 pointer
  replay frames and 15 refreshed GIFs completed. Transition/theme/DPI montages and
  selected raw captures were reviewed. Both documented after PNGs were refreshed.
- SHA-256 checks confirmed that 1267 core source files stayed unchanged during
  validation and capture. No new CPU/GPU performance measurements are claimed.

The full UI **library** run is **not green**: 4460 passed, 95 failed, 23 ignored.
92 failure names match the historical record; this is not an independently rerun
full baseline of the new upstream. Three additional, unchanged upstream
`selected_fonts_commit_pattern_0/2/3` Probe tests select the installed `Inter Hewn`
font instead of their expected `Inter` after ArrowDown+Enter. The Probe renders
Character/options controls without docking; the Windows font registry confirms
that family is installed. Fonts and unrelated type code were not changed.
The complete workspace test suite was not run.

Strict UI/engine all-targets Clippy also remains **failed**. The standard command
stops at upstream `text/src/psd_path.rs:157` (`overly_complex_bool_expr`). Additional
`--no-deps` diagnostic runs expose `engine/src/crop_fill.rs:78` (`nonminimal_bool`),
`canvas/strokes.rs:495,500` and `widgets/text_field.rs:29` (`collapsible_match`),
`dialogs/vp_ui.rs:989` (`nonminimal_bool`), and
`dialogs/color_picker_ui/tests.rs:130` (`manual_range_contains`). All six affected
files have no diff from upstream/main. No lint/CI suppression or unrelated
text/PSD/painting fixes were added to this panel PR. Shared Clippy passed separately.

Current logs, source/capture hashes, raw frames and GIFs are saved in the separate
handoff package at `plan/panel-repair/home-portable-2026-10-30` on branch
`codex/panel-layout-handoff-2026-10-30`. The original package stays historical.
The existing PR remains a draft pending contributor desktop acceptance and review
of the documented non-green gates. Native focus/IME, physical monitors, mixed DPI
and real GPU stalls remain unverified. The contributor builds the product EXE;
it was not built or launched by the agent.

## Historical checkpoint

Validated 2026-10-30 against upstream `382b047c30483024159aaa63831947bc99a5e28b`.
The two final transition regressions were demonstrated failing before the fix:
full height on join/detach, and attached top-bar collapse/tab insertion.

- 136 panel tests passed (two opt-in benchmarks ignored in the regular filter); dependency contract and related docking/workspace/Tools/visibility/overflow filters passed.
- 2711 engine tests passed; adversarial command `panic_hunt` passed separately.
- 346 shared docking tests passed with the documented optional test inputs.
- Formatting, UI/engine all-targets Clippy, shared-library Clippy, engine/UI doctest checks, 31-crate layer check, 24-crate wasm plus codecs/heif, and parity/scorecard regeneration passed.
- 570 offscreen matrix PNGs across three Pro themes and 100/125/150/200% scale, including the sanitized problem layout, plus 515 replay frames; 72 physical-pixel seam checks passed.
- Five scoped CPU hover/drag runs completed; results are in [Scoped CPU timing](#scoped-cpu-timing), and capture reproduction is in [the demo](panel-docking-demo.md).
- The full UI run is **not green**: 92 failures. 92 match independently recorded historical baseline failures; 0 do not match that record. This is separate from the passing panel suite.

The full UI command was `cargo test -p photocraft-ui-egui --lib --tests -- --test-threads=4`.
Its summaries are:

```text
test result: FAILED. 4303 passed; 92 failed; 21 ignored; 0 measured; 0 filtered out; finished in 221.62s
```

The baseline comparison is historical, not a new full run of current upstream.
No claim is made that the complete application test suite is green.

## Scoped CPU timing

Five current-code CPU runs: full UI at 1280×800, 125%, no document and no GPU;
600 measured frames per phase/run after 40 warmup frames. Values below are
medians of per-run percentiles in milliseconds; max is the largest observed
frame across the five runs. This is not native GPU/FPS measurement or a new
before/after comparison.

| Phase | p50 ms | p95 ms | p99 ms | max ms |
|---|---:|---:|---:|---:|
| idle | 2.373 | 3.426 | 3.846 | 10.357 |
| hover | 2.260 | 3.032 | 3.587 | 10.600 |
| drag | 2.475 | 3.561 | 4.999 | 9.480 |

## Reproduction and limits

Run the commands in `AGENTS.md` for `photocraft-ui-egui` and `photocraft-engine`.
The capture reproduction is in [the demo](panel-docking-demo.md); the shared
crate's optional test input recipe is in `vendor/craft-ui/PATCH.md`.
When sharing a target directory between checkouts, build xtask into an isolated
target as documented in [the compatibility contract](panel-docking-contract.md),
because its root is compiled into the helper.

The rendered matrix includes the sanitized saved workspace, join/detach, resize,
header hover, compact/full chains, tab insertion, all four docking edges and
theme/DPI changes. Source fingerprints and capture hashes were compared after
the gates. The X glyph keeps `half = 5.0 * scale + 0.35`.

Native Windows capture previously timed out. Physical monitor/mixed-DPI/IME
interaction and real GPU stalls remain to be checked in the contributor's
desktop build. The main application EXE was not built or launched.
