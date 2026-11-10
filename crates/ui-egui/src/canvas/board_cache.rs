//! Storyboard cache, step 1: what the canvas shows over a layer (a storyboard's artboard, most
//! usefully) kept aside when the layer is hidden, and put back when it is shown again, instead of
//! compositing it once more.
//!
//! When a layer is hidden, what the display texture shows over its reach is copied aside (GPU to
//! GPU, no readback); when it is shown again with nothing that composites there changed since,
//! the copy goes back. On a storyboard file of ~450 passes, showing a board recomposited ~150 MP
//! (about a second); the copy is milliseconds.
//!
//! The copy is used only when it is exact: the document as it was when the copy was taken and
//! the document now must composite the same over that area ([`super::doc_diff`]), on the same
//! display transform, with no preview showing. Otherwise the layer redraws as before.
//! Off with `PHOTOCRAFT_NO_BOARD_CACHE` (to compare).

use std::collections::HashMap;
use std::sync::Arc;

use photocraft_doc::{DocId, Document, LayerContent, LayerId};
use photocraft_geom::Rect;

use crate::PhotocraftApp;
use crate::gpu_canvas::{CanvasSnapshot, GpuCanvas};

/// One kept picture: the board's rectangle as the canvas showed it, and the document it showed.
struct Shot {
    doc: Arc<Document>,
    display_key: u64,
    snap: CanvasSnapshot,
    /// For least-recently-used eviction.
    used: u64,
}

/// The kept pictures of every open document's hidden layers.
#[derive(Default)]
pub(crate) struct BoardCache {
    shots: HashMap<(DocId, LayerId), Shot>,
    clock: u64,
}

fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PHOTOCRAFT_NO_BOARD_CACHE").is_none())
}

/// GPU memory the kept pictures may take together: a quarter of the compositor's layer budget
/// (about 1.5 GB on a 16 GB card, a few hundred MB on a small one).
fn budget(gpu: &GpuCanvas) -> u64 {
    gpu.memory_budget().map_or(512 << 20, |b| b / 4)
}

/// Smallest area (pixels) worth keeping: smaller layers redraw quickly anyway.
const MIN_PX: u64 = 4 << 20;

/// Where showing layer `l` of `doc` composites: an artboard's rectangle, else the layer's reach
/// on the canvas. `None` when that can't be bounded (an adjustment layer reaching everything).
fn reach(doc: &Document, l: &photocraft_doc::Layer) -> Option<Rect> {
    let canvas = doc.bounds();
    let r = match &l.content {
        LayerContent::Group(g) if g.artboard.is_some() => g.artboard.as_ref().map(|a| a.rect),
        _ => photocraft_compose::change_bounds(l, canvas),
    }?;
    let r = r.intersect(&canvas);
    (r.size().area() >= MIN_PX).then_some(r)
}

fn meets(a: &Rect, b: &Rect) -> bool {
    !a.intersect(b).is_empty()
}

/// Called with the area about to be redrawn on document `id`'s display texture (`damage`), when
/// the canvas last showed `shown` and now shows `doc`: keeps the pictures of boards being hidden,
/// puts back those of boards being shown, and returns what is still to redraw (empty when a
/// put-back picture covers it all).
pub(crate) fn apply(app: &mut PhotocraftApp, gpu: &GpuCanvas, id: DocId, doc: &Arc<Document>, display_key: u64, damage: Rect) -> Rect {
    if !enabled() || damage.is_empty() {
        return damage;
    }
    retain(app);
    let Some((shown, shown_key)) = app.canvas_states.get(&id).and_then(|s| s.shown.clone()) else { return damage };
    if shown_key != display_key || Arc::ptr_eq(&shown, doc) {
        return damage;
    }
    let pending = super::sweep::pending(app, id);
    let before: HashMap<LayerId, &photocraft_doc::Layer> = shown.walk().into_iter().map(|(_, _, l)| (l.id, l)).collect();
    let mut restored: Vec<Rect> = Vec::new();
    for (_, _, l) in doc.walk() {
        let Some(was) = before.get(&l.id) else { continue };
        if was.visible == l.visible {
            continue;
        }
        if was.visible {
            // Being hidden: keep what the canvas shows over it, if that is all drawn.
            let Some(rect) = reach(&shown, was).filter(|r| meets(r, &damage)) else { continue };
            if pending.iter().any(|b| meets(b, &rect)) {
                continue;
            }
            keep(app, gpu, id, l.id, &l.name, rect, &shown, display_key);
        } else if let Some(rect) = reach(doc, l).filter(|r| meets(r, &damage))
            && put_back(app, gpu, id, l.id, &l.name, rect, doc, display_key)
        {
            restored.push(rect);
        }
    }
    if restored.iter().any(|r| r.contains_rect(&damage)) { Rect::EMPTY } else { damage }
}

#[allow(clippy::too_many_arguments)]
fn keep(app: &mut PhotocraftApp, gpu: &GpuCanvas, id: DocId, board: LayerId, name: &str, rect: Rect, shown: &Arc<Document>, display_key: u64) {
    let Some(snap) = gpu.snapshot_rect(id.0, rect) else { return };
    let limit = budget(gpu);
    if snap.bytes > limit {
        return;
    }
    let cache = &mut app.board_cache;
    cache.shots.remove(&(id, board));
    // Least recently used first, until the new one fits.
    while cache.shots.values().map(|s| s.snap.bytes).sum::<u64>() + snap.bytes > limit {
        let Some(oldest) = cache.shots.iter().min_by_key(|(_, s)| s.used).map(|(k, _)| *k) else { break };
        cache.shots.remove(&oldest);
    }
    cache.clock += 1;
    log::info!("storyboard cache: kept \"{name}\" ({}x{}, {} MB; {} kept)", rect.width(), rect.height(), snap.bytes >> 20, cache.shots.len() + 1);
    cache.shots.insert((id, board), Shot { doc: shown.clone(), display_key, snap, used: cache.clock });
}

#[allow(clippy::too_many_arguments)]
fn put_back(app: &mut PhotocraftApp, gpu: &GpuCanvas, id: DocId, board: LayerId, name: &str, rect: Rect, doc: &Document, display_key: u64) -> bool {
    let cache = &mut app.board_cache;
    let Some(shot) = cache.shots.get_mut(&(id, board)) else { return false };
    // Exact only if nothing that composites inside the rectangle changed since it was kept.
    let same = shot.display_key == display_key && shot.snap.rect == rect && still_exact(&shot.doc, doc, rect);
    if !same {
        log::info!("storyboard cache: \"{name}\" changed since it was kept; redrawing it");
        cache.shots.remove(&(id, board));
        return false;
    }
    if !gpu.restore_rect(id.0, &shot.snap) {
        cache.shots.remove(&(id, board));
        return false;
    }
    cache.clock += 1;
    shot.used = cache.clock;
    log::info!("storyboard cache: put back \"{name}\" instead of redrawing {:.0} MP", rect.size().area() as f64 / 1_048_576.0);
    true
}

/// Whether a picture of `rect` taken when the canvas showed `kept` is what `now` shows there:
/// nothing that composites inside it changed since.
fn still_exact(kept: &Document, now: &Document, rect: Rect) -> bool {
    super::doc_diff::same_pixels(kept, now) || super::doc_diff::changed_area(kept, now).is_some_and(|r| r.is_empty() || !meets(&r, &rect))
}

/// Forget the pictures of closed documents.
fn retain(app: &mut PhotocraftApp) {
    let live: Vec<DocId> = app.session.documents().iter().map(|d| d.doc.id).collect();
    app.board_cache.shots.retain(|(doc, _), _| live.contains(doc));
}

#[cfg(test)]
mod tests {
    use photocraft_engine::Session;
    use serde_json::json;

    use super::*;

    /// A 6000×3000 document with a filled layer on each half; returns the session and the left
    /// layer's id.
    fn halves() -> (Session, LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 6000, "height": 3000, "background": "transparent"})).unwrap();
        let left = s.active().unwrap().active_layer.unwrap();
        s.edit("fill", |doc, _| {
            doc.layer_mut(left).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 3000, 3000), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let right = s.active().unwrap().active_layer.unwrap();
        s.edit("fill", |doc, _| {
            doc.layer_mut(right).unwrap().surface_mut().unwrap().fill_rect(Rect::new(3000, 0, 6000, 3000), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
        (s, left)
    }

    #[test]
    fn a_kept_picture_is_used_only_while_nothing_under_it_changed() {
        let (mut s, left) = halves();
        let kept = s.active().unwrap().doc.clone();
        let doc = |s: &Session| s.active().unwrap().doc.clone();
        let rect = reach(&kept, kept.layer(left).unwrap()).expect("a 9 MP layer is worth keeping");
        // Hidden and shown again, nothing else: the picture is exact.
        s.execute("layer.hideLayers", json!({"layer": left.0})).unwrap();
        s.execute("layer.showLayers", json!({"layer": left.0})).unwrap();
        assert!(still_exact(&kept, &doc(&s), rect));
        // A change on the other half: still exact here.
        let right = s.active().unwrap().doc.layers[1].id;
        s.edit("paint", |doc, _| {
            doc.layer_mut(right).unwrap().surface_mut().unwrap().fill_rect(Rect::new(4500, 100, 4600, 200), &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        assert!(still_exact(&kept, &doc(&s), rect), "a change elsewhere doesn't matter");
        // A change inside the picture's area: redraw instead.
        s.edit("paint", |doc, _| {
            doc.layer_mut(right).unwrap().surface_mut().unwrap().fill_rect(Rect::new(500, 100, 600, 200), &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        assert!(!still_exact(&kept, &doc(&s), rect), "something under it changed");
    }
}
