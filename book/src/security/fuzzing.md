# Fuzzing and regression testing

PhotoCraft has `cargo-fuzz` workspaces next to its primary untrusted-format crates:

| Crate | Targets |
|---|---|
| `crates/psd` | `parse`, `descriptor`, `image_source_data`, `grd`, `ase`, `aco`, `patterns`, `filter_effects`, `metadata`, `tagged`, `resources`, `asl`, `abr`, `atn` |
| `crates/codecs` | `decode_any`, `decode_png`, `decode_jpeg`, `decode_tiff`, `decode_exr`, `decode_heif`, `decode_gif`, `decode_webp` |
| `crates/raw` | `develop`, `ljpeg` |
| `crates/affinity` | `preview`, `import_preview` (`--features io`), `stream` |
| `crates/format` | `load` |
| `crates/io` | `import_xcf` (raw and gzipped), `import_pdn`, `import_kra`, `import_ora`, `import_svg` (through `photocraft_io::import`, the File › Open path) |
| `crates/cms` | `icc_profile` (parse, then transforms to and from sRGB in every intent), `lut_file` (.cube, .3dl, .look) |

The `Fuzz (nightly)` workflow (`.github/workflows/fuzz-nightly.yml`) runs every target for five minutes each night, thirty minutes on Sundays, and on demand (with a chosen duration). Each target's corpus is kept in the Actions cache between runs and minimised with `cargo fuzz cmin`, so coverage builds up over nights; `import_kra` and `import_ora`, whose ZIP containers libFuzzer seldom builds from nothing, also start from small valid archives that `crates/io/fuzz/seeds.py` writes. A crash fails that target's leg and uploads the input as an artifact kept for three days; report it as `SECURITY.md` describes. The run's log (which includes libFuzzer's base64 copy of a small crashing input and the stack trace) and the artifact are as public as the repository. A new target must be added to that workflow's matrix.

Run a target from its crate directory with nightly Rust and `cargo-fuzz` installed:

```sh
cd crates/psd
cargo fuzz run parse
```

The normal test suites complement libFuzzer with property tests, structured malformed cases, truncation sweeps, bit mutation, bomb-like headers, archive corruption, and command `panic_hunt` coverage.

## Security properties

A useful fuzz target should detect more than memory-safety crashes. Relevant failures include:

- panic or abort;
- integer overflow or out-of-bounds access;
- unreasonable allocation or decompression expansion;
- timeout, infinite loop, or algorithmic complexity spike;
- recursion/stack exhaustion;
- malformed input accepted into an invalid internal state;
- inconsistent results between detection and explicit-format decode;
- corrupted `.pcraft` content accepted despite hash/CRC mismatch.

## Corpus policy

Every confirmed defect should produce a minimized, permanent regression test when disclosure and licensing allow it. Store only synthetic or redistributable inputs. Do not commit personal documents, customer data, credentials, proprietary assets, or an undisclosed weaponized sample.

A regression entry should record the affected parser, expected error class, resource budget, and issue/advisory reference after disclosure. Fuzz crashes must be reproduced under a normal test before the fix is considered durable.

## Future program

Priority additions are PSB-specific structures, XMP and the other XML inputs (`.kys`, camera-raw XMP), preferences and plug-in manifests, native directory bundles, pattern parsers, embedded smart objects, the JSON control protocol, and a seed corpus with a dictionary per text format. A fast replay of each stored corpus on pull requests remains future work.
