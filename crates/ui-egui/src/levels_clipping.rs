//! Photoshop's Levels and Curves clipping display (#3328, #14383). While Alt is held and the black
//! or white Input slider of Levels is dragged, or the black or white end point of Curves (by the
//! point or by its input slider), the canvas shows what that end clips instead of the preview. Levels
//! shows Photoshop's threshold display (#3328):
//! - black end: the image turns white; a channel clipped to black drops to 0, so pixels clipped in
//!   every channel are black and pixels clipped in red alone are cyan;
//! - white end: the image turns black; a channel clipped to white rises to 255, so red alone is
//!   red and all three are white.
//!
//! Curves shows each clipped channel in its own colour: on the composite curve red, green or blue
//! for one channel, yellow, cyan or magenta for two, and black (black end) or white (white end)
//! for all three; on the Red, Green or Blue curve only that channel, in its colour. Releasing Alt or the mouse shows the preview again; the document is never touched.
//! Grayscale documents show black and white only. CMYK and Lab documents, and commands that edit a
//! channel, a layer mask or a non-pixel layer, keep the normal preview.
//!
//! The Image › Adjustments dialogs show the target layer's pixels. A Levels or Curves layer's
//! Properties panel shows the composite under that layer, the input the adjustment layer sees.
//!
//! The display is an opaque texture over the visible part of the image, at about the screen's
//! resolution: one texel per block of `factor`×`factor` document pixels. A block shows a channel
//! clipped when any pixel in it is, so a lone clipped highlight stays visible zoomed out. Clipping
//! is a threshold on each channel's sample (the input stretch is monotone), so the target's
//! per-block minimum and maximum are scanned once per view and a slider move only compares them
//! with the new thresholds.
//!
//! State is the open dialog's clipping display (`HostedAdjustDialog::clipping`), or the Properties
//! panel's (`PropertiesAdjust::clipping`): the Levels and Curves panels report an Alt-drag into it
//! and clear it when the drag ends, and automation can set it as a dialog's `clipping` field. OK
//! never sees it: it is not a parameter of the adjustment.

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point;
use craft::imaging::ui::adjust::LevelsClip;
use egui::{Color32, Mesh, TextureOptions, pos2};
use photocraft_color::{ColorMode, SampleType};
use photocraft_compose::adjust::{curve_lut, levels_float, levels_q};
use photocraft_doc::adjust::{CurvePoint, LevelsChannel, ToneSpace};
use photocraft_doc::{Adjustment, DocId, Document, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::PhotocraftApp;

/// A pixel counts where the selection covers at least this much (the command blends the rest).
const SELECTED: f32 = 0.5;

/// Which ends the Levels input levels clip a sample `v` (0..1) of one channel to, as
/// (black, white): the channel's own input range or, after it, the composite's. `quantum` is the
/// document's (whole levels, as the compositor rounds them; None: 32-bit float, unclamped). Gamma
/// and the output levels don't matter: what reaches an end of an input range is flattened there,
/// wherever the output levels then put it.
pub fn classify(master: &LevelsChannel, channel: &LevelsChannel, v: f32, quantum: Option<f32>) -> (bool, bool) {
    let eval = |ch: &LevelsChannel, v: f32| if quantum.is_some() { levels_q(ch, v, quantum) } else { levels_float(ch, v) };
    let full = |ch: &LevelsChannel| LevelsChannel { out_black: 0.0, out_white: 1.0, ..ch.clone() };
    let stretch = |ch: &LevelsChannel| LevelsChannel { gamma: 1.0, ..full(ch) };
    let own = eval(&stretch(channel), v);
    let composite = eval(&stretch(master), eval(&full(channel), v));
    (own <= 0.0 || composite <= 0.0, own >= 1.0 || composite >= 1.0)
}

/// How far a sample may sit past an end point's input and still be flattened there: the inputs are
/// whole levels, and an 8-bit sample on that level must count.
const AT_END: f32 = 1e-5;

/// A curve's output at `v` (0..1) through its LUT ([`curve_lut`]), linearly between entries.
fn eval(table: &[f32], v: f32) -> f32 {
    let Some(last) = table.len().checked_sub(1) else { return v };
    let x = v.clamp(0.0, 1.0) * last as f32;
    let i = (x as usize).min(last);
    let j = (i + 1).min(last);
    let f = x - i as f32;
    match (table.get(i), table.get(j)) {
        (Some(a), Some(b)) => a * (1.0 - f) + b * f,
        _ => v,
    }
}

/// The inputs of a curve's end points (0..1); a curve of fewer than two points is the identity, as
/// the compositor draws it.
fn ends(points: &[CurvePoint]) -> (f32, f32) {
    match (points.first(), points.last()) {
        (Some(first), Some(last)) if points.len() >= 2 => (first.input.clamp(0.0, 1.0), last.input.clamp(0.0, 1.0)),
        _ => (0.0, 1.0),
    }
}

/// Which ends a Curves adjustment flattens a sample `v` (0..1) of one channel at, as (black,
/// white): its own curve, which is constant below its black end point's input and above its white
/// one's, or after it the composite curve. Outputs don't matter, as in Levels: what reaches an end
/// point's input is flattened there, wherever its output puts it.
pub fn classify_curves(master: &[CurvePoint], channel: &[CurvePoint], v: f32) -> (bool, bool) {
    let (own_black, own_white) = ends(channel);
    let (black, white) = ends(master);
    let through = eval(&curve_lut(channel), v);
    (v <= own_black + AT_END || through <= black + AT_END, v >= own_white - AT_END || through >= white - AT_END)
}

/// A sample as a 16-bit key: exact for 8- and 16-bit documents (their samples are on this grid),
/// 32-bit samples clamped to 0..1 (beyond is clipped either way). Non-finite: None.
fn key(v: f32) -> Option<u16> {
    v.is_finite().then(|| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16)
}

/// Per channel (R, G, B): the highest key clipped to black and the lowest clipped to white (None:
/// no key clips that way).
pub type Thresholds = [(Option<u16>, Option<u16>); 3];

/// The [`Thresholds`] of the Levels or Curves `adj` in a document of `depth`, every channel's.
/// None for other adjustments and for Levels and Curves outside the RGB channel space (which
/// Grayscale documents use too).
pub fn thresholds(adj: &Adjustment, depth: SampleType) -> Option<Thresholds> {
    thresholds_of(adj, depth, None)
}

/// [`thresholds`] for the curve shown: `only` a channel (0 red, 1 green, 2 blue) leaves the other
/// channels unclipped; Levels shows every channel whatever is shown. A Curves curve drawn with the
/// pencil has no end points to drag, so it has no display.
pub fn thresholds_of(adj: &Adjustment, depth: SampleType, only: Option<usize>) -> Option<Thresholds> {
    if let Adjustment::Curves { master, per_channel, space: ToneSpace::Rgb, pencil: false, .. } = adj {
        return Some(std::array::from_fn(|c| match only {
            Some(shown) if shown != c => (None, None),
            _ => curve_thresholds(master, &per_channel[c]),
        }));
    }
    let Adjustment::Levels { master, per_channel, space: ToneSpace::Rgb, .. } = adj else { return None };
    let quantum = photocraft_compose::adjustment_quantum(depth);
    let at = |ch: &LevelsChannel, k: u16| classify(master, ch, f32::from(k) / 65535.0, quantum);
    Some(std::array::from_fn(|c| {
        let ch = &per_channel[c];
        // Black clipping holds below a key and white clipping above one (the stretch is monotone).
        let black = (at(ch, 0).0).then(|| last_true(|k| at(ch, k).0));
        let white = (at(ch, u16::MAX).1).then(|| first_true(|k| at(ch, k).1));
        (black, white)
    }))
}

/// One channel's Curves thresholds: the keys from 0 up that all clip to black and from the top
/// down that all clip to white. Exact for the monotone curves the end points make; a curve that
/// turns back flattens a middle range too, which the display leaves out.
fn curve_thresholds(master: &[CurvePoint], channel: &[CurvePoint]) -> (Option<u16>, Option<u16>) {
    let (own_black, own_white) = ends(channel);
    let (black, white) = ends(master);
    let table = curve_lut(channel);
    let at = |k: u16| {
        let v = f32::from(k) / 65535.0;
        let through = eval(&table, v);
        (v <= own_black + AT_END || through <= black + AT_END, v >= own_white - AT_END || through >= white - AT_END)
    };
    let low = (0..=u16::MAX).take_while(|&k| at(k).0).last();
    let high = (0..=u16::MAX).rev().take_while(|&k| at(k).1).last();
    (low, high)
}

/// The largest key where `f` holds, given it holds at 0 and stops holding at most once.
fn last_true(f: impl Fn(u16) -> bool) -> u16 {
    let (mut lo, mut hi) = (0u32, u32::from(u16::MAX) + 1);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if f(mid as u16) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo as u16
}

/// The smallest key where `f` holds, given it holds at `u16::MAX` and starts holding at most once.
fn first_true(f: impl Fn(u16) -> bool) -> u16 {
    let (mut lo, mut hi) = (-1i32, i32::from(u16::MAX));
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if f(mid as u16) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi as u16
}

/// Per block of the scanned area: each channel's minimum (0..3) and maximum (3..6) key over the
/// pixels that count. A block with none has min > max.
type Extremes = [u16; 6];
const NONE: Extremes = [u16::MAX, u16::MAX, u16::MAX, 0, 0, 0];

/// The target's per-block extremes over `rect` (document pixels, `factor`×`factor` blocks).
pub struct Stats {
    rect: Rect,
    factor: u32,
    size: [usize; 2],
    blocks: Vec<Extremes>,
}

/// Scan `surface` (a layer's pixels) over `rect` in `factor`×`factor` blocks. Pixels count when
/// visible (alpha > 0, finite) and, with a `selection`, at least half selected.
pub fn scan(surface: &Surface, selection: Option<&Surface>, rect: Rect, factor: u32) -> Stats {
    let factor = factor.max(1);
    let f = factor as usize;
    let size = [rect.width().div_ceil(factor) as usize, rect.height().div_ceil(factor) as usize];
    let mut blocks = vec![NONE; size[0] * size[1]];
    let row = |by: usize, out: &mut [Extremes]| {
        let y0 = rect.y0.saturating_add((by * f) as i32);
        let band = Rect::new(rect.x0, y0, rect.x1, y0.saturating_add(factor as i32).min(rect.y1));
        let width = band.width() as usize;
        let mut px = vec![[0.0f32; 4]; width * band.height() as usize];
        surface.read_rgba_into(band, &mut px);
        let mut coverage = Vec::new();
        let stride = selection.map_or(1, |s| s.channels().max(1));
        if let Some(s) = selection {
            s.read_region_into(band, &mut coverage);
        }
        for (i, p) in px.iter().enumerate() {
            let visible = p[3] > 0.0;
            let selected = selection.is_none() || coverage.get(i * stride).is_some_and(|k| *k >= SELECTED);
            if !visible || !selected {
                continue;
            }
            let (Some(r), Some(g), Some(b)) = (key(p[0]), key(p[1]), key(p[2])) else { continue };
            let Some(e) = out.get_mut((i % width.max(1)) / f) else { continue };
            for (c, k) in [r, g, b].into_iter().enumerate() {
                e[c] = e[c].min(k);
                e[c + 3] = e[c + 3].max(k);
            }
        }
    };
    if size[0] > 0 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            blocks.par_chunks_mut(size[0]).enumerate().for_each(|(by, out)| row(by, out));
        }
        #[cfg(target_arch = "wasm32")]
        for (by, out) in blocks.chunks_mut(size[0]).enumerate() {
            row(by, out);
        }
    }
    Stats { rect, factor, size, blocks }
}

/// Scans the composite of `doc` over `rect` (document pixels, `factor`×`factor` blocks, `rect`
/// on the block grid) as [`scan`] scans a layer. Zoomed out, the composite is drawn at the block
/// size, one pixel per block (a proxy, so a lone clipped pixel can drop out). The selection is
/// left out: an adjustment layer acts on the whole image.
pub fn scan_composite(doc: &Document, rect: Rect, factor: u32) -> Stats {
    let factor = factor.max(1);
    let f = factor as i32;
    let size = [rect.width().div_ceil(factor) as usize, rect.height().div_ceil(factor) as usize];
    let mut blocks = vec![NONE; size[0] * size[1]];
    let (bx, by) = (rect.x0.div_euclid(f), rect.y0.div_euclid(f));
    let area = Rect::new(bx, by, bx.saturating_add(size[0] as i32), by.saturating_add(size[1] as i32));
    let proxy = (factor > 1).then(|| photocraft_compose::proxy::proxy_document(doc, factor));
    let _ = photocraft_compose::render_bands(proxy.as_ref().unwrap_or(doc), area, 0, |band| -> Result<(), ()> {
        let width = band.rect.width().max(1) as usize;
        for (i, p) in band.px.iter().enumerate() {
            // Transparent and NaN alpha don't count.
            if p[3].is_nan() || p[3] <= 0.0 {
                continue;
            }
            let (Some(r), Some(g), Some(b)) = (key(p[0]), key(p[1]), key(p[2])) else { continue };
            let x = (band.rect.x0 - area.x0) as usize + i % width;
            let y = (band.rect.y0 - area.y0) as usize + i / width;
            let Some(e) = blocks.get_mut(y * size[0] + x) else { continue };
            for (c, k) in [r, g, b].into_iter().enumerate() {
                e[c] = e[c].min(k);
                e[c + 3] = e[c + 3].max(k);
            }
        }
        Ok(())
    });
    Stats { rect, factor, size, blocks }
}

/// The display colour of a pixel clipped to black in the channels of `black` and to white in
/// those of `white` (bit 0 red, 1 green, 2 blue).
pub fn colour(end: LevelsClip, black: u8, white: u8) -> Color32 {
    let on = |mask: u8, c: u8| if mask & (1 << c) != 0 { 255 } else { 0 };
    match end {
        LevelsClip::Shadows => {
            let m = 7 ^ black;
            Color32::from_rgb(on(m, 0), on(m, 1), on(m, 2))
        }
        // `LevelsClip` is non-exhaustive: an end the framework adds shows as the white slider's display.
        _ => Color32::from_rgb(on(white, 0), on(white, 1), on(white, 2)),
    }
}

/// Curves' display colour of a pixel whose channels in `clipped` (bit 0 red, 1 green, 2 blue) clip
/// at `end`: the clipped channels at 255, so one channel shows in its colour and two mix; on white
/// for the black end and on black for the white end, and black or white where all three clip.
pub fn curves_colour(end: LevelsClip, clipped: u8) -> Color32 {
    let on = |c: u8| if clipped & (1 << c) != 0 { 255 } else { 0 };
    match (end, clipped & 7) {
        (LevelsClip::Shadows, 0) => Color32::WHITE,
        (LevelsClip::Shadows, 7) => Color32::BLACK,
        _ => Color32::from_rgb(on(0), on(1), on(2)),
    }
}

/// Which adjustment's colours a display shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    /// Levels: [`colour`].
    Levels,
    /// Curves: [`curves_colour`].
    Curves,
}

impl Palette {
    fn of(adj: &Adjustment) -> Self {
        if matches!(adj, Adjustment::Curves { .. }) { Palette::Curves } else { Palette::Levels }
    }

    /// The colour of a pixel whose channels in `clipped` clip at `end`.
    pub fn colour(self, end: LevelsClip, clipped: u8) -> Color32 {
        match (self, end) {
            (Palette::Curves, _) => curves_colour(end, clipped),
            (Palette::Levels, LevelsClip::Shadows) => colour(end, clipped, 0),
            (Palette::Levels, _) => colour(end, 0, clipped),
        }
    }
}

/// The Levels display of `stats` for `end` under the per-channel `thresholds` ([`image_in`]).
pub fn image(stats: &Stats, thresholds: &Thresholds, end: LevelsClip) -> Vec<Color32> {
    image_in(stats, thresholds, end, Palette::Levels)
}

/// The display of `stats` for `end` under the per-channel `thresholds` in `palette`'s colours, one
/// colour per block (in parallel off the web: this runs on every slider move).
pub fn image_in(stats: &Stats, thresholds: &Thresholds, end: LevelsClip, palette: Palette) -> Vec<Color32> {
    // Integer bounds (none clipped: below every key / above every key) and the 8 possible colours.
    let lo = thresholds.map(|(lo, _)| lo.map_or(-1, i32::from));
    let hi = thresholds.map(|(_, hi)| hi.map_or(i32::from(u16::MAX) + 1, i32::from));
    let shadows = end == LevelsClip::Shadows;
    let palette: [Color32; 8] = std::array::from_fn(|m| palette.colour(end, m as u8));
    let block = |e: &Extremes| {
        let mut mask = 0usize;
        for c in 0..3 {
            let (min, max) = (i32::from(e[c]), i32::from(e[c + 3]));
            let clipped = if shadows { min <= max && min <= lo[c] } else { min <= max && max >= hi[c] };
            mask |= usize::from(clipped) << c;
        }
        palette[mask]
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        stats.blocks.par_iter().map(block).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        stats.blocks.iter().map(block).collect()
    }
}

/// What the display scans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// The pixels of the dialog's target layer.
    Layer,
    /// The composite under the Properties panel's adjustment layer.
    Below,
}

/// What the display needs from the app: the end shown, the Levels or Curves adjustment and the
/// channel its curve shows, for the active document's target layer.
struct Request {
    end: LevelsClip,
    adj: Adjustment,
    /// The one channel shown (0 red, 1 green, 2 blue), or every channel.
    only: Option<usize>,
    doc: std::sync::Arc<Document>,
    revision: u64,
    target: LayerId,
    source: Source,
}

/// The adjustments with a clipping display.
fn shows_clipping(kind: &str) -> bool {
    matches!(kind, "levels" | "curves")
}

/// The channel the Channel select's entry `shown` stands for in an RGB or Gray document's list
/// (`values::tone_channels`: the composite, then Red, Green and Blue): None for the composite.
fn shown_channel(shown: usize) -> Option<usize> {
    shown.checked_sub(1).filter(|c| *c < 3)
}

/// Whether a Levels or Curves dialog is open. A Smart Filter's dialog edits the filter, not the layer's pixels, so
/// it has no display (`adjust_preview` keeps it off the layer path too).
fn dialog(app: &PhotocraftApp) -> Option<&crate::adjust_dialog::OpenAdjust> {
    crate::adjust_dialog::open_value(app).filter(|open| shows_clipping(&open.kind) && open.smart_filter.is_none())
}

/// The display asked for on document `idx`: by an open Levels or Curves dialog, else by the
/// Properties panel of a Levels or Curves layer. None when nothing shows clipping, `idx` isn't the
/// active document, or the document can't show it: CMYK and Lab, a channel, mask or Quick Mask
/// target, or a dialog target that isn't a pixel layer.
fn request(app: &PhotocraftApp, idx: usize) -> Option<Request> {
    match dialog(app) {
        Some(open) => dialog_request(app, open, idx),
        None => properties_request(app, idx),
    }
}

/// The display of the open dialog `open`.
fn dialog_request(app: &PhotocraftApp, open: &crate::adjust_dialog::OpenAdjust, idx: usize) -> Option<Request> {
    let hosted = crate::adjust_dialog::hosted(app, open.id)?;
    let end = hosted.clipping()?;
    let params = crate::adjust_dialog::open_params(app)?;
    if app.session.active_index() != Some(idx) || crate::adjust_preview::targets_channel(app, idx, &open.kind, &params) {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    if !matches!(st.doc.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return None;
    }
    let target = st.active_layer?;
    if !matches!(st.doc.layer(target).map(|l| &l.content), Some(LayerContent::Raster(_))) {
        return None;
    }
    let adj = photocraft_engine::adjust_params::from_params(&open.kind, &params, None, st.doc.mode).ok()?;
    let only = shown_channel(hosted.channel());
    Some(Request { end, adj, only, doc: st.doc.clone(), revision: st.revision, target, source: Source::Layer })
}

/// The display the Properties panel asks for: the active layer is the Levels or Curves layer it
/// shows, drawn in this pass or the one before (a panel closed mid-drag leaves no display behind).
fn properties_request(app: &PhotocraftApp, idx: usize) -> Option<Request> {
    let record = app.adjust_session.as_ref()?;
    let end = record.clipping?;
    if !record.drawn_recently(app.ctx()) || app.session.active_index() != Some(idx) {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    if !matches!(st.doc.mode, ColorMode::Rgb | ColorMode::Grayscale) || st.active_layer != Some(record.layer) {
        return None;
    }
    let Some(LayerContent::Adjustment(committed)) = st.doc.layer(record.layer).map(|l| &l.content) else { return None };
    let kind = photocraft_engine::commands::adjustment_kind(committed);
    if !shows_clipping(kind) {
        return None;
    }
    let adj = match &app.live_adjust {
        Some((layer, params)) if *layer == record.layer => photocraft_engine::adjust_params::from_params(kind, params, None, st.doc.mode).ok()?,
        _ => committed.clone(),
    };
    let only = shown_channel(record.channel);
    Some(Request { end, adj, only, doc: st.doc.clone(), revision: st.revision, target: record.layer, source: Source::Below })
}

/// The document as the adjustment layer `layer` sees it: that layer and every layer above it hidden.
fn below(doc: &Document, layer: LayerId) -> Document {
    let mut d = doc.clone();
    crate::tone::hide_above(&mut d.layers, layer);
    if let Some(l) = d.layer_mut(layer) {
        l.visible = false;
    }
    d
}

/// The display's per-view state on the app.
pub struct Cache {
    /// What `stats` scanned: document, revision, target, its source, area and block size.
    key: (DocId, u64, LayerId, Source, Rect, u32),
    stats: Stats,
    /// The end, thresholds and colours the texture shows.
    drawn: Option<(LevelsClip, Thresholds, Palette)>,
    tex: Option<egui::TextureHandle>,
}

/// Most blocks the display builds (2048 × 2048). A 2560×1440 canvas still gets one block per
/// device pixel, a bigger one bigger blocks: the block stats stay under 50 MB, the texture at
/// 16 MB, and a slider move near the 2 ms `levels_clipping_bench` measures for 3.7M blocks.
const MAX_BLOCKS: u64 = 2048 * 2048;

/// The visible part of the document in `xf` and the block size giving about one texel per screen
/// pixel (`ppp` physical pixels per point), within `max_side` texels a side and [`MAX_BLOCKS`].
fn view_area(view: &CanvasView, bounds: Rect, ppp: f32, max_side: u32) -> Option<(Rect, u32)> {
    // `CanvasView::zoom` is screen points per document unit, so the device zoom takes the pixel ratio.
    let device_zoom = view.zoom as f32 * ppp;
    if !(device_zoom.is_finite() && device_zoom > 0.0) {
        return None;
    }
    let visible = crate::canvas::visible_doc_rect(view).intersect(&bounds);
    if visible.is_empty() {
        return None;
    }
    let max_side = max_side.max(1);
    let (w, h) = (visible.width(), visible.height());
    let budget = ((u64::from(w) * u64::from(h)) as f64 / MAX_BLOCKS as f64).sqrt().ceil() as u32;
    let mut factor = ((1.0 / device_zoom).floor().clamp(1.0, 65536.0) as u32).max(w.div_ceil(max_side)).max(h.div_ceil(max_side)).max(budget);
    // Snap to the block grid so panning keeps the blocks where they were. That can widen the area
    // by up to a block, so grow the blocks until the snapped area fits.
    loop {
        let f = i32::try_from(factor).ok()?;
        let r = Rect::new(visible.x0.div_euclid(f) * f, visible.y0.div_euclid(f) * f, visible.x1, visible.y1).intersect(&bounds);
        let (bw, bh) = (r.width().div_ceil(factor), r.height().div_ceil(factor));
        if bw <= max_side && bh <= max_side && u64::from(bw) * u64::from(bh) <= MAX_BLOCKS {
            return Some((r, factor));
        }
        factor = factor.checked_add(1)?;
    }
}

/// The clipping display over the image in canvas `view` of document `idx`, if one is asked for:
/// the shapes the canvas paints under its controls. Drops the cache once no Levels or Curves
/// dialog is open and the Properties panel shows none. Only the main canvas calls it: the one
/// cache follows that view.
pub fn shapes(app: &mut PhotocraftApp, ctx: &egui::Context, view: &CanvasView, idx: usize) -> Vec<egui::Shape> {
    if dialog(app).is_none() && properties_request(app, idx).is_none() {
        app.levels_clipping = None;
        return Vec::new();
    }
    // A background job holds the workers and may replace the document: no thresholds, scan or
    // image until it ends.
    if app.session.has_jobs() {
        return Vec::new();
    }
    let Some(req) = request(app, idx) else { return Vec::new() };
    let Some(thr) = thresholds_of(&req.adj, req.doc.pixel_format().sample, req.only) else { return Vec::new() };
    let max_side = (ctx.input(|i| i.max_texture_side) as u32).clamp(256, 8192);
    let Some((rect, factor)) = view_area(view, req.doc.bounds(), ctx.pixels_per_point(), max_side) else { return Vec::new() };
    let key = (req.doc.id, req.revision, req.target, req.source, rect, factor);
    if app.levels_clipping.as_ref().is_none_or(|c| c.key != key) {
        let stats = match req.source {
            Source::Layer => {
                let Some(LayerContent::Raster(surface)) = req.doc.layer(req.target).map(|l| &l.content) else { return Vec::new() };
                scan(surface, req.doc.selection.as_ref(), rect, factor)
            }
            Source::Below => scan_composite(&below(&req.doc, req.target), rect, factor),
        };
        let tex = app.levels_clipping.take().and_then(|c| c.tex);
        app.levels_clipping = Some(Cache { key, stats, drawn: None, tex });
    }
    let Some(cache) = app.levels_clipping.as_mut() else { return Vec::new() };
    let palette = Palette::of(&req.adj);
    if cache.drawn != Some((req.end, thr, palette)) || cache.tex.is_none() {
        let img = egui::ColorImage::new(cache.stats.size, image_in(&cache.stats, &thr, req.end, palette));
        match &mut cache.tex {
            Some(t) => t.set(img, TextureOptions::NEAREST),
            None => cache.tex = Some(ctx.load_texture("levels-clipping", img, TextureOptions::NEAREST)),
        }
        cache.drawn = Some((req.end, thr, palette));
    }
    let Some(tex) = cache.tex.as_ref().map(egui::TextureHandle::id) else { return Vec::new() };
    // The texture covers whole blocks; the last row and column may reach past the area.
    let s = &cache.stats;
    let f = s.factor as f32;
    let uv = [s.rect.width() as f32 / (s.size[0] as f32 * f).max(1.0), s.rect.height() as f32 / (s.size[1] as f32 * f).max(1.0)];
    let (x0, y0, x1, y1) = (s.rect.x0 as f32, s.rect.y0 as f32, s.rect.x1 as f32, s.rect.y1 as f32);
    let mut mesh = Mesh::with_texture(tex);
    for (p, t) in [((x0, y0), (0.0, 0.0)), ((x1, y0), (uv[0], 0.0)), ((x1, y1), (uv[0], uv[1])), ((x0, y1), (0.0, uv[1]))] {
        mesh.vertices.push(egui::epaint::Vertex { pos: view.to_screen(Point::new(f64::from(p.0), f64::from(p.1))), uv: pos2(t.0, t.1), color: Color32::WHITE });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    vec![egui::Shape::mesh(mesh)]
}

/// [`shapes`] added to `painter`.
#[cfg(test)]
pub fn paint(app: &mut PhotocraftApp, ctx: &egui::Context, painter: &egui::Painter, view: &CanvasView, idx: usize) {
    painter.extend(shapes(app, ctx, view, idx));
}

#[cfg(test)]
#[path = "levels_clipping_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "curves_clipping_tests.rs"]
mod curves_tests;
