//! #14816: Layer › Export As and Layer › Quick Export as PNG always write an RGBA PNG: an opaque
//! layer keeps alpha 255, a layer at 50 % opacity keeps its opacity as alpha, and a pixel nothing
//! painted inside the exported bounds is stored as 255,255,255,0. File › Export As of a whole
//! document keeps writing RGB for an opaque composite, and so does File › Export › Quick Export.

use std::cell::RefCell;
use std::rc::Rc;

use photocraft_codecs::ChannelLayout;
use serde_json::{Map, Value, json};

use super::{effective_source_profile, settings, source_document, write_export};
use crate::{FileDialogAnswer, FileDialogReply, FileDialogRequest, PhotocraftApp, Services, menus};

/// What the test writer was handed: the path and the encoded bytes.
type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;
/// The file dialogs the app asked for, still waiting for their answer.
type Open = Rc<RefCell<Vec<(FileDialogRequest, FileDialogReply)>>>;

/// An app with one document that really encodes, whose writer hands the bytes back to the test:
/// what the layer export paths produce can then be read without touching the disc.
fn app(width: u32, height: u32) -> (PhotocraftApp, Written, Open) {
    let (written, open): (Written, Open) = (Rc::default(), Rc::default());
    let (w, held) = (written.clone(), open.clone());
    let services = Services {
        export: Some(Box::new(|doc, path, s| {
            let out = photocraft_io::export(doc, path, &s.io_options()).map_err(|e| e.to_string())?;
            Ok((out.bytes, out.warnings))
        })),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            w.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        })),
        file_dialog: Some(Box::new(move |request, _parent, reply| held.borrow_mut().push((request, reply)))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": width, "height": height})).unwrap();
    (app, written, open)
}

/// The active layer, as the commands name it.
fn active(app: &PhotocraftApp) -> u64 {
    app.session.active().unwrap().active_layer.unwrap().0
}

/// A new layer, filled with `color` over `rect` (the whole layer when `rect` is `None`).
fn painted(app: &mut PhotocraftApp, name: &str, color: &str, rect: Option<(i32, i32, i32, i32)>) -> u64 {
    app.run("layer.new.layer", json!({"name": name})).unwrap();
    if let Some((x, y, width, height)) = rect {
        app.run("select.rect", json!({"x": x, "y": y, "width": width, "height": height})).unwrap();
    }
    app.run("edit.fill", json!({"color": color})).unwrap();
    if rect.is_some() {
        app.run("select.deselect", json!({})).unwrap();
    }
    active(app)
}

/// The Export As dialog's PNG fields, for one layer, several layers, or the whole document.
fn fields(layer: Option<u64>) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("format".into(), json!("png"));
    f.insert("scale".into(), json!(100));
    if let Some(layer) = layer {
        f.insert("__layer".into(), json!(layer));
    }
    f
}

/// What Export As writes for these fields, past the destination prompt.
fn export_as(app: &mut PhotocraftApp, written: &Written, f: &Map<String, Value>) -> Vec<u8> {
    let doc = source_document(app, f).unwrap();
    let profile = effective_source_profile(app, &doc);
    written.borrow_mut().clear();
    write_export(app, &doc, f, "layer.png", &profile).unwrap();
    written.borrow()[0].1.clone()
}

/// The user answers the file dialog that is open.
fn answer(open: &Open, path: &str) {
    let path = path.to_string();
    let (_, reply) = open.borrow_mut().pop().expect("a file dialog is open");
    std::thread::spawn(move || reply.send(Some(FileDialogAnswer::SaveTo(path)))).join().unwrap();
}

/// A decoded PNG: its size, its channels and its samples.
fn png(bytes: &[u8]) -> (u32, u32, ChannelLayout, Vec<u8>) {
    let img = photocraft_codecs::decode(bytes).unwrap();
    let (w, h) = img.dimensions();
    (w, h, img.layout(), img.data().to_vec())
}

/// The pixel of a decoded 8-bit PNG at `x`,`y`.
fn px(data: &[u8], w: u32, channels: usize, x: u32, y: u32) -> Vec<u8> {
    let at = (y * w + x) as usize * channels;
    data[at..at + channels].to_vec()
}

/// The pixels of an 8-bit RGBA PNG with a zero alpha.
fn clear_pixels(data: &[u8], w: u32, h: u32) -> Vec<(u32, u32)> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|(x, y)| px(data, w, 4, *x, *y)[3] == 0).collect()
}

#[test]
fn an_opaque_layer_png_keeps_its_alpha_channel() {
    let (mut app, written, _open) = app(100, 100);
    let layer = painted(&mut app, "Square", "#1e1ec8", None);
    let (w, h, layout, data) = png(&export_as(&mut app, &written, &fields(Some(layer))));
    assert_eq!((w, h), (100, 100));
    assert_eq!(layout, ChannelLayout::Rgba, "the reference application writes RGBA for an opaque layer");
    assert_eq!(px(&data, w, 4, 50, 50), vec![30, 30, 200, 255]);
    assert!(data.as_chunks::<4>().0.iter().all(|p| p[3] == 255), "alpha 255 everywhere");
}

#[test]
fn a_half_opaque_layer_keeps_its_opacity_as_alpha() {
    let (mut app, written, _open) = app(60, 60);
    let layer = painted(&mut app, "Green", "#00a000", None);
    app.run("layer.setProps", json!({"layer": layer, "opacity": 0.5})).unwrap();
    let (w, _h, layout, data) = png(&export_as(&mut app, &written, &fields(Some(layer))));
    assert_eq!(layout, ChannelLayout::Rgba);
    assert_eq!(px(&data, w, 4, 30, 30), vec![0, 160, 0, 128], "the measured 50 % green layer");
}

#[test]
fn a_groups_empty_corner_is_stored_as_white_with_alpha_zero() {
    let (mut app, written, _open) = app(70, 60);
    painted(&mut app, "Yellow", "#ffff00", Some((0, 0, 40, 40)));
    painted(&mut app, "Cyan", "#00ffff", Some((30, 30, 40, 30)));
    app.run("select.allLayers", json!({})).unwrap();
    app.run("layer.new.groupFromLayers", json!({"name": "Pair"})).unwrap();
    let group = active(&app);
    let (w, h, layout, data) = png(&export_as(&mut app, &written, &fields(Some(group))));
    assert_eq!((w, h), (70, 60), "the group's bounds are its children's union");
    assert_eq!(layout, ChannelLayout::Rgba);
    assert_eq!(px(&data, w, 4, 0, 59), vec![255, 255, 255, 0], "the empty corner of the union bounds");
    // Where a child paints, it keeps its own colour.
    assert_eq!(px(&data, w, 4, 20, 20), vec![255, 255, 0, 255]);
    assert_eq!(px(&data, w, 4, 50, 45), vec![0, 255, 255, 255]);
}

/// #14990: the group's empty area is white right up to its children - the one-pixel ring next to a
/// child's pixels is empty area too, not something the child painted.
#[test]
fn a_groups_empty_area_is_white_up_to_the_children() {
    let (mut app, written, _open) = app(70, 60);
    painted(&mut app, "Yellow", "#ffff00", Some((0, 0, 40, 40)));
    painted(&mut app, "Cyan", "#00ffff", Some((30, 30, 40, 30)));
    app.run("select.allLayers", json!({})).unwrap();
    app.run("layer.new.groupFromLayers", json!({"name": "Pair"})).unwrap();
    let group = active(&app);
    let (w, h, _layout, data) = png(&export_as(&mut app, &written, &fields(Some(group))));
    assert_eq!((w, h), (70, 60));
    // One row above the cyan layer's top edge, and one pixel right of the yellow layer's edge:
    // empty area both, next to a child's coverage.
    assert_eq!(px(&data, w, 4, 45, 29), vec![255, 255, 255, 0], "the ring above the cyan child");
    assert_eq!(px(&data, w, 4, 45, 30), vec![0, 255, 255, 255], "one row down, the cyan child paints");
    assert_eq!(px(&data, w, 4, 40, 20), vec![255, 255, 255, 0], "the ring right of the yellow child");
    assert_eq!(px(&data, w, 4, 39, 20), vec![255, 255, 0, 255], "one pixel left, the yellow child paints");
}

/// #14990: an outside stroke's own colour survives in its clear corner: the leaf's painted box grown
/// by the effect's margin is where the effect painted, and the group's bounds are that box.
#[test]
fn a_group_of_a_stroked_layer_keeps_the_effects_corner() {
    let (mut app, written, _open) = app(60, 60);
    let layer = painted(&mut app, "Square", "#000000", Some((16, 16, 24, 24)));
    app.run("layer.layerStyle.stroke", json!({"layer": layer, "size": 5, "position": "outside", "color": "#000000"})).unwrap();
    app.run("select.allLayers", json!({})).unwrap();
    app.run("layer.new.groupFromLayers", json!({"name": "Framed"})).unwrap();
    let group = active(&app);
    let (w, h, layout, data) = png(&export_as(&mut app, &written, &fields(Some(group))));
    assert_eq!((w, h), (34, 34), "the leaf's 24 x 24 plus 5 px of stroke on each side");
    assert_eq!(layout, ChannelLayout::Rgba);
    assert_eq!(px(&data, w, 4, 0, 0), vec![0, 0, 0, 0], "the corner the stroke reaches keeps its black");
    assert!(
        clear_pixels(&data, w, h).iter().all(|p| px(&data, w, 4, p.0, p.1) == vec![0, 0, 0, 0]),
        "everything inside the grown box keeps the effect's colour"
    );
}

/// #14990: the same corner on a plain pixel layer, a kind the reference application did not measure
/// for clear pixels: it keeps today's 0,0,0,0.
#[test]
fn a_stroked_pixel_layer_keeps_its_clear_pixels_as_they_are() {
    let (mut app, written, _open) = app(60, 60);
    let layer = painted(&mut app, "Square", "#000000", Some((16, 16, 24, 24)));
    app.run("layer.layerStyle.stroke", json!({"layer": layer, "size": 5, "position": "outside", "color": "#000000"})).unwrap();
    let (w, h, layout, data) = png(&export_as(&mut app, &written, &fields(Some(layer))));
    assert_eq!((w, h), (34, 34));
    assert_eq!(layout, ChannelLayout::Rgba);
    assert_eq!(px(&data, w, 4, 0, 0), vec![0, 0, 0, 0]);
    assert!(clear_pixels(&data, w, h).iter().all(|p| px(&data, w, 4, p.0, p.1) == vec![0, 0, 0, 0]), "an unmeasured kind is left alone");
}

/// #14990: a plain pixel layer with a clear area inside its bounds is unmeasured too, so its clear
/// pixels stay as they are instead of turning white.
#[test]
fn an_unmeasured_layer_kind_keeps_its_clear_pixels() {
    let (mut app, written, _open) = app(60, 40);
    app.run("layer.new.layer", json!({"name": "Bars"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 40})).unwrap();
    app.run("select.rect", json!({"x": 30, "y": 0, "width": 10, "height": 40, "mode": "add"})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let layer = active(&app);
    let (w, h, _layout, data) = png(&export_as(&mut app, &written, &fields(Some(layer))));
    assert_eq!((w, h), (40, 40), "the layer's bounds are its two bars' union");
    assert_eq!(px(&data, w, 4, 20, 20), vec![0, 0, 0, 0], "the gap between the bars keeps today's colour");
    assert_eq!(px(&data, w, 4, 5, 20), vec![255, 0, 0, 255], "and the bars themselves are as they were");
}

#[test]
fn quick_export_of_a_layer_writes_rgba() {
    let (mut app, written, open) = app(40, 30);
    painted(&mut app, "Square", "#1e1ec8", None);
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "layer.quickExportAsPng", Value::Null).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, "layer.png");
    app.poll_file_dialog(&ctx, None);
    assert_eq!(written.borrow().len(), 1, "one PNG for the layer");
    let (w, h, layout, data) = png(&written.borrow()[0].1);
    assert_eq!((w, h), (40, 30));
    assert_eq!(layout, ChannelLayout::Rgba, "Quick Export as PNG of a layer keeps its alpha channel");
    assert!(data.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
}

#[test]
fn quick_export_of_several_layers_writes_one_rgba_png_each() {
    let (mut app, written, open) = app(40, 30);
    painted(&mut app, "Upper", "#ff0000", Some((0, 0, 20, 20)));
    painted(&mut app, "Lower", "#0000ff", Some((10, 10, 20, 20)));
    app.run("select.allLayers", json!({})).unwrap();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "layer.quickExportAsPng", Value::Null).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, "layers.png");
    app.poll_file_dialog(&ctx, None);
    assert_eq!(written.borrow().len(), 2, "one file per selected layer");
    for (path, bytes) in written.borrow().iter() {
        assert_eq!(png(bytes).2, ChannelLayout::Rgba, "{path}");
    }
}

#[test]
fn export_as_of_the_whole_opaque_document_still_writes_rgb() {
    let (mut app, written, _open) = app(40, 30);
    // The Background alone: an opaque composite, which File › Export As keeps writing as RGB.
    let (w, h, layout, data) = png(&export_as(&mut app, &written, &fields(None)));
    assert_eq!((w, h), (40, 30));
    assert_eq!(layout, ChannelLayout::Rgb, "a whole document's opaque composite still gets an RGB file");
    assert_eq!(data.len(), 40 * 30 * 3);

    // File › Export › Quick Export as PNG shares the encoder and keeps it too.
    let doc = app.session.active().unwrap().doc.clone();
    let out = photocraft_io::export(&doc, "photo.png", &Default::default()).unwrap();
    assert_eq!(png(&out.bytes).2, ChannelLayout::Rgb);
}

#[test]
fn the_layer_commands_write_rgba_too() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _written, _open) = app(40, 30);
    let layer = painted(&mut app, "Square", "#1e1ec8", None);
    for (cmd, name) in [("layer.exportAs", "export-as.png"), ("layer.quickExportAsPng", "quick.png")] {
        let path = dir.path().join(name).to_string_lossy().into_owned();
        app.run(cmd, json!({"layer": layer, "path": path})).unwrap();
        let (_w, _h, layout, data) = png(&std::fs::read(&path).unwrap());
        assert_eq!(layout, ChannelLayout::Rgba, "{cmd}");
        assert!(data.as_chunks::<4>().0.iter().all(|p| p[3] == 255), "{cmd}: alpha 255 everywhere");
    }
}

/// The rule rides on the dialog's settings, and only on the layer branches' settings.
#[test]
fn only_the_layer_branches_ask_for_the_kept_alpha() {
    let (mut app, _written, _open) = app(8, 8);
    let layer = painted(&mut app, "Square", "#ff0000", None);
    assert!(settings(&fields(Some(layer))).layer_alpha, "Export As of a layer asks for it");
    let mut several = fields(None);
    several.insert("__layers".into(), json!([layer]));
    assert!(settings(&several).layer_alpha, "and so does Export As of several layers");
    assert!(!settings(&fields(None)).layer_alpha, "File > Export As asks for nothing");
}
