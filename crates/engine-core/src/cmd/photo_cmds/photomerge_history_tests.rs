//! A Photomerge result opens with the History and targeted layers Photoshop leaves (#14094): the
//! first snapshot plus one state, "Auto-Blend Layers" (Blend Images Together on, every source
//! layer targeted), "Move" (Blend off, only the bottom layer targeted) or "Fill" (the fill option).

use super::*;

/// One colour per 5x5 block of a 700x300 scene, hashed from the block, over a horizontal ramp.
fn scene_pixel(x: usize, y: usize) -> [f32; 4] {
    let h = (((x / 5) as u64 * 73_856_093) ^ ((y / 5) as u64 * 19_349_663)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let c = |shift: u32| ((h >> shift) & 0xff) as f32 / 255.0;
    [0.5 * c(8) + 0.5 * x as f32 / 700.0, c(24), c(40), 1.0]
}

/// A session with t1.tif, t2.tif and t3.tif open: 300x300 tiles of the scene cut at x 0, 200
/// and 400, so neighbours overlap by 100 px (the issue's measured set). t2.tif's colours are
/// scaled by `t2_exposure`, which Auto-Blend Layers' gain compensation then evens out.
fn exposed_tiles(t2_exposure: f32) -> Session {
    let mut s = crate::session();
    for (k, x0) in [0usize, 200, 400].into_iter().enumerate() {
        let mut d = Document::new(format!("t{}.tif", k + 1), Size::new(300, 300), ColorMode::Rgb, SampleType::U8);
        let fmt = d.pixel_format();
        let mut data = vec![0.0f32; 300 * 300 * fmt.channels()];
        for (i, o) in data.chunks_exact_mut(fmt.channels()).enumerate() {
            let [r, g, b, a] = scene_pixel(x0 + i % 300, i / 300);
            let e = if k == 1 { t2_exposure } else { 1.0 };
            photocraft_raster::from_rgba_into(&fmt, [r * e, g * e, b * e, a], o);
        }
        let mut surf = Surface::new(fmt);
        surf.write_region(Rect::new(0, 0, 300, 300), &data);
        d.layers.push(Layer::new("Background", LayerContent::Raster(surf)));
        s.add_document(d, None);
    }
    s
}

/// [`exposed_tiles`] with the three tiles exposed alike.
fn tiles() -> Session {
    exposed_tiles(1.0)
}

/// Merges the open tiles and returns the result's History rows (the first snapshot excluded).
fn merge(s: &mut Session, params: Value) -> Vec<String> {
    let mut p = json!({"useOpenDocuments": true});
    p.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
    let r = s.execute("file.automate.photomerge", p).unwrap();
    assert_eq!(r["placed"].as_array().map(Vec::len), Some(3), "{r}");
    assert_eq!(r["document"], json!(3), "the result is a new, fourth document");
    s.active().unwrap().history.entries()
}

/// The targeted layers' names, top of the Layers panel first.
fn targeted(s: &Session) -> Vec<String> {
    let st = s.active().unwrap();
    st.selected_layers().iter().rev().map(|id| st.doc.layer(*id).unwrap().name.clone()).collect()
}

/// The layer names from the top of the Layers panel down, with whether each has a mask.
fn stack(s: &Session) -> Vec<(String, bool)> {
    s.active().unwrap().doc.layers.iter().rev().map(|l| (l.name.clone(), l.mask.is_some())).collect()
}

/// `names` from the top down, each with (`masked`) or without a mask.
fn expect(names: &[&str], masked: bool) -> Vec<(String, bool)> {
    names.iter().map(|n| (n.to_string(), masked)).collect()
}

const SOURCES: [&str; 3] = ["t1.tif", "t2.tif", "t3.tif"];

#[test]
fn blend_on_is_one_auto_blend_layers_step_targeting_every_source() {
    for layout in ["reposition", "collage", "perspective"] {
        let mut s = tiles();
        assert_eq!(merge(&mut s, json!({"layout": layout})), ["Auto-Blend Layers"], "{layout}");
        assert_eq!(stack(&s), expect(&SOURCES, true), "{layout}");
        assert_eq!(targeted(&s), SOURCES, "{layout}: every source layer is targeted");
        let inspect = s.execute("document.inspect", json!({})).unwrap();
        assert_eq!(inspect["selectedLayers"].as_array().map(Vec::len), Some(3), "{layout}: {inspect}");
        let st = s.active().unwrap();
        assert_eq!(st.active_layer, st.doc.top_layer(), "{layout}: the top layer stays the key layer");
        // The first snapshot stays, named after the document.
        assert_eq!(st.snapshots.len(), 1, "{layout}");
        assert_eq!(st.snapshots[0].name, st.doc.name, "{layout}");
    }
}

#[test]
fn going_back_before_auto_blend_restores_the_aligned_layers_without_masks() {
    // t2.tif darker, so blending changes its pixels, not only its mask.
    let mut s = exposed_tiles(0.7);
    merge(&mut s, json!({"layout": "reposition"}));
    let blended: Document = (*s.active().unwrap().doc).clone();
    // The aligned layers before blending: what the same merge gives with Blend off.
    let mut unblended = exposed_tiles(0.7);
    merge(&mut unblended, json!({"layout": "reposition", "blend": false}));
    let aligned: Document = (*unblended.active().unwrap().doc).clone();
    s.execute("history.selectSnapshot", json!({"snapshot": 0})).unwrap();
    assert_eq!(stack(&s), expect(&SOURCES, false));
    let st = s.active().unwrap();
    assert_eq!(st.doc.size, aligned.size);
    let area = Rect::new(0, 0, aligned.size.width as i32, aligned.size.height as i32);
    let pixels = |l: &Layer| l.surface().unwrap().read_region(area);
    for ((now, before), after) in st.doc.layers.iter().zip(&aligned.layers).zip(&blended.layers) {
        assert_eq!(now.id, after.id, "{}: the snapshot keeps the result's layers", now.name);
        assert!(pixels(now) == pixels(before), "{}: the snapshot holds the aligned, unblended pixels", now.name);
    }
    assert!(pixels(&blended.layers[1]) != pixels(&aligned.layers[1]), "the blend changed t2.tif's pixels");
    // Selecting the snapshot is a state of its own; undoing it brings the masks back.
    assert_eq!(st.history.entries(), ["Auto-Blend Layers".to_string(), st.doc.name.clone()]);
    s.execute("history.undo", json!({})).unwrap();
    assert_eq!(stack(&s), expect(&SOURCES, true));
    assert_eq!(targeted(&s), SOURCES, "the Auto-Blend Layers state targets every source");
}

#[test]
fn blend_off_is_one_move_step_targeting_only_the_bottom_layer() {
    let mut s = tiles();
    assert_eq!(merge(&mut s, json!({"layout": "reposition", "blend": false})), ["Move"]);
    assert_eq!(stack(&s), expect(&SOURCES, false));
    assert_eq!(targeted(&s), ["t3.tif"], "only the last source, the bottom layer");
    let inspect = s.execute("document.inspect", json!({})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.active_layer, st.doc.layers.first().map(|l| l.id));
    assert_eq!(inspect["selectedLayers"], json!([st.doc.layers[0].id.0]), "{inspect}");
    assert_eq!(inspect["activeLayer"], json!(st.doc.layers[0].id.0), "{inspect}");
}

#[test]
fn fill_is_one_fill_step_without_auto_blend_layers() {
    let mut s = tiles();
    assert_eq!(merge(&mut s, json!({"layout": "reposition", "contentAwareFill": true})), ["Fill"]);
    // Not measured: the target after the fill keeps today's, the top (filled) layer.
    let st = s.active().unwrap();
    assert_eq!(st.selected_layers(), st.doc.top_layer().into_iter().collect::<Vec<_>>());
    assert_eq!(stack(&s).len(), 4);
    assert!(stack(&s)[1..].iter().all(|(_, masked)| *masked), "the sources keep their seam masks");
}
