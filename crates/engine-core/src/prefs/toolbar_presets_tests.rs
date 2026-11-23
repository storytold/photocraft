//! Toolbar presets (`.tbr`) through `edit.toolbar`'s `loadPreset` / `savePreset` (#14624).

use super::*;
use crate::Session;
use photocraft_paint::tile::{b64_decode, b64_encode};
use serde_json::{Value, json};

const MOVE: &str = "tools.select.move";

fn save(s: &mut Session) -> Vec<u8> {
    let r = s.execute("edit.toolbar", json!({"savePreset": true})).unwrap();
    b64_decode(r["preset"].as_str().unwrap()).unwrap()
}

fn load(s: &mut Session, bytes: &[u8]) -> crate::Result<Value> {
    s.execute("edit.toolbar", json!({"loadPreset": {"data": b64_encode(bytes)}}))
}

/// The letter a tool command answers now.
fn key(s: &Session, id: &str) -> Option<String> {
    s.prefs().shortcut(id, default_letter(id)).map(str::to_string)
}

/// A tool entry as the reference application writes it.
fn tool(code: &[u8; 4], letter: Option<&str>) -> D {
    let d = object().with("toolKey", D::Integer(i32::from_be_bytes(*code)));
    D::Descriptor(match letter {
        Some(l) => d.with("keyboardShortcut", D::Text(UnicodeString::new_nul(l))),
        None => d,
    })
}

/// The default preset's `tlst` groups.
fn default_toolbar() -> Vec<Vec<D>> {
    let d = Descriptor::from_bytes(&save(&mut crate::session())).unwrap();
    let Some(D::List(groups)) = d.get("tlst") else { panic!("tlst is a list") };
    groups.iter().map(|g| if let D::List(t) = g { t.clone() } else { panic!("a group is a list") }).collect()
}

/// The default preset with its `tlst` replaced.
fn with_toolbar(groups: Vec<Vec<D>>) -> Vec<u8> {
    let mut d = Descriptor::from_bytes(&save(&mut crate::session())).unwrap();
    for (k, v) in &mut d.items {
        if k.is("tlst") {
            *v = D::List(groups.iter().cloned().map(D::List).collect());
        }
    }
    d.to_bytes()
}

#[test]
fn every_tool_has_its_own_preset_key() {
    for (id, _, _) in crate::shell_commands::TOOLS {
        let name = crate::prefs::ToolbarCustomization::tool_name(id.trim_start_matches("tools.select.")).unwrap();
        assert!(TOOL_KEYS.iter().any(|(n, _)| *n == name), "{id} has no .tbr key, so a saved preset would leave it out");
        assert_eq!(command_id(&name), *id);
    }
    for (i, (_, a)) in TOOL_KEYS.iter().enumerate() {
        assert!(TOOL_KEYS.iter().skip(i + 1).all(|(_, b)| a != b), "{} is used twice", String::from_utf8_lossy(*a));
    }
}

#[test]
fn a_saved_preset_has_the_reference_layout() {
    let bytes = save(&mut crate::session());
    // Name: one NUL; class `null`.
    assert_eq!(bytes[..14], [0, 0, 0, 1, 0, 0, 0, 0, 0, 0, b'n', b'u', b'l', b'l']);
    let d = Descriptor::from_bytes(&bytes).unwrap();
    let keys: Vec<String> = d.items.iter().map(|(k, _)| String::from_utf8_lossy(k.as_bytes()).into_owned()).collect();
    assert_eq!(keys, ["tver", "tpad", "fgbg", "qkmm", "sImI", "scmd", "shov", "nana", "ddkb", "tlst", "oflt"]);
    let Some(D::List(groups)) = d.get("tlst") else { panic!("tlst is a list") };
    let D::List(first) = &groups[0] else { panic!("a group is a list") };
    let D::Descriptor(mv) = &first[0] else { panic!("a tool is an object") };
    assert_eq!(mv.get("toolKey"), Some(&D::Integer(i32::from_be_bytes(*b"arwT"))));
    assert_eq!(mv.get_text("keyboardShortcut").as_deref(), Some("V"));
    let blur = groups.iter().flat_map(|g| if let D::List(t) = g { t.clone() } else { vec![] }).find_map(|t| match t {
        D::Descriptor(t) if t.get("toolKey") == Some(&D::Integer(i32::from_be_bytes(*b"blur"))) => Some(t),
        _ => None,
    });
    assert!(blur.is_some_and(|b| b.get("keyboardShortcut").is_none()), "a tool without a letter has no keyboardShortcut");
}

#[test]
fn a_preset_round_trips_the_lists_their_groups_and_the_letters() {
    let mut s = crate::session();
    s.execute("edit.toolbar", json!({"toolbar": [["Zoom", "Hand"], ["Move", "Lasso"]], "extras": [["Artboard", "Brush"], ["Frame"]]})).unwrap();
    s.execute("edit.keyboardShortcuts", json!({"set": {MOVE: "Q", "tools.select.blur": "X", "tools.select.brush": ""}, "removeConflicts": false})).unwrap();
    let saved = s.prefs().toolbar.layout();
    let bytes = save(&mut s);

    let mut t = crate::session();
    assert_ne!(t.prefs().toolbar.layout(), saved);
    let r = load(&mut t, &bytes).unwrap();
    assert_eq!(t.prefs().toolbar.layout(), saved);
    assert_eq!(r["toolbar"], json!(saved.toolbar));
    for id in [MOVE, "tools.select.blur", "tools.select.brush", "tools.select.artboard", "tools.select.lasso"] {
        assert_eq!(key(&t, id), key(&s, id), "{id}");
    }
}

#[test]
fn a_hand_edited_preset_appends_missing_tools_and_takes_letters_unchecked() {
    let mut s = crate::session();
    let quick_mask = key(&s, "select.editInQuickMaskMode");
    let mut groups = default_toolbar();
    groups.retain(|g| g.first() != Some(&tool(b"zoom", Some("Z"))));
    groups[0][0] = tool(b"arwT", Some("q"));
    load(&mut s, &with_toolbar(groups)).unwrap();
    let layout = s.prefs().toolbar.layout();
    assert!(layout.toolbar.iter().flatten().all(|t| t != "Zoom"));
    assert_eq!(layout.extras.last(), Some(&vec!["Zoom".to_string()]), "a tool listed nowhere gets a group of its own in Extra Tools");
    assert_eq!(key(&s, MOVE).as_deref(), Some("Q"));
    assert_eq!(key(&s, "tools.select.artboard").as_deref(), Some("V"));
    assert_eq!(key(&s, "select.editInQuickMaskMode"), quick_mask, "letters from a preset are not checked for conflicts");
}

#[test]
fn unknown_tools_are_skipped_and_a_repeated_tool_keeps_its_first_place_and_letter() {
    let mut s = crate::session();
    let unknown = tool(b"what", Some("K"));
    load(&mut s, &with_toolbar(vec![vec![unknown, tool(b"arwT", Some("Q"))], vec![tool(b"arwT", Some("X")), tool(b"pntb", Some("B"))]])).unwrap();
    let layout = s.prefs().toolbar.layout();
    assert_eq!(layout.toolbar[..2], [vec!["Move".to_string()], vec!["Brush".to_string()]]);
    assert_eq!(key(&s, MOVE).as_deref(), Some("Q"));
}

#[test]
fn a_shortcut_that_is_not_a_letter_leaves_the_tool_letter_alone() {
    let mut s = crate::session();
    load(&mut s, &with_toolbar(vec![vec![tool(b"arwT", Some("5")), tool(b"pntb", None), tool(b"blur", Some(""))]])).unwrap();
    assert_eq!(key(&s, MOVE).as_deref(), Some("V"));
    assert_eq!(key(&s, "tools.select.brush"), None, "no keyboardShortcut is no letter");
    assert_eq!(key(&s, "tools.select.blur"), None);
}

#[test]
fn a_damaged_preset_is_refused_and_changes_nothing() {
    let mut s = crate::session();
    s.execute("edit.toolbar", json!({"extras": [["Zoom"]]})).unwrap();
    let before = s.prefs().clone();
    let good = save(&mut s);
    let mut bad: Vec<Vec<u8>> = vec![Vec::new(), vec![b'x'; 100], [good.as_slice(), &[0]].concat()];
    bad.extend((1..good.len()).map(|n| good[..n].to_vec()));
    bad.push(object().with("tver", D::Integer(1)).to_bytes());
    bad.push(object().with("tlst", D::List(vec![D::Integer(3)])).with("oflt", D::List(vec![])).to_bytes());
    for bytes in &bad {
        assert!(load(&mut s, bytes).is_err(), "{} bytes", bytes.len());
        assert!(*s.prefs() == before, "a refused preset of {} bytes changed the preferences", bytes.len());
    }
}

#[test]
fn presets_are_read_and_written_by_path() {
    let dir = std::env::temp_dir().join(format!("photocraft-tbr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("mine.tbr");
    let path = file.to_str().unwrap();
    let mut s = crate::session();
    let r = s.execute("edit.toolbar", json!({"extras": [["Zoom"]], "savePreset": {"path": path}})).unwrap();
    assert_eq!(r["presetPath"], json!(path));
    let saved = s.prefs().toolbar.layout();

    let mut t = crate::session();
    t.execute("edit.toolbar", json!({"loadPreset": {"path": path}})).unwrap();
    assert_eq!(t.prefs().toolbar.layout(), saved);

    // A target that can't be written keeps the change from happening at all.
    let missing = dir.join("no").join("such.tbr");
    assert!(t.execute("edit.toolbar", json!({"reset": true, "savePreset": {"path": missing.to_str().unwrap()}})).is_err());
    assert_eq!(t.prefs().toolbar.layout(), saved);
    assert!(t.execute("edit.toolbar", json!({"loadPreset": {"path": dir.join("absent.tbr").to_str().unwrap()}})).is_err());
    assert!(t.execute("edit.toolbar", json!({"loadPreset": "mine.tbr"})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
