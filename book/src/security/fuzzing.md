# Fuzzing and regression testing

PhotoCraft has `cargo-fuzz` workspaces next to its primary untrusted-format crates:

| Crate | Targets |
|---|---|
| `crates/psd` | `parse`, `descriptor`, `image_source_data`, `grd`, `ase`, `aco`, `patterns`, `filter_effects`, `metadata`, `tagged`, `resources`, `asl`, `abr`, `atn` |
| `crates/codecs` | `decode_any`, `decode_png`, `decode_jpeg`, `decode_tiff`, `decode_exr`, `decode_heif`, `decode_gif`, `decode_webp`, and one per second-tier format: `decode_dds`, `decode_dicom`, `decode_dpx` (DPX and Cineon), `decode_fits`, `decode_iff`, `decode_pcx`, `decode_pnm`, `decode_tga`, `decode_xpm` |
| `crates/raw` | `develop`, `ljpeg` |
| `crates/affinity` | `preview`, `import_preview` (`--features io`), `stream` |
| `crates/format` | `load` |
| `crates/io` | `import_xcf` (raw and gzipped), `import_pdn`, `import_kra`, `import_ora`, `import_svg` (through `photocraft_io::import`, the File › Open path); `camera_raw_xmp` (Camera Raw settings sidecars, loaded, saved and loaded again); `export_roundtrip` (a PSD or .pcraft input written as PSD, flat and layered TIFF and .pcraft, each of which must open again) |
| `crates/cms` | `icc_profile` (parse, then transforms to and from sRGB in every intent), `lut_file` (.cube, .3dl, .look) |
| `crates/engine` | `kys` (a .kys set imported through `edit.keyboardShortcuts`; the set it exports must import again), `prefs` (`preferences.json` with its presets, the additional plug-ins folder kept off; what it saves must load back unchanged), `file_info` (a document's XMP packet read by File Info and the camera data panel, then File Info fields written into it and read back), `layer_xmp` (`layer.xmpMetadata`, then a PSD save and open that must keep the packet) |
| `crates/plugins` | `manifest` (parse and validate, then a JSON round trip), `module` (WebAssembly validation and load, then one filter run under tight fuel, memory and time limits) |
| `crates/text` | `font_bytes` (font and collection bytes registered, laid out and rasterised), `type_layer` (a PSD type layer's `TySh` block and EngineData text, each written and read back) |

The `Fuzz (nightly)` workflow (`.github/workflows/fuzz-nightly.yml`) runs every target for five minutes each night, thirty minutes on Sundays, and on demand (with a chosen duration). Each target's corpus is kept in the Actions cache between runs and minimised with `cargo fuzz cmin`, so coverage builds up over nights; `import_kra` and `import_ora`, whose ZIP containers libFuzzer seldom builds from nothing, also start from small valid archives that `crates/io/fuzz/seeds.py` writes. A target with no corpus yet, and every target on Sundays, first adds the seeds its crate's generator builds (see Seeds and dictionaries), and a target with a dictionary is run with it. A crash fails that target's leg and uploads the input as an artifact kept for three days; report it as `SECURITY.md` describes. The run's log (which includes libFuzzer's base64 copy of a small crashing input and the stack trace) and the artifact are as public as the repository. A new target must be added to that workflow's matrix.

Run a target from its crate directory with nightly Rust and `cargo-fuzz` installed:

```sh
cd crates/psd
cargo fuzz run parse
```

Set `PHOTOCRAFT_NO_SYSTEM_FONTS=1` (as the workflow does) so targets that draw type use the bundled fonts only and don't start by scanning the installed ones, and `PHOTOCRAFT_NO_PRINT=1` so nothing can reach a printer.

## Seeds and dictionaries

The repository commits no binary seeds. Each fuzz crate except `psd`, `raw`, `affinity` and `format` has a seed generator, `fuzz/examples/seeds.rs`, that builds small valid inputs in code (documents exported by our own writers, built-in ICC profiles and LUTs, the bundled fonts, handwritten XML and JSON in Photoshop's spelling) and fails if any of them doesn't load, so a seed can't quietly rot into an input that only reaches the error path. `import_kra` and `import_ora` are seeded by `crates/io/fuzz/seeds.py` instead; `import_xcf` and `import_pdn` have no seeds yet: PhotoCraft writes neither format.

```sh
cd crates/io/fuzz
cargo run --example seeds                      # every target of the crate
cargo run --example seeds -- export_roundtrip  # one target; writes corpus/<target>/seed-*
```

Dictionaries (libFuzzer's `-dict`) live in `fuzz/dicts/`: `psd.dict` (file and block signatures, tagged-block and resource keys, descriptor type codes; also used by `export_roundtrip`), the io `xml.dict` (SVG, OpenRaster, Krita, XMP and Camera Raw names), the engine `xml.dict` (.kys and XMP) and `json.dict` (preferences keys and JSON edge values), `plugin.dict` (manifest keys, the WebAssembly magic and ABI export names), `icc.dict` (ICC signatures) and `lut.dict` (.cube, .3dl and .look keywords). The workflow's matrix names each target's dictionary; locally:

```sh
cd crates/cms
cargo fuzz run icc_profile -- -dict=fuzz/dicts/icc.dict
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

Priority additions are PSB-specific structures, native directory bundles, pattern parsers, embedded smart objects, workspaces (read by the desktop shell), the JSON control protocol, IPP printer responses, and seeds for XCF and PDN. A fast replay of each stored corpus on pull requests remains future work.
