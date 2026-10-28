# RAW decode performance across camera brands

This is the historical pre-optimization run. The [follow-up implementation and measurements](raw-decode-optimization.md) close all 20 integration gaps and supersede its current-support counts. Its timings and artifacts are retained unchanged for comparison.

Measured on **2026-10-28 (Asia/Shanghai)**, 01:19–01:22 local time. This is an opt-in native
benchmark of `photocraft-raw`'s decode API, covering **88 CC0 originals, 20 vendor groups,
24 file extensions, 3,589,711,499 input bytes (3.59 GB)**. Every selected file was attempted by
all four paths. This covers a representative camera/mode matrix; it is not every camera,
compression setting, or non-RAW file format PhotoCraft can open.

## Findings

- Builtin decoded **50/88**, forced FlashRaw through PhotoCraft **56/88**, Auto **68/88**,
  and the FlashRaw library directly **87/88**. Auto used FlashRaw for 56 and Builtin fallback
  for 12. All 20 Auto failures decoded in the library-direct test, exposing identification,
  representation or metadata-bridge gaps. These counts measure sensor decode, not UI import.
- There were **2,349 successful timed calls**, plus untimed validation and warmups. No Rust
  panic, process crash or timeout occurred in this run. This is finite test evidence, not
  proof against arbitrary malformed inputs.
- **46 files** gave exactly equal geometry, complete sample arrays and PhotoCraft metadata
  between Builtin and Auto. Only these are eligible for the Builtin/Auto speed ratios below.
  **18 further files** decoded under Auto without a working Builtin reference; Auto and forced
  FlashRaw agreed, but share the same decoding implementation.
- **4 files differed between Builtin and Auto**, including samples inside the output crop:
  Leica D-Lux 7 RWL, Nikon P7000 NRW, Panasonic GH5 RW2 and Sony R1 SR2. They are excluded from
  speed rankings. A difference alone does not establish which decoder is correct; independent
  LibRaw checks are recorded separately below.
- Gains depend on the codec. Canon 5D IV / 6D II / T7 CR2 improved **3.46× / 3.80× / 3.95×**;
  Nikon Z8 lossless NEF improved **3.14×**. Z6's packed uncompressed 12/14-bit samples improved
  **12.29× / 14.15×** (these are not lossless-compressed NEFs). Adobe lossless-JPEG DNG became
  **17.0% slower** (58.925 → 68.956 ms), and the small Olympus E-1 file became **1.324 ms slower**
  (0.762 → 2.085 ms). FlashRaw is not a uniform speed improvement.

## Method and practical limits

Apple M1 Pro, 10 physical/logical CPU cores, 16 GiB RAM, aarch64 macOS, Rust 1.98.0.
Native default release profile: opt-level 3, thin LTO, one codegen unit; FlashRaw **0.1.1**
CPU/SIMD/rayon, without its GPU, ICC, mmap or JPEG XL features. PhotoCraft's native Builtin
JPEG XL support remains available. No source decoder behaviour was changed for this benchmark.
Normal desktop applications stayed open; our builds and downloads completed before timing.
This is one workstation/run, not a controlled cross-hardware or browser performance study.

Each input is read once. An untimed pass checks all sensor samples and PhotoCraft metadata,
then drops its decoded planes. Two warmups and nine measured calls follow for each successful
backend, rotating the starting backend each round. The reported value is the median; min,
max and all nine values are retained in the raw JSON. Each file runs in a new process.
A backend that fails validation decoding is recorded as an error and gets no timing, rather
than a misleading zero-millisecond result.

Timing includes parsing, allocations, decompression, the metadata bridge and PhotoCraft's
sensor postprocessing, such as DNG saturation detection. It excludes disk reads, destruction
of the returned output, validation comparisons, demosaicing, colour/preview fitting,
development, document creation and UI work. Thread-pool startup is warmed. These numbers
measure warm-cache decode throughput, not complete open latency, peak memory or cold-start cost.

Library-direct output omits PhotoCraft's metadata and sensor policy. It can linearize/scale
samples differently; its time is diagnostic and cannot be substituted for PhotoCraft's Auto
time. For example, Adobe lossless-JPEG DNG took 21.619 ms directly versus 68.956 ms in Auto,
and uncompressed DNG took 1.943 ms directly versus 38.250 ms in Auto. This difference suggests
PhotoCraft-side work deserves profiling; it is not a direct measurement of metadata cost alone.
Both DNG routes call the full-sensor saturation detector in `crates/raw/src/saturation.rs`.

## Vendor coverage

Counts include successful decode calls with flagged sample differences. Timing ranges include
those flagged cases and are not accuracy claims. FlashRaw/Builtin counts refer to Auto routing.

| Vendor group | Files | Auto decoded | FlashRaw / Builtin | Flagged differences | Auto median range, ms |
|---|---:|---:|---:|---:|---:|
| Apple | 1 | 1 | 0 / 1 | 0 | 39.038–39.038 |
| Canon | 18 | 17 | 16 / 1 | 0 | 22.099–145.445 |
| DJI | 1 | 1 | 1 / 0 | 0 | 34.759–34.759 |
| Epson | 1 | 1 | 1 / 0 | 0 | 8.824–8.824 |
| Fujifilm | 6 | 6 | 6 / 0 | 0 | 5.533–130.003 |
| GoPro | 2 | 2 | 2 / 0 | 0 | 84.743–95.716 |
| Google | 2 | 2 | 2 / 0 | 0 | 37.745–147.807 |
| Hasselblad | 2 | 0 | 0 / 0 | 0 | — |
| Kodak | 2 | 1 | 1 / 0 | 0 | 30.839–30.839 |
| Leica | 4 | 4 | 4 / 0 | 1 | 6.330–114.575 |
| Minolta | 1 | 0 | 0 / 0 | 0 | — |
| Nikon | 14 | 14 | 14 / 0 | 1 | 4.930–104.106 |
| Olympus / OM System | 5 | 1 | 1 / 0 | 0 | 2.085–2.085 |
| Panasonic | 7 | 2 | 2 / 0 | 1 | 7.584–14.434 |
| Pentax | 2 | 0 | 0 / 0 | 0 | — |
| Phase One | 1 | 0 | 0 / 0 | 0 | — |
| Ricoh | 1 | 1 | 1 / 0 | 0 | 70.196–70.196 |
| Samsung | 3 | 2 | 0 / 2 | 0 | 193.379–782.231 |
| Sigma | 2 | 1 | 1 / 0 | 0 | 180.744–180.744 |
| Sony | 13 | 12 | 4 / 8 | 1 | 4.188–63.745 |

## All selected files

Times are milliseconds. `ERR` is a failed sensor decode, with the exact diagnostic in the TSV/JSON.
`DIFF` excludes that file from speed comparisons. `new` means Builtin has no successful reference.
The library column is the standalone library path with the same feature build, not another app mode.
Source paths, compression descriptions, file sizes and SHA-256 pins are in `samples.json`.

| Camera / mode / filename | Builtin | Forced FlashRaw | Auto | Library direct | Auto route | Consistency / speedup |
|---|---:|---:|---:|---:|---|---|
| Canon EOS 5D Mark III (Adobe DNG) · lossless JPEG DNG · `5G4A9394-compressed-lossless.DNG` | 58.925 | 70.830 | 68.956 | 21.619 | FlashRaw | equal; 0.85× |
| Canon EOS 5D Mark III (Adobe DNG) · lossy JPEG DNG · `5G4A9394-compressed-lossy.DNG` | 147.428 | ERR | 145.445 | 21.518 | Builtin fallback | equal; 1.01× |
| Canon EOS 5D Mark III (Adobe DNG) · uncompressed DNG · `5G4A9394-uncompressed.DNG` | 37.644 | 38.345 | 38.250 | 1.943 | FlashRaw | equal; 0.98× |
| Apple iPhone 12 Pro · ProRAW · `IMG_1361.DNG` | 35.666 | ERR | 39.038 | 27.764 | Builtin fallback | equal; 0.91× |
| Canon EOS R5 Mark II · RAW · `RAW.CR3` | ERR | 84.107 | 84.002 | 84.351 | FlashRaw | new |
| Canon EOS R6 Mark III · RAW · `IMG_4241.CR3` | ERR | 60.395 | 60.282 | 60.146 | FlashRaw | new |
| Canon EOS 5D Mark IV · RAW · `B13A0734.CR2` | 199.187 | 56.477 | 57.574 | 32.070 | FlashRaw | equal; 3.46× |
| Canon EOS 6D Mark II · RAW · `6DII-RAW.CR2` | 190.619 | 49.885 | 50.105 | 28.664 | FlashRaw | equal; 3.80× |
| Canon EOS 90D · RAW · `Canon-90D-RAW-ISO-100.CR3` | ERR | 32.391 | 32.559 | 32.773 | FlashRaw | new |
| Canon EOS R5 · C-RAW · `Canon_EOS_R5_CRAW_ISO_100_nocrop_nodual.CR3` | ERR | 101.435 | 101.579 | 101.575 | FlashRaw | new |
| Canon EOS R5 · RAW · `Canon_EOS_R5_RAW_ISO_100_nocrop_nodual.CR3` | ERR | 81.747 | 81.249 | 81.193 | FlashRaw | new |
| Canon EOS R50 · RAW · `Canon_R50_ISO100_RAW_mechanicShutter.CR3` | ERR | 44.816 | 45.158 | 44.633 | FlashRaw | new |
| Canon EOS R6 Mark II · RAW · `canon-r6m2-img_0346-raw-normal-full.cr3` | ERR | 45.839 | 45.923 | 45.642 | FlashRaw | new |
| Canon EOS R7 · RAW · `443A0157.CR3` | ERR | 60.137 | 60.737 | 60.485 | FlashRaw | new |
| Canon EOS R8 · RAW · `RAW_FULL_FRAME.CR3` | ERR | 45.895 | 45.561 | 45.271 | FlashRaw | new |
| Canon EOS Rebel T7 / 2000D · RAW · `IMG_0662.CR2` | 194.617 | 49.278 | 49.236 | 30.705 | FlashRaw | equal; 3.95× |
| Canon PowerShot G6 · additional format coverage · `CRW_2097.CRW` | ERR | ERR | ERR | 35.849 | error | error |
| Canon PowerShot SX50 HS · existing regression corpus · `CRW_4061.DNG` | 27.111 | 22.239 | 22.099 | 1.023 | FlashRaw | equal; 1.23× |
| Canon PowerShot SX50 HS · existing regression corpus · `IMG_4059.CR2` | 73.748 | 25.394 | 25.155 | 17.991 | FlashRaw | equal; 2.93× |
| DJI Phantom 4 Pro · DNG · `DJI_0220.DNG` | 34.121 | 34.739 | 34.759 | 1.610 | FlashRaw | equal; 0.98× |
| Epson R-D1 · additional format coverage · `_EPS0672.ERF` | ERR | 8.822 | 8.824 | 5.593 | FlashRaw | new |
| Fujifilm GFX100 II · lossless compressed · `_DSF0267.RAF` | ERR | 128.378 | 130.003 | 124.519 | FlashRaw | new |
| Fujifilm X-H2 · compressed · `Fuji-X-H2-Compressed-Raw.RAF` | ERR | 92.733 | 91.004 | 85.908 | FlashRaw | new |
| Fujifilm X-S20 · lossless compressed · `DSCF2526.RAF` | ERR | 41.693 | 41.530 | 37.703 | FlashRaw | new |
| Fujifilm X-T4 · uncompressed · `DSCF0266.RAF` | 6.722 | 5.590 | 5.533 | 2.106 | FlashRaw | equal; 1.22× |
| Fujifilm X-T5 · lossless compressed · `DSCF0021.RAF` | ERR | 72.271 | 72.472 | 68.171 | FlashRaw | new |
| Fujifilm X100VI · lossless compressed · `DSCF0075.RAF` | ERR | 72.181 | 71.930 | 68.551 | FlashRaw | new |
| Google Pixel 6 Pro · DNG · `PXL_20220910_093206982.dng` | 32.306 | 37.324 | 37.745 | 10.854 | FlashRaw | equal; 0.86× |
| Google Pixel 8 Pro · DNG · `PXL_20240415_103400204.RAW-02.ORIGINAL.dng` | 127.393 | 147.308 | 147.807 | 43.289 | FlashRaw | equal; 0.86× |
| GoPro HERO10 Black · GPR · `GOPR0018.GPR` | ERR | 84.242 | 84.743 | 46.160 | FlashRaw | new |
| GoPro HERO11 Black · GPR · `GOPR0123.GPR` | ERR | 96.596 | 95.716 | 50.819 | FlashRaw | new |
| Hasselblad CFV-50c · additional format coverage · `20200215_BERLIN_OldNew_WestHafen_B0000420.3FR` | ERR | ERR | ERR | 63.903 | error | error |
| Hasselblad X2D 100C · compressed · `Job_0079.fff` | ERR | ERR | ERR | 137.434 | error | error |
| Kodak DC50 · additional format coverage · `RAW_KODAK_DC50.KDC` | ERR | ERR | ERR | 2.302 | error | error |
| Kodak DCS Pro 14N · additional format coverage · `D4C50031.DCR` | ERR | 30.714 | 30.839 | 27.379 | FlashRaw | new |
| Leica M10-R · DNG · `M10R0324.DNG` | 302.841 | 115.413 | 114.575 | 46.466 | FlashRaw | equal; 2.64× |
| Leica SL2 · DNG · `L1020010.DNG` | 98.978 | 84.091 | 82.555 | 4.772 | FlashRaw | equal; 1.20× |
| Leica D-Lux 7 · additional format coverage · `L1023208.RWL` | 8.636 | 6.700 | 6.330 | 6.648 | FlashRaw | DIFF |
| Leica Q2 · DNG · `L1000750.DNG` | 98.462 | 82.043 | 82.340 | 4.513 | FlashRaw | equal; 1.20× |
| Minolta Dynax 7D · additional format coverage · `PICT3410.MRW` | ERR | ERR | ERR | 0.402 | error | error |
| Nikon Z6III · lossless · `Nikon_Z6__3_Lossless_compressed_FX.NEF` | 166.449 | 57.600 | 56.081 | 52.870 | FlashRaw | equal; 2.97× |
| Nikon Coolpix P7000 · additional format coverage · `RAW_NIKON_P7000.NRW` | 7.728 | 9.558 | 9.539 | 6.233 | FlashRaw | DIFF |
| Nikon D3 · existing regression corpus · `JD1_8203.NEF` | 77.681 | 31.273 | 32.072 | 30.694 | FlashRaw | equal; 2.42× |
| Nikon D750 · 14-bit lossless · `lossless_compressed_14_bit.NEF` | 143.228 | 57.198 | 58.258 | 54.441 | FlashRaw | equal; 2.46× |
| Nikon D850 · 14-bit lossless · `Nikon-D850-14bit-lossless-compressed.NEF` | 284.258 | 98.409 | 104.106 | 99.617 | FlashRaw | equal; 2.73× |
| Nikon Z50 · lossless · `DSC_0450.NEF` | 146.396 | 50.505 | 49.101 | 45.231 | FlashRaw | equal; 2.98× |
| Nikon Z 6 · existing regression corpus · `DSC_0750.NEF` | 70.069 | 5.689 | 5.700 | 2.346 | FlashRaw | equal; 12.29× |
| Nikon Z 6 · existing regression corpus · `DSC_0751.NEF` | 152.356 | 55.438 | 54.618 | 51.269 | FlashRaw | equal; 2.79× |
| Nikon Z 6 · existing regression corpus · `DSC_0753.NEF` | 69.758 | 4.709 | 4.930 | 1.774 | FlashRaw | equal; 14.15× |
| Nikon Z 6 · existing regression corpus · `DSC_0754.NEF` | 151.644 | 57.328 | 57.379 | 53.508 | FlashRaw | equal; 2.64× |
| Nikon Z7II · lossless · `Z72_0027.NEF` | 287.750 | 102.753 | 101.544 | 101.660 | FlashRaw | equal; 2.83× |
| Nikon Z8 · 14-bit lossless · `Nikon_Z8_raw_14_bit_lossless_compression.NEF` | 313.685 | 106.218 | 99.764 | 101.263 | FlashRaw | equal; 3.14× |
| Nikon Z9 · 14-bit lossless · `Nikon_-_NIKON_Z_9_-_14bit_compressed_(Lossless).NEF` | 292.492 | 98.739 | 101.595 | 95.349 | FlashRaw | equal; 2.88× |
| Nikon Zf · lossless · `DSC_0040.NEF` | 159.077 | 54.189 | 53.516 | 50.481 | FlashRaw | equal; 2.97× |
| Olympus E-1 · existing regression corpus · `E_1__C106743_gredos.ORF` | 0.762 | 2.110 | 2.085 | 0.432 | FlashRaw | equal; 0.37× |
| Olympus E-5 · existing regression corpus · `_7061961_copy.ORF` | ERR | ERR | ERR | 76.369 | error | error |
| Olympus E-M1 Mark III · ORF · `_3160531.ORF` | ERR | ERR | ERR | 123.358 | error | error |
| OM System OM-1 Mark II · ORF · `20mp-12bit.orf` | ERR | ERR | ERR | 114.088 | error | error |
| OM System OM-5 · ORF · `C1291096.ORF` | ERR | ERR | ERR | 123.984 | error | error |
| Panasonic DC-G9 · existing regression corpus · `P1000475.RW2` | 12.673 | 14.644 | 14.434 | 14.110 | FlashRaw | equal; 0.88× |
| Panasonic Lumix G9II · RW2 · `P1000019.RW2` | ERR | ERR | ERR | 27.798 | error | error |
| Panasonic Lumix GH5 · RW2 · `_T012010.RW2` | 9.785 | 7.408 | 7.584 | 7.280 | FlashRaw | DIFF |
| Panasonic Lumix GH6 · RW2 · `4-3.RW2` | ERR | ERR | ERR | 29.225 | error | error |
| Panasonic Lumix S1R · RW2 · `P1000077.RW2` | ERR | ERR | ERR | 15.789 | error | error |
| Panasonic Lumix S5 · RW2 · `P1047510.RW2` | ERR | ERR | ERR | 4.063 | error | error |
| Panasonic Lumix S5II · RW2 · `P1126458_mechanical.RW2` | ERR | ERR | ERR | 26.885 | error | error |
| Pentax K-1 Mark II · PEF · `_K1I9780.PEF` | ERR | ERR | ERR | 80.790 | error | error |
| Pentax K-3 Mark III · PEF · `_IMG0115.PEF` | ERR | ERR | ERR | 59.319 | error | error |
| Phase One IQ4 150MP · IIQ L · `IQ4_150MP_L_14.iiq` | ERR | ERR | ERR | 81.126 | error | error |
| Ricoh GR III · DNG · `R0000357.DNG` | 169.993 | 71.456 | 70.196 | 28.794 | FlashRaw | equal; 2.42× |
| Samsung Galaxy S23 Ultra · DNG · `20231214_130645.dng` | 781.144 | ERR | 782.231 | 207.206 | Builtin fallback | equal; 1.00× |
| Samsung NX1 · additional format coverage · `2016-07-23-142101_sam_9364.srw` | ERR | ERR | ERR | 147.903 | error | error |
| Samsung Galaxy S22 · JPEG XL · `20250305_173647.dng` | 193.458 | ERR | 193.379 | ERR | Builtin fallback | equal; 1.00× |
| Sigma fp L · DNG · `SDIM0915.DNG` | 155.841 | 180.572 | 180.744 | 53.448 | FlashRaw | equal; 0.86× |
| Sigma SIGMA DP2 Merrill · additional format coverage · `DP2M1726.X3F` | ERR | ERR | ERR | 100.805 | error | error |
| Sony DSC-F828 · additional format coverage · `DSC06227.SRF` | ERR | ERR | ERR | 3.036 | error | error |
| Sony DSC-R1 · additional format coverage · `_DSC1477.SR2` | 3.959 | 4.202 | 4.188 | 0.879 | FlashRaw | DIFF |
| Sony DSC-RX0 · existing regression corpus · `DSC00009.ARW` | 8.254 | ERR | 8.349 | 4.547 | Builtin fallback | equal; 0.99× |
| Sony A1 · compressed · `A1_full_compressed.ARW` | 16.197 | ERR | 16.126 | 13.886 | Builtin fallback | equal; 1.00× |
| Sony A6400 · compressed · `DSC00087.ARW` | 6.812 | ERR | 6.944 | 7.213 | Builtin fallback | equal; 0.98× |
| Sony A6700 · lossless · `DSC00001.ARW` | 32.589 | 30.775 | 30.484 | 30.712 | FlashRaw | equal; 1.07× |
| Sony A7C II · compressed · `DSC02435.ARW` | 9.836 | ERR | 10.018 | 9.256 | Builtin fallback | equal; 0.98× |
| Sony A7 III · compressed · `_DSC0009.ARW` | 7.773 | ERR | 7.792 | 6.830 | Builtin fallback | equal; 1.00× |
| Sony A7 IV · lossless · `ILCE-7M4_DSC06674_FullFrame-LossLess-Compressed-Large.ARW` | 35.999 | 35.207 | 35.252 | 36.877 | FlashRaw | equal; 1.02× |
| Sony A7 IV · compressed · `ILCE-7M4_DSC06677_FullFrame-Raw-Compressed.ARW` | 10.658 | ERR | 10.888 | 9.692 | Builtin fallback | equal; 0.98× |
| Sony A7R V · lossless · `7RM5-LosslessCompressedLarge.ARW` | 63.085 | 60.492 | 63.745 | 63.382 | FlashRaw | equal; 0.99× |
| Sony A9 III · compressed · `DSC00111.ARW` | 8.129 | ERR | 7.779 | 7.114 | Builtin fallback | equal; 1.05× |
| Sony ZV-E10 · compressed · `DSC00002.ARW` | 7.137 | ERR | 7.197 | 6.910 | Builtin fallback | equal; 0.99× |

## Auto sensor failures

The standalone FlashRaw library decoded each of these 20 files. The exact PhotoCraft error is
shown below. Some formats are outside the app's advertised RAW extensions; these diagnostics
call the RAW crate directly and do not verify the higher-level I/O routing or file chooser.
The I/O layer may offer an explicitly labelled embedded preview, which is not sensor decoding.

| File | PhotoCraft Auto diagnostic |
|---|---|
| `Canon/PowerShot G6/CRW_2097.CRW` | not a camera raw file |
| `Hasselblad/CFV-50c/20200215_BERLIN_OldNew_WestHafen_B0000420.3FR` | malformed camera raw: lossless JPEG predictor 8 |
| `Hasselblad/X2D 100C/Job_0079.fff` | malformed camera raw: lossless JPEG predictor 8 |
| `Kodak/DC50/RAW_KODAK_DC50.KDC` | unsupported camera raw: raw compression scheme 32867 |
| `Minolta/Dynax 7D/PICT3410.MRW` | not a camera raw file |
| `Olympus/E-5/_7061961_copy.ORF` | unsupported camera raw: Olympus compressed (or packed) ORF is not decoded |
| `Olympus/E-M1MarkIII/_3160531.ORF` | unsupported camera raw: Olympus compressed (or packed) ORF is not decoded |
| `OM System/OM-1 Mark II/20mp-12bit.orf` | unsupported camera raw: Olympus compressed (or packed) ORF is not decoded |
| `OM System/OM-5/C1291096.ORF` | unsupported camera raw: Olympus compressed (or packed) ORF is not decoded |
| `Panasonic/DC-G9M2/P1000019.RW2` | unsupported camera raw: Panasonic RW2 raw format 8 (vendor-compressed) is not decoded yet |
| `Panasonic/DC-GH6/4-3.RW2` | unsupported camera raw: Panasonic RW2 raw format 8 (vendor-compressed) is not decoded yet |
| `Panasonic/DC-S1R/P1000077.RW2` | unsupported camera raw: Panasonic RW2 raw format 6 (vendor-compressed) is not decoded yet |
| `Panasonic/DC-S5/P1047510.RW2` | unsupported camera raw: Panasonic RW2 raw format 7 (vendor-compressed) is not decoded yet |
| `Panasonic/DC-S5M2/P1126458_mechanical.RW2` | unsupported camera raw: Panasonic RW2 raw format 8 (vendor-compressed) is not decoded yet |
| `Pentax/K-1 Mark II/_K1I9780.PEF` | unsupported camera raw: Pentax compressed PEF |
| `Pentax/K-3 Mark III/_IMG0115.PEF` | unsupported camera raw: Pentax compressed PEF |
| `Phase One/IQ4 150MP/IQ4_150MP_L_14.iiq` | not a camera raw file |
| `Samsung/NX1/2016-07-23-142101_sam_9364.srw` | not a camera raw file |
| `Sigma/SIGMA DP2 Merrill/DP2M1726.X3F` | not a camera raw file |
| `Sony/DSC-F828/DSC06227.SRF` | not a camera raw file |

The sole library-direct failure was Samsung Galaxy S22 JPEG XL DNG: JPEG XL is disabled in this
FlashRaw feature build. PhotoCraft Auto decoded that file through Builtin/jxl-oxide in 193.379 ms.
This result must not be described as the FlashRaw library lacking JPEG XL support generally.

## Sample differences and independent checks

All four differing pairs have the same full-plane geometry. Counts below compare Auto against
Builtin, not against a proprietary renderer. See `sample-differences.jsonl` for coordinates,
common deltas, levels and metadata. Geometry and active/crop bounds come from PhotoCraft.

| Camera | Full samples different | Inside output crop | Maximum absolute u16 delta | Metadata difference |
|---|---:|---:|---:|---|
| Leica D-Lux 7 | 3,066 / 17,183,488 | 3,044 | 2,560 | none |
| Nikon Coolpix P7000 | 9,899,495 / 10,046,688 | 9,899,495 | 4,065 | white |
| Panasonic Lumix GH5 | 4,397 / 20,550,656 | 4,328 | 2,048 | none |
| Sony DSC-R1 | 10,343,026 / 10,390,272 | 10,343,026 | 64,770 | white |

**Independent result:** rawpy **0.27.1** with LibRaw **0.22.1** matched Auto on **every sample**
in each of these four complete planes (zero differences, including margins). Builtin differed
from LibRaw by the same counts shown above. This is evidence of Builtin compatibility defects;
forcing Builtin as a fallback would not resolve these cases. They remain excluded from speed
ratios because the compared outputs differ. LibRaw's white levels for NRW (4095) and SR2
(16383) also agree with Auto; Builtin returns 4074.525 and 65012.305 respectively. This does not validate camera colour rendering.

The complete independent-check results are in `libraw-full-frame.json`. The check uses rawpy's full
`raw_image` including margins, before demosaicing or normalization. LibRaw is a test-only external
oracle here; it is not a dependency of PhotoCraft. Agreement with it does not validate camera
colour calibration or replace synthetic regressions for a future decoder correction.

## Reproduce and inspect

```sh
python3 perf/raw-decode/run.py --output target/raw-decode-results
# One file, read once; JSON on stdout:
cargo run --release -p photocraft-raw --example rawdecodebench -- PATH_TO_RAW
# Diagnose full-plane/active/crop sample differences:
cargo run --release -p photocraft-raw --example rawsamplediff -- PATH_TO_RAW
```

- [Sample manifest](../perf/raw-decode/samples.json): 88 source URLs, sizes, SHA-256 pins and licenses.
- [Raw timings](../perf/raw-decode/m1-pro-2026-10-28/results.jsonl): all results, all successful timings and errors.
- [Spreadsheet-friendly TSV](../perf/raw-decode/m1-pro-2026-10-28/results.tsv): one row per original.
- [Machine/source fingerprint](../perf/raw-decode/m1-pro-2026-10-28/machine.json): hardware, toolchain, tested source and binary hashes.
- [Sample differences](../perf/raw-decode/m1-pro-2026-10-28/sample-differences.jsonl): full-plane/crop counts and metadata.
- [Independent full-frame checks](../perf/raw-decode/m1-pro-2026-10-28/libraw-full-frame.json).

Source selection: the [FlashRaw popular-camera list](https://github.com/ybouane/flashraw/blob/ee9d8b5e250a9d39bb2c0028b62998095778fbd2/benches/popular-cameras.tsv)
and [CC0 corpus manifest](https://github.com/ybouane/flashraw/blob/ee9d8b5e250a9d39bb2c0028b62998095778fbd2/tools/corpus/manifest.tsv),
pinned at that immutable revision, plus PhotoCraft's existing 14-file camera corpus and additional
format/compression representatives from the same manifest. Originals are from [raw.pixls.us](https://raw.pixls.us/),
stay in gitignored `corpus/` and are not shipped with the app. This manifest does not enlarge the
normal CI corpus job. The [rawpy API documentation](https://letmaik.github.io/rawpy/api/rawpy.RawPy.html)
defines the independent full-image buffer used by the supplemental check.

Validation: default raw tests (112 unit + 108 integration), no-default-feature raw tests
(112 + 106), corpus raw tests (113 + 115), benchmark diagnostic tests (4), clippy with all targets,
clippy without default features, formatting, dependency layering and the 24-crate wasm gate plus
HEIF. These gates check the diagnostic tools and existing regression suite; the four discovered
Builtin/Auto differences remain separate findings and are not silently accepted as new oracles.
No performance baseline, parity percentage or decoder routing was changed.
