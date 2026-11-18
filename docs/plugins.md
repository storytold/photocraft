# Plug-ins

PhotoCraft does not host native Photoshop plug-ins (`.8BF`, CEP, UXP): that needs unsafe FFI into
arbitrary machine code and cannot work in the browser build. Instead, plug-ins are **WebAssembly
modules** run in a sandbox. One module works on macOS, Windows, Linux and the web.

PhotoCraft runs two kinds:

- **Filters** (`"kind": "filter"`, PhotoCraft's own, ABI v1 below): change the pixels of the
  active layer, a targeted channel or a smart object, with live preview. Host: `crates/plugins`.
- **Command plug-ins** (`"kind": "command"`, shared by every Craft app): act on the document only
  through PhotoCraft's commands, the same ones the menus, the CLI, the control channel and MCP
  run. Host: the shared [`craft-plugin`](https://github.com/storytold/craft-libs/tree/main/crates/plugin)
  crate ([RFC-0046](https://github.com/storytold/craft-contrib/blob/main/rfcs/0046-shared-plugin-runtime.md)),
  whose README documents the manifest (`craft-plugin.json`), the bundle and the ABI.

The engine side is `crates/engine/src/plugin.rs` (rules, runs, installing) and the `plugin.*`
commands in `crates/engine/src/cmd/plugin_cmds.rs`.

## Using plug-ins

In the app: **Filter › Plug-ins** lists the installed filters, then each command plug-in's
commands (a generated dialog asks for their parameters), then **Install Plug-in…** (a `.wasm`
filter by path) and **Manage Plug-ins…**: the installed plug-ins, Remove, Install from File… (a
`.craftplugin` bundle or a `.wasm` filter), install from a GitHub repository (`owner/repo` or
`owner/repo@tag`), catalogs (add, remove, browse and install their packages) and Check for
Updates.

Installing a bundle always shows a **confirmation** first: the plug-in's name, version, author,
kind, the permissions it asks for and the commands it may call, the menu items it adds, where it
comes from, its SHA-256 and its licence. A plug-in from a repository that no catalog you use lists
says so plainly ("not in any catalog you use"). Nothing is written before **Install**. An update
that asks for other permissions is confirmed again.

| Command | Params | What it does |
|---|---|---|
| `plugin.list` | `{}` | `{"plugins": [filters {id, name, version, kind, author, description, params, paramsSchema, overlap, area, size, source, sha256?, origin?, stored?}, then command plug-ins {id, name, version, kind, permissions, commands, sha256, origin, stored}], "folder": report, "problems", "catalogs", "store", "last"}` |
| `plugin.install` | `{"path": "/x/y.wasm"}` (native) or `{"data": "<base64>"}`, `"replace": bool = true` | A bare filter module: validates and installs it for the process; returns its description. |
| `plugin.install` | `{"path": "/x/y.craftplugin"}`, `{"data": "<base64 bundle>", "name"?}`, `{"repo": "owner/repo[@tag]"}` or `{"catalog": "<index.json URL>", "id", "version"?}`; `"review"?`, `"sha256"?`, `"replace"?` | A bundle (either kind). `review: true` checks and describes it without installing (the confirmation); `sha256` installs only that reviewed bundle, without downloading it again. |
| `plugin.update` | `{"id"?, "apply"?: bool}` | Checks each bundle plug-in's catalog or repository; `apply` installs the updates that ask for no other permissions. |
| `plugin.remove` | `{"id": "…"}` | Uninstalls (a bundle leaves the Plug-ins folder, with its settings). |
| `plugin.run` | `{"id": "…", "params": {…}}` (plug-in params may also be top-level keys), `"command"?`, `"fuel"?` | A filter runs on the active pixel layer, targeted alpha channel / Quick Mask, or smart object. A command plug-in runs `command` (default: its first). Either is one undo step. |
| `plugin.reload` | `{"path": "folder"}` (default: the preference folder) | Loads every `*.wasm` in a folder; a bad module is reported and skipped. |
| `plugin.catalog.list` | `{"url"?}` | The packages PhotoCraft can run in one catalog, or in every catalog added. |
| `plugin.catalog.add` | `{"url", "remove"?: bool}` | Adds (or removes) a catalog: an `index.json` on GitHub's hosts over `https://`, or `file://`. |

### Where plug-ins are kept

- **Bundles** (both kinds) go to the per-user store, `Plug-ins/` in PhotoCraft's settings folder
  (`PHOTOCRAFT_CONFIG_DIR`, the portable `PhotoCraftData`, or the platform's), one folder per
  plug-in named by a hash of its id, with the bundle and a record of where it came from and its
  SHA-256 (craft-plugin's `store`). The app reads it at startup: filters found there join
  Filter › Plug-ins. The CLI, MCP and the web build install bundles for the session only.
- **Bare `.wasm` filters** are installed per process, as before: `plugin.install` with a `.wasm`
  path or base64 module, or the preference folder. Edit › Preferences › Plug-ins › *Use Additional
  Plug-ins Folder* plus *Additional Plug-ins Folder*: when both are set, every `*.wasm` in that
  folder (not recursive) is installed at startup and whenever the preference changes (native builds
  only). That folder is still read as it was; nothing is moved out of it.
- **Catalogs and plug-in settings** are kept with the preferences, in the `[plugins]` section of
  the settings file.

Plug-ins are installed per process, not per document. A smart filter recorded from a filter
plug-in (`{"command": "plugin.run", "params": {"id", "params"}}`) re-runs on re-render while that
plug-in is installed; if it is missing, the filter is skipped (the cached pixels stay as they
were), like other filters PhotoCraft does not implement.

### Installing from GitHub and catalogs

Downloads are `https://` on GitHub's hosts only and are checked before anything is kept: size
limits, the SHA-256 (pinned by the catalog, or from the release's `SHA256SUMS` or GitHub's asset
digest), the manifest, the API version, the `apps` entry (a bundle must name `photocraft`), and the
module compiled but not run. The desktop app downloads with the client it checks for updates with;
the CLI and MCP have no network access for plug-ins. Automation (MCP, the control channel) cannot
read `file://` catalogs, cannot install from a path outside its file access, and installs for
the session only, never into the Plug-ins folder.

## Command plug-ins

A command plug-in's manifest names PhotoCraft in `apps` with the shared command ABI it was written
for (1–6), asks for `read` or `write` access and may narrow the commands it calls:

```json
{
  "id": "io.github.example.tidy-layers",
  "name": "Tidy layers",
  "version": "1.0.0",
  "main": "tidy.wasm",
  "kind": "command",
  "apps": { "photocraft": { "api": 6 } },
  "permissions": { "access": "write", "commands": ["layer.*"] },
  "commands": [{ "id": "run", "label": "Tidy Layers", "params": [] }]
}
```

- **One undo step.** A run's commands become one history step named after the plug-in (and the
  command, when it has several). A run that fails (a trap, an exhausted budget, a non-zero status)
  puts the document, its targeted layers and the history back as they were, and none of its calls
  is journaled. A run that changed nothing leaves no step. A recorded action records the
  `plugin.run`, not its calls.
- **Read access.** Every call runs as its own nested step; when a `read` plug-in's call changed the
  document (its selection included), the call is taken back and refused with "… may only read it".
- **Refused commands.** Whatever the manifest says, a plug-in may not call: `plugin.*`, `file.*`,
  `app.*`, `pref.*`, `window.*`, Undo and Redo (craft-plugin's shared list), and PhotoCraft's
  preferences and settings dialogs (`prefs.*`, `edit.preferences*`, Color Settings, shortcuts,
  menus, toolbar, presets), the history (`history.*`, step backward/forward, purge), other
  documents (`document.activate`, `document.move`, document presets and recent files, Duplicate),
  actions, scripts and batches (`actions.*`, `automate.*`), view, workspace, panels, tools and the
  interface, the clipboard (copy, cut, paste), the user's preset libraries (brushes, swatches,
  gradients, patterns, styles, shapes: listing and applying stay), the session and its jobs, and
  sharing. Parameters that name a file or folder
  (`path`, `folder`, `file`, `url`, a profile given as a file…) or another document (`document`,
  `sourceDocument`…) are refused for every command. A call that opens, closes or switches
  documents is taken back and refused.
- **Settings.** A plug-in keeps its own settings (JSON values, keys up to 128 characters, 64 KiB in
  all), saved with the preferences and removed with the plug-in.
- **Not yet:** plug-in panels (`ui_show`; the run says so in its result) and `saveFile`.

## The sandbox (filters)

The host is [`wasmi`](https://github.com/wasmi-labs/wasmi), a pure-Rust WebAssembly interpreter
(MIT OR Apache-2.0). It builds for `wasm32-unknown-unknown` too, so plug-ins also run in the web
build. Limits (`photocraft_plugins::Limits`, defaults):

| Limit | Default | On violation |
|---|---|---|
| Host imports | none: no WASI, file system, network, clock or randomness | module rejected at install |
| Module size | 32 MiB | rejected |
| Linear memory | 512 MiB per instance | `memory.grow` returns -1; oversized initial memory fails |
| Linear memory per run | 2 GiB across parallel bands (4 instances at the 512 MiB cap) | fewer bands run at once |
| Instructions (fuel) | 50 M per call + 4,000 per sample for `pc_filter` | `Limit("instruction budget")` |
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

## Filter ABI v1

A filter is a core WebAssembly module (MVP + the usual post-MVP features: bulk memory, sign
extension, multi-value, saturating conversions; no SIMD, no memory64) with **no imports** and these
exports. All integers are `i32` unless noted; pointers are offsets into the module's memory.

| Export | Signature | Meaning |
|---|---|---|
| `memory` | memory | The module's linear memory. |
| `pc_abi_version` | `() -> i32` | Must return `1`. |
| `pc_manifest` | `() -> i64` | `(len << 32) \| ptr` of the UTF-8 manifest JSON (≤ 64 KiB). |
| `pc_alloc` | `(size) -> ptr` | A block of `size` bytes the host may write, or `0` on failure. Never freed: each band gets a fresh instance. |
| `pc_filter` | `(buf, buf_len, width, height, channels, format, params, params_len) -> i32` | Filters the pixels in place; `0` = success, anything else is an error code. |

### Manifest

A filter's manifest is embedded in its module (`pc_manifest`). In a `.craftplugin` bundle it also
has a `craft-plugin.json` with `"kind": "filter"` and `"apps": {"photocraft": {"api": 1}}`, whose
`id` must be the module's own.

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
  Invisible padding at the ends (white space, joiners, Hangul fillers, blank Braille and other
  default-ignorable blanks, and variation selectors cut off by them) is removed before these checks,
  so it does not count towards the limit; marks that belong to a kept character stay.
  `kind`: `"filter"` (the only kind in v1).
- `params`: in dialog order. Names are `[A-Za-z0-9_]`, not starting with `_`, not `id`/`layer`.
  The host validates the user's values before calling: missing values take the default, numbers are
  clamped to `min..max` (ints rounded), a wrong type or an unknown choice is an error.
- `unit` (optional, on `number` and `int` params): `"px"` marks a length in document pixels (a
  radius, distance, shift or size); `"none"`, the default, is everything else (levels, factors,
  counts, percentages). Any other value (an unknown string, a number…) is treated as `"none"`, as
  when the field is absent, so older manifests keep installing. A filter dialog's live preview on a
  large document runs on a reduced copy (Preferences › Performance › Low Resolution Previews), and
  the host divides each `px` param by that reduction (ints rounded, kept within `min..max`) so the
  preview shows the same lengths OK applies; when a `px` param with a positive `min` would drop below
  it, the preview runs less reduced instead. Unitless params reach the preview unchanged, whatever
  their name. OK, actions and smart filters always get the values as given.
- `overlap` (0–256): pixels of context the filter needs around each output pixel (a blur radius).
  The host pads each band by `overlap` on every side, repeating the edge pixels of the canvas (or of
  off-canvas layer content), and keeps only the inner rectangle of the result.
- `area`: `"content"` (default) filters the layer's non-transparent bounds grown by `overlap`;
  `"canvas"` filters the whole canvas, including transparent pixels (for generators).

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

The host converts the layer into bands (tile-aligned blocks of at most ~4 MiB of `f32`), runs them in
parallel on native builds, writes the results back in the layer's format, then blends with the
selection: `out = original + (filtered − original) × coverage`, exactly as built-in filters do. A
plug-in never sees the selection and needs nothing special to support it.

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
