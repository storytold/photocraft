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
Validated alternating-byte rows emit their exact 128-byte literal packets directly, avoiding
the general packet scan for constant 16-bit mask samples.
Packet boundaries, count widths and tails stay exact. Failed RLE capacity hints fall back
to fallible row growth. Native ZIP output uses bounded blocks, then compacts once to its encoded
size; no capacity is reserved from raw plane size. WebAssembly keeps its existing growing writer.
Large prediction inputs split on whole rows; float byte-plane shuffling and delta coding
share initialized scratch. Row resets, plane-boundary predecessors, deflate backend, level
and input bytes stay unchanged.

Owned float planes predict in place and return to scratch after encoding; codec errors keep
unmodified raw bytes. Surface/fill conversion reuses sample storage, filter-cache whitening
runs during gathering, and opacity checks compare stored precision. Native CMYK reads five
samples into compositor storage, preserving defaults, non-finite cleanup and masks. Workers
restore document colour-profile scopes and never enter pool work while holding tile guards.

Pattern/filter records backpatch lengths in final buffers; validated raw pattern planes are
borrowed. Linked frames reserve trailers before appending. Smart export moves generated caches
and uniquely owned global payloads; shared imported bytes detach. Placement, identity and
frame-relative padding checks remain intact. Pattern writers reject unsupported sample depths.
Mask gathering checks dimensions and reserves fallibly; failed masks make serialization return
an error through the existing channel source, preserving the public model API.

## Pattern sampling and blending

Finite integer placements use wrapped rows; bilinear taps reuse indices. Unit-alpha samples
avoid divisions by one after the scalar signed-zero adjustment. Rotation, minification and
fractional-alpha arithmetic stay unchanged. Immutable finite metadata preserves scalar
fallbacks for non-finite texels and coordinates. Normal and finite Multiply/Screen avoid
repeated dispatch; non-finite evaluation, alpha, Fill, gamma and Lab behavior stay in callers.

## Rebased fixed-main release comparison

The table measures source `1eb800645369d3da187e0363180968f1dc6e2845` rebased onto
`e55e2f3577ad08cb5ff84779a4eee1b8003bf98f`. Only documentation changes follow this measured source.
Fixed main remains `292ce6ab55a3c18636fe599cfbddc20ebc32cea9`; its saved measurements were not rerun or changed.
No intermediate branch or individual commit was timed.

Both runs use the same harness, immutable fixtures, thin LTO and one codegen unit on arm64
macOS with 16 logical CPUs. Six 6000 × 4000 photo workflows cover RGB8/16/32f and CMYK16/32f;
an authored layered-mask PSD adds a smaller design case. Each scenario runs in a fresh process,
with two warmups and seven samples. Byte/hash checks occur outside the timer. Spill uses a
64 MiB tile budget. Builds/tests run at reduced priority; measurements run at default priority
after all task build caches are removed. Relevant upstream dependency changes are included.

| Export | Main ms | Branch ms | Elapsed change | Peak RSS MiB, main → branch | Peak RSS change |
| --- | ---: | ---: | ---: | ---: | ---: |
| photo-rgb8 export | 213.30 | 282.91 | +32.6% | 886 → 997 | +12.5% |
| photo-rgb16 export | 256.17 | 294.11 | +14.8% | 2137 → 1822 | -14.7% |
| photo-rgb16-masked export | 340.13 | 178.66 | -47.5% | 2619 → 2445 | -6.6% |
| photo-rgb32-masked export | 2070.99 | 1904.45 | -8.0% | 3976 → 3288 | -17.3% |
| photo-cmyk16 export | 341.17 | 206.40 | -39.5% | 2944 → 2552 | -13.3% |
| photo-cmyk32-masked export | 2160.86 | 1707.67 | -21.0% | 4490 → 4289 | -4.5% |
| authored-masks export | 42.23 | 12.77 | -69.8% | 238 → 235 | -1.4% |
| photo-rgb16 export-psb | 272.91 | 115.06 | -57.8% | 2154 → 1811 | -15.9% |
| photo-cmyk32-masked export-psb | 2179.54 | 1650.54 | -24.3% | 4474 → 4295 | -4.0% |
| photo-rgb16 export-spill | 322.60 | 176.82 | -45.2% | 2030 → 1866 | -8.1% |
| **Mean (10 scenarios)** | **819.99** | **652.94** | **-20.4%** | **2594.8 → 2360.0** | **-9.1%** |

The mean row compares arithmetic column means using unrounded data. Mean peak RSS changes **-9.1%** (**234.8 MiB less**), with reductions in **9 of 10** scenarios. This averages scenario peaks, not a combined application peak.

The unmasked RGB8 and RGB16 PSD timings exceed fixed main in this run; both observations
are retained. Their cause has not been isolated.

RSS includes setup, warmups and allocator retention. Timings exclude thumbnails and final disk
write/flush/rename. The branch uses the recorded power settings; main's power mode was not recorded.
Import controls change -2.4% to +37.6% versus the fixed baseline. Upstream has changed import paths since that baseline, and the runs were not paired for power, thermal or background load. These whole-branch observations do not isolate the contribution of this PR or its individual commits.
The standard fixtures do not isolate pattern or smart-cache speedups.

## Fidelity and review

Fresh untimed comparisons against the fetched main match all 24 standard/large PSD/PSB
files, seven import hashes, three standalone pattern blocks and both complete f32 compositor
hashes. Upstream staged/banded output, fallback and cancellation tests also pass. Large fixtures
cover masked RGB/CMYK compositions, 2048-square patterns at every depth, filtered embedded
sources and reopened smart objects. Saved pre-rebase stack samples cover compositor, pattern
and smart export; counts are not gains.

Independent scalar oracles cover 91,137 packet cases, stored depths, transforms, HDR, signed
zeros, infinities and distinct quiet/signalling NaNs. Pattern/blend arithmetic compares exact bits
except for [NaN payloads](https://doc.rust-lang.org/std/primitive.f32.html#nan-bit-patterns),
which Rust does not guarantee across arithmetic evaluations; NaN
results must still match. Normal blend copies remain bit-exact even for NaNs. Encoded-file
comparisons remain byte-exact. Nested one-, two- and four-worker oracles preserve bytes,
ordering and profile scopes. Workers own disjoint output
and private scratch; no new production threads, locks or mutable shared caches were added.
No unresolved correctness/race issue was found; review used ownership and deterministic
checks, not a race detector.

Fresh PSD/I/O release/corpus suites pass,
including staged/banded output, fallback and cancellation. Strict all-target Clippy,
formatting, layering, wasm and affected performance smoke pass. Unchanged smoke reports are
explicitly reused. The same two known release SVG failures remain excluded. The unchanged
raster and compositor source retains its previously passed checks, including the test-only
reservation mutex fix; fresh compositor hashes also match main.
The SVG exclusions are:

- `svg::tests::bounded_embedded_svg_gets_only_the_nesting_depth_the_drawing_left`
- `svg::an_embedded_svg_gets_only_the_nesting_depth_the_drawing_left`

Further deflate, blend/sampling arithmetic and conversion changes need separate evidence of
exact output and worthwhile benefit. No practical speedup ceiling is claimed.
