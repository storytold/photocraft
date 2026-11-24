use photocraft_engine::color_cmds::ColorExt;
use std::sync::Arc;

use craft::behavior::PixelSize;
use craft::imaging::ui::raw_develop::{RawDevelopOpen, RawDevelopSource, RawDevelopWorkspace, RawFilmItem, RawTarget};
use craft::ui::dialog::HostedWorkspace;

use super::open::{Coverage, RawOpen, Target};
use super::{CameraRaw, LayerId, PROXY_SIDE, PcRawSource, PhotocraftApp, RawSession, Rect, SourceInit, hosted_mut, to_framework};
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

/// `area` of `surf` averaged over `k`×`k` blocks, row-major (`ceil(w/k)` × `ceil(h/k)`). A block
/// past the last row repeats that row, as a clamped edge; each source row is still read once,
/// weighted by the block rows it stands for (a one-row domain read the same row `k` times, which
/// is quadratic in its width).
fn box_average(surf: &photocraft_raster::Surface, area: Rect, k: usize) -> Vec<[f32; 4]> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let (pw, ph) = (w.div_ceil(k), h.div_ceil(k));
    let mut proxy = vec![[0.0f32; 4]; pw * ph];
    let mut row = vec![[0.0f32; 4]; w];
    for py in 0..ph {
        // f64: a weighted f32 sample near f32::MAX would overflow to infinity.
        let mut acc = vec![[0.0f64; 5]; pw];
        for sy in py * k..((py + 1) * k).min(h) {
            let weight = if sy == h - 1 { ((py + 1) * k - sy) as f64 } else { 1.0 };
            let y = area.y0 + sy as i32;
            surf.read_rgba_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
            for (x, q) in row.iter().enumerate() {
                let a = &mut acc[x / k];
                for c in 0..4 {
                    a[c] += f64::from(q[c]) * weight;
                }
                a[4] += weight;
            }
        }
        for (x, a) in acc.iter().enumerate() {
            proxy[py * pw + x] = [0, 1, 2, 3].map(|c| (a[c] / a[4]) as f32);
        }
    }
    proxy
}

/// What the preview develops, and its size: the engine filter domain, including pixels outside the
/// canvas, since spatial Camera Raw stages (vignette, grain, local contrast) must agree with the
/// committed result.
pub(super) fn preview_area(canvas: Rect, surf: &photocraft_raster::Surface) -> Result<(Rect, usize, usize), String> {
    let area = canvas.union(&surf.content_bounds());
    let width = area.x1.checked_sub(area.x0).filter(|n| (1..=1_048_576).contains(n));
    let height = area.y1.checked_sub(area.y0).filter(|n| (1..=1_048_576).contains(n));
    let (Some(w), Some(h)) = (width, height) else {
        return Err("RAW Develop preview bounds are too large".into());
    };
    let (w, h) = (w as usize, h as usize);
    if w.checked_mul(h).is_none_or(|pixels| pixels > 512_000_000) {
        return Err("RAW Develop preview bounds are too large".into());
    }
    Ok((area, w, h))
}

/// `area` of `surf` (`w`×`h`) box-averaged down to at most [`PROXY_SIDE`] pixels on its long side:
/// the proxy, its size, and the side of the blocks averaged.
pub(super) fn box_proxy(surf: &photocraft_raster::Surface, area: Rect, w: usize, h: usize) -> (Vec<[f32; 4]>, usize, usize, usize) {
    let k = w.max(h).div_ceil(PROXY_SIDE).max(1);
    let (pw, ph) = (w.div_ceil(k), h.div_ceil(k));
    let proxy = box_average(surf, area, k);
    (proxy, pw, ph, k)
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
    let (area, w, h) = preview_area(canvas, &surf)?;
    let (proxy, pw, ph, k) = box_proxy(&surf, area, w, h);
    // Coverage at the centre of each proxy block.
    let sample = |value: &dyn Fn(i32, i32) -> f32| {
        (0..ph)
            .flat_map(|y| (0..pw).map(move |x| (area.x0 + (x * k + k / 2).min(w - 1) as i32, area.y0 + (y * k + k / 2).min(h - 1) as i32)))
            .map(|(x, y)| value(x, y).clamp(0.0, 1.0))
            .collect::<Vec<_>>()
    };
    let selection = match &coverage {
        Coverage::Selection => st.doc.selection.as_ref().map(|mask| sample(&|x, y| mask.sample_channel(x, y, 0))),
        Coverage::Mask(mask) => Some(sample(&|x, y| mask.value(x, y))),
        Coverage::None => None,
    };
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
    let source = Arc::new(PcRawSource::new(SourceInit {
        proxy,
        selection,
        proxy_size: (pw, ph),
        full_size: (w, h),
        block: k,
        float,
        display,
        gpu: gpu.clone(),
        encoding_gamma,
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

#[cfg(test)]
mod box_average_tests {
    use super::{Rect, box_average};
    use photocraft_color::PixelFormat;
    use photocraft_raster::Surface;

    /// The clamped reference: every block row read, past the last row the last row again.
    fn reference(surf: &Surface, area: Rect, k: usize) -> Vec<[f32; 4]> {
        let (w, h) = (area.width() as usize, area.height() as usize);
        let (pw, ph) = (w.div_ceil(k), h.div_ceil(k));
        let mut out = Vec::new();
        let mut row = vec![[0.0f32; 4]; w];
        for py in 0..ph {
            let mut acc = vec![[0.0f32; 5]; pw];
            for dy in 0..k {
                let y = area.y0 + (py * k + dy).min(h - 1) as i32;
                surf.read_rgba_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
                for (x, q) in row.iter().enumerate() {
                    for c in 0..4 {
                        acc[x / k][c] += q[c];
                    }
                    acc[x / k][4] += 1.0;
                }
            }
            out.extend(acc.iter().map(|a| [a[0] / a[4], a[1] / a[4], a[2] / a[4], a[3] / a[4]]));
        }
        out
    }

    #[test]
    fn blocks_past_the_last_row_match_the_clamped_average() {
        let mut surf = Surface::new(PixelFormat::RGBA32F);
        for x in 0..37 {
            for y in 0..5 {
                surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &[x as f32 / 37.0, y as f32 / 5.0, 0.5, 1.0]);
            }
        }
        for (area, k) in [(Rect::new(0, 0, 37, 5), 2), (Rect::new(0, 0, 37, 5), 4), (Rect::new(0, 2, 37, 3), 16), (Rect::new(3, 0, 37, 5), 7)] {
            let (got, want) = (box_average(&surf, area, k), reference(&surf, area, k));
            assert_eq!(got.len(), want.len());
            for (g, w) in got.iter().zip(&want) {
                assert!(g.iter().zip(w).all(|(a, b)| (a - b).abs() < 1e-5), "{area:?} k={k}: {g:?} vs {w:?}");
            }
        }
    }

    /// Samples near f32::MAX stay finite: the weighted sums are held in f64.
    #[test]
    fn huge_samples_do_not_overflow() {
        let mut surf = Surface::new(PixelFormat::RGBA32F);
        let big = f32::MAX / 2.0;
        surf.fill_rect(Rect::new(0, 0, 2, 1), &[big, -big, big, 1.0]);
        let got = box_average(&surf, Rect::new(0, 0, 2, 1), 4);
        assert!(got.iter().flatten().all(|v| v.is_finite()), "{got:?}");
    }
}
