# Channel transform and Move benchmarks

Measured on 2026-11-03. Baseline code: `d2db062b3ea5b0a82a6297f7fee671d57100623b`;
candidate code: `48bbe4cb76df443c6651f13ab64ab8292178881a` (includes main `e3a84b628dc7bb6b9cd821ee089fa3775f9ad67b`).
This compares two PR integrations, including intervening upstream changes; it
neither isolates one fix nor compares the feature against bare main.

## Archived evidence

[Download all raw results and provenance](https://github.com/hidoba/photocraft/releases/download/pr-4648-benchmark-evidence-20261103/photocraft-pr4648-benchmark-evidence-1791b10b1.zip)
([ZIP checksum](https://github.com/hidoba/photocraft/releases/download/pr-4648-benchmark-evidence-20261103/photocraft-pr4648-benchmark-evidence-1791b10b1.zip.sha256), [archive page](https://github.com/hidoba/photocraft/releases/tag/pr-4648-benchmark-evidence-20261103)).
The archive preserves all 35 original evidence files byte-for-byte from
`1791b10b1904726b333b7737c852d150e5e82c83`, including historical and intermediate
runs, full summaries, binary/source hashes, validation records and the complete
quick-performance report. `MANIFEST.json` records every payload member's byte
length and SHA-256. The historical measurement narrative and PR description are
included for context.

ZIP SHA-256: `89dc13a6981c87a5aaccf01f4a7094a06ced5bf1a51d95869658f62901c693f1`.
Download the ZIP and adjacent checksum into the same folder, then verify with
`sha256sum -c photocraft-pr4648-benchmark-evidence-1791b10b1.zip.sha256`.

## Environment and sampling

- AMD Ryzen 7 9700X; software Vulkan llvmpipe (LLVM 21.1.8, Mesa 26.0.8).
- Rust 1.99.0, release profile, thin LTO, one codegen unit; one Cargo worker.
- Two Rayon workers and two llvmpipe workers; no physical GPU used.
- Synthetic 6000 × 4000 RGB8 layer without a pixel selection; completed
  1000 × 700 offscreen frames, including rendering/readback.
- Three interleaved processes per build/case/renderer, nine warm frames per
  geometry/process: 27 frames per table entry. All 324 Move and 1,080 Transform
  frames are retained; builds and tests were idle during timing.
- Warp uses the harness's 1 × 1 affine-patch deformation. p50 is the median;
  p95 is `sorted[floor(0.95 * (n - 1))]`.

## Current comparison

All values are **p50 / p95 milliseconds**. GPU enabled means the application's
accelerated path on the software adapter; CPU means its fallback, still presented
through software Vulkan.

| Workflow / preview renderer | Baseline | Candidate |
|---|---:|---:|
| One-channel Move / GPU enabled | 20.14 / 23.12 | 18.66 / 23.56 |
| Red + Blue Move / GPU enabled | 18.86 / 23.49 | 19.19 / 23.63 |
| Whole-layer Move control / GPU enabled | 339.66 / 371.31 | 341.73 / 395.24 |
| One-channel Move / CPU | 166.46 / 190.65 | 173.03 / 189.82 |
| Red + Blue Move / CPU | 211.25 / 224.56 | 217.60 / 241.25 |
| Whole-layer Move control / CPU | 137.56 / 143.70 | 124.19 / 132.96 |
| Composite control Affine / GPU enabled | 12.07 / 12.96 | 12.18 / 12.96 |
| Composite control Warp / GPU enabled | 17.10 / 18.11 | 17.35 / 17.85 |
| Red, RGB visible Affine / GPU enabled | 20.89 / 22.05 | 21.18 / 22.03 |
| Red, RGB visible Warp / GPU enabled | 57.34 / 58.24 | 58.61 / 61.04 |
| Red only Affine / GPU enabled | 20.50 / 21.71 | 20.00 / 20.79 |
| Red only Warp / GPU enabled | 57.15 / 58.87 | 56.65 / 58.13 |
| Red + Blue, RGB visible Affine / GPU enabled | 20.74 / 22.37 | 20.35 / 21.48 |
| Red + Blue, RGB visible Warp / GPU enabled | 63.73 / 65.60 | 84.58 / 85.81 |
| Red + Blue only Affine / GPU enabled | 20.65 / 21.59 | 20.36 / 21.59 |
| Red + Blue only Warp / GPU enabled | 63.32 / 64.32 | 84.38 / 86.14 |
| Composite control Affine / CPU | 11.49 / 11.91 | 11.66 / 12.03 |
| Composite control Warp / CPU | 16.36 / 16.70 | 16.05 / 16.78 |
| Red, RGB visible Affine / CPU | 76.00 / 83.76 | 75.56 / 82.11 |
| Red, RGB visible Warp / CPU | 104.05 / 106.64 | 102.99 / 105.49 |
| Red only Affine / CPU | 106.99 / 113.44 | 103.85 / 106.46 |
| Red only Warp / CPU | 139.16 / 145.22 | 135.15 / 139.26 |
| Red + Blue, RGB visible Affine / CPU | 130.25 / 133.77 | 130.82 / 135.36 |
| Red + Blue, RGB visible Warp / CPU | 186.82 / 190.82 | 186.06 / 189.53 |
| Red + Blue only Affine / CPU | 174.08 / 180.41 | 172.55 / 175.79 |
| Red + Blue only Warp / CPU | 232.03 / 234.02 | 228.71 / 233.48 |

Two-channel software Warp is 32.7–33.3% slower in this comparison. A further
540 cache-disabled frames reproduced a 31.4–31.5% increase across the two
visibility cases. That confirmation repeated all five targets, including the
composite control, in three interleaved processes per build with
`MESA_SHADER_CACHE_DISABLE=true`; its samples are archived separately. None were
excluded. The slowdown remains unresolved and no code cause has been isolated.
Identical shared WGSL source bytes are embedded in both binaries; this does not
prove identical generated SPIR-V or driver machine code.

Other accelerated affine/single-channel Warp medians stayed within 2.5%; CPU
Transform medians changed by less than 3%. CPU one-channel/set Move rose
3.9% / 3.0%, while its whole-layer control fell 9.7%. These small software-renderer
runs do not establish statistical equivalence or a hardware-GPU guarantee.
An earlier `e5e5f49a6` → `d2db062b3` comparison recorded a separate 8.7–12.8%
CPU Warp slowdown; the current comparison starts after that integration. Its
complete measurements remain in the archive.

## Reproduction

Use the same populated `CRAFT_FONTS_DIR` checkout for both revisions; see
[Fonts](../development.md#fonts-craft-fonts). The harness source is unchanged
between the recorded baseline and candidate. Its SHA-256 values are:

- `channel_move_gpu_bench.rs`: `6cc17cedfe96dcd3e66527500f4269d143a54671903a3c80eccadda0d6d6ccf0`
- `channel_transform_gpu_bench.rs`: `305f6f5f051bd7fa2773ae1030a51872814392528f90d2c7a23b11cc3916fb07`

Run from each source checkout. These software-driver paths are those used on
Ubuntu; point them at the installed Mesa manifests on other distributions.

```sh
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0
export RAYON_NUM_THREADS=2 LP_NUM_THREADS=2
export WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
export __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json
export __GLX_VENDOR_LIBRARY_NAME=mesa
cargo +1.99.0 build -j1 --release -p photocraft-ui-egui \
  --example channel_move_gpu_bench --example channel_transform_gpu_bench
target/release/examples/channel_move_gpu_bench --step 21
target/release/examples/channel_move_gpu_bench --step 21 --set
target/release/examples/channel_move_gpu_bench --step 21 --whole
target/release/examples/channel_transform_gpu_bench --sets
target/release/examples/channel_transform_gpu_bench --sets --cpu
MESA_SHADER_CACHE_DISABLE=true target/release/examples/channel_transform_gpu_bench --sets
cargo +1.99.0 xtask perf --quick
```

Also run each Move case with `--cpu`. Repeat each build/case/renderer in three
alternating baseline/candidate processes, using the harness's nine warm frames.
Keep primary and cache-disabled measurements separate. Full commands, binary
hashes and the original clean build records are preserved in the archive.

## Validation at the measured candidate

8,645 tests passed (engine 3,263; UI 4,961; compose 421), with 46 ignored. Strict
Clippy, formatting, layering, wasm/HEIF, doctests and adversarial commands passed.
The conditional corpus check skipped unchanged corpus paths; no new full corpus
run is claimed. Software GL passed 58 Transform and 43 Move checks; both Vulkan
preview/commit matrices passed. All 16 matched screenshots were byte-identical
within each renderer. Sparse-mask before/affine/Warp captures were inspected.
The standard quick-performance run was uncontended with no enforced failures;
automatic baseline comparison was skipped because its baseline is from another
machine, and non-enforced overruns remain in the archived report. Windows/DX12
was not run locally. Final independent reviews found no actionable issues.

The evidence cleanup changes documentation and data placement only. Application
code, regression tests, benchmark harnesses and dependencies are unchanged.

## 2026-11-11 — Move correctness follow-up

CPU command timings on a 6000 × 4000 RGB16 raster, with two Rayon workers. Baseline: `30a282996b6d62379acf2cd7200947867b58677a` (this PR with main `58e0a152b`). Candidate: `445a95d470f245e8986cf933f315403efb87ccac`, including main `d0299c417`. The intervening main change only repairs cursor tests/rendering; it does not change the engine measured here. Both builds use the same `multi_channel_bench` source and Rust 1.99.0 release profile.

Three interleaved processes per build and selection case, one warmup and six repeats per target/operation/process: 432 warm samples. Target modes are interleaved inside each process. No compiler or PhotoCraft test process ran during measurement. These are CPU command latencies, not completed UI frame or hardware-GPU measurements.

| Selection | Operation | Target | Baseline p50, ms | Candidate p50, ms | Same work? |
|---|---|---|---:|---:|---|
| None | Move | Whole layer | 18.55 | 18.51 | Yes |
| None | Move | Red | 83.11 | 80.15 | Yes |
| None | Move | Red + Blue | 132.64 | 133.55 | Yes |
| None | Wrapped Move | Whole layer | 24.42 | 23.99 | Yes |
| None | Wrapped Move | Red | 83.56 | 160.80 | No |
| None | Wrapped Move | Red + Blue | 133.73 | 135.18 | No |
| 16 × 16 | Move | Whole layer | 18.75 | 18.71 | Yes |
| 16 × 16 | Move | Red | 84.24 | 101.49 | No |
| 16 × 16 | Move | Red + Blue | 192.50 | 193.92 | Yes |
| 16 × 16 | Wrapped Move | Whole layer | 24.41 | 24.32 | Yes |
| 16 × 16 | Wrapped Move | Red | 84.05 | 102.26 | No |
| 16 × 16 | Wrapped Move | Red + Blue | 193.17 | 196.69 | No |

The old native-channel path ignored `wrap`; its single-channel selected path also ignored the selection. Those rows compare incorrect work with corrected work. In particular, corrected whole-channel wrapped Red takes 160.80 ms and corrected selected Red about 101–102 ms on this fixture. The selection case still uses the existing scalar extraction/writeback path. These results do not compare against the old floating-selection UI path or establish dragging-speed equivalence for it.

Comparable channel paths: unselected Red 83.11 → 80.15 ms, unselected Red + Blue 132.64 → 133.55 ms, selected Red + Blue 192.50 → 193.92 ms. Small samples and a shared desktop do not establish statistical equivalence. GPU uniform-only dragging retains its cached-source path; resource validation is performed at capture and mesh-buffer growth, not on every offset. Wrapped channel Move currently uses the CPU preview. No new GPU-frame speed claim is made.

Reproduce on each revision with the identical candidate harness (copy only the example to the baseline checkout):

```sh
cargo +1.99.0 run --locked -j1 --release -p photocraft-engine --example multi_channel_bench -- --move --subset
cargo +1.99.0 run --locked -j1 --release -p photocraft-engine --example multi_channel_bench -- --move --subset --region
```

`CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, and `RAYON_NUM_THREADS=2` were used. Harness SHA-256: `ed1d417e7c4e1199eeb808983b4a5e666b7bfd4b79028c13a1e3bf30f84a094a`. Raw samples, binary hashes and compiler-artifact provenance are retained locally for publication with this follow-up. The older archived comparisons above remain unchanged.

### Additional review check — 2026-11-11

The final review follow-up (`5d4933e0a74026afd7b19ed54ee1103822a28784`) was compared with `445a95d470f245e8986cf933f315403efb87ccac`, using the identical harness and environment above. Each command has a 16 × 16 selection on the 24 MP RGB16 fixture. Three interleaved processes per build supplied 216 warm CPU command samples; compilation and PhotoCraft tests were idle during measurement. The whole-layer row remains a whole-layer command control.

| Operation | Target | Before p50, ms | After p50, ms | Change |
|---|---|---:|---:|---:|
| Move | Whole layer | 23.83 | 23.81 | -0.09% |
| Move | Red | 111.87 | 113.49 | +1.45% |
| Move | Red + Blue | 210.69 | 214.02 | +1.58% |
| Wrapped Move | Whole layer | 30.72 | 30.89 | +0.57% |
| Wrapped Move | Red | 112.48 | 113.81 | +1.18% |
| Wrapped Move | Red + Blue | 215.58 | 216.45 | +0.40% |

Native medians were 0.4–1.6% higher; controls changed by −0.1% / +0.6%. These small shared-desktop samples do not establish statistical equivalence. This is command latency, not UI-frame or physical-GPU performance. The resampling/writeback and cached GPU uniform path are unchanged. Ordinary masks are priced once before capture; implicit coverage is checked before materialization, and full-period wrapped no-ops skip both.

An intermediate run (also retained) showed native medians +1.3–2.3%, prompting removal of duplicate ordinary source pricing. Raw records and provenance for both runs remain local pending publication; no historical archive or original sample was replaced.
