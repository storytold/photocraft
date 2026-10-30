# PhotoCraft engine

The Rust API and command registry are documented in the crate’s rustdoc.

### Private Sta.sh submissions

`Session::execute("file.submitDeviantArt", params)` performs a private upload through
`Session::deviantart_submit`, an authenticated host transport (`deviantart_cmds::SubmitFn`).
The desktop host keeps OAuth credentials in its existing protected storage and installs
this transport when connected; credentials never appear in command parameters or journals.
Hosts without a transport, and WebAssembly, return an explicit unavailable error.

Parameters: `format` (`saved`, `png`, or `psd`, default `saved`), optional `path` for
`saved` (otherwise the active document's saved path), optional `title` (200 characters)
and `tags` (comma/whitespace separated, 4096 bytes). PNG/PSD export the active document
in memory using `photocraft-io`; uploads are capped at 30 MiB. The result contains `name`, `itemId`,
`body` (the Sta.sh response), and export `warnings`. No public publishing or automatic
submission is implied. Automatic save/export hooks remain opt-in host settings.

The egui shell opens the settings dialog for an empty menu invocation and uses this
same engine command on its background worker when confirmed. Passing parameters through
`ui.menu.invoke` dispatches the engine operation directly. Headless hosts may install
an authenticated transport through the same public API; no transport means an error.
Existing automation authorization restrictions still apply; this command does not grant
ambient file access or bypass the host's command policy.
