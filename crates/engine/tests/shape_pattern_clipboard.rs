//! Vector style clipboards retain the pattern tile after its source document closes.
use photocraft_doc::{Color, Fill, LayerContent, Pattern, PixelFormat, ShapeLayer, ShapeStroke};
use photocraft_engine::{Session, vector_cmds::shape_fill};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::json;

fn imported_pattern() -> Pattern {
    let mut surface = Surface::new(PixelFormat::RGBA16);
    surface.fill_rect(Rect::new(0, 0, 4, 4), &[0.13, 0.27, 0.91, 1.0]);
    Pattern::new("$$$/Patterns/Test/Blue=Imported Blue", surface, 4, 4)
}

fn new_doc(s: &mut Session, depth: u32) {
    s.execute("file.new", json!({"width":64,"height":48,"depth":depth,"background":"transparent"})).unwrap();
}

fn shape(s: &Session) -> &ShapeLayer {
    let st = s.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("shape") };
    sh
}

#[test]
fn patterned_fill_and_stroke_clipboards_keep_cow_tiles_after_source_closes() {
    for depth in [8, 16, 32] {
        for imported in [false, true] {
            let mut s = Session::new();
            new_doc(&mut s, depth);
            let pattern = if imported {
                let p = imported_pattern();
                std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).patterns.push(p.clone());
                s.patterns.items.clear();
                p
            } else {
                s.patterns.items[0].clone()
            };
            let paint = json!({"pattern":pattern.name,"id":pattern.id,"scale":100,"angle":0,"link":false,"phase":[0,0]});
            s.execute("shape.create", json!({"rect":[8,8,40,24],"fill":paint,"stroke":{"width":4,"fill":paint}})).unwrap();
            let copied_fill = shape(&s).fill.clone();
            let copied_stroke = shape(&s).stroke.clone();
            let fill_pixel = shape(&s).cache.as_ref().unwrap().rgba(24, 20);
            let stroke_pixel = shape(&s).cache.as_ref().unwrap().rgba(7, 20);
            let original = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            s.execute("path.style.copyFill", json!({})).unwrap();
            s.execute("path.style.copyStroke", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc, original);
            assert_eq!(s.active().unwrap().history.past_len(), history);
            // Editing the source tile after Copy must not change the copied COW snapshot.
            std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).patterns[0].surface.fill_rect(pattern.rect(), &[1.0, 0.0, 0.0, 1.0]);
            s.patterns.items.clear();
            assert!(s.close(0).is_some());
            for (paste, sample, expected) in [("path.style.pasteFill", (24, 20), fill_pixel), ("path.style.pasteStroke", (7, 20), stroke_pixel)] {
                new_doc(&mut s, depth);
                s.execute("shape.create", json!({"rect":[8,8,40,24],"fill":null})).unwrap();
                assert!(s.active().unwrap().doc.patterns.is_empty());
                let before = s.active().unwrap().doc.clone();
                let history = s.active().unwrap().history.past_len();
                s.execute(paste, json!({})).unwrap();
                let after = s.active().unwrap().doc.clone();
                assert_eq!(after.patterns, vec![pattern.clone()]);
                assert_eq!(s.active().unwrap().history.past_len(), history + 1);
                if paste == "path.style.pasteFill" {
                    assert_eq!(shape(&s).fill, copied_fill);
                } else {
                    assert_eq!(shape(&s).stroke, copied_stroke);
                }
                let pixel = shape(&s).cache.as_ref().unwrap().rgba(sample.0, sample.1);
                assert!(pixel[3] > 0.99, "{paste}: copied pattern remained transparent");
                assert!(
                    pixel.iter().zip(expected).all(|(a, b)| (*a - b).abs() < 0.005),
                    "{paste}, imported={imported}, depth={depth}: {pixel:?} != {expected:?}"
                );
                s.execute("edit.undo", json!({})).unwrap();
                assert_eq!(s.active().unwrap().doc, before);
                assert!(s.active().unwrap().doc.patterns.is_empty());
                s.execute("edit.redo", json!({})).unwrap();
                assert_eq!(s.active().unwrap().doc, after);
            }
        }
    }
}

#[test]
fn replacing_public_clipboards_with_plain_paints_ignores_old_pattern_snapshots() {
    let mut s = Session::new();
    new_doc(&mut s, 8);
    s.execute("shape.create", json!({"rect":[8,8,40,24],"fill":{"pattern":"Checkerboard"},"stroke":{"width":4,"fill":{"pattern":"Checkerboard"}}})).unwrap();
    s.execute("path.style.copyFill", json!({})).unwrap();
    s.execute("path.style.copyStroke", json!({})).unwrap();
    new_doc(&mut s, 8);
    s.execute("shape.create", json!({"rect":[8,8,40,24],"fill":null})).unwrap();
    let fill = Fill::Solid(Color::rgba(0.13, 0.27, 0.91, 0.73));
    let stroke = ShapeStroke { paint: fill.clone(), width: 4.0, ..Default::default() };
    s.path_fill_clipboard = Some(fill.clone());
    s.path_stroke_clipboard = Some(stroke.clone());
    s.execute("path.style.pasteFill", json!({})).unwrap();
    s.execute("path.style.pasteStroke", json!({})).unwrap();
    assert_eq!(shape(&s).fill, Some(fill));
    assert_eq!(shape(&s).stroke, Some(stroke));
    assert!(s.active().unwrap().doc.patterns.is_empty());
}

#[test]
fn legacy_pattern_clipboard_without_a_snapshot_resolves_the_library() {
    let mut s = Session::new();
    let p = s.patterns.items[0].clone();
    for old_id in ["", "obsolete-library-id"] {
        new_doc(&mut s, 16);
        s.execute("shape.create", json!({"rect":[8,8,40,24],"fill":null})).unwrap();
        s.path_fill_clipboard = Some(Fill::Pattern { name: p.name.clone(), id: old_id.into(), scale: 1.0, angle: 0.0, link: true, phase: (0.0, 0.0) });
        s.execute("path.style.pasteFill", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.patterns, vec![p.clone()]);
        assert_eq!(shape(&s).fill, Some(Fill::Pattern { name: p.name.clone(), id: p.id.clone(), scale: 1.0, angle: 0.0, link: true, phase: (0.0, 0.0) }));
        assert!(shape(&s).cache.as_ref().unwrap().rgba(24, 20)[3] > 0.99);
    }
}

#[test]
fn copied_name_only_and_obsolete_ids_bind_to_the_snapshot_despite_destination_conflicts() {
    for old_id in ["", "obsolete-imported-id"] {
        for stroke in [false, true] {
            let mut s = Session::new();
            new_doc(&mut s, 16);
            let pattern = imported_pattern();
            std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).patterns.push(pattern.clone());
            let paint = json!({"pattern":pattern.name,"id":pattern.id,"scale":37.37,"angle":17.3,"link":false,"phase":[-2.7,3.1]});
            s.execute("shape.create", json!({"rect":[8,8,40,24],"fill":paint,"stroke":{"width":4,"fill":paint}})).unwrap();
            let id = s.active().unwrap().active_layer.unwrap();
            let doc = std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc);
            let LayerContent::Shape(source) = &mut doc.layer_mut(id).unwrap().content else { panic!("source shape") };
            if let Some(Fill::Pattern { id, .. }) = &mut source.fill {
                *id = old_id.into();
            }
            if let Some(ShapeStroke { paint: Fill::Pattern { id, .. }, .. }) = &mut source.stroke {
                *id = old_id.into();
            }
            let mut expected_fill = shape(&s).fill.clone().unwrap();
            let mut expected_stroke = shape(&s).stroke.clone().unwrap();
            if let Fill::Pattern { id, .. } = &mut expected_fill {
                *id = pattern.id.clone();
            }
            if let Fill::Pattern { id, .. } = &mut expected_stroke.paint {
                *id = pattern.id.clone();
            }
            let original = s.active().unwrap().doc.clone();
            s.execute(if stroke { "path.style.copyStroke" } else { "path.style.copyFill" }, json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc, original, "normalization changes the clipboard, not imported source metadata");
            if stroke {
                assert_eq!(s.path_stroke_clipboard, Some(expected_stroke.clone()));
            } else {
                assert_eq!(s.path_fill_clipboard, Some(expected_fill.clone()));
            }
            s.patterns.items.clear();
            new_doc(&mut s, 16);
            let mut conflicting = pattern.clone();
            conflicting.id = if old_id.is_empty() { "destination-pattern-id".into() } else { old_id.into() };
            conflicting.surface.fill_rect(pattern.rect(), &[1.0, 0.0, 0.0, 1.0]);
            std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).patterns.push(conflicting.clone());
            s.execute("shape.create", json!({"rect":[8,8,40,24],"fill":null})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            s.execute(if stroke { "path.style.pasteStroke" } else { "path.style.pasteFill" }, json!({})).unwrap();
            let after = s.active().unwrap().doc.clone();
            assert_eq!(s.active().unwrap().history.past_len(), history + 1);
            assert_eq!(after.patterns, vec![conflicting, pattern.clone()]);
            if stroke {
                assert_eq!(shape(&s).stroke, Some(expected_stroke));
            } else {
                assert_eq!(shape(&s).fill, Some(expected_fill));
            }
            let pixel = shape(&s).cache.as_ref().unwrap().rgba(if stroke { 7 } else { 24 }, 20);
            assert!(
                pixel[2] > 0.85 && pixel[0] < 0.2 && pixel[3] > 0.99,
                "stroke={stroke}, old_id={old_id:?}: destination pattern replaced copied blue: {pixel:?}"
            );
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc, before);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc, after);
        }
    }
}

#[test]
fn unresolved_and_conflicting_pattern_clipboards_fail_atomically() {
    let mut s = Session::new();
    new_doc(&mut s, 16);
    let p = imported_pattern();
    std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).patterns.push(p.clone());
    s.execute("shape.create", json!({"rect":[8,8,40,24],"fill":{"pattern":p.name,"id":p.id}})).unwrap();
    s.execute("path.style.copyFill", json!({})).unwrap();
    new_doc(&mut s, 16);
    s.execute("shape.create", json!({"rect":[8,8,40,24],"fill":null})).unwrap();
    let mut conflicting = p.clone();
    conflicting.surface.fill_rect(p.rect(), &[1.0, 0.0, 0.0, 1.0]);
    std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).patterns.push(conflicting);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let error = s.execute("path.style.pasteFill", json!({})).unwrap_err();
    assert!(error.to_string().contains("different pattern"));
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
    s.path_fill_clipboard = Some(Fill::Pattern { name: "Missing".into(), id: "missing".into(), scale: 1.0, angle: 0.0, link: true, phase: (0.0, 0.0) });
    assert!(s.execute("path.style.pasteFill", json!({})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

#[test]
fn pattern_parser_falls_back_to_imported_display_names_and_rejects_conflicting_ids() {
    let mut s = Session::new();
    new_doc(&mut s, 16);
    let p = imported_pattern();
    std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).patterns.push(p.clone());
    let before = s.active().unwrap().doc.clone();
    let fill = shape_fill(&s, &json!({"pattern":"Imported Blue","id":"obsolete-id","angle":17,"phase":[3,4]})).unwrap().unwrap();
    assert_eq!(fill, Fill::Pattern { name: "Imported Blue".into(), id: p.id.clone(), scale: 1.0, angle: 17.0, link: true, phase: (3.0, 4.0) });
    assert!(shape_fill(&s, &json!({"pattern":s.patterns.items[0].name,"id":p.id})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
}
