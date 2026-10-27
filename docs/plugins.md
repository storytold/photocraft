# Plug-ins: the WebAssembly plug-in API (ABI v1)

PhotoCraft does not host native Photoshop plug-ins (`.8BF`, CEP, UXP): that needs unsafe FFI into
arbitrary machine code and cannot work in the browser build. Instead, plug-ins are **WebAssembly
modules** run in a sandbox. One `.wasm` file works on macOS, Windows, Linux and the web.

This page is the contract for plug-in authors and the reference for the host
(`crates/plugins`, the `plugin.*` commands in `crates/engine/src/plugin_cmds.rs`).

## Using plug-ins

| Command | Params | What it does |
|---|---|---|
| `plugin.install` | `{"path": "/x/y.wasm"}` (native) or `{"data": "<base64>"}`, `"replace": bool = true` | Validates and installs a module; returns its description. |
| `plugin.list` | `{}` | `{"plugins": [{id, name, version, kind, author, description, params, paramsSchema, overlap, area, size, source}], "folder": report}` |
| `plugin.run` | `{"id": "…", "params": {…}}` (plug-in params may also be top-level keys) | Runs a filter plug-in on the active pixel layer, targeted alpha channel / Quick Mask, or smart object. One undo step. |
| `plugin.remove` | `{"id": "…"}` | Uninstalls. |
| `plugin.reload` | `{"path": "folder"}` (default: the preference folder) | Loads every `*.wasm` in a folder; a bad module is reported and skipped. |

In the app, installed filters appear under **Filter › Plug-ins** (with a generated dialog and live
preview when they have parameters), next to **Install Plug-in…**.

**Preference folder.** Edit › Preferences › Plug-ins › *Use Additional Plug-ins Folder* plus
*Additional Plug-ins Folder*: when both are set, every `*.wasm` in that folder (not recursive) is
installed at startup and whenever the preference changes (native builds only; the web build installs
from bytes).

Plug-ins are installed **per process**, not per document. A smart filter recorded from a plug-in
(`{"command": "plugin.run", "params": {"id", "params"}}`) re-runs on re-render while that plug-in is
installed; if it is missing, the filter is skipped (the cached pixels stay as they were), like other
filters PhotoCraft does not implement.

### Presets

Every plug-in dialog has a **Presets** row: a list of saved names (choosing one applies it and
updates the live preview), **Save As…** and **Delete**. Plug-ins get this automatically; there is
nothing to declare and the ABI is unchanged.

The desktop app keeps each plug-in's presets in one file named after its id, in a `plugins` folder
inside the settings folder (`PHOTOCRAFT_CONFIG_DIR`, the portable data folder, or
`~/.config/photocraft` on Linux, `~/Library/Application Support/Photocraft` on macOS,
`%APPDATA%\Photocraft` on Windows), for example
`~/.config/photocraft/plugins/com.stack-tech.freecandy7.json`. Capital letters in an id are written
as `%XX` (`Foo` → `%46oo`), so ids that differ only in case never share a file:

```json
{
  "version": 1,
  "presets": {
    "Soft glow": { "amount": 25.0, "mode": "screen", "center": [50.0, 40.0] }
  }
}
```

- **Plug-ins have no file access.** The host reads and writes this file on the plug-in's behalf;
  the plug-in never sees the path, the file or other plug-ins' presets, and only ever receives
  parameter values through `pc_filter` as before.
- The file name comes only from the validated plug-in id; ids with `..`, a leading dot or a path
  separator get no presets. Preset names are 1–64 printable characters without `/`, `\` or `..`.
- Saves are atomic (a temporary file in the same folder, then a rename).
- A missing, unreadable or damaged file just means no presets (a warning is logged). Only a
  missing or readable file is replaced on save: a damaged or unreadable file, or one with another
  `version`, is never overwritten (saving reports an error instead), so its presets aren't lost.
- Stored values are checked against the plug-in's current manifest when listed and applied:
  unknown keys and values of the wrong type are dropped, numbers and points are clamped into
  range, unknown choices are dropped (the parameter keeps its default), and a preset with nothing
  usable left is skipped while the rest still load.
- A plug-in may ship immutable factory settings in a top-level `presets` object mapping display names to parameter objects. These appear under **Built-in presets**, including when user storage is unavailable (such as the web build). With `presetSelector`, each built-in must explicitly include a supported selector value. Names and counts use the user-preset limits; values use the same normalization, bounds and schema filtering. The 64 KiB manifest limit still applies.
- A top-level `presetPreserve` array may name parameters that built-in recipes leave unchanged when they omit those parameters. For example, hair-style recipes can retain the user's chosen coat type and colors. An explicitly supplied value still takes precedence. These retained controls do not mark the selected built-in recipe Custom when edited. Keys must be unique, refer to existing parameters, and exclude `presetSelector`. Omitted controls outside this list still reset to their defaults within the current effect. **Your presets** remain complete saved settings and are not affected by this list.
- Stored settings appear under **Your presets** for the selected effect. **Save As…** always writes user settings, even when starting from a built-in. Identical names in the two sources remain distinct; **Delete** is available only for user settings. Without a user store, Save As and Delete are disabled.

## The sandbox

The host is [`wasmi`](https://github.com/wasmi-labs/wasmi), a pure-Rust WebAssembly interpreter
(MIT OR Apache-2.0). It builds for `wasm32-unknown-unknown` too, so plug-ins also run in the web
build. Limits (`photocraft_plugins::Limits`, defaults):

| Limit | Default | On violation |
|---|---|---|
| Host imports | none: no WASI, file system, network, clock or randomness | module rejected at install |
| Module size | 32 MiB | rejected |
| Linear memory | 1 GiB per native instance; 512 MiB on the web | `memory.grow` returns -1; oversized initial memory fails |
| Instructions (fuel) | 50 M per call + 8,000 per sample for `pc_filter`, with at least four samples of work allowed per pixel. Web only: desktop builds give `pc_filter` no instruction budget, so complex plug-ins are bounded by the wall clock and memory cap instead (a runaway plug-in stops after 60 s rather than at the budget) | `Limit("instruction budget")` |
| Wall clock | 60 s per run (native; checked every 20 M instructions) | `Limit("time budget")` |
| Call depth | 1,024 frames | trap |
| Module shape | wasmi's strict limits (≤ 10,000 functions, 1 memory, …) | rejected |

Every band runs in a **fresh instance**, so no state survives between calls, and parallel bands
never share memory. Traps, exhausted budgets, malformed modules, bad manifests, out-of-range
pointers and non-zero return codes all become errors; the document is left untouched (the edit is
transactional). When a failure stops sibling bands, those bands retain the originating error
instead of replacing it with a generic cancellation message. Output samples that are NaN or
infinite keep their original value; integer-depth
documents and alpha are clamped to 0–1.

wasmi is built with its *portable* (loop) dispatcher: the default tail-call dispatcher relies on LLVM
sibling-call optimisation, and when that doesn't happen a long-running module overflows the host
stack, which aborts the process instead of returning an error.

## ABI v1

A plug-in is a core WebAssembly module (MVP + the usual post-MVP features: bulk memory, sign
extension, multi-value, saturating conversions; no SIMD, no memory64) with **no imports** and these
exports. All integers are `i32` unless noted; pointers are offsets into the module's memory.

| Export | Signature | Meaning |
|---|---|---|
| `memory` | memory | The module's linear memory. |
| `pc_abi_version` | `() -> i32` | Must return `1`. |
| `pc_manifest` | `() -> i64` | `(len << 32) \| ptr` of the UTF-8 manifest JSON (≤ 64 KiB). |
| `pc_alloc` | `(size) -> ptr` | A block of `size` bytes the host may write, or `0` on failure. Never freed: each band gets a fresh instance. |
| `pc_filter` | `(buf, buf_len, width, height, channels, format, params, params_len) -> i32` | Filters the pixels in place; `0` = success, anything else is an error code. |
| `pc_overlap` (optional) | `(params, params_len) -> i32` | Required context in pixels for this run's resolved parameters. Replaces manifest `overlap` when present. |

The optional `pc_overlap` export extends ABI v1 without changing its version. Before allocating
pixel buffers, the host calls it once in a fresh instance with UTF-8 JSON containing the validated
parameters, including defaults. This JSON has no `_image` field: no band exists yet. The function
must return a deterministic non-negative radius that includes every source read and auxiliary
stencil used by those parameters. The host uses the same result for input padding, content/shape
output expansion, and `_image.overlap` in every filter band. No state from negotiation survives
into the fresh filter instances.

Native builds accept 0–1,536 pixels, supporting effects reaching 1,500 pixels with room for
sampling stencils. Web builds retain a maximum of 256 pixels. A negative or excessive result is
an explicit overlap-budget error; the host never clips a requested radius. An export with the
wrong signature is rejected at installation. Negotiation runs under the ordinary base fuel,
linear-memory and run-deadline limits. Plug-ins without the export keep their existing static
manifest behavior. Older hosts ignore this optional export, so plug-ins requiring it must check
the `_image.overlap` they receive and fail if it is insufficient.

### Manifest

```json
{
  "id": "org.example.tone",
  "name": "Tone…",
  "version": "1.0.0",
  "kind": "filter",
  "author": "Example",
  "description": "What it does.",
  "params": {
    "amount": {"type": "number", "min": 0, "max": 100, "default": 50},
    "shift":  {"type": "number", "min": 0, "max": 40, "default": 4, "unit": "px"},
    "steps":  {"type": "int", "min": 1, "max": 16, "default": 4},
    "mono":   {"type": "bool", "default": false},
    "mode":   {"type": "choice", "options": ["soft", "hard"], "default": "soft"}
  },
  "overlap": 0,
  "area": "content"
}
```

- `id`: 1–64 characters from `A-Z a-z 0-9 . _ -` (reverse-DNS recommended). Installing a module
  with the same id replaces the old one.
- `name`: the menu label (1–64 printable characters). Any script works, but control characters and
  Unicode format characters (bidi overrides and marks, zero-width spaces, soft hyphens) are rejected,
  except the zero-width joiner and non-joiner, and at least one character must be visible.
  `kind`: `"filter"` (the only kind in v1).
- `params`: up to 256 controls, in dialog order. Names are `[A-Za-z0-9_]`, not starting with `_`, not `id`/`layer`.
  Types: `number`, `int`, `bool`, `choice`, `point`, `color` and `angle` (below).
  The host validates the user's values before calling: missing values take the default, numbers are
  clamped to `min..max` (ints rounded), a wrong type or an unknown choice is an error.
- `group` (optional, any parameter type): the dialog tab the parameter sits on, 1–32 printable
  characters (the same rules as `name`). The dialog shows parameters without a `group` first,
  then one tab per group in the order each first appears; only the selected tab's
  parameters are shown (the first tab at first). Groups change only the layout: presets and the
  values the plug-in receives are the same.
  `"start": {"type": "point", "default": [12, 12], "group": "Bolt"}`
- `label` (optional, any parameter type): a dialog label of 1–64 printable Unicode characters,
  using the same visible-character and format-character rules as `name`. Leading and trailing
  spaces are trimmed; control characters are rejected even at the edges. Without a label,
  the dialog uses the parameter name. Labels do not change the keys the plug-in receives.
- `visibleWhen` (optional, any parameter type): an object of 1–8 equality predicates. Every
  predicate must match the current parameter values for the control to appear. Keys must name
  another `choice` or `bool` parameter; values must be one of the choice's option strings or a
  JSON boolean, respectively. A conditional controller is allowed only when the dependent
  repeats **all** the controller's visibility predicates with the same values. Self references
  and cycles are rejected. Hidden values are still validated, saved in presets, and sent to
  the plug-in. This metadata can be combined with `group`.
- `presetSelector` (optional, top level): names an unconditional `choice` parameter used by
  the dialog to scope presets to the selected effect. The selector's stable option values
  identify the preset scope; its parameter name remains an ordinary plug-in input.

For example, a suite can show a style selector for Swirl and a strength control only for
Swirl's Distort style. The strength control repeats the style selector's `effect` predicate:

```json
"presetSelector": "effect",
"params": {
  "effect": {"type": "choice", "options": ["swirl", "glow"], "label": "Effect"},
  "swirl_style": {
    "type": "choice", "options": ["distort", "rays"], "label": "Style",
    "visibleWhen": {"effect": "swirl"}
  },
  "strength": {
    "type": "number", "min": 0, "max": 100, "default": 50, "label": "Strength",
    "visibleWhen": {"effect": "swirl", "swirl_style": "distort"}
  }
}
```

Manifests that omit these optional fields keep the existing dialog and preset behavior.

- `unit` (optional, on `number` and `int` params): `"px"` marks a length in document pixels (a
  radius, distance, shift or size); `"none"`, the default, is everything else (levels, factors,
  counts, percentages). Any other value (an unknown string, a number…) is treated as `"none"`, as
  when the field is absent, so older manifests keep installing. A filter dialog's live preview on a
  large document runs on a reduced copy (Preferences › Performance › Low Resolution Previews), and
  the host divides each `px` param by that reduction (ints rounded, kept within `min..max`) so the
  preview shows the same lengths OK applies; when a `px` param with a positive `min` would drop below
  it, the preview runs less reduced instead. Unitless params reach the preview unchanged, whatever
  their name. A plug-in that marks no param `px` previews at full resolution instead, since its
  lengths can't be scaled. OK, actions and smart filters always get the values as given.
- `overlap` (0–1,536 on native builds; 0–256 on the web): static pixels of context the filter needs
  around each output pixel (a blur radius), or the fallback for hosts without `pc_overlap`.
  The host pads each band by `overlap` on every side, repeating the edge pixels of the canvas (or of
  off-canvas layer content), and keeps only the inner rectangle of the result. When `pc_overlap`
  is present, its per-run result replaces this value throughout the run.
- `area`: `"content"` (default) filters the layer's non-transparent bounds grown by `overlap`;
  `"canvas"` filters the whole canvas, including transparent pixels (for generators).
- `clampContext`: optional boolean, default `false`. A plug-in that understands the output
  rectangle described below may opt into clipping source context to the image extent plus a
  36-pixel boundary guard for native overlaps above 256 pixels. This avoids processing large
  repeated margins beyond the image. The guard preserves edge stencils: source colour repeats
  at the extent, while selection coverage remains zero outside it. Existing plug-ins and
  ordinary overlaps of 256 pixels or less retain symmetric, untrimmed margins.
- `clampContextWhen`: optional choice scope, using the same `{ "parameter": "effect", "values":
  ["fire", "drip"] }` structure and validation as `selectionShape`. When present, clipping is
  enabled only for a matching resolved choice and still requires `clampContext: true`. This lets
  suites retain full source margins for effects whose derived fields need distant edge context.

#### Selections as shape geometry

Ordinary filters receive the original layer pixels, and the selection clips and feathers their
output. An effect suite whose effects need a silhouette can opt in per choice:

```json
"selectionShape": {"parameter": "effect", "values": ["bevel", "shadow", "glow"]}
```

The parameter must be an unconditional `choice`; values must contain 1–64 unique supported
choices. When a selection is active and the resolved choice matches, the host multiplies the
input alpha by selection coverage. RGB/Gray inputs without alpha gain a temporary alpha channel
in the filter buffer; `channels`, `format`, and `_image.alpha` describe that actual buffer. The
output surface retains its original format. Other document colour models remain in their own
channel layout, with the same temporary-alpha rule.

The filter sees a cut-out of the selected photo, including feathering and existing layer alpha.
Its output is composited back into the photo, replacing the selected cut-out while preserving the
unselected remainder. This avoids doubling source alpha or applying feather coverage twice.
Effects may extend outside the selection by at most the resolved `overlap`, within the filter's
ordinary content/canvas output bounds; pixels beyond that region are unchanged. The selection
mask has zero coverage outside its bounds, including in the band's neighbourhood pixels.

`_image.shapeX`, `shapeY`, `shapeWidth`, and `shapeHeight` report the full selection's document
bounds, identical in every band. Existing canvas and band coordinates keep their usual meaning.
The extra geometry fields are omitted when shape selection is inactive. With no active selection,
the plug-in sees the layer's original alpha and the usual filtering behavior.

Preview and Apply dispatch the same engine command. Selected shape effects currently require a
pixel layer: smart objects reject the operation with an actionable error because their shared
smart-filter mask cannot retain per-effect input geometry. Clear the selection or rasterize the
smart object explicitly before applying one. Existing ordinary smart filters are unaffected.

#### Points on the canvas: `point` and `guide`

A `point` parameter is a position the user picks on the image. Its value is `[x, y]` in **percent of
the canvas width and height** (0–100), like the other position-style numbers, so it does not depend
on the document size. `default` is required: two numbers, each within 0–100.

```json
"params": {
  "start": {"type": "point", "default": [12, 12]},
  "end":   {"type": "point", "default": [88, 62]}
},
"guide": {"from": "start", "to": "end"}
```

- While the plug-in's dialog is open, every point shows a handle on the canvas. Pressing near a
  handle drags it; pressing elsewhere on the image moves the first point there. The dialog's X / Y
  fields and the live preview follow the handle as it moves.
- `guide` (optional) names two different `point` parameters: the canvas draws a dotted line between
  them. A manifest whose `guide` names a missing or non-`point` parameter is rejected.
- The plug-in receives the value as an array in `params`, e.g. `{"start": [12.0, 12.0], …}`. The
  host clamps both coordinates to 0–100; anything other than an array of two numbers is an error.
- In the registry notation (`plugin.list` `params`) a point reads `"start":point=[12,12]`.

#### Colours: `color`

A `color` parameter is an opaque colour the user picks with a swatch and colour picker. `default` is
required and must be a `"#rrggbb"` hex string (either case; no alpha, no `#rgb` short form); anything
else rejects the manifest.

```json
"params": {
  "inner": {"type": "color", "default": "#cfeaff"}
}
```

- The plug-in receives the same string, lowercase, in `params`: `{"inner": "#cfeaff"}`. A value that
  is not `"#rrggbb"` is an error. The live preview follows the picker as the colour changes.
- Saved presets keep the hex string; a stored colour that is not a valid `"#rrggbb"` is dropped.
- In the registry notation a colour reads `"inner":color=#cfeaff`.

#### Angles: `angle`

An `angle` parameter is a direction in degrees, set with a dial and a number field. `default` is a
number from 0 to 360 inclusive (0 when omitted); anything else rejects the manifest.

```json
"params": {
  "direction": {"type": "angle", "default": 45, "group": "Bolt"}
}
```

- 0 points right (east) and the angle grows clockwise on screen: 90 is down, 270 is up.
- The plug-in receives a plain number in `params`: `{"direction": 45}`, always within 0–360.
  Values outside that range wrap around (370 becomes 10, -90 becomes 270); a non-number is an error.
- Saved presets keep the number, wrapped the same way on load; a stored non-number is dropped.
- In the registry notation an angle reads `"direction":angle=45`.

### Pixels

`buf` holds `width × height × channels` **little-endian `f32` samples**, interleaved, row-major,
normalised to 0–1 at every bit depth (32-bit documents may hold values above 1). The buffer is
`width × height` including the overlap margins. Channels are the document's own colour model, with no
conversion:

`format` = `depth | alpha << 8 | mode << 16`

- `depth` (bits 0–7): the document's bit depth, 8, 16 or 32 (informational: samples are always `f32`).
- `alpha` (bit 8): 1 when the last channel is (straight, not premultiplied) alpha.
- `mode` (bits 16–23): the colour mode, numbered as in the PSD file format: 0 Bitmap, 1 Grayscale,
  2 Indexed (stored as RGB), 3 RGB, 4 CMYK, 7 Multichannel, 8 Duotone (stored as gray), 9 Lab.

Alpha channels and Quick Mask targets arrive as one grayscale channel.

The host converts the layer into bands (tile-aligned output blocks of at most ~4 MiB of `f32`), runs them in
parallel on native builds, writes the results back in the layer's format, then blends with the
selection: `out = original + (filtered − original) × coverage`, exactly as built-in filters do. A
normal filter needs nothing special to support selection clipping; an opted-in shape effect
receives the masked silhouette described above.

Context margins are additional to the ordinary output-band target. Band width shrinks when
necessary to fit the memory budget; an impossible minimum band fails before pixel allocation.
For overlaps above 256 pixels, runs use larger tile-aligned rectangles in both dimensions,
sized against the actual input, clipped when the plug-in opts in for that choice. This amortizes large neighborhoods
across useful output pixels; it does not change the requested radius or effect geometry.

Large-context runs reserve 16 MiB for module/allocator overhead, then limit input to one third
of the remaining linear-memory cap, leaving room for source copies and intermediate fields.
They use one worker and bound the estimated active memory to 2 GiB, counting the full instance
cap and up to three host pixel buffers. Ordinary overlaps keep the existing band target and
parallelism. Caller-supplied lower memory limits remain lower. These are planning budgets, not
extra memory granted to the plug-in: the per-instance cap and 60-second native deadline still
apply. Large effects must keep their own scratch allocations and work bounded; they can fail
explicitly if those resource limits are exhausted.

### Parameters

`params` points at `params_len` bytes of UTF-8 JSON: the validated parameters, plus `_image` with
the band's geometry:

```json
{"amount": 50, "mode": "soft",
 "_image": {"x": 0, "y": 256, "width": 1024, "height": 256, "overlap": 0,
            "canvasWidth": 6000, "canvasHeight": 4000,
            "mode": "rgb", "depth": 8, "alpha": true, "channels": 4}}
```

`x, y` is the document position of the buffer's top-left sample (including the margins), so
position-dependent filters (gradients, vignettes) are seamless across bands.

For a clipped `clampContext` run, `_image` also includes `outputX`, `outputY`, `outputWidth` and
`outputHeight`. These are the output rectangle relative to the supplied buffer, in pixels.
The plug-in must use this rectangle instead of assuming `overlap` pixels on every side; only
this rectangle is retained by the host. `_image.overlap` still reports the requested dependency
radius even when context at an image edge was trimmed. For example, a whole 64 × 64 image with
300 pixels of requested context receives a 136 × 136 buffer at `x: -36, y: -36`, and its output is
`outputX: 36, outputY: 36, outputWidth: 64, outputHeight: 64`. When these fields are absent, the
original symmetric-margin contract applies. Sampling beyond a clipped input must clamp at its
edge; plug-ins needing explicit extended margins should leave `clampContext` disabled.

## Writing a plug-in in Rust

[`examples/plugins/invert-rs`](../examples/plugins/invert-rs) is a complete `no_std` plug-in
(no allocator, under 1 KiB built). Build it with:

```sh
rustup target add wasm32-unknown-unknown
examples/plugins/invert-rs/build.sh      # cargo build --release --target wasm32-unknown-unknown
```

The script also refreshes `crates/plugins/tests/fixtures/invert.wasm`, the test fixture built from
that source. Any language that targets core WebAssembly works (C, Zig, AssemblyScript, WAT…), as
long as the module has no imports: in Rust, that means `no_std` or a `std` build that never touches
WASI.

## Performance

The interpreter costs roughly 10–100× native speed per instruction, so plug-ins suit per-pixel and
small-neighbourhood filters. Because bands run in parallel, the example Invert on a 6000 × 4000
RGBA 8-bit layer (24 MP, release, 14-core Apple Silicon under heavy load) took 1.1–1.6 s through
`plugin.run`, versus 3.6–5.1 s for the (single-threaded) built-in Image › Adjustments › Invert
(`cargo run --release -p photocraft-engine --example bench_plugin_invert`; plug-in alone:
`cargo run --release -p photocraft-plugins --example bench_invert`). Peak extra memory is about three bands per worker thread (input,
original for the selection blend, and the instance's memory), i.e. ~12 MiB per thread.

## Not in v1

- Panel / UI plug-ins, plug-ins that read other layers or documents, host callbacks (progress,
  cancel), plug-in-provided file formats.
- Filter › Last Filter does not repeat plug-in runs yet.
