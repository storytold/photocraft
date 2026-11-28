//! Toolbar presets (`.tbr`) through `edit.toolbar`'s `loadPreset` / `savePreset` (#14624).

use super::*;
use crate::Session;
use craft_psd::descriptor::{Descriptor, Id, UnicodeString, Value as D};
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

/// A tool entry with a shortcut, or with none.
fn tool(code: &[u8; 4], letter: Option<&str>) -> Tool {
    Tool { key: *code, shortcut: letter.map_or(Shortcut::Absent, |l| Shortcut::Text(l.into())) }
}

/// The default preset's toolbar groups.
fn default_toolbar() -> Vec<Vec<Tool>> {
    tbr::read(&save(&mut crate::session())).unwrap().toolbar
}

/// The default preset with its toolbar groups replaced.
fn with_toolbar(groups: Vec<Vec<Tool>>) -> Vec<u8> {
    tbr::write(&ToolbarPreset { toolbar: groups, ..tbr::read(&save(&mut crate::session())).unwrap() }).unwrap()
}

/// A descriptor as presets hold them, for files the shared reader refuses.
fn object() -> Descriptor {
    Descriptor { name: UnicodeString::new_nul(""), class_id: Id::new("null"), items: Vec::new() }
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

/// The file layout is `craft_psd::tbr`'s; this checks what PhotoCraft puts in it.
#[test]
fn a_saved_preset_holds_the_tools_their_letters_and_the_default_toggles() {
    let preset = tbr::read(&save(&mut crate::session())).unwrap();
    assert_eq!(preset.toolbar[0][0], tool(b"arwT", Some("V")));
    let blur = preset.toolbar.iter().flatten().find(|t| t.key == *b"blur");
    assert_eq!(blur.map(|t| &t.shortcut), Some(&Shortcut::Absent), "a tool without a letter has no shortcut");
    assert_eq!(
        ToolbarPreset { toolbar: Vec::new(), extras: Vec::new(), ..preset },
        ToolbarPreset::default(),
        "settings PhotoCraft has none of yet keep their defaults"
    );
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

#[test]
fn a_hidden_list_given_with_a_preset_applies_after_it() {
    let bytes = save(&mut crate::session());
    let mut s = crate::session();
    s.execute("edit.toolbar", json!({"loadPreset": {"data": b64_encode(&bytes)}, "hidden": ["Zoom"]})).unwrap();
    let layout = s.prefs().toolbar.layout();
    assert!(layout.toolbar.iter().flatten().all(|t| t != "Zoom"), "the explicit hidden list wins over the preset's toolbar");
    assert!(layout.extras.iter().flatten().any(|t| t == "Zoom"));

    let mut t = crate::session();
    load(&mut t, &bytes).unwrap();
    assert!(t.prefs().toolbar.layout().toolbar.iter().flatten().any(|t| t == "Zoom"), "without `hidden` the preset's layout is kept");
}

#[test]
fn an_extras_list_given_with_a_preset_takes_its_tools_off_the_presets_toolbar() {
    let bytes = save(&mut crate::session());
    let mut s = crate::session();
    s.execute("edit.toolbar", json!({"loadPreset": {"data": b64_encode(&bytes)}, "extras": [["Zoom"]]})).unwrap();
    let layout = s.prefs().toolbar.layout();
    assert!(layout.toolbar.iter().flatten().all(|t| t != "Zoom"), "the given extras list wins over the preset's toolbar");
    assert_eq!(layout.extras.first(), Some(&vec!["Zoom".to_string()]));

    // A toolbar given too keeps a tool both lists name.
    let mut t = crate::session();
    t.execute("edit.toolbar", json!({"loadPreset": {"data": b64_encode(&bytes)}, "toolbar": [["Zoom"]], "extras": [["Zoom"]]})).unwrap();
    let layout = t.prefs().toolbar.layout();
    assert_eq!(layout.toolbar, [vec!["Zoom".to_string()]]);
    assert!(layout.extras.iter().flatten().all(|t| t != "Zoom"));
}
