//! Histogram panel (Window › Histogram): Compact, Expanded and All Channels
//! views, a channel and a source selector, statistics for the level under the pointer or a
//! dragged tonal range, the cached-data warning and Uncached Refresh, and the adjusted histogram
//! over the original while an adjustment previews.
//!
//! Histograms are counted off the UI thread (`tone::compute_histograms_at`) for the document
//! revision, source, selection and preview they describe, and kept until one of those changes;
//! the panel shows the last one with the cached-data warning until the new one lands. Like
//! Photoshop's panel it follows the selection (`tone::HistScope::Selection`): a selection change
//! makes every cached histogram stale, however few pixels it moved. The framework's scope panel
//! draws the counts and reads the pointer; this module counts, keys and caches them.

use std::sync::Arc;

use craft::imaging::scopes::histogram_statistics;
use craft::imaging::ui::adjust::{ToneChannel, ToneSpace as UiToneSpace};
use craft::imaging::ui::histogram::{ChannelColor, LevelRange};
use craft::imaging::ui::scopes::{
    ChannelOverlay, HistogramSet, ScopeChannel, ScopeCommand, ScopeFrame, ScopeMenuAction, ScopePanel, ScopeSettings, ScopeSource, ScopeView,
};
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandRef, CommandSet as _, CommandState};
use craft::ui::runtime::env::Label;
use photocraft_color::ColorMode;
use photocraft_doc::adjust::ToneSpace;
use photocraft_doc::{DocId, Document, LayerContent};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::tone::{self, HistScope, HistSource, Histograms};

/// Where the panel options are kept between launches (`prefs.dialogs`).
const OPTIONS_PREF: &str = "histogramPanel";

/// The Channel menu. Which entries a document offers depends on its colour mode
/// ([`channel_options`]); one it lacks shows as its composite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistChannel {
    /// RGB, CMYK, Lab's Lightness or Gray.
    #[default]
    Composite,
    Red,
    Green,
    Blue,
    Cyan,
    Magenta,
    Yellow,
    Black,
    A,
    B,
    Luminosity,
    /// The colour channels overlaid in their colours.
    Colors,
}

impl HistChannel {
    /// Every channel, in the order of [`HistChannel::key`]'s names.
    const ALL: [HistChannel; 12] = [
        HistChannel::Composite,
        HistChannel::Red,
        HistChannel::Green,
        HistChannel::Blue,
        HistChannel::Cyan,
        HistChannel::Magenta,
        HistChannel::Yellow,
        HistChannel::Black,
        HistChannel::A,
        HistChannel::B,
        HistChannel::Luminosity,
        HistChannel::Colors,
    ];

    /// The channel's key in `ScopeSettings::channel`, in `ui.inspect` and in the saved options.
    pub fn key(self) -> &'static str {
        match self {
            HistChannel::Composite => "composite",
            HistChannel::Red => "red",
            HistChannel::Green => "green",
            HistChannel::Blue => "blue",
            HistChannel::Cyan => "cyan",
            HistChannel::Magenta => "magenta",
            HistChannel::Yellow => "yellow",
            HistChannel::Black => "black",
            HistChannel::A => "a",
            HistChannel::B => "b",
            HistChannel::Luminosity => "luminosity",
            HistChannel::Colors => craft::imaging::ui::scopes::COLORS_CHANNEL,
        }
    }

    /// The channel `key` names; an unknown or empty key is the composite ([`resolve_channel`] then applies the mode).
    pub fn from_key(key: &str) -> Self {
        Self::ALL.into_iter().find(|c| c.key() == key).unwrap_or_default()
    }

    /// The channel as the tone editors name it, `None` for the two the editors lack (Luminosity and the Colors overlay).
    fn tone_channel(self, mode: ColorMode) -> Option<ToneChannel> {
        Some(match self {
            HistChannel::Composite if gray(mode) => ToneChannel::Gray,
            HistChannel::Composite => match tone_space(mode) {
                ToneSpace::Rgb => ToneChannel::Composite(UiToneSpace::Rgb),
                ToneSpace::Cmyk => ToneChannel::Composite(UiToneSpace::Cmyk),
                ToneSpace::Lab => ToneChannel::Lightness,
            },
            HistChannel::Red => ToneChannel::Red,
            HistChannel::Green => ToneChannel::Green,
            HistChannel::Blue => ToneChannel::Blue,
            HistChannel::Cyan => ToneChannel::Cyan,
            HistChannel::Magenta => ToneChannel::Magenta,
            HistChannel::Yellow => ToneChannel::Yellow,
            HistChannel::Black => ToneChannel::Black,
            HistChannel::A => ToneChannel::A,
            HistChannel::B => ToneChannel::B,
            HistChannel::Luminosity | HistChannel::Colors => return None,
        })
    }

    /// The colour the channel draws in; Luminosity and the composite are neutral.
    fn ink(self, mode: ColorMode) -> ChannelColor {
        self.tone_channel(mode).map_or(ChannelColor::Neutral, |channel| channel.color())
    }
}

/// What the histogram counts (the Source menu, shown for documents with several layers).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistSourceChoice {
    /// The composite of every layer.
    #[default]
    EntireImage,
    /// The selected layer on its own.
    SelectedLayer,
    /// The selected adjustment layer and every layer beneath it.
    AdjustmentComposite,
}

impl HistSourceChoice {
    /// The choice's key in `ScopeSettings::source` and in `ui.inspect`.
    pub fn key(self) -> &'static str {
        match self {
            HistSourceChoice::EntireImage => "entireImage",
            HistSourceChoice::SelectedLayer => "selectedLayer",
            HistSourceChoice::AdjustmentComposite => "adjustmentComposite",
        }
    }

    /// The choice `key` names; an unknown or empty key is the entire image.
    pub fn from_key(key: &str) -> Self {
        [HistSourceChoice::EntireImage, HistSourceChoice::SelectedLayer, HistSourceChoice::AdjustmentComposite]
            .into_iter()
            .find(|c| c.key() == key)
            .unwrap_or_default()
    }
}

/// Saves the panel options with the preferences.
pub fn persist_options(app: &mut PhotocraftApp) {
    if let Ok(value) = serde_json::to_value(&app.ui.histogram.settings) {
        app.session.prefs.edit(|p| {
            p.dialogs.insert(OPTIONS_PREF.into(), value);
        });
    }
}

/// Restores the saved panel options; unreadable ones keep the defaults.
pub fn restore_options(app: &mut PhotocraftApp) {
    if let Some(v) = app.session.prefs().dialogs.get(OPTIONS_PREF)
        && let Ok(settings) = serde_json::from_value::<ScopeSettings>(v.clone())
    {
        app.ui.histogram.settings = settings;
    }
}

/// The selected level range as whole levels, in order.
pub(crate) fn range_levels(range: Option<LevelRange>) -> Option<[u8; 2]> {
    let level = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    range.map(|r| [level(r.start), level(r.end)])
}

// ---------------------------------------------------------------------------------------------
// Channels

/// The channels a document in `mode` offers, with their labels, in menu order.
pub fn channel_options(mode: ColorMode) -> Vec<(HistChannel, &'static str)> {
    use HistChannel::*;
    match tone_space(mode) {
        _ if gray(mode) => vec![(Composite, "Gray")],
        ToneSpace::Cmyk => vec![
            (Composite, "CMYK"),
            (Cyan, "Cyan"),
            (Magenta, "Magenta"),
            (Yellow, "Yellow"),
            (Black, "Black"),
            (Luminosity, "Luminosity"),
            (Colors, "Colors"),
        ],
        ToneSpace::Lab => vec![(Composite, "Lightness"), (A, "a"), (B, "b"), (Luminosity, "Luminosity")],
        ToneSpace::Rgb => vec![(Composite, "RGB"), (Red, "Red"), (Green, "Green"), (Blue, "Blue"), (Luminosity, "Luminosity"), (Colors, "Colors")],
    }
}

fn gray(mode: ColorMode) -> bool {
    matches!(mode, ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone)
}

/// The space the panel counts a document of `mode` in.
pub fn tone_space(mode: ColorMode) -> ToneSpace {
    photocraft_engine::adjust_params::default_space(mode)
}

/// `want`, or the composite when a document of `mode` doesn't have it.
pub fn resolve_channel(mode: ColorMode, want: HistChannel) -> HistChannel {
    if channel_options(mode).iter().any(|(c, _)| *c == want) { want } else { HistChannel::Composite }
}

/// The single colour channels of a document of `mode` (All Channels View; Colors overlays them).
pub fn color_channels(mode: ColorMode) -> &'static [HistChannel] {
    use HistChannel::*;
    match tone_space(mode) {
        _ if gray(mode) => &[],
        ToneSpace::Cmyk => &[Cyan, Magenta, Yellow, Black],
        ToneSpace::Lab => &[A, B],
        ToneSpace::Rgb => &[Red, Green, Blue],
    }
}

/// The 256 bins of `channel` in `h` (counted in the space of a document of `mode`).
pub fn bins(h: &Histograms, mode: ColorMode, channel: HistChannel) -> &[u32; 256] {
    use HistChannel::*;
    let i = match (tone_space(mode), channel) {
        (_, Luminosity) => return &h.lum,
        (ToneSpace::Rgb, Red) | (ToneSpace::Cmyk, Cyan) | (ToneSpace::Lab, A) => 1,
        (ToneSpace::Rgb, Green) | (ToneSpace::Cmyk, Magenta) | (ToneSpace::Lab, B) => 2,
        (ToneSpace::Rgb, Blue) | (ToneSpace::Cmyk, Yellow) => 3,
        (ToneSpace::Cmyk, Black) => 4,
        _ => 0,
    };
    &h[i]
}

/// The label a channel is listed under in a document of `mode`.
pub fn channel_label(mode: ColorMode, channel: HistChannel) -> &'static str {
    channel_options(mode).into_iter().find(|(c, _)| *c == channel).map_or("", |(_, l)| l)
}

/// The level of `channel` at a straight RGBA 0..1 pixel, as the histogram bins it (the Color
/// Sampler marks).
pub fn channel_level(mode: ColorMode, channel: HistChannel, rgba: [f32; 4]) -> u8 {
    use HistChannel::*;
    let rgb = [rgba[0], rgba[1], rgba[2]].map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) });
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let lum = || q(rgb.iter().zip(tone::LUMINOSITY).map(|(v, w)| v * w).sum());
    match (tone_space(mode), channel) {
        (_, Luminosity) => lum(),
        (ToneSpace::Rgb, Red) => q(rgb[0]),
        (ToneSpace::Rgb, Green) => q(rgb[1]),
        (ToneSpace::Rgb, Blue) => q(rgb[2]),
        (ToneSpace::Rgb, _) => ((u16::from(q(rgb[0])) + u16::from(q(rgb[1])) + u16::from(q(rgb[2]))) / 3) as u8,
        (ToneSpace::Cmyk, c) => {
            let ink = photocraft_color::convert::rgb_to_cmyk(rgb);
            let v = match c {
                Cyan => ink[0],
                Magenta => ink[1],
                Yellow => ink[2],
                Black => ink[3],
                _ => ink.iter().sum::<f32>() / 4.0,
            };
            q(1.0 - v)
        }
        (ToneSpace::Lab, c) => {
            let l = photocraft_color::convert::srgb_to_lab(rgb);
            match c {
                A => q((l[1] + 128.0) / 255.0),
                B => q((l[2] + 128.0) / 255.0),
                _ => q(l[0] / 100.0),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Computing and caching

/// What one panel histogram was counted from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistKey {
    pub doc: DocId,
    pub revision: u64,
    /// `tone::tag` of the source, space and selection (the panel counts `HistScope::Selection`,
    /// so a selection change is a new key however few pixels it moved).
    pub source: u64,
    /// The adjustment preview's display key; 0 for the document itself.
    pub preview: u64,
    /// The highest cache level allowed (1: an Uncached Refresh).
    pub max_level: u32,
}

/// A counted panel histogram.
#[derive(Clone, Debug)]
pub struct PanelHist {
    pub key: HistKey,
    /// When it landed (ms), for throttling recounts while the document keeps changing.
    pub at: f64,
    pub hist: Arc<Histograms>,
}

/// A histogram being counted off the UI thread: its key, and the receiver of (key, ms, result).
pub(crate) type HistJob = (HistKey, std::sync::mpsc::Receiver<(HistKey, f64, Arc<Histograms>)>);

/// The source the panel counts for `choice` in `st`: Entire Image whenever the choice doesn't
/// apply (a single layer, no selected layer, or Adjustment Composite without an adjustment layer
/// selected).
pub fn resolve_source(st: &photocraft_engine::DocState, choice: HistSourceChoice) -> (HistSourceChoice, HistSource) {
    if st.doc.layer_count() < 2 {
        return (HistSourceChoice::EntireImage, tone::IMAGE);
    }
    let selected = st.active_layer.and_then(|id| st.doc.layer(id));
    match (choice, selected) {
        (HistSourceChoice::SelectedLayer, Some(l)) => (choice, HistSource::Layer(l.id)),
        (HistSourceChoice::AdjustmentComposite, Some(l)) if matches!(l.content, LayerContent::Adjustment(_)) => (choice, HistSource::UpTo(l.id)),
        _ => (HistSourceChoice::EntireImage, tone::IMAGE),
    }
}

/// Whether Adjustment Composite applies to `st` (an adjustment layer is selected).
fn adjustment_selected(st: &photocraft_engine::DocState) -> bool {
    st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| matches!(l.content, LayerContent::Adjustment(_)))
}

/// Preferences › Performance › Cache Levels: the highest level the panel counts at by default.
fn max_cache_level(app: &PhotocraftApp) -> u32 {
    app.session.prefs().performance.cache_levels.clamp(1, tone::MAX_CACHE_LEVEL)
}

/// The source the panel counts: Compact View shows no Source menu and represents the entire image.
fn source_choice(opts: &ScopeSettings) -> HistSourceChoice {
    if opts.view == ScopeView::Compact { HistSourceChoice::EntireImage } else { HistSourceChoice::from_key(&opts.source) }
}

/// The key of the document's histogram the panel wants now, with the space and source it counts.
fn committed_key(app: &PhotocraftApp) -> Option<(HistKey, ToneSpace, HistSource)> {
    let st = app.session.active()?;
    let (doc, revision) = (st.doc.id, st.revision);
    let space = tone_space(st.doc.mode);
    let (_, source) = resolve_source(st, source_choice(&app.ui.histogram.settings));
    let max_level = if app.hist_uncached == Some((doc, revision)) { 1 } else { max_cache_level(app) };
    // The panel counts the selected pixels (HistScope::Selection), so its key carries the
    // selection's identity: a selection change makes every cached histogram stale. The mask is
    // hashed per revision (refreshed by `update`); hashing here is the between-frames cold path.
    let sel = match app.hist_sel_key {
        Some((d, r, k)) if d == doc && r == revision => k,
        _ => tone::selection_key(&st.doc),
    };
    let source_tag = tone::tag(source, space, HistScope::Selection, sel);
    Some((HistKey { doc, revision, source: source_tag, preview: 0, max_level }, space, source))
}

/// The document an open adjustment previews on the canvas (an Image › Adjustments dialog through
/// its preview layer, or an adjustment layer's Properties drag) and its display key.
fn adjustment_preview(app: &mut PhotocraftApp) -> Option<(Arc<Document>, u64)> {
    let idx = app.session.active_index()?;
    if let Some(shown) = crate::adjust_preview::display_doc(app, idx) {
        return Some(shown);
    }
    app.live_adjust.as_ref()?;
    let (doc, key) = crate::canvas::display_doc(app, idx);
    (key != 0).then_some((doc, key))
}

/// What the panel wants shown now: the document's histogram key, and the preview's document and
/// key while an adjustment previews.
struct Want {
    committed: HistKey,
    preview: Option<(Arc<Document>, HistKey)>,
    space: ToneSpace,
    source: HistSource,
}

fn want(app: &mut PhotocraftApp) -> Option<Want> {
    let preview = adjustment_preview(app);
    let (committed, space, source) = committed_key(app)?;
    let preview = preview.map(|(d, k)| (d, HistKey { preview: k | 1, ..committed }));
    Some(Want { committed, preview, space, source })
}

/// Refreshes the memoized selection key for the active document's revision: the panel's keying
/// asks for it every frame, the hash walks every tile of the selection mask, and any change to
/// the selection arrives as a new document revision — so the same revision always keeps its key.
fn refresh_sel_key(app: &mut PhotocraftApp) {
    let Some(st) = app.session.active() else { return };
    if matches!(app.hist_sel_key, Some((d, r, _)) if d == st.doc.id && r == st.revision) {
        return;
    }
    app.hist_sel_key = Some((st.doc.id, st.revision, tone::selection_key(&st.doc)));
}

/// Brings the panel's histograms up to date as far as one frame should: lands a finished count,
/// keeps entries a view-only revision didn't invalidate, and starts the next count (at most one at
/// a time, at most every 250 ms per entry). While a count runs, the panel asks the framework to run
/// it again in 30 ms to land the result: a worker thread cannot wake one settled region, and the
/// panel's version holds no worker state.
pub fn update(app: &mut PhotocraftApp, ctx: &egui::Context) {
    refresh_sel_key(app);
    let want = want(app);
    // A finished worker lands its histogram in the slot its key names — unless it was computed
    // for another document or selection, which is dropped at the door: caching it would reset
    // the panel's 250 ms throttle without ever being shown fresh. A result for the same
    // document and selection still lands when the revision moved on under it (continuous
    // edits): the panel shows the newer data, marked stale, instead of waiting for an edit-free
    // moment. A worker that died without sending clears the job so the next frame schedules a
    // new one.
    if let Some((_, rx)) = &app.hist_job {
        match rx.try_recv() {
            Ok((key, ms, hist)) => {
                app.perf.span("histogram", ms);
                let wanted = want.as_ref().is_some_and(|w| key.doc == w.committed.doc && key.source == w.committed.source);
                if wanted {
                    // egui's clock works on every target; `gpu_canvas::now_ms` is always 0 on wasm32.
                    let entry = PanelHist { key, at: ctx.input(|i| i.time) * 1000.0, hist };
                    if key.preview == 0 {
                        app.doc_hist = Some(entry);
                    } else {
                        app.doc_hist_preview = Some(entry);
                    }
                }
                app.hist_job = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => app.hist_job = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
    // Revisions that changed no pixels since the histogram's (selecting a layer, say) keep it
    // (#125), and an Uncached Refresh with it — but only for the same selection: a selection
    // change moves no pixels yet changes what the panel counts.
    if let Some(st) = app.session.active() {
        let (doc, rev) = (st.doc.id, st.revision);
        if let Some((d, r)) = app.hist_uncached
            && d == doc
            && tone::pixels_kept_since(st, r)
        {
            app.hist_uncached = Some((doc, rev));
        }
        let same_selection = want.as_ref().map(|w| w.committed.source);
        for e in [app.doc_hist.as_mut(), app.doc_hist_preview.as_mut()].into_iter().flatten() {
            if e.key.doc == doc && tone::pixels_kept_since(st, e.key.revision) && same_selection.is_none_or(|s| e.key.source == s) {
                e.key.revision = rev;
            }
        }
    }
    let Some(want) = want else { return };
    if want.preview.is_none() {
        app.doc_hist_preview = None;
    }
    if app.hist_job.is_some() {
        poll(ctx);
        return;
    }
    let now = ctx.input(|i| i.time) * 1000.0;
    let due = |e: &Option<PanelHist>| e.as_ref().is_none_or(|e| e.key.doc != want.committed.doc || now - e.at > 250.0);
    let stale = |e: &Option<PanelHist>, k: &HistKey| e.as_ref().is_none_or(|e| e.key != *k);
    // An Uncached Refresh is asked for, so it doesn't wait out the throttle (Cache Levels 1 alone
    // counts at level 1 too, and still waits).
    let forced = app.hist_uncached == Some((want.committed.doc, want.committed.revision));
    let job = if stale(&app.doc_hist, &want.committed) {
        (forced || due(&app.doc_hist)).then(|| (app.session.active().map(|st| st.doc.clone()), want.committed))
    } else if let Some((doc, key)) = &want.preview
        && stale(&app.doc_hist_preview, key)
    {
        due(&app.doc_hist_preview).then(|| (Some(doc.clone()), *key))
    } else {
        None
    };
    match job {
        Some((Some(doc), key)) => {
            spawn(app, doc, key, want.source, want.space);
            poll(ctx);
        }
        Some((None, _)) => {}
        None if stale(&app.doc_hist, &want.committed) || want.preview.as_ref().is_some_and(|(_, k)| stale(&app.doc_hist_preview, k)) => {
            craft::ui::runtime::frame::request_repaint_after(ctx, 0.26);
        }
        None => {}
    }
}

/// Runs the panel again in 30 ms to land the running count.
fn poll(ctx: &egui::Context) {
    craft::ui::runtime::frame::request_repaint_after(ctx, 0.03);
}

/// Counts `doc` for `key` on a worker thread (inline on the web, where there are none, or when a
/// spawn fails), delivered through `app.hist_job`.
fn spawn(app: &mut PhotocraftApp, doc: Arc<Document>, key: HistKey, source: HistSource, space: ToneSpace) {
    let (tx, rx) = std::sync::mpsc::channel();
    #[cfg(not(target_arch = "wasm32"))]
    let spawned = {
        let doc = doc.clone();
        let tx = tx.clone();
        std::thread::Builder::new()
            .name("histogram".into())
            .spawn(move || {
                let t0 = crate::gpu_canvas::now_ms();
                let h = Arc::new(tone::compute_histograms_at(&doc, source, space, HistScope::Selection, key.max_level));
                let _ = tx.send((key, crate::gpu_canvas::now_ms() - t0, h));
            })
            .is_ok()
    };
    #[cfg(target_arch = "wasm32")]
    let spawned = false;
    if !spawned {
        let t0 = crate::gpu_canvas::now_ms();
        let h = Arc::new(tone::compute_histograms_at(&doc, source, space, HistScope::Selection, key.max_level));
        let _ = tx.send((key, crate::gpu_canvas::now_ms() - t0, h));
    }
    app.hist_job = Some((key, rx));
}

/// Uncached Refresh: recount the active document's current state at cache level 1 (every pixel).
pub fn uncached_refresh(app: &mut PhotocraftApp) {
    if let Some(st) = app.session.active() {
        app.hist_uncached = Some((st.doc.id, st.revision));
    }
}

/// The histogram the panel shows now: the preview's while an adjustment previews (with the
/// document's as the original), else the document's. `stale` when it isn't the one wanted yet.
struct Shown {
    hist: Arc<Histograms>,
    original: Option<Arc<Histograms>>,
    stale: bool,
    mode: ColorMode,
    layers: usize,
    adjustment: bool,
}

fn shown(app: &mut PhotocraftApp) -> Option<Shown> {
    let want = want(app)?;
    let st = app.session.active()?;
    let (mode, layers, adjustment) = (st.doc.mode, st.doc.layer_count(), adjustment_selected(st));
    let same_doc = |e: &PanelHist| e.key.doc == want.committed.doc;
    let committed = app.doc_hist.as_ref().filter(|e| same_doc(e));
    let preview = app.doc_hist_preview.as_ref().filter(|e| same_doc(e) && want.preview.is_some());
    Some(match (preview, committed) {
        (Some(p), c) => Shown {
            hist: p.hist.clone(),
            original: c.map(|c| c.hist.clone()),
            stale: want.preview.as_ref().is_none_or(|(_, k)| *k != p.key),
            mode,
            layers,
            adjustment,
        },
        (None, Some(c)) => Shown { hist: c.hist.clone(), original: None, stale: c.key != want.committed || want.preview.is_some(), mode, layers, adjustment },
        (None, None) => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// The panel

/// The scope panel's channels for `h`: the mode's channel list without the Colors overlay entry (the set's `colors`
/// offers it), each in its colour.
fn scope_channels(h: &Histograms, mode: ColorMode) -> Vec<ScopeChannel<'_>> {
    let colors = color_channels(mode);
    channel_options(mode)
        .into_iter()
        .filter(|(c, _)| *c != HistChannel::Colors)
        .map(|(c, label)| ScopeChannel::new(c.key(), tl!(label), c.ink(mode), bins(h, mode, c).as_slice(), colors.contains(&c)))
        .collect()
}

/// The Color Samplers' ticks on the plot: each sampler's level in `channel` (the composite while Colors shows).
fn sampler_markers(app: &mut PhotocraftApp, mode: ColorMode, channel: HistChannel) -> Vec<(f32, Label)> {
    let mark_channel = if channel == HistChannel::Colors { HistChannel::Composite } else { channel };
    crate::panels::sampler_readouts(app)
        .iter()
        .filter_map(|v| {
            let rgb = v["rgb"].as_array()?.iter().filter_map(Value::as_f64).map(|c| (c / 255.0) as f32).collect::<Vec<_>>();
            let (&red, &green, &blue) = (rgb.first()?, rgb.get(1)?, rgb.get(2)?);
            Some((f32::from(channel_level(mode, mark_channel, [red, green, blue, 1.0])), Label::from(v["number"].to_string())))
        })
        .collect()
}

/// Histogram panel (Window › Histogram). A second pass in the same frame draws the same counts and reports only its
/// own input: `update` lands a finished count once (the job is gone on the second pass) and the output flags come from
/// the scope panel's pass.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    if app.session.active().is_none() {
        ui.label(egui::RichText::new(tl!("No document")).color(t.text_faint));
        return;
    }
    update(app, ui.ctx());
    let Some(s) = shown(app) else {
        ui.label(egui::RichText::new(tl!("Calculating…")).color(t.text_faint));
        return;
    };
    let channel = resolve_channel(s.mode, HistChannel::from_key(&app.ui.histogram.settings.channel));
    let markers = sampler_markers(app, s.mode, channel);
    let has_selection = app.session.active().is_some_and(|st| st.doc.selection.is_some());

    let channels = scope_channels(&s.hist, s.mode);
    let original = s.original.as_ref().map(|h| scope_channels(h, s.mode));
    let mut set = HistogramSet::new(&channels).original(original.as_deref()).pixels(s.hist.pixel_count).stale(s.stale).cache_level(s.hist.level);
    if channel_options(s.mode).iter().any(|(c, _)| *c == HistChannel::Colors) {
        set = set.colors(if tone_space(s.mode) == ToneSpace::Cmyk { ChannelOverlay::Ink } else { ChannelOverlay::Light });
    }
    let mut sources = Vec::new();
    if s.layers > 1 {
        sources.push(ScopeSource::new(HistSourceChoice::EntireImage.key(), tl!("Entire Image")));
        sources.push(ScopeSource::new(HistSourceChoice::SelectedLayer.key(), tl!("Selected Layer")));
        if s.adjustment {
            sources.push(ScopeSource::new(HistSourceChoice::AdjustmentComposite.key(), tl!("Adjustment Composite")));
        }
    }
    let frame = ScopeFrame::histogram(set);
    let out = ScopePanel::new(&mut app.ui.histogram).frame(&frame).sources(&sources).markers(&markers).has_selection(has_selection).show(ui);
    if out.settings_changed {
        persist_options(app);
    }
    // The next `update` recounts for a changed source or channel from the settings.
    if out.refresh {
        uncached_refresh(app);
    }
}

/// The rows of the panel menu: Uncached Refresh, the three views and the two checks.
pub(crate) fn panel_rows(_app: &PhotocraftApp) -> Vec<PanelRow> {
    ScopePanel::panel_rows()
}

/// The state of every scope command: the radio and check marks from the settings; Uncached Refresh needs a document.
pub(crate) fn command_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    let active = app.session.active();
    let has_selection = active.is_some_and(|st| st.doc.selection.is_some());
    ScopeCommand::states(&app.ui.histogram, has_selection)
        .into_iter()
        .map(|(command, state)| {
            let state = if command == ScopeCommand::Refresh { state.enabled(active.is_some()) } else { state };
            (CommandRef::from(command), state)
        })
        .collect()
}

/// Runs a scope command the menu or a key fired: the settings change, then they persist; Uncached Refresh recounts.
/// Returns whether the command belongs to this panel.
pub(crate) fn run_fired(app: &mut PhotocraftApp, _ctx: &egui::Context, command: &CommandRef) -> bool {
    let Some(command) = ScopeCommand::from_ref(command) else { return false };
    match ScopePanel::run_command(&mut app.ui.histogram, command) {
        Some(ScopeMenuAction::Refresh) => uncached_refresh(app),
        Some(ScopeMenuAction::SettingsChanged) => persist_options(app),
        _ => {}
    }
    true
}

/// The statistics `ui.inspect` reports for `bins`, inspecting `levels`. `pixels` is the count the Pixels row reports:
/// every pixel the count kept, transparent ones included, so with a selection it is the selection's size, while the
/// moments and ratios come from the bins, which hold only the visible pixels. `None` when nothing was counted.
fn statistics_json(bins: &[u32], levels: Option<[u8; 2]>, pixels: u64) -> Option<Value> {
    let visible = histogram_statistics(bins, levels);
    if visible.is_none() && pixels == 0 {
        return None;
    }
    // A selection of only transparent pixels has a count but no visible one: every moment is 0.
    let sorted = levels.map(|[a, b]| [a.min(b), a.max(b)]);
    Some(match visible {
        Some(s) => json!({
            "mean": s.mean,
            "stdDev": s.standard_deviation,
            "median": s.median,
            "pixels": pixels,
            "level": s.level,
            "count": s.count,
            "percentile": s.percentile,
            "clipped": s.clipped,
        }),
        None => json!({
            "mean": 0.0,
            "stdDev": 0.0,
            "median": 0,
            "pixels": pixels,
            "level": sorted,
            "count": sorted.map(|_| 0),
            "percentile": sorted.map(|_| 0.0),
            "clipped": [0.0, 0.0],
        }),
    })
}

/// The panel's state for `ui.inspect` (`histogram`): its options as they apply to the active
/// document, and the shown histogram's statistics for the selected range.
pub fn inspect(app: &PhotocraftApp) -> Value {
    let opts = &app.ui.histogram.settings;
    let Some(st) = app.session.active() else { return json!({"options": opts}) };
    let mode = st.doc.mode;
    let channel = resolve_channel(mode, HistChannel::from_key(&opts.channel));
    let (source, _) = resolve_source(st, source_choice(opts));
    let preview = app.doc_hist_preview.as_ref().filter(|e| e.key.doc == st.doc.id);
    let entry = preview.or(app.doc_hist.as_ref()).filter(|e| e.key.doc == st.doc.id);
    let committed = committed_key(app).map(|(k, _, _)| k);
    let ch = if channel == HistChannel::Colors { HistChannel::Composite } else { channel };
    let range = range_levels(app.ui.histogram.range);
    json!({
        "options": opts,
        "channel": channel.key(),
        "channels": channel_options(mode).iter().map(|(c, _)| c.key()).collect::<Vec<_>>(),
        "source": source.key(),
        "sourceShown": st.doc.layer_count() > 1 && opts.view != ScopeView::Compact,
        "range": range,
        "preview": preview.is_some(),
        "revision": entry.map(|e| e.key.revision),
        "current": entry.zip(committed).is_some_and(|(e, k)| e.key == HistKey { preview: e.key.preview, ..k }),
        "cacheLevel": entry.map(|e| e.hist.level.max(1)),
        "maxCacheLevel": max_cache_level(app),
        "stats": entry.and_then(|e| statistics_json(bins(&e.hist, mode, ch), range, e.hist.pixel_count)),
    })
}

#[cfg(test)]
mod tests;
