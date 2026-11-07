//! PhotoCraft's own web preview: a separate desktop window, so the page can be moved anywhere on
//! the desktop and resized like any other window — an embedded panel is trapped inside
//! PhotoCraft's own window.
//!
//! The window is an egui deferred viewport: eframe creates a real OS window for it (with the title
//! asked for below), the user drags it by its own title bar and resizes it from its own borders.
//! The tool strip under that title bar is drawn by egui (zoom −/+, screenshot, open in browser,
//! close); a WebView2 child window covers everything below the strip. Because the page is a native
//! child window it swallows pointer events, which is why the controls live on the strip.
//!
//! The page and the window's handle live in `photocraft-webwin` (that crate is where the two
//! `unsafe` calls they need are allowed). Its handle is found by walking this process's windows for
//! the preview's title; the page is then created as a child of that window and kept sized to its
//! client area. When WebView2 cannot be created the caller falls back to the system browser.
//!
//! The page can be screenshotted into the selection box ([`crate::web_capture`] + the engine's
//! `edit.placeImage`): the window's client rectangle in screen coordinates is what gets captured.

use std::sync::{Arc, Mutex};

use crate::PhotocraftApp;

/// The tool strip's height in window points; the page covers everything below it.
pub const TITLE_H: f32 = 36.0;

/// The preview window's viewport.
pub fn viewport() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("photocraft-web-preview")
}

/// What the tool strip asks the main thread to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmd {
    ZoomIn,
    ZoomOut,
    ResetZoom,
    Shot,
    Browser,
    Close,
}

/// State the window's callback and the main thread share.
#[derive(Default)]
struct Shared {
    url: String,
    zoom: f64,
    notice: Option<String>,
    /// The window's content area on the desktop (window points), as its callback sees it.
    inner: Option<egui::Rect>,
    /// The window's own scale (points per pixel), likewise.
    ppp: f32,
    /// The window was closed with its own close button (or by the OS).
    close_requested: bool,
    /// Controls pressed since the last pump.
    cmds: Vec<Cmd>,
}

/// An open preview window.
pub struct Preview {
    pub url: String,
    pub zoom: f64,
    pub notice: Option<String>,
    pub notice_at: f64,
    shared: Arc<Mutex<Shared>>,
    /// The preview window's top-level handle, once eframe has created it.
    hwnd: Option<isize>,
    /// Consecutive frames the window's handle was not found yet.
    misses: u32,
    /// The page inside that window.
    page: Option<photocraft_webwin::Page>,
}

/// Show `url` in the preview window, opening it when it is not open yet; reuses the window when it
/// already shows that page. Returns an error when the platform cannot embed a browser, so the
/// caller can fall back to the system browser.
pub fn open(app: &mut PhotocraftApp, url: &str) -> Result<(), String> {
    if !cfg!(windows) {
        return Err(format!("in-app preview is only supported on Windows ({url})"));
    }
    if let Some(p) = app.web_preview.as_mut() {
        if p.url == url {
            return Ok(());
        }
        p.url = url.to_string();
        p.notice = None;
        p.notice_at = 0.0;
        let mut s = p.shared.lock().unwrap_or_else(|e| e.into_inner());
        s.url = url.to_string();
        s.notice = None;
        drop(s);
        if let Some(page) = p.page.as_ref() {
            page.load(url);
            page.zoom(p.zoom);
        }
        return Ok(());
    }
    let shared = Arc::new(Mutex::new(Shared { url: url.to_string(), zoom: 1.0, ppp: 1.0, ..Default::default() }));
    app.web_preview = Some(Preview { url: url.to_string(), zoom: 1.0, notice: None, notice_at: 0.0, shared, hwnd: None, misses: 0, page: None });
    Ok(())
}

/// Close the preview (dropping the page closes its WebView2 controller).
pub fn close(app: &mut PhotocraftApp) {
    app.web_preview = None;
}

/// The URL the preview is showing, if it is open.
pub fn url(app: &PhotocraftApp) -> Option<&str> {
    app.web_preview.as_ref().map(|p| p.url.as_str())
}

/// The page zoom (1.0 = 100 %).
pub fn zoom(app: &PhotocraftApp) -> f64 {
    app.web_preview.as_ref().map_or(1.0, |p| p.zoom)
}

/// Zoom the page. Clamped to 25 %–400 %, like a browser's own limits.
pub fn set_zoom(app: &mut PhotocraftApp, zoom: f64) {
    let zoom = zoom.clamp(0.25, 4.0);
    if let Some(p) = app.web_preview.as_mut() {
        p.zoom = zoom;
        if let Ok(mut s) = p.shared.lock() {
            s.zoom = zoom;
        }
        if let Some(page) = p.page.as_ref() {
            page.zoom(zoom);
        }
    }
}

/// The message the last screenshot left, if any.
pub fn notice(app: &PhotocraftApp) -> Option<&str> {
    app.web_preview.as_ref().and_then(|p| p.notice.as_deref())
}

/// Replace the message (a success note, or an error when it starts with `!`).
pub fn set_notice(app: &mut PhotocraftApp, msg: impl Into<String>) {
    let msg = msg.into();
    if let Some(p) = app.web_preview.as_mut() {
        p.notice = Some(msg.clone());
        p.notice_at = 0.0;
        if let Ok(mut s) = p.shared.lock() {
            s.notice = Some(msg);
        }
    }
}

/// Keep the preview window alive and run what its tool strip asked: called every frame from
/// `qr_ui::windows`.
pub fn pump(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.web_preview.is_none() {
        return;
    }
    let Some(shared) = app.web_preview.as_ref().map(|p| p.shared.clone()) else { return };
    let title = tl!("Web Preview").to_string();
    let builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size([980.0, 820.0])
        .with_min_inner_size([380.0, 280.0])
        .with_resizable(true);
    ctx.show_viewport_deferred(viewport(), builder, move |ctx, _class| draw_strip(ctx, &shared));
    sync(app, ctx);
    notice_bar(app, ctx);
    // The page is a native window: egui sees no input while the pointer is over it, so keep the
    // frames coming and the strip's own repaints cheap.
    ctx.request_repaint();
}

/// The tool strip drawn over the top of the preview window.
fn draw_strip(ctx: &egui::Context, shared: &Arc<Mutex<Shared>>) {
    let t = crate::theme::Tokens::get(ctx);
    let (url, zoom, notice, width) = {
        let mut s = match shared.lock() {
            Ok(s) => s,
            Err(e) => e.into_inner(),
        };
        // The strip is the only party that knows where the window is and how it is scaled.
        s.inner = ctx.input(|i| i.viewport().inner_rect);
        s.ppp = ctx.pixels_per_point();
        if ctx.input(|i| i.viewport().close_requested()) {
            s.close_requested = true;
        }
        // The window is found again by this title on every frame, so keep it exactly as asked for
        // (a shell can rename windows, and the lookup would silently stop matching).
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(tl!("Web Preview").to_string()));
        (s.url.clone(), s.zoom, s.notice.clone(), s.inner.map_or(960.0, |r| r.width()))
    };
    let mut cmds: Vec<Cmd> = Vec::new();
    egui::Area::new(egui::Id::unique("preview-tool-strip"))
        .fixed_pos(egui::pos2(0.0, 0.0))
        .order(egui::Order::Middle)
        .interactable(true)
        .show(ctx, |ui| {
            egui::Frame::NONE.fill(t.card).show(ui, |ui| {
                ui.set_min_width(width);
                ui.allocate_ui(egui::vec2(width, TITLE_H), |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(tl!("Web Preview")).strong());
                        ui.add_space(6.0);
                        let text = notice.clone().unwrap_or_else(|| short_url(&url, 46));
                        let color = if notice.as_deref().is_some_and(|n| n.starts_with('!')) {
                            ui.visuals().error_fg_color
                        } else {
                            t.text_dim
                        };
                        ui.label(egui::RichText::new(text.trim_start_matches('!')).color(color).size(11.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if crate::icons::button(ui, "x", 18.0, false, tl!("Close")).clicked() {
                                cmds.push(Cmd::Close);
                            }
                            if crate::widgets::secondary_button(ui, tl!("Open in Browser"), 0.0).clicked() {
                                cmds.push(Cmd::Browser);
                            }
                            if crate::widgets::secondary_button(ui, tl!("Screenshot"), 0.0).clicked() {
                                cmds.push(Cmd::Shot);
                            }
                            if crate::widgets::secondary_button(ui, "+", 0.0).clicked() {
                                cmds.push(Cmd::ZoomIn);
                            }
                            if crate::widgets::secondary_button(ui, &format!("{}%", (zoom * 100.0).round() as i32), 0.0).clicked() {
                                cmds.push(Cmd::ResetZoom);
                            }
                            if crate::widgets::secondary_button(ui, "-", 0.0).clicked() {
                                cmds.push(Cmd::ZoomOut);
                            }
                        });
                    });
                });
            });
        });
    if !cmds.is_empty() {
        if let Ok(mut s) = shared.lock() {
            s.cmds.extend(cmds);
        }
        // The main thread is idle while the pointer is over the page: wake it for the action.
        ctx.request_repaint();
    }
}

/// `url` shortened to `max` characters with an ellipsis (char-safe for multi-byte text).
fn short_url(url: &str, max: usize) -> String {
    let mut s: String = url.chars().take(max).collect();
    if url.chars().count() > max {
        s.push('…');
    }
    s
}

/// Follow the preview window: create the page once its handle exists, keep it sized to the client
/// area, then run the tool strip's actions here (on the thread the page lives in).
fn sync(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let mut shot: Option<(i32, i32, u32, u32)> = None;
    let mut browser_url: Option<String> = None;
    let mut do_close = false;
    let mut do_reset = false;
    let mut zoom_step = 0.0f64;
    {
        let Some(p) = app.web_preview.as_mut() else { return };
        if p.hwnd.is_none() {
            match photocraft_webwin::find_window(tl!("Web Preview")) {
                Some(h) => p.hwnd = Some(h),
                None => {
                    p.misses += 1;
                    // eframe creates the window on a later frame; say so when it never shows up.
                    if p.misses == 240 {
                        p.notice = Some(format!("!{}", tl!("Could not open the preview window")));
                        p.notice_at = 0.0;
                    }
                    ctx.request_repaint();
                    return;
                }
            }
        }
        let hwnd = p.hwnd.unwrap_or(0);
        let closed = p.shared.lock().map(|s| s.close_requested).unwrap_or(false);
        if closed || !photocraft_webwin::is_window(hwnd) {
            p.notice = None;
            do_close = true;
        } else {
            let ppp = f64::from(p.shared.lock().map(|s| s.ppp).unwrap_or(1.0).max(0.5));
            let strip = f64::from(TITLE_H) * ppp;
            // The OS reports the client area in device pixels, so neither the page nor the
            // screenshot has to guess at egui's points: this is the same rectangle Windows uses.
            let Some((cx, cy, cw, ch)) = photocraft_webwin::client_rect(hwnd) else {
                ctx.request_repaint();
                return;
            };
            let size = (f64::from(cw), f64::from(ch));
            if p.page.is_none() {
                match photocraft_webwin::attach(hwnd, &p.url, strip, size, p.zoom) {
                    Ok(page) => p.page = Some(page),
                    Err(e) => {
                        p.notice = Some(format!("!{e}"));
                        p.notice_at = 0.0;
                        do_close = true;
                    }
                }
            } else if let Some(page) = p.page.as_mut() {
                page.fit(strip, size);
            }
            let cmds = p.shared.lock().map(|mut s| std::mem::take(&mut s.cmds)).unwrap_or_default();
            for cmd in cmds {
                match cmd {
                    Cmd::Shot => {
                        shot = Some((
                            cx,
                            cy + strip.round() as i32,
                            cw,
                            (f64::from(ch) - strip).round().max(1.0) as u32,
                        ));
                    }
                    Cmd::Browser => browser_url = Some(p.url.clone()),
                    Cmd::Close => do_close = true,
                    Cmd::ResetZoom => do_reset = true,
                    Cmd::ZoomIn => zoom_step += 0.1,
                    Cmd::ZoomOut => zoom_step -= 0.1,
                }
            }
            if zoom_step != 0.0 || do_reset {
                let next = if do_reset { 1.0 } else { p.zoom + zoom_step };
                let next = next.clamp(0.25, 4.0);
                p.zoom = next;
                if let Ok(mut s) = p.shared.lock() {
                    s.zoom = next;
                }
                if let Some(page) = p.page.as_ref() {
                    page.zoom(next);
                }
            }
        }
    }
    if do_close {
        close(app);
        ctx.send_viewport_cmd_to(viewport(), egui::ViewportCommand::Close);
        return;
    }
    if let Some(url) = browser_url {
        let _ = crate::links::open(app, ctx, &url);
    }
    if let Some(rect) = shot {
        place_shot(app, rect);
        // The document changed on this thread's watch: ask for the frame that shows it, otherwise
        // the canvas keeps its previous pixels until some other input arrives.
        ctx.request_repaint();
    }
}

/// Take the page from the screen, strip the 3D view's XYZ axes, and let the engine fit it into the
/// rectangle drawn with the marquee tool.
fn place_shot(app: &mut PhotocraftApp, (x, y, w, h): (i32, i32, u32, u32)) {
    use serde_json::json;
    // Anything covering the preview would be captured instead of the page — the main window most
    // of all. Raise the preview and give the compositor a moment to draw it before reading.
    if let Some(hwnd) = app.web_preview.as_ref().and_then(|p| p.hwnd) {
        photocraft_webwin::bring_to_front(hwnd);
        std::thread::sleep(std::time::Duration::from_millis(180));
    }
    match crate::web_capture::capture_screen(x, y, w, h) {
        Ok(mut shot) => {
            crate::web_capture::strip_axes(&mut shot);
            match crate::web_capture::write_png(&shot) {
                Ok(path) => match app.run("edit.placeImage", json!({ "path": path })) {
                    Ok(_) => set_notice(app, tl!("Screenshot placed on the canvas").to_string()),
                    Err(e) => set_notice(app, format!("!{e}")),
                },
                Err(e) => set_notice(app, format!("!{e}")),
            }
        }
        Err(e) => set_notice(app, format!("!{e}")),
    }
}

/// A short-lived note over the canvas (the screenshot's result, or why it failed).
fn notice_bar(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    let Some(p) = app.web_preview.as_ref() else { return };
    let Some(text) = p.notice.clone() else { return };
    if p.notice_at == 0.0 {
        if let Some(p) = app.web_preview.as_mut() {
            p.notice_at = now;
        }
    } else if now - p.notice_at > 8.0 {
        if let Some(p) = app.web_preview.as_mut() {
            p.notice = None;
            p.notice_at = 0.0;
        }
        return;
    }
    let t = crate::theme::Tokens::get(ctx);
    let error = text.starts_with('!');
    let body = text.trim_start_matches('!').to_string();
    let at = egui::pos2(app.last_canvas_rect.center().x - 200.0, app.last_canvas_rect.top() + 12.0);
    egui::Area::new(egui::Id::unique("qr-shot-notice")).fixed_pos(at).order(egui::Order::Tooltip).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(t.card)
            .stroke(egui::Stroke::new(1.0, t.card_border))
            .corner_radius(egui::CornerRadius::same(t.radius_lg as u8))
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_max_width(400.0);
                let color = if error { ui.visuals().error_fg_color } else { t.text };
                ui.label(egui::RichText::new(body).color(color).size(13.0));
            });
    });
    ctx.request_repaint();
}
