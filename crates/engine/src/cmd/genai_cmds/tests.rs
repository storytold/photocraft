use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image, SampleType as CodecSample};
use photocraft_doc::Document;
use photocraft_geom::Rect;

use super::pixels::{Reference, context_area, match_surroundings, nearest_ratio, request_shape, surroundings_fill};
use super::*;
use crate::genai::{FillJob, GenerativeAi, JobPoll, SentInput, SentItem, SentKind, ServiceKind, SetupView};

/// A stand-in for the shared AI layer: answers every job with a solid colour at the request's
/// size (or `size` when set), and keeps what it was sent. With `vary` each job's colour differs;
/// with `flaky` every second job fails; with `busy` the first that many starts answer that the
/// service is over its rate limit; with `remote` sending needs the person's agreement until a
/// remembered one is given.
struct Fake {
    color: [u8; 3],
    vary: bool,
    flaky: bool,
    /// How long each job takes.
    delay: std::time::Duration,
    /// Answer with the picture sent, the masked placeholder replaced by the average colour around
    /// it as a model would, every channel moved by this much (a model's colour drift).
    echo: Option<i16>,
    size: Option<(u32, u32)>,
    fail: Option<AiFailure>,
    jpeg: bool,
    busy: usize,
    remote: bool,
    status: Mutex<AiStatus>,
    /// A remembered agreement is on file.
    remembered: AtomicBool,
    agreed: Mutex<Vec<ConsentTerm>>,
    /// Starts so far (each one a request sent), and the jobs asked to stop.
    calls: AtomicUsize,
    cancels: AtomicUsize,
    plans: AtomicUsize,
    /// Jobs in progress, and the most there were at once.
    open: AtomicUsize,
    peak: AtomicUsize,
    sent: Mutex<Vec<FillRequest>>,
}

fn fake(color: [u8; 3]) -> Fake {
    Fake {
        color,
        vary: false,
        flaky: false,
        delay: std::time::Duration::ZERO,
        echo: None,
        size: None,
        fail: None,
        jpeg: false,
        busy: 0,
        remote: false,
        status: Mutex::new(AiStatus::Ready { model: "Fake 1".into(), connection: "Fake".into(), local: true }),
        remembered: AtomicBool::new(false),
        agreed: Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
        cancels: AtomicUsize::new(0),
        plans: AtomicUsize::new(0),
        open: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        sent: Mutex::new(Vec::new()),
    }
}

/// `f` as the session's generative AI; the returned handle shows what it was sent.
fn install(s: &mut Session, f: Fake) -> Arc<Fake> {
    let f = Arc::new(f);
    s.set_generative_ai(Some(Arc::new(FakeAi(Arc::clone(&f)))));
    f
}

struct FakeAi(Arc<Fake>);

struct FakePlan {
    fake: Arc<Fake>,
    request: FillRequest,
    disclosure: Disclosure,
    /// "This time" agreed on this plan.
    agreed: AtomicBool,
}

struct FakeJob {
    fake: Arc<Fake>,
    result: std::result::Result<Vec<u8>, AiFailure>,
    ready: std::time::Instant,
    cancelled: bool,
    open: bool,
}

impl FakeJob {
    fn close(&mut self) {
        if std::mem::take(&mut self.open) {
            self.fake.open.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Drop for FakeJob {
    fn drop(&mut self) {
        self.close();
    }
}

impl FillJob for FakeJob {
    fn poll(&mut self) -> std::result::Result<JobPoll, AiFailure> {
        if self.cancelled {
            self.close();
            return Ok(JobPoll::Cancelled);
        }
        if std::time::Instant::now() < self.ready {
            return Ok(JobPoll::Working(Some(50)));
        }
        self.close();
        Ok(match &self.result {
            Ok(img) => JobPoll::Done(vec![img.clone()]),
            Err(f) => JobPoll::Failed(f.clone()),
        })
    }
    fn cancel(&mut self) -> std::result::Result<(), AiFailure> {
        self.fake.cancels.fetch_add(1, Ordering::SeqCst);
        self.cancelled = true;
        Ok(())
    }
}

impl FillPlan for FakePlan {
    fn disclosure(&self) -> &Disclosure {
        &self.disclosure
    }
    fn needs_consent(&self) -> bool {
        self.fake.remote && !self.agreed.load(Ordering::SeqCst) && !self.fake.remembered.load(Ordering::SeqCst)
    }
    fn agree(&self, term: ConsentTerm) -> std::result::Result<(), AiFailure> {
        self.fake.agreed.lock().unwrap().push(term);
        if term == ConsentTerm::ThisTime {
            self.agreed.store(true, Ordering::SeqCst);
        } else {
            self.fake.remembered.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
    fn start(&self) -> std::result::Result<Box<dyn FillJob>, AiFailure> {
        if self.needs_consent() {
            return Err(AiFailure::new(AiErrorKind::ConsentRequired, "sending needs your agreement first"));
        }
        let f = &self.fake;
        let n = f.calls.fetch_add(1, Ordering::SeqCst);
        if n < f.busy {
            return Err(AiFailure { kind: AiErrorKind::RateLimited, message: "the service is over its rate limit".into(), retry_after_secs: None });
        }
        f.sent.lock().unwrap().push(self.request.clone());
        let open = f.open.fetch_add(1, Ordering::SeqCst) + 1;
        f.peak.fetch_max(open, Ordering::SeqCst);
        let result = match &f.fail {
            Some(e) => Err(e.clone()),
            None if f.flaky && n % 2 == 1 => Err(AiFailure::new(AiErrorKind::Provider, "flaky")),
            None => Ok(answer(f, &self.request, n)),
        };
        Ok(Box::new(FakeJob { fake: Arc::clone(f), result, ready: std::time::Instant::now() + f.delay, cancelled: false, open: true }))
    }
}

/// What the fake model returns for job `n`.
fn answer(f: &Fake, r: &FillRequest, n: usize) -> Vec<u8> {
    let img = codecs::decode(&r.source_png).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
    if let Some(d) = f.echo {
        let mask = codecs::decode(&r.mask_png).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
        let masked = |i: usize| mask.data()[i / 4 * 4] > 127;
        let (mut sum, mut count) = ([0u64; 3], 0u64);
        for (i, px) in img.data().as_chunks::<4>().0.iter().enumerate() {
            if !masked(i * 4) {
                (0..3).for_each(|c| sum[c] += u64::from(px[c]));
                count += 1;
            }
        }
        let around = sum.map(|v| (v / count.max(1)) as u8);
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
        return codecs::encode(&out, Format::Png, &Default::default()).unwrap();
    }
    let color = if f.vary { [f.color[0].wrapping_add(n as u8 * 60), f.color[1], f.color[2]] } else { f.color };
    let (w, h) = f.size.unwrap_or((img.width(), img.height()));
    let px: Vec<u8> = (0..w * h).flat_map(|_| [color[0], color[1], color[2], 255]).collect();
    let out = Image::from_u8(w, h, ChannelLayout::Rgba, px).unwrap();
    if f.jpeg {
        let rgb = out.convert(ChannelLayout::Rgb, CodecSample::U8);
        return codecs::encode(&rgb, Format::Jpeg, &Default::default()).unwrap();
    }
    codecs::encode(&out, Format::Png, &Default::default()).unwrap()
}

impl GenerativeAi for FakeAi {
    fn status(&self) -> AiStatus {
        self.0.status.lock().unwrap().clone()
    }
    fn plan(&self, request: FillRequest) -> std::result::Result<Arc<dyn FillPlan>, AiFailure> {
        self.0.plans.fetch_add(1, Ordering::SeqCst);
        let items = vec![
            SentItem { input: SentInput::Source, kind: SentKind::Image, bytes: Some(request.source_png.len() as u64) },
            SentItem { input: SentInput::Mask, kind: SentKind::Image, bytes: Some(request.mask_png.len() as u64) },
            SentItem { input: SentInput::Prompt, kind: SentKind::Text, bytes: Some(request.prompt.len() as u64) },
        ];
        let disclosure = Disclosure {
            destination: if self.0.remote { "https://fake.example".into() } else { "fake".into() },
            connection: "Fake".into(),
            model: "Fake 1".into(),
            local: !self.0.remote,
            items,
        };
        Ok(Arc::new(FakePlan { fake: Arc::clone(&self.0), request, disclosure, agreed: AtomicBool::new(false) }))
    }
    fn setup(&self) -> std::result::Result<SetupView, AiFailure> {
        Ok(SetupView::default())
    }
    fn connect(&self, _: ServiceKind, _: &str) -> std::result::Result<(), AiFailure> {
        Ok(())
    }
    fn disconnect(&self, _: &str) -> std::result::Result<(), AiFailure> {
        Ok(())
    }
    fn choose(&self, _: &str, _: &str) -> std::result::Result<(), AiFailure> {
        Ok(())
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
fn needs_generative_ai_and_a_selection() {
    let mut s = session(64, 64);
    assert!(!s.is_enabled(FILL), "no generative AI: off");
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    assert!(!s.is_enabled(FILL), "still none");
    assert_eq!(s.disabled_reason(FILL).as_deref(), Some(NO_AI));
    assert!(s.execute(FILL, json!({"prompt": "a cat"})).is_err());
    let f = install(&mut s, fake([255, 0, 0]));
    assert!(s.is_enabled(FILL));
    s.execute("select.deselect", json!({})).unwrap();
    assert!(!s.is_enabled(FILL), "no selection: off");
    assert!(f.sent.lock().unwrap().is_empty());
}

#[test]
fn unavailable_greys_the_tool_out_with_the_layers_reason_and_sends_nothing() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let f = install(&mut s, fake([255, 0, 0]));
    let reason = "Generative fill has no model chosen. Open Set up AI to choose one.";
    *f.status.lock().unwrap() = AiStatus::Unavailable { reason: reason.into() };
    assert!(!s.is_enabled(FILL));
    // The reason as the shared layer wrote it.
    assert_eq!(s.disabled_reason(FILL).as_deref(), Some(reason));
    assert!(s.execute(FILL, json!({"prompt": "x"})).is_err());
    assert_eq!(s.generative_status(), AiStatus::Unavailable { reason: reason.into() });
    assert_eq!(f.plans.load(Ordering::SeqCst), 0, "nothing was even planned");
    // It's asked again each time: once ready, the tool is on.
    *f.status.lock().unwrap() = AiStatus::Ready { model: "M".into(), connection: "C".into(), local: false };
    assert!(s.is_enabled(FILL));
    assert_eq!(s.execute(FILLS, json!({})).unwrap()["status"]["state"], "ready");
}

#[test]
fn the_results_land_in_a_new_group_masked_by_the_selection_and_undo_in_one_step() {
    let mut s = session(200, 100);
    s.execute("select.rect", json!({"x": 80, "y": 30, "width": 40, "height": 40})).unwrap();
    let f = install(&mut s, fake([255, 0, 0]));
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

    // What was sent: the prompt, the picture and a mask transparent (and white) only over the
    // selection, opaque black elsewhere, the size of the picture.
    let sent = f.sent.lock().unwrap();
    assert_eq!(sent.len(), 3, "three variations by default, each its own request");
    assert_eq!(f.plans.load(Ordering::SeqCst), 1, "planned once");
    assert_eq!(sent[0].prompt, "a red   balloon");
    let mask = codecs::decode(&sent[0].mask_png).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
    let img = codecs::decode(&sent[0].source_png).unwrap();
    assert_eq!((mask.width(), mask.height()), (img.width(), img.height()));
    let area = Rect::from_xywh(r["bounds"][0].as_i64().unwrap() as i32, r["bounds"][1].as_i64().unwrap() as i32, 1, 1);
    let at = |x: i32, y: i32| {
        let i = ((y - area.y0) as usize * mask.width() as usize + (x - area.x0) as usize) * 4;
        [mask.data()[i], mask.data()[i + 3]]
    };
    assert_eq!(at(100, 50), [255, 0], "white and transparent: fill here");
    assert_eq!(at(78, 50), [0, 255], "black and opaque: keep");
    drop(sent);

    assert!(s.undo());
    assert_eq!(doc(&s).layers.len(), before);
    assert_eq!(rgb_at(&s, 100, 50), [255, 255, 255]);
}

#[test]
fn an_empty_prompt_is_allowed_and_names_the_layer_generative_fill() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let f = install(&mut s, fake([0, 0, 255]));
    let r = s.execute(FILL, json!({})).unwrap();
    let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(doc(&s).layer(id).unwrap().name, "Generative Fill");
    assert_eq!(f.sent.lock().unwrap()[0].prompt, CONTINUE_PROMPT, "models need words: continue the picture");
    assert_eq!(s.generative_prompt(id), Some(""));
    let long = "word ".repeat(30);
    s.execute(FILL, json!({"prompt": long})).unwrap();
    assert!(doc(&s).layers.last().unwrap().name.ends_with('…'));
}

#[test]
fn a_result_of_another_size_is_scaled_to_the_area() {
    let mut s = session(300, 300);
    s.execute("select.rect", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    install(&mut s, Fake { size: Some((1024, 1024)), ..fake([0, 255, 0]) });
    s.execute(FILL, json!({"prompt": "grass"})).unwrap();
    assert_eq!(rgb_at(&s, 150, 150), [0, 255, 0]);
    assert_eq!(rgb_at(&s, 101, 101), [0, 255, 0]);
}

#[test]
fn a_jpeg_result_is_read_too() {
    let mut s = session(120, 120);
    s.execute("select.rect", json!({"x": 30, "y": 30, "width": 60, "height": 60})).unwrap();
    install(&mut s, Fake { jpeg: true, ..fake([0, 0, 255]) });
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
    let auth = AiFailure::new(AiErrorKind::Auth, "OpenAI refused the API key. Check it in Set up AI.");
    install(&mut s, Fake { fail: Some(auth), ..fake([0, 0, 0]) });
    let e = s.execute(FILL, json!({"prompt": "x"})).unwrap_err().to_string();
    assert!(e.contains("refused the API key"), "the layer's message: {e}");
    assert_eq!(s.active().unwrap().history.past_len(), history);
    assert!(s.execute(FILL, json!({"prompt": 5})).is_err());
    assert!(s.execute(FILL, json!({"prompt": "x".repeat(MAX_PROMPT + 1)})).is_err());
}

#[test]
fn the_document_keeps_its_mode_and_depth() {
    for (mode, depth) in [("rgb", 16), ("grayscale", 8), ("cmyk", 8), ("rgb", 32)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64, "mode": mode, "depth": depth, "background": "#ffffff"})).unwrap();
        s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
        install(&mut s, fake([128, 128, 128]));
        let r = s.execute(FILL, json!({"prompt": "x"})).unwrap();
        let d = doc(&s);
        let l = d.layer(photocraft_doc::LayerId(r["variations"][0].as_u64().unwrap())).unwrap();
        let f = l.surface().unwrap().format();
        assert_eq!((f.mode, f.sample), (d.mode, d.depth), "{mode} {depth}");
    }
}

#[test]
fn the_context_area_grows_toward_a_common_shape_inside_the_canvas() {
    let canvas = Rect::from_xywh(0, 0, 1000, 600);
    let same = |w, h| (w, h);
    // A 30² selection gets the 64 px minimum margin on every side, a 100² one its own size.
    assert_eq!(context_area(Rect::from_xywh(400, 200, 30, 30), canvas, same), Rect::new(336, 136, 494, 294));
    assert_eq!(context_area(Rect::from_xywh(400, 200, 100, 100), canvas, same), Rect::new(300, 100, 600, 400));
    // At an edge the margin is cut off by the canvas.
    assert_eq!(context_area(Rect::from_xywh(0, 0, 100, 100), canvas, same), Rect::new(0, 0, 200, 200));
    // A 16:9 shape widens the area around its centre.
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
    // The shapes asked for: square, 3:2 or 2:3, whichever is nearest.
    assert_eq!(request_shape(500, 500), (1, 1));
    assert_eq!(request_shape(1920, 1080), (3, 2));
    assert_eq!(request_shape(400, 900), (2, 3));
    assert_eq!(request_shape(0, 0), (1, 1));
    assert_eq!(nearest_ratio(5, 5, &[]), None);
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
    let f = install(&mut s, Fake { vary: true, ..fake([0, 0, 255]) });
    let r = s.execute(FILL, json!({"prompt": "a boat", "variations": variations})).unwrap();
    (s, f, r)
}

#[test]
fn a_fill_makes_three_variations_with_the_first_showing() {
    let (s, f, r) = filled(None);
    let id = r["layer"].as_u64().unwrap();
    assert_eq!(r["variations"].as_array().unwrap().len(), 3);
    let names: Vec<_> = group(&s, id).children().unwrap().iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, ["Variation 1", "Variation 2", "Variation 3"]);
    assert_eq!(shown(&s, id), [true, false, false]);
    assert_eq!(rgb_at(&s, 50, 50), variation_rgb(&s, id, 0, 50, 50));
    assert_eq!(f.sent.lock().unwrap().len(), 3);
    assert_eq!(s.generative_layer(), Some(photocraft_doc::LayerId(id)));
    assert_eq!(s.generative_prompt(photocraft_doc::LayerId(id)), Some("a boat"));

    let (_, f, r) = filled(Some(1));
    assert_eq!(r["variations"].as_array().unwrap().len(), 1);
    assert_eq!(f.sent.lock().unwrap().len(), 1);
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    install(&mut s, fake([0, 0, 0]));
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
    install(&mut s, fake([0, 0, 0]));
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
    let (mut s, f, r) = filled(None);
    let id = r["layer"].as_u64().unwrap();
    s.execute(SELECT_VARIATION, json!({"index": 1})).unwrap();
    // A later edit to the canvas doesn't change what is sent: the same picture, mask and prompt.
    s.execute("select.deselect", json!({})).unwrap();
    let history = s.active().unwrap().history.past_len();
    let more = s.execute(MORE, json!({"count": 2})).unwrap();
    assert_eq!(more["variations"].as_array().unwrap().len(), 2);
    let names: Vec<_> = group(&s, id).children().unwrap().iter().map(|l| l.name.clone()).collect();
    assert_eq!(names, ["Variation 1", "Variation 2", "Variation 3", "Variation 4", "Variation 5"]);
    assert_eq!(shown(&s, id), [false, false, false, true, false], "the first new one shows");
    let sent = f.sent.lock().unwrap();
    assert_eq!(sent.len(), 5);
    assert_eq!(sent[4], sent[0]);
    assert_eq!(sent[4].prompt, "a boat");
    drop(sent);
    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
    assert!(s.undo());
    assert_eq!(shown(&s, id), [false, true, false]);
    assert!(s.execute(MORE, json!({"count": 9})).is_err());
    // When the tool can't run there is nothing to ask.
    *f.status.lock().unwrap() = AiStatus::Unavailable { reason: "off".into() };
    assert!(!s.is_enabled(MORE));
    s.set_generative_ai(None);
    assert!(!s.is_enabled(MORE));
    assert!(s.is_enabled(SELECT_VARIATION), "flipping between results needs no AI");
}

#[test]
fn a_failed_variation_is_dropped_when_others_succeed() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    install(&mut s, Fake { flaky: true, ..fake([0, 200, 0]) });
    let r = s.execute(FILL, json!({"prompt": "x", "variations": 3})).unwrap();
    assert_eq!(r["variations"].as_array().unwrap().len(), 2);
    assert_eq!(rgb_at(&s, 16, 16), [0, 200, 0]);
}

#[test]
fn the_selected_pixels_are_never_sent() {
    // A red square under the selection on white: the model sees the white around it carried
    // in, dimmed (no red), and the surroundings as they are.
    let mut s = session(120, 120);
    s.execute("select.rect", json!({"x": 40, "y": 40, "width": 40, "height": 40})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let f = install(&mut s, fake([0, 0, 255]));
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    let sent = f.sent.lock().unwrap();
    let img = codecs::decode(&sent[0].source_png).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
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
    let f = install(&mut s, Fake { delay, ..fake([0, 0, 0]) });
    let start = std::time::Instant::now();
    let r = s.execute(FILL, json!({"variations": 4})).unwrap();
    assert_eq!(r["variations"].as_array().unwrap().len(), 4);
    assert_eq!(f.peak.load(Ordering::SeqCst), 4, "all four jobs were running together");
    // One after another would take 1.6 s.
    assert!(start.elapsed() < delay * 2, "{:?}", start.elapsed());
}

#[test]
fn a_models_colour_drift_is_taken_out() {
    // The fake redraws the picture 40 levels darker everywhere. Outside the selection that shows
    // it drifted, so the fill (white, from around the selection) comes back white, not 215.
    let mut s = session(160, 160);
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap();
    install(&mut s, Fake { echo: Some(-40), ..fake([0, 0, 0]) });
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

// ---- Consent ----------------------------------------------------------------------------------

#[test]
fn a_remote_model_waits_for_consent_and_sends_nothing_until_then() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let f = install(&mut s, Fake { remote: true, ..fake([0, 0, 255]) });
    let history = s.active().unwrap().history.past_len();
    let e = s.execute(FILL, json!({"prompt": "x", "variations": 2})).unwrap_err().to_string();
    assert!(e.contains("agreement"), "{e}");
    assert!(f.sent.lock().unwrap().is_empty(), "nothing is sent without agreement");
    let fills = s.generative_fills();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].state, "consent");
    assert_eq!(fills[0].error, None, "waiting is not an error");
    let id = fills[0].id;
    // What the prompt shows: where it goes and what is sent, with kinds and sizes.
    let d = s.generative_fill_disclosure(id).unwrap();
    assert_eq!(d.destination, "https://fake.example");
    assert!(!d.local);
    assert_eq!(
        d.items.iter().map(|i| (i.input, i.kind)).collect::<Vec<_>>(),
        [(SentInput::Source, SentKind::Image), (SentInput::Mask, SentKind::Image), (SentInput::Prompt, SentKind::Text)]
    );
    assert!(d.items[0].bytes.unwrap() > 0);
    // Agreeing this time sends it, with the plan the person saw (not planned again).
    for bad in [json!({"fill": id}), json!({"fill": id, "term": "forever"}), json!({"fill": id + 1, "term": "this_time"})] {
        assert!(s.execute(AGREE, bad.clone()).is_err(), "{bad}");
    }
    let r = s.execute(AGREE, json!({"fill": id, "term": "this_time"})).unwrap();
    assert_eq!(r["term"], "this_time");
    assert!(r["layer"].is_u64(), "it landed: {r}");
    assert_eq!(f.sent.lock().unwrap().len(), 2);
    assert_eq!(f.plans.load(Ordering::SeqCst), 1);
    assert!(s.generative_fills().is_empty());
    assert_eq!(rgb_at(&s, 16, 16), [0, 0, 255]);
    assert_eq!(s.active().unwrap().history.past_len(), history + 1, "one undo step");
    // "This time" isn't remembered: the next fill asks again.
    assert!(s.execute(FILL, json!({"prompt": "y", "variations": 1})).is_err());
    let id = s.generative_fills()[0].id;
    // A remembered yes covers it and the ones after.
    s.execute(AGREE, json!({"fill": id, "term": "one_hour"})).unwrap();
    assert_eq!(*f.agreed.lock().unwrap(), [ConsentTerm::ThisTime, ConsentTerm::OneHour]);
    s.execute(FILL, json!({"prompt": "z", "variations": 1})).unwrap();
    assert_eq!(f.sent.lock().unwrap().len(), 4);
}

#[test]
fn refusing_consent_discards_the_fill_and_sends_nothing() {
    let mut s = session(64, 64);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let f = install(&mut s, Fake { remote: true, ..fake([0, 0, 255]) });
    let job = started(&mut s, FILL, json!({"prompt": "x", "variations": 1}));
    assert!(s.wait_job(job).is_err());
    let id = s.generative_fills()[0].id;
    assert_eq!(s.generative_fills()[0].state, "consent");
    s.execute(DISCARD, json!({"fills": [id]})).unwrap();
    assert!(s.generative_fills().is_empty());
    assert!(f.sent.lock().unwrap().is_empty());
    assert_eq!(doc(&s).layers.len(), 1);
    assert!(f.agreed.lock().unwrap().is_empty());
}

#[test]
fn a_remembered_yes_covers_the_next_fills_planned() {
    let mut s = session(200, 100);
    let f = install(&mut s, Fake { remote: true, ..fake([255, 0, 0]) });
    let mut jobs = Vec::new();
    for x in [10, 120] {
        s.execute("select.rect", json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap();
        jobs.push(started(&mut s, FILL, json!({"prompt": "x", "variations": 1})));
    }
    for j in jobs {
        assert!(s.wait_job(j).is_err());
    }
    let ids: Vec<u64> = s.generative_fills().iter().map(|f| f.id).collect();
    assert_eq!(ids.len(), 2);
    assert!(s.generative_fills().iter().all(|f| f.state == "consent"));
    // The background start of each agreement is its own job.
    let first = started(&mut s, AGREE, json!({"fill": ids[0], "term": "until_revoked"}));
    s.wait_job(first).unwrap();
    // The other one's plan predates the yes, so it still asks; Retry plans again and goes.
    let second = started(&mut s, RETRY, json!({"fill": ids[1]}));
    s.wait_job(second).unwrap();
    assert_eq!(f.sent.lock().unwrap().len(), 2);
    assert_eq!(doc(&s).layers.len(), 3);
}

// ---- Fills that run side by side --------------------------------------------------------------

fn slow(delay_ms: u64) -> Fake {
    Fake { delay: std::time::Duration::from_millis(delay_ms), ..fake([255, 0, 0]) }
}

fn started(s: &mut Session, id: &str, p: Value) -> JobId {
    match s.start(id, p).unwrap() {
        crate::jobs::Started::Job(j) => j,
        other => panic!("{id} ran inline: {other:?}"),
    }
}

#[test]
fn fills_run_in_the_background_without_locking_and_land_above_their_layer() {
    let mut s = session(200, 100);
    let background = s.active().unwrap().active_layer;
    install(&mut s, slow(300));
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    let history = s.active().unwrap().history.past_len();
    let first = started(&mut s, FILL, json!({"prompt": "a", "variations": 1}));
    let doc_id = doc(&s).id;
    assert!(s.job_on(doc_id).is_none(), "the document isn't locked");
    // The person goes on: a new selection and a second fill while the first generates.
    s.execute("select.rect", json!({"x": 120, "y": 40, "width": 40, "height": 40})).unwrap();
    let second = started(&mut s, FILL, json!({"prompt": "b", "variations": 1}));
    let fills = s.generative_fills();
    assert_eq!(fills.len(), 2);
    assert_eq!(fills[0].name, "a");
    assert_eq!(fills[0].bounds, [10, 10, 40, 40]);
    assert_eq!(fills[1].bounds, [120, 40, 160, 80]);
    assert!(fills.iter().all(|f| f.document == doc_id && f.anchor == background && f.state != "failed" && !f.more));
    assert_eq!(s.generative_fill_mask(fills[1].id).unwrap().content_bounds(), photocraft_geom::Rect::new(120, 40, 160, 80));
    let a = s.wait_job(first).unwrap();
    let b = s.wait_job(second).unwrap();
    assert!(s.generative_fills().is_empty());
    // Both landed, each masked by its own selection, and the active layer is still the one
    // the person was on.
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
fn a_result_lands_on_its_own_document_wherever_the_person_is() {
    let mut s = session(64, 64);
    install(&mut s, slow(200));
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
fn at_most_five_fills_talk_to_the_service_and_the_rest_wait_their_turn() {
    let mut s = session(400, 100);
    let f = install(&mut s, slow(300));
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
    assert_eq!(f.peak.load(Ordering::SeqCst), MAX_PARALLEL);
    assert_eq!(doc(&s).layers.len(), 1 + MAX_PARALLEL + 1);
}

#[test]
fn a_failed_fill_waits_for_retry_or_discard_and_keeps_the_error_kind() {
    let mut s = session(64, 64);
    let offline = AiFailure::new(AiErrorKind::Network, "Could not reach the service: offline");
    install(&mut s, Fake { fail: Some(offline), ..fake([0, 0, 0]) });
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let job = started(&mut s, FILL, json!({"prompt": "x", "variations": 1}));
    assert!(s.wait_job(job).unwrap_err().to_string().contains("offline"));
    let fills = s.generative_fills();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].state, "failed");
    assert!(fills[0].error.as_deref().unwrap().contains("offline"));
    assert_eq!(fills[0].error_kind, Some(AiErrorKind::Network));
    let id = fills[0].id;
    // The selection moves on; Retry still sends the area it was started with, planned again
    // with the model chosen now.
    s.execute("select.deselect", json!({})).unwrap();
    let f = install(&mut s, fake([0, 255, 0]));
    let again = started(&mut s, RETRY, json!({"fill": id}));
    assert_eq!(s.generative_fills()[0].state, "running");
    s.wait_job(again).unwrap();
    assert!(s.generative_fills().is_empty());
    assert_eq!(rgb_at(&s, 16, 16), [0, 255, 0]);
    assert_eq!(f.plans.load(Ordering::SeqCst), 1);
    // Bad ids and fills that haven't failed are refused.
    for bad in [json!({}), json!({"fill": "1"}), json!({"fill": 999}), json!({"fills": []})] {
        assert!(s.execute(RETRY, bad.clone()).is_err(), "{bad}");
        assert!(s.execute(DISCARD, bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn discarding_a_fill_cancels_its_jobs_and_its_result_never_lands() {
    let mut s = session(64, 64);
    let f = install(&mut s, slow(2_000));
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let job = started(&mut s, FILL, json!({"prompt": "x", "variations": 2}));
    let id = s.generative_fills()[0].id;
    // Wait until its jobs are with the service.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while f.open.load(Ordering::SeqCst) < 2 {
        assert!(std::time::Instant::now() < deadline, "the jobs never started");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    s.execute(DISCARD, json!({"fill": id})).unwrap();
    assert!(s.generative_fills().is_empty());
    assert!(s.job(job).is_none());
    s.join_cancelled_jobs();
    s.poll_jobs();
    assert_eq!(f.cancels.load(Ordering::SeqCst), 2, "each of its jobs was asked to stop");
    assert_eq!(f.open.load(Ordering::SeqCst), 0);
    assert_eq!(doc(&s).layers.len(), 1);
    // A failed one is just forgotten.
    install(&mut s, Fake { fail: Some(AiFailure::new(AiErrorKind::Auth, "bad key")), ..fake([0, 0, 0]) });
    let job = started(&mut s, FILL, json!({"prompt": "x", "variations": 1}));
    let _ = s.wait_job(job);
    let fills = s.generative_fills();
    assert_eq!(fills[0].error_kind, Some(AiErrorKind::Auth));
    let id = fills[0].id;
    assert!(s.execute(RETRY, json!({"fill": id + 1})).is_err());
    s.execute(DISCARD, json!({"fill": id})).unwrap();
    assert!(s.execute(FILLS, json!({})).unwrap()["fills"].as_array().unwrap().is_empty());
}

#[test]
fn a_result_is_dropped_when_the_canvas_changed_size_or_its_document_closed() {
    let mut s = session(64, 64);
    install(&mut s, slow(300));
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
    let f = install(&mut s, Fake { busy: 2, ..fake([0, 0, 255]) });
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    s.execute(FILL, json!({"prompt": "x", "variations": 1})).unwrap();
    assert_eq!(f.calls.load(Ordering::SeqCst), 3);
    assert_eq!(rgb_at(&s, 16, 16), [0, 0, 255]);
    // Past the pauses the error stands.
    let f = install(&mut s, Fake { busy: 100, ..fake([0, 0, 255]) });
    let e = s.execute(FILL, json!({"prompt": "x", "variations": 1})).unwrap_err().to_string();
    assert!(e.contains("rate limit"), "{e}");
    assert_eq!(f.calls.load(Ordering::SeqCst), 4);
}

#[test]
fn each_separate_area_can_be_filled_on_its_own() {
    let mut s = session(200, 100);
    install(&mut s, fake([255, 0, 0]));
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

#[test]
fn consent_terms_use_the_shared_layers_ids() {
    let ids: Vec<&str> = ConsentTerm::ALL.iter().map(|t| t.id()).collect();
    assert_eq!(ids, ["this_time", "one_hour", "one_day", "until_revoked"]);
    assert!(ConsentTerm::ALL.iter().all(|t| ConsentTerm::from_id(t.id()) == Some(*t)));
    assert_eq!(ConsentTerm::from_id("forever"), None);
}

/// Review follow-ups (#10903): feathered edges, colour matching, framing, diagnostics.
mod review;
