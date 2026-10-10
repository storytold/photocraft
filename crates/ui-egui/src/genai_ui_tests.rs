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
        Capabilities { generate: false, edit: true, variations: false }
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
    assert_eq!(app.session.prefs().dialogs.get(FILL), Some(&json!({"prompt": "a red square"})));
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
    h.get_by_label("Provider");
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
    if let (Some(dir), Ok(img)) = (&dir, h.render()) {
        img.save(std::path::Path::new(dir).join("genai-prompt.png")).unwrap();
    }
}
