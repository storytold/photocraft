use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::Session;
use photocraft_engine::genai::{
    AiErrorKind, AiFailure, ConnectionRow, Disclosure, FillJob, FillPlan, FillRequest, GenerativeAi, JobPoll, ModelRow, SentInput, SentItem, SentKind,
    ServiceKind, SetupView,
};
use photocraft_engine::genai_cmds::{AGREE, DISCARD, RETRY};

use super::*;
use crate::Services;

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
    (app, f)
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

fn fill_item(app: &PhotocraftApp) -> Option<crate::menus::MenuItem> {
    crate::menus::menu_items(app).into_iter().find(|i| i.id == FILL)
}

fn open_prompt_dialog(app: &mut PhotocraftApp) -> u64 {
    let ctx = egui::Context::default();
    crate::menus::invoke(app, &ctx, FILL, json!({})).unwrap()["dialog"].as_u64().unwrap()
}

fn dialog_with(app: &PhotocraftApp, mark: &str) -> Option<u64> {
    app.ui.dialogs.iter().find(|d| d.fields.contains_key(mark)).map(|d| d.id)
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

#[test]
fn the_web_build_lists_no_generative_fill() {
    let mut web = PhotocraftApp::new(Session::new(), Services::default());
    web.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert!(fill_item(&web).is_none(), "no generative AI there: no item");
    assert!(!web.session.generative_status().is_ready());
}

#[test]
fn unavailable_greys_the_menu_item_and_says_why() {
    let reason = "Generative fill has no model chosen. Open Set up AI to choose one.";
    let (mut app, f) = app_with(fake());
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    assert!(fill_item(&app).unwrap().enabled);
    *f.status.lock().unwrap() = AiStatus::Unavailable { reason: reason.into() };
    let item = fill_item(&app).expect("listed on the desktop");
    assert!(!item.enabled, "greyed out");
    // Asked each time it's about to be used: the shared layer's reason, as it is.
    let ctx = egui::Context::default();
    assert_eq!(crate::menus::invoke(&mut app, &ctx, FILL, json!({})).unwrap_err(), reason);
    assert!(app.ui.dialogs.is_empty());
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
        if !shown {
            continue;
        }
        h.get_by_label("Generative Fill…").click();
        h.run_steps(2);
        let app = h.state();
        let setup = app.ui.dialogs.iter().any(|d| crate::prefs_ui::is_preferences(&d.fields) && d.fields["section"] == SETUP_SECTION);
        let prompt = dialog_with(app, PROMPT_MARK).is_some();
        // Set up, it asks for a prompt; not set up, it opens the setup instead.
        assert_eq!((prompt, setup), (status.is_ready(), !status.is_ready()), "{status:?}");
    }
    // Configured but no selection: shown, greyed out.
    let (app, _) = app_with(fake());
    let mut h = harness(|ui, app: &mut PhotocraftApp| options_bar_button(app, ui), app);
    h.run_steps(2);
    assert!(h.get_by_label("Generative Fill…").accesskit_node().is_disabled(), "nothing to fill yet");
    // The web build has no Generative AI: never, even when asked for.
    let mut web = PhotocraftApp::new(Session::new(), Services::default());
    web.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    web.run("prefs.set", json!({"path": "integrations.alwaysShowGenerativeFillButton", "value": true})).unwrap();
    let mut h = harness(|ui, app: &mut PhotocraftApp| options_bar_button(app, ui), web);
    h.run_steps(2);
    assert!(h.query_by_label("Generative Fill…").is_none());
}

#[test]
fn showing_the_button_before_setup_is_opt_in() {
    assert!(!photocraft_engine::prefs::Preferences::default().integrations.always_show_generative_fill_button);
}

#[test]
fn the_prompt_dialog_runs_the_fill_and_remembers_the_prompt() {
    let (mut app, f) = app_with(Fake { remote: false, ..fake() });
    let ctx = egui::Context::default();
    // No selection: Generative Fill can't run, and says why.
    assert!(crate::menus::invoke(&mut app, &ctx, FILL, json!({})).is_err());
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let id = open_prompt_dialog(&mut app);
    let fields = &app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields;
    assert_eq!(ok_label(fields).as_deref(), Some("Generate 3"), "says how many it makes");
    assert_eq!(fields["__command"], FILL, "automation confirming it is judged as the command");
    assert!(can_confirm(&app, fields));
    app.ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!("a red square"));
    let before = layers(&app);
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(layers(&app), before + 1);
    assert_eq!(app.session.active().unwrap().doc.layers.last().unwrap().name, "a red square");
    assert_eq!(f.sent.load(Ordering::SeqCst), 3, "three variations");
    assert_eq!(app.session.prefs().dialogs.get(FILL), Some(&json!({"prompt": "a red square", "separate": true})));
    let (_, params) = app.session.journal.iter().rev().find(|(c, _)| c == FILL).unwrap();
    assert_eq!(params, &json!({"prompt": "a red square"}));
}

#[test]
fn a_prompt_left_open_while_the_tool_stops_working_explains_and_offers_setup() {
    let (mut app, f) = app_with(fake());
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let id = open_prompt_dialog(&mut app);
    let reason = "The key for OpenAI is missing. Open Set up AI to add it.";
    *f.status.lock().unwrap() = AiStatus::Unavailable { reason: reason.into() };
    assert!(!can_confirm(&app, &app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields), "Generate is greyed");
    let mut h = harness(
        move |ui, app: &mut PhotocraftApp| {
            let mut fields = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
            body(app, ui, &mut fields);
        },
        app,
    );
    h.run_steps(2);
    h.get_by_label(reason);
    h.get_by_label("Set Up Generative AI…").click();
    h.run_steps(2);
    let app = h.into_state();
    assert!(
        app.ui.dialogs.iter().any(|d| crate::prefs_ui::is_preferences(&d.fields) && d.fields["section"] == SETUP_SECTION),
        "Preferences › Integrations opened"
    );
}

#[test]
fn consent_is_asked_before_anything_is_sent_and_this_time_is_not_remembered() {
    let (mut app, f) = app_with(Fake { remote: true, ..fake() });
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let id = open_prompt_dialog(&mut app);
    app.ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!("a kite"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(f.sent.load(Ordering::SeqCst), 0, "nothing sent yet");
    let consent = dialog_with(&app, CONSENT_MARK).expect("the consent prompt opened");
    let fields = app.ui.dialogs.iter().find(|d| d.id == consent).unwrap().fields.clone();
    assert_eq!(fields["destination"], "https://fake.example");
    assert_eq!(fields["__command"], AGREE, "automation confirming it is refused as `layer.generative.agree`");
    assert_eq!(fields["items"].as_array().unwrap().len(), 3);
    assert_eq!(ok_label(&fields).as_deref(), Some("Send"));
    assert_eq!(cancel_label(&fields), Some("Don't Send"));
    // It shows where, what (with sizes) and the four choices.
    let mut h = harness(
        move |ui, app: &mut PhotocraftApp| {
            let mut fields = app.ui.dialogs.iter().find(|d| d.id == consent).unwrap().fields.clone();
            body(app, ui, &mut fields);
        },
        app,
    );
    h.run_steps(2);
    h.get_by_label("Generative Fill will send this to https://fake.example:");
    h.get_by_label_contains("The picture around the selection · ");
    h.get_by_label_contains("The selection's shape · ");
    h.get_by_label_contains("Your prompt");
    for choice in ["Only this time", "For an hour", "For a day", "Until I turn it off"] {
        h.get_by_label(choice);
    }
    let mut app = h.into_state();
    crate::dialogs::confirm(&mut app, consent).unwrap();
    assert_eq!(f.sent.load(Ordering::SeqCst), 3);
    assert!(app.session.generative_fills().is_empty());
    assert_eq!(app.session.active().unwrap().doc.layers.last().unwrap().name, "a kite");
    // The next fill asks again.
    let id = open_prompt_dialog(&mut app);
    crate::dialogs::confirm(&mut app, id).unwrap();
    let consent = dialog_with(&app, CONSENT_MARK).expect("asked again");
    // Until turned off: remembered, so the one after goes straight through.
    app.ui.dialog_mut(consent).unwrap().fields.insert("term".into(), json!("until_revoked"));
    crate::dialogs::confirm(&mut app, consent).unwrap();
    let id = open_prompt_dialog(&mut app);
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(dialog_with(&app, CONSENT_MARK).is_none());
    assert_eq!(f.sent.load(Ordering::SeqCst), 9);
}

#[test]
fn dont_send_discards_the_fill_and_nothing_is_sent() {
    let (mut app, f) = app_with(Fake { remote: true, ..fake() });
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let id = open_prompt_dialog(&mut app);
    crate::dialogs::confirm(&mut app, id).unwrap();
    let consent = dialog_with(&app, CONSENT_MARK).unwrap();
    let before = layers(&app);
    crate::dialogs::cancel(&mut app, consent).unwrap();
    assert!(app.session.generative_fills().is_empty());
    assert_eq!(f.sent.load(Ordering::SeqCst), 0);
    assert_eq!(layers(&app), before);
}

#[test]
fn one_answer_covers_every_area_filled_at_once() {
    let (mut app, f) = app_with(Fake { remote: true, ..fake() });
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 16, "height": 16, "mode": "add"})).unwrap();
    let id = open_prompt_dialog(&mut app);
    crate::dialogs::confirm(&mut app, id).unwrap();
    let consent = dialog_with(&app, CONSENT_MARK).unwrap();
    assert_eq!(app.ui.dialogs.iter().find(|d| d.id == consent).unwrap().fields["fills"].as_array().unwrap().len(), 2);
    crate::dialogs::confirm(&mut app, consent).unwrap();
    assert_eq!(f.sent.load(Ordering::SeqCst), 6);
    assert_eq!(app.session.active().unwrap().doc.layers.iter().filter(|l| l.is_group()).count(), 2);
}

#[test]
fn setup_lists_models_chooses_and_connects_without_the_key_in_any_field() {
    let (app, f) = app_with(fake());
    let mut h = harness(|ui, app: &mut PhotocraftApp| prefs_section(app, ui), app);
    h.run_steps(2);
    h.get_by_label("Generative AI");
    h.get_by_label("Generative Fill uses Fill One (Fake).");
    h.get_by_label("Fake");
    h.get_by_label("Fill One").click();
    h.run_steps(2);
    assert_eq!(*f.chosen.lock().unwrap(), Some(("fake".into(), "fill-1".into())), "the suite's choice, through the shared layer");
    h.state_mut().genai.setup.key = format!("  {SECRET}\n");
    h.run_steps(1);
    h.get_by_label("Connect").click();
    for _ in 0..200 {
        h.run_steps(1);
        if f.connected.lock().unwrap().len() == 1 && h.query_by_label("Connected.").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.get_by_label("Connected.");
    assert_eq!(f.connected.lock().unwrap()[0].0, ServiceKind::OpenAi);
    let app = h.into_state();
    assert!(app.genai.setup.key.is_empty(), "the typed key is handed over and forgotten");
    let dialogs = serde_json::to_string(&app.ui.dialogs).unwrap();
    let prefs = serde_json::to_string(&app.session.prefs().to_json()).unwrap();
    assert!(!dialogs.contains(SECRET) && !prefs.contains(SECRET), "never in dialog fields or preferences");
}

#[test]
fn separate_areas_are_filled_one_by_one_unless_asked_otherwise() {
    let (mut app, _) = app_with(Fake { remote: false, ..fake() });
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 16, "height": 16, "mode": "add"})).unwrap();
    let id = open_prompt_dialog(&mut app);
    let fields = |app: &PhotocraftApp, id: u64| app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    assert_eq!(fields(&app, id)["__areas"], 2);
    assert_eq!(fields(&app, id)["separate"], true, "on unless turned off");
    app.ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!("x"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let groups = |app: &PhotocraftApp| -> Vec<(String, photocraft_geom::Rect)> {
        let doc = &app.session.active().unwrap().doc;
        doc.layers.iter().filter(|l| l.is_group()).map(|l| (l.name.clone(), l.mask.as_ref().unwrap().surface.content_bounds())).collect()
    };
    let r = |x, y, w, h| photocraft_geom::Rect::from_xywh(x, y, w, h);
    assert_eq!(groups(&app), [("x".into(), r(4, 4, 12, 12)), ("x".into(), r(40, 40, 16, 16))]);
    // Turned off, the selection is filled as one; the choice is remembered.
    let id = open_prompt_dialog(&mut app);
    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
    f.insert("prompt".into(), json!("y"));
    f.insert("separate".into(), json!(false));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(groups(&app).last().unwrap(), &("y".into(), r(4, 4, 52, 52)));
    assert_eq!(app.session.prefs().dialogs.get(FILL).unwrap()["separate"], false);
    let id = open_prompt_dialog(&mut app);
    assert_eq!(fields(&app, id)["separate"], false);
}

// ---- Fills that run side by side --------------------------------------------------------------

/// [`app_with`], running jobs in the background as the desktop app does.
fn background(f: Fake) -> (PhotocraftApp, Arc<Fake>) {
    let (mut app, f) = app_with(f);
    app.background_jobs = true;
    (app, f)
}

fn generate(app: &mut PhotocraftApp, prompt: &str) {
    let id = open_prompt_dialog(app);
    app.ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!(prompt));
    crate::dialogs::confirm(app, id).unwrap();
}

/// Tick the shell until `done` holds (at most 10 s).
fn tick_until(app: &mut PhotocraftApp, done: impl Fn(&PhotocraftApp) -> bool) {
    let ctx = egui::Context::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !done(app) {
        assert!(std::time::Instant::now() < deadline, "timed out: {:?}", app.session.generative_fills());
        crate::jobs_ui::tick(app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn group_count(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.iter().filter(|l| l.is_group()).count()
}

#[test]
fn in_the_background_fills_block_nothing_and_land_where_they_were_started() {
    let (mut app, _) = background(Fake { delay: std::time::Duration::from_millis(300), remote: false, ..fake() });
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
    // The status bar counts them.
    let mut h = harness(|ui, app: &mut PhotocraftApp| status_fills(app, ui), app);
    h.run_steps(2);
    assert!(h.query_by_label("2 generating").is_some() || h.query_by_label("1 generating · 1 waiting").is_some(), "status readout");
    let mut app = h.into_state();
    tick_until(&mut app, |app| app.session.generative_fills().is_empty() && !app.session.has_jobs());
    assert_eq!(group_count(&app), 2);
    assert_eq!(app.session.active().unwrap().active_layer, bg, "the person's layer stays active");
    assert!(app.ui.status.starts_with("Generated"), "{}", app.ui.status);
}

#[test]
fn a_background_fill_waiting_for_consent_asks_and_shows_in_the_layers_panel() {
    let (mut app, f) = background(Fake { remote: true, ..fake() });
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    generate(&mut app, "lamp");
    tick_until(&mut app, |app| app.session.generative_fills().first().is_some_and(|f| f.state == "consent") && !app.session.has_jobs());
    let consent = dialog_with(&app, CONSENT_MARK).expect("the job's end asked");
    assert!(!app.ui.status_error, "waiting isn't an error: {}", app.ui.status);
    let doc = app.session.active().unwrap().doc.id;
    let mut h = harness(move |ui, app: &mut PhotocraftApp| layer_rows_unplaced(app, ui, doc, &[]), app);
    h.run_steps(2);
    h.get_by_label("Needs your OK: lamp");
    let mut app = h.into_state();
    crate::dialogs::confirm(&mut app, consent).unwrap();
    tick_until(&mut app, |app| app.session.generative_fills().is_empty() && !app.session.has_jobs());
    assert_eq!(group_count(&app), 1);
    assert_eq!(f.sent.load(Ordering::SeqCst), 3);
}

#[test]
fn a_failed_fill_keeps_its_row_offers_retry_and_can_be_discarded() {
    let offline = AiFailure::new(AiErrorKind::Network, "Could not reach the service: offline");
    let (mut app, f) = background(Fake { fail: Some(offline), delay: std::time::Duration::from_millis(20), ..fake() });
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    generate(&mut app, "lamp");
    tick_until(&mut app, |app| app.session.generative_fills().first().is_some_and(|f| f.state == "failed") && !app.session.has_jobs());
    assert!(app.ui.status_error && app.ui.status.contains("offline"), "{}", app.ui.status);
    let doc = app.session.active().unwrap().doc.id;
    let mut h = harness(move |ui, app: &mut PhotocraftApp| layer_rows_unplaced(app, ui, doc, &[]), app);
    h.run_steps(2);
    h.get_by_label("Failed: lamp");
    let mut app = h.into_state();
    // Retry with a working service lands it.
    let working: photocraft_engine::genai::SharedAi = Arc::new(Ai(Arc::new(Fake { delay: std::time::Duration::from_millis(20), ..fake() })));
    app.session.set_generative_ai(Some(working));
    let id = app.session.generative_fills()[0].id;
    app.run(RETRY, json!({"fill": id})).unwrap();
    tick_until(&mut app, |app| app.session.generative_fills().is_empty() && !app.session.has_jobs());
    assert_eq!(group_count(&app), 1);
    // Discard stops one that is still generating; nothing lands.
    let slow: photocraft_engine::genai::SharedAi = Arc::new(Ai(Arc::new(Fake { delay: std::time::Duration::from_millis(300), ..fake() })));
    app.session.set_generative_ai(Some(slow));
    app.run("select.rect", json!({"x": 30, "y": 30, "width": 12, "height": 12})).unwrap();
    generate(&mut app, "gone");
    let id = app.session.generative_fills()[0].id;
    app.run(DISCARD, json!({"fill": id})).unwrap();
    app.session.join_cancelled_jobs();
    tick_until(&mut app, |app| !app.session.has_jobs());
    assert!(app.session.generative_fills().is_empty());
    assert_eq!(group_count(&app), 1);
    let _ = f;
}

#[test]
fn a_refused_key_says_so_and_offers_setup() {
    let auth = AiFailure::new(AiErrorKind::Auth, "OpenAI refused the key. Check it in Set up AI.");
    let (mut app, _) = background(Fake { fail: Some(auth), ..fake() });
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    generate(&mut app, "lamp");
    tick_until(&mut app, |app| app.session.generative_fills().first().is_some_and(|f| f.state == "failed") && !app.session.has_jobs());
    assert!(app.ui.status.contains("refused the key"), "the layer's message: {}", app.ui.status);
    let doc = app.session.active().unwrap().doc.id;
    let mut h = harness(move |ui, app: &mut PhotocraftApp| layer_rows_unplaced(app, ui, doc, &[]), app);
    h.run_steps(2);
    let mut app = h.into_state();
    // The tag offers the way to fix it.
    let mut h = harness(
        move |ui, app: &mut PhotocraftApp| {
            let xf = crate::canvas::ViewXform { rect: ui.max_rect(), zoom: 4.0, center: [32.0, 32.0], flip: false, rotation: 0.0, aspect: 1.0 };
            let painter = ui.painter().clone();
            paint_pending(app, &ui.ctx().clone(), &painter, &xf, doc);
        },
        std::mem::replace(&mut app, PhotocraftApp::new(Session::new(), Services::default())),
    );
    h.run_steps(2);
    h.get_by_label("Set Up Generative AI…").click();
    h.run_steps(2);
    let app = h.into_state();
    assert!(app.ui.dialogs.iter().any(|d| crate::prefs_ui::is_preferences(&d.fields)));
}

#[test]
fn the_canvas_draws_fills_in_progress() {
    let (mut app, _) = background(Fake { delay: std::time::Duration::from_millis(500), ..fake() });
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    generate(&mut app, "a");
    let doc = app.session.active().unwrap().doc.id;
    let mut h = harness(
        move |ui, app: &mut PhotocraftApp| {
            let xf = crate::canvas::ViewXform { rect: ui.max_rect(), zoom: 4.0, center: [32.0, 32.0], flip: false, rotation: 0.0, aspect: 1.0 };
            let painter = ui.painter().clone();
            paint_pending(app, &ui.ctx().clone(), &painter, &xf, doc);
        },
        app,
    );
    h.run_steps(2);
    // Its tag: what it is doing, with Cancel.
    assert!(h.query_by_label("Generating").is_some() || h.query_by_label_contains("Waiting").is_some());
    h.get_by_label("Cancel this Generative Fill").click();
    h.run_steps(2);
    let mut app = h.into_state();
    app.session.join_cancelled_jobs();
    tick_until(&mut app, |app| !app.session.has_jobs());
    assert!(app.session.generative_fills().is_empty());
}

/// Properties shows a generative layer's variations: clicking one shows it, and Generate More
/// adds three.
#[test]
fn properties_show_the_variations() {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 760.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let (mut app, _) = app_with(Fake { remote: false, ..fake() });
        app.run("select.rect", json!({"x": 8, "y": 8, "width": 32, "height": 24})).unwrap();
        app
    });
    h.state_mut().run(FILL, json!({"prompt": "a red kite"})).unwrap();
    for _ in 0..200 {
        h.run_steps(1);
        if h.state().session.generative_layer().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let group = h.state().session.generative_layer().expect("the fill finished");
    h.run_steps(4);
    let shown = |h: &egui_kittest::Harness<'_, PhotocraftApp>| -> Vec<bool> {
        h.state().session.active().unwrap().doc.layer(group).unwrap().children().unwrap().iter().map(|l| l.visible).collect()
    };
    assert_eq!(shown(&h), [true, false, false]);
    h.get_by_label("Variations");
    h.get_by_label("Generate More");
    if let (Some(dir), Ok(img)) = (std::env::var_os("GENAI_SNAPSHOT_DIR"), h.render()) {
        img.save(std::path::Path::new(&dir).join("genai-variations.png")).unwrap();
    }
    // The Layers panel lists "Variation 3" too; the thumbnail is the one in Properties, above it.
    let thumb = h.query_all_by_role_and_label(egui::accesskit::Role::Button, "Variation 3").min_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y)).unwrap();
    thumb.click();
    h.run_steps(4);
    assert_eq!(shown(&h), [false, false, true]);
    h.get_by_label("Generate More").click();
    for _ in 0..200 {
        h.run_steps(1);
        if shown(&h).len() == 6 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(shown(&h), [false, false, false, true, false, false]);
}

/// Renders the prompt, the consent prompt and the setup for a look
/// (`GENAI_SNAPSHOT_DIR=… cargo test -p photocraft-ui-egui --lib genai_ui::tests::the_windows_render`).
#[test]
fn the_windows_render() {
    let Some(dir) = std::env::var_os("GENAI_SNAPSHOT_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let (mut app, _) = app_with(Fake { remote: true, ..fake() });
        app.run("select.rect", json!({"x": 8, "y": 8, "width": 32, "height": 24})).unwrap();
        app.ui.tool = crate::state::Tool::RectMarquee;
        app
    });
    h.run_steps(6);
    let save = |h: &mut egui_kittest::Harness<PhotocraftApp>, name: &str| {
        if let Ok(img) = h.render() {
            img.save(dir.join(name)).unwrap();
        }
    };
    let id = open_prompt_dialog(h.state_mut());
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!("a red kite"));
    h.run_steps(6);
    save(&mut h, "genai-prompt.png");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.run_steps(6);
    save(&mut h, "genai-consent.png");
    let consent = dialog_with(h.state(), CONSENT_MARK).unwrap();
    crate::dialogs::cancel(h.state_mut(), consent).unwrap();
    open_setup(h.state_mut());
    h.run_steps(8);
    save(&mut h, "genai-setup.png");
}

/// Review follow-ups (#10903).
mod review;
