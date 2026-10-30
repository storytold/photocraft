//! Submission dialog and optional gallery/profile controls.
use super::{DEVELOPER_PAGE, PhotocraftApp, REDIRECT_URI, Value, errors, json, start_connect};

pub(super) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut serde_json::Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.set_min_width(440.0);
    ui.label(egui::RichText::new(tl!("Submitting needs an internet connection.")).color(t.text_dim));
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(tl!(
            "Create a Public app on DeviantArt, without a client secret. Copy the address below into the redirect field, paste the client id here, then connect."
        ))
        .color(t.text_dim),
    );
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.monospace(REDIRECT_URI);
        if crate::widgets::secondary_button(ui, tl!("Copy"), 72.0).clicked() {
            ui.ctx().copy_text(REDIRECT_URI.to_string());
            app.ui.status = tl!("Copied the redirect address.").to_string();
            app.ui.status_error = false;
        }
    });
    ui.add_space(4.0);
    if crate::widgets::secondary_button(ui, tl!("Open the DeviantArt developer page"), 260.0).clicked() {
        crate::links::open(app, ui.ctx(), DEVELOPER_PAGE);
    }
    ui.add_space(8.0);
    ui.label(tl!("DeviantArt app id"));
    let mut client_id = f.get("clientId").and_then(Value::as_str).unwrap_or("").to_string();
    if ui.add(egui::TextEdit::singleline(&mut client_id).desired_width(ui.available_width())).changed() {
        f.insert("clientId".into(), json!(client_id.clone()));
    }
    ui.label(egui::RichText::new(tl!("The DeviantArt login stays in the system keychain, not in PhotoCraft's settings file.")).color(t.text_dim));
    ui.add_space(4.0);
    let connected = app.deviantart.ready();
    if connected {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Connected")).strong());
            if crate::widgets::secondary_button(ui, tl!("Connect again"), 150.0).clicked() {
                app.deviantart.client_id = client_id.clone();
                if let Err(error) = start_connect(app, "") {
                    app.ui.status = error;
                    app.ui.status_error = true;
                }
            }
            ui.add_enabled_ui(app.deviantart_job.is_none(), |ui| {
                if crate::widgets::secondary_button(ui, tl!("Disconnect DeviantArt"), 170.0).clicked() {
                    errors::disconnect(app);
                }
            });
        });
    } else if crate::widgets::secondary_button(ui, tl!("Connect DeviantArt"), 220.0).clicked() {
        app.deviantart.client_id = client_id;
        if let Err(error) = start_connect(app, "") {
            app.ui.status = error;
            app.ui.status_error = true;
        }
    }
    ui.add_space(8.0);
    ui.label(tl!("Title"));
    let mut title = f.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    if ui.add(egui::TextEdit::singleline(&mut title).desired_width(ui.available_width())).changed() {
        f.insert("title".into(), json!(title));
    }
    ui.label(tl!("Tags"));
    let mut tags = f.get("tags").and_then(Value::as_str).unwrap_or("").to_string();
    if ui.add(egui::TextEdit::singleline(&mut tags).desired_width(ui.available_width())).changed() {
        f.insert("tags".into(), json!(tags));
    }
    ui.label(egui::RichText::new(tl!("Letters, numbers and underscores, separated by commas.")).color(t.text_dim));
    ui.add_space(8.0);
    ui.label(tl!("Submit as"));
    let format = f.get("format").and_then(Value::as_str).unwrap_or("psd").to_string();
    ui.horizontal(|ui| {
        for (id, label) in [("file", tl!("Saved file")), ("psd", "PSD"), ("png", "PNG")] {
            if ui.selectable_label(format == id, label).clicked() {
                f.insert("format".into(), json!(id));
            }
        }
    });
    ui.label(egui::RichText::new(tl!("PNG is what DeviantArt shows. A PSD is only stored as a file in Sta.sh.")).color(t.text_dim));
    ui.label(egui::RichText::new(tl!("OK uploads to Sta.sh. Nothing is public until you publish.")).color(t.text_dim));
    ui.add_space(8.0);
    let mut more = f.get("more").and_then(Value::as_bool).unwrap_or(false);
    if crate::widgets::checkbox(ui, &mut more, tl!("More options")).changed() {
        f.insert("more".into(), json!(more));
    }
    if !more {
        return;
    }
    ui.add_space(4.0);
    let mut publishing = f.get("publishingOptions").and_then(Value::as_bool).unwrap_or(false);
    if crate::widgets::checkbox(ui, &mut publishing, tl!("Publishing options")).changed() {
        f.insert("publishingOptions".into(), json!(publishing));
    }
    if publishing {
        super::publish_ui::body(app, ui, f);
    }
    let mut management = f.get("managementOptions").and_then(Value::as_bool).unwrap_or(false);
    if crate::widgets::checkbox(ui, &mut management, tl!("Gallery and profile")).changed() {
        f.insert("managementOptions".into(), json!(management));
    }
    if management {
        super::management_ui::body(app, ui, f);
    }
    ui.add_space(8.0);
    crate::widgets::section_label(ui, "Submit automatically");
    ui.add_space(4.0);
    for (key, label) in [
        ("onSave", tl!("Also submit when I Save")),
        ("onSaveAs", tl!("Also submit when I Save As")),
        ("onSaveCopy", tl!("Also submit when I Save a Copy")),
        ("onExport", tl!("Also submit when I Export")),
    ] {
        let mut on = f.get(key).and_then(Value::as_bool).unwrap_or(false);
        if crate::widgets::checkbox(ui, &mut on, label).changed() {
            f.insert(key.into(), json!(on));
        }
    }
}
