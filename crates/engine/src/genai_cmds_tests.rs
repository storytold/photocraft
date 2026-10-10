use std::sync::{Arc, Mutex};

use photocraft_genai::{Capabilities, EditRequest, GenError, GeneratedImage, ProviderInfo};

use super::*;

/// Answers every edit with a solid colour at the request's size (or `size` when set), and keeps
/// what it was sent.
struct Fake {
    color: [u8; 3],
    size: Option<(u32, u32)>,
    fail: Option<GenError>,
    aspect: Option<(u32, u32)>,
    jpeg: bool,
    sent: Mutex<Vec<EditRequest>>,
}

impl Fake {
    fn new(color: [u8; 3]) -> Arc<Fake> {
        Arc::new(Fake { color, size: None, fail: None, aspect: None, jpeg: false, sent: Mutex::new(Vec::new()) })
    }
}

impl ImageProvider for Fake {
    fn info(&self) -> ProviderInfo {
        ProviderInfo { id: "fake", name: "Fake", model: "fake-1".into(), host: "example.invalid" }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { generate: false, edit: true, variations: false }
    }
    fn edit_aspect(&self, w: u32, h: u32) -> (u32, u32) {
        self.aspect.unwrap_or((w, h))
    }
    fn edit(&self, r: &EditRequest) -> photocraft_genai::Result<GeneratedImage> {
        self.sent.lock().unwrap().push(r.clone());
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        let (w, h) = self.size.or_else(|| photocraft_genai::png_size(&r.image)).unwrap();
        let px: Vec<u8> = (0..w * h).flat_map(|_| [self.color[0], self.color[1], self.color[2], 255]).collect();
        let img = Image::from_u8(w, h, ChannelLayout::Rgba, px).unwrap();
        if self.jpeg {
            let rgb = img.convert(ChannelLayout::Rgb, CodecSample::U8);
            return Ok(GeneratedImage { bytes: codecs::encode(&rgb, Format::Jpeg, &Default::default()).unwrap(), mime: "image/jpeg".into() });
        }
        Ok(GeneratedImage { bytes: codecs::encode(&img, Format::Png, &Default::default()).unwrap(), mime: "image/png".into() })
    }
}

fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "#ffffff"})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn rgb_at(s: &Session, x: i32, y: i32) -> [u8; 3] {
    let d = doc(s);
    let p = photocraft_compose::flatten(d).get(x, y);
    [0, 1, 2].map(|c| (p[c] * 255.0).round() as u8)
}

#[test]
fn needs_a_provider_and_a_selection() {
    let mut s = session(64, 64);
    assert!(!s.is_enabled(FILL), "no provider: off");
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    assert!(!s.is_enabled(FILL), "still no provider");
    assert!(s.execute(FILL, json!({"prompt": "a cat"})).is_err());
    let fake = Fake::new([255, 0, 0]);
    s.set_image_provider(Some(fake.clone()));
    assert!(s.is_enabled(FILL));
    s.execute("select.deselect", json!({})).unwrap();
    assert!(!s.is_enabled(FILL), "no selection: off");
    assert!(fake.sent.lock().unwrap().is_empty());
}

#[test]
fn the_result_lands_on_a_new_layer_masked_by_the_selection_and_undoes_in_one_step() {
    let mut s = session(200, 100);
    s.execute("select.rect", json!({"x": 80, "y": 30, "width": 40, "height": 40})).unwrap();
    let fake = Fake::new([255, 0, 0]);
    s.set_image_provider(Some(fake.clone()));
    let before = doc(&s).layers.len();
    let history = s.active().unwrap().history.past_len();
    let r = s.execute(FILL, json!({"prompt": "  a red   balloon "})).unwrap();

    let d = doc(&s);
    assert_eq!(d.layers.len(), before + 1);
    let layer = d.layer(photocraft_doc::LayerId(r["layer"].as_u64().unwrap())).unwrap();
    assert_eq!(layer.name, "a red balloon");
    assert!(layer.mask.is_some(), "masked by the selection, like Photoshop's generative layer");
    assert_eq!(s.active().unwrap().active_layer, Some(layer.id));
    assert_eq!(rgb_at(&s, 100, 50), [255, 0, 0], "inside the selection: the result");
    assert_eq!(rgb_at(&s, 70, 50), [255, 255, 255], "outside it: untouched, though the result covers it");
    assert_eq!(rgb_at(&s, 5, 5), [255, 255, 255]);
    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
    assert!(doc(&s).selection.is_some(), "the selection stays");

    // What was sent: the prompt, the image and a mask that is white only over the selection.
    let sent = fake.sent.lock().unwrap();
    assert_eq!(sent[0].prompt, "a red   balloon");
    let mask = codecs::decode(&sent[0].mask).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
    let area = Rect::from_xywh(r["bounds"][0].as_i64().unwrap() as i32, r["bounds"][1].as_i64().unwrap() as i32, 1, 1);
    let at = |x: i32, y: i32| mask.data()[((y - area.y0) as usize * mask.width() as usize + (x - area.x0) as usize) * 4];
    assert_eq!(at(100, 50), 255);
    assert_eq!(at(78, 50), 0);
    drop(sent);

    assert!(s.undo());
    assert_eq!(doc(&s).layers.len(), before);
    assert_eq!(rgb_at(&s, 100, 50), [255, 255, 255]);
}

#[test]
fn an_empty_prompt_is_allowed_and_names_the_layer_generative_fill() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    s.set_image_provider(Some(Fake::new([0, 0, 255])));
    let r = s.execute(FILL, json!({})).unwrap();
    let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(doc(&s).layer(id).unwrap().name, "Generative Fill");
    let long = "word ".repeat(30);
    s.execute(FILL, json!({"prompt": long})).unwrap();
    assert!(doc(&s).layers.last().unwrap().name.ends_with('…'));
}

#[test]
fn a_result_of_another_size_is_scaled_to_the_area() {
    let mut s = session(300, 300);
    s.execute("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    let fake = Arc::new(Fake { size: Some((1024, 1024)), ..Arc::into_inner(Fake::new([0, 255, 0])).unwrap() });
    s.set_image_provider(Some(fake));
    s.execute(FILL, json!({"prompt": "grass"})).unwrap();
    assert_eq!(rgb_at(&s, 150, 150), [0, 255, 0]);
    assert_eq!(rgb_at(&s, 101, 101), [0, 255, 0]);
}

#[test]
fn a_jpeg_result_is_read_too() {
    let mut s = session(120, 120);
    s.execute("select.rect", json!({"x": 30, "y": 30, "width": 60, "height": 60})).unwrap();
    let fake = Arc::new(Fake { jpeg: true, ..Arc::into_inner(Fake::new([0, 0, 255])).unwrap() });
    s.set_image_provider(Some(fake));
    s.execute(FILL, json!({"prompt": "blue"})).unwrap();
    let [r, g, b] = rgb_at(&s, 60, 60);
    assert!(r < 10 && g < 10 && b > 245, "{r} {g} {b}");
    assert_eq!(rgb_at(&s, 10, 10), [255, 255, 255]);
}

#[test]
fn failures_leave_no_history_and_say_why() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let history = s.active().unwrap().history.past_len();
    let fake = Arc::new(Fake { fail: Some(GenError::Auth), ..Arc::into_inner(Fake::new([0, 0, 0])).unwrap() });
    s.set_image_provider(Some(fake));
    let e = s.execute(FILL, json!({"prompt": "x"})).unwrap_err().to_string();
    assert!(e.contains("API key"), "{e}");
    assert_eq!(s.active().unwrap().history.past_len(), history);
    assert!(s.execute(FILL, json!({"prompt": 5})).is_err());
    assert!(s.execute(FILL, json!({"prompt": "x".repeat(photocraft_genai::MAX_PROMPT + 1)})).is_err());
}

#[test]
fn the_document_keeps_its_mode_and_depth() {
    for (mode, depth) in [("rgb", 16), ("grayscale", 8), ("cmyk", 8), ("rgb", 32)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64, "mode": mode, "depth": depth, "background": "#ffffff"})).unwrap();
        s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
        s.set_image_provider(Some(Fake::new([128, 128, 128])));
        let r = s.execute(FILL, json!({"prompt": "x"})).unwrap();
        let d = doc(&s);
        let l = d.layer(photocraft_doc::LayerId(r["layer"].as_u64().unwrap())).unwrap();
        let f = l.surface().unwrap().format();
        assert_eq!((f.mode, f.sample), (d.mode, d.depth), "{mode} {depth}");
    }
}

#[test]
fn the_context_area_grows_toward_the_providers_shape_inside_the_canvas() {
    let canvas = Rect::from_xywh(0, 0, 1000, 600);
    let same = |w, h| (w, h);
    // A 100² selection gets a 64 px margin on every side.
    assert_eq!(context_area(Rect::from_xywh(400, 200, 100, 100), canvas, same), Rect::new(336, 136, 564, 364));
    // At an edge the margin is cut off by the canvas.
    assert_eq!(context_area(Rect::from_xywh(0, 0, 100, 100), canvas, same), Rect::new(0, 0, 164, 164));
    // A provider that only makes 16:9 widens the area around its centre.
    let r = context_area(Rect::from_xywh(400, 200, 100, 100), canvas, |_, _| (16, 9));
    assert_eq!(r.height(), 228);
    assert!((f64::from(r.width()) / f64::from(r.height()) - 16.0 / 9.0).abs() < 0.01, "{r:?}");
    assert_eq!(r.x0 + r.x1, 336 + 564, "centred");
    // Against the canvas edge it shifts inward instead of leaving the canvas.
    let r = context_area(Rect::from_xywh(0, 0, 100, 100), canvas, |_, _| (16, 9));
    assert_eq!(r.x0, 0);
    assert!(r.x1 <= 1000);
    // It can't grow past the canvas: the remaining mismatch is scaled away.
    let r = context_area(Rect::from_xywh(0, 0, 1000, 600), canvas, |_, _| (21, 9));
    assert_eq!(r, canvas);
}
