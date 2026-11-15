//! Cancel (its button, Esc, closing the window) asks "Discard changes to …?" before throwing away
//! anything changed since the dialog opened, in the develop view and the classic panel alike:
//! nothing changed closes at once; Keep Editing (the default: Enter, and Esc) goes back with
//! everything as it was; Discard closes without applying. Esc first puts down what is up.
use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::{PhotocraftApp, Services, camera_raw_ui, theme::ThemeKind};
use serde_json::{Value, json};

fn dng() -> Vec<u8> {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let (w, h) = (96, 64);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
    spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
    spec.build()
}

fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    photocraft_ui_egui::control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}

fn settle(h: &mut Harness<'_, PhotocraftApp>) {
    for _ in 0..600 {
        h.run_steps(1);
        let r = inspect(h);
        if r.is_null() || r["previewPending"] == false && r["detailPending"] == false {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn ui(h: &mut Harness<'_, PhotocraftApp>, v: Value) -> Value {
    let ctx = h.ctx.clone();
    let r = camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({ "ui": v })).unwrap().unwrap();
    settle(h);
    r
}

fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key) {
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE });
    }
    h.run_steps(3);
}

fn harness(open: impl FnOnce(&mut PhotocraftApp) + 'static) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1600.0, 1000.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let services = Services {
            import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        open(&mut app);
        app
    });
    h.ctx.all_styles_mut(|s| s.animation_time = 0.0);
    settle(&mut h);
    h
}

/// A raw opening in the develop view.
fn develop() -> Harness<'static, PhotocraftApp> {
    let h = harness(|app| {
        app.open_bytes("IMG_0001.dng", &dng()).unwrap();
    });
    assert_eq!(inspect(&h)["panel"], "develop");
    h
}

fn documents(h: &Harness<'_, PhotocraftApp>) -> usize {
    h.state().session.documents().len()
}

/// Nothing changed: Esc and the Cancel button close at once, without asking.
#[test]
fn no_prompt_when_nothing_changed() {
    let mut h = develop();
    // opening Masking and putting the Zoom over it is no change
    key(&mut h, Key::K);
    key(&mut h, Key::Z);
    key(&mut h, Key::Escape);
    assert!(inspect(&h).is_object() && inspect(&h)["discardPrompt"] == false, "the first Esc gives the brush back");
    key(&mut h, Key::Escape);
    assert!(inspect(&h).is_null(), "the second cancels, unasked: {}", inspect(&h));
    assert_eq!(documents(&h), 0, "the raw wasn't opened");
    // an edit undone is no change either; the Cancel button
    let mut h = develop();
    ui(&mut h, json!({"develop": {"light": {"exposure": 1.0}}}));
    ui(&mut h, json!({"undo": true}));
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(inspect(&h).is_null(), "Cancel closes unasked");
}

/// A change asks; Keep Editing (Esc, Enter, the button) keeps everything, Discard closes without
/// applying anything.
#[test]
fn a_change_asks_before_it_is_discarded() {
    let mut h = develop();
    ui(&mut h, json!({"develop": {"light": {"exposure": 1.0}}}));
    key(&mut h, Key::K);
    ui(&mut h, json!({"local": {"brush": {"points": [[0.2, 0.3], [0.4, 0.35]]}}}));
    let edited = inspect(&h)["develop"].clone();
    assert_eq!(edited["masks"].as_array().map(Vec::len), Some(1));
    // Esc (the brush is up, nothing else): the prompt, the dialog stays
    key(&mut h, Key::Escape);
    let r = inspect(&h);
    assert_eq!(r["discardPrompt"], true, "Esc with changes asks");
    let message = "Discard changes to “IMG_0001.dng”?";
    assert!(h.query_by_label(message).is_some(), "the prompt names the file");
    // Esc keeps editing
    key(&mut h, Key::Escape);
    let r = inspect(&h);
    assert_eq!((r["discardPrompt"].as_bool(), r["develop"].clone()), (Some(false), edited.clone()), "Esc keeps editing, everything kept");
    // Enter is Keep Editing too (the default)
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(inspect(&h)["discardPrompt"], true, "the Cancel button asks");
    key(&mut h, Key::Enter);
    assert_eq!(inspect(&h)["discardPrompt"], false, "Enter keeps editing");
    // the Keep Editing button; the edit is still undoable afterwards
    key(&mut h, Key::Escape);
    h.get_by_label("Keep Editing").click();
    h.run_steps(3);
    let r = inspect(&h);
    assert_eq!((r["discardPrompt"].as_bool(), r["develop"].clone()), (Some(false), edited));
    ui(&mut h, json!({"undo": true}));
    assert_eq!(inspect(&h)["develop"]["masks"].as_array().map(Vec::len), Some(0), "the history is intact");
    ui(&mut h, json!({"redo": true}));
    // Discard: closed, nothing opened or applied
    key(&mut h, Key::Escape);
    assert_eq!(inspect(&h)["discardPrompt"], true);
    h.get_by_label("Discard").click();
    h.run_steps(3);
    assert!(inspect(&h).is_null(), "Discard closes the dialog");
    assert_eq!(documents(&h), 0, "and opens nothing");
}

/// The classic panel (the filter on a layer) asks the same way, and Discard leaves the layer as it
/// was; a control client's cancel is never asked.
#[test]
fn the_classic_panel_asks_too() {
    let mut h = harness(|app| {
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    });
    let ctx = h.ctx.clone();
    camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    settle(&mut h);
    assert_eq!(inspect(&h)["panel"], "classic");
    let history = h.state().session.active().unwrap().history.past_len();
    ui(&mut h, json!({"set": {"exposure": 1.0}}));
    key(&mut h, Key::Escape);
    assert_eq!(inspect(&h)["discardPrompt"], true);
    h.get_by_label("Discard").click();
    h.run_steps(3);
    assert!(inspect(&h).is_null());
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history, "nothing applied");
    // the control channel's cancel doesn't ask
    let ctx = h.ctx.clone();
    camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    settle(&mut h);
    ui(&mut h, json!({"set": {"exposure": 1.0}}));
    let r = ui(&mut h, json!({"cancel": true}));
    assert_eq!(r["cancelled"], true);
    assert!(inspect(&h).is_null());
}

/// `PHOTOCRAFT_LC_SNAPSHOTS=<dir>` saves the prompt over the develop view (`discard-prompt.png`, on
/// the GPU).
#[test]
fn discard_prompt_snapshot() {
    let Ok(dir) = std::env::var("PHOTOCRAFT_LC_SNAPSHOTS") else { return };
    let _lock = photocraft_testkit::gpu_lock();
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1600.0, 1000.0)).wgpu().build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let services = Services {
            import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.open_bytes("IMG_0001.dng", &dng()).unwrap();
        app
    });
    h.ctx.all_styles_mut(|s| s.animation_time = 0.0);
    settle(&mut h);
    ui(&mut h, json!({"develop": {"light": {"exposure": 1.0}}}));
    key(&mut h, Key::Escape);
    std::fs::create_dir_all(&dir).unwrap();
    h.render().unwrap().save(std::path::Path::new(&dir).join("discard-prompt.png")).unwrap();
}
