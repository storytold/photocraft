//! Object Selection › Object Finder (#4087): with the Object Selection tool active, resting the
//! pointer on the canvas highlights the object a click there would select, as Photoshop's Object
//! Finder does. A click (without dragging) commits that object (`select.object` with `"point"`;
//! ⇧ adds, ⌥ subtracts, ⇧⌥ intersects); dragging keeps the rectangle mode.
//!
//! The lookup is `photocraft_engine::smartselect_cmds::find_object`, run after the pointer has
//! been still for [`DEBOUNCE`] seconds on a worker thread (inline on the web), so the UI never
//! waits for it. The job also reduces the object to the framework's `HoverMask` and asks for a
//! repaint when it sends. Results go into the session's `object_finder` cache, which the click
//! reuses and which serves every point of an object already found; it is keyed by the document's
//! pixels, so an edit drops it. The preview itself is keyed by document revision, sampled layer and
//! Sample All Layers, and clears when the pointer leaves the canvas, the tool changes, a button
//! goes down or the option is off. Quality of the object found is out of scope here (#2788).
//!
//! The options bar has a Refresh button and a gear menu (#12857): Object Finder on/off, Object
//! Subtract (stored only) and Object Finder Mode. Auto Refresh re-analyses after every edit, as
//! above. Manual Refresh keeps the analysed pixels (`Finder::basis`) across the pointer leaving,
//! a tool switch and selection-only changes; it analyses nothing until Refresh is clicked
//! ([`refresh`]): not on the first hover, on another document, or after a pixel edit.

use photocraft_engine::smartselect_cmds::SmartSelectExt;
use std::sync::Arc;

use craft::behavior::kurbo::{Point, Rect};
use craft::ui::canvas_controls::{HoverMask, HoverOutline, HoverShape};
use craft::ui::contract::Show as _;
use photocraft_algo::selection::Region;
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_engine::smartselect_cmds::{empty_sampled_area, find_object, sampled_target};

use crate::PhotocraftApp;
use crate::canvas::tools::in_gesture;
use crate::state::{ObjectFinderMode, Tool};

/// Seconds the pointer must rest before an object is looked up.
pub const DEBOUNCE: f64 = 0.12;

/// What a lookup job sends: the object. Its hover mask is made on the UI thread, whose
/// `texture_serials` number it.
type Found = Option<Arc<Region>>;

/// A lookup running off the UI thread.
struct Job {
    at: [i32; 2],
    doc: Arc<Document>,
    rx: std::sync::mpsc::Receiver<Found>,
}

/// The hover state of the active document (`PhotocraftApp::object_finder`).
pub(crate) struct Finder {
    doc: DocId,
    revision: u64,
    target: Option<LayerId>,
    all_layers: bool,
    /// The pointer pixel and when it got there.
    at: [i32; 2],
    since: f64,
    job: Option<Job>,
    /// Points already looked up that found nothing (the background).
    misses: Vec<[i32; 2]>,
    shown: Option<Arc<Region>>,
    /// The hover mask of `shown`, with the object it was made from.
    mask: Option<(Arc<Region>, HoverMask)>,
    /// The document whose pixels were last analysed (Manual Refresh compares against it).
    basis: Arc<Document>,
}

/// What [`update`] decided.
#[derive(Clone, Debug, Default)]
pub(crate) struct Update {
    /// The object to highlight.
    pub shown: Option<Arc<Region>>,
    /// A lookup is pending or running: a repaint is already asked for.
    #[cfg(test)]
    pub busy: bool,
    /// Why nothing can be found here: [`empty_layer_hint`] when the sampled layer has no pixels under the pointer.
    pub hint: Option<&'static str>,
}

/// The status-bar hint while hovering where the sampled layer has no pixels (the click reports the same as an error).
fn empty_layer_hint() -> &'static str {
    tl!("The active layer has no pixels here — turn on Sample All Layers or select a layer with pixels")
}

/// Whether the preview applies now (`pressed`: a pointer button is down).
fn active(app: &PhotocraftApp, pressed: bool) -> bool {
    app.ui.tool == Tool::ObjectSelection
        && app.ui.tool_options.object_finder
        && !pressed
        && !in_gesture(app)
        && app.ui.transform.is_none()
        && app.session.active().is_some()
}

/// The hover mask of an object: the one place a region becomes the tint the outline shows.
/// `serials` (the app's `texture_serials`) keeps its texture apart from earlier masks'.
fn mask_of(serials: &mut craft::ui::primitives::Serials, r: &Region) -> HoverMask {
    let b = r.bbox;
    HoverMask::sampled(serials, Rect::new(f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)), |x, y| r.at(x.floor() as i32, y.floor() as i32))
}

/// Advances the hover preview with the pointer at `at` (document px; `None` = off the canvas)
/// at time `now` (seconds). The state it keeps is the finder's own: a second pass in the same
/// frame finds the finished job already taken and the same object shown, so it changes nothing.
/// While the rest has not yet lasted [`DEBOUNCE`] it asks `ctx` for one repaint at its end.
pub(crate) fn update(app: &mut PhotocraftApp, at: Option<[f64; 2]>, pressed: bool, now: f64, ctx: &egui::Context) -> Update {
    let at = at.filter(|p| p[0].is_finite() && p[1].is_finite());
    let (Some(at), true) = (at, active(app, pressed)) else {
        idle(app);
        return Update::default();
    };
    let Some(st) = app.session.active() else { return Update::default() };
    let all_layers = app.ui.tool_options.sample_all.object_selection;
    let target = sampled_target(&st.doc, st.active_layer, all_layers);
    let (doc, revision, layer) = (st.doc.id, st.revision, st.active_layer);
    let px = [at[0].floor() as i32, at[1].floor() as i32];
    let fresh = app.object_finder.as_ref().is_some_and(|f| f.doc == doc && f.revision == revision && f.target == target && f.all_layers == all_layers);
    let doc_arc = Arc::clone(&st.doc);
    if !fresh {
        let manual = app.ui.tool_options.object_finder_mode == ObjectFinderMode::Manual;
        match app.object_finder.as_mut() {
            // Manual Refresh: a selection-only change keeps the analysis, a pixel edit makes it stale.
            Some(f) if manual && f.doc == doc => {
                if f.target == target && f.all_layers == all_layers && photocraft_engine::layer_multi_cmds::same_pixels(&f.basis, &doc_arc) {
                    f.revision = revision;
                } else {
                    return Update::default();
                }
            }
            // Manual Refresh analyses nothing until Refresh is clicked: no finder yet, or one kept
            // for another document, waits for [`refresh`] (#12857).
            _ if manual => return Update::default(),
            _ => app.object_finder = start(app),
        }
    }
    // The click reports an empty sampled layer; the hover says so too instead of showing nothing.
    let here = photocraft_geom::Rect::from_xywh(px[0], px[1], 1, 1);
    if doc_arc.bounds().contains(px[0], px[1]) && empty_sampled_area(&doc_arc, layer, all_layers, here).is_some() {
        return Update {
            shown: None,
            #[cfg(test)]
            busy: false,
            hint: Some(empty_layer_hint()),
        };
    }
    let Some(f) = app.object_finder.as_mut() else { return Update::default() };
    if f.at != px {
        f.at = px;
        f.since = now;
    }
    // A finished lookup: remember it (only for the pixels it was computed from).
    if let Some(job) = &f.job {
        match job.rx.try_recv() {
            Ok(found) => {
                if let Some(r) = found {
                    if photocraft_engine::layer_multi_cmds::same_pixels(&job.doc, &doc_arc) {
                        app.session.object_finder_mut().insert(&doc_arc, target, Arc::clone(&r));
                        let mask = mask_of(&mut app.texture_serials, &r);
                        f.mask = Some((r, mask));
                    }
                } else {
                    f.misses.push(job.at);
                    if f.misses.len() > 64 {
                        f.misses.remove(0);
                    }
                }
                f.job = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => f.job = None,
        }
    }
    let inside = |r: &Region| r.at(px[0], px[1]) >= 0.5;
    if !f.shown.as_deref().is_some_and(inside) {
        f.shown = app.session.object_finder().lookup(&doc_arc, target, at[0] as f32, at[1] as f32);
    }
    match &f.shown {
        Some(r) if !f.mask.as_ref().is_some_and(|(m, _)| Arc::ptr_eq(m, r)) => f.mask = Some((Arc::clone(r), mask_of(&mut app.texture_serials, r))),
        None => f.mask = None,
        _ => {}
    }
    let missed = f.misses.iter().any(|m| (m[0] - px[0]).abs() <= 2 && (m[1] - px[1]).abs() <= 2);
    if f.shown.is_none() && f.job.is_none() && !missed && now - f.since >= DEBOUNCE && doc_arc.bounds().contains(px[0], px[1]) {
        f.job = Some(spawn(Arc::clone(&doc_arc), layer, all_layers, px, at, ctx.clone()));
    }
    let resting = f.shown.is_none() && !missed && now - f.since < DEBOUNCE;
    if resting {
        craft::ui::runtime::frame::request_repaint_after(ctx, DEBOUNCE - (now - f.since));
    }
    #[cfg(test)]
    let busy = f.job.is_some() || resting;
    Update {
        shown: f.shown.clone(),
        #[cfg(test)]
        busy,
        hint: None,
    }
}

/// Starts the lookup at `at`; the job asks `ctx` for a repaint when it has sent its result.
fn spawn(doc: Arc<Document>, layer: Option<LayerId>, all_layers: bool, px: [i32; 2], at: [f64; 2], ctx: egui::Context) -> Job {
    let (tx, rx) = std::sync::mpsc::channel();
    let work = {
        let doc = Arc::clone(&doc);
        move || {
            let found = find_object(&doc, layer, all_layers, at[0] as f32, at[1] as f32).map(Arc::new);
            let _ = tx.send(found);
            ctx.request_repaint();
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let spawned = std::thread::Builder::new().name("object-finder".into()).spawn(work);
        if spawned.is_err() {
            // No thread: the receiver reports Disconnected and the next rest retries.
        }
    }
    #[cfg(target_arch = "wasm32")]
    work();
    Job { at: px, doc, rx }
}

/// Drops the preview. Auto Refresh forgets the finder; Manual Refresh keeps the analysed
/// pixels so the pointer leaving or a tool switch doesn't lose them.
fn idle(app: &mut PhotocraftApp) {
    if app.ui.tool_options.object_finder_mode == ObjectFinderMode::Manual
        && let Some(f) = app.object_finder.as_mut()
    {
        f.shown = None;
        f.job = None;
        f.mask = None;
        f.misses.clear();
    } else {
        app.object_finder = None;
    }
}

/// Clears the preview (a tool switch or an edit made elsewhere is caught by [`update`] too).
pub(crate) fn clear(app: &mut PhotocraftApp) {
    idle(app);
}

/// The options bar's Refresh button: the next [`update`] analyses the current pixels. Under
/// Manual Refresh this is the only way a finder starts.
pub(crate) fn refresh(app: &mut PhotocraftApp) {
    app.object_finder = start(app);
}

/// A new finder for the active document's current pixels, sampled layer and Sample All Layers.
/// Its pointer position is unset, so the first hover starts the debounce.
fn start(app: &PhotocraftApp) -> Option<Finder> {
    let st = app.session.active()?;
    let all_layers = app.ui.tool_options.sample_all.object_selection;
    Some(Finder {
        doc: st.doc.id,
        revision: st.revision,
        target: sampled_target(&st.doc, st.active_layer, all_layers),
        all_layers,
        at: [i32::MIN, i32::MIN],
        since: 0.0,
        job: None,
        misses: Vec::new(),
        shown: None,
        mask: None,
        basis: Arc::clone(&st.doc),
    })
}

/// Updates the preview for this frame and shows the highlight. `at` is the pointer in document
/// units on the active document's canvas, `None` anywhere else.
pub(crate) fn show(ui: &mut egui::Ui, app: &mut PhotocraftApp, at: Option<Point>) {
    let ctx = ui.ctx().clone();
    let (pressed, now) = ctx.input(|i| (i.pointer.any_down(), i.time));
    let up = update(app, at.map(|p| [p.x, p.y]), pressed, now, &ctx);
    match up.hint {
        Some(h) if app.ui.status != h => {
            app.ui.status = h.to_owned();
            app.ui.status_error = false;
        }
        // The pointer moved on: drop our hint, never another message.
        None if app.ui.status == empty_layer_hint() => app.ui.status.clear(),
        _ => {}
    }
    if up.shown.is_none() {
        return;
    }
    let Some((_, mask)) = app.object_finder.as_ref().and_then(|f| f.mask.as_ref()) else { return };
    HoverOutline::new(tl!("Object Finder"), Some(HoverShape::Mask(mask.clone()))).show(ui);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A 200 × 160 document: a red disc (centre 100, 80, radius 45) on blue.
    pub(crate) fn disc_app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 160})).unwrap();
        app.session
            .edit("paint", |doc, a| {
                let s = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
                s.fill_rect(photocraft_geom::Rect::new(0, 0, 200, 160), &[0.2, 0.4, 0.8, 1.0]);
                for y in 0..160 {
                    for x in 0..200 {
                        let (dx, dy) = (x as f32 + 0.5 - 100.0, y as f32 + 0.5 - 80.0);
                        if dx * dx + dy * dy <= 45.0 * 45.0 {
                            s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &[0.9, 0.2, 0.1, 1.0]);
                        }
                    }
                }
                Ok(())
            })
            .unwrap();
        app.ui.tool = Tool::ObjectSelection;
        app
    }

    /// Hovers at `at` until the lookup settles (time advances past the debounce).
    pub(crate) fn hover(app: &mut PhotocraftApp, at: [f64; 2]) -> Update {
        let ctx = app.ctx().clone();
        let mut t = 100.0;
        loop {
            let u = update(app, Some(at), false, t, &ctx);
            if !u.busy || t > 130.0 {
                return u;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            t += 0.05;
        }
    }

    #[test]
    fn hovering_an_object_highlights_it_after_a_rest() {
        let mut app = disc_app();
        let ctx = app.ctx().clone();
        // Not before the pointer has rested.
        let u = update(&mut app, Some([100.0, 80.0]), false, 50.0, &ctx);
        assert!(u.shown.is_none() && u.busy);
        let u = hover(&mut app, [100.0, 80.0]);
        let r = u.shown.expect("the disc is highlighted");
        assert!(r.at(100, 80) >= 0.5 && r.at(60, 80) >= 0.5 && r.at(5, 5) < 0.5);
        // Elsewhere on the same object: served from the cache, no new lookup.
        let u = update(&mut app, Some([120.0, 90.0]), false, 200.0, &ctx);
        assert!(u.shown.is_some_and(|s| Arc::ptr_eq(&s, &r)) && !u.busy);
        assert_eq!(app.session.object_finder().len(), 1);
        // The background highlights nothing.
        assert!(hover(&mut app, [4.0, 4.0]).shown.is_none());
    }

    #[test]
    fn hovering_an_empty_active_layer_gives_a_hint_not_silence() {
        let mut app = disc_app();
        let ctx = app.ctx().clone();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        let u = update(&mut app, Some([100.0, 80.0]), false, 100.0, &ctx);
        assert!(u.shown.is_none() && !u.busy);
        assert_eq!(u.hint, Some(empty_layer_hint()));
        // Sample All Layers on: no hint, the object is looked up.
        app.ui.tool_options.sample_all.object_selection = true;
        assert!(update(&mut app, Some([100.0, 80.0]), false, 100.0, &ctx).hint.is_none());
        assert!(hover(&mut app, [100.0, 80.0]).shown.is_some());
    }

    #[test]
    fn preview_clears_on_leave_tool_switch_button_option_and_edits() {
        let mut app = disc_app();
        let ctx = app.ctx().clone();
        assert!(hover(&mut app, [100.0, 80.0]).shown.is_some());
        assert!(update(&mut app, None, false, 300.0, &ctx).shown.is_none());
        assert!(app.object_finder.is_none());
        assert!(hover(&mut app, [100.0, 80.0]).shown.is_some());
        assert!(update(&mut app, Some([100.0, 80.0]), true, 300.0, &ctx).shown.is_none());
        app.ui.tool = Tool::Lasso;
        assert!(update(&mut app, Some([100.0, 80.0]), false, 300.0, &ctx).shown.is_none());
        app.ui.tool = Tool::ObjectSelection;
        app.ui.tool_options.object_finder = false;
        assert!(update(&mut app, Some([100.0, 80.0]), false, 300.0, &ctx).shown.is_none());
        app.ui.tool_options.object_finder = true;
        assert!(hover(&mut app, [100.0, 80.0]).shown.is_some());
        // A pixel edit: the cached object is dropped and the preview starts over.
        app.session.execute("paint.pencil", json!({"points": [[5, 5], [30, 5]], "size": 4, "color": "#000000"})).unwrap();
        let u = update(&mut app, Some([100.0, 80.0]), false, 400.0, &ctx);
        assert!(u.shown.is_none() && u.busy);
    }
}
