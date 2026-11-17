# Document Info: an example PhotoCraft panel plug-in

A complete **panel** plug-in (plug-in ABI v2) in about 530 lines of `no_std` Rust, with no allocator
and no imports: it draws a small widget tree the host renders in the application's own theme, reads
the active document when asked, adds a type layer to it by asking the host to run a command, keeps a
note between sessions, and watches the document for changes with a timer instead of polling. The ABI is documented in
[`docs/plugins-v2.md`](../../../docs/plugins-v2.md).

It is deliberately a *conformance* example rather than a useful one: between them, its five controls
exercise the whole surface a panel needs — `ui.view` and `ui.set` for the widgets, `ui.notice` for a
message, `doc.info` for a question the host answers later, `doc.command` for asking the host to run an
engine command (`type.create`, gated by the `addLayers` capability this manifest declares),
`timer.set`/`timer.clear` for time, `storage.set` for state that outlives the instance, and the events
`init`, `ui.click`, `ui.change`, `doc.changed`, `result` and `tick`. The host's tests install this module and drive it end to end
(`crates/plugins/tests/abi_v2.rs`, `crates/ui-egui/src/plugin_panel_ui_tests.rs`).

## Build

```sh
rustup target add wasm32-unknown-unknown
./build.sh
```

`build.sh` runs `cargo build --release --target wasm32-unknown-unknown` and copies the module to
`crates/plugins/tests/fixtures/panel.wasm`, the test fixture the host's tests use. This crate is not a
member of the PhotoCraft workspace.

## Install

```sh
# Into a running app (control channel) or the CLI, as a command:
plugin.install {"path": "examples/plugins/panel-rs/target/wasm32-unknown-unknown/release/photocraft_plugin_panel.wasm"}
plugin.panel.open {"id": "org.photocraft.example.panel"}
```

Or copy the `.wasm` into the folder set in Edit › Preferences › Plug-ins (with *Use Additional
Plug-ins Folder* on); it then appears under Window › Plug-ins, and picking that row opens its window.

## What it answers

| Control | Operations it sends | What it shows |
|---|---|---|
| Read Document | `doc.info` | the active document's size when the `result` comes back, or a `ui.notice` warning when the host refuses |
| New Type Layer | `doc.command` | a `type.create` the host runs, one undo step like any edit; the status line shows the new layer's id, or the reason the host refused |
| Watch Document | `timer.set` / `timer.clear` | the label toggles, and one `tick` a second redraws the view |
| Save Note, or Enter in the field | `storage.set` | the note, kept across close, restart and reopen; the status line says so. Emptying the field sends a `null` value, which deletes the key |
| every event | `ui.view` | the whole tree, re-sent |

The reply to each event is a JSON **array** of operations, written into the buffer `pc_alloc` handed
out and returned as `len << 32 | ptr`; `0` means "nothing to say", which is what a quiet tick answers.
