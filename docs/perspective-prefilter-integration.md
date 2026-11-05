# Perspective prefilter integration validation

This note records the 2026-10-24 integration repair for #6376 on top of main
`1c3db2e5`. It does not replace the PR's disclosed quality/performance tradeoffs
or establish Photoshop pixel equivalence.

## Preserve the upstream alpha correction

Main's #6224 (implementation `59d42f39`, fixing #6203) un-premultiplies sampled
colors by the **unclamped** alpha, then clamps the output alpha. Keys bicubic
weights can overshoot one at a half-transparent/opaque transition; dividing by
the clamped alpha instead produces a light border. The integrated affine path
already had this correction, but the new per-pixel perspective path did not.
The repair applies the same denominator to perspective sampling.

`perspective_keeps_the_colour_of_a_soft_edged_sprite` isolates a genuinely
projective magnification that samples the source only, not reduced cells or the
valve fallback. A half-transparent border surrounds an opaque interior. The
test covers Gray, RGB, CMYK and Lab at U8, U16 and F32, including signed/HDR RGB,
nonempty output, finite samples and bounded alpha. Before the fix, Gray U8
color 0.2 became 0.21568628 at an opaque output pixel. The same test passes after
the correction.

## Independently validate the five changed goldens

The old golden set came from `28d35d2c`, before the alpha correction. It cannot
require preserving the old light-border bug after integration. The replacement
values were established without using the new perspective sampler as an oracle:

1. Extract the production portion of `crates/algo/src/transform.rs` from
   `28d35d2c`, without its historical tests.
2. Run the existing golden sources, maps, clipping, format matrix and FNV digest
   helpers against that implementation, using the integration's dependencies.
   It reproduces **all 11 original hashes**, including the single-factor
   perspective output. This also checks that intervening resampler changes do
   not alter these fixtures.
3. Make exactly one production substitution in that historical implementation:
   `acc[c] / al` becomes `acc[c] / acc[a]`, the upstream alpha correction.
4. Run the identical harness. Exactly these five hashes change:

| Case | Original | Alpha-corrected reference |
| --- | --- | --- |
| rhombus | `26c1dbfe5adde94f` | `39e23d115d411aaa` |
| skew-flatten | `c1b6bec3d847a18d` | `508ed060c886bca1` |
| translation | `bdaeae899c65a684` | `94b5f983d9b94f18` |
| rotate30 | `da82a3ebd6eb35ca` | `6d04793cd2003f4c` |
| single-factor perspective Bicubic | `b1ebcdacc0ca4dea` | `0f3cb43559b78a32` |

The other six hashes remain unchanged. These are the only golden changes.
The affine and forced-valve tests continue to assert exact output bytes against
the independently corrected pre-perspective-filter implementation.

## Preserve clip-selected horizon sides

Main's #7557/#7558 deliberately lets a crop wholly on one side of the inverse
horizon draw that side, even when the source majority is on the opposite side.
The old integration test incorrectly required equality between such a crop and
a larger horizon-crossing crop that chooses the source majority.

For the existing fixture, inverse homogeneous w is `1 - (x + y) / 225` and the
source majority has negative w. The negative-quadrant crop has positive w, so
the two outputs should differ. The updated test retains horizon-crossing
comparisons and an explicitly same-side crop. A separate regression checks all
three interpolation modes at three depths: the opposite-side crop draws its
interior, the larger majority-side result does not, and nested crops selecting
that opposite side agree byte for byte. No horizon-selection production code
was changed.

## Local verification

On Linux with the existing Rust 1.95 toolchain and the unchanged lockfile:

- `cargo test --locked --offline -p photocraft-algo --lib --tests`: 682 unit tests
  and 13 integration tests passed; 26 unit tests remain intentionally ignored.
- `cargo clippy --locked --offline -p photocraft-algo --all-targets -- -D warnings`:
  passed.
- `cargo fmt -p photocraft-algo -- --check`: passed.
- `cargo test --locked --offline -p photocraft-algo --doc`: successful, with no
  doctests defined.
- The six clipped-warp tests were rerun after the final four-corner assertions
  and passed.
- `cargo check --locked --offline -p photocraft-algo --target wasm32-unknown-unknown`:
  passed.
- `cargo xtask layers`: passed for 31 crates.

These are scoped local checks, not a full workspace or Windows pass. Hosted CI
must verify the published exact commit. No new performance measurement is
claimed by this correctness repair.
