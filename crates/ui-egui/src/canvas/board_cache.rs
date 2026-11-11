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
    kept: Kept,
    display_key: u64,
    snap: CanvasSnapshot,
    /// For least-recently-used eviction.
    used: u64,
}

/// What a kept picture shows, apart from its GPU copy (so tests need no GPU).
struct Kept {
    /// What the canvas showed, or since [`Kept::follow`] the current document with the board
    /// shown, which composites the same in `rect` and keeps no older tiles alive.
    doc: Arc<Document>,
    /// The picture's rectangle on the canvas.
    rect: Rect,
    /// When [`Kept::follow`] last moved `doc` up to the current document (`now_ms`).
    followed: f64,
    /// [`Kept::follow`] was put off within its interval: [`tick`] follows it once the interval is
    /// over, with or without another edit.
    due: bool,
}

impl Kept {
    /// Move up to `doc`: if the picture is still exact for `doc` with layer `board` shown, hold
    /// that document (the board shown) instead, which shares its tiles with what the session
    /// holds, so no tiles History has let go stay alive outside Memory Usage. Within
    /// [`FOLLOW_MS`] of the last time, only marks the picture `due`, so painting doesn't compare
    /// documents every frame. False when the picture isn't exact any more: showing its board would
    /// redraw it anyway, so it should go.
    fn follow(&mut self, board: LayerId, doc: &Document, now_ms: f64) -> bool {
        if now_ms - self.followed < FOLLOW_MS {
            self.due = true;
            return true;
        }
        let mut now = doc.clone();
        let Some(layer) = now.layer_mut(board) else { return false };
        layer.visible = true;
        if !still_exact(&self.doc, &now, self.rect) {
            return false;
        }
        self.doc = Arc::new(now);
        self.followed = now_ms;
        self.due = false;
        true
    }
}

/// The kept pictures of every open document's hidden layers.
#[derive(Default)]
pub(crate) struct BoardCache {
    shots: HashMap<(DocId, LayerId), Shot>,
    clock: u64,
}

/// Off with `PHOTOCRAFT_NO_BOARD_CACHE`, read once.
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

/// Whether the two rectangles share a pixel.
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
    let toggled: Vec<LayerId> =
        doc.walk().into_iter().filter(|(_, _, l)| before.get(&l.id).is_some_and(|was| was.visible != l.visible)).map(|(_, _, l)| l.id).collect();
    follow(app, id, doc, &toggled);
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
            keep(app, gpu, id, l.id, rect, &shown, display_key);
        } else if let Some(rect) = reach(doc, l).filter(|r| meets(r, &damage))
            && put_back(app, gpu, id, l.id, rect, doc, display_key)
        {
            restored.push(rect);
        }
    }
    if restored.iter().any(|r| r.contains_rect(&damage)) { Rect::EMPTY } else { damage }
}

/// Keep what the display texture of document `id` shows over `rect` as layer `board`'s picture,
/// taken when the canvas showed `shown`, evicting the least recently used pictures to fit.
fn keep(app: &mut PhotocraftApp, gpu: &GpuCanvas, id: DocId, board: LayerId, rect: Rect, shown: &Arc<Document>, display_key: u64) {
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
    // The layer's id, not its name: names are the user's text and stay out of the log.
    log::info!("storyboard cache: kept layer {} ({}x{}, {} MB; {} kept)", board.0, rect.width(), rect.height(), snap.bytes >> 20, cache.shots.len() + 1);
    let kept = Kept { doc: shown.clone(), rect: snap.rect, followed: crate::gpu_canvas::now_ms(), due: false };
    cache.shots.insert((id, board), Shot { kept, display_key, snap, used: cache.clock });
}

/// Put layer `board`'s kept picture back over `rect` of document `id`'s display texture, if it is
/// still exact for `doc`; drops a picture that isn't. Whether it went back.
fn put_back(app: &mut PhotocraftApp, gpu: &GpuCanvas, id: DocId, board: LayerId, rect: Rect, doc: &Document, display_key: u64) -> bool {
    let cache = &mut app.board_cache;
    let Some(shot) = cache.shots.get_mut(&(id, board)) else { return false };
    // Exact only if nothing that composites inside the rectangle changed since it was kept.
    let same = shot.display_key == display_key && shot.snap.rect == rect && still_exact(&shot.kept.doc, doc, rect);
    if !same {
        log::info!("storyboard cache: layer {} changed since it was kept; redrawing it", board.0);
        cache.shots.remove(&(id, board));
        return false;
    }
    if !gpu.restore_rect(id.0, &shot.snap) {
        cache.shots.remove(&(id, board));
        return false;
    }
    cache.clock += 1;
    shot.used = cache.clock;
    log::info!("storyboard cache: put back layer {} instead of redrawing {:.0} MP", board.0, rect.size().area() as f64 / 1_048_576.0);
    true
}

/// Whether a picture of `rect` taken when the canvas showed `kept` is what `now` shows there:
/// nothing that composites inside it changed since.
/// A change just outside the rectangle counts when a layer effect (a shadow, a stroke) can carry
/// it inside.
fn still_exact(kept: &Document, now: &Document, rect: Rect) -> bool {
    if super::doc_diff::same_pixels(kept, now) {
        return true;
    }
    let reach = super::textures::effect_reach(&kept.layers).max(super::textures::effect_reach(&now.layers));
    super::doc_diff::changed_area(kept, now).is_some_and(|r| r.is_empty() || !meets(&r.inflate(reach), &rect))
}

/// How often (ms) [`Kept::follow`] moves a kept picture's document up to the current one.
const FOLLOW_MS: f64 = 1000.0;

/// Move document `id`'s kept pictures up to `doc` ([`Kept::follow`]), except those of the
/// `toggled` layers (being kept or put back now), and drop those that aren't exact any more.
fn follow(app: &mut PhotocraftApp, id: DocId, doc: &Document, toggled: &[LayerId]) {
    let now_ms = crate::gpu_canvas::now_ms();
    let mut stale = Vec::new();
    for (&(d, board), shot) in &mut app.board_cache.shots {
        if d == id && !toggled.contains(&board) && !shot.kept.follow(board, doc, now_ms) {
            stale.push((d, board));
        }
    }
    for k in stale {
        app.board_cache.shots.remove(&k);
    }
}

/// Follow the pictures [`Kept::follow`] put off, once their interval is over, even if no edit
/// or redraw comes after: otherwise the last edits of a burst left a picture holding tiles that
/// History then let go. Called every frame; asks for a frame until none is due ([`schedule`]).
pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let mut due: Vec<DocId> = app.board_cache.shots.iter().filter(|(_, s)| s.kept.due).map(|(&(d, _), _)| d).collect();
    if due.is_empty() {
        return;
    }
    due.sort_by_key(|d| d.0);
    due.dedup();
    for id in due {
        let Some(doc) = app.session.documents().iter().find(|st| st.doc.id == id).map(|st| st.doc.clone()) else { continue };
        follow(app, id, &doc, &[]);
    }
    schedule(app, ctx);
}

/// Ask for a frame in time for [`tick`] to follow the pictures that are due. Called after
/// [`apply`] too, which can make one due after this frame's `tick` found none.
pub(crate) fn schedule(app: &PhotocraftApp, ctx: &egui::Context) {
    if app.board_cache.shots.values().any(|s| s.kept.due) {
        ctx.request_repaint_after(std::time::Duration::from_millis(FOLLOW_MS as u64));
    }
}

impl BoardCache {
    /// Forget the pictures of documents that `live` says are closed: their GPU snapshots and the
    /// documents they hold.
    pub(crate) fn retain_docs(&mut self, live: impl Fn(DocId) -> bool) {
        self.shots.retain(|(doc, _), _| live(*doc));
    }
}

/// Forget the pictures of closed documents.
fn retain(app: &mut PhotocraftApp) {
    let live: Vec<DocId> = app.session.documents().iter().map(|d| d.doc.id).collect();
    app.board_cache.retain_docs(|doc| live.contains(&doc));
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

    /// A kept picture is exact only while nothing that composites inside it changed.
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

    /// A change outside the picture counts when a layer effect reaches into it: the right half's
    /// 150 px stroke covers the left half's edge, so painting just past the edge is a change there.
    #[test]
    fn a_change_an_effect_carries_into_the_picture_counts() {
        let paint = |s: &mut Session| {
            let right = s.active().unwrap().doc.layers[1].id;
            s.edit("paint", |doc, _| {
                doc.layer_mut(right).unwrap().surface_mut().unwrap().fill_rect(Rect::new(3100, 100, 3110, 110), &[0.0, 1.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        };
        // Without the stroke, a change on the next tile over is elsewhere.
        let (mut s, left) = halves();
        let kept = s.active().unwrap().doc.clone();
        let rect = reach(&kept, kept.layer(left).unwrap()).unwrap();
        paint(&mut s);
        assert!(still_exact(&kept, &s.active().unwrap().doc.clone(), rect), "out of reach");
        // With it, the same change reaches the picture.
        let (mut s, left) = halves();
        s.execute("layer.layerStyle.stroke", json!({"size": 150})).unwrap();
        let kept = s.active().unwrap().doc.clone();
        assert!(super::super::textures::effect_reach(&kept.layers) >= 150, "the stroke reaches 150 px");
        let rect = reach(&kept, kept.layer(left).unwrap()).unwrap();
        paint(&mut s);
        assert!(!still_exact(&kept, &s.active().unwrap().doc.clone(), rect), "the stroke carries it inside");
    }

    /// A picture whose follow was put off doesn't keep the old document once the interval is
    /// over, even with no edit after (what `tick` does every frame): the board was hidden outside
    /// History, two edits came within the interval, and History then let them go. Showing the
    /// board again, also outside History, still finds the picture exact.
    #[test]
    fn a_put_off_picture_lets_the_old_document_go_without_another_edit() {
        let (mut s, left) = halves();
        let shown = s.active().unwrap().doc.clone();
        let rect = reach(&shown, shown.layer(left).unwrap()).unwrap();
        let mut kept = Kept { doc: shown, rect, followed: 0.0, due: false };
        let old = Arc::downgrade(&kept.doc);
        let set_visible = |s: &mut Session, on: bool| Arc::make_mut(&mut s.active_mut().unwrap().doc).layer_mut(left).unwrap().visible = on;
        let paint = |s: &mut Session, x: i32| {
            let right = s.active().unwrap().doc.layers[1].id;
            s.edit("paint", |doc, _| {
                doc.layer_mut(right).unwrap().surface_mut().unwrap().fill_rect(Rect::new(x, 100, x + 10, 110), &[0.0, 1.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        };
        set_visible(&mut s, false);
        paint(&mut s, 4500);
        paint(&mut s, 5000);
        assert!(kept.follow(left, &s.active().unwrap().doc, 500.0));
        assert!(kept.due, "put off within the interval");
        s.active_mut().unwrap().history.clear();
        assert!(old.upgrade().is_some(), "only the picture holds the old document now");
        assert!(kept.follow(left, &s.active().unwrap().doc, 1500.0), "still exact: the edits are elsewhere");
        assert!(!kept.due);
        assert!(old.upgrade().is_none(), "the old document is gone");
        set_visible(&mut s, true);
        assert!(still_exact(&kept.doc, &s.active().unwrap().doc, rect), "showing the board can put the picture back");
    }
}
