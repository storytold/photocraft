use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamKind};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_engine::color_cmds::ColorExt;

use super::*;

const COLOR_LOOKUP: &str = "image.adjustments.colorLookup";
const LENS_BLUR: &str = "filter.blur.lensBlur";
const REPLACE_COLOR: &str = "image.adjustments.replaceColor";

fn new_app() -> Box<PhotocraftApp> {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn fields_of(app: &PhotocraftApp, id: DialogId) -> DialogFields {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("open dialog").fields().clone()
}

/// The dialog's values as the command's JSON params.
fn params_json(fields: &DialogFields) -> Value {
    serde_json::to_value(fields).expect("fields serialize")
}

fn field_json(app: &PhotocraftApp, id: DialogId, key: &str) -> Value {
    params_json(&fields_of(app, id)).get(key).cloned().unwrap_or(Value::Null)
}

fn try_set(app: &mut PhotocraftApp, id: DialogId, key: &str, value: Value) -> Result<(), String> {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, key, value)
}

fn set(app: &mut PhotocraftApp, id: DialogId, key: &str, value: Value) {
    try_set(app, id, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
}

fn passes(app: &mut PhotocraftApp, n: usize) {
    let ctx = app.ctx().clone();
    for _ in 0..n {
        crate::testing::run_ui(&ctx, Default::default(), |ui| crate::dialogs::run(app, ui));
    }
}

fn is_open(app: &PhotocraftApp, id: DialogId) -> bool {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).is_some()
}

/// OK, then the passes in which the family runs (or refuses, for a held OK). True when the dialog closed.
fn ok(app: &mut PhotocraftApp, id: DialogId) -> bool {
    let ctx = app.ctx().clone();
    // A refusal comes back as the Err; the dialog staying open is what the callers read.
    let _ = crate::dialogs::confirm(app, &ctx, id);
    passes(app, 3);
    !is_open(app, id)
}

fn cancel(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.cancel_by_id(&ctx, id);
    passes(app, 2);
}

fn history(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// The app drawn through `dialogs::run`, as the window shows its dialogs.
fn harness(app: Box<PhotocraftApp>, size: egui::Vec2) -> Harness<'static, Box<PhotocraftApp>> {
    let h =
        Harness::builder().with_context(app.ctx().clone()).with_size(size).build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h
}

fn top_dialog(app: &PhotocraftApp) -> DialogId {
    app.framework.dialogs.stack().top().expect("a dialog is open")
}

#[test]
fn color_settings_working_spaces_list_installed_profiles_of_their_space() {
    // #6323: a press condition installed in the system can be the CMYK working space, as in
    // Photoshop's Color Settings; each working-space menu lists only profiles of its space.
    use craft::cms::{ColorSpace, ProfileClass};
    let p = |path: &str, class, color_space| photocraft_engine::installed_profiles::InstalledProfile {
        path: path.into(),
        description: path.to_uppercase(),
        class,
        color_space,
    };
    let installed = [
        p("gracol.icc", ProfileClass::Output, ColorSpace::Cmyk),
        p("monitor.icc", ProfileClass::Display, ColorSpace::Rgb),
        p("printer-rgb.icc", ProfileClass::Output, ColorSpace::Rgb),
        p("scanner.icc", ProfileClass::Input, ColorSpace::Rgb),
        p("gray.icc", ProfileClass::Output, ColorSpace::Gray),
        p("link.icc", ProfileClass::DeviceLink, ColorSpace::Cmyk),
    ];
    let paths = |key: &str| rows::installed_choices("edit.colorSettings", key, &installed).into_iter().map(|(path, _)| path).collect::<Vec<_>>();
    assert_eq!(paths("workingCmyk"), ["gracol.icc"]);
    assert_eq!(paths("workingRgb"), ["monitor.icc"]);
    assert_eq!(paths("workingGray"), ["gray.icc"]);
    assert!(paths("policyCmyk").is_empty() && paths("intent").is_empty());
    assert_eq!(rows::installed_choices("edit.colorSettings", "workingCmyk", &installed)[0].1, "GRACOL.ICC", "listed by description");
    // The profile dialogs keep listing every destination profile.
    assert_eq!(rows::installed_choices("edit.convertToProfile", "profile", &installed).len(), 5);
    assert!(rows::installed_choices("filter.blur.gaussianBlur", "profile", &installed).is_empty());
}

#[test]
fn indexed_matte_tool_seeds_use_document_encoded_rgb() {
    use craft::cms::Transform;

    let mut session = photocraft_engine::Session::new();
    session.color_mut().settings.working_rgb = "display-p3".into();
    session.execute("file.new", json!({"width": 2, "height": 2, "background": "transparent"})).unwrap();
    session.execute("edit.assignProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    // The app holds the tool colours as a preferences section: they are set after it starts.
    app.run("tools.setColors", json!({"foreground": "#3366cc", "background": "#cc6633"})).unwrap();
    let colors = rows::indexed_matte_defaults(&app);

    let doc = app.session.active().unwrap().doc.clone();
    let source = app.session.color().working(photocraft_doc::ColorMode::Rgb);
    let destination = photocraft_engine::color_cmds::document_profile(&doc);
    let transform = Transform::new(&source, &destination, app.session.color().settings.intent(), app.session.color().settings.bpc).unwrap();
    for (value, input) in [("foreground", [0.2, 0.4, 0.8]), ("background", [0.8, 0.4, 0.2])] {
        let mut output = [0.0; 16];
        transform.eval(&input, &mut output).unwrap();
        let got = rows::indexed_matte_seed(&colors, value).unwrap_or_else(|| panic!("{value}: no seed"));
        for c in 0..3 {
            assert!((got[c] - output[c]).abs() < 1e-4, "{value}: channel {c} is {} want {}", got[c], output[c]);
        }
    }
}

#[test]
fn only_direction_angles_get_the_dial() {
    // Every listed command still has a degree `angle` row, so the list cannot go stale.
    let mut app = new_app();
    let mut has_dial = |cmd: &str| {
        let spec = photocraft_engine::commands::find(cmd).unwrap_or_else(|| panic!("{cmd} is not a command")).params;
        params(&mut app, cmd, spec).iter().any(|p| p.key == "angle" && matches!(p.kind, ParamKind::Angle { .. }))
    };
    for cmd in ANGLE_DIAL {
        assert!(has_dial(cmd), "{cmd} has no angle dial");
    }
    // Rotation amounts and angles with another zero keep their slider.
    for cmd in
        ["filter.distort.twirl", "image.rotation.arbitrary", "image.crop", "filter.blurGallery.tiltShift", "filter.lensCorrection", "filter.render.flame"]
    {
        assert!(!has_dial(cmd), "{cmd}");
    }
}

/// #5806: OK in Displace with no map asks for the map file, as Photoshop does, and displaces
/// with the chosen file; Cancel in the picker applies nothing; a map already given runs at once.
#[test]
fn displace_asks_for_its_map_after_ok() {
    let dir = std::env::temp_dir().join(format!("photocraft-displace-map-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let map = dir.join("map.png");
    let mut m = photocraft_engine::Session::new();
    m.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    std::fs::write(&map, photocraft_io::export(&m.active().unwrap().doc, "map.png", &Default::default()).unwrap().bytes).unwrap();
    // The first pick is answered with the map; the second has nothing queued, so it is cancelled.
    let files = std::sync::Arc::new(craft::host::ScriptedFiles::answering([craft::host::Picked::Paths(vec![map.clone()])]));
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { files: Some(files.clone()), ..Default::default() });
    app.run("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let steps = history(&app);
    let id = open(&mut app, "filter.distort.displace").unwrap();
    ok(&mut app, id);
    assert!(app.file_dialog_open(), "OK leaves the map picker pending");
    // Another document activated while the picker is open is left alone.
    app.run("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    let other_steps = app.session.documents()[1].history.past_len();
    // The frame acts on the picker's answer.
    app.poll_file_dialog();
    assert_eq!(files.requests().len(), 1, "OK asks for the map");
    assert_eq!(app.session.active_index(), Some(0), "the document that asked is shown");
    assert_eq!(history(&app), steps + 1, "Displace ran with the chosen map");
    assert_eq!(app.session.documents()[1].history.past_len(), other_steps, "not on the other document");
    // Cancelled: nothing is applied.
    let id = open(&mut app, "filter.distort.displace").unwrap();
    ok(&mut app, id);
    app.poll_file_dialog();
    assert_eq!(files.requests().len(), 2);
    assert_eq!(history(&app), steps + 1);
    // A map given in the dialog runs without asking.
    assert!(!needs_displace_map("filter.distort.displace", &json!({"mapPath": "/maps/x.psd"})));
    assert!(!needs_displace_map("filter.distort.displace", &json!({"mapDocument": 0})));
    assert!(needs_displace_map("filter.distort.displace", &json!({"mapDocument": -1, "mapPath": " "})));
    assert!(!needs_displace_map("filter.blur.gaussianBlur", &json!({})));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn remembering_dialogs_never_persists_paths_or_document_indices() {
    let dir = tempfile::tempdir().unwrap();
    let map = dir.path().join("map.png");
    let mut m = photocraft_engine::Session::new();
    m.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    std::fs::write(&map, photocraft_io::export(&m.active().unwrap().doc, "map.png", &Default::default()).unwrap().bytes).unwrap();
    let mut app = new_app();
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let id = open(&mut app, "filter.distort.displace").unwrap();
    set(&mut app, id, "horizontal", json!(7));
    set(&mut app, id, "mapPath", json!(map.to_str().unwrap()));
    set(&mut app, id, "mapDocument", json!("0"));
    assert!(ok(&mut app, id), "OK with a map given closes the dialog");
    let again = open(&mut app, "filter.distort.displace").unwrap();
    let f = fields_of(&app, again);
    assert_eq!(f.number("horizontal"), Some(7.0), "an ordinary value is remembered");
    assert!(f.text("mapPath").unwrap_or("").is_empty(), "a path is never remembered");
    assert_ne!(f.int("mapDocument"), Some(0), "a document index is never remembered");
}

/// #14465 review: the Rotate Canvas dialog's resampling follows Preferences › General › Image
/// Interpolation on every run. The reference application has no Interpolation row (#14462), so
/// the dialog files no interpolation, before or after an OK, and the command reads the
/// preference (`rotate_canvas_interpolation_pref_tests` in the engine). The other rows of the
/// dialog keep opening from the document (angle and direction are the ruler's, #12035).
#[test]
fn arbitrary_rotation_interpolation_reopens_at_the_preference() {
    const CMD: &str = "image.rotation.arbitrary";
    let mut app = new_app();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();

    let id = open(&mut app, CMD).unwrap();
    assert_eq!(field_json(&app, id, "interpolation"), Value::Null, "no interpolation value: the preference decides");
    set(&mut app, id, "angle", json!(30.0));
    assert!(ok(&mut app, id), "OK rotates and files its values");

    // A later opening files none either, whatever the preference says.
    app.session.edit_prefs(|p| p.general.image_interpolation = photocraft_engine::prefs::Interpolation::Nearest);
    let id = open(&mut app, CMD).unwrap();
    assert_eq!(field_json(&app, id, "interpolation"), Value::Null, "a reopened dialog files no interpolation");
    cancel(&mut app, id);
}

/// A layer-id param has no number field: the Auto-Align dialog used to write `reference: 0`
/// back, which the engine rejects as not a selected layer (#674).
#[test]
fn layer_id_params_stay_out_of_the_dialog() {
    let mut app = new_app();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let spec = photocraft_engine::commands::find("edit.autoAlignLayers").unwrap().params;
    assert!(!params(&mut app, "edit.autoAlignLayers", spec).iter().any(|p| p.key == "reference"), "no reference row");
    let id = open(&mut app, "edit.autoAlignLayers").expect("Auto-Align opens");
    assert!(!fields_of(&app, id).contains("reference"), "no reference field");
}

/// Layer › Layer Style › Global Light (#3054): the dialog opens with the active document's
/// shared lighting, keeps it on an unchanged OK, never takes a stale remembered value, tracks
/// document switching, applies edits and leaves the document alone on Cancel.
#[test]
fn global_light_dialog_reflects_active_document_light() {
    const CMD: &str = "layer.layerStyle.globalLight";
    let mut app = new_app();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let doc1_id = app.session.active().unwrap().doc.id;
    let light = |app: &PhotocraftApp| {
        let gl = app.session.active().unwrap().doc.global_light;
        (gl.angle, gl.altitude)
    };

    // 0. A fresh document starts at angle 120 and altitude 30.
    let d_default = open(&mut app, CMD).unwrap();
    assert_eq!(fields_of(&app, d_default).number("angle"), Some(120.0));
    assert_eq!(fields_of(&app, d_default).number("altitude"), Some(30.0));
    ok(&mut app, d_default);
    assert_eq!(light(&app), (120.0, 30.0));

    // 1. The document's light is changed to 30 / 45.
    app.run(CMD, json!({"angle": 30.0, "altitude": 45.0})).unwrap();
    assert_eq!(light(&app), (30.0, 45.0));

    // 2. The dialog opens with that light, and an unchanged OK keeps it.
    let d_id = open(&mut app, CMD).unwrap();
    assert_eq!(fields_of(&app, d_id).number("angle"), Some(30.0));
    assert_eq!(fields_of(&app, d_id).number("altitude"), Some(45.0));
    ok(&mut app, d_id);
    assert_eq!(light(&app), (30.0, 45.0));

    // 3. A value remembered from an earlier OK does not override the document.
    let d_id = open(&mut app, CMD).unwrap();
    set(&mut app, d_id, "angle", json!(75.0));
    set(&mut app, d_id, "altitude", json!(60.0));
    ok(&mut app, d_id);
    app.run(CMD, json!({"angle": 30.0, "altitude": 45.0})).unwrap();
    let d_id = open(&mut app, CMD).unwrap();
    assert_eq!(fields_of(&app, d_id).number("angle"), Some(30.0), "reads the active document, not the remembered 75");
    assert_eq!(fields_of(&app, d_id).number("altitude"), Some(45.0));

    // 4. Cancel keeps the document's light.
    set(&mut app, d_id, "angle", json!(10.0));
    cancel(&mut app, d_id);
    assert_eq!(light(&app), (30.0, 45.0));

    // 5. Each document opens the dialog at its own light.
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let doc2_id = app.session.active().unwrap().doc.id;
    assert_ne!(doc1_id, doc2_id);
    app.run(CMD, json!({"angle": -45.0, "altitude": 15.0})).unwrap();
    let d_id2 = open(&mut app, CMD).unwrap();
    assert_eq!(fields_of(&app, d_id2).number("angle"), Some(-45.0));
    assert_eq!(fields_of(&app, d_id2).number("altitude"), Some(15.0));
    cancel(&mut app, d_id2);
    assert!(app.session.set_active(0));
    let d_id1 = open(&mut app, CMD).unwrap();
    assert_eq!(fields_of(&app, d_id1).number("angle"), Some(30.0));
    assert_eq!(fields_of(&app, d_id1).number("altitude"), Some(45.0));

    // 6. An edit in the dialog updates the active document only.
    set(&mut app, d_id1, "angle", json!(-60.0));
    set(&mut app, d_id1, "altitude", json!(20.0));
    ok(&mut app, d_id1);
    assert_eq!(light(&app), (-60.0, 20.0));
    assert!(app.session.set_active(1));
    assert_eq!(light(&app), (-45.0, 15.0));
}

/// View › Proof Setup (#3055): Custom Proof Setup opens with the active document's proof
/// condition, keeps it on an unchanged OK, never takes a stale remembered value, tracks
/// document switching, applies edits and leaves the document alone on Cancel.
#[test]
fn proof_setup_dialog_reflects_active_document_proof_condition() {
    let mut app = new_app();
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let doc1_id = app.session.active().unwrap().doc.id;

    // 0. A fresh document defaults to working-cmyk.
    let d_fresh = open(&mut app, "view.proofSetup").unwrap();
    assert_eq!(fields_of(&app, d_fresh).text("profile"), Some("working-cmyk"));
    ok(&mut app, d_fresh);
    assert_eq!(app.session.color().proof(doc1_id).setup.name, "working-cmyk");

    // 1. A preset sets sRGB and turns Proof Colors on.
    crate::menus::invoke(&mut app, &ctx, "view.proofSetup.internetStandardRgb", Value::Null).unwrap();
    assert_eq!(app.session.color().proof(doc1_id).setup.name, "srgb");
    assert!(app.session.color().proof(doc1_id).enabled);

    // 2. The dialog starts with the document's condition; an unchanged OK keeps it and Proof Colors on.
    let d_id = open(&mut app, "view.proofSetup").unwrap();
    let f = fields_of(&app, d_id);
    assert_eq!(f.text("profile"), Some("srgb"));
    assert_eq!(f.text("intent"), Some("relative"));
    assert_eq!(f.bool("bpc"), Some(true));
    assert_eq!(f.bool("simulatePaper"), Some(false));
    ok(&mut app, d_id);
    let pv = app.session.color().proof(doc1_id);
    assert_eq!(pv.setup.name, "srgb");
    assert!(pv.enabled);

    // 3. A value remembered from an earlier OK does not override the document.
    let d_id = open(&mut app, "view.proofSetup").unwrap();
    set(&mut app, d_id, "profile", json!("display-p3"));
    ok(&mut app, d_id);
    app.run("view.proofSetup", json!({"profile": "srgb", "intent": "relative", "bpc": true, "simulatePaper": false})).unwrap();
    let d_id = open(&mut app, "view.proofSetup").unwrap();
    assert_eq!(fields_of(&app, d_id).text("profile"), Some("srgb"), "reads the active document, not the remembered one");

    // 4. Cancel keeps the document.
    set(&mut app, d_id, "profile", json!("display-p3"));
    cancel(&mut app, d_id);
    assert_eq!(app.session.color().proof(doc1_id).setup.name, "srgb");

    // 5. Each document opens the dialog at its own condition.
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let doc2_id = app.session.active().unwrap().doc.id;
    assert_ne!(doc1_id, doc2_id);
    app.run("view.proofSetup", json!({"profile": "display-p3", "intent": "perceptual", "bpc": false, "simulatePaper": true})).unwrap();
    let d_id2 = open(&mut app, "view.proofSetup").unwrap();
    let f2 = fields_of(&app, d_id2);
    assert_eq!(f2.text("profile"), Some("display-p3"));
    assert_eq!(f2.text("intent"), Some("perceptual"));
    assert_eq!(f2.bool("bpc"), Some(false));
    assert_eq!(f2.bool("simulatePaper"), Some(true));
    cancel(&mut app, d_id2);
    assert!(app.session.set_active(0));
    let d_id1 = open(&mut app, "view.proofSetup").unwrap();
    assert_eq!(fields_of(&app, d_id1).text("profile"), Some("srgb"));

    // 6. An edit in the dialog updates the active document only.
    set(&mut app, d_id1, "profile", json!("adobe-rgb-compat"));
    ok(&mut app, d_id1);
    assert_eq!(app.session.color().proof(doc1_id).setup.name, "adobe-rgb-compat");
    assert_eq!(app.session.color().proof(doc2_id).setup.name, "display-p3");
}

/// The Color Lookup dialog's own fields run the command: `fileName` used to be filled with `0`
/// (no dialog kind for `text?`, so every OK failed) and `lutFormat` with `cube`, which made a
/// .3dl file from the LUT folder fail to parse.
#[test]
fn color_lookup_dialog_defaults_run_a_3dl_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("inverse.3dl");
    let mut lut = String::from("3DMESH\nMesh 1 12\n0 512 1023\n");
    for r in 0..3 {
        for g in 0..3 {
            for b in 0..3 {
                let v = [r, g, b].map(|c| 4095 - 4095 * c / 2);
                lut.push_str(&format!("{} {} {}\n", v[0], v[1], v[2]));
            }
        }
    }
    std::fs::write(&path, lut).unwrap();
    let mut app = new_app();
    app.run("file.new", json!({"width": 4, "height": 4, "background": "#000000"})).unwrap();
    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    app.run(COLOR_LOOKUP, params_json(&fields_of(&app, id))).unwrap();
    set(&mut app, id, "file", json!(path.to_str().unwrap()));
    app.run(COLOR_LOOKUP, params_json(&fields_of(&app, id))).unwrap();
    let px = app.session.active().unwrap().doc.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 1, 1));
    assert!(px[0] > 0.99 && px[1] > 0.99 && px[2] > 0.99, "black through the inverse .3dl is white: {px:?}");
}

/// #9754: Image › Adjustments › Color Lookup lists the LUT folder's files (the framework
/// `LutPicker`), and one picked from it is applied through `file`.
#[test]
fn color_lookup_dialog_lists_and_applies_lut_folder_files() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("invert.cube");
    std::fs::write(&path, "LUT_3D_SIZE 2\n1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n").unwrap();
    let path = path.to_str().unwrap().to_string();
    let mut app = new_app();
    app.run("prefs.set", json!({"path": "fileHandling.lutFolder", "value": tmp.path().to_str().unwrap()})).unwrap();
    app.run("file.new", json!({"width": 4, "height": 4, "background": "#ff0000"})).unwrap();
    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    set(&mut app, id, "file", json!(path));
    assert_eq!(fields_of(&app, id).text("file"), Some(path.as_str()));
    assert!(ok(&mut app, id), "OK with the picked file closes the dialog");
    let px = app.session.active().unwrap().doc.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 1, 1));
    assert!(px[0] < 0.01 && px[1] > 0.99 && px[2] > 0.99, "the picked non-identity LUT was applied: {px:?}");
    assert_eq!(history(&app), 1);
}

/// #7400: Lens Blur's Source lists the document's alpha channels, each choosable even when two
/// share a name; like the other references it isn't remembered, so a channel source opens as None.
#[test]
fn lens_blur_source_lists_the_alpha_channels() {
    let mut app = new_app();
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app.run("channel.new", json!({"name": "Depth Map", "fill": "white"})).unwrap();
    app.run("channel.new", json!({"name": "Depth Map", "fill": "black"})).unwrap();
    let id = open(&mut app, LENS_BLUR).unwrap();
    assert!(!fields_of(&app, id).contains("depthChannel"), "depthChannel is no dialog field");
    // The menu's values are channel indices, so two channels with one name are both choosable.
    set(&mut app, id, "depthMap", json!("channel:0"));
    set(&mut app, id, "depthMap", json!("channel:1"));
    assert!(try_set(&mut app, id, "depthMap", json!("channel:2")).is_err(), "only the document's channels are offered");
    ok(&mut app, id);
    let again = open(&mut app, LENS_BLUR).unwrap();
    assert_eq!(fields_of(&app, again).text("depthMap"), Some("none"), "no channel was remembered with it");
}

#[test]
fn new_filters_have_dialogs_or_run_directly() {
    for (id, dialog) in [
        ("filter.stylize.oilPaint", true),
        ("filter.blur.lensBlur", true),
        ("filter.blurGallery.irisBlur", true),
        ("filter.other.custom", true),
        ("filter.distort.displace", true),
        ("filter.pixelate.mezzotint", true),
        ("filter.render.lightingEffects", true),
        ("filter.render.relight", true),
        ("filter.other.colorToAlpha", true),
        ("filter.blur.gaussianBlur", true),
        ("filter.pixelate.facet", false),
        ("filter.pixelate.fragment", false),
        ("filter.video.ntscColors", false),
        ("filter.stylize.findEdges", false),
    ] {
        assert_eq!(has_dialog(id), dialog, "{id}");
    }
    // Every filter spec parses, and every dialog command has plain parameter keys.
    for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("filter.") || has_dialog(c.id)) {
        for p in Param::parse_list(c.params) {
            assert!(p.key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_'), "{}: junk parameter {:?}", c.id, p.key);
        }
    }
}

#[test]
fn preview_runs_engine_command_on_proxy() {
    let mut app = new_app();
    app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 64})).unwrap();
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let revision = app.session.active().unwrap().revision;
    let id = open(&mut app, "filter.blur.gaussianBlur").unwrap();
    set(&mut app, id, "radius", json!(4.0));
    // The Preview box is on from the dialog's first drawn pass.
    passes(&mut app, 2);
    let ctx = app.ctx().clone();
    crate::canvas::ensure_filter_preview(&mut app, 0, &ctx);
    let shown = app.filter_preview.as_ref().and_then(|p| p.result.clone()).expect("the live preview ran");
    let p = shown.layers[0].surface().unwrap().pixel(32, 32);
    assert!(p[0] > 0.2 && p[0] < 0.8, "edge blurred: {p:?}");
    assert_eq!(app.session.active().unwrap().revision, revision, "the preview edits nothing");
}

/// #7423 (review on #7799): re-editing a Displace smart filter and picking another open
/// document as its map previews with that document, as OK applies it. The preview session held
/// only the edited document, and the map's `mapDocument` sits inside the filter's `params`.
#[test]
fn smart_displace_preview_uses_the_map_document() {
    let mut s = photocraft_engine::Session::new();
    // Documents 0 and 1: a white and a black map, opened before the edited one.
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "black"})).unwrap();
    s.execute("file.new", json!({"width": 48, "height": 32, "background": "transparent"})).unwrap();
    s.edit("square", |doc, active| {
        let fmt = doc.pixel_format();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(photocraft_geom::Rect::new(10, 10, 20, 12), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        Ok(())
    })
    .unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("filter.distort.displace", json!({"horizontal": 10, "vertical": 0, "mapDocument": 0})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let layer = s.active().unwrap().active_layer;
    let documents: Vec<_> = s.documents().iter().map(|d| d.doc.clone()).collect();
    for map in [1, 0] {
        let p = json!({"index": 0, "params": {"mapDocument": map}});
        let mut applied = photocraft_engine::Session::new();
        for d in &documents {
            applied.add_document((**d).clone(), None);
        }
        applied.execute(SMART_FILTER_SET_PARAMS, p.clone()).unwrap();
        let want = photocraft_compose::render(&applied.active().unwrap().doc, doc.bounds()).into_px();
        let preview = preview_document_for_target(
            &doc,
            layer,
            SMART_FILTER_SET_PARAMS,
            &p,
            1,
            None,
            photocraft_engine::channel_cmds::ChannelTarget::Composite,
            &documents,
            &ColorSettings::default(),
            None,
        )
        .unwrap_or_else(|| panic!("map {map}: no preview"));
        assert!(photocraft_compose::render(&preview, doc.bounds()).px == want, "map {map}: the preview differs from what OK applies");
    }
}

/// #10839: Layer is a visible choice tied to Source, never a remembered or name-based reference.
#[test]
fn match_color_source_layer_picker_keeps_references_with_the_source() {
    use photocraft_doc::{Color, ColorMode, Layer, SampleType, Size};

    let mut source = Document::with_background("reference", Size::new(16, 16), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    source.layers[0].name = "Sample".into();
    let bottom = source.layers[0].id;
    source.layers.push(Layer::raster("Sample", source.pixel_format()));
    let top = source.layers[1].id;
    // A different document can contain copied layer IDs. Changing Source must still reset Layer.
    let mut other = Document::with_background("other reference", source.size, source.mode, source.depth, Color::WHITE);
    other.layers = source.layers.clone();
    let mut session = photocraft_engine::Session::new();
    session.add_document(source, None);
    session.add_document(other, None);
    session.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut app = PhotocraftApp::new(session, Default::default());
    let id = open(&mut app, "image.adjustments.matchColor").unwrap();
    assert_eq!(fields_of(&app, id).text("source"), Some("-1"));
    assert!(field_json(&app, id, "sourceLayer").is_null(), "None starts with no layer reference");
    assert!(try_set(&mut app, id, "sourceLayer", json!(bottom)).is_err(), "Layer is disabled while Source is None");
    set(&mut app, id, "source", json!("0"));
    set(&mut app, id, "sourceLayer", json!(bottom));
    assert_eq!(field_json(&app, id, "sourceLayer"), json!(bottom), "the bottom layer is identified by ID");
    set(&mut app, id, "source", json!("1"));
    passes(&mut app, 2);
    assert!(field_json(&app, id, "sourceLayer").is_null(), "a new source returns to Merged even with copied IDs");
    set(&mut app, id, "sourceLayer", json!(top));
    assert_eq!(field_json(&app, id, "sourceLayer"), json!(top));
    set(&mut app, id, "source", json!("-1"));
    passes(&mut app, 2);
    assert_eq!(fields_of(&app, id).text("source"), Some("-1"));
    assert!(field_json(&app, id, "sourceLayer").is_null());

    // A confirmed source is never remembered.
    set(&mut app, id, "source", json!("0"));
    set(&mut app, id, "sourceLayer", json!(bottom));
    ok(&mut app, id);
    let reopened = open(&mut app, "image.adjustments.matchColor").unwrap();
    assert_eq!(fields_of(&app, reopened).text("source"), Some("-1"));
    assert!(field_json(&app, reopened, "sourceLayer").is_null(), "source references are never remembered");
}

/// Send an actual control-protocol request and require its synchronous reply.
fn match_color_control(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (request, _) = crate::control::ControlRequest::new(method, params);
    let crate::control::Reply::Done(reply) = crate::control::handle(app, ctx, &request) else { panic!("{method} should finish immediately") };
    reply
}

/// Held OK (K20): the refused OK leaves the dialog open and edits nothing.
#[test]
fn match_color_refuses_a_changed_destination_without_closing() {
    let mut app = new_app();
    for _ in 0..2 {
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    }
    let original = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({"name": "Other"})).unwrap();
    let other = app.session.active().unwrap().active_layer.unwrap();
    app.session.select_layer(original).unwrap();
    let ctx = app.ctx().clone();
    let id = open(&mut app, MATCH_COLOR).unwrap();
    let steps: Vec<_> = app.session.documents().iter().map(|st| st.history.past_len()).collect();
    let forged = json!((app.session.documents()[0].doc.id, app.session.documents()[0].active_layer));
    assert!(try_set(&mut app, id, MATCH_COLOR_DESTINATION, forged).is_err(), "the destination is not a field a caller sets");

    let refuse = |app: &mut PhotocraftApp, index: usize| {
        assert!(crate::canvas::ensure_filter_preview(app, index, &ctx).is_none());
        assert!(!ok(app, id), "refusal leaves the dialog open");
        assert_eq!(app.session.active_index(), Some(index), "refusal does not navigate");
        assert_eq!(app.session.documents().iter().map(|st| st.history.past_len()).collect::<Vec<_>>(), steps);
    };
    match_color_control(&mut app, &ctx, "ui.window.open", json!({"document": 0}));
    refuse(&mut app, 0);
    match_color_control(&mut app, &ctx, "ui.window.open", json!({"document": 1}));
    app.session.select_layer(other).unwrap();
    refuse(&mut app, 1);

    app.session.select_layer(original).unwrap();
    // A `layer` parameter that names another layer than the active one is refused as a changed destination.
    let destination = crate::dialogs::params::captured(&app, MATCH_COLOR);
    let named = |layer| json!({ "layer": layer }).as_object().cloned().unwrap();
    assert!(validate_match_color_destination(&app, MATCH_COLOR, &destination, &named(other.0)).is_err());
    assert!(validate_match_color_destination(&app, MATCH_COLOR, &destination, &named(original.0)).is_ok());
    assert!(ok(&mut app, id), "the matching destination is accepted");
}

#[test]
fn match_color_explicit_unavailable_source_layer_is_refused_with_or_without_a_frame() {
    for render in [false, true] {
        let mut app = new_app();
        for _ in 0..2 {
            app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        }
        let ctx = app.ctx().clone();
        let id = open(&mut app, MATCH_COLOR).unwrap();
        let missing = u64::MAX;
        // PLAN: assumed the `sourceLayer` row accepts any layer id; the held OK checks it exists.
        set(&mut app, id, "source", json!("0"));
        set(&mut app, id, "sourceLayer", json!(missing));
        if render {
            // Force a source-choice refresh as well as drawing the layer picker.
            app.session.set_active(0);
            app.session.execute("layer.new.layer", json!({"name": "New source layer"})).unwrap();
            app.session.set_active(1);
            passes(&mut app, 1);
            assert_eq!(field_json(&app, id, "sourceLayer"), json!(missing), "drawing keeps an unavailable reference");
            assert!(crate::canvas::ensure_filter_preview(&mut app, 1, &ctx).is_none());
        }
        let steps = history(&app);
        assert!(!ok(&mut app, id), "render={render}: the missing layer refuses OK and the dialog stays open");
        assert_eq!(history(&app), steps, "render={render}");
    }
}

/// Exercise actual nested command dialogs, rather than mutating the source behind the UI's gates.
fn match_color_after_source_change(change: &str) {
    use photocraft_doc::{Color, ColorMode, Layer, SampleType, Size};
    use photocraft_geom::Rect;
    let area = Rect::new(0, 0, 8, 8);
    let make = |name| Document::with_background(name, Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut source = make("reference");
    source.layers[0].surface_mut().unwrap().fill_rect(area, &[0.9, 0.2, 0.1, 1.0]);
    let mut reference = Layer::raster("blue reference", source.pixel_format());
    reference.surface_mut().unwrap().fill_rect(area, &[0.1, 0.3, 0.9, 1.0]);
    let reference_id = reference.id;
    source.layers.push(reference);
    let mut target = make("target");
    target.layers[0].surface_mut().unwrap().fill_rect(area, &[0.8, 0.6, 0.3, 1.0]);
    // A copied layer ID must never make a shifted source index identify the target document.
    target.layers[0].id = reference_id;
    let target_id = target.id;
    let mut session = photocraft_engine::Session::new();
    session.add_document(make("earlier"), None);
    session.add_document(source, None);
    session.select_layer(reference_id).unwrap();
    session.add_document(target, None);
    let mut app = PhotocraftApp::new(session, Default::default());
    app.sync_views();
    let ctx = app.ctx().clone();
    let root = open(&mut app, "image.adjustments.matchColor").unwrap();
    if !change.starts_with("select-") {
        set(&mut app, root, "source", json!("1"));
        set(&mut app, root, "sourceLayer", json!(reference_id));
    } else {
        assert_eq!(fields_of(&app, root).text("source"), Some("-1"), "{change}");
    }
    let (command, field, value) = if change == "delete-layer" {
        ("layer.delete", "layer", json!(reference_id))
    } else {
        ("file.close", "document", json!(if change == "close-source" { 1 } else { 0 }))
    };
    let params = Value::Object([(field.into(), value.clone())].into_iter().collect());
    assert_eq!(
        match_color_control(&mut app, &ctx, "engine.execute", json!({"command": command, "params": params}))["ok"],
        false,
        "{change}: ordinary edits are modal-blocked"
    );
    if change == "delete-layer" {
        match_color_control(&mut app, &ctx, "ui.window.open", json!({"document": 1}));
        assert_eq!(app.session.active_index(), Some(1), "{change}");
    }
    if change == "delete-layer" {
        // Deleting a layer from a panel asks first: the Delete prompt opens over Match Color and its Delete answer runs
        // the command.
        let prompt = crate::dialogs::delete_layer_prompt::intercept(&mut app, command, &json!({"layer": reference_id, "__trash": true}))
            .unwrap_or_else(|| panic!("{change}: {command} asks first"))
            .unwrap();
        let child = app
            .framework
            .dialogs
            .open_dialogs()
            .into_iter()
            .map(|dialog| dialog.id)
            .find(|id| prompt["dialog"].as_u64() == Some(id.value()))
            .expect("the Delete prompt is open");
        assert!(ok(&mut app, child), "{change}: the nested command is confirmed");
    } else {
        let child = open(&mut app, command).unwrap_or_else(|e| panic!("{change}: {command} opens over Match Color: {e}"));
        set(&mut app, child, field, value.clone());
        assert!(ok(&mut app, child), "{change}: the nested command is confirmed");
    }
    let target_index = app.session.documents().iter().position(|st| st.doc.id == target_id).unwrap();
    match_color_control(&mut app, &ctx, "ui.window.open", json!({"document": target_index}));
    assert_eq!(app.session.active().unwrap().doc.id, target_id, "{change}");
    if change == "delete-layer" {
        assert!(app.session.documents()[1].doc.layer(reference_id).is_none(), "the supported nested command really deleted the selected source layer");
        assert!(
            app.session.execute("image.adjustments.matchColor", json!({"source": 1, "sourceLayer": reference_id})).is_err(),
            "the explicit engine API still refuses missing IDs"
        );
        passes(&mut app, 1);
        assert!(field_json(&app, root, "sourceLayer").is_null(), "{change}: the displayed UI falls back to Merged");
    } else {
        assert_eq!(app.session.documents().len(), 2, "{change}: the supported nested command really closed a document");
    }
    if change == "select-with-frame" {
        passes(&mut app, 1);
    }
    if change.starts_with("select-") {
        assert_eq!(app.session.documents()[0].doc.name, "reference", "{change}: the explicitly chosen current index is the blue source");
        if change == "select-invalid" {
            // A row that refuses the index is as good as a refused OK: it never becomes another document.
            if try_set(&mut app, root, "source", json!("99")).is_err() {
                return;
            }
        } else {
            set(&mut app, root, "source", json!("0"));
        }
    }
    let closed = ok(&mut app, root);
    let st = app.session.active().unwrap();
    if change == "close-source" || change == "select-invalid" {
        assert!(!closed, "{change}: a closed or invalid source must not silently become another document");
        assert_eq!(st.history.past_len(), 0, "{change}");
    } else {
        assert!(closed, "{change}: OK applies");
        let pixel = st.doc.layers[0].surface().unwrap().rgba(4, 4);
        assert_eq!(pixel[2] > pixel[0], change != "delete-layer", "{change}: {pixel:?}");
        assert_eq!(st.history.past_len(), 1, "{change}");
    }
}

/// The source identity rules are one mechanism: every way the source can change gives the same answer.
#[test]
fn match_color_source_identity_follows_the_documents() {
    for change in ["delete-layer", "close-earlier", "close-source", "select-with-frame", "select-without-frame", "select-invalid"] {
        match_color_after_source_change(change);
    }
}

/// Match Color's preview reads the Source document from the open documents, as OK does: the
/// preview session held only the edited document, so a Source showed no change (#4851).
#[test]
fn match_color_preview_uses_the_source_document() {
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    let solid = |name: &str, c: [f32; 4]| {
        let mut d = Document::with_background(name, Size::new(32, 32), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        d.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 32, 32), &c);
        d.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 16, 32), &[c[0] * 0.5, c[1] * 0.5, c[2] * 0.5, 1.0]);
        d
    };
    // The source document is open before the edited one, so indices differ between sessions.
    let mut s = photocraft_engine::Session::new();
    s.add_document(solid("reference", [0.1, 0.3, 0.9, 1.0]), None);
    s.add_document(solid("texture", [0.9, 0.5, 0.2, 1.0]), None);
    let doc = s.active().unwrap().doc.clone();
    let layer = doc.layers[0].id;
    let documents: Vec<_> = s.documents().iter().map(|d| d.doc.clone()).collect();
    for (source, source_layer, label) in [
        (0, None, "another document"),
        (1, None, "the edited document itself"),
        (0, Some(documents[0].layers[0].id), "another document's layer"),
        (1, Some(layer), "the edited document's layer"),
    ] {
        let mut p = json!({"source": source});
        if let Some(layer) = source_layer {
            p["sourceLayer"] = json!(layer);
        }
        let mut applied = photocraft_engine::Session::new();
        for d in &documents {
            applied.add_document((**d).clone(), None);
        }
        applied.execute("image.adjustments.matchColor", p.clone()).unwrap();
        let want = applied.active().unwrap().doc.layers[0].surface().unwrap().pixel(24, 16);
        let preview = preview_document_for_target(
            &doc,
            Some(layer),
            "image.adjustments.matchColor",
            &p,
            1,
            None,
            photocraft_engine::channel_cmds::ChannelTarget::Composite,
            &documents,
            &ColorSettings::default(),
            None,
        )
        .unwrap_or_else(|| panic!("{label}: no preview"));
        let got = preview.layers[0].surface().unwrap().pixel(24, 16);
        assert!((0..3).all(|c| (got[c] - want[c]).abs() < 1.0 / 255.0 + 1e-4), "{label}: preview {got:?}, OK gives {want:?}");
    }
    // The preview is of the matched image: the orange texture took the reference's blue.
    let preview = preview_document_for_target(
        &doc,
        Some(layer),
        "image.adjustments.matchColor",
        &json!({"source": 0}),
        2,
        None,
        photocraft_engine::channel_cmds::ChannelTarget::Composite,
        &documents,
        &ColorSettings::default(),
        None,
    )
    .unwrap();
    let px = preview.layers[0].surface().unwrap().pixel(12, 8);
    assert!(px[2] > px[0], "proxy preview matched to the reference: {px:?}");
}

#[test]
fn core_filter_previews_keep_minimum_settings_after_downsampling() {
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    use photocraft_geom::Rect;

    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = Document::with_background("preview", Size::new(32, 32), ColorMode::Rgb, depth, Color::WHITE);
        let layer = doc.layers[0].id;
        doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 16, 32), &[0.0, 0.0, 0.0, 1.0]);
        for (suffix, params) in [
            ("blur.boxBlur", json!({"radius":1})),
            ("blur.motionBlur", json!({"distance":1})),
            ("blur.surfaceBlur", json!({"radius":1})),
            ("noise.median", json!({"radius":1})),
            ("noise.dustAndScratches", json!({"radius":1})),
            ("pixelate.mosaic", json!({"cellSize":2})),
            ("stylize.emboss", json!({"height":1})),
            ("other.minimum", json!({"radius":0.2})),
            ("other.maximum", json!({"radius":0.2})),
            ("distort.wave", json!({"wavelengthMin":1,"wavelengthMax":2,"amplitudeMin":1,"amplitudeMax":1})),
        ] {
            let command = format!("filter.{suffix}");
            for k in [2, 4] {
                let proxy = crate::proxy::proxy_document(&doc, k);
                let expected = preview_document(&proxy, Some(layer), &command, &params, 1).unwrap();
                let out = preview_document(&doc, Some(layer), &command, &params, k).unwrap_or_else(|| panic!("{command}: {depth:?}, proxy factor {k}"));
                assert_eq!(
                    out.layers[0].surface().unwrap().read_region(proxy.bounds()),
                    expected.layers[0].surface().unwrap().read_region(proxy.bounds()),
                    "{command}: {depth:?}, proxy factor {k}"
                );
            }
        }
    }
}

/// A serde type error from numbers the dialog sends as JSON floats (`1.0`) for an integer
/// field ("invalid type: floating point `1.0`, expected u32"), as opposed to a legitimate
/// refusal such as "no selection" or "needs a map".
fn float_type_error(e: &str) -> bool {
    e.contains("invalid type: floating point")
}

/// #2501: every schema dialog, opened with its default fields, runs its command (OK) and its
/// live preview without a type error from integer fields sent as floats.
#[test]
fn every_dialog_default_runs_without_float_type_errors() {
    let ids: Vec<&str> = photocraft_engine::command_specs().iter().map(|c| c.id).filter(|id| has_dialog(id)).collect();
    assert!(ids.len() > 100, "{} dialogs", ids.len());
    let mut failures = Vec::new();
    for id in ids {
        let mut app = new_app();
        app.run("file.new", json!({"width": 24, "height": 24})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let Ok(dialog) = open(&mut app, id) else { continue };
        // PLAN: the params are the dialog's fields as JSON; dialogs-w24's `run_command` builds the same map.
        let params = params_json(&fields_of(&app, dialog));
        let st = app.session.active().unwrap();
        let (doc, active) = ((*st.doc).clone(), st.active_layer);
        if let Err(e) = app.session.execute(id, params.clone()) {
            let e = e.to_string();
            if float_type_error(&e) {
                failures.push(format!("{id} OK {params}: {e}"));
            }
        }
        // The preview swallows the error; rerun its params at full size to see it.
        if preview_document(&doc, active, id, &params, 1).is_none() {
            let mut s = photocraft_engine::Session::new();
            s.add_document(doc, None);
            if let Some(a) = active {
                s.select_layer(a).unwrap();
            }
            if let Err(e) = s.execute(id, params.clone()).map_err(|e| e.to_string())
                && float_type_error(&e)
            {
                failures.push(format!("{id} preview {params}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn preview_stays_inside_the_selection_at_every_proxy_factor() {
    use photocraft_doc::SampleType;
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc =
            Document::with_background("p", photocraft_doc::Size::new(64, 64), photocraft_doc::ColorMode::Rgb, depth, photocraft_doc::Color::gray(0.5));
        let bg = doc.layers[0].id;
        let mut sel = photocraft_raster::Surface::new(photocraft_doc::PixelFormat::GRAY8);
        sel.fill_rect(photocraft_geom::Rect::new(0, 0, 32, 64), &[1.0]);
        doc.selection = Some(sel);
        for k in [1, 2, 4] {
            let out = preview_document(&doc, Some(bg), "filter.noise.addNoise", &json!({"amount": 100.0}), k).unwrap();
            let s = out.layers[0].surface().unwrap();
            let half = 32 / k as i32;
            let changed = |x0: i32, x1: i32| (0..64 / k as i32).any(|y| (x0..x1).any(|x| s.pixel(x, y) != doc.layers[0].surface().unwrap().pixel(0, 0)));
            assert!(changed(0, half), "{depth:?} k={k}: noise inside the selection");
            assert!(!changed(half, 64 / k as i32), "{depth:?} k={k}: nothing outside the selection");
        }
    }
}

/// Edit › Define Pattern… asks for a name, as Photoshop's Pattern Name dialog does: it starts
/// from the document's name, never from a remembered one, and is numbered past the library.
#[test]
fn define_pattern_asks_for_a_name() {
    let mut app = new_app();
    app.run("file.new", json!({"width": 8, "height": 8, "name": "ask.jpg"})).unwrap();
    let define = |app: &mut PhotocraftApp, name: Option<&str>| {
        let id = open(app, "edit.definePattern").expect("Define Pattern opens a dialog");
        let shown = fields_of(app, id).text("name").unwrap().to_string();
        if let Some(n) = name {
            set(app, id, "name", json!(n));
        }
        assert!(ok(app, id));
        shown
    };
    assert_eq!(define(&mut app, Some("Bricks")), "ask", "prefilled with the document's name");
    assert_eq!(define(&mut app, None), "ask", "Bricks didn't take it");
    assert_eq!(define(&mut app, None), "ask 2", "numbered past the library");
    let names: Vec<&str> = app.session.patterns.items.iter().map(|p| p.name.as_str()).collect();
    for n in ["Bricks", "ask", "ask 2"] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
}

/// A filtered test image with structure at every scale (low-frequency waves plus a ramp).
fn waves(n: u32) -> Document {
    let mut doc = Document::with_background(
        "w",
        photocraft_doc::Size::new(n, n),
        photocraft_doc::ColorMode::Rgb,
        photocraft_doc::SampleType::U8,
        photocraft_doc::Color::WHITE,
    );
    let s = doc.layers[0].surface_mut().unwrap();
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let v = 0.5 + 0.4 * (x as f32 / 23.0).sin() * (y as f32 / 31.0).cos();
            s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &[v, 1.0 - v, x as f32 / n as f32, 1.0]);
        }
    }
    doc
}

/// Mean absolute RGB difference of two documents, compared at 1/4 scale.
fn diff_at_quarter(a: &Document, b: &Document) -> f32 {
    let quarter = |d: &Document| photocraft_compose::flatten(&crate::proxy::proxy_document(d, (d.size.width / 64).max(1)));
    let (a, b) = (quarter(a), quarter(b));
    assert_eq!(a.px.len(), b.px.len());
    a.px.iter().zip(&b.px).map(|(p, q)| (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>() / 3.0).sum::<f32>() / a.px.len() as f32
}

/// #2063: these filters drew their features in proxy pixels (noise, dots, dither, strokes,
/// kernels, scan lines, contour lines, streaks, depths, a tree's segments), so a 1/4 preview
/// showed them 4x larger than OK applied them. Their preview now runs at full resolution and
/// matches the applied result. Dust & Scratches (#3265) is the mirror image of those: a proxy
/// averages away the very specks the filter exists to remove, so its preview showed an
/// untouched image at a blocky scale.
#[test]
fn fixed_size_filters_preview_at_the_scale_they_apply() {
    let doc = sharp(256);
    let gray = grayscale(&doc);
    let edge = json!([0, 0, 0, 0, 0, 0, 0, -1, 0, 0, 0, -1, 4, -1, 0, 0, 0, -1, 0, 0, 0, 0, 0, 0, 0]);
    for (cmd, p) in [
        ("filter.distort.ripple", json!({"amount": 300.0, "size": "large"})),
        ("filter.stylize.diffuse", json!({"mode": "normal"})),
        ("filter.stylize.oilPaint", json!({})),
        ("filter.render.flame", json!({})),
        ("filter.render.fibers", json!({})),
        ("filter.pixelate.mezzotint", json!({"type": "mediumDots"})),
        ("filter.noise.addNoise", json!({"amount": 12.5})),
        ("filter.noise.dustAndScratches", json!({"radius": 4.0, "threshold": 0.0})),
        ("filter.noise.reduceNoise", json!({})),
        ("filter.other.custom", json!({"kernel": edge})),
        // #10474: Minimum and Maximum work on whole-pixel radii (`round()` / `floor()`), so at
        // k = 4 a typed 1.5 px radius previewed as 0.375 px = 0 px: the proxy showed the untouched
        // document while OK writes a real 1.5 / 1 px effect. A radius under 2 px previews at k = 1.
        ("filter.other.minimum", json!({"radius": 1.5, "preserve": "squareness"})),
        ("filter.other.minimum", json!({"radius": 1.5, "preserve": "roundness"})),
        ("filter.other.maximum", json!({"radius": 1.5, "preserve": "squareness"})),
        ("filter.other.maximum", json!({"radius": 1.5, "preserve": "roundness"})),
        ("filter.video.deInterlace", json!({})),
        ("filter.stylize.traceContour", json!({})),
        ("filter.stylize.wind", json!({"method": "blast"})),
        ("filter.stylize.extrude", json!({})),
        ("filter.render.tree", json!({"baseTreeType": 1})),
        ("filter.sharpen.sharpen", json!({})),
        ("filter.sharpen.sharpenMore", json!({})),
        ("filter.sharpen.sharpenEdges", json!({})),
        ("filter.stylize.findEdges", json!({})),
        ("filter.stylize.emboss", json!({})),
        ("filter.noise.despeckle", json!({})),
        ("filter.gallery.graphicPen", json!({})),
        ("filter.gallery.stainedGlass", json!({"cellSize": 10})),
        ("image.mode.indexedColor", json!({"colors": 16})),
        ("image.mode.bitmap", json!({"method": "diffusion"})),
    ] {
        let doc = if cmd == "image.mode.bitmap" { &gray } else { &doc };
        let bg = doc.layers[0].id;
        let applied = preview_document(doc, Some(bg), cmd, &p, 1).unwrap();
        let old = preview_document(doc, Some(bg), cmd, &p, 4).unwrap();
        assert!(diff_at_quarter(&applied, &old) > 0.005, "{cmd}: a 1/4 proxy differs from the result");
        let k = preview_factor(cmd, &p, 4);
        assert_eq!(k, 1, "{cmd}");
        let shown = preview_document(doc, Some(bg), cmd, &p, k).unwrap();
        assert!(diff_at_quarter(&applied, &shown) < 1e-4, "{cmd}: the preview is what OK applies");
    }
    // Without dithering, the colour-mode conversions keep the proxy.
    assert_eq!(preview_factor("image.mode.indexedColor", &json!({"dither": "none"}), 4), 4);
    assert_eq!(preview_factor("image.mode.bitmap", &json!({"method": "threshold"}), 4), 4);
}

/// #6530: the Content-Aware Fill preview runs at full resolution, for every Sampling choice,
/// because its patch search finds different matches on a reduced copy.
#[test]
fn content_aware_fill_previews_at_full_resolution() {
    for params in [json!({}), json!({"sampling": "auto"}), json!({"sampling": "rectangular"})] {
        assert_eq!(preview_factor("edit.contentAwareFill", &params, 4), 1, "{params}");
    }
}

/// #2063: proxy previews scaled what isn't a length (Tree's canvas fraction, Lens Correction's
/// perspective), clamped negative lengths to +0.1 (Offset left/up), left Shadows/Highlights'
/// radii in document pixels and rounded Mosaic's 10 px cells to 3 proxy px (12 px).
#[test]
fn proxy_previews_scale_only_lengths() {
    let doc = waves(256);
    let bg = doc.layers[0].id;
    let error = |cmd: &str, p: Value| {
        let k = preview_factor(cmd, &p, 4);
        let applied = preview_document(&doc, Some(bg), cmd, &p, 1).unwrap();
        let shown = preview_document(&doc, Some(bg), cmd, &p, k).unwrap();
        (k, mean_abs(&every_kth(&applied, k), &every_kth(&shown, 1)))
    };
    for (cmd, p) in [
        ("filter.lensCorrection", json!({"vertical": -100.0, "horizontal": 60.0})),
        ("filter.other.offset", json!({"horizontal": -40, "vertical": -24, "undefinedAreas": "wrap"})),
        (
            "image.adjustments.shadowsHighlights",
            json!({"shadowAmount": 50.0, "shadowRadius": 30.0, "highlightAmount": 50.0, "highlightRadius": 30.0, "blackClip": 0.0, "whiteClip": 0.0}),
        ),
    ] {
        let (k, diff) = error(cmd, p);
        assert_eq!(k, 4, "{cmd} keeps the proxy");
        assert!(diff < 0.01, "{cmd}: the preview shows what OK applies ({diff})");
    }
    // Mosaic's cells divide evenly: 10 px at k = 2 is 5 proxy px; 40 px keeps k = 4.
    assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": 10.0}), 4), 2);
    assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": 40.0}), 4), 4);
    assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": 7.0}), 4), 1);
    assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": "x"}), 4), 4);
    let (k, diff) = error("filter.pixelate.mosaic", json!({"cellSize": 10.0}));
    assert!(k == 2 && diff < 0.01, "Mosaic: {diff}");
    // Roundness at a fractional radius: at 6.5 px, k = 3 reaches floor(2.17) = 2 proxy px along an
    // axis (6 px), where k = 4 reached floor(1.63) = 1 (4 px), so the chosen factor previews closer
    // to what OK applies than k = 4 did (#10474).
    for cmd in ["filter.other.minimum", "filter.other.maximum"] {
        let p = json!({"radius": 6.5, "preserve": "roundness"});
        let (k, diff) = error(cmd, p.clone());
        let applied = preview_document(&doc, Some(bg), cmd, &p, 1).unwrap();
        let at_four = preview_document(&doc, Some(bg), cmd, &p, 4).unwrap();
        let four = mean_abs(&every_kth(&applied, 4), &every_kth(&at_four, 1));
        assert!(k == 3 && diff < four, "{cmd}: k = {k} differs by {diff}, k = 4 by {four}");
    }
    // Minimum and Maximum keep at least one proxy pixel of radius (#10474), and a large radius
    // keeps a proxy instead of a full-size preview that costs minutes at radius 500. Both keep
    // their whole-pixel radius within 1 px of OK's. Squareness rounds: 6 px is 2 proxy px at k = 3
    // (6 px), not at k = 4 (8 px); 50 px is 17 at k = 3 (51 px), not 13 at k = 4 (52 px). Roundness's
    // disc reaches floor(r) along an axis: 6 px is 2 at k = 3 (6 px), not 1 at k = 4 (4 px); 50 px
    // is 25 at k = 2, not 12 at k = 4 or 16 at k = 3 (48 px).
    let factors = |cmd: &str, preserve: &str| {
        [0.2, 1.5, 2.5, 3.9, 6.0, 50.0, 500.0, 9999.0].map(|radius| preview_factor(cmd, &json!({"radius": radius, "preserve": preserve}), 4))
    };
    for cmd in ["filter.other.minimum", "filter.other.maximum"] {
        assert_eq!(factors(cmd, "squareness"), [1, 1, 2, 3, 3, 3, 4, 4], "{cmd} squareness");
        assert_eq!(factors(cmd, "roundness"), [1, 1, 2, 3, 3, 2, 4, 4], "{cmd} roundness");
        assert_eq!(preview_factor(cmd, &json!({"radius": 6.0}), 4), 3, "{cmd}: Squareness is the default");
        assert_eq!(preview_factor(cmd, &json!({"radius": "x"}), 4), 4, "{cmd}: a junk radius keeps the proxy");
    }
    // Tree's and Picture Frame's sizes are relative, not pixels (and not shown as "px").
    assert!(!is_pixel_param_of("filter.render.tree", "size") && !is_pixel_param_of("filter.render.pictureFrame", "size"));
    assert!(is_pixel_param_of("filter.stylize.extrude", "size"));
}

/// #218: Warp Text renders the type again from the layer model, which a reduced proxy doesn't
/// scale: the preview showed the text k times too large. It is the full-size result, reduced.
#[test]
fn warp_text_preview_matches_the_full_size_result_on_a_reduced_proxy() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 4000, "height": 3000})).unwrap();
    s.execute("type.create", json!({"x": 300, "y": 1600, "text": "Warp", "size": 400})).unwrap();
    let st = s.active().unwrap();
    let (doc, layer) = ((*st.doc).clone(), st.active_layer.unwrap());
    let p = json!({"style": "arc", "bend": 40.0});
    let k = preview_factor("type.warpText", &p, crate::proxy::preview_factor(&doc, true));
    assert!(k > 1, "the preview runs on a reduced proxy");
    let cache = |d: &Document| match &d.layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Text(t) => t.cache.clone().unwrap(),
        _ => panic!("a type layer"),
    };
    let full = crate::proxy::proxy_document(&preview_document(&doc, Some(layer), "type.warpText", &p, 1).unwrap(), k);
    let shown = preview_document(&doc, Some(layer), "type.warpText", &p, k).unwrap();
    assert_eq!(shown.size, full.size);
    assert_eq!(cache(&shown).content_bounds(), cache(&full).content_bounds());
    assert_eq!(photocraft_compose::flatten(&shown).px, photocraft_compose::flatten(&full).px);
}

/// #5766: Curves on a smart object adds a smart filter, which re-renders the source through
/// the object's placement. On a reduced proxy that placement stayed in full-size document
/// coordinates, so the preview showed the object `k` times too large until OK.
#[test]
fn smart_object_curves_preview_keeps_its_size_on_a_reduced_proxy() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 16, "y": 16, "width": 16, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"contents": "black"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let st = s.active().unwrap();
    let (doc, layer) = ((*st.doc).clone(), st.active_layer.unwrap());
    let alpha = |d: &Document, x: i32, y: i32| match &d.layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.cache.as_ref().unwrap().pixel(x, y).last().copied().unwrap(),
        _ => panic!("a smart object"),
    };
    let k = 4;
    let params = json!({"points": [[0, 20], [90, 150], [255, 230]]});
    let shown = preview_document(&doc, Some(layer), "image.adjustments.curves", &params, k).unwrap();
    let full = preview_document(&doc, Some(layer), "image.adjustments.curves", &params, 1).unwrap();
    let photocraft_doc::LayerContent::Smart(full_sm) = &full.layer(layer).unwrap().content else { panic!("a smart object") };
    let reference = photocraft_algo::resample::resize_surface(full_sm.cache.as_ref().unwrap(), 0.25, 0.25, photocraft_algo::resample::Resample::Bicubic);
    let photocraft_doc::LayerContent::Smart(sm) = &shown.layer(layer).unwrap().content else { panic!("a smart object") };
    assert_eq!(sm.smart_filters.len(), 1, "the preview adds the Curves smart filter");
    assert_eq!(shown.size, crate::proxy::proxy_document(&doc, k).size);
    // The 16 px square at (16, 16) is a 4 px square at (4, 4) on the proxy, not 16 px at (16, 16).
    // Bicubic reduction blends the square's boundary with transparency; compare all alpha
    // values with a reduced full-size result, and require opacity only in the interior.
    for y in 0..shown.size.height as i32 {
        for x in 0..shown.size.width as i32 {
            let expected = reference.pixel(x, y).last().copied().unwrap();
            let actual = alpha(&shown, x, y);
            assert!((actual - expected).abs() <= 1.0 / 255.0, "({x}, {y}): alpha {actual}, expected {expected}");
        }
    }
    for (x, y) in [(5, 5), (5, 6), (6, 6)] {
        assert!(alpha(&shown, x, y) > 0.99, "({x}, {y}) is inside the reduced square");
    }
    for (x, y) in [(4, 4), (7, 7)] {
        assert!(alpha(&shown, x, y) > 0.5, "({x}, {y}) is a corner of the reduced square");
    }
    for (x, y) in [(2, 2), (9, 9), (12, 12), (15, 15)] {
        assert!(alpha(&shown, x, y) < 0.01, "({x}, {y}) is outside the reduced square");
    }
}

/// #2063: a pixel-sized parameter divided by the proxy factor was clamped to the command's
/// minimum (Pointillize's 5 px cells became 3 px proxy cells = 12 px), so the preview factor
/// drops until every pixel-sized value fits its range.
#[test]
fn proxy_previews_never_clamp_pixel_sized_params() {
    let pointillize = |c: f64| preview_factor("filter.pixelate.pointillize", &json!({"cellSize": c}), 4);
    assert_eq!((pointillize(5.0), pointillize(7.0), pointillize(12.0), pointillize(300.0)), (1, 2, 4, 4));
    assert_eq!(preview_factor("filter.pixelate.crystallize", &json!({"cellSize": 10.0}), 4), 3);
    assert_eq!(preview_factor("filter.pixelate.colorHalftone", &json!({"maxRadius": 8.0}), 4), 2);
    assert_eq!(preview_factor("filter.blur.gaussianBlur", &json!({"radius": 8.0}), 4), 4, "unclamped filters keep the proxy");
    // Hostile params never panic and leave the factor alone; a full-size document stays at 1.
    assert_eq!(preview_factor("filter.pixelate.pointillize", &json!({"cellSize": "x"}), 4), 4);
    assert_eq!(preview_factor("filter.pixelate.pointillize", &json!(null), 4), 4);
    assert_eq!(preview_factor("no.such.command", &json!({"cellSize": 1.0}), 4), 4);
    assert_eq!(preview_factor("filter.pixelate.pointillize", &json!({"cellSize": 5.0}), 0), 1);
    // Every previewed command, at each pixel-sized param's minimum, default and in between.
    let previewed = photocraft_engine::command_specs().iter().filter(|c| has_dialog(c.id) && (c.id.starts_with("filter.") || PREVIEWED.contains(&c.id)));
    for c in previewed {
        for p in Param::parse_list(c.params) {
            let ParamKind::Range { min, default, .. } = p.kind else { continue };
            if min <= 0.0 || !is_pixel_param_of(c.id, &p.key) {
                continue;
            }
            for v in [min, default, min * 2.5] {
                let k = preview_factor(c.id, &json!({ p.key.clone(): v }), 4);
                assert!(v / f64::from(k) >= min - 1e-9, "{} {}={v}: k={k} clamps to {min}", c.id, p.key);
            }
        }
    }
}

/// Sharp structure at every scale: [`waves`] with hard squares from 2 to 32 px, a fine
/// per-pixel texture and an interlaced band (odd rows inverted) at the bottom.
fn sharp(n: u32) -> Document {
    let mut doc = waves(n);
    let s = doc.layers[0].surface_mut().unwrap();
    let mut x0 = 4;
    for (i, size) in [2, 3, 4, 6, 8, 12, 16, 24, 32].into_iter().enumerate() {
        for y0 in (4..n as i32 * 3 / 4 - size).step_by((size * 3) as usize) {
            let v = if (i + y0 as usize / 7).is_multiple_of(2) { 0.05 } else { 0.95 };
            s.fill_rect(photocraft_geom::Rect::new(x0, y0, x0 + size, y0 + size), &[v, v * 0.8, 1.0 - v, 1.0]);
        }
        x0 += size + 6;
    }
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let mut px = s.pixel(x, y);
            if (x * 7 + y * 13) % 5 == 0 {
                px.iter_mut().take(3).for_each(|c| *c = (*c + 0.08).min(1.0));
            }
            if y >= n as i32 * 3 / 4 && y % 2 == 1 {
                px[0] = 1.0 - px[0];
            }
            s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &px);
        }
    }
    doc
}

/// [`waves`] with strong per-pixel noise, the input of noise reduction.
fn noisy(n: u32) -> Document {
    let mut doc = waves(n);
    let s = doc.layers[0].surface_mut().unwrap();
    let hash = |x: i32, y: i32| ((x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663)).wrapping_mul(2_654_435_761) >> 8;
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let mut px = s.pixel(x, y);
            for (c, v) in px.iter_mut().take(3).enumerate() {
                *v = (*v + (hash(x * 3 + c as i32, y) as f32 / (1u32 << 24) as f32 - 0.5) * 0.3).clamp(0.0, 1.0);
            }
            s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &px);
        }
    }
    doc
}

/// How a preview at factor `k` compares with what OK applies (factor 1) on a document.
#[derive(Debug)]
struct PreviewError {
    /// Mean absolute RGB difference at the pixels the proxy samples.
    diff: f32,
    /// Feature scale of the preview shown at document size, relative to the result: 1 when
    /// the features match, 1/k when the preview draws them k times too large.
    scale: f32,
    /// How much the preview changes the image relative to how much OK does (sampled); NaN
    /// when OK barely changes it.
    change: f32,
}

/// Mean absolute difference of neighbouring pixels at the document's own resolution.
fn detail(d: &Document) -> f32 {
    let b = photocraft_compose::flatten(d);
    let w = b.rect.width() as usize;
    let mut sum = 0.0;
    for (i, p) in b.px.iter().enumerate() {
        for q in [b.px.get(i + 1).filter(|_| (i + 1) % w != 0), b.px.get(i + w)].into_iter().flatten() {
            sum += (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>() / 3.0;
        }
    }
    sum / b.px.len().max(1) as f32
}

/// Pixels of `d` at every `k`-th row and column: the document pixels a factor-`k` proxy
/// samples, so a preview at `k` compares with them one to one.
fn every_kth(d: &Document, k: u32) -> Vec<[f32; 4]> {
    let b = photocraft_compose::flatten(d);
    let w = b.rect.width() as usize;
    let k = k as usize;
    let h = b.px.len() / w.max(1);
    (0..h.div_ceil(k))
        .flat_map(|y| (0..w.div_ceil(k)).map(move |x| (x * k, y * k)))
        .map(|(x, y)| b.px.get(y * w + x).copied().unwrap_or([f32::NAN; 4]))
        .collect()
}

fn mean_abs(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
    a.iter().zip(b).map(|(p, q)| (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>() / 3.0).sum::<f32>() / a.len().max(1) as f32
}

/// `None` when the command can't run on `doc` at all (e.g. Displace without a map).
fn preview_error(doc: &Document, command: &str, p: &Value, k: u32) -> Option<PreviewError> {
    let bg = doc.layers[0].id;
    let applied = preview_document(doc, Some(bg), command, p, 1)?;
    let shown = preview_document(doc, Some(bg), command, p, k).unwrap_or_else(|| panic!("{command} {p}: the k = {k} preview failed"));
    let (a, s) = (every_kth(&applied, k), every_kth(&shown, 1));
    let (src, src_k) = (every_kth(doc, k), every_kth(&crate::proxy::proxy_document(doc, k), 1));
    let applied_change = mean_abs(&a, &src);
    Some(PreviewError {
        diff: mean_abs(&a, &s),
        scale: detail(&shown) / k as f32 / detail(&applied).max(1e-5),
        change: if applied_change > 0.01 { mean_abs(&s, &src_k) / applied_change } else { f32::NAN },
    })
}

/// The parameter sets to preview `command` with: the dialog's defaults, and each pixel-sized
/// value at its minimum and at a large value (whole numbers as integers: Tree and Picture
/// Frame read some ranges as `u32`).
fn sweep_params(app: &mut PhotocraftApp, command: &str) -> Vec<(String, Value)> {
    let Ok(id) = open(app, command) else { return Vec::new() };
    let base = params_json(&fields_of(app, id));
    cancel(app, id);
    let mut out = vec![("default".to_string(), base.clone())];
    for p in Param::parse_list(photocraft_engine::commands::find(command).unwrap().params) {
        if !is_pixel_param_of(command, &p.key) {
            continue;
        }
        let values = match p.kind {
            ParamKind::Int { .. } => vec![-40.0, 40.0],
            ParamKind::Range { min, max, default, .. } => [min, (default * 4.0).clamp(min, max.min(64.0))].into_iter().filter(|v| *v != default).collect(),
            _ => continue,
        };
        for v in values {
            let mut q = base.clone();
            q[p.key.as_str()] = json!(v);
            out.push((format!("{}={v}", p.key), q));
        }
    }
    for (_, p) in &mut out {
        for v in p.as_object_mut().into_iter().flat_map(|o| o.values_mut()) {
            if let Some(x) = v.as_f64().filter(|x| x.fract() == 0.0 && x.abs() < 1e9) {
                *v = json!(x as i64);
            }
        }
    }
    out
}

/// Every previewed command with its parameter sets.
fn previewed_cases() -> Vec<(&'static str, String, Value)> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ids: Vec<&'static str> =
        photocraft_engine::command_specs().iter().map(|c| c.id).filter(|id| has_dialog(id) && (id.starts_with("filter.") || PREVIEWED.contains(id))).collect();
    ids.into_iter().flat_map(|id| sweep_params(&mut app, id).into_iter().map(move |(label, p)| (id, label, p))).collect()
}

fn grayscale(d: &Document) -> Document {
    let mut s = photocraft_engine::Session::new();
    s.add_document(d.clone(), None);
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.active().map(|d| (*d.doc).clone()).unwrap()
}

/// Smooth, sharp and noisy test images, each in RGB and grayscale (for the modes that need it).
fn test_images(n: u32) -> [(&'static str, Document, Document); 3] {
    let with_gray = |d: Document| (grayscale(&d), d);
    let ((ws, w), (ss, s), (ns, no)) = (with_gray(waves(n)), with_gray(sharp(n)), with_gray(noisy(n)));
    [("smooth", w, ws), ("sharp", s, ss), ("noisy", no, ns)]
}

fn image_for<'a>(command: &str, rgb: &'a Document, gray: &'a Document) -> &'a Document {
    if command == "image.mode.bitmap" || command == "image.mode.duotone" { gray } else { rgb }
}

/// Previews that only approximate the result at a proxy although their features have the
/// right size: dots and cells placed on the proxy's pixel grid (Color Halftone, Pointillize)
/// and tile edges resampled (Tiles). Two more only differ on a proxy as small as this test's
/// (64 x 64 px): Picture Frame's minimum line widths, and Shadows/Highlights' 0.01 % black and
/// white clips, which pick single pixels out of 4096 (its radii are checked in
/// `proxy_previews_scale_only_lengths`).
const APPROXIMATE_AT_PROXY: &[&str] = &[
    "filter.pixelate.colorHalftone",
    "filter.pixelate.pointillize",
    "filter.stylize.tiles",
    "filter.render.pictureFrame",
    "image.adjustments.shadowsHighlights",
];

/// Why a preview doesn't show what OK applies, if it doesn't: a clearly different result or
/// features at another size on the smooth image, or (on the sharp and noisy images, where a
/// sub-pixel radius can't show on a proxy) a filter that acts more strongly in the preview,
/// i.e. over a k times larger area.
fn mismatch(image: &str, e: &PreviewError) -> Option<String> {
    let off = if image == "smooth" { e.diff > 0.03 || (!(0.6..=1.6).contains(&e.scale) && e.diff > 0.002) } else { e.change > 1.25 };
    off.then(|| format!("{e:?}"))
}

/// #2063, every previewed command: the live preview at the factor [`preview_factor`] picks
/// for a 1/4 proxy shows what OK applies: the same features at the same size, on smooth,
/// sharp and noisy images, at the defaults and at small and large pixel sizes.
#[test]
fn every_preview_shows_what_ok_applies() {
    let cases: Vec<_> = previewed_cases().into_iter().filter(|(id, _, p)| preview_factor(id, p, 4) > 1 && !APPROXIMATE_AT_PROXY.contains(id)).collect();
    // One thread per test image: about 270 filter runs at 256 x 256.
    let bad: Vec<String> = std::thread::scope(|scope| {
        let runs: Vec<_> = test_images(256)
            .into_iter()
            .map(|(image, rgb, gray)| {
                let cases = &cases;
                scope.spawn(move || {
                    let mut bad = Vec::new();
                    for (id, label, p) in cases {
                        let k = preview_factor(id, p, 4);
                        if let Some(why) = preview_error(image_for(id, &rgb, &gray), id, p, k).and_then(|e| mismatch(image, &e)) {
                            bad.push(format!("{id} ({label}) at k = {k} on the {image} image: {why}"));
                        }
                    }
                    bad
                })
            })
            .collect();
        runs.into_iter().flat_map(|r| r.join().unwrap()).collect()
    });
    assert!(bad.is_empty(), "previews that differ from the result:\n{}", bad.join("\n"));
}

/// The table behind [`every_preview_shows_what_ok_applies`]: every previewed command at a
/// plain 1/4 proxy and at its chosen factor, on each test image.
/// `cargo test -p photocraft-ui-egui --lib filter_dialog::tests::sweep_all_previews -- --ignored --nocapture`
#[test]
#[ignore]
fn sweep_all_previews() {
    let images = test_images(256);
    println!("command | params | image | k | diff k=4 | scale k=4 | change k=4 | diff k | scale k | change k");
    for (id, label, p) in previewed_cases() {
        let k = preview_factor(id, &p, 4);
        for (image, rgb, gray) in &images {
            let doc = image_for(id, rgb, gray);
            match (preview_error(doc, id, &p, 4), preview_error(doc, id, &p, k)) {
                (Some(a), Some(b)) => println!(
                    "{id} | {label} | {image} | {k} | {:.4} | {:.2} | {:.2} | {:.4} | {:.2} | {:.2}",
                    a.diff, a.scale, a.change, b.diff, b.scale, b.change
                ),
                _ => println!("{id} | {label} | {image} | {k} | fails"),
            }
        }
    }
}

/// The profiles Assign Profile offers for a new document converted with `mode` (if any),
/// and which of the command's built-in profiles the command itself accepts for it.
fn assign_profile_offers(mode: Option<&str>) -> (Vec<String>, Vec<String>) {
    let mut app = new_app();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    if let Some(mode) = mode {
        app.run(mode, json!({})).unwrap();
    }
    let spec = photocraft_engine::commands::find("edit.assignProfile").unwrap();
    let ids = |rows: Vec<Param>| {
        let row = rows.into_iter().find(|p| p.key == "profile").expect("a profile row");
        let ParamKind::Choice { options, .. } = row.kind else { panic!("profile is a menu") };
        options.into_iter().map(|(id, _)| id).collect::<Vec<String>>()
    };
    let offered = ids(params(&mut app, "edit.assignProfile", spec.params));
    let accepted =
        ids(Param::parse_list(spec.params)).into_iter().filter(|name| app.session.execute("edit.assignProfile", json!({"profile": name})).is_ok()).collect();
    (offered, accepted)
}

/// #4951: Assign Profile lists the profiles the command accepts for the document's colour
/// mode, and none of the others.
#[test]
fn assign_profile_offers_only_profiles_of_the_documents_colour_mode() {
    for mode in [None, Some("image.mode.cmyk"), Some("image.mode.grayscale")] {
        let (offered, accepted) = assign_profile_offers(mode);
        for name in &accepted {
            assert!(offered.contains(name), "{mode:?}: {name} can be assigned, so it is offered");
        }
        for name in ["srgb", "coated-cmyk", "sgray"] {
            assert_eq!(offered.iter().any(|o| o == name), accepted.iter().any(|a| a == name), "{mode:?}: {name}");
        }
        assert!(offered.iter().any(|o| o == "working") && offered.iter().any(|o| o == "none"), "{mode:?}: working space and none always fit");
    }
    let (rgb, _) = assign_profile_offers(None);
    assert!(rgb.iter().any(|o| o == "srgb") && !rgb.iter().any(|o| o == "coated-cmyk"), "an RGB document gets RGB profiles");
    let (cmyk, _) = assign_profile_offers(Some("image.mode.cmyk"));
    assert!(cmyk.iter().any(|o| o == "coated-cmyk") && !cmyk.iter().any(|o| o == "srgb"), "a CMYK document gets CMYK profiles");
}

/// #9087: Artboard from Layers opens at the selected layers' size and the next Artboard N, as
/// in the reference application, never at a size remembered from other layers.
#[test]
fn artboard_from_layers_opens_at_the_selected_layers_size() {
    let mut app = new_app();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 10, "y": 5, "width": 20, "height": 30})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let id = open(&mut app, "layer.new.artboardFromLayers").unwrap();
    let f = fields_of(&app, id);
    assert_eq!((f.int("width"), f.int("height")), (Some(20), Some(30)), "the selected layers' size");
    assert_eq!(f.text("name"), Some("Artboard 1"));
    assert!(ok(&mut app, id));
    let doc = &app.session.active().unwrap().doc;
    let boards = doc.artboards();
    assert_eq!(boards.len(), 1);
    assert_eq!((boards[0].1, boards[0].2.rect), ("Artboard 1", photocraft_geom::Rect::new(10, 5, 30, 35)));
}

/// #4143: Replace Color shows the From colour (defaulting to the foreground) and its canvas
/// eyedropper samples the merged image into the dialog.
#[test]
fn replace_color_eyedropper_samples_the_image() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "#00ff00"})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    let id = open(&mut app, REPLACE_COLOR).unwrap();
    passes(&mut app, 2);
    assert_eq!(fields_of(&app, id).color("color"), Some([1.0, 0.0, 0.0]), "the dialog starts from the foreground colour");
    // #6279: armed as it opens, so the first click on the image samples.
    assert!(crate::dialogs::canvas_input::armed(&app).is_some());
    assert!(color_eyedropper_sample(&mut app, 4.0, 4.0, egui::Modifiers::NONE));
    assert_eq!(fields_of(&app, id).color("color"), Some([0.0, 1.0, 0.0]));
    // #4457: where it was picked, for Localized Color Clusters.
    assert_eq!(field_json(&app, id, "points"), json!([[4.0, 4.0]]));
    // Off-image: the click is consumed, nothing changes and the eyedropper stays armed.
    assert!(color_eyedropper_sample(&mut app, -1.0, 4.0, egui::Modifiers::NONE));
    assert_eq!(fields_of(&app, id).color("color"), Some([0.0, 1.0, 0.0]));
    assert!(crate::dialogs::canvas_input::armed(&app).is_some());
}

/// A press at the document point `(x, y)` with the dialog's eyedropper armed, as the canvas's pick takes it.
fn color_eyedropper_sample(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) -> bool {
    let consumed = crate::dialogs::canvas_input::pick_dropper(app, [x, y], craft::keymap::Mods::from_egui(&mods, craft::keymap::Platform::current()));
    // The host applies the sampled colour on the next pass, as the frame after the press does.
    passes(app, 1);
    consumed
}

/// The app's samples follow the framework's Alt reset: after it, the next samples carry an
/// order the command takes (CodeRabbit review on #10443).
#[test]
fn replace_color_reset_forgets_the_click_order() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "#00ff00"})).unwrap();
    let app = PhotocraftApp::new(s, crate::Services::default());
    let mut h = harness(app, egui::vec2(900.0, 800.0));
    let id = open(h.state_mut(), REPLACE_COLOR).unwrap();
    h.run_steps(3);
    assert!(color_eyedropper_sample(h.state_mut(), 2.0, 2.0, egui::Modifiers::NONE));
    assert!(color_eyedropper_sample(h.state_mut(), 9.0, 9.0, egui::Modifiers::ALT));
    h.get_by_label("Cancel").click_modifiers(egui::Modifiers::ALT);
    h.run_steps(3);
    assert!(is_open(h.state(), id), "Alt+Cancel resets and keeps the dialog open");
    assert!(color_eyedropper_sample(h.state_mut(), 5.0, 5.0, egui::Modifiers::SHIFT));
    assert!(color_eyedropper_sample(h.state_mut(), 7.0, 7.0, egui::Modifiers::SHIFT));
    let f = params_json(&fields_of(h.state(), id));
    assert_eq!(f["order"], json!(["+", "+"]), "{f:?}");
    let p = json!({"color": f["color"], "points": f["points"], "order": f["order"], "hue": 30});
    h.state_mut().session.execute(REPLACE_COLOR, p).unwrap();
}

/// #6278: a colour picked for the Result swatch sets Hue, Saturation and Lightness so the
/// sampled colour becomes it.
#[test]
fn replace_color_result_swatch_sets_the_sliders() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.run("tools.setColors", json!({"foreground": "#cc3333"})).unwrap();
    let mut h = harness(app, egui::vec2(900.0, 900.0));
    let id = open(h.state_mut(), REPLACE_COLOR).unwrap();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::ColorWell, "Result").click();
    h.run_steps(3);
    let picker = top_dialog(h.state());
    assert_ne!(picker, id, "the Color Picker opened over the dialog");
    // PLAN: assumed the Color Picker child keeps its colour under `color` as a hex string.
    set(h.state_mut(), picker, "color", json!("#3355cc"));
    let ctx = h.ctx.clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, picker).expect("OK is enabled");
    h.run_steps(3);
    let f = fields_of(h.state(), id);
    let n = |k: &str| f.number(k).unwrap() as f32;
    let got = photocraft_algo::tone::replace_color_result([0.8, 0.2, 0.2], n("hue"), n("saturation"), n("lightness"));
    let want = crate::color_picker_ui::parse_hex("#3355cc").unwrap();
    assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.02), "the sliders turn the sampled colour into the pick: {got:?}");
    assert!(n("hue") != 0.0, "the hue moved");
    // Moving a slider afterwards is not mistaken for another pick.
    set(h.state_mut(), id, "lightness", json!(10.0));
    h.run_steps(2);
    assert_eq!(fields_of(h.state(), id).number("lightness"), Some(10.0));
}

/// #4457: Localized Color Clusters and its Range, as in Photoshop's dialog; the sampled point
/// and both settings reach the command.
#[test]
fn replace_color_localized_color_clusters() {
    // Left half red, right half red too: localized on the left point, the right half stays.
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 10, "background": "#ff0000"})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    let id = open(&mut app, REPLACE_COLOR).unwrap();
    passes(&mut app, 2);
    assert!(color_eyedropper_sample(&mut app, 2.0, 5.0, egui::Modifiers::NONE));
    set(&mut app, id, "localized", json!(true));
    set(&mut app, id, "range", json!(10));
    set(&mut app, id, "hue", json!(120));
    let f = params_json(&fields_of(&app, id));
    assert_eq!((f["localized"].clone(), f["points"].clone()), (json!(true), json!([[2.0, 5.0]])));
    // The command with the dialog's fields, small range and a hue shift: only near (2, 5).
    app.run(REPLACE_COLOR, f).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let px = |x: i32| doc.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(x, 5, x + 1, 6));
    assert!(px(2)[1] > 0.5, "near the point: shifted to green {:?}", px(2));
    assert!(px(38)[0] > 0.9 && px(38)[1] < 0.1, "far away: still red {:?}", px(38));
}

#[test]
fn match_color_control_source_changes_reset_layer_even_with_copied_ids() {
    use photocraft_doc::{Color, ColorMode, Layer, SampleType, Size};
    use photocraft_geom::Rect;
    let area = Rect::new(0, 0, 8, 8);
    let mut source = Document::with_background("first", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let bottom = source.layers[0].id;
    let mut cover = Layer::raster("cover", source.pixel_format());
    cover.surface_mut().unwrap().fill_rect(area, &[0.9, 0.2, 0.1, 1.0]);
    source.layers.push(cover);
    let mut other = source.clone();
    other.id = Document::new("identity", source.size, source.mode, source.depth).id;
    other.name = "second".into();
    other.layers[0].surface_mut().unwrap().fill_rect(area, &[0.1, 0.8, 0.2, 1.0]);
    for frame in [false, true] {
        let mut session = photocraft_engine::Session::new();
        session.add_document(source.clone(), None);
        session.add_document(other.clone(), None);
        session.execute("file.new", json!({"width":8,"height":8,"background":"white"})).unwrap();
        let mut app = PhotocraftApp::new(session, Default::default());
        app.sync_views();
        for explicit_layer in [false, true] {
            let id = open(&mut app, "image.adjustments.matchColor").unwrap();
            set(&mut app, id, "source", json!("0"));
            set(&mut app, id, "sourceLayer", json!(bottom));
            set(&mut app, id, "source", json!("0"));
            assert_eq!(field_json(&app, id, "sourceLayer"), json!(bottom), "unchanged Source retains Layer");
            set(&mut app, id, "source", json!("1"));
            if frame {
                passes(&mut app, 1);
            }
            assert!(field_json(&app, id, "sourceLayer").is_null(), "a different Source returns to Merged");
            if explicit_layer {
                set(&mut app, id, "sourceLayer", json!(bottom));
            }
            assert!(ok(&mut app, id), "frame={frame}, explicit={explicit_layer}: OK applies");
            let pixel = app.session.active().unwrap().doc.layers[0].surface().unwrap().rgba(4, 4);
            assert_eq!(pixel[1] > pixel[0], explicit_layer, "explicit layer samples green; Merged samples red: {pixel:?}");
            app.run("edit.undo", json!({})).unwrap();
        }
        assert_eq!(*app.session.documents()[0].doc, source);
        assert_eq!(*app.session.documents()[1].doc, other);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn match_color_preview_refreshes_when_a_source_job_lands() {
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    use photocraft_geom::Rect;
    let area = Rect::new(0, 0, 8, 8);
    {
        let mut source = Document::with_background("reference", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        source.layers[0].surface_mut().unwrap().fill_rect(area, &[0.1, 0.3, 0.9, 1.0]);
        let mut session = photocraft_engine::Session::new();
        session.add_document(source, None);
        // Start before opening the dialog; applying a completed source job is a normal producer.
        let photocraft_engine::jobs::Started::Job(job) = session
            .start_job(
                "test.source",
                json!({}),
                "Source pixels",
                true,
                |_| Ok(()),
                move |session, ()| {
                    session.edit("Source pixels", |doc, active| {
                        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(area, &[0.9, 0.2, 0.1, 1.0]);
                        Ok(())
                    })?;
                    Ok(Value::Null)
                },
            )
            .unwrap()
        else {
            panic!("native source job");
        };
        session.execute("file.new", json!({"width":8,"height":8,"background":"white"})).unwrap();
        let mut app = PhotocraftApp::new(session, Default::default());
        app.sync_views();
        app.background_jobs = false;
        let target = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let ctx = app.ctx().clone();
        let id = open(&mut app, "image.adjustments.matchColor").unwrap();
        set(&mut app, id, "source", json!("0"));
        // The Preview box is on from the dialog's first drawn pass.
        passes(&mut app, 2);
        let preview = |app: &mut PhotocraftApp, blue: bool| {
            crate::canvas::ensure_filter_preview(app, 1, &ctx).expect("inline preview");
            let pixel = app.filter_preview.as_ref().unwrap().result.as_ref().unwrap().layers[0].surface().unwrap().rgba(4, 4);
            assert_eq!(pixel[2] > pixel[0], blue, "preview follows current source pixels: {pixel:?}");
            pixel
        };
        preview(&mut app, true);
        let before_key = app.filter_preview.as_ref().unwrap().key.clone();
        let cached = app.filter_preview.as_ref().unwrap().result.as_ref().unwrap().clone();
        preview(&mut app, true);
        assert!(std::sync::Arc::ptr_eq(&cached, app.filter_preview.as_ref().unwrap().result.as_ref().unwrap()), "unchanged source reuses cached preview");
        app.session.wait_job(job).unwrap();
        assert_eq!(app.session.active_index(), Some(1));
        assert_eq!(app.session.active().unwrap().revision, revision, "target revision remains unchanged");
        assert!(std::sync::Arc::ptr_eq(&target, &app.session.active().unwrap().doc));
        let shown = preview(&mut app, false);
        let after_key = app.filter_preview.as_ref().unwrap().key.clone();
        assert_ne!(before_key, after_key);
        assert_ne!(
            app.filter_live.session(),
            Some(crate::canvas::filter_preview::session_of(&before_key)),
            "old-source worker results must not be republished as slider updates"
        );
        ok(&mut app, id);
        let applied = app.session.active().unwrap().doc.layers[0].surface().unwrap().rgba(4, 4);
        assert!(shown.iter().zip(applied).all(|(a, b)| (a - b).abs() <= 1.0 / 255.0), "preview agrees with OK");
        assert_eq!(history(&app), 1);
    }
}

#[test]
fn preview_source_revisions_follow_only_referenced_documents() {
    let mut session = photocraft_engine::Session::new();
    for _ in 0..2 {
        session.execute("file.new", json!({"width":1,"height":1})).unwrap();
    }
    let source = &session.documents()[0];
    let want = vec![(source.doc.id, source.revision)];
    for (command, params) in [
        ("image.adjustments.matchColor", json!({"source":0})),
        ("filter.distort.displace", json!({"mapDocument":0})),
        (SMART_FILTER_SET_PARAMS, json!({"params":{"mapDocument":0}})),
    ] {
        assert_eq!(preview_source_revisions(&session, command, &params), want, "{command}");
    }
    for params in [json!({}), json!({"source":-1}), json!({"source":99})] {
        assert!(preview_source_revisions(&session, "image.adjustments.matchColor", &params).is_empty());
    }
}

/// The Indexed Color matte menu and its colour picker reach the command: each fixed choice and a
/// colour picked under Other… change the edge colour OK applies.
#[test]
fn indexed_matte_dropdown_and_picker_change_the_applied_edge_color() {
    use egui::accesskit::Role;

    for (choice, matte) in [("Black", [0.0, 0.0, 0.0]), ("50% Gray", [128.0 / 255.0; 3]), ("Netscape Gray", [192.0 / 255.0; 3]), ("Other…", [0.0, 0.0, 1.0])]
    {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 2, "height": 1, "depth": 32, "background": "transparent"})).unwrap();
        session
            .edit("fixture", |doc, active| {
                let bounds = doc.bounds();
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().write_region(bounds, &[1.0, 0.0, 0.0, 0.75, 0.0, 0.0, 0.0, 0.0]);
                Ok(())
            })
            .unwrap();
        let app = PhotocraftApp::new(session, crate::Services::default());
        let mut harness = harness(app, egui::vec2(900.0, 1000.0));
        let id = open(harness.state_mut(), "image.mode.indexedColor").unwrap();
        set(harness.state_mut(), id, "palette", json!("exact"));
        harness.run_steps(3);
        harness.query_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some("None")).expect("Matte dropdown").click();
        harness.run_steps(2);
        harness.get_by_label(choice).click();
        harness.run_steps(3);
        if choice == "Other…" {
            assert_eq!(top_dialog(harness.state()), id, "the matte colour is a row of the dialog, not a child picker");
            set(harness.state_mut(), id, "matteColor", json!([0.0, 0.0, 1.0]));
            harness.run_steps(3);
        }
        harness.get_by_label("OK").click();
        harness.run_steps(3);
        let doc = &harness.state().session.active().unwrap().doc;
        assert_eq!(doc.mode, photocraft_color::ColorMode::Indexed);
        let surface = doc.layers[0].surface().unwrap();
        let edge = surface.rgba(0, 0);
        let expected = [0.75 + 0.25 * matte[0], 0.25 * matte[1], 0.25 * matte[2]];
        assert!(edge.iter().zip(expected).all(|(got, want)| (got - want).abs() <= 1.0 / 255.0) && edge[3] == 1.0, "{choice}: {edge:?} vs {expected:?}");
        assert_eq!(surface.rgba(1, 0)[3], 0.0);
    }
}

/// Other… shows the Matte Color row in the same dialog, None hides it again.
#[test]
fn indexed_matte_other_shows_the_color_row_and_none_hides_it() {
    use egui::accesskit::Role;

    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 2, "height": 1, "depth": 32, "background": "transparent"})).unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let mut harness = harness(app, egui::vec2(900.0, 1000.0));
    let id = open(harness.state_mut(), "image.mode.indexedColor").unwrap();
    harness.run_steps(3);
    assert!(try_set(harness.state_mut(), id, "matteColor", json!([0.0, 0.0, 1.0])).is_err(), "no colour row while the matte is None");
    harness.query_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some("None")).expect("Matte dropdown").click();
    harness.run_steps(2);
    harness.get_by_label("Other…").click();
    harness.run_steps(3);
    assert_eq!(top_dialog(harness.state()), id, "Other… opens no child dialog");
    assert_eq!(fields_of(harness.state(), id).text("matte"), Some("custom"));
    set(harness.state_mut(), id, "matteColor", json!([0.0, 0.0, 1.0]));

    harness.query_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some("Other…")).expect("Matte dropdown").click();
    harness.run_steps(2);
    harness.get_by_label("None").click();
    harness.run_steps(3);
    assert_eq!(fields_of(harness.state(), id).text("matte"), Some("none"));
    assert!(try_set(harness.state_mut(), id, "matteColor", json!([0.0, 0.0, 1.0])).is_err(), "None hides the colour row");
}

/// An alpha channel chosen as Lens Blur's depth map reaches the command as the `channel` source and its index: the
/// menu's own `channel:N` id was sent as is, and the engine refused it at OK and in the preview.
#[test]
fn lens_blur_alpha_channel_source_runs() {
    let mut app = new_app();
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    for name in ["Mask", "Depth"] {
        app.run("select.all", json!({})).unwrap();
        app.run("select.saveSelection", json!({"name": name})).unwrap();
    }
    app.run("select.deselect", json!({})).unwrap();
    let id = open(&mut app, LENS_BLUR).unwrap();
    passes(&mut app, 2);
    set(&mut app, id, "depthMap", json!("channel:1"));
    passes(&mut app, 2);
    let params = command_params(LENS_BLUR, &fields_of(&app, id));
    assert_eq!((&params["depthMap"], &params["depthChannel"]), (&json!("channel"), &json!(1)), "{params}");
    app.run(LENS_BLUR, params).expect("the command takes the channel source");
    // The other sources pass through.
    set(&mut app, id, "depthMap", json!("transparency"));
    let params = command_params(LENS_BLUR, &fields_of(&app, id));
    assert_eq!((&params["depthMap"], params.get("depthChannel")), (&json!("transparency"), None));
}

/// A Lens Blur depth channel is an index into the document the dialog opened on: when the control channel makes another
/// document active under the open dialog, preview and OK refuse the channel instead of blurring by that document's
/// channel at the same index; back on the first document, OK applies it there.
#[test]
fn lens_blur_refuses_a_depth_channel_after_the_document_changed() {
    let mut app = new_app();
    for _ in 0..2 {
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        for name in ["Mask", "Depth"] {
            app.run("select.all", json!({})).unwrap();
            app.run("select.saveSelection", json!({"name": name})).unwrap();
        }
        app.run("select.deselect", json!({})).unwrap();
    }
    let ctx = app.ctx().clone();
    match_color_control(&mut app, &ctx, "ui.window.open", json!({"document": 0}));
    let id = open(&mut app, LENS_BLUR).unwrap();
    passes(&mut app, 2);
    set(&mut app, id, "depthMap", json!("channel:1"));
    passes(&mut app, 2);
    let steps: Vec<_> = app.session.documents().iter().map(|st| st.history.past_len()).collect();

    match_color_control(&mut app, &ctx, "ui.window.open", json!({"document": 1}));
    assert!(crate::canvas::ensure_filter_preview(&mut app, 1, &ctx).is_none(), "no preview with the other document's channel");
    assert!(!ok(&mut app, id), "OK is refused and the dialog stays open");
    assert_eq!(app.session.documents().iter().map(|st| st.history.past_len()).collect::<Vec<_>>(), steps, "nothing was blurred");
    // Sources that name no channel are not bound to a document.
    let opened = crate::dialogs::params::captured(&app, LENS_BLUR);
    assert!(validate_lens_blur_document(&app, LENS_BLUR, &opened, Some("transparency")).is_ok());

    match_color_control(&mut app, &ctx, "ui.window.open", json!({"document": 0}));
    assert!(ok(&mut app, id), "on the document it opened on, the channel is accepted");
    assert_eq!(app.session.documents()[0].history.past_len(), steps[0] + 1, "Lens Blur ran on the first document");
    assert_eq!(app.session.documents()[1].history.past_len(), steps[1], "the second document is untouched");
}
