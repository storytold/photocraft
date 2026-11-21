//! Edit › Content-Aware Scale as Photoshop has it (#3096): no dialog, a transform box on the layer
//! (or the selection) that only scales and moves. The options bar has the reference point X/Y,
//! W/H, Amount, Protect (an alpha channel) and Protect Skin Tones. While the box changes, the
//! command's own seam carving runs on a reduced copy in the background and its latest result is
//! drawn through the box; ↩ runs `edit.contentAwareScale` with the box's size and position (one
//! history step) and Esc cancels.
//!
//! With params (actions, the CLI, MCP, the control channel) the command runs directly.

use std::sync::mpsc::{Receiver, TryRecvError};

use craft::imaging::ui::transform_preview::{PreviewPixels, PreviewTextures};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, FieldLayout, Props as _, Show as _, ShowContainer as _, Sizing};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::UnitKind;
use craft::ui::text::NumberField;
use egui::ColorImage;
use photocraft_engine::edit_menu_cmds::ContentAwareScalePreview;
use photocraft_engine::jobs::JobCtx;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{ContentAwareOptions, FreeTransform};

pub const COMMAND: &str = "edit.contentAwareScale";

/// Long side of the live preview's reduced copy, px: carving it takes milliseconds, so the
/// preview follows a drag.
const PROXY_SIDE: u32 = 384;

/// How long a preview may run for a box that has changed since before it is cancelled. Shorter
/// would cancel every preview during a drag, so none would show until the drag stops (the filter
/// previews' settle time).
const STALE_MS: f64 = 150.0;

/// The preview being computed: what for, its result channel and its cancellation.
struct Running {
    key: Value,
    rx: Receiver<Result<ColorImage, String>>,
    job: JobCtx,
    started_ms: f64,
}

/// The live preview of an open Content-Aware Scale box (kept with the Free Transform preview).
#[derive(Default)]
pub(crate) struct Live {
    running: Option<Running>,
    /// The latest finished preview and what it was computed for.
    shown: Option<(Value, PreviewTextures)>,
    /// The key of the latest preview that failed or was refused: `tick` does not start it again
    /// while the box still asks for it, so a failing box costs nothing per frame.
    failed: Option<Value>,
    /// Texture names must differ from one result to the next.
    serial: u64,
}

impl Drop for Live {
    fn drop(&mut self) {
        // A closed box stops its preview at the next seam.
        if let Some(r) = &self.running {
            r.job.cancel();
        }
    }
}

impl Live {
    /// The latest result to draw through the box `t`, with how many document px one of its
    /// texels covers there (so the texture picks the right level). `None` until a preview has
    /// finished, and for an unscaled box (the full-resolution pixels show then).
    pub(crate) fn shown(&self, t: &FreeTransform) -> Option<(&PreviewTextures, f32)> {
        let (_, tex) = self.shown.as_ref()?;
        let [w, h] = box_size(t);
        if [w, h] == source_size(t) {
            return None;
        }
        let [tw, th] = tex.size();
        let texel = (w / tw.max(1) as f64).max(h / th.max(1) as f64);
        Some((tex, if texel.is_finite() { texel as f32 } else { 1.0 }))
    }
}

/// Edit › Content-Aware Scale from the menu: open the box (`Err` says why it can't).
pub fn begin(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<Value, String> {
    if app.ui.transform.is_some() {
        return Err("Commit or cancel the transform first".into());
    }
    if let Some(why) = app.session.disabled_reason(COMMAND) {
        return Err(why);
    }
    // The command's own checks and area (the selection's bounds, else the layer's content).
    let region = ContentAwareScalePreview::new(&app.session, &json!({}), PROXY_SIDE).map_err(|e| e.to_string())?.region();
    crate::transform_tool::begin_content_aware(app, ctx, region, Live::default())?;
    Ok(json!({"transform": app.ui.transform}))
}

/// The box's size, document px (its bounding box: the box only scales and moves).
fn box_size(t: &FreeTransform) -> [f64; 2] {
    let (x0, y0, x1, y1) = bounds(t);
    [(x1 - x0).round().max(1.0), (y1 - y0).round().max(1.0)]
}

fn source_size(t: &FreeTransform) -> [f64; 2] {
    let rect = t.session.rect();
    [rect.width().round().max(1.0), rect.height().round().max(1.0)]
}

fn bounds(t: &FreeTransform) -> (f64, f64, f64, f64) {
    t.session
        .quad()
        .iter()
        .fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |(x0, y0, x1, y1), q| (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y)))
}

/// `edit.contentAwareScale`'s params for the box: its size and options, and with `place` its
/// position.
pub(crate) fn params(t: &FreeTransform, o: &ContentAwareOptions, place: bool) -> Value {
    let [w, h] = box_size(t);
    let mut p = json!({
        "width": w,
        "height": h,
        "amount": o.amount.clamp(0.0, 100.0),
        "protect": o.protect.as_deref().unwrap_or("none"),
        "protectSkinTones": o.protect_skin_tones,
    });
    if place {
        let (x0, y0, _, _) = bounds(t);
        p["x"] = json!(x0.round());
        p["y"] = json!(y0.round());
    }
    p
}

/// ↩: run the command for the box. An untouched box changes nothing and adds no step.
pub(crate) fn commit(app: &mut PhotocraftApp, t: &FreeTransform, o: &ContentAwareOptions) {
    if t.session.is_untouched() {
        return;
    }
    if let Err(e) = app.run(COMMAND, params(t, o, true)) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Once per frame while the box is open: take a finished preview, cancel one gone stale and start
/// one for the box as it is now.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(t) = app.ui.transform.as_ref() else { return };
    let Some(o) = t.content_aware.as_ref() else { return };
    let unscaled = box_size(t) == source_size(t);
    let want = params(t, o, false);
    let now = crate::gpu_canvas::now_ms();
    let Some(live) = app.transform_preview.as_mut().and_then(|pv| pv.content_aware.as_mut()) else { return };
    if let Some(r) = &live.running {
        let finished = match r.rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("Content-Aware Scale preview stopped".into())),
        };
        if let Some(result) = finished {
            let key = live.running.take().map(|r| r.key).unwrap_or_default();
            // A failed or cancelled preview keeps the last one and is not asked for again while
            // the box stays as it is; the commit reports real errors.
            match result {
                Ok(image) => {
                    live.serial += 1;
                    live.failed = None;
                    let pixels = PreviewPixels::new(image, [1.0, 1.0]);
                    live.shown = Some((key, PreviewTextures::new(&mut app.texture_serials, format!("content-aware-scale-{}", live.serial), pixels)));
                }
                // A preview cancelled because the box moved on is not a failure of that box.
                Err(_) if key == want => live.failed = Some(key),
                Err(_) => {}
            }
        } else {
            if r.key != want && now - r.started_ms >= STALE_MS {
                r.job.cancel();
            }
            ctx.request_repaint();
            return;
        }
    }
    if unscaled || live.shown.as_ref().is_some_and(|(k, _)| *k == want) || live.failed.as_ref() == Some(&want) {
        return;
    }
    let preview = match ContentAwareScalePreview::new(&app.session, &want, PROXY_SIDE) {
        Ok(p) => p,
        Err(e) => {
            app.ui.status = e.to_string();
            if let Some(live) = app.transform_preview.as_mut().and_then(|pv| pv.content_aware.as_mut()) {
                live.failed = Some(want);
            }
            return;
        }
    };
    let Some(live) = app.transform_preview.as_mut().and_then(|pv| pv.content_aware.as_mut()) else { return };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let job = JobCtx::new();
    let work = job.clone();
    let repaint = ctx.clone();
    let compute = move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let surf = preview.run(&work).map_err(|e| e.to_string())?;
            let [w, h] = preview.size();
            Ok(crate::transform_tool::read_surface(&surf, photocraft_geom::Rect::from_xywh(0, 0, w, h), usize::MAX).image)
        }))
        .unwrap_or_else(|_| Err("Content-Aware Scale preview failed".into()));
        let _ = tx.send(result);
        repaint.request_repaint();
    };
    // No threads (the browser): the reduced copy is small enough to carve inline.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let spawned = std::thread::Builder::new().name("content-aware-scale-preview".into()).spawn(compute);
        if spawned.is_err() {
            return;
        }
    }
    #[cfg(target_arch = "wasm32")]
    compute();
    live.running = Some(Running { key: want, rx, job, started_ms: now });
    ctx.request_repaint();
}

/// The bar's group after the reference point, position and size: Amount, Protect and Protect Skin
/// Tones. It reports and applies only what the user changed in this pass (the controls hold the
/// session's values and a second pass of a frame gets no input), writing the options to the open
/// session's `content_aware`.
pub fn fields(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &FreeTransform) {
    let Some(o) = t.content_aware.clone() else { return };
    let mut next = o.clone();
    let caption = FieldLayout::Beside;
    Divider::vertical().show(ui);
    Field::new(tl!("Amount")).layout(caption).sizing(Sizing::Content).show(ui, |ui, _| {
        let out = NumberField::new(tl!("Amount"), Bind::given(Some(f64::from(o.amount)))).range(0.0..=100.0).unit_kind(UnitKind::Percent).show(ui);
        if let Some(Some(amount)) = out.edit.events.landed() {
            next.amount = (*amount as f32).clamp(0.0, 100.0);
        }
    });
    Divider::vertical().show(ui);
    Field::new(tl!("Protect")).layout(caption).sizing(Sizing::Content).show(ui, |ui, _| {
        let channels: Vec<String> = app.session.active().map(|st| st.doc.channels.iter().map(|c| c.name.clone()).collect()).unwrap_or_default();
        let mut entries: Vec<Entry<Option<String>>> = vec![Entry::new(None, tl!("None"))];
        entries.extend(channels.iter().map(|name| Entry::new(Some(name.clone()), name.clone())));
        let pick = Select::new(tl!("Protect"), Bind::given(o.protect.clone()), &entries).sizing(Sizing::Content).show(ui);
        if let Some(protect) = pick.value() {
            next.protect = protect.clone();
        }
    });
    let skin = Checkbox::new(Bind::given(o.protect_skin_tones)).label(tl!("Protect skin tones")).show(ui);
    if let Some(on) = skin.value().copied() {
        next.protect_skin_tones = on;
    }
    if next != o
        && let Some(s) = app.ui.transform.as_mut()
    {
        s.content_aware = Some(next);
    }
}

#[cfg(test)]
mod tests {
    use craft::behavior::kurbo::{Point, Rect};

    use super::*;

    #[test]
    fn partial_colour_sets_cannot_start_content_aware_or_mesh_transform_previews() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = app.ctx().clone();
        app.run("file.new", json!({"width":32,"height":24})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#336699"})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        for cmd in [COMMAND, "edit.puppetWarp", "edit.perspectiveWarp"] {
            let result = match cmd {
                COMMAND => begin(&mut app, &ctx).map(|_| ()),
                "edit.puppetWarp" => crate::puppet_ui::begin(&mut app, &ctx, cmd),
                _ => crate::perspective_ui::begin(&mut app, &ctx, cmd),
            };
            assert!(result.is_err(), "{cmd}");
            assert!(app.ui.transform.is_none());
            assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        }
    }

    #[test]
    fn menu_opens_a_scale_box_and_enter_commits_one_step() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = app.ctx().clone();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 8, "y": 8, "width": 40, "height": 24})).unwrap();
        app.run("edit.fill", json!({"color": "#c03020"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.sync_views();
        let past = app.session.active().unwrap().history.past_len();
        // The menu opens the box, not a dialog.
        let r = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
        assert!(r.get("dialog").is_none() && !app.dialog_open(), "{r}");
        let t = app.ui.transform.clone().unwrap();
        assert_eq!(t.session.rect(), Rect::new(8.0, 8.0, 48.0, 32.0));
        assert!(t.content_aware.is_some());
        // The box only scales and moves: no Warp.
        assert!(!t.session.allows().warp);
        // The right edge narrows the box to 24 px.
        app.ui.transform.as_mut().unwrap().edit_box(|b| b.quad = [Point::new(8.0, 8.0), Point::new(32.0, 8.0), Point::new(32.0, 32.0), Point::new(8.0, 32.0)]);
        assert_eq!(
            app.ui.transform.as_ref().unwrap().session.quad(),
            [Point::new(8.0, 8.0), Point::new(32.0, 8.0), Point::new(32.0, 32.0), Point::new(8.0, 32.0)]
        );
        // The live preview carves a reduced copy in the background.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.transform_preview.as_ref().and_then(|pv| pv.content_aware.as_ref()).is_none_or(|l| l.shown.is_none()) {
            assert!(std::time::Instant::now() < deadline, "no live preview");
            tick(&mut app, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // ↩ (the options bar's commit) runs the command: one step, the new size, in place.
        crate::transform_tool::commit(&mut app);
        assert!(app.ui.transform.is_none());
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), past + 1);
        assert_eq!(st.history.undo_label(), Some("Content-Aware Scale"));
        let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
        assert_eq!([b.x0, b.y0, b.width() as i32, b.height() as i32], [8, 8, 24, 24]);
    }

    /// On the Background the box's preview shows the vacated area in the background colour, as
    /// the command leaves it.
    #[test]
    fn on_the_background_the_preview_vacates_to_the_background_colour() {
        for depth in [8, 16] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let ctx = app.ctx().clone();
            app.run("file.new", json!({"width": 64, "height": 48, "depth": depth, "background": "white"})).unwrap();
            app.session.tools.background = [0.0, 0.0, 1.0, 1.0];
            app.run("select.rect", json!({"x": 8, "y": 8, "width": 40, "height": 24})).unwrap();
            app.sync_views();
            crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
            let pv = app.transform_preview.as_ref().unwrap();
            let id = app.session.active().unwrap().active_layer.unwrap();
            let px = |x, y| pv.doc.layer(id).unwrap().surface().unwrap().rgba(x, y);
            assert_eq!(px(20, 20), [0.0, 0.0, 1.0, 1.0], "{depth}-bit: under the box");
            assert_eq!(px(4, 4), [1.0, 1.0, 1.0, 1.0], "{depth}-bit: outside it");
        }
    }
}
