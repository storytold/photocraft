//! #13732: Export As's Canvas Size pads or crops the exported image around its centre. Width and
//! Height start at the export's image size and follow Scale; a side over 15000 px is clamped with
//! a warning; Reset is enabled only while the canvas differs.

use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_doc::{Color, SampleType, Size};
use photocraft_geom::Rect;
use std::{cell::RefCell, rc::Rc};

type Writes = Rc<RefCell<Vec<Vec<u8>>>>;

const RED: [u8; 3] = [200, 30, 30];
const GREEN: [u8; 3] = [30, 160, 30];

/// 200 x 150 RGB/8 whose Background is red on the left half and green on the right.
fn halves() -> Document {
    let mut doc = Document::with_background("Halves", Size::new(200, 150), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let surface = doc.layers[0].surface_mut().unwrap();
    let fmt = surface.format();
    let rgb = |c: [u8; 3]| photocraft_raster::from_rgba(&fmt, [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0]);
    surface.fill_rect(Rect::new(0, 0, 100, 150), &rgb(RED));
    surface.fill_rect(Rect::new(100, 0, 200, 150), &rgb(GREEN));
    doc
}

fn export(doc: &Document, extra: Value) -> (Vec<u8>, String) {
    let mut f = json!({"format": "png", "scale": 100, "width": 200, "height": 150, "profile": "document", "embedProfile": false}).as_object().unwrap().clone();
    f.extend(extra.as_object().unwrap().clone());
    let fmt = s_fmt(&f);
    let out = export_document(doc, &f, None, &photocraft_engine::color_cmds::document_profile(doc)).unwrap();
    (photocraft_io::export(&out, &format!("x.{fmt}"), &settings(&f).io_options()).unwrap().bytes, fmt)
}

/// The decoded file's size and its pixel at (x, 75).
fn read(bytes: &[u8], fmt: &str, xs: &[u32]) -> ((u32, u32), Vec<[u8; 4]>) {
    let doc = photocraft_io::import(&format!("x.{fmt}"), bytes).unwrap().document;
    let w = doc.size.width as usize;
    let px = photocraft_compose::flatten(&doc).into_px();
    let row = |x: u32| px[75 * w + x as usize].map(|v| (v * 255.0).round() as u8);
    ((doc.size.width, doc.size.height), xs.iter().map(|&x| row(x)).collect())
}

fn near(got: [u8; 4], want: [u8; 4], tolerance: u8) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= tolerance)
}

#[test]
fn a_wider_canvas_centres_the_image_transparent_in_png_and_white_otherwise() {
    let doc = halves();
    let canvas = json!({"canvasWidth": 300, "canvasHeight": 150});
    let (png, fmt) = export(&doc, canvas.clone());
    let (size, px) = read(&png, &fmt, &[10, 49, 50, 149, 150, 249, 250, 290]);
    assert_eq!(size, (300, 150));
    let [r, g] = [RED, GREEN].map(|c| [c[0], c[1], c[2], 255]);
    assert_eq!(px, vec![[0; 4], [0; 4], r, r, g, g, [0; 4], [0; 4]], "centred, transparent on both sides");

    let mut off = canvas.as_object().unwrap().clone();
    off.insert("transparency".into(), json!(false));
    let (png, fmt) = export(&doc, Value::Object(off));
    let (_, px) = read(&png, &fmt, &[10, 290, 60]);
    assert_eq!(px, vec![[255; 4], [255; 4], r], "PNG without Transparency: white");

    let mut jpg = canvas.as_object().unwrap().clone();
    jpg.insert("format".into(), json!("jpg"));
    let (bytes, fmt) = export(&doc, Value::Object(jpg));
    let (size, px) = read(&bytes, &fmt, &[10, 290, 75]);
    assert_eq!(size, (300, 150));
    assert!(near(px[0], [255; 4], 3) && near(px[1], [255; 4], 3) && near(px[2], r, 6), "JPG: white sides, {px:?}");
}

#[test]
fn a_narrower_canvas_crops_around_the_centre() {
    let (png, fmt) = export(&halves(), json!({"canvasWidth": 100, "canvasHeight": 150}));
    let (size, px) = read(&png, &fmt, &[0, 49, 50, 99]);
    assert_eq!(size, (100, 150));
    let [r, g] = [RED, GREEN].map(|c| [c[0], c[1], c[2], 255]);
    assert_eq!(px, vec![r, r, g, g], "the middle 100 px of the image");
}

/// The canvas is in the export's pixels: a reduced preview gets the same proportion.
#[test]
fn the_canvas_scales_with_a_reduced_preview_and_is_off_when_equal() {
    let f = json!({"scale": 100, "width": 200, "height": 150, "canvasWidth": 300, "canvasHeight": 150}).as_object().unwrap().clone();
    assert_eq!(export_canvas(&f, 200.0, 150.0), Some((300.0, 150.0)));
    assert_eq!(export_canvas(&f, 100.0, 75.0), Some((150.0, 75.0)));
    let same = json!({"width": 200, "height": 150, "canvasWidth": 200, "canvasHeight": 150}).as_object().unwrap().clone();
    assert_eq!(export_canvas(&same, 200.0, 150.0), None);
    let none = json!({"width": 200, "height": 150}).as_object().unwrap().clone();
    assert_eq!(export_canvas(&none, 200.0, 150.0), None, "no canvas: today's export");
}

fn harness() -> (Harness<'static, PhotocraftApp>, Writes) {
    let writes = Writes::default();
    let captured = writes.clone();
    let h = Harness::builder().with_size(egui::vec2(1100.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let (picker, _) = crate::file_dialog::fake(vec![Some(crate::FileDialogAnswer::SaveTo("out.png".into()))]);
        let services = crate::Services {
            file_dialog: Some(picker),
            export: Some(Box::new(|doc, path, settings| {
                photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(move |_, bytes| {
                captured.borrow_mut().push(bytes.to_vec());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.session.add_document(halves(), None);
        app
    });
    (h, writes)
}

fn open_dialog(h: &mut Harness<'_, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(6);
}

fn type_into(h: &mut Harness<'_, PhotocraftApp>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::SpinButton, label).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in text.chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
}

fn value(h: &Harness<'_, PhotocraftApp>, label: &str) -> Option<f64> {
    h.get_by_role_and_label(Role::SpinButton, label).accesskit_node().numeric_value()
}

fn reset_enabled(h: &Harness<'_, PhotocraftApp>) -> bool {
    !h.get_by_role_and_label(Role::Button, "Reset").accesskit_node().is_disabled()
}

const LIMIT: &str = "Canvas Size is limited to 15000 pixels";

#[test]
fn canvas_fields_start_at_the_image_size_follow_scale_clamp_and_reset() {
    let (mut h, _) = harness();
    open_dialog(&mut h);
    h.get_by_label("Canvas Size");
    assert_eq!((value(&h, "Canvas Width"), value(&h, "Canvas Height")), (Some(200.0), Some(150.0)));
    assert!(!reset_enabled(&h), "Reset greyed while the canvas is the image size");

    type_into(&mut h, "Scale", "50");
    assert_eq!((value(&h, "Canvas Width"), value(&h, "Canvas Height")), (Some(100.0), Some(75.0)), "follows Scale");

    type_into(&mut h, "Canvas Width", "99999");
    assert_eq!(value(&h, "Canvas Width"), Some(15000.0), "clamped");
    assert!(h.query_by_label_contains(LIMIT).is_some(), "with a warning");
    assert!(reset_enabled(&h));

    h.get_by_role_and_label(Role::Button, "Reset").click();
    h.run_steps(4);
    assert_eq!((value(&h, "Canvas Width"), value(&h, "Canvas Height")), (Some(100.0), Some(75.0)));
    assert!(h.query_by_label_contains(LIMIT).is_none());
    assert!(!reset_enabled(&h));

    type_into(&mut h, "Canvas Height", "0");
    assert_eq!(value(&h, "Canvas Height"), Some(1.0), "0 becomes 1");
}

#[test]
fn exporting_with_a_canvas_leaves_the_document_and_its_history_alone() {
    let (mut h, writes) = harness();
    let before = h.state().session.active().unwrap().doc.clone();
    let states = h.state().session.active().unwrap().history.entries().len();
    open_dialog(&mut h);
    type_into(&mut h, "Canvas Width", "300");
    h.get_by_role_and_label(Role::Button, "Export").click();
    h.run_steps(6);
    let (size, _) = read(&writes.borrow()[0], "png", &[0]);
    assert_eq!(size, (300, 150));
    let st = h.state().session.active().unwrap();
    assert!(Arc::ptr_eq(&before, &st.doc), "the document is untouched");
    assert_eq!(st.history.entries().len(), states, "no history step");
}
