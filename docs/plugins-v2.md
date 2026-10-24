# Plug-ins ABI v2 (proposal): panel plug-ins with loopback network access

> **Status:** proposal, not implemented · **Proposed:** 2026-10-12 by Mehran Ahmadi (author of
> [OpenLayer](https://github.com/MehranMarxian/OpenLayer)) · **Tracks:** #219 (*Plug-in ABI v2:
> panel kind, progress/cancel, other layers*), scorecard `AUTO-219-6`, gap G16 · **Related:** #41

This proposal extends the WebAssembly plug-in ABI ([`plugins.md`](plugins.md)) with a second kind,
`panel`: a long-lived plug-in that draws a declarative panel, reads the document, adds layers as one
undo step, and, if the user grants it, talks over HTTP to a server **on the user's own machine**.
It keeps every v1 guarantee, including the strongest one: **a plug-in module still has no imports.**
All effects are data the plug-in returns and the host carries out, so the sandbox stays checkable
by reading the module's import table.

The first client is OpenLayer, an MIT-licensed ComfyUI plugin for Photoshop with 14 generation tools
and 34 workflow presets. The first implementation PR is sized to run two of its tools end to end in
PhotoCraft: **Text to Image** and **Inpaint**.

## Contents

1. [Why](#why)
2. [Goals and non-goals](#goals-and-non-goals)
3. [The model: events in, operations out](#the-model-events-in-operations-out)
4. [ABI v2](#abi-v2) (exports, manifest, events, operations, blobs)
5. [Panels](#panels)
6. [Document access](#document-access)
7. [Network capability](#network-capability)
8. [Limits](#limits)
9. [Lifecycle and threading](#lifecycle-and-threading)
10. [Consent and security](#consent-and-security)
11. [Commands, control channel and MCP](#commands-control-channel-and-mcp)
12. [Worked examples](#worked-examples)
13. [Implementation plan](#implementation-plan)
14. [Testing](#testing)
15. [Open questions for the maintainers](#open-questions-for-the-maintainers)
16. [Later (v2.x)](#later-v2x)

## Why

- **#219 already plans it.** The issue lists "a declarative `panel` kind rendered by the app,
  progress and cancel host calls, access to other layers" for ABI v2. This document is a concrete
  design for that line, plus the one capability a real panel needs and v1 forbids: network.
- **Users ask for local models** (#2313, #1996, #2682). ComfyUI is the most common way artists run
  them, and it already runs as a local HTTP server.
- **v1 can't host such a plug-in, by design:**
  - `runtime.rs` rejects any module with an import, so there is no way to reach a server.
  - Every band runs in a fresh instance (`filter_buffer`), so a plug-in can't hold state while a
    generation takes 10–60 s.
  - `Kind::Filter` is the only kind (`manifest.rs`), so there is no UI beyond a generated dialog,
    and a plug-in can't add a layer.
- **Without it, the alternative is worse.** An external tool can already drive PhotoCraft over the
  control channel (`doc.render`, `doc.place`, `engine.execute`), but it needs the token, read and
  write roots, and a separate window. Nothing in the app tells the user what it can touch. A panel
  plug-in is safer: it is sandboxed, its capabilities are declared and approved in the app, and
  everything it does is an ordinary undo step.

## Goals and non-goals

**Goals**

- A `panel` plug-in kind with a host-drawn, declarative UI that follows `theme::Tokens`, so it
  matches the app.
- Reading the active layer, the composite and the selection, and adding layers (with masks) as one
  named undo step, through the engine like any command.
- HTTP to **loopback addresses only** (`127.0.0.0/8`, `::1`, `localhost`), declared in the
  manifest and approved by the user.
- Long-running work (polling a server for minutes) without blocking the UI, the document or other
  jobs.
- Everything drivable by agents: panel state in `ui.inspect`, every button press a command.
- Every v1 rule still holds: no imports, never crash, never break wasm, no colour assumptions.

**Non-goals for v2.0**

- Hosting UXP, CEP or `.8BF` (unchanged: [`plugins.md`](plugins.md)).
- Network access to anything off the machine. That is a policy question for #41, not for this ABI.
- PhotoCraft shipping, choosing or downloading any model. The plug-in and the server are the
  user's.
- WebSockets, file-system access, plug-in-provided file formats, scripting other plug-ins.

## The model: events in, operations out

A v2 panel plug-in is a **state machine**. The host calls one export, `pc_event`, with an event as
JSON. The plug-in updates its state and returns a JSON array of **operations** (requests) for the
host. The host carries them out asynchronously and later delivers each result as another event.

```text
             ┌────────── event (JSON) ──────────┐
   host ─────┤ init, ui.click, http.response,   ├────► pc_event(ptr, len) ─┐
             │ timer, doc.captured, …           │                         │  plug-in state
             └──────────────────────────────────┘                         │  (one instance,
   host ◄──── [ {op: "http.request", …},  ◄──────────── returns ops ───────┘   lives on)
               {op: "ui.view", …}, … ]
```

Why this shape:

- **No imports.** v1's import check stays exactly as it is. Reviewing what a plug-in *can* do
  means reading its manifest, not auditing host functions.
- **No blocking inside the interpreter.** wasmi never waits on a socket. That is also what makes it
  work on the web build, where `fetch` is asynchronous and `jobs::run` runs inline.
- **One call at a time per instance.** Events are delivered in order, never concurrently, so a
  plug-in needs no locking and the host needs no re-entrancy.
- **Easy to test.** A test feeds events and asserts on the returned operations, with no network
  and no window.

## ABI v2

### Exports

| Export | Signature | Meaning |
|---|---|---|
| `memory` | memory | As v1. |
| `pc_abi_version` | `() -> i32` | Returns `2`. The host keeps accepting `1` for filters. |
| `pc_manifest` | `() -> i64` | As v1: `(len << 32) \| ptr` of the manifest JSON (≤ 64 KiB). |
| `pc_alloc` | `(size) -> ptr` | As v1, but blocks may be freed (below). `0` = out of memory. |
| `pc_free` | `(ptr, size) -> ()` | **New, panel kind only.** The host frees the event block after `pc_event` returns. |
| `pc_event` | `(ptr, len) -> i64` | **New.** `ptr..ptr+len` holds one UTF-8 JSON event. Returns `(len << 32) \| ptr` of a UTF-8 JSON array of operations (≤ 1 MiB), which the host reads, then frees with `pc_free`. `0` means "no operations". |

A `filter` plug-in may also declare ABI 2; it then gets the v2 manifest fields but keeps
`pc_filter` and the fresh-instance-per-band model. Only `panel` plug-ins get a persistent
instance and `pc_event`.

### Manifest additions

```json
{
  "id": "com.openlayer.comfyui",
  "name": "OpenLayer",
  "version": "0.38.0",
  "kind": "panel",
  "author": "Mehran Ahmadi",
  "description": "Local AI layers from your own ComfyUI.",
  "panel": {"title": "OpenLayer", "minWidth": 260},
  "capabilities": {
    "document": ["read", "addLayers"],
    "network": {"hosts": ["127.0.0.1:8188"], "userCanEdit": true},
    "storage": true
  }
}
```

| Field | Meaning |
|---|---|
| `kind: "panel"` | New kind. The panel is listed under **Window › Plug-ins › {name}**. |
| `panel.title`, `panel.minWidth` | Dock tab label (1–64 printable characters, the same rule as `name`) and minimum width in points (160–640). |
| `capabilities.document` | `"read"` (`doc.info`, `doc.capture`) and/or `"addLayers"` (`doc.commit`). Anything not listed is refused at run time with an `error` event. |
| `capabilities.network.hosts` | Loopback `host:port` pairs; `"127.0.0.1:*"` asks for any port. Anything that isn't loopback makes the manifest invalid. `userCanEdit` lets the user change the list in Preferences (a ComfyUI on another port). |
| `capabilities.storage` | A small per-plug-in key/value store (≤ 1 MiB) kept with the preferences. |

The v1 manifest rules (id, name, `params`) are unchanged. A capability the plug-in doesn't declare
is never available, whatever the user grants.

### Events (host → plug-in)

Every event is `{"type": …, "now": <ms since the Unix epoch>, …}`. `now` is the only clock a
plug-in gets.

| `type` | Fields | When |
|---|---|---|
| `init` | `host {app, version, abi, platform}`, `locale`, `storage {…}`, `entropy` (32 bytes, hex), `grants {…}` | Once, after instantiation. `entropy` seeds the plug-in's own PRNG (generation seeds); v2 has no randomness import, as v1. |
| `ui.click` | `id`, `values {widgetId: value}` | A `button` was pressed. `values` holds every input in the panel. |
| `ui.change` | `id`, `value`, `values` | An input was committed (Enter, focus lost, slider released, choice picked). Not sent per keystroke. |
| `panel.visible` | `visible` | The panel was shown or hidden. A plug-in may pause polling while hidden. |
| `doc.changed` | `doc` (or `null`), `revision` | The active document changed or was closed. Coalesced to at most one per frame. |
| `result` | `id`, `ok`, then the operation's result fields, or `error` | An operation that has an `id` finished (see the operations below). |
| `cancel` | — | The user pressed Cancel on the plug-in's progress (status bar or panel). |

### Operations (plug-in → host)

Each operation is `{"op": …, "id"?: <string or number>, …}`. Operations with an `id` get exactly one
`result` event; the others are fire-and-forget. Unknown operations, bad fields, and capabilities
that weren't granted all produce `result {ok: false, error}` (or, with no `id`, a notice in the
panel). They never panic.

| `op` | Fields | Result |
|---|---|---|
| `ui.view` | `view` (a widget tree, see [Panels](#panels)) | — |
| `ui.set` | `values {widgetId: value}` | — Changes input values without replacing the view. |
| `ui.progress` | `fraction` (0–1 or `null` for indeterminate), `text`, `cancellable`, or `done: true` | — Mirrors the status-bar job UI and its Cancel button. |
| `ui.notice` | `level` (`info`, `warning`, `error`), `text` | — A host notice, as `notices` in `ui.inspect`. |
| `timer.set` | `id`, `ms` (16–600,000) | `result` after `ms`. |
| `timer.clear` | `id` | — |
| `storage.set` | `key`, `value` (any JSON; `null` deletes) | — |
| `doc.info` | `id` | `doc`, `revision`, `name`, `width`, `height`, `mode`, `depth`, `activeLayer {id, name, kind, bounds}`, `selection {x, y, width, height}` or `null`. |
| `doc.capture` | `id`, `source`, `rect`, `alpha`, `maxSide?`, `bits?` | `doc`, `revision`, `blob`, `rect {x, y, width, height}`, `width`, `height` (see [Document access](#document-access)). |
| `doc.commit` | `id`, `doc`, `label`, `layers [ … ]`, `ifRevision?` | `layers [{id, name}]` (see [Document access](#document-access)). |
| `http.request` | `id`, `method`, `url`, `headers?`, `body?`, `response`, `timeoutMs?` | `status`, `headers`, and `json`, `text` or `blob` (see [Network](#network-capability)). |
| `blob.free` | `blob` | — |

### Blobs

Image bytes never pass through the interpreter. The host keeps them as **blobs**: immutable byte
buffers owned by one plug-in instance and named by an opaque id (`"b:17"`). A capture returns a
blob, an HTTP response can be kept as a blob, an HTTP body or a multipart part can be a blob, a
commit takes blobs, and an `image` widget shows one.

The interpreter costs 10–100× native speed ([`plugins.md`](plugins.md#performance)), so encoding a
24 MP PNG inside a plug-in would take minutes. With blobs it costs nothing, and the plug-in's
memory stays at the size of its JSON. Blobs are freed by `blob.free`, when the instance stops, and
when the per-instance total would exceed its cap ([Limits](#limits)). In the last case the
operation that would cross the cap fails instead.

## Panels

The panel is a tree of widgets the host draws with egui and `theme::Tokens`. A plug-in can't
choose colours, fonts or sizes. That keeps every plug-in on-theme in light and dark, and makes the
panel readable through `ui.inspect`.

| Widget | Fields | Notes |
|---|---|---|
| `column`, `row` | `children` | Layout. |
| `section` | `title`, `collapsed?`, `children` | Collapsible group (its state is kept per plug-in). |
| `label` | `text`, `level?` (`muted`, `info`, `warning`, `error`) | Plain text, no markup. |
| `text` | `id`, `value`, `placeholder?`, `multiline?`, `maxLength?` (≤ 8,000) | Prompt fields. |
| `number` | `id`, `value`, `min`, `max`, `step?`, `slider?` | `step` is honoured. |
| `toggle` | `id`, `value`, `label` | |
| `choice` | `id`, `value`, `options [{value, label}]` | ≤ 256 options. |
| `button` | `id`, `label`, `primary?`, `enabled?` | Sends `ui.click`. |
| `progress` | `fraction` or `null` | Inside the panel; `ui.progress` drives the status bar. |
| `image` | `blob`, `maxHeight?` | Shows a PNG or JPEG blob, decoded off the UI thread with the same caps as capture. |
| `separator` | — | |

Ids are `[A-Za-z0-9_.-]{1,64}` and unique in the view. The tree has at most 512 widgets and depth
16. A bad view is rejected whole with a notice, and the last good view stays.

Example (the start of OpenLayer's Text to Image):

```json
{"op": "ui.view", "view": {"type": "column", "children": [
  {"type": "choice", "id": "tool", "value": "txt2img",
   "options": [{"value": "txt2img", "label": "Text to Image"}, {"value": "inpaint", "label": "Inpaint"}]},
  {"type": "text", "id": "prompt", "value": "", "multiline": true, "placeholder": "Describe the image"},
  {"type": "section", "title": "Settings", "collapsed": true, "children": [
    {"type": "number", "id": "steps", "value": 4, "min": 1, "max": 50, "step": 1, "slider": true},
    {"type": "number", "id": "seed", "value": 0, "min": 0, "max": 4294967295, "step": 1}]},
  {"type": "button", "id": "generate", "label": "Generate", "primary": true},
  {"type": "label", "id": "status", "text": "ComfyUI: connected", "level": "muted"}]}}
```

## Document access

### `doc.capture`

| Field | Values |
|---|---|
| `source` | `"composite"` (merged visible pixels), `"layer"` (the active layer's own pixels, or `{"layer": id}`), `"selection"` (the selection as grayscale coverage). |
| `rect` | `"canvas"`, `"selection"` (its bounds), `{"selection": {"pad": px}}` (bounds grown by `pad`, for inpaint context), or `{x, y, width, height}`. Always clamped to the canvas. An empty rect is an error. |
| `alpha` | `"source"` (default), `"none"`, `"selection"` (alpha = selection coverage), `"selectionInverted"` (alpha = 1 − coverage). |
| `maxSide` | Optional downscale (Lanczos) so the longest side is at most this. The result reports the real `width`/`height`. |
| `bits` | `8` (default) or `16`. |

The result is a PNG blob in **sRGB**, converted from the document's own mode and depth through
`photocraft-cms` (`transform::cached`), never assumed (Rule 2). Gray, CMYK, Lab and 32-bit
documents work the same way. `rect` reports where the pixels came from, so the plug-in can put the
result back in the same place.

`"selectionInverted"` exists because ComfyUI's `LoadImage` reads `mask = 1 − alpha`. OpenLayer's
inpaint presets send one PNG with the mask in its alpha (`fluxFillMaskBridge.ts`), and the host can
build that in one pass.

Captures run as background work against a copy-on-write snapshot. They don't lock the document,
and a capture is cancelled if its document closes.

### `doc.commit`

```json
{"op": "doc.commit", "id": "c1", "doc": 3, "label": "OpenLayer: Inpaint",
 "layers": [{"blob": "b:21", "name": "OpenLayer_Inpaint", "x": 812, "y": 440,
             "scaleTo": {"width": 512, "height": 512},
             "mask": {"blob": "b:19"}, "above": "active"}]}
```

- One commit is **one undo step**, named `label`, applied through `Session::edit`: journal,
  history, damage and post-command hooks behave as for any command. Up to 32 layers per commit
  (OpenLayer's Unflatten makes several).
- Each layer is a new pixel layer decoded from a PNG or JPEG blob, converted **from sRGB into the
  document's mode and depth** through `photocraft-cms`, and placed at `x, y` in document
  coordinates. `scaleTo` resamples first (results come back at the model's size).
- `mask` is `{"blob": …}` (a grayscale PNG, the same size as the layer after `scaleTo`) or
  `"selection"` (the current selection). It becomes the layer's mask. That replaces the multi-step
  "mask sandwich" (temporary layers, a composite-channel load, then cleanup) OpenLayer needs in
  Photoshop to turn a saved selection into a layer mask.
- `above`: `"active"` (default) or `"top"`.
- `doc` must be the document the work started from. If it was closed, the commit fails and nothing
  is touched. If `ifRevision` is given and the document changed since then, the commit fails
  instead, so the plug-in can ask the artist. Without it, the commit applies to the current state,
  as Photoshop's own result import does.
- If a background job holds the document lock, the commit waits for it (up to 60 s) rather than
  failing. A plug-in can't start an engine job itself.

## Network capability

### What a plug-in can send

```json
{"op": "http.request", "id": "up1", "method": "POST", "url": "http://127.0.0.1:8188/upload/image",
 "body": {"multipart": [
   {"name": "image", "filename": "ol_src_3f2a.png", "contentType": "image/png", "blob": "b:18"},
   {"name": "type", "text": "input"}, {"name": "overwrite", "text": "true"}]},
 "response": "json", "timeoutMs": 60000}
```

- `method`: `GET`, `POST`, `PUT`, `DELETE`.
- `url`: `http://` only (loopback needs no TLS). The host must literally be `127.x.x.x`, `[::1]`
  or `localhost`, and `host:port` must be on the granted list. `localhost` is connected as
  `127.0.0.1` with no DNS lookup, so DNS rebinding can't redirect it.
- `body`: `{"json": …}`, `{"text": "…"}`, `{"blob": id}` or `{"multipart": [parts]}`. The host
  builds multipart bodies itself (boundary, filenames, content types). OpenLayer had to hand-build
  them because UXP's `FormData` drops filenames.
- `headers`: `Content-Type`, `Accept` and `Authorization` only. The host sets `Host`,
  `Content-Length` and `User-Agent: PhotoCraft-plugin/<id>`, and sends no cookies.
- `response`: `"json"` (parsed, ≤ 8 MiB), `"text"` (≤ 8 MiB), `"blob"` (≤ 256 MiB, kept host-side)
  or `"none"`.
- Redirects are not followed. A 3xx is returned as is.

The result is `{status, headers {content-type, …}, json | text | blob}`. A connection error,
timeout, size cap or refused URL gives `ok: false` and a plain-language `error` ("Nothing is
listening on 127.0.0.1:8188", "the response is larger than 8 MiB").

### Host implementation

- **Trait in `photocraft-plugins`** (`HostNet`), so the crate stays L4 and wasm-clean. The engine
  holds an `Arc<dyn HostNet>` injected at startup, like the other platform services.
- **Native:** requests run on a small pool (≤ 8 in flight per plug-in) and never on the UI thread.
  The workspace has no HTTP client today. Since only loopback HTTP is allowed, there are two cheap
  options: `ureq` 3 with default features off (no TLS, MIT OR Apache-2.0), or about 300 lines over
  `std::net::TcpStream` (HTTP/1.1, `Content-Length` and chunked). Either way there is no TLS and
  no async runtime.
- **Web:** `fetch` through `web-sys` in `apps/photocraft-web`. Browsers allow
  `http://127.0.0.1` from a secure page in principle, but Private Network Access and CORS
  (`ComfyUI --enable-cors-header`) still need checking in each browser. **Proposal: the first PR
  ships the web build with a `HostNet` that always answers "network is not available in the web
  build yet".** The wasm gate stays green, and web support follows once it is verified.
- **Headless CLI and `serve`:** no network service by default, so CI and agents never reach the
  network by accident.

## Limits

| Limit | Default | On violation |
|---|---|---|
| Module size, linear memory, call depth | As v1 (32 MiB, 512 MiB, 1,024) | As v1 |
| Fuel per `pc_event` | 50 M instructions (v1's `fuel_base`) | Instance stopped: `Limit("instruction budget")` |
| Wall clock per `pc_event` | 2 s (native) | Instance stopped |
| Operations per event | 64, total ≤ 1 MiB | Event rejected, instance stopped |
| Pending timers | 32 | `result {ok: false}` |
| HTTP requests in flight | 8 | Queued (up to 64), then refused |
| HTTP inline response | 8 MiB | `ok: false` |
| Blob size / total per instance | 256 MiB / 1 GiB | `ok: false` |
| Capture size | 16,384 px per side and 64 MP before `maxSide` | `ok: false` |
| Storage | 1 MiB per plug-in | `storage.set` ignored, notice shown |
| Event queue | 1,024 (inputs and `doc.changed` coalesce) | Oldest UI events dropped |

## Lifecycle and threading

- **Instantiated when the panel is first opened** (or at startup, if it was open when the app last
  quit). There is one instance per plug-in per process, matching v1's registry.
- **Native:** each instance lives on its own worker thread with an event queue. Operations that
  need the `Session` (`doc.*`) go to the UI thread and are drained once per frame next to
  `Session::poll_jobs`, so a plug-in never touches the document from a worker. Captures and commit
  decoding run as `jobs::run` work.
- **Web:** the instance runs on the main thread. A `pc_event` that runs out of its fuel slice
  yields and resumes next frame (wasmi's `call_resumable`, already used by `Plugin::call`), so the
  UI keeps drawing.
- **Failure:** a trap, an exhausted budget, a malformed operation list or a bad pointer stops the
  instance. Its blobs, timers and in-flight requests are dropped, the panel shows the error with a
  **Restart** button, and the document is untouched. Three failures within a minute leave it
  stopped until the user restarts it.
- **Reinstall or remove:** the old instance is stopped, and the new one gets a fresh `init`.

## Consent and security

**At install** (and when an update **widens** its capabilities), PhotoCraft shows what the plug-in
asks for in plain words:

> **OpenLayer** wants to:
> - read the active layer, the visible canvas and the selection of your open documents
> - add layers to your documents (each addition can be undone)
> - connect to **127.0.0.1:8188**, a program on this computer. What it sends there may include
>   pixels from your documents.
>
> [Don't allow] [Allow]

- Grants are stored in Preferences › Plug-ins, keyed by plug-in id and module SHA-256, and can be
  revoked or edited there. An update with the same or fewer capabilities keeps its grant.
- **Automation can't grant capabilities.** `prefs.set` already refuses the `plugIns` section
  (`crates/automation/src/workspace.rs`), so an agent with the control token can't give a plug-in
  network access. Installing over the control channel installs the plug-in ungranted. Its panel
  then asks the user in the app.
- **What this doesn't protect against:** a granted plug-in can send document pixels to the
  loopback service it was granted. That is the feature, and it is why the prompt says so. The
  service itself (ComfyUI and its custom nodes) is outside PhotoCraft's sandbox, exactly as it is
  today when an artist uses it alongside the app.
- Network-capable plug-ins never run in headless sessions unless the host passes a `HostNet`
  explicitly.

## Commands, control channel and MCP

Following "everything is a command" and #219's xtask rule that every panel button routes through a
command id:

| Command | Params | What it does |
|---|---|---|
| `plugin.panel.open` / `plugin.panel.close` | `{id}` | Shows or hides the panel (Window › Plug-ins). |
| `plugin.panel.inspect` | `{id}` | `{state: running/stopped/error, view, values, progress, grants}`. Also included in `ui.inspect` as `pluginPanels`. |
| `plugin.panel.event` | `{id, event: {type: "ui.click" or "ui.change", id, value?}}` | What a click or edit in the panel does, so agents and tests drive it exactly like a user. |
| `plugin.panel.restart` | `{id}` | Restarts a stopped instance. |
| `plugin.panel.commit` | internal | The undo step `doc.commit` creates, recorded in history under the plug-in's `label`. It isn't recordable in Actions: its blobs exist only in the running instance. |

`plugin.list` gains `kind: "panel"`, `capabilities` and `grants`. The existing `plugin.install`,
`plugin.remove` and `plugin.reload` work unchanged.

## Worked examples

### Text to Image (OpenLayer preset `txt2img-flux2-klein`, Apache-2.0)

| # | Event in | Operations out |
|---|---|---|
| 1 | `ui.click {id: "generate", values: {prompt, steps, seed}}` | `doc.info {id: 1}`, `ui.progress {fraction: null, text: "Submitting", cancellable: true}` |
| 2 | `result {id: 1, doc: 3, width: 4000, height: 3000, …}` | `http.request {id: 2, POST /prompt, body: {json: {prompt: <workflow>, client_id}}, response: "json"}` |
| 3 | `result {id: 2, status: 200, json: {prompt_id: "a1"}}` | `timer.set {id: "t", ms: 1000}` |
| 4 | `result {id: "t"}` | `http.request {id: 3, GET /history/a1}`, `http.request {id: 4, GET /queue}` |
| 5 | (repeats 3–4 until history shows `a1` done; status text shows the queue position) | |
| 6 | `result {id: 3, json: {a1: {outputs: …}}}` | `http.request {id: 5, GET /view?filename=…&type=output, response: "blob"}` |
| 7 | `result {id: 5, blob: "b:4"}` | `doc.commit {id: 6, doc: 3, label: "OpenLayer: Text to Image", layers: [{blob: "b:4", name: "OpenLayer_Generated", x: 1488, y: 988}]}` |
| 8 | `result {id: 6, layers: [{id: 57}]}` | `ui.progress {done: true}`, `blob.free {blob: "b:4"}`, `ui.set {values: {…}}` |

`cancel` at any step: the plug-in dequeues **its own** prompt (`POST /queue {"delete": ["a1"]}`).
If the prompt is already running, it stops waiting and discards the result. It never sends a
global `/interrupt`: ComfyUI may be shared with other work, and OpenLayer already follows that
rule.

### Inpaint (preset `inpaint-flux2-klein`, crop and stitch)

| # | Event in | Operations out |
|---|---|---|
| 1 | `ui.click {id: "generate", values: {tool: "inpaint", prompt, contextPadding: 64}}` | `doc.info {id: 1}` |
| 2 | `result {id: 1, selection: null}` | `ui.notice {level: "warning", text: "Make a selection first"}`. Stop. |
| 2′ | `result {id: 1, doc: 3, selection: {x: 900, y: 520, width: 340, height: 300}}` | `doc.capture {id: 2, source: "composite", rect: {selection: {pad: 64}}, alpha: "selectionInverted"}`, `doc.capture {id: 3, source: "selection", rect: {selection: {pad: 64}}}` |
| 3 | `result {id: 2, blob: "b:7", rect: {x: 836, y: 456, width: 468, height: 428}}` and `result {id: 3, blob: "b:8"}` | `http.request {id: 4, POST /upload/image, multipart: [image: b:7, …]}` |
| 4 | `result {id: 4, json: {name: "ol_src_…png"}}` | `/prompt` with the uploaded name injected, then poll as in Text to Image |
| 5 | `result {…, blob: "b:9"}` (the stitched patch, 468 × 428) | `doc.commit {id: 9, doc: 3, label: "OpenLayer: Inpaint", layers: [{blob: "b:9", name: "OpenLayer_Inpaint", x: 836, y: 456, mask: {blob: "b:8"}}]}` |

The mask is the selection as it was **when the capture was taken**. The artist can keep working
during the 10–60 s of generation, and the patch still lands under the same edge.

## Implementation plan

### PR 1 (PhotoCraft): ABI v2 core, enough for Text to Image and Inpaint

Hours are Opus 5.5 agent wall-clock hours, calibrated as in
[target-app-parity](target-app-parity.md#remaining-effort-and-calibration).

| Part | Where | Hours |
|---|---|---:|
| ABI v2 load path: `pc_event`/`pc_free`, the panel manifest and capabilities, persistent `Instance`, event queue, fuel and wall-clock per event | `crates/plugins/src/{runtime,manifest}.rs`, new `instance.rs`, `ops.rs` | 14–22 |
| Blob store, operation validation, timers, storage | `crates/plugins/src/{blobs,ops}.rs` | 6–10 |
| `HostNet` trait, native loopback client, web stub | `crates/plugins/src/net.rs`, app wiring | 8–14 |
| `doc.info`, `doc.capture` and `doc.commit` with cms conversion, masks, one undo step; the `plugin.panel.*` commands | `crates/engine/src/plugin_cmds.rs` (or a new `plugin_panel_cmds.rs`) | 10–16 |
| Declarative panel renderer, dock tab, Window › Plug-ins, consent dialog, Preferences › Plug-ins grants | `crates/ui-egui/src/plugin_ui.rs`, new `plugin_panel_ui.rs`, `prefs_ui` | 12–20 |
| `ui.inspect` / MCP exposure, docs (`plugins.md` gains a v2 section) | `crates/ui-egui/src/control.rs`, `docs/` | 3–6 |
| Tests (see [Testing](#testing)) | `crates/plugins/tests/abi_v2.rs`, engine tests | 8–12 |
| **Total** | | **61–100** |

### Alongside, in OpenLayer's repo: the reference plug-in

A Rust crate (`photocraft/` in the OpenLayer repo) building `openlayer.wasm`. It holds the two
presets' API graphs and their injection maps, generated from `presetRegistry.ts` at build time so
the TypeScript registry stays the single source of truth. It handles the ComfyUI round trip
(upload, prompt, poll, view, own-prompt cancel) and the two tools above. About 16–28 h. It doubles
as the end-to-end test plug-in for PR 1 against a fake ComfyUI.

### Later PRs

The other twelve OpenLayer tools need nothing new except **canvas growth** (Outpaint),
**document resize** (Upscale's "resize document" mode) and **layer groups** (Unflatten). Each is
one more `doc.commit` option, so it is added when its tool is ported.

## Testing

- **Hostile modules** (WAT fixtures, as in `tests/abi.rs`): `pc_event` returning pointers outside
  memory, lengths over the cap, invalid UTF-8 or JSON, 10,000 operations, infinite loops, traps,
  memory growth to the cap, unknown ops, ops for capabilities that weren't granted, view trees
  2,000 deep. Each must become an error or a stopped instance, never a panic.
- **`panic_hunt`** covers the `plugin.panel.*` commands with adversarial params (Rule 9).
- **Fake loopback server** in tests (a `TcpListener` on port 0 serving ComfyUI-shaped replies):
  slow responses, a connection dropped mid-body, 500s, oversized bodies, chunked encoding,
  redirects (must not be followed), non-loopback URLs (must be refused before connecting).
- **Document round trips:** capture → commit with no model in the loop (the fake server echoes the
  upload) on 8/16/32-bit RGB, Gray, CMYK and Lab documents, checking the pixels come back within
  conversion tolerance and in the right place. Commit after the document closed; commit with a
  stale `ifRevision`; commit while a job holds the lock.
- **wasm gate:** `cargo xtask wasm` stays green with the stub `HostNet`.
- **Visual check:** the snapshot example renders a panel from a fixture view in light and dark.

## Open questions for the maintainers

1. **#41.** Is loopback-only network for third-party plug-ins acceptable before #41's backend
   decision? This proposal's position: PhotoCraft still ships no model, no AI code and no network
   path off the machine. It gains a sandboxed way for the user's own local tools to add layers.
   That is compatible with #41's planned provider interface, and a later `provider` kind could make
   a local ComfyUI one of its backends (below).
2. **HTTP client:** `ureq` without TLS, or a hand-written HTTP/1.1 client over `std::net`?
3. **Web build:** ship the stub first (proposed), or require web support in PR 1?
4. **Distribution and trust:** are unsigned `.wasm` files plus the consent prompt enough for v2.0,
   or should a plug-in catalogue or signature come first?
5. **Storage location:** inside the preferences file, or a per-plug-in file next to it (AppImage and
   Flatpak paths)?
6. **One panel per plug-in,** or several (OpenLayer has a separate Preview panel in Photoshop)?
   This proposal says one for v2.0; the `image` widget covers the preview.

## Later (v2.x)

- **`provider` kind for #41:** a plug-in that implements Edit › Generative Fill / Generative
  Expand behind the provider interface #41 plans, so the menu items work with a local backend.
- **WebSocket** (`ws://` loopback) for live progress and OpenLayer's Live Painting.
- **Filter plug-ins on ABI 2:** `progress` and `cancel` for `pc_filter` and Last Filter repeats
  (the rest of `AUTO-219-6`).
- **Reading other layers** for filter plug-ins (also `AUTO-219-6`). Panel plug-ins get it through
  `doc.capture {source: {layer: id}}` from v2.0.
- **Remote hosts**, only if and as #41 decides.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-12 | major | First version of the proposal |
