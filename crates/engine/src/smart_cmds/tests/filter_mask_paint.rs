//! FILE-215-3: painting a smart object's filter mask (`"target":"filterMask"`) hides the smart
//! filters where it is black, in one undo step, and the painted mask is saved.

use super::*;

/// A vertical black stroke over x = 8 into the filter mask, as the Brush tool sends it.
fn stroke(s: &mut Session, target: Option<&str>) -> Result<Value> {
    let mut p = json!({"points": [[8, 2], [8, 46]], "size": 8, "hardness": 1, "color": "#000000"});
    if let Some(t) = target {
        p["target"] = json!(t);
    }
    s.execute("paint.stroke", p)
}

#[test]
fn painting_the_filter_mask_hides_the_filters_there_in_one_undo_step() {
    for depth in DEPTHS {
        let mut s = session(depth);
        paint(&mut s);
        let original = flat(&s);
        convert(&mut s);
        // Without smart filters there is no filter mask to paint.
        assert!(matches!(stroke(&mut s, Some("filterMask")), Err(EngineError::Disabled(..))), "depth {depth}");
        s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
        let blurred = flat(&s);
        assert!(active_smart(&s).filter_mask.is_none());
        let steps = s.active().unwrap().history.past_len();

        stroke(&mut s, Some("filterMask")).unwrap();
        let mask = active_smart(&s).filter_mask.expect("painting starts a white filter mask");
        assert_eq!((mask.value(8, 20), mask.value(30, 20)), (0.0, 1.0), "depth {depth}");
        let px = flat(&s);
        let at = |v: &Vec<[f32; 4]>, x: i32, y: i32| v[(y * W + x) as usize];
        assert!(max_diff(&[at(&px, 8, 20)], &[at(&original, 8, 20)]) < 1e-6, "depth {depth}: black hides the blur");
        assert_eq!(at(&px, 30, 20), at(&blurred, 30, 20), "depth {depth}: white keeps it");
        assert!(active_smart(&s).smart_filters.len() == 1, "the stroke is no smart filter");

        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one step");
        s.execute("edit.undo", json!({})).unwrap();
        assert!(active_smart(&s).filter_mask.is_none());
        assert_eq!(flat(&s), blurred, "depth {depth}: undo brings the blur back");
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(flat(&s), px);

        // The painted mask is saved: .pcraft and PSD give it back.
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

/// ⌥-click on the filter mask thumbnail shows it; painting then goes into it without a target.
#[test]
fn a_shown_filter_mask_takes_the_paint() {
    let mut s = session(8);
    paint(&mut s);
    convert(&mut s);
    s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
    assert!(s.execute("view.layerMask", json!({"mode": "gray", "mask": "filter"})).is_err(), "no filter mask yet");
    stroke(&mut s, Some("filterMask")).unwrap();
    let v = s.execute("view.layerMask", json!({"mode": "gray", "mask": "filter"})).unwrap();
    assert_eq!(v["mode"], "gray");
    assert!(s.active().unwrap().channel_view.shown_layer_mask(&s.active().unwrap().doc).is_some());
    s.execute("paint.stroke", json!({"points": [[30, 2], [30, 46]], "size": 8, "hardness": 1, "color": "#000000"})).unwrap();
    let mask = active_smart(&s).filter_mask.unwrap();
    assert_eq!(mask.value(30, 20), 0.0, "the stroke went into the filter mask");
    let d = s.active().unwrap();
    assert!(matches!(d.doc.layer(d.active_layer.unwrap()).unwrap().content, LayerContent::Smart(_)), "still a smart object");
    // Bad params are refused, not a panic.
    assert!(s.execute("view.layerMask", json!({"mask": 3})).is_err());
}
