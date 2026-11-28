//! #14196: Edit › Auto-Align Layers opens the reference application's dialog: a Projection group of six radios over
//! three columns (Auto, Perspective, Collage / Cylindrical, Spherical, Reposition) with Auto selected on a fresh
//! start, a Lens Correction group (Vignette Removal, Geometric Distortion, both off; Collage and Reposition grey and
//! clear them), the selected projection's description line and the warnings under them, OK and Cancel. No
//! Interpolation or Registration rows, Alt keeps Cancel a Cancel, and the last run's projection (also a scripted
//! one) is what the dialog reopens with.

use craft::ui::dialog::DialogId;
use egui::accesskit::{Role, Toggled};
use egui::{Event, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::new_layer_ui::testing::{fields, set, top};

const COMMAND: &str = "edit.autoAlignLayers";
/// The radios in reading order: the first row, then the second.
const RADIOS: [&str; 6] = ["Auto", "Perspective", "Collage", "Cylindrical", "Spherical", "Reposition"];

type H = Harness<'static, Box<PhotocraftApp>>;

/// The deterministic texture of the engine's Auto-Align tests, written into layer `id` at offset (dx, dy).
fn paint_texture(app: &mut PhotocraftApp, id: LayerId, dx: i32, dy: i32) {
    app.session
        .edit("tex", |doc, _| {
            let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
            let mut seed = 99u64;
            let mut rnd = || {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (seed >> 33) as f64 / (1u64 << 31) as f64
            };
            surf.fill_rect(photocraft_geom::Rect::new(dx, dy, dx + 240, dy + 180), &[0.4, 0.4, 0.4, 1.0]);
            for _ in 0..120 {
                let (x, y) = ((rnd() * 220.0) as i32, (rnd() * 160.0) as i32);
                let (sw, sh) = (4 + (rnd() * 14.0) as i32, 4 + (rnd() * 10.0) as i32);
                let v = rnd() as f32;
                surf.fill_rect(photocraft_geom::Rect::new(dx + x, dy + y, dx + x + sw, dy + y + sh), &[v, 1.0 - v, v * 0.5, 1.0]);
            }
            Ok(())
        })
        .unwrap();
}

/// A 300x240 document with two textured layers, the second offset, both selected: Auto-Align can run on it.
fn two_layers(app: &mut PhotocraftApp) {
    app.run("file.new", json!({"width": 300, "height": 240, "background": "transparent"})).unwrap();
    let mut ids = Vec::new();
    for (name, dx, dy) in [("A", 20, 20), ("B", 31, 13)] {
        app.run("layer.new.layer", json!({"name": name})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        paint_texture(app, id, dx, dy);
        ids.push(id);
    }
    app.run("layer.select", json!({"layer": ids[0].0})).unwrap();
    app.run("layer.select", json!({"layer": ids[1].0, "mode": "add"})).unwrap();
}

/// The whole app with the two-layer document, before the dialog opens.
fn harness() -> H {
    let mut h = Harness::builder().with_size(vec2(1600.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        two_layers(&mut app);
        app
    });
    h.run_steps(3);
    h
}

/// Opens the dialog through the menu and draws a few frames.
fn open(h: &mut H) -> DialogId {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    let id = top(h.state());
    h.run_steps(4);
    id
}

fn dialog() -> (H, DialogId) {
    let mut h = harness();
    let id = open(&mut h);
    (h, id)
}

fn radio(h: &H, label: &str) -> egui::Rect {
    h.get_by_role_and_label(Role::RadioButton, label).rect()
}

fn checked(h: &H, role: Role, label: &str) -> bool {
    h.get_by_role_and_label(role, label).accesskit_node().toggled() == Some(Toggled::True)
}

fn disabled(h: &H, role: Role, label: &str) -> bool {
    h.get_by_role_and_label(role, label).accesskit_node().is_disabled()
}

fn sent(h: &H, id: DialogId) -> Value {
    crate::filter_dialog::command_params(COMMAND, fields(h.state(), id))
}

fn shows(h: &H, text: &str) -> bool {
    h.query_all_by_label(text).next().is_some()
}

#[test]
fn six_projection_radios_stand_three_by_two_with_auto_selected() {
    let (h, _) = dialog();
    let rects: Vec<egui::Rect> = RADIOS.iter().map(|label| radio(&h, label)).collect();
    for row in [&rects[0..3], &rects[3..6]] {
        for r in &row[1..] {
            assert!((r.center().y - row[0].center().y).abs() < 4.0, "one row: {row:?}");
        }
        assert!(row[0].right() < row[1].left() && row[1].right() < row[2].left(), "left to right: {row:?}");
    }
    assert!(rects[3].top() > rects[0].bottom(), "Cylindrical, Spherical and Reposition stand under the first row");
    for col in 0..3 {
        assert!((rects[col].left() - rects[col + 3].left()).abs() < 4.0, "column {col} lines up");
    }
    assert!(checked(&h, Role::RadioButton, "Auto"), "a fresh dialog selects Auto");
    for label in &RADIOS[1..] {
        assert!(!checked(&h, Role::RadioButton, label), "{label} starts clear");
    }
}

#[test]
fn the_description_line_follows_the_selected_radio() {
    let (mut h, id) = dialog();
    assert!(shows(&h, "Determine the best projection automatically."));
    for (label, line) in [
        ("Perspective", "Only allow perspective image transform."),
        ("Collage", "Allow image rotation, scale and translation."),
        ("Cylindrical", "Only allow cylindrical image transform."),
        ("Spherical", "Only allow spherical image transform."),
        ("Reposition", "Only allow image translation."),
    ] {
        h.get_by_role_and_label(Role::RadioButton, label).click();
        h.run_steps(3);
        assert!(shows(&h, line), "{label}: {line}");
        assert!(!shows(&h, "Determine the best projection automatically."), "{label}: one description line");
        assert_eq!(sent(&h, id)["projection"], json!(label.to_lowercase()));
    }
}

#[test]
fn lens_correction_has_two_clear_boxes_that_collage_and_reposition_grey_and_clear() {
    let (mut h, id) = dialog();
    assert!(shows(&h, "Projection") && shows(&h, "Lens Correction"), "the two groups are named");
    for label in ["Vignette Removal", "Geometric Distortion"] {
        assert!(!checked(&h, Role::CheckBox, label), "{label} starts off");
    }
    h.get_by_role_and_label(Role::CheckBox, "Geometric Distortion").click();
    h.run_steps(3);
    assert!(checked(&h, Role::CheckBox, "Geometric Distortion"));
    assert_eq!(sent(&h, id)["geometricCorrection"], json!(true), "Geometric Distortion drives geometricCorrection");
    for picked in ["Collage", "Reposition"] {
        h.get_by_role_and_label(Role::RadioButton, picked).click();
        h.run_steps(3);
        assert!(disabled(&h, Role::CheckBox, "Geometric Distortion") && disabled(&h, Role::CheckBox, "Vignette Removal"), "{picked} greys both");
        assert!(!checked(&h, Role::CheckBox, "Geometric Distortion"), "{picked} clears it");
        assert_eq!(sent(&h, id)["geometricCorrection"], json!(false));
    }
    h.get_by_role_and_label(Role::RadioButton, "Auto").click();
    h.run_steps(3);
    assert!(!disabled(&h, Role::CheckBox, "Geometric Distortion"), "Auto enables it again");
    assert!(!checked(&h, Role::CheckBox, "Geometric Distortion"), "still cleared");
}

#[test]
fn no_interpolation_or_registration_rows_and_the_command_keeps_them() {
    let (h, id) = dialog();
    assert!(h.query_by_label("Interpolation").is_none(), "no Interpolation row");
    assert!(h.query_by_label("Registration").is_none(), "no Registration row");
    let params = sent(&h, id);
    assert!(params.get("interpolation").is_none() && params.get("registration").is_none(), "the command's defaults apply: {params}");
    let spec = photocraft_engine::commands::find(COMMAND).unwrap();
    let keys: Vec<String> = craft::ui::patterns::form_dialogs::Param::parse_list(spec.params).into_iter().map(|p| p.key).collect();
    assert!(keys.iter().any(|k| k == "interpolation") && keys.iter().any(|k| k == "registration"), "{keys:?}");
}

#[test]
fn the_metadata_warning_shows_for_layers_without_camera_metadata() {
    let (h, _) = dialog();
    assert!(
        shows(
            &h,
            "Warning: Could not detect camera metadata. Use the \"File > Scripts > Load Images into Stack...\" script to preserve metadata for Vignette Removal and/or Geometric Distortion."
        ),
        "the document has no EXIF"
    );
    assert!(
        !shows(
            &h,
            "Warning: For better Vignette Removal results, shoot with the same camera, lens, aperture, resolution, orientation, focal length and focus settings."
        ),
        "Vignette Removal is off"
    );
}

/// The text block's lines: the description, the metadata warning, then the vignette warning while it is ticked.
#[test]
fn the_text_block_lists_the_warnings_in_order() {
    use super::auto_align_ui::{METADATA_WARNING, VIGNETTE_WARNING, lines};
    assert_eq!(lines("reposition", false, true), vec!["Only allow image translation."]);
    assert_eq!(lines("auto", false, false), vec!["Determine the best projection automatically.", METADATA_WARNING]);
    assert_eq!(lines("auto", true, false), vec!["Determine the best projection automatically.", METADATA_WARNING, VIGNETTE_WARNING]);
    assert_eq!(lines("spherical", true, true), vec!["Only allow spherical image transform.", VIGNETTE_WARNING]);
}

#[test]
fn alt_keeps_cancel_and_escape_cancels() {
    let (mut h, _) = dialog();
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.run_steps(2);
    assert!(h.query_by_label("Reset").is_none(), "Alt does not turn Cancel into Reset");
    h.get_by_role_and_label(Role::Button, "Cancel");
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    let before = h.state().session.active().unwrap().history.past_len();
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_label("Only allow image translation.").is_none() && !shows(&h, "Lens Correction"), "Escape closes the dialog");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before, "Escape leaves the history alone");
}

#[test]
fn ok_runs_and_the_dialog_reopens_with_the_projection_and_ticks_it_ran_with() {
    let (mut h, id) = dialog();
    h.get_by_role_and_label(Role::RadioButton, "Perspective").click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::CheckBox, "Geometric Distortion").click();
    h.run_steps(2);
    let before = h.state().session.active().unwrap().history.past_len();
    crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before + 1, "OK runs Auto-Align");
    let id = open(&mut h);
    assert!(checked(&h, Role::RadioButton, "Perspective"), "reopens with Perspective");
    assert!(checked(&h, Role::CheckBox, "Geometric Distortion"), "and the tick confirmed with OK");
    assert_eq!(sent(&h, id)["projection"], json!("perspective"));
}

#[test]
fn a_scripted_run_decides_the_projection_the_dialog_reopens_with() {
    let mut h = harness();
    // An OK with Geometric Distortion ticked first, so the script's run has something to replace.
    let id = open(&mut h);
    h.get_by_role_and_label(Role::CheckBox, "Geometric Distortion").click();
    h.run_steps(2);
    crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
    h.state_mut().run(COMMAND, json!({"projection": "reposition"})).unwrap();
    h.run_steps(2);
    let id = open(&mut h);
    assert!(checked(&h, Role::RadioButton, "Reposition"), "the scripted projection");
    assert!(!checked(&h, Role::CheckBox, "Geometric Distortion"), "a run without the lens keys leaves the box clear");
    assert!(shows(&h, "Only allow image translation."));
    assert_eq!(sent(&h, id)["projection"], json!("reposition"));
}

#[test]
fn an_agent_sets_the_projection_by_its_id_and_a_bad_one_is_refused() {
    let (mut h, id) = dialog();
    set(h.state_mut(), id, "projection", json!("cylindrical")).unwrap();
    h.run_steps(3);
    assert!(checked(&h, Role::RadioButton, "Cylindrical"));
    assert_eq!(sent(&h, id)["projection"], json!("cylindrical"));
    // An unknown id never reaches the command: the row keeps a projection it knows.
    let _ = set(h.state_mut(), id, "projection", json!("sideways"));
    h.run_steps(3);
    let projection = sent(&h, id)["projection"].as_str().unwrap().to_owned();
    assert!(RADIOS.iter().any(|r| r.to_lowercase() == projection), "{projection}");
}
