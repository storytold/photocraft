//! Exporting one layer suggests a file name with every character a file name can't hold
//! (`< > : " / \ | ? *`) turned into `_`, one for one (#14810). Other characters, spaces
//! included (also at either end), stay, and names without such characters suggest what they did
//! before. Unmeasured cases (control characters, very long names) follow the multi-layer
//! export's `export_name`.

use super::*;
use craft::host::ScriptedFiles;

/// The file name Layer › Quick Export as PNG suggests for a layer named `name` (the Save dialog
/// stays open), and the whole save request as text.
fn suggested_for(name: &str) -> (String, String) {
    let files = Arc::new(ScriptedFiles::held());
    let services = crate::Services { files: Some(files.clone()), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 20, "height": 20})).unwrap();
    app.run("layer.new.layer", json!({"name": name})).unwrap();
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    quick_export_layer_png(&mut app, layer).unwrap();
    let asked = files.requests();
    let [request] = asked.as_slice() else { panic!("one Save dialog: {asked:?}") };
    (request.file_name.clone().unwrap_or_default(), format!("{request:?}"))
}

#[test]
fn a_layer_named_with_illegal_characters_suggests_underscores() {
    let (file, path) = suggested_for("a/b:c*?");
    assert_eq!(file, "a_b_c__.png", "{path}");
    assert!(!path.contains("a/b"), "the name is not read as a folder: {path}");
}

#[test]
fn each_illegal_character_becomes_one_underscore() {
    for c in ['<', '>', ':', '"', '/', '\\', '|', '?', '*'] {
        let (file, path) = suggested_for(&format!("x{c}y"));
        assert_eq!(file, "x_y.png", "{c:?}: {path}");
    }
}

#[test]
fn other_characters_and_spaces_stay() {
    for (name, want) in [("Artboard 1", "Artboard 1.png"), ("Layer 1", "Layer 1.png"), ("ünï-cødé_(2)", "ünï-cødé_(2).png")] {
        let (file, path) = suggested_for(name);
        assert_eq!(file, want, "{path}");
    }
}

/// Spaces at either end are part of the name and stay, as they did before #14810.
#[test]
fn boundary_spaces_stay() {
    let (file, path) = suggested_for(" Artboard 1 ");
    assert_eq!(file, " Artboard 1 .png", "{path}");
    let (file, path) = suggested_for(" a/b ");
    assert_eq!(file, " a_b .png", "{path}");
}

#[test]
fn names_of_only_dots_and_whitespace_fall_back_to_layer() {
    for name in ["...", "   ", "\u{3000}", ". \u{a0}."] {
        let (file, path) = suggested_for(name);
        assert_eq!(file, "layer.png", "{name:?}: {path}");
    }
    assert_eq!(layer_export_name(""), "layer");
}
