# MCP conventions

## Install and register

From the repository root, with the [source-build prerequisites](development.md#prerequisites)
installed, install the CLI independently of the build directory:

```sh
cargo install --path apps/photocraft-cli --locked
# POSIX shells, for Cargo's default install location:
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
photocraft-cli --help
photocraft-cli mcp --help
```

Cargo installs a binary, not a symlink into `target/`, so it survives `cargo clean`.
Re-run the install command after updating the checkout.
By default the executable goes in `$CARGO_HOME/bin` (`~/.cargo/bin` when unset), or
`%USERPROFILE%\.cargo\bin` on Windows. If you set `--root`, `CARGO_INSTALL_ROOT`, or
Cargo's `install.root`, use that installation root's `bin` directory instead. Add it
to the `PATH` inherited by the MCP client, not just an interactive shell, and restart
the client after changing its environment.

Register the installed command explicitly (replace `/work/project` with an existing folder):

```sh
claude mcp add photocraft -- photocraft-cli mcp --automation-read-root /work/project --automation-write-root /work/project
```

Other MCP clients can use this bare-name configuration:

```json
{"mcpServers":{"photocraft":{"command":"photocraft-cli","args":["mcp","--automation-read-root","/work/project","--automation-write-root","/work/project"]}}}
```

Linux `.deb` and `.rpm` packages already install `/usr/bin/photocraft-cli`; those users
can skip the source install. Other release locations are listed in the
[installed-release guide](../book/src/automation/mcp.md#from-an-installed-release).
Installing the CLI does not start or register an MCP server automatically.

If a client reports `ENOENT` or "Executable not found in $PATH", verify the install with
`command -v photocraft-cli` (PowerShell: `Get-Command photocraft-cli`) in the client's
environment. If that environment cannot be changed, set JSON `command` to the absolute
installed binary path, not a literal `~` or `$HOME` expression. Avoid symlinks into
`target/release`: cleaning the build directory breaks them. A source build alone does
not put the CLI on `PATH`.

## Start a session

PhotoCraft follows the same conventions as FilmCraft #28. Start a headless session:

```sh
photocraft-cli mcp --automation-read-root /work/project --automation-write-root /work/project
```

Paths are relative to the corresponding root, or absolute paths beneath it; without a root that
filesystem authority is absent. Bridge mode uses `--bridge` and `--control-token-file` as
described in [development.md](development.md). UI tools require a running desktop app.

A client has the full rights of the roots it is given: it can read any file below the read
root, and `doc_save`, `doc_export` and the export tools can create files anywhere below the
write root. `doc_save` and `doc_export` replace an existing file only when its extension is an
image or document format PhotoCraft writes (PNG, JPEG, TIFF, PSD, PSB, .pcraft, OpenRaster,
PDF, …) or its bytes are an image; anything else (`src/main.rs`, `.env`) is refused and left
untouched. The multi-file exports (layers, data sets) never replace a file. An agent reads
text from documents (layer names, metadata) that may try to steer it, so prefer a dedicated
output folder as the write root over a whole project.

Use absolute root paths in MCP client configurations, since a relative root is resolved from
the client's launch directory. In bridge mode, configure roots on the **desktop app**; root
flags on `photocraft-cli mcp --bridge` are rejected. Explicit `--control-token` and
`--control-token-file` flags require `--bridge` (or `PHOTOCRAFT_CONTROL_PORT`) and are mutually
exclusive. Bridge credentials can also come from `PHOTOCRAFT_CONTROL_TOKEN` or `PHOTOCRAFT_CONTROL_TOKEN_FILE`.

MCP and `serve` launch options (all but `--help`/`-h`) take one non-empty value each and cannot
be repeated. Missing values, stray positional arguments and options for the wrong mode are
usage errors (exit 2), reported on stderr before the server starts; stdout stays available for
protocol messages. For `serve`, explicit credential flags require `--port` (or
`PHOTOCRAFT_CONTROL_PORT`); stdio needs no control token.

## Tools and resources

- `command_list`: discover engine ids, parameter docs and current enabled state; accepts
  `filter` and `enabled_only`. Returns a bare JSON array of
  `{id, label, menu, shortcut, params, enabled, disabledReason, needsDocument}` objects (see
  [Command list replies](control-protocol.md#command-list-replies)).
- `command_run`: `id`, optional `params` and `wait`. Command parameters remain engine-defined.
- `command_batch`: ordered `steps`, optional `stop_on_error` (default true); stops at the first
  error unless false. Not an atomic transaction: earlier successful steps stay applied, with
  no rollback. Existing per-step results and reply budgets are preserved.
- `doc_inspect`: layer tree, history and selection; an empty session returns `document:null`.
  Every layer (including groups and nested children) reports raster-only `hasMask`,
  `hasVectorMask` for vector mask presence, and `vectorMaskEnabled` (`true`/`false` when
  present, `null` when absent). A disabled vector mask still has `hasVectorMask:true`.
- `render_preview`: bounded PNG content; `index` and `max_side` (default 1024, maximum 2048).
- `ui_inspect` and `ui_screenshot`: live app state/window in bridge mode.

Existing document/session/job/UI tools remain listed, including the documented
`doc_render_preview` spelling. There are no hidden legacy aliases. Every tool has a title
and all four annotation hints. Generic command/control tools conservatively advertise writes;
file saves may replace existing image and document files. Hints do not grant permission.

After a failed batch, `completed` and per-step `results` report successful steps, not a
rolled-back transaction. Inspect the document and its history with `doc_inspect`; commands
do not necessarily add one history entry each. For undoable edits, use `command_run` with
`{"id":"edit.undo","params":{}}` once per undoable history entry added, inspecting history
as you recover. Undo does not roll back files written to disk. If a reply-budget error says
the current step may have completed, inspect its effects before retrying or undoing it.

Before experiments, use `command_run` with `image.duplicate` and `{}` params, or
`history.newDocument` and `{}` params (copies the current state by default). Both create a
new active document, leaving the source unchanged; run experimental edits on that copy.
The optional `state` parameter of `history.newDocument` selects a retained historical state.

Unknown tool arguments return JSON-RPC `-32602`, naming the key, before execution. Batch
step envelopes are strict too. Nested engine `params` retain their command-specific handling.
Tool execution failures return `isError:true`. A worker panic becomes a tool error and the
existing poisoned-session recovery keeps serving. Invalid JSON lines receive `-32700` with
`id:null`; the next line is still processed.

Embedded Smart Objects support `command_run` with `layer.smartObjects.editContents` and
`layer.smartObjects.convertToLayers`, optionally targeting `params.layer`. Use
`layer.smartObjects.saveContents` in an opened contents document to update its parent in memory;
save nested contents from the inside outward, then `doc_save` the parent to a scoped path.
`doc_save` on a contents document exports that document rather than updating its parent.
No filesystem root is needed for the in-memory commands. External linked sources that require
a file read remain refused. See the [control policy](control-protocol.md#engine-commands)
for the shared restrictions.

`photocraft://document` and `photocraft://commands` return live JSON matching `doc_inspect`
and `command_list`. Resource reads are uncached; tool/resource catalogs are private and cached
for ten minutes. List/read responses include the MCP 2026-07-28 result/cache fields.

## Task tools

Besides the generic tools, headless mode offers task-shaped image tools with typed, validated
arguments. Each tool runs ordinary engine commands through `command_run`'s path (same
authorization, history and filesystem roots). Their document edits join the history, so
`edit.undo` reverts them like any other command. Files they write are not rolled back:
`save_image`, `export_image` and `batch_process` change the disk, and undo does not restore an
earlier file.
Agents need no command ids or parameter grammars for everyday edits. `command_run` stays available
for the rest of the engine's commands.

| Tool | Arguments | Engine commands |
|---|---|---|
| `open_image` | `path` | `doc_open` |
| `inspect_image` | `path` (read without touching the session) or `index` | open + `doc_inspect` + close |
| `save_image` | `path` (extension picks the format), `quality`, `overwrite` | `doc_save` |
| `export_image` | `path`, `format`, `quality`, `overwrite` | `doc_export` |
| `resize_image` | `width` and/or `height` (one side keeps the aspect ratio), or `percent`, or `fit` (+`enlarge`), `resample` | `image.imageSize` |
| `crop_image` | `x`, `y`, `width`, `height` | `image.crop` |
| `rotate_image` | `angle` (clockwise; multiples of 90 are lossless), `interpolation` | `image.imageRotation.*`, `image.rotation.arbitrary` |
| `flip_image` | `direction`: horizontal \| vertical | `image.imageRotation.flipCanvas*` |
| `adjust_brightness` / `adjust_contrast` | `amount` (-150..150 / -50..100) | `image.adjustments.brightnessContrast` |
| `adjust_saturation` | `amount` (-100..100) | `image.adjustments.hueSaturation` |
| `apply_blur` | `kind`: gaussian \| box \| motion \| surface, `radius`, `angle`, `distance`, `threshold` | `filter.blur.*` |
| `apply_sharpen` | `kind`: unsharp_mask \| smart \| basic, `amount`, `radius`, `threshold`, `reduce_noise` | `filter.sharpen.*` |
| `remove_background` | `refine`, `apply_mask`, `trim`, `background_color` | `layer.removeBackground` (+ mask apply, trim, solid fill) |
| `add_text` | `text`, `x`, `y`, `size`, `font`, `color`, `align`, `name` | `type.create` |
| `manage_layers` | `action` (list, new, duplicate, delete, rename, set_props, select, bring_forward, send_backward, bring_to_front, send_to_back, merge_down, flatten), `layer` (id or exact name), `name`, `visible`, `opacity` (%), `blend` | `layer.*` |
| `batch_process` | `input_dir`, `output_dir`, `operations`, `format`, `quality`, `suffix`, `extensions`, `max_files`, `overwrite`, `stop_on_error` | open → operations → save → close, per file |

Behaviour shared by the task tools:

- **Structured replies.** Every reply carries `structuredContent`, and the same JSON as a text
  block for text-only clients. Every success has `"ok": true` and `"tool"`. The rest depends on
  the tool:
  - The editing tools and `manage_layers` return `result` (the engine's reply) and `document`
    (`{index, name, width, height, mode, depth, resolution, layerCount, …}`, plus `layers` for
    `manage_layers`).
  - `open_image` returns `document` and `warnings`.
  - `save_image` and `export_image` return `path`, `warnings` and `document`.
  - `inspect_image` returns the summary fields at the top level (`width`, `height`, `layers`, …,
    and `file` for a path).
  - `batch_process` returns `processed`, `failed`, `skipped`, `truncated` and `files` at the top
    level.

  Failure is `isError: true` with `{"ok": false, "tool": …, "error": "<reason>"}`. Unknown argument keys are
  JSON-RPC `-32602` errors before anything runs. Out-of-range values name the accepted range.
- **No silent overwrites.** `save_image`, `export_image` and `batch_process` refuse an existing
  target unless `overwrite: true`. Even then, the write root's rule still applies: only image and
  document files are ever replaced. Missing folders below the write root are created.
- **Bounded work.** `resize_image` and `crop_image` refuse results over 256 megapixels.
  `open_image`, `inspect_image` and `batch_process` read a file's header first. A PNG, JPEG,
  PSD/PSB, WebP, OpenEXR, GIF, BMP, TGA, ICO, QOI or HDR that declares more than 256 megapixels
  is refused before it is read in full or decoded. Other formats rely on the decoder's own
  limits. `batch_process` looks at no more than 500 files (default 100) and skips inputs over
  512 MiB. It applies at most 32 operations and handles one file at a time, closing each document
  before opening the next.
- **Batches never replace an unread input.** Before writing anything, `batch_process` checks
  every planned output. The whole batch is refused when an output would replace another file of
  the input folder, for example `photo.jpg` becoming `photo.png` next to the original
  `photo.png`. It is also refused when two inputs would write the same output. A file may still
  be written back over its own source with `overwrite: true`.
- **Headless only.** In bridge mode the task tools reply with an error that points to
  `command_run`. The live app takes the same command ids.

`batch_process` operations name a task tool and carry its arguments (without `index`):

```json
{"input_dir": "photos", "output_dir": "web", "format": "webp", "quality": 82, "suffix": "_1080",
 "operations": [
   {"tool": "resize_image", "width": 1080, "height": 1080, "fit": true},
   {"tool": "adjust_contrast", "amount": 10},
   {"tool": "apply_sharpen", "kind": "unsharp_mask", "amount": 60, "radius": 1}
 ]}
```

The reply lists every file with `status` (`ok`, `failed` or `skipped`, with a reason) and the
totals `processed`, `failed`, `skipped` and `truncated`. A failing file never stops the batch
unless `stop_on_error` is true.

### Logging

Set `PHOTOCRAFT_LOG` (`error`, `warn`, `info`, `debug` or `trace`) in the server's environment
to log to **stderr**. Stdout carries only the protocol. Unset, the server logs nothing. Each level
includes the ones above it:

- `warn`: every batch file that failed, whatever the stage (name, path, decoding, an operation,
  saving). The reply's `failed` counter and per-file `status` are the authoritative record:
  check them rather than relying on the log.
- `info`: also every task tool call with its duration and outcome (and the MCP session's own
  start-up events).
- `debug`: also each engine command a task tool runs, with its parameters.

### Client configuration

Claude Code (user scope; use `--scope project` to write `.mcp.json` into the project):

```sh
claude mcp add photocraft --env PHOTOCRAFT_LOG=warn -- photocraft-cli mcp --automation-read-root /work/project --automation-write-root /work/project/out
```

A project `.mcp.json` file ([`mcp-examples/claude-code.mcp.json`](mcp-examples/claude-code.mcp.json)):

```json
{"mcpServers": {"photocraft": {"command": "photocraft-cli",
  "args": ["mcp", "--automation-read-root", "/work/project", "--automation-write-root", "/work/project/out"],
  "env": {"PHOTOCRAFT_LOG": "warn"}}}}
```

Codex (`~/.codex/config.toml`, [`mcp-examples/codex-config.toml`](mcp-examples/codex-config.toml)):

```toml
[mcp_servers.photocraft]
command = "photocraft-cli"
args = ["mcp", "--automation-read-root", "/work/project", "--automation-write-root", "/work/project/out"]
env = { PHOTOCRAFT_LOG = "warn" }
# batch_process over a large folder can take minutes
tool_timeout_sec = 600
```

On Windows, use the full path of `photocraft-cli.exe` as `command` when it isn't on `PATH`.
Write JSON backslashes as `\\`, or use forward slashes, which Windows accepts.

### Building

`scripts/build-mcp.sh` (Linux, macOS) and `scripts/build-mcp.ps1` (Windows) build the release
CLI with `--locked` and run the MCP test suites. `--install` / `-Install` then puts
`photocraft-cli` on `PATH` with `cargo install`. `--check-targets` / `-CheckTargets` also
type-checks the MCP crates for the other desktop platforms.

## Progress and cancellation

A direct headless `command_run` of `file.export.renderVideo` reports frame counts when
`_meta.progressToken` is present (a string or number). Notifications increase strictly,
are throttled to at most ten per second except completion, and include `total`. No token
means no progress notifications. Ping remains responsive while rendering.

```json
{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"command_run","arguments":{"id":"file.export.renderVideo","params":{"dir":"frames","format":"png"}},"_meta":{"progressToken":"export-20"}}}
{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":20}}
```

Create a document and timeline first (`doc_new`, then `timeline.create`). Export uses the
configured write root and creates `dir` beneath it. Filenames must be single components;
absolute paths, traversal and escaping symlinks are rejected. Existing target filenames
are refused, including symlinks, so a cancelled or failed job cannot replace an earlier export.
Only files actually created by this job are removed on cancellation, error or an escaped panic;
unrelated files and the destination directory remain. PNG sequences and animated GIF share
the engine renderer. The document and playhead are unchanged.

Cancellation suppresses the request response and stops at a frame boundary (the current
frame/encoding may finish first). Unknown request ids are ignored. Render Video stays a
synchronous command even with `wait:false`; other job-capable commands keep their existing
`jobs_list`/`jobs_cancel` behavior. Batch exports do not emit MCP progress or support per-step
MCP cancellation. Nested engine actions and bridge Render Video remain denied by the existing
ambient-filesystem policy; the new scoped writer is available for direct headless commands.
