use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use egui_kittest::kittest::Queryable;
use photocraft_engine::Session;
use photocraft_engine::genai::{
    AiErrorKind, AiFailure, ConnectionRow, Disclosure, FillJob, FillPlan, FillRequest, GenerativeAi, JobPoll, ModelRow, SentInput, SentItem, SentKind,
    ServiceKind, SetupView,
};

use super::*;
use crate::Services;
use crate::new_layer_ui::testing::set;

const SECRET: &str = "sk-ui-test-not-a-real-key";

/// A stand-in for the shared AI layer: fills with solid `color` at the size sent, after `delay`;
/// `remote` asks for consent until a remembered yes; `fail` fails every job.
struct Fake {
    status: Mutex<AiStatus>,
    color: [u8; 3],
    delay: std::time::Duration,
    remote: bool,
    fail: Option<AiFailure>,
    remembered: AtomicBool,
    sent: AtomicUsize,
    /// What `connect` was given (the key, for checking where it went).
    connected: Mutex<Vec<(ServiceKind, String)>>,
    chosen: Mutex<Option<(String, String)>>,
    /// How long reading the setup takes (a slow keychain).
    setup_delay: std::time::Duration,
}

fn fake() -> Fake {
    Fake {
        status: Mutex::new(AiStatus::Ready { model: "Fill One".into(), connection: "Fake".into(), local: false, billing: Default::default() }),
        color: [255, 0, 0],
        delay: std::time::Duration::ZERO,
        remote: false,
        fail: None,
        remembered: AtomicBool::new(false),
        sent: AtomicUsize::new(0),
        connected: Mutex::new(Vec::new()),
        chosen: Mutex::new(None),
        setup_delay: std::time::Duration::ZERO,
    }
}

struct Ai(Arc<Fake>);

struct Plan {
    fake: Arc<Fake>,
    request: FillRequest,
    disclosure: Disclosure,
    agreed: AtomicBool,
}

struct Job {
    result: Result<Vec<u8>, AiFailure>,
    ready: std::time::Instant,
    cancelled: bool,
}

impl FillJob for Job {
    fn poll(&mut self) -> Result<JobPoll, AiFailure> {
        if self.cancelled {
            return Ok(JobPoll::Cancelled);
        }
        if std::time::Instant::now() < self.ready {
            return Ok(JobPoll::Working(None));
        }
        Ok(match &self.result {
            Ok(png) => JobPoll::Done(vec![png.clone()]),
            Err(f) => JobPoll::Failed(f.clone()),
        })
    }
    fn cancel(&mut self) -> Result<(), AiFailure> {
        self.cancelled = true;
        Ok(())
    }
}

impl FillPlan for Plan {
    fn disclosure(&self) -> &Disclosure {
        &self.disclosure
    }
    fn needs_consent(&self) -> bool {
        self.fake.remote && !self.agreed.load(Ordering::SeqCst) && !self.fake.remembered.load(Ordering::SeqCst)
    }
    fn agree(&self, term: photocraft_engine::genai::ConsentTerm) -> Result<(), AiFailure> {
        if term == photocraft_engine::genai::ConsentTerm::ThisTime {
            self.agreed.store(true, Ordering::SeqCst);
        } else {
            self.fake.remembered.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
    fn start(&self) -> Result<Box<dyn FillJob>, AiFailure> {
        if self.needs_consent() {
            return Err(AiFailure::new(AiErrorKind::ConsentRequired, "needs agreement"));
        }
        self.fake.sent.fetch_add(1, Ordering::SeqCst);
        let img = photocraft_codecs::decode(&self.request.source_png).unwrap();
        let (w, h) = (img.width(), img.height());
        let c = self.fake.color;
        let px: Vec<u8> = (0..w * h).flat_map(|_| [c[0], c[1], c[2], 255]).collect();
        let out = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, px).unwrap();
        let png = photocraft_codecs::encode(&out, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let result = match &self.fake.fail {
            Some(f) => Err(f.clone()),
            None => Ok(png),
        };
        Ok(Box::new(Job { result, ready: std::time::Instant::now() + self.fake.delay, cancelled: false }))
    }
}

impl GenerativeAi for Ai {
    fn status(&self) -> AiStatus {
        self.0.status.lock().unwrap().clone()
    }
    fn plan(&self, request: FillRequest) -> Result<Arc<dyn FillPlan>, AiFailure> {
        let items = vec![
            SentItem { input: SentInput::Source, kind: SentKind::Image, bytes: Some(request.source_png.len() as u64) },
            SentItem { input: SentInput::Mask, kind: SentKind::Image, bytes: Some(request.mask_png.len() as u64) },
            SentItem { input: SentInput::Prompt, kind: SentKind::Text, bytes: Some(request.prompt.len() as u64) },
        ];
        let disclosure = Disclosure { destination: "https://fake.example".into(), connection: "Fake".into(), model: "Fill One".into(), local: false, items };
        Ok(Arc::new(Plan { fake: Arc::clone(&self.0), request, disclosure, agreed: AtomicBool::new(false) }))
    }
    fn setup(&self) -> Result<SetupView, AiFailure> {
        std::thread::sleep(self.0.setup_delay);
        let chosen = self.0.chosen.lock().unwrap().clone();
        Ok(SetupView {
            connections: vec![ConnectionRow {
                id: "fake".into(),
                label: "Fake".into(),
                address: Some("https://fake.example/v1".into()),
                local: false,
                problem: None,
            }],
            models: vec![ModelRow {
                connection: "fake".into(),
                model: "fill-1".into(),
                label: "Fill One".into(),
                description: "Online".into(),
                selected: chosen.is_some(),
            }],
            store: None,
        })
    }
    fn connect(&self, kind: ServiceKind, value: &str) -> Result<(), AiFailure> {
        self.0.connected.lock().unwrap().push((kind, value.to_owned()));
        Ok(())
    }
    fn disconnect(&self, _: &str) -> Result<(), AiFailure> {
        Ok(())
    }
    fn choose(&self, c: &str, m: &str) -> Result<(), AiFailure> {
        *self.0.chosen.lock().unwrap() = Some((c.into(), m.into()));
        Ok(())
    }
}

/// An app with `f` as its generative AI and an open 64² white document.
fn app_with(f: Fake) -> (PhotocraftApp, Arc<Fake>) {
    let f = Arc::new(f);
    let ai: photocraft_engine::genai::SharedAi = Arc::new(Ai(Arc::clone(&f)));
    let mut app = PhotocraftApp::new(Session::new(), Services { genai: Some(ai), ..Default::default() });
    app.run("file.new", json!({"width": 64, "height": 64, "background": "#ffffff"})).unwrap();
    (*app, f)
}

/// A harness drawing `f` with PhotoCraft's fonts and theme.
/// (The first frame runs before the fonts are in: it draws nothing.)
fn harness<'a>(mut f: impl FnMut(&mut egui::Ui, &mut PhotocraftApp) + 'a, app: PhotocraftApp) -> egui_kittest::Harness<'a, PhotocraftApp> {
    let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                f(ui, app);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

/// Edit › Generative Fill as the menus list it.
fn fill_row(app: &PhotocraftApp) -> Option<(String, bool)> {
    crate::menus::menu_rows(app).into_iter().find(|r| r.command.id == FILL).map(|r| (r.command.id.to_string(), r.enabled))
}

/// Open the prompt through the menu, as a click on Edit › Generative Fill does.
fn open_prompt_dialog(app: &mut PhotocraftApp) -> DialogId {
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, FILL, json!({})).unwrap();
    open_dialog(app, FILL).expect("the prompt opened")
}

/// Fill the selection with `prompt` through the prompt dialog.
fn generate(app: &mut PhotocraftApp, prompt: &str) {
    let id = open_prompt_dialog(app);
    set(app, id, "prompt", json!(prompt)).unwrap();
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
}

/// The generative layers of the active document.
fn groups(app: &PhotocraftApp) -> Vec<Layer> {
    app.session.active().unwrap().doc.layers.iter().filter(|l| l.generative.is_some()).cloned().collect()
}

/// Tick the shell until `done` holds (at most 10 s).
fn tick_until(app: &mut PhotocraftApp, done: impl Fn(&PhotocraftApp) -> bool) {
    let ctx = app.ctx().clone();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !done(app) {
        assert!(std::time::Instant::now() < deadline, "timed out: {:?}", app.session.generative_fills());
        crate::jobs_ui::tick(app, &ctx);
        feed(app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn the_web_build_lists_no_generative_fill() {
    let mut web = PhotocraftApp::new(Session::new(), Services::default());
    web.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert!(fill_row(&web).is_none(), "no generative AI there: no item");
    assert!(!web.session.generative_status().is_ready());
}

#[test]
fn unavailable_greys_the_menu_item_and_the_prompt_offers_setup() {
    let reason = "Generative fill has no model chosen. Open Set up AI to choose one.";
    let (mut app, f) = app_with(fake());
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    assert!(fill_row(&app).unwrap().1);
    *f.status.lock().unwrap() = AiStatus::Unavailable { reason: reason.into() };
    assert!(!fill_row(&app).expect("listed on the desktop").1, "greyed out");
    assert_eq!(app.session.disabled_reason(FILL).as_deref(), Some(reason), "the shared layer's reason, as it is");
    // Opened anyway (a shortcut), the prompt can't Generate, and its footer opens the setup.
    let id = open_prompt_dialog(&mut app);
    let ctx = app.ctx().clone();
    assert!(crate::dialogs::confirm(&mut app, &ctx, id).is_err());
    footer(&mut app, &ctx, id, SET_UP);
    assert!(open_dialog(&app, setup::KEY).is_some());
}

/// FredApps on #2879: no setup promotion in the options bar. The button is there once a model is
/// set up; before that only if the person turned it on, and then it opens the setup.
#[test]
fn the_options_bar_button_shows_once_set_up_or_when_asked_for() {
    let ready = AiStatus::Ready { model: "M".into(), connection: "C".into(), local: true, billing: Default::default() };
    let not_set_up = AiStatus::Unavailable { reason: "No model is chosen.".into() };
    for (status, always, shown) in [(ready.clone(), false, true), (ready, true, true), (not_set_up.clone(), false, false), (not_set_up, true, true)] {
        let (mut app, f) = app_with(fake());
        app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
        app.run("prefs.set", json!({"path": "integrations.alwaysShowGenerativeFillButton", "value": always})).unwrap();
        *f.status.lock().unwrap() = status.clone();
        let mut h = harness(|ui, app: &mut PhotocraftApp| options_bar_button(app, ui), app);
        h.run_steps(2);
        assert_eq!(h.query_by_label("Generative Fill…").is_some(), shown, "{status:?}, always {always}");
    }
}

#[test]
fn the_prompt_dialog_runs_the_fill_and_remembers_the_prompt() {
    let (mut app, _) = app_with(fake());
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let ctx = app.ctx().clone();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .with_context(ctx.clone())
        .build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::run(app, ui), app);
    let id = open_prompt_dialog(h.state_mut());
    h.run_steps(2);
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "prompt", json!("a red kite")).unwrap();
    h.run_steps(2);
    h.get_by_label("Generate 3").click();
    h.run_steps(3);
    assert_eq!(groups(h.state()).len(), 1);
    assert_eq!(groups(h.state())[0].name, "a red kite");
    h.state_mut().run("select.rect", json!({"x": 30, "y": 30, "width": 8, "height": 8})).unwrap();
    let id = open_prompt_dialog(h.state_mut());
    h.run_steps(2);
    assert_eq!(crate::new_layer_ui::testing::fields(h.state(), id).text("prompt"), Some("a red kite"), "remembered");
}

#[test]
fn consent_is_asked_before_anything_is_sent_and_this_time_is_not_remembered() {
    let (mut app, f) = app_with(Fake { remote: true, ..fake() });
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    generate(&mut app, "lamp");
    assert_eq!(f.sent.load(Ordering::SeqCst), 0, "nothing sent before the answer");
    let consent = open_dialog(&app, consent::KEY).expect("asked");
    assert!(!app.ui.status_error, "waiting isn't an error: {}", app.ui.status);
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, consent).unwrap();
    assert_eq!(f.sent.load(Ordering::SeqCst), 3);
    assert_eq!(groups(&app).len(), 1);
    assert!(!f.remembered.load(Ordering::SeqCst), "only this time");
    // Asked again next time.
    app.run("select.rect", json!({"x": 30, "y": 30, "width": 8, "height": 8})).unwrap();
    generate(&mut app, "lamp 2");
    assert!(open_dialog(&app, consent::KEY).is_some());
}

#[test]
fn dont_send_discards_the_fill_and_nothing_is_sent() {
    let (mut app, f) = app_with(Fake { remote: true, ..fake() });
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    generate(&mut app, "lamp");
    let consent = open_dialog(&app, consent::KEY).expect("asked");
    let ctx = app.ctx().clone();
    crate::dialogs::cancel(&mut app, &ctx, consent).unwrap();
    feed(&mut app, &ctx);
    assert_eq!(f.sent.load(Ordering::SeqCst), 0);
    assert!(app.session.generative_fills().is_empty(), "discarded");
    assert!(open_dialog(&app, consent::KEY).is_none(), "not asked again");
    assert!(groups(&app).is_empty());
}

#[test]
fn one_answer_covers_every_area_filled_at_once() {
    let (mut app, f) = app_with(Fake { remote: true, ..fake() });
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 16, "height": 16, "mode": "add"})).unwrap();
    generate(&mut app, "x");
    assert_eq!(consent::asked_fills(&app).len(), 2, "both areas wait on the one prompt");
    let consent = open_dialog(&app, consent::KEY).unwrap();
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, consent).unwrap();
    assert_eq!(f.sent.load(Ordering::SeqCst), 6);
    assert_eq!(groups(&app).len(), 2);
}

#[test]
fn separate_areas_are_filled_one_by_one_unless_asked_otherwise() {
    let (mut app, _) = app_with(fake());
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 16, "height": 16, "mode": "add"})).unwrap();
    let id = open_prompt_dialog(&mut app);
    assert_eq!(crate::new_layer_ui::testing::fields(&app, id).bool("separate"), Some(true), "on unless turned off");
    set(&mut app, id, "prompt", json!("x")).unwrap();
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let bounds =
        |app: &PhotocraftApp| -> Vec<photocraft_geom::Rect> { groups(app).iter().map(|l| l.mask.as_ref().unwrap().surface.content_bounds()).collect() };
    let r = photocraft_geom::Rect::from_xywh;
    assert_eq!(bounds(&app).len(), 2);
    assert_eq!(bounds(&app), [r(0, 0, 26, 26), r(30, 30, 34, 34)], "each area's mask, grown by 10 px inside the canvas");
    // Turned off, the selection is filled as one.
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 16, "height": 16, "mode": "add"})).unwrap();
    let id = open_prompt_dialog(&mut app);
    set(&mut app, id, "prompt", json!("y")).unwrap();
    set(&mut app, id, "separate", json!(false)).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert_eq!(bounds(&app).len(), 3);
}

#[test]
fn setup_connects_and_chooses_without_the_key_in_any_field() {
    let (mut app, f) = app_with(fake());
    let id = open_setup(&mut app);
    let ctx = app.ctx().clone();
    tick_until(&mut app, |app| setup::read_for_test(app).is_some());
    let read = setup::read_for_test(&app).unwrap().unwrap();
    assert_eq!(read.models.len(), 1);
    setup::connect_for_test(&mut app, ServiceKind::OpenAi, SECRET);
    tick_until(&mut app, |_| !f.connected.lock().unwrap().is_empty());
    assert_eq!(f.connected.lock().unwrap()[0], (ServiceKind::OpenAi, SECRET.to_owned()));
    let fields = crate::new_layer_ui::testing::fields(&app, id);
    assert!(fields.iter().all(|(_, v)| !format!("{v:?}").contains(SECRET)), "the key is never in the dialog's fields");
    setup::choose_for_test(&mut app, "fake", "fill-1");
    assert_eq!(*f.chosen.lock().unwrap(), Some(("fake".into(), "fill-1".into())));
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert!(open_dialog(&app, setup::KEY).is_none());
}

/// A setup read that takes a while (the keychain) never holds the frame up.
#[test]
fn a_slow_setup_read_never_holds_up_the_frame() {
    let (mut app, _) = app_with(Fake { setup_delay: std::time::Duration::from_millis(400), ..fake() });
    open_setup(&mut app);
    let ctx = app.ctx().clone();
    let t = std::time::Instant::now();
    feed(&mut app, &ctx);
    assert!(t.elapsed() < std::time::Duration::from_millis(200), "{:?}", t.elapsed());
    assert!(setup::read_for_test(&app).is_none(), "not read yet");
    tick_until(&mut app, |app| setup::read_for_test(app).is_some());
}

// ---- Fills that run side by side --------------------------------------------------------------

/// [`app_with`], running jobs in the background as the desktop app does.
fn background(f: Fake) -> (PhotocraftApp, Arc<Fake>) {
    let (mut app, f) = app_with(f);
    app.background_jobs = true;
    (app, f)
}

#[test]
fn in_the_background_fills_block_nothing_and_land_where_they_were_started() {
    let (mut app, _) = background(Fake { delay: std::time::Duration::from_millis(300), ..fake() });
    let bg = app.session.active().unwrap().active_layer;
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    generate(&mut app, "a");
    // No progress window, no lock, and the selection let go for the next one.
    assert!(app.session.active_job().is_none());
    assert!(crate::jobs_ui::job_in_view(&app).is_none(), "the status bar's job readout leaves it to its own");
    assert!(app.session.active().unwrap().doc.selection.is_none());
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 16, "height": 16})).unwrap();
    generate(&mut app, "b");
    let fills = app.session.generative_fills();
    assert_eq!(fills.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    tick_until(&mut app, |app| app.session.generative_fills().is_empty() && !app.session.has_jobs());
    assert_eq!(groups(&app).len(), 2);
    assert_eq!(app.session.active().unwrap().active_layer, bg, "the person's layer stays active");
    assert!(app.ui.status.starts_with("Generated"), "{}", app.ui.status);
}

#[test]
fn a_background_fill_waiting_for_consent_asks() {
    let (mut app, f) = background(Fake { remote: true, ..fake() });
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    generate(&mut app, "lamp");
    tick_until(&mut app, |app| app.session.generative_fills().first().is_some_and(|f| f.state == "consent") && !app.session.has_jobs());
    let consent = open_dialog(&app, consent::KEY).expect("the job's end asked");
    assert!(!app.ui.status_error, "waiting isn't an error: {}", app.ui.status);
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, consent).unwrap();
    tick_until(&mut app, |app| app.session.generative_fills().is_empty() && !app.session.has_jobs());
    assert_eq!(groups(&app).len(), 1);
    assert_eq!(f.sent.load(Ordering::SeqCst), 3);
}

#[test]
fn a_failed_fill_offers_retry_and_can_be_discarded() {
    let offline = AiFailure::new(AiErrorKind::Network, "Could not reach the service: offline");
    let (mut app, _) = background(Fake { fail: Some(offline), delay: std::time::Duration::from_millis(20), ..fake() });
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    generate(&mut app, "x");
    tick_until(&mut app, |app| app.session.generative_fills().first().is_some_and(|f| f.state == "failed") && !app.session.has_jobs());
    assert!(app.ui.status.contains("offline"), "{}", app.ui.status);
    let id = app.session.generative_fills()[0].id;
    app.run(photocraft_engine::genai_cmds::DISCARD, json!({"fill": id})).unwrap();
    assert!(app.session.generative_fills().is_empty());
}

/// The canvas draws each fill in progress: its veil, outline and tag.
#[test]
fn the_canvas_draws_fills_in_progress() {
    let (mut app, _) = background(Fake { delay: std::time::Duration::from_secs(5), ..fake() });
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    generate(&mut app, "kite");
    let mut h = harness(
        |ui, app: &mut PhotocraftApp| {
            let rect = ui.max_rect();
            let view = craft::behavior::CanvasView::new(rect, craft::behavior::kurbo::Point::new(32.0, 32.0), 4.0);
            paint_pending(app, ui, rect, view);
        },
        app,
    );
    h.run_steps(3);
    h.get_by_label("Generating");
    h.get_by_label("Cancel this Generative Fill");
}

/// Properties shows a generative layer's variations, its prompt and Generate: clicking a
/// thumbnail makes the layer show it (one undo step), and Generate adds three more.
#[test]
fn properties_show_the_variations_the_prompt_and_generate() {
    let (mut app, f) = app_with(Fake { remote: false, ..fake() });
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 32, "height": 24})).unwrap();
    app.run(FILL, json!({"prompt": "a red kite"})).unwrap();
    let layer = app.session.generative_layer().expect("the fill finished");
    let steps = app.session.active().unwrap().history.past_len();
    let mut h = harness(|ui, app: &mut PhotocraftApp| variations_section(app, ui), app);
    h.run_steps(3);
    let generative = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().doc.layer(layer).unwrap().generative.clone().unwrap();
    assert_eq!((generative(&h).variations.len(), generative(&h).selected), (3, 0));
    h.get_by_label("Variations");
    h.get_by_label("a red kite");
    h.get_by_label("Variation 3").click();
    h.run_steps(3);
    assert_eq!(generative(&h).selected, 2);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "one undo step");
    h.get_by_label("Generate").click();
    h.run_steps(3);
    assert_eq!((generative(&h).variations.len(), generative(&h).selected), (6, 3), "three more, the first of them showing");
    assert_eq!(f.sent.load(Ordering::SeqCst), 6);
    h.get_by_label("Variation 6");
}

/// Review follow-ups (#10903).
mod review;
