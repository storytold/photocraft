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
    assert_eq!(app.session.prefs().dialogs.get(FILL), Some(&json!({"prompt": "a red square", "mode": "fill"})));
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
    assert_eq!(app.session.prefs().dialogs.get(FILL), Some(&json!({"prompt": "make it blue", "mode": "edit"})));
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
    assert_eq!(app.session.prefs().dialogs.get(FILL), Some(&json!({"prompt": "", "mode": "edit"})), "the tab chosen is kept");

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
