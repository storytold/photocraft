//! #15650: a PDF-compatible AI that another application placed with its artwork's bounds as the
//! frame keeps that frame when the layer is re-rendered, instead of filling it with the page.

use super::*;

/// A 400 × 300 pt artboard with a red 100 × 50 pt rectangle whose top-left is at (100, 50).
fn artwork() -> Vec<u8> {
    pdf(&["1 0 0 rg 100 200 100 50 re f"])
}

/// 400 × 300 pt artboards, one per entry of `contents`.
fn pdf(contents: &[&str]) -> Vec<u8> {
    let kids: Vec<String> = (0..contents.len()).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
    let mut objects = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string(), format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), contents.len())];
    for (i, content) in contents.iter().enumerate() {
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Contents {} 0 R /Resources << >> >>", 4 + 2 * i));
        objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

/// A 200 × 150 document with the artwork placed as Photoshop places it: its 100 × 50 bounds
/// are the frame, drawn at (20, 30), with the pixels Photoshop saved as the cache.
fn placed(frame: (f64, f64)) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 150, "background": "transparent"})).unwrap();
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let transform = Affine::translate(20.0, 30.0);
    let mut cache = Surface::new(doc.pixel_format());
    cache.fill_rect(Rect::new(20, 30, 120, 80), &RED);
    let source = SmartSource::Embedded { file_name: "Logo.ai".into(), bytes: Arc::new(artwork()) };
    let mut sm = SmartObject::new(source, transform, Some(cache));
    let spec = photocraft_io::smart_map::PlacedSpec {
        idnt: "logo",
        placed: "logo",
        transform,
        perspective: None,
        size: frame,
        dpi: 72.0,
        warp: None,
        filter_fx: None,
    };
    sm.psd_raw = Some(Arc::new(photocraft_io::smart_map::sold_bytes(None, &spec, &mut vec![])));
    let layer = Layer::new("Logo", LayerContent::Smart(sm));
    let id = layer.id;
    doc.layers.push(layer);
    (s, id)
}

fn cache(s: &Session, id: LayerId) -> Surface {
    smart(&s.active().unwrap().doc, id).unwrap().cache.clone().unwrap()
}

#[test]
fn image_size_keeps_a_placed_ai_at_its_placed_bounds() {
    let (mut s, id) = placed((100.0, 50.0));
    s.execute("image.imageSize", json!({"width": 180, "height": 135})).unwrap();
    let px = cache(&s, id);
    let b = px.content_bounds();
    assert!((17..=19).contains(&b.x0) && (26..=28).contains(&b.y0) && (107..=109).contains(&b.x1) && (71..=73).contains(&b.y1), "{b:?}");
    let centre = px.pixel(63, 49);
    assert!(centre.iter().zip(RED).all(|(a, b)| (a - b).abs() < 0.02), "{centre:?}");
}

#[test]
fn a_placed_ai_re_renders_in_its_frame() {
    let (s, id) = placed((100.0, 50.0));
    let doc = &s.active().unwrap().doc;
    let rendered = render(doc, smart(doc, id).unwrap()).unwrap().unwrap();
    assert_eq!(rendered.content_bounds(), Rect::new(20, 30, 120, 80));
}

#[test]
fn a_frame_matching_neither_the_artwork_nor_the_page_keeps_the_saved_pixels() {
    let (mut s, id) = placed((100.0, 100.0));
    let doc = &s.active().unwrap().doc;
    assert!(render(doc, smart(doc, id).unwrap()).is_err());
    s.execute("image.imageSize", json!({"width": 100, "height": 75})).unwrap();
    assert_eq!(cache(&s, id).content_bounds(), Rect::new(10, 15, 60, 40), "the saved pixels are scaled instead");
}

#[test]
fn a_frame_that_is_the_pdfs_own_canvas_opens_the_pdf_as_usual() {
    let canvas = photocraft_io::pdf_default_canvas(&artwork()).unwrap();
    let (s, id) = placed((canvas.0 as f64, canvas.1 as f64));
    let doc = &s.active().unwrap().doc;
    let rendered = render(doc, smart(doc, id).unwrap()).unwrap().unwrap();
    // PhotoCraft's own placement: the whole page, on white, from the placement's origin.
    assert_eq!(rendered.pixel(25, 35), vec![1.0, 1.0, 1.0, 1.0]);
    assert_eq!(rendered.content_bounds(), Rect::new(20, 30, 20 + canvas.0 as i32, 30 + canvas.1 as i32));
}

#[test]
fn convert_to_layers_keeps_a_placed_ai_at_its_placed_bounds() {
    let (mut s, id) = placed((100.0, 50.0));
    s.execute("layer.smartObjects.convertToLayers", json!({"layer": id.0})).unwrap();
    let doc = &s.active().unwrap().doc;
    let LayerContent::Raster(px) = &doc.layers.last().unwrap().content else { panic!("a pixel layer: {:?}", doc.layers.last().unwrap().content.kind_name()) };
    assert_eq!(px.content_bounds(), Rect::new(20, 30, 120, 80));
}

#[test]
fn convert_to_layers_keeps_the_saved_pixels_of_a_frame_matching_neither_box() {
    let (mut s, id) = placed((100.0, 100.0));
    s.execute("layer.smartObjects.convertToLayers", json!({"layer": id.0})).unwrap();
    let doc = &s.active().unwrap().doc;
    let layer = doc.layers.last().unwrap();
    let LayerContent::Raster(px) = &layer.content else { panic!("a pixel layer: {:?}", layer.content.kind_name()) };
    assert_eq!(px.content_bounds(), Rect::new(20, 30, 120, 80), "the saved pixels, where they were placed");
    assert_eq!(px.pixel(70, 55), RED.to_vec());
    assert_eq!(layer.name, "Logo");
}

#[test]
fn convert_to_layers_refuses_saved_pixels_that_show_smart_filters() {
    let (mut s, id) = placed((100.0, 100.0));
    let filter = SmartFilter { command: "image.adjustments.invert".into(), params: json!({}), blend: BlendMode::Normal, opacity: 1.0, visible: true };
    let set_filter = |s: &mut Session, visible: bool| {
        let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
        let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("a smart object") };
        sm.smart_filters = vec![SmartFilter { visible, ..filter.clone() }];
    };
    set_filter(&mut s, true);
    let error = s.execute("layer.smartObjects.convertToLayers", json!({"layer": id.0})).unwrap_err();
    assert!(error.to_string().contains("rasterize it instead"), "{error}");
    set_filter(&mut s, false);
    s.execute("layer.smartObjects.convertToLayers", json!({"layer": id.0})).unwrap();
}

#[test]
fn placements_of_different_pages_of_one_pdf_render_their_own_page() {
    let bytes = Arc::new(pdf(&["1 0 0 rg 100 200 100 50 re f", "0 0 1 rg 100 200 100 50 re f"]));
    let (mut s, id) = placed((100.0, 50.0));
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let LayerContent::Smart(first) = &mut doc.layer_mut(id).unwrap().content else { panic!("a smart object") };
    first.source = SmartSource::Embedded { file_name: "Logo.ai".into(), bytes: bytes.clone() };
    let mut second = first.clone();
    let mut d = photocraft_io::smart_map::sold_descriptor(second.psd_raw.as_ref().unwrap()).unwrap();
    d.items.iter_mut().filter(|(k, _)| k.is("PgNm")).for_each(|(_, v)| *v = craft_psd::descriptor::Value::Integer(2));
    let spec = photocraft_io::smart_map::PlacedSpec {
        idnt: "logo",
        placed: "logo",
        transform: second.transform,
        perspective: None,
        size: (100.0, 50.0),
        dpi: 72.0,
        warp: None,
        filter_fx: None,
    };
    second.psd_raw = Some(Arc::new(photocraft_io::smart_map::sold_bytes(Some(&d), &spec, &mut vec![])));
    let doc = &s.active().unwrap().doc;
    let colour = |sm: &SmartObject| render(doc, sm).unwrap().unwrap().pixel(70, 55);
    assert_eq!(colour(smart(doc, id).unwrap()), RED.to_vec());
    assert_eq!(colour(&second), vec![0.0, 0.0, 1.0, 1.0], "page 2's blue artwork, not page 1's from the cache");
}

#[test]
fn warp_grid_and_splits_keep_the_saved_pixels_of_a_frame_matching_neither_box() {
    let (mut s, id) = placed((100.0, 100.0));
    s.active_mut().unwrap().active_layer = Some(id);
    for (cmd, params) in
        [("edit.transform.warpGrid", json!({"size": 3})), ("edit.transform.splitWarpCrosswise", json!({})), ("edit.transform.removeWarpSplit", json!({}))]
    {
        s.execute(cmd, params).unwrap_or_else(|e| panic!("{cmd}: {e}"));
        assert_eq!(cache(&s, id).content_bounds(), Rect::new(20, 30, 120, 80), "{cmd} keeps the saved pixels");
        assert!(smart(&s.active().unwrap().doc, id).unwrap().warp.is_some(), "{cmd} stores the warp");
    }
}

#[test]
fn warp_grid_that_reshapes_an_edited_mesh_fails_rather_than_keep_mismatched_pixels() {
    let (mut s, id) = placed((100.0, 100.0));
    s.active_mut().unwrap().active_layer = Some(id);
    s.execute("edit.transform.warpGrid", json!({"size": 3})).unwrap();
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("a smart object") };
    let mesh = sm.warp.as_mut().and_then(|w| w.mesh.as_mut()).expect("a 3 × 3 mesh");
    // An inner control point, moved: a single patch can't follow that bend.
    mesh.points[3 * 10 + 4][1] += 20.0;
    let error = s.execute("edit.transform.warpGrid", json!({"size": 1})).unwrap_err();
    assert!(error.to_string().contains("cropped"), "{error}");
    assert_eq!(cache(&s, id).content_bounds(), Rect::new(20, 30, 120, 80), "the edit is undone");
}

#[test]
fn removing_a_split_that_drops_an_edit_inside_one_small_patch_fails() {
    let (mut s, id) = placed((100.0, 100.0));
    s.active_mut().unwrap().active_layer = Some(id);
    s.execute("edit.transform.warpGrid", json!({"size": 64})).unwrap();
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("a smart object") };
    let mesh = sm.warp.as_mut().and_then(|w| w.mesh.as_mut()).expect("a 64 × 64 mesh");
    // The second handle of the column patch over [31/64, 32/64], on the row at t = 1/2. Removing
    // the split at 1/2 rebuilds that column from the other handles, so the bend is lost, but only
    // between 31/64 and 32/64, where no point of a 1/16 grid lies.
    let nx = 3 * 64 + 1;
    mesh.points[96 * nx + 95][1] += 5.0;
    let error = s.execute("edit.transform.removeWarpSplit", json!({})).unwrap_err();
    assert!(error.to_string().contains("cropped"), "{error}");
    assert_eq!(cache(&s, id).content_bounds(), Rect::new(20, 30, 120, 80), "the edit is undone");
}

fn deny_invert(id: &str, _: &serde_json::Value) -> crate::Result<()> {
    if id == "image.adjustments.invert" { Err(EngineError::Other("invert isn't allowed here".into())) } else { Ok(()) }
}

#[test]
fn warp_grid_reports_a_smart_filter_the_session_may_not_run() {
    let (mut s, id) = placed((100.0, 100.0));
    s.active_mut().unwrap().active_layer = Some(id);
    let doc = Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("a smart object") };
    sm.smart_filters.push(SmartFilter { command: "image.adjustments.invert".into(), params: json!({}), blend: BlendMode::Normal, opacity: 1.0, visible: true });
    s.authorize = Some(deny_invert);
    let error = s.execute("edit.transform.warpGrid", json!({"size": 3})).unwrap_err();
    assert!(error.to_string().contains("isn't allowed"), "{error}");
}
