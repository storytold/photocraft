# PSD export throughput and byte equivalence

In-memory layer, mask, pattern, filter and saved-channel planes gather directly from tiles.
Background layers omit unused transparency. Raw merged output owns its completed planes,
reserves saved-channel capacity and encodes native CMYK into final slices. Native rendering
uses the full compositor band budget; wasm keeps smaller bands. Imports retain main's decoder
and scoped workers. Upstream's large-layer banding, cancellation and scratch-backed encoded
storage remain intact. No dependency versions change; native I/O and PSD use the locked Rayon.

## Encoding and storage

RLE scans each row once on the caller's Rayon pool and joins owned chunks in row order.
Long packets use eight-byte checks after a scalar prefix; short packets keep scalar scanning.
Packet boundaries, count widths and tails stay exact. Large prediction inputs split on whole
rows; float byte-plane shuffling and delta coding share initialized scratch. Row resets,
plane-boundary predecessors, deflate backend, level and input bytes stay unchanged.

Owned float planes predict in place and return to scratch after encoding; codec errors keep
unmodified raw bytes. Surface/fill conversion reuses sample storage, filter-cache whitening
runs during gathering, and opacity checks compare stored precision. Native CMYK reads five
samples into compositor storage, preserving defaults, non-finite cleanup and masks. Workers
restore document colour-profile scopes and never enter pool work while holding tile guards.

Pattern/filter records backpatch lengths in final buffers; validated raw pattern planes are
borrowed. Linked frames reserve trailers before appending. Smart export moves generated caches
and uniquely owned global payloads; shared imported bytes detach. Placement, identity and
frame-relative padding checks remain intact.

## Pattern sampling and blending

Finite integer placements use wrapped rows; bilinear taps reuse indices. Unit-alpha samples
avoid divisions by one after the scalar signed-zero adjustment. Rotation, minification and
fractional-alpha arithmetic stay unchanged. Immutable finite metadata preserves scalar
fallbacks for non-finite texels and coordinates. Normal and finite Multiply/Screen avoid
repeated dispatch; non-finite evaluation, alpha, Fill, gamma and Lab behavior stay in callers.

## Saved fixed-main release comparison

The table records measured source `6f536b1f69fa5f6917b77eccf0fa2fcb87d59092` before rebasing
onto `8e82c0e728ee03eb0ce7e5d12289f98bae1595c9`. The rebased tip has fresh functional checks
but was not retimed. Fixed main remains `292ce6ab55a3c18636fe599cfbddc20ebc32cea9`.
Its saved measurements and executable were reused unchanged. No individual commit was timed.

Both runs used identical harnesses, fixtures, dependencies, thin LTO and one codegen unit on
arm64 macOS with 16 logical CPUs. Six 6000 × 4000 photo workflows derive from pinned corpus
image P1000475.RW2, covering RGB8/16/32f, CMYK16/32f, Background, gradient masks and saved alpha;
the authored `masks.psd` adds a 640 × 1394 design case. Each scenario used a fresh process,
two warmups and seven samples, with byte/hash checks outside the timer. The spill budget was
64 MiB. Builds/tests ran at reduced priority; measurements ran at default priority afterward.

| Export | Main ms | Branch ms | Elapsed change | Peak RSS MiB, main → branch | Peak RSS change |
| --- | ---: | ---: | ---: | ---: | ---: |
| photo-rgb8 export | 213.30 | 89.33 | -58.1% | 886 → 979 | +10.5% |
| photo-rgb16 export | 256.17 | 80.91 | -68.4% | 2137 → 1815 | -15.1% |
| photo-rgb16-masked export | 340.13 | 136.29 | -59.9% | 2619 → 2441 | -6.8% |
| photo-rgb32-masked export | 2070.99 | 1066.42 | -48.5% | 3976 → 3023 | -24.0% |
| photo-cmyk16 export | 341.17 | 144.32 | -57.7% | 2944 → 2538 | -13.8% |
| photo-cmyk32-masked export | 2160.86 | 1130.26 | -47.7% | 4490 → 4309 | -4.0% |
| authored-masks export | 42.23 | 9.67 | -77.1% | 238 → 233 | -2.0% |
| photo-rgb16 export-psb | 272.91 | 80.81 | -70.4% | 2154 → 1814 | -15.8% |
| photo-cmyk32-masked export-psb | 2179.54 | 1136.50 | -47.9% | 4474 → 4305 | -3.8% |
| photo-rgb16 export-spill | 322.60 | 119.89 | -62.8% | 2030 → 1894 | -6.7% |
| **Mean (10 scenarios)** | **819.99** | **399.44** | **-51.3%** | **2594.8 → 2335.1** | **-10.0%** |

The mean row compares arithmetic column means using unrounded data. Mean peak RSS is
**10.0% lower (259.7 MiB less)**, with reductions in **9 of 10** scenarios; RGB8 increases.
This averages scenario peaks, not a combined application peak. RSS includes setup, warmups
and allocator retention. Timings exclude thumbnails and final disk write/flush/rename.

High Power was selected for the branch; main's power mode was not recorded. Unchanged import
controls also improved 27–40%, so whole-branch timing differences cannot be attributed entirely
to code or individual changes. These fixtures do not isolate pattern or smart-cache gains.

## Fidelity and review

Fresh untimed comparisons against the fetched main match all 24 standard/large PSD/PSB
files, seven import hashes, three standalone pattern blocks and both complete f32 compositor
hashes. Upstream staged/banded output, fallback and cancellation tests also pass. Large fixtures
cover masked RGB/CMYK compositions, 2048-square patterns at every depth, filtered embedded
sources and reopened smart objects. Saved pre-rebase stack samples cover compositor, pattern
and smart export; counts are not gains.

Independent scalar oracles cover 90,913 packet cases, stored depths, transforms, HDR, signed
zeros, infinities and distinct quiet/signalling NaNs. Nested one-, two- and four-worker oracles
preserve bytes, ordering and profile scopes. Workers own disjoint output and private scratch;
no new threads, locks or mutable shared caches were added. No unresolved correctness/race
issue was found; review used ownership and deterministic checks, not a race detector.

Fresh rebased-source gates passed: compositor 422 unit + five integration; PSD 329 unit + 212
integration/corpus + doctest; I/O 563 unit + 574 integration/corpus; raster 57 unit + 22
integration. Strict all-target Clippy, formatting, 31-crate layering, wasm and affected
performance smoke passed, with unchanged smoke reports explicitly reused. Two known main
release SVG stack-overflow tests remain excluded. Raster units ran serially after an unchanged
global-reservation test collided in the parallel suite;
its isolated rerun also passed. The SVG exclusions are:

- `svg::tests::bounded_embedded_svg_gets_only_the_nesting_depth_the_drawing_left`
- `svg::an_embedded_svg_gets_only_the_nesting_depth_the_drawing_left`

Further deflate, blend/sampling arithmetic and conversion changes need separate evidence of
exact output and worthwhile benefit. No practical speedup ceiling is claimed.
