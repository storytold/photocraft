# RAW decode optimization and closed integration gaps

Measured 2026-10-28 on Apple M1 Pro (10 cores, 16 GiB), native ARM64 release. This implements
the follow-up to the [historical 88-file benchmark](raw-decode-performance.md), retaining its
original artifacts and sample manifest. Scope: the same **88 CC0 originals, 20 vendor groups,
24 extensions and 3.59 GB**, rather than every camera/mode in existence.

## Result

- Auto now decodes **88/88**, up from **68/88**. All 20 formerly failed integrations also pass
  full-resolution **RGB16 document import and Exposure +0.5 re-development**, including the
  Phase One IQ4 150 MP file. These are sensor-derived documents, with expected crop/orientation.
- Auto uses **69 FlashRaw / 19 Builtin** routes in this ARM64 matrix. Forced PhotoCraft FlashRaw
  succeeds on **79/88** (previously 56); Builtin remains 50/88. Library-direct FlashRaw is now
  88/88 (previously 87), with native JPEG XL enabled.
- All **68** previously successful Auto files retain identical full-plane and Sensor metadata
  consistency fingerprints. Their before/after geometric-mean decode speedup is
  **1.38×**; 59 file medians improve, and none slows by more than
  3.1%. Summing the 68 per-file medians gives
  **4439.2 → 2909.5 ms (34.5% less)**. This sum is a
  corpus statistic, not a timed batch-open or user-interface latency.
- Independent **LibRaw 0.22.1 / rawpy 0.27.1** checks match **every one of 694,104,304 u16
  samples** from all 20 new integrations, across each full sensor including margins. There are
  no crop substitutions, channel exclusions or scaled comparisons in those 20 checks.
- **2,745 successful timed calls** complete without a Rust panic,
  process crash or timeout. Finite valid-file evidence is complemented by synthetic malformed
  input and resource-limit regressions; it is not a proof for arbitrary inputs.

| Camera / mode | Previous Auto (ms) | Optimized Auto (ms) | Before / after | Selected |
|---|---:|---:|---:|---|
| Canon EOS 5D Mark III (Adobe DNG) — lossless JPEG DNG | 68.956 | 27.196 | 2.54× | Builtin |
| Canon EOS 5D Mark III (Adobe DNG) — lossy JPEG DNG | 145.445 | 39.276 | 3.70× | Builtin |
| Canon EOS 5D Mark III (Adobe DNG) — uncompressed DNG | 38.250 | 5.243 | 7.30× | Builtin |
| Samsung Galaxy S23 Ultra — DNG | 782.231 | 139.115 | 5.62× | FlashRaw |
| Leica Q2 — DNG | 82.340 | 12.405 | 6.64× | FlashRaw |
| Leica SL2 — DNG | 82.555 | 13.320 | 6.20× | FlashRaw |
| Google Pixel 6 Pro — DNG | 37.745 | 13.953 | 2.71× | Builtin |
| Olympus E-1 — existing regression corpus | 2.085 | 0.729 | 2.86× | Builtin |

## How Auto selects a decoder

FlashRaw remains primary. Routing inspects encoding/layout and never matches a filename,
camera-name exception list or corpus hash. It does not race two decoders, measure startup
latency or perform two full decodes to choose a route. Explicit Builtin/FlashRaw selections
never silently switch; `decode_with_backend_traced` reports the actual supplier of samples.

The measured ARM64 preference selects Builtin for small lossless-JPEG DNG tiles (at most
1,000,000 samples/tile), whole uncompressed 16-bit DNG strips, native JPEG XL DNG, RW2
RawFormat 5 and uncompressed 16-bit ORF. Packed 12/14-bit DNG, large/strip lossless JPEG,
modern vendor codecs and other supported encodings retain FlashRaw. Other architectures keep
FlashRaw first for these performance choices until measured. Representation compatibility
(float DNG, lossy JPEG DNG, sub-16-bit Deflate, Sony cRAW tone curves) remains independent of
architecture. Wasm still excludes JPEG XL and browser worker/thread-pool startup.

The four known wrong Builtin outputs—Leica D-Lux 7 RWL, Nikon P7000 NRW, Panasonic GH5 RW2
and Sony R1 SR2—remain on FlashRaw. Their previous independent full-plane LibRaw agreement is
retained, along with their unchanged Auto fingerprints. A faster wrong sample plane is not an
eligible performance alternative. Builtin defects themselves remain open. Differences of a few
percent between sequential measurements do not establish a universally fastest decoder.

## Implementation

- Split codec support from metadata extraction. Add byte-based recognition, native picker
  extensions and validated frame metadata for CRW, MRW, SRW, X3F, IIQ, SRF, KDC, 3FR, FFF,
  compressed ORF/PEF and modern RW2. Preserve existing per-format calibration policies. Check
  frame geometry, active/crop bounds, levels and sample layout; reanchor repeating black levels
  and row/column deltas to the active origin, including scalar-to-RGB broadcasting.
- Use the exactly pinned FlashRaw facade's **public low-level lossless-JPEG codec** for stored
  DNG samples. This avoids irreversible facade linearization and preserves non-injective CLUTs;
  no inverse lookup or private format parser is used. Point-transform restoration, segment and
  codestream dimensions, caller allocation/scratch limits, strict end-of-stream and dependency
  panic containment have regressions.
- Replace repeated integer-domain MapPolynomial evaluation with a bounded exact lookup when
  spatial/black-level conditions permit it. Compose in original order with original rounding.
  Preserve the original evaluator for other cases. Exhaustive 8/16-bit tests compare results.
- Compute saturation histograms using a precomputed CFA phase/period, bounded native parallel
  bands and sparse upper-half counts. Period populations supply the omitted dark-bin count;
  reference tests cover crops, origins, Bayer/X-Trans and arbitrary periods/band boundaries.
- Enable native FlashRaw JPEG XL and reject mismatched/incomplete codestream geometry before
  the facade can pad malformed data. Keep native Builtin JXL as the faster measured route.
- Reject RW2 RawFormat 5 streams missing actual sample bytes while permitting omitted unused
  final-page padding. The diagnostic `rawlook` now returns output-write errors and creates
  missing output directories, with a regression for its observed write-failure panic.

Sensor decoding does not establish calibrated colour. Non-DNG colour still uses existing
PhotoCraft profiles or per-file camera-JPEG fitting. Sony F828's RGBE CFA and emerald sensor
values are preserved, but RGB development projects emerald onto green and explicitly warns
that colour is approximate; four-channel calibration remains open. New SRF, X3F, PEF and RW2
renderings were visually inspected, including accepted camera-JPEG fits. No parity percentage
or hour estimate is increased on the strength of this finite matrix.

## Verification and reproduction

The native run uses default release opt-level 3, thin LTO, one codegen unit, FlashRaw 0.1.1
CPU/SIMD/rayon plus native JPEG XL. Builds, downloads and validation tools finish before the
recorded timing run, followed by a 20-second cooldown; normal desktop applications remain open.
Each file is read once, untimed full-plane/metadata validation is dropped before timing, then
2 warmups and 9 rotating measured calls execute per successful backend. Parsing, allocation,
codec and PhotoCraft metadata are included. Disk I/O, destruction, demosaic/development,
colour fitting and document/UI construction are excluded. This is warm-cache decode timing
on one workstation, not browser, cross-hardware or end-to-end opening performance.

```sh
python3 perf/raw-decode/run.py --output target/raw-decode-results --cooldown-seconds 20
cargo test -p photocraft-raw --features corpus,performance-corpus
cargo test -p photocraft-io --features performance-corpus --test it formerly_unsupported_formats_reach_full_sensor_document_import
```

Passed raw tests with features (238), feature-disabled raw tests (221), normal I/O tests
(886 passed, 9 ignored), all 20 former-failure import/re-develop cases, 24 desktop service
tests, rawlook's write-error regression, touched-crate all-target clippy with/without default
RAW features, workspace layers (31 crates), wasm (24 crates plus HEIF), the full corpus gate
(**5,107 passed / 34 ignored**) and current-source `cargo xtask perf --quick`. The quick gate
skips baseline hardware comparison because its stored M4 Pro differs from this M1 Pro; budgets
and baseline are unchanged. `cargo xtask scorecard`, formatting and diff checks also pass.

[Raw results and provenance](../perf/raw-decode/m1-pro-optimized-2026-10-28/results.jsonl),
[machine/source/binary hashes](../perf/raw-decode/m1-pro-optimized-2026-10-28/machine.json),
[before/after TSV](../perf/raw-decode/m1-pro-optimized-2026-10-28/results.tsv),
[summary JSON](../perf/raw-decode/m1-pro-optimized-2026-10-28/summary.json),
[independent new-20 full-plane checks](../perf/raw-decode/m1-pro-optimized-2026-10-28/libraw-new20.json),
[previous-Auto consistency](../perf/raw-decode/m1-pro-optimized-2026-10-28/previous-auto-consistency.json).
The previous-Auto check uses same-toolchain Rust DefaultHasher 64-bit fingerprints of the
complete Vec and Debug metadata without data; these are consistency fingerprints, not SHA-256
proofs. The independent new-format checks retain SHA-256 hashes of both u16 little-endian
planes. Corpus originals remain gitignored, CC0-attributed, and absent from distributables.

## Complete same-file Auto comparison

“Unsupported” has no successful time or numeric speedup. Each row is a separate file;
min/max and all nine observations for each backend remain in the JSONL. New-support rows
measure decode only; the RGB16 import/re-develop tests are separate correctness checks.

| Camera / mode | Previous Auto (ms) | Optimized Auto (ms) | Before / after | Selected |
|---|---:|---:|---:|---|
| Canon EOS 5D Mark III (Adobe DNG) — lossless JPEG DNG | 68.956 | 27.196 | 2.54× | Builtin |
| Canon EOS 5D Mark III (Adobe DNG) — lossy JPEG DNG | 145.445 | 39.276 | 3.70× | Builtin |
| Canon EOS 5D Mark III (Adobe DNG) — uncompressed DNG | 38.250 | 5.243 | 7.30× | Builtin |
| Apple iPhone 12 Pro — ProRAW | 39.038 | 35.414 | 1.10× | Builtin |
| Canon EOS R5 Mark II — RAW | 84.002 | 83.784 | 1.00× | FlashRaw |
| Canon EOS R6 Mark III — RAW | 60.282 | 59.656 | 1.01× | FlashRaw |
| Canon EOS 5D Mark IV — RAW | 57.574 | 54.767 | 1.05× | FlashRaw |
| Canon EOS 6D Mark II — RAW | 50.105 | 49.411 | 1.01× | FlashRaw |
| Canon EOS 90D — RAW | 32.559 | 33.563 | 0.97× | FlashRaw |
| Canon EOS R5 — C-RAW | 101.579 | 101.206 | 1.00× | FlashRaw |
| Canon EOS R5 — RAW | 81.249 | 80.723 | 1.01× | FlashRaw |
| Canon EOS R50 — RAW | 45.158 | 44.738 | 1.01× | FlashRaw |
| Canon EOS R6 Mark II — RAW | 45.923 | 45.244 | 1.02× | FlashRaw |
| Canon EOS R7 — RAW | 60.737 | 59.515 | 1.02× | FlashRaw |
| Canon EOS R8 — RAW | 45.561 | 45.168 | 1.01× | FlashRaw |
| Canon EOS Rebel T7 / 2000D — RAW | 49.236 | 49.267 | 1.00× | FlashRaw |
| Canon PowerShot G6 — additional format coverage | unsupported | 31.897 | new support | FlashRaw |
| Canon PowerShot SX50 HS — existing regression corpus | 22.099 | 3.465 | 6.38× | FlashRaw |
| Canon PowerShot SX50 HS — existing regression corpus | 25.155 | 24.549 | 1.02× | FlashRaw |
| DJI Phantom 4 Pro — DNG | 34.759 | 5.083 | 6.84× | Builtin |
| Epson R-D1 — additional format coverage | 8.824 | 8.853 | 1.00× | FlashRaw |
| Fujifilm GFX100 II — lossless compressed | 130.003 | 127.979 | 1.02× | FlashRaw |
| Fujifilm X-H2 — compressed | 91.004 | 88.635 | 1.03× | FlashRaw |
| Fujifilm X-S20 — lossless compressed | 41.530 | 41.162 | 1.01× | FlashRaw |
| Fujifilm X-T4 — uncompressed | 5.533 | 5.617 | 0.98× | FlashRaw |
| Fujifilm X-T5 — lossless compressed | 72.472 | 71.809 | 1.01× | FlashRaw |
| Fujifilm X100VI — lossless compressed | 71.930 | 71.937 | 1.00× | FlashRaw |
| Google Pixel 6 Pro — DNG | 37.745 | 13.953 | 2.71× | Builtin |
| Google Pixel 8 Pro — DNG | 147.807 | 52.777 | 2.80× | Builtin |
| GoPro HERO10 Black — GPR | 84.743 | 48.852 | 1.73× | FlashRaw |
| GoPro HERO11 Black — GPR | 95.716 | 57.105 | 1.68× | FlashRaw |
| Hasselblad CFV-50c — additional format coverage | unsupported | 63.239 | new support | FlashRaw |
| Hasselblad X2D 100C — compressed | unsupported | 135.374 | new support | FlashRaw |
| Kodak DC50 — additional format coverage | unsupported | 2.233 | new support | FlashRaw |
| Kodak DCS Pro 14N — additional format coverage | 30.839 | 30.690 | 1.00× | FlashRaw |
| Leica M10-R — DNG | 114.575 | 52.491 | 2.18× | FlashRaw |
| Leica SL2 — DNG | 82.555 | 13.320 | 6.20× | FlashRaw |
| Leica D-Lux 7 — additional format coverage | 6.330 | 6.417 | 0.99× | FlashRaw |
| Leica Q2 — DNG | 82.340 | 12.405 | 6.64× | FlashRaw |
| Minolta Dynax 7D — additional format coverage | unsupported | 0.409 | new support | FlashRaw |
| Nikon Z6III — lossless | 56.081 | 51.106 | 1.10× | FlashRaw |
| Nikon Coolpix P7000 — additional format coverage | 9.539 | 9.326 | 1.02× | FlashRaw |
| Nikon D3 — existing regression corpus | 32.072 | 28.810 | 1.11× | FlashRaw |
| Nikon D750 — 14-bit lossless | 58.258 | 50.119 | 1.16× | FlashRaw |
| Nikon D850 — 14-bit lossless | 104.106 | 90.495 | 1.15× | FlashRaw |
| Nikon Z50 — lossless | 49.101 | 44.599 | 1.10× | FlashRaw |
| Nikon Z 6 — existing regression corpus | 5.700 | 5.702 | 1.00× | FlashRaw |
| Nikon Z 6 — existing regression corpus | 54.618 | 50.112 | 1.09× | FlashRaw |
| Nikon Z 6 — existing regression corpus | 4.930 | 4.773 | 1.03× | FlashRaw |
| Nikon Z 6 — existing regression corpus | 57.379 | 53.547 | 1.07× | FlashRaw |
| Nikon Z7II — lossless | 101.544 | 89.986 | 1.13× | FlashRaw |
| Nikon Z8 — 14-bit lossless | 99.764 | 86.044 | 1.16× | FlashRaw |
| Nikon Z9 — 14-bit lossless | 101.595 | 88.925 | 1.14× | FlashRaw |
| Nikon Zf — lossless | 53.516 | 50.166 | 1.07× | FlashRaw |
| Olympus E-1 — existing regression corpus | 2.085 | 0.729 | 2.86× | Builtin |
| Olympus E-5 — existing regression corpus | unsupported | 76.775 | new support | FlashRaw |
| Olympus E-M1 Mark III — ORF | unsupported | 123.130 | new support | FlashRaw |
| OM System OM-1 Mark II — ORF | unsupported | 114.145 | new support | FlashRaw |
| OM System OM-5 — ORF | unsupported | 123.529 | new support | FlashRaw |
| Panasonic DC-G9 — existing regression corpus | 14.434 | 12.496 | 1.16× | Builtin |
| Panasonic Lumix G9II — RW2 | unsupported | 25.927 | new support | FlashRaw |
| Panasonic Lumix GH5 — RW2 | 7.584 | 7.138 | 1.06× | FlashRaw |
| Panasonic Lumix GH6 — RW2 | unsupported | 27.608 | new support | FlashRaw |
| Panasonic Lumix S1R — RW2 | unsupported | 14.818 | new support | FlashRaw |
| Panasonic Lumix S5 — RW2 | unsupported | 3.425 | new support | FlashRaw |
| Panasonic Lumix S5II — RW2 | unsupported | 25.330 | new support | FlashRaw |
| Pentax K-1 Mark II — PEF | unsupported | 73.129 | new support | FlashRaw |
| Pentax K-3 Mark III — PEF | unsupported | 55.959 | new support | FlashRaw |
| Phase One IQ4 150MP — IIQ L | unsupported | 79.527 | new support | FlashRaw |
| Ricoh GR III — DNG | 70.196 | 32.735 | 2.14× | FlashRaw |
| Samsung Galaxy S23 Ultra — DNG | 782.231 | 139.115 | 5.62× | FlashRaw |
| Samsung NX1 — additional format coverage | unsupported | 147.001 | new support | FlashRaw |
| Samsung Galaxy S22 — JPEG XL | 193.379 | 190.691 | 1.01× | Builtin |
| Sigma fp L — DNG | 180.744 | 67.676 | 2.67× | Builtin |
| Sigma SIGMA DP2 Merrill — additional format coverage | unsupported | 101.763 | new support | FlashRaw |
| Sony DSC-F828 — additional format coverage | unsupported | 2.930 | new support | FlashRaw |
| Sony DSC-R1 — additional format coverage | 4.188 | 4.140 | 1.01× | FlashRaw |
| Sony DSC-RX0 — existing regression corpus | 8.349 | 8.201 | 1.02× | Builtin |
| Sony A1 — compressed | 16.126 | 15.729 | 1.03× | Builtin |
| Sony A6400 — compressed | 6.944 | 7.073 | 0.98× | Builtin |
| Sony A6700 — lossless | 30.484 | 30.416 | 1.00× | FlashRaw |
| Sony A7C II — compressed | 10.018 | 9.753 | 1.03× | Builtin |
| Sony A7 III — compressed | 7.792 | 7.685 | 1.01× | Builtin |
| Sony A7 IV — lossless | 35.252 | 34.835 | 1.01× | FlashRaw |
| Sony A7 IV — compressed | 10.888 | 10.454 | 1.04× | Builtin |
| Sony A7R V — lossless | 63.745 | 60.893 | 1.05× | FlashRaw |
| Sony A9 III — compressed | 7.779 | 7.805 | 1.00× | Builtin |
| Sony ZV-E10 — compressed | 7.197 | 6.950 | 1.04× | Builtin |
