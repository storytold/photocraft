//! Public publishing controls, independent of gallery and profile management.
use super::extras::start_call;
use super::{PhotocraftApp, Value, json, start_connect};

pub(super) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut serde_json::Map<String, Value>) {
    ui.label(tl!("Sta.sh item id"));
    let mut item_id = match f.get("itemId") {
        Some(Value::String(value)) => value.clone(),
        Some(_) => String::new(),
        None => app.deviantart.last_item_id.clone(),
    };
    if ui.add(egui::TextEdit::singleline(&mut item_id).desired_width(120.0)).changed() {
        f.insert("itemId".into(), json!(item_id.clone()));
    }
    let mut public_ok = f.get("publicOk").and_then(Value::as_bool).unwrap_or(false);
    if crate::widgets::checkbox(ui, &mut public_ok, tl!("Publish this publicly")).changed() {
        f.insert("publicOk".into(), json!(public_ok));
    }
    let mut mature = f.get("mature").and_then(Value::as_bool).unwrap_or(false);
    if crate::widgets::checkbox(ui, &mut mature, tl!("Not for all ages")).changed() {
        f.insert("mature".into(), json!(mature));
    }
    ui.label(tl!("License"));
    let mut creative_commons = f.get("creativeCommons").and_then(Value::as_bool).unwrap_or(false);
    if crate::widgets::checkbox(ui, &mut creative_commons, tl!("Creative Commons")).changed() {
        f.insert("creativeCommons".into(), json!(creative_commons));
    }
    let mut commercial = f.get("commercial").and_then(Value::as_bool).unwrap_or(false);
    if crate::widgets::checkbox(ui, &mut commercial, tl!("Commercial use")).changed() {
        f.insert("commercial".into(), json!(commercial));
    }
    if crate::widgets::secondary_button(ui, tl!("Publish"), 160.0).clicked() {
        if !public_ok {
            app.ui.status = tl!("Confirm that this publish is public.").to_string();
            app.ui.status_error = true;
            return;
        }
        let Some(item_id) = item_id.trim().parse::<i64>().ok().filter(|id| *id > 0) else {
            app.ui.status = tl!("Upload with OK first. Publish uses the Sta.sh item from that upload.").to_string();
            app.ui.status_error = true;
            return;
        };
        if !app.deviantart.scopes.split_whitespace().any(|scope| scope == "publish") {
            app.ui.status = tl!("Reconnect to DeviantArt to grant permission to publish.").to_string();
            if let Err(error) = start_connect(app, "publish") {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        } else if let Err(error) = start_call(
            app,
            &json!({"op":"publish","itemId": item_id, "mature": mature, "level": "moderate", "classes": [], "creativeCommons": creative_commons, "commercial": commercial, "modify": "no"}).to_string(),
        ) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
}
