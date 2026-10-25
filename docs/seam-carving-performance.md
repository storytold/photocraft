# Content-Aware Scale performance

2026-10-22. Two things are recorded here: what this change saves, and a correction to the P28 row's
recorded number, which predates the seam-search proxies and reads as hundreds of times worse than
the code does today.

## The change

Content-Aware Scale shrinks an image by removing one seam at a time. On large images the seams are
searched on a vertically averaged copy (`carve_width_proxy_height`), and a panel column found there
is not an original column: each seam already removed has shifted the panel left. That was tracked
with an `idx` map — one `usize` per proxy pixel, built up front and shifted by every seam.

The original column is recoverable without the map: for the `i`-th column already taken from a row,
`o - i` is the panel column it occupied, so the taken columns at or left of panel column `x` are
those with `o - i <= x`, and the original column is `x` plus that count. `o - i` never decreases
along the row's list, so that count is a binary search and the original column inserts where the
search stopped; the rows are independent and update in parallel, like the pixel shift.

Counting it by scanning the row's list instead is a comparison per already-removed column, so its
cost grows with the square of the seam count and a deep shrink pays it on every row and every seam.
The binary search removes that term; what is left per removed column is one insert into a list of at
most `seam_count` entries, i.e. an element move rather than a comparison.

Output is unchanged: the same original columns reach the assembly pass, in the same order, so every
pixel written is the pixel the map produced. The 4-channel cases below hash identically (FNV-1a over
the output's raw `f32` bits); the 3-channel case is unchanged within timer noise.

## Timings

Windows 11, Intel i5-10300H (4 cores / 8 threads), rustc 1.98.1, `--release`, default rayon pool.
The timer covers `seam::carve_width` only: document read, tile writeback, history and the undo that
follows the command are outside it. Each cell is the same binary run before and after the change, in
the same session, nothing else running.

| Case | Before (ms) | After (ms) | Change | Output |
|---|---:|---:|---:|---|
| 4000 × 3000 → 3600, 4 ch | 1424.7 | 1132.9 | −20.5 % | identical |
| 6000 × 4000 → 5400, 4 ch | 3009.3 | 2559.4 | −15.0 % | identical |
| 7200 × 5000 → 6480, 4 ch | 4352.8 | 3766.9 | −13.5 % | identical |

Deep shrinks, where the counting scan was quadratic, on the same machine and the same binary with
only the recording swapped (three alternating runs per side, `min`/`median`):

| Case | Scan (ms) | Binary search (ms) | Change | Output |
|---|---:|---:|---:|---|
| 6000 × 4000 → 600, 4 ch (5 400 seams) | 16 640 / 16 757 | 13 915 / 14 066 | −16 % | identical, fnv `0f7eaa6af3cf1557` |
| 6000 × 4000 → 3000, 4 ch (3 000 seams) | 10 617 / 10 814 | 9 936 / 10 012 | −6 % | identical, fnv `1780ee9bc9077ee9` |

The element move that remains is still linear in the row's list length, so a shrink this deep is not
down to the index map's cost yet on four cores; a rank bitvector over the row would remove it, and is
not in this change.

The map was 24 MB next to a 36 MB pixel buffer at 24 MP, so the saving tracks how many bytes of row
the seams have to shift. A three-channel case was measured too and moved inside the timer noise, so
it is not reported as a win here. Further seam-search savings have to come from the dynamic program
itself, not from bookkeeping.

## P28's recorded number is stale

`perf/baseline.json` records P28 (Content-Aware Scale on 24 MP to 90 % width) as 954 192 ms from the
2026-10-05 baseline, which is why `perf/budgets.toml` carries `enforce = false` for that row. The
seam-search proxies and the per-seam energy update landed after that capture:

| Tree | 6000 × 4000 → 5400, 4 ch |
|---|---:|
| `f5d2bcb` (the 2026-10-05 baseline commit) | 266 039 ms |
| `1104b6f` (main, plus this change) | 2 559 ms |

`f5d2bcb` has no `PROXY_SHRINK_MIN_HEIGHT`: it carves at full height with a full energy recompute per
seam. The 954 s in the baseline was that code on a machine whose recorded load average was 226–290
on 14 cores. The two example runs above are 104× apart (266 039 ms against 2 559 ms); that is the
seam-search core alone, and it is not the same measurement as the row's 954 s, which also includes a
document read and writeback the timer above excludes.

The row itself is not flipped to `enforce = true` here: the budget is set for the macOS/aarch64
machine class and only a run on that class can vouch for it. The carving core above is 2.56 s at
24 MP on a 4-core laptop CPU; the budgeted machine is faster, but the row also includes the document
read and writeback this measurement excludes. Flip it in the PR whose nightly run shows the row
under 3 s, and delete this paragraph with it.

## Reproduce

```sh
cargo build --release -p photocraft-algo --example bench_seam
./target/release/examples/bench_seam 6000 4000 5400 4   # width height new_width channels (1..=4)
```

Build the base and the changed tree with separate `CARGO_TARGET_DIR`s and run the same line in both;
the printed FNV hash of the output must match, and the millisecond figure is the row above. The
example builds deterministic gradient-plus-detail content, so two runs of the same binary agree.

## Correctness and repository checks

`cargo test --release -p photocraft-algo seam` passes, including `carver_matches_reference`,
`output_bits_are_pinned`, `tall_proxy_shrink_preserves_full_resolution_salient_and_protected_pixels`
and the cancellation test. `cargo fmt --all --check` and
`cargo clippy -p photocraft-algo --all-targets -- -D warnings` are clean. No command, UI or format
contract changed; `Carver`'s `track` mode is still used by the enlargement path, which needs the
original columns it was written for.
