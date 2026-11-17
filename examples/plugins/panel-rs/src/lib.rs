//! Document Info: an example PhotoCraft panel plug-in (plug-in ABI v2).
//!
//! `no_std`, no imports, no allocator. The host sends JSON events into a block it takes from our
//! `pc_alloc`, and we answer with a JSON array of operations: build a view, set values, raise
//! notices, ask the host about the document, arm a timer, keep something in plug-in storage.
//!
//! It is small, but not a toy: it reads the document on demand (`doc.info`), follows the document as
//! it changes instead of polling (`doc.changed`), remembers a note between sessions (`storage.set`),
//! runs a timer while its Watch button is on, and *acts* on the document by asking the host to run an
//! engine command (`doc.command`, the New Type Layer button) — the five things a real panel does, and
//! the whole of what the ABI asks of one.
//!
//! Everything here is deliberately plain: a fixed heap, a fixed reply buffer, and a hand-rolled
//! scanner for the handful of fields it reads. A real plug-in in a language with an allocator and a
//! JSON parser would use those instead; nothing in the ABI prefers this.

#![no_std]

use core::cell::UnsafeCell;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

const MANIFEST: &str = r#"{
  "id": "org.photocraft.example.panel",
  "name": "Document Info (WebAssembly)",
  "version": "1.0.0",
  "kind": "panel",
  "author": "PhotoCraft contributors",
  "description": "An example panel plug-in: reads the document, adds a type layer, keeps a note, and watches for changes.",
  "panel": { "title": "Document Info", "minWidth": 260 },
  "capabilities": { "document": ["read", "addLayers"], "storage": true }
}"#;

/// The reply buffer: every answer is written here and handed back as `len << 32 | ptr`.
const OUT_CAP: usize = 8 * 1024;
/// The heap `pc_alloc` hands blocks out of. No `memory.grow`: a panel that grows its memory on every
/// event would creep towards the host's cap for no reason.
const HEAP_CAP: usize = 64 * 1024;
/// Longest note kept, as it arrived from the host.
const NOTE_CAP: usize = 256;
/// Longest status line. Only ASCII without quotes is ever written here, so it can go into the view
/// as a JSON string without escaping.
const STATUS_CAP: usize = 96;

/// A single-instance cell: one plug-in instance is one wasm instance, driven by one host thread, and
/// the host never calls back in while `pc_event` runs — so plain interior mutability is enough.
/// `Sync` is required only because the cells below are `static`.
struct Cell<T>(UnsafeCell<T>);

// SAFETY: see `Cell`: nothing here is reachable from more than one thread, and access is never
// re-entrant.
unsafe impl<T> Sync for Cell<T> {}

struct State {
    /// The note, kept exactly as the host sent it: the *content* of a JSON string, escapes and all,
    /// so it can be written straight back into a reply without being re-escaped.
    note: [u8; NOTE_CAP],
    note_len: usize,
    /// What the panel says about itself, built by us (see `STATUS_CAP`).
    status: [u8; STATUS_CAP],
    status_len: usize,
    /// The Watch button: while it is on, a timer fires once a second and re-arms itself.
    watching: bool,
    ticks: u32,
    reads: u32,
}

impl State {
    const fn new() -> State {
        State { note: [0; NOTE_CAP], note_len: 0, status: [0; STATUS_CAP], status_len: 0, watching: false, ticks: 0, reads: 0 }
    }

    /// Writes the status line: `prefix`, then "`<w> x <h>`" when dimensions are given.
    fn status(&mut self, prefix: &str, dims: Option<(u64, u64)>) {
        let mut out = [0u8; STATUS_CAP];
        let mut n = 0;
        push_into(&mut out, &mut n, prefix.as_bytes());
        if let Some((w, h)) = dims {
            push_into(&mut out, &mut n, b" ");
            push_num(&mut out, &mut n, w);
            push_into(&mut out, &mut n, b" x ");
            push_num(&mut out, &mut n, h);
        }
        self.status = out;
        self.status_len = n;
    }

    /// The status line with `tail` (ASCII we did not build, like a layer id) after the prefix.
    fn status_tail(&mut self, prefix: &str, tail: &[u8]) {
        let mut out = [0u8; STATUS_CAP];
        let mut n = 0;
        push_into(&mut out, &mut n, prefix.as_bytes());
        push_into(&mut out, &mut n, b" ");
        push_into(&mut out, &mut n, tail);
        self.status = out;
        self.status_len = n;
    }

    /// Takes the note from an event. A note longer than we can hold is refused rather than truncated:
    /// half of a JSON string is not a string.
    fn set_note(&mut self, content: &[u8]) {
        if content.len() <= NOTE_CAP {
            self.note = [0; NOTE_CAP];
            self.note[..content.len()].copy_from_slice(content);
            self.note_len = content.len();
            // An empty note is no note: the reply sends a `null` value, which the host reads as a
            // delete, and saying otherwise would be a lie.
            self.status(if content.is_empty() { "Note cleared" } else { "Note saved to plug-in storage" }, None);
        } else {
            self.status("That note is too long to keep", None);
        }
    }
}

static STATE: Cell<State> = Cell(UnsafeCell::new(State::new()));

struct Heap {
    bytes: [u8; HEAP_CAP],
    cursor: usize,
}

static HEAP: Cell<Heap> = Cell(UnsafeCell::new(Heap { bytes: [0; HEAP_CAP], cursor: 0 }));
static OUT: Cell<[u8; OUT_CAP]> = Cell(UnsafeCell::new([0; OUT_CAP]));

#[unsafe(no_mangle)]
pub extern "C" fn pc_abi_version() -> u32 {
    2
}

/// `len << 32 | ptr` of the UTF-8 manifest JSON.
#[unsafe(no_mangle)]
pub extern "C" fn pc_manifest() -> u64 {
    ((MANIFEST.len() as u64) << 32) | MANIFEST.as_ptr() as u64
}

/// A block of `size` bytes for the host to write an event into, 8-byte aligned, or 0 when the heap is
/// full (the host treats 0 as out of memory).
#[unsafe(no_mangle)]
pub extern "C" fn pc_alloc(size: u32) -> u32 {
    // SAFETY: `Cell`'s contract: one instance, one thread, no re-entry.
    let heap = unsafe { &mut *HEAP.0.get() };
    let start = (heap.cursor + 7) & !7;
    let end = start + size as usize;
    if end > HEAP_CAP {
        return 0;
    }
    heap.cursor = end;
    (heap.bytes.as_ptr() as usize + start) as u32
}

/// Gives a block back. Only the block at the top of the heap can be released — which is every block
/// the host frees, since it allocates one event block per call and frees it before the next. Pointers
/// outside the heap (the static reply buffer) are ignored.
#[unsafe(no_mangle)]
pub extern "C" fn pc_free(ptr: u32, size: u32) {
    // SAFETY: as in `pc_alloc`.
    let heap = unsafe { &mut *HEAP.0.get() };
    let base = heap.bytes.as_ptr() as usize;
    let offset = (ptr as usize).wrapping_sub(base);
    if offset <= HEAP_CAP && offset + size as usize == heap.cursor {
        heap.cursor = offset;
    }
}

/// Answers one event: `len << 32 | ptr` of the operations JSON, or 0 for "no operations".
#[unsafe(no_mangle)]
pub extern "C" fn pc_event(ptr: u32, len: u32) -> i64 {
    // SAFETY: the host passes a block it got from `pc_alloc`, holding `len` bytes it wrote.
    let event = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
    // SAFETY: as in `pc_alloc`.
    let out = unsafe { &mut *OUT.0.get() };
    let n = reply(event, out);
    if n == 0 {
        return 0;
    }
    ((n as u64) << 32 | out.as_ptr() as u64) as i64
}

/// The panel: read the event, update what it remembers, and write the operations that say so.
fn reply(event: &[u8], out: &mut [u8; OUT_CAP]) -> usize {
    // SAFETY: as in `pc_alloc`.
    let st = unsafe { &mut *STATE.0.get() };
    let mut ask_for_document = false;
    let mut make_text = false;
    let mut arm_timer: Option<bool> = None;
    let mut save_note = false;
    let mut refused: Option<&[u8]> = None;
    match top_str(event, b"type").unwrap_or_default() {
        b"init" => {
            // Storage comes with `init`: pick the note back up where the last session left it.
            if let Some(note) = top_field(event, b"storage").and_then(|s| top_str(s, b"note")) {
                if note.len() <= NOTE_CAP {
                    st.note = [0; NOTE_CAP];
                    st.note[..note.len()].copy_from_slice(note);
                    st.note_len = note.len();
                }
            }
            st.status("Ready", None);
        }
        b"ui.click" => match top_str(event, b"id").unwrap_or_default() {
            b"read" => {
                ask_for_document = true;
                st.reads += 1;
                st.status("Asking the host for the document", None);
            }
            b"watch" => {
                st.watching = !st.watching;
                arm_timer = Some(st.watching);
                st.status(if st.watching { "Watching the document" } else { "Not watching" }, None);
            }
            b"text" => {
                // The host runs this for us and answers with what the command returned. Which
                // commands a panel may run is the *host's* list, intersected with the capability
                // above: `init.grants.commands` is what this panel actually has.
                make_text = true;
                st.status("Asking the host for a type layer", None);
            }
            b"save" => {
                save_note = true;
                st.status(if st.note_len == 0 { "Note cleared" } else { "Note saved to plug-in storage" }, None);
            }
            _ => {}
        },
        b"ui.change" => {
            if top_str(event, b"id") == Some(&b"note"[..]) {
                if let Some(value) = top_str(event, b"value") {
                    st.set_note(value);
                    save_note = true;
                }
            }
        }
        b"result" => match top_str(event, b"id").unwrap_or_default() {
            b"info" => {
                if top_bool(event, b"ok") == Some(true) {
                    let w = top_u64(event, b"width").unwrap_or(0);
                    let h = top_u64(event, b"height").unwrap_or(0);
                    st.status("Document", Some((w, h)));
                } else {
                    st.status("The host would not read the document", None);
                    refused = top_str(event, b"error");
                }
            }
            // The answer to `doc.command`: the command's own result, or the reason it was refused.
            b"made" => match (top_bool(event, b"ok"), top_u64(event, b"layer")) {
                (Some(true), Some(layer)) => {
                    let mut id = [0u8; 20];
                    let mut n = 0;
                    push_num(&mut id, &mut n, layer);
                    st.status_tail("Type layer", &id[..n]);
                }
                _ => {
                    st.status("The host would not add a type layer", None);
                    refused = top_str(event, b"error");
                }
            },
            // A timer is one-shot: the tick that just fired re-arms the next one.
            b"tick" => {
                st.ticks += 1;
                if st.watching {
                    arm_timer = Some(true);
                }
            }
            _ => {}
        },
        // The document changed, or was closed (`doc` is null then). No polling anywhere.
        b"doc.changed" => match top_field(event, b"doc") {
            None | Some(b"null") => st.status("No document", None),
            Some(doc) => {
                let w = top_u64(doc, b"width").unwrap_or(0);
                let h = top_u64(doc, b"height").unwrap_or(0);
                st.status("Document", Some((w, h)));
            }
        },
        _ => {}
    }
    let mut w = W::new(out);
    w.view(st);
    if ask_for_document {
        w.op(r#"{"op":"doc.info","id":"info"}"#);
    }
    if make_text {
        w.op(r#"{"op":"doc.command","id":"made","command":"type.create","params":{"x":40,"y":60,"text":"Balloon","size":24}}"#);
    }
    match arm_timer {
        Some(true) => w.op(r#"{"op":"timer.set","id":"tick","ms":1000}"#),
        Some(false) => w.op(r#"{"op":"timer.clear","id":"tick"}"#),
        None => {}
    }
    if save_note {
        if st.note_len == 0 {
            // An empty note is no note: `null` is the ABI's delete, so nothing is left for the next
            // instance to pick up.
            w.op(r#"{"op":"storage.set","key":"note","value":null}"#);
        } else {
            // The note is already JSON string *content*, so it goes back in as it came.
            w.op(r#"{"op":"storage.set","key":"note","value":""#);
            w.push(&st.note[..st.note_len]);
            w.push(r#""}"#);
        }
    }
    if let Some(error) = refused {
        w.notice("warning", error);
    }
    w.finish()
}

/// Writes the operations array — the whole answer to one event.
struct W<'a> {
    buf: &'a mut [u8],
    n: usize,
    /// The buffer ran out: the reply is no longer valid JSON.
    full: bool,
    /// Whether the next operation is the first one, which takes no comma.
    first: bool,
}

impl<'a> W<'a> {
    fn new(buf: &'a mut [u8]) -> W<'a> {
        let mut w = W { buf, n: 0, full: false, first: true };
        w.push(b"[");
        w
    }

    /// Closes the array and gives its length — or 0, which is always allowed and means "no
    /// operations": a truncated reply is not JSON, and the host would rightly reject the batch.
    fn finish(&mut self) -> usize {
        self.push(b"]");
        if self.full { 0 } else { self.n }
    }

    fn push(&mut self, bytes: impl AsRef<[u8]>) {
        let bytes = bytes.as_ref();
        if self.n + bytes.len() > self.buf.len() {
            self.full = true;
            return;
        }
        self.buf[self.n..self.n + bytes.len()].copy_from_slice(bytes);
        self.n += bytes.len();
    }

    fn op(&mut self, text: &str) {
        if self.first {
            self.first = false;
        } else {
            self.push(b",");
        }
        self.push(text.as_bytes());
    }

    /// A decimal number, for the parts of the view that carry one.
    fn num(&mut self, v: u64) {
        let mut tmp = [0u8; 20];
        let mut n = 0;
        push_num(&mut tmp, &mut n, v);
        self.push(&tmp[..n]);
    }

    fn notice(&mut self, level: &str, text: &[u8]) {
        self.op(r#"{"op":"ui.notice","level":""#);
        self.push(level.as_bytes());
        self.push(r#"","text":""#);
        self.push(text);
        self.push(r#""}"#);
    }

    /// The whole view, rebuilt on every event: the host keeps no model of its own, so this is how a
    /// panel updates a label.
    fn view(&mut self, st: &State) {
        self.op(r#"{"op":"ui.view","view":{"type":"column","children":["#);
        self.push(r#"{"type":"label","text":""#);
        self.push(&st.status[..st.status_len]);
        self.push(r#"","level":"info"}"#);
        self.push(r#",{"type":"text","id":"note","value":""#);
        self.push(&st.note[..st.note_len]);
        self.push(r#"","placeholder":"A note, kept between sessions"}"#);
        self.push(r#",{"type":"row","children":["#);
        self.push(r#"{"type":"button","id":"read","label":"Read Document","primary":true}"#);
        self.push(r#",{"type":"button","id":"watch","label":""#);
        let label: &[u8] = if st.watching { b"Stop Watching" } else { b"Watch Document" };
        self.push(label);
        self.push(r#""}"#);
        self.push(r#",{"type":"button","id":"save","label":"Save Note"}]}"#);
        self.push(r#",{"type":"separator"}"#);
        self.push(r#",{"type":"label","text":"Reads "#);
        self.num(st.reads as u64);
        self.push(b", ticks ");
        self.num(st.ticks as u64);
        self.push(r#"","level":"muted"}"#);
        self.push(r#",{"type":"label","text":"An example ABI v2 panel: this view is JSON this plug-in sent.","level":"muted"}"#);
        // A row of its own: four buttons side by side would be wider than this panel is.
        self.push(r#",{"type":"row","children":[{"type":"button","id":"text","label":"New Type Layer"}]}"#);
        self.push(r#"]}}"#);
    }
}

fn push_into(out: &mut [u8], n: &mut usize, bytes: &[u8]) {
    let take = bytes.len().min(out.len() - *n);
    out[*n..*n + take].copy_from_slice(&bytes[..take]);
    *n += take;
}

fn push_num(out: &mut [u8], n: &mut usize, v: u64) {
    let mut tmp = [0u8; 20];
    let mut i = tmp.len();
    let mut v = v;
    loop {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    push_into(out, n, &tmp[i..]);
}

// ---------------------------------------------------------------- reading events

/// The raw JSON value of `key` in the event's *own* object — nested objects and arrays are skipped
/// whole, so a layer's `"width"` is never mistaken for the document's.
fn top_field<'a>(json: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut i = 0;
    let mut depth = 0usize;
    while i < json.len() {
        match json[i] {
            b'"' => {
                let start = i + 1;
                let end = end_of_string(json, start)?;
                if depth == 1 && &json[start..end] == key {
                    let mut j = end + 1;
                    while j < json.len() && (json[j] == b' ' || json[j] == b':') {
                        j += 1;
                    }
                    let value_end = end_of_value(json, j)?;
                    return Some(&json[j..value_end]);
                }
                i = end + 1;
            }
            b'{' | b'[' => {
                depth += 1;
                i += 1;
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// The content of a string value: `top_field` without its quotes.
fn top_str<'a>(json: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let raw = top_field(json, key)?;
    let inner = raw.strip_prefix(b"\"")?;
    inner.strip_suffix(b"\"")
}

fn top_u64(json: &[u8], key: &[u8]) -> Option<u64> {
    let raw = top_field(json, key)?;
    let mut v: u64 = 0;
    let mut any = false;
    for &b in raw {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add(u64::from(b - b'0'))?;
        any = true;
    }
    any.then_some(v)
}

fn top_bool(json: &[u8], key: &[u8]) -> Option<bool> {
    match top_field(json, key)? {
        b"true" => Some(true),
        b"false" => Some(false),
        _ => None,
    }
}

/// The index of the `"` that closes the string whose content starts at `start`.
fn end_of_string(json: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i < json.len() {
        match json[i] {
            b'\\' => i += 2,
            b'"' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// The index just past the value starting at `start` (a string, an object, an array, or a scalar).
fn end_of_value(json: &[u8], start: usize) -> Option<usize> {
    match json.get(start)? {
        b'"' => end_of_string(json, start + 1).map(|e| e + 1),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut i = start;
            while i < json.len() {
                match json[i] {
                    b'"' => i = end_of_string(json, i + 1)?,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            None
        }
        _ => {
            let mut i = start;
            while i < json.len() && !matches!(json[i], b',' | b'}' | b']') {
                i += 1;
            }
            Some(i)
        }
    }
}
