//! QR code recognition: the dock panel next to "Revision" (Layers group, tab "QR").
//!
//! The panel lists every code the engine's read-only `qr.scan` query decoded (content + location)
//! with Copy and Open Link actions, and can re-scan the whole document or the current selection.
//! Because `qr.scan` is a query, a re-scan never touches the history.
//!
//! "Open Link" shows the page in PhotoCraft's own preview window ([`crate::web_preview`]), which is
//! a real desktop window the user can move anywhere and resize like any other; when WebView2 is
//! unavailable the system browser opens it instead.
//!
//! State lives in [`PhotocraftApp`]'s `ui.qr`, serde-round-tripped with the workspace.

use serde_json::{Value, json};

use crate::PhotocraftApp;

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QrState {
    /// The codes decoded by the last scan, in document coordinates.
    pub codes: Vec<QrCode>,
    /// The last scan failed (message to show in the panel).
    pub error: Option<String>,
    /// The last scan's mode: `Some(true)` = selection, `Some(false)` = whole document.
    pub selection_mode: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QrCode {
    pub x: f64,
    pub y: f64,
    pub width: i64,
    pub height: i64,
    pub content: String,
}

/// Run the engine's `qr.scan` query and keep its result in `app.ui.qr`.
pub fn scan(app: &mut PhotocraftApp, selection_only: bool) -> Result<Value, String> {
    let r = app.run("qr.scan", json!({"selectionOnly": selection_only}))?;
    let qr = &mut app.ui.qr;
    qr.codes = r["codes"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| QrCode {
                    x: c["x"].as_f64().unwrap_or(0.0),
                    y: c["y"].as_f64().unwrap_or(0.0),
                    width: c["width"].as_i64().unwrap_or(0),
                    height: c["height"].as_i64().unwrap_or(0),
                    content: c["content"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    qr.error = None;
    qr.selection_mode = Some(selection_only);
    Ok(r)
}

/// Show `url` in the in-app preview window, falling back to the system browser.
pub fn open_url(app: &mut PhotocraftApp, ctx: &egui::Context, url: &str) {
    if let Err(e) = crate::web_preview::open(app, url) {
        crate::web_preview::set_notice(app, format!("!{e}"));
        let _ = crate::links::open(app, ctx, url);
    }
}

/// The "QR" dock panel: scan buttons, the last scan's status and its decoded codes.
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut scan_mode: Option<bool> = None;
    let mut copy: Option<String> = None;
    let mut open_link: Option<String> = None;

    ui.horizontal(|ui| {
        if crate::widgets::secondary_button(ui, tl!("Scan Document"), 0.0).clicked() {
            scan_mode = Some(false);
        }
        if crate::widgets::secondary_button(ui, tl!("Scan Selection"), 0.0).clicked() {
            scan_mode = Some(true);
        }
    });
    ui.add_space(4.0);
    if let Some(err) = &app.ui.qr.error {
        ui.colored_label(ui.visuals().error_fg_color, err);
    } else if let Some(mode) = app.ui.qr.selection_mode {
        let n = app.ui.qr.codes.len();
        let scope = if mode { tl!("the selection") } else { tl!("the document") };
        if n == 0 {
            ui.colored_label(t.text_dim, crate::i18n::fmt(tl!("No QR code found in {scope}"), &[("scope", scope)]));
        } else {
            let n_label = if n == 1 { tl!("1 QR code found").to_string() } else { crate::i18n::fmt(tl!("{n} QR codes found"), &[("n", &n.to_string())]) };
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Found in {scope}: {n_label}"), &[("scope", scope), ("n_label", &n_label)])).color(t.text_dim));
        }
    }
    if app.ui.qr.codes.is_empty() {
        if app.ui.qr.selection_mode.is_none() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(tl!("Scan a document to recognize QR codes; scan a selection to recognize only the codes inside it.")).color(t.text_dim).size(12.0));
        }
        if let Some(mode) = scan_mode {
            if let Err(e) = scan(app, mode) {
                app.ui.qr.error = Some(e);
            }
        }
        return;
    }
    ui.add_space(6.0);
    crate::widgets::hairline(ui);
    let codes = app.ui.qr.codes.clone();
    for (i, c) in codes.iter().enumerate() {
        if i > 0 {
            ui.add_space(6.0);
            crate::widgets::hairline(ui);
            ui.add_space(6.0);
        }
        ui.label(egui::RichText::new(&c.content));
        let where_label = crate::i18n::fmt(tl!("({x}, {y}) · {w}×{h} px"), &[("x", &(c.x as i64).to_string()), ("y", &(c.y as i64).to_string()), ("w", &c.width.to_string()), ("h", &c.height.to_string())]);
        ui.label(egui::RichText::new(where_label).color(t.text_dim).size(11.0));
        ui.horizontal(|ui| {
            if crate::widgets::secondary_button(ui, tl!("Copy"), 0.0).clicked() {
                copy = Some(c.content.clone());
            }
            let is_link = c.content.starts_with("http://") || c.content.starts_with("https://");
            if is_link && crate::widgets::secondary_button(ui, tl!("Open Link"), 0.0).clicked() {
                open_link = Some(c.content.clone());
            }
        });
    }
    if let Some(mode) = scan_mode {
        if let Err(e) = scan(app, mode) {
            app.ui.qr.error = Some(e);
        }
    }
    if let Some(text) = copy {
        ui.ctx().copy_text(text);
    }
    if let Some(url) = open_link {
        open_url(app, ui.ctx(), &url);
    }
}

/// Called with the other floating panels: keeps the preview window alive and runs its actions.
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::web_preview::pump(app, ctx);
}
