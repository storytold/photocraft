use photocraft_engine::color_cmds::ColorExt;
use std::sync::Arc;

use craft::behavior::PixelSize;
use craft::imaging::ui::raw_develop::{RawDevelopOpen, RawDevelopSource, RawDevelopWorkspace, RawFilmItem, RawTarget};
use craft::ui::dialog::HostedWorkspace;
use photocraft_engine::commands::raw_auto_cmds::Proxy;

use super::open::{Coverage, RawOpen, Target};
use super::{CameraRaw, LayerId, PcRawSource, PhotocraftApp, RawSession, SourceInit, hosted_mut, to_framework};
use serde_json::json;

/// Camera Raw Defaults › Sharpen For: output sharpening, folded into the Camera Raw step `rest`
/// as the workflow's Sharpen For is. Returns what it applied.
pub(super) fn output_sharpening(d: &photocraft_engine::prefs::RawDefaults, rest: &mut CameraRaw) -> Vec<&'static str> {
    match d.sharpen_for.preset() {
        Some((amount, radius)) if rest.sharpen_amount == 0.0 => {
            rest.sharpen_amount = amount;
            rest.sharpen_radius = radius;
            vec!["sharpenFor"]
        }
        _ => Vec::new(),
    }
}

/// The Camera Raw Defaults [`apply_raw_conversions`] applies. Resolution is not among them: the
/// open itself sets it (`file_cmds::raw_defaults`), and a re-develop keeps it.
pub(super) fn raw_conversions(d: &photocraft_engine::prefs::RawDefaults) -> Vec<&'static str> {
    use photocraft_engine::prefs::{RawColorSpace, RawDepth};
    let mut out = Vec::new();
    // Raws develop into ProPhoto-compatible RGB (`photocraft_io::raw`); the other spaces are a
    // conversion.
    if d.color_space != RawColorSpace::ProPhoto {
        out.push("colorSpace");
    }
    if d.bit_depth == RawDepth::Eight {
        out.push("bitDepth");
    }
    if d.open_as_smart_object {
        out.push("openAsSmartObject");
    }
    out
}

/// Camera Raw Defaults (Preferences) after the Camera Raw step: the colour space and bit depth
/// the developed document opens in, and Open as Smart Object.
pub(super) fn apply_raw_conversions(app: &mut PhotocraftApp, index: usize, d: &photocraft_engine::prefs::RawDefaults) -> Result<(), String> {
    app.session.set_active(index);
    for step in raw_conversions(d) {
        let (id, params) = match step {
            "colorSpace" => ("edit.convertToProfile", json!({"profile": d.color_space.profile_id(), "intent": "relative"})),
            "bitDepth" => ("image.mode.bits8", json!({})),
            _ => ("layer.smartObjects.convertToSmartObject", json!({})),
        };
        app.session.execute(id, params).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A raw being opened: what the open-time dialog starts from.
pub(super) struct RawStart {
    pub(super) raw: RawOpen,
    /// The settings Reset restores when they differ from the ones shown (a filmstrip raw shown again).
    pub(super) original: Option<CameraRaw>,
    /// Camera Raw Defaults › Apply auto tone: the shown settings start from Auto.
    pub(super) auto_tone: bool,
}

/// The camera tone curve a raw was developed with: Shadows / Highlights act on the raw data in linear light before it, and
/// the full-size preview retones the whole frame with it, as Open will.
fn tone_curve(raw: &RawOpen) -> Option<Arc<Vec<[f32; 2]>>> {
    raw.decoded.as_ref().map(|d| Arc::new(d.tone_curve().to_vec()))
}

/// The filmstrip tiles of the open-time dialog (one per queued raw) and the one shown.
fn film_items(app: &PhotocraftApp, raw: &RawOpen) -> (Vec<RawFilmItem>, usize) {
    let mut items: Vec<RawFilmItem> = app.raw_shelf.iter().map(|h| RawFilmItem::new(h.raw.name.clone())).collect();
    let current = match app.raw_shelf.iter().position(|h| h.raw.document == raw.document) {
        Some(i) => i,
        None => {
            items.push(RawFilmItem::new(raw.name.clone()));
            items.len() - 1
        }
    };
    (items, current)
}

/// Opens RAW Develop as a filter (`target` is [`Target::NewFilter`] or [`Target::SmartFilter`]) on `surf`.
pub(super) fn open_pixels(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    layer: LayerId,
    surf: photocraft_raster::Surface,
    params: CameraRaw,
    coverage: Coverage,
    target: Target,
) -> Result<(), String> {
    open_session(app, ctx, layer, surf, params, coverage, target, None)
}

/// Opens, or on a filmstrip pick shows in the open workspace, the open-time dialog of `start.raw` on `surf`.
pub(super) fn open_raw_pixels(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    layer: LayerId,
    surf: photocraft_raster::Surface,
    params: CameraRaw,
    start: RawStart,
) -> Result<(), String> {
    open_session(app, ctx, layer, surf, params, Coverage::None, Target::OpenRaw, Some(start))
}

#[allow(clippy::too_many_arguments)]
fn open_session(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    layer: LayerId,
    surf: photocraft_raster::Surface,
    params: CameraRaw,
    coverage: Coverage,
    target: Target,
    raw: Option<RawStart>,
) -> Result<(), String> {
    // One RAW Develop at a time; the open-time dialog shows another of its raws in place.
    let showing_another = raw.is_some() && app.raw.as_ref().is_some_and(|s| s.target == Target::OpenRaw) && hosted_mut(app).is_some();
    if hosted_mut(app).is_some() && !showing_another {
        return Err(tl!("RAW Develop is already open").to_string());
    }
    let st = app.session.active().ok_or("no document")?;
    let canvas = st.doc.bounds();
    let name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    // The engine derives the same value for the command (`lens_cmds::camera_raw_cmd`).
    let encoding_gamma = photocraft_engine::lens_cmds::encoding_gamma(&st.doc);
    let proxy = Proxy::new(&surf, canvas)?;
    let selection = match &coverage {
        Coverage::Selection => st.doc.selection.as_ref().map(|mask| proxy.sample_coverage(|x, y| mask.sample_channel(x, y, 0))),
        Coverage::Mask(mask) => Some(proxy.sample_coverage(|x, y| mask.value(x, y))),
        Coverage::None => None,
    };
    let shadow_noise = proxy.shadow_noise(&surf, encoding_gamma, selection.as_deref());
    let Proxy { pixels, width: pw, height: ph, full_width: w, full_height: h, block: k, area } = proxy;
    let source = photocraft_engine::color_cmds::composite_profile(&st.doc);
    let display = app.session.color().canvas_display(&st.doc).map_err(|e| e.to_string())?;
    let lab_transform = craft::cms::cached(&source, craft::cms::Builtin::LabD50.profile(), Default::default()).map_err(|e| e.to_string())?;
    let scope_transform = craft::cms::cached(&source, craft::cms::Builtin::Srgb.profile(), Default::default()).map_err(|e| e.to_string())?;
    let float = surf.format().sample == photocraft_color::SampleType::F32;
    // Builds the GPU pass the first time any Camera Raw dialog opens.
    let gpu = crate::camera_raw_gpu::for_app(app);
    let detail = crate::dialogs::camera_raw_detail_ui::DetailPreview::new(
        surf,
        area,
        match coverage {
            Coverage::Selection => st
                .doc
                .selection
                .clone()
                .map(crate::dialogs::camera_raw_detail_ui::Coverage::Selection)
                .unwrap_or(crate::dialogs::camera_raw_detail_ui::Coverage::None),
            Coverage::Mask(mask) => crate::dialogs::camera_raw_detail_ui::Coverage::Mask(mask),
            Coverage::None => crate::dialogs::camera_raw_detail_ui::Coverage::None,
        },
    );
    let navigation = crate::canvas::document_display::navigation(app);
    let checkerboard = crate::canvas::document_display::checkerboard(app);
    let auto_history = raw
        .as_ref()
        .and_then(|start| app.raw_shelf.iter().find(|held| held.raw.document == start.raw.document))
        .map(|held| held.auto_history.clone())
        .unwrap_or_default();
    let source = Arc::new(PcRawSource::new(SourceInit {
        proxy: pixels,
        selection,
        proxy_size: (pw, ph),
        full_size: (w, h),
        block: k,
        float,
        display,
        gpu: gpu.clone(),
        encoding_gamma,
        shadow_noise,
        auto_history,
        lab_transform,
        scope_transform,
        detail,
        ctx: ctx.clone(),
    }));
    let original = raw.as_ref().and_then(|r| r.original.clone()).unwrap_or_else(|| params.clone());
    let mut settings = params;
    let mut raw_open = None;
    let mut subject = name.clone();
    let mut demosaic = None;
    let mut film = None;
    if let Some(start) = raw {
        if start.auto_tone {
            photocraft_algo::camera_raw::auto_tone(&source.proxy, &mut settings);
        }
        source.set_raw_curve(tone_curve(&start.raw));
        subject = start.raw.name.clone();
        demosaic = Some(to_framework(start.raw.demosaic));
        film = Some(film_items(app, &start.raw));
        raw_open = Some(start.raw);
    }
    let raw_target = match target {
        Target::NewFilter => RawTarget::Filter,
        Target::SmartFilter(_) => RawTarget::SmartFilter,
        Target::OpenRaw => RawTarget::OpenRaw,
    };
    let dyn_source: Arc<dyn RawDevelopSource> = source.clone();
    if showing_another {
        let (_, current) = film.unwrap_or_default();
        if let Some(hosted) = hosted_mut(app) {
            hosted.set_source(dyn_source);
            hosted.workspace_mut().show_item(current, settings, original.clone(), demosaic);
        }
    } else {
        let mut open = RawDevelopOpen::new(subject, PixelSize::new(w as u32, h as u32), raw_target, settings).original(original.clone()).navigation(navigation);
        if let Some(checker) = checkerboard {
            open = open.checkerboard(checker);
        }
        if let Some(demosaic) = demosaic {
            open = open.demosaic(demosaic);
        }
        if let Some((items, current)) = film {
            open = open.filmstrip(items, current);
        }
        // OK is held: a failed filter command keeps the workspace open with its settings.
        app.framework.dialogs.open(HostedWorkspace::new(RawDevelopWorkspace::new(open), dyn_source).held_ok());
    }
    app.raw = Some(RawSession {
        layer,
        target,
        raw_open,
        original,
        source,
        #[cfg(test)]
        gpu,
    });
    if target == Target::OpenRaw {
        super::filmstrip::sync_pictures(app, ctx);
    }
    Ok(())
}
