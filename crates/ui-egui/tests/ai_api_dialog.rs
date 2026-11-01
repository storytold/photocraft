//! API settings require an explicit save and never put secrets in serializable UI state.
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::json;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(800.0, 600.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut session = photocraft_engine::Session::new();
        session.edit_prefs(|p| p.interface.language = "en".into());
        session.execute("file.new", json!({"width":16,"height":12})).unwrap();
        PhotocraftApp::new(session, Services::default())
    });
    let ctx = h.ctx.clone();
    photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "edit.ai.codex", json!({})).unwrap();
    h.run_steps(3);
    h
}

#[test]
fn selector_exposes_all_services_and_opens_no_browser_or_generation() {
    let mut h = harness();
    h.get_by_label("Image service").click();
    h.run_steps(2);
    assert!(h.query_by_label("Gemini / Nano Banana").is_some());
    assert!(h.query_by_label("Doubao / Seedream").is_some());
    assert!(h.query_by_label("FLUX / Black Forest Labs").is_some());
    h.get_by_label("Gemini / Nano Banana").click();
    h.run_steps(3);
    assert!(h.query_by_label("Model ID").is_some());
    assert!(h.query_by_label("Save API settings").is_some());
    assert!(h.get_by_label("OK").accesskit_node().is_disabled());
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
    assert!(!h.state().session.ai_account.pending());
}

#[test]
fn saved_session_key_is_hidden_ready_with_prompt_and_removed_explicitly() {
    let mut h = harness();
    h.state_mut().ui.dialogs[0].fields.insert("provider".into(), json!("seedream"));
    h.state_mut().ui.dialogs[0].fields.insert("prompt".into(), json!("Make the circle yellow"));
    h.run_steps(3);
    h.get_by_role(egui::accesskit::Role::PasswordInput).click();
    h.run_steps(2);
    h.get_by_role(egui::accesskit::Role::PasswordInput).type_text("private-ui-test-key");
    h.run_steps(2);
    assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "must save explicitly");
    assert!(!serde_json::to_string(&h.state().ui).unwrap().contains("private-ui-test-key"));
    assert!(!format!("{:?}", h.state().ui.ai_api).contains("private-ui-test-key"));
    let save_rect = h.get_by_label("Save API settings").rect();
    assert!(save_rect.bottom() <= 600.0, "Save must stay on screen: {save_rect:?}");
    h.get_by_label("Save API settings").click();
    h.run_steps(3);
    assert!(h.state().session.ai_api.has_key("seedream").unwrap(), "Save failed: {:?}; button: {save_rect:?}", h.state().ui.ai_api);
    assert!(!h.get_by_label("OK").accesskit_node().is_disabled(), "API state: {:?}; fields: {:?}", h.state().ui.ai_api, h.state().ui.dialogs[0].fields);
    assert!(!format!("{:?}", h.state().session.journal).contains("private-ui-test-key"));
    let fields = &h.state().ui.dialogs[0].fields;
    let (command, params) = photocraft_ui_egui::ai_api_ui::edit_command(fields, "edit.ai.codex".into(), json!({}));
    assert_eq!(command, "edit.ai.api");
    assert_eq!(params, json!({"provider":"seedream","prompt":"Make the circle yellow"}));
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1, "saving is local only");
    h.get_by_label("Model ID").click();
    h.run_steps(2);
    h.get_by_label("Model ID").type_text("-variant");
    h.run_steps(2);
    assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "model changes need a save");
    h.get_by_label("Save API settings").click();
    h.run_steps(3);
    assert!(!h.get_by_label("OK").accesskit_node().is_disabled(), "same address keeps its key");
    h.get_by_label("Remove API key").click();
    h.run_steps(3);
    assert!(h.get_by_label("OK").accesskit_node().is_disabled());
    assert!(!h.state().session.ai_api.has_key("seedream").unwrap());
}

#[test]
fn flux_configuration_routes_only_to_the_explicit_flux_provider() {
    let mut h = harness();
    h.state_mut().session.execute("ai.provider.settings", json!({"action":"save","provider":"flux","profile":photocraft_engine::cmd::ai_provider_cmds::preset("flux"),"apiKey":"synthetic-flux-key","rememberKey":false})).unwrap();
    h.state_mut().ui.dialogs[0].fields.insert("provider".into(), json!("flux"));
    h.state_mut().ui.dialogs[0].fields.insert("prompt".into(), json!("make yellow"));
    h.run_steps(4);
    assert!(!h.get_by_label("OK").accesskit_node().is_disabled());
    let (command, params) = photocraft_ui_egui::ai_api_ui::edit_command(&h.state().ui.dialogs[0].fields, "edit.ai.codex".into(), json!({}));
    assert_eq!(command, "edit.ai.api");
    assert_eq!(params, json!({"provider":"flux","prompt":"make yellow"}));
    assert!(!serde_json::to_string(&h.state().ui).unwrap().contains("synthetic-flux-key"));
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
}

#[test]
fn pasted_seedream_key_trims_boundary_whitespace_and_reports_saved() {
    let mut h = harness();
    h.state_mut().ui.dialogs[0].fields.insert("provider".into(), json!("seedream"));
    h.state_mut().ui.dialogs[0].fields.insert("prompt".into(), json!("make yellow"));
    h.run_steps(3);
    h.get_by_role(egui::accesskit::Role::PasswordInput).click();
    h.run_steps(2);
    h.event(egui::Event::Paste(" \r\n synthetic-boundary-key \n ".into()));
    h.run_steps(2);
    h.get_by_label("Save API settings").click();
    h.run_steps(3);
    assert!(h.state().session.ai_api.has_key("seedream").unwrap());
    assert!(h.query_by_label("API settings saved").is_some());
    assert!(h.query_by_label("API key configured — leave empty to keep it").is_some());
    assert!(!h.get_by_label("OK").accesskit_node().is_disabled());
    assert!(!format!("{:?}", h.state().ui.ai_api).contains("synthetic-boundary-key"));
    assert!(!serde_json::to_string(&h.state().ui).unwrap().contains("synthetic-boundary-key"));
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
}

#[test]
fn invalid_internal_whitespace_or_hidden_characters_are_rejected_with_chinese_feedback() {
    for key in ["synthetic key", "synthetic\u{200b}key", "   "] {
        let mut h = harness();
        h.state_mut().session.execute("prefs.set", json!({"path":"interface.language","value":"zh-hans"})).unwrap();
        h.state_mut().ui.dialogs[0].fields.insert("provider".into(), json!("seedream"));
        h.run_steps(3);
        h.get_by_role(egui::accesskit::Role::PasswordInput).click();
        h.run_steps(2);
        h.get_by_role(egui::accesskit::Role::PasswordInput).type_text(key);
        h.run_steps(2);
        h.get_by_label("保存 API 配置").click();
        h.run_steps(3);
        assert!(!h.state().session.ai_api.has_key("seedream").unwrap());
        assert!(h.query_by_label("密钥包含空格、换行或无效字符，请重新复制完整密钥。首尾空白会自动忽略。").is_some());
        assert!(h.get_by_label("确定").accesskit_node().is_disabled());
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
    }
}

#[test]
fn cancelling_drops_secret_draft_without_saving() {
    let mut h = harness();
    h.state_mut().ui.dialogs[0].fields.insert("provider".into(), json!("gemini"));
    h.run_steps(3);
    h.get_by_role(egui::accesskit::Role::PasswordInput).click();
    h.run_steps(2);
    h.get_by_role(egui::accesskit::Role::PasswordInput).type_text("unsaved-private-key");
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(h.state().ui.ai_api, Default::default());
    assert!(!h.state().session.ai_api.has_key("gemini").unwrap());
}

#[test]
fn all_languages_update_ai_controls_and_saved_feedback_without_losing_the_prompt() {
    use photocraft_ui_egui::i18n::{Lang, tr};
    let mut h = harness();
    h.state_mut().session.execute("ai.provider.settings", json!({"action":"save","provider":"seedream","profile":photocraft_engine::cmd::ai_provider_cmds::preset("seedream"),"apiKey":"local-translation-key","rememberKey":false})).unwrap();
    h.state_mut().ui.dialogs[0].fields.insert("provider".into(), json!("seedream"));
    h.state_mut().ui.dialogs[0].fields.insert("prompt".into(), json!("Keep this instruction unchanged"));
    h.state_mut().session.execute("prefs.set", json!({"path":"interface.language","value":"zh-hans"})).unwrap();
    h.run_steps(4);
    h.get_by_label("保存 API 配置").click();
    h.run_steps(3);
    assert!(h.query_by_label("API 配置已保存").is_some());
    for lang in Lang::all() {
        h.state_mut().session.execute("prefs.set", json!({"path":"interface.language","value":lang.code()})).unwrap();
        h.run_steps(3);
        for key in ["Image service", "Model ID", "Save API settings", "API settings saved"] {
            assert!(h.query_by_label(tr(lang, key)).is_some(), "{}: missing {key}", lang.code());
        }
        assert_eq!(h.state().ui.dialogs[0].fields["prompt"], "Keep this instruction unchanged");
        assert!(!h.get_by_label(tr(lang, "OK")).accesskit_node().is_disabled());
        h.state_mut().ui.dialogs[0].fields.insert("provider".into(), json!("codex"));
        h.run_steps(3);
        for key in ["Image service", "Sign in with ChatGPT", "Check connection"] {
            assert!(h.query_by_label(tr(lang, key)).is_some(), "{}: missing {key}", lang.code());
        }
        h.state_mut().ui.dialogs[0].fields.insert("provider".into(), json!("seedream"));
    }
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
    assert!(!h.state().session.ai_account.pending());
    assert!(!serde_json::to_string(&h.state().ui).unwrap().contains("local-translation-key"));
}

#[test]
fn expanded_api_settings_keep_confirmation_buttons_on_a_small_screen() {
    let mut h = harness();
    h.state_mut().ui.dialogs[0].fields.insert("provider".into(), json!("gemini"));
    h.run_steps(3);
    h.get_by_label("API address and protocol").click();
    h.run_steps(3);
    assert!(h.query_by_label("API address").is_some());
    for label in ["OK", "Cancel"] {
        let rect = h.get_by_label(label).rect();
        assert!(rect.top() >= 0.0 && rect.bottom() <= 600.0, "{label} outside the window: {rect:?}");
    }
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 1);
}
