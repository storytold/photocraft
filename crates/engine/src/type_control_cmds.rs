//! `type.insertControl`: insert an invisible bidi or joining control into a type layer (spec
//! 5.1.8, review finding 8). Phone numbers and prices inside Arabic need them: "keep this number
//! left-to-right" is an LRI … PDI pair around it. One command with no menu item (PhotoCraft's
//! menus mirror Photoshop's one to one); the command palette names it by task.

use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The `char` names and their characters.
pub const CONTROLS: [(&str, char); 10] = [
    ("lrm", '\u{200E}'),
    ("rlm", '\u{200F}'),
    ("alm", '\u{061C}'),
    ("lri", '\u{2066}'),
    ("rli", '\u{2067}'),
    ("fsi", '\u{2068}'),
    ("pdi", '\u{2069}'),
    ("zwj", '\u{200D}'),
    ("zwnj", '\u{200C}'),
    ("tatweel", '\u{0640}'),
];

const CMD: &str = "type.insertControl";
/// POP DIRECTIONAL ISOLATE: closes an isolate.
const PDI: char = '\u{2069}';

/// The control character for a `char` name.
pub fn control_char(name: &str) -> Option<char> {
    CONTROLS.iter().find(|(n, _)| *n == name).map(|&(_, c)| c)
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn index(v: &Value, what: &str) -> Result<usize> {
    let n = v.as_u64().ok_or_else(|| bad(format!("{what} must be a character index")))?;
    usize::try_from(n).map_err(|_| bad(format!("{what} is out of range")))
}

/// Byte offset of character `ci` (the text length past the end).
fn byte_at(text: &str, ci: usize) -> usize {
    text.char_indices().nth(ci).map_or(text.len(), |(b, _)| b)
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let ch = p
        .get("char")
        .and_then(Value::as_str)
        .and_then(control_char)
        .ok_or_else(|| bad(format!("`char` must be one of {}", CONTROLS.map(|(n, _)| n).join(", "))))?;
    let id = match p.get("layer") {
        None | Some(Value::Null) => s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into()))?,
        Some(v) => LayerId(v.as_u64().ok_or_else(|| bad("`layer` must be a layer id"))?),
    };
    // The caret or the selection, in characters; neither: the end of the text (as type.insertText).
    let span = match (p.get("range"), p.get("at")) {
        (Some(Value::Array(r)), _) if r.len() == 2 => {
            let (a, b) = (index(r.first().unwrap_or(&Value::Null), "range[0]")?, index(r.get(1).unwrap_or(&Value::Null), "range[1]")?);
            Some((a.min(b), a.max(b)))
        }
        (Some(_), _) => return Err(bad("`range` must be [startChar, endChar]")),
        (None, Some(v)) => {
            let i = index(v, "`at`")?;
            Some((i, i))
        }
        (None, None) => None,
    };
    let isolate = matches!(ch, '\u{2066}' | '\u{2067}' | '\u{2068}');
    let p = json!({ "layer": id.0 });
    let selection = crate::type_cmds::with_text_layer(s, &p, "Insert Control Character", |t, _, _| {
        let n = t.text.chars().count();
        let (a, b) = span.unwrap_or((n, n));
        if b > n {
            return Err(bad(format!("index {b} is past the end of the text ({n} characters)")));
        }
        let (ba, bb) = (byte_at(&t.text, a), byte_at(&t.text, b));
        let mut buf = [0u8; 4];
        if isolate && a < b {
            // One step: the closing PDI first, so the opening isolate doesn't move its offset.
            let mut pdi = [0u8; 4];
            crate::type_cmds::replace_text(t, bb, bb, PDI.encode_utf8(&mut pdi));
            crate::type_cmds::replace_text(t, ba, ba, ch.encode_utf8(&mut buf));
            Ok([a.saturating_add(1), b.saturating_add(1)])
        } else {
            crate::type_cmds::replace_text(t, ba, bb, ch.encode_utf8(&mut buf));
            Ok([a.saturating_add(1); 2])
        }
    })?;
    Ok(json!({ "layer": id.0, "caret": selection[1], "selection": selection }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Insert Control Character",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id?, "char":"lrm|rlm|alm|lri|rli|fsi|pdi|zwj|zwnj|tatweel", "at":char? (default: end of text), "range":[startChar,endChar]? (a selection)} → {"layer","caret","selection":[start,end]} (characters: where the caller's caret and selection go). With a non-empty range, lri/rli/fsi wrap it in that isolate plus PDI, as one history step; any other char replaces it. Palette: "Keep Selection Left-to-Right", "Keep Selection Right-to-Left", "Insert Tatweel"…"##,
        enabled: has_doc,
        journal: true,
        run,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::LayerContent;

    fn session_with(text: &str) -> (Session, u64) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 300, "height": 100})).unwrap();
        let id = s.execute("type.create", json!({"x": 10, "y": 50, "text": text, "size": 20})).unwrap()["layer"].as_u64().unwrap();
        (s, id)
    }

    fn text_of(s: &Session, id: u64) -> String {
        match &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content {
            LayerContent::Text(t) => t.text.clone(),
            other => panic!("{}", other.kind_name()),
        }
    }

    #[test]
    fn inserts_each_control_at_the_caret() {
        for (name, ch) in CONTROLS {
            let (mut s, id) = session_with("ab");
            let r = s.execute("type.insertControl", json!({"layer": id, "char": name, "at": 1})).unwrap();
            assert_eq!(text_of(&s, id), format!("a{ch}b"), "{name}");
            assert_eq!((r["caret"].clone(), r["selection"].clone()), (json!(2), json!([2, 2])), "{name}");
        }
        // No position: the end of the text, like type.insertText.
        let (mut s, id) = session_with("ab");
        s.execute("type.insertControl", json!({"layer": id, "char": "tatweel"})).unwrap();
        assert_eq!(text_of(&s, id), "ab\u{640}");
    }

    #[test]
    fn an_isolate_wraps_the_selection_in_one_undo_step() {
        let (mut s, id) = session_with("رقم 0551234567 فقط");
        let steps = s.active().unwrap().history.entries().len();
        let r = s.execute("type.insertControl", json!({"layer": id, "char": "lri", "range": [14, 4]})).unwrap();
        assert_eq!(text_of(&s, id), "رقم \u{2066}0551234567\u{2069} فقط");
        assert_eq!(r["selection"], json!([5, 15]), "the same digits, now inside the isolate");
        assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(text_of(&s, id), "رقم 0551234567 فقط");
        for (name, ch) in [("rli", '\u{2067}'), ("fsi", '\u{2068}')] {
            let (mut s, id) = session_with("abc");
            s.execute("type.insertControl", json!({"layer": id, "char": name, "range": [1, 2]})).unwrap();
            assert_eq!(text_of(&s, id), format!("a{ch}b\u{2069}c"), "{name}");
        }
        // Any other control replaces the selection, like typing it.
        let (mut s, id) = session_with("abc");
        s.execute("type.insertControl", json!({"layer": id, "char": "zwnj", "range": [1, 2]})).unwrap();
        assert_eq!(text_of(&s, id), "a\u{200C}c");
    }

    #[test]
    fn bad_params_fail_gracefully() {
        let (mut s, id) = session_with("ab");
        let pixel = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        let steps = s.active().unwrap().history.entries().len();
        for p in [
            json!({"layer": id, "at": 0}),
            json!({"layer": id, "char": "nbsp", "at": 0}),
            json!({"layer": id, "char": 7, "at": 0}),
            json!({"layer": id, "char": "lrm", "at": 3}),
            json!({"layer": id, "char": "lrm", "at": -1}),
            json!({"layer": id, "char": "lrm", "at": "1"}),
            json!({"layer": id, "char": "lri", "range": [0]}),
            json!({"layer": id, "char": "lri", "range": [0, 99]}),
            json!({"layer": id, "char": "lri", "range": "0,1"}),
            json!({"layer": id, "char": "lri", "range": [0.5, 1]}),
            json!({"layer": pixel, "char": "lrm", "at": 0}),
            json!({"layer": 9_999_999, "char": "lrm", "at": 0}),
            json!({"layer": "x", "char": "lrm", "at": 0}),
            json!({"char": "lrm", "at": 0}),
        ] {
            assert!(s.execute("type.insertControl", p.clone()).is_err(), "{p}");
        }
        assert_eq!(text_of(&s, id), "ab");
        assert_eq!(s.active().unwrap().history.entries().len(), steps, "no step for a failed call");
        let mut bare = Session::new();
        assert!(matches!(bare.execute("type.insertControl", json!({"char": "lrm", "at": 0})), Err(EngineError::Disabled(..))));
    }

    #[test]
    fn is_a_journaled_command_without_a_menu() {
        let spec = crate::command_specs().iter().find(|c| c.id == "type.insertControl").unwrap();
        assert!(spec.journal && spec.menu.is_empty());
    }

    #[test]
    fn consecutive_calls_with_one_coalesce_key_share_a_history_step() {
        let (mut s, id) = session_with("ab");
        let steps = s.active().unwrap().history.entries().len();
        s.execute("type.insertControl", json!({"layer": id, "char": "lrm", "at": 1, "coalesce": "k"})).unwrap();
        s.execute("type.insertControl", json!({"layer": id, "char": "rlm", "at": 2, "coalesce": "k"})).unwrap();
        assert_eq!(text_of(&s, id), "a\u{200E}\u{200F}b");
        assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
        s.execute("type.insertControl", json!({"layer": id, "char": "zwj", "at": 0})).unwrap();
        assert_eq!(s.active().unwrap().history.entries().len(), steps + 2, "no key: a step of its own");
    }
}
