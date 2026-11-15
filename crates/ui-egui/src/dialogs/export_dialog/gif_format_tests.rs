//! #13720: Export As offers GIF right after JPG. GIF has no quality, transparency or embedded
//! profile controls; fully transparent pixels keep one transparent index and partly transparent
//! ones are matted on white.

use super::*;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::{Color, Layer, SampleType, Size};
use photocraft_geom::Rect;
use std::{cell::RefCell, rc::Rc};

type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;
type Asked = Rc<RefCell<Vec<crate::file_dialog::FileDialogRequest>>>;

/// 200 x 150 RGB/8 with a hidden white Background and a green (0,160,0) strip along the bottom
/// on a layer at 50 % opacity: the top is fully transparent, the strip half transparent.
fn strip_doc() -> Document {
    let mut doc = Document::with_background("Strip", Size::new(200, 150), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    doc.layers[0].visible = false;
    let fmt = doc.pixel_format();
    let mut strip = Layer::raster("Strip", fmt);
    strip.opacity = 0.5;
    strip.surface_mut().unwrap().fill_rect(Rect::new(0, 120, 200, 150), &photocraft_raster::from_rgba(&fmt, [0.0, 160.0 / 255.0, 0.0, 1.0]));
    doc.layers.push(strip);
    doc
}

fn gif_fields() -> Map<String, Value> {
    json!({"format": "gif", "scale": 100}).as_object().unwrap().clone()
}

fn export_gif(doc: &Document, f: &Map<String, Value>) -> Vec<u8> {
    let out = export_document(doc, f, None, &photocraft_engine::color_cmds::document_profile(doc)).unwrap();
    photocraft_io::export(&out, "x.gif", &settings(f).io_options()).unwrap().bytes
}

/// Whether the file has a Graphic Control Extension with its transparency flag set.
fn has_transparent_index(bytes: &[u8]) -> bool {
    bytes.windows(4).any(|w| w[0] == 0x21 && w[1] == 0xf9 && w[2] == 4 && w[3] & 1 == 1)
}

fn pixel(bytes: &[u8], x: u32, y: u32) -> [u8; 4] {
    let doc = photocraft_io::import("x.gif", bytes).unwrap().document;
    let flat = photocraft_compose::flatten(&doc);
    let w = doc.size.width as usize;
    flat.into_px()[y as usize * w + x as usize].map(|v| (v * 255.0).round() as u8)
}

fn assert_near(got: [u8; 4], want: [u8; 4]) {
    assert!(got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1), "expected {want:?} within 1 level, got {got:?}");
}

#[test]
fn gif_follows_jpg_in_the_format_list() {
    let keys: Vec<&str> = FORMATS.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, ["png", "jpg", "gif", "webp", "tif", "tga", "ico", "jxl"]);
    assert!(valid_setting("format", &json!("gif")));
}

#[test]
fn gif_is_gif89a_with_a_transparent_index_and_partial_alpha_matted_on_white() {
    let bytes = export_gif(&strip_doc(), &gif_fields());
    assert_eq!(bytes.get(..6), Some(b"GIF89a".as_slice()));
    assert!(has_transparent_index(&bytes), "one transparent palette index");
    assert_near(pixel(&bytes, 10, 140), [127, 207, 127, 255]);
    assert_eq!(pixel(&bytes, 10, 10)[3], 0, "fully transparent stays transparent");
}

/// Transparency off, remembered from another format, doesn't fill GIF's transparent pixels: GIF
/// shows no Transparency checkbox to turn it back on.
#[test]
fn a_remembered_transparency_off_does_not_flatten_a_gif() {
    let mut f = gif_fields();
    f.insert("transparency".into(), json!(false));
    let bytes = export_gif(&strip_doc(), &f);
    assert_eq!(pixel(&bytes, 10, 10)[3], 0);
    assert_near(pixel(&bytes, 10, 140), [127, 207, 127, 255]);
}

fn harness() -> (Harness<'static, PhotocraftApp>, Writes, Asked) {
    let writes = Writes::default();
    let captured = writes.clone();
    let (picker, asked) = crate::file_dialog::fake(vec![Some(crate::FileDialogAnswer::SaveTo("out.gif".into()))]);
    let mut picker = Some(picker);
    let h = Harness::builder().with_size(egui::vec2(1100.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let services = crate::Services {
            file_dialog: picker.take(),
            export: Some(Box::new(|doc, path, settings| {
                photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(move |path, bytes| {
                captured.borrow_mut().push((path.to_string(), bytes.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.session.add_document(strip_doc(), None);
        app
    });
    (h, writes, asked)
}

fn open_dialog(h: &mut Harness<'_, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(6);
}

fn shown(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

#[test]
fn choosing_gif_hides_its_controls_suggests_a_gif_name_and_is_remembered() {
    let (mut h, writes, asked) = harness();
    open_dialog(&mut h);
    assert!(shown(&h, "Transparency") && shown(&h, "Embed Color Profile"), "PNG shows both");
    h.get_by_role_and_label(Role::ComboBox, "Format").click();
    h.run_steps(4);
    h.get_by_label("GIF").click();
    h.run_steps(6);
    for hidden in ["Quality", "Transparency", "Lossless", "Embed Color Profile"] {
        assert!(!shown(&h, hidden), "{hidden} is hidden for GIF");
    }
    assert!(shown(&h, "Convert to sRGB"), "Convert to sRGB stays");

    h.get_by_role_and_label(Role::Button, "Export").click();
    h.run_steps(6);
    let suggested = asked.borrow().iter().find_map(|r| match r {
        crate::file_dialog::FileDialogRequest::Save { suggested } => Some(suggested.clone()),
        _ => None,
    });
    assert!(suggested.as_deref().is_some_and(|s| s.ends_with("Strip.gif")), "suggested {suggested:?}");
    let writes = writes.borrow();
    let (_, bytes) = writes.first().expect("one file written");
    assert_eq!(bytes.get(..6), Some(b"GIF89a".as_slice()));

    let id = open(h.state_mut()).unwrap();
    assert_eq!(h.state_mut().ui.dialog_mut(id).unwrap().fields["format"], "gif", "GIF remembered");
}
