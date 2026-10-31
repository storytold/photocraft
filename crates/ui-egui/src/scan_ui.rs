//! File › Import › Scan… A small dialog: pick a scanner, a resolution and a mode, then open the
//! page as a document. Listing and scanning block the main thread (Image Capture delivers its
//! callbacks there), so each one is armed for a frame first and the window can paint its status.

use egui::{Align2, RichText, vec2};

use crate::theme::Tokens;
use crate::{PhotocraftApp, ScanBuffer, ScanRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    /// Set on the frame the user asks. The next frame only paints, and the one after does the work.
    ArmList,
    List,
    ArmAcquire,
    Acquire,
}

/// Runtime state for the scan dialog. Not part of [`crate::UiState`]: it is not saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScanUi {
    open: bool,
    devices: Vec<crate::ScanDevice>,
    selected: usize,
    resolution: u32,
    /// 0 colour, 1 gray, 2 black and white.
    mode: u8,
    status: String,
    phase: Phase,
}

impl Default for ScanUi {
    fn default() -> Self {
        Self { open: false, devices: Vec::new(), selected: 0, resolution: 300, mode: 0, status: String::new(), phase: Phase::Idle }
    }
}

/// Open the dialog and look for scanners on the following frames.
pub(crate) fn open(app: &mut PhotocraftApp) {
    app.scan = ScanUi {
        open: true,
        devices: Vec::new(),
        selected: 0,
        resolution: 300,
        mode: 0,
        status: tl!("Looking for scanners…").to_string(),
        phase: Phase::ArmList,
    };
}

/// Move an armed list or scan forward by one stage, and run it when its stage comes due.
pub(crate) fn advance(app: &mut PhotocraftApp) {
    if !app.scan.open {
        return;
    }
    match app.scan.phase {
        Phase::ArmList => app.scan.phase = Phase::List,
        Phase::List => {
            app.scan.phase = Phase::Idle;
            list_now(app);
        }
        Phase::ArmAcquire => app.scan.phase = Phase::Acquire,
        Phase::Acquire => {
            app.scan.phase = Phase::Idle;
            acquire_now(app);
        }
        Phase::Idle => {}
    }
}

pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.scan.open {
        return;
    }
    advance(app);
    if !app.scan.open {
        return;
    }
    let t = Tokens::get(ctx);
    let mut close = false;
    let names: Vec<String> = app.scan.devices.iter().map(|d| d.name.clone()).collect();
    let mut selected = if names.is_empty() { 0 } else { app.scan.selected.min(names.len() - 1) };
    let mut resolution = app.scan.resolution;
    let mut mode = app.scan.mode;
    let busy = app.scan.phase != Phase::Idle;
    let status = app.scan.status.clone();
    egui::Window::new(tl!("Scan"))
        .id(egui::Id::unique("photocraft-scan"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_min_width(280.0);
            ui.label(RichText::new(tl!("Scan")).color(t.text).size(13.0).strong());
            crate::widgets::hairline(ui);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Scanner")).color(t.text_dim).size(11.0));
                let shown = names.get(selected).cloned().unwrap_or_else(|| tl!("No scanner found").to_string());
                craft_ui::ComboBox::from_id_salt("scan-device").selected_text(shown).width(180.0).show_ui(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut selected, i, name);
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Resolution")).color(t.text_dim).size(11.0));
                craft_ui::ComboBox::from_id_salt("scan-dpi").selected_text(format!("{resolution}")).width(80.0).show_ui(ui, |ui| {
                    for dpi in [75, 150, 300, 600] {
                        ui.selectable_value(&mut resolution, dpi, dpi.to_string());
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Mode")).color(t.text_dim).size(11.0));
                let label = mode_label(mode);
                craft_ui::ComboBox::from_id_salt("scan-mode").selected_text(label).width(140.0).show_ui(ui, |ui| {
                    for (value, name) in [(0, tl!("Color")), (1, tl!("Grayscale")), (2, tl!("Black and White"))] {
                        ui.selectable_value(&mut mode, value, name);
                    }
                });
            });
            if !status.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(&status).color(t.text).size(11.0));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new(tl!("Refresh"))).clicked() {
                    app.scan.phase = Phase::ArmList;
                    app.scan.status = tl!("Looking for scanners…").to_string();
                }
                let can_scan = !busy && !names.is_empty();
                if ui.add_enabled(can_scan, egui::Button::new(tl!("Scan"))).clicked() {
                    app.scan.phase = Phase::ArmAcquire;
                    app.scan.status = tl!("Scanning…").to_string();
                }
                if ui.button(tl!("Cancel")).clicked() {
                    close = true;
                }
            });
        });
    app.scan.selected = selected;
    app.scan.resolution = resolution;
    app.scan.mode = mode;
    if close {
        app.scan.open = false;
        app.scan.phase = Phase::Idle;
    } else if app.scan.phase != Phase::Idle {
        ctx.request_repaint();
    }
}

fn mode_label(mode: u8) -> String {
    match mode {
        1 => tl!("Grayscale").to_string(),
        2 => tl!("Black and White").to_string(),
        _ => tl!("Color").to_string(),
    }
}

fn mode_key(mode: u8) -> &'static str {
    match mode {
        1 => "gray",
        2 => "bw",
        _ => "color",
    }
}

fn list_now(app: &mut PhotocraftApp) {
    let Some(list) = app.services.scan_list.as_ref() else {
        app.scan.devices.clear();
        app.scan.status = tl!("Scanning needs the desktop app on this computer.").to_string();
        return;
    };
    match list(std::time::Duration::from_secs(3)) {
        Ok(devices) => {
            app.scan.devices = devices;
            if app.scan.selected >= app.scan.devices.len() {
                app.scan.selected = 0;
            }
            app.scan.status =
                if app.scan.devices.is_empty() { tl!("No scanner is connected. Check the cable, then press Refresh.").to_string() } else { String::new() };
        }
        Err(err) => {
            app.scan.devices.clear();
            app.scan.status = err;
        }
    }
}

fn acquire_now(app: &mut PhotocraftApp) {
    let Some(device) = app.scan.devices.get(app.scan.selected).cloned() else {
        app.scan.status = tl!("No scanner is selected.").to_string();
        return;
    };
    let Some(acquire) = app.services.scan_acquire.as_ref() else {
        app.scan.status = tl!("Scanning needs the desktop app on this computer.").to_string();
        return;
    };
    let request = ScanRequest { device_id: device.id, resolution: app.scan.resolution, mode: mode_key(app.scan.mode).to_string() };
    let buffer = match acquire(request) {
        Ok(buffer) => buffer,
        Err(err) => {
            app.scan.status = err;
            return;
        }
    };
    let pages = buffer.pages;
    match photocraft_engine::scan_cmds::open_scanned(&mut app.session, scanned_image(&device.name, app.scan.mode, &buffer)) {
        Ok(_) => {
            app.sync_views();
            app.scan.open = false;
            app.scan.phase = Phase::Idle;
            if pages > 1 {
                crate::notices::post(app, tl!("Scan"), vec![tl!("Only the first page was opened.").to_string()], false, None);
            }
        }
        Err(err) => app.scan.status = err.to_string(),
    }
}

fn scanned_image(name: &str, mode: u8, buffer: &ScanBuffer) -> photocraft_engine::scan_cmds::ScannedImage {
    let pixels = if mode == 0 {
        photocraft_engine::scan_cmds::ScannedPixels::Rgba(buffer.rgba.clone())
    } else {
        let gray = buffer.rgba.as_chunks::<4>().0.iter().map(|px| px[0]).collect();
        photocraft_engine::scan_cmds::ScannedPixels::Gray(gray)
    };
    photocraft_engine::scan_cmds::ScannedImage {
        name: if name.is_empty() { "Scan".into() } else { name.to_string() },
        width: buffer.width,
        height: buffer.height,
        resolution_dpi: if buffer.resolution == 0 { 300.0 } else { buffer.resolution as f32 },
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PhotocraftApp, ScanDevice, Services};
    use photocraft_engine::Session;
    use serde_json::json;

    fn app(services: Services) -> (PhotocraftApp, egui::Context) {
        (PhotocraftApp::new(Session::new(), services), egui::Context::default())
    }

    #[test]
    fn the_menu_opens_the_dialog_and_explains_a_missing_scanner_service() {
        let (mut app, ctx) = app(Services::default());
        crate::menus::invoke(&mut app, &ctx, "file.import.scan", json!({})).unwrap();
        assert!(app.scan.open);
        advance(&mut app);
        assert_eq!(app.scan.phase, Phase::List);
        advance(&mut app);
        assert!(!app.scan.open || app.scan.phase == Phase::Idle);
        assert!(app.scan.status.contains("desktop app"));
        assert!(app.session.documents().is_empty());
    }

    #[test]
    fn a_scanned_page_opens_as_a_document() {
        let services = Services {
            scan_list: Some(Box::new(|_| Ok(vec![ScanDevice { id: "desk".into(), name: "Desk".into() }]))),
            scan_acquire: Some(Box::new(|req| {
                assert_eq!(req.device_id, "desk");
                assert_eq!(req.resolution, 300);
                Ok(ScanBuffer { width: 2, height: 1, rgba: vec![255, 0, 0, 255, 0, 255, 0, 255], resolution: req.resolution, pages: 1 })
            })),
            ..Default::default()
        };
        let (mut app, ctx) = app(services);
        crate::menus::invoke(&mut app, &ctx, "file.import.scan", json!({})).unwrap();
        advance(&mut app);
        advance(&mut app);
        assert_eq!(app.scan.devices.len(), 1);
        app.scan.phase = Phase::ArmAcquire;
        advance(&mut app);
        advance(&mut app);
        assert!(!app.scan.open);
        let doc = &app.session.active().unwrap().doc;
        assert_eq!((doc.size.width, doc.size.height, doc.mode), (2, 1, photocraft_color::ColorMode::Rgb));
        assert_eq!(doc.layers[0].surface().unwrap().pixel(0, 0), vec![1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn pixel_params_skip_the_dialog() {
        let (mut app, ctx) = app(Services::default());
        let rgba = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [0u8, 0, 0, 255]);
        crate::menus::invoke(&mut app, &ctx, "file.import.scan", json!({"width": 1, "height": 1, "rgba": rgba})).unwrap();
        assert!(!app.scan.open);
        assert_eq!(app.session.documents().len(), 1);
    }
}
