# Minimum and Maximum performance

2026-10-11. Minimum and Maximum with Preserve: Roundness take the extreme over a disc, one horizontal window per disc row. Two changes remove repeated work without changing the result:

- **Shared rows.** Disc rows with the same half-width need the same horizontal extremes of a source row. That covers both halves of the disc, and the runs of equal widths near its middle. Each source row's extreme is now worked out once per half-width, and kept until the last disc row that needs it. Before, it was redone for every disc row. The rows are then combined per pixel in the same order as before: top to bottom, `pick(running, row)`. LLVM leaves unspecified which zero `fmax(+0, −0)` returns, so keeping the order keeps even zero signs identical on every target.
- **Fewer overheads.** The window is read into a plane once per channel, instead of one `Image::get` per sample per disc row. The running extremes reuse their buffers instead of allocating three per row. They walk blocks of `k` instead of testing `i % k` per sample; the picks and their order are the same. Squareness shares these running extremes.

Scratch: the window plane plus, at most, one extreme row set per half-width in use, all for one channel of one tile. That's about 33 MB per worker at radius 100 with the default 256 px tiles.

## Timings

Apple M2 Pro (6 performance + 4 efficiency cores, 16 GB), macOS, rustc 1.99.0, release profile. Base: `e46f119e0303e1ba2230e1c0ad5ccc20f4b3ed5d` (`main`), built in a separate target directory. Each entry is the median of three alternating pairs: before/after, after/before, before/after. The load average was 6–16, from background macOS work. [Raw timings and hashes](min-max-performance.json).

Timed by `crates/algo/examples/bench_min_max.rs`: one `apply_in` on a 6000×4000 RGBA8 photo-like layer with soft alpha, fixture construction outside the timer. Every run wrote its complete output as little-endian f32 sample bits; the last column says whether the SHA-256 of the before and after outputs matched.

| Filter | Preserve | Radius | Before (ms) | After (ms) | Speed-up | Same bits |
|---|---|---:|---:|---:|---:|---|
| Maximum | Roundness | 1 | 199 | 161 | 1.23× | yes |
| Maximum | Roundness | 5 | 684 | 239 | 2.86× | yes |
| Maximum | Roundness | 20 | 3,860 | 948 | 4.07× | yes |
| Maximum | Roundness | 50 | 12,665 | 3,834 | 3.30× | yes |
| Maximum | Roundness | 100 | 34,536 | 12,168 | 2.84× | yes |
| Minimum | Roundness | 20 | 3,588 | 942 | 3.81× | yes |
| Maximum | Squareness | 5 | 293 | 263 | 1.11× | yes |
| Maximum | Squareness | 50 | 563 | 420 | 1.34× | yes |
| Minimum | Squareness | 50 | 505 | 422 | 1.20× | yes |

Radius 1 gains less: its disc rows are all different widths except the outer pair. An exact disc extreme still costs O(radius) per pixel here. A vertical decomposition of the half-width bands would make it O(distinct widths), but it changes the order of the picks, and with it which zero `fmax` returns where +0 and −0 meet. So it isn't in this change.

## Reproduce

```sh
git worktree add --detach ../photocraft-base e46f119e03
cp crates/algo/examples/bench_min_max.rs ../photocraft-base/crates/algo/examples/
(cd ../photocraft-base && CARGO_TARGET_DIR=target cargo build --release -p photocraft-algo --example bench_min_max)
cargo build --release -p photocraft-algo --example bench_min_max

../photocraft-base/target/release/examples/bench_min_max max round 50 before.bin
target/release/examples/bench_min_max max round 50 after.bin
cmp before.bin after.bin
```

Arguments: `min|max`, `round|square`, the radius, and an optional output file. Repeat each pair three times, reversing the order for the middle pair.

## Correctness

The previous disc implementation (one pass per disc row) is kept in the tests as the oracle. `round_min_max_matches_one_pass_per_disc_row` compares every sample's bits for:

- integer and fractional radii from 0.5 to 25;
- minimum and maximum;
- outputs inside, reaching past, and wider than the source;
- ties, +0 and −0, values outside 0..1, and NaNs.

The existing direct-window oracle covers Squareness. Breaking the shared-row cache (a wrong window row offset) fails the disc test.
