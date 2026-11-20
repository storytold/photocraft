//! Object Selection on a segmentation model: `select.object`'s `"method"`.

use std::sync::{Arc, Mutex};

use super::*;
use crate::object_model::{ObjectModel, SegmentAnswer, SegmentRequest};

type Reply = Box<dyn Fn(&SegmentRequest) -> std::result::Result<SegmentAnswer, String> + Send + Sync>;

/// A stand-in model: records every request and answers with `reply`.
struct Fake {
    unavailable: Option<String>,
    calls: Mutex<Vec<SegmentRequest>>,
    reply: Reply,
}

impl Fake {
    fn new(reply: Reply) -> Arc<Self> {
        Arc::new(Self { unavailable: None, calls: Mutex::new(Vec::new()), reply })
    }
    fn unavailable(why: &str) -> Arc<Self> {
        Arc::new(Self { unavailable: Some(why.into()), calls: Mutex::new(Vec::new()), reply: Box::new(|_| Err("not run".into())) })
    }
    /// A disc filling the requested box (the whole picture without one).
    fn disc() -> Arc<Self> {
        Self::new(Box::new(|req| Ok(SegmentAnswer { mask: disc_mask(req), matched: true })))
    }
    fn calls(&self) -> Vec<SegmentRequest> {
        self.calls.lock().unwrap().clone()
    }
}

impl ObjectModel for Fake {
    fn unavailable(&self) -> Option<String> {
        self.unavailable.clone()
    }
    fn segment(&self, req: &SegmentRequest, _cancelled: &(dyn Fn() -> bool + Sync)) -> std::result::Result<SegmentAnswer, String> {
        self.calls.lock().unwrap().push(req.clone());
        (self.reply)(req)
    }
}

fn disc_mask(req: &SegmentRequest) -> Vec<u8> {
    let (w, h) = (req.width as f32, req.height as f32);
    let [x0, y0, x1, y1] = req.area.unwrap_or([0.0, 0.0, 1.0, 1.0]);
    let (cx, cy, rx, ry) = ((x0 + x1) * 0.5 * w, (y0 + y1) * 0.5 * h, (x1 - x0) * 0.5 * w, (y1 - y0) * 0.5 * h);
    (0..req.height)
        .flat_map(|y| (0..req.width).map(move |x| (x, y)))
        .map(|(x, y)| {
            let (dx, dy) = ((x as f32 + 0.5 - cx) / rx.max(0.5), (y as f32 + 0.5 - cy) / ry.max(0.5));
            if dx * dx + dy * dy <= 1.0 { 255 } else { 0 }
        })
        .collect()
}

fn session(w: i32, h: i32, model: Option<&Arc<Fake>>) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": 8})).unwrap();
    s.object_model = model.map(|m| Arc::clone(m) as Arc<dyn ObjectModel>);
    s
}

fn cov(s: &Session, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn undo_label(s: &Session) -> Option<String> {
    s.active().unwrap().history.undo_label().map(str::to_string)
}

#[test]
fn auto_uses_the_model_when_it_is_ready() {
    let m = Fake::disc();
    let mut s = session(200, 160, Some(&m));
    let r = s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true})).unwrap();
    assert_eq!(r["method"], "model");
    assert_eq!(r["changed"], true);
    assert!(r.get("modelUnavailable").is_none());
    assert_eq!(m.calls().len(), 1);
    assert!(cov(&s, 100, 80) > 0.99, "the disc's centre is selected");
    assert!(cov(&s, 51, 41) < 0.01, "the rectangle's corner is outside the disc");
    assert_eq!(undo_label(&s).as_deref(), Some("Object Selection"));
}

#[test]
fn auto_falls_back_to_classic_and_says_why() {
    let m = Fake::unavailable("the model is not downloaded");
    let mut s = session(200, 160, Some(&m));
    let r = s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true})).unwrap();
    assert_eq!(r["method"], "classic");
    assert_eq!(r["modelUnavailable"], "the model is not downloaded");
    assert!(m.calls().is_empty());
}

#[test]
fn auto_without_a_model_is_classic_without_a_reason() {
    let mut s = session(200, 160, None);
    let r = s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true})).unwrap();
    assert_eq!(r["method"], "classic");
    assert!(r.get("modelUnavailable").is_none());
}

#[test]
fn classic_never_asks_the_model() {
    let m = Fake::disc();
    let mut s = session(200, 160, Some(&m));
    let r = s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true, "method": "classic"})).unwrap();
    assert_eq!(r["method"], "classic");
    assert!(m.calls().is_empty());
}

#[test]
fn model_method_errors_when_unavailable_or_missing() {
    let m = Fake::unavailable("not set up");
    let mut s = session(200, 160, Some(&m));
    let e = s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true, "method": "model"})).unwrap_err();
    assert!(e.to_string().contains("not set up"), "{e}");
    let mut s = session(200, 160, None);
    assert!(s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true, "method": "model"})).is_err());
    assert!(s.active().unwrap().doc.selection.is_none());
}

#[test]
fn unknown_or_mistyped_method_errors() {
    let m = Fake::disc();
    let mut s = session(200, 160, Some(&m));
    for method in [json!("fast"), json!(5), json!(["model"]), json!(true)] {
        assert!(s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true, "method": method})).is_err());
    }
    assert!(m.calls().is_empty());
}

#[test]
fn the_area_sent_is_the_rectangle_inside_the_crop() {
    let m = Fake::disc();
    let mut s = session(200, 160, Some(&m));
    s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true})).unwrap();
    // The crop is the rectangle plus 16 px on every side: (34, 24)..(166, 136).
    let req = m.calls().remove(0);
    assert_eq!((req.width, req.height), (132, 112));
    assert_eq!(req.rgb.len(), 132 * 112 * 3);
    let [x0, y0, x1, y1] = req.area.unwrap();
    assert!(close(x0, 16.0 / 132.0) && close(y0, 16.0 / 112.0), "{x0} {y0}");
    assert!(close(x1, 116.0 / 132.0) && close(y1, 96.0 / 112.0), "{x1} {y1}");
    assert!(req.clicks.is_empty());
}

#[test]
fn a_rectangle_on_the_canvas_edge_keeps_its_area_in_range() {
    let m = Fake::disc();
    let mut s = session(200, 160, Some(&m));
    s.execute("select.object", json!({"rect": [0, 0, 60, 60], "sampleAllLayers": true})).unwrap();
    // Crop (0, 0)..(76, 76): the margin is cut off by the canvas on the near sides.
    let req = m.calls().remove(0);
    assert_eq!((req.width, req.height), (76, 76));
    let [x0, y0, x1, y1] = req.area.unwrap();
    assert!(close(x0, 0.0) && close(y0, 0.0) && close(x1, 60.0 / 76.0) && close(y1, 60.0 / 76.0));
    assert!(cov(&s, 30, 30) > 0.99);
}

#[test]
fn a_mask_of_the_wrong_size_is_an_error_and_changes_nothing() {
    let m = Fake::new(Box::new(|_| Ok(SegmentAnswer { mask: vec![255; 3], matched: true })));
    let mut s = session(200, 160, Some(&m));
    let before = undo_label(&s);
    let e = s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true})).unwrap_err();
    assert!(e.to_string().contains("wrong size"), "{e}");
    assert!(s.active().unwrap().doc.selection.is_none());
    assert_eq!(undo_label(&s), before);
}

#[test]
fn nothing_matched_changes_nothing() {
    let m = Fake::new(Box::new(|req| Ok(SegmentAnswer { mask: vec![255; (req.width * req.height) as usize], matched: false })));
    let mut s = session(200, 160, Some(&m));
    let r = s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true})).unwrap();
    assert_eq!(r["changed"], false);
    assert_eq!(r["method"], "model");
    assert!(s.active().unwrap().doc.selection.is_none());
    let m = Fake::new(Box::new(|req| Ok(SegmentAnswer { mask: vec![0; (req.width * req.height) as usize], matched: true })));
    let mut s = session(200, 160, Some(&m));
    assert_eq!(s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true})).unwrap()["changed"], false);
}

#[test]
fn a_model_error_reaches_the_caller() {
    let m = Fake::new(Box::new(|_| Err("the model ran out of memory".into())));
    let mut s = session(200, 160, Some(&m));
    let e = s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true})).unwrap_err();
    assert!(e.to_string().contains("the model ran out of memory"), "{e}");
    assert!(s.active().unwrap().doc.selection.is_none());
}

#[test]
fn add_and_subtract_combine_with_the_selection() {
    let m = Fake::disc();
    let mut s = session(200, 160, Some(&m));
    s.execute("select.object", json!({"rect": [20, 20, 60, 60], "sampleAllLayers": true})).unwrap();
    s.execute("select.object", json!({"rect": [120, 80, 60, 60], "sampleAllLayers": true, "mode": "add"})).unwrap();
    assert!(cov(&s, 50, 50) > 0.99 && cov(&s, 150, 110) > 0.99);
    s.execute("select.object", json!({"rect": [20, 20, 60, 60], "sampleAllLayers": true, "mode": "subtract"})).unwrap();
    assert!(cov(&s, 50, 50) < 0.01 && cov(&s, 150, 110) > 0.99);
}

#[test]
fn hard_edge_off_still_selects() {
    let m = Fake::disc();
    let mut s = session(200, 160, Some(&m));
    let r = s.execute("select.object", json!({"rect": [50, 40, 100, 80], "sampleAllLayers": true, "hardEdge": false})).unwrap();
    assert_eq!(r["changed"], true);
    assert!(cov(&s, 100, 80) > 0.99);
}

#[test]
fn a_model_click_sends_one_positive_click() {
    let m = Fake::disc();
    let mut s = session(200, 160, Some(&m));
    let r = s.execute("select.object", json!({"point": [100, 80], "sampleAllLayers": true, "method": "model"})).unwrap();
    assert_eq!(r["method"], "model");
    let req = m.calls().remove(0);
    assert_eq!((req.width, req.height), (200, 160));
    assert_eq!(req.clicks, vec![(0.5, 0.5, true)]);
    assert!(req.area.is_none());
}

#[test]
fn a_click_on_auto_keeps_the_classical_finder() {
    let m = Fake::disc();
    let mut s = session(200, 160, Some(&m));
    let r = s.execute("select.object", json!({"point": [100, 80], "sampleAllLayers": true})).unwrap();
    assert_eq!(r["method"], "classic");
    assert!(m.calls().is_empty());
}

#[test]
fn a_big_canvas_is_downscaled_for_the_model() {
    let m = Fake::disc();
    let mut s = session(2100, 1100, Some(&m));
    s.execute("select.object", json!({"rect": [0, 0, 2100, 1100], "sampleAllLayers": true})).unwrap();
    let req = m.calls().remove(0);
    assert!(req.width.max(req.height) <= 2048, "{}x{}", req.width, req.height);
    assert_eq!((req.width, req.height), (1050, 550));
    assert_eq!(req.rgb.len(), (req.width * req.height * 3) as usize);
    // The refined selection still covers the full-resolution canvas.
    assert!(cov(&s, 1050, 550) > 0.99);
}

#[test]
fn adversarial_params_never_panic() {
    let m = Fake::disc();
    let mut s = session(200, 160, Some(&m));
    let rects = [
        json!([-1e30, -1e30, 1e30, 1e30]),
        json!([150, 120, -100, -80]),
        json!([1e30, 1e30, 5, 5]),
        json!([0, 0, 0, 0]),
        json!([300, 300, 10, 10]),
        json!("big"),
        json!([1, 2]),
    ];
    for method in [json!("model"), json!("auto"), json!("classic"), json!(null), json!({})] {
        for rect in &rects {
            let _ = s.execute("select.object", json!({"rect": rect, "sampleAllLayers": true, "method": method}));
        }
    }
    for point in [json!([-5, -5]), json!([1e30, 0]), json!(["a", "b"]), json!([200, 160])] {
        let _ = s.execute("select.object", json!({"point": point, "sampleAllLayers": true, "method": "model"}));
    }
}

#[test]
fn prepare_expands_by_the_larger_of_16_px_and_15_percent() {
    let canvas = Rect::new(0, 0, 1000, 1000);
    let p = object_model_select::prepare(canvas, object_model_select::Prompt::Box(Rect::new(300, 300, 700, 400))).unwrap().unwrap();
    // 15% of 400 is 60, of 100 is 15 (so 16).
    assert_eq!(p.crop, Rect::new(240, 284, 760, 416));
    assert_eq!(p.step, 1);
    assert!(object_model_select::prepare(canvas, object_model_select::Prompt::Box(Rect::new(2000, 2000, 2100, 2100))).unwrap().is_none());
}

#[test]
fn upsample_interpolates_and_tolerates_short_masks() {
    let up = object_model_select::upsample(&[0, 255], 2, 1, 4, 1);
    assert_eq!(up.len(), 4);
    assert!(up[0] < up[1] && up[1] < up[2] && up[2] < up[3]);
    assert_eq!(object_model_select::upsample(&[], 2, 2, 3, 3), vec![0.0; 9]);
    assert!(object_model_select::upsample(&[1], 0, 0, 3, 3).iter().all(|v| *v == 0.0));
}
