# PSD export throughput and byte equivalence

Layer, mask, pattern, filter and saved-channel planes gather directly from tiles, including
large layers exported in bands.
Background layers omit unused transparency. Raw merged output owns its completed planes,
reserves saved-channel capacity and encodes native CMYK into final slices. Native rendering
uses the full compositor band budget; wasm keeps smaller bands. Imports retain main's decoder
and scoped workers. Upstream's large-layer banding, cancellation and scratch-backed encoded
storage remain intact. No dependency versions change; native I/O and PSD use the locked Rayon.

## Encoding and storage

RLE scans each row once on the caller's Rayon pool and joins owned chunks in row order.
Long packets use eight-byte checks after a scalar prefix; short packets keep scalar scanning.
Validated alternating-byte rows emit their exact 128-byte literal packets directly, avoiding
the general packet scan for constant 16-bit mask samples.
Packet boundaries, count widths and tails stay exact. Failed RLE capacity hints fall back
to fallible row growth. Native ZIP output uses bounded blocks, then compacts once to its encoded
size; no capacity is reserved from raw plane size. WebAssembly keeps its existing growing writer.
Upstream #9745's single-pass 16/32-bit prediction and restoration stay intact. Large prediction
inputs split on whole rows and reuse initialized float scratch. Row resets, plane-boundary
predecessors, deflate backend, level and input bytes stay unchanged.

Owned float planes predict in place and return to scratch after encoding; codec errors keep
unmodified raw bytes. Surface/fill conversion reuses sample storage. Filter-cache whitening
runs during gathering and borrows rows that need no colour replacement. Opacity checks
compare stored precision. Native CMYK reads five
samples into compositor storage, preserving defaults, non-finite cleanup and masks. Workers
restore document colour-profile scopes and never enter pool work while holding tile guards.

Pattern/filter records backpatch lengths in final buffers; validated raw pattern planes are
borrowed. Linked frames reserve trailers before appending. Smart export moves generated caches
and uniquely owned global payloads; shared imported bytes detach. Placement, identity and
frame-relative padding checks remain intact. Pattern writers reject unsupported sample depths.
Mask gathering checks dimensions and reserves fallibly; failed masks make serialization return
an error through the existing channel source, preserving the public model API.

## Pattern sampling and blending

Finite integer placements evaluate at most one tile period, then copy repeated output pixels
and rows bit for bit; bilinear taps reuse indices. Unit-alpha samples
avoid divisions by one after the scalar signed-zero adjustment. Rotation, minification and
fractional-alpha arithmetic stay unchanged. Immutable finite metadata preserves scalar
fallbacks for non-finite texels and coordinates. Normal and finite Multiply/Screen avoid
repeated dispatch; overflowing finite Screen inputs retain main's wider-precision HDR retry.
Non-finite evaluation, alpha, Fill, gamma and Lab behavior stay in callers.

## Rebased fixed-main release comparison

The table measures final production source `636ef36e6c8d645ca5045b7d1ff954374e7943b9` rebased onto
`6d99175b18e8b82dd7e7399a330236ffbafb5e73`, including upstream's [#9745](https://github.com/storytold/photocraft/pull/9745).
Measurements were taken on November 11, 2026, after the rebase and all source fixes. The later
documentation commit changes no measured production code. Fixed main remains
`292ce6ab55a3c18636fe599cfbddc20ebc32cea9`; its saved measurements were neither rerun nor changed.

Both runs use the same harness, immutable fixtures, thin LTO and one codegen unit on arm64
macOS with 16 logical CPUs. Six 6000 × 4000 photo workflows cover RGB8/16/32f and CMYK16/32f;
an authored layered-mask PSD adds a smaller design case. Each scenario runs once in a fresh
process, with two warmups and seven samples. Byte/hash checks occur outside the timer.
Spill uses a 64 MiB tile budget. Builds and tests run at reduced priority; the final branch
measurements run at default priority on AC High Power, after build-cache cleanup and with
no concurrent task builds or tests. No intermediate branch or individual commit was timed.

| Export | Main ms | Branch ms | Elapsed change | Peak RSS MiB, main → branch | Peak RSS change |
| --- | ---: | ---: | ---: | ---: | ---: |
| photo-rgb8 export | 213.30 | 81.59 | -61.7% | 886 → 975 | +10.0% |
| photo-rgb16 export | 256.17 | 76.28 | -70.2% | 2137 → 1814 | -15.1% |
| photo-rgb16-masked export | 340.13 | 105.35 | -69.0% | 2619 → 2439 | -6.9% |
| photo-rgb32-masked export | 2070.99 | 1047.53 | -49.4% | 3976 → 3244 | -18.4% |
| photo-cmyk16 export | 341.17 | 131.93 | -61.3% | 2944 → 2534 | -13.9% |
| photo-cmyk32-masked export | 2160.86 | 1105.86 | -48.8% | 4490 → 4256 | -5.2% |
| authored-masks export | 42.23 | 8.42 | -80.1% | 238 → 235 | -1.4% |
| photo-rgb16 export-psb | 272.91 | 78.90 | -71.1% | 2154 → 1811 | -15.9% |
| photo-cmyk32-masked export-psb | 2179.54 | 1100.67 | -49.5% | 4474 → 4232 | -5.4% |
| photo-rgb16 export-spill | 322.60 | 112.70 | -65.1% | 2030 → 1899 | -6.5% |
| **Mean (10 scenarios)** | **819.99** | **384.92** | **-53.1%** | **2594.8 → 2343.8** | **-9.7%** |

The mean row compares arithmetic column means using unrounded data. Mean peak RSS changes **-9.7%** (**251.0 MiB less**), with reductions in **9 of 10** scenarios. This averages scenario peaks, not a combined application peak.

RSS includes setup, warmups and allocator retention. Timings exclude thumbnails and final disk
write/flush/rename. Fixed main's power mode was not recorded. Import controls change -42.2%
to -26.2% versus the fixed baseline.
The baseline and branch runs were not paired for power, thermal or background load. The
comparison includes upstream changes, including #9745 and later compositor work; it does
not isolate this PR's contribution. The standard fixtures do not separately time pattern
or smart-cache paths.

## Fidelity and review

Fresh untimed comparisons against the fetched main match all 24 standard/large PSD/PSB
files, seven fixed-input import hashes, three standalone pattern blocks and both complete
f32 compositor hashes. They also match the preserved canonical bytes. Large fixtures cover
masked RGB/CMYK compositions, 2048-square patterns at every depth, filtered embedded sources
and reopened smart objects. The main reference and branch use the duplicate-TIFF-declaration
build repair from [#13129](https://github.com/storytold/photocraft/pull/13129).

Upstream's 16/32-bit single-pass prediction loops and its complete `unpredict` implementation
are retained. Prediction workers reuse initialized float scratch; scalar-reference, wrapping,
row-boundary and shared-worker RLE oracles pass. Main's reservation-test mutex replaces this
branch's equivalent test-only mutex. Pattern gathering retains main's shared memory allowance
and fallible allocations. The Screen fast path preserves main's wider-precision HDR overflow
retry; its scalar oracle covers both signs of `f32::MAX`.

Independent scalar oracles cover 91,137 packet cases, stored depths, transforms, HDR, signed
zeros, infinities and distinct quiet/signalling NaNs. Pattern/blend arithmetic compares exact
bits except [NaN payloads](https://doc.rust-lang.org/std/primitive.f32.html#nan-bit-patterns),
whose propagation Rust does not guarantee; NaN results must still match. Normal blend copies
and encoded-file comparisons remain byte-exact. Nested worker tests preserve ordering and
profile scopes. Workers own disjoint output and private scratch; ownership and deterministic
checks found no unresolved introduced race. No race detector was run.

PSD/I/O unit, integration and corpus checks pass apart from one unchanged upstream SVG test:
`svg::retained_deep_svg_groups_lower_render_and_drop_on_a_bounded_worker` expects a successful
rasterization where main now returns its depth-limit error. That I/O integration run passed
635 tests and failed this one; [#13128](https://github.com/storytold/photocraft/pull/13128)
contains its correction. Dependency-inclusive Clippy also reports main's unchanged TIFF
`prepare` complexity, covered by the same PR. Strict all-target Clippy with `--no-deps` passes
for PSD, I/O, raster and compositor. Layering, formatting and all 23 wasm crate checks pass.
Raster/compositor unit and integration checks pass after rerunning the corrected blend oracle
and the disk-sensitive tests following cache cleanup; the free-space guard stays unchanged.

The local changed-function audit documents 108/108 functions. Successful mask-error tests
retain original error variants across buffered and streaming writes. Banded ZIP retains
the exact input boundaries and encoded bytes; periodic pattern and whitening oracles remain
exact. Only the final branch received a fresh timing run; no additional performance-smoke
benchmarks were run.
