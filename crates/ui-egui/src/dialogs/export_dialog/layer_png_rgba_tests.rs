//! #14816: Layer › Export As and Layer › Quick Export as PNG always write an RGBA PNG: an opaque
//! layer keeps alpha 255, a layer at 50 % opacity keeps its opacity as alpha, and a pixel nothing
//! painted inside the exported bounds is stored as 255,255,255,0. File › Export As of a whole
//! document keeps writing RGB for an opaque composite, and so does File › Export › Quick Export.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use photocraft_codecs::ChannelLayout;
use serde_json::{Map, Value, json};

use super::{effective_source_profile, estimate, export_document, preview_proxy, settings, source_document, source_scoped_settings, write_export};
#[cfg(not(target_arch = "wasm32"))]
use super::{estimate_native, exact_estimate};
use crate::{PhotocraftApp, Services, menus};
use craft::host::{Picked, ScriptedFiles};

/// What the test writer was handed: the path and the encoded bytes.
type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// An app with one document that really encodes, whose writer hands the bytes back to the test:
/// what the layer export paths produce can then be read without touching the disc. Its file
/// dialogs stay open until the test answers them.
fn app(width: u32, height: u32) -> (Box<PhotocraftApp>, Written, Arc<ScriptedFiles>) {
    let written = Written::default();
    let w = written.clone();
    let files = Arc::new(ScriptedFiles::held());
    let services = Services {
        files: Some(files.clone()),
        export: Some(Box::new(|doc, path, s| {
            let out = photocraft_io::export(doc, path, &s.io_options()).map_err(|e| e.to_string())?;
            Ok((out.bytes, out.warnings))
        })),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            w.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": width, "height": height})).unwrap();
    (app, written, files)
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
fn answer(files: &ScriptedFiles, path: &str) {
    files.answer(Picked::Paths(vec![path.into()])).expect("a file dialog is open");
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

/// #14968 review: the exported group's own layer style paints as much as a child's does. The
/// fully clear corner of a 5 px outside stroke on the exported group's root must stay the
/// stroke's black, exactly as when the same stroke sits on a leaf inside the group.
#[test]
fn a_group_with_its_own_stroke_keeps_the_effects_corner() {
    let (mut app, written, _open) = app(60, 60);
    painted(&mut app, "Square", "#000000", Some((16, 16, 24, 24)));
    app.run("select.allLayers", json!({})).unwrap();
    app.run("layer.new.groupFromLayers", json!({"name": "Framed"})).unwrap();
    let group = active(&app);
    app.run("layer.layerStyle.stroke", json!({"layer": group, "size": 5, "position": "outside", "color": "#000000"})).unwrap();
    let (w, h, layout, data) = png(&export_as(&mut app, &written, &fields(Some(group))));
    assert_eq!((w, h), (34, 34), "the child's 24 x 24 plus the group's own 5 px of stroke on each side");
    assert_eq!(layout, ChannelLayout::Rgba);
    assert_eq!(px(&data, w, 4, 0, 0), vec![0, 0, 0, 0], "the corner the group's own stroke reaches keeps its black");
    assert!(
        clear_pixels(&data, w, h).iter().all(|p| px(&data, w, 4, p.0, p.1) == vec![0, 0, 0, 0]),
        "everything inside the box grown by the group's own stroke keeps the effect's colour"
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
    let (mut app, written, files) = app(40, 30);
    painted(&mut app, "Square", "#1e1ec8", None);
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "layer.quickExportAsPng", Value::Null).unwrap();
    answer(&files, "layer.png");
    app.poll_file_dialog();
    assert_eq!(written.borrow().len(), 1, "one PNG for the layer");
    let (w, h, layout, data) = png(&written.borrow()[0].1);
    assert_eq!((w, h), (40, 30));
    assert_eq!(layout, ChannelLayout::Rgba, "Quick Export as PNG of a layer keeps its alpha channel");
    assert!(data.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
}

#[test]
fn quick_export_of_several_layers_writes_one_rgba_png_each() {
    let (mut app, written, files) = app(40, 30);
    painted(&mut app, "Upper", "#ff0000", Some((0, 0, 20, 20)));
    painted(&mut app, "Lower", "#0000ff", Some((10, 10, 20, 20)));
    app.run("select.allLayers", json!({})).unwrap();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "layer.quickExportAsPng", Value::Null).unwrap();
    answer(&files, "layers.png");
    app.poll_file_dialog();
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

/// #14816 review: which of a layer's clear pixels read as white must not depend on how the PNG is
/// written. In the version under review that was decided per band, and the last row of a band is a
/// band boundary at every band size - down to the image's own bottom row, where a group's children
/// reach least. The document Export As hands the encoder already stores its never-painted area as
/// 255,255,255,0, so every band size writes the whole-image file's bytes and whites that row.
#[test]
fn a_group_layer_png_stores_the_same_pixels_whatever_the_band_size() {
    let (mut app, _written, _open) = app(70, 60);
    painted(&mut app, "Yellow", "#ffff00", Some((0, 0, 40, 40)));
    painted(&mut app, "Cyan", "#00ffff", Some((30, 30, 40, 30)));
    app.run("select.allLayers", json!({})).unwrap();
    app.run("layer.new.groupFromLayers", json!({"name": "Pair"})).unwrap();
    let group = active(&app);
    let doc = source_document(&app, &fields(Some(group))).unwrap();
    let opts = photocraft_io::ExportOptions { layer_alpha: true, ..Default::default() };
    let buffered = photocraft_io::export_flat(&doc, photocraft_codecs::Format::Png, &opts).unwrap();
    let (w, h, layout, whole) = png(&buffered.bytes);
    assert_eq!((w, h, layout), (70, 60, ChannelLayout::Rgba), "the whole-image layer PNG");
    // The baseline the bands are held to: the bottom row is clear up to the cyan child, and the
    // pixels nothing painted inside the group's bounds are stored white with a zero alpha.
    assert_eq!(px(&whole, w, 4, 0, 59), vec![255, 255, 255, 0], "the buffered file whites its clear bottom row");
    assert_eq!(px(&whole, w, 4, 15, 59), vec![255, 255, 255, 0], "and the clear pixels along that row");
    assert_eq!(px(&whole, w, 4, 50, 59), vec![0, 255, 255, 255], "where the cyan child paints, its colour stays");
    let ctl = photocraft_raster::Interrupt::NONE;
    for rows in [1, 7] {
        let streamed = photocraft_io::export_png_streamed_with_renderer(&doc, &opts, &ctl, rows, None).unwrap();
        let (bw, bh, blayout, band) = png(&streamed.bytes);
        assert_eq!((bw, bh, blayout), (w, h, layout), "written {rows} rows at a time");
        assert_eq!(band, whole, "{rows} rows at a time stores different RGBA bytes");
        assert_eq!(px(&band, w, 4, 0, 59), vec![255, 255, 255, 0], "{rows} rows at a time whites the bottom row too");
        assert_eq!(px(&band, w, 4, 50, 59), vec![0, 255, 255, 255], "{rows} rows at a time keeps the child's colour");
    }
}

/// #14816 review: a group's own bounds are the union of its children's, so a nested group paints
/// nothing of its own - the leaf boxes decide which pixels nothing ever painted. Wrapping that group
/// in a second group must therefore not stop its empty corner from being stored white, in either
/// encoder.
#[test]
fn a_nested_group_layer_png_whites_the_corner_its_leaves_never_painted() {
    let (mut app, _written, _open) = app(70, 60);
    painted(&mut app, "Yellow", "#ffff00", Some((0, 0, 40, 40)));
    painted(&mut app, "Cyan", "#00ffff", Some((30, 30, 40, 30)));
    app.run("select.allLayers", json!({})).unwrap();
    app.run("layer.new.groupFromLayers", json!({"name": "Pair"})).unwrap();
    let inner = active(&app);
    app.run("layer.new.groupFromLayers", json!({"layer": inner, "name": "Outer"})).unwrap();
    let outer = active(&app);
    let doc = source_document(&app, &fields(Some(outer))).unwrap();
    let opts = photocraft_io::ExportOptions { layer_alpha: true, ..Default::default() };
    let buffered = photocraft_io::export_flat(&doc, photocraft_codecs::Format::Png, &opts).unwrap();
    let (w, h, layout, whole) = png(&buffered.bytes);
    assert_eq!((w, h, layout), (70, 60, ChannelLayout::Rgba), "the outer group's bounds are its leaves' union");
    assert_eq!(px(&whole, w, 4, 0, 59), vec![255, 255, 255, 0], "the corner no leaf of the nested group ever painted");
    assert_eq!(px(&whole, w, 4, 50, 59), vec![0, 255, 255, 255], "where the cyan leaf paints, its colour stays");
    let ctl = photocraft_raster::Interrupt::NONE;
    for rows in [1, 7] {
        let streamed = photocraft_io::export_png_streamed_with_renderer(&doc, &opts, &ctl, rows, None).unwrap();
        let (_bw, _bh, _blayout, band) = png(&streamed.bytes);
        assert_eq!(band, whole, "{rows} rows at a time stores different RGBA bytes");
    }
}

/// #14968 review: the one case where an intermediate group does add its own (union) box to the
/// reach is a group with an enabled layer style - a plain nested group adds nothing, pinned by
/// `a_nested_group_layer_png_whites_the_corner_its_leaves_never_painted`. The reference
/// application was not measured for a styled intermediate group, so that case keeps today's
/// behaviour: the box grown by the style keeps its written colours, its empty corners included.
#[test]
fn an_intermediate_group_with_a_style_keeps_its_union_box_as_it_is() {
    let (mut app, written, _open) = app(70, 60);
    painted(&mut app, "Yellow", "#ffff00", Some((0, 0, 40, 40)));
    painted(&mut app, "Cyan", "#00ffff", Some((30, 30, 40, 30)));
    app.run("select.allLayers", json!({})).unwrap();
    app.run("layer.new.groupFromLayers", json!({"name": "Pair"})).unwrap();
    let inner = active(&app);
    app.run("layer.layerStyle.stroke", json!({"layer": inner, "size": 5, "position": "outside", "color": "#000000"})).unwrap();
    app.run("layer.new.groupFromLayers", json!({"layer": inner, "name": "Outer"})).unwrap();
    let outer = active(&app);
    let (w, h, layout, data) = png(&export_as(&mut app, &written, &fields(Some(outer))));
    assert_eq!((w, h), (70, 60), "the children still touch the canvas corners, and the 5 px group stroke is clipped by it");
    assert_eq!(layout, ChannelLayout::Rgba);
    // No leaf painted (0,59), and the 5 px stroke of the styled inner group never reaches it
    // either - but it lies inside that group's grown box, and stays as it was written.
    assert_eq!(px(&data, w, 4, 0, 59), vec![0, 0, 0, 0], "unmeasured: a styled intermediate group's grown box keeps its colours");
    assert_eq!(px(&data, w, 4, 50, 45), vec![0, 255, 255, 255], "where a leaf paints, its colour stays");
}

/// #14968 review: the whitened area must read the same however many child rectangles build the
/// reach. A 100 x 100 group of 100 small children: the empty corner and the gaps between
/// children are stored white exactly as for two children. (The finding's 4096 x 4096 with 1000
/// rectangles is a cost scenario, not an output one, and too large to run in a unit test.)
#[test]
fn a_group_with_many_child_rectangles_still_whites_its_empty_area() {
    let mut doc = photocraft_doc::Document::new("Many", photocraft_doc::Size::new(100, 100), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let fmt = doc.pixel_format();
    let mut kids = Vec::new();
    for k in 0..100 {
        let (x, y) = ((k % 10) * 9, (k / 10) * 9);
        let mut s = photocraft_raster::Surface::new(fmt);
        s.fill_rect(photocraft_doc::Rect::new(x, y, x + 3, y + 3), &[0.0, 1.0, 0.0, 1.0]);
        kids.push(photocraft_doc::Layer::new("Dot", photocraft_doc::LayerContent::Raster(s)));
    }
    let group = photocraft_doc::Layer::group("Many", kids);
    let id = group.id;
    doc.layers = vec![group].into();
    let alone = photocraft_engine::layer_menu_cmds::layer_document(&doc, id).unwrap();
    let opts = photocraft_io::ExportOptions { layer_alpha: true, ..Default::default() };
    let out = photocraft_io::export_flat(&alone, photocraft_codecs::Format::Png, &opts).unwrap();
    let (w, h, layout, data) = png(&out.bytes);
    assert_eq!((w, h, layout), (84, 84, ChannelLayout::Rgba), "the 100 children's union");
    assert_eq!(px(&data, w, 4, 0, 79), vec![255, 255, 255, 0], "the gap along the left edge no child painted");
    assert_eq!(px(&data, w, 4, 4, 4), vec![255, 255, 255, 0], "the gap between four neighbouring children");
    assert_eq!(px(&data, w, 4, 1, 1), vec![0, 255, 0, 255], "where a child paints, its colour stays");
}

/// A 70 x 60 document holding one group of two offset children, so its bottom row is clear up to
/// x=30; `paints` are the two children's pixel values in the document's own channel order.
fn group_document(
    mode: photocraft_doc::ColorMode,
    depth: photocraft_doc::SampleType,
    paints: [&[f32]; 2],
) -> (photocraft_doc::Document, photocraft_doc::LayerId) {
    let mut doc = photocraft_doc::Document::new("Unmeasured", photocraft_doc::Size::new(70, 60), mode, depth);
    let fmt = doc.pixel_format();
    let leaves = [(photocraft_doc::Rect::new(0, 0, 40, 40), paints[0]), (photocraft_doc::Rect::new(30, 30, 70, 60), paints[1])].map(|(r, px)| {
        let mut s = photocraft_raster::Surface::new(fmt);
        s.fill_rect(r, px);
        photocraft_doc::Layer::new("Leaf", photocraft_doc::LayerContent::Raster(s))
    });
    let group = photocraft_doc::Layer::group("Pair", leaves.into());
    let id = group.id;
    doc.layers = vec![group].into();
    (doc, id)
}

/// The reference application was measured for the clear-pixel colour of a group export on an RGB
/// document at 8 bits (#14816, #14990), so a 16-bit or a grayscale document keeps the colours its
/// layers left instead of storing the never-painted area as white.
#[test]
fn a_group_export_in_an_unmeasured_mode_or_depth_keeps_its_clear_pixels() {
    let opts = photocraft_io::ExportOptions { layer_alpha: true, ..Default::default() };
    let magenta16 = [0.9, 0.0, 0.9, 1.0];
    let teal16 = [0.0, 0.6, 0.6, 1.0];
    let light8 = [0.9, 1.0];
    let dark8 = [0.2, 1.0];
    let cases: [(&str, photocraft_doc::ColorMode, photocraft_doc::SampleType, [&[f32]; 2]); 2] = [
        ("16-bit RGB", photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U16, [&magenta16, &teal16]),
        ("8-bit grayscale", photocraft_doc::ColorMode::Grayscale, photocraft_doc::SampleType::U8, [&light8, &dark8]),
    ];
    for (label, mode, depth, paints) in cases {
        let (doc, id) = group_document(mode, depth, paints);
        let alone = photocraft_engine::layer_menu_cmds::layer_document(&doc, id).unwrap();
        let out = photocraft_io::export_flat(&alone, photocraft_codecs::Format::Png, &opts).unwrap();
        let img = photocraft_codecs::decode(&out.bytes).unwrap();
        let (w, h) = img.dimensions();
        let channels = img.layout().channels();
        let bytes = img.data().len() / (w as usize * h as usize * channels);
        let at = ((h - 1) as usize * w as usize) * channels * bytes;
        assert_eq!(
            img.data()[at..at + channels * bytes].to_vec(),
            vec![0u8; channels * bytes],
            "{label}: a clear pixel of the bottom row keeps its colour, it is not turned white"
        );
    }
}

/// A document in `mode` at 8 bits with one layer opaque over the whole 30 x 20 canvas, added as
/// the app's active document; the new layer's id.
fn opaque_layer_in(app: &mut PhotocraftApp, mode: photocraft_doc::ColorMode, paint: &[f32]) -> photocraft_doc::LayerId {
    let mut doc = photocraft_doc::Document::new("Opaque", photocraft_doc::Size::new(30, 20), mode, photocraft_doc::SampleType::U8);
    let fmt = doc.pixel_format();
    let mut s = photocraft_raster::Surface::new(fmt);
    s.fill_rect(photocraft_doc::Rect::new(0, 0, 30, 20), paint);
    let layer = photocraft_doc::Layer::new("Opaque", photocraft_doc::LayerContent::Raster(s));
    let id = layer.id;
    doc.layers = vec![layer].into();
    app.session.add_document(doc, None);
    id
}

/// #14968 review: under the dialog's default "Convert to sRGB", `export_document` converts a
/// grayscale or CMYK document into an RGB 8-bit one before the encoder applies the kept-alpha
/// rule - and that rule was measured for RGB 8-bit sources. It must follow the source document,
/// not the converted copy: an opaque layer of a grayscale or CMYK source keeps the RGB file
/// (alpha dropped) it had before #14816, as those modes are marked unmeasured.
#[test]
fn an_opaque_layer_of_an_unmeasured_source_mode_keeps_todays_file_under_srgb() {
    let (mut app, written, _open) = app(40, 30);
    let cases: [(&str, photocraft_doc::ColorMode, &[f32]); 2] =
        [("grayscale 8-bit", photocraft_doc::ColorMode::Grayscale, &[0.5, 1.0]), ("CMYK 8-bit", photocraft_doc::ColorMode::Cmyk, &[0.2, 0.7, 0.1, 0.05, 1.0])];
    for (label, mode, paint) in cases {
        let id = opaque_layer_in(&mut app, mode, paint);
        let mut f = fields(Some(id.0));
        f.insert("profile".into(), json!("srgb"));
        let (w, h, layout, data) = png(&export_as(&mut app, &written, &f));
        assert_eq!((w, h), (30, 20), "{label}");
        assert_eq!(layout, ChannelLayout::Rgb, "{label}: unmeasured by #14816, so an opaque layer export still drops its alpha, as before the change");
        assert_eq!(data.len(), 30 * 20 * 3, "{label}: the sRGB dialog still wrote an RGB file");
    }
}

/// The app for the estimate tests: a real encoder, plus a log of the settings `estimate` was
/// handed, so what the "~ N KB" label encodes is observable without touching the disc.
fn estimate_app() -> (Box<PhotocraftApp>, Rc<RefCell<Vec<crate::services::ExportSettings>>>) {
    let asked: Rc<RefCell<Vec<crate::services::ExportSettings>>> = Rc::default();
    let log = asked.clone();
    let services = Services {
        export: Some(Box::new(move |doc, path, s| {
            log.borrow_mut().push(s.clone());
            let out = photocraft_io::export(doc, path, &s.io_options()).map_err(|e| e.to_string())?;
            Ok((out.bytes, out.warnings))
        })),
        ..Default::default()
    };
    (PhotocraftApp::new(photocraft_engine::Session::new(), services), asked)
}

/// #14968 review (estimate edge): the dialog's size estimates run the same encoder as the writer,
/// but at the review's head they asked it for the unscoped `settings(f)` while `write_export` and
/// `confirm_layers` scope the kept alpha on the source document. For a grayscale source under
/// "Convert to sRGB" the written file is RGB while the label sized an RGBA encode of the converted
/// copy. Every estimate path must ask for exactly the settings the writer will use: the proxy
/// `estimate` (its encoder settings observed directly), the native proxy `estimate_native`, and
/// the full `exact_estimate`.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn the_size_estimates_use_the_scoped_settings_for_an_unmeasured_source_mode() {
    let (mut app, written, _open) = app(40, 30);
    let id = opaque_layer_in(&mut app, photocraft_doc::ColorMode::Grayscale, &[0.5, 1.0]);
    let mut f = fields(Some(id.0));
    f.insert("profile".into(), json!("srgb"));
    let doc = source_document(&app, &f).unwrap();
    let profile = effective_source_profile(&app, &doc);
    // The writer's answer: an RGB file (pinned by the test above); this test is about its size.
    let file = export_as(&mut app, &written, &f);
    assert_eq!(png(&file).2, ChannelLayout::Rgb);
    // The converted copy every estimate path encodes, and what an unscoped RGBA encode of it
    // costs: the symptom, and proof the two encodes differ in size, so a length can tell them apart.
    let full = export_document(&doc, &f, None, &profile).unwrap();
    let rgba = photocraft_io::export(&full, "estimate.png", &settings(&f).io_options()).unwrap().bytes;
    assert_eq!(png(&rgba).2, ChannelLayout::Rgba, "the unscoped settings size an RGBA encode");
    assert_ne!(rgba.len(), file.len(), "the RGB file and the RGBA encode differ in size");
    // The proxy paths: this small document keeps the proxy at full size, so its estimate is the
    // encode's own byte count.
    let proxy = preview_proxy(&doc, &f, &profile).unwrap();
    assert_eq!(estimate_native(&doc, &proxy, &f), Some(file.len() as u64));
    // `estimate` goes through the app's exporter: ask it directly what it was handed.
    let (app2, asked) = estimate_app();
    assert_eq!(estimate(&app2, &doc, &proxy, &f), Some(file.len() as u64));
    assert_eq!(asked.borrow().as_slice(), &[source_scoped_settings(&doc, &f)], "the estimate encodes with the writer's settings");
    // The exact estimate encodes the full image; its length must be the writer's file size.
    assert_eq!(exact_estimate(&doc, &f, &profile), Some(file.len() as u64));
}

/// The measured layer case, the estimate side: an RGB 8-bit document with a partly transparent
/// layer keeps the kept-alpha request in the estimates too, so the label sizes the RGBA encode the
/// writer really writes. Guard test, passes before and after the fix (the unscoped settings agree
/// with the scoped ones for this source).
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn the_size_estimates_keep_rgba_for_the_measured_layer_case() {
    let (mut app, written, _open) = app(60, 60);
    let layer = painted(&mut app, "Green", "#00a000", None);
    app.run("layer.setProps", json!({"layer": layer, "opacity": 0.5})).unwrap();
    let f = fields(Some(layer));
    let doc = source_document(&app, &f).unwrap();
    let profile = effective_source_profile(&app, &doc);
    let file = export_as(&mut app, &written, &f);
    assert_eq!(png(&file).2, ChannelLayout::Rgba, "the writer keeps the alpha channel");
    let proxy = preview_proxy(&doc, &f, &profile).unwrap();
    assert_eq!(estimate_native(&doc, &proxy, &f), Some(file.len() as u64));
    let (app2, asked) = estimate_app();
    assert_eq!(estimate(&app2, &doc, &proxy, &f), Some(file.len() as u64));
    assert_eq!(asked.borrow().as_slice(), &[source_scoped_settings(&doc, &f)], "and asks for it with the writer's settings");
    assert!(asked.borrow()[0].layer_alpha, "the RGB 8-bit source keeps the request");
    assert_eq!(exact_estimate(&doc, &f, &profile), Some(file.len() as u64));
}
