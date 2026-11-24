//! FILE-215-3: commands on a smart object's filter mask (`"target":"filterMask"`), and the mask
//! saved with the document. Painting it with the brush: `brush_cmds/filter_mask_stroke_tests.rs`.

use super::*;

/// The filter mask a vertical black stroke of width 8 over x = 8 leaves, set as the paint command
/// would: a white mask is made first, then the column is blacked and the object re-rendered.
fn black_filter_mask_column(s: &mut Session) {
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("setup", |doc, _| {
        let mask = crate::smart_cmds::filter_mask_surface(doc, id)?;
        let black = photocraft_raster::from_rgba(&mask.format(), [0.0, 0.0, 0.0, 1.0]);
        let r = Rect::new(4, 2, 12, 46);
        let data: Vec<f32> = (0..(r.width() * r.height()) as usize).flat_map(|_| black.iter().copied()).collect();
        mask.write_region(r, &data);
        crate::smart_cmds::refresh_after_filter_mask_edit(doc, id)
    })
    .unwrap();
}

/// The filter mask is saved: .pcraft and PSD give it back.
#[test]
fn a_filter_mask_survives_pcraft_and_psd() {
    for depth in DEPTHS {
        let mut s = session(depth);
        paint(&mut s);
        convert(&mut s);
        s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
        black_filter_mask_column(&mut s);
        let mask = active_smart(&s).filter_mask.expect("a filter mask");
        let doc = s.active().unwrap().doc.clone();
        let id = s.active().unwrap().active_layer.unwrap();
        let back = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap()).unwrap();
        assert_eq!(smart(&back, id).unwrap().filter_mask, smart(&doc, id).unwrap().filter_mask, "depth {depth}: .pcraft");
        let psd = photocraft_io::export(&doc, "psd", &Default::default()).unwrap();
        let back = photocraft_io::import("x.psd", &psd.bytes).unwrap().document;
        let sm = back.walk().into_iter().find_map(|(_, _, l)| if let LayerContent::Smart(sm) = &l.content { Some(sm.clone()) } else { None }).unwrap();
        let (mut a, mut b) = (Vec::new(), Vec::new());
        sm.filter_mask.as_ref().unwrap().values_into(doc.bounds(), &mut a);
        mask.values_into(doc.bounds(), &mut b);
        assert_eq!(a, b, "depth {depth}: PSD");
    }
}

/// With its filter mask targeted, the commands that don't edit a filter mask act on the smart
/// object as without the target: Edit › Transform flips it, and Convert for Smart Filters stays
/// disabled (it would nest the smart object).
#[test]
fn the_filter_mask_target_leaves_the_smart_object_commands_alone() {
    let mut s = session(8);
    paint(&mut s);
    convert(&mut s);
    s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
    black_filter_mask_column(&mut s);
    let steps = s.active().unwrap().history.past_len();
    assert!(
        matches!(s.execute("filter.convertForSmartFilters", json!({"target": "filterMask"})), Err(EngineError::Disabled(..))),
        "the layer is already a smart object"
    );
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    let before = active_smart(&s).transform;
    s.execute("edit.transform.flipHorizontal", json!({"target": "filterMask"})).unwrap();
    assert_ne!(active_smart(&s).transform, before, "the smart object flips");
    // A shown filter mask doesn't send them the target either.
    s.execute("view.layerMask", json!({"mode": "gray", "mask": "filter"})).unwrap();
    assert_eq!(crate::channel_cmds::inject_target(&s, "edit.transform.flipHorizontal", json!({})), json!({}));
    assert_eq!(crate::channel_cmds::inject_target(&s, "filter.convertForSmartFilters", json!({})), json!({}));
    assert_eq!(crate::channel_cmds::inject_target(&s, "paint.stroke", json!({}))["target"], "filterMask");
    // Curves on the shown filter mask edits that gray plane, not the layer mask or the colours.
    let curve = crate::channel_cmds::isolated_curve_channel(s.active().unwrap()).expect("a gray target");
    assert_eq!(curve.gray, Some(crate::channel_cmds::GraySurface::FilterMask));
    let flipped = active_smart(&s).transform;
    s.execute("edit.transform.flipHorizontal", json!({})).unwrap();
    assert_ne!(active_smart(&s).transform, flipped, "the smart object flips again");
    // Moving the smart object doesn't re-render its filters as a filter-mask edit.
    let p = json!({"dx": 2, "target": "filterMask"});
    assert_eq!(crate::channel_cmds::filter_mask_layer(&s, "layer.translate", &p), None);
    assert!(crate::channel_cmds::filter_mask_layer(&s, "paint.stroke", &p).is_some());
    // Every other routed command edits it and re-renders the object: Equalize too, though it
    // takes no layer mask target by default.
    for cmd in ["image.adjustments.shadowsHighlights", "plugin.run", "filter.blur.average"] {
        assert!(crate::channel_cmds::filter_mask_layer(&s, cmd, &p).is_some(), "{cmd}");
    }
    let steps = s.active().unwrap().history.past_len();
    s.execute("image.adjustments.equalize", json!({"target": "filterMask"})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    assert_eq!(active_smart(&s).smart_filters.len(), 1, "no new smart filter: the filter mask was equalized");
}
