# Plug-ins: the WebAssembly plug-in API (ABI v2 panels)

ABI v1 ([`docs/plugins.md`](plugins.md)) hosts one kind of plug-in: a filter, a pure function from
pixels to pixels, run in a fresh instance per band with no state of its own. A panel needs more: it
holds a model — the text the user has typed, the document it is watching — and needs to ask the
host questions (*what is the active document? capture this region; add these layers*) and be told
when things change. Neither fits a function torn down after every call.

ABI v2 adds **panels**: a plug-in the host keeps **one live instance of**, sends **events** to, and
which answers with a JSON **array of operations** for the host to carry out. The ABI version is
`2`; a panel declares `"kind": "panel"`. v1 is unchanged: a filter still declares `kind: "filter"`,
still exports `pc_filter`, and still runs per band, and an ABI 2 filter keeps the per-band model and
only gains the v2 manifest fields. Both kinds are installed, listed, removed and reloaded by the
same `plugin.*` commands, in one `.wasm`-per-plug-in world.

The sandbox is the same one, and its defining choice is unchanged: **a v2 module has no host
imports at all** — the shape [#4301](https://github.com/storytold/photocraft/pull/4301) argues for in
its own v2 proposal, and the one this host builds. There is no `pc_host.*` import surface, no WASI,
no file system, no network, no clock, no randomness. Everything a panel says and everything it is
told crosses through the single `pc_event` entry point, as UTF-8 JSON. The only clock is the `now`
field on every event; the only randomness is the `entropy` string in `init`. The host owns the widget
tree, the event queue, the timers and the image bytes; the plug-in owns only its own state machine.

This page is the contract for panel authors and the reference for the host
(`crates/plugins`, the `plugin.panel.*` commands in `crates/engine/src/plugin_panel_cmds.rs`).

## Exports

A panel is a core WebAssembly module with **no imports** and these exports. All integers are `i32`
unless noted; pointers are offsets into the module's linear memory.

| Export | Signature | Meaning |
|---|---|---|
| `memory` | memory | The module's linear memory. |
| `pc_abi_version` | `() -> i32` | Must return `2` for a panel. This host accepts `1` and `2`; a module that declares `kind: "panel"` at a version below 2 is rejected. |
| `pc_manifest` | `() -> i64` | `(len << 32) \| ptr` of the UTF-8 manifest JSON (≤ 64 KiB). Read once at load. |
| `pc_alloc` | `(size) -> ptr` | A block of `size` bytes the host may write, or `0` on failure. |
| `pc_free` | `(ptr, size)` | Releases a block the host no longer needs. Required for panels. |
| `pc_event` | `(ptr, len) -> i64` | Handles one event; returns `(len << 32) \| ptr` of the UTF-8 JSON operations array, or `0` for "no operations". |

`pc_alloc`, `pc_free` and `pc_event` must be typed exactly as above or the module is rejected at
load (`pc_alloc` is checked for every plug-in, `pc_event` and `pc_free` for a panel, `pc_filter` for
a filter). A panel must not also be a filter, and a filter is refused by
`plugin.panel.open` ("is a filter, not a panel").

### The `pc_event` calling convention

For each event the host calls `pc_alloc(len)` (never zero bytes — a zero-byte allocation is not
portable, so at least one is asked for), writes the event's UTF-8 JSON into the block after checking
it lies inside the module's memory and within the host's memory cap, calls `pc_event(ptr, len)` with
the true event length, then calls `pc_free` on the event block and again on the reply block the
return value named. A `pc_alloc` that returns `0` fails with `plug-in failed: pc_alloc returned 0
(out of memory)`.

Both buffers are handed back after every event, so a long-lived panel does not grow without bound.
The module's `pc_free` must release a block the host allocated; the example only releases the block
at the top of its heap, which is sufficient because the host allocates one event block per call and
frees it before the next.

The reply is an **array** of operation objects, encoded as UTF-8 JSON. `0` means "no operations".
An empty or whitespace-only reply also means "no operations". What else happens:

| Reply | Result |
|---|---|
| `0` (or an empty/whitespace reply) | no operations; nothing is carried out |
| a reply block pointing outside memory | `plug-in ABI: pc_event returned a block outside memory` |
| more than 1 MiB of operations | `plug-in exceeded its pc_event returned N bytes of operations (limit 1048576)` |
| not JSON | `plug-in operation: operations: …` (with a short printable preview of the reply) |
| valid JSON but not an array (for example `{"op": …}`) | the same `plug-in operation` error |
| more than 64 operations | `plug-in exceeded its N operations (limit 64)` |
| an unknown `op`, or one whose fields fail validation | `plug-in operation: …` |
| a trap, or an exhausted instruction/time budget | `plug-in trapped: …` / `plug-in exceeded its instruction budget` |

Any of these stops the panel (see **Sandbox and limits**).

## Manifest

A panel returns this from `pc_manifest`. The v2 additions are `kind`, `panel` and `capabilities`;
everything else has the v1 rules in [`docs/plugins.md`](plugins.md).

```json
{
  "id": "org.example.lettering", "name": "Lettering…", "version": "1.0.0",
  "kind": "panel", "author": "Example", "description": "Bubble lettering over loopback.",
  "panel": { "title": "Lettering", "minWidth": 300 },
  "capabilities": {
    "document": ["read", "addLayers"],
    "network": { "hosts": ["127.0.0.1:8188"], "userCanEdit": true },
    "storage": true
  }
}
```

- `kind`: `"filter"` or `"panel"`. A panel **must** declare the `panel` block; a `panel` block on a
  filter is rejected ("`panel` is only for a `panel` plug-in").
- `panel.title`: the dock-tab label (1–64 printable characters; `name`'s rule, including the ban on
  control characters and Unicode format characters, applies). It is what the user sees, in the
  window title and the Window › Plug-ins row.
- `panel.minWidth`: the narrowest the user may drag the panel, in points. Default `260`, and it
  must be in `160..=640`.
- `capabilities.document`: `[]` (default), `["read"]` or `["read", "addLayers"]`. `"read"` covers
  `doc.info`, `doc.capture` and the reading commands `doc.command` may run; `"addLayers"` covers
  `doc.commit` and the commands that change the document (see [`doc.command`](#doccommand)).
- `capabilities.network`: loopback HTTP access. `hosts` lists 1–16 `host` or `host:port` entries;
  only `127.0.0.0/8`, `[::1]` and `localhost` pass, with an optional port or `:*` for any port
  (`"127.0.0.1:*"`, `"localhost:8188"`). `userCanEdit` (default `false`) is the ABI's intent that
  the user may edit the list in Preferences.
- `capabilities.storage`: a per-plug-in key/value store (default `false`).
- `overlap` and `area` describe pixels and mean nothing to a panel. They still parse (a panel
  carries no pixel declaration in practice).

Unknown capability keys are ignored rather than fatal, so a manifest from a host that grew a
capability still installs. Everything is off unless declared.

Installing a module with the same id replaces the old one (the v1 rule); this also ends the running
panel instance, and the old module's storage survives (it is an upgrade of the same plug-in).

## Lifecycle and events

The host starts one instance when the panel is first opened (`plugin.panel.open`) or restarted
(`plugin.panel.restart`), sends it `init`, and keeps it for the life of the process. Every event is
a JSON object with a `"type"` and a `"now"` (milliseconds since the Unix epoch; `0` in the web
build, which has no clock in `std`). A plug-in may reply to any event with any operations; a reply
may carry several operations, and they are applied in order.

| Event | Shape | When |
|---|---|---|
| `init` | `{"type":"init","now":…,"host":{…},"locale":"en-US","storage":{…},"entropy":"<hex>","grants":{…}}` | Once, when the instance starts. `host` is `{"name":"PhotoCraft","version":…,"build":…,"platform":…,"arch":…}`; `storage` is everything this plug-in stored in earlier sessions; `entropy` is 32 bytes of hex from the clock and a counter, for seeding a PRNG; `grants` is `{"storage": <declared>, "document": […], "network": null, "commands": […]}` — what the host actually allows, where `commands` is the engine commands this panel may ask for with `doc.command` (empty when the manifest asks for no document capability). |
| `panel.visible` | `{"type":"panel.visible","now":…,"visible":bool}` | The panel was shown or hidden. A plug-in may pause polling while hidden. |
| `doc.changed` | `{"type":"doc.changed","now":…,"doc":<document or null>,"revision":n}` | The active document changed (id or revision), or was closed (`doc` is `null`), since the panel last heard. Sent from `plugin.panel.tick`. The document is carried in full only when the manifest asks for the `read` capability; without it `revision` still says a change happened, and `doc` is `null` — a capability is not granted by an event that merely arrives. |
| `ui.click` | `{"type":"ui.click","now":…,"id":"<widget>","values":{…}}` | A button was pressed. `values` is every input's current value: what `ui.set` last set or the UI committed, plus the values the view declares for inputs the plug-in has not set itself. |
| `ui.change` | `{"type":"ui.change","now":…,"id":"<widget>","value":…,"values":{…}}` | An input was committed: Enter, focus lost, slider released, choice picked. Not sent per keystroke. |
| `result` | `{"type":"result","now":…,"id":…,"ok":bool, …fields}` | An operation that carried an `id` finished, or a timer came due. On success the operation's fields are merged in; on failure the event carries `"error": "<message>"`. |
| `cancel` | `{"type":"cancel","now":…}` | The user pressed Cancel on the panel's progress bar. |

A due timer does not get an event type of its own: it arrives as a `result` event whose `id` is the
timer's id, `ok: true` and no other fields. (The id of a `timer.set` names the event it will later
fire, not a result, which is why timers are the one operation an `id` does not attach to.)
`ui.click`, `ui.change` and `cancel` come from the UI (`plugin.panel.event`); `init`,
`panel.visible`, `doc.changed` and `result` are synthesised by the host.

Events are queued per instance: a newer `doc.changed` or `panel.visible` replaces a pending one of
the same type (they describe a state), while `ui.*` events do not coalesce, and when the queue is
full the oldest `ui.*` event is dropped first. The host carries out whatever a plug-in answers with
until the queue is empty or a cap is hit; a panel that answers every event with an event of its own
is stopped ("the panel queued more than 64 events without settling").

## Operations

A reply is a JSON array of these. `op` is the operation name; `id` (where an operation has one) is
a string or a number, echoed back in the `result` event. Operations without an `id` are
fire-and-forget. The whole reply is validated before any of it is carried out; a bad operation
rejects the batch and stops the panel. `doc.command` is carried out; `doc.capture`, `doc.commit` and
`http.request` are **validated** exactly as written below — their shapes are part of the contract —
and then refused with the `result` shown, so a panel is told what happened rather than meeting
silence.

| Op | Shape | Semantics |
|---|---|---|
| `ui.view` | `{"op":"ui.view","view":<widget>}` | Draw this widget tree; it replaces the previous one. A bad view rejects the whole reply and stops the panel. No `result`. |
| `ui.set` | `{"op":"ui.set","values":{"<id>":<value>,…}}` | Change input values without replacing the view. Keys must be widget ids. Values win over the `value` a widget declared. No `result`. |
| `ui.progress` | `{"op":"ui.progress","fraction":0..1\|null,"text":"…","cancellable":bool,"done":bool}` | The panel's progress bar. `fraction: null` is indeterminate. `done: true` retires the bar. `text` ≤ 4000 characters. No `result`. |
| `ui.notice` | `{"op":"ui.notice","level":"info"\|"warning"\|"error","text":"…"}` | A host notice shown under the panel's view. `text` must be non-empty and ≤ 4000 characters. The host keeps the last 32. No `result`. |
| `timer.set` | `{"op":"timer.set","id":"<name>","ms":n}` | Fires a `result` event `n` ms from now. `ms` in `16..=600000`. Re-arming an armed id moves it and does not count as a new timer. |
| `timer.clear` | `{"op":"timer.clear","id":"<name>"}` | Disarms a timer. No `result`. |
| `storage.set` | `{"op":"storage.set","key":"<name>","value":<any JSON>}` | Writes to the plug-in's own store; a `null` value deletes the key. No `result`. |
| `doc.info` | `{"op":"doc.info","id":…}` | Answers `result` with the active document (or the one named by the command's `doc` parameter) merged into the event. With no document open, `ok: false` and `"error": "no document is open"`. |
| `doc.command` | `{"op":"doc.command","id":…,"command":"type.create","params":{…}}` | Runs an engine command and answers `result` with what it returned. Only the commands in `init.grants.commands` are run; the rest are refused with the reason. |
| `doc.capture` | `{"op":"doc.capture","id":…,"source":…,"rect":…,"alpha":…,"maxSide":…,"bits":…}` | Refused in this build: `result` with `ok: false`, ``"error": "this host does not capture pixels yet: `doc.capture` was not carried out"``. |
| `doc.commit` | `{"op":"doc.commit","id":…,"doc":n,"label":"…","layers":[…],"ifRevision":…}` | Refused in this build: `result` with `ok: false`, ``"error": "this host does not add layers yet: `doc.commit` was not carried out"``. |
| `http.request` | `{"op":"http.request","id":…,"method":…,"url":…,"headers":{…},"body":…,"response":…,"timeoutMs":…}` | Refused in this build: `result` with `ok: false`, ``"error": "this host makes no network requests: `http.request` was not carried out"``. |
| `blob.free` | `{"op":"blob.free","blob":"b:1"}` | Releases a blob. Idempotent: an unknown id is not an error. No `result`. |

### `doc.command`

The way a panel *acts*: it names an engine command and gets the command's own result back. A panel
can read the document without it (`doc.info` carries the layer stack, the selection bounds and the
history), but it cannot make a layer or move one by itself — that is what this is for.

```json
[{"op": "doc.command", "id": "made", "command": "type.create",
  "params": {"x": 4, "y": 8, "text": "Balloon", "size": 12}}]
```

- `command` (1–64 characters): a command id, `A–Z a–z 0–9 . _ -`, starting with a lowercase letter
  and not ending in `.`. `params` (optional, default `{}`): the command's own parameters, an object
  or `null`, at most 64 KiB — the params are what the command documents, and a panel gets no special
  shape.
- The answer is an ordinary `result` event: `ok: true` with the command's returned fields merged in
  (`{"id":"made","ok":true,"layer":3}`), or `ok: false` with `"error"` carrying the reason.
- The command runs as the user's, with the user's settings and history: **one undo step**, and
  `edit.undo` takes it back like any other edit. A panel cannot undo, redo or replay anything —
  those commands are not in the list.

Two gates decide whether a command runs, and both are the **host's** decision:

1. it is in the host's short, reviewed list (below), not a namespace of commands;
2. the manifest declares the document capability it needs.

A manifest is written by whoever ships the plug-in, so it can ask for anything; it only ever
*requests*. `init.grants.commands` is what the panel actually got, and a panel should offer what is
on that list and say so about the rest. The refusals are answers, not silence:

- **A command the host does not hand to panels** (`file.open`, `edit.undo`, `actions.play`, …):
  *"this host does not let a panel run …"*, and the panel is pointed at `init.grants.commands`, which
  lists what it may run.
- **A command its manifest did not ask the capability for** (`type.create` for a panel declaring only
  `"read"`): *"needs the manifest's `capabilities.document` to include `addLayers`"*.

What a panel may run, with the capability each needs: **`read`** — `type.info` (a layer's type: font,
size, text), `type.fonts` (the fonts to list in a style editor); **`addLayers`** — `type.create`,
`type.edit` (shape a text box, move a layer's type, set a style's anti-aliasing, step a size) and
`type.setStyle`. That is the whole list, and it is what the workflow this ABI was designed around —
manga and comic lettering, which is what the list was cut down to — actually emits, and nothing else:
`doc.info` already carries the layer stack, the selection bounds and the history, so reading needs no
command at all, and a row nothing calls is permission surface nobody has had to justify. Growing the
list is a deliberate act in `plugin_panel_cmds::permissions`, with a comment saying why — so a command
added to the engine tomorrow is not reachable by a panel until someone decides it is, and a panel that
wants `layer.align.*` or a colour picker has to have that decision made for it.

Nothing that reaches outside the document is on it, by construction: no `file.*` (a panel must not
read or write the user's disk), no `plugin.*` (no self-install), no `edit.preferences`, no
`actions.*` (a recorded action can run any command at all, which would walk around the list), no
`script.*`, and nothing from the shell's own `view.*`/`window.*`.

A panel that asks for a command from inside the answer to that command — or that asks for one on
every answer — never settles. After 64 round trips the host stops it with
`"the panel asked for more than 64 commands without settling"` and the state `stopped`, rather than
run engine commands for it forever. Commands are run **outside** the panel lock, one panel at a
time, so a granted command that touches a panel cannot wait for itself.

### `doc.capture`

- `source`: `"composite"` (merged visible pixels), `"layer"` (the active layer's own pixels),
  `"selection"`, or `{"layer": <id>}` for one layer by id (`0` is not a layer id).
- `rect`: `"canvas"`, `"selection"` (the selection's bounds), `{"selection": {"pad": px}}` (the
  selection grown by `pad`, for inpaint context, `0..=16384`), or an explicit
  `{"x":…,"y":…,"width":…,"height":…}` with finite values and a positive area (clamped to the
  canvas by the host).
- `alpha` (default `"source"`): `"source"`, `"none"` (fully opaque), `"selection"` (coverage) or
  `"selectionInverted"`.
- `maxSide` (optional): `1..=16384`.
- `bits` (optional): `8` or `16`.

### `doc.commit`

One undo step, adding 1–32 layers:

```json
[{"op": "doc.commit", "id": "c1", "doc": 3, "label": "Lettering",
  "layers": [{"blob": "b:21", "name": "Bubble_1", "x": 812, "y": 440,
              "scaleTo": {"width": 512, "height": 512},
              "mask": {"blob": "b:19"}, "above": "active"}]}]
```

- `doc`: the document id the panel started from; `0` is rejected.
- `label`: the undo-step label, 1–128 characters.
- Each layer: `blob` (a PNG or JPEG the host already holds), optional `name` (≤ 64 characters),
  `x`/`y` (finite), optional `scaleTo` (`{"width":…,"height":…}`, each side `1..=32768`), optional
  `mask` (`"selection"` or `{"blob": "<grayscale PNG>"}`), and `above` (`"active"`, the default, or
  `"top"`).
- `ifRevision` (optional): the revision the panel believes the document is at.

### `http.request`

- `method`: `GET`, `POST`, `PUT` or `DELETE`.
- `url`: `http://` and a loopback authority only. `https:` and non-loopback hosts are rejected. The
  granted-host check (against `capabilities.network.hosts`) is the host's.
- `headers` (optional): only `Content-Type`, `Accept` and `Authorization`, and only with string
  values.
- `body` (optional): `{"json": …}`, `{"text": "…"}`, `{"blob": "b:1"}`, or
  `{"multipart": [{"name":"…","filename":"…","contentType":"…","blob":"b:1"}, {"name":"…","text":"…"}]}`
  with 1–64 parts, 1–256-byte names, and exactly one of `blob`/`text` per part. The host builds the
  boundary, filenames and content types.
- `response`: `"none"`, `"json"`, `"text"` or `"blob"`.
- `timeoutMs` (optional): `1..=600000`.

## Views

A `ui.view` describes widgets; the host draws them with egui and `theme::Tokens`. A plug-in cannot
choose a colour, font or size, so a panel is on-theme in light and dark in every theme for free.
The tree is checked whole: at most 512 widgets, at most 16 levels deep, at most 256 options in a
`choice`. Labels, section titles and placeholders are at most 256 characters; a `text` value is at
most 65 536 characters, or the widget's own `maxLength` when that is smaller.

| `type` | Fields | Draws as | Events |
|---|---|---|---|
| `column` | `children` | children stacked vertically | — |
| `row` | `children` | children in a wrapping line | — |
| `section` | `title`, `collapsed` (default `false`), `children` | a collapsible group; open/closed is remembered by the host per panel | — |
| `label` | `text`, `level` (optional) | plain text, no markup; wraps. `level` is `muted`, `info`, `warning` or `error` | — |
| `text` | `id`, `value`, `placeholder`, `multiline` (default `false`), `maxLength` | a prompt field; with `multiline`, a taller scrolling script box | `ui.change` with the string |
| `number` | `id`, `value`, `min`, `max`, `step`, `slider` (default `false`) | a field, or a slider when `slider` is set; `step` snaps to its grid | `ui.change` with the number |
| `toggle` | `id`, `value`, `label` | a checkbox | `ui.change` with the bool |
| `choice` | `id`, `value`, `options: [{"value","label"}]` | a dropdown; `value` must be one of the options | `ui.change` with the string |
| `button` | `id`, `label`, `primary` (default `false`), `enabled` (default `true`) | a button; a disabled one shows a hover hint | `ui.click` |
| `progress` | `fraction` (number or `null`) | a bar inside the panel; `null` is indeterminate | — |
| `image` | `blob`, `maxHeight` (optional) | see **Document images** | — |
| `separator` | — | a hairline | — |

```json
{"type": "column", "children": [
  {"type": "choice", "id": "tool", "value": "txt2img",
   "options": [{"value": "txt2img", "label": "Text to Image"}]},
  {"type": "text", "id": "prompt", "value": "", "multiline": true, "placeholder": "Describe the image"},
  {"type": "section", "title": "Settings", "collapsed": true, "children": [
    {"type": "number", "id": "steps", "value": 4, "min": 1, "max": 50, "step": 1, "slider": true}]},
  {"type": "button", "id": "generate", "label": "Generate", "primary": true},
  {"type": "label", "text": "connected", "level": "muted"}]}
```

- **Ids.** `text`, `number`, `toggle`, `choice` and `button` have an `id`: 1–64 characters from
  `A-Z a-z 0-9 . _ -`, unique within the view. `column`, `row`, `section`, `label`, `progress`,
  `image` and `separator` have none.
- **Ranges.** A `number` needs finite `value`, `min` and `max` with `min <= max`, and a positive
  finite `step` if given. A `choice` needs `value` to be one of its options; each option needs a
  non-empty value of at most 256 bytes and a label of at most 256 characters. A `progress` fraction
  is `0..=1` or `null`.
- **Text editing.** The host buffers what is typed. A `text` is committed as `ui.change` when the
  widget **loses focus** and the buffer differs from the value the host holds; egui ends a
  single-line edit on Enter, so Enter commits a single-line field, while Enter in a `multiline`
  field is a newline and the commit is leaving the field. Keystrokes never cross the sandbox. If
  the plug-in's own value changes (through `ui.set` or a new view), the buffer is dropped and the
  plug-in's value wins.
- **Layout.** The panel is a resizable floating window: `panel.minWidth` is both its default and
  its minimum width, its body scrolls vertically (up to 600 px tall), and fields take the available
  width. `panel.title` is the window title.
- **Not drawn.** An `image` is drawn as a placeholder row (`[<blob>] Image`) with a hover hint,
  because this host hands out no blobs (see below).

## Document images

A plug-in never touches image bytes: encoding a 24 MP PNG inside the 10–100×-slower interpreter
would take minutes. The host holds the bytes and gives the plug-in an opaque id, `"b:<n>"`, which a
plug-in can only name if the host gave it that id. The intended flow is `doc.capture` → an `image`
widget or a `doc.commit`, then `blob.free`; ids live as long as the instance except where freed, and
a module is capped at 256 MiB per blob and 1 GiB of blobs in total.

**Status.** In this build the pixel path is not implemented: `doc.capture` and `doc.commit` are
validated and then refused with an `ok: false` `result`, `http.request` is refused the same way, and
no blob is ever handed out. An `image` widget therefore draws its placeholder, and `blob.free`
remains part of the ABI (and is idempotent) but has nothing to release. The sandbox, limits and
failure handling these operations will run under are already in place.

## Storage

Storage belongs to the **plug-in**, not to a run of it: a panel that lost its settings when it was
restarted, or hidden and shown again, would have nowhere to keep anything. `storage.set` writes a
value (any JSON value) under a key; a `null` value **deletes** the key, which is the store's only way
to shrink — without it a panel that reached its key limit could never make room. A plug-in may keep
at most 256 keys and 1 MiB of values (each counted as JSON), and the store is read back in `init`, so
a panel opens where it left off rather than empty and a restart after a failure gets the same store.

Today the store lives **in memory only**, in a process-wide map keyed by plug-in id: it survives
closing, hiding and restarting the panel, and it is forgotten when the plug-in is removed
(`plugin.remove`) — replacing a plug-in with an upgrade of itself keeps it. It is not written to
disk. (The manifest's `capabilities.storage` is what the host reports in `grants`; there is no file
beside the preferences yet.)

## Sandbox and limits

The host is [`wasmi`](https://github.com/wasmi-labs/wasmi), the same pure-Rust interpreter v1 uses,
so panels run in the web build too. A panel is **one persistent instance**, not a fresh one per
call: its memory, its state, its blobs and its timers live for as long as the process keeps it.
Limits (`photocraft_plugins::Limits`, defaults):

| Limit | Default | On violation |
|---|---|---|
| Host imports | none: no WASI, file system, network, clock or randomness | module rejected at install |
| Module size | 32 MiB | rejected |
| Linear memory | 512 MiB per instance | `memory.grow` returns -1; oversized initial memory fails |
| Instructions (fuel) | 50 M per call, on `pc_event`, `pc_alloc` and `pc_free` alike | `Limit("instruction budget")` |
| Wall clock per host call | 2 s (`panel_wall_time_ms`), shared by every event that call pumps rather than granted afresh to each (native builds only — the web build has no monotonic clock in `std` and is bounded by fuel alone) | `Limit("time budget")`; the panel is stopped |
| Call depth | 1,024 frames | trap |
| Event and reply size | 1 MiB each, and at most 64 operations per reply | `Limit` |
| Input values (`ui.set` and what views declare) | 512 keys, 4 MiB as JSON | `Limit`; the panel is stopped |
| Timers | 32 pending, `16..=600000` ms | `Limit` |
| Notices kept | 32, each ≤ 4000 characters | older ones dropped |
| Events queued | 1 024 | the oldest `ui.*` event is dropped first |
| Pump steps per command | 64 | the panel is stopped ("without settling") |
| Blobs | 256 MiB each, 1 GiB total | `Limit` (no path produces one yet) |

Notices, timers, blobs and the queue are **per-instance** and are dropped when the instance stops.
Two counters are deliberately **per-plug-in** and outlive the instance: the storage map (above) and
the failure window — **three failures inside 60 seconds** trip it, after which `plugin.panel.open`
refuses with "failed repeatedly; start it again with `plugin.panel.restart`", and only an explicit
`plugin.panel.restart` (which clears the window) starts it again.

**Failure behaviour.** Any failure — a trap, an exhausted budget, an unparsable or invalid reply, a
bad pointer, an operation that passed validation but cannot be carried out (a store or value map over
its cap) — stops the instance and counts towards the failure window. A reply is validated whole
before any of it is applied; an operation that fails while being carried out stops the panel partway
through that reply instead, so what came before it stands and the panel is stopped rather than left
running on the rest of a batch it cannot see. Its queue, timers and blobs are dropped and it stays
stopped, reporting the same reason on every later event until restarted. The user sees the panel's
window replaced by "This plug-in stopped.", the reason, and a **Restart Plug-in** button.

A failure is not an undo, and the document is not off limits: **a panel does change the document.**
`doc.command` runs the allow-listed `type.*` commands for real (see [`doc.command`](#doccommand)) —
the same commands the Type tool runs, each an ordinary edit with its own history entry and its own
undo step — so the edits a panel made before it failed stay on the document, as undoable as any
other, and only the operations the host had not carried out yet are dropped. What a panel cannot do
is reach *past* the document: `doc.capture`, `doc.commit` and `http.request` are refused in this build
(see below), so it can neither see pixels nor add them, nor reach the network.

Ordering is what bounds a failing reply, and why the host stops a panel rather than carry on. The
commands a reply asks for are queued as that reply is applied and run only once all of it has been
applied, so a reply that fails partway through ends its own batch: the operations before the failure
stand, what follows it is dropped, and **the commands it had queued are dropped with the panel**. A
stopped panel runs nothing, however its commands reached it — `plugin.panel.close` on a stopped panel's
window closes the window and changes nothing.

**Panels are process-wide.** There is one instance per plug-in id for the whole process, shared by
every session (the installed plug-ins and the failure windows are process-wide too, as in v1). The
host drives an instance under one process-wide lock on the calling thread, so a hostile panel is
bounded by one 2 s wall-clock budget per host call — shared by every event that call pumps, not
granted afresh to each — and is then stopped for good; it can delay other panels by at most that.
`plugin.panel.tick` announces the active document of whichever session called it, and the UI calls it
once a frame, and asks for a frame when the nearest timer of a visible panel is due.

## Host commands

The `plugin.panel.*` commands drive a panel. Every one of them builds an event, pumps the queue,
carries out the operations that come back, and repeats until the panel settles, so a click in the
UI and the same command from a test or an agent are one code path. A `doc.command` a panel asks for
is run on the way, between pumping its reply and handing it the answer; none of these commands
returns until the panel has settled or been stopped.

| Command | Params | Returns |
|---|---|---|
| `plugin.panel.open` | `{"id":text,"doc":index?}` | The panel's inspection JSON (below). Starts the instance with `init` if it is not open; if it is, only sends `panel.visible` true. Refuses if the plug-in is a filter, or was stopped. |
| `plugin.panel.close` | `{"id":text,"stop":bool=false}` | `{"id","visible":false,"open":bool}`; with `stop` true, `open` is `false` and a `stopped` field says whether it had an instance. Default: hide it and send `panel.visible` false, keeping the instance. With `stop` true: end the instance and drop everything it held. |
| `plugin.panel.inspect` | `{"id":text}` | The panel's inspection JSON, or `{"id","state":"closed","open":false}` if it has no instance. |
| `plugin.panel.event` | `{"id":text,"event":{"type":"ui.click"\|"ui.change"\|"cancel","id":text?,"value":json?}}` | The panel's inspection JSON after it settles. The host supplies `now` and the panel's own values; a caller names a widget and a value and nothing else. An event the ABI does not have is rejected. |
| `plugin.panel.tick` | `{"id":text?}` | `{"id","timers":n,"revision":n,"panels":n,"errors":[…]}`, where `timers` is how many fired. Fires the timers that came due and tells each panel the document changed; one panel's failure is reported in `errors` and does not stop the others. |
| `plugin.panel.restart` | `{"id":text}` | The panel's inspection JSON. Starts a fresh instance at `init` and clears the failure window. |

The inspection JSON is what a UI or an agent needs about one panel: `id`, `name`, `abi`, `state`
(`"running"` or `"stopped"`), `error`, `visible`, `view`, `values`, `progress`, `notices`, `storage`,
`capabilities` (what the manifest asked for), `grants` (what the host allows, `commands` included),
`timers`, `blobs`, `blobBytes`, `queued` and `events` (the event count pumped since it started).

A panel is installed, listed, removed and reloaded by the v1 commands, which now report v2 fields
too:

| Command | Params | Notes |
|---|---|---|
| `plugin.install` | `{"path": "/x/y.wasm"}` (native) or `{"data": "<base64>"}`, `"replace": bool = true` | Validates and installs; returns its description. Replacing an id ends its running panel instance. |
| `plugin.list` | `{}` | `{"plugins": [ … ], "folder": report}`. Each plug-in carries `kind` and `abi` (2 for a panel), plus `panel` and `capabilities`; a panel also carries `open` (whether it has an instance), so a UI can put each in the right menu. |
| `plugin.remove` | `{"id": …}` | Uninstalls; ends the instance and forgets the plug-in's storage. |
| `plugin.reload` | `{"path": "folder"}` (default: the preference folder) | Loads every `*.wasm` in a folder; a bad module is reported and skipped. |

## Using panels in the app

Installed panels appear under **Window › Plug-ins**, one row per panel, labelled with the plug-in's
`panel.title` (falling back to `name`). Filters are unchanged and stay under **Filter › Plug-ins**.

- The row's **checkmark means the panel is on screen** (`visible`), not merely that it has an
  instance. Picking the row shows a hidden panel or hides a visible one.
- **Closing the window** (its Close box, or picking the row again) hides the panel and tells it so
  with `panel.visible` false; its instance keeps running, with its state, timers and blobs intact.
  The same row brings it back as it was left.
- **Restarting a stopped panel** is the **Restart Plug-in** button that replaces the body of its
  window, which runs `plugin.panel.restart`.
- The UI ticks every open panel once a frame (`plugin.panel.tick`, run in the background so it does
  not clear the status bar), which fires due timers and announces document changes.
- Errors surface in the **status bar**: opening a panel that traps at `init`, sending an event to a
  stopped panel, and the like.

## Writing a panel in Rust

[`examples/plugins/panel-rs`](../examples/plugins/panel-rs) is a complete `no_std` panel: no
allocator, a fixed reply buffer, and a hand-rolled scanner for the few fields of an event it reads.
Build it with:

```sh
rustup target add wasm32-unknown-unknown
examples/plugins/panel-rs/build.sh      # cargo build --release --target wasm32-unknown-unknown
```

The script also refreshes `crates/plugins/tests/fixtures/panel.wasm`, the artifact the host's tests
run — so the real plug-in a person would install is the ABI's conformance test. Install the built
module with `plugin.install {"path": "…/photocraft_plugin_panel.wasm"}` (native) or
`{"data": "<base64>"}`, then open it under Window › Plug-ins.

The example is small but not a toy: it reads the document on demand (`doc.info`), follows it as it
changes instead of polling (`doc.changed`), remembers a note between sessions (`storage.set`), and
runs a timer while its Watch button is on — re-arming the one-shot timer from the `result` that
fired it. Nothing in the ABI prefers its approach: a real plug-in in a language with an allocator
and a JSON parser would use those.

The other end of that range is the workflow this ABI was designed for: TyperTools/TypeR 1.4.9, the
manga and comic lettering panel for Photoshop, as a native panel — a Rust port of some 6,500 lines,
carrying the original's MIT licence (© 2017 Swirt), with its own script parser, its own style library
and that library's JSON format, lettering a page through those five commands and nothing else. It is
not part of this repository: it is kept in its own repository, with attribution to the original
authors, and installs here as one `.wasm` module like any other panel. What it did shape is everything
above — the allow-list was cut down to what it emits, the widget set to what it draws, and the ABI's
rough edges are the ones it hit: a panel follows the document rather than being told about its own
edits, and an action it cannot take is refused on screen rather than attempted.

## Not in v2 (yet)

- **The pixel and network paths.** `doc.capture`, `doc.commit` and `http.request` are validated and
  then refused with an `ok: false` result; no blob is ever handed out, so the `image` widget draws a
  placeholder and there is no document-image path in flight at all.
- **Persistent storage.** The store is in memory only; it is not written to a file, and it is lost
  when the process ends.
- **`doc.capture`, `doc.commit` and the network.** The document capability is real for reading and
  for the commands in the allow-list, and `capabilities.network` is reported back in `plugin.list`,
  but `grants.network` is always `null`: no blob is ever handed out, so a panel cannot read or add
  pixels, and `http.request` refuses to run.
- **Panels acting through anything but `doc.command`.** A panel reaches the application through the
  short, reviewed allow-list of engine commands (see [`doc.command`](#doccommand)); a document
  capability on its own grants nothing beyond `doc.info`.
- **Per-instance worker threads.** Instances are driven on the calling thread under one process-wide
  lock.
- **`ui.set` deleting an input.** Assigning a widget id `null` overwrites the value as `null`; there
  is no "reset to the declared value" operation.
- **Panel-provided menu items and file formats.** A panel draws into its own window only.
