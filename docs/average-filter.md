# Average filter: transparency reference

The Average filter replaces colour while retaining each pixel's transparency.
Two synthetic cases were executed in Photoshop **26.6.1 on Windows x64** on
2026-10-23. These are observations of RGB8 raster layers, without a selection or
transparency lock; they are not a claim of parity in other colour modes or at
other bit depths.

Each input is a 16 × 16 RGBA PNG. Its left eight columns are red and its right
eight columns are blue. Open it in Photoshop, run Filter › Blur › Average, and
save a PNG copy. Inspect the file's RGBA values, including invisible colours.

| Input left / right RGBA | Output left / right RGBA |
|---|---|
| `(255,0,0,255)` / `(0,0,255,0)` | `(128,0,128,255)` / `(128,0,128,0)` |
| `(255,0,0,64)` / `(0,0,255,192)` | `(128,0,128,64)` / `(128,0,128,192)` |

An opaque control produces `(128,0,128,255)` throughout. Thus transparency is
neither a weight in this colour average nor a channel to replace with its mean.

PhotoCraft previously alpha-weighted the RGB mean and replaced alpha with its
mean. On the first case that predicts translucent red throughout, rather than
purple pixels with their original transparency. The fix uses selection coverage
for the existing mean and leaves each pixel's alpha alone. Selection mixing and
the existing colour conversion path are unchanged. In particular, this does not
establish Photoshop parity for feathered selection weighting or CMYK/Lab means.

The engine tests construct their own pixels; no Photoshop binary, proprietary
code, or reference image is included. They cover transparent and partially
transparent pixels, selection boundaries, one history step, undo and redo at
8/16/32-bit storage. The higher-depth tests establish PhotoCraft invariants;
the external reference measurements above are RGB8 only.

Run the focused regressions with:

```sh
cargo test -p photocraft-engine --lib average_preserves
```

## Smart filters

The PSD importer already recognizes the optionless `Avrg` descriptor (see
[#6297](https://github.com/storytold/photocraft/pull/6297)). Recognition alone did
not execute it: the shared renderer returned `None`, leaving the source pixels
unchanged, and the raster-only command was disabled on Smart Objects.

Average now uses the shared filter command and algorithm. It can be added to a
Smart Object, toggled, undone and redone, or re-rendered from an imported record.
The raster path retains its canvas limit, selection weighting and alpha behavior.
Like other core filters, new invocations validate parameters before editing.

The integration tests create their own red/blue Smart Objects at RGB8/16/32f.
They check the dispatcher, command enablement, one history step, visibility,
undo/redo, and re-rendering after clearing both the layer and stage caches:

```sh
cargo test -p photocraft-engine --test it average_smart
```

An additional, explicitly ignored test accepts a local Photoshop oracle. To
reproduce it in Photoshop, create an opaque 16 × 16 RGB8 image with eight red
columns followed by eight blue columns, convert the layer to a Smart Object,
apply Average and save a PSD. Photoshop 26.6.1 on Windows produces purple
`(128,0,128,255)` throughout. Then run, in PowerShell:

```powershell
$env:PHOTOCRAFT_AVERAGE_PSD = 'C:\path\to\average-smart.psd'
cargo test -p photocraft-engine --test it average_photoshop_psd_rerenders_from_source -- --ignored --nocapture
```

This test discards the imported layer cache and the smart-filter stage cache,
then rebuilds from the embedded source. Merely importing or flattening the PSD
would reuse Photoshop's saved pixels and would not prove execution. The local
PSD is not committed and the opt-in test does not raise a corpus floor.

## Bounded scratch memory

Average reduces the source in row-major order, in segments of at most 4,096
pixels. It then mixes the result in parallel groups of at most eight raster
tiles. Selection coverage is read by row. The reduction keeps the dense
implementation's order of floating-point additions; the output is independent
of tile scheduling.

At five channels, the reduction's pixel and coverage buffers total at most
96 KiB. The output groups hold at most 10 MiB of decoded pixel results, plus
up to 8 KiB of coverage rows. Source, output and history tiles are separate
from those scratch bounds and still scale with the image. Empty source tiles
participate: their default RGB affects the mean, and changing invisible RGB can
require output tiles even where alpha is zero.

The streaming surface path limits work to 512 × 1024² pixels rather than that
many decoded channel samples. A synthetic 196 MP test starts the operation and
cancels after the first progress report; it does not run a complete 196 MP
filter. Other tests check cancellation during both passes and compare exact
sample bits against the previous arithmetic across RGB, Gray, CMYK and Lab,
8/16/32-bit storage, negative coordinates, tile/segment edges and partial masks.
These are preservation tests, not new Photoshop colour-mode parity claims.

The release example runs a 6,000 × 4,000 RGBA8 case and hashes every output
sample. Run each mode in a fresh process and collect peak working set externally:

```sh
cargo run -p photocraft-algo --release --example average_memory -- dense
cargo run -p photocraft-algo --release --example average_memory -- stream
cargo run -p photocraft-algo --release --example average_memory -- dense selected
cargo run -p photocraft-algo --release --example average_memory -- stream selected
```

`dense` reproduces the original `map_pixels` workspace with the transparency
correction: raw samples, original RGBA, processed RGBA and coverage, about
52 bytes/pixel for RGBA. `stream` calls the current public algorithm dispatcher.
The timer covers filtering; peak working set covers the complete process,
including source construction and output hashing. This measures CPU algorithms,
not document history, UI, GPU or Photoshop performance.

### Repeated measurements (2026-10-24)

Measured on an Intel Core i7-11370H (4 cores / 8 threads), Windows x64/MSVC,
Rust 1.99.1, with the normal release profile. The executable was rebuilt from
`1c38945f8ba94c989c609ec142964367df33835e`, whose tested upstream base is
`1c3db2e5c35a4f8c544011e57c8026ccb9741926`. These measurements supersede the
earlier single-run observations from before that rebase.

Each case ran five dense/stream pairs, alternating modes in fresh processes
after the compiler and other benchmark processes had exited. The table reports
median filtering time and median per-process peak working set; MB is decimal.
Peak working set was read through Windows' process high-water mark at 25 ms
intervals. It includes source construction and output hashing as described above.

| 24 MP RGBA8 case | Dense median time | Streaming median time | Dense median peak working set | Streaming median peak working set |
|---|---:|---:|---:|---:|
| No selection | 1.914 s | 1.098 s | 1,453.8 MB | 213.2 MB |
| Partial selection | 2.626 s | 1.169 s | 1,459.5 MB | 218.9 MB |

All 20 processes exited successfully. All ten full-output hashes in each case
match: `9d843acc1e6106bf` without a selection and `f8e0e6427433728c` with a partial
selection. Filtering times ranged from 1.772–1.977 s dense and 0.948–1.188 s
streaming without a selection, and 2.455–2.745 s dense and 1.095–1.311 s streaming
with the partial selection. Five runs on one Windows machine establish these
observations, not a cross-platform performance guarantee.

[Per-run measurements and executable SHA-256](average-filter-benchmark-2026-10-24.json)
retain the raw timings, peak-memory readings, hashes and source commit.

### Windows peak-working-set collector

For a reproducible collection, build first, then launch the executable directly
so Cargo and the compiler are excluded from the measured process:

```powershell
cargo build -p photocraft-algo --release --example average_memory --locked
$exe = (Resolve-Path 'target/release/examples/average_memory.exe').Path
$results = foreach ($selected in $false, $true) {
    foreach ($run in 1..5) {
        foreach ($mode in 'dense', 'stream') {
            $arguments = @($mode)
            if ($selected) { $arguments += 'selected' }
            $stdout = [IO.Path]::GetTempFileName()
            $stderr = [IO.Path]::GetTempFileName()
            $process = Start-Process -FilePath $exe -ArgumentList $arguments `
                -PassThru -WindowStyle Hidden `
                -RedirectStandardOutput $stdout -RedirectStandardError $stderr
            $null = $process.Handle # Retain the handle so ExitCode remains available after exit.
            $peak = 0L
            do {
                $process.Refresh()
                $peak = [Math]::Max($peak, $process.PeakWorkingSet64)
            } while (-not $process.WaitForExit(25))
            $process.WaitForExit()
            if ($process.ExitCode -ne 0 -or $peak -le 0) {
                throw "Benchmark failed or no peak was observed: $(Get-Content $stderr -Raw)"
            }
            [pscustomobject]@{
                mode = $mode
                selected = $selected
                run = $run
                peak_working_set_bytes = $peak
                output = (Get-Content $stdout -Raw).Trim()
            }
            $process.Dispose()
            Remove-Item -LiteralPath $stdout, $stderr
        }
    }
}
$results | ConvertTo-Json | Set-Content -Encoding utf8 average-memory-runs.json
```

`System.Diagnostics.Process.PeakWorkingSet64` is the Windows process peak
working-set counter, in bytes, rather than the instantaneous `WorkingSet64` or
private-byte count. The collector retains the largest high-water mark observed
at 25 ms intervals. A peak first reached after the last successful sample may
be missed; this is sampled process evidence, not allocator instrumentation.
Redirects keep each process's reported filtering time and full-output hash
alongside its memory reading. Adjust the executable path if using
`CARGO_TARGET_DIR` or an explicit target triple.

This is a reproduction command for the documented counter and sampling method.
The original 2026-10-24 collection script was not retained with the artifact;
these instructions do not establish its exact implementation or constitute new
measurements on the current branch.
