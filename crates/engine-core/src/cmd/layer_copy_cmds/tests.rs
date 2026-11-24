use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::Session;

fn doc(s: &Session, i: usize) -> &Document {
    &s.documents()[i].doc
}

fn copied(r: &Value) -> LayerId {
    LayerId(r["layers"][0].as_u64().unwrap())
}

#[test]
fn copying_linked_layers_does_not_join_an_unrelated_destination_group() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let src_a = LayerId(s.execute("layer.new.layer", json!({"name": "Source A"})).unwrap()["layer"].as_u64().unwrap());
    let src_b = LayerId(s.execute("layer.new.layer", json!({"name": "Source B"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("link source", |d, _| {
        d.layer_mut(src_a).unwrap().link_group = Some(1);
        d.layer_mut(src_b).unwrap().link_group = Some(1);
        Ok(())
    })
    .unwrap();

    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let dst_a = LayerId(s.execute("layer.new.layer", json!({"name": "Destination A"})).unwrap()["layer"].as_u64().unwrap());
    let dst_b = LayerId(s.execute("layer.new.layer", json!({"name": "Destination B"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("link destination", |d, _| {
        d.layer_mut(dst_a).unwrap().link_group = Some(1);
        d.layer_mut(dst_b).unwrap().link_group = Some(1);
        Ok(())
    })
    .unwrap();
    s.set_active(0);
    let result = s.execute("layer.copyToDocument", json!({"document": 1, "layers": [src_a.0, src_b.0]})).unwrap();
    let ids: Vec<LayerId> = result["layers"].as_array().unwrap().iter().map(|id| LayerId(id.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 2);
    let d = doc(&s, 1);
    assert_eq!(d.layer(dst_a).unwrap().link_group, Some(1));
    assert_eq!(d.layer(dst_b).unwrap().link_group, Some(1));
    let first_group = d.layer(ids[0]).unwrap().link_group;
    assert_eq!(first_group, d.layer(ids[1]).unwrap().link_group, "the copied pair stays linked");
    assert!(first_group.is_some());
    assert_ne!(first_group, Some(1), "copied links must not merge with existing ones");

    assert!(s.undo());
    assert!(doc(&s, 1).layer(ids[0]).is_none());
    assert!(s.redo());
    assert_eq!(doc(&s, 1).layer(ids[0]).unwrap().link_group, first_group);
}

#[test]
fn copied_link_groups_are_distinct_from_each_other_and_destination() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let a = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.edit("link independent source groups", |d, _| {
        d.layer_mut(a).unwrap().link_group = Some(1);
        d.layer_mut(b).unwrap().link_group = Some(2);
        Ok(())
    })
    .unwrap();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.set_active(0);
    let copied = s.execute("layer.copyToDocument", json!({"document": 1, "layers": [a.0, b.0]})).unwrap();
    let ids: Vec<LayerId> = copied["layers"].as_array().unwrap().iter().map(|id| LayerId(id.as_u64().unwrap())).collect();
    let first = doc(&s, 1).layer(ids[0]).unwrap().link_group;
    let second = doc(&s, 1).layer(ids[1]).unwrap().link_group;
    assert!(first.is_some() && second.is_some());
    assert_ne!(first, second);
}

/// The type layer `id` of document `i`.
fn text(s: &Session, i: usize, id: LayerId) -> &photocraft_doc::TextLayer {
    match &doc(s, i).layer(id).unwrap().content {
        LayerContent::Text(t) => t,
        other => panic!("not type: {other:?}"),
    }
}

/// Bounds and summed alpha of a layer's rendered pixels.
fn ink(s: &Session, i: usize, id: LayerId) -> (Rect, f64) {
    let surf = doc(s, i).layer(id).unwrap().surface().unwrap();
    let b = surf.content_bounds();
    let a = surf.read_region(b).chunks(surf.channels()).map(|px| f64::from(*px.last().unwrap())).sum();
    (b, a)
}

/// Document 0 at `from` ppi with a 24 pt "H" (leading 30 pt, baseline shift 2 pt, first-line
/// indent 6 pt) and document 1 at `to` ppi; the type layer is selected in document 0.
fn type_docs(from: u32, to: u32) -> (Session, LayerId) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": from})).unwrap();
    let t = LayerId(
        s.execute("type.create", json!({"text": "H", "size": 24, "leading": 30, "baselineShift": 2, "x": 128, "y": 100})).unwrap()["layer"].as_u64().unwrap(),
    );
    s.execute("type.setStyle", json!({"layer": t.0, "firstLineIndent": 6})).unwrap();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": to})).unwrap();
    s.set_active(0);
    s.execute("layer.select", json!({"layer": t.0})).unwrap();
    (s, t)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4 * b.abs().max(1.0)
}

/// Checks a copy of `src` (document 0) as `dst` (document 1) at `pt` points (the reference
/// application's rewritten size for 24 pt), then that an edit keeps that size and the pixels.
fn check_rewritten(s: &mut Session, src: LayerId, dst: LayerId, pt: f32, label: &str) {
    let k = pt / 24.0;
    let (from, to) = (text(s, 0, src), text(s, 1, dst));
    assert_eq!(to.transform, from.transform, "{label}: no compensating transform");
    assert!(close(to.size_pt, pt), "{label}: {} pt", to.size_pt);
    let run = &to.runs[0].style;
    assert!(close(run.size_pt, pt), "{label}: run {} pt", run.size_pt);
    assert!(close(run.leading_pt.unwrap(), 30.0 * k), "{label}: leading {:?}", run.leading_pt);
    assert!(close(run.baseline_shift_pt, 2.0 * k), "{label}: baseline shift {}", run.baseline_shift_pt);
    assert_eq!(run.tracking, from.runs[0].style.tracking, "{label}: tracking is in 1/1000 em");
    assert!(close(to.paragraphs[0].style.first_line_indent_pt, 6.0 * k), "{label}: indent {}", to.paragraphs[0].style.first_line_indent_pt);
    let (b0, a0) = ink(s, 0, src);
    assert_eq!(ink(s, 1, dst).0, b0, "{label}: the copy's pixels");
    // Re-laid out at the destination's resolution, the copy renders as the source does.
    s.execute("type.edit", json!({"layer": dst.0, "text": "M"})).unwrap();
    s.execute("type.edit", json!({"layer": dst.0, "text": "H"})).unwrap();
    assert!(close(text(s, 1, dst).size_pt, pt), "{label}: an edit keeps {} pt", text(s, 1, dst).size_pt);
    let (b1, a1) = ink(s, 1, dst);
    assert_eq!(b1, b0, "{label}: after an edit, {b1:?} vs {b0:?}");
    assert!((a1 - a0).abs() <= a0 * 0.005, "{label}: coverage {a1} vs {a0}");
}

/// #9560: a type layer copied between documents of different resolutions keeps its pixels and
/// takes the destination's point size, as in the reference application (measured: 24 pt from
/// 72 ppi is 5.76 pt at 300 ppi, from 300 ppi 100 pt at 72 ppi, with no transform; later edits
/// keep that size). It used to keep 24 pt and re-lay out about 4.2 times larger or smaller on
/// its next edit, and then kept 24 pt behind a compensating transform.
#[test]
fn copied_type_takes_the_destinations_point_size_and_keeps_its_pixels() {
    for (from, to, pt) in [(72, 300, 5.76), (300, 72, 100.0)] {
        let (mut s, t) = type_docs(from, to);
        let id = copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap());
        check_rewritten(&mut s, t, id, pt, &format!("copy {from}→{to}"));
    }
}

/// The same through Copy and Paste in Place of the whole type layer.
#[test]
fn pasted_type_takes_the_destinations_point_size_and_keeps_its_pixels() {
    for (from, to, pt) in [(72, 300, 5.76), (300, 72, 100.0)] {
        let (mut s, t) = type_docs(from, to);
        s.execute("edit.copy", json!({})).unwrap();
        s.set_active(1);
        let id = LayerId(s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap()["layer"].as_u64().unwrap());
        check_rewritten(&mut s, t, id, pt, &format!("paste {from}→{to}"));
    }
}

/// Paragraph text keeps its box (in pixels) and its wrapping: only the point sizes change.
#[test]
fn copied_paragraph_type_keeps_its_box() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 72})).unwrap();
    let t =
        LayerId(s.execute("type.create", json!({"text": "Hello wrapping world", "size": 24, "box": [20, 20, 120, 150]})).unwrap()["layer"].as_u64().unwrap());
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 300})).unwrap();
    s.set_active(0);
    s.execute("layer.select", json!({"layer": t.0})).unwrap();
    let id = copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap());
    assert_eq!(text(&s, 1, id).shape, text(&s, 0, t).shape);
    assert!(close(text(&s, 1, id).size_pt, 5.76));
    let (b0, _) = ink(&s, 0, t);
    s.execute("type.edit", json!({"layer": id.0, "text": "Hello wrapping world"})).unwrap();
    assert_eq!(ink(&s, 1, id).0, b0, "the same lines after a re-layout");
}

/// Equal resolutions change nothing; one that isn't a positive number lays out at 72 ppi, so it
/// is treated as 72 rather than producing an infinite or zero size.
#[test]
fn retargeting_type_survives_bad_resolutions() {
    let mut d = Document::new("", photocraft_geom::Size::new(10, 10), ColorMode::Rgb, SampleType::U8);
    d.layers.push(photocraft_doc::Layer::new("t", LayerContent::Text(photocraft_doc::TextLayer { text: "H".into(), size_pt: 24.0, ..Default::default() })));
    let size = |d: &Document| match &d.layers[0].content {
        LayerContent::Text(t) => t.size_pt,
        _ => panic!("not type"),
    };
    d.resolution_dpi = 72.0;
    crate::canvas_geom::retarget_type_resolution(&mut d, 72.0);
    assert_eq!(size(&d), 24.0);
    for bad in [0.0, -5.0, f32::NAN, f32::INFINITY] {
        crate::canvas_geom::retarget_type_resolution(&mut d, bad);
        assert_eq!(size(&d), 24.0, "{bad}");
        d.resolution_dpi = bad;
        crate::canvas_geom::retarget_type_resolution(&mut d, 72.0);
        assert_eq!(size(&d), 24.0, "from {bad}");
    }
    // Absurd ratios stay finite and positive.
    d.resolution_dpi = f32::MAX;
    crate::canvas_geom::retarget_type_resolution(&mut d, f32::MIN_POSITIVE);
    assert!(size(&d).is_finite() && size(&d) > 0.0, "{}", size(&d));
    crate::canvas_geom::retarget_type_resolution(&mut d, f32::MAX);
    assert!(size(&d).is_finite() && size(&d) > 0.0, "{}", size(&d));
}
