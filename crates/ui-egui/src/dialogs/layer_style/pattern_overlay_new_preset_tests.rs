//! #14969: Pattern Overlay copies its current pattern to the library without changing the
//! overlay or the pending document. Copies remain library presets after Cancel.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::Pattern;
use photocraft_engine::Session;
use photocraft_engine::preset_store::{self, MemBackend};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use super::*;

const COPY: &str = "Create a new preset from the current pattern";
const FIRST_ID: &str = "00000000-0000-0000-0000-000000000001";
const SECOND_ID: &str = "00000000-0000-0000-0000-000000000002";

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.layerStyle.patternOverlay", json!({"pattern": "Checkerboard"})).unwrap();
    app
}

fn harness(mut app: PhotocraftApp, page: &str) -> Harness<'static, PhotocraftApp> {
    open(&mut app, Some(page)).unwrap();
    let mut h = Harness::builder()
        .with_size(vec2(1200.0, 900.0))
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ProMedium))
        .build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    h.run_steps(4);
    h
}

fn copy_click(h: &mut Harness<'static, PhotocraftApp>) {
    h.get_by_label(COPY).click();
    h.run_steps(3);
}

fn same_contents(a: &Pattern, b: &Pattern) {
    assert_eq!((&a.name, a.width, a.height, a.surface.format()), (&b.name, b.width, b.height, b.surface.format()));
    assert_eq!(a.surface.read_region(a.rect()), b.surface.read_region(b.rect()));
}

fn current_pattern(app: &PhotocraftApp) -> String {
    app.ui.dialogs[0].fields["effects"][0]["params"]["pattern"].as_str().unwrap().to_owned()
}

#[test]
fn plus_button_follows_the_pattern_arrow_and_copies_without_a_name_dialog_or_selection_change() {
    let mut h = harness(app(), "patternOverlay");
    let control = h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Pattern");
    let plus = h.get_by_label(COPY);
    assert!(control.rect().right() <= plus.rect().left(), "the plus is right of the pattern arrow");
    let before_doc = h.state().session.active().unwrap().doc.clone();
    let source_id = current_pattern(h.state());
    let source = photocraft_engine::pattern_cmds::resolve(&h.state().session, &source_id).unwrap();
    let count = h.state().session.patterns.items.len();
    let dialog_id = h.state().ui.dialogs[0].id;
    copy_click(&mut h);
    assert_eq!(h.state().ui.dialogs.len(), 1, "no Pattern Name dialog opens");
    assert_eq!(h.state().ui.dialogs[0].id, dialog_id);
    assert_eq!(h.state().session.patterns.items.len(), count + 1);
    let copied = h.state().session.patterns.items.last().unwrap();
    assert_ne!(copied.id, source.id);
    same_contents(copied, &source);
    assert_eq!(current_pattern(h.state()), source_id);
    assert_eq!(h.state().session.active().unwrap().doc, before_doc);
    assert_eq!(h.state().session.journal.last().unwrap().0, "edit.definePattern");
    assert_eq!(h.state().session.journal.last().unwrap().1["fromPattern"], source_id);
    assert!(h.state().ui.dialogs[0].fields["patternList"].as_array().unwrap().iter().any(|row| row[0] == copied.id));
}

#[test]
fn two_clicks_append_two_distinct_presets_and_cancel_keeps_them_in_the_library_and_store() {
    let mem = MemBackend::default();
    let mut app = app();
    assert!(app.session.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
    let mut h = harness(app, "patternOverlay");
    let before_doc = h.state().session.active().unwrap().doc.clone();
    let source_id = current_pattern(h.state());
    let source = photocraft_engine::pattern_cmds::resolve(&h.state().session, &source_id).unwrap();
    let count = h.state().session.patterns.items.len();
    copy_click(&mut h);
    copy_click(&mut h);
    assert_eq!(h.state().session.patterns.items.len(), count + 2);
    let copies = h.state().session.patterns.items[count..].to_vec();
    assert_ne!(copies[0].id, copies[1].id);
    for copy in &copies {
        assert_ne!(copy.id, source.id);
        same_contents(copy, &source);
        assert!(h.state().session.presets.pattern_groups.iter().any(|group| group.items.contains(&copy.id)));
    }
    assert_eq!(current_pattern(h.state()), source_id);
    assert_eq!(h.state().ui.dialogs.len(), 1);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(h.state().session.active().unwrap().doc, before_doc);
    assert_eq!(h.state().session.patterns.items[count..], copies);
    let mut reopened = Session::new();
    assert!(reopened.attach_preset_store(preset_store::open(Box::new(mem))).is_empty());
    for copy in &copies {
        let restored = photocraft_engine::pattern_cmds::resolve(&reopened, &copy.id).unwrap();
        same_contents(&restored, copy);
    }
}

#[test]
fn an_empty_pattern_list_disables_the_button_without_changing_the_library() {
    let mut app = app();
    app.session.patterns.items.clear();
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).patterns.clear();
    let mut h = harness(app, "patternOverlay");
    assert!(h.get_by_label(COPY).accesskit_node().is_disabled());
    h.get_by_label(COPY).click();
    h.run_steps(3);
    assert!(h.state().session.patterns.items.is_empty());
    assert_eq!(h.state().ui.dialogs.len(), 1);
}

#[test]
fn invalid_copy_inputs_and_an_empty_source_leave_library_groups_and_document_unchanged() {
    let mut app = app();
    let empty = Pattern {
        id: "empty-pattern".into(),
        name: "Empty".into(),
        width: 0,
        height: 0,
        surface: Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, false)),
    };
    app.session.patterns.items.push(empty);
    let library = app.session.patterns.items.clone();
    let groups = serde_json::to_value(&app.session.presets.pattern_groups).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    for key in [Value::Null, json!(""), json!(" "), json!(false), json!(7), json!([]), json!({}), json!("missing-pattern"), json!("empty-pattern")] {
        assert!(app.run("edit.definePattern", json!({"fromPattern": key})).is_err());
        assert_eq!(app.session.patterns.items, library);
        assert_eq!(serde_json::to_value(&app.session.presets.pattern_groups).unwrap(), groups);
        assert_eq!(app.session.active().unwrap().doc, doc);
    }
    let no_doc_error = Session::new().execute("edit.definePattern", json!({})).unwrap_err().to_string();
    let copy_no_doc_error = Session::new().execute("edit.definePattern", json!({"fromPattern": "Checkerboard"})).unwrap_err().to_string();
    assert_eq!(copy_no_doc_error, no_doc_error);
}

fn collision_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let source = s.patterns.items[0].clone();
    s.patterns.items.push(Pattern { id: FIRST_ID.into(), ..source.clone() });
    std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).patterns.push(Pattern { id: SECOND_ID.into(), ..source });
    s
}

#[test]
fn fresh_copy_ids_are_independent_uuid_v4_values_and_skip_local_collisions() {
    let mut first = collision_session();
    let mut independent = collision_session();
    let source = first.patterns.items[0].id.clone();
    let params = json!({"fromPattern": source});
    let id = first.execute("edit.definePattern", params.clone()).unwrap()["pattern"].as_str().unwrap().to_owned();
    let other = independent.execute("edit.definePattern", params.clone()).unwrap()["pattern"].as_str().unwrap().to_owned();
    assert_ne!(id, other, "independent installations must create independent copy identities");
    let second = first.execute("edit.definePattern", params).unwrap()["pattern"].as_str().unwrap().to_owned();
    assert_ne!(second, id);
    for id in [&id, &other, &second] {
        assert!(![FIRST_ID, SECOND_ID, &source].contains(&id.as_str()));
        let parts = id.split('-').collect::<Vec<_>>();
        assert_eq!(parts.iter().map(|part| part.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12]);
        assert!(parts.iter().all(|part| part.chars().all(|c| c.is_ascii_hexdigit())));
        assert_eq!(parts[2].as_bytes()[0], b'4', "UUID version 4");
        assert!(matches!(parts[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'), "UUID RFC variant");
    }
}

fn tiled_pattern(sample: SampleType, width: u32) -> Pattern {
    let format = PixelFormat::new(ColorMode::Rgb, sample, true);
    let mut surface = Surface::new(format);
    let pixels = (0..width * 2).flat_map(|i| [if i % 3 == 0 { 1.0 } else { 0.0 }, (i % 2) as f32, 0.5, 0.75]).collect::<Vec<_>>();
    surface.write_region(Rect::new(0, 0, width as i32, 2), &pixels);
    Pattern::new("Same name", surface, width, 2)
}

#[test]
fn copying_preserves_source_name_dimensions_depth_alpha_and_exact_pixels() {
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let source = tiled_pattern(sample, 4);
        s.patterns.items.push(source.clone());
        let result = s.execute("edit.definePattern", json!({"fromPattern": source.id, "name": "ignored", "rect": [0, 0, 0, 0]})).unwrap();
        let copy = s.patterns.items.last().unwrap();
        same_contents(copy, &source);
        assert_ne!(copy.id, source.id);
        assert_eq!(result["info"]["name"], source.name);
        assert_eq!(s.patterns.items[s.patterns.items.len() - 2], source);
    }
}

fn stored_pattern() -> (MemBackend, Pattern) {
    let mem = MemBackend::default();
    let mut s = Session::new();
    assert!(s.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
    let source = tiled_pattern(SampleType::U8, 200);
    s.patterns.items.push(source.clone());
    s.sync_preset_store();
    (mem, source)
}

#[test]
fn a_stored_preview_is_copied_at_its_full_size_with_full_pixels() {
    let (mem, source) = stored_pattern();
    let mut s = Session::new();
    assert!(s.attach_preset_store(preset_store::open(Box::new(mem))).is_empty());
    assert_eq!(s.patterns.stored.get(&source.id), Some(&(200, 2)));
    assert!(s.patterns.items.iter().find(|p| p.id == source.id).unwrap().width < source.width);
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("edit.definePattern", json!({"fromPattern": source.id})).unwrap();
    same_contents(s.patterns.items.last().unwrap(), &source);
}

#[test]
fn failure_to_load_a_stored_source_does_not_append_a_copy_or_change_the_document() {
    let (mem, source) = stored_pattern();
    let index: Value = serde_json::from_slice(mem.files.lock().unwrap().get(preset_store::PATTERNS_FILE).unwrap()).unwrap();
    let file = index["patterns"].as_array().unwrap().iter().find(|p| p["id"] == source.id).unwrap()["file"].as_str().unwrap().to_owned();
    mem.files.lock().unwrap().remove(&file);
    let mut s = Session::new();
    assert!(s.attach_preset_store(preset_store::open(Box::new(mem))).is_empty());
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let library = s.patterns.items.clone();
    let doc = s.active().unwrap().doc.clone();
    let error = s.execute("edit.definePattern", json!({"fromPattern": source.id})).unwrap_err().to_string();
    assert!(error.contains("could not be loaded"), "{error}");
    assert_eq!(s.patterns.items, library);
    assert_eq!(s.active().unwrap().doc, doc);
}

#[test]
fn ordinary_define_keeps_selection_rect_name_and_same_content_deduplication() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6, "background": "#336699"})).unwrap();
    s.execute("select.rect", json!({"x": 1, "y": 2, "width": 3, "height": 2})).unwrap();
    let first = s.execute("edit.definePattern", json!({"name": "Selected"})).unwrap()["pattern"].as_str().unwrap().to_owned();
    let pattern = s.patterns.items.last().unwrap();
    assert_eq!((pattern.width, pattern.height, pattern.name.as_str()), (3, 2, "Selected"));
    let count = s.patterns.items.len();
    let second = s.execute("edit.definePattern", json!({"name": "Selected"})).unwrap();
    assert_eq!(second["pattern"], first);
    assert_eq!(s.patterns.items.len(), count);
    s.execute("edit.definePattern", json!({"name": "Rectangle", "rect": [0, 0, 2, 1]})).unwrap();
    let pattern = s.patterns.items.last().unwrap();
    assert_eq!((pattern.width, pattern.height, pattern.name.as_str()), (2, 1, "Rectangle"));
}

#[test]
fn non_pattern_overlay_pages_do_not_show_the_copy_button() {
    let h = harness(app(), "colorOverlay");
    assert!(h.query_by_label(COPY).is_none());
}

#[test]
fn a_copied_pattern_survives_reload_beside_an_unreadable_reserved_store_id() {
    let mem = MemBackend::default();
    let mut seed = Session::new();
    assert!(seed.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
    let reserved = Pattern { id: FIRST_ID.into(), name: "Retained unreadable pattern".into(), ..seed.patterns.items[2].clone() };
    seed.patterns.items.push(reserved);
    seed.sync_preset_store();
    let index: Value = serde_json::from_slice(mem.files.lock().unwrap().get(preset_store::PATTERNS_FILE).unwrap()).unwrap();
    let reserved_entry = index["patterns"].as_array().unwrap().iter().find(|entry| entry["id"] == FIRST_ID).unwrap().clone();
    let file = reserved_entry["file"].as_str().unwrap().to_owned();
    let unreadable = b"retained but unreadable PAT asset".to_vec();
    mem.files.lock().unwrap().insert(file.clone(), unreadable.clone());
    drop(seed);

    let mut s = Session::new();
    assert!(!s.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
    assert!(!s.patterns.items.iter().any(|pattern| pattern.id == FIRST_ID), "the reserved id is indexed but not loaded");
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let source = s.patterns.items[0].clone();
    let result = s.execute("edit.definePattern", json!({"fromPattern": source.id})).unwrap();
    let id = result["pattern"].as_str().unwrap().to_owned();
    s.sync_preset_store();
    assert_eq!(mem.files.lock().unwrap().get(&file), Some(&unreadable), "the unreadable asset remains untouched");
    drop(s);

    let mut reopened = Session::new();
    assert!(!reopened.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
    let copied = photocraft_engine::pattern_cmds::resolve(&reopened, &id).expect("the new copy must survive reload beside the retained unreadable asset");
    same_contents(&copied, &source);
    assert_ne!(id, FIRST_ID);
    assert!(!reopened.patterns.items.iter().any(|pattern| pattern.id == FIRST_ID));
    let index: Value = serde_json::from_slice(mem.files.lock().unwrap().get(preset_store::PATTERNS_FILE).unwrap()).unwrap();
    let entries = index["patterns"].as_array().unwrap();
    assert!(entries.contains(&reserved_entry), "the original unreadable pattern keeps its index entry");
    assert!(entries.iter().any(|entry| entry["id"] == id));
    assert_eq!(mem.files.lock().unwrap().get(&file), Some(&unreadable));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn copies_from_independent_installations_coexist_after_real_pat_export_and_import() {
    let mut first = Session::new();
    let mut second = Session::new();
    assert!(first.attach_preset_store(preset_store::open(Box::new(MemBackend::default()))).is_empty());
    assert!(second.attach_preset_store(preset_store::open(Box::new(MemBackend::default()))).is_empty());
    first.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    second.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert_eq!(first.patterns.items, second.patterns.items, "the installations start with the same pattern identities");
    assert!(first.active().unwrap().doc.patterns.is_empty());
    assert!(second.active().unwrap().doc.patterns.is_empty());
    let source_first = first.patterns.items[0].clone();
    let source_second = second.patterns.items[1].clone();
    let id_first = first.execute("edit.definePattern", json!({"fromPattern": source_first.id})).unwrap()["pattern"].as_str().unwrap().to_owned();
    let id_second = second.execute("edit.definePattern", json!({"fromPattern": source_second.id})).unwrap()["pattern"].as_str().unwrap().to_owned();

    let directory = std::env::temp_dir().join(format!("photocraft-pattern-copy-installations-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let first_path = directory.join("first.pat").to_string_lossy().into_owned();
    let second_path = directory.join("second.pat").to_string_lossy().into_owned();
    first.execute("pattern.export", json!({"path": first_path, "patterns": [id_first]})).unwrap();
    second.execute("pattern.export", json!({"path": second_path, "patterns": [id_second]})).unwrap();
    let mut combined = Session::new();
    assert!(combined.attach_preset_store(preset_store::open(Box::new(MemBackend::default()))).is_empty());
    combined.execute("pattern.import", json!({"path": first_path})).unwrap();
    same_contents(&photocraft_engine::pattern_cmds::resolve(&combined, &id_first).unwrap(), &source_first);
    combined.execute("pattern.import", json!({"path": second_path})).unwrap();
    std::fs::remove_file(&first_path).unwrap();
    std::fs::remove_file(&second_path).unwrap();
    std::fs::remove_dir(&directory).unwrap();

    let preserved_first = photocraft_engine::pattern_cmds::resolve(&combined, &id_first).unwrap();
    assert_eq!(preserved_first.name, source_first.name, "importing the second installation must preserve the first copied pattern name");
    same_contents(&preserved_first, &source_first);
    same_contents(&photocraft_engine::pattern_cmds::resolve(&combined, &id_second).unwrap(), &source_second);
    assert_ne!(id_first, id_second);
}
