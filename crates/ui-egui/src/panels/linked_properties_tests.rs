//! #14121: the Properties panel tells a linked smart object apart, as Photoshop does: its header
//! reads "Linked Smart Object", W and H are read-only while X and Y still move it, the linked
//! file's full path sits below Transform, and its Quick Actions are Edit Contents, Embed and
//! Convert to Layers. With the file missing the path gets a red mark and Embed is greyed, until the
//! file is back.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, SmartSource};

use super::*;

/// The Embed Quick Action's button: Layer › Smart Objects › Embed Linked, by its command's name.
const EMBED_BUTTON: &str = "Embed Linked";

/// A 100 × 60 px PNG in a fresh temporary folder, named `name`; returns the folder and the file.
fn png(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join(name);
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 100, "height": 60, "background": "#336699"})).unwrap();
    s.execute("file.saveACopy", json!({"path": file.to_string_lossy(), "layers": false})).unwrap();
    (directory, file)
}

/// A 400 × 300 px document with `file` placed through `command` (`file.placeLinked` or
/// `file.placeEmbedded`) centred at 200, 150 at its own size, with the Properties panel drawn.
fn panel(command: &str, file: &std::path::Path) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run(command, json!({"path": file.to_string_lossy(), "center": [200, 150], "fit": false})).unwrap();
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(vec2(320.0, 1200.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::panels::properties::panel(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    frames(&mut h, 4);
    h
}

/// `n` frames as the app's frame runs them: the command states are published before the input (a
/// command button's label and enablement), and the commands the frame fired run after it.
fn frames(h: &mut Harness<'_, Box<PhotocraftApp>>, n: usize) {
    for _ in 0..n {
        let ctx = h.ctx.clone();
        h.state_mut().framework_before_input(&ctx);
        h.step();
        h.state_mut().framework_before_input(&ctx);
        crate::shortcuts::handle(h.state_mut(), &ctx);
    }
}

/// The active layer of the active document.
fn active(h: &Harness<'_, Box<PhotocraftApp>>) -> photocraft_doc::Layer {
    let st = h.state().session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
}

/// The Quick Actions buttons drawn, in reading order (top to bottom, then left to right).
fn quick_actions_shown(h: &Harness<'_, Box<PhotocraftApp>>) -> Vec<String> {
    let names = ["Edit Contents", EMBED_BUTTON, "Convert to Layers"];
    let mut found: Vec<(f32, f32, String)> = names
        .iter()
        .filter_map(|n| {
            h.query_all_by_label(n)
                .find(|node| node.accesskit_node().role() == egui::accesskit::Role::Button)
                .map(|node| (node.rect().top(), node.rect().left(), n.to_string()))
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    found.into_iter().map(|(_, _, n)| n).collect()
}

/// Whether the Embed button is greyed.
fn embed_greyed(h: &Harness<'_, Box<PhotocraftApp>>) -> bool {
    h.get_by_label(EMBED_BUTTON).accesskit_node().is_disabled()
}

/// Whether the size field named `label` (Width or Height) is shown and read-only.
fn read_only(h: &Harness<'_, Box<PhotocraftApp>>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_read_only()
}

/// The path the row shows for `file`: the full absolute path.
fn shown(file: &std::path::Path) -> String {
    photocraft_engine::doc_path::resolve_in(&file.to_string_lossy(), None).unwrap().display().to_string()
}

#[test]
fn a_linked_smart_object_shows_its_kind_path_read_only_size_and_actions() {
    let (_directory, file) = png("small-100x60-72.png");
    let h = panel("file.placeLinked", &file);
    assert!(h.query_by_label("Linked Smart Object").is_some(), "the header names the kind");
    // W and H are read-only, and show the size; X and Y stay editable.
    assert!(read_only(&h, "Width") && read_only(&h, "Height"), "read-only W/H");
    for (label, value) in [("Width", "100"), ("Height", "60")] {
        let shown_value = h.get_by_label(label).accesskit_node().value().map(|v| v.to_string()).unwrap_or_default();
        assert!(shown_value.contains(value), "{label}: {shown_value}");
    }
    assert!(h.query_by_label("Maintain aspect ratio").is_none(), "nothing to link");
    assert!(!read_only(&h, "X position") && !read_only(&h, "Y position"));
    // The full absolute path, below the size.
    let path = shown(&file);
    assert!(std::path::Path::new(&path).is_absolute() && path.ends_with("small-100x60-72.png"), "{path}");
    let row = h.get_by_label(&path).rect();
    assert!(row.top() >= h.get_by_label("Height").rect().bottom(), "{row:?} below the size");
    assert!(h.query_by_label(MISSING_MARK).is_none());
    assert_eq!(quick_actions_shown(&h), ["Edit Contents", EMBED_BUTTON, "Convert to Layers"]);
    assert!(!embed_greyed(&h));
}

#[test]
fn an_embedded_smart_object_keeps_its_header_fields_and_actions() {
    let (_directory, file) = png("small-100x60-72.png");
    let h = panel("file.placeEmbedded", &file);
    assert!(h.query_by_label("Linked Smart Object").is_none());
    assert!(h.query_by_label("Smart Object Layer").is_some());
    assert!(!read_only(&h, "Width") && !read_only(&h, "Height"), "W/H fields");
    assert!(h.query_by_label("Maintain aspect ratio").is_some());
    assert!(h.query_by_label(&shown(&file)).is_none(), "no path row");
    assert_eq!(quick_actions_shown(&h), ["Edit Contents", "Convert to Layers"]);
}

/// A PSD's placed layer keeps the uuid of its PSD file-table entry as its path; it is not a file
/// on disk, so the panel shows it as it did before.
#[test]
fn a_psd_placed_layer_is_not_shown_as_a_linked_file() {
    let (_directory, file) = png("small-100x60-72.png");
    let mut h = panel("file.placeEmbedded", &file);
    {
        let st = h.state_mut().session.active_mut().unwrap();
        let id = st.active_layer.unwrap();
        let LayerContent::Smart(sm) = &mut std::sync::Arc::make_mut(st.doc_mut()).layer_mut(id).unwrap().content else { panic!("a smart object") };
        sm.source = SmartSource::Linked { path: "a1b2c3d4-0000-1111-2222-333344445555".into() };
        sm.psd_raw = Some(std::sync::Arc::new(vec![0; 4]));
    }
    frames(&mut h, 4);
    assert!(h.query_by_label("Linked Smart Object").is_none());
    assert!(!read_only(&h, "Width"));
    assert!(h.query_by_label_contains("a1b2c3d4").is_none());
    assert!(!quick_actions_shown(&h).contains(&EMBED_BUTTON.to_string()));
}

#[test]
fn embed_makes_the_linked_smart_object_embedded() {
    let (_directory, file) = png("small-100x60-72.png");
    let mut h = panel("file.placeLinked", &file);
    h.get_by_label(EMBED_BUTTON).click();
    frames(&mut h, 4);
    let LayerContent::Smart(sm) = &active(&h).content else { panic!("still a smart object") };
    assert!(matches!(&sm.source, SmartSource::Embedded { file_name, .. } if file_name == "small-100x60-72.png"), "{:?}", sm.source);
    assert!(h.query_by_label("Smart Object Layer").is_some(), "an embedded smart object now");
    assert!(!read_only(&h, "Width"), "W/H are fields again");
    assert!(h.query_by_label(&shown(&file)).is_none());
    // One undo step puts the link back.
    h.state_mut().run("edit.undo", json!({})).unwrap();
    frames(&mut h, 4);
    assert!(h.query_by_label(&shown(&file)).is_some());
}

#[test]
fn a_missing_linked_file_marks_the_path_and_greys_embed_until_it_is_back() {
    let (directory, file) = png("small-100x60-72.png");
    let mut h = panel("file.placeLinked", &file);
    let path = shown(&file);
    let look = |h: &mut Harness<'_, Box<PhotocraftApp>>| {
        h.state_mut().run("layer.smartObjects.updateChanged", json!({})).unwrap();
        frames(h, 4);
    };
    look(&mut h);
    assert!(h.query_by_label(MISSING_MARK).is_none());
    assert!(!embed_greyed(&h));

    // Renamed while the document is open.
    let away = directory.path().join("renamed.png");
    std::fs::rename(&file, &away).unwrap();
    look(&mut h);
    let mark = h.get_by_label(MISSING_MARK).rect();
    let row = h.get_by_label(&path).rect();
    assert!(mark.right() <= row.left() + 1.0 && mark.bottom() >= row.top() && mark.top() <= row.bottom(), "{mark:?} before {row:?}");
    assert!(embed_greyed(&h), "Embed is greyed");
    assert_eq!(quick_actions_shown(&h), ["Edit Contents", EMBED_BUTTON, "Convert to Layers"], "still there, in order");
    h.get_by_label(EMBED_BUTTON).click();
    frames(&mut h, 4);
    let LayerContent::Smart(sm) = &active(&h).content else { panic!("still a smart object") };
    assert!(matches!(sm.source, SmartSource::Linked { .. }), "a greyed Embed does nothing");
    assert!(read_only(&h, "Width"), "the size stays read-only");

    // Renamed back: the normal display returns with nothing else done.
    std::fs::rename(&away, &file).unwrap();
    look(&mut h);
    assert!(h.query_by_label(MISSING_MARK).is_none());
    assert!(!embed_greyed(&h));
}
