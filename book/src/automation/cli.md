# Command-line interface

`apps/photocraft-cli` provides the following current subcommands:

| Command | Purpose |
|---|---|
| `convert` | Open one document and export it to another format |
| `info` | Print document size, mode, depth, and layer information as JSON |
| `run` | Open or create a document, execute engine commands, and optionally save |
| `batch` | Apply an action list to files in one input directory |
| `droplet` | Run a PhotoCraft droplet against files or folders |
| `commands` | List command IDs and parameter documentation |
| `mcp` | Start the MCP server on stdio, optionally bridged to the desktop app |
| `serve` | Start the JSON-lines headless server on stdio or loopback TCP |

Use `photocraft-cli --help` (or `photocraft-cli <subcommand> --help`) as the executable source of truth. A typical bounded command run is:

```sh
photocraft-cli run input.psd \
  --cmd image.adjustments.invert \
  --out output.png
```

### Missing font warnings

If a type layer uses a font family that is not installed, `convert` and `run`
write a warning to standard error before exporting; they do not silently present
the fallback rendering as an exact match. `info --compact` includes the same
warning in its `warnings` JSON array, without writing it to stderr. `run`
also adds `warnings` to the JSON result for the command that first exposes
the missing family. A multi-command `run` reports each missing family once,
even if later commands still use it.

Warnings do not change the saved document's font declarations and do not stop
the operation. To list or replace the missing families intentionally, use
`type.resolveMissingFonts` on the document, or supply the fonts with `--font-dir`.

### Fonts that are not installed

`--font-dir <dir>` loads the font files in a folder and its subfolders (`.ttf`,
`.otf`, `.ttc`, `.otc`), or one font file, before anything opens. Type layers,
PSD text and `type.fonts` can then use them as well as the bundled and installed
fonts. `convert`, `info`, `run`, `batch`, `serve` and `mcp` take it, and it can
be given more than once:

```sh
photocraft-cli convert --font-dir brand/fonts poster.psd poster.png
```

A missing path, a folder without font files, or a folder none of whose files
load is an error, and nothing else runs. A file that can't be loaded is skipped
with a warning on standard error. Each folder is limited to 4096 files and 1 GiB,
and a file over 256 MiB is skipped. With `mcp --bridge` the flag is an error:
the fonts would load into `photocraft-cli`, not the app.

For output that doesn't depend on the machine (CI jobs, containers), set
`PHOTOCRAFT_NO_SYSTEM_FONTS=1` so that only the bundled fonts and the
`--font-dir` fonts are used.

`--font-dir` is launch configuration, like the automation roots, so it isn't
limited to the read root. A `serve` or `mcp` client can add fonts from under
the read root with the `type.registerFonts` command (`{"path": "fonts"}`).

### Export selected adjustment layers as a .cube LUT

The engine command `file.export.colorLookupTables` creates a 3D LUT from an
identity RGB lattice. It can export all visible top-level adjustment layers
(the default), the current Layers panel selection, or a supplied list of
adjustment layer IDs. The `scope` and `layers` parameters are mutually
exclusive when `scope` is `selected`:

```json
{"size": 33, "scope": "selected", "path": "chosen-look.cube"}
```

For automation that does not use the Layers panel, supply `"layers": [10, 14]`
instead. Input ID order does not change the final color look: adjustments are
always baked in **document stacking order**, bottom to top. When `path` is
omitted, the command returns the .cube text in `cube`; both forms return
`size` and `layerCount`.

Valid LUT dimensions are 2–129 (same maximum as PhotoCraft's LUT reader).
Selected IDs must exist, be visible top-level adjustment layers, and contain
no duplicates; invalid selections report an error rather than silently
exporting a different look. Masks, vector masks and clipping are spatial
operations and cannot be represented faithfully by a color-only LUT, so the
baker omits them. Group-contained adjustments are not exported as independent
layers because that would lose their group's blending semantics.

Paths supplied to the current CLI are ordinary operating-system paths. The CLI does not restrict them to a workspace root. Batch output creates the requested output directory, and save operations may create parent directories. `batch` refuses an output directory that is its input directory (the results would replace the originals) unless `--in-place` is given. Run untrusted action files only in an appropriately isolated account or environment.

CLI parsing reports malformed flags and JSON as errors, and a flag the subcommand doesn't take (a typo such as `--fromat`) is a usage error (exit status 2) rather than being ignored. It is not a security policy layer. Engine commands remain responsible for validating their own parameters.
