//! Onion skin for the frame animation: the neighbouring frames drawn faintly over the canvas, the
//! earlier ones tinted red and the later ones green, so the next pose can be drawn against them.
//! It only changes what the canvas shows; nothing is saved or exported.

use std::sync::Arc;

use craft::behavior::PixelSize;
use craft::ui::canvas::{PixelRect, RasterSource};
use craft::ui::primitives::Revision;
use egui::{Color32, ColorImage};
use photocraft_doc::{AnimFrame, DocId, Document, FrameAnimation, Layer, LayerId};
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
    image: Arc<ColorImage>,
    /// Counts the pictures this cache rendered: a new picture is a new revision for the raster display.
    serial: u64,
}

/// A ghost frame as a raster display reads it: the composited neighbour, reduced, drawn over the canvas
/// with its tint.
pub struct GhostSource {
    image: Arc<ColorImage>,
    tint: Color32,
    size: PixelSize,
    revision: Revision,
}

impl RasterSource for GhostSource {
    fn size(&self) -> PixelSize {
        self.size
    }

    fn revision(&self) -> Revision {
        self.revision
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        (since == self.revision).then(PixelRect::default)
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let [ow, oh] = out.size;
        let (iw, ih) = (self.image.size[0], self.image.size[1]);
        let (dw, dh) = (i64::from(self.size.width), i64::from(self.size.height));
        if ow == 0 || oh == 0 || iw == 0 || ih == 0 || dw == 0 || dh == 0 {
            return;
        }
        let factor = i64::from(factor.max(1));
        let t = self.tint.to_array().map(u32::from);
        for y in 0..oh {
            let dy = i64::from(region.y0) + y as i64 * factor + factor / 2;
            for x in 0..ow {
                let dx = i64::from(region.x0) + x as i64 * factor + factor / 2;
                // Past the document the ghost has nothing.
                let px = if dx < 0 || dy < 0 || dx >= dw || dy >= dh {
                    Color32::TRANSPARENT
                } else {
                    let gx = ((dx * iw as i64) / dw).clamp(0, iw as i64 - 1) as usize;
                    let gy = ((dy * ih as i64) / dh).clamp(0, ih as i64 - 1) as usize;
                    let p = self.image.pixels.get(gy * iw + gx).copied().unwrap_or(Color32::TRANSPARENT).to_array().map(u32::from);
                    Color32::from_rgba_premultiplied((p[0] * t[0] / 255) as u8, (p[1] * t[1] / 255) as u8, (p[2] * t[2] / 255) as u8, (p[3] * t[3] / 255) as u8)
                };
                if let Some(slot) = out.pixels.get_mut(y * ow + x) {
                    *slot = px;
                }
            }
        }
    }
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
    /// The pictures rendered so far, for the ghosts' serials.
    rendered: u64,
}

fn with_cache<R>(ctx: &egui::Context, f: impl FnOnce(&mut GhostCache) -> R) -> R {
    let id = egui::Id::unique("onion-skin-ghosts");
    let cell = ctx.data_mut(|d| d.get_temp::<std::sync::Arc<std::sync::Mutex<GhostCache>>>(id));
    let cell = cell.unwrap_or_else(|| {
        let c = std::sync::Arc::new(std::sync::Mutex::new(GhostCache::default()));
        ctx.data_mut(|d| d.insert_temp(id, c.clone()));
        c
    });
    let mut g = cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(&mut g)
}

/// The ghost pictures to draw over the canvas, farthest first, each with its tint. Empty unless the
/// onion skin is on, the Timeline is open and `canvas_doc`, the document being drawn, is the active one
/// with a frame animation (Window › Arrange tiles and floating windows show other documents).
pub fn ghosts(app: &PhotocraftApp, ctx: &egui::Context, canvas_doc: DocId) -> Vec<GhostSource> {
    let settings = app.ui.timeline.onion.clamped();
    // No ghosts while playing: every step would composite a new neighbour on the UI thread.
    if !settings.enabled || !app.ui.timeline.show_onion || !crate::panel_docking::is_shown(app, "timeline") || app.ui.timeline.playing() {
        return Vec::new();
    }
    let Some(st) = app.session.active().filter(|st| st.doc.id == canvas_doc) else { return Vec::new() };
    // A stale `current` (the document changed under the panel) draws nothing rather than indexing past the frames.
    let Some(a) = st.doc.frame_animation.as_ref().filter(|a| a.current < a.frames.len()) else { return Vec::new() };
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
                let image = render(&st.doc, a, i);
                c.rendered += 1;
                c.ghosts.insert(i, Ghost { key, image, serial: c.rendered });
            }
            if let Some(g) = c.ghosts.get(&i) {
                let tint = ghost_tint(settings, distance, earlier);
                let revision = Revision::of(&(st.doc.id.0, g.serial, tint.to_array()));
                out.push((distance, GhostSource { image: g.image.clone(), tint, size: PixelSize::new(st.doc.size.width, st.doc.size.height), revision }));
            }
        }
    });
    out.sort_by_key(|g| std::cmp::Reverse(g.0));
    out.into_iter().map(|(_, source)| source).collect()
}

/// What a ghost of frame `i` shows while frame `current` is on the canvas: the layers that look different
/// there (drawn in the ghost frame and not now). A layer both frames show the same way, such as an opaque
/// Background, would only wash out what is drawn on top of it.
///
/// A layer is drawn in a frame when its own eye and those of the groups above it are on. A group that holds a
/// layer which differs stays shown, or the compositor would skip what is inside it; a group the ghost frame
/// hides shows nothing, whatever is inside it.
pub fn ghost_frame(layers: &[Layer], a: &FrameAnimation, i: usize) -> AnimFrame {
    let (ghost, now) = (&a.frames[i], &a.frames[a.current]);
    /// `there` and `here`: every group above is on in the ghost frame, and in the current frame.
    fn walk(layers: &[Layer], ghost: &AnimFrame, now: &AnimFrame, (there, here): (bool, bool), hidden: &mut Vec<LayerId>) -> bool {
        let mut any = false;
        for l in layers {
            let drawn_there = there && ghost.hidden.binary_search(&l.id).is_err();
            let drawn_here = here && now.hidden.binary_search(&l.id).is_err();
            let inside = l.children().is_some_and(|c| walk(c, ghost, now, (drawn_there, drawn_here), hidden));
            let shown = (drawn_there && !drawn_here) || inside;
            if !shown {
                hidden.push(l.id);
            }
            any |= shown;
        }
        any
    }
    let mut hidden = Vec::new();
    walk(layers, ghost, now, (true, true), &mut hidden);
    hidden.sort();
    AnimFrame { delay_ms: 0, hidden }
}

/// Frame `i` of the animation as a ghost, composited and reduced.
fn render(doc: &Document, a: &FrameAnimation, i: usize) -> Arc<ColorImage> {
    let mut d = doc.clone();
    let only = FrameAnimation { frames: vec![Arc::new(ghost_frame(&d.layers, a, i))], current: 0, loops: Default::default() };
    only.apply(0, &mut d.layers);
    let img = photocraft_compose::thumbnail(&d, GHOST_SIDE);
    Arc::new(ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ghost of 2 × 2 pixels over a document of 4 × 4.
    fn source(tint: Color32) -> GhostSource {
        let px = [Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 255, 0), Color32::from_rgb(0, 0, 255), Color32::from_rgb(255, 255, 0)];
        GhostSource {
            image: Arc::new(ColorImage { size: [2, 2], source_size: egui::vec2(2.0, 2.0), pixels: px.to_vec() }),
            tint,
            size: PixelSize::new(4, 4),
            revision: Revision::of(&1u8),
        }
    }

    #[test]
    fn a_ghost_source_maps_its_picture_onto_the_document_and_tints_it() {
        let s = source(Color32::WHITE);
        let mut out = ColorImage::filled([4, 4], Color32::TRANSPARENT);
        s.read(PixelRect::new(0, 0, 4, 4), 1, &mut out);
        assert_eq!(out.pixels[0], Color32::from_rgb(255, 0, 0), "top left");
        assert_eq!(out.pixels[3], Color32::from_rgb(0, 255, 0), "top right");
        assert_eq!(out.pixels[12], Color32::from_rgb(0, 0, 255), "bottom left");
        assert_eq!(out.pixels[15], Color32::from_rgb(255, 255, 0), "bottom right");
        // A reduced read (a texel per two pixels) takes the picture's own pixels.
        let mut half = ColorImage::filled([2, 2], Color32::TRANSPARENT);
        s.read(PixelRect::new(0, 0, 4, 4), 2, &mut half);
        assert_eq!(half.pixels, s.image.pixels);
        // The tint multiplies every channel, alpha included.
        let faint = source(Color32::from_rgba_premultiplied(128, 128, 128, 128));
        let mut tinted = ColorImage::filled([1, 1], Color32::TRANSPARENT);
        faint.read(PixelRect::new(0, 0, 1, 1), 1, &mut tinted);
        assert_eq!(tinted.pixels[0].a(), 128);
        assert_eq!(tinted.pixels[0].r(), 128, "red at half strength");
        // Past the document there is nothing, and a read of a region it reaches past stays in bounds.
        let mut past = ColorImage::filled([4, 1], Color32::from_rgb(9, 9, 9));
        s.read(PixelRect::new(2, 0, 6, 1), 1, &mut past);
        assert_eq!(past.pixels[3], Color32::TRANSPARENT);
        assert_ne!(past.pixels[0], Color32::TRANSPARENT);
    }

    #[test]
    fn a_ghost_source_reads_only_its_own_revision_as_unchanged() {
        let s = source(Color32::WHITE);
        assert_eq!(s.damage(s.revision), Some(PixelRect::default()));
        assert_eq!(s.damage(Revision::of(&2u8)), None, "another revision: read everything");
    }

    #[test]
    fn neighbours_stop_at_the_ends() {
        let s = OnionSkin { enabled: true, before: 2, after: 2, opacity: 50 };
        assert_eq!(neighbours(s, 0, 3), [(1, 1, false), (2, 2, false)]);
        assert_eq!(neighbours(s, 1, 4), [(0, 1, true), (2, 1, false), (3, 2, false)]);
        assert!(neighbours(OnionSkin { before: 0, after: 0, ..s }, 1, 4).is_empty());
    }

    #[test]
    fn a_ghost_shows_only_what_differs_from_the_shown_frame() {
        let all = [layer(1), layer(2)];
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

    fn layer(id: u64) -> Layer {
        let mut l = Layer::group("L", Vec::new());
        l.id = LayerId(id);
        l
    }

    fn frames(hidden: [Vec<LayerId>; 2]) -> Vec<std::sync::Arc<AnimFrame>> {
        hidden.into_iter().map(|hidden| AnimFrame { delay_ms: 0, hidden }.into()).collect()
    }

    #[test]
    fn a_ghost_keeps_the_group_around_a_layer_that_differs() {
        // Group 1 is shown in both frames; the layer inside it, 2, only in the ghost frame.
        let mut group = layer(1);
        if let Some(c) = group.children_mut() {
            c.push(layer(2));
        }
        let a = FrameAnimation { frames: frames([vec![], vec![LayerId(2)]]), current: 1, loops: Default::default() };
        assert!(ghost_frame(&[group], &a, 0).hidden.is_empty(), "the group stays shown, or its child would never draw");
    }

    #[test]
    fn a_ghost_shows_the_children_of_a_group_that_only_the_ghost_frame_shows() {
        // Group 1 holds layer 2. The ghost frame shows the group, the current frame hides it, and the child's own
        // eye is on in both: what the ghost adds is the group's contents.
        let mut group = layer(1);
        if let Some(c) = group.children_mut() {
            c.push(layer(2));
        }
        let a = FrameAnimation { frames: frames([vec![], vec![LayerId(1)]]), current: 1, loops: Default::default() };
        assert!(ghost_frame(&[group], &a, 0).hidden.is_empty(), "the group and its child both draw in the ghost");
    }

    #[test]
    fn a_ghost_shows_nothing_of_a_group_the_ghost_frame_hides() {
        // Group 1 holds layer 2, whose own eye is on in both frames. The ghost frame hides the group, the current
        // frame shows it: the ghost adds nothing, however its child looks.
        let mut group = layer(1);
        if let Some(c) = group.children_mut() {
            c.push(layer(2));
        }
        let a = FrameAnimation { frames: frames([vec![LayerId(1)], vec![]]), current: 1, loops: Default::default() };
        assert_eq!(ghost_frame(&[group.clone()], &a, 0).hidden, [LayerId(1), LayerId(2)]);
        // And a child shown in the ghost frame, with the group on in both, is what the ghost adds.
        let b = FrameAnimation { frames: frames([vec![], vec![LayerId(2)]]), current: 1, loops: Default::default() };
        assert!(ghost_frame(&[group], &b, 0).hidden.is_empty());
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
