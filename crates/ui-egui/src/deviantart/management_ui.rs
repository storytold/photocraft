//! Optional gallery/edit/profile controls. Private uploads and publishing do not depend on this surface.
use super::extras::{avatar_file, gallery_section, start_call};
use super::{PhotocraftApp, Value, clean_tags, json, start_connect};

pub(super) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut serde_json::Map<String, Value>) {
    gallery_section(app, ui, f);
    ui.add_space(8.0);
    ui.label(tl!("Deviation id"));
    let mut deviation_id = f.get("deviationId").and_then(Value::as_str).unwrap_or("").to_string();
    if ui.add(egui::TextEdit::singleline(&mut deviation_id).desired_width(ui.available_width())).changed() {
        f.insert("deviationId".into(), json!(deviation_id.clone()));
    }
    if crate::widgets::secondary_button(ui, tl!("Update title and tags"), 220.0).clicked() {
        let tags = clean_tags(f.get("tags").and_then(Value::as_str).unwrap_or(""));
        let title = f.get("title").and_then(Value::as_str).unwrap_or("");
        if deviation_id.trim().is_empty() {
            app.ui.status = tl!("Enter the deviation id first.").to_string();
            app.ui.status_error = true;
        } else if let Err(error) = start_call(app, &json!({"op":"edit","deviationId": deviation_id.trim(), "title": title, "tags": tags}).to_string()) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
    ui.label(tl!("Profile picture"));
    let avatar = f.get("avatar").and_then(Value::as_str).unwrap_or("png").to_string();
    ui.horizontal(|ui| {
        for (id, label) in [("png", "PNG"), ("jpg", "JPEG")] {
            if ui.selectable_label(avatar == id, label).clicked() {
                f.insert("avatar".into(), json!(id));
            }
        }
    });
    if crate::widgets::secondary_button(ui, tl!("Set as DeviantArt profile picture"), 260.0).clicked() {
        if !app.deviantart.scopes.split_whitespace().any(|scope| scope == "user.manage") {
            app.ui.status = tl!("Reconnect to DeviantArt to grant permission to change your profile picture.").to_string();
            if let Err(error) = start_connect(app, "browse user.manage") {
                app.ui.status = error;
                app.ui.status_error = true;
            }
            return;
        }
        match avatar_file(app, &avatar) {
            Ok(path) => {
                if let Err(error) = start_call(app, &json!({"op":"avatar","path": path}).to_string()) {
                    app.ui.status = error;
                    app.ui.status_error = true;
                }
            }
            Err(error) => {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
    }
}
