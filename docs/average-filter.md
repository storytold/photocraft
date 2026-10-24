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

### Initial measurements (2026-10-23)

Initial measurements on an Intel Core i7-11370H (4 cores / 8 threads), Windows x64, Rust 1.99.1, normal release profile. One fresh-process pair per case, before rebasing from main `e6d6fb45a` to `1c3db2e5c`; the Average kernel and benchmark source are unchanged, but these numbers have not yet been remeasured with the updated dependencies. MB is decimal.

| 24 MP RGBA8 case | Dense filter time | Streaming filter time | Dense peak working set | Streaming peak working set |
|---|---:|---:|---:|---:|
| No selection | 1.909 s | 1.322 s | 1,453.8 MB | 213.2 MB |
| Partial selection | 3.329 s | 1.278 s | 1,459.5 MB | 218.6 MB |

All four processes exited successfully. Full-output hashes match within each pair: `9d843acc1e6106bf` without a selection and `f8e0e6427433728c` with a partial selection. These are single-run observations, not repeated medians; repeated measurements on the rebased head are pending.
