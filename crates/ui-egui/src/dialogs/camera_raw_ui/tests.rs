use std::sync::Arc;

use craft::host::{PickKind, Picked, ScriptedFiles};
use craft::imaging::raw::RawDevelopSettings as CameraRaw;
use craft::imaging::ui::raw_develop::RawDevelopWorkspace;
use craft::ui::dialog::HostedWorkspace;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use super::convert::open_pixels;
use super::open::{Coverage, Target, document_index, open_raw};
use super::sidecar::{existing_sidecar_is_ours, read_bounded};
use super::*;
use crate::PhotocraftApp;

/// The first raw waiting for the open-time dialog, taken off its filmstrip.
fn take_queued(app: &mut PhotocraftApp) -> Option<RawOpen> {
    (!app.raw_shelf.is_empty()).then(|| app.raw_shelf.remove(0).raw)
}

/// One frame of the app, whose hosted dialogs show the workspace and `take` what it reports. The framework's job
/// runtime delivers a develop only inside the app's frame.
fn frame(app: &mut PhotocraftApp) {
    crate::canvas::testing::frame(app);
}

/// The settings the open RAW Develop workspace holds. The workspace's session is `CameraRaw`
/// itself, so there is no conversion.
fn shown(app: &PhotocraftApp) -> CameraRaw {
    app.framework.dialogs.get::<HostedWorkspace<RawDevelopWorkspace>>().expect("RAW Develop is open").workspace().session().clone()
}

/// The app's state beside the hosted workspace.
fn session(app: &PhotocraftApp) -> &RawSession {
    app.raw.as_ref().expect("RAW Develop is open")
}

/// The proxy pixels of the last develop that landed.
fn processed(app: &PhotocraftApp) -> Vec<[f32; 4]> {
    session(app).source.developed().expect("a develop has landed").processed.clone()
}

/// Whether the open-time dialog knows how the raw was developed (its camera tone curve).
fn knows_raw_curve(app: &PhotocraftApp) -> bool {
    session(app).source.has_raw_curve()
}

/// Runs frames until the develop of the settings in force has landed. The first pass after a
/// change starts its develop; later passes wait for the runtime's job to deliver.
fn settle(app: &mut PhotocraftApp) {
    if app.raw.is_none() {
        return;
    }
    frame(app);
    for _ in 0..2000 {
        frame(app);
        // The develop waits on the framework's queue until a request finishes it, as `waitPreview` does.
        craft::ui::runtime::pending::finish_jobs(app.ctx(), std::time::Duration::from_secs(30));
        frame(app);
        // The workspace's window draws a pass or two after it opens, and its first develop starts then: the frame
        // holds no histogram until that develop has landed.
        let state = describe(app);
        if state["previewPending"] == false && !state["histogram"].is_null() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let d = describe(app);
    let open: Vec<_> = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.key).collect();
    panic!(
        "the RAW Develop preview never finished: pending={} histogram={} revision={} detail={} dialogs={open:?} raw_none={}",
        d["previewPending"],
        d["histogram"].is_null(),
        d["previewRevision"],
        d["detailPending"],
        app.raw.is_none()
    );
}

/// Done on the open raw: the request queues it and the workspace's next pass answers it, so the reply is the one the
/// result pump of that frame left.
fn done(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<Value, String> {
    menu(app, ctx, "filter.cameraRaw", &json!({"ui": {"done": true}})).unwrap()?;
    // The workspace reports the request on a pass of its own, and the pump answers it in the frame after.
    for _ in 0..50 {
        frame(app);
        if let Some(reply) = app.raw_reply.take() {
            return reply;
        }
        craft::ui::runtime::pending::finish_jobs(app.ctx(), std::time::Duration::from_secs(30));
    }
    panic!("the frame answered Done")
}

/// A `menu` request followed by the develop it asks for. The reply describes the open dialog
/// once its preview has landed; a request that closed the dialog answers with its own reply.
fn develop(app: &mut PhotocraftApp, ctx: &egui::Context, params: Value) -> Value {
    let reply = menu(app, ctx, "filter.cameraRaw", &params).unwrap().unwrap();
    if app.raw.is_none() {
        return reply;
    }
    settle(app);
    describe(app)
}

#[test]
fn preview_texture_uses_linear_document_display_encoding() {
    let mut doc = photocraft_doc::Document::new("test", photocraft_geom::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F32);
    doc.icc_profile = Some(craft::cms::Builtin::LinearSrgb.profile().to_bytes());
    let mut color = photocraft_engine::color_cmds::ColorState::default();
    color.settings.monitor_profile = "srgb".into();
    let display = color.canvas_display(&doc).unwrap();
    let px = [[0.25, 0.25, 0.25, 1.0]];
    let preview = image(&px, 1, 1, &display);
    let expected = display.to_rgba8(&photocraft_compose::Buffer::filled(Rect::new(0, 0, 1, 1), px[0]));
    assert_eq!(preview.pixels[0].to_array(), expected.pixels[..4]);
    assert_ne!(preview.pixels[0].r(), 64, "linear light must be encoded before display");
}

#[test]
fn histogram_follows_preview_and_view_changes_do_not_render_or_edit() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let initial = develop(&mut app, &ctx, json!({}));
    assert_eq!(initial["histogram"]["samples"], 64 * 48);
    assert_eq!(initial["histogram"]["red"][128], 64 * 48);
    let updated = develop(&mut app, &ctx, json!({"ui": {"set": {"exposure": 1.0}}}));
    assert_ne!(updated["histogram"]["red"], initial["histogram"]["red"]);
    let before = develop(&mut app, &ctx, json!({"ui": {"before": true}}));
    assert_eq!(before["histogram"]["source"], "before");
    assert_eq!(before["histogram"]["red"], initial["histogram"]["red"]);
    assert_eq!(before["previewRevision"], updated["previewRevision"], "Before starts no develop");
    let after = develop(&mut app, &ctx, json!({"ui": {"before": false, "set": {"exposure": 1}}}));
    assert_eq!(after["histogram"], updated["histogram"]);
    assert_eq!(after["previewRevision"], updated["previewRevision"], "the same settings start no develop");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    let pixel = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(0, 0);
    assert!((pixel[0] - 128.0 / 255.0).abs() < 0.0001);
}

/// A layer with tonal variation, a selection over part of it and the dialog open on it.
fn worker_app() -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#406080"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
    open(&mut app, &ctx).unwrap();
    settle(&mut app);
    (app, ctx)
}

/// The histogram of a fresh dialog that develops `settings` in one request: what a develop of
/// exactly those settings gives.
fn single_develop_histogram(settings: Value) -> Value {
    let (mut app, ctx) = worker_app();
    develop(&mut app, &ctx, json!({"ui": {"set": settings}}))["histogram"].clone()
}

/// The dialog's controls develop the proxy off the UI thread (#5964): settings changed while a
/// develop runs are developed next, and the frame shown ends on the latest settings.
#[test]
fn worker_preview_matches_the_synchronous_render_and_ends_on_the_latest_settings() {
    let (mut app, ctx) = worker_app();
    assert_eq!(describe(&app)["hasScopeSelection"], true);
    let revision = describe(&app)["previewRevision"].as_u64().unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
    // The develop of the first request starts; a drag moves on while it runs.
    frame(&mut app);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"highlights": -60, "shadows": 45}}})).unwrap().unwrap();
    settle(&mut app);
    let last = describe(&app);
    assert!(last["previewRevision"].as_u64().unwrap() > revision, "a develop landed");
    assert_eq!(last["previewPending"], false);
    let now = shown(&app);
    assert_eq!((now.exposure, now.highlights, now.shadows), (1.0, -60.0, 45.0));
    // The runtime owns the real job; its published pixels must also match the synchronous proxy calculation.
    let synchronous = session(&app).source.proxy_job(&now).run();
    assert_eq!(processed(&app), synchronous.px, "the real worker develops exactly the same proxy pixels");
    assert_eq!(session(&app).source.developed().unwrap().after_histogram.channels, synchronous.histogram.channels);
    assert!(craft::ui::runtime::pending::finish_jobs(&ctx, std::time::Duration::from_secs(30)), "all actual preview jobs have terminated");
    assert_eq!(
        last["histogram"],
        single_develop_histogram(json!({"exposure": 1.0, "highlights": -60, "shadows": 45})),
        "the frame is a develop of the latest settings"
    );
}

/// An old job held after its real cancellation check. Drop always releases it and drains its queue, including on
/// an assertion failure; all waits have bounds. This never substitutes for the production menu or develop job.
#[cfg(not(target_arch = "wasm32"))]
struct HeldPreview {
    ctx: egui::Context,
    release: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    timed_out: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(not(target_arch = "wasm32"))]
impl HeldPreview {
    fn release_and_finish(&self) -> bool {
        let (released, wake) = &*self.release;
        *released.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        wake.notify_all();
        craft::ui::runtime::pending::finish_jobs(&self.ctx, std::time::Duration::from_secs(30))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for HeldPreview {
    fn drop(&mut self) {
        let _ = self.release_and_finish();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn hold_old_preview(app: &PhotocraftApp) -> (HeldPreview, std::sync::mpsc::Receiver<u64>) {
    let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let timed_out = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (arrived, waiting) = std::sync::mpsc::channel();
    let held = HeldPreview { ctx: app.ctx().clone(), release: release.clone(), timed_out: timed_out.clone() };
    *session(app).source.before_preview_publish.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(move |order, settings| {
        if settings.highlights != -80.0 {
            return;
        }
        let _ = arrived.send(order);
        let (released, wake) = &*release;
        let released = released.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (released, _) =
            wake.wait_timeout_while(released, std::time::Duration::from_secs(30), |released| !*released).unwrap_or_else(std::sync::PoisonError::into_inner);
        if !*released {
            timed_out.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }));
    (held, waiting)
}

/// Observe only the newer publication: `settle` deliberately waits for the older held job too.
#[cfg(not(target_arch = "wasm32"))]
fn wait_for_newer_preview(app: &mut PhotocraftApp, order: u64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        frame(app);
        if session(app)
            .source
            .developed()
            .is_some_and(|result| result.revision > order && result.settings.exposure == -1.0 && result.settings.highlights == 0.0)
        {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "the real newer request must publish while the older job is held");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// An older develop that lands after a newer request must not replace the frame of the newer
/// settings.
#[test]
fn a_stale_worker_never_replaces_a_newer_synchronous_render() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (mut app, ctx) = worker_app();
        let (held, arrived) = hold_old_preview(&app);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"highlights": -80}}})).unwrap().unwrap();
        frame(&mut app);
        let old_order = arrived.recv_timeout(std::time::Duration::from_secs(10)).expect("the real old request passed cancellation and reached publication");
        assert!(session(&app).source.pending(), "the old job remains live at the gate");
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"highlights": 0, "exposure": -1.0}}})).unwrap().unwrap();
        wait_for_newer_preview(&mut app, old_order);
        let landed = describe(&app);
        let shown_pixels = processed(&app);
        assert_eq!(landed["previewPending"], true, "observing the new publication did not release the older job");
        assert!(landed["previewRevision"].as_u64().unwrap() > old_order);
        assert!(held.release_and_finish(), "every actual queued or running develop has terminated");
        assert!(!held.timed_out.load(std::sync::atomic::Ordering::Relaxed), "the old result was released by the test, not its deadline");
        frame(&mut app);
        frame(&mut app);
        let later = describe(&app);
        assert_eq!(later["previewPending"], false);
        assert_eq!(
            (&later["previewRevision"], &later["histogram"]),
            (&landed["previewRevision"], &landed["histogram"]),
            "the old result reached publication last and did not replace the newer frame"
        );
        assert_eq!(processed(&app), shown_pixels, "the old result did not replace the displayed proxy pixels");
        assert_eq!(
            session(&app).source.proxy_job(&shown(&app)).run().px,
            shown_pixels,
            "the pixels still match a synchronous develop of the control channel's settings"
        );
        assert_eq!(landed["histogram"], single_develop_histogram(json!({"exposure": -1.0})), "the frame shown is the one for the latest settings");
    }
    #[cfg(target_arch = "wasm32")]
    {
        let (mut app, ctx) = worker_app();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"highlights": -80}}})).unwrap().unwrap();
        // The develop of the older request is running.
        frame(&mut app);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"highlights": 0, "exposure": -1.0}}})).unwrap().unwrap();
        settle(&mut app);
        let landed = describe(&app);
        assert!(craft::ui::runtime::pending::finish_jobs(&ctx, std::time::Duration::from_secs(30)), "all preview jobs have terminated");
        frame(&mut app);
        frame(&mut app);
        let later = describe(&app);
        assert_eq!((&later["previewRevision"], &later["histogram"]), (&landed["previewRevision"], &landed["histogram"]), "nothing replaced the newer frame");
        assert_eq!(landed["histogram"], single_develop_histogram(json!({"exposure": -1.0})), "the frame shown is the one for the latest settings");
    }
}

#[test]
fn smart_filter_double_click_edits_the_stored_camera_raw_filter_in_its_own_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    // An older editor could save stacked curve inputs; the filter must stay editable.
    let legacy = json!([[0.0, 0.0], [60.0, 40.0], [60.0, 200.0], [255.0, 255.0]]);
    app.run("filter.cameraRaw", json!({"exposure": 1.0, "contrast": 20})).unwrap();
    app.run("layer.smartFilter.setParams", json!({"index": 0, "params": {"pointCurve": legacy}})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let smart = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.clone(),
        _ => panic!("not a smart object"),
    };
    let steps = app.session.active().unwrap().history.past_len();

    let sm = smart(&app);
    crate::smart_ui::open_editor(&mut app, &ctx, layer.0, &sm, 0);
    assert_eq!(app.dialog_count(), 1, "no generic parameter form");
    let opened = develop(&mut app, &ctx, json!({"ui": {}}));
    assert_eq!(opened["smartFilter"], 0);
    assert_eq!(opened["params"]["exposure"], 1.0);
    assert_eq!(opened["params"]["pointCurve"], legacy);
    // The preview starts from the pixels below the filter, not the filtered cache.
    let before = develop(&mut app, &ctx, json!({"ui": {"before": true}}));
    assert_eq!(before["histogram"]["red"][128], 64 * 48);

    // Resetting a control to its default must overwrite the stored value.
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.0}, "before": false, "commit": true}})).unwrap();
    assert!(r.is_ok(), "{r:?}");
    assert!(app.raw.is_none());
    let sm = smart(&app);
    assert_eq!(sm.smart_filters.len(), 1, "edited in place, not added");
    assert_eq!(sm.smart_filters[0].params["exposure"], 0.0);
    assert_eq!(sm.smart_filters[0].params["contrast"], 20.0);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(smart(&app).smart_filters[0].params["exposure"], 1.0);

    // A broken new curve is still rejected; an unknown index is an error, not a panic.
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": layer.0, "index": 0}})).unwrap().unwrap();
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"pointCurve": [[10, 10]]}}})).unwrap().is_err());
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": layer.0, "index": 7}})).unwrap().is_err());
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": "x"}})).unwrap().is_err());
}

#[test]
fn distant_sparse_pixels_cannot_trigger_an_unbounded_proxy_scan() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width":64,"height":48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let mut source = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    source.write_region(Rect::new(30_000, 30_000, 30_001, 30_001), &[0.5, 0.5, 0.5, 1.0]);
    let result = open_pixels(&mut app, &ctx, layer, source, CameraRaw::default(), Coverage::None, Target::NewFilter);
    assert!(result.unwrap_err().contains("bounds are too large"));
    assert!(app.raw.is_none());
}

#[test]
fn dialog_drives_the_engine() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    assert_eq!(r["proxy"], json!([64, 48]));
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "vignetteAmount": -40}}})).unwrap().unwrap();
    assert_eq!(r["params"]["exposure"], 1.0);
    assert_eq!(command_params(&shown(&app)), json!({"exposure": 1.0, "vignetteAmount": -40.0}));
    let before = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 24);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert!(app.raw.is_none());
    let after = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 24);
    assert!(after[0] > before[0] + 0.1, "{before:?} → {after:?}");
    // Cancel leaves the document alone.
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
    assert_eq!(r["cancelled"], true);
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": "x"}}})).unwrap().is_err());
}

std::thread_local! {
    /// Held by a test's thread from its first `raw_app` to the test's end.
    static RAW_OPEN_TURN: std::cell::RefCell<Option<std::sync::MutexGuard<'static, ()>>> = const { std::cell::RefCell::new(None) };
}

/// An app whose importer is the real one, opening a synthetic DNG.
///
/// Raw opens keep their decode in one process-wide list of two (`photocraft_io::raw`), so tests
/// that open raws take turns: a parallel test's open would otherwise evict the decode another
/// test is about to take. The turn is held until the test's thread ends (nextest runs each test
/// in its own process, where this costs nothing).
fn raw_app() -> (Box<PhotocraftApp>, Vec<u8>) {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    RAW_OPEN_TURN.with(|held| {
        let mut held = held.borrow_mut();
        if held.is_none() {
            *held = Some(TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        }
    });
    let services = crate::Services {
        import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
            photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        ..Default::default()
    };
    let (w, h) = (48, 32);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
    spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    // The pixel comparisons below read the developed ProPhoto values: no conversion on open
    // (Camera Raw Defaults › Color space is Adobe RGB otherwise).
    app.run("prefs.set", json!({"path": "rawDefaults.colorSpace", "value": "proPhoto"})).unwrap();
    (app, spec.build())
}

fn mean_red(app: &PhotocraftApp) -> f32 {
    let st = app.session.active().unwrap();
    let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    (0..8).map(|i| s.rgba(4 + i * 4, 16)[0]).sum::<f32>() / 8.0
}

#[test]
fn raw_opens_in_camera_raw_and_open_redevelops() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    let warnings = app.open_bytes("IMG_0001.dng", &dng).unwrap();
    assert!(warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)), "{warnings:?}");
    // A raw has no embedded profile (ProPhoto RGB is the space it is developed into), so Ask
    // When Opening (on by default) has nothing to ask about.
    assert!(!app.dialog_open(), "no profile prompt for a raw");
    let raw = take_queued(&mut app).expect("a raw open queues the dialog");
    open_raw(&mut app, &ctx, raw).unwrap();
    assert_eq!(session(&app).target, Target::OpenRaw);
    assert_eq!(describe(&app)["openingRaw"]["name"], "IMG_0001.dng");
    let before = mean_red(&app);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "temperature": 30, "contrast": 20}}})).unwrap().unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], true, "{r}");
    assert!(r.get("filter").is_some(), "contrast is applied as a RAW Develop step: {r}");
    assert!(app.raw.is_none());
    assert_eq!(app.session.documents().len(), 1);
    let st = app.session.active().unwrap();
    assert_eq!(st.saved_revision, st.revision, "Open leaves the document unmodified");
    assert_eq!(st.doc.depth, photocraft_color::SampleType::U16);
    assert!(mean_red(&app) > before + 0.05, "{before} → {}", mean_red(&app));
}

fn pixels(app: &PhotocraftApp) -> Vec<f32> {
    let st = app.session.active().unwrap();
    let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    (0..32).flat_map(|y| (0..48).map(move |x| (x, y))).flat_map(|(x, y)| s.rgba(x, y)).collect()
}

/// #9432: a raw open decodes the file once. The dialog's preview curve and Open's re-develop
/// reuse the open's sensor data and look, give the pixels of decoding again, and let go of
/// the sensor data when the dialog closes.
#[test]
fn a_raw_open_decodes_the_file_once_and_frees_it_with_the_dialog() {
    use photocraft_io::raw::decodes_on_this_thread;
    let ctx = egui::Context::default();
    let develop = |reuse: bool, commit: bool| {
        let (mut app, mut dng) = raw_app();
        // Kept decodes are found by file contents: other tests open the same synthetic DNG.
        dng.extend_from_slice(b"a_raw_open_decodes_the_file_once");
        let before = decodes_on_this_thread();
        app.open_bytes("IMG_0007.dng", &dng).unwrap();
        let raw = take_queued(&mut app).unwrap();
        open_raw(&mut app, &ctx, raw).unwrap();
        let raw_open = app.raw.as_mut().unwrap().raw_open.as_mut().unwrap();
        let sensor = raw_open.decoded.as_ref().map(std::sync::Arc::downgrade).expect("the dialog holds the open's decode");
        if !reuse {
            raw_open.decoded = None;
        }
        assert!(sensor.upgrade().is_some() || !reuse);
        let ui = if commit { json!({"set": {"exposure": 1.0, "temperature": 30}, "commit": true}) } else { json!({"cancel": true}) };
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": ui})).unwrap().unwrap();
        assert!(app.raw.is_none());
        assert!(sensor.upgrade().is_none(), "the sensor data is freed when the dialog is closed");
        (decodes_on_this_thread() - before, if commit { pixels(&app) } else { Vec::new() })
    };
    let (decodes, reused) = develop(true, true);
    assert_eq!(decodes, 1, "open, dialog and commit share one decode");
    let (decodes, again) = develop(false, true);
    assert_eq!(decodes, 2, "without the kept decode the commit decodes again");
    assert_eq!(reused, again, "the same pixels either way");
    assert_eq!(develop(true, false).0, 1, "Cancel decodes nothing more");
}

#[test]
fn the_demosaic_method_is_chosen_at_open() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.open_bytes("IMG_0002.dng", &dng).unwrap();
    let ahd = pixels(&app);
    let raw = take_queued(&mut app).unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    let d = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"demosaic": "bilinear"}})).unwrap().unwrap();
    assert_eq!(d["openingRaw"]["demosaic"], "bilinear");
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"demosaic": "amaze"}})).unwrap().is_err());
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], true, "another method re-develops: {r}");
    assert!(r.get("filter").is_none(), "nothing else to apply: {r}");
    let bilinear = pixels(&app);
    assert!(ahd.iter().zip(&bilinear).any(|(a, b)| (a - b).abs() > 1e-3), "bilinear differs from AHD");
    let st = app.session.active().unwrap();
    assert_eq!(st.saved_revision, st.revision);

    // The preference sets the method of the open itself; Open with it changes nothing.
    app.run("prefs.set", json!({"path": "rawDefaults.demosaic", "value": "bilinear"})).unwrap();
    app.open_bytes("IMG_0003.dng", &dng).unwrap();
    assert_eq!(pixels(&app), bilinear, "developed with the preference's method");
    let raw = take_queued(&mut app).unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    assert_eq!(describe(&app)["openingRaw"]["demosaic"], "bilinear");
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], false, "{r}");

    // Only the open-time dialog has the choice.
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"demosaic": "ahd"}})).unwrap().is_err());

    // The preference may change while a background open runs: the dialog starts from the
    // method the open developed with (its note), not from the newer preference.
    let warnings = app.open_bytes("IMG_0004.dng", &dng).unwrap();
    take_queued(&mut app);
    app.run("prefs.set", json!({"path": "rawDefaults.demosaic", "value": "ahd"})).unwrap();
    let document = app.session.active().unwrap().doc.id;
    queue_open_dialog(&mut app, document, "IMG_0004.dng", None, Some(&std::sync::Arc::new(dng.clone())), &warnings).unwrap();
    let raw = take_queued(&mut app).unwrap();
    use photocraft_io::raw::Demosaic;
    assert_eq!((raw.developed_with, raw.demosaic), (Demosaic::Bilinear, Demosaic::Bilinear));
}

#[test]
fn shadows_of_a_raw_open_act_on_the_raw_data_and_the_preview_shows_it() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.open_bytes("IMG_0002.dng", &dng).unwrap();
    let raw = take_queued(&mut app).unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    settle(&mut app);
    assert!(knows_raw_curve(&app), "the preview knows how the raw was developed");
    let mean = |px: &[[f32; 4]]| px.iter().map(|p| p[0] + p[1] + p[2]).sum::<f32>() / px.len() as f32;
    let preview_before = mean(&processed(&app));
    let before = mean_red(&app);
    develop(&mut app, &ctx, json!({"ui": {"set": {"shadows": 100}}}));
    let after = mean(&processed(&app));
    assert!(after > preview_before + 0.01, "{preview_before} → {after}");
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], true, "{r}");
    assert!(r.get("filter").is_none(), "Shadows acts while developing, not as a later filter step: {r}");
    assert!(mean_red(&app) > before + 0.01, "{before} → {}", mean_red(&app));
}

#[test]
fn clarity_texture_and_dehaze_of_a_raw_open_act_on_the_raw_data() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.open_bytes("IMG_0003.dng", &dng).unwrap();
    let raw = take_queued(&mut app).unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    settle(&mut app);
    let before = mean_red(&app);
    let preview_before = processed(&app);
    develop(&mut app, &ctx, json!({"ui": {"set": {"clarity": 60, "texture": 40, "dehaze": 50}}}));
    assert_ne!(processed(&app), preview_before, "the preview shows the raw-stage change");
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], true, "{r}");
    assert!(r.get("filter").is_none(), "they act while developing, not as a later filter step: {r}");
    assert!(mean_red(&app) < before, "Dehaze darkens: {before} → {}", mean_red(&app));
}

#[test]
fn background_redevelop_keeps_the_tab_position_and_view() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 10, "height": 10})).unwrap();
    app.open_bytes("mid.dng", &dng).unwrap();
    app.run("file.new", json!({"width": 12, "height": 12})).unwrap();
    app.sync_views();
    let raw = take_queued(&mut app).unwrap();
    let old_id = raw.document;
    assert_eq!(document_index(&app, old_id), Some(1));
    app.ui.views[1].zoom = 3.0;
    open_raw(&mut app, &ctx, raw).unwrap();
    let before = mean_red(&app);
    // The re-develop and the Camera Raw step run as jobs; the UI thread only polls.
    app.background_jobs = true;
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "contrast": 20}}})).unwrap().unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["pending"], true, "{r}");
    assert!(app.raw.is_none());
    for _ in 0..3000 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if app.raw_redevelop.is_empty() && !app.session.has_jobs() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.raw_redevelop.is_empty() && !app.session.has_jobs(), "the jobs never finished");
    let docs = app.session.documents();
    assert_eq!(docs.len(), 3);
    assert_eq!((docs[0].doc.size.width, docs[2].doc.size.width), (10, 12), "the other tabs keep their places");
    assert_ne!(docs[1].doc.id, old_id, "re-developed");
    assert_eq!(docs[1].doc.depth, photocraft_color::SampleType::U16);
    assert_eq!(docs[1].saved_revision, docs[1].revision, "Open leaves the document unmodified");
    assert_eq!(app.session.active_index(), Some(1));
    assert_eq!(app.ui.views[1].zoom, 3.0, "the view carries over to the re-developed document");
    assert!(mean_red(&app) > before + 0.05, "{before} → {}", mean_red(&app));
}

/// #7789: Preferences › Raw Defaults › Resolution sets a raw's ppi when the app opens it
/// itself (no background job) and from automation, and a Camera Raw re-develop keeps it.
#[test]
fn a_raw_opens_and_redevelops_at_the_raw_defaults_resolution() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.run("prefs.set", json!({"path": "rawDefaults.resolution", "value": 240})).unwrap();
    app.open_bytes("IMG_0004.dng", &dng).unwrap();
    assert_eq!(app.session.active().unwrap().doc.resolution_dpi, 240.0);
    let raw = take_queued(&mut app).unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["redeveloped"], true, "{r}");
    assert_eq!(app.session.active().unwrap().doc.resolution_dpi, 240.0, "kept by the re-develop");
    app.open_automation_bytes("IMG_0005.dng", &dng).unwrap();
    assert_eq!(app.session.active().unwrap().doc.resolution_dpi, 240.0, "automation open");
}

/// Raws opened together share the dialog in a filmstrip, each with its own settings; Open
/// develops them all, Cancel closes them all. A filmstrip pick reaches the app as the
/// workspace's `Show` event, which `take` answers with `show_raw`; the trial calls that answer.
#[test]
fn several_raws_share_the_dialog_and_keep_their_own_settings() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.open_bytes("one.dng", &dng).unwrap();
    app.open_bytes("two.dng", &dng).unwrap();
    app.open_bytes("three.dng", &dng).unwrap();
    assert_eq!(app.raw_shelf.len(), 3);
    assert!(app.raw_shelf.iter().all(|h| h.thumb.is_some()), "each has a thumbnail");
    let ids: Vec<_> = app.raw_shelf.iter().map(|h| h.raw.document).collect();
    filmstrip::open_held(&mut app, &ctx, 0).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
    filmstrip::show_raw(&mut app, &ctx, 1).unwrap();
    let d = session(&app);
    let open = d.raw_open.as_ref().unwrap();
    assert_eq!((open.document, open.name.as_str(), shown(&app).exposure), (ids[1], "two.dng", 0.0));
    assert_eq!(app.raw_shelf[0].kept.as_ref().map(|(p, _)| p.exposure), Some(1.0), "the first keeps its settings");
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 30}}})).unwrap().unwrap();
    filmstrip::show_raw(&mut app, &ctx, 0).unwrap();
    assert_eq!(shown(&app).exposure, 1.0, "back to the first, as it was left");
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["opened"].as_array().map(Vec::len), Some(3), "{r}");
    assert!(app.raw_shelf.is_empty() && app.raw.is_none());
    assert_eq!(app.session.documents().len(), 3, "the never-shown third opens too");
    assert!(app.session.documents().iter().all(|st| st.saved_revision == st.revision), "Open leaves them unmodified");
    // Cancel closes every raw in the filmstrip.
    app.open_bytes("four.dng", &dng).unwrap();
    app.open_bytes("five.dng", &dng).unwrap();
    filmstrip::open_held(&mut app, &ctx, 0).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
    assert!(app.raw_shelf.is_empty());
    assert_eq!(app.session.documents().len(), 3);
}

/// A raw whose Open fails stays in the filmstrip with its settings, and the dialog comes back
/// for it; the raws that opened leave it.
#[test]
fn a_raw_that_fails_to_open_stays_in_the_filmstrip_with_its_settings() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.open_bytes("one.dng", &dng).unwrap();
    app.open_bytes("two.dng", &dng).unwrap();
    app.open_bytes("three.dng", &dng).unwrap();
    let ids: Vec<_> = app.raw_shelf.iter().map(|h| h.raw.document).collect();
    // Two's file can no longer be developed, and it was left with its own settings.
    let mut kept = cr!(exposure: 0.5);
    kept.contrast = 20.0;
    app.raw_shelf[1].raw.bytes = Some(std::sync::Arc::new(b"not a raw file".to_vec()));
    app.raw_shelf[1].kept = Some((kept.clone(), CameraRaw::default()));
    filmstrip::open_held(&mut app, &ctx, 0).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["opened"].as_array().map(Vec::len), Some(2), "one and three open: {r}");
    let left: Vec<_> = app.raw_shelf.iter().map(|h| h.raw.document).collect();
    assert_eq!(left, vec![ids[1]], "only the failed raw stays in the filmstrip");
    assert_eq!(app.raw_shelf[0].kept.as_ref().map(|(p, _)| p), Some(&kept), "with its settings");
    // The dialog comes back for it with them; when every raw fails, the dialog and its
    // filmstrip stay.
    filmstrip::open_held(&mut app, &ctx, 0).unwrap();
    let current = shown(&app);
    assert_eq!((current.exposure, current.contrast), (0.5, 20.0));
    app.open_bytes("four.dng", &dng).unwrap();
    app.raw_shelf[1].raw.bytes = Some(std::sync::Arc::new(b"not a raw file either".to_vec()));
    app.raw_shelf[1].kept = Some((kept.clone(), CameraRaw::default()));
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().is_err());
    assert_eq!(app.raw_shelf.len(), 2, "both raws stay in the filmstrip");
    assert!(app.raw.is_some(), "and the dialog stays open");
    // Cancel still closes every raw in it.
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
    assert!(app.raw_shelf.is_empty());
    assert_eq!(app.session.documents().len(), 2, "one and three stay open");
}

/// A raw that can't be shown leaves things as they were: the dialog stays on the raw being left,
/// and the other keeps its place in the filmstrip with its settings.
#[test]
fn a_raw_that_cannot_be_shown_keeps_the_dialog_and_the_filmstrip() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.open_bytes("one.dng", &dng).unwrap();
    app.open_bytes("two.dng", &dng).unwrap();
    let ids: Vec<_> = app.raw_shelf.iter().map(|h| h.raw.document).collect();
    let kept = cr!(exposure: 0.5);
    app.raw_shelf[1].kept = Some((kept.clone(), CameraRaw::default()));
    filmstrip::open_held(&mut app, &ctx, 0).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
    let two = document_index(&app, ids[1]).unwrap();
    app.session.close(two);
    assert!(filmstrip::show_raw(&mut app, &ctx, 1).is_err());
    assert!(app.raw.is_some(), "the dialog stays");
    assert_eq!((session(&app).raw_open.as_ref().unwrap().document, shown(&app).exposure), (ids[0], 1.0));
    assert_eq!(app.raw_shelf.len(), 2);
    assert_eq!(app.raw_shelf[1].kept.as_ref().map(|(p, _)| p), Some(&kept));
}

/// Leaving a raw before its preview has caught up: its thumbnail shows the settings it keeps, not
/// the frame before them. The thumbnail is the app's `raw_shelf[..].thumb`; `set_item_picture`
/// only forwards it to the workspace's tile.
#[test]
fn a_raw_left_before_its_preview_lands_gets_a_thumbnail_of_its_settings() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.open_bytes("one.dng", &dng).unwrap();
    app.open_bytes("two.dng", &dng).unwrap();
    filmstrip::open_held(&mut app, &ctx, 0).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 2.0}}})).unwrap().unwrap();
    // The develop of exposure 2 has not started: the thumbnail is made from the settings.
    filmstrip::keep_settings(&mut app);
    let thumb = app.raw_shelf[0].thumb.clone().expect("a thumbnail");
    settle(&mut app);
    filmstrip::keep_settings(&mut app);
    assert!(thumb == app.raw_shelf[0].thumb.clone().unwrap(), "the thumbnail matches a finished develop of the same settings");
}

/// Open returns once a background develop has started; if that develop then fails, the raw comes
/// back to the filmstrip with its settings, as one that failed at once stays there.
#[test]
fn a_raw_whose_background_develop_fails_comes_back_to_the_filmstrip() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.open_bytes("one.dng", &dng).unwrap();
    app.open_bytes("two.dng", &dng).unwrap();
    let ids: Vec<_> = app.raw_shelf.iter().map(|h| h.raw.document).collect();
    let (kept, from) = (cr!(exposure: 0.5, contrast: 20.0), cr!(contrast: 5.0));
    app.raw_shelf[1].raw.bytes = Some(std::sync::Arc::new(b"not a raw file".to_vec()));
    app.raw_shelf[1].kept = Some((kept.clone(), from.clone()));
    filmstrip::open_held(&mut app, &ctx, 0).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
    app.background_jobs = true;
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["opened"].as_array().map(Vec::len), Some(2), "both started: {r}");
    assert!(app.raw_shelf.is_empty());
    for _ in 0..3000 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if app.raw_redevelop.is_empty() && !app.session.has_jobs() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.raw_redevelop.is_empty() && !app.session.has_jobs(), "the jobs never finished");
    let left: Vec<_> = app.raw_shelf.iter().map(|h| h.raw.document).collect();
    assert_eq!(left, vec![ids[1]], "only the failed raw comes back");
    assert_eq!(app.raw_shelf[0].kept, Some((kept, from)), "with its settings");
    app.background_jobs = false;
    filmstrip::open_held(&mut app, &ctx, 0).unwrap();
    assert_eq!(shown(&app).contrast, 20.0);
}

/// Camera Raw Defaults' Auto reads the same pixels whether a raw was shown or not, so Open gives
/// it the same settings either way: a fine pattern that a sparse sample misreads shows it.
#[test]
fn auto_tone_of_a_never_shown_raw_matches_the_dialogs() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    app.run("prefs.set", json!({"path": "rawDefaults.applyAutoTone", "value": true})).unwrap();
    app.run("file.new", json!({"width": 1800, "height": 1200})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let st = app.session.active_mut().unwrap();
    let id = st.doc.layers.iter().find(|l| l.surface().is_some()).unwrap().id;
    st.active_layer = Some(id);
    let document = st.doc.id;
    let Some(photocraft_doc::LayerContent::Raster(s)) = std::sync::Arc::make_mut(&mut st.doc).layer_mut(id).map(|l| &mut l.content) else {
        panic!("a pixel layer")
    };
    let format = s.format();
    let (dark, light) = (photocraft_raster::from_rgba(&format, [0.1, 0.1, 0.1, 1.0]), photocraft_raster::from_rgba(&format, [0.9, 0.9, 0.9, 1.0]));
    for y in 0..1200 {
        for x in 0..1800 {
            s.write_pixel(x, y, if (x + y) % 2 == 0 { &dark } else { &light });
        }
    }
    let method = photocraft_io::raw::Demosaic::Ahd;
    let raw = RawOpen { document, name: "pattern.dng".into(), path: None, bytes: None, decoded: None, developed_with: method, demosaic: method, crop: None };
    let (unshown, _) = filmstrip::first_settings(&app, &raw);
    open_raw(&mut app, &ctx, raw).unwrap();
    let current = shown(&app);
    let tone = |p: &CameraRaw| [p.exposure, p.contrast, p.highlights, p.shadows, p.whites, p.blacks];
    assert_eq!(tone(&unshown), tone(&current));
}

#[test]
fn cancel_closes_the_raw_and_the_preference_skips_the_dialog() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.open_bytes("a.dng", &dng).unwrap();
    let raw = take_queued(&mut app).unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
    assert_eq!(r["cancelled"], true);
    assert!(app.session.documents().is_empty(), "Cancel closes the raw being opened");
    // Unchanged settings: Open keeps the default develop and adds no step.
    app.open_bytes("b.dng", &dng).unwrap();
    let raw = take_queued(&mut app).unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!((r["document"].as_u64(), r["redeveloped"].as_bool(), r.get("filter")), (Some(0), Some(false), None), "{r}");
    assert_eq!(r.get("defaults"), None, "ProPhoto, 16-bit: no RAW Develop Default to apply: {r}");
    // Preference off: raws open straight away.
    app.run("prefs.set", json!({"path": "rawDefaults.openInCameraRaw", "value": false})).unwrap();
    app.open_bytes("c.dng", &dng).unwrap();
    assert!(app.raw_shelf.is_empty());
    // A non-raw never queues the dialog; a closed document is an error, not a panic.
    assert!(
        open_raw(
            &mut app,
            &ctx,
            RawOpen {
                document: photocraft_doc::DocId(u64::MAX),
                name: "x".into(),
                path: None,
                bytes: None,
                developed_with: Default::default(),
                demosaic: Default::default(),
                decoded: None,
                crop: None
            }
        )
        .is_err()
    );
}

#[test]
fn calibration_changes_the_preview_and_round_trips_through_xmp() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.open_bytes("IMG_0005.dng", &dng).unwrap();
    let raw = take_queued(&mut app).unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    settle(&mut app);
    let before = processed(&app);
    develop(&mut app, &ctx, json!({"ui": {"set": {"shadowTint": 100, "calibrationHue": [100, 0, 0], "calibrationSat": [0, 0, -100]}}}));
    assert!(
        processed(&app).iter().zip(&before).any(|(a, b)| (a[0] - b[0]).abs() > 0.01 || (a[1] - b[1]).abs() > 0.01),
        "the Calibration panel changes the preview"
    );
    let xmp = photocraft_io::camera_raw_xmp::save(&shown(&app));
    assert!(xmp.contains("crs:ShadowTint=\"+100\"") && xmp.contains("crs:RedHue=\"+100\"") && xmp.contains("crs:BlueSaturation=\"-100\""), "{xmp}");
    let mut back = CameraRaw::default();
    photocraft_io::camera_raw_xmp::load(&mut back, xmp.as_bytes()).unwrap();
    assert_eq!((back.shadow_tint, back.calibration_hue, back.calibration_sat), (100.0, [100.0, 0.0, 0.0], [0.0, 0.0, -100.0]));
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"auto": "yes"}})).unwrap().is_err(), "auto must be a boolean");
}

#[test]
fn auto_sets_the_light_sliders_from_the_image() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.open_bytes("IMG_0006.dng", &dng).unwrap();
    let raw = take_queued(&mut app).unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    assert!(shown(&app).is_identity());
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"auto": true}})).unwrap().unwrap();
    let p = shown(&app);
    assert!(!p.is_identity(), "Auto chose something: {p:?}");
    assert!(p.saturation == 0.0 && p.hsl_hue == [0.0; 8], "only the Light panel: {p:?}");
    // The preference does the same when a raw opens without a sidecar.
    app.run("prefs.set", json!({"path": "rawDefaults.applyAutoTone", "value": true})).unwrap();
    app.open_bytes("IMG_0007.dng", &dng).unwrap();
    let raw = take_queued(&mut app).unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    assert_eq!(shown(&app), p, "Apply auto tone opens the dialog with Auto's settings");
}

#[test]
fn a_sidecar_fills_the_dialog_and_open_writes_it_back() {
    use std::sync::{Arc, Mutex};
    let (mut app, dng) = raw_app();
    let dir = std::env::temp_dir().join(format!("photocraft-sidecar-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let raw_path = dir.join("IMG_0008.cr2").to_string_lossy().into_owned();
    std::fs::write(&raw_path, &dng).unwrap();
    assert_eq!(sidecar_path(&raw_path), dir.join("IMG_0008.xmp").to_string_lossy());
    assert_eq!(sidecar_path("/a/b.c/noext"), "/a/b.c/noext.xmp");
    std::fs::write(
        sidecar_path(&raw_path),
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"PhotoCraft\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\" crs:Exposure2012=\"+1.50\" crs:ShadowTint=\"20\" crs:Temperature=\"5500\"/></rdf:RDF></x:xmpmeta>",
    )
    .unwrap();
    type Written = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
    let written: Written = Arc::default();
    let sink = written.clone();
    app.services.write = Some(Box::new(move |p: &str, b: &[u8]| {
        sink.lock().unwrap().push((p.to_string(), b.to_vec()));
        Ok(())
    }));
    let ctx = app.ctx().clone();
    app.open_bytes("IMG_0008.cr2", &dng).unwrap();
    let mut raw = take_queued(&mut app).unwrap();
    raw.path = Some(raw_path.clone());
    open_raw(&mut app, &ctx, raw).unwrap();
    let current = shown(&app);
    assert_eq!((current.exposure, current.shadow_tint), (1.5, 20.0), "the sidecar fills the dialog");
    assert!(
        app.ui.status.contains("IMG_0008.xmp") && app.ui.status.contains("Temperature"),
        "the absolute white balance is reported as not supported: {}",
        app.ui.status
    );
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 15}}})).unwrap().unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    let w = written.lock().unwrap().clone();
    assert_eq!(w.len(), 1, "Open writes the sidecar once: {:?}", w.iter().map(|(p, _)| p).collect::<Vec<_>>());
    assert_eq!(w[0].0, sidecar_path(&raw_path));
    let text = String::from_utf8(w[0].1.clone()).unwrap();
    assert!(text.contains("crs:Exposure2012=\"+1.50\"") && text.contains("crs:Contrast2012=\"+15\"") && text.contains("crs:ShadowTint=\"+20\""), "{text}");
    // A DNG keeps its settings inside: no sidecar beside it. And the preference turns writing off.
    written.lock().unwrap().clear();
    let dng_path = dir.join("IMG_0009.dng").to_string_lossy().into_owned();
    std::fs::write(&dng_path, &dng).unwrap();
    app.open_bytes("IMG_0009.dng", &dng).unwrap();
    let mut raw = take_queued(&mut app).unwrap();
    raw.path = Some(dng_path);
    open_raw(&mut app, &ctx, raw).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 15}, "commit": true}})).unwrap().unwrap();
    assert!(written.lock().unwrap().is_empty(), "no sidecar for a DNG");
    app.run("prefs.set", json!({"path": "rawDefaults.writeXmpSidecars", "value": false})).unwrap();
    app.open_bytes("IMG_0008.cr2", &dng).unwrap();
    let mut raw = take_queued(&mut app).unwrap();
    raw.path = Some(raw_path.clone());
    open_raw(&mut app, &ctx, raw).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 30}, "commit": true}})).unwrap().unwrap();
    assert!(written.lock().unwrap().is_empty(), "the preference stops the write");
    // Without the dialog, the sidecar still develops the file.
    app.run("prefs.set", json!({"path": "rawDefaults.openInCameraRaw", "value": false})).unwrap();
    let before = app.session.documents().len();
    app.open_file(&raw_path, &dng).unwrap();
    assert!(app.raw_shelf.is_empty());
    assert_eq!(app.session.documents().len(), before + 1);
    let st = app.session.active().unwrap();
    assert_eq!(st.saved_revision, st.revision, "opening with a sidecar leaves the document unmodified");
    assert!(mean_red(&app) > 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn camera_raw_defaults_apply_when_a_raw_opens() {
    let (mut app, dng) = raw_app();
    app.run("prefs.set", json!({"values": {"rawDefaults.openInCameraRaw": false, "rawDefaults.colorSpace": "srgb", "rawDefaults.bitDepth": "8", "rawDefaults.resolution": 240.0, "rawDefaults.sharpenFor": "screen", "rawDefaults.openAsSmartObject": true}})).unwrap();
    app.open_bytes("IMG_0010.dng", &dng).unwrap();
    assert!(app.raw_shelf.is_empty());
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.depth, photocraft_color::SampleType::U8, "Bit depth 8");
    assert_eq!(st.doc.resolution_dpi, 240.0, "Resolution");
    let profile = st.doc.icc_profile.as_deref().and_then(|p| craft::cms::Profile::parse(p).ok()).map(|p| p.description).unwrap_or_default();
    assert!(profile.to_lowercase().contains("srgb"), "Color space: {profile}");
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert!(matches!(layer.content, photocraft_doc::LayerContent::Smart(_)), "Open as Smart Object");
    assert_eq!(st.saved_revision, st.revision, "the defaults are part of opening: unmodified");
    // The dialog's Open applies them too, and reports what it applied.
    app.run("prefs.set", json!({"values": {"rawDefaults.openInCameraRaw": true, "rawDefaults.openAsSmartObject": false, "rawDefaults.bitDepth": "16"}}))
        .unwrap();
    let ctx = app.ctx().clone();
    app.open_bytes("IMG_0011.dng", &dng).unwrap();
    let raw = take_queued(&mut app).unwrap();
    open_raw(&mut app, &ctx, raw).unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["defaults"], json!(["sharpenFor", "colorSpace"]), "{r}");
    assert!(r.get("filter").is_some(), "Sharpen For is a RAW Develop step: {r}");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.depth, photocraft_color::SampleType::U16);
    assert_eq!(st.saved_revision, st.revision);
}

type Written = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

/// A sidecar as Lightroom Classic leaves one beside a raw: its own toolkit, a crop, a rating
/// and a keyword next to the develop settings (values synthetic).
const LIGHTROOM_SIDECAR: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0-c000 1.000000, 0000/00/00-00:00:00        ">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   xmp:Rating="4"
   crs:Exposure2012="+1.00"
   crs:Sharpness="40"
   crs:HasCrop="True"
   crs:CropTop="0.1">
   <dc:subject><rdf:Bag><rdf:li>harbour</rdf:li></rdf:Bag></dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
"#;

/// `raw_app` whose `write` service records what it is asked to write, and a raw file in a fresh
/// folder with `sidecar` beside it when given. Returns the app, the raw's bytes, the folder,
/// the raw's path and the recorded writes.
fn raw_with_sidecar(tag: &str, sidecar: Option<&str>) -> (Box<PhotocraftApp>, Vec<u8>, std::path::PathBuf, String, Written) {
    let (mut app, dng) = raw_app();
    let dir = std::env::temp_dir().join(format!("photocraft-sidecar-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let raw_path = dir.join("IMG_0100.cr2").to_string_lossy().into_owned();
    std::fs::write(&raw_path, &dng).unwrap();
    if let Some(text) = sidecar {
        std::fs::write(sidecar_path(&raw_path), text).unwrap();
    }
    let written: Written = Default::default();
    let sink = written.clone();
    app.services.write = Some(Box::new(move |p: &str, b: &[u8]| {
        sink.lock().unwrap().push((p.to_string(), b.to_vec()));
        Ok(())
    }));
    (app, dng, dir, raw_path, written)
}

/// Opens the raw at `raw_path` in the open-time dialog, as File › Open does.
fn open_dialog_at(app: &mut PhotocraftApp, ctx: &egui::Context, dng: &[u8], raw_path: &str) {
    app.open_bytes("IMG_0100.cr2", dng).unwrap();
    let mut raw = take_queued(app).unwrap();
    raw.path = Some(raw_path.to_string());
    open_raw(app, ctx, raw).unwrap();
}

/// The notices about the sidecar (the open's own import notes aside).
fn notice_titles(app: &PhotocraftApp) -> Vec<String> {
    app.ui.notices.iter().map(|n| n.title.clone()).filter(|t| t.contains(".xmp")).collect()
}

/// A Lightroom or Bridge sidecar holds settings PhotoCraft can't keep (crop, masks, rating,
/// keywords): Open uses its develop settings but never writes over it, and says so.
#[test]
fn a_foreign_sidecar_is_never_overwritten() {
    let (mut app, dng, dir, raw_path, written) = raw_with_sidecar("foreign", Some(LIGHTROOM_SIDECAR));
    let ctx = app.ctx().clone();
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    assert_eq!(shown(&app).exposure, 1.0, "the sidecar's settings fill the dialog");
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 15}}})).unwrap().unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert!(written.lock().unwrap().is_empty(), "nothing written over Lightroom's sidecar");
    assert_eq!(std::fs::read_to_string(sidecar_path(&raw_path)).unwrap(), LIGHTROOM_SIDECAR);
    let titles = notice_titles(&app);
    assert!(titles.iter().any(|t| t.contains("IMG_0100.xmp") && t.contains("another application")), "{titles:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bridge writes a sidecar with only a rating (no Camera Raw settings): it is not PhotoCraft's
/// to replace, with or without a change in the dialog.
#[test]
fn a_rating_only_sidecar_is_left_alone() {
    let bridge = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="5"/></rdf:RDF></x:xmpmeta>"#;
    let (mut app, dng, dir, raw_path, written) = raw_with_sidecar("rating", Some(bridge));
    let ctx = app.ctx().clone();
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert!(written.lock().unwrap().is_empty(), "Open with no change writes nothing");
    assert!(notice_titles(&app).is_empty(), "and has nothing to report: {:?}", notice_titles(&app));
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 10}, "commit": true}})).unwrap().unwrap();
    assert!(written.lock().unwrap().is_empty(), "nor with a change");
    assert_eq!(std::fs::read_to_string(sidecar_path(&raw_path)).unwrap(), bridge);
    let _ = std::fs::remove_dir_all(&dir);
}

/// #9985: the crop of a Lightroom sidecar crops the opened raw (with the dialog's Open and
/// without the dialog), and neither its edges nor the raw's file name are named as unsupported.
/// A straightened crop isn't applied and is named. PhotoCraft's own sidecar keeps the crop.
#[test]
fn a_sidecar_crop_crops_the_opened_raw() {
    let lightroom = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:RawFileName="IMG_0100.cr2" crs:HasCrop="True" crs:CropTop="0.25" crs:CropLeft="0.125" crs:CropBottom="0.75" crs:CropRight="0.625" crs:CropAngle="0"/></rdf:RDF></x:xmpmeta>"#;
    let (mut app, dng, dir, raw_path, written) = raw_with_sidecar("crop", Some(lightroom));
    let ctx = app.ctx().clone();
    let size = |app: &PhotocraftApp| app.session.active().map(|st| (st.doc.size.width, st.doc.size.height));
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    assert!(app.ui.status.contains("IMG_0100.xmp") && !app.ui.status.contains("not supported"), "{}", app.ui.status);
    assert_eq!(size(&app), Some((48, 32)), "the dialog previews the whole frame");
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(size(&app), Some((24, 16)), "Open crops the 48x32 frame to x 6..30, y 8..24");
    let st = app.session.active().unwrap();
    assert_eq!(st.saved_revision, st.revision, "the crop is part of opening: unmodified");
    assert!(written.lock().unwrap().is_empty(), "Lightroom's sidecar is kept");
    app.run("prefs.set", json!({"path": "rawDefaults.openInCameraRaw", "value": false})).unwrap();
    app.open_file(&raw_path, &dng).unwrap();
    assert_eq!(size(&app), Some((24, 16)), "without the dialog too");
    app.run("prefs.set", json!({"path": "rawDefaults.openInCameraRaw", "value": true})).unwrap();
    // Straightened: the rectangle is measured on the turned frame, so nothing is cropped.
    std::fs::write(sidecar_path(&raw_path), lightroom.replace(r#"crs:CropAngle="0""#, r#"crs:CropAngle="1.38""#)).unwrap();
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    assert!(app.ui.status.contains("CropAngle") && !app.ui.status.contains("RawFileName"), "{}", app.ui.status);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(size(&app), Some((48, 32)));
    // PhotoCraft's own sidecar: a changed setting rewrites it, with the crop it was opened with.
    let crop = photocraft_io::camera_raw_xmp::Crop { top: 0.25, left: 0.125, bottom: 0.75, right: 0.625 };
    std::fs::write(sidecar_path(&raw_path), photocraft_io::camera_raw_xmp::save_with_crop(&CameraRaw::default(), Some(crop))).unwrap();
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 15}, "commit": true}})).unwrap().unwrap();
    assert_eq!(size(&app), Some((24, 16)));
    let w = written.lock().unwrap().clone();
    assert_eq!(w.len(), 1);
    let mut back = CameraRaw::default();
    let r = photocraft_io::camera_raw_xmp::load(&mut back, &w[0].1).unwrap();
    assert_eq!((back.contrast, r.crop), (15.0, Some(crop)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// #9985 with the filmstrip: raws opened together each open with their own sidecar's crop, also
/// the one that is never shown and the one that was shown and left.
#[test]
fn filmstrip_raws_open_with_their_sidecar_crops() {
    let lightroom = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:HasCrop="True" crs:CropTop="0.25" crs:CropLeft="0.125" crs:CropBottom="0.75" crs:CropRight="0.625" crs:CropAngle="0"/></rdf:RDF></x:xmpmeta>"#;
    let (mut app, dng, dir, raw_path, _) = raw_with_sidecar("filmcrop", Some(lightroom));
    let paths: Vec<String> = (1..3).map(|i| dir.join(format!("IMG_010{i}.cr2")).to_string_lossy().into_owned()).collect();
    for p in &paths {
        std::fs::write(p, &dng).unwrap();
        std::fs::write(sidecar_path(p), lightroom).unwrap();
    }
    let ctx = app.ctx().clone();
    app.open_file(&raw_path, &dng).unwrap();
    for p in &paths {
        app.open_file(p, &dng).unwrap();
    }
    assert_eq!(app.raw_shelf.len(), 3);
    filmstrip::open_held(&mut app, &ctx, 0).unwrap();
    filmstrip::show_raw(&mut app, &ctx, 1).unwrap();
    filmstrip::show_raw(&mut app, &ctx, 0).unwrap();
    let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(r["opened"].as_array().map(Vec::len), Some(3), "{r}");
    let sizes: Vec<_> = app.session.documents().iter().map(|st| (st.doc.size.width, st.doc.size.height)).collect();
    assert_eq!(sizes, vec![(24, 16); 3], "shown, shown and left, and never shown: all cropped");
    let _ = std::fs::remove_dir_all(&dir);
}

/// With Open in Camera Raw off, opening a raw develops it with its sidecar and writes nothing
/// back: nothing changed.
#[test]
fn opening_without_the_dialog_does_not_rewrite_the_sidecar() {
    let (mut app, dng, dir, raw_path, written) = raw_with_sidecar("nodialog", Some(LIGHTROOM_SIDECAR));
    app.run("prefs.set", json!({"path": "rawDefaults.openInCameraRaw", "value": false})).unwrap();
    app.open_file(&raw_path, &dng).unwrap();
    assert!(app.raw_shelf.is_empty());
    assert_eq!(app.session.documents().len(), 1);
    assert!(written.lock().unwrap().is_empty(), "just opening never writes the sidecar");
    assert_eq!(std::fs::read_to_string(sidecar_path(&raw_path)).unwrap(), LIGHTROOM_SIDECAR);
    assert!(notice_titles(&app).is_empty(), "{:?}", notice_titles(&app));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A control or MCP request has no authority to write beside the user's files: its commit
/// develops the raw but writes no sidecar. The same Open from the dialog does.
#[test]
fn a_control_request_never_writes_the_sidecar() {
    fn allow(_: &str, _: &Value) -> photocraft_engine::Result<()> {
        Ok(())
    }
    let (mut app, dng, dir, raw_path, written) = raw_with_sidecar("control", None);
    app.services.automation_authorize = Some(allow);
    let ctx = app.ctx().clone();
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    let commit = json!({"id": "filter.cameraRaw", "params": {"ui": {"set": {"contrast": 15}, "commit": true}}});
    let (req, _rx) = crate::control::ControlRequest::new("ui.menu.invoke", commit);
    let crate::control::Reply::Done(reply) = crate::control::handle(&mut app, &ctx, &req) else { panic!("expected a reply") };
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(app.raw.is_none(), "committed");
    assert!(written.lock().unwrap().is_empty(), "no sidecar from a control request");
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 15}, "commit": true}})).unwrap().unwrap();
    assert_eq!(written.lock().unwrap().len(), 1, "the user's own Open writes it");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A sidecar that can't be written is reported in a notice that outlasts the status line the
/// develop and the Camera Raw step write after it.
#[test]
fn a_sidecar_write_error_stays_reported() {
    let (mut app, dng, dir, raw_path, _) = raw_with_sidecar("readonly", None);
    app.services.write = Some(Box::new(|_: &str, _: &[u8]| Err("read-only volume".to_string())));
    let ctx = app.ctx().clone();
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.5, "contrast": 15}, "commit": true}})).unwrap().unwrap();
    let errors: Vec<_> = app.ui.notices.iter().filter(|n| n.tone == craft::theme::variant::Tone::Bad).map(|n| n.title.clone()).collect();
    assert!(errors.iter().any(|t| t.contains("IMG_0100.xmp") && t.contains("read-only volume")), "{errors:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn open_with_settings(app: &mut PhotocraftApp, ctx: &egui::Context, dng: &[u8], name: &str) {
    app.open_bytes(name, dng).unwrap();
    let raw = take_queued(app).unwrap();
    open_raw(app, ctx, raw).unwrap();
    menu(app, ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.5, "contrast": 60, "saturation": 40, "vibrance": 30}}})).unwrap().unwrap();
}

fn profile_name(app: &PhotocraftApp) -> String {
    let st = app.session.active().unwrap();
    st.doc.icc_profile.as_deref().and_then(|p| craft::cms::Profile::parse(p).ok()).map(|p| p.description).unwrap_or_default()
}

/// Camera Raw renders in its own space and converts to the workflow space last, so Open gives
/// what the dialog previewed on the developed ProPhoto pixels: with the default Adobe RGB the
/// result is the Camera Raw step on the ProPhoto develop, then the conversion. Inline and with
/// the re-develop as a background job.
#[test]
fn the_camera_raw_step_runs_before_the_workflow_conversion() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    open_with_settings(&mut app, &ctx, &dng, "a.dng");
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    app.run("edit.convertToProfile", json!({"profile": "adobe-rgb-compat", "intent": "relative"})).unwrap();
    let want = pixels(&app);
    let close = |a: &[f32], b: &[f32]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
    for background in [false, true] {
        let (mut app, dng) = raw_app();
        app.run("prefs.set", json!({"path": "rawDefaults.colorSpace", "value": "adobeRgb"})).unwrap();
        open_with_settings(&mut app, &ctx, &dng, "b.dng");
        app.background_jobs = background;
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        if background {
            assert_eq!(r["pending"], true, "the re-develop runs as a job: {r}");
        } else {
            assert_eq!(r["defaults"], json!(["colorSpace"]), "{r}");
        }
        for _ in 0..3000 {
            crate::jobs_ui::tick(&mut app, &ctx);
            if app.raw_redevelop.is_empty() && !app.session.has_jobs() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.raw_redevelop.is_empty() && !app.session.has_jobs(), "the jobs never finished");
        assert!(profile_name(&app).to_lowercase().contains("adobe"), "background {background}: {}", profile_name(&app));
        assert!(close(&pixels(&app), &want), "background {background}: the step ran on other pixels than the preview");
        let st = app.session.active().unwrap();
        assert_eq!(st.saved_revision, st.revision, "background {background}: unmodified");
    }
}

/// Two raws opened one after the other while the first one's re-develop still runs in the
/// background: each lands with its Camera Raw step and conversions and is left unmodified.
#[test]
fn two_raws_opened_back_to_back_both_finish_their_open() {
    let (mut app, dng) = raw_app();
    let ctx = app.ctx().clone();
    app.run("prefs.set", json!({"path": "rawDefaults.colorSpace", "value": "adobeRgb"})).unwrap();
    open_with_settings(&mut app, &ctx, &dng, "a.dng");
    app.background_jobs = true;
    let a = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(a["pending"], true, "the re-develop runs as a job: {a}");
    app.background_jobs = false;
    open_with_settings(&mut app, &ctx, &dng, "b.dng");
    app.background_jobs = true;
    let b = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
    assert_eq!(b["pending"], true, "{b}");
    for _ in 0..3000 {
        crate::jobs_ui::tick(&mut app, &ctx);
        if app.raw_redevelop.is_empty() && !app.session.has_jobs() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.raw_redevelop.is_empty() && !app.session.has_jobs(), "the jobs never finished");
    assert_eq!(app.session.documents().len(), 2);
    for i in 0..2 {
        app.session.set_active(i);
        assert!(profile_name(&app).to_lowercase().contains("adobe"), "document {i}: {}", profile_name(&app));
        let st = app.session.active().unwrap();
        assert_eq!(st.saved_revision, st.revision, "document {i} unmodified");
    }
}

/// A sidecar is read only from a regular file of at most `MAX_XMP_BYTES`: a folder, or a file
/// one byte larger, is not read (a pipe or device named like one would never end).
#[test]
fn sidecars_are_read_from_small_regular_files_only() {
    let dir = std::env::temp_dir().join(format!("photocraft-sidecar-bounded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("folder.xmp")).unwrap();
    let cap = photocraft_io::camera_raw_xmp::MAX_XMP_BYTES;
    let (fits, larger) = (dir.join("fits.xmp"), dir.join("larger.xmp"));
    std::fs::write(&fits, vec![b' '; cap]).unwrap();
    std::fs::write(&larger, vec![b' '; cap + 1]).unwrap();
    let s = |p: std::path::PathBuf| p.to_string_lossy().into_owned();
    assert_eq!(read_bounded(&s(fits)).map(|b| b.len()), Some(cap));
    assert_eq!(read_bounded(&s(larger.clone())), None);
    assert_eq!(read_bounded(&s(dir.join("folder.xmp"))), None);
    assert_eq!(read_bounded(&s(dir.join("missing.xmp"))), None);
    assert_eq!(existing_sidecar_is_ours(&s(dir.join("folder.xmp"))), Some(false), "something is there: never written over");
    assert_eq!(existing_sidecar_is_ours(&s(larger)), Some(false));
    assert_eq!(existing_sidecar_is_ours(&s(dir.join("missing.xmp"))), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Save Settings… and Load Settings… as a user's request reaches them: the `{ui}` keys become the
/// workspace's `SaveSettings` / `LoadSettings` events, which `take` answers with a file pick. The
/// file service is the framework's scripted one.
#[test]
fn save_and_load_settings_round_trip_through_xmp_files() {
    use std::sync::Mutex;
    type Written = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
    let written: Written = Arc::default();
    let w = written.clone();
    let foreign = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Clarity2012="+25" crs:LensProfileEnable="1"/>
            </rdf:RDF></x:xmpmeta>"#;
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec!["/out/Look.xmp".into()])]));
    let services = crate::Services {
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.lock().unwrap().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        files: Some(files.clone()),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = app.ctx().clone();
    // The frames that deliver the pick: the workspace reports the event, `take` asks for the file,
    // and the app's poll hands the answer back.
    let deliver = |app: &mut PhotocraftApp| {
        frame(app);
        app.poll_file_dialog();
        frame(app);
    };
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#808080"})).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.75, "contrast": -20, "hslSat": [0, 0, 0, 30, 0, 0, 0, 0]}}}))
        .unwrap()
        .unwrap();
    let saved_params = shown(&app);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"saveSettings": true}})).unwrap().unwrap();
    deliver(&mut app);
    let asked = files.requests();
    assert!(asked.len() == 1 && asked[0].kind == PickKind::Save, "Save Settings asked for one save file");
    let (path, bytes) = written.lock().unwrap()[0].clone();
    assert_eq!(path, "/out/Look.xmp");
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains("crs:Exposure2012=\"+0.75\"") && text.contains("crs:SaturationAdjustmentGreen=\"+30\""), "{text}");
    // Reset to the defaults, then load the saved file back.
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.0, "contrast": 0, "hslSat": [0, 0, 0, 0, 0, 0, 0, 0]}}})).unwrap().unwrap();
    assert_eq!(shown(&app), CameraRaw::default());
    files.queue(Picked::Contents(vec![("Look.xmp".into(), bytes)]));
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
    deliver(&mut app);
    assert_eq!(shown(&app), saved_params);
    // The next answer is an empty file: an error, settings untouched.
    files.queue(Picked::Contents(vec![("Look.xmp".into(), Vec::new())]));
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
    deliver(&mut app);
    assert_eq!(shown(&app), saved_params);
    // A foreign preset applies what it holds over the current settings and reports the rest.
    files.queue(Picked::Contents(vec![("Other.xmp".into(), foreign.to_vec())]));
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
    deliver(&mut app);
    let p = shown(&app);
    assert_eq!((p.clarity, p.exposure), (25.0, 0.75));
    assert!(app.ui.status.contains("Other.xmp") && app.ui.status.contains('1'), "{}", app.ui.status);
    files.queue(Picked::Contents(vec![("Broken.xmp".into(), b"<nope".to_vec())]));
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"loadSettings": true}})).unwrap().unwrap();
    deliver(&mut app);
    assert_eq!(shown(&app).clarity, 25.0);
}

/// Done keeps the settings in the raw's sidecar and opens nothing,
/// even with the preference that writes sidecars on Open turned off.
#[test]
fn done_saves_the_sidecar_and_opens_nothing() {
    let (mut app, dng, dir, raw_path, written) = raw_with_sidecar("done", None);
    let ctx = app.ctx().clone();
    app.run("prefs.set", json!({"path": "rawDefaults.writeXmpSidecars", "value": false})).unwrap();
    let before = app.session.documents().len();
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    let set = json!({"contrast": 20, "clarity": 15});
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": set}})).unwrap().unwrap();
    let r = done(&mut app, &ctx).unwrap();
    assert_eq!(r["saved"], json!(["IMG_0100.xmp"]), "{r}");
    assert!(app.raw.is_none() && app.raw_shelf.is_empty());
    assert_eq!(app.session.documents().len(), before, "Done opens nothing");
    let w = written.lock().unwrap().clone();
    assert_eq!(w.len(), 1);
    let mut back = CameraRaw::default();
    photocraft_io::camera_raw_xmp::load(&mut back, &w[0].1).unwrap();
    assert_eq!((back.contrast, back.clarity), (20.0, 15.0));
    // Done is only for raws being opened, and refused per key: the other keys of the request
    // still apply.
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"done": true}})).unwrap().is_err());
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    let before = shown(&app);
    let refused = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 40}, "done": true}}));
    assert!(refused.unwrap().is_err(), "a Done on a filter is refused");
    assert_eq!(shown(&app).contrast, 40.0, "the accepted key applied on its own");
    assert_eq!(shown(&app).exposure, before.exposure);
    assert!(app.raw.is_some(), "a refused Done closes nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A sidecar that can't be written keeps the dialog and the raw open with their settings;
/// default settings are nothing to save, not an error.
#[test]
fn done_keeps_the_raws_open_when_a_sidecar_write_fails() {
    let (mut app, dng, dir, raw_path, _) = raw_with_sidecar("done-fails", None);
    let ctx = app.ctx().clone();
    app.services.write = Some(Box::new(|_: &str, _: &[u8]| Err("read-only volume".to_string())));
    open_dialog_at(&mut app, &ctx, &dng, &raw_path);
    let docs = app.session.documents().len();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 20}}})).unwrap().unwrap();
    let refused = done(&mut app, &ctx).unwrap_err();
    assert!(!refused.is_empty(), "the refusal names the write error");
    assert_eq!(app.raw.as_ref().map(|_| shown(&app).contrast), Some(20.0), "the edit is still there");
    assert_eq!(app.session.documents().len(), docs, "the raw stays open");
    assert!(app.ui.status_error);
    // At the defaults there is nothing to write: Done closes, without an error.
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"contrast": 0}}})).unwrap().unwrap();
    let r = done(&mut app, &ctx).unwrap();
    assert_eq!((r["done"].clone(), r["saved"].clone()), (json!(true), json!([])), "{r}");
    assert!(app.raw.is_none() && !app.ui.status_error, "{}", app.ui.status);
    let _ = std::fs::remove_dir_all(&dir);
}
