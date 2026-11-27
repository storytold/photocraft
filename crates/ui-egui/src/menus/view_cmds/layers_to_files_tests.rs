use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

const COMMAND: &str = "file.export.layersToFiles";

#[test]
fn layers_to_files_menu_alerts_for_background_only_and_opens_for_multilayer() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = app.ctx().clone();
        assert!(!crate::menus::is_enabled(&app, COMMAND));

        app.run("file.new", json!({"width": 50, "height": 40, "background": "white"})).unwrap();
        assert!(crate::menus::is_enabled(&app, COMMAND), "the menu must be reachable to show the alert");
        let error = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap_err();
        assert_eq!(error, "Layers to Files needs a document with multiple layers.");

        let dialogs = app.framework.dialogs.open_dialogs();
        assert_eq!(dialogs.len(), 1);
        assert_eq!(dialogs[0].key.as_deref(), Some("alert"));
        assert_eq!(dialogs[0].fields.as_ref().unwrap()["message"], error.as_str());

        let mut h =
            Harness::builder().with_context(ctx.clone()).with_size(egui::vec2(900.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
        h.run_steps(4);
        h.get_by_role_and_label(egui::accesskit::Role::Button, "OK");
        h.key_press(egui::Key::Enter);
        h.run_steps(4);
        assert!(h.state().framework.dialogs.open_dialogs().is_empty());

        let app = h.state_mut();
        app.run("layer.new.layer", json!({"name": "Ink"})).unwrap();
        crate::menus::invoke(app, &ctx, COMMAND, json!({})).unwrap();
        let dialogs = app.framework.dialogs.open_dialogs();
        assert_eq!(dialogs.len(), 1);
        assert_eq!(dialogs[0].key.as_deref(), Some(COMMAND));
    });
}
