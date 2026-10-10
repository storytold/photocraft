# Large-canvas brush performance

PhotoCraft keeps document pixels in sparse, copy-on-write 256² native-format tiles. A
20,000² document does not require allocating every layer in full. The GPU compositor
already uploads changed tiles and refreshes the damaged canvas region and its mip levels.

## Brush execution

Large dabs (radius at least 128 pixels) are binned into 64² coverage tiles in bounded
batches of 64 dabs. Native builds process independent tiles with Rayon; WebAssembly
uses the same tile math serially. Every tile evaluates its dabs in the original order.
Rasterization writes coverage directly, avoiding a dense dab mask and a second pass
over that mask. Small brushes and Wet Edges retain the serial raster path.

Compositing reads the original surface, applies coverage, selection, opacity, blend mode
and channel rules, and encodes native pixels on workers. Results are published as row
copies in bounded batches. ICC CMYK context is propagated explicitly to workers.
There is no reduction in resolution, dab count, texture filtering or working precision.

On pointer release the normal paint command can adopt the tiles already rendered by
the live stroke. It checks the command, source document and revision, symmetry, resolved
brush, target, parameters and complete sample sequence. A mismatch uses normal rendering.
The command still passes through ordinary dispatch, journaling, channel restrictions and
undo/redo. This avoids replaying the entire gesture on the UI thread at release.

## GPU coverage

Native wgpu builds also install a compute coverage backend on the canvas's own device.
Each invocation owns one pixel and evaluates its ordered dab list, accumulating in f32.
It supports computed round/elliptical tips, rotation, flips, projection, flow/opacity and
computed dual tips. Wet Edges retains the upstream stroke-depth CPU path. Sampled tips,
noise, per-tip texture and per-dab
native colour accumulation retain the CPU implementation. Stroke-level masks, native
compositing and colour conversion continue through the existing engine.

Dispatch is automatic for sufficiently dense batches: at least eight million estimated
dab pixels and approximately four overlapping dabs per coverage pixel. Small or dispersed
batches stay on the CPU because GPU transfers cost more than they save. Work is bounded
to 64 dabs and 1,024 coverage tiles per dispatch; larger jobs use CPU fallback. Pooled
coverage/readback buffers together use at most 32 MiB, plus small job/dab buffers. The
kernel is warmed at native canvas setup, before the first pointer gesture.

Coverage is returned to CPU storage before compositing. Consequently the existing native
pixel formats, COW history, saves, cancellation and recovery remain available; this is
not a fully GPU-resident document backend. Device loss or unsupported compute limits use
CPU fallback. WebAssembly continues to use the CPU coverage path.

GPU floating-point arithmetic is not guaranteed bit-identical to CPU arithmetic. Tests
bound coverage error to 3e-5 and compare native pixels at RGB 8/16/32f, CMYK8 and Gray16;
8-bit differences are limited to one unit, 16-bit to two units, and float to 3e-5. Soft,
hard, wet, elliptical, rotated, aliased, dual and projected tips are covered. The serial
and parallel CPU implementations remain bit-identical in their regression cases.

To compare native CPU/GPU coverage on the same device:

```sh
cargo test --release -p photocraft-gpu --test brush completed_compute_brush_benchmark -- --ignored --nocapture
cargo run --release -p photocraft-ui-egui --example brush_latency_bench -- --size 20000 --brush 1000 --frames 60 --spacing-percent 1 --step 120
PHOTOCRAFT_GPU_BRUSH=0 cargo run --release -p photocraft-ui-egui --example brush_latency_bench -- --size 20000 --brush 1000 --frames 60 --spacing-percent 1 --step 120
```

The environment variable disables GPU brush coverage while leaving GPU compositing on.
The sustained benchmark reports completed GPU brush batches to distinguish acceleration
from fallback. On the Apple M5, a dense 24-point hard-brush batch measured 15.141 ms CPU
versus 6.955 ms GPU, including binning, transfers and completed computation. Single-point
and four-point batches were slower on the GPU, which motivated the automatic threshold.

## Reproducing measurements

```sh
cargo run --release -p photocraft-paint --example bench_brush
cargo test --release -p photocraft-paint thousand_pixel_brush_release_comparison -- --ignored --nocapture
cargo run --release -p photocraft-ui-egui --example brush_latency_bench -- --size 20000 --brush 1000 --frames 60
```

The last benchmark waits for completed GPU work on every sample, including compositing
and mip generation. It measures a soft round brush at 5% spacing and 30% flow on a
default white background. It excludes physical input, the full UI event loop, presentation
and monitor scanout. It requires access to a real GPU adapter.

2026-10-10, Apple M5/Metal, release build, 20,000² RGBA8, 1,000 px brush, two separate runs:

| Measurement | 60 samples | 180 samples |
|---|---:|---:|
| CPU painting, median / p95 | 3.021 / 6.254 ms | 2.410 / 3.006 ms |
| Painting + completed GPU work, median / p95 | 7.528 / 13.227 ms | 5.264 / 7.351 ms |
| Painting + completed GPU work, p99 / maximum | 17.941 / 41.104 ms | 9.398 / 17.839 ms |
| Commit | 0.412 ms | 0.450 ms |
| Initial canvas refresh | 2,646.8 ms | 482.1 ms |
| Canvas display texture bytes, including mips | 2,133,331,536 | 2,133,331,536 |

System load and driver caches affect these runs; the longer run traverses more tile
boundaries. Neither run is a hardware-independent performance budget.

The isolated 24-dab, 1,000 px raster comparison measured 82.792 → 13.095 ms for plain
round tips and 103.621 → 19.618 ms for per-tip texture. This is a raster-only comparison,
not an end-to-end speedup. Coverage is compared exactly against the serial implementation.
Regression tests also compare native pixels at 8/16/32-bit RGB and 8-bit CMYK, sampled,
textured, dual, rotated, wet-edge, noisy, colour-dynamic and aliased tips.

## Remaining limits

These measurements support typical 60 Hz painting for this particular workload and
machine; they do not establish a universal frame-time guarantee. Cold allocation,
occasional frame outliers, dense multilayer documents, expensive layer effects and other
hardware need separate measurements. Mixer and smudge have different dependencies.

Painting remains CPU-authoritative, with optional compute coverage acceleration. Fully
GPU-resident painting and a bounded virtual canvas/mip cache are further architectural
work. The display currently allocates the
full canvas, and large high-bit canvases retain the existing RGBA8 display fallback.
Document pixels keep their native depth. Replacing this hybrid path with GPU-resident
painting requires equivalent precision, colour semantics, ordered accumulation, recovery
and save/history coherence across the remaining brush types.
