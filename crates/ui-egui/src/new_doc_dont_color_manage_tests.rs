//! #14032: New Document's Color Profile list offers Don't Color Manage right after the working
//! space, for every colour mode, and it creates an untagged document.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::new_doc_ui::{color_mode_of, command_params, profile_options, set_mode};
use crate::state::DialogKind;

const MODES: [&str; 4] = ["gray", "rgb", "cmyk", "lab"];

fn options(mode: &str) -> Vec<(String, String)> {
    profile_options(color_mode_of(mode).map(photocraft_engine::color_cmds::mode_space), "sRGB IEC61966-2.1", &[])
}

#[test]
fn dont_color_manage_is_second_for_every_mode() {
    for mode in MODES {
        let opts = options(mode);
        assert_eq!(opts[0].0, "working", "{mode}");
        assert_eq!((opts[1].0.as_str(), opts[1].1.as_str()), ("none", "Don't Color Manage"), "{mode}");
        assert_eq!(opts.iter().filter(|(v, _)| v == "none").count(), 1, "{mode}");
    }
}

#[test]
fn create_with_dont_color_manage_sends_none_and_makes_an_untagged_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let f = app.new_document_fields();
    let id = app.ui.open_dialog(DialogKind::NewDocument, f);
    let d = app.ui.dialog_mut(id).unwrap();
    for (k, v) in [("width", json!(32)), ("height", json!(24)), ("mode", json!("rgb")), ("profile", json!("none"))] {
        d.fields.insert(k.into(), v);
    }
    assert_eq!(command_params(&d.fields)["profile"], "none");
    crate::dialogs::confirm(&mut app, id).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert_eq!((doc.size.width, doc.size.height), (32, 24));
    assert!(doc.icc_profile.is_none(), "Don't Color Manage embeds no profile");
}

#[test]
fn every_mode_creates_an_untagged_document_with_none() {
    for mode in MODES {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 2, "height": 2, "mode": mode, "profile": "none"})).unwrap_or_else(|e| panic!("{mode}: {e}"));
        assert!(s.active().unwrap().doc.icc_profile.is_none(), "{mode}");
    }
}

#[test]
fn switching_mode_keeps_dont_color_manage() {
    let mut f = Map::new();
    f.insert("mode".into(), json!("rgb"));
    f.insert("profile".into(), json!("none"));
    for mode in ["cmyk", "gray", "lab", "rgb"] {
        set_mode(&mut f, mode);
        assert_eq!(f["mode"], mode);
        assert_eq!(f.get("profile").and_then(Value::as_str), Some("none"), "{mode}");
        assert!(options(mode).iter().any(|(v, _)| v == "none"), "the dropdown still offers it in {mode}");
    }
}

#[test]
fn switching_mode_still_drops_a_profile_of_the_old_space() {
    let mut f = Map::new();
    f.insert("mode".into(), json!("rgb"));
    f.insert("profile".into(), json!(craft_cms::Builtin::AdobeRgbCompat.id()));
    set_mode(&mut f, "cmyk");
    assert!(!f.contains_key("profile"), "{f:?}");
}

#[test]
fn size_resolution_and_name_changes_keep_it() {
    let mut f = Map::new();
    f.insert("profile".into(), json!("none"));
    for (k, v) in [("width", json!(640)), ("height", json!(480)), ("name", json!("Poster"))] {
        f.insert(k.into(), v);
    }
    crate::new_doc_ui::set_resolution(&mut f, 300.0);
    assert_eq!(command_params(&f)["profile"], "none");
}
