//! What the embedding API (#1614) agrees with its host page, kept free of browser types so it is
//! tested natively: the `?embed=` origin, the `postMessage` calls and replies, the version info,
//! the bridge's limits, the open-claim policy, and the Photopea-compatible mode (a
//! `#{"files":…,"server":…}` config in the URL hash, refused in this build, and Photopea's save
//! body: https://www.photopea.com/api/).

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
pub fn info_with(doc: Option<&DocInfo>) -> Value {
    let doc = match doc {
        None => Value::Null,
        Some(d) => json!({ "id": d.id, "name": d.name }),
    };
    json!({ "app": env!("CARGO_PKG_VERSION"), "format": photocraft_format::FORMAT_VERSION, "doc": doc })
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
/// and `postMessage`) come through the one open path. A handed-over byte shape is gated on its
/// true length before any view of it is made; a fetched URL body is counted chunk by chunk and
/// refused at the crossing, whether or not it declared a `Content-Length`.
pub const OPEN_TOO_BIG: &str = "open: the file is larger than 64 MiB (67108864 bytes)";

/// The fixed rejection when an open's bytes can't be read at all: a value that isn't an
/// ArrayBuffer, a typed array or a Blob, or anything its conversion throws (a Proxy of a Blob,
/// a typed array whose elements can't convert). One text for both — the host can't act on the
/// difference, and neither can the app.
pub const OPEN_READ: &str = "open couldn't read the data as bytes: it takes an ArrayBuffer, a typed array or a Blob";

/// The fixed rejection when the *string* an open got isn't a URL the app may fetch itself: not
/// an `http(s)` URL once resolved against the app's own location, or not on the `?embed=`
/// origin — the one origin the bridge answers to, and the one an open fetches on with the
/// page's session cookies (how an embedder's authenticated asset endpoints work). The same
/// refusal for a string too long to be a URL (over [`MAX_CALL_BYTES`] at the worst-unit bound
/// of [`call_string_cost`] — it is never copied into Rust to find out) and for the URL form
/// with the bridge off, no `?embed=` origin to name the one origin at all: a host holding the
/// bytes (the app's own page) sends them.
pub const OPEN_URL: &str =
    "open couldn't take the string as a same-origin URL: it takes an http(s) URL on the ?embed= origin, resolved against the app's own location";

/// The fixed rejection when the fetch an open's URL asked for doesn't answer with the file: a
/// network failure, a refused connection, a redirect (redirects are never followed), an HTTP
/// error status or a body-stream failure is the one text (the host can retry; it can't act on
/// the difference). A direct 2xx answer's body answers to the streaming byte budget
/// ([`MAX_OPEN_BYTES`], [`OPEN_TOO_BIG`]), never imported without that count.
pub const OPEN_FETCH: &str = "open couldn't fetch the URL: the fetch failed or the server didn't answer a 2xx status";

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

/// The response face of a URL open, checked **before any byte of the body is read**: an answer
/// that is 2xx *and* never followed a redirect — anything else is the one fixed [`OPEN_FETCH`]
/// — and a declared `Content-Length` over
/// [`MAX_OPEN_BYTES`] refused as [`OPEN_TOO_BIG`] before the read starts. `redirect: "error"`
/// makes any 3xx fail the fetch itself, so a `redirected` response shouldn't exist; the
/// re-check here is the belt under that. A missing, unreadable or negative length is not an
/// error: it decides nothing (the streaming read counts the real bytes as they arrive,
/// [`stream_step`]) — only a length that *can* be read and is over the budget refuses.
pub fn response_gate(ok: bool, redirected: bool, content_length: Option<&str>) -> Result<(), String> {
    if !ok || redirected {
        return Err(OPEN_FETCH.into());
    }
    match content_length.and_then(|v| v.trim().parse::<u64>().ok()) {
        Some(n) if n > MAX_OPEN_BYTES as u64 => Err(OPEN_TOO_BIG.into()),
        _ => Ok(()),
    }
}

/// One step of a URL open's streaming read: `chunk` bytes arrived on top of `total`. [`Ok`] is
/// the new running total — never past [`MAX_OPEN_BYTES`] — and the crossing itself refuses
/// [`OPEN_TOO_BIG`], the same text the handed-over byte path gives, so a too-large file spells
/// the same refusal whichever side noticed. The caller cancels the reader at the refusal, so
/// nothing past the budget is ever buffered: a response without a `Content-Length`, or with a
/// lying one, can only push the running count to the crossing, where the read stops. An
/// overflowing count refuses too, never wraps.
pub fn stream_step(total: usize, chunk: usize) -> Result<usize, String> {
    let next = total.checked_add(chunk).ok_or_else(|| OPEN_TOO_BIG.to_string())?;
    if next > MAX_OPEN_BYTES {
        return Err(OPEN_TOO_BIG.into());
    }
    Ok(next)
}

/// Whether opening `name` adds a document to the session: preset files (brushes, gradients,
/// styles, swatches, preferences) go to their libraries instead, so those opens settle at once.
/// The list mirrors `preset_files_ui::PRESET_EXTS` in `ui-egui`.
pub fn expects_document(name: &str) -> bool {
    const PRESET_EXTS: &[&str] = &["abr", "grd", "asl", "pat", "aco", "act", "ase", "kys", "psp"];
    !PRESET_EXTS.contains(&extension(name).as_str())
}

/// The document an open of `name` settles on: the first document new since the import started
/// (`before`) whose name is `name`, *exactly*. Nothing else — there is no fallback to any
/// other new document, so a document of a different name is never this open's (two opens
/// cannot answer with each other's, whatever timing lands them in one pass), and an open whose
/// document never comes — its dialog cancelled, or a `.psdt` the app opens as "Untitled-N" —
/// waits out its deadline and rejects. `None` while nothing of the name has appeared: the
/// import is still running, or is held behind a dialog in the app.
pub fn landed_doc(before: &[u64], name: &str, docs: &[(u64, String)]) -> Option<(u64, String)> {
    docs.iter().find(|(id, n)| !before.contains(id) && n == name).map(|(id, n)| (*id, n.clone()))
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
    /// The bytes are still on their way (a URL fetch or a byte read): nothing of the open has
    /// run, and no document can be its answer yet.
    pub fetching: bool,
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
    /// The deadline passed while the bytes were still on their way: settle [`OPEN_TIMEOUT`]
    /// and release — nothing of the open has run, so nothing can still land. (The pass also
    /// kills the in-flight transfer; the abort handle lives with the entry's source.)
    AbortFetch,
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
/// 2. the bytes are still on their way: wait, or at the deadline settle [`OPEN_TIMEOUT`] and
///    release (nothing of the open has run);
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
    if p.fetching {
        return if p.timed_out { OpenStep::AbortFetch } else { OpenStep::Wait };
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

/// The origin of an absolute `http(s)` URL; `None` for anything else, a relative URL included.
pub fn url_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    origin(&format!("{scheme}://{}", rest.get(..end)?))
}

/// Whether `resolved` — the string an open got, already resolved against the app's location —
/// is a URL the app may fetch itself: an `http(s)` URL whose origin is exactly `embed`, the
/// `?embed=` origin (as [`allowed_origin`] writes it; `None` when the bridge is off, and then
/// nothing is a URL this build fetches). [`OPEN_URL`] for everything else: another origin,
/// another scheme, and no bridge at all — the same fixed refusal, decided before any fetch.
pub fn url_on_origin(resolved: &str, embed: Option<&str>) -> Result<(), String> {
    match (url_origin(resolved), embed) {
        (Some(origin), Some(embed)) if origin == embed => Ok(()),
        _ => Err(OPEN_URL.into()),
    }
}

/// A call from the host page.
#[derive(Debug, PartialEq)]
pub enum Call {
    /// Open the document in the message's `data` under `name`: bytes (an ArrayBuffer, a typed
    /// array, a Blob) or a same-origin URL string the app fetches itself. No `name`: a byte
    /// open is "document", a URL open takes the URL's own ([`file_name`]).
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

/// The Photopea-compatible config in the URL hash: `files` opened at start, and saves in one of
/// `server.formats` POSTed to `server.url` instead of downloaded.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct PhotopeaConfig {
    pub files: Vec<String>,
    pub server_url: Option<String>,
    pub formats: Vec<String>,
}

impl PhotopeaConfig {
    /// Reads the URL hash (without its `#`); `None` when it is empty or isn't a JSON object.
    pub fn from_hash(hash: &str) -> Option<Self> {
        let raw = hash.trim_start_matches('#');
        if raw.is_empty() {
            return None;
        }
        let decoded = if raw.starts_with('{') { raw.to_string() } else { percent_decode(raw) };
        let v: Value = serde_json::from_str(&decoded).ok().filter(Value::is_object)?;
        let strings = |v: Option<&Value>| -> Vec<String> {
            v.and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
        };
        Some(Self {
            files: strings(v.get("files")),
            server_url: v.pointer("/server/url").and_then(Value::as_str).map(str::to_string),
            formats: strings(v.pointer("/server/formats")).into_iter().map(|f| f.to_lowercase()).collect(),
        })
    }

    /// Why the Photopea-compatible mode is off in this build: its save queue holds one pending
    /// upload for the whole app, so saves of two open documents could be merged — one dropped for
    /// the other, or an older copy of one landing after its newer one. Our host drives the app
    /// same-origin through `window.photocraft` and the `?embed=` bridge, which don't touch that
    /// queue, so the mode is refused outright rather than left half-open: a JSON config in the
    /// URL hash is rejected with this one message (logged once by `embed::photopea_config`; the
    /// app then opens as usual). To switch the mode back on, first bind each pending upload to
    /// the document it saves, then take this gate out.
    pub const DISABLED: &str = "the Photopea-compatible mode is disabled in this build: a config in the URL hash is ignored (drive the app with window.photocraft on its own page, or the ?embed= postMessage bridge)";

    /// The config in the URL hash — **refused in this build** ([`Self::DISABLED`]). Without a
    /// config in the hash there is nothing to read and the result is `Ok(None)`.
    pub fn trusted(hash: &str, query: &str, parent_origin: Option<&str>) -> Result<Option<Self>, String> {
        if Self::from_hash(hash).is_some() {
            return Err(Self::DISABLED.into());
        }
        Self::trusted_inner(hash, query, parent_origin)
    }

    /// [`Self::trusted`]'s trust rules, which a re-enabled mode would apply again (kept for its
    /// tests): the config is honoured only on a page opened with `?embed=<origin>` in `query`
    /// inside a frame whose parent page is on that origin (`parent_origin`, `None` when the app
    /// isn't framed or the parent's origin can't be told): the config sends the user's saves to
    /// `server.url`, and a link anyone can send sets both the hash and `?embed=`, so only the
    /// browser's word on who embeds the app counts (photocraft#2263 review). `server.url` has to be
    /// on the embed origin, and each of `files` on it or on an origin listed in `?embed_files=`
    /// (comma-separated, percent-encoded). `Ok(None)` when the hash holds no config; `Err` says why
    /// one was refused.
    fn trusted_inner(hash: &str, query: &str, parent_origin: Option<&str>) -> Result<Option<Self>, String> {
        let Some(config) = Self::from_hash(hash) else { return Ok(None) };
        let embed = allowed_origin(query).ok_or("the Photopea-style config in the URL needs ?embed=<origin of the page that embeds PhotoCraft>")?;
        let parent = parent_origin.ok_or("the Photopea-style config in the URL is only read in a frame whose page is on the ?embed= origin, and the app isn't framed or its page's origin is unknown")?;
        if origin(parent).as_deref() != Some(embed.as_str()) {
            return Err(format!("the page embedding the app is on {parent}, not on the ?embed= origin {embed}"));
        }
        let mut file_origins = vec![embed.clone()];
        if let Some(list) = param(query, "embed_files") {
            for o in list.split(',').map(percent_decode) {
                file_origins.push(origin(&o).ok_or_else(|| format!("?embed_files= has {o:?}, which isn't an http(s) origin"))?);
            }
        }
        // Keep server.url on the embed origin even though the parent was checked: in Firefox the
        // parent's origin comes from the referrer, which a client-side redirect on that origin can
        // hand to a page framed by someone else.
        if let Some(url) = &config.server_url
            && url_origin(url).as_deref() != Some(embed.as_str())
        {
            return Err(format!("server.url {url} isn't on the embed origin {embed}"));
        }
        if let Some(url) = config.files.iter().find(|f| !url_origin(f).is_some_and(|o| file_origins.contains(&o))) {
            return Err(format!("{url} isn't on the embed origin or an ?embed_files= origin"));
        }
        Ok(Some(config))
    }

    /// Whether a save to `path` goes to the server rather than a download.
    pub fn saves(&self, path: &str) -> bool {
        self.server_url.is_some() && self.formats.iter().any(|f| *f == extension(path))
    }
}

/// The lower-case extension of `path` (empty when it has none).
pub fn extension(path: &str) -> String {
    std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// The file name a URL points at, without its query, percent-decoded; "document" when it has none.
pub fn file_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = percent_decode(path.rsplit('/').next().unwrap_or(path));
    if name.trim().is_empty() { "document".into() } else { name }
}

/// Photopea's save body: a JSON header `{"source","versions":[{"format","start","size"}]}` padded
/// with spaces to 2000 bytes, then the file.
pub fn save_body(source: &str, format: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    const HEADER_SIZE: usize = 2000;
    let header = json!({ "source": source, "versions": [{ "format": format, "start": 0, "size": bytes.len() }] }).to_string();
    if header.len() > HEADER_SIZE {
        return Err("the save header is longer than 2000 bytes".into());
    }
    let mut body = header.into_bytes();
    body.resize(HEADER_SIZE, b' ');
    body.extend_from_slice(bytes);
    Ok(body)
}

/// What to tell the page about a save the server answered: the text of an `app.echoToOE("…")` in
/// its JSON `script` (Photopea runs that script), else "saved" (2xx) or "error".
pub fn reply_message(ok: bool, reply: &str) -> String {
    let script = serde_json::from_str::<Value>(reply).ok().and_then(|v| v.get("script").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
    if let Some(start) = script.find("echoToOE(") {
        let rest = script.get(start + "echoToOE(".len()..).unwrap_or("");
        if let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'')
            && let Some(end) = rest.get(1..).and_then(|r| r.find(q))
        {
            return rest.get(1..1 + end).unwrap_or("").to_string();
        }
    }
    if ok { "saved".into() } else { "error".into() }
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
        assert_eq!(
            info(),
            json!({"app": env!("CARGO_PKG_VERSION"), "format": photocraft_format::FORMAT_VERSION, "doc": null}),
            "the key is always there, so a host can read info().doc without checking"
        );
        let doc = DocInfo { id: 7, name: "Poster.psd".into() };
        assert_eq!(
            info_with(Some(&doc)),
            json!({"app": env!("CARGO_PKG_VERSION"), "format": photocraft_format::FORMAT_VERSION, "doc": {"id": 7, "name": "Poster.psd"}})
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
        assert_eq!(landed_doc(&[1], "Poster.psd", &docs), Some((2, "Poster.psd".to_string())), "the new one with the asked-for name");
        assert_eq!(landed_doc(&[1], "no such name", &docs), None, "no fallback: a document of another name is never this open's");
        assert_eq!(landed_doc(&[1, 2, 3], "Poster.psd", &docs), None, "nothing new yet");
        assert_eq!(landed_doc(&[], "Poster.psd", &[]), None, "no documents at all");
        let named_late = [(1, "Old.psd".to_string()), (4, "other.psd".to_string()), (2, "Poster.psd".to_string())];
        assert_eq!(
            landed_doc(&[1], "Poster.psd", &named_late),
            Some((2, "Poster.psd".to_string())),
            "the named one wins over an earlier new one it doesn't name"
        );
        // The document of that name that was already there doesn't answer: only what is new
        // since this import started can.
        assert_eq!(landed_doc(&[1], "Poster.psd", &[(1, "Poster.psd".to_string())]), None, "the old same-named document isn't this open's");
    }

    #[test]
    fn two_opens_of_one_name_each_answer_their_own_document() {
        // Serialized: A settles before B is even taken (a second open while one is in flight is
        // refused OPEN_BUSY), so B's `before` holds everything A saw plus A's document — and
        // the duplicate the app opens under the same name for B answers B, never A's again.
        let first = [(1, "Old.psd".to_string()), (2, "a.psd".to_string())];
        assert_eq!(landed_doc(&[1], "a.psd", &first), Some((2, "a.psd".to_string())));
        let second = [(1, "Old.psd".to_string()), (2, "a.psd".to_string()), (4, "a.psd".to_string())];
        assert_eq!(landed_doc(&[1, 2], "a.psd", &second), Some((4, "a.psd".to_string())), "the new duplicate, not the first open's document a second time");
    }

    #[test]
    fn a_cancelled_open_and_a_renamed_template_never_claim_another_document() {
        // The reviewer's counter-example under the one-open design: A ("a.psd") waits behind a
        // cancelled dialog while a document of another name ("b.psd", B's would-be file)
        // appears — nothing lands it as A's; A rejects (at its deadline, or at once through
        // the prompt read) and "b.psd" stays in the session, nobody's answer. A `.psdt`,
        // which the app opens as "Untitled-N", is the same shape: never claimed.
        assert_eq!(landed_doc(&[], "a.psd", &[(9, "b.psd".to_string())]), None);
        assert_eq!(landed_doc(&[1], "card.psdt", &[(1, "Old.psd".to_string()), (2, "Untitled-1".to_string())]), None);
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
        // 2. bytes on their way: wait; at the deadline, settle the timeout (nothing ran).
        assert_eq!(open_step(&OpenPass { fetching: true, ..pass() }), OpenStep::Wait);
        assert_eq!(open_step(&OpenPass { fetching: true, timed_out: true, ..pass() }), OpenStep::AbortFetch);
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
        assert_eq!(landed_doc(&before, "a.psd", &docs), Some((5, "a.psd".to_string())), "B's own new document, never A's late one");
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
        assert_eq!(call(json!({"photocraft": "a", "method": "save"})), Some((json!("a"), Ok(Call::Save { format: "pcraft".into() }))));
        assert_eq!(call(json!({"photocraft": 2, "method": "open", "name": "x.psd"})), Some((json!(2), Ok(Call::Open { name: Some("x.psd".into()) }))));
        assert_eq!(
            call(json!({"photocraft": 2, "method": "open"})),
            Some((json!(2), Ok(Call::Open { name: None }))),
            "no name: a byte open is \"document\", a URL open derives its own from the URL"
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
        assert!(export_name("a.psd", "../x").is_err());
        assert!(export_name("a.psd", "").is_err());
    }

    #[test]
    fn the_photopea_config_comes_from_the_hash() {
        let raw = r#"{"files":["https://s.test/a.psd"],"server":{"url":"https://s.test/save","formats":["PSD"]}}"#;
        let encoded: String = raw.bytes().map(|b| format!("%{b:02X}")).collect();
        let c = PhotopeaConfig::from_hash(&format!("#{encoded}")).expect("a config");
        assert_eq!(c, PhotopeaConfig::from_hash(raw).expect("a config"), "encoded or not");
        assert_eq!(c.files, ["https://s.test/a.psd"]);
        assert!(c.saves("a.psd") && !c.saves("a.png"));
        assert_eq!(PhotopeaConfig::from_hash(""), None);
        assert_eq!(PhotopeaConfig::from_hash("#section"), None);
        assert!(!PhotopeaConfig::from_hash(r#"{"files":[]}"#).expect("a config").saves("a.psd"), "no server, no upload");
    }

    #[test]
    fn the_photopea_mode_is_refused_in_this_build() {
        let config = r#"{"files":["https://s.test/a.psd"],"server":{"url":"https://s.test/save","formats":["psd"]}}"#;
        for (hash, query, parent) in [
            (config, "embed=https%3A%2F%2Fs.test", Some("https://s.test")),
            (config, "", None),
            ("#%7B%22files%22%3A%5B%5D%7D", "embed=https://s.test", Some("https://s.test")),
        ] {
            assert_eq!(PhotopeaConfig::trusted(hash, query, parent), Err(PhotopeaConfig::DISABLED.into()), "{hash}");
        }
        assert_eq!(PhotopeaConfig::trusted("", "embed=https://s.test", Some("https://s.test")), Ok(None), "no config: nothing to refuse");
        assert_eq!(PhotopeaConfig::trusted("#section", "", None), Ok(None));
    }

    #[test]
    fn the_photopea_config_would_need_a_frame_on_the_embed_origin() {
        let config = |server: &str, files: &str| format!(r#"{{"files":[{files}],"server":{{"url":"{server}","formats":["psd"]}}}}"#);
        let ok = config("https://admin.test/save/1", r#""https://admin.test/a.psd""#);
        let embed = "embed=https%3A%2F%2Fadmin.test";
        let admin = Some("https://admin.test");
        assert!(PhotopeaConfig::trusted_inner(&ok, embed, admin).expect("trusted").is_some());
        assert_eq!(PhotopeaConfig::trusted_inner("", "", None), Ok(None), "no config, nothing to refuse");
        assert_eq!(PhotopeaConfig::trusted_inner("#section", "cpu", None), Ok(None));
        // The link from the review: a bare link can't send saves anywhere.
        let attack = config("https://attacker.example/s", "");
        assert!(PhotopeaConfig::trusted_inner(&attack, "", admin).is_err(), "no ?embed=");
        assert!(PhotopeaConfig::trusted_inner(&ok, "", admin).is_err(), "not even to a plausible server");
        assert!(PhotopeaConfig::trusted_inner(&attack, embed, admin).is_err(), "server on another origin");
        for server in
            ["https://admin.test.attacker.example/s", "https://admin.test@attacker.example/s", "http://admin.test/s", "/save/1", "javascript:alert(1)"]
        {
            assert!(PhotopeaConfig::trusted_inner(&config(server, ""), embed, admin).is_err(), "{server}");
        }
        assert!(PhotopeaConfig::trusted_inner(&config("https://ADMIN.test:443/save", ""), embed, admin).is_ok(), "the origin as a browser writes it");
        // A link sets ?embed= too: only the page that really frames the app counts (the CodeRabbit
        // link, opened as a page or framed by someone else).
        let theirs = config("https://attacker.example/s", "");
        let their_embed = "embed=https%3A%2F%2Fattacker.example";
        assert!(PhotopeaConfig::trusted_inner(&theirs, their_embed, None).is_err(), "not framed");
        assert!(PhotopeaConfig::trusted_inner(&ok, their_embed, admin).is_err(), "framed by another page than ?embed= names");
        assert!(PhotopeaConfig::trusted_inner(&ok, embed, None).is_err(), "not framed, even with plausible values");
        assert!(PhotopeaConfig::trusted_inner(&ok, embed, Some("https://admin.test.attacker.example")).is_err());
        assert!(PhotopeaConfig::trusted_inner(&ok, embed, Some("null")).is_err(), "an opaque origin");
        assert!(PhotopeaConfig::trusted_inner(&ok, embed, Some("https://ADMIN.test:443")).is_ok(), "written as the browser may write it");
        // Files: the embed origin, or one listed in ?embed_files=.
        let cdn = config("https://admin.test/save/1", r#""https://admin.test/a.psd","https://cdn.test/b.psd""#);
        assert!(PhotopeaConfig::trusted_inner(&cdn, embed, admin).is_err());
        assert!(PhotopeaConfig::trusted_inner(&cdn, &format!("{embed}&embed_files=https%3A%2F%2Fcdn.test"), admin).is_ok());
        assert!(PhotopeaConfig::trusted_inner(&cdn, &format!("{embed}&embed_files=https://other.test,https://cdn.test"), admin).is_ok());
        assert!(PhotopeaConfig::trusted_inner(&cdn, &format!("{embed}&embed_files=*"), admin).is_err());
        assert!(PhotopeaConfig::trusted_inner(&config("https://admin.test/save/1", r#""file:///etc/passwd""#), embed, admin).is_err());
        // No server: files still have to be trusted, since opening one posts its name to the page.
        assert!(PhotopeaConfig::trusted_inner(r#"{"files":["https://cdn.test/b.psd"]}"#, embed, admin).is_err());
    }

    #[test]
    fn the_url_form_is_the_embed_origins_alone() {
        // N5: a string open is the app fetching the URL itself, so the one origin it may fetch
        // from is named — the ?embed= origin — and nothing else: another origin, another
        // scheme, and no bridge at all each refuse the same fixed text, decided before any
        // fetch runs. The refusal texts are the host's contract, locked character for
        // character like the byte form's are.
        assert_eq!(
            OPEN_URL,
            "open couldn't take the string as a same-origin URL: it takes an http(s) URL on the ?embed= origin, resolved against the app's own location"
        );
        assert_eq!(OPEN_FETCH, "open couldn't fetch the URL: the fetch failed or the server didn't answer a 2xx status");
        assert_ne!(OPEN_URL, OPEN_FETCH);
        assert_ne!(OPEN_URL, OPEN_READ, "the string form and the byte form each have their own unreadable");
        assert_ne!(OPEN_FETCH, OPEN_TIMEOUT, "a fetch that failed is not a wait that gave up");
        let embed = Some("https://admin.test");
        assert_eq!(url_on_origin("https://admin.test/api/asset/3", embed), Ok(()), "the embed origin itself");
        assert_eq!(url_on_origin("https://admin.test/a/b.psd?x=1#f", embed), Ok(()), "path, query and hash are not the origin");
        assert_eq!(url_on_origin("https://ADMIN.test:443/a", embed), Ok(()), "as a browser writes the origin");
        for other in [
            "https://admin.test.attacker.example/a.psd",
            "https://attacker.example/admin.test/a.psd",
            "https://user@admin.test/a.psd",
            "http://admin.test/a.psd",
            "https://admin.test:8443/a.psd",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/plain,x",
            "/api/asset/3",
        ] {
            assert_eq!(url_on_origin(other, embed), Err(OPEN_URL.into()), "{other}");
        }
        assert_eq!(
            url_on_origin("https://admin.test/a", None),
            Err(OPEN_URL.into()),
            "no ?embed= origin: the URL form is the embedder's channel, nothing is fetched"
        );
    }

    #[test]
    fn a_url_string_rides_under_the_call_budget_before_it_is_copied() {
        // The window face has no message envelope to weigh and the bridge exempts `data`, so
        // the URL string bounds itself before it is copied into Rust: the same worst-unit
        // bound the budget walk charges a member, and a longer string is OPEN_URL without
        // ever being spelled out (10_923 units spell at 65_840, past the 64 KiB budget).
        assert!(call_string_cost(10_922).is_some_and(|cost| cost <= MAX_CALL_BYTES), "a URL of 10_922 units rides");
        assert!(call_string_cost(10_973).is_some_and(|cost| cost > MAX_CALL_BYTES), "a longer string is refused as a URL before the copy");
    }

    #[test]
    fn a_url_opens_under_the_name_its_url_points_at() {
        // N5's name default, the one-arg bridge.open(url) form: the URL's last path segment —
        // query and hash stripped, percent-decoded — with a trailing slash (a resolved href
        // always carries the path's /) leaving no segment and falling back to "document".
        assert_eq!(file_name("https://admin.test/api/asset/7"), "7");
        assert_eq!(file_name("https://admin.test/files/My%20File.psd?download=1"), "My File.psd");
        assert_eq!(file_name("https://admin.test/files/poster.psd#page=2"), "poster.psd");
        assert_eq!(file_name("https://admin.test/files/"), "document");
        assert_eq!(file_name("https://admin.test/"), "document", "an origin-only URL has no segment to name");
    }

    #[test]
    fn a_url_fetches_only_a_direct_2xx_answer_inside_the_byte_budget() {
        // P1-3: a redirect is never followed — `redirect: "error"` makes any 3xx fail the
        // fetch itself, and `redirected` on the answer is the belt under it (both refused
        // with the one OPEN_FETCH). P1-2's first half: a declared Content-Length over the
        // byte budget refuses OPEN_TOO_BIG *before the body is read*; a length that can't be
        // read decides nothing — the stream counts the real bytes.
        assert_eq!(response_gate(true, false, None), Ok(()), "a plain 2xx with no length");
        assert_eq!(response_gate(true, false, Some("24576000")), Ok(()));
        assert_eq!(response_gate(true, false, Some(&MAX_OPEN_BYTES.to_string())), Ok(()), "the limit itself");
        assert_eq!(response_gate(true, false, Some("67108865")), Err(OPEN_TOO_BIG.into()));
        assert_eq!(response_gate(true, false, Some(" 67108865 ")), Err(OPEN_TOO_BIG.into()), "whitespace is still a number");
        assert_eq!(response_gate(true, false, Some("68719476736")), Err(OPEN_TOO_BIG.into()), "64 GiB declared: refused before the read");
        for unreadable in ["", "abc", "-1", "1e9", "0x10", "999999999999999999999999"] {
            assert_eq!(response_gate(true, false, Some(unreadable)), Ok(()), "{unreadable}: not a length — the stream decides");
        }
        assert_eq!(response_gate(false, false, None), Err(OPEN_FETCH.into()), "an HTTP error status");
        assert_eq!(response_gate(false, true, None), Err(OPEN_FETCH.into()));
        // The redirect lock: even a 2xx that followed a redirect is refused — remove the
        // `redirected` term and this line goes red.
        assert_eq!(response_gate(true, true, None), Err(OPEN_FETCH.into()), "a followed redirect never reaches the body, however it answered");
        assert_eq!(response_gate(true, true, Some("12")), Err(OPEN_FETCH.into()), "the redirect check comes before the length");
    }

    #[test]
    fn a_url_stream_counts_its_bytes_and_refuses_at_the_crossing() {
        // P1-2's second half: the body is read as a stream whose running count refuses the
        // moment it crosses MAX_OPEN_BYTES — no Content-Length needed, a lying one can only
        // push the count to the crossing. A chunked body that never declares a length is
        // bounded by the count alone; the count never wraps.
        assert_eq!(response_gate(true, false, None), Ok(()), "no Content-Length does not exempt the body");
        let mut total = 0usize;
        for chunk in [0, 1, 1024, 65_536, 1024 * 1024] {
            total = stream_step(total, chunk).expect("under the budget");
        }
        assert_eq!(total, 1_115_137, "0+1+1024+65536+1048576");
        // Up to the limit exactly, one byte over: the crossing chunk refuses.
        let mut total = 0usize;
        while total + 65536 <= MAX_OPEN_BYTES {
            total = stream_step(total, 65536).expect("under the budget");
        }
        assert_eq!(total, MAX_OPEN_BYTES, "whole MiB chunks land exactly on the limit");
        assert_eq!(stream_step(total, 1), Err(OPEN_TOO_BIG.into()), "one byte past: refused at the crossing");
        assert_eq!(stream_step(0, MAX_OPEN_BYTES), Ok(MAX_OPEN_BYTES), "the limit itself in one chunk");
        assert_eq!(stream_step(0, MAX_OPEN_BYTES + 1), Err(OPEN_TOO_BIG.into()));
        assert_eq!(stream_step(usize::MAX, 1), Err(OPEN_TOO_BIG.into()), "the count overflows: refused, never wrapped");
        assert_eq!(stream_step(usize::MAX, usize::MAX), Err(OPEN_TOO_BIG.into()));
    }

    #[test]
    fn url_fetch_cancel_abort_and_redirect_calls_are_not_optional() {
        // The browser calls themselves are wasm-only, so the pure tests above cannot see a
        // deleted call. Lock the wiring in the actual embed.rs functions, scoped per function
        // and per settle arm: removing reader.cancel(), either abort.abort(), the deadline's
        // abort_source(), redirect: "error", or the fetch's signal must turn this test red.
        // Browser verification still covers what those calls do; this covers their presence.
        fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
            let (_, rest) = source.split_once(start).expect("function or settle arm start");
            rest.split_once(end).map(|(body, _)| body).expect("function or settle arm end")
        }

        let embed = include_str!("embed.rs");
        let fetch = between(embed, "async fn fetch_open", "\n/// Opens `data` as `name`.");
        assert!(fetch.contains("init.set_redirect(web_sys::RequestRedirect::Error);"), "every redirect must fail before it can be followed");
        assert!(fetch.contains("init.set_signal(Some(&abort.signal()));"), "the entry's AbortController must be attached to fetch");
        assert!(fetch.contains("let _ = reader.cancel();"), "the over-budget stream must be cancelled at the crossing");
        assert!(fetch.contains("abort.abort();"), "the over-budget fetch itself must be aborted too");
        let response_gate_block = between(fetch, "protocol::response_gate", "let Some(body) = resp.body()");
        assert!(response_gate_block.contains("abort.abort();"), "declared Content-Length over-limit must abort before returning, same as streaming over-limit");

        let settle = between(embed, "fn settle_opens", "\n/// Kills the transfer");
        let deadline_fetch = between(settle, "protocol::OpenStep::AbortFetch =>", "protocol::OpenStep::TimeoutPending =>");
        assert!(deadline_fetch.contains("abort_source(&p);"), "a fetch-phase deadline must abort before releasing the window");
        let deadline_import = between(settle, "protocol::OpenStep::TimeoutPending =>", "protocol::OpenStep::HoldWindow =>");
        assert!(deadline_import.contains("abort_source(&p);"), "entering terminate-or-observe must not leave a source transfer alive");

        let abort = between(embed, "fn abort_source", "\n/// Encodes the saves");
        assert!(abort.contains("source.borrow_mut().abort.take()"), "the shared cell must hand over its controller exactly once");
        assert!(abort.contains("abort.abort();"), "the deadline's controller must actually be aborted");
    }

    #[test]
    fn url_origins() {
        assert_eq!(url_origin("http://[::1]:8080/x").as_deref(), Some("http://[::1]:8080"));
        assert_eq!(url_origin("https://admin.test/login?next=/a").as_deref(), Some("https://admin.test"), "a referrer is a full URL");
        assert_eq!(url_origin("https://Admin.test:443/a/b?c#d").as_deref(), Some("https://admin.test"));
        assert_eq!(url_origin("http://localhost:8080").as_deref(), Some("http://localhost:8080"));
        assert_eq!(url_origin("https://a.test?x").as_deref(), Some("https://a.test"));
        assert_eq!(url_origin("https://user@a.test/"), None);
        assert_eq!(url_origin("/relative"), None);
        assert_eq!(url_origin("data:text/plain,x"), None);
    }

    #[test]
    fn photopeas_save_body_and_reply() {
        let body = save_body("https://s.test/a.psd", "psd", b"8BPS").expect("a body");
        assert_eq!(body.len(), 2004);
        let header: Value = serde_json::from_slice(body.get(..2000).unwrap_or_default().trim_ascii_end()).expect("JSON header");
        assert_eq!(header, json!({"source": "https://s.test/a.psd", "versions": [{"format": "psd", "start": 0, "size": 4}]}));
        assert_eq!(body.get(2000..), Some(&b"8BPS"[..]));
        assert!(save_body(&"x".repeat(2000), "psd", b"").is_err());
        assert_eq!(reply_message(true, r#"{"script":"app.echoToOE(\"stored 12\")"}"#), "stored 12");
        assert_eq!(reply_message(true, r#"{"script":"app.echoToOE('ok')"}"#), "ok");
        assert_eq!(reply_message(true, "not json"), "saved");
        assert_eq!(reply_message(false, ""), "error");
        assert_eq!(file_name("https://s.test/dir/My%20File.psd?v=2"), "My File.psd");
        assert_eq!(file_name("https://s.test/"), "document");
    }
}
