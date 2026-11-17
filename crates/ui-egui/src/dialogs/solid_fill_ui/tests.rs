use craft::color_ui::ui::{ColorPickerDialog, PickedColor};
use craft::theme::Srgba;
use egui::vec2;
use egui_kittest::Harness;
use photocraft_color::{Color, ColorMode};
use photocraft_doc::{Fill, LayerContent, LayerId};
use serde_json::json;

use super::*;
use crate::color_picker_ui::testing;

fn harness(ppp: f32) -> (Harness<'static, PhotocraftApp>, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": 16})).unwrap();
    let id = LayerId(s.execute("layer.newFillLayer.solidColor", json!({"color": "#33669980"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Other"})).unwrap();
    let mut h =
        Harness::builder().with_size(vec2(1440.0, 900.0)).with_pixels_per_point(ppp).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
        });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    (h, id)
}

/// A CMYK document with no fill layer yet.
fn cmyk_app() -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("image.mode.cmyk", json!({})).unwrap();
    PhotocraftApp::new(s, crate::Services::default())
}

/// OK on picker `dialog`, and the error its commit reported: a refused commit leaves the error in the status line.
fn ok(app: &mut PhotocraftApp, dialog: DialogId) -> Result<(), String> {
    app.ui.status = Default::default();
    app.ui.status_error = false;
    crate::dialogs::testing::confirm(app, dialog).unwrap();
    if app.ui.status_error { Err(app.ui.status.to_string()) } else { Ok(()) }
}

/// Types CMYK `inks` (0..1) into picker `id`: the colour they convert to, as the picker's 8-bit hex shows it, with the
/// inks kept as typed.
fn type_inks(app: &mut PhotocraftApp, id: DialogId, inks: [f32; 4]) {
    let rgb = photocraft_color::convert::cmyk_to_rgb(inks).map(|c| (c.clamp(0.0, 1.0) * 255.0).round() / 255.0);
    let picker = app.framework.dialogs.get_mut_by_id::<ColorPickerDialog>(id).expect("the picker is open");
    // The picker's CMYK fields are percent.
    picker.set_color(PickedColor::cmyk(Srgba::from_rgb_array(rgb), inks.map(|v| v * 100.0)));
}

/// The solid fill of the active layer.
fn active_fill(app: &PhotocraftApp) -> Color {
    let st = app.session.active().unwrap();
    match st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Fill(Fill::Solid(c)) => c,
        _ => panic!("not a solid fill"),
    }
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    same_inks(a, b)
}

#[test]
fn new_solid_color_from_menu_cancel_keeps_document_and_history_unchanged() {
    let (mut h, _) = harness(1.0);
    let app = h.state_mut();
    let before = app.session.active().unwrap().doc.clone();
    let active = app.session.active().unwrap().active_layer;
    let steps = app.session.active().unwrap().history.entries().len();

    let dialog = open_new(app).expect("a new Solid Color fill opens the shared Color Picker");
    assert!(
        matches!(color_picker_ui::target(app, dialog), Some(PickTarget::NewSolidFill { document }) if *document == before.id),
        "the picker is for a new fill of the active document"
    );
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "no layer exists until OK");
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps);

    testing::set_color(app, dialog, "#ff8800");
    crate::dialogs::testing::cancel(app, dialog).unwrap();
    assert!(app.framework.dialogs.get_by_id::<ColorPickerDialog>(dialog).is_none());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "Cancel must not create or delete a layer");
    assert_eq!(app.session.active().unwrap().active_layer, active);
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps, "Cancel must not record a history entry");
}

#[test]
fn new_solid_color_from_menu_ok_creates_one_layer_with_chosen_or_default_color() {
    let (mut h, _) = harness(1.0);
    let app = h.state_mut();
    let steps = app.session.active().unwrap().history.entries().len();

    // OK with the initial foreground colour still creates the fill layer.
    let first = open_new(app).unwrap();
    ok(app, first).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    assert!(matches!(app.session.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Fill(Fill::Solid(_))));
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 1);

    let second = open_new(app).unwrap();
    testing::set_color(app, second, "#ff8800");
    ok(app, second).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let LayerContent::Fill(Fill::Solid(color)) = app.session.active().unwrap().doc.layer(id).unwrap().content else {
        panic!("expected the newly created Solid Color fill");
    };
    assert_eq!(color.to_rgba8()[..3], [255, 136, 0]);
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 2);

    // Creation with a colour given is not interactive: no picker opens.
    let open_before = app.framework.dialogs.stack().ids().count();
    let explicit = app.run("layer.newFillLayer.solidColor", json!({"color": "#123456"})).unwrap();
    assert!(explicit["layer"].as_u64().is_some());
    assert_eq!(app.framework.dialogs.stack().ids().count(), open_before, "parameterised creation stays non-interactive");
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 3);
}

#[test]
fn new_solid_color_picker_rejects_a_different_active_document() {
    let (mut h, _) = harness(1.0);
    let app = h.state_mut();
    let dialog = open_new(app).unwrap();
    testing::set_color(app, dialog, "#ff8800");
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.entries().len();
    assert!(ok(app, dialog).is_err());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps);
}

#[test]
fn picker_previews_without_editing_cancel_discards_and_ok_is_one_step() {
    let (mut h, id) = harness(1.0);
    let app = h.state_mut();
    app.run("layer.select", json!({"layer": id.0})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let LayerContent::Fill(Fill::Solid(original)) = before.layer(id).unwrap().content else { panic!() };
    let steps = app.session.active().unwrap().history.entries().len();
    let tools = app.session.tools.foreground;
    let dialog = open(app).unwrap();
    testing::set_color(app, dialog, "#ff8800");
    let (preview, key) = display_doc(app, 0).unwrap();
    let (again, again_key) = display_doc(app, 0).unwrap();
    assert_eq!(key, again_key);
    assert!(Arc::ptr_eq(&preview, &again), "preview is cached");
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "preview does not dirty the real document");
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps);
    let LayerContent::Fill(Fill::Solid(color)) = preview.layer(id).unwrap().content else { panic!() };
    assert_eq!(color.to_rgba8(), [255, 136, 0, 128]);
    assert_eq!(thumbnail_color(app, before.id, id, original), color);
    crate::dialogs::testing::cancel(app, dialog).unwrap();
    assert!(display_doc(app, 0).is_none());
    assert!(app.solid_fill_preview.is_none());
    assert_eq!(thumbnail_color(app, before.id, id, original), original);
    let dialog = open(app).unwrap();
    testing::set_color(app, dialog, "#ff8800");
    let (_, reopened_key) = display_doc(app, 0).unwrap();
    assert_ne!(key, reopened_key, "reopening a picker never reuses a cancelled preview key");
    ok(app, dialog).unwrap();
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 1);
    assert_eq!(app.session.tools.foreground, tools, "editing a fill doesn't change the tool colour");
    assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Fill(Fill::Solid(color)));
    let bytes = photocraft_format::save_to_bytes(&app.session.active().unwrap().doc, &Default::default()).unwrap();
    let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(loaded.layer(id).unwrap().content, LayerContent::Fill(Fill::Solid(color)));
    assert_eq!(loaded.layer(id).unwrap().mask, before.layer(id).unwrap().mask);
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "fill.psd", &Default::default()).unwrap().bytes;
    let loaded = photocraft_io::import("fill.psd", &bytes).unwrap().document;
    let fill = loaded.walk().into_iter().find(|(_, _, l)| matches!(l.content, LayerContent::Fill(Fill::Solid(_)))).unwrap().2;
    let LayerContent::Fill(Fill::Solid(saved)) = fill.content else { panic!() };
    // PSD's existing SoCo descriptor stores the fill's colour components, not Color.alpha.
    assert_eq!(&saved.to_rgba8()[..3], &color.to_rgba8()[..3]);
    assert!(fill.mask.is_some());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(*app.session.active().unwrap().doc, *before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Fill(Fill::Solid(color)));
}

#[test]
fn unchanged_ok_keeps_imported_model_precision_and_alpha() {
    let (mut h, id) = harness(1.0);
    let app = h.state_mut();
    app.run("layer.select", json!({"layer": id.0})).unwrap();
    let color = Color { mode: ColorMode::Lab, c: [0.512345, 0.498765, 0.56789, 0.0], alpha: 0.345678 };
    app.run(cmds::SET, json!({"color": color})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let dialog = open(app).unwrap();
    ok(app, dialog).unwrap();
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
}

#[test]
fn picker_never_applies_to_a_different_document_with_the_same_layer_id() {
    let (mut h, id) = harness(1.0);
    let app = h.state_mut();
    app.run("layer.select", json!({"layer": id.0})).unwrap();
    let dialog = open(app).unwrap();
    testing::set_color(app, dialog, "#ff0000");
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    app.run("layer.newFillLayer.solidColor", json!({})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    assert!(display_doc(app, app.session.active_index().unwrap()).is_none());
    assert!(ok(app, dialog).is_err());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
}

#[test]
fn cmyk_lab_preview_thumbnail_cancel_and_ok_are_consistent() {
    for (command, mode) in [("image.mode.cmyk", ColorMode::Cmyk), ("image.mode.lab", ColorMode::Lab)] {
        let (mut h, id) = harness(1.0);
        let app = h.state_mut();
        app.run(command, json!({})).unwrap();
        app.run("layer.select", json!({"layer": id.0})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let LayerContent::Fill(Fill::Solid(original)) = before.layer(id).unwrap().content else { panic!() };
        assert_eq!(before.mode, mode);
        let count = app.session.active().unwrap().history.entries().len();
        let dialog = open(app).unwrap();
        testing::set_color(app, dialog, "#ff8800");
        let (shown, _) = display_doc(app, app.session.active_index().unwrap()).unwrap();
        let LayerContent::Fill(Fill::Solid(preview)) = shown.layer(id).unwrap().content else { panic!() };
        assert_eq!(preview, Color::rgba(1.0, 136.0 / 255.0, 0.0, original.alpha).in_mode(mode));
        assert_eq!(thumbnail_color(app, before.id, id, original), preview);
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().history.entries().len(), count);
        crate::dialogs::testing::cancel(app, dialog).unwrap();
        assert!(display_doc(app, app.session.active_index().unwrap()).is_none());
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().history.entries().len(), count);
        let dialog = open(app).unwrap();
        testing::set_color(app, dialog, "#ff8800");
        ok(app, dialog).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Fill(Fill::Solid(preview)));
        assert_eq!(app.session.active().unwrap().history.entries().len(), count + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, *before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Fill(Fill::Solid(preview)));
    }
}

/// #8568: CMYK 50/50/50/100 typed into the Solid Color picker of a CMYK document is the fill's colour after OK, for a
/// new fill and for an edited one, not what its sRGB stand-in converts back to through the document profile.
#[test]
fn cmyk_typed_into_the_picker_is_the_fills_colour_in_a_cmyk_document() {
    let mut app = cmyk_app();
    let inks = [0.5, 0.5, 0.5, 1.0];
    let id = open_new(&mut app).unwrap();
    type_inks(&mut app, id, inks);
    ok(&mut app, id).unwrap();
    assert_eq!(active_fill(&app), Color { mode: ColorMode::Cmyk, c: inks, alpha: 1.0 });

    let edit = [0.25, 0.75, 0.0, 0.1];
    let id = open(&mut app).unwrap();
    type_inks(&mut app, id, edit);
    ok(&mut app, id).unwrap();
    assert!(close(active_fill(&app).c, edit), "{:?}", active_fill(&app).c);
    // Reopening the picker shows the stored inks.
    let id = open(&mut app).unwrap();
    let shown = testing::current(&app, id).unwrap();
    assert!(close(inks_of(&shown).expect("the picker shows the fill's inks"), edit));

    // Other inks with the same display colour are still an edit.
    let near = [0.5, 0.5, 0.499, 1.0];
    let id = open(&mut app).unwrap();
    type_inks(&mut app, id, near);
    ok(&mut app, id).unwrap();
    let rich = [0.5, 0.5, 0.5, 1.0];
    let id = open(&mut app).unwrap();
    type_inks(&mut app, id, rich);
    let opening = app.framework.dialogs.get_by_id::<ColorPickerDialog>(id).and_then(|d| d.opening()).expect("the picker remembers its opening colour");
    assert_eq!(rgb_hex(testing::current(&app, id).unwrap().color), rgb_hex(opening.color), "both inks show as the same colour");
    ok(&mut app, id).unwrap();
    assert_eq!(active_fill(&app).c, rich);
}

/// Inks that share the picker's hex with the fill's own are still a preview and a thumbnail, as they are an edit on OK
/// (#8568).
#[test]
fn same_hex_inks_preview_and_thumbnail_as_ok_commits_them() {
    let mut app = cmyk_app();
    let near = [0.5, 0.5, 0.499, 1.0];
    let id = open_new(&mut app).unwrap();
    type_inks(&mut app, id, near);
    ok(&mut app, id).unwrap();

    let rich = [0.5, 0.5, 0.5, 1.0];
    let id = open(&mut app).unwrap();
    type_inks(&mut app, id, rich);
    let opening = app.framework.dialogs.get_by_id::<ColorPickerDialog>(id).and_then(|d| d.opening()).expect("the picker remembers its opening colour");
    assert_eq!(rgb_hex(testing::current(&app, id).unwrap().color), rgb_hex(opening.color), "both inks show as the same colour");

    let st = app.session.active().unwrap();
    let (document, layer) = (st.doc.id, st.active_layer.unwrap());
    let LayerContent::Fill(Fill::Solid(original)) = st.doc.layer(layer).unwrap().content else { panic!("not a solid fill") };
    assert_eq!(thumbnail_color(&app, document, layer, original).c, rich);
    let (shown, _) = display_doc(&mut app, 0).expect("a preview of the new inks");
    let LayerContent::Fill(Fill::Solid(previewed)) = shown.layer(layer).unwrap().content else { panic!("not a solid fill") };
    assert_eq!(previewed.c, rich);
    // OK commits what the preview showed.
    ok(&mut app, id).unwrap();
    assert_eq!(active_fill(&app).c, rich);
}

/// An RGB fill inside a CMYK document that the picker gives inks at the unchanged hex is an edit; in an RGB document,
/// which ignores the inks, it is not.
#[test]
fn inks_given_to_a_non_cmyk_fill_in_a_cmyk_document_are_an_edit() {
    let original = Color::rgba(0.2, 0.3, 0.4, 1.0);
    let picked = PickedColor::cmyk(Srgba::from_rgb_array(original.to_rgb()), [30.0, 20.0, 10.0, 40.0]);
    assert!(is_edit(&picked, original, ColorMode::Cmyk));
    assert!(!is_edit(&picked, original, ColorMode::Rgb));
    assert!(picked_color(&picked, original, ColorMode::Cmyk).is_some_and(|c| c.mode == ColorMode::Cmyk && close(c.c, [0.3, 0.2, 0.1, 0.4])));
}

/// #8568: a new fill's picker opens with the inks the foreground was picked as, so OK keeps them.
#[test]
fn a_new_fills_picker_is_seeded_with_the_foregrounds_inks() {
    let mut app = cmyk_app();
    let hex = |inks: [f32; 4]| {
        let [r, g, b] = photocraft_color::convert::cmyk_to_rgb(inks).map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        format!("#{r:02x}{g:02x}{b:02x}")
    };
    let rich = [0.5, 0.5, 0.5, 1.0];
    app.session.execute("tools.setColors", json!({"foreground": hex(rich), "foregroundCmyk": rich})).unwrap();
    let id = open_new(&mut app).unwrap();
    let seeded = testing::current(&app, id).unwrap();
    assert!(close(inks_of(&seeded).expect("the picker shows inks"), rich), "seeded with the foreground's inks");
    ok(&mut app, id).unwrap();
    assert_eq!(active_fill(&app).c, rich, "OK keeps the seeded inks");
}
