//! #12047: a type layer named from its text keeps following that text across a PSD save and
//! reopen. `DocState::typed_names` lives in the session only, so after a reopen the stored layer
//! name and the round-tripped text alone decide whether the name still follows text edits. This
//! pins that: names the current rule and the rule before it gave, and Duplicate Layer copies of
//! those names, keep following; a user-given name stays the user's even with the typed-name
//! record gone.

use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::Session;
use serde_json::json;

/// Saves the active document of `s` as a PSD and returns a session opened on the result.
fn reopen(s: &Session) -> Session {
    let out = photocraft_io::export(&s.active().unwrap().doc, "psd", &Default::default()).unwrap();
    let reopened = photocraft_io::import("x.psd", &out.bytes).unwrap().document;
    let mut s = Session::new();
    s.add_document(reopened, None);
    s
}

fn layer_named(s: &Session, name: &str) -> LayerId {
    s.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == name).unwrap_or_else(|| panic!("no layer {name} after reopen")).id
}

fn text_of(s: &Session, id: LayerId) -> String {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.text.clone(),
        other => panic!("layer {} reopened as {}, not a type layer", id.0, other.kind_name()),
    }
}

fn name_of(s: &Session, id: LayerId) -> String {
    s.active().unwrap().doc.layer(id).unwrap().name.clone()
}

/// #12047: after a real PSD save and reopen, layers named by either naming rule - and a saved
/// Duplicate Layer copy of a name the previous rule gave - still follow their text; the file
/// round trip itself is what keeps the name and the text it was derived from matching.
#[test]
fn text_named_layers_keep_following_their_text_after_a_psd_reopen() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 480, "height": 320})).unwrap();
    let long_name = "x".repeat(40);
    let long_text = "x".repeat(50);
    // The first layer takes its name from the current rule at creation: every line run
    // together (#12047). The others carry the previous rule's names, as documents saved
    // before the rule change has them: first non-empty line cut at 40 characters, and
    // Duplicate Layer copies of those.
    for (y, name, text) in
        [(0, "Hello\nworld", "Hello\nworld"), (40, "Hello", "Hello\nworld"), (80, "Hello copy", "Hello\nworld"), (120, long_name.as_str(), long_text.as_str())]
    {
        let id = s.execute("type.create", json!({"x": 0, "y": y, "text": text})).unwrap()["layer"].as_u64().unwrap();
        if name != text {
            s.execute("layer.renameLayer", json!({"layer": id, "name": name})).unwrap();
        }
    }
    // The reopen: the PSD round trip keeps the type layers as type layers, with their names and
    // their text, so every derived name still matches the text it came from.
    let mut s = reopen(&s);
    for (name, text) in [("Helloworld", "Hello\nworld"), ("Hello", "Hello\nworld"), ("Hello copy", "Hello\nworld"), (long_name.as_str(), long_text.as_str())] {
        let id = layer_named(&s, name);
        assert_eq!(text_of(&s, id), text, "{name}: the PSD round trip keeps the text the name was derived from");
    }
    // A reopened session has no typed-name record; only the stored names decide, and all four
    // follow their new text.
    let edits = [("Helloworld", "Bye"), ("Hello", "Later"), ("Hello copy", "Done"), (long_name.as_str(), "Short")];
    for (name, new_text) in edits {
        let id = layer_named(&s, name);
        s.execute("type.edit", json!({"layer": id.0, "text": new_text})).unwrap();
        assert_eq!(name_of(&s, id), new_text, "{name} keeps following its text after the reopen");
    }
}

/// #12047: a user-given name survives a PSD save and reopen and keeps NOT following the text,
/// even though the typed-name record that protects copy-shaped names in a live session is
/// in-memory only: the name itself, matching no rule, is what keeps it.
#[test]
fn a_users_name_stays_the_users_across_a_psd_reopen() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 480, "height": 320})).unwrap();
    let id = s.execute("type.create", json!({"x": 0, "y": 0, "text": "Hello\nworld"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.renameLayer", json!({"layer": id, "name": "My Brand"})).unwrap();
    let mut s = reopen(&s);
    let id = layer_named(&s, "My Brand");
    s.execute("type.edit", json!({"layer": id.0, "text": "Bye"})).unwrap();
    assert_eq!(name_of(&s, id), "My Brand", "a name neither rule gave stays the user's");
}
