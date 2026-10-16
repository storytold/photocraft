//! Onion skin for the frame animation: the neighbouring frames drawn faintly over the canvas, the
//! earlier ones tinted red and the later ones green, so the next pose can be drawn against them.
//! It only changes what the canvas shows; nothing is saved or exported.

use egui::{Color32, TextureHandle, TextureOptions};
use photocraft_doc::{AnimFrame, DocId, Document, FrameAnimation, LayerId, layer_ids};
use serde::{Deserialize, Serialize};

use crate::PhotocraftApp;

/// Longest side of a ghost frame, in pixels.
const GHOST_SIDE: u32 = 1024;
/// Most neighbouring frames shown on either side.
pub const MAX_NEIGHBOURS: u8 = 5;

/// The onion-skin settings (Photoshop's Onion Skin Settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnionSkin {
    pub enabled: bool,
    /// Earlier frames shown.
    pub before: u8,
    /// Later frames shown.
    pub after: u8,
    /// Opacity of the nearest frame, in percent; the others fade with their distance.
    pub opacity: u8,
}

impl Default for OnionSkin {
    fn default() -> Self {
        OnionSkin { enabled: false, before: 1, after: 0, opacity: 40 }
    }
}

impl OnionSkin {
    /// Keeps the counts and the opacity in range, whatever was stored.
    pub fn clamped(self) -> Self {
        OnionSkin { before: self.before.min(MAX_NEIGHBOURS), after: self.after.min(MAX_NEIGHBOURS), opacity: self.opacity.clamp(1, 100), ..self }
    }
}

/// The frames to ghost for frame `current` of `len`, as (frame, distance, earlier).
pub fn neighbours(s: OnionSkin, current: usize, len: usize) -> Vec<(usize, u32, bool)> {
    let s = s.clamped();
    let mut out = Vec::new();
    for d in 1..=u32::from(s.before) {
        if let Some(i) = current.checked_sub(d as usize) {
            out.push((i, d, true));
        }
    }
    for d in 1..=u32::from(s.after) {
        let i = current + d as usize;
        if i < len {
            out.push((i, d, false));
        }
    }
    out
}

/// The tint a ghost is drawn with: red before, green after, fainter the further it is.
pub fn ghost_tint(s: OnionSkin, distance: u32, earlier: bool) -> Color32 {
    let alpha = f32::from(s.clamped().opacity) / 100.0 / distance.max(1) as f32;
    let base = if earlier { Color32::from_rgb(255, 120, 120) } else { Color32::from_rgb(120, 255, 140) };
    base.gamma_multiply(alpha)
}

struct Ghost {
    /// The layers the ghost frame and the shown frame hid when this was drawn, and the pixel version.
    key: (Vec<LayerId>, Vec<LayerId>, u64),
    tex: TextureHandle,
}

#[derive(Default)]
struct GhostCache {
    doc: Option<DocId>,
    rev: u64,
    /// Counts changes to the pixels: selecting a frame doesn't change them.
    epoch: u64,
    /// The last revision that was only frame selections since `rev` (see `continues_navigation`).
    pure_nav: Option<u64>,
    ghosts: std::collections::HashMap<usize, Ghost>,
}

fn with_cache<R>(ctx: &egui::Context, f: impl FnOnce(&mut GhostCache) -> R) -> R {
    let id = egui::Id::new("onion-skin-ghosts");
    let cell = ctx.data_mut(|d| d.get_temp::<std::sync::Arc<std::sync::Mutex<GhostCache>>>(id));
    let cell = cell.unwrap_or_else(|| {
        let c = std::sync::Arc::new(std::sync::Mutex::new(GhostCache::default()));
        ctx.data_mut(|d| d.insert_temp(id, c.clone()));
        c
    });
    let mut g = cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(&mut g)
}

/// The ghost textures to draw over the canvas, farthest first, each with its tint. Empty unless the
/// onion skin is on, the Timeline is open and `canvas_doc`, the document being drawn, is the active one
/// with a frame animation (Window › Arrange tiles and floating windows show other documents).
pub fn ghosts(app: &PhotocraftApp, ctx: &egui::Context, canvas_doc: DocId) -> Vec<(egui::TextureId, Color32)> {
    let settings = app.ui.timeline.onion.clamped();
    // No ghosts while playing: every step would composite a new neighbour on the UI thread.
    if !settings.enabled || !app.ui.timeline.show_onion || !app.ui.timeline.open || app.ui.timeline.playing {
        return Vec::new();
    }
    let Some(st) = app.session.active().filter(|st| st.doc.id == canvas_doc) else { return Vec::new() };
    let Some(a) = st.doc.frame_animation.as_ref() else { return Vec::new() };
    let wanted = neighbours(settings, a.current, a.frames.len());
    let nav = crate::frame_strip_ui::nav_rev(ctx);
    let mut out = Vec::new();
    with_cache(ctx, |c| {
        if c.doc != Some(st.doc.id) {
            *c = GhostCache { doc: Some(st.doc.id), rev: st.revision, ..Default::default() };
        }
        if c.rev != st.revision {
            // Only selections since the cache last looked leave the ghosts' pixels as they were.
            if nav == Some(st.revision) && crate::frame_strip_ui::continues_navigation(c.rev, c.pure_nav, st.revision) {
                c.pure_nav = Some(st.revision);
            } else {
                c.epoch += 1;
                c.pure_nav = None;
            }
            c.rev = st.revision;
        }
        let keep: Vec<usize> = wanted.iter().map(|w| w.0).collect();
        c.ghosts.retain(|i, _| keep.contains(i));
        // One new ghost per UI frame, so stepping through a long animation stays smooth.
        let mut budget = 1;
        for &(i, distance, earlier) in &wanted {
            let key = (a.frames[i].hidden.clone(), a.frames[a.current].hidden.clone(), c.epoch);
            if c.ghosts.get(&i).is_none_or(|g| g.key != key) {
                if budget == 0 {
                    ctx.request_repaint();
                    continue;
                }
                budget -= 1;
                let tex = render(ctx, &st.doc, a, i, c.ghosts.remove(&i).map(|g| g.tex));
                c.ghosts.insert(i, Ghost { key, tex });
            }
            if let Some(g) = c.ghosts.get(&i) {
                out.push((distance, g.tex.id(), ghost_tint(settings, distance, earlier)));
            }
        }
    });
    out.sort_by_key(|g| std::cmp::Reverse(g.0));
    out.into_iter().map(|(_, id, tint)| (id, tint)).collect()
}

/// What a ghost of frame `i` shows while frame `current` is on the canvas: the layers that look different
/// there (shown in one and not the other). A layer both frames show the same
/// way, such as an opaque Background, would only wash out what is drawn on top of it.
pub fn ghost_frame(all: &std::collections::HashSet<LayerId>, a: &FrameAnimation, i: usize) -> AnimFrame {
    let (ghost, now) = (&a.frames[i], &a.frames[a.current]);
    let differs = |id: &LayerId| {
        let shown_there = ghost.hidden.binary_search(id).is_err();
        let shown_now = now.hidden.binary_search(id).is_err();
        shown_there && !shown_now
    };
    let mut hidden: Vec<LayerId> = all.iter().filter(|id| !differs(id)).copied().collect();
    hidden.sort();
    AnimFrame { delay_ms: 0, hidden }
}

/// Frame `i` of the animation as a ghost, composited and reduced.
fn render(ctx: &egui::Context, doc: &Document, a: &FrameAnimation, i: usize, reuse: Option<TextureHandle>) -> TextureHandle {
    let mut d = doc.clone();
    let only = FrameAnimation { frames: vec![std::sync::Arc::new(ghost_frame(&layer_ids(&d.layers), a, i))], current: 0, loops: Default::default() };
    only.apply(0, &mut d.layers);
    let img = photocraft_compose::thumbnail(&d, GHOST_SIDE);
    let color = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
    match reuse {
        Some(mut tex) => {
            tex.set(color, TextureOptions::LINEAR);
            tex
        }
        None => ctx.load_texture(format!("onion-ghost-{i}"), color, TextureOptions::LINEAR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbours_stop_at_the_ends() {
        let s = OnionSkin { enabled: true, before: 2, after: 2, opacity: 50 };
        assert_eq!(neighbours(s, 0, 3), [(1, 1, false), (2, 2, false)]);
        assert_eq!(neighbours(s, 1, 4), [(0, 1, true), (2, 1, false), (3, 2, false)]);
        assert!(neighbours(OnionSkin { before: 0, after: 0, ..s }, 1, 4).is_empty());
    }

    #[test]
    fn a_ghost_shows_only_what_differs_from_the_shown_frame() {
        let all: std::collections::HashSet<LayerId> = [1, 2].map(LayerId).into();
        // Layer 1 (a Background) is shown in both frames; layer 2 only in the ghost frame.
        let a = FrameAnimation {
            frames: vec![AnimFrame { delay_ms: 0, hidden: vec![] }.into(), AnimFrame { delay_ms: 0, hidden: vec![LayerId(2)] }.into()],
            current: 1,
            loops: Default::default(),
        };
        assert_eq!(ghost_frame(&all, &a, 0).hidden, [LayerId(1)], "the shared Background stays out of the ghost");
        // A frame ghosted against itself shows nothing.
        let same = FrameAnimation { current: 0, ..a.clone() };
        assert_eq!(ghost_frame(&all, &same, 0).hidden.len(), 2);
    }

    #[test]
    fn settings_are_kept_in_range() {
        let s = OnionSkin { enabled: true, before: 99, after: 200, opacity: 0 }.clamped();
        assert_eq!((s.before, s.after, s.opacity), (MAX_NEIGHBOURS, MAX_NEIGHBOURS, 1));
    }

    #[test]
    fn farther_ghosts_are_fainter_and_directions_differ() {
        let s = OnionSkin { enabled: true, before: 2, after: 2, opacity: 80 };
        let (near, far) = (ghost_tint(s, 1, true), ghost_tint(s, 2, true));
        assert!(near.a() > far.a());
        assert_ne!(ghost_tint(s, 1, true), ghost_tint(s, 1, false));
    }
}
