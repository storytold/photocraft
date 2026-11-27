//! Puppet Warp and Perspective Warp (on-canvas modes) and the lookup of the open full-window workspace.
//!
//! The two warps are previews over plain data (pins, planes); committing runs the engine command with that data
//! (`edit.puppetWarp`, `edit.perspectiveWarp`, or the smart-object twins), so the result is one history step and
//! exactly replayable. This module holds their runtime state and the hooks the shell calls (menus, keys, canvas
//! controls, options bar). The full-window workspaces (Liquify, Content-Aware Fill, Select and Mask, Filter Gallery,
//! Adaptive Wide Angle, Vanishing Point, RAW Develop) live on the dialog host; [`modal_workspace`] and
//! [`workspace_fields`] read them from there.

use std::sync::Arc;

use craft::ui::dialog::dialog_id;
use egui::{Color32, ColorImage};
use photocraft_doc::{DocId, Document, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The key of RAW Develop's workspace; the filmstrip switches the active document itself, so it is the one workspace
/// not bound to the document it opened on.
const RAW_DEVELOP: &str = "filter.cameraRaw";

/// The keys of the hosted full-window workspaces, RAW Develop included.
pub(crate) const WORKSPACES: [&str; 7] = [
    "select.selectAndMask",
    "filter.liquify",
    "edit.contentAwareFill",
    "filter.filterGallery",
    photocraft_engine::lens_cmds::WIDE,
    "filter.vanishingPoint",
    RAW_DEVELOP,
];

/// Runtime state of the on-canvas warps (not serialisable: textures, solvers).
#[derive(Default)]
pub struct Distort {
    /// Puppet Warp's session.
    pub puppet: Option<crate::puppet_ui::PuppetSession>,
    /// Perspective Warp's session.
    pub perspective: Option<crate::perspective_ui::PerspSession>,
    /// The document the open sessions belong to, noted by [`drop_foreign`] on the first frame they are open.
    owner: Option<photocraft_doc::DocId>,
}

impl Distort {
    /// Whether Puppet Warp or Perspective Warp is open.
    pub fn active(&self) -> bool {
        self.puppet.is_some() || self.perspective.is_some()
    }

    /// Short description for `ui.inspect`.
    pub fn describe(&self) -> Value {
        json!({
            "puppet": self.puppet.as_ref().map(|p| p.describe()),
            "perspective": self.perspective.as_ref().map(|p| p.describe()),
        })
    }
}

/// The active layer's pixels (a pixel layer, or a smart object's rendering) and its id.
pub(crate) fn active_pixels(app: &PhotocraftApp) -> Result<(LayerId, Surface, bool), String> {
    let st = app.session.active().ok_or("no document")?;
    let id = st.active_layer.ok_or("no active layer")?;
    let l = st.doc.layer(id).ok_or("no layer")?;
    match &l.content {
        LayerContent::Raster(s) => Ok((id, s.clone(), false)),
        LayerContent::Smart(sm) => sm.cache.clone().map(|c| (id, c, true)).ok_or_else(|| "the smart object has no pixels".to_string()),
        other => Err(format!("needs a pixel layer or smart object (active layer is {} {} layer)", other.article(), other.kind_name())),
    }
}

/// The colour set full-window filters capture, or Null when none is targeted: the command then
/// routes a targeted mask, filter mask, alpha channel or Quick Mask itself, as without colour sets.
pub(crate) fn colour_set_target(app: &PhotocraftApp) -> Value {
    let target = crate::canvas::paint_target(app);
    if target.get("colors").is_some() { target } else { Value::Null }
}

/// `params` with the captured `target`, if any (see [`colour_set_target`]).
pub(crate) fn with_filter_target(mut params: Value, target: &Value) -> Value {
    if !target.is_null()
        && let Some(o) = params.as_object_mut()
    {
        o.insert("target".into(), target.clone());
    }
    params
}

/// Full-window filters capture their layer target; scalar planes use ordinary filter dialogs.
/// Without a colour set they open on the layer's pixels or smart object, as before colour sets.
pub(crate) fn active_filter_target(app: &PhotocraftApp) -> Result<(LayerId, Surface, Value), String> {
    let target = colour_set_target(app);
    if target.is_null() {
        // Colour rows left selected on a smart object: the engine refuses the command, so refuse
        // before any work is done in the workspace.
        if let Some(st) = app.session.active()
            && matches!(st.channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Color(_))
            && photocraft_engine::mask_view_cmds::current(st).is_none()
            && st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| matches!(l.content, LayerContent::Smart(_)))
        {
            return Err("Rasterize the layer before editing its colour channels".into());
        }
        let (layer, surface, _) = active_pixels(app)?;
        return Ok((layer, surface, Value::Null));
    }
    let st = app.session.active().ok_or("no document")?;
    let layer = st.active_layer.ok_or("no active layer")?;
    let params = serde_json::json!({"target":target});
    if photocraft_engine::color_channels::explicit(&params, photocraft_engine::channel_cmds::color_count(&st.doc)).map_err(|e| e.to_string())?.is_some()
        && !matches!(st.doc.layer(layer).map(|l| &l.content), Some(LayerContent::Raster(_)))
    {
        return Err("Rasterize the layer before editing its colour channels".into());
    }
    let surface = photocraft_engine::channel_cmds::read_target_surface(&st.doc, Some(layer), &params).map_err(|e| e.to_string())?.clone();
    Ok((layer, surface, target))
}

#[cfg(test)]
pub(crate) mod component_tests {
    use super::*;

    pub fn app(mode: &str, depth: u32) -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":64,"height":48,"mode":mode,"depth":depth})).unwrap();
        app.session
            .edit("Native detail", |doc, id| {
                let surface = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
                let n = surface.format().mode.color_channels();
                for y in 0..48 {
                    for x in 0..64 {
                        let mut pixel: Vec<_> = (0..n).map(|k| ((x * 13 + y * 7 + k * 43) % 201 + 20) as f32 / 255.0).collect();
                        pixel.push(0.65);
                        surface.write_pixel(x as i32, y as i32, &pixel);
                    }
                }
                Ok(())
            })
            .unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app
    }

    pub fn without_alpha(mut app: Box<PhotocraftApp>) -> Box<PhotocraftApp> {
        app.session
            .edit("Alpha-less native raster", |doc, id| {
                let bounds = doc.bounds();
                let surface = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
                let old = surface.format();
                let format = photocraft_color::PixelFormat::new(old.mode, old.sample, false);
                let raw = surface.read_region(bounds);
                let raw: Vec<_> = raw.chunks_exact(old.channels()).flat_map(|pixel| pixel.iter().take(format.channels()).copied()).collect();
                let mut replacement = Surface::new(format);
                replacement.write_region(bounds, &raw);
                *surface = replacement;
                Ok(())
            })
            .unwrap();
        app
    }

    pub fn protected(before: &Surface, after: &Surface, bounds: Rect) {
        assert_eq!(before.format(), after.format());
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                for k in (0..before.channels()).filter(|k| ![0, 2].contains(k)) {
                    assert_eq!(before.sample_channel(x, y, k).to_bits(), after.sample_channel(x, y, k).to_bits(), "component {k} at {x},{y}");
                }
            }
        }
    }
}

/// The document with layer `id` hidden (what shows behind a live preview of that layer).
pub(crate) fn without_layer(doc: &Document, id: LayerId) -> Arc<Document> {
    let mut d = doc.clone();
    if let Some(l) = d.layer_mut(id) {
        l.visible = false;
    }
    Arc::new(d)
}

/// A texture image of `surf` over `r`, downsampled so the longer side is at most `max_side`.
pub(crate) fn surface_image(surf: &Surface, r: Rect, max_side: usize) -> ColorImage {
    let (w, h) = (r.width().max(1) as usize, r.height().max(1) as usize);
    let k = w.max(h).div_ceil(max_side).max(1);
    let (tw, th) = (w.div_ceil(k), h.div_ceil(k));
    let mut px = vec![Color32::TRANSPARENT; tw * th];
    let mut row = vec![[0u8; 4]; w];
    for ty in 0..th {
        let y = r.y0 + (ty * k) as i32;
        surf.read_rgba8_into(Rect::new(r.x0, y, r.x1, y + 1), &mut row);
        for tx in 0..tw {
            let p = row[(tx * k).min(w - 1)];
            let a = if surf.format().alpha { p[3] } else { 255 };
            px[ty * tw + tx] = Color32::from_rgba_unmultiplied(p[0], p[1], p[2], a);
        }
    }
    ColorImage::new([tw, th], px)
}

/// The menu id of the open full-window workspace (Liquify, Content-Aware Fill, Select and Mask, Filter Gallery,
/// Adaptive Wide Angle, Vanishing Point, RAW Develop). These are modal: other menu commands wait for OK or Cancel
/// (`menus::menu_gate`). The newest open workspace wins, and a dialog it opened (a colour picker) does not hide it.
pub(crate) fn modal_workspace(app: &PhotocraftApp) -> Option<&'static str> {
    app.framework.dialogs.open_dialogs().iter().rev().find_map(|d| {
        let key = d.key.as_deref()?;
        WORKSPACES.iter().copied().find(|w| *w == key)
    })
}

/// The fields of the open workspace `key` names, as its dialog reports them; `None` when it is not open. The one
/// host lookup behind every workspace's `describe`.
pub(crate) fn workspace_fields(app: &PhotocraftApp, key: &str) -> Option<Value> {
    app.framework.dialogs.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(key)).and_then(|d| d.fields)
}

/// Whether the workspace `key` names is open.
#[cfg(not(target_arch = "wasm32"))]
fn workspace_open(app: &PhotocraftApp, key: &str) -> bool {
    app.framework.dialogs.open_dialogs().iter().any(|d| d.key.as_deref() == Some(key))
}

fn active_doc(app: &PhotocraftApp) -> Option<DocId> {
    app.session.active().map(|st| st.doc.id)
}

fn on_doc(app: &PhotocraftApp, document: DocId) -> bool {
    active_doc(app) == Some(document)
}

/// Drops Puppet / Perspective Warp when the document they were opened on is no longer active, so they cannot steal
/// undo, block layer delete, or draw another document's pixels (#5115).
pub fn cancel_stale(app: &mut PhotocraftApp) {
    let active = active_doc(app);
    if app.distort.puppet.as_ref().is_some_and(|p| Some(p.document) != active) {
        app.distort.puppet = None;
    }
    if app.distort.perspective.as_ref().is_some_and(|p| Some(p.document) != active) {
        app.distort.perspective = None;
    }
}

/// The document shown on the canvas while Puppet or Perspective Warp previews a layer.
pub fn display_doc(app: &PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let id = app.session.documents().get(idx).map(|st| st.doc.id)?;
    if let Some(p) = &app.distort.puppet
        && p.document == id
    {
        return Some((p.preview_doc.clone(), (1 << 42) + p.key));
    }
    if let Some(p) = &app.distort.perspective
        && p.document == id
    {
        return Some((p.preview_doc.clone(), (1 << 42) + p.key));
    }
    None
}

/// Menu / control routing. `Some` when this module handles the invocation. A workspace id opens its workspace
/// through the workspace's module and answers with that module's `describe`; a `ui` call goes to the workspace's
/// control surface on the dialog host.
pub fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let empty = params.as_object().is_none_or(|o| o.is_empty());
    let ui = params.get("ui");
    #[cfg(not(target_arch = "wasm32"))]
    let hosted = |app: &mut PhotocraftApp, key: &str, ui: &Value| crate::control::dialogs::workspace_ui(app, ctx, key, ui);
    match id {
        "filter.filterGallery" if empty => Some(crate::gallery_ui::open(app).map(|_| json!({"gallery": crate::gallery_ui::describe(app)}))),
        #[cfg(not(target_arch = "wasm32"))]
        "filter.filterGallery" if ui.is_some() && workspace_open(app, id) => Some(hosted(app, id, ui.unwrap_or(&Value::Null))),
        "filter.liquify" if empty => Some(crate::liquify_ui::open(app, ctx).map(|_| json!({"liquify": crate::liquify_ui::describe(app)}))),
        #[cfg(not(target_arch = "wasm32"))]
        "filter.liquify" if ui.is_some() && workspace_open(app, id) => Some(hosted(app, id, ui.unwrap_or(&Value::Null))),
        #[cfg(not(target_arch = "wasm32"))]
        "edit.contentAwareFill" if ui.is_some() && workspace_open(app, id) => Some(hosted(app, id, ui.unwrap_or(&Value::Null))),
        // `ui` addresses the workspace: closed, that is an error, not a fill with the defaults.
        "edit.contentAwareFill" if ui.is_some() => Some(Err("Content-Aware Fill is not open".into())),
        #[cfg(not(target_arch = "wasm32"))]
        "select.selectAndMask" if ui.is_some() && workspace_open(app, id) => Some(hosted(app, id, ui.unwrap_or(&Value::Null))),
        "select.selectAndMask" if ui.is_some() => Some(Err("Select and Mask is not open".into())),
        "select.selectAndMask" => Some(crate::select_mask_ui::open(app).map(|_| json!({"selectAndMask": crate::select_mask_ui::describe(app)}))),
        "edit.puppetWarp" | "layer.smartObjects.puppetWarp" if empty => {
            Some(crate::puppet_ui::begin(app, ctx, id).map(|_| json!({"puppet": app.distort.describe()["puppet"]})))
        }
        "edit.puppetWarp" | "layer.smartObjects.puppetWarp" if ui.is_some() && app.distort.puppet.is_some() => {
            Some(crate::puppet_ui::control(app, ui.unwrap_or(&Value::Null)))
        }
        "edit.perspectiveWarp" | "layer.smartObjects.perspectiveWarp" if empty => {
            Some(crate::perspective_ui::begin(app, ctx, id).map(|_| json!({"perspective": app.distort.describe()["perspective"]})))
        }
        "edit.perspectiveWarp" | "layer.smartObjects.perspectiveWarp" if ui.is_some() && app.distort.perspective.is_some() => {
            Some(crate::perspective_ui::control(app, ui.unwrap_or(&Value::Null)))
        }
        "filter.vanishingPoint" if empty => Some(crate::vp_ui::begin(app, ctx).map(|_| json!({"vanishingPoint": crate::vp_ui::describe(app)}))),
        #[cfg(not(target_arch = "wasm32"))]
        "filter.vanishingPoint" if ui.is_some() && workspace_open(app, id) => Some(hosted(app, id, ui.unwrap_or(&Value::Null))),
        // Workspace controls must not fall through to engine edits when no workspace is open.
        "filter.filterGallery" | "filter.liquify" | "filter.vanishingPoint" if ui.is_some() => Some(Err(format!("{id} is not open"))),
        _ => None,
    }
}

/// Keyboard while a warp is open. Puppet Warp takes ⌘Z and ⇧⌘Z for its own pin history; Enter, Esc and Delete are
/// the warp controls' (they report apply, discard and delete). True when the key handling of the rest of the app
/// should be skipped; a second pass in a frame finds the keys already consumed.
pub fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    cancel_stale(app);
    if app.distort.puppet.is_some() {
        let cmd_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        if ctx.input_mut(|i| i.consume_key(cmd_shift, egui::Key::Z)) {
            crate::puppet_ui::redo(app);
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
            crate::puppet_ui::undo(app);
        }
        return true;
    }
    app.distort.perspective.is_some()
}

/// Draws and runs Puppet Warp or Perspective Warp as canvas controls over the document at `at`, when the session
/// belongs to the active document.
pub(crate) fn show_in_canvas(app: &mut PhotocraftApp, ui: &mut egui::Ui, at: egui::Rect) {
    cancel_stale(app);
    if app.distort.puppet.as_ref().is_some_and(|p| on_doc(app, p.document)) {
        crate::puppet_ui::canvas(app, ui, at);
    }
    if app.distort.perspective.as_ref().is_some_and(|p| on_doc(app, p.document)) {
        crate::perspective_ui::canvas(app, ui, at);
    }
}

/// Options bar for the on-canvas modes. True when drawn.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    cancel_stale(app);
    if app.distort.puppet.is_some() {
        crate::puppet_ui::options_bar(app, ui);
        return true;
    }
    if app.distort.perspective.is_some() {
        crate::perspective_ui::options_bar(app, ui);
        return true;
    }
    false
}

/// Drops the open sessions and workspaces once the active document is not the one they were started on (closed, or
/// another one opened or switched to). Without this, Puppet Warp's preview of the old document replaced the new
/// document's canvas (#5092). RAW Develop stays: its filmstrip switches the active document itself. Each pass applies
/// only the state it reads, so a second pass in a frame finds the owner already cleared and does nothing more.
pub fn drop_foreign(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let current = active_doc(app);
    let bound = app.distort.active() || modal_workspace(app).is_some_and(|key| key != RAW_DEVELOP);
    if !bound {
        app.distort.owner = None;
    } else if app.distort.owner.is_none() {
        app.distort.owner = current;
    } else if app.distort.owner != current {
        app.distort.puppet = None;
        app.distort.perspective = None;
        for key in WORKSPACES.into_iter().filter(|key| *key != RAW_DEVELOP) {
            app.framework.dialogs.cancel_by_id(ctx, dialog_id(key));
        }
        app.distort.owner = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_geom::Rect;
    use serde_json::json;

    use craft::behavior::kurbo::Point;
    use craft::imaging::ui::warp::pin::{Pin, PinEdit};

    /// A closed workspace request must not open a workspace or escape to an engine edit, on any target.
    #[test]
    fn closed_workspace_ui_requests_are_consumed_without_editing() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = app.ctx().clone();
        app.session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        app.session.execute("select.rect", json!({"x": 1, "y": 1, "width": 1, "height": 1})).unwrap();
        let before = app.session.active().unwrap().history.past_len();
        let document = app.session.active().unwrap().doc.clone();
        for id in ["edit.contentAwareFill", "select.selectAndMask", "filter.filterGallery", "filter.liquify", "filter.vanishingPoint"] {
            for ui in [Value::Null, json!({}), json!({"cancel": true}), json!({"sampling": "rectangular"})] {
                let reply = menu(&mut app, &ctx, id, &json!({"ui": ui}));
                assert!(matches!(reply, Some(Err(ref error)) if error.contains("not open")), "{id}: {reply:?}");
                assert_eq!(modal_workspace(&app), None, "{id} must not open a workspace");
                assert!(Arc::ptr_eq(&document, &app.session.active().unwrap().doc), "{id} must not edit the document");
            }
        }
        assert_eq!(app.session.active().unwrap().history.past_len(), before);
        assert!(app.caf.is_none());
    }

    fn paint_bar(app: &mut PhotocraftApp) {
        app.session
            .edit("paint", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
    }

    fn pin(x: f64, y: f64) -> Pin {
        Pin::new(Point::new(x, y), Point::new(x, y))
    }

    /// #5115: Puppet Warp is per-document. Switching tabs drops the session so Ctrl+Z and
    /// layer delete apply to the new document, and the warp mesh does not leak onto it.
    #[test]
    fn switching_documents_drops_puppet_and_restores_undo_and_delete() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = app.ctx().clone();
        app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        paint_bar(&mut app);
        app.sync_views();
        menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
        let _ = crate::puppet_ui::session_apply(&mut app, PinEdit::replace(&[], vec![pin(50.0, 40.0)], "Add Pin"));
        drop_foreign(&mut app, &ctx);
        assert!(app.distort.puppet.is_some());
        let first = app.session.active().unwrap().doc.id;
        app.session.execute("file.new", json!({"width": 60, "height": 40, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.sync_views();
        drop_foreign(&mut app, &ctx);
        assert!(app.distort.puppet.is_none(), "puppet does not follow the new document");
        assert!(display_doc(&app, app.session.active_index().unwrap()).is_none());
        let layers_before = app.session.active().unwrap().doc.layers.len();
        app.session.execute("layer.delete", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers_before - 1);
        assert!(app.session.set_active(0));
        app.sync_views();
        assert_eq!(app.session.active().unwrap().doc.id, first);
        let past = app.session.active().unwrap().history.past_len();
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), past - 1);
    }

    #[test]
    fn committed_puppet_is_one_undo_step_and_the_layer_stays_deletable() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = app.ctx().clone();
        app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        paint_bar(&mut app);
        app.sync_views();
        menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
        let held = vec![pin(25.0, 40.0)];
        let _ = crate::puppet_ui::session_apply(&mut app, PinEdit::replace(&[], held.clone(), "Add Pin"));
        let mut both = held.clone();
        both.push(pin(75.0, 40.0));
        let _ = crate::puppet_ui::session_apply(&mut app, PinEdit::replace(&held, both.clone(), "Add Pin"));
        let mut dragged = both.clone();
        dragged[1].at = Point::new(75.0, 20.0);
        let _ = crate::puppet_ui::session_apply(&mut app, PinEdit::replace(&both, dragged, "Move Pin"));
        let before = app.session.active().unwrap().history.past_len();
        crate::puppet_ui::commit(&mut app);
        assert!(app.distort.puppet.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), before);
        let layers = app.session.active().unwrap().doc.layers.len();
        app.session.execute("layer.delete", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers - 1);
    }

    #[test]
    fn switching_documents_drops_liquify() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = app.ctx().clone();
        app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        paint_bar(&mut app);
        crate::liquify_ui::open(&mut app, &ctx).unwrap();
        drop_foreign(&mut app, &ctx);
        assert_eq!(modal_workspace(&app), Some("filter.liquify"));
        app.session.execute("file.new", json!({"width": 40, "height": 40, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.sync_views();
        drop_foreign(&mut app, &ctx);
        crate::dialogs::take_results(&mut app, &ctx);
        assert_eq!(modal_workspace(&app), None);
        let past = app.session.active().unwrap().history.past_len();
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), past - 1);
    }
}
