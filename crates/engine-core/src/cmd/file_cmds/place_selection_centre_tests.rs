//! File › Place Embedded with a selection (#14102): the placed image is centred on the
//! selection's bounds and the selection removed, in the place's one history step, as measured in
//! the reference application (400 × 300 document, selection 300,200 to 380,280).

use super::*;

/// A 400 × 300 document with the selection 300,200 to 380,280 (`extra` adds to `select.rect`'s
/// params: an ellipse, a feather), or none.
fn session(selection: Option<Value>) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": 8})).unwrap();
    if let Some(extra) = selection {
        let mut p = json!({"x": 300, "y": 200, "width": 80, "height": 80});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        s.execute("select.rect", p).unwrap();
    }
    s
}

/// A gray `w` × `h` PNG.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgb, vec![128; (w * h * 3) as usize]).unwrap();
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()
}

/// Places a `w` × `h` file with `params`; its box.
fn place(s: &mut Session, w: u32, h: u32, params: Value) -> [f64; 4] {
    let r = place_bytes(s, "src.png", png(w, h), None, &params).unwrap();
    let b: Vec<f64> = r["bounds"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    [b[0], b[1], b[2], b[3]]
}

/// File › Place Embedded's params: centre on the selection.
fn menu() -> Value {
    json!({"centerOnSelection": true})
}

fn selection_bounds(s: &Session) -> Option<photocraft_geom::Rect> {
    s.active().unwrap().doc.selection.as_ref().map(photocraft_raster::Surface::content_bounds)
}

/// The measured case: a 120 × 90 file lands at 280,195 to 400,285 (centre 340,240) and a 100 × 60
/// one at 290,210 to 390,270; the selection is gone.
#[test]
fn a_place_centres_on_the_selection_and_removes_it() {
    for (w, h, want) in [(120, 90, [280.0, 195.0, 400.0, 285.0]), (100, 60, [290.0, 210.0, 390.0, 270.0])] {
        let mut s = session(Some(json!({})));
        assert_eq!(place(&mut s, w, h, menu()), want, "{w}×{h}");
        assert_eq!(selection_bounds(&s), None, "{w}×{h}: the selection is removed");
    }
}

/// One history step: Undo takes the layer away and brings the selection back; Redo places it
/// again without the selection.
#[test]
fn the_place_and_the_deselect_are_one_step() {
    let mut s = session(Some(json!({})));
    let (layers, steps) = (s.active().unwrap().doc.layer_count(), s.active().unwrap().history.past_len());
    place(&mut s, 100, 60, menu());
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layer_count(), layers);
    assert_eq!(selection_bounds(&s), Some(photocraft_geom::Rect::new(300, 200, 380, 280)), "Undo brings the selection back");
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc.layer_count(), layers + 1);
    assert_eq!(selection_bounds(&s), None);
}

/// The same place as a background job lands in the same place and removes the selection too.
#[test]
fn a_place_job_centres_on_the_selection_too() {
    let mut s = session(Some(json!({})));
    let started = s.start_place_bytes("src.png", png(100, 60), None, menu()).unwrap();
    let r = match started {
        crate::jobs::Started::Done(v) => v,
        crate::jobs::Started::Job(id) => {
            let t = std::time::Instant::now();
            loop {
                if let Some(e) = s.poll_jobs().into_iter().find(|e| e.id == id) {
                    match e.outcome {
                        crate::jobs::JobOutcome::Done(v) => break v,
                        other => panic!("the place failed: {other:?}"),
                    }
                }
                assert!(t.elapsed() < std::time::Duration::from_secs(60), "the place did not finish");
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    };
    assert_eq!(r["bounds"], json!([290.0, 210.0, 390.0, 270.0]));
    assert_eq!(selection_bounds(&s), None);
}

/// Without a selection the place centres on the canvas, as before: 100 × 60 at 150,120.
#[test]
fn without_a_selection_the_place_centres_on_the_canvas() {
    let mut s = session(None);
    assert_eq!(place(&mut s, 100, 60, menu()), [150.0, 120.0, 250.0, 180.0]);
}

/// The selection wins over an explicit `center` when asked to centre on it; without
/// `centerOnSelection` (a drop on the canvas, a script) `center` and the canvas centre are used as
/// before and the selection stays.
#[test]
fn an_explicit_center_or_no_request_keeps_the_selection() {
    let mut s = session(Some(json!({})));
    assert_eq!(place(&mut s, 100, 60, json!({"centerOnSelection": true, "center": [100, 100]})), [290.0, 210.0, 390.0, 270.0]);

    let mut s = session(Some(json!({})));
    assert_eq!(place(&mut s, 100, 60, json!({"center": [100, 100]})), [50.0, 70.0, 150.0, 130.0]);
    assert_eq!(selection_bounds(&s), Some(photocraft_geom::Rect::new(300, 200, 380, 280)));

    let mut s = session(Some(json!({})));
    assert_eq!(place(&mut s, 100, 60, json!({})), [150.0, 120.0, 250.0, 180.0]);
    assert_eq!(selection_bounds(&s), Some(photocraft_geom::Rect::new(300, 200, 380, 280)));
}

/// A selection that isn't a hard-edged rectangle (not measured) leaves the place as it was:
/// centred on the canvas, the selection kept.
#[test]
fn an_elliptical_or_feathered_selection_keeps_todays_place() {
    for extra in [json!({"ellipse": true}), json!({"feather": 4})] {
        let mut s = session(Some(extra.clone()));
        let before = selection_bounds(&s);
        assert_eq!(place(&mut s, 100, 60, menu()), [150.0, 120.0, 250.0, 180.0], "{extra}");
        assert_eq!(selection_bounds(&s), before, "{extra}: the selection stays");
    }
}
