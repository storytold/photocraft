# RAW decode measurements

The native benchmark compares PhotoCraft's Builtin, forced FlashRaw and Auto paths, plus the
FlashRaw library directly for diagnosing integration gaps. Camera originals are CC0, downloaded
into gitignored `corpus/`, verified by byte length and SHA-256, and never packaged with PhotoCraft.
`samples.json` records every source URL and pin. This suite is opt-in and does not expand the
normal CI corpus download.

From the repository root:

```sh
python3 perf/raw-decode/run.py --output target/raw-decode-results --cooldown-seconds 20
```

Requires Rust, Python 3.11+ and curl. The runner verifies/downloads the complete sample set,
then builds the release example with Cargo.lock, and only then starts timing. Finish other
CPU-heavy work first. The executable uses default release settings and the default FlashRaw
features, including native FlashRaw JPEG XL; no FlashRaw GPU, ICC or mmap feature is enabled.
`--cooldown-seconds` optionally idles after building (0–60 seconds). Each file runs in a new
process with a 180-second ceiling, and all backends execute sequentially inside that process.

Each file is read into memory once. An untimed validation pass compares complete sample arrays,
geometry and PhotoCraft metadata against Builtin (or Auto if Builtin cannot decode it). All
validation planes are dropped before timing. Then each supported backend gets two warmups and
nine timed calls, rotating the starting backend each round. Parsing, allocation, decompression
and the PhotoCraft metadata bridge are timed; disk reads, output destruction, comparisons,
demosaicing, colour fitting, development and document/UI construction are excluded. These are
warm-cache decode measurements, not total file-open latency or first-use thread-pool cost.

`results.jsonl` retains every timing, min/median/max, exact comparison results and all errors.
A failure has no successful median. The benchmark catches Rust panics at its diagnostic boundary,
records them as failures and continues. Process failures and timeouts are recorded by the runner.
Each result also records the actual backend selected by Auto. `machine.json` records the hardware, toolchain, binary hash, source hashes and manifest hash so
uncommitted implementations can be identified too. `stderr.log` preserves dependency diagnostics.

Only speed ratios for Auto versus Builtin with equal geometry, samples and metadata are
meaningful app comparisons. Library-direct calls omit PhotoCraft metadata and may linearize or
scale samples differently; a direct mismatch is not automatically corrupt output. Agreement
between backends is a consistency check, not an independent camera-colour or codec oracle.
Unsupported sensor data here may still be opened as an explicitly labelled embedded preview by
PhotoCraft's higher-level I/O; preview extraction is deliberately excluded from these results.

The [report](../../docs/raw-decode-performance.md) contains the measured camera matrix. Raw
measurement artifacts from that run live here alongside the sample manifest. Re-running writes
to `target/raw-decode-results` by default and leaves the recorded run unchanged.

The supplemental `sample-differences.jsonl` and `libraw-full-frame.json` record the four
Builtin discrepancies found in this run. Auto matched every full-plane u16 sample from
LibRaw 0.22.1 (rawpy 0.27.1); the independent oracle is test tooling, with no PhotoCraft
dependency or LibRaw speed comparison. `rawsamplediff` reproduces the Auto/Builtin
full-plane, active-area and crop diagnostics.

The [optimization report](../../docs/raw-decode-optimization.md) records the current 88/88
Auto result and full-resolution RGB16 import/re-development of the 20 former integration gaps.
`m1-pro-optimized-2026-10-28/` preserves the new timings, actual routes, before/after table,
previous-Auto consistency fingerprints and independent full-plane LibRaw checks of those 20
files. The historical `m1-pro-2026-10-28/` directory is unchanged. Performance routing is
encoding-based; the ARM64 measurements do not establish optimal routing on other architectures.
