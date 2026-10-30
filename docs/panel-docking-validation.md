# Panel docking repair validation

Validated 2026-10-30 against upstream `382b047c30483024159aaa63831947bc99a5e28b`.
The two final transition regressions were demonstrated failing before the fix:
full height on join/detach, and attached top-bar collapse/tab insertion.

- 136 panel tests passed (two opt-in benchmarks ignored in the regular filter); dependency contract and related docking/workspace/Tools/visibility/overflow filters passed.
- 2711 engine tests passed; adversarial command `panic_hunt` passed separately.
- 346 shared docking tests passed with the documented optional test inputs.
- Formatting, UI/engine all-targets Clippy, shared-library Clippy, engine/UI doctest checks, 31-crate layer check, 24-crate wasm plus codecs/heif, and parity/scorecard regeneration passed.
- 570 offscreen matrix PNGs across three Pro themes and 100/125/150/200% scale, including the sanitized problem layout, plus 515 replay frames; 72 physical-pixel seam checks passed.
- Five scoped CPU hover/drag runs completed; reproduction and results are in [validation notes](panel-docking-demo.md).
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
