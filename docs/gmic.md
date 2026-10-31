# G'MIC filters

[G'MIC](https://gmic.eu) is a free, open-source image-processing program with more than a
thousand filters, the same set its G'MIC-Qt plug-in offers in GIMP and Krita. PhotoCraft runs the
**`gmic` command-line program installed on the computer**. It does not ship, link or bundle G'MIC.
G'MIC is C++ under the CeCILL licences, and PhotoCraft stays Rust-only, `unsafe`-free and
MIT/Apache-licensed, so the two talk only through files and a child process. No G'MIC source,
filter definition or library is in this repository.

This page covers using G'MIC from the app and automation, and the host (`crates/gmic`, the
`gmic.*` commands in `crates/engine/src/cmd/gmic_cmds.rs`, Filter › G'MIC and About Plug-ins in
`crates/ui-egui/src/menus/plugins_ui.rs`). Sandboxed WebAssembly plug-ins are a separate mechanism,
described in [plugins.md](plugins.md).

## Using G'MIC

Install G'MIC, then use **Filter › G'MIC**. The first time, PhotoCraft asks G'MIC for its filter
list. This takes about a minute and is then cached per G'MIC version. Each filter appears by
category with a generated dialog and live preview. **G'MIC Command…** runs any G'MIC pipeline.

| Command | Params | What it does |
|---|---|---|
| `gmic.status` | `{"refresh": bool = false}` | `{found, path, version, error, catalog, filters, catalogError, available}`. `refresh` looks for G'MIC again. |
| `gmic.list` | `{"refresh": bool = false, "wait": bool = false, "category": text}` | `{gmicVersion, categories, filters: [{id, name, category, command, params, defaults}]}`. Loads in the background and answers `{"loading": true}` until ready, unless `wait`. `refresh` rebuilds the list. |
| `gmic.filter` | `{"id": "Deformations/Kaleidoscope [Polar]", "params": {…}}` (filter params may also be top-level keys) | Runs a catalogue filter on the active pixel layer, targeted alpha channel / Quick Mask, or smart object. One undo step. |
| `gmic.run` | `{"command": "fx_smooth_anisotropic 60"}` | Runs a raw G'MIC pipeline on the active pixel layer or targeted channel. One undo step. |

Filter parameters use the keys `gmic.list` reports. Numbers are clamped to their range, choices
take the option label or its index, colours take `"#rrggbb"` or `[r, g, b(, a)]` (0–255), and a
point is two keys, `<name>X` and `<name>Y`, in percent. Missing keys take G'MIC's defaults. A value
of the wrong type is an error, and nothing is changed.

```json
{"method": "engine.execute", "params": {"command": "gmic.filter",
  "params": {"id": "Deformations/Kaleidoscope [Polar]", "angleCut": 25, "boundary": "Mirror"}}}
```

Runs started as background jobs (`Session::start`) can be cancelled; cancelling kills the G'MIC
process.

### Filter › G'MIC and About Plug-ins

G'MIC lives in the Filter menu, where Photoshop users find third-party filters, right after
Filter › Plug-ins (the WebAssembly plug-ins). **Filter › G'MIC** appears once PhotoCraft finds
G'MIC. It lists the filters by category, then G'MIC Command…, Refresh Filters and About G'MIC….

**About Plug-ins…** (at the end of Filter › Plug-ins, and in **Help**) explains both kinds of plug-in and lists what is
installed. It shows whether G'MIC was found and, if not, how to install it on this platform. On
Linux it reads `/etc/os-release` and suggests the matching command (`pacman`, `apt`, `dnf` or
`zypper`); on macOS it suggests Homebrew; on Windows it points to the command-line zip on gmic.eu.
It also offers **Download G'MIC…** (opens gmic.eu), **Locate G'MIC…**, **Check Again**, and links
to this page and [plugins.md](plugins.md). PhotoCraft never downloads or installs G'MIC itself.

## Finding the program

In order:

1. Edit › Preferences › Plug-ins › **G'MIC executable** (`plugIns.gmicPath`; Locate G'MIC… sets it);
2. the `PHOTOCRAFT_GMIC` environment variable;
3. `gmic` (`gmic.exe` on Windows) on `PATH`;
4. on macOS and Linux, `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, `/opt/local/bin` and
   `/snap/bin` (apps started from the Finder don't get the shell's `PATH`).

The candidate must answer `gmic -v - echo ${-strver}` with a version, or it is reported as not a
working G'MIC. The desktop app looks for G'MIC in the background when it first draws the menu bar.
Tests, the CLI and the web build do not look unless a command asks.

## How a run works

Each run uses a fresh temporary folder, removed afterwards:

- `in.cimg`: the canvas area of the layer, in G'MIC's own uncompressed `.cimg` format (float32,
  planar);
- `run.gmic`: the pipeline as the body of a one-command G'MIC file
  (`photocraft_run :\n  <pipeline>`), so G'MIC parses it exactly once, as script;
- `gmic -v - run.gmic in.cimg photocraft_run output out.cimg,float32`, started in that folder with
  no stdin. Both output pipes are drained (the last 64 KiB kept for error messages);
- `out.cimg`: read back with every size checked.

G'MIC sees the whole canvas, as in G'MIC-Qt, so position-dependent filters line up. The selection
then limits and feathers the result exactly as for built-in filters:
`out = original + (filtered − original) × coverage`. Lock Transparency keeps the layer's alpha.

**Pixels and bit depth.** Samples are sent as float32 in G'MIC's 0–255 convention at every bit
depth, so 16-bit and 32-bit documents keep their precision. Results are clamped to 0–1 at 8 and 16
bits and for alpha; 32-bit results are not clamped. NaN or infinite results keep the original
value. Only RGB and Grayscale documents (with or without alpha) are supported; convert other modes
with Image › Mode first. If a filter drops alpha, the layer keeps its own. A grey result on a
colour layer fills every colour channel. A filter that changes the image size is refused, because
filters that resize the image aren't supported yet.

**The filter list** comes from G'MIC itself: `gmic update` (G'MIC downloads its current filter
definitions from gmic.eu into its own configuration folder, as G'MIC-Qt does), then
`gmic parse_gui json`. The JSON (`gmic_json_1.0`) is parsed into filters with typed parameters.
Parameters that are only decoration (notes, separators, links) are dropped. Information pages
(command `_none_`) and commands that aren't plain names are skipped. The result is cached as
`catalog-<version>.json` in the platform cache folder: `~/.cache/photocraft/gmic` (Linux),
`~/Library/Caches/photocraft/gmic` (macOS), `%LOCALAPPDATA%\photocraft\gmic` (Windows). Offline
is fine once G'MIC has downloaded its definitions. A G'MIC that never could returns no filters,
with an error saying so; raw commands still work.

## Limits

| Limit | Value | On violation |
|---|---|---|
| Filter run time | 300 s | the process is killed; `took longer than 300 s` |
| Filter list rebuild | 600 s (+ 60 s for `gmic update`) | the process is killed; error |
| Image sent | ½ Gi samples (e.g. 36 MP RGBA is 144 Mi) | error before anything runs |
| Result read back | 1 Gi samples, 2D only, uncompressed | error; the document is unchanged |
| Filter list | 64 MiB, 20,000 filters, 256 parameters each | error / extra entries ignored |
| Pipeline | 1 MiB, no NUL | error |
| Text parameters | 64 KiB | error |

Every failure is an error; nothing a G'MIC run does can panic PhotoCraft. Failures include a
missing program, a crash, a non-zero exit (G'MIC's own `*** Error ***` message is reported), a
timeout, missing or malformed output, and a changed size. The edit is transactional: the document
changes only when the run succeeds.

## Security

G'MIC script is a programming language. It can run other programs and read and write files.
PhotoCraft therefore treats raw pipelines as code the user chose to run:

- **`gmic.run` is never recorded in a document.** On a smart object it is refused. Opening a PSD
  or `.pcraft` file never runs a pipeline it carries.
- **Smart filters record only catalogue filters** (`gmic.filter` with an id and parameter values).
  On re-render the command line is rebuilt from the installed catalogue. The command must be a
  plain G'MIC command name, and every text value is quoted with `"`, `\`, `$`, `{` and `}` escaped,
  so a value can't end its string or trigger substitution. Filters with file or folder parameters
  are not re-run from smart filters. If G'MIC or the filter is missing, the smart filter is skipped
  and the cached pixels stay, like other filters PhotoCraft doesn't implement.
- Recorded actions and automation can run `gmic.run`, like any other command. The MCP and control
  channels apply their usual authorization.

## Not in this version

- Filters that change the image size, take several layers (G'MIC-Qt's input modes) or produce
  several output layers: the first output image is used, and size changes are refused.
- G'MIC-Qt's preview commands, zoom and split previews: previews run the filter itself on the
  proxy image.
- CMYK, Lab, Indexed and other modes.
- Translated G'MIC filter names (G'MIC's own localised filters are listed as G'MIC names them).
- The web build (it cannot start programs).

## Verification

The unit and integration tests need no G'MIC. They cover:

- `.cimg` round trips and malformed files;
- catalogue parsing, parameter validation and quoting, using a hand-written `parse_gui`-shaped
  fixture;
- the runner, against stand-in `gmic` shell scripts on Unix: errors, timeouts, cancellation,
  output floods, and the temporary folder being removed;
- the engine commands, including undo, the selection and smart filters;
- the Filter › G'MIC and About Plug-ins menu items.

```sh
cargo test -p photocraft-gmic -p photocraft-engine -p photocraft-ui-egui
```

Checks against a real G'MIC are opt-in. They use `gmic` from `PATH`, or `PHOTOCRAFT_GMIC`. The
catalogue test needs network the first time G'MIC downloads its definitions.

```sh
GMIC_TEST=1 cargo test -p photocraft-gmic --test runner -- --ignored --nocapture
```

On 2026-10-11, with G'MIC 4.0.5 (Arch Linux package), both passed:

- a `.cimg` round trip, `mirror x add 100`, quoting, error capture, and `fx_kaleidoscope` on a
  16-bit RGBA layer;
- the full catalogue: 1,218 filters in 52 categories. Every filter's default command line is well
  formed, and a cached reload took under 5 s.

A filter run costs about 0.1 s of process start-up and stdlib loading on top of the filter itself.
Building the catalogue the first time took 43 s.
