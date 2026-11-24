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

/// Document 0 at 72 ppi with "Hello" in Paragraph Style 1 (24 pt) and "He" also in Character
/// Style 1 (24 pt, underlined); document 1 at 300 ppi, its own styles made by `dest`.
fn styled_type_docs(dest: impl FnOnce(&mut Session)) -> (Session, LayerId) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 72})).unwrap();
    let t = s.execute("type.create", json!({"text": "Hello", "size": 12, "x": 100, "y": 100})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.paragraphStyle.new", json!({"attrs": {"size": 24}, "fromSelection": false, "apply": true, "layer": t})).unwrap();
    s.execute("type.characterStyle.new", json!({"attrs": {"size": 24, "underline": true}, "fromSelection": false, "apply": true, "layer": t, "range": [0, 2]}))
        .unwrap();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 300})).unwrap();
    dest(&mut s);
    s.set_active(0);
    s.execute("layer.select", json!({"layer": t})).unwrap();
    (s, LayerId(t))
}

/// The styles panel state of `range` in layer `id` of document 1.
fn styles_of(s: &mut Session, id: LayerId, range: [usize; 2]) -> (Value, Value) {
    s.set_active(1);
    let c = s.execute("type.characterStyle.list", json!({"layer": id.0, "range": range})).unwrap();
    let p = s.execute("type.paragraphStyle.list", json!({"layer": id.0, "range": range})).unwrap();
    (c, p)
}

/// #10500: type copied to a document of another resolution brings the styles it uses, as in
/// the reference application (measured with Duplicate Layer from 72 to 300 ppi): a style the
/// destination lacks arrives under its name, scaled like the text (24 pt → 5.76 pt), so the
/// copy has no override; it never takes over an unrelated destination style with the same id.
#[test]
fn copied_type_brings_its_styles_scaled() {
    for paste in [false, true] {
        let (mut s, _) = styled_type_docs(|s| {
            s.execute("type.characterStyle.new", json!({"name": "Unrelated", "attrs": {"size": 50}, "fromSelection": false})).unwrap();
            s.execute("type.paragraphStyle.new", json!({"name": "Unrelated", "attrs": {"size": 50}, "fromSelection": false})).unwrap();
        });
        let id = if paste {
            s.execute("edit.copy", json!({})).unwrap();
            s.set_active(1);
            LayerId(s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap()["layer"].as_u64().unwrap())
        } else {
            copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap())
        };
        assert!(close(text(&s, 1, id).runs[0].style.size_pt, 5.76), "paste {paste}");
        let (c, p) = styles_of(&mut s, id, [0, 2]);
        let names = |v: &Value| v["styles"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names(&c), ["None", "Unrelated", "Character Style 1"], "paste {paste}");
        assert_eq!(names(&p), ["Basic Paragraph", "Unrelated", "Paragraph Style 1"], "paste {paste}");
        let (cs, ps) = (&c["styles"][2], &p["styles"][2]);
        assert!(close(cs["attrs"]["size_pt"].as_f64().unwrap() as f32, 5.76), "{cs}");
        assert!(close(ps["characterAttrs"]["size_pt"].as_f64().unwrap() as f32, 5.76), "{ps}");
        assert_eq!((&c["current"]["character"], &c["current"]["characterOverride"]), (&cs["id"], &json!(false)), "paste {paste}");
        assert_eq!((&p["current"]["paragraph"], &p["current"]["paragraphOverride"]), (&ps["id"], &json!(false)), "paste {paste}");
        // The source keeps its 24 pt styles.
        assert_eq!(doc(&s, 0).text_styles.character[0].attrs["size_pt"], json!(24.0));
    }
}

/// A destination that already has a style of the same name keeps its definition (24 pt), and
/// the copy, at 5.76 pt, shows a local override ("+"), as in the reference application.
#[test]
fn copied_type_uses_the_destinations_same_named_style() {
    let (mut s, _) = styled_type_docs(|s| {
        s.execute("type.characterStyle.new", json!({"name": "Filler", "fromSelection": false})).unwrap();
        s.execute("type.characterStyle.new", json!({"name": "Character Style 1", "attrs": {"size": 24, "underline": true}, "fromSelection": false})).unwrap();
    });
    let id = copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap());
    let (c, _) = styles_of(&mut s, id, [0, 2]);
    assert_eq!(c["styles"].as_array().unwrap().len(), 3, "no second Character Style 1: {c}");
    let existing = &c["styles"][2];
    assert_eq!(existing["attrs"]["size_pt"], json!(24.0));
    assert_eq!((&c["current"]["character"], &c["current"]["characterOverride"]), (&existing["id"], &json!(true)));
    assert!(close(text(&s, 1, id).runs[0].style.size_pt, 5.76));
}

/// Duplicate Layer › New Document brings the styles the type uses (at the same size).
#[test]
fn type_duplicated_to_a_new_document_keeps_its_styles() {
    let (mut s, t) = styled_type_docs(|_| {});
    let r = s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    let (i, id) = (r["document"].as_u64().unwrap() as usize, LayerId(r["layer"].as_u64().unwrap()));
    s.set_active(i);
    let c = s.execute("type.characterStyle.list", json!({"layer": id.0, "range": [0, 2]})).unwrap();
    let p = s.execute("type.paragraphStyle.list", json!({"layer": id.0, "range": [0, 2]})).unwrap();
    assert_eq!(c["styles"][1]["name"], "Character Style 1");
    assert_eq!(c["styles"][1]["attrs"]["size_pt"], json!(24.0));
    assert_eq!((&c["current"]["character"], &c["current"]["characterOverride"]), (&c["styles"][1]["id"], &json!(false)));
    assert_eq!((&p["current"]["paragraph"], &p["current"]["paragraphOverride"]), (&p["styles"][1]["id"], &json!(false)));
    assert_eq!(text(&s, i, id).runs, text(&s, 0, t).runs);
}

/// Document 0 (RGB, 72 ppi) with "Hello" whose "He" is in Character Style 1 (red); document 1
/// (CMYK, 72 ppi). The type layer is selected in document 0.
fn red_styled_type_docs() -> (Session, LayerId) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 200, "height": 100, "resolution": 72})).unwrap();
    let t = s.execute("type.create", json!({"text": "Hello", "size": 12, "x": 20, "y": 50})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.characterStyle.new", json!({"attrs": {"color": "#ff0000"}, "fromSelection": false, "apply": true, "layer": t, "range": [0, 2]})).unwrap();
    s.execute("file.new", json!({"width": 200, "height": 100, "resolution": 72, "mode": "cmyk"})).unwrap();
    s.set_active(0);
    s.execute("layer.select", json!({"layer": t})).unwrap();
    (s, LayerId(t))
}

/// The colour Character Style 1 of document `i` sets, and whether "He" of `id` overrides it.
fn style_color(s: &mut Session, i: usize, id: LayerId) -> (photocraft_color::Color, bool) {
    s.set_active(i);
    let c = s.execute("type.characterStyle.list", json!({"layer": id.0, "range": [0, 2]})).unwrap();
    let style = c["styles"].as_array().unwrap().iter().find(|x| x["name"] == "Character Style 1").unwrap();
    (serde_json::from_value(style["attrs"]["color"].clone()).unwrap(), c["current"]["characterOverride"].as_bool().unwrap())
}

/// Type copied to a document of another colour mode converts its run colours, and the style it
/// brings converts with the same transform: the run matches its style (no override), so a later
/// change to the style's colour still reaches it. Converting the document itself (Image › Mode,
/// Convert to Profile with and without dither) keeps its styles in step the same way.
#[test]
fn copied_type_styles_convert_their_colours_with_the_type() {
    let (mut s, _) = red_styled_type_docs();
    let id = copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap());
    let run = text(&s, 1, id).runs[0].style.color;
    assert_eq!(run.mode, ColorMode::Cmyk, "the run converted");
    let (style, over) = style_color(&mut s, 1, id);
    assert_eq!(style, run, "the copied style converted with the run");
    assert!(!over, "no false override");
    for (cmd, p) in [
        ("image.mode.cmyk", json!({})),
        ("edit.convertToProfile", json!({"profile": "coated-cmyk", "dither": false})),
        ("edit.convertToProfile", json!({"profile": "coated-cmyk", "dither": true})),
        ("edit.convertToProfile", json!({"profile": "lab-d50"})),
    ] {
        let (mut s, t) = red_styled_type_docs();
        s.execute(cmd, p.clone()).unwrap();
        let run = text(&s, 0, t).runs[0].style.color;
        assert_ne!(run.mode, ColorMode::Rgb, "{cmd} {p}: the run converted");
        let (style, over) = style_color(&mut s, 0, t);
        assert_eq!(style, run, "{cmd} {p}: the style converted with the run");
        assert!(!over, "{cmd} {p}: no false override");
    }
}

/// Duplicate Layer › New Document keeps the source's Basic Paragraph: type that inherits its
/// size from it has no override there, and Clear Override leaves it as it was.
#[test]
fn type_duplicated_to_a_new_document_keeps_the_sources_basic_paragraph() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let t = LayerId(s.execute("type.create", json!({"text": "Hello", "size": 12, "x": 20, "y": 50})).unwrap()["layer"].as_u64().unwrap());
    s.execute("type.paragraphStyle.set", json!({"id": 0, "attrs": {"size": 30}})).unwrap();
    assert!(close(text(&s, 0, t).runs[0].style.size_pt, 30.0), "the type inherits Basic Paragraph");
    let basic = doc(&s, 0).text_styles.basic.clone();
    s.execute("layer.select", json!({"layer": t.0})).unwrap();
    let r = s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    let (i, id) = (r["document"].as_u64().unwrap() as usize, LayerId(r["layer"].as_u64().unwrap()));
    assert_eq!(doc(&s, i).text_styles.basic, basic);
    s.set_active(i);
    let c = s.execute("type.characterStyle.list", json!({"layer": id.0})).unwrap();
    let p = s.execute("type.paragraphStyle.list", json!({"layer": id.0})).unwrap();
    assert_eq!((&c["current"]["characterOverride"], &p["current"]["paragraphOverride"]), (&json!(false), &json!(false)), "{c} {p}");
    s.execute("type.paragraphStyle.clearOverride", json!({"layer": id.0})).unwrap();
    s.execute("type.characterStyle.clearOverride", json!({"layer": id.0})).unwrap();
    assert!(close(text(&s, i, id).runs[0].style.size_pt, 30.0), "{}", text(&s, i, id).runs[0].style.size_pt);
}
