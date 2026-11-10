use super::*;
use photocraft_doc::text::TextShape;
use photocraft_doc::vector::{Path, Subpath};
use photocraft_doc::{LayerContent, LayerId, TextLayer};
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": 8, "background": "white"})).unwrap();
    s
}

fn session_with_box_text(words: &str, size: f32, b: [f32; 4]) -> (Session, LayerId) {
    let mut s = session();
    let r = s.execute("type.create", json!({"x": 0, "y": 0, "text": words, "size": size, "color": "#000000", "box": b})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert!(matches!(text(&s, id).shape, TextShape::Box { .. }));
    (s, id)
}

fn session_with_point_text(words: &str, size: f32) -> (Session, LayerId) {
    let mut s = session();
    let r = s.execute("type.create", json!({"x": 100, "y": 100, "text": words, "size": size, "color": "#000000"})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(text(&s, id).shape, TextShape::Point);
    (s, id)
}

fn text(s: &Session, id: LayerId) -> TextLayer {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.clone(),
        c => panic!("not text: {}", c.kind_name()),
    }
}

/// Largest and smallest effective type size of a solved Dynamic Text box (base size × line
/// scale). Box text keeps its base size; each solved line carries its own scale.
fn effective(s: &Session, id: LayerId) -> (f32, f32) {
    let t = text(s, id);
    let lines = t.fit.as_ref().map(|f| f.lines.clone()).unwrap_or_default();
    assert!(!lines.is_empty(), "solved lines: {:?}", t.fit);
    lines.iter().map(|l| t.size_pt * l.scale).fold((f32::MIN, f32::MAX), |(hi, lo), v| (hi.max(v), lo.min(v)))
}

fn history_len(s: &Session) -> usize {
    s.active().unwrap().history.entries().len()
}

fn lock_all(s: &mut Session, id: LayerId) {
    s.execute("layer.setProps", json!({"layer": id.0, "locked": true})).unwrap();
}

#[test]
fn convert_fits_box_text_in_one_step() {
    let (mut s, id) = session_with_box_text("Hello dynamic world", 12.0, [0.0, 0.0, 300.0, 100.0]);
    assert!(s.is_enabled("type.convertToDynamicText"));
    assert!(!s.is_enabled("type.releaseDynamicText") && !s.is_enabled("type.setDynamicText"));
    let before = history_len(&s);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    assert_eq!(history_len(&s), before + 1);
    let t = text(&s, id);
    assert!(t.fit.is_some() && t.size_pt == 12.0, "base size kept: {}", t.size_pt);
    assert!(effective(&s, id).1 > 12.0, "fills the big box: {:?}", effective(&s, id));
    assert!(!s.is_enabled("type.convertToDynamicText"), "already Dynamic Text");
    assert!(s.is_enabled("type.releaseDynamicText") && s.is_enabled("type.setDynamicText"));
    // Converting twice is refused without a history step.
    assert!(s.execute("type.convertToDynamicText", json!({"layer": id.0})).is_err());
    assert_eq!(history_len(&s), before + 1);
}

#[test]
fn point_text_becomes_box_then_fits() {
    let (mut s, id) = session_with_point_text("Hi", 12.0);
    let before = history_len(&s);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    assert_eq!(history_len(&s), before + 1, "box and fit in one step");
    assert!(matches!(text(&s, id).shape, TextShape::Box { .. }) && text(&s, id).fit.is_some());
    assert!(s.undo());
    assert_eq!(text(&s, id).shape, TextShape::Point);
    assert!(text(&s, id).fit.is_none());
}

/// Box text at 12 pt converted into area text in a 300×200 rectangle path.
fn session_with_area_text(words: &str) -> (Session, LayerId) {
    let (mut s, id) = session_with_point_text(words, 12.0);
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polygon(&[(50.0, 50.0), (350.0, 50.0), (350.0, 250.0), (50.0, 250.0)])]));
        Ok(())
    })
    .unwrap();
    s.execute("type.fillPath", json!({"layer": id.0})).unwrap();
    (s, id)
}

#[test]
fn area_text_fits() {
    let (mut s, id) = session_with_area_text("Well, that went better than expected.");
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let t = text(&s, id);
    assert!(matches!(t.shape, TextShape::Area { .. }) && t.fit.is_some());
    // Per-line sizes: the base 12 pt is kept and the solved lines fill the area.
    assert!(t.size_pt == 12.0 && run_sizes(&t) == [12.0], "{} {:?}", t.size_pt, run_sizes(&t));
    assert!(effective(&s, id).0 > 12.0, "fills the area: {:?}", effective(&s, id));
    // Typing refits from the base sizes; toggling off gives back the own 12 pt.
    s.execute("type.edit", json!({"layer": id.0, "text": "Well, that went better than expected. Much better."})).unwrap();
    assert_eq!(text(&s, id).size_pt, 12.0);
    s.execute("type.releaseDynamicText", json!({"layer": id.0, "keepLook": false})).unwrap();
    let t = text(&s, id);
    assert!(t.fit.is_none() && matches!(t.shape, TextShape::Area { .. }));
    assert!(t.char_runs().iter().all(|r| (r.style.size_pt - 12.0).abs() < 0.01) && (t.size_pt - 12.0).abs() < 0.01, "{} {:?}", t.size_pt, run_sizes(&t));
}

#[test]
fn area_toggle_off_restores_exact_sizes() {
    let (mut s, id) = session_with_area_text("Well, that went better than expected. Much better.");
    let before = text(&s, id);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    assert!(run_sizes(&text(&s, id)) == [12.0] && text(&s, id).size_pt == 12.0, "base sizes stay while fitted");
    assert!(!text(&s, id).fit.as_ref().unwrap().lines.is_empty());
    s.execute("type.releaseDynamicText", json!({"layer": id.0, "keepLook": false})).unwrap();
    let t = text(&s, id);
    assert!(t.fit.is_none());
    assert_eq!((t.text.as_str(), run_sizes(&t), t.size_pt, &t.shape), (before.text.as_str(), run_sizes(&before), before.size_pt, &before.shape));
}

#[test]
fn legacy_area_scale_file_reverts() {
    // A fitted Area layer as saved by the uniform-fit build: runs at 24 pt, fit.area_scale 2.0,
    // no lines. One refresh solves lines; keepLook:false gives back 12 pt.
    let (mut s, id) = session_with_area_text("Well, that went better than expected.");
    s.edit("legacy", |doc, _| {
        if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
            photocraft_text::fit::apply_scale(t, 2.0);
            t.fit = Some(TextFit { area_scale: 2.0, ..TextFit::default() });
        }
        Ok(())
    })
    .unwrap();
    s.execute("type.edit", json!({"layer": id.0, "text": "Well, that went better than expected."})).unwrap();
    let t = text(&s, id);
    assert!(!t.fit.as_ref().unwrap().lines.is_empty() && t.size_pt == 24.0 && t.fit.as_ref().unwrap().area_scale == 2.0, "{:?}", t.fit);
    s.execute("type.releaseDynamicText", json!({"layer": id.0, "keepLook": false})).unwrap();
    let t = text(&s, id);
    assert!(t.fit.is_none() && (t.size_pt - 12.0).abs() < 1e-3 && run_sizes(&t).iter().all(|v| (v - 12.0).abs() < 1e-3), "{} {:?}", t.size_pt, run_sizes(&t));
}

#[test]
fn area_release_keeps_the_look() {
    let (mut s, id) = session_with_area_text("Hey! Did you see what Dynamic Text does to a speech bubble?");
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let fitted = glyphs(&s, id);
    let before = history_len(&s);
    s.execute("type.releaseDynamicText", json!({"layer": id.0})).unwrap();
    assert_eq!(history_len(&s), before + 1);
    let t = text(&s, id);
    assert!(t.fit.is_none() && matches!(t.shape, TextShape::Area { .. }));
    assert!(run_sizes(&t).len() >= 2 && !run_sizes(&t).contains(&12.0), "explicit per-line sizes: {:?}", run_sizes(&t));
    let baked = glyphs(&s, id);
    let mut from = 0;
    for (i, g) in baked.iter().enumerate() {
        let hit = fitted[from..].iter().position(|f| f.0 == g.0 && (f.1 - g.1).abs() <= 0.5 && (f.2 - g.2).abs() <= 0.5);
        let Some(off) = hit else { panic!("glyph {i} {g:?} moved") };
        from += off + 1;
    }
}

#[test]
fn typing_refits_and_stays_bounded() {
    let (mut s, id) = session_with_box_text("Hi", 12.0, [0.0, 0.0, 200.0, 80.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let big = effective(&s, id).0;
    let before = history_len(&s);
    s.execute("type.edit", json!({"layer": id.0, "text": "Hi ".repeat(400)})).unwrap();
    assert_eq!(history_len(&s), before + 1, "one bounded refit per edit");
    let (small, smallest) = effective(&s, id);
    assert!(small < big, "{big} -> {small}");
    // Width-only sizing: lines don't shrink below about half the base size to fit the height;
    // the text past the box bottom is hidden instead.
    let t = text(&s, id);
    let lines = t.fit.as_ref().unwrap().lines.clone();
    assert!(lines.iter().all(|l| l.scale >= photocraft_text::fit::MIN_PT / 12.0) && smallest >= photocraft_text::fit::MIN_PT - 1e-4, "{lines:?}");
    assert!(lines.len() < 336, "not one word per line: {} lines", lines.len());
    let dpi = s.active().unwrap().doc.resolution_dpi;
    let out = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner()).layout(&t, dpi);
    assert!(out.overflowed && out.hidden, "the rest is hidden past the box bottom");
}

#[test]
fn resize_then_undo_restores_box_and_lines() {
    let (mut s, id) = session_with_box_text("Hello there", 12.0, [0.0, 0.0, 300.0, 100.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let (shape0, fit0, size0) = (text(&s, id).shape.clone(), text(&s, id).fit, effective(&s, id).0);
    let before = history_len(&s);
    s.execute("type.edit", json!({"layer": id.0, "box": [0.0, 0.0, 120.0, 100.0]})).unwrap();
    assert_eq!(history_len(&s), before + 1);
    assert!(effective(&s, id).0 < size0);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(text(&s, id).shape, shape0);
    assert_eq!(text(&s, id).fit, fit0, "box and solved lines come back in one step");
}

/// Glyph pen positions (px) of a layer, as the shared text engine lays it out at the document's dpi.
fn glyphs(s: &Session, id: LayerId) -> Vec<(u32, f32, f32)> {
    let dpi = s.active().unwrap().doc.resolution_dpi;
    let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    eng.layout(&text(s, id), dpi).glyphs.iter().map(|g| (g.id, g.x, g.y)).collect()
}

fn run_sizes(t: &TextLayer) -> Vec<f32> {
    let mut v: Vec<f32> = t.char_runs().iter().map(|r| r.style.size_pt).collect();
    v.dedup();
    v
}

#[test]
fn release_keeps_sizes() {
    let (mut s, id) = session_with_box_text("Hello dynamic world of text", 12.0, [0.0, 0.0, 300.0, 150.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let (hi, lo) = effective(&s, id);
    s.execute("type.releaseDynamicText", json!({"layer": id.0})).unwrap();
    let t = text(&s, id);
    assert!(t.fit.is_none());
    // The fitted look is now ordinary runs: the base 12 pt is gone, each line has its own size.
    let sizes = run_sizes(&t);
    assert!(sizes.len() >= 2 && sizes.iter().all(|&v| v >= lo - 0.05 && v <= hi + 0.05), "{sizes:?} vs {lo}..{hi}");
    assert!(t.text.contains('\u{3}'), "{:?}", t.text);
    let before = history_len(&s);
    assert!(s.execute("type.releaseDynamicText", json!({"layer": id.0})).unwrap_err().to_string().contains("not Dynamic Text"));
    assert!(s.execute("type.setDynamicText", json!({"layer": id.0, "lineGap": 10})).unwrap_err().to_string().contains("not Dynamic Text"));
    assert_eq!(history_len(&s), before);
    // Released text no longer refits: a smaller box keeps the sizes.
    s.execute("type.edit", json!({"layer": id.0, "box": [0.0, 0.0, 60.0, 150.0]})).unwrap();
    assert_eq!(run_sizes(&text(&s, id)), sizes);
}

#[test]
fn release_without_keep_look_restores_the_text() {
    let (mut s, id) = session_with_box_text("Hello dynamic world of text", 12.0, [0.0, 0.0, 300.0, 150.0]);
    let before = text(&s, id);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    s.execute("type.setDynamicText", json!({"layer": id.0, "wordGap": 40})).unwrap();
    s.execute("type.releaseDynamicText", json!({"layer": id.0, "keepLook": false})).unwrap();
    let t = text(&s, id);
    assert!(t.fit.is_none());
    assert_eq!((t.text.as_str(), run_sizes(&t), t.size_pt, &t.shape), (before.text.as_str(), run_sizes(&before), before.size_pt, &before.shape));
    // One undo step brings the fitted look back.
    s.execute("edit.undo", json!({})).unwrap();
    assert!(text(&s, id).fit.is_some());
    let n = history_len(&s);
    assert!(s.execute("type.releaseDynamicText", json!({"layer": id.0, "keepLook": 1})).unwrap_err().to_string().contains("keepLook"));
    assert_eq!(history_len(&s), n);
}

#[test]
fn release_bakes_without_moving_glyphs() {
    let (mut s, id) = session_with_box_text("Hello dynamic world of text", 12.0, [0.0, 0.0, 300.0, 150.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    s.execute("type.setDynamicText", json!({"layer": id.0, "wordGap": 40, "lineGap": 15})).unwrap();
    let fitted = glyphs(&s, id);
    assert!(fitted.len() > 20);
    let before = history_len(&s);
    s.execute("type.releaseDynamicText", json!({"layer": id.0})).unwrap();
    assert_eq!(history_len(&s), before + 1);
    assert!(text(&s, id).fit.is_none());
    let baked = glyphs(&s, id);
    assert!(!baked.is_empty());
    let lines = text(&s, id).text.matches('\u{3}').count() + 1;
    assert!(fitted.len() - baked.len() <= 2 * lines, "{} fitted vs {} baked glyphs, {lines} lines", fitted.len(), baked.len());
    // The fitted layout shapes each line alone and keeps an invisible mark glyph per line.
    let mut from = 0;
    for (i, g) in baked.iter().enumerate() {
        let hit = fitted[from..].iter().position(|f| f.0 == g.0 && (f.1 - g.1).abs() <= 0.5 && (f.2 - g.2).abs() <= 0.5);
        let Some(off) = hit else { panic!("glyph {i} {g:?} moved") };
        from += off + 1;
    }
    // Undo brings the fit back.
    s.execute("edit.undo", json!({})).unwrap();
    assert!(text(&s, id).fit.is_some());
}

#[test]
fn typing_recomputes_from_base() {
    let (mut s, id) = session_with_box_text("Hello dynamic world", 12.0, [0.0, 0.0, 300.0, 150.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let base = text(&s, id);
    s.execute("type.edit", json!({"layer": id.0, "text": "Hello dynamic world and a good deal more words"})).unwrap();
    assert_eq!(run_sizes(&text(&s, id)), vec![12.0], "base sizes untouched while typing");
    s.execute("type.edit", json!({"layer": id.0, "text": "Hello dynamic world"})).unwrap();
    let again = text(&s, id);
    assert_eq!(again.runs, base.runs);
    assert_eq!(again.size_pt, base.size_pt);
    assert_eq!(again.fit, base.fit, "same text, same lines");
}

#[test]
fn refresh_fills_lines() {
    let (mut s, id) = session_with_box_text("Hello dynamic world", 12.0, [0.0, 0.0, 300.0, 150.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    // A file saved without solved lines: loaded with the list empty.
    s.edit("empty lines", |doc, _| {
        if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content)
            && let Some(f) = t.fit.as_mut()
        {
            f.lines.clear();
        }
        Ok(())
    })
    .unwrap();
    assert!(text(&s, id).fit.unwrap().lines.is_empty());
    s.execute("type.edit", json!({"layer": id.0, "text": "Hello dynamic world"})).unwrap();
    assert!(!text(&s, id).fit.unwrap().lines.is_empty());
}

#[test]
fn gaps_validate_and_apply() {
    let (mut s, id) = session_with_box_text("a b c d e f", 12.0, [0.0, 0.0, 300.0, 300.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let before = history_len(&s);
    for bad in [
        json!({"wordGap": "x"}),
        json!({"lineGap": 1000}),
        json!({"wordGap": -101}),
        json!({"lineGap": -51}),
        json!({"lineGap": true}),
        json!({"wordGap": 200.5}),
    ] {
        let mut p = bad.clone();
        p["layer"] = json!(id.0);
        let e = s.execute("type.setDynamicText", p).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { ref cmd, .. } if cmd == "type.setDynamicText"), "{bad}: {e}");
    }
    assert_eq!(history_len(&s), before);
    let gaps = |s: &Session| text(s, id).fit.map(|f| (f.word_gap, f.line_gap));
    assert_eq!(gaps(&s), Some((0.0, 0.0)));
    s.execute("type.setDynamicText", json!({"layer": id.0, "lineGap": 50})).unwrap();
    assert_eq!(history_len(&s), before + 1);
    assert_eq!(text(&s, id).fit.unwrap().line_gap, 50.0);
    assert_eq!(text(&s, id).fit.unwrap().word_gap, 0.0, "unspecified gap kept");
    s.execute("type.setDynamicText", json!({"layer": id.0, "wordGap": -50, "lineGap": 200})).unwrap();
    assert_eq!(gaps(&s), Some((-50.0, 200.0)));
    // Word gap goes down to -100, line gap only to -50; each error names its own range.
    s.execute("type.setDynamicText", json!({"layer": id.0, "wordGap": -100, "lineGap": -50})).unwrap();
    assert_eq!(text(&s, id).fit.unwrap().word_gap, -100.0);
    let e = s.execute("type.setDynamicText", json!({"layer": id.0, "wordGap": -101})).unwrap_err().to_string();
    assert!(e.contains("-100 to 200"), "{e}");
    let e = s.execute("type.setDynamicText", json!({"layer": id.0, "lineGap": -51})).unwrap_err().to_string();
    assert!(e.contains("-50 to 200"), "{e}");
}

#[test]
fn bigger_line_gap_shrinks_the_fit() {
    let words = "one two three four five six seven eight nine ten eleven twelve";
    let (mut s, id) = session_with_box_text(words, 12.0, [0.0, 0.0, 200.0, 200.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let size0 = effective(&s, id).0;
    s.execute("type.setDynamicText", json!({"layer": id.0, "lineGap": 200})).unwrap();
    assert!(effective(&s, id).0 < size0, "{size0} -> {:?}", effective(&s, id));
}

#[test]
fn refused_targets() {
    // Vertical.
    let (mut s, id) = session_with_box_text("abc", 12.0, [0.0, 0.0, 100.0, 100.0]);
    s.execute("type.orientation.vertical", json!({"layer": id.0})).unwrap();
    assert!(!s.is_enabled("type.convertToDynamicText"));
    let before = history_len(&s);
    let e = s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap_err().to_string();
    assert!(e.contains("vertical"), "{e}");
    assert_eq!(history_len(&s), before);
    assert!(text(&s, id).fit.is_none());

    // Right-to-left paragraphs.
    let (mut s, id) = session_with_box_text("abc", 12.0, [0.0, 0.0, 100.0, 100.0]);
    s.execute("type.setStyle", json!({"layer": id.0, "direction": "rtl"})).unwrap();
    assert!(!s.is_enabled("type.convertToDynamicText"));
    let e = s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap_err().to_string();
    assert!(e.contains("right-to-left"), "{e}");
    assert!(text(&s, id).fit.is_none());

    // Path text.
    let (mut s, id) = session_with_point_text("abc", 12.0);
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polyline(&[(10.0, 200.0), (390.0, 200.0)])]));
        Ok(())
    })
    .unwrap();
    s.execute("type.attachToPath", json!({"layer": id.0})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { .. }));
    assert!(!s.is_enabled("type.convertToDynamicText"));
    let e = s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap_err().to_string();
    assert!(e.contains("path"), "{e}");
    assert!(text(&s, id).fit.is_none());

    // Several layers at once.
    let (mut s, a) = session_with_box_text("abc", 12.0, [0.0, 0.0, 100.0, 100.0]);
    let b = LayerId(s.execute("type.create", json!({"x": 0, "y": 0, "text": "def", "box": [0, 120, 100, 100]})).unwrap()["layer"].as_u64().unwrap());
    let e = s.execute("type.convertToDynamicText", json!({"layers": [a.0, b.0]})).unwrap_err().to_string();
    assert!(e.contains("one type layer"), "{e}");
    assert!(text(&s, a).fit.is_none() && text(&s, b).fit.is_none());

    // Not a type layer.
    let mut s = session();
    let bg = s.active().unwrap().active_layer.unwrap();
    assert!(!s.is_enabled("type.convertToDynamicText"));
    assert!(s.execute("type.convertToDynamicText", json!({"layer": bg.0})).is_err());
    for p in [json!({"layer": "x"}), json!({"layer": u64::MAX})] {
        assert!(s.execute("type.convertToDynamicText", p.clone()).is_err());
        assert!(s.execute("type.releaseDynamicText", p.clone()).is_err());
        assert!(s.execute("type.setDynamicText", p).is_err());
    }
}

#[test]
fn a_fitted_layer_loaded_as_vertical_must_be_released_first() {
    // The commands refuse to make a fitted layer vertical; a file can still hold one. Edits are
    // refused until it is released, and the solver never sees it.
    let (mut s, id) = session_with_box_text("Hello there", 12.0, [0.0, 0.0, 300.0, 100.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    s.edit("vertical", |doc, _| {
        if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
            t.orientation = photocraft_doc::text::Orientation::Vertical;
        }
        Ok(())
    })
    .unwrap();
    let (size, before) = (text(&s, id).size_pt, history_len(&s));
    assert_release_first(s.execute("type.edit", json!({"layer": id.0, "text": "Hello there ".repeat(50)})), "type.edit");
    assert_eq!(history_len(&s), before);
    s.execute("type.updateAllTextLayers", json!({})).unwrap();
    assert!((text(&s, id).size_pt - size).abs() < 1e-4, "the solver never sees vertical text");
    s.execute("type.releaseDynamicText", json!({"layer": id.0})).unwrap();
    assert!(text(&s, id).fit.is_none());
    s.execute("type.edit", json!({"layer": id.0, "text": "Hello again"})).unwrap();
}

#[test]
fn locked_layer_is_refused() {
    let (mut s, id) = session_with_box_text("abc", 12.0, [0.0, 0.0, 100.0, 100.0]);
    lock_all(&mut s, id);
    assert!(s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap_err().to_string().contains("locked"));
    assert!(text(&s, id).fit.is_none());
}

#[test]
fn psd_export_warns_for_dynamic_text() {
    let (mut s, id) = session_with_box_text("Hello dynamic world of text", 12.0, [0.0, 0.0, 300.0, 150.0]);
    let doc = s.active().unwrap().doc.clone();
    let plain = photocraft_io::export(&doc, "x.psd", &photocraft_io::ExportOptions::default()).unwrap();
    assert!(!plain.warnings.iter().any(|w| w.contains("Dynamic Text")), "{:?}", plain.warnings);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let (hi, lo) = effective(&s, id);
    let doc = s.active().unwrap().doc.clone();
    let r = photocraft_io::export(&doc, "x.psd", &photocraft_io::ExportOptions::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("Dynamic Text is saved as regular text")), "{:?}", r.warnings);
    // The PSD holds the baked look: ordinary paragraph text, a break and a size per line.
    let back = photocraft_io::import("x.psd", &r.bytes).unwrap().document;
    let t = back
        .walk()
        .into_iter()
        .find_map(|(_, _, l)| match &l.content {
            LayerContent::Text(t) => Some(t.clone()),
            _ => None,
        })
        .expect("still a type layer");
    assert!(t.fit.is_none());
    assert!(matches!(t.shape, TextShape::Box { .. }));
    let sizes = run_sizes(&t);
    assert!(sizes.len() >= 2 && sizes.iter().all(|&v| v >= lo - 0.1 && v <= hi + 0.1), "{sizes:?} vs {lo}..{hi}");
    assert!(t.text.contains('\u{3}'), "{:?}", t.text);
}

#[test]
fn psd_export_is_baked() {
    let (mut s, id) = session_with_box_text("Hello dynamic world of text", 12.0, [0.0, 0.0, 300.0, 150.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    // A stale TySh (an older build's, made from the unbaked layer) is replaced too.
    s.edit("stale", |doc, _| {
        if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
            t.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::build_tysh(&TextLayer { fit: None, ..t.clone() }, 72.0, None)));
        }
        Ok(())
    })
    .unwrap();
    let doc = s.active().unwrap().doc.clone();
    let r = photocraft_io::export(&doc, "x.psd", &photocraft_io::ExportOptions::default()).unwrap();
    let back = photocraft_io::import("x.psd", &r.bytes).unwrap().document;
    let t = back
        .walk()
        .into_iter()
        .find_map(|(_, _, l)| match &l.content {
            LayerContent::Text(t) => Some(t.clone()),
            _ => None,
        })
        .expect("type layer");
    let want = photocraft_text::fit::bake(&mut photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner()), &text(&s, id), 72.0).unwrap();
    assert_eq!(t.text, want.text);
    assert!(t.text.contains('\u{3}'));
    let (got, exp) = (run_sizes(&t), run_sizes(&want));
    assert_eq!(got.len(), exp.len(), "{got:?} vs {exp:?}");
    assert!(got.iter().zip(&exp).all(|(a, b)| (a - b).abs() < 0.05), "{got:?} vs {exp:?}");
    // The layer itself is still Dynamic Text.
    assert!(text(&s, id).fit.is_some());
}

#[test]
fn fitted_layer_refuses_changes_that_would_stop_the_fit() {
    let (mut s, id) = session_with_box_text("Hello dynamic world", 12.0, [0.0, 0.0, 300.0, 100.0]);
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polyline(&[(10.0, 200.0), (390.0, 200.0)])]));
        Ok(())
    })
    .unwrap();
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let before = history_len(&s);
    for (cmd, params) in [
        ("type.orientation.vertical", json!({"layer": id.0})),
        ("type.setStyle", json!({"layer": id.0, "direction": "rtl"})),
        ("type.attachToPath", json!({"layer": id.0})),
    ] {
        match s.execute(cmd, params) {
            Err(EngineError::BadParams { cmd: c, msg }) => {
                assert_eq!(c, cmd);
                assert!(msg.contains("Release Dynamic Text first"), "{cmd}: {msg}");
            }
            other => panic!("{cmd}: {other:?}"),
        }
    }
    assert_eq!(history_len(&s), before, "no history step");
    let t = text(&s, id);
    assert!(t.fit.is_some() && t.orientation == photocraft_doc::text::Orientation::Horizontal);
    // After releasing, the same commands work.
    s.execute("type.releaseDynamicText", json!({"layer": id.0})).unwrap();
    s.execute("type.orientation.vertical", json!({"layer": id.0})).unwrap();
}

#[test]
fn set_dynamic_text_without_gaps_is_refused() {
    let (mut s, id) = session_with_box_text("Hello", 12.0, [0.0, 0.0, 300.0, 100.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let before = history_len(&s);
    for p in [json!({"layer": id.0}), json!({"layer": id.0, "wordGap": null})] {
        match s.execute("type.setDynamicText", p) {
            Err(EngineError::BadParams { cmd, msg }) => {
                assert_eq!(cmd, "type.setDynamicText");
                assert!(msg.contains("give wordGap or lineGap"), "{msg}");
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(history_len(&s), before);
}

#[test]
fn convert_release_convert_stays_sane() {
    let (mut s, id) = session_with_box_text("Hello dynamic world of text", 12.0, [0.0, 0.0, 300.0, 150.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    s.execute("type.releaseDynamicText", json!({"layer": id.0})).unwrap();
    let released = run_sizes(&text(&s, id));
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    // The released text is already as big as the box allows, so refitting it scales by about 1.
    let t = text(&s, id);
    let lines = t.fit.as_ref().map(|f| f.lines.clone()).unwrap_or_default();
    assert!(!lines.is_empty() && lines.iter().all(|l| (l.scale - 1.0).abs() < 0.25), "{lines:?} on {released:?}");
}

#[test]
fn release_refuses_a_look_it_cannot_keep() {
    let (mut s, id) = session_with_box_text("The quick brown fox jumps over the lazy dog", 12.0, [0.0, 0.0, 100.0, 150.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    // A file with one solved line for text far wider than the box.
    s.edit("bad lines", |doc, _| {
        if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content)
            && let Some(f) = t.fit.as_mut()
        {
            f.lines = vec![photocraft_doc::text::FitLine { start: 0, scale: 1.0 }];
        }
        Ok(())
    })
    .unwrap();
    let before = history_len(&s);
    let e = s.execute("type.releaseDynamicText", json!({"layer": id.0})).unwrap_err();
    assert!(matches!(e, EngineError::BadParams { ref cmd, .. } if cmd == "type.releaseDynamicText"), "{e}");
    assert_eq!(history_len(&s), before);
    assert!(text(&s, id).fit.is_some());
    let doc = s.active().unwrap().doc.clone();
    let r = photocraft_io::export(&doc, "x.psd", &photocraft_io::ExportOptions::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("could not be baked")), "{:?}", r.warnings);
}

/// `r` is the guard's refusal, named after `cmd`.
fn assert_release_first(r: Result<Value>, cmd: &str) {
    match r {
        Err(EngineError::BadParams { cmd: c, msg }) => {
            assert_eq!(c, cmd);
            assert!(msg.contains("Release Dynamic Text first"), "{cmd}: {msg}");
        }
        other => panic!("{cmd}: {other:?}"),
    }
}

#[test]
fn convert_to_point_text_on_a_fitted_layer_is_refused() {
    let (mut s, id) = session_with_box_text("Hello dynamic world", 12.0, [0.0, 0.0, 300.0, 100.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let (before, t0) = (history_len(&s), text(&s, id));
    assert_release_first(s.execute("type.convertToPointText", json!({"layer": id.0})), "type.convertToPointText");
    assert_eq!(history_len(&s), before, "no history step");
    let t = text(&s, id);
    assert_eq!((t.text, t.shape), (t0.text, t0.shape));
    assert!(t.fit.is_some());
}

#[test]
fn paragraph_styles_cannot_make_a_fitted_layer_right_to_left() {
    let (mut s, id) = session_with_box_text("Hello dynamic world", 12.0, [0.0, 0.0, 300.0, 100.0]);
    let plain =
        s.execute("type.paragraphStyle.new", json!({"name": "Plain", "fromSelection": false, "attrs": {"align": "left"}})).unwrap()["id"].as_u64().unwrap();
    let rtl =
        s.execute("type.paragraphStyle.new", json!({"name": "Rtl", "fromSelection": false, "attrs": {"direction": "rtl"}})).unwrap()["id"].as_u64().unwrap();
    s.execute("type.paragraphStyle.apply", json!({"id": plain, "layer": id.0})).unwrap();
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let before = history_len(&s);
    assert_release_first(s.execute("type.paragraphStyle.apply", json!({"id": rtl, "layer": id.0})), "type.paragraphStyle.apply");
    assert_release_first(s.execute("type.paragraphStyle.set", json!({"id": plain, "attrs": {"direction": "rtl"}})), "type.paragraphStyle.set");
    assert_eq!(history_len(&s), before, "no history step");
    let t = text(&s, id);
    assert!(t.fit.is_some() && fit_refusal(&t).is_none(), "{:?}", t.paragraph_runs());
}

#[test]
fn auto_direction_right_to_left_text_is_refused() {
    // Hebrew in an Auto-direction paragraph lays out right to left.
    let (mut s, id) = session_with_box_text("שלום עולם", 12.0, [0.0, 0.0, 300.0, 100.0]);
    assert!(fit_refusal(&text(&s, id)).is_some_and(|m| m.contains("right-to-left")));
    assert!(!s.is_enabled("type.convertToDynamicText"));
    let before = history_len(&s);
    let e = s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap_err().to_string();
    assert!(e.contains("right-to-left"), "{e}");
    assert_eq!(history_len(&s), before);
    assert!(text(&s, id).fit.is_none());
    // A paragraph that starts with Latin letters stays left to right.
    let (mut s, id) = session_with_box_text("Hi שלום", 12.0, [0.0, 0.0, 300.0, 100.0]);
    assert!(fit_refusal(&text(&s, id)).is_none());
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    // Typing text that turns the fitted layer right to left is refused.
    let before = history_len(&s);
    assert_release_first(s.execute("type.edit", json!({"layer": id.0, "text": "שלום Hi"})), "type.edit");
    assert_eq!(history_len(&s), before);
    assert_eq!(text(&s, id).text, "Hi שלום");
}

#[test]
fn release_and_psd_export_name_condensed_type() {
    let (mut s, id) = session_with_box_text("The quick brown fox jumps over the lazy dog", 12.0, [0.0, 0.0, 300.0, 300.0]);
    s.execute("type.setStyle", json!({"layer": id.0, "horizontalScale": 80})).unwrap();
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    let before = history_len(&s);
    match s.execute("type.releaseDynamicText", json!({"layer": id.0})) {
        Err(EngineError::BadParams { cmd, msg }) => {
            assert_eq!(cmd, "type.releaseDynamicText");
            assert_eq!(msg, "Release Dynamic Text can't keep condensed type (Horizontal Scale below 100%) yet; set it to 100% or more first");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(history_len(&s), before);
    assert!(text(&s, id).fit.is_some());
    let name = s.active().unwrap().doc.layer(id).unwrap().name.clone();
    let doc = s.active().unwrap().doc.clone();
    let r = photocraft_io::export(&doc, "x.psd", &photocraft_io::ExportOptions::default()).unwrap();
    let dt: Vec<&String> = r.warnings.iter().filter(|w| w.contains("Dynamic Text")).collect();
    assert_eq!(dt.len(), 1, "{:?}", r.warnings);
    assert!(dt[0].contains(&format!("\"{name}\"")) && dt[0].contains("condensed type (Horizontal Scale below 100%)"), "{dt:?}");
}

#[test]
fn typing_after_release_at_a_former_line_end_takes_plain_tracking() {
    let (mut s, id) = session_with_box_text("The quick brown fox jumps over the lazy dog", 12.0, [0.0, 0.0, 300.0, 300.0]);
    s.execute("type.convertToDynamicText", json!({"layer": id.0})).unwrap();
    s.execute("type.setDynamicText", json!({"layer": id.0, "wordGap": 60})).unwrap();
    s.execute("type.releaseDynamicText", json!({"layer": id.0})).unwrap();
    let released = text(&s, id);
    // The word gap became tracking on the spaces inside lines, so plain runs are not a given.
    assert!(released.char_runs().iter().any(|r| r.style.tracking != 0.0), "{:?}", released.char_runs());
    let brk = released.text.find('\u{3}').expect("a baked line break");
    let at = released.text[..brk].chars().count();
    s.execute("type.insertText", json!({"layer": id.0, "text": "x", "at": at})).unwrap();
    let typed = text(&s, id);
    assert_eq!(typed.text, format!("{}x{}", &released.text[..brk], &released.text[brk..]));
    let mut end = 0;
    let run = typed.char_runs().into_iter().find(|r| {
        end += r.len;
        end > brk
    });
    let run = run.expect("a run covers the typed character");
    assert_eq!(run.style.tracking, 0.0, "the typed character takes the word's plain tracking");
}
