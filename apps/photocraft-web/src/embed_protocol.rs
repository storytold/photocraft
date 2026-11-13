//! What the embedding API (#1614) agrees with its host page, kept free of browser types so it is
//! tested natively: the `?embed=` origin, the `postMessage` calls and replies, the version info,
//! the bridge's limits, and the open-claim policy.

use serde_json::{Value, json};

/// The app and `.pcraft` format versions with no document open, for a host that keeps the CLI and
/// the web build in step (a newer `.pcraft` fails to open with `FormatError::TooNew`).
///
/// Only its test calls it — the app serves itself through [`info_with`] — so it is test-gated;
/// without the gate the wasm build (which compiles this module outside tests) warns it dead.
#[cfg(test)]
pub fn info() -> Value {
    info_with(None)
}

/// [`info`] with the document the session is showing: a host that opened a file can check that
/// the document it was told about (`open`'s `doc.id`) is the one the app has active now.
///
/// The versions come from the constants every frontend reports — `build_info::VERSION` in the
/// engine crate (the workspace version `cargo xtask version` manages, which a release build may
/// override) and the format crate's `FORMAT_VERSION` — the same pair the About dialog and
/// `--version` give.
pub fn info_with(doc: Option<&DocInfo>) -> Value {
    let doc = match doc {
        None => Value::Null,
        Some(d) => json!({ "id": d.id, "name": d.name }),
    };
    json!({ "app": photocraft_engine::build_info::VERSION, "format": photocraft_format::FORMAT_VERSION, "doc": doc })
}

/// The active document as `info()` reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct DocInfo {
    /// The document's session id (`DocId`), stable for the document's lifetime.
    pub id: u64,
    /// Its name, as the app shows it in the tab.
    pub name: String,
}

// ---------- the bridge's limits ----------

/// The largest file `open` takes, on either face: 64 MiB — 2.6× the largest production asset
/// this bridge serves (24.5 MB), so nothing real is refused, while a page can't make the tab
/// allocate without bound. Checked on the incoming size, before any copy is made.
pub const MAX_OPEN_BYTES: usize = 64 * 1024 * 1024;

/// The fixed refusal for a larger `open` (the text is part of the host's contract) — distinct
/// from [`OPEN_READ`], so a host can tell "too big" from "unreadable". Both faces (`window`
/// and `postMessage`) come through the one open path, and a byte shape is gated on its true
/// length before any view of it is made.
pub const OPEN_TOO_BIG: &str = "open: the file is larger than 64 MiB (67108864 bytes)";

/// The fixed rejection when an open's bytes can't be read at all: a value that isn't an
/// ArrayBuffer, a typed array or a Blob, or anything its conversion throws (a Proxy of a Blob,
/// a typed array whose elements can't convert). One text for both — the host can't act on the
/// difference, and neither can the app.
pub const OPEN_READ: &str = "open couldn't read the data as bytes: it takes an ArrayBuffer, a typed array or a Blob";

/// The most a bridge message may weigh outside its `data` member: 64 KiB — a call is an id, a
/// method and a name — weighed on the message's structured clone *before* anything decodes it,
/// so the JSON face (stringify + parse, and the memory that doubles) never starts on a message
/// over the budget. `data` is exempt: it is the one member allowed to be large, and answers to
/// [`MAX_OPEN_BYTES`] on its own.
pub const MAX_CALL_BYTES: usize = 64 * 1024;

/// How deep a bridge message may nest containers before it is refused with [`CALL_TOO_BIG`],
/// whatever it weighs: a call is one object deep, so deeper nesting is filler — including the
/// nested kind that weighs nothing a byte at a time.
pub const MAX_CALL_DEPTH: usize = 8;

/// The fixed refusal for a message over either (the text is part of the host's contract).
pub const CALL_TOO_BIG: &str =
    "the message is larger than 64 KiB (65536 bytes) or deeper than 8 levels without its data; send the file in data and only the call in the message";

/// The fixed refusal when a bridge message carries binary data — a typed array, a `DataView`,
/// an `ArrayBuffer` — anywhere outside its `data` member. Binary rides only `data` (where
/// [`MAX_OPEN_BYTES`] answers it): the JSON face would spell a view's every byte out — a
/// 16 KiB `Uint8Array` that any byte-based estimate could be lied to about (a forged
/// `byteLength` property sizes the estimate, the spelled-out elements are what stringify
/// pays) — so nothing binary on the metadata plane is estimated at all; it is refused whole.
/// The message around `data` is the call's shape: an id, a method, a name, a format.
pub const CALL_BINARY: &str = "the message carries binary data (a typed array, a DataView or an ArrayBuffer) outside its data member; send the file in data and only the call in the message";

/// What a string member of a bridge message costs the JSON face, as an upper bound:
/// `JSON.stringify` spells it out with the two quotes, and a unit's worst spelling is six
/// characters — a control character escapes to `\u0000`, and three bytes of the UTF-8 `String`
/// Rust gets is the *cheapest* a non-ASCII unit becomes (a lone surrogate turns into U+FFFD,
/// three itself). Counting a string's UTF-16 units (its `length`) passes 64 KiB of CJK at
/// 192 KiB; the bound doesn't. `None` when the bound overflows `usize`.
pub fn call_string_cost(units: usize) -> Option<usize> {
    units.checked_mul(6).and_then(|n| n.checked_add(2))
}

/// What an array member costs before a single element is visited: stringify spells every index
/// from 0 to `length` — a sparse array's holes as `null,` — so `length` × 5 bytes plus the
/// brackets is the floor. `new Array(1_000_000)` owns one key (`length`, about all an own-keys
/// walk ever sees of it) yet serializes to megabytes; the floor refuses it on the length alone,
/// and the elements are walked only when the floor fits. `None` when it overflows `usize`.
pub fn call_array_floor(len: usize) -> Option<usize> {
    len.checked_mul(5).and_then(|n| n.checked_add(2))
}

/// How many `postMessage` calls may wait for their reply at once; the next is answered
/// [`BUSY`] instead of queued, so a page can't pile work onto the app.
pub const MAX_PENDING_CALLS: usize = 8;

/// The fixed refusal when [`MAX_PENDING_CALLS`] calls already wait for their reply.
pub const BUSY: &str = "busy: 8 calls are already waiting for their replies; send another once they answer";

/// The fixed refusal when an `open` arrives while another is still in flight — accepted, not
/// yet settled: its import may be running, or waiting for its document (behind a dialog in the
/// app, up to [`OPEN_TIMEOUT_MS`]). One open at a time is what keeps two opens from ever
/// answering with each other's document; it is the same busy semantic [`MAX_PENDING_CALLS`]
/// gives calls, with its own text (that one counts calls waiting for replies, this one an open
/// waiting for its document).
pub const OPEN_BUSY: &str = "busy: another open is still waiting for its document; open again once it answers";

/// How long an open waits for its document to enter the session before its promise rejects.
/// One open is in flight at a time, so this is also the longest a dead one — its dialog
/// cancelled, its document never coming — locks every later open behind [`OPEN_BUSY`]; the
/// 120 s this used to be did that for two full minutes. 30 s still spans a person answering a
/// dialog over the import (the reason the wait exists), and a dialog cancelled in the app ends
/// the wait at once (the embed pass reads the app's open prompts), so the deadline is only the
/// fallback for what cannot be read.
pub const OPEN_TIMEOUT_MS: f64 = 30_000.0;

/// The fixed rejection when that wait gives up — at its deadline, or at once when the app was
/// seen holding the file behind an open prompt and now holds nothing, with no document of the
/// asked-for name arrived (the dialog was cancelled; the file was disposed of).
pub const OPEN_TIMEOUT: &str = "open gave up waiting for the document: it is held behind a dialog in the app, or never finished opening";

/// `Ok(())` while `len` — the incoming size, when it is already known — fits
/// [`MAX_OPEN_BYTES`].
pub fn check_open_bytes(len: Option<usize>) -> Result<(), String> {
    match len {
        Some(n) if n > MAX_OPEN_BYTES => Err(OPEN_TOO_BIG.into()),
        _ => Ok(()),
    }
}

/// What the bytes-of-a-view read refuses with, *before* any copy is made (the embed side's
/// `bytes_of_view` feeds it what it read off the JS answer): the over-budget sentinel `false`
/// the view picker resolves with — mapped to [`OPEN_TOO_BIG`], verbatim, so a too-big open
/// spells the same refusal whichever side noticed; a source that turned out not to be a view
/// ([`OPEN_READ`]); or a view whose own length is over [`MAX_OPEN_BYTES`] ([`OPEN_TOO_BIG`]
/// again, gated before the copy it would pay for). `None`: read on.
pub fn view_refusal(too_big: bool, is_view: bool, view_len: usize) -> Option<String> {
    if too_big {
        return Some(OPEN_TOO_BIG.into());
    }
    if !is_view {
        return Some(OPEN_READ.into());
    }
    check_open_bytes(Some(view_len)).err()
}

/// The `offset..offset+length` range out of `whole`: the whole copy when the range spans it,
/// the slice when it sits inside, and empty for anything past the end — a hostile range can
/// only fail to read, never panic or over-allocate (the addition saturates, and an `f64`
/// `NaN` or negative has already cast to 0, an infinite to `usize::MAX`).
pub fn view_range(whole: Vec<u8>, offset: usize, length: usize) -> Vec<u8> {
    let end = offset.saturating_add(length);
    if offset == 0 && end >= whole.len() {
        return whole;
    }
    whole.get(offset..end).map(<[u8]>::to_vec).unwrap_or_default()
}

/// Whether opening `name` adds a document to the session: preset files (brushes, gradients,
/// styles, swatches, preferences) go to their libraries instead, so those opens settle at once.
/// The list mirrors `preset_files_ui::PRESET_EXTS` in `ui-egui`.
pub fn expects_document(name: &str) -> bool {
    const PRESET_EXTS: &[&str] = &["abr", "grd", "asl", "pat", "aco", "act", "ase", "kys", "psp"];
    !PRESET_EXTS.contains(&extension(name).as_str())
}

/// The document an open settles on: the first document new since the import started
/// (`before`). A named open (`asked`) takes only a document of that very name, *exactly* —
/// there is no fallback to any other new document, so a document of a different name is never
/// a named open's (two opens cannot answer with each other's, whatever timing lands them in
/// one pass), and one whose document never comes — its dialog cancelled, or a `.psdt` the app
/// opens as "Untitled-N" — waits out its deadline and rejects. An open that asked for no name
/// (`asked` `None`) takes its import's own admitted document under whatever name the session
/// gave it: a `.pcraft` keeps the name its manifest persisted (#3312), so the "document" a
/// name-less import runs under is a file-name hint, never a name the open could claim — the
/// claim is the new `DocId`, the open answers with the document's own name, and the imported
/// document is never renamed to the hint. `None` while nothing of the sort has appeared:
/// the import is still running, or is held behind a dialog in the app.
pub fn landed_doc(before: &[u64], asked: Option<&str>, docs: &[(u64, String)]) -> Option<(u64, String)> {
    docs.iter().find(|(id, n)| !before.contains(id) && asked.is_none_or(|a| n == a)).map(|(id, n)| (*id, n.clone()))
}

// ---------- the bridge's window policy ----------
//
// The embed pass reads the window tree and the `MessageEvent` fields on the JS side; what it
// does with them is decided here, on plain data, so it is testable natively.

/// Where a bridge reply goes, as the embed pass reads the window tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReplyTarget {
    /// No parent window at all.
    Detached,
    /// The app's own window: `window.parent()` returns the window itself at top level.
    OwnWindow,
    /// A parent window other than the app's own — the documented iframe.
    Parent,
}

/// Whether a reply may be posted to `target`: only a distinct parent. A post to the app's
/// own window is refused *before* anything is posted: the browser detaches a transferred
/// buffer while it serializes the message and only then drops the message on the
/// `targetOrigin` check (`postMessage` itself returns normally), so a self-post would read
/// as delivered — posted, buffer gone — while its bytes reached no page, and a save settled
/// on it would clear the unsaved state of a document nobody has, hiding the unsaved-work
/// prompt.
pub fn posts_to(target: ReplyTarget) -> bool {
    matches!(target, ReplyTarget::Parent)
}

/// Whether a bridge save counts as delivered — the moment its document may be marked clean:
/// the reply was posted (to a distinct parent, [`posts_to`], and the browser took the call)
/// and the transfer finished — `buffer_len == 0`, the browser owns the buffer now, so the
/// view on the app's side reads detached. A post refused, a self-post, or a buffer still in
/// the app's hands keeps the document dirty.
pub fn delivered(posted: bool, buffer_len: u32) -> bool {
    posted && buffer_len == 0
}

/// Who a bridge message came from, as the listener reads the event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MessageSource {
    /// The app's framing parent — a distinct window (at top level there is none).
    Parent,
    /// Some other window: a sibling iframe the embedder hosts at the `?embed=` origin posts
    /// with a matching origin, and so does any other window that origin holds.
    Other,
    /// No source readable — or no parent the app could be framed by at all.
    None,
}

/// Whether the bridge takes a posted message: its origin must be the `?embed=` origin the
/// bridge was installed with, **and** its source must be the framing parent — a distinct
/// window. The origin alone can't say who sent the message, and a request from any window
/// but the parent could never be answered (replies go only to the parent) yet would still
/// run — its `open` or `save` executed by the app on a stranger's behalf. At top level
/// (`MessageSource::None`) the app has no embedder, and nothing is taken.
pub fn takes_call(origin_is_embed: bool, source: MessageSource) -> bool {
    origin_is_embed && source == MessageSource::Parent
}

// ---------- the open settle policy (a pure state machine) ----------

/// What one pass over an open found — everything the settle decision may depend on, as plain
/// data. The embed pass itself is wasm-only; it reads the app and fills this in, and
/// [`open_step`] decides on it alone, so the whole wait policy — the timeout's
/// cancel-and-observe path included — is testable natively.
#[derive(Debug, Default, PartialEq)]
pub struct OpenPass {
    /// The import's failure, when it failed (File › Open's error).
    pub failed: Option<String>,
    /// The bytes are still being read: nothing of the open has run, and no document can be its
    /// answer yet.
    pub reading: bool,
    /// The asked-for document, when it has landed: new since the import started, of the exact
    /// name ([`landed_doc`]).
    pub landed: Option<(u64, String)>,
    /// The app still holds anything of the file: an open prompt on screen, a file queued
    /// behind one, or an opening job of its name. The observation's release condition, and
    /// half of the cancelled-in-the-app read below.
    pub app_holds: bool,
    /// The pass has seen the app hold the file before (the latch, carried pass to pass).
    pub held: bool,
    /// The wait's deadline has passed.
    pub timed_out: bool,
    /// The entry observes an import it could not cancel: it holds the one-open window,
    /// settling nothing — its promise settled at its deadline — until the app is done with
    /// the file ([`OpenStep::HoldWindow`] → [`OpenStep::Release`]).
    pub observing: bool,
    /// The name opens no document (a preset file: brushes, gradients… go to libraries).
    pub preset: bool,
}

/// What [`open_step`] says a pass must do with the open. The settling arms end the entry (the
/// one-open window opens); [`OpenStep::HoldWindow`] keeps it held without settling anything.
#[derive(Debug, PartialEq)]
pub enum OpenStep {
    /// Settle the promise with the error; release the window.
    SettleErr(String),
    /// Settle the promise with the warnings and the document (`None`: a preset file, which
    /// opens no document); release the window.
    SettleOk(Option<(u64, String)>),
    /// The deadline passed with the asked-for document never landed, so the import is still
    /// able to finish it: settle [`OPEN_TIMEOUT`], and terminate the underlying import
    /// *before* the window may open — through the app's own Cancel, whose public face is
    /// `open_prompt::answer(app, None)` (it drops every queued file and stops the reads still
    /// running). When even that cancel is refused — the damaged-file confirmation's answer is
    /// crate-private — the entry turns into an observer instead: [`OpenStep::HoldWindow`]
    /// until the app is done with the file, then [`OpenStep::Release`]. Either way, no
    /// document of a timed-out open can land into a later same-name open's claim.
    TimeoutPending,
    /// Hold the one-open window, settling nothing (a timed-out open's promise is already
    /// settled): later opens keep answering [`OPEN_BUSY`] until whatever the app still holds
    /// of the file is decided one way or the other.
    HoldWindow,
    /// The observation is over — the app holds nothing of the file anymore, so nothing can
    /// still land its document. Release the window; nothing settles (the promise settled at
    /// the deadline), and whatever document arrived in the meantime stays in the session,
    /// nobody's answer.
    Release,
    /// Nothing to settle or decide yet: the open waits — for its bytes, its document, its
    /// dialog's answer or its deadline.
    Wait,
}

/// The one settle decision for an open's pass, pure on the [`OpenPass`] — the whole wait
/// policy, in order:
///
/// 0. the import failed: settle at once with its error;
/// 1. the entry observes an import it could not cancel: hold the window while the app still
///    holds anything of the file, release when it doesn't — settling nothing either way, the
///    promise already settled at the deadline;
/// 2. the bytes are still being read: wait, or at the deadline settle [`OPEN_TIMEOUT`] and
///    release (nothing of the open has run — no import started, so there is nothing to cancel
///    or observe, unlike the step below, whose Cancel would stop unrelated queued files too);
/// 3. a preset file: settle at once with no document, deadline or not;
/// 4. the deadline passed — checked **before** the claim, so a request past its deadline never
///    takes a document that appeared in the deadline-crossing frame: with its document
///    already landed, settle [`OPEN_TIMEOUT`] and release (the document stays, untouched);
///    without it, [`OpenStep::TimeoutPending`] — the import must be terminated or observed
///    before the window opens;
/// 5. the asked-for document landed: the claim, settle with it;
/// 6. the app was seen holding the file and holds nothing now: it was disposed of (its dialog
///    cancelled) — settle [`OPEN_TIMEOUT`] at once rather than at the deadline;
/// 7. otherwise wait.
pub fn open_step(p: &OpenPass) -> OpenStep {
    if let Some(e) = &p.failed {
        return OpenStep::SettleErr(e.clone());
    }
    if p.observing {
        return if p.app_holds { OpenStep::HoldWindow } else { OpenStep::Release };
    }
    if p.reading {
        return if p.timed_out { OpenStep::SettleErr(OPEN_TIMEOUT.into()) } else { OpenStep::Wait };
    }
    if p.preset {
        return OpenStep::SettleOk(None);
    }
    if p.timed_out {
        return if p.landed.is_some() { OpenStep::SettleErr(OPEN_TIMEOUT.into()) } else { OpenStep::TimeoutPending };
    }
    if let Some(doc) = &p.landed {
        return OpenStep::SettleOk(Some(doc.clone()));
    }
    if p.held && !p.app_holds {
        return OpenStep::SettleErr(OPEN_TIMEOUT.into());
    }
    OpenStep::Wait
}

/// The page origin allowed to drive the app over `postMessage`: `?embed=<origin>` in the query
/// (`location.search`, with or without its `?`), percent-decoded and written as browsers write
/// `MessageEvent.origin` (lower case, no default port, no trailing `/`). Only an `http(s)` origin
/// (scheme, host and optional port) counts; anything else leaves the bridge off.
pub fn allowed_origin(query: &str) -> Option<String> {
    origin(&percent_decode(param(query, "embed")?))
}

/// The value of `name` in a query string (`location.search`, with or without its `?`), raw.
fn param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
}

/// `s` as an origin written the way `MessageEvent.origin` writes one, when it is an `http(s)`
/// origin (scheme, host and optional port; nothing after them but an optional `/`).
fn origin(s: &str) -> Option<String> {
    let origin = s.to_ascii_lowercase();
    let origin = origin.strip_suffix('/').unwrap_or(&origin);
    let (scheme, rest) = origin.split_once("://")?;
    let default_port = match scheme {
        "https" => ":443",
        "http" => ":80",
        _ => return None,
    };
    let host = rest.strip_suffix(default_port).unwrap_or(rest);
    let valid = !host.is_empty() && !host.contains(['/', '?', '#', '@', ' ', '\\']);
    valid.then(|| format!("{scheme}://{host}"))
}

/// A call from the host page.
#[derive(Debug, PartialEq)]
pub enum Call {
    /// Open the document in the message's `data` under `name`: bytes (an ArrayBuffer, a typed
    /// array, a Blob). No `name`: the import runs under the "document" hint and the open
    /// settles as the document it admits, whatever the app named it (a `.pcraft` keeps its
    /// own).
    Open {
        name: Option<String>,
    },
    /// The active document's bytes in `format` (an extension: `pcraft`, `psd`, `png`…).
    Save {
        format: String,
    },
    IsDirty,
    Info,
}

/// Reads a `postMessage` call: `{photocraft: <id>, method, …}` → (id, call). The id (a number or a
/// string) comes back in the reply. Messages without a `photocraft` id aren't for the app: `None`.
/// The control channel (`window.photocraft.request`) isn't offered here: the embedding page names
/// itself in `?embed=`, so any site could iframe the app that way and drive every command.
pub fn parse_call(msg: &Value) -> Option<(Value, Result<Call, String>)> {
    let id = msg.get("photocraft").filter(|v| v.is_number() || v.is_string())?.clone();
    let str_of = |k: &str| msg.get(k).and_then(Value::as_str).map(str::to_string);
    let call = match msg.get("method").and_then(Value::as_str) {
        Some("open") => Ok(Call::Open { name: str_of("name") }),
        Some("save") => Ok(Call::Save { format: str_of("format").unwrap_or_else(|| photocraft_format::EXTENSION.into()) }),
        Some("isDirty") => Ok(Call::IsDirty),
        Some("info") => Ok(Call::Info),
        Some("request") => Err("`request` is only for the app's own page (window.photocraft.request)".to_string()),
        Some(other) => Err(format!("unknown method `{other}`")),
        None => Err("`method` is required".to_string()),
    };
    Some((id, call))
}

/// The reply to call `id`: `{photocraft: id, ok: true, result}` or `{…, ok: false, error}`.
pub fn reply(id: &Value, result: Result<Value, String>) -> Value {
    match result {
        Ok(result) => json!({ "photocraft": id, "ok": true, "result": result }),
        Err(error) => json!({ "photocraft": id, "ok": false, "error": error }),
    }
}

/// The file name the active document is exported under for `format`: its name with the format's
/// extension. `format` is an extension, letters and digits only.
pub fn export_name(doc_name: &str, format: &str) -> Result<String, String> {
    let format = format.trim().trim_start_matches('.').to_ascii_lowercase();
    if format.is_empty() || !format.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("`{format}` isn't a file format"));
    }
    let stem = doc_name.rsplit_once('.').map_or(doc_name, |(stem, _)| stem);
    let stem = if stem.trim().is_empty() { "document" } else { stem };
    Ok(format!("{stem}.{format}"))
}

/// The lower-case extension of `path` (empty when it has none).
pub fn extension(path: &str) -> String {
    std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// `%XX` escapes decoded as UTF-8 (invalid sequences kept as they are).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = |j: usize| bytes.get(j).and_then(|c| (*c as char).to_digit(16));
        if b == b'%'
            && let (Some(h), Some(l)) = (hex(i + 1), hex(i + 2))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_gives_the_versions_and_the_active_document() {
        // #1614's requirement 5: a host checks a `.pcraft` against `FormatError::TooNew` with
        // these two fields, so both are locked against the constants every other frontend
        // reports (the About dialog, `--version`) — not against literals that could drift.
        assert_eq!(info()["app"], json!(photocraft_engine::build_info::VERSION), "the app version, from the engine's build_info");
        assert_eq!(info()["format"], json!(photocraft_format::FORMAT_VERSION), "the .pcraft format version, from the format crate");
        assert!(info()["app"].as_str().is_some_and(|v| !v.is_empty()), "a version a host can compare");
        assert!(info()["format"].is_u64(), "the format version is the number FormatError::TooNew compares against");
        assert_eq!(
            info(),
            json!({"app": photocraft_engine::build_info::VERSION, "format": photocraft_format::FORMAT_VERSION, "doc": null}),
            "the key is always there, so a host can read info().doc without checking"
        );
        let doc = DocInfo { id: 7, name: "Poster.psd".into() };
        assert_eq!(
            info_with(Some(&doc)),
            json!({"app": photocraft_engine::build_info::VERSION, "format": photocraft_format::FORMAT_VERSION, "doc": {"id": 7, "name": "Poster.psd"}})
        );
    }

    #[test]
    fn opens_over_the_byte_budget_are_refused_before_any_copy() {
        assert_eq!(check_open_bytes(None), Ok(()), "an unknown shape is measured once the bytes are in hand");
        assert_eq!(check_open_bytes(Some(0)), Ok(()));
        assert_eq!(check_open_bytes(Some(MAX_OPEN_BYTES)), Ok(()), "the limit itself is allowed");
        assert_eq!(check_open_bytes(Some(MAX_OPEN_BYTES + 1)), Err(OPEN_TOO_BIG.into()));
        assert_eq!(MAX_OPEN_BYTES, 67_108_864, "64 MiB, 2.6× the 24.5 MB largest production asset");
        assert_eq!(OPEN_READ, "open couldn't read the data as bytes: it takes an ArrayBuffer, a typed array or a Blob");
        assert_ne!(OPEN_TOO_BIG, OPEN_READ, "the two refusals are the host's only way to tell too big from unreadable");
    }

    #[test]
    fn bridge_messages_have_a_weight_and_depth_budget_before_any_decode() {
        assert_eq!(MAX_CALL_BYTES, 65_536, "64 KiB — a call is an id, a method and a name");
        assert_eq!(MAX_CALL_DEPTH, 8, "a call is one object deep");
        assert_eq!(
            CALL_TOO_BIG,
            "the message is larger than 64 KiB (65536 bytes) or deeper than 8 levels without its data; send the file in data and only the call in the message"
        );
        const { assert!(MAX_CALL_BYTES < MAX_OPEN_BYTES / 16, "the call around the data is a fraction of what the data may be") };
    }

    #[test]
    fn a_sparse_array_is_weighed_by_its_length_not_its_keys() {
        // The N2 lock: `new Array(1_000_000)` owns one key (`length`), which an own-keys walk
        // weighs at under a hundred bytes, while `JSON.stringify` spells a megabyte of `null,`.
        // The floor alone refuses it — before a single element is visited.
        assert_eq!(call_array_floor(1_000_000), Some(5_000_002), "5 bytes a hole, 2 the brackets");
        assert!(call_array_floor(1_000_000).is_some_and(|floor| floor > MAX_CALL_BYTES), "refused on the length alone");
        // A working-sized array is walked, not refused on its floor.
        assert!(call_array_floor(10_000).is_some_and(|floor| floor <= MAX_CALL_BYTES));
        assert_eq!(call_array_floor(0), Some(2), "an empty array spells []");
        // A length past counting (the caller saturates Infinity to `usize::MAX`): refuse,
        // never undercount.
        assert_eq!(call_array_floor(usize::MAX), None);
    }

    #[test]
    fn strings_are_weighed_by_their_utf8_and_escape_upper_bound() {
        // The N2 lock, string half: 20 000 units of CJK are 60 KB as UTF-8 (and up to 120 KB
        // spelled) — their unit count (20 000) passed 64 KiB, the bound refuses them. A
        // NUL-heavy string spells `\u0000`, six characters a unit: covered by the same bound.
        assert_eq!(call_string_cost(1), Some(8), "six for the worst unit, two for the quotes");
        assert!(call_string_cost(10_000).is_some_and(|cost| cost <= MAX_CALL_BYTES));
        assert!(call_string_cost(20_000).is_some_and(|cost| cost > MAX_CALL_BYTES), "60 KB of CJK or 120 KB of escapes doesn't ride the JSON face");
        assert_eq!(call_string_cost(usize::MAX), None, "an overflowing bound refuses, never undercounts");
    }

    #[test]
    fn the_bridge_limits_are_fixed_numbers_and_text() {
        assert_eq!(MAX_PENDING_CALLS, 8);
        assert_eq!(
            OPEN_TIMEOUT_MS, 30_000.0,
            "one open at a time makes the deadline also the lock on every later open behind OPEN_BUSY — at 120 s that was two minutes; 30 s still spans answering a dialog over an import, and a cancelled one now rejects at once through the prompt read"
        );
        assert_eq!(OPEN_TIMEOUT, "open gave up waiting for the document: it is held behind a dialog in the app, or never finished opening");
        assert_eq!(BUSY, "busy: 8 calls are already waiting for their replies; send another once they answer");
        assert_eq!(OPEN_BUSY, "busy: another open is still waiting for its document; open again once it answers");
        assert_ne!(OPEN_BUSY, BUSY, "both busy, each with the count that is true for it");
    }

    #[test]
    fn preset_files_open_without_a_document() {
        assert!(expects_document("Poster.psd"));
        assert!(expects_document("document"), "no extension: a document");
        for ext in ["abr", "grd", "asl", "pat", "aco", "act", "ase", "kys", "psp"] {
            assert!(!expects_document(&format!("library.{ext}")), "{ext}");
        }
    }

    #[test]
    fn an_open_lands_only_as_a_document_of_its_exact_name() {
        let docs = [(1, "Old.psd".to_string()), (2, "Poster.psd".to_string()), (3, "later.psd".to_string())];
        assert_eq!(landed_doc(&[1], Some("Poster.psd"), &docs), Some((2, "Poster.psd".to_string())), "the new one with the asked-for name");
        assert_eq!(landed_doc(&[1], Some("no such name"), &docs), None, "no fallback: a document of another name is never this open's");
        assert_eq!(landed_doc(&[1, 2, 3], Some("Poster.psd"), &docs), None, "nothing new yet");
        assert_eq!(landed_doc(&[], Some("Poster.psd"), &[]), None, "no documents at all");
        let named_late = [(1, "Old.psd".to_string()), (4, "other.psd".to_string()), (2, "Poster.psd".to_string())];
        assert_eq!(
            landed_doc(&[1], Some("Poster.psd"), &named_late),
            Some((2, "Poster.psd".to_string())),
            "the named one wins over an earlier new one it doesn't name"
        );
        // The document of that name that was already there doesn't answer: only what is new
        // since this import started can.
        assert_eq!(landed_doc(&[1], Some("Poster.psd"), &[(1, "Poster.psd".to_string())]), None, "the old same-named document isn't this open's");
    }

    #[test]
    fn two_opens_of_one_name_each_answer_their_own_document() {
        // Serialized: A settles before B is even taken (a second open while one is in flight is
        // refused OPEN_BUSY), so B's `before` holds everything A saw plus A's document — and
        // the duplicate the app opens under the same name for B answers B, never A's again.
        let first = [(1, "Old.psd".to_string()), (2, "a.psd".to_string())];
        assert_eq!(landed_doc(&[1], Some("a.psd"), &first), Some((2, "a.psd".to_string())));
        let second = [(1, "Old.psd".to_string()), (2, "a.psd".to_string()), (4, "a.psd".to_string())];
        assert_eq!(
            landed_doc(&[1, 2], Some("a.psd"), &second),
            Some((4, "a.psd".to_string())),
            "the new duplicate, not the first open's document a second time"
        );
    }

    #[test]
    fn a_cancelled_open_and_a_renamed_template_never_claim_another_document() {
        // The reviewer's counter-example under the one-open design: A ("a.psd") waits behind a
        // cancelled dialog while a document of another name ("b.psd", B's would-be file)
        // appears — nothing lands it as A's; A rejects (at its deadline, or at once through
        // the prompt read) and "b.psd" stays in the session, nobody's answer. A `.psdt`,
        // which the app opens as "Untitled-N", is the same shape: never claimed.
        assert_eq!(landed_doc(&[], Some("a.psd"), &[(9, "b.psd".to_string())]), None);
        assert_eq!(landed_doc(&[1], Some("card.psdt"), &[(1, "Old.psd".to_string()), (2, "Untitled-1".to_string())]), None);
    }

    #[test]
    fn an_open_without_a_name_settles_as_the_document_the_import_admitted() {
        // R3-4: `open` without a name runs its import under the "document" file-name hint,
        // but a `.pcraft` lands under the name its manifest persisted — an exact-"document"
        // claim could never see it, and the open rejected at its 30 s deadline with its
        // document sitting open in the session. The name-less claim is the import's own
        // admitted document: the first `DocId` new since the import started, under whatever
        // name the session gave it — answered back under that name, never renamed to the
        // hint.
        let docs = [(1, "Old.psd".to_string()), (2, "Poster.pcraft".to_string())];
        assert_eq!(landed_doc(&[1], None, &docs), Some((2, "Poster.pcraft".to_string())), "the admitted document, under its own name");
        assert_eq!(landed_doc(&[1, 2], None, &docs), None, "nothing new since the import: still waiting");
        assert_eq!(landed_doc(&[], None, &[]), None, "no documents at all");
        assert_eq!(
            landed_doc(&[], None, &[(5, "Untitled-1".to_string())]),
            Some((5, "Untitled-1".to_string())),
            "a template the app renames is a name-less open's too: it asked for no name"
        );
        // A named open still demands its exact name — and the name-less hint is not a name
        // anyone asked for, so it claims nothing by itself.
        assert_eq!(landed_doc(&[1], Some("document"), &docs), None, "the persisted name is not the hint the import ran under");
    }

    /// R3-4's walkthrough, end to end on the pure policy: a default save (no format asked)
    /// writes `Poster.pcraft`; `open` without a name imports it under the "document" hint,
    /// and the session adopts the name the manifest persisted — so the claim must settle on
    /// that document by id, under the save's own name, where the old exact-"document" match
    /// never could.
    #[test]
    fn a_default_save_reopened_without_a_name_settles_by_its_own_name() {
        use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType, Size};
        use photocraft_io::{ExportOptions, export};
        use photocraft_raster::Surface;

        let mut doc = Document::new("Poster.psd", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
        let mut surface = Surface::new(doc.pixel_format());
        surface.fill_rect(doc.bounds(), &[1.0, 0.5, 0.25, 1.0]);
        doc.layers.push(Layer::new("Background", LayerContent::Raster(surface)));
        let name = export_name(&doc.name, photocraft_format::EXTENSION).expect("a name");
        let saved = export(&doc, &name, &ExportOptions::default()).expect("the default .pcraft save");
        let reopened = photocraft_format::load_from_bytes(&saved.bytes).expect("the save loads back");
        assert_eq!(reopened.name, "Poster.pcraft", "the document the app admits for the reopened save");
        // The session held one document (id 1) when the reopen's import started; the admitted
        // one (id 7) answers under its own name.
        let session = [(7, reopened.name.clone())];
        assert_eq!(landed_doc(&[1], None, &session), Some((7, "Poster.pcraft".to_string())), "the name-less reopen settles, by id");
        assert_eq!(landed_doc(&[1], Some("document"), &session), None, "the exact-hint claim the old policy made could only time out on it");
    }

    #[test]
    fn a_reply_posted_to_the_apps_own_window_is_never_delivered() {
        // R3-1: at top level `window.parent()` is the app's own window. A post there has the
        // browser detach the transferred buffer while it serializes the message and drop the
        // message on the origin check right after — `postMessage` itself returns normally —
        // so a reply "posted" to it reads as delivered (posted, buffer gone) while its bytes
        // reached no page, and the in-place save settled on it would clear a document's
        // unsaved state, hiding the unsaved-work prompt. The post is refused before it is
        // made, and delivery is what it always meant: a post that went out to a distinct
        // parent, whose buffer left.
        assert!(!posts_to(ReplyTarget::OwnWindow), "top level: never post to the app's own window");
        assert!(!posts_to(ReplyTarget::Detached), "no parent to post to at all");
        assert!(posts_to(ReplyTarget::Parent), "the documented iframe: a distinct parent");
        assert!(delivered(true, 0), "posted to a distinct parent, transfer taken");
        assert!(!delivered(false, 0), "a post refused — top level, no bridge — is not delivery");
        assert!(!delivered(true, 4_096), "a buffer still in the app's hands is not delivery");
        assert!(!delivered(false, 4_096), "nothing posted, nothing transferred");
    }

    #[test]
    fn the_bridge_takes_calls_only_from_the_framing_parent() {
        // R3-3: the origin check alone lets any window at the `?embed=` origin drive the app —
        // a sibling iframe of the embedder's page among them. A request from any window but
        // the parent could never be answered (replies go to the parent only), but it would
        // still run — its `open`/`save` executed by the app. The source must be the framing
        // parent, and at top level, where there is none, nothing is taken at all.
        assert!(takes_call(true, MessageSource::Parent), "the embedder: the right window at the right origin");
        assert!(!takes_call(true, MessageSource::Other), "a sibling at the right origin is still not the embedder");
        assert!(!takes_call(true, MessageSource::None), "not framed: no embedder to serve");
        assert!(!takes_call(false, MessageSource::Parent), "the right window at the wrong origin is refused too");
        assert!(!takes_call(false, MessageSource::Other));
    }

    /// [`OpenPass`] for an ordinary document open, nothing unusual found yet.
    fn pass() -> OpenPass {
        OpenPass::default()
    }

    #[test]
    fn an_open_settles_as_the_state_machine_says() {
        // Every arm of open_step, in the pass order it decides them — removing or reordering
        // any arm turns its line red.
        // 0. the import failed: its error, at once, whatever else is true.
        assert_eq!(
            open_step(&OpenPass { failed: Some("not a PSD".into()), observing: true, timed_out: true, ..pass() }),
            OpenStep::SettleErr("not a PSD".into())
        );
        // 1. an observer: nothing ever settles again — the window holds while the app holds,
        //    and opens when it doesn't (the promise settled at the deadline).
        assert_eq!(open_step(&OpenPass { observing: true, app_holds: true, ..pass() }), OpenStep::HoldWindow);
        assert_eq!(
            open_step(&OpenPass { observing: true, app_holds: false, landed: Some((7, "a.psd".into())), ..pass() }),
            OpenStep::Release,
            "a landed document doesn't re-open the claim either"
        );
        assert_eq!(open_step(&OpenPass { observing: true, app_holds: false, ..pass() }), OpenStep::Release);
        // 2. bytes on their way: wait; at the deadline, settle the timeout and release cleanly
        //    (nothing ran — no import started, so there is nothing to cancel or observe).
        assert_eq!(open_step(&OpenPass { reading: true, ..pass() }), OpenStep::Wait);
        assert_eq!(open_step(&OpenPass { reading: true, timed_out: true, ..pass() }), OpenStep::SettleErr(OPEN_TIMEOUT.into()));
        // 3. a preset file settles at once with no document — deadline or not, prompt or not.
        assert_eq!(open_step(&OpenPass { preset: true, ..pass() }), OpenStep::SettleOk(None));
        assert_eq!(open_step(&OpenPass { preset: true, timed_out: true, app_holds: true, ..pass() }), OpenStep::SettleOk(None));
        // 4. the deadline, before the claim: a document that landed with it is nobody's
        //    answer — the open still rejects, the document stays in the session untouched.
        assert_eq!(open_step(&OpenPass { timed_out: true, landed: Some((2, "a.psd".into())), ..pass() }), OpenStep::SettleErr(OPEN_TIMEOUT.into()));
        // 4'. the deadline with nothing landed: the import must be terminated or observed
        //     before the window may open.
        assert_eq!(open_step(&OpenPass { timed_out: true, app_holds: true, ..pass() }), OpenStep::TimeoutPending);
        assert_eq!(open_step(&OpenPass { timed_out: true, app_holds: false, ..pass() }), OpenStep::TimeoutPending);
        // 5. the claim: the asked-for document, new and exactly named.
        assert_eq!(open_step(&OpenPass { landed: Some((2, "a.psd".into())), ..pass() }), OpenStep::SettleOk(Some((2, "a.psd".into()))));
        // 6. the app was seen holding the file and holds nothing now: disposed of — reject at
        //    once; while it still holds anything (a prompt, a job of the name), wait.
        assert_eq!(open_step(&OpenPass { held: true, app_holds: false, ..pass() }), OpenStep::SettleErr(OPEN_TIMEOUT.into()));
        assert_eq!(open_step(&OpenPass { held: true, app_holds: true, ..pass() }), OpenStep::Wait);
        // 7. nothing yet: wait.
        assert_eq!(open_step(&pass()), OpenStep::Wait);
    }

    #[test]
    fn a_timed_out_open_terminates_or_observes_before_the_window_opens() {
        // P1-1's walkthrough, as the machine sees it: A opens; its import is held behind a
        // dialog past its deadline. The promise rejects OPEN_TIMEOUT (TimeoutPending), and
        // the window may only open once nothing can still land A's document: the driver
        // terminates the import through the app's Cancel, and only the one cancel that the
        // public API refuses (the damaged-file confirmation) leaves the entry observing —
        // hold while the app holds anything of the file, release when it doesn't.
        let mut a = pass();
        assert_eq!(open_step(&a), OpenStep::Wait, "A waits behind the dialog (the app holds it)");
        a.held = true;
        a.app_holds = true;
        assert_eq!(open_step(&a), OpenStep::Wait);
        a.timed_out = true;
        assert_eq!(open_step(&a), OpenStep::TimeoutPending, "the deadline: reject, then terminate or observe");
        // The cancel was refused (a damage front): the entry observes. A document of the name
        // landing while it observes changes nothing — the promise settled at the deadline.
        a.observing = true;
        a.landed = Some((4, "a.psd".into()));
        assert_eq!(open_step(&a), OpenStep::HoldWindow, "the landed document is nobody's answer; the window holds");
        a.app_holds = false;
        assert_eq!(open_step(&a), OpenStep::Release, "the app is done with the file: the window opens");
        // B, opened after the release, claims only its own document — A's landed one is in
        // B's before, and a same-name document of B's own is B's answer (the audit's A/B
        // counter-example, same name, different contents).
        let before = [1, 4];
        let docs = [(1, "Old.psd".to_string()), (4, "a.psd".to_string()), (5, "a.psd".to_string())];
        assert_eq!(landed_doc(&before, Some("a.psd"), &docs), Some((5, "a.psd".to_string())), "B's own new document, never A's late one");
    }

    #[test]
    fn a_cancelled_dialog_ends_the_wait_only_when_nothing_still_opens_the_name() {
        // The disposed-of read (step 6) needs the app to hold *nothing* of the file: a prompt
        // gone is not enough while a job of the same name is still opening (an approved
        // high-memory decode runs on) — the entry waits, and the deadline machinery takes it
        // from there. `app_holds` is the pass's half of that read: prompt up, file queued, or
        // an opening job of the name.
        assert_eq!(open_step(&OpenPass { held: true, app_holds: false, ..pass() }), OpenStep::SettleErr(OPEN_TIMEOUT.into()));
        assert_eq!(
            open_step(&OpenPass { held: true, app_holds: true, timed_out: true, ..pass() }),
            OpenStep::TimeoutPending,
            "still opening: not disposed of, the deadline applies"
        );
    }

    #[test]
    fn binary_on_the_metadata_plane_is_refused_whole() {
        // The N2 root fix, text lock: binary anywhere outside `data` is not weighed — the call
        // is refused whole. The walk that decides it reads the clone in the embed module, which
        // is wasm-only and cannot sit in this native test build; this locks its refusal text
        // (the duck it refuses on is the embed side's, covered by the host's browser checks).
        assert_eq!(
            CALL_BINARY,
            "the message carries binary data (a typed array, a DataView or an ArrayBuffer) outside its data member; send the file in data and only the call in the message"
        );
        assert_ne!(CALL_BINARY, CALL_TOO_BIG, "a host can tell binary-in-metadata from over-budget");
    }

    #[test]
    fn a_key_name_is_weighed_by_its_length_alone() {
        // The N2 pre-allocation lock, key half: a member key's name is charged by its UTF-16
        // length before any of it is copied (the embed walk reads `JsString::length` and only
        // ever spells a name of "data"'s own length into Rust) — a giant top-level key is
        // refused on the length alone: 10 973 units spell at 65 840, past the 64 KiB budget;
        // 10 922 (65 334) rides.
        assert!(call_string_cost(10_973).is_some_and(|cost| cost > MAX_CALL_BYTES));
        assert!(call_string_cost(10_922).is_some_and(|cost| cost <= MAX_CALL_BYTES));
    }

    #[test]
    fn the_over_budget_view_answers_open_too_big_verbatim() {
        // N4's mapping as a real assertion: the sentinel the view picker resolves with
        // (`false` — a real byte shape over MAX_OPEN_BYTES, found before any view of it is
        // made) maps to OPEN_TOO_BIG, character for character; a value that isn't a view maps
        // to OPEN_READ; a view whose own length is over the budget refuses the same
        // OPEN_TOO_BIG, before the copy it would pay for.
        assert_eq!(view_refusal(true, false, 0).as_deref(), Some(OPEN_TOO_BIG));
        assert_eq!(view_refusal(true, false, 0).as_deref(), Some("open: the file is larger than 64 MiB (67108864 bytes)"));
        assert_eq!(view_refusal(false, false, 0).as_deref(), Some(OPEN_READ));
        assert_eq!(view_refusal(false, true, MAX_OPEN_BYTES + 1).as_deref(), Some(OPEN_TOO_BIG));
        assert_eq!(view_refusal(false, true, MAX_OPEN_BYTES), None, "the limit itself is allowed");
        assert_eq!(view_refusal(false, true, 0), None, "an empty view reads on");
        // The range out of the copy: whole, slice, and a hostile range can only fail to read.
        assert_eq!(view_range(vec![1, 2, 3, 4], 0, 4), vec![1, 2, 3, 4]);
        assert_eq!(view_range(vec![1, 2, 3, 4], 0, usize::MAX), vec![1, 2, 3, 4], "a range past the end takes the whole copy");
        assert_eq!(view_range(vec![1, 2, 3, 4], 1, 2), vec![2, 3]);
        assert_eq!(view_range(vec![1, 2, 3, 4], 3, u8::MAX as usize), Vec::<u8>::new(), "past the end reads as empty");
        assert_eq!(view_range(Vec::<u8>::new(), 0, 0), Vec::<u8>::new());
    }

    #[test]
    fn the_embed_origin_is_an_http_origin_or_nothing() {
        assert_eq!(allowed_origin("embed=https%3A%2F%2Fadmin.example.com"), Some("https://admin.example.com".into()));
        assert_eq!(allowed_origin("?embed=https://admin.example.com"), Some("https://admin.example.com".into()), "location.search keeps its ?");
        assert_eq!(allowed_origin("embed=https://Admin.Example.com:443/"), Some("https://admin.example.com".into()), "as MessageEvent.origin writes it");
        assert_eq!(allowed_origin("embed=http://host:8443"), Some("http://host:8443".into()));
        assert_eq!(allowed_origin("webgl&embed=http://localhost:8080"), Some("http://localhost:8080".into()));
        assert_eq!(allowed_origin("embed=https://example.com/path"), None, "an origin has no path");
        assert_eq!(allowed_origin("embed=*"), None);
        assert_eq!(allowed_origin("embed=javascript:alert(1)"), None);
        assert_eq!(allowed_origin("cpu"), None);
    }

    #[test]
    fn calls_are_read_from_messages_with_an_id() {
        let call = |v: Value| parse_call(&v).map(|(id, c)| (id, c.map_err(|_| ())));
        assert_eq!(call(json!({"type": "something else"})), None, "not for the app");
        assert_eq!(call(json!({"photocraft": 1, "method": "save", "format": "psd"})), Some((json!(1), Ok(Call::Save { format: "psd".into() }))));
        assert_eq!(
            call(json!({"photocraft": "a", "method": "save"})),
            Some((json!("a"), Ok(Call::Save { format: photocraft_format::EXTENSION.into() }))),
            "no format: the app's own .pcraft, the layered default"
        );
        assert_eq!(call(json!({"photocraft": 2, "method": "open", "name": "x.psd"})), Some((json!(2), Ok(Call::Open { name: Some("x.psd".into()) }))));
        assert_eq!(
            call(json!({"photocraft": 2, "method": "open"})),
            Some((json!(2), Ok(Call::Open { name: None }))),
            "no name: the import's hint is \"document\", the claim the admitted document"
        );
        assert_eq!(
            call(json!({"photocraft": 3, "method": "request", "request": "ui.inspect"})),
            Some((json!(3), Err(()))),
            "no control channel over postMessage"
        );
        assert_eq!(call(json!({"photocraft": 5, "method": "rm -rf"})), Some((json!(5), Err(()))));
        assert_eq!(call(json!({"photocraft": {"x": 1}, "method": "info"})), None, "the id is a number or a string");
        assert_eq!(reply(&json!(7), Ok(json!(true))), json!({"photocraft": 7, "ok": true, "result": true}));
        assert_eq!(reply(&json!(7), Err("no".into())), json!({"photocraft": 7, "ok": false, "error": "no"}));
    }

    #[test]
    fn exports_take_the_documents_name_and_the_formats_extension() {
        assert_eq!(export_name("Poster.psd", "png"), Ok("Poster.png".into()));
        assert_eq!(export_name("Untitled-1", ".PSD"), Ok("Untitled-1.psd".into()));
        assert_eq!(export_name("", "pcraft"), Ok("document.pcraft".into()));
        assert_eq!(export_name("Poster.psd", photocraft_format::EXTENSION), Ok("Poster.pcraft".into()), "the save default keeps the layers");
        assert!(export_name("a.psd", "../x").is_err());
        assert!(export_name("a.psd", "").is_err());
    }

    /// #1614's requirement 2, the `.pcraft` half: a bridge save without a `format` asks for the
    /// app's own format, and the exact export the web shell runs for it (`export_name` for the
    /// name, `photocraft_io::export` for the bytes) answers with a `.pcraft` that loads back.
    #[test]
    fn a_pcraft_save_round_trips_through_the_embed_export_path() {
        use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType, Size};
        use photocraft_io::{ExportOptions, export};
        use photocraft_raster::Surface;

        let mut doc = Document::new("Poster.psd", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
        let mut surface = Surface::new(doc.pixel_format());
        surface.fill_rect(doc.bounds(), &[1.0, 0.5, 0.25, 1.0]);
        doc.layers.push(Layer::new("Background", LayerContent::Raster(surface)));
        let name = export_name(&doc.name, photocraft_format::EXTENSION).expect("a name");
        assert_eq!(name, "Poster.pcraft");
        let saved = export(&doc, &name, &ExportOptions::default()).expect("the .pcraft export the bridge's save asks for");
        assert!(photocraft_format::is_pcraft(&saved.bytes), "a .pcraft, not a download of some other format");
        let loaded = photocraft_format::load_from_bytes(&saved.bytes).expect("the saved bytes load back");
        // A `.pcraft` manifest reports the file it lives in (#3312), so the loaded document is
        // named after the export, with the canvas intact.
        assert_eq!((loaded.name.as_str(), loaded.size.width, loaded.size.height), ("Poster.pcraft", 8, 8), "the document survives the round trip");
    }
}
