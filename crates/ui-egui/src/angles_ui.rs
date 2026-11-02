//! Edit › Generative Angles… (ShotoPhopEX, plan Phase 5): the picture seen from another camera,
//! with fal's Multiple Angles LoRA for Qwen-Image-Edit-2511 (`generate.angles`). A small window
//! beside the canvas: an orbit widget (the subject in the middle, the camera dragged around it
//! and up or down, snapping to the 8 directions and 4 heights the LoRA knows), the direction,
//! height and distance as menus, the number of variations, and Generate with its estimate. The
//! window stays open so angle after angle can be tried; the results land in the Variations strip.

use std::f32::consts::PI;

use egui::{Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use photocraft_engine::generate_cmds::{ANGLE_ROTATIONS, ANGLE_TILTS, ANGLE_ZOOMS, ANGLES, ANGLES_TEMPLATE, angle_prompt, angles_template};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// The window's state in `UiState::angles`. Never saved.
#[derive(Clone, Debug, PartialEq)]
pub struct AnglesUi {
    pub open: bool,
    /// Degrees around the subject from the front (0, 45 … 315).
    pub rotation: u32,
    /// The camera's height (-30, 0, 30, 60).
    pub tilt: i32,
    /// 0 wide, 5 medium, 10 close-up.
    pub zoom: u32,
    pub variations: u8,
    /// The run it started and shows as running.
    pub job: Option<u64>,
}

impl Default for AnglesUi {
    fn default() -> Self {
        Self { open: false, rotation: 45, tilt: 0, zoom: 5, variations: 1, job: None }
    }
}

/// The menu item and the hub card: open the window (a picture must be open).
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != ANGLES || params.as_object().is_some_and(|o| !o.is_empty()) {
        return None;
    }
    if let Err(why) = photocraft_engine::commands::find(ANGLES).map_or(Ok(()), |c| (c.enabled)(&app.session)) {
        return Some(Err(why));
    }
    crate::generate_panel::load_catalog(app);
    crate::generate_panel::probe_once(app);
    app.ui.angles.open = true;
    Some(Ok(json!({"angles": true})))
}

/// Snap a free direction (degrees, any value) to the nearest of the 8.
pub fn snap_rotation(deg: f32) -> u32 {
    let d = deg.rem_euclid(360.0);
    (((d / 45.0).round() as u32) % 8) * 45
}

/// Snap a free height (degrees) to the nearest of the 4.
pub fn snap_tilt(deg: f32) -> i32 {
    ANGLE_TILTS.iter().map(|(t, _)| *t).min_by(|a, b| (*a as f32 - deg).abs().total_cmp(&(*b as f32 - deg).abs())).unwrap_or(0)
}

/// Where the camera sits on the orbit widget: the ring is seen from slightly above (the front
/// at the bottom, nearest), height lifts the camera.
fn camera_pos(c: Pos2, r: f32, rotation: u32, tilt: i32) -> Pos2 {
    let a = rotation as f32 * PI / 180.0;
    pos2(c.x + r * a.sin(), c.y + 0.35 * r * a.cos() - tilt as f32 / 60.0 * 0.75 * r)
}

/// The orbit widget; true when the camera moved.
fn orbit(ui: &mut egui::Ui, a: &mut AnglesUi, t: &Tokens) -> bool {
    let size = vec2(ui.available_width().min(240.0), 150.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let c = rect.center() + vec2(0.0, 18.0);
    let r = (size.x * 0.38).min(78.0);
    let p = ui.painter();
    p.rect_filled(rect, t.radius_sm, t.field);
    // The sphere and its ring (the ground plane), the 8 stops on it.
    p.circle_stroke(c - vec2(0.0, 0.2 * r), r * 0.95, Stroke::new(1.0, t.separator));
    let ring: Vec<Pos2> = (0..=48).map(|i| camera_pos(c, r, i * 360 / 48, 0)).collect();
    p.add(egui::Shape::line(ring, Stroke::new(1.0, t.text_faint)));
    for (deg, _) in ANGLE_ROTATIONS {
        p.circle_filled(camera_pos(c, r, deg, 0), 2.5, t.text_faint);
    }
    // The subject.
    let subject = Rect::from_center_size(c - vec2(0.0, 10.0), vec2(14.0, 22.0));
    p.rect_filled(subject, 3.0, t.text_dim);
    p.text(subject.center_bottom() + vec2(0.0, 8.0), egui::Align2::CENTER_CENTER, tl!("front"), egui::FontId::proportional(9.0), t.text_faint);
    // The camera, looking at the subject; closer is bigger.
    let cam = camera_pos(c, r, a.rotation, a.tilt);
    let foot = camera_pos(c, r, a.rotation, 0);
    p.line_segment([foot, cam], Stroke::new(1.0, t.text_faint));
    p.line_segment([cam, subject.center()], Stroke::new(1.0, t.accent.gamma_multiply(0.6)));
    let size = 5.0 + a.zoom as f32 * 0.5;
    p.circle_filled(cam, size, t.accent);
    p.circle_stroke(cam, size, Stroke::new(1.5, Color32::WHITE));
    // Drag or click: the nearest direction around, the nearest height up or down.
    let mut moved = false;
    if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
        let d = pos - c;
        // Undo the ring's squash to read the direction; the rest of the vertical offset is height.
        let rot = snap_rotation(d.x.atan2(d.y.max(-0.35 * r) / 0.35) * 180.0 / PI);
        let ground = camera_pos(c, r, rot, 0).y;
        let tilt = snap_tilt((ground - pos.y) / (0.75 * r) * 60.0);
        moved = (rot, tilt) != (a.rotation, a.tilt);
        a.rotation = rot;
        a.tilt = tilt;
    }
    let words = angle_prompt(a.rotation, a.tilt, a.zoom).unwrap_or_default();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, tl!("Camera")));
    resp.on_hover_text(words.trim_start_matches("<sks> "));
    moved
}

/// The window, while open.
pub fn window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.angles.open {
        return;
    }
    if app.session.active().is_none() {
        app.ui.angles.open = false;
        return;
    }
    let t = Tokens::get(ctx);
    let running = app.ui.angles.job.map(photocraft_engine::jobs::JobId).and_then(|j| app.session.job(j));
    if running.is_none() {
        app.ui.angles.job = None;
    }
    let eta = estimate(app);
    let template = angles_template(&app.session);
    let missing = template == ANGLES_TEMPLATE && app.ui.generate.catalog.iter().find(|m| m.id == ANGLES_TEMPLATE).is_some_and(|m| m.installed == Some(false));
    let mut open = true;
    let mut generate = false;
    let mut a = app.ui.angles.clone();
    egui::Window::new(tl!("Generative Angles")).id(egui::Id::unique("generative-angles")).open(&mut open).resizable(false).default_width(260.0).show(
        ctx,
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            orbit(ui, &mut a, &t);
            let row = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
                ui.horizontal(|ui| {
                    ui.add_sized(vec2(64.0, 20.0), egui::Label::new(egui::RichText::new(label).color(t.text_dim)));
                    add(ui);
                });
            };
            let dirs: Vec<(u32, &str)> = ANGLE_ROTATIONS.iter().map(|(d, w)| (*d, tl!(*w))).collect();
            row(ui, tl!("Direction"), &mut |ui| {
                crate::widgets::dropdown(ui, "angles-rotation", tl!("Rotation"), &mut a.rotation, &dirs, 170.0);
            });
            let tilts: Vec<(i32, &str)> = ANGLE_TILTS.iter().map(|(d, w)| (*d, tl!(*w))).collect();
            row(ui, tl!("Height"), &mut |ui| {
                crate::widgets::dropdown(ui, "angles-tilt", tl!("Tilt"), &mut a.tilt, &tilts, 170.0);
            });
            let zooms: Vec<(u32, &str)> = ANGLE_ZOOMS.iter().map(|(d, w)| (*d, tl!(*w))).collect();
            row(ui, tl!("Distance"), &mut |ui| {
                crate::widgets::dropdown(ui, "angles-zoom", tl!("Zoom"), &mut a.zoom, &zooms, 170.0);
            });
            row(ui, tl!("Variations"), &mut |ui| {
                crate::widgets::dropdown(ui, "angles-variations", tl!("Variations"), &mut a.variations, &[(1u8, "1"), (2, "2"), (3, "3"), (4, "4")], 50.0);
            });
            if missing {
                ui.label(
                    egui::RichText::new(tl!(
                        "Needs qwen-image-edit-2511-multiple-angles-lora.safetensors in the Models folder’s loras subfolder (fal, Apache-2.0)"
                    ))
                    .small()
                    .color(t.warning),
                );
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| match &running {
                Some(j) => {
                    let label = format!("{} · {:.0}%", tl!("Cancel"), j.progress.clamp(0.0, 1.0) * 100.0);
                    if crate::widgets::secondary_button(ui, &label, 150.0).clicked() {
                        crate::jobs_ui::cancel(app, j.id);
                    }
                }
                None => {
                    let label = match eta.map(crate::generate_panel::eta_label).filter(|s| !s.is_empty()) {
                        Some(e) => format!("{} · {e}", tl!("Generate")),
                        None => tl!("Generate").to_string(),
                    };
                    generate = crate::widgets::primary_button(ui, &label, 150.0).clicked();
                }
            });
        },
    );
    a.job = app.ui.angles.job;
    app.ui.angles = AnglesUi { open: open && app.ui.angles.open, ..a };
    if generate {
        let _ = run(app);
    }
}

/// Seconds the run will most likely take (the template's formula at the document's size).
fn estimate(app: &PhotocraftApp) -> Option<f32> {
    let d = app.session.active()?;
    let (w, h) = (d.doc.size.width, d.doc.size.height);
    photocraft_engine::generate_cmds::estimate_seconds(angles_template(&app.session), w, h, u32::from(app.ui.angles.variations.max(1)), 0)
}

/// Generate: `generate.angles` with the window's camera (a background job in the desktop app).
pub fn run(app: &mut PhotocraftApp) -> Result<Value, String> {
    let a = app.ui.angles.clone();
    let params = json!({"rotation": a.rotation, "tilt": a.tilt, "zoom": a.zoom, "variations": a.variations.clamp(1, 4)});
    match app.run(ANGLES, params) {
        Ok(v) => {
            app.ui.angles.job = v.get("pending").and_then(Value::as_bool).filter(|p| *p).and_then(|_| v.get("job").and_then(Value::as_u64));
            Ok(v)
        }
        Err(e) => {
            crate::notices::error(app, format!("{}: {e}", tl!("Generative Angles")));
            Err(e)
        }
    }
}

/// `ui.inspect.angles`.
pub fn inspect(app: &PhotocraftApp) -> Value {
    let a = &app.ui.angles;
    json!({
        "open": a.open, "rotation": a.rotation, "tilt": a.tilt, "zoom": a.zoom, "variations": a.variations, "job": a.job,
        "prompt": angle_prompt(a.rotation, a.tilt, a.zoom), "eta": estimate(app),
    })
}

/// `ui.set {angles: {open, rotation, tilt, zoom, variations, generate}}`: every field checked
/// before any is applied; `generate: true` runs it.
pub fn set(app: &mut PhotocraftApp, p: &Value) -> Result<(), String> {
    let Some(v) = p.get("angles") else { return Ok(()) };
    let o = v.as_object().ok_or("angles must be an object")?;
    if let Some(k) = o.keys().find(|k| !["open", "rotation", "tilt", "zoom", "variations", "generate"].contains(&k.as_str())) {
        return Err(format!("angles has no field `{k}`"));
    }
    let mut next = app.ui.angles.clone();
    if let Some(x) = o.get("open") {
        next.open = x.as_bool().ok_or("angles.open must be true or false")?;
    }
    if let Some(x) = o.get("rotation") {
        next.rotation = x.as_u64().filter(|r| ANGLE_ROTATIONS.iter().any(|(d, _)| u64::from(*d) == *r)).ok_or("angles.rotation must be 0, 45, … 315")? as u32;
    }
    if let Some(x) = o.get("tilt") {
        next.tilt = x.as_i64().filter(|r| ANGLE_TILTS.iter().any(|(d, _)| i64::from(*d) == *r)).ok_or("angles.tilt must be -30, 0, 30 or 60")? as i32;
    }
    if let Some(x) = o.get("zoom") {
        next.zoom = x.as_u64().filter(|r| ANGLE_ZOOMS.iter().any(|(d, _)| u64::from(*d) == *r)).ok_or("angles.zoom must be 0, 5 or 10")? as u32;
    }
    if let Some(x) = o.get("variations") {
        next.variations = x.as_u64().filter(|n| (1..=4).contains(n)).ok_or("angles.variations must be 1 to 4")? as u8;
    }
    let generate = match o.get("generate") {
        Some(x) => x.as_bool().ok_or("angles.generate must be true or false")?,
        None => false,
    };
    if next.open && app.session.active().is_none() {
        return Err("Generative Angles needs an open picture".into());
    }
    app.ui.angles = next;
    if generate {
        run(app)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "angles_ui_tests.rs"]
mod tests;
