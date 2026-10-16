use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use photocraft_engine::Session;
use photocraft_genai::{ApiKey, Capabilities, EditRequest, GeneratedImage, ImageProvider, ProviderInfo};

use egui_kittest::kittest::Queryable;

use super::*;
use crate::{GenAiServices, Services};

const SECRET: &str = "AIza-ui-test-secret";

/// Fills with solid red at the request's size and remembers the key it was built with.
struct Fake {
    model: String,
}

impl ImageProvider for Fake {
    fn info(&self) -> ProviderInfo {
        ProviderInfo { id: "gemini", name: "Google Gemini", model: self.model.clone(), host: "example.invalid" }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { generate: false, edit: true, variations: false, edit_in_place: true, max_references: 4 }
    }
    fn edit(&self, r: &EditRequest) -> photocraft_genai::Result<GeneratedImage> {
        let (w, h) = photocraft_genai::png_size(&r.image).unwrap();
        let px: Vec<u8> = (0..w * h).flat_map(|_| [255u8, 0, 0, 255]).collect();
        let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, px).unwrap();
        let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        Ok(GeneratedImage { bytes, mime: "image/png".into() })
    }
}

type Store = Rc<RefCell<HashMap<String, String>>>;

fn services(store: &Store) -> GenAiServices {
    let (a, b, c) = (store.clone(), store.clone(), store.clone());
    GenAiServices {
        load_key: Box::new(move |p| Ok(a.borrow().get(p).cloned())),
        save_key: Box::new(move |p, k| {
            b.borrow_mut().insert(p.into(), k.into());
            Ok(())
        }),
        delete_key: Box::new(move |p| {
            c.borrow_mut().remove(p);
            Ok(())
        }),
        provider: Box::new(|_, model, key: ApiKey| {
            assert_eq!(key.expose(), SECRET);
            Ok(Arc::new(Fake { model: model.into() }))
        }),
        key_store_name: "the test store",
    }
}

fn app_with(store: &Store) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Services { genai: Some(services(store)), ..Default::default() });
    app.run("file.new", json!({"width": 64, "height": 64, "background": "#ffffff"})).unwrap();
    app
}

fn fill_in_menu(app: &PhotocraftApp) -> bool {
    crate::menus::menu_items(app).iter().any(|i| i.id == FILL)
}

fn settings_dialog(app: &PhotocraftApp) -> Option<u64> {
    app.ui.dialogs.iter().find(|d| d.fields.contains_key(SETTINGS_MARK)).map(|d| d.id)
}

#[test]
fn off_by_default_with_no_menu_item_and_no_key_read() {
    let store = Store::default();
    let app = app_with(&store);
    assert!(app.session.image_provider().is_none());
    assert!(!fill_in_menu(&app), "no \"coming soon\" item before a provider is set up");
    // The web build (no services) has no settings at all.
    let mut web = PhotocraftApp::new(Session::new(), Services::default());
    assert!(restore(&mut web).is_ok());
    assert!(web.session.image_provider().is_none());
}

#[test]
fn settings_save_the_key_outside_the_dialog_and_install_the_provider() {
    let store = Store::default();
    let mut app = app_with(&store);
    // Clicking Generative Fill before setup opens the settings.
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, FILL, json!({})).unwrap();
    let id = settings_dialog(&app).expect("settings open");
    app.genai.key_input = format!("  {SECRET}\n");
    let fields = serde_json::to_string(&app.ui.dialogs).unwrap();
    assert!(!fields.contains(SECRET), "the key is never in dialog fields (ui.inspect)");
    let r = crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(r["ready"], true);
    assert_eq!(store.borrow().get("gemini").map(String::as_str), Some(SECRET));
    assert!(app.genai.key_input.is_empty());
    assert!(fill_in_menu(&app));
    let prefs = serde_json::to_string(&app.session.prefs().to_json()).unwrap();
    assert!(!prefs.contains(SECRET), "never in preferences");
    assert!(prefs.contains(photocraft_genai::gemini::DEFAULT_MODEL));

    // A fresh launch restores it from the store.
    let mut next = PhotocraftApp::new(Session::new(), Services { genai: Some(services(&store)), ..Default::default() });
    next.session.prefs.edit(|p| p.dialogs = app.session.prefs().dialogs.clone());
    restore(&mut next).unwrap();
    assert!(next.session.image_provider().is_some());
}

#[test]
fn choosing_none_turns_it_off_and_cancel_forgets_the_typed_key() {
    let store = Store::default();
    store.borrow_mut().insert("gemini".into(), SECRET.into());
    let mut app = app_with(&store);
    let id = open_settings(&mut app);
    assert_eq!(app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["hasKey"], true);
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.session.image_provider().is_some(), "the saved key is used");

    let id = open_settings(&mut app);
    app.genai.key_input = "typed but cancelled".into();
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(app.genai.key_input.is_empty());

    let id = open_settings(&mut app);
    app.ui.dialog_mut(id).unwrap().fields.insert("provider".into(), json!(""));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.session.image_provider().is_none());
    assert!(!fill_in_menu(&app));
}

#[test]
fn the_prompt_dialog_runs_the_fill_and_remembers_the_prompt() {
    let store = Store::default();
    store.borrow_mut().insert("gemini".into(), SECRET.into());
    let mut app = app_with(&store);
    app.session.prefs.edit(|p| p.dialogs.insert(PREF.into(), json!({"provider": "gemini", "model": "m"})));
    restore(&mut app).unwrap();
    let ctx = egui::Context::default();
    // No selection: Generative Fill can't run, and says why.
    assert!(crate::menus::invoke(&mut app, &ctx, FILL, json!({})).is_err());
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let r = crate::menus::invoke(&mut app, &ctx, FILL, json!({})).unwrap();
    let id = r["dialog"].as_u64().unwrap();
    assert_eq!(crate::genai_ui::ok_label(&app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields), Some("Generate"));
    app.ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!("a red square"));
    let layers = app.session.active().unwrap().doc.layers.len();
    crate::dialogs::confirm(&mut app, id).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), layers + 1);
    assert_eq!(doc.layers.last().unwrap().name, "a red square");
    assert_eq!(app.session.prefs().dialogs.get(FILL), Some(&json!({"prompt": "a red square", "mode": "fill", "separate": true})));
    let (cmd, params) = app.session.journal.iter().rev().find(|(c, _)| c == FILL).unwrap();
    assert_eq!((cmd.as_str(), params), (FILL, &json!({"prompt": "a red square", "mode": "fill"})), "no references: none sent");
}

/// An app with the provider set up, an open 64² document with a selection, and the prompt
/// dialog's id.
fn prompt_ready(store: &Store) -> PhotocraftApp {
    store.borrow_mut().insert("gemini".into(), SECRET.into());
    let mut app = app_with(store);
    app.session.prefs.edit(|p| p.dialogs.insert(PREF.into(), json!({"provider": "gemini", "model": "m"})));
    restore(&mut app).unwrap();
    app
}

fn open_prompt_dialog(app: &mut PhotocraftApp) -> u64 {
    let ctx = egui::Context::default();
    crate::menus::invoke(app, &ctx, FILL, json!({})).unwrap()["dialog"].as_u64().unwrap()
}

fn fields(app: &PhotocraftApp, id: u64) -> &Map<String, Value> {
    &app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields
}

fn last_fill(app: &PhotocraftApp) -> Value {
    app.session.journal.iter().rev().find(|(c, _)| c == FILL).map(|(_, p)| p.clone()).unwrap()
}

#[test]
fn edit_mode_and_reference_documents_reach_the_command_and_are_remembered() {
    let store = Store::default();
    let mut app = prompt_ready(&store);
    // A green document to use as a reference, then the one to edit.
    app.run("file.new", json!({"width": 32, "height": 32, "background": "#00ff00"})).unwrap();
    let green = app.session.active().unwrap().doc.id.0;
    app.run("file.new", json!({"width": 64, "height": 64, "background": "#ffffff"})).unwrap();
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let id = open_prompt_dialog(&mut app);
    assert_eq!(fields(&app, id)["mode"], "fill", "Fill until Edit is chosen");
    assert_eq!(fields(&app, id)["references"], json!([]));
    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
    f.insert("mode".into(), json!("edit"));
    f.insert("prompt".into(), json!("  make it blue "));
    f.insert("references".into(), json!([{"document": green}]));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(last_fill(&app), json!({"prompt": "make it blue", "mode": "edit", "references": [{"document": green}]}));
    assert_eq!(app.session.prefs().dialogs.get(FILL), Some(&json!({"prompt": "make it blue", "mode": "edit", "separate": true})));
    assert_eq!(app.session.active().unwrap().doc.layers.last().unwrap().name, "make it blue");

    // The next prompt starts in Edit with the same reference.
    let id = open_prompt_dialog(&mut app);
    assert_eq!(fields(&app, id)["mode"], "edit");
    assert_eq!(fields(&app, id)["references"], json!([{"document": green}]));
    // Edit with no prompt removes, as Fill does: Edit needs something to change.
    app.ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!(""));
    app.ui.dialog_mut(id).unwrap().fields.insert("references".into(), json!([]));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(last_fill(&app), json!({"prompt": "", "mode": "fill"}));
    assert_eq!(app.session.prefs().dialogs.get(FILL), Some(&json!({"prompt": "", "mode": "edit", "separate": true})), "the tab chosen is kept");

    // A reference to a document that has been closed since is dropped.
    app.genai.references = vec![json!({"document": 987_654_321}), json!({"document": green})];
    let id = open_prompt_dialog(&mut app);
    assert_eq!(fields(&app, id)["references"], json!([{"document": green}]));
}

#[test]
fn image_files_are_added_as_references_by_path_after_they_open() {
    let dir = std::env::temp_dir().join(format!("photocraft-genai-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("bottle.png");
    let px: Vec<u8> = (0..40 * 20).flat_map(|_| [200u8, 30, 40, 255]).collect();
    let img = photocraft_codecs::Image::from_u8(40, 20, photocraft_codecs::ChannelLayout::Rgba, px).unwrap();
    std::fs::write(&good, photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()).unwrap();
    let bad = dir.join("broken.png");
    std::fs::write(&bad, b"not an image").unwrap();
    let store = Store::default();
    let mut app = prompt_ready(&store);
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let answers = [&bad, &good].map(|p| Some(crate::file_dialog::FileDialogAnswer::Paths(vec![p.to_string_lossy().into_owned()])));
    app.services.file_dialog = Some(crate::file_dialog::fake(answers.into()).0);
    let id = open_prompt_dialog(&mut app);
    let ctx = egui::Context::default();
    // A file that doesn't open is reported now and not added.
    pick_reference_file(&mut app).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(app.ui.status.contains("broken.png"), "{}", app.ui.status);
    assert_eq!(fields(&app, id)["references"], json!([]));
    pick_reference_file(&mut app).unwrap();
    app.poll_file_dialog(&ctx, None);
    let entry = json!({"path": good.to_string_lossy()});
    assert_eq!(fields(&app, id)["references"], json!([entry.clone()]));
    assert!(matches!(app.genai.previews.get(&reference_key(&entry)), Some(Preview::Image(i)) if i.size == [40, 20]), "a preview, never enlarged");
    assert_eq!(reference_name(&app, &entry).as_deref(), Some("bottle.png"));
    app.ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!("this bottle"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(last_fill(&app)["references"], json!([entry]));
    let _ = std::fs::remove_dir_all(dir);
}

/// Renders Generative AI Settings and the prompt for a look (`GENAI_SNAPSHOT_DIR=… cargo test`).
#[test]
fn the_dialogs_render() {
    let store = Store::default();
    store.borrow_mut().insert("gemini".into(), SECRET.into());
    let store2 = store.clone();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 760.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = app_with(&store2);
        open_settings(&mut app);
        app
    });
    h.run_steps(6);
    // The label and the dropdown named after it.
    assert!(h.get_all_by_label("Provider").count() >= 1);
    h.get_by_label("Remove Key");
    h.get_by_label("Get a Gemini API key from Google AI Studio");
    let dir = std::env::var_os("GENAI_SNAPSHOT_DIR");
    if let (Some(dir), Ok(img)) = (&dir, h.render()) {
        img.save(std::path::Path::new(dir).join("genai-settings.png")).unwrap();
    }
    let id = settings_dialog(h.state()).unwrap();
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.state_mut().run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, FILL, json!({})).unwrap();
    h.run_steps(6);
    h.get_by_label("Generate");
    h.get_by_label("Fill selection");
    h.get_by_label("Add Reference…");
    if let (Some(dir), Ok(img)) = (&dir, h.render()) {
        img.save(std::path::Path::new(dir).join("genai-prompt.png")).unwrap();
    }
    // Edit, with the document itself as a reference.
    h.get_by_label("Edit selection").click();
    h.run_steps(2);
    let doc = h.state().session.active().unwrap().doc.id.0;
    let prompt = h.state().ui.dialogs[0].id;
    let f = &mut h.state_mut().ui.dialog_mut(prompt).unwrap().fields;
    assert_eq!(f["mode"], "edit");
    f.insert("prompt".into(), json!("make the label black and the text white"));
    f.insert("references".into(), json!([{"document": doc}]));
    h.run_steps(6);
    h.get_by_label("Remove");
    if let Some(dir) = &dir
        && let Ok(img) = h.render()
    {
        img.save(std::path::Path::new(dir).join("genai-prompt-edit.png")).unwrap();
    }
    // Add Reference… offers an image file or an open document.
    h.get_by_label("Add Reference…").click();
    h.run_steps(4);
    h.get_by_label("Image File…");
    h.get_by_label("Open Documents");
    if let Some(dir) = &dir
        && let Ok(img) = h.render()
    {
        img.save(std::path::Path::new(dir).join("genai-add-reference.png")).unwrap();
    }
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    // The options bar of a selection tool: the button sits after Select and Mask….
    let prompt = h.state().ui.dialogs[0].id;
    crate::dialogs::cancel(h.state_mut(), prompt).unwrap();
    h.state_mut().ui.tool = crate::state::Tool::RectMarquee;
    h.run_steps(6);
    h.get_by_label("Generative Fill…");
    if let (Some(dir), Ok(img)) = (&dir, h.render()) {
        img.save(std::path::Path::new(dir).join("genai-options-bar.png")).unwrap();
    }
}

/// Properties shows a generative layer's variations: clicking one shows it, and Generate More
/// adds three (`GENAI_SNAPSHOT_DIR=… cargo test` saves a look).
#[test]
fn properties_show_the_variations() {
    let store = Store::default();
    store.borrow_mut().insert("gemini".into(), SECRET.into());
    let store2 = store.clone();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 760.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = app_with(&store2);
        app.session.prefs.edit(|p| p.dialogs.insert(PREF.into(), json!({"provider": "gemini", "model": "m"})));
        restore(&mut app).unwrap();
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

// ---- Fills that run side by side --------------------------------------------------------------

/// Answers after `delay` with solid blue, or (with `fail`) says the service can't be reached.
struct Slow {
    delay: std::time::Duration,
    fail: bool,
}

impl ImageProvider for Slow {
    fn info(&self) -> ProviderInfo {
        ProviderInfo { id: "gemini", name: "Google Gemini", model: "slow".into(), host: "example.invalid" }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { generate: false, edit: true, variations: false, edit_in_place: true, max_references: 4 }
    }
    fn edit(&self, r: &EditRequest) -> photocraft_genai::Result<GeneratedImage> {
        std::thread::sleep(self.delay);
        if self.fail {
            return Err(photocraft_genai::GenError::Network("offline".into()));
        }
        let (w, h) = photocraft_genai::png_size(&r.image).unwrap();
        let px: Vec<u8> = (0..w * h).flat_map(|_| [0u8, 0, 255, 255]).collect();
        let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, px).unwrap();
        let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        Ok(GeneratedImage { bytes, mime: "image/png".into() })
    }
}

/// [`prompt_ready`], running jobs in the background as the desktop app does, with `provider`.
fn background(store: &Store, provider: Slow) -> PhotocraftApp {
    let mut app = prompt_ready(store);
    app.background_jobs = true;
    app.session.set_image_provider(Some(Arc::new(provider)));
    app
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

fn groups(app: &PhotocraftApp) -> Vec<(String, photocraft_geom::Rect)> {
    let doc = &app.session.active().unwrap().doc;
    doc.layers.iter().filter(|l| l.is_group()).map(|l| (l.name.clone(), l.mask.as_ref().unwrap().surface.content_bounds())).collect()
}

#[test]
fn separate_areas_are_filled_one_by_one_unless_asked_otherwise() {
    let store = Store::default();
    let mut app = prompt_ready(&store);
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 16, "height": 16, "mode": "add"})).unwrap();
    let id = open_prompt_dialog(&mut app);
    assert_eq!(fields(&app, id)["__areas"], 2);
    assert_eq!(fields(&app, id)["separate"], true, "on unless turned off");
    app.ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!("x"));
    crate::dialogs::confirm(&mut app, id).unwrap();
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

#[test]
fn in_the_background_fills_block_nothing_and_land_where_they_were_started() {
    let store = Store::default();
    let mut app = background(&store, Slow { delay: std::time::Duration::from_millis(300), fail: false });
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
    let mut h = egui_kittest::Harness::new_ui_state(|ui, app: &mut PhotocraftApp| crate::genai_ui::status_fills(app, ui), app);
    h.run_steps(2);
    let shown = h.query_by_label("2 generating").is_some() || h.query_by_label("1 generating · 1 waiting").is_some();
    assert!(shown, "status readout");
    let mut app = h.into_state();
    tick_until(&mut app, |app| app.session.generative_fills().is_empty() && !app.session.has_jobs());
    assert_eq!(groups(&app).len(), 2);
    assert_eq!(app.session.active().unwrap().active_layer, bg, "the user's layer stays active");
    assert!(app.ui.status.starts_with("Generated"), "{}", app.ui.status);
}

#[test]
fn a_failed_fill_keeps_its_row_offers_retry_and_can_be_discarded() {
    let store = Store::default();
    let mut app = background(&store, Slow { delay: std::time::Duration::from_millis(20), fail: true });
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    generate(&mut app, "lamp");
    // Failed, and its job's end reported.
    tick_until(&mut app, |app| app.session.generative_fills().first().is_some_and(|f| f.state == "failed") && !app.session.has_jobs());
    assert!(app.ui.status_error && app.ui.status.contains("offline"), "{}", app.ui.status);
    let doc = app.session.active().unwrap().doc.id;
    // Its row stands where the layer will land, with Retry and Discard.
    let mut h = egui_kittest::Harness::new_ui_state(move |ui, app: &mut PhotocraftApp| crate::genai_ui::layer_rows_unplaced(app, ui, doc, &[]), app);
    h.run_steps(2);
    h.get_by_label("Failed: lamp");
    let mut app = h.into_state();
    // Retry with a working service lands it.
    app.session.set_image_provider(Some(Arc::new(Slow { delay: std::time::Duration::from_millis(20), fail: false })));
    let id = app.session.generative_fills()[0].id;
    app.run(photocraft_engine::genai_cmds::RETRY, json!({"fill": id})).unwrap();
    tick_until(&mut app, |app| app.session.generative_fills().is_empty() && !app.session.has_jobs());
    assert_eq!(groups(&app).len(), 1);
    // Discard stops one that is still generating; nothing lands.
    app.session.set_image_provider(Some(Arc::new(Slow { delay: std::time::Duration::from_millis(300), fail: false })));
    app.run("select.rect", json!({"x": 30, "y": 30, "width": 12, "height": 12})).unwrap();
    generate(&mut app, "gone");
    let id = app.session.generative_fills()[0].id;
    app.run(photocraft_engine::genai_cmds::DISCARD, json!({"fill": id})).unwrap();
    app.session.join_cancelled_jobs();
    tick_until(&mut app, |app| !app.session.has_jobs());
    assert!(app.session.generative_fills().is_empty());
    assert_eq!(groups(&app).len(), 1);
}

#[test]
fn the_canvas_draws_fills_in_progress() {
    let store = Store::default();
    let mut app = background(&store, Slow { delay: std::time::Duration::from_millis(500), fail: false });
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    generate(&mut app, "a");
    let doc = app.session.active().unwrap().doc.id;
    let mut h = egui_kittest::Harness::new_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            let xf = crate::canvas::ViewXform { rect: ui.max_rect(), zoom: 4.0, center: [32.0, 32.0], flip: false, rotation: 0.0, aspect: 1.0 };
            let painter = ui.painter().clone();
            crate::genai_ui::paint_pending(app, &ui.ctx().clone(), &painter, &xf, doc);
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

/// Renders fills side by side for a look (`GENAI_SNAPSHOT_DIR=… cargo test`): six objects
/// selected together and filled each on its own (five generating, one waiting its turn), one
/// that failed, the Layers panel's rows and the status bar's list.
#[test]
fn fills_side_by_side_render() {
    let Some(dir) = std::env::var_os("GENAI_SNAPSHOT_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    let store = Store::default();
    store.borrow_mut().insert("gemini".into(), SECRET.into());
    let store2 = store.clone();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 860.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = background(&store2, Slow { delay: std::time::Duration::from_millis(30), fail: true });
        app.run("file.new", json!({"width": 1200, "height": 800, "background": "#d9d2c3"})).unwrap();
        // A floor, and seven "objects" to remove.
        app.run("select.rect", json!({"x": 0, "y": 520, "width": 1200, "height": 280})).unwrap();
        app.run("edit.fill", json!({"color": "#8a7a66"})).unwrap();
        for (i, (x, y, w, hh, c)) in [
            (90, 380, 90, 160, "#2f5d8a"),
            (300, 430, 70, 110, "#a8432f"),
            (470, 300, 60, 60, "#e0b03c"),
            (640, 420, 120, 120, "#3c7d4a"),
            (860, 360, 80, 180, "#6b3d8a"),
            (1020, 460, 100, 80, "#c76a2b"),
            (560, 600, 140, 60, "#444444"),
        ]
        .into_iter()
        .enumerate()
        {
            let _ = i;
            app.run("select.rect", json!({"x": x, "y": y, "width": w, "height": hh})).unwrap();
            app.run("edit.fill", json!({"color": c})).unwrap();
        }
        app.run("select.deselect", json!({})).unwrap();
        app
    });
    h.run_steps(4);
    let save = |h: &mut egui_kittest::Harness<PhotocraftApp>, name: &str| {
        if let Ok(img) = h.render() {
            img.save(dir.join(name)).unwrap();
        }
    };
    // One that failed: the service couldn't be reached.
    h.state_mut().run("select.rect", json!({"x": 545, "y": 590, "width": 170, "height": 80})).unwrap();
    generate(h.state_mut(), "remove the rug");
    for _ in 0..200 {
        h.run_steps(1);
        if h.state().session.generative_fills().first().is_some_and(|f| f.state == "failed") && !h.state().session.has_jobs() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Six objects selected together, filled each on its own: five at a time, one waits.
    h.state_mut().session.set_image_provider(Some(Arc::new(Slow { delay: std::time::Duration::from_secs(30), fail: false })));
    for (i, (x, y, w, hh)) in [(80, 370, 110, 180), (290, 420, 90, 130), (460, 290, 80, 80), (630, 410, 140, 140), (850, 350, 100, 200), (1010, 450, 120, 100)]
        .into_iter()
        .enumerate()
    {
        let mode = if i == 0 { "replace" } else { "add" };
        h.state_mut().run("select.rect", json!({"x": x, "y": y, "width": w, "height": hh, "mode": mode})).unwrap();
    }
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, FILL, json!({})).unwrap();
    let id = h.state().ui.dialogs[0].id;
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("prompt".into(), json!(""));
    h.run_steps(6);
    h.get_by_label("Fill each of the 6 separate areas on its own");
    save(&mut h, "genai-areas-prompt.png");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    for _ in 0..30 {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(h.state().session.generative_fills().len(), 7);
    save(&mut h, "genai-side-by-side.png");
    // The status bar's list.
    h.get_by_label_contains("generating").click();
    h.run_steps(4);
    save(&mut h, "genai-side-by-side-list.png");
    h.state_mut().session.cancel_all_jobs();
}

/// The tags sit over the canvas: clicking one's Cancel stops that fill and nothing else; the
/// tool under it (the Brush here) never sees the click.
#[test]
fn a_tags_cancel_takes_the_click_from_the_canvas() {
    let store = Store::default();
    store.borrow_mut().insert("gemini".into(), SECRET.into());
    let store2 = store.clone();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = background(&store2, Slow { delay: std::time::Duration::from_secs(20), fail: false });
        app.run("file.new", json!({"width": 600, "height": 400, "background": "#ffffff"})).unwrap();
        app.ui.tool = crate::state::Tool::Brush;
        app
    });
    h.run_steps(4);
    for x in [100, 350] {
        h.state_mut().run("select.rect", json!({"x": x, "y": 200, "width": 60, "height": 60})).unwrap();
        generate(h.state_mut(), "");
        h.run_steps(2);
    }
    h.run_steps(4);
    assert_eq!(h.state().session.generative_fills().len(), 2);
    let history = h.state().session.active().unwrap().history.past_len();
    // The tag on the canvas, not the Layers panel row's × (leftmost of the two).
    let tag = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.get_all_by_label("Cancel this Generative Fill").min_by(|a, b| a.rect().left().total_cmp(&b.rect().left())).unwrap().rect();
    let r = tag(&h);
    h.input_mut().events.push(egui::Event::PointerMoved(r.center()));
    h.run_steps(1);
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton { pos: r.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.run_steps(1);
    }
    h.run_steps(4);
    assert_eq!(h.state().session.generative_fills().len(), 1, "that one stopped");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history, "no brush stroke under the tag");
    // The same click on the canvas itself paints.
    let at = tag(&h).center() + egui::vec2(0.0, 80.0);
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        h.run_steps(2);
    }
    h.run_steps(4);
    assert!(h.state().session.active().unwrap().history.past_len() > history, "a click on the canvas paints");
    h.state_mut().session.cancel_all_jobs();
}
