# Control protocol

The desktop app listens on `127.0.0.1:<port>` (loopback only). Start it with a token file so the credential is not exposed in the process command line:

```sh
photocraft --control 7878 --control-token-file /private/path/photocraft-control.token \
  --automation-read-root /work/project --automation-write-root /work/project
```

If the file does not exist, PhotoCraft creates it with a fresh 256-bit token. On Unix the new file is mode `0600`; on Windows, protect it with an appropriate user-only ACL. An existing file is reused. If neither a token nor token file is configured, PhotoCraft generates a token for that launch and writes it to standard error. `PHOTOCRAFT_CONTROL_TOKEN` and `PHOTOCRAFT_CONTROL_TOKEN_FILE` are the environment-variable equivalents.

The first request on every TCP connection must authenticate. No other method is dispatched before this succeeds:

```json
{"id": "auth", "method": "auth", "params": {"token": "<64 hexadecimal characters>"}}
```

After the successful authentication reply, each request is one JSON line:

```json
{"id": 1, "method": "ui.inspect", "params": {}}
```

Each reply is one JSON line with the same `id`:

```json
{"id": 1, "ok": true, "result": { ... }}
{"id": 2, "ok": false, "error": "unknown tool `foo`"}
```

The transport is `apps/photocraft/src/control_server.rs`, and the handlers are in `crates/ui-egui/src/control.rs`. The MCP server (`photocraft-cli mcp --bridge 127.0.0.1:<port>`, crate `photocraft-automation`) wraps this same protocol. See [MCP bridge](#mcp-bridge) below.

## Methods

- `engine.execute {command, params}`: run any engine or UI command by id. Engine commands run directly with their default params and never open a dialog. Use `ui.menu.invoke` for menu-click behaviour, which opens a command's dialog when no params are given. `params` must be a JSON object (omit it or pass `null` for none); an array, string, number or boolean is an error naming the command, in every transport (control channel, `serve`, MCP and the CLI)
- `engine.commands`: list commands with enablement
- `ui.inspect`: full UI state (tool, panels, views, dialogs, windows, window size). The menu tree is not included; use `ui.menu.list`. `view` holds the View/Window/Type preferences: `screen_mode`, `extras`, `show` and `snap_to` flags, `flip_horizontal`, `arrange` (Window › Arrange layout), pixel aspect, font preview size, language options. `perf.timings.gpuInfo` holds the graphics adapter, backend, driver, the backend chosen at launch and why, the canvas renderer (`gpu`/`cpu`) and, after a device loss, `lost` (`help.systemInfo` returns the same as `info`)
- `ui.set {tool?, panels?, dockTabs?, dock?, dockWidth?, colorPanel?, maskTarget?, vectorMaskTarget?, selectionMode?, zoom?, center?, fit?, theme?, brushSize?, brushSection?, brushTab?, brushesView?, brushPicker?, brushPickerView?}`: change UI state; any other field is an error, checked before anything changes (`theme` is `pro`, `proMedium`, `studio`, `studioLight` or `classic`; `selectionMode` is the selection tools' options-bar mode, 0 New, 1 Add, 2 Subtract, 3 Intersect; `brushSection` indexes the Brush Settings sections, `brushTab` 0 = Brush Settings, 1 = Brushes; `brushesView` and `brushPickerView` are `list` or `grid` for the Brushes panel and the Brush Preset picker; `brushPicker` is `[x, y]` in screen points to open the Brush Preset picker there (as a right-click with a painting tool does) or `null` to close it, and picking a preset in it is `tools.setBrush {"preset": name}`; `dock` is `{order: ["layers", …], heights: {"properties": 180}, collapsed: ["color"]}`, the right-dock groups top to bottom, their heights in points and the groups collapsed to their tab strip; `dockWidth` sets the right dock width in points, clamped to 250..520; `colorPanel` is `{background}`, whether the Color panel edits the background colour)
  - `typeTransform` is `null` outside a temporary Ctrl/Cmd Type-tool drag. During a drag it reports its source document/revision, original affine, oriented `frame` (layer, local bounds, document-space corners and pivot), and captured `gesture`. Drive it with `ui.pointer` and `command: true` at `down`; `move`/`up` may release the modifier. The document/history remain unchanged during preview, and `up` applies one `type.edit` within `textEdit`'s session. Pressing Escape during the drag cancels only the preview. This pointer state is never restored from saved UI state.
- `ui.set {tool?, panels?, dockTabs?, dock?, dockWidth?, colorPanel?, maskTarget?, vectorMaskTarget?, selectionMode?, zoom?, center?, fit?, theme?, brushSize?, brushSection?, brushTab?, brushesView?}`: change UI state; any other field is an error, checked before anything changes (`theme` is `pro`, `proMedium`, `studio`, `studioLight` or `classic`; `selectionMode` is the selection tools' options-bar mode, 0 New, 1 Add, 2 Subtract, 3 Intersect; `brushSection` indexes the Brush Settings sections, `brushTab` 0 = Brush Settings, 1 = Brushes; `dock` is `{order: ["layers", …], heights: {"properties": 180}, collapsed: ["color"]}`, the right-dock groups top to bottom, their heights in points and the groups collapsed to their tab strip; `dockWidth` sets the right dock width in points, clamped to 250..520; `colorPanel` is `{background}`, whether the Color panel edits the background colour)
- `ui.menu.invoke {id, wait?}` / `ui.menu.list`: activate a menu item by id; list the menu tree. A menu item that starts a background job (a filter without a dialog, such as Blur More) replies with the job's result once it has been applied; with `"wait": false` the reply is `{job, pending: true}` at once
- `ui.set` also accepts `gradientBlendMode` (a layer blend mode name, such as `Difference`) and `gradientClassic` (boolean) for the Gradient tool options bar.
  Both gradient modes use the current preset, including its opacity stops. Select `Foreground to Transparent` through `gradient.presets.select`, or pass custom `stops` and `transparency: [[0,100],[1,0]]` (location 0..1, opacity 0..100). Use `applyToLayer: false` to edit only the tool preset. Classic gradient drags dispatch `paint.gradient` without overriding them.
- `ui.dialog.open {kind, fields?}` (kinds `newDocument`, `about`, `layerStyle {effect?}`, `colorPicker {target: foreground|background}`, `command {command}`) / `ui.dialog.set {dialog, field, value}` / `ui.dialog.confirm {dialog, wait?}` / `ui.dialog.cancel {dialog}`. Like `ui.menu.invoke`, `ui.dialog.confirm` waits for a background job its command starts (a filter dialog's OK) and replies with the result; `"wait": false` replies `{job, pending: true}` at once
  - Layer Style (`layerStyle {effect?}` opens on that effect kind): the fields hold the dialog's whole state, so agents read and drive it like a user. `effects` lists every effect instance in the layer's order as `{id, kind, on, params, fx}` (`fx` is the effect snapshot the dialog loaded; OK edits that snapshot with `params`, so anything the dialog doesn't model survives). `selected` picks the page (`blendingOptions` or an instance id), `preview` (default on) gates the live canvas preview, `p:blendingOptions` edits blend mode/opacity/fill opacity, and `patternList` names the usable patterns. OK replaces the layer's effects in one step (`layer.layerStyle.replace`); Cancel discards. "New Style…" saves the pending state via `style.presets.new {effects}`, a swatch click applies a style preset, and Make/Reset Default use `layer.layerStyle.makeDefault` / `layer.layerStyle.defaultFor`
  - Preferences (`ui.menu.invoke {id: "edit.preferences.interface"}`) edits the sections in its `values` field. `ui.dialog.apply {dialog}` saves those values through `prefs.set` and keeps the same dialog and section open. `ui.dialog.confirm` saves and closes; `ui.dialog.cancel` discards only edits made since the last successful Apply. Invalid values return an error without closing the Apply dialog or changing the saved preferences. Settings marked for the next launch still require a restart.
  - Edit › Fill… (`ui.menu.invoke {id: "edit.fill"}`, Shift+F5, Shift+Backspace) opens the Fill dialog; its fields are `edit.fill`'s params (`contents`, `color`, `pattern`, `colorAdaptation`, `mode`, `opacity`, `preserveTransparency`), and OK remembers them in the preferences (`dialogs["edit.fill"]`).
  - A pixel tool pressed on a type, shape, Smart Object or fill layer (e.g. through `ui.pointer`) opens the "Rasterize?" prompt instead of painting: a dialog with `__rasterize` (`type|shape|smartObject|fill`), `message`, `layer`, `tool` and `at`. `ui.dialog.confirm` runs the `layer.rasterize.*` command and then paints at `at` (two history states, `{"rasterized", "painted"}`); `ui.dialog.cancel` does nothing.
- `ui.window.open {document?}` / `ui.window.close {window}`: extra document windows
- `ui.pointer {events: [{kind: down|move|up, x, y, pressure?, tiltX?, tiltY?, rotation?}], modifiers?, button?}`: drive the active tool in document coordinates (pressure 0..1, tilt in degrees -90..90, barrel rotation 0..360: a simulated pen). Modifier flags (`shift`, `alt`, `command`, `ctrl`, `space`) go either at the top level or grouped under `modifiers`; when `modifiers` is present, top-level flags are ignored. `space: true` holds Space (the Crop frame, marquee, lasso or shape being drawn then moves instead of growing). `button: "right"` (or `"secondary"`) is the right button: with the Move tool (or `command: true` with any tool) it opens the canvas layer menu (`layerMenu` in `ui.inspect` lists the layers there, topmost first; select one with `layer.select`); with Marquee, Lasso, Magic Wand, Object Selection, or Pen it opens a tool context menu (`canvasToolMenu` in `ui.inspect`, with ordered command ids, separators, and enabled states); with a painting tool it opens the Brush Preset picker, or erases with Preferences › Tools › Right-click with painting tools = erase. The menus open at the screen position of the pressed document point. Use `ui.context.choose {id}` for enabled tool-context actions. The Pen menu is documented in `docs/context-menu-parity.md`. With a Color Picker as the top dialog, `down` and `move` sample the image into its new colour instead (its eyedropper, like a click on the canvas) and the tool is not driven.
  - Magnetic Lasso (`tool: "magneticLasso"`): a `down`/`up` pair is a click. The first sets the first fastening point (its modifiers set the selection mode); later clicks fasten a point, or close the border on the first point. `move` events trace the border with or without a button held, fastening points by themselves; with `alt: true` a click draws a straight segment and a drag a freehand one. `ui.inspect` shows the border in progress under `magnetic` (`path`, `anchors`, `live`, `mode`). `ui.key` Enter closes it along the edges, Backspace removes the last fastening point, Escape cancels. To select along edges in one call instead, use the `select.magneticLasso` command with rough points around the shape.
- `ui.key {key, command?, shift?, alt?, ctrl?}` (flags may also be grouped under `modifiers`): press and release a key, e.g. `{"key": "ArrowLeft", "shift": true}`
- `ui.type {text}`: type text (goes to the focused widget, or to the canvas while the Type tool is editing)
- `ui.resize {width, height}`: resize the main window
- `ui.gpu.simulateLoss {error?}`: act as if the wgpu device was lost (or reported an error with `error: true`). The app switches to the CPU image compositor for the rest of the session and shows the same recovery warning as on a real loss. `gpuFallbackNotice` in `ui.inspect` contains the reason while the warning is open. Keep Using CPU saves CPU compatibility for the next launch; Retry GPU saves GPU mode and asks the user to save and restart. The window renderer still requires a working graphics or software adapter. Returns `wasActive` and `gpuInfo`. For testing the fallback
- `ui.screenshot {path?, focus?}`: capture the main window (PNG). With no path the reply contains
  base64 PNG data; a path is relative to the automation write root. Raises the window first
  (default) because occluded macOS windows stop rendering
- `ui.focus`: bring the main window to the front
- `app.open {path}` / `app.save {path}`: relative file I/O through the configured automation roots (`app.open` reads under the read root, `app.save` writes under the write root; absolute paths, `..` and paths escaping the root are refused, and both fail closed when no root was granted). Both reply with `warnings` (import/export notes such as "adjustment layer flattened"; `[]` when none), also shown to the user in the status bar and as a notice (`notices` in `ui.inspect`); `app.open` also returns the `path` and document `name`, `app.save` the `path` written. `app.save` without `path` writes back only to the document's own PSD, PSB or `.pcraft` file, like File › Save. Automation opens and saves never fire script events. Use these two rather than `file.open`, `file.save`, `file.saveAs` or `file.saveACopy`, which the control channel refuses (see [Engine commands](#engine-commands))
- `app.quit`

## Engine commands

Change the UI language without restarting with `prefs.set`:
`{"path":"interface.language","value":"fr"}`. Supported codes and preview/apply behaviour
are documented in [UI localisation](localization.md). `prefs.get` and the existing preference
store expose and persist the same setting; scripts keep using canonical command IDs.

`engine.execute` runs any command by id. `engine.commands` (or the engine command `command.list`) lists them all, with labels, menu paths, shortcuts, a parameter description, and whether each is currently enabled. Examples:

| Command | Params |
|---|---|
| `file.new` | `{"width":1920,"height":1080,"mode":"rgb","depth":8,"background":"white"}` |
| `layer.new.layer` | `{"name":"Ink"}` |
| `layer.select` | `{"layer":id,"mode":"replace\|toggle\|range\|add"}`: ⌘-click = toggle, ⇧-click = range; `document.inspect` reports `selectedLayers` and a per-layer `selected` flag |
| `channel.target` / `channel.setVisible` | Channels panel target and eyes (`"composite"`, colour name, alpha index or name, `"quickMask"`). `document.inspect` (and `channel.list`) report `channels`: composite/colour/alpha rows with visibility, channel options, `quickMask`, `target`. Pixel commands without a `target` param follow the targeted channel |
| `layer.setProps` | `{"layer":id?,"name":…,"visible":…,"opacity":0..1,"blend":"Multiply"}` |
| `layer.setExpanded` | `{"layer":id?,"expanded":bool?,"all":bool?}`: open/close a group in the Layers panel (no `expanded`: toggle; `all`: every group). A view change saved with the document (PSD open/closed folder), not an undo step; `document.inspect` reports `expanded` on groups |
| `layer.newAdjustmentLayer.hueSaturation` | `{"hue":30,"saturation":10}` |
| `paint.stroke` | `{"points":[[x,y,pressure],…],"size":20,"color":"#ff0000"}` |
| `select.rect` | `{"x":0,"y":0,"width":100,"height":50,"mode":"add","ellipse":false}` |
| `document.inspect` | `{}`: layer tree, history, selection bounds |
| `document.pixel` | `{"x":10,"y":10}`: composite RGBA |
| `type.hitTest` | `{"layer":id?,"x":px,"y":px}`: character under a document point. No `layer` picks the topmost visible type layer there. Result `{"layer","index","line","inside"}` |
| `type.caret` | `{"layer":id?,"index":char}`: caret segment in document pixels, `{"index","line","segment":[[x,y],[x,y]]}` (rotated and vertical type included) |
| `type.navigate` | `{"layer":id?,"index":char,"move":"wordPrev\|wordNext\|linePrev\|lineNext\|lineStart\|lineEnd\|start\|end","x":px?}`: neighbouring caret. `x` keeps the column across line moves. Result `{"index"}` |
| `actions.list` | `{}` → `{actions:[{name, steps}], recording}` (the name being recorded, or null) |
| `actions.get` | `{"action": name or index}` → `{name, steps:[[id, params], …]}`, the shape `file.automate.batch` and droplets take |
| `actions.record` | `{"name":"Red"}` starts a new action (default name `Action N`). `{"action": name or index}` appends to one that exists |
| `actions.stop` | `{}` → `{action, steps}`. Copies replayable journal entries since `actions.record` (queries and `actions.*` omitted) |
| `actions.play` | `{"action": name or index, "from": step?}`. `from` and `failed.step` are 0-based. Returns `{action, ran, failed?:{step, id, error}}` and still returns ok when a step fails, so a partial run is reported. Leaves one history step per step that ran. Refuses to play while a play is already running. On an untrusted session each step is authorized the same way as a top-level command |
| `actions.delete` | `{"action": name or index}` → `{deleted}`. Refused while recording |

UI-level commands (`view.zoomIn`, `window.theme.pro`, `edit.search`, …) are also accepted by `engine.execute` and `ui.menu.invoke`.

Commands that read or write files by path, instead of through the automation roots, are refused with "automation command `…` uses ambient filesystem paths and is disabled; use capability-scoped document methods". That covers every `file.*` command except `file.new`, the `file.close*` commands and a few path-free ones such as `file.fileInfo`, so `file.open`, `file.save`, `file.saveAs` and `file.saveACopy` always fail here: open and save with `app.open` / `app.save`. Path parameters of other commands (`layer.exportAs {path}`, `filter.distort.displace {mapPath}`, preset imports, and plug-in install/reload) are refused the same way. `prefs.set` rejects whole-preference updates and file-backed sections (`colorSettings`, `scriptEvents`, `historyLog`, `plugIns` and `scratchDisks`) so automation cannot configure ambient file access indirectly. The desktop app also refuses `image.mode.*`, which can load the colour profiles set in its preferences. The rules are in `crates/automation/src/workspace.rs`.

`type.editText` starts inline editing of the active type layer and selects all its text, like
double-clicking its thumbnail in Layers. Text input, Commit and Cancel use the existing Type tool
editing session; non-type layers return an error without changing the tool or document.

### Background jobs (#210)

Long commands (every `filter.*`, `edit.contentAwareFill`, `edit.contentAwareScale`, `file.automate.photomerge`, `brush.presets.importAbr`) and file opens run as background jobs in the desktop app: the window keeps drawing, the status bar shows progress with a Cancel button, and jobs that lock the active document show a modal progress dialog (Esc cancels). `engine.execute`, `ui.menu.invoke` and `ui.dialog.confirm` still wait for the result by default; pass `"wait": false` to get `{job, pending: true}` at once, then poll `jobs.list` (`state`: running, done, failed, cancelled; `progress` 0–1; the result or error) and stop it with `jobs.cancel {job}`. A cancelled or failed job leaves the document unchanged. While a job runs, commands that would edit its document fail with "… is still running on this document". `ui.inspect` reports `jobs` (running jobs, opening files). Set `PHOTOCRAFT_INLINE_JOBS=1` to run everything inline.

## Preferences

Preferences (Edit › Preferences, grouped like Photoshop's dialog sections) live in the engine, so
the same commands work in the app, the CLI and headless MCP. Paths are dotted camelCase keys:
`<section>.<key>`, with sections `general`, `interface`, `workspace`, `tools`, `historyLog`,
`fileHandling`, `export`, `performance`, `scratchDisks`, `cursors`, `transparencyAndGamut`,
`unitsAndRulers`, `guidesGridAndSlices`, `plugIns`, `type`, `enhancedControls`, `rawDefaults`,
`integrations`, plus `shortcuts.<command id>`, `menus` (`hidden`, `colors.<id>`), `toolbar`
(`hidden`, `order`) and `colorSettings` (Edit › Color Settings).

| Command | Params |
|---|---|
| `prefs.get` | `{"path":"performance.historyStates"}`; no path returns everything |
| `prefs.set` | `{"path":"cursors.painting","value":"precise"}` or `{"values":{"unitsAndRulers.rulers":"cm","guidesGridAndSlices.gridColor":"#ff8800"}}`. Values are validated (choices, ranges, `#rrggbb` colours, shortcut syntax); a batch applies all or nothing. A section path takes an object and merges it key by key |
| `prefs.reset` | `{"path":"performance"}` (a section or key); no path resets everything. An individual `shortcuts.<id>` or `menus.colors.<id>` reset removes the override and returns `null`, including when it is already absent |
| `edit.preferences.<section>` | the section's values; in the app (no params) it opens the Preferences dialog on that section |
| `edit.keyboardShortcuts` | `{"set":{"edit.fill":"Cmd+Shift+F"},"reset":true\|["id",…],"removeConflicts":true,"filter":"blur","list":false}`: returns overrides, matching commands and conflicts. A shortcut moved to another command is removed from its old owner unless `removeConflicts` is false. `""` removes a shortcut, `null` restores the default. The held temporary tools are bindable too: `tools.temporary.hand` (Space; also repositions a selection being drawn), `tools.temporary.zoomIn` (Cmd+Space), `tools.temporary.zoomOut` (Cmd+Alt+Space); they list with `"hold": true`. In the app it opens Keyboard Shortcuts and Menus |
| `edit.menus` / `edit.toolbar` | `{"hide":["edit.fade"],"show":[…],"color":{"edit.fill":"red"},"reset":false}` / `{"hidden":["Sponge"],"order":[…]}` |
| `edit.colorSettings` | `{"workingRgb":"display-p3","workingCmyk":"coated-cmyk","workingGray":"sgray","policyRgb":"preserve\|convert\|off",…,"askOnMismatch":true,"intent":"perceptual","bpc":true}`; honoured when opening files (`file.openAs`, the app's File › Open) and by Image › Mode and "working" profile specs. `color.profileMismatch {"action":"preserve\|convert\|discard\|assignWorking"}` answers the mismatch prompt |

Values the app honours live: history states (every open document), effect-cache budget, interface
theme, canvas colour and border, checkerboard size and colours (CPU and GPU canvas), gamut warning
colour and opacity, guide/grid/smart-guide colours and styles, grid spacing and subdivisions, ruler
units (rulers, Info panel, Image Size default unit), image interpolation (Image Size default),
painting and other cursors, zoom with scroll wheel, Use Shift Key for Tool Switch, keyboard
shortcuts, hidden and coloured menu items, autosave interval and crash recovery, and the history
log text file. GPU on/off and the GPU tile size apply at the next launch.

The desktop app stores them in `preferences.json` in the platform config directory (macOS
`~/Library/Application Support/Photocraft`, Windows `%APPDATA%\Photocraft`, Linux
`$XDG_CONFIG_HOME/photocraft`; override with `PHOTOCRAFT_CONFIG_DIR`); autosaves go to its
`Recovery` folder, and the window size and position, the dock width and floating-panel geometry
to `ui.ron` beside it (a file holding values that would crash startup is discarded). In portable mode (a `portable.txt` or `PhotoCraft.portable` file beside the
executable, as in the Windows portable zip) that directory is `PhotoCraftData` next to the
executable instead. The web build keeps them in `localStorage`. A save writes only the values
this instance changed since it last loaded or saved them over what storage holds now, so a second
browser tab (or app window) never reverts the other's changes; for a value changed in both, the
latest save wins. A failed write stays pending and is retried (after 2 s, doubling up to 30 s, and
sooner after a further change); the status bar reports the first failure and a notice a
persistent one.

User and imported (`.abr`) brush presets live in the config directory's `Presets` folder: one
`.pcbrushes` JSON file per preset group, content-addressed tip bitmaps under `tips/`, and an
`index.json` with the group order and deleted built-ins (see `photocraft_engine::preset_store`).
The store loads in the background at launch and syncs after every brush preset change; built-ins
are never written. The Actions list is `actions.json` in that same folder and survives a restart.
Headless CLI/MCP sessions and the web build keep brush presets and the action list for the session
only, unless a store is attached. Gradient presets (including imported `.grd` groups) persist with
the preferences.

## Snapping

With View › Snap on, tool gestures snap to the View › Snap To targets (guides and grid while they
are shown, layer edges and centres, document bounds and centre, selection edges) within 8 screen
pixels: Move tool drags (the moved layers' bounds), marquee, crop, shape, type-box and pen points,
Free Transform handles and drags inside the box, and guides. Holding Ctrl disables snapping for
the drag. With View › Show › Smart Guides on, the Move tool also snaps to other layers and draws
magenta alignment lines. `ui.pointer` drives the same code, so agents get identical results.

## MCP bridge

`photocraft-automation` provides an MCP server built on the official Rust SDK (`rmcp`). It runs in one of two modes:

- **Headless** (`photocraft-cli mcp`): an in-process `photocraft_engine::Session`. There is no window.
- **Bridge** (`photocraft-cli mcp --bridge 127.0.0.1:7878 --control-token-file <path>`): every tool is forwarded to a running `photocraft --control 7878 --control-token-file <path>` over this protocol, so agents see and drive the live app.

The bridge keeps one authenticated TCP connection open. It reconnects and authenticates once if a request fails, and it skips reply lines whose `id` doesn't match the request (for example, stale replies to requests that timed out). It only accepts loopback addresses, because the app only listens on loopback. Supply its bearer token with `--control-token-file`, `--control-token`, `PHOTOCRAFT_CONTROL_TOKEN_FILE`, or `PHOTOCRAFT_CONTROL_TOKEN`:

```sh
photocraft-cli mcp --bridge 127.0.0.1:7878 \
  --control-token-file /private/path/photocraft-control.token
```

How each MCP tool maps onto control methods in bridge mode:

| MCP tool | Control method |
|---|---|
| `command_run {id, params, wait?}` | `engine.execute {command: id, params, wait}` |
| `jobs_list` / `jobs_cancel {job?}` | `jobs.list` / `jobs.cancel {job?}` |
| `command_list {filter?, enabled_only?}` | `engine.commands` (filtered by the MCP server) |
| `doc_new {…}` | `engine.execute {command: "file.new", params}` |
| `doc_inspect` | `engine.execute {command: "document.inspect"}` |
| `doc_open {path}` | `app.open {path}` |
| `doc_save {path}` / `doc_export {path}` | `app.save {path}` |
| `doc_render_preview {max_side?}` | `ui.screenshot`, returned directly as PNG image content |
| `session_list`, `ui_inspect` | `ui.inspect` |
| `ui_screenshot {max_side?}` | `ui.screenshot`, returned as PNG image content |
| `ui_pointer {events, modifiers?, button?}` | `ui.pointer` (other arguments are an error) |
| `ui_menu_invoke {id}` | `ui.menu.invoke` |
| `ui_set {fields}` | `ui.set` |
| `control_call {method, params}` | any method, passed through unchanged |

`doc_select` and `doc_close` work only in headless mode. The `ui_*` tools and `control_call` work only in bridge mode; in headless mode they return a tool error that explains how to start bridge mode.

**Security note:** TCP control uses a bearer token, not client identity or general per-method
authorization. A client that possesses the token receives the non-filesystem control surface,
including UI input, command execution, and application control. Keep token files private, do not
commit or log tokens, and do not pass a token directly on a shared system where process command
lines are visible. The protocol is unencrypted and must remain on loopback; do not tunnel or proxy
it to an untrusted host.

Filesystem access fails closed unless launch-time read and/or write roots are granted with
`--automation-read-root` and `--automation-write-root` (or
`PHOTOCRAFT_AUTOMATION_READ_ROOT` / `PHOTOCRAFT_AUTOMATION_WRITE_ROOT`). Request paths must be
non-empty, forward-slash relative paths beneath the applicable root. Absolute paths, parent
traversal, alternate separators, drive/device/stream prefixes, malformed components, and link
escapes are rejected before file effects. Read and write authority are independent; the parent of
a new output file must already exist. Engine commands that still use ambient filesystem paths are
disabled for automation until they are migrated to the same capability interface. Interactive
desktop file pickers retain normal user-selected access.

## Headless server

`photocraft-cli serve` keeps one headless engine session (no window, no GPU) and answers the same
JSON-lines envelope on stdio, or on `127.0.0.1:<port>` with `--port <port>` (loopback only; each
authenticated connection shares the session). TCP uses the same first-frame `auth` exchange and
token options as desktop control. Stdio does not require this TCP handshake because access is
inherited from the process pipe. It is the fastest way for a script or agent to make many edits:
no MCP framing, no app start-up per command. Configure its file access with the same
`--automation-read-root` and `--automation-write-root` flags. Implementation:
`crates/automation/src/rpc.rs`.

| Method | Params |
|---|---|
| `engine.execute` | `{command, params?, wait?}`: any engine command (`wait: false` starts a long one as a background job: `{job, pending}`) |
| `jobs.list` / `jobs.cancel` | `{}` / `{job?}`: background jobs; cancel one or all. Every request (and MCP tool call) first applies the jobs that finished, so `doc.save`, `doc.inspect`, `doc.render` and `session.list` include a finished job's result without polling `jobs.list` first |
| `engine.commands` | `{filter?}`: registry with params docs and enablement |
| `session.list` | open documents and the active index |
| `doc.open` / `doc.new` | `{path}` / `file.new` params |
| `doc.save` | `{path?, format?, quality?, index?}` (`.pcraft` native, else export by extension). Without `path` only a PSD, PSB or `.pcraft` document is written back to its own file, in its own format; anything else is an error and the file is left unchanged |
| `doc.inspect` | `{index?}`: same JSON as `document.inspect` |
| `doc.render` | `{index?, maxSide? (1024; 0 = full), path?}`: PNG to `path`, else `{mime, base64}` |
| `doc.select` / `doc.close` | `{index}` / `{index?}` |
| `batch` | `{steps: [{command, params?, wait?} \| {method, params?}], stopOnError? (true)}` → `{completed, failed, results}` (a step with `wait: false` starts a long command as a background job, like `engine.execute`) |
| `methods` | the list above |

```sh
printf '%s\n' \
  '{"id":1,"method":"doc.open","params":{"path":"in.jpg"}}' \
  '{"id":2,"method":"batch","params":{"steps":[{"command":"image.adjustments.invert"},{"command":"filter.blur.gaussianBlur","params":{"radius":3}}]}}' \
  '{"id":3,"method":"doc.save","params":{"path":"out.png"}}' | \
  photocraft-cli serve --automation-read-root /work/project --automation-write-root /work/project
```

The MCP server has the same batching as the `command_batch` tool (`{steps:[{id, params, wait?}], stop_on_error}`),
in headless and bridge mode. A step with `wait: false` returns `{job, pending}` at once, as `command_run`
does; later steps that edit the same document fail with a message naming the job until it ends.

## Transport limits

The desktop and headless TCP listeners currently enforce:

- a 1 MiB maximum encoded request line;
- an 8 MiB maximum encoded JSON reply, including the newline;
- at most 16 simultaneously serviced connections per listener;
- a 30-second socket read/write timeout;
- at most 256 steps in a headless `batch` or MCP `command_batch` request.

An oversized line, excess connection, unauthenticated request, or unauthorized filesystem path is
rejected before command dispatch or file effects. The headless JSON-lines stdio server also
enforces the request and reply byte ceilings; MCP over stdio (`photocraft-cli mcp`) does not
cap request bytes, because the MCP SDK reads its own request lines (only MCP tool results are
checked). A TCP connection is closed after an oversized
line; on stdio an oversized or non-UTF-8 line gets one error reply (`id: null`), the rest of that
line is skipped without being dispatched, and the session and its open documents keep serving. MCP tool results are checked as encoded JSON,
including the text/image content envelope, and the MCP bridge bounds incoming desktop replies.
A `batch` or `command_batch` stops at the first step whose result no longer fits the reply budget
(that step may have run; later ones do not). MCP charges each result at its size escaped inside
the text content, so the reply still lists `completed`, `failed` and every result so far, ending
with the budget error.

Headless automation previews (`doc.render` / MCP `doc_render_preview`) allow a maximum requested
edge of 2048 pixels and a source document of at most 67,108,864 pixels. `maxSide: 0` (MCP
`max_side: 0`) still means full size, but fails if the document's longest edge exceeds 2048.
PNG results are capped at 5 MiB before base64 encoding or writing a rendered file. Preview
dimension/source checks run before compositing; the encoded-PNG check runs after encoding.
Explicit trusted-local CLI rendering keeps its existing behavior.

Batch replies have an aggregate byte budget with space reserved for the outer reply and ID.
Exhausting it stops later steps even when `stopOnError` is false. A result can exceed the reply
budget after an edit has run: the error says the operation may have completed. Earlier steps
are not rolled back; inspect state before retrying. The MCP bridge drops an oversized incoming
reply without automatically retrying the operation. Bridged screenshots are decoded with
8192-pixel edge, 16,777,216-pixel, and 64 MiB allocation ceilings, even without downscaling.

These ceilings do not implement explicit JSON-depth policy, total session/document-memory
accounting, compositor scratch-space accounting, command cancellation/duration limits, or
general per-method capabilities. Desktop screenshot capture/encoding and document import/export
still need their own operation budgets; the desktop reply ceiling applies after the UI creates
its response. A bounded output does not imply bounded command cost.

### Rendering modes

`performance.renderingMode` accepts `auto`, `gpu`, or `cpu`. Automatic is the default for new
settings; older `useGpu: false` or `gpuBackend: cpu` preferences continue to select CPU mode
until an explicit mode is saved. Changes apply at the next launch. Automatic and GPU both
fall back on graphics errors rather than risk documents. CPU compatibility composites images
on the CPU and prefers software window adapters, when available. macOS still uses Metal for
the window. A failed window renderer initialization retries once in CPU compatibility mode;
a driver process crash is detected by the startup marker on the next launch.

### Camera Raw dialog

On the desktop the dialog is a window of its own (title bar, moved and resized like any window), as Adobe Camera Raw is; the main window is dimmed and takes no input while it is open, and closing the window is Cancel. Where the platform has no extra windows (the web build) it is drawn over the main window.

**Screenshots:** `ui.screenshot` captures the main window only, and eframe cannot capture an
immediate viewport, so while Camera Raw has its own window the PNG shows the dimmed main window,
not the dialog. Read the dialog's state through `ui.inspect` (`cameraRaw`: params, histogram,
zoom, control rectangles) and drive it with the `ui` params below. To *see* it, render offscreen
(`cargo run -p photocraft-ui-egui --example snapshot -- --script '[["ui.menu.invoke", {"id": "filter.cameraRaw"}]]'`
or an `egui_kittest` harness): those embed
viewports, so the dialog is drawn over the main window and appears in the image.

`ui.menu.invoke {"id":"filter.cameraRaw","params":{}}` opens Camera Raw on the active
RGB/Grayscale layer. `params: {"smartFilter": {"layer": id, "index": i}}` opens it on an existing
Camera Raw smart filter instead (as double-clicking the filter in the Layers panel does): the
stored settings over the pixels below that filter, previewed through the filter mask; commit
then runs `layer.smartFilter.setParams` with every setting. `params.ui` accepts `set` (filter settings), `auto`, `before`, `scope`, `view`, `commit` and
`cancel`. All parts of one request are validated before any is applied; a rejected request
leaves the settings, view state, preferences and document unchanged (and closes a dialog it
opened). Unknown `ui` or settings properties, non-boolean `auto` / `before` / `commit` / `cancel`, and
`commit` together with `cancel` are errors. Point curves (`pointCurve`, `pointCurveRed`,
`pointCurveGreen`, `pointCurveBlue`) are empty or 2–16 finite points in 0–255 with inputs at
least one level apart. Commit dispatches one `filter.cameraRaw` engine command; a failed commit
keeps the dialog open for correction. Nothing else writes document history.

`ui: {"auto": true}` estimates exposure, contrast, highlights, shadows, whites, blacks,
vibrance and saturation from the original preview, weighted by alpha and selection/filter-mask
coverage. It updates those sliders and the preview without committing. Temperature, tint and
all other controls stay unchanged. Repeating Auto with the same source and white balance gives
the same settings. A combined `set` is staged first, then Auto replaces its eight controls;
`commit: true` can apply the result in the same request. Auto is a conservative SDR estimate,
not Adobe's Auto algorithm or a guarantee that every scene should be middle grey.

The read-only engine command `cameraRaw.autoSettings` exposes the same estimator to CLI/MCP:
`{"layer": id?, "temperature": 0, "tint": 0}` samples the active pixel layer (or the specified
layer), returning `{"settings": {...}}`. An editor can instead supply up to 16384 straight RGBA
`samples`, with alpha multiplied by coverage, for example when editing a smart filter's input.
Apply the returned settings with `filter.cameraRaw` to commit them; the query itself never edits
the document. Samples use the existing Camera Raw filter's document-encoded RGB working domain.

Native RAW import selects a user-supplied or bundled model-matched DCP when available (55 bundled
model identifiers, no Adobe installation needed). `raw.profiles`
accepts `{"model":"Nikon", "refresh":false}` (omit model for all cameras).
`document.inspect.rawProfile` and `cameraRaw.openingRaw.profile` identify the applied
calibration/look, provenance and Picture Control limitations. See [camera profiles](camera-profiles.md);
profile availability does not imply RAW decoder support.

**Opening a raw file.** An interactive open (File › Open, Open Recent, drag and drop, the
command line) of a camera raw developed from its sensor data shows this dialog first, titled
"Camera Raw (name)", with **Open** and **Cancel**, as Photoshop opens raws in Adobe Camera Raw.
`ui.inspect.cameraRaw.openingRaw` is then `{name, path}`. Commit (Open) re-develops the raw when
Temperature, Tint or Exposure changed (as-shot white-balance gains and develop exposure on the
sensor data; without an as-shot white balance, or without the file's bytes, they stay RGB
adjustments), applies the remaining settings as one `filter.cameraRaw` step and leaves the
document unmodified; its result is `{document, redeveloped, filter?}`. Cancel closes the
document. `app.open` and other automation opens never show the dialog. The preference
`rawDefaults.openInCameraRaw` (default `true`) turns it off.

Imported PSD Camera Raw filters whose processing settings are all mapped or neutral use this
same editor. `params.__cameraRawPsd` is reserved import/export metadata: preserve it when editing
stored parameters. Other processing fields or versions remain opaque Photoshop filters. See
[PSD Smart Filters](camera-raw-histogram.md#psd-smart-filters) for the supported settings and export
limits.

The response and `ui.inspect.cameraRaw` contain:

- `histogram`: `source` (`before` | `after`), `size`, `approximate`, 256-bin `red` / `green` /
  `blue`, `samples`, `transparent`, `invalid`, per-channel `underflow` / `overflow`, and exact
  endpoint counters `shadows` (≤0) / `highlights` (≥1). Counts describe the bounded preview
  proxy in its RGB sample domain, not full-resolution or ICC display-gamut statistics.
- `previewRevision` (advances only when the proxy is re-developed), `renderMs`, `histogramMs`
  (CPU only; zero on WebAssembly).
- `curveState` (`selected`, `drag`) and `curveRect: [left, top, right, bottom] | null` (null while
  the Curve section is closed). Changing `set.pointCurve` cancels a stale gesture.
- `scope` (the view state below), `hasScopeSelection`, `vectorscope` (`bins`, `samples`,
  `before`, `selectedRegion`) or null, `scopeMs`, `scopeRevision`, `overlayRevision`.
- `pointerReadout` and `samplerReadouts`: `position`, `space` (`rgb` 0–255 in the preview sample
  space, or `lab` through the document ICC profile → D50), `values`, `alpha`; null on invisible
  pixels.
- `hoverSample`: `position`, `rgb` (histogram domain) and `hueSaturation` (turns, 0–1; only
  while the vectorscope is shown), or null.
- `hoveredZone`, `previewRect`, `scopeRect`, `vectorscopeRect`.
- `view` (`zoom: null` for fit, or a numeric factor where 1 = 100%, `center: [u,v]`,
  `hand`), `zoom` (effective factor, null before layout), `sourceSize`, `viewportRect`.
  At 100%, one source pixel maps to one physical display pixel, including on HiDPI displays.
- `previewApproximate`, `detailPending`, `detailError`: whether the visible image still uses
  the proxy, whether full-resolution refinement is running, and any refinement error.

`params.ui.view` changes navigation only: `zoom: "fit" | 0.0001..=16`, `center: [u,v]`
(finite coordinates in 0..=1, clamped to keep the image within view), `hand: bool`. Fit recentres
and follows window resizing; navigation resets when opening a different Camera Raw dialog.
Invalid mixed requests are atomic, just like scope and filter settings. View requests do not
re-develop the proxy or change filter parameters/history.

With the Zoom tool (Z), click toggles Fit / 100%; click-drag right/left scrubs in/out.
Ctrl-drag (Command on macOS) fits a rectangular region. H selects Hand; Space temporarily pans,
including while placing colour samplers. Alt/Option + wheel zooms at the pointer; an unmodified
wheel pans. Pinch zooms. Ctrl/Command +/- zoom, 0 fits, and Alt/Option + Ctrl/Command + 0 shows
100%; Ctrl/Command+Shift-click also selects 100%. Right-click opens Fit / 100% / preset
percentages. Tool double-clicks fit the image.
Text fields retain their keys and wheel gestures over the settings panel still scroll it.

The proxy remains the fast path while changing settings. Where more source pixels are needed,
Camera Raw lazily refines with the **same engine Camera Raw pipeline** at pixelScale=1 and the
same selection/filter-mask coverage. Native refinement runs off the UI thread (one revision at
a time); stale results are discarded. Only a bounded visible crop is uploaded to the GPU, and
pan/zoom reuse developed pixels. Before and neutral settings read the original pixels directly.
Absurd proxy domains (over 512 MP, including distant sparse off-canvas pixels) are rejected
before reading pixels. Full-resolution refinement is capped at 64 MP to bound the existing full-image engine pipeline;
above that it explicitly reports unavailability and keeps the proxy. Oversized viewport crops
also retain the proxy. Without a browser worker, wasm explicitly retains the filtered proxy
rather than block its UI; Before and neutral settings still crop the original source pixels.
The histogram and vectorscope continue to analyse the bounded proxy; readouts and clipping
warnings use the full-resolution pixels once refinement is visible.

`params.ui.scope` changes presentation only: `shadows` / `highlights` (clipping warnings, U / O),
`lab`, `samplerTool` (S), `vectorscope`, `selectedRegion` (requires a document selection),
`redRight`, `hideSkinLine`, `floating`, `floatingRect: [left, top, right, bottom]` (200×150 to
4096×4096 points), `sample: [u,v] | null`, and the probe list: `samplers: [[u,v], …]` replaces it,
or `clearSamplers: true`, `removeSampler: index` and `addSampler: [u,v]` edit it, applied in that
order (the two forms can't be combined). Coordinates are normalized to the displayed proxy; at
most nine probes. `tone: {zone: "blacks|shadows|exposure|highlights|whites", delta}` adjusts the
same parameter as a histogram drag (`reset: true` sets it to 0); Exposure clamps to ±5 stops, the
others to ±100.

Hover, probe movement, theme changes and panel resizing never re-develop the image; Before/After
and region changes rebuild only the dependent analysis. Display options and the floating panel
geometry persist in preferences `dialogs["filter.cameraRaw.scope"]`; probes and vectorscope
visibility reset when the dialog opens. HDR scopes are not implemented. See
[camera-raw-histogram.md](camera-raw-histogram.md).
