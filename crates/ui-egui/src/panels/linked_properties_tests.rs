//! #14121: the Properties panel tells a linked smart object apart, as Photoshop does: its header
//! reads "Linked Smart Object", W and H are read-only text while X and Y still move it, the linked
//! file's full path sits below Transform, and its Quick Actions are Edit Contents, Embed and
//! Convert to Layers. With the file missing the path gets a red mark and Embed is greyed, until the
//! file is back.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, SmartSource};

use super::*;

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
/// `file.placeEmbedded`) at 150, 120, at its own size, with the Properties header and body of the
/// active layer drawn in a 280 px wide panel.
fn panel(command: &str, file: &std::path::Path) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run(command, json!({"path": file.to_string_lossy(), "center": [200, 150], "fit": false})).unwrap();
    let mut h = Harness::builder().with_size(vec2(280.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().cumulative_pass_nr() == 0 {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                return;
            }
            let state = app.session.active().unwrap();
            let layer = state.doc.layer(state.active_layer.unwrap()).unwrap().clone();
            crate::props_layout::header(ui, &layer);
            properties(app, ui, &layer);
        },
        app,
    );
    h.run_steps(3);
    h
}

/// The active layer of the active document.
fn active(h: &Harness<'_, PhotocraftApp>) -> photocraft_doc::Layer {
    let st = h.state().session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
}

/// The active layer's box as the Transform section shows it: [x, y, w, h].
fn bounds(h: &mut Harness<'_, PhotocraftApp>) -> [i32; 4] {
    let layer = active(h);
    let r = h.state_mut().layer_frame_bounds(&layer, layer.surface().unwrap());
    [r.x0, r.y0, r.width() as i32, r.height() as i32]
}

/// The Quick Actions buttons drawn, in reading order (top to bottom, then left to right).
fn quick_actions_shown(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    let names = ["Edit Contents", "Embed", "Convert to Layers", "Rasterize Smart Object", "Convert to Linked…"];
    let mut found: Vec<(f32, f32, String)> =
        names.iter().filter_map(|n| h.query_by_label(n).map(|node| (node.rect().top(), node.rect().left(), n.to_string()))).collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    found.into_iter().map(|(_, _, n)| n).collect()
}

/// Whether the button named `label` is greyed.
fn greyed(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

/// The painted galley of the text `text`: (rows, whether it was cut short with "…").
fn painted(h: &Harness<'_, PhotocraftApp>, text: &str) -> Option<(usize, bool)> {
    h.output().shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.job.text == text => Some((t.galley.rows.len(), t.galley.elided)),
        _ => None,
    })
}

/// The path the row shows for `file`: the full absolute path.
fn shown(file: &std::path::Path) -> String {
    photocraft_engine::doc_path::resolve_in(&file.to_string_lossy(), None).unwrap().display().to_string()
}

#[test]
fn a_linked_smart_object_shows_its_kind_path_read_only_size_and_actions() {
    let (_directory, file) = png("small-100x60-72.png");
    let mut h = panel("file.placeLinked", &file);
    assert_eq!(bounds(&mut h), [150, 120, 100, 60]);
    let name = active(&h).name;
    assert!(h.query_by_label(&format!("{name} — Linked Smart Object")).is_some(), "the header names the kind");
    // W and H are text, not fields; X and Y stay fields.
    assert!(h.query_by_label("Width 100 px").is_some() && h.query_by_label("Height 60 px").is_some());
    assert!(h.query_by_label("Width").is_none() && h.query_by_label("Height").is_none(), "no W/H fields");
    assert!(h.query_by_label("Link width and height").is_none(), "nothing to link");
    assert!(h.query_by_label("X").is_some() && h.query_by_label("Y").is_some());
    // The full absolute path, below Transform and above Align and Distribute.
    let path = shown(&file);
    assert!(std::path::Path::new(&path).is_absolute() && path.ends_with("small-100x60-72.png"), "{path}");
    let row = h.get_by_label(&path).rect();
    let y = h.get_by_label("Height 60 px").rect().bottom();
    let align = crate::props_layout::sections_drawn(&h.ctx).into_iter().find(|(t, _)| t == "Align and Distribute").unwrap().1;
    assert!(row.top() > y && row.bottom() <= align.top(), "{row:?} between the size ({y}) and Align ({align:?})");
    assert_eq!(painted(&h, &path).map(|(_, elided)| elided), Some(false), "shown whole");
    assert!(h.query_by_label(MISSING_MARK).is_none());
    assert_eq!(quick_actions_shown(&h), ["Edit Contents", "Embed", "Convert to Layers"]);
    assert!(!greyed(&h, "Embed"));
}

#[test]
fn an_embedded_smart_object_keeps_its_header_fields_and_actions() {
    let (_directory, file) = png("small-100x60-72.png");
    let h = panel("file.placeEmbedded", &file);
    let name = active(&h).name;
    assert!(h.query_by_label(&format!("{name} — Smart Object Layer")).is_some());
    assert!(h.query_by_label("Width").is_some() && h.query_by_label("Height").is_some(), "W/H fields");
    assert!(h.query_by_label("Width 100 px").is_none());
    assert!(h.query_by_label(&shown(&file)).is_none(), "no path row");
    assert_eq!(quick_actions_shown(&h), ["Edit Contents", "Convert to Layers", "Rasterize Smart Object"]);
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
        let LayerContent::Smart(sm) = &mut st.doc.layer_mut(id).unwrap().content else { panic!("a smart object") };
        sm.source = SmartSource::Linked { path: "a1b2c3d4-0000-1111-2222-333344445555".into() };
        sm.psd_raw = Some(std::sync::Arc::new(vec![0; 4]));
    }
    h.run_steps(3);
    let name = active(&h).name;
    assert!(h.query_by_label(&format!("{name} — Smart Object Layer")).is_some());
    assert!(h.query_by_label("Width").is_some());
    assert!(h.query_by_label_contains("a1b2c3d4").is_none());
    assert!(!quick_actions_shown(&h).contains(&"Embed".to_string()));
}

#[test]
fn x_and_y_still_move_a_linked_smart_object() {
    let (_directory, file) = png("small-100x60-72.png");
    let mut h = panel("file.placeLinked", &file);
    for (field, text) in [("X", "200"), ("Y", "40")] {
        h.get_by_label(field).click();
        h.run();
        for c in text.chars() {
            h.event(egui::Event::Text(c.to_string()));
            h.run();
        }
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
    }
    assert_eq!(bounds(&mut h), [200, 40, 100, 60], "moved, the same size");
    assert!(h.query_by_label("Width 100 px").is_some());
}

#[test]
fn embed_makes_the_linked_smart_object_embedded() {
    let (_directory, file) = png("small-100x60-72.png");
    let mut h = panel("file.placeLinked", &file);
    h.get_by_label("Embed").click();
    h.run_steps(3);
    let LayerContent::Smart(sm) = &active(&h).content else { panic!("still a smart object") };
    assert!(matches!(&sm.source, SmartSource::Embedded { file_name, .. } if file_name == "small-100x60-72.png"), "{:?}", sm.source);
    let name = active(&h).name;
    assert!(h.query_by_label(&format!("{name} — Smart Object Layer")).is_some(), "an embedded smart object now");
    assert!(h.query_by_label("Width").is_some(), "W/H are fields again");
    assert!(h.query_by_label(&shown(&file)).is_none());
    assert_eq!(quick_actions_shown(&h), ["Edit Contents", "Convert to Layers", "Rasterize Smart Object"]);
    // One undo step puts the link back.
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label(&shown(&file)).is_some());
}

#[test]
fn edit_contents_and_convert_to_layers_run_on_a_linked_smart_object() {
    let (_directory, file) = png("small-100x60-72.png");
    let mut h = panel("file.placeLinked", &file);
    let parent = h.state().session.active().unwrap().doc.id;
    h.get_by_label("Edit Contents").click();
    h.run_steps(3);
    assert_eq!(h.state().session.documents().len(), 2, "the contents open as a document");
    assert_ne!(h.state().session.active().unwrap().doc.id, parent);
    let i = h.state().session.documents().iter().position(|d| d.doc.id == parent).unwrap();
    h.state_mut().run("document.activate", json!({"document": i})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.id, parent);
    h.get_by_label("Convert to Layers").click();
    h.run_steps(3);
    assert!(matches!(active(&h).content, LayerContent::Raster(_)), "unpacked to a pixel layer");
    assert_eq!(bounds(&mut h), [150, 120, 100, 60], "where the smart object was");
}

#[test]
fn a_missing_linked_file_marks_the_path_and_greys_embed_until_it_is_back() {
    // A long name, so the path doesn't fit the panel on one line.
    let (directory, file) = png(&format!("{}-small-100x60-72.png", "linked".repeat(12)));
    let mut h = panel("file.placeLinked", &file);
    let path = shown(&file);
    let look = |h: &mut Harness<'_, PhotocraftApp>| {
        h.state_mut().run("layer.smartObjects.updateChanged", json!({})).unwrap();
        h.run_steps(3);
    };
    look(&mut h);
    assert!(h.query_by_label(MISSING_MARK).is_none());
    let (rows, elided) = painted(&h, &path).unwrap();
    assert!(rows > 1 && !elided, "the whole path, wrapped: {rows} rows");

    // Renamed while the document is open.
    let away = directory.path().join("renamed.png");
    std::fs::rename(&file, &away).unwrap();
    look(&mut h);
    let mark = h.get_by_label(MISSING_MARK).rect();
    let row = h.get_by_label(&path).rect();
    assert!(mark.right() <= row.left() + 20.0 && mark.top() >= row.top() - 1.0 && mark.bottom() <= row.bottom() + 1.0, "{mark:?} at the start of {row:?}");
    assert_eq!(painted(&h, &path), Some((1, true)), "cut short with …");
    assert!(greyed(&h, "Embed"), "Embed is greyed");
    assert_eq!(quick_actions_shown(&h), ["Edit Contents", "Embed", "Convert to Layers"], "still there, in order");
    h.get_by_label("Embed").click();
    h.run_steps(3);
    let LayerContent::Smart(sm) = &active(&h).content else { panic!("still a smart object") };
    assert!(matches!(sm.source, SmartSource::Linked { .. }), "a greyed Embed does nothing");
    assert!(h.query_by_label("Width 100 px").is_some(), "the size stays shown");

    // Renamed back: the normal display returns with nothing else done.
    std::fs::rename(&away, &file).unwrap();
    look(&mut h);
    assert!(h.query_by_label(MISSING_MARK).is_none());
    assert_eq!(painted(&h, &path).map(|(_, elided)| elided), Some(false));
    assert!(!greyed(&h, "Embed"));
}
