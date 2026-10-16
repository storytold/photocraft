use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use photocraft_genai::{Capabilities, EditMode, EditRequest, GenError, GeneratedImage, ProviderInfo, ReferenceImage};

use super::*;

/// Answers every edit with a solid colour at the request's size (or `size` when set), and keeps
/// what it was sent. With `vary` each call's colour differs; with `flaky` every second call fails;
/// with `busy` the first that many calls answer that the service is over its rate limit.
struct Fake {
    color: [u8; 3],
    vary: bool,
    flaky: bool,
    /// How long each call takes.
    delay: std::time::Duration,
    /// Answer with the image sent, the masked placeholder replaced by the average colour around
    /// it as a model would, every channel moved by this much (a model's colour drift).
    echo: Option<i16>,
    calls: AtomicUsize,
    size: Option<(u32, u32)>,
    fail: Option<GenError>,
    aspect: Option<(u32, u32)>,
    jpeg: bool,
    busy: usize,
    /// Calls in progress, and the most there were at once.
    open: AtomicUsize,
    peak: AtomicUsize,
    sent: Mutex<Vec<EditRequest>>,
}

impl Fake {
    fn new(color: [u8; 3]) -> Arc<Fake> {
        Arc::new(Fake {
            color,
            vary: false,
            flaky: false,
            delay: std::time::Duration::ZERO,
            echo: None,
            calls: AtomicUsize::new(0),
            size: None,
            fail: None,
            aspect: None,
            jpeg: false,
            busy: 0,
            open: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            sent: Mutex::new(Vec::new()),
        })
    }
}

impl ImageProvider for Fake {
    fn info(&self) -> ProviderInfo {
        ProviderInfo { id: "fake", name: "Fake", model: "fake-1".into(), host: "example.invalid" }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { generate: false, edit: true, variations: false, edit_in_place: true, max_references: 4 }
    }
    fn edit_aspect(&self, w: u32, h: u32) -> (u32, u32) {
        self.aspect.unwrap_or((w, h))
    }
    fn edit(&self, r: &EditRequest) -> photocraft_genai::Result<GeneratedImage> {
        self.sent.lock().unwrap().push(r.clone());
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let open = self.open.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(open, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        self.open.fetch_sub(1, Ordering::SeqCst);
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        if n < self.busy {
            return Err(GenError::Quota);
        }
        if self.flaky && n % 2 == 1 {
            return Err(GenError::Refused("flaky".into()));
        }
        if let Some(d) = self.echo {
            let img = codecs::decode(&r.image).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
            let mask = codecs::decode(&r.mask).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
            let masked = |i: usize| mask.data()[i / 4 * 4] > 127;
            let (mut sum, mut n) = ([0u64; 3], 0u64);
            for (i, px) in img.data().as_chunks::<4>().0.iter().enumerate() {
                if !masked(i * 4) {
                    (0..3).for_each(|c| sum[c] += u64::from(px[c]));
                    n += 1;
                }
            }
            let around = sum.map(|v| (v / n.max(1)) as u8);
            let px: Vec<u8> = img
                .data()
                .iter()
                .enumerate()
                .map(|(i, &v)| match i % 4 {
                    3 => v,
                    c => (i16::from(if masked(i) { around[c] } else { v }) + d).clamp(0, 255) as u8,
                })
                .collect();
            let out = Image::from_u8(img.width(), img.height(), ChannelLayout::Rgba, px).unwrap();
            return Ok(GeneratedImage { bytes: codecs::encode(&out, Format::Png, &Default::default()).unwrap(), mime: "image/png".into() });
        }
        let color = if self.vary { [self.color[0].wrapping_add(n as u8 * 60), self.color[1], self.color[2]] } else { self.color };
        let (w, h) = self.size.or_else(|| photocraft_genai::png_size(&r.image)).unwrap();
        let px: Vec<u8> = (0..w * h).flat_map(|_| [color[0], color[1], color[2], 255]).collect();
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
fn the_results_land_in_a_new_group_masked_by_the_selection_and_undo_in_one_step() {
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
    assert!(layer.is_group());
    assert!(layer.mask.is_some(), "masked by the selection, like Photoshop's generative layer");
    assert_eq!(s.active().unwrap().active_layer, Some(layer.id));
    assert_eq!(rgb_at(&s, 100, 50), [255, 0, 0], "inside the selection: the result");
    assert_eq!(rgb_at(&s, 70, 50), [255, 255, 255], "outside it: untouched, though the result covers it");
    assert_eq!(rgb_at(&s, 5, 5), [255, 255, 255]);
    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
    assert!(doc(&s).selection.is_some(), "the selection stays");

    // What was sent: the prompt, the image and a mask that is white only over the selection.
    let sent = fake.sent.lock().unwrap();
    assert_eq!(sent.len(), 3, "three variations by default");
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
        let l = d.layer(photocraft_doc::LayerId(r["variations"][0].as_u64().unwrap())).unwrap();
        let f = l.surface().unwrap().format();
        assert_eq!((f.mode, f.sample), (d.mode, d.depth), "{mode} {depth}");
    }
}

#[test]
fn the_context_area_grows_toward_the_providers_shape_inside_the_canvas() {
    let canvas = Rect::from_xywh(0, 0, 1000, 600);
    let same = |w, h| (w, h);
    // A 30² selection gets the 64 px minimum margin on every side, a 100² one twice its size.
    assert_eq!(context_area(Rect::from_xywh(400, 200, 30, 30), canvas, same), Rect::new(336, 136, 494, 294));
    assert_eq!(context_area(Rect::from_xywh(400, 200, 100, 100), canvas, same), Rect::new(200, 0, 700, 500));
    // At an edge the margin is cut off by the canvas.
    assert_eq!(context_area(Rect::from_xywh(0, 0, 100, 100), canvas, same), Rect::new(0, 0, 300, 300));
    // A provider that only makes 16:9 widens the area around its centre.
    let r = context_area(Rect::from_xywh(400, 200, 30, 30), canvas, |_, _| (16, 9));
    assert_eq!(r.height(), 158);
    assert!((f64::from(r.width()) / f64::from(r.height()) - 16.0 / 9.0).abs() < 0.01, "{r:?}");
    assert!((r.x0 + r.x1 - (336 + 494)).abs() <= 1, "centred: {r:?}");
    // Against the canvas edge it shifts inward instead of leaving the canvas.
    let r = context_area(Rect::from_xywh(0, 0, 100, 100), canvas, |_, _| (16, 9));
    assert_eq!(r.x0, 0);
    assert!(r.x1 <= 1000);
    // It can't grow past the canvas: the remaining mismatch is scaled away.
    let r = context_area(Rect::from_xywh(0, 0, 1000, 600), canvas, |_, _| (21, 9));
    assert_eq!(r, canvas);
}

fn group(s: &Session, id: u64) -> &photocraft_doc::Layer {
    doc(s).layer(photocraft_doc::LayerId(id)).unwrap()
}

fn shown(s: &Session, id: u64) -> Vec<bool> {
    group(s, id).children().unwrap().iter().map(|l| l.visible).collect()
}

/// The colour variation `i` of group `id` holds at (`x`, `y`).
fn variation_rgb(s: &Session, id: u64, i: usize, x: i32, y: i32) -> [u8; 3] {
    let l = &group(s, id).children().unwrap()[i];
    let surf = l.surface().unwrap();
    let mut px = [0.0f32; 4];
    surf.read_pixel(x, y, &mut px[..surf.channels()]);
    let p = photocraft_raster::to_rgba(&surf.format(), &px[..surf.channels()]);
    [0, 1, 2].map(|c| (p[c] * 255.0).round() as u8)
}

fn filled(variations: Option<u64>) -> (Session, Arc<Fake>, Value) {
    let mut s = session(100, 100);
    s.execute("select.rect", json!({"x": 30, "y": 30, "width": 40, "height": 40})).unwrap();
    let fake = Arc::new(Fake { vary: true, ..Arc::into_inner(Fake::new([0, 0, 255])).unwrap() });
    s.set_image_provider(Some(fake.clone()));
    let r = s.execute(FILL, json!({"prompt": "a boat", "variations": variations})).unwrap();
    (s, fake, r)
}

#[test]
fn a_fill_makes_three_variations_with_the_first_showing() {
    let (s, fake, r) = filled(None);
    let id = r["layer"].as_u64().unwrap();
    assert_eq!(r["variations"].as_array().unwrap().len(), 3);
    let names: Vec<_> = group(&s, id).children().unwrap().iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, ["Variation 1", "Variation 2", "Variation 3"]);
    assert_eq!(shown(&s, id), [true, false, false]);
    assert_eq!(rgb_at(&s, 50, 50), variation_rgb(&s, id, 0, 50, 50));
    assert_eq!(fake.sent.lock().unwrap().len(), 3);
    assert_eq!(s.generative_layer(), Some(photocraft_doc::LayerId(id)));
    assert_eq!(s.generative_prompt(photocraft_doc::LayerId(id)), Some("a boat"));

    let (_, fake, r) = filled(Some(1));
    assert_eq!(r["variations"].as_array().unwrap().len(), 1);
    assert_eq!(fake.sent.lock().unwrap().len(), 1);
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    s.set_image_provider(Some(Fake::new([0, 0, 0])));
    for bad in [json!(0), json!(5), json!("3"), json!(-1)] {
        assert!(s.execute(FILL, json!({"variations": bad})).is_err(), "{bad}");
    }
}

#[test]
fn choosing_a_variation_shows_it_alone_in_one_undo_step() {
    let (mut s, _, r) = filled(None);
    let id = r["layer"].as_u64().unwrap();
    let colors: Vec<_> = (0..3).map(|i| variation_rgb(&s, id, i, 50, 50)).collect();
    assert_ne!(colors[0], colors[1]);
    let history = s.active().unwrap().history.past_len();
    s.execute(SELECT_VARIATION, json!({"index": 2})).unwrap();
    assert_eq!(shown(&s, id), [false, false, true]);
    assert_eq!(rgb_at(&s, 50, 50), colors[2]);
    assert_eq!(rgb_at(&s, 10, 10), [255, 255, 255], "still masked by the selection");
    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
    // With a variation itself active, the command still finds its group.
    let child = group(&s, id).children().unwrap()[0].id;
    s.execute("layer.select", json!({"layer": child.0})).unwrap();
    s.execute(SELECT_VARIATION, json!({"index": 1})).unwrap();
    assert_eq!(shown(&s, id), [false, true, false]);
    assert!(s.execute(SELECT_VARIATION, json!({"index": 3})).is_err());
    assert!(s.execute(SELECT_VARIATION, json!({})).is_err());
    assert!(s.undo());
    assert_eq!(shown(&s, id), [false, false, true]);
}

#[test]
fn variation_commands_need_a_generative_layer() {
    let mut s = session(64, 64);
    s.set_image_provider(Some(Fake::new([0, 0, 0])));
    assert!(!s.is_enabled(SELECT_VARIATION));
    assert!(!s.is_enabled(MORE));
    // A group that wasn't made by Generative Fill isn't one.
    s.execute("type.create", json!({"text": "Hi", "size": 20, "x": 5, "y": 30})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    assert_eq!(s.generative_layer(), None);
    assert!(s.execute(SELECT_VARIATION, json!({"index": 0})).is_err());
    assert!(s.execute(MORE, json!({})).is_err());
}

#[test]
fn generate_more_resends_the_same_request_and_adds_variations() {
    let (mut s, fake, r) = filled(None);
    let id = r["layer"].as_u64().unwrap();
    s.execute(SELECT_VARIATION, json!({"index": 1})).unwrap();
    // A later edit to the canvas doesn't change what is sent: the same image, mask and prompt.
    s.execute("select.deselect", json!({})).unwrap();
    let history = s.active().unwrap().history.past_len();
    let more = s.execute(MORE, json!({"count": 2})).unwrap();
    assert_eq!(more["variations"].as_array().unwrap().len(), 2);
    let names: Vec<_> = group(&s, id).children().unwrap().iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, ["Variation 1", "Variation 2", "Variation 3", "Variation 4", "Variation 5"]);
    assert_eq!(shown(&s, id), [false, false, false, true, false], "the first new one shows");
    let sent = fake.sent.lock().unwrap();
    assert_eq!(sent.len(), 5);
    assert_eq!(sent[4].image, sent[0].image);
    assert_eq!(sent[4].mask, sent[0].mask);
    assert_eq!(sent[4].prompt, "a boat");
    drop(sent);
    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
    assert!(s.undo());
    assert_eq!(shown(&s, id), [false, true, false]);
    assert!(s.execute(MORE, json!({"count": 9})).is_err());
    // Without a provider there is nothing to ask.
    s.set_image_provider(None);
    assert!(!s.is_enabled(MORE));
    assert!(s.is_enabled(SELECT_VARIATION), "flipping between results needs no provider");
}

#[test]
fn a_failed_variation_is_dropped_when_others_succeed() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let fake = Arc::new(Fake { flaky: true, ..Arc::into_inner(Fake::new([0, 200, 0])).unwrap() });
    s.set_image_provider(Some(fake));
    let r = s.execute(FILL, json!({"prompt": "x", "variations": 3})).unwrap();
    assert_eq!(r["variations"].as_array().unwrap().len(), 2);
    assert_eq!(rgb_at(&s, 16, 16), [0, 200, 0]);
}

#[test]
fn the_selected_pixels_are_never_sent() {
    // A red square under the selection on white: the provider sees the white around it carried
    // in, dimmed (no red), and the surroundings as they are.
    let mut s = session(120, 120);
    s.execute("select.rect", json!({"x": 40, "y": 40, "width": 40, "height": 40})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let fake = Fake::new([0, 0, 255]);
    s.set_image_provider(Some(fake.clone()));
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    let sent = fake.sent.lock().unwrap();
    let img = codecs::decode(&sent[0].image).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
    let (x0, y0) = (r["bounds"][0].as_i64().unwrap() as i32, r["bounds"][1].as_i64().unwrap() as i32);
    let at = |x: i32, y: i32| {
        let i = ((y - y0) as usize * img.width() as usize + (x - x0) as usize) * 4;
        [img.data()[i], img.data()[i + 1], img.data()[i + 2]]
    };
    for (x, y) in [(60, 60), (41, 41), (79, 60)] {
        let [r, g, b] = at(x, y);
        assert!(r == g && g == b && (80..=100).contains(&r), "({x},{y}) {:?}: the white around it, dimmed", at(x, y));
    }
    assert_eq!(at(30, 30), [255, 255, 255], "the context is sent as it is");
}

#[test]
fn the_fill_takes_the_colours_beside_it() {
    // Left half black, right half white, a hole across the middle: each side of the hole takes
    // the colour next to it.
    let (w, h) = (64usize, 32usize);
    let original: Vec<f32> = (0..w * h).flat_map(|i| if i % w < 32 { [0.0; 3] } else { [1.0; 3] }).collect();
    let coverage: Vec<f32> = (0..w * h).map(|i| if (16..48).contains(&(i % w)) && (8..24).contains(&(i / w)) { 1.0 } else { 0.0 }).collect();
    let fill = surroundings_fill(&original, &coverage, w, h);
    assert!(fill[(16 * w + 17) * 3] < 0.2, "{}", fill[(16 * w + 17) * 3]);
    assert!(fill[(16 * w + 46) * 3] > 0.8, "{}", fill[(16 * w + 46) * 3]);
}

#[test]
fn variations_are_requested_at_the_same_time() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let delay = std::time::Duration::from_millis(400);
    let fake = Arc::new(Fake { delay, ..Arc::into_inner(Fake::new([0, 0, 0])).unwrap() });
    s.set_image_provider(Some(fake));
    let start = std::time::Instant::now();
    let r = s.execute(FILL, json!({"variations": 4})).unwrap();
    assert_eq!(r["variations"].as_array().unwrap().len(), 4);
    // One after another would take 1.6 s.
    assert!(start.elapsed() < delay * 2, "{:?}", start.elapsed());
}

#[test]
fn a_models_colour_drift_is_taken_out() {
    // The fake redraws the image 40 levels darker everywhere. Outside the selection that shows
    // it drifted, so the fill (white, from around the selection) comes back white, not 215.
    let mut s = session(160, 160);
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap();
    let fake = Arc::new(Fake { echo: Some(-40), ..Arc::into_inner(Fake::new([0, 0, 0])).unwrap() });
    s.set_image_provider(Some(fake));
    s.execute(FILL, json!({"variations": 1})).unwrap();
    for (x, y) in [(80, 80), (52, 52), (108, 108)] {
        let [r, g, b] = rgb_at(&s, x, y);
        assert!([r, g, b].iter().all(|&c| c >= 252), "({x},{y}): {r} {g} {b}");
    }
}

#[test]
fn drift_that_varies_across_the_image_leaves_no_seam() {
    // A flat mid-grey photo; the model's result drifts from -0.1 on the left to +0.1 on the right.
    let (w, h) = (100u32, 80u32);
    let n = (w * h) as usize;
    let inside = |i: usize| {
        let (x, y) = (i % w as usize, i / w as usize);
        (30..70).contains(&x) && (20..60).contains(&y)
    };
    let reference = Reference { original: vec![0.5; n * 3], coverage: (0..n).map(|i| if inside(i) { 1.0 } else { 0.0 }).collect(), w, h };
    let mut px: Vec<f32> = (0..n)
        .flat_map(|i| {
            let d = 0.2 * (i % w as usize) as f32 / w as f32 - 0.1;
            [0.5 + d, 0.5 + d, 0.5 + d, 1.0]
        })
        .collect();
    let stats = match_surroundings(&mut px, &reference);
    assert!(stats.starts_with("mean difference"), "{stats}");
    let worst = (0..n).filter(|&i| inside(i)).map(|i| (px[i * 4] - 0.5).abs()).fold(0.0f32, f32::max);
    assert!(worst < 0.02, "worst {worst}");
}

/// The image a request sent, as RGBA8, and the colour at document (`x`, `y`) given the area's
/// top-left corner.
fn sent_rgb(sent: &EditRequest, origin: (i32, i32), x: i32, y: i32) -> [u8; 3] {
    let img = codecs::decode(&sent.image).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
    let i = ((y - origin.1) as usize * img.width() as usize + (x - origin.0) as usize) * 4;
    [img.data()[i], img.data()[i + 1], img.data()[i + 2]]
}

#[test]
fn edit_mode_sends_the_selected_pixels_as_they_are() {
    // The same red square as `the_selected_pixels_are_never_sent`: in Edit mode the provider sees
    // it, since it is to change what is there.
    let mut s = session(120, 120);
    s.execute("select.rect", json!({"x": 40, "y": 40, "width": 40, "height": 40})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let fake = Fake::new([0, 0, 255]);
    s.set_image_provider(Some(fake.clone()));
    let r = s.execute(FILL, json!({"prompt": "make it blue", "mode": "edit", "variations": 1})).unwrap();
    let origin = (r["bounds"][0].as_i64().unwrap() as i32, r["bounds"][1].as_i64().unwrap() as i32);
    let sent = fake.sent.lock().unwrap();
    assert_eq!(sent[0].mode, EditMode::Edit);
    assert_eq!(sent[0].prompt, "make it blue");
    for (x, y) in [(60, 60), (41, 41), (79, 60)] {
        assert_eq!(sent_rgb(&sent[0], origin, x, y), [255, 0, 0], "({x},{y})");
    }
    assert_eq!(sent_rgb(&sent[0], origin, 30, 30), [255, 255, 255]);
    let mask = codecs::decode(&sent[0].mask).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
    let at = |x: i32, y: i32| mask.data()[((y - origin.1) as usize * mask.width() as usize + (x - origin.0) as usize) * 4];
    assert_eq!((at(60, 60), at(30, 30)), (255, 0), "the mask is the same as in Fill");
    drop(sent);
    // The result lands like a fill's: masked by the selection, one undo step.
    assert_eq!(rgb_at(&s, 60, 60), [0, 0, 255]);
    assert_eq!(rgb_at(&s, 30, 30), [255, 255, 255]);
    assert_eq!(doc(&s).layers.last().unwrap().name, "make it blue");
}

#[test]
fn fill_stays_the_default_and_edit_needs_a_prompt() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let fake = Fake::new([0, 0, 0]);
    s.set_image_provider(Some(fake.clone()));
    s.execute(FILL, json!({"variations": 1})).unwrap();
    s.execute(FILL, json!({"variations": 1, "mode": "fill"})).unwrap();
    {
        let sent = fake.sent.lock().unwrap();
        assert!(sent.iter().all(|r| r.mode == EditMode::Fill && r.references.is_empty()));
    }
    for p in [json!({"mode": "edit"}), json!({"mode": "edit", "prompt": "   "})] {
        let e = s.execute(FILL, p.clone()).unwrap_err().to_string();
        assert!(e.contains("prompt"), "{p}: {e}");
    }
    for bad in [json!("Edit"), json!(1), json!(true), json!(["edit"]), json!("")] {
        assert!(s.execute(FILL, json!({"prompt": "x", "mode": bad})).is_err(), "{bad}");
    }
    assert_eq!(fake.sent.lock().unwrap().len(), 2, "nothing sent for bad params");
}

/// A provider that fills but neither edits in place nor takes references.
#[test]
fn a_provider_without_edit_or_references_says_so() {
    struct Plain(Arc<Fake>);
    impl ImageProvider for Plain {
        fn info(&self) -> ProviderInfo {
            self.0.info()
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities { generate: false, edit: true, variations: false, edit_in_place: false, max_references: 0 }
        }
        fn edit(&self, r: &EditRequest) -> photocraft_genai::Result<GeneratedImage> {
            self.0.edit(r)
        }
    }
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let fake = Fake::new([0, 0, 0]);
    s.set_image_provider(Some(Arc::new(Plain(fake.clone()))));
    let e = s.execute(FILL, json!({"prompt": "x", "mode": "edit"})).unwrap_err().to_string();
    assert!(e.contains("can't edit"), "{e}");
    let id = doc(&s).id.0;
    let e = s.execute(FILL, json!({"prompt": "x", "references": [{"document": id}]})).unwrap_err().to_string();
    assert!(e.contains("can't use reference images"), "{e}");
    s.execute(FILL, json!({"prompt": "x", "variations": 1, "references": []})).unwrap();
    assert_eq!(fake.sent.lock().unwrap().len(), 1);
}

/// A unique scratch folder under the system temp dir.
fn scratch(name: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("photocraft-genai-{name}-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn reference_images_come_from_files_and_open_documents() {
    // A 3000×1500 green picture on disk and an open 200×400 red document as references.
    let dir = scratch("refs");
    let path = dir.join("green.png");
    let px: Vec<u8> = (0..3000 * 1500).flat_map(|_| [0u8, 200, 0, 255]).collect();
    let png = codecs::encode(&Image::from_u8(3000, 1500, ChannelLayout::Rgba, px).unwrap(), Format::Png, &Default::default()).unwrap();
    std::fs::write(&path, png).unwrap();
    let mut s = session(64, 64);
    s.execute("file.new", json!({"width": 200, "height": 400, "background": "#ff0000"})).unwrap();
    let red = doc(&s).id.0;
    s.execute("file.new", json!({"width": 64, "height": 64, "background": "#ffffff"})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let fake = Fake::new([0, 0, 255]);
    s.set_image_provider(Some(fake.clone()));
    let refs = json!([path.to_string_lossy(), {"document": red}]);
    let r = s.execute(FILL, json!({"prompt": "this bottle", "mode": "edit", "references": refs, "variations": 2})).unwrap();
    {
        let sent = fake.sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        let refs = &sent[0].references;
        assert_eq!(refs.len(), 2);
        assert_eq!(sent[1].references, *refs, "every variation gets them");
        let look = |r: &ReferenceImage| {
            assert_eq!(r.mime, "image/jpeg");
            let img = codecs::decode(&r.bytes).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
            let mid = ((img.height() / 2 * img.width() + img.width() / 2) * 4) as usize;
            ((img.width(), img.height()), [img.data()[mid], img.data()[mid + 1], img.data()[mid + 2]])
        };
        let (size, [r0, g0, b0]) = look(&refs[0]);
        assert_eq!(size, (1024, 512), "scaled down to 1024 on the long side");
        assert!(r0 < 10 && (190..=210).contains(&g0) && b0 < 10, "{r0} {g0} {b0}");
        let (size, [r1, g1, b1]) = look(&refs[1]);
        assert_eq!(size, (200, 400), "small enough to go as it is");
        assert!(r1 > 245 && g1 < 10 && b1 < 10, "{r1} {g1} {b1}");
    }
    // Generate More sends the same references again.
    let group = r["layer"].as_u64().unwrap();
    s.execute(MORE, json!({"layer": group, "count": 1})).unwrap();
    let sent = fake.sent.lock().unwrap();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[2].references, sent[0].references);
    assert_eq!(sent[2].mode, EditMode::Edit);
    drop(sent);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_references_fail_with_a_reason_and_send_nothing() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let fake = Fake::new([0, 0, 0]);
    s.set_image_provider(Some(fake.clone()));
    let id = doc(&s).id.0;
    let missing = scratch("missing").join("nope.png");
    let not_image = scratch("junk").join("junk.png");
    std::fs::write(&not_image, b"not an image").unwrap();
    let cases = [
        json!("x"),
        json!({"document": id}),
        json!([1]),
        json!([""]),
        json!([{"document": "1"}]),
        json!([{"document": -1}]),
        json!([{"document": 999_999_999}]),
        json!([{"path": 5}]),
        json!([{"path": "a", "document": id}]),
        json!([{}]),
        json!([null]),
        json!([[]]),
        json!([{"document": id}, {"document": id}, {"document": id}, {"document": id}, {"document": id}]),
        json!([missing.to_string_lossy()]),
        json!([not_image.to_string_lossy()]),
    ];
    for refs in cases {
        assert!(s.execute(FILL, json!({"prompt": "x", "references": refs})).is_err(), "{refs}");
    }
    let e = s.execute(FILL, json!({"prompt": "x", "references": [missing.to_string_lossy()]})).unwrap_err().to_string();
    assert!(e.contains("nope.png"), "names the file: {e}");
    assert!(fake.sent.lock().unwrap().is_empty());
    // Four is the most, and the active document itself may be one.
    s.execute(FILL, json!({"prompt": "x", "variations": 1, "references": [{"document": id}, {"document": id}, {"document": id}, {"document": id}]})).unwrap();
    assert_eq!(fake.sent.lock().unwrap()[0].references.len(), 4);
}

// ---- Fills that run side by side --------------------------------------------------------------

fn slow(delay_ms: u64) -> Arc<Fake> {
    Arc::new(Fake { delay: std::time::Duration::from_millis(delay_ms), ..Arc::into_inner(Fake::new([255, 0, 0])).unwrap() })
}

fn started(s: &mut Session, id: &str, p: Value) -> crate::jobs::JobId {
    match s.start(id, p).unwrap() {
        crate::jobs::Started::Job(j) => j,
        other => panic!("{id} ran inline: {other:?}"),
    }
}

#[test]
fn fills_run_in_the_background_without_locking_and_land_above_their_layer() {
    let mut s = session(200, 100);
    let background = s.active().unwrap().active_layer;
    s.set_image_provider(Some(slow(300)));
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    let history = s.active().unwrap().history.past_len();
    let first = started(&mut s, FILL, json!({"prompt": "a", "variations": 1}));
    let doc_id = doc(&s).id;
    assert!(s.job_on(doc_id).is_none(), "the document isn't locked");
    // The user goes on: a new selection and a second fill while the first generates.
    s.execute("select.rect", json!({"x": 120, "y": 40, "width": 40, "height": 40})).unwrap();
    let second = started(&mut s, FILL, json!({"prompt": "b", "variations": 1}));
    let fills = s.generative_fills();
    assert_eq!(fills.len(), 2);
    assert_eq!(fills[0].name, "a");
    assert_eq!(fills[0].bounds, [10, 10, 40, 40]);
    assert_eq!(fills[1].bounds, [120, 40, 160, 80]);
    assert!(fills.iter().all(|f| f.document == doc_id && f.anchor == background && f.state != "failed"));
    assert_eq!(s.generative_fill_mask(fills[1].id).unwrap().content_bounds(), photocraft_geom::Rect::new(120, 40, 160, 80));
    let a = s.wait_job(first).unwrap();
    let b = s.wait_job(second).unwrap();
    assert!(s.generative_fills().is_empty());
    // Both landed, each masked by its own selection, and the active layer is still the one
    // the user was on.
    assert_eq!(s.active().unwrap().active_layer, background);
    assert_eq!(rgb_at(&s, 20, 20), [255, 0, 0]);
    assert_eq!(rgb_at(&s, 130, 50), [255, 0, 0]);
    assert_eq!(rgb_at(&s, 80, 50), [255, 255, 255]);
    // Each lands right above the layer that was active, as a new layer would: the later one
    // under the earlier.
    let names: Vec<_> = doc(&s).layers.iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, ["Background", "b", "a"]);
    assert_ne!(a["layer"], b["layer"]);
    // The second selection plus one step per fill.
    assert_eq!(s.active().unwrap().history.past_len(), history + 3);
    assert!(s.undo());
    assert_eq!(doc(&s).layers.len(), 2, "each fill is its own undo step");
}

#[test]
fn a_result_lands_on_its_own_document_wherever_the_user_is() {
    let mut s = session(64, 64);
    s.set_image_provider(Some(slow(200)));
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let job = started(&mut s, FILL, json!({"prompt": "x", "variations": 1}));
    let first = doc(&s).id;
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "#000000"})).unwrap();
    let viewing = s.active_index();
    s.wait_job(job).unwrap();
    assert_eq!(s.active_index(), viewing, "the view stays on the other document");
    assert_eq!(doc(&s).layers.len(), 1);
    let filled = s.documents().iter().find(|st| st.doc.id == first).unwrap();
    assert_eq!(filled.doc.layers.len(), 2);
}

#[test]
fn at_most_five_fills_talk_to_the_provider_and_the_rest_wait_their_turn() {
    let mut s = session(400, 100);
    let fake = slow(300);
    s.set_image_provider(Some(fake.clone()));
    let mut jobs = Vec::new();
    for i in 0..MAX_PARALLEL + 1 {
        s.execute("select.rect", json!({"x": i * 60 + 5, "y": 10, "width": 20, "height": 20})).unwrap();
        jobs.push(started(&mut s, FILL, json!({"prompt": format!("p{i}"), "variations": 1})));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let queued = loop {
        let fills = s.generative_fills();
        if let Some(f) = fills.iter().find(|f| f.state == "queued") {
            break f.clone();
        }
        assert!(std::time::Instant::now() < deadline, "none waited: {fills:?}");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert_eq!(queued.queue_position, 1);
    for j in jobs {
        s.wait_job(j).unwrap();
    }
    assert_eq!(fake.peak.load(Ordering::SeqCst), MAX_PARALLEL);
    assert_eq!(doc(&s).layers.len(), 1 + MAX_PARALLEL + 1);
}

#[test]
fn a_failed_fill_waits_for_retry_or_discard() {
    let mut s = session(64, 64);
    s.set_image_provider(Some(Arc::new(Fake { fail: Some(GenError::Network("offline".into())), ..Arc::into_inner(Fake::new([0, 0, 0])).unwrap() })));
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let job = started(&mut s, FILL, json!({"prompt": "x", "variations": 1}));
    assert!(s.wait_job(job).unwrap_err().to_string().contains("offline"));
    let fills = s.generative_fills();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].state, "failed");
    assert!(fills[0].error.as_deref().unwrap().contains("offline"));
    let id = fills[0].id;
    // The selection moves on; Retry still sends the area it was started with.
    s.execute("select.deselect", json!({})).unwrap();
    s.set_image_provider(Some(Fake::new([0, 255, 0])));
    let again = started(&mut s, RETRY, json!({"fill": id}));
    assert_eq!(s.generative_fills()[0].state, "running");
    s.wait_job(again).unwrap();
    assert!(s.generative_fills().is_empty());
    assert_eq!(rgb_at(&s, 16, 16), [0, 255, 0]);
    // Bad ids and fills that haven't failed are refused.
    for bad in [json!({}), json!({"fill": "1"}), json!({"fill": 999})] {
        assert!(s.execute(RETRY, bad.clone()).is_err(), "{bad}");
        assert!(s.execute(DISCARD, bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn discarding_a_fill_stops_it_and_its_result_never_lands() {
    let mut s = session(64, 64);
    s.set_image_provider(Some(slow(300)));
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let job = started(&mut s, FILL, json!({"prompt": "x", "variations": 1}));
    let id = s.generative_fills()[0].id;
    s.execute(DISCARD, json!({"fill": id})).unwrap();
    assert!(s.generative_fills().is_empty());
    assert!(s.job(job).is_none());
    s.join_cancelled_jobs();
    s.poll_jobs();
    assert_eq!(doc(&s).layers.len(), 1);
    // A failed one is just forgotten.
    s.set_image_provider(Some(Arc::new(Fake { fail: Some(GenError::Auth), ..Arc::into_inner(Fake::new([0, 0, 0])).unwrap() })));
    let job = started(&mut s, FILL, json!({"prompt": "x", "variations": 1}));
    let _ = s.wait_job(job);
    let id = s.generative_fills()[0].id;
    assert!(s.execute(RETRY, json!({"fill": id + 1})).is_err());
    s.execute(DISCARD, json!({"fill": id})).unwrap();
    assert!(s.execute(FILLS, json!({})).unwrap()["fills"].as_array().unwrap().is_empty());
}

#[test]
fn a_result_is_dropped_when_the_canvas_changed_size_or_its_document_closed() {
    let mut s = session(64, 64);
    s.set_image_provider(Some(slow(300)));
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let job = started(&mut s, FILL, json!({"prompt": "x", "variations": 1}));
    s.execute("image.canvasSize", json!({"width": 80, "height": 80})).unwrap();
    assert!(s.wait_job(job).unwrap_err().to_string().contains("canvas"));
    assert!(s.generative_fills().is_empty(), "not something Retry could place");
    assert!(doc(&s).layers.iter().all(|l| !l.is_group()));

    let job = started(&mut s, FILL, json!({"prompt": "x", "variations": 1}));
    let closing = s.active_index().unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.close(closing).unwrap();
    s.tidy_generative_fills();
    assert!(s.generative_fills().is_empty());
    assert!(s.job(job).is_none());
}

#[test]
fn a_busy_service_is_asked_again() {
    let mut s = session(64, 64);
    let fake = Arc::new(Fake { busy: 2, ..Arc::into_inner(Fake::new([0, 0, 255])).unwrap() });
    s.set_image_provider(Some(fake.clone()));
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    s.execute(FILL, json!({"prompt": "x", "variations": 1})).unwrap();
    assert_eq!(fake.calls.load(Ordering::SeqCst), 3);
    assert_eq!(rgb_at(&s, 16, 16), [0, 0, 255]);
    // Past the pauses the error stands.
    let fake = Arc::new(Fake { busy: 100, ..Arc::into_inner(Fake::new([0, 0, 255])).unwrap() });
    s.set_image_provider(Some(fake.clone()));
    let e = s.execute(FILL, json!({"prompt": "x", "variations": 1})).unwrap_err().to_string();
    assert!(e.contains("rate limit"), "{e}");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 4);
}

#[test]
fn each_separate_area_can_be_filled_on_its_own() {
    let mut s = session(200, 100);
    s.set_image_provider(Some(Fake::new([255, 0, 0])));
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("select.rect", json!({"x": 120, "y": 50, "width": 40, "height": 40, "mode": "add"})).unwrap();
    let areas = s.execute(AREAS, json!({})).unwrap();
    assert_eq!(areas["areas"], json!([[10, 10, 40, 40], [120, 50, 160, 90]]));
    let r = s.execute(FILL, json!({"prompt": "x", "variations": 1, "area": 1})).unwrap();
    let mask = &group(&s, r["layer"].as_u64().unwrap()).mask.as_ref().unwrap().surface;
    assert_eq!(mask.content_bounds(), photocraft_geom::Rect::new(120, 50, 160, 90), "masked by that area alone");
    assert_eq!(rgb_at(&s, 20, 20), [255, 255, 255]);
    assert_eq!(rgb_at(&s, 130, 60), [255, 0, 0]);
    for bad in [json!(2), json!(-1), json!("0")] {
        assert!(s.execute(FILL, json!({"prompt": "x", "area": bad})).is_err(), "{bad}");
    }
    s.execute("select.deselect", json!({})).unwrap();
    assert!(s.execute(AREAS, json!({})).is_err(), "needs a selection");
}
