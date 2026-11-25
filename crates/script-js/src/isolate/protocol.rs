//! The messages between the app and a script host, their framing, and the checks the app makes
//! on everything a host sends.
//!
//! A frame is a 4-byte little-endian length followed by that many bytes of JSON. Each direction
//! has a hard cap; a reader rejects a longer frame from its length alone, before allocating.
//! The host side is untrusted: a frame from it that is too long, malformed, out of order or asks
//! for anything outside [`COMMANDS`] ends the run.

use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ModalKind;
use crate::value::MAX_TEXT_UNITS;

/// Largest frame the app accepts from a host, in encoded bytes. This is a different bound from
/// the one on a dialog's arguments, which counts their strings' UTF-16 units before conversion
/// (at most 16 Mi): encoded, a unit takes one byte or, escaped, up to six, so arguments within
/// that bound may still make a frame past this one. Such a dialog never reaches the app, and
/// nothing waits for an answer: the host checks a frame's encoded size before it sends any, and
/// the run ends with the reason.
pub const MAX_FROM_HOST: usize = 64 << 20;
/// Largest frame a host accepts from the app (the source, or a command result).
pub const MAX_TO_HOST: usize = 256 << 20;
/// Longest output line the host sends; longer lines are cut and marked.
pub const MAX_LINE: usize = 1 << 20;
/// Longest failure message the app keeps from a host.
pub const MAX_MESSAGE: usize = 64 << 10;

/// App to host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToHost {
    /// The first message: run this script.
    Start {
        source: String,
        /// `app.name` for this run (`None` keeps the default).
        app_name: Option<String>,
        /// File name shown in script error messages (`None` uses `"script"`).
        #[serde(default)]
        script_name: Option<String>,
        /// Bytes of memory the host may map (where the platform enforces it in-process).
        memory_limit: u64,
    },
    /// The result of the last [`FromHost::Execute`].
    Executed { result: Result<Value, String> },
    /// The answer to the last [`FromHost::Dialog`].
    Dialog { answer: Value },
}

/// Host to app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromHost {
    /// Run a session command; the host waits for [`ToHost::Executed`].
    Execute { id: String, params: Value },
    /// A line of script output. Needs no reply.
    Output { line: String },
    /// Show a dialog; the host waits for [`ToHost::Dialog`].
    Dialog { kind: DialogKind, args: Value },
    /// The run ended: `None` when the script finished, or the message for the user.
    Finished { error: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
}

impl From<ModalKind> for DialogKind {
    fn from(k: ModalKind) -> Self {
        match k {
            ModalKind::Alert => Self::Alert,
            ModalKind::Confirm => Self::Confirm,
            ModalKind::Prompt => Self::Prompt,
        }
    }
}

impl From<DialogKind> for ModalKind {
    fn from(k: DialogKind) -> Self {
        match k {
            DialogKind::Alert => Self::Alert,
            DialogKind::Confirm => Self::Confirm,
            DialogKind::Prompt => Self::Prompt,
        }
    }
}

#[derive(Debug)]
pub enum FrameError {
    Io(io::Error),
    /// The stream ended inside a frame.
    Truncated,
    /// The length prefix is over the cap.
    TooLarge(usize),
    /// A message whose encoding passed the cap (given here) before it was finished.
    OverCap(usize),
    /// A message within the byte cap that holds more values than the reader takes
    /// ([`MAX_FRAME_VALUES`]).
    TooManyValues,
    /// A message nested deeper than the reader parses ([`MAX_FRAME_DEPTH`]).
    TooDeep,
    /// Not a message of the expected type.
    Malformed(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Truncated => f.write_str("the message was cut off"),
            Self::TooLarge(n) => write!(f, "a message of {n} bytes is over the limit"),
            Self::OverCap(cap) => write!(f, "a message is over the limit of {cap} bytes"),
            Self::TooManyValues => write!(f, "a message holds more than {MAX_FRAME_VALUES} values"),
            Self::TooDeep => write!(f, "a message is nested more than {MAX_FRAME_DEPTH} deep"),
            Self::Malformed(e) => write!(f, "a malformed message ({e})"),
        }
    }
}

/// A buffer that takes at most `cap` bytes; a write past them fails and keeps nothing more.
struct Capped {
    body: Vec<u8>,
    cap: usize,
}

impl Write for Capped {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.body.len().saturating_add(data.len()) > self.cap {
            return Err(std::io::Error::other("over the cap"));
        }
        self.body.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Write one frame. A message over `cap` is not written.
///
/// # Errors
/// What [`encode_frame`] and [`send_frame`] return. Nothing is written when encoding fails.
pub fn write_frame<W: Write, T: Serialize>(w: &mut W, msg: &T, cap: usize) -> Result<(), FrameError> {
    send_frame(w, &encode_frame(msg, cap)?)
}

/// Encode one message as a frame's body, checked against the reader's limits.
///
/// # Errors
/// [`FrameError::OverCap`] over the cap (the encoding stops there), [`FrameError::TooManyValues`]
/// with more values than the reader takes ([`MAX_FRAME_VALUES`]), [`FrameError::TooDeep`] nested
/// deeper than it parses ([`MAX_FRAME_DEPTH`]).
pub fn encode_frame<T: Serialize>(msg: &T, cap: usize) -> Result<Vec<u8>, FrameError> {
    // Encoded into a buffer that refuses to grow past the cap: escaping can make a message's
    // encoding several times the size of its text, and that is never held in memory.
    let mut capped = Capped { body: Vec::new(), cap };
    if let Err(e) = serde_json::to_writer(&mut capped, msg) {
        return Err(if e.is_io() { FrameError::OverCap(cap) } else { FrameError::Malformed(e.to_string()) });
    }
    let body = capped.body;
    // The reader's other limits, checked here: a frame it would refuse ends its side of the link.
    if values_in(&body) > MAX_FRAME_VALUES {
        return Err(FrameError::TooManyValues);
    }
    if too_deep(&body) {
        return Err(FrameError::TooDeep);
    }
    Ok(body)
}

/// Write an encoded body as one frame: its length, then the body.
///
/// # Errors
/// The stream's error, or a body too long for the length prefix.
pub fn send_frame<W: Write>(w: &mut W, body: &[u8]) -> Result<(), FrameError> {
    let len = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge(body.len()))?;
    w.write_all(&len.to_le_bytes()).map_err(FrameError::Io)?;
    w.write_all(body).map_err(FrameError::Io)?;
    w.flush().map_err(FrameError::Io)
}

/// Read one frame. `Ok(None)` when the stream ended cleanly between frames.
///
/// # Errors
/// Over the cap (checked before allocating), cut off, malformed, or the stream's error.
pub fn read_frame<R: Read, T: DeserializeOwned>(r: &mut R, cap: usize) -> Result<Option<T>, FrameError> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < len.len() {
        let Some(rest) = len.get_mut(got..) else { break };
        match r.read(rest) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(FrameError::Io(e)),
        }
    }
    let len = usize::try_from(u32::from_le_bytes(len)).map_err(|_| FrameError::TooLarge(usize::MAX))?;
    if len > cap {
        return Err(FrameError::TooLarge(len));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).map_err(|e| if e.kind() == io::ErrorKind::UnexpectedEof { FrameError::Truncated } else { FrameError::Io(e) })?;
    // Parsed values take far more memory than their text when they are tiny (`[0,0,0,…]`): a
    // frame within the size cap could still need gigabytes. Count them before parsing.
    if values_in(&body) > MAX_FRAME_VALUES {
        return Err(FrameError::Malformed(format!("more than {MAX_FRAME_VALUES} values in one message")));
    }
    serde_json::from_slice(&body).map(Some).map_err(|e| FrameError::Malformed(e.to_string()))
}

/// Most JSON values one message may hold: far more than any command reply or dialog a script
/// can make.
pub const MAX_FRAME_VALUES: usize = 1 << 20;

/// About how many JSON values `body` holds: the structural characters outside strings, which
/// open or separate values. Stops counting just past [`MAX_FRAME_VALUES`].
pub(crate) fn values_in(body: &[u8]) -> usize {
    let (mut n, mut in_string, mut escaped) = (1usize, false, false);
    for &b in body {
        if in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
        } else {
            match b {
                b'"' => in_string = true,
                b'{' | b'[' | b',' | b':' => {
                    n += 1;
                    if n > MAX_FRAME_VALUES {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    n
}

/// Deepest nesting of arrays and objects a message may have: the reader's JSON parser refuses
/// deeper ones (its limit is 128 levels, and it counts the one it refuses).
pub const MAX_FRAME_DEPTH: usize = 127;

/// Whether `body` nests arrays and objects deeper than [`MAX_FRAME_DEPTH`], counting the
/// brackets outside strings. Stops at the first level past it.
pub(crate) fn too_deep(body: &[u8]) -> bool {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for &b in body {
        if in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
        } else {
            match b {
                b'"' => in_string = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > MAX_FRAME_DEPTH {
                        return true;
                    }
                }
                b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    false
}

pub use crate::commands::{COMMANDS, Param, check_command, cut};

/// Check a host's dialog arguments. They must look exactly as the script globals build them in
/// process: a list of strings (`String(value)` for each argument), each at most
/// [`MAX_TEXT_UNITS`] UTF-16 units. The frame cap ([`MAX_FROM_HOST`]) bounds their total.
///
/// # Errors
/// What is wrong with them, for the failure message.
pub fn check_dialog_args(kind: DialogKind, args: &Value) -> Result<(), String> {
    match kind {
        DialogKind::Alert | DialogKind::Confirm | DialogKind::Prompt => {
            let Value::Array(items) = args else { return Err("dialog arguments that are not a list".into()) };
            for (i, item) in items.iter().enumerate() {
                let Value::String(text) = item else { return Err(format!("dialog argument {i} is not text")) };
                check_text(text).map_err(|e| format!("dialog argument {i} {e}"))?;
            }
            Ok(())
        }
    }
}

fn check_text(text: &str) -> Result<(), String> {
    // A UTF-8 string has at least as many bytes as UTF-16 units, so only long ones are counted.
    if text.len() > MAX_TEXT_UNITS && text.encode_utf16().count() > MAX_TEXT_UNITS {
        return Err(format!("is longer than {MAX_TEXT_UNITS} UTF-16 units"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_round_trip_and_stop_at_a_clean_end() {
        let mut buf = Vec::new();
        let msg = FromHost::Output { line: "héllo".into() };
        write_frame(&mut buf, &msg, MAX_FROM_HOST).unwrap();
        write_frame(&mut buf, &FromHost::Finished { error: None }, MAX_FROM_HOST).unwrap();
        let mut r = buf.as_slice();
        assert_eq!(read_frame::<_, FromHost>(&mut r, MAX_FROM_HOST).unwrap(), Some(msg));
        assert_eq!(read_frame::<_, FromHost>(&mut r, MAX_FROM_HOST).unwrap(), Some(FromHost::Finished { error: None }));
        assert!(read_frame::<_, FromHost>(&mut r, MAX_FROM_HOST).unwrap().is_none());
    }

    #[test]
    fn oversize_cut_off_and_malformed_frames_are_rejected() {
        // A 4 GiB length is refused from the prefix alone.
        let mut r: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, b'{'];
        assert!(matches!(read_frame::<_, FromHost>(&mut r, MAX_FROM_HOST), Err(FrameError::TooLarge(_))));
        let mut r: &[u8] = &[10, 0, 0, 0, b'{'];
        assert!(matches!(read_frame::<_, FromHost>(&mut r, MAX_FROM_HOST), Err(FrameError::Truncated)));
        let mut r: &[u8] = &[2, 0];
        assert!(matches!(read_frame::<_, FromHost>(&mut r, MAX_FROM_HOST), Err(FrameError::Truncated)));
        let mut r: &[u8] = &[2, 0, 0, 0, b'{', b'}'];
        assert!(matches!(read_frame::<_, FromHost>(&mut r, MAX_FROM_HOST), Err(FrameError::Malformed(_))));
        let mut buf = Vec::new();
        assert!(matches!(write_frame(&mut buf, &FromHost::Output { line: "x".repeat(64) }, 16), Err(FrameError::OverCap(16))));
        assert!(buf.is_empty(), "nothing is written for an oversize message");
    }

    #[test]
    fn dialog_arguments_must_be_a_list_of_bounded_strings() {
        assert!(check_dialog_args(DialogKind::Alert, &json!([])).is_ok());
        assert!(check_dialog_args(DialogKind::Alert, &json!(["Name", "x"])).is_ok());
        assert!(check_dialog_args(DialogKind::Alert, &json!(["é".repeat(MAX_TEXT_UNITS)])).is_ok(), "two bytes but one unit per character");
        let mut nested = json!("deep");
        for _ in 0..100 {
            nested = json!([nested]);
        }
        for args in [
            json!(null),
            json!("text"),
            json!({"0": "a"}),
            json!([1]),
            json!([null]),
            json!(["a", {"b": "c"}]),
            json!([nested]),
            json!(["x".repeat(MAX_TEXT_UNITS + 1)]),
        ] {
            let shown = args.to_string();
            assert!(check_dialog_args(DialogKind::Alert, &args).is_err(), "{}", cut(&shown, 80));
        }
    }

    #[test]
    fn a_message_of_many_tiny_values_is_refused_before_it_is_parsed() {
        let many = format!("{{\"Output\":{{\"line\":\"x\",\"extra\":[{}0]}}}}", "0,".repeat(MAX_FRAME_VALUES));
        let mut buf = Vec::new();
        buf.extend_from_slice(&u32::try_from(many.len()).unwrap().to_le_bytes());
        buf.extend_from_slice(many.as_bytes());
        let err = read_frame::<_, FromHost>(&mut io::Cursor::new(buf), MAX_FROM_HOST).unwrap_err();
        assert!(matches!(err, FrameError::Malformed(ref m) if m.contains("values")), "{err:?}");
        // Commas and brackets inside text are not values.
        assert_eq!(values_in(br#"{"line":"a,b,[c]:{\",\"}"}"#), 3, "one object and its one member");
    }

    /// A small deterministic generator, so a failure names its seed.
    fn bytes(seed: u64, len: usize) -> Vec<u8> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x.to_le_bytes()[0]
            })
            .collect()
    }

    /// Frames from a host are untrusted: random bytes, and valid frames with their bytes changed,
    /// end in a message or an error, never a panic or a hang. The fuzz target in `fuzz/` explores
    /// further; this runs with every test run.
    #[test]
    fn random_and_damaged_frames_never_panic() {
        let alphabet = br#"{}[]":,0123456789.-eE tfnul\ "#;
        let mut valid = Vec::new();
        write_frame(&mut valid, &FromHost::Dialog { kind: DialogKind::Prompt, args: json!(["Name?", "Untitled", "Save"]) }, MAX_FROM_HOST).unwrap();
        for seed in 0..3000u64 {
            let mut data = bytes(seed, (seed as usize % 300) + 1);
            if seed % 3 == 1 {
                // JSON-like noise, which gets past the first bytes of a parse.
                data = data.iter().map(|b| alphabet[usize::from(*b) % alphabet.len()]).collect();
                let mut framed = u32::try_from(data.len()).unwrap().to_le_bytes().to_vec();
                framed.extend_from_slice(&data);
                data = framed;
            } else if seed % 3 == 2 {
                // A valid frame with a few bytes changed.
                data = valid.clone();
                for (i, b) in bytes(seed, 4).into_iter().enumerate() {
                    let at = (usize::from(b) * 7 + i * 31) % data.len();
                    data[at] = b;
                }
            }
            let _ = values_in(&data);
            let mut input = data.as_slice();
            for _ in 0..64 {
                match read_frame::<_, FromHost>(&mut input, MAX_FROM_HOST) {
                    Ok(Some(FromHost::Dialog { kind, args })) => drop(check_dialog_args(kind, &args)),
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => break,
                }
            }
            if let Ok(v) = serde_json::from_slice::<Value>(&data) {
                let _ = check_dialog_args(DialogKind::Alert, &v);
            }
        }
    }

    /// Whatever a writer sends, the reader takes: a reply nested past what the reader parses is
    /// refused before any of it is written, as one with too many values is.
    #[test]
    fn a_frame_nested_past_the_readers_limit_is_not_written() {
        let nested = |n: usize| (0..n).fold(json!(0), |v, _| json!([v]));
        let mut written = 0;
        for n in 0..200 {
            let mut buf = Vec::new();
            match write_frame(&mut buf, &ToHost::Executed { result: Ok(nested(n)) }, MAX_TO_HOST) {
                Ok(()) => {
                    written += 1;
                    let read: Option<ToHost> =
                        read_frame(&mut buf.as_slice(), MAX_TO_HOST).unwrap_or_else(|e| panic!("{n} deep was written but not read: {e}"));
                    assert!(read.is_some());
                }
                Err(e) => {
                    assert!(matches!(e, FrameError::TooDeep), "{n}: {e}");
                    assert!(buf.is_empty(), "nothing is written");
                }
            }
        }
        assert!(written > 100, "shallow replies are still written");
        assert!(written < 200, "the deepest are refused");
    }

    #[test]
    fn cut_respects_character_boundaries() {
        assert_eq!(cut("aé", 2), "a");
        assert_eq!(cut("abc", 5), "abc");
    }
}
