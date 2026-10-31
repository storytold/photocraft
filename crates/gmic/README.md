# photocraft-gmic

G'MIC filters for PhotoCraft through the user's own `gmic` command-line program. G'MIC
([gmic.eu](https://gmic.eu), CeCILL licensed) is never linked or bundled. This crate:

- finds the program;
- hands it pixels in G'MIC's uncompressed `.cimg` format;
- runs it as a child process with a deadline and cancellation;
- reads the result back.

It also turns G'MIC's own filter catalogue (`gmic parse_gui json`) into typed parameters and
command lines. It has no `unsafe` and never panics on hostile input: every file, JSON value and
parameter is checked. It builds for `wasm32-unknown-unknown`, where every run returns
`Error::Unsupported`. User documentation: [`docs/gmic.md`](../../docs/gmic.md).

```rust
use photocraft_gmic::{runner, RunOptions, Catalog};

let gmic = runner::find(None)?;                           // preference path, $PHOTOCRAFT_GMIC, PATH
let out = photocraft_gmic::apply(&gmic, &surface, canvas, selection.as_ref(),
                                 "fx_smooth_anisotropic 60", &RunOptions::default())?;

let catalog: Catalog = gmic.load_catalog(Some(&cache_dir), false, &RunOptions::default())?;
let f = catalog.get("Deformations/Kaleidoscope [Polar]").unwrap();
let notation = f.notation();                              // dialog parameter spec
let pipeline = f.command_line(&serde_json::json!({"angleCut": 25}))?;
```

## Modules

| Module | What it does |
|---|---|
| `cimg` | `.cimg` image lists: `encode` (float32, little-endian) and `decode` (every sample type, either byte order; compressed and 3D data rejected; total samples capped). |
| `catalog` | `Catalog::parse` reads `parse_gui json` (`gmic_json_1.0`). Each `Filter` has an `id` (`category/name`), its `command`, and `params` in argument order. `notation`, `defaults`, `args`, `command_line` and `quote` turn dialog values into a safe G'MIC command line. |
| `runner` | `locate`, `Gmic::probe`, `find`. `Gmic::run(images, pipeline, opts)` runs in a temporary folder that is always removed. `update`, `catalog_json`, `load_catalog` (with an on-disk cache per G'MIC version), `default_cache_dir`. `RunOptions` holds the timeout and a cancellation callback. |
| `apply` | `apply(gmic, surface, canvas, selection, pipeline, opts)`: runs a pipeline on a `photocraft_raster::Surface` at any bit depth (RGB or Grayscale), with selection feathering, alpha handling and a size check. |

## Sources

Behaviour, not code, comes from:

- G'MIC's public reference: `parse_gui`, input/output and `.cimg` documentation at gmic.eu.
- Observing the G'MIC 4.0.5 binary: the `.cimg` header and planar layout it writes, the JSON
  `parse_gui json` prints, its error messages, and its quoting rules in command files.

The test fixtures are hand-written in those shapes; none come from G'MIC's filter definitions.

## Tests

```sh
cargo test -p photocraft-gmic                                    # no G'MIC needed (Unix: stand-in scripts)
GMIC_TEST=1 cargo test -p photocraft-gmic --test runner -- --ignored --nocapture   # a real gmic
```
