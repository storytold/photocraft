//! #13721: PNG's Smaller File (8-bit) writes a palette PNG (colour type 3 with tRNS) whose
//! transparency is binary: clear pixels stay clear and every other pixel is opaque in its own
//! colour, with no matte.

use super::*;
use egui::accesskit::{Role, Toggled};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_doc::{Color, Layer, SampleType, Size};
use photocraft_geom::Rect;
use std::{cell::RefCell, rc::Rc};

const SMALL: &str = "Smaller File (8-bit)";
const TRANSPARENCY: &str = "Transparency";
type Writes = Rc<RefCell<Vec<Vec<u8>>>>;

/// 200 x 150 RGB/8: `Background` red on the left half and white on the right, `Top` a blue square
/// 50,25-150,125 and `Semi` a green strip 0,100-200,150 at 50 % layer opacity.
fn test_doc(background_visible: bool) -> Document {
    let mut doc = Document::with_background("Palette", Size::new(200, 150), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let fmt = doc.pixel_format();
    let rgb = |r: u8, g: u8, b: u8| photocraft_raster::from_rgba(&fmt, [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, 1.0]);
    doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 100, 150), &rgb(200, 30, 30));
    doc.layers[0].visible = background_visible;
    let mut top = Layer::raster("Top", fmt);
    top.surface_mut().unwrap().fill_rect(Rect::new(50, 25, 150, 125), &rgb(30, 30, 200));
    let mut semi = Layer::raster("Semi", fmt);
    semi.opacity = 0.5;
    semi.surface_mut().unwrap().fill_rect(Rect::new(0, 100, 200, 150), &rgb(0, 160, 0));
    doc.layers.push(top);
    doc.layers.push(semi);
    doc
}

fn fields(extra: Value) -> Map<String, Value> {
    let mut f = json!({"format": "png", "scale": 100, "profile": "document", "embedProfile": false}).as_object().unwrap().clone();
    f.extend(extra.as_object().unwrap().clone());
    f
}

fn export_png(doc: &Document, f: &Map<String, Value>) -> Vec<u8> {
    let out = export_document(doc, f, None, &photocraft_engine::color_cmds::document_profile(doc)).unwrap();
    photocraft_io::export(&out, "x.png", &settings(f).io_options()).unwrap().bytes
}

/// IHDR's colour type: 2 RGB, 3 palette, 6 RGBA.
fn color_type(png: &[u8]) -> u8 {
    assert_eq!(&png[12..16], b"IHDR");
    png[25]
}

fn has_trns(png: &[u8]) -> bool {
    png.windows(4).any(|w| w == b"tRNS")
}

fn pixel(png: &[u8], x: usize, y: usize) -> [u8; 4] {
    let doc = photocraft_io::import("x.png", png).unwrap().document;
    let w = doc.size.width as usize;
    photocraft_compose::flatten(&doc).into_px()[y * w + x].map(|v| (v * 255.0).round() as u8)
}

#[test]
fn smaller_file_writes_a_palette_png_with_binary_transparency() {
    let png = export_png(&test_doc(false), &fields(json!({"png8": true})));
    assert_eq!(color_type(&png), 3, "palette PNG");
    assert!(has_trns(&png), "tRNS chunk");
    assert_eq!(pixel(&png, 10, 10)[3], 0, "clear stays clear");
    assert_eq!(pixel(&png, 10, 140), [0, 160, 0, 255], "50 % green is opaque in its own colour, not matted");
    assert_eq!(pixel(&png, 100, 75), [30, 30, 200, 255]);
}

#[test]
fn smaller_file_keeps_every_colour_of_a_small_image() {
    let png = export_png(&test_doc(true), &fields(json!({"png8": true})));
    assert_eq!(color_type(&png), 3);
    assert_eq!(pixel(&png, 10, 10), [200, 30, 30, 255]);
    assert_eq!(pixel(&png, 190, 10), [255, 255, 255, 255]);
    assert_eq!(pixel(&png, 100, 75), [30, 30, 200, 255]);
}

/// Unticked (the default) PNG stays 32-bit; Transparency off remembered from before still keeps
/// an 8-bit file transparent, since the box shows ticked while 8-bit is on.
#[test]
fn unticked_stays_truecolour_and_ticked_ignores_transparency_off() {
    let png = export_png(&test_doc(false), &fields(json!({})));
    assert_eq!(color_type(&png), 6, "default PNG is RGBA");
    let png = export_png(&test_doc(false), &fields(json!({"png8": true, "transparency": false})));
    assert_eq!(color_type(&png), 3);
    assert_eq!(pixel(&png, 10, 10)[3], 0);
    assert!(!png8(&fields(json!({"png8": true, "format": "tif"}))), "only PNG has the option");
    assert!(valid_setting("png8", &json!(true)) && !valid_setting("png8", &json!("yes")));
}

fn harness() -> (Harness<'static, PhotocraftApp>, Writes) {
    let writes = Writes::default();
    let captured = writes.clone();
    let h = Harness::builder().with_size(egui::vec2(1100.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
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
        app.session.add_document(test_doc(false), None);
        app
    });
    (h, writes)
}

fn open_dialog(h: &mut Harness<'_, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(6);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    h.run_steps(6);
}

/// (ticked, enabled)
fn checkbox(h: &Harness<'_, PhotocraftApp>, label: &str) -> (bool, bool) {
    let node = h.get_by_role_and_label(Role::CheckBox, label);
    let node = node.accesskit_node();
    (node.toggled() == Some(Toggled::True), !node.is_disabled())
}

fn choose_format(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    click(h, Role::ComboBox, "Format");
    h.get_by_label(label).click();
    h.run_steps(6);
}

#[test]
fn the_checkbox_is_png_only_greys_transparency_and_gives_it_back() {
    let (mut h, _) = harness();
    open_dialog(&mut h);
    assert_eq!(checkbox(&h, SMALL), (false, true), "PNG shows it unticked");
    click(&mut h, Role::CheckBox, TRANSPARENCY);
    assert_eq!(checkbox(&h, TRANSPARENCY), (false, true));
    click(&mut h, Role::CheckBox, SMALL);
    assert_eq!(checkbox(&h, TRANSPARENCY), (true, false), "greyed out and shown ticked");
    click(&mut h, Role::CheckBox, SMALL);
    assert_eq!(checkbox(&h, TRANSPARENCY), (false, true), "its own earlier state is back");
    for other in ["TIFF", "JPG"] {
        choose_format(&mut h, other);
        assert!(h.query_by_label(SMALL).is_none(), "{other} has no 8-bit option");
    }
}

#[test]
fn the_choice_is_remembered_after_export_not_after_cancel() {
    let (mut h, writes) = harness();
    open_dialog(&mut h);
    click(&mut h, Role::CheckBox, SMALL);
    click(&mut h, Role::Button, "Cancel");
    open_dialog(&mut h);
    assert_eq!(checkbox(&h, SMALL), (false, true), "Cancel forgets it");
    click(&mut h, Role::CheckBox, SMALL);
    click(&mut h, Role::Button, "Export");
    assert_eq!(color_type(&writes.borrow()[0]), 3);
    open_dialog(&mut h);
    assert_eq!(checkbox(&h, SMALL), (true, true), "Export remembers it");
}
