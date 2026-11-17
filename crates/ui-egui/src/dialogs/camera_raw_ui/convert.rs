use photocraft_engine::commands::raw_auto_cmds::Proxy;

use super::open::{Coverage, Target};
use super::{CameraRaw, CameraRawDialog, LayerId, MAX_SAMPLERS, PhotocraftApp, RgbHistogram, Value, json};

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

pub(super) fn open_pixels(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    layer: LayerId,
    surf: photocraft_raster::Surface,
    params: CameraRaw,
    coverage: Coverage,
    target: Target,
) -> Result<(), String> {
    let st = app.session.active().ok_or("no document")?;
    let canvas = st.doc.bounds();
    let name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    let mut params = params;
    if params.encoding_gamma == 0.0 {
        params.encoding_gamma = photocraft_engine::lens_cmds::encoding_gamma(&st.doc);
    }
    let proxy = Proxy::new(&surf, canvas)?;
    let selection = match &coverage {
        Coverage::Selection => st.doc.selection.as_ref().map(|mask| proxy.sample_coverage(|x, y| mask.sample_channel(x, y, 0))),
        Coverage::Mask(mask) => Some(proxy.sample_coverage(|x, y| mask.value(x, y))),
        Coverage::None => None,
    };
    let shadow_noise = proxy.shadow_noise(&surf, params.encoding_gamma, selection.as_deref());
    let Proxy { pixels: proxy, width: pw, height: ph, full_width: w, full_height: h, area, .. } = proxy;
    let source = photocraft_engine::color_cmds::composite_profile(&st.doc);
    let display = app.session.color.canvas_display(&st.doc).map_err(|e| e.to_string())?;
    let lab_transform = craft_cms::cached(&source, craft_cms::Builtin::LabD50.profile(), Default::default()).map_err(|e| e.to_string())?;
    let scope_transform = craft_cms::cached(&source, craft_cms::Builtin::Srgb.profile(), Default::default()).map_err(|e| e.to_string())?;
    if let Some(saved) = app.session.prefs().dialogs.get("filter.cameraRaw.scope")
        && saved.get("samplers").and_then(Value::as_array).is_none_or(|a| a.len() <= MAX_SAMPLERS)
        && let Ok(view) = serde_json::from_value(saved.clone())
    {
        app.ui.camera_raw_scope = view;
    }
    app.ui.camera_raw_preview = Default::default();
    app.ui.camera_raw_scope.samplers.clear();
    // Keep Camera Raw compact on open; vectorscope is an opt-in context-menu view.
    app.ui.camera_raw_scope.vectorscope = false;
    if selection.is_none() {
        app.ui.camera_raw_scope.selected_region = false;
    }
    let float = surf.format().sample == photocraft_color::SampleType::F32;
    // Builds the GPU pass the first time any Camera Raw dialog opens.
    let gpu = crate::camera_raw_gpu::for_app(app);
    let mut d = CameraRawDialog {
        layer,
        layer_name: name,
        original: params.clone(),
        params,
        target,
        before_histogram: RgbHistogram::from_rgba(&proxy),
        processed: Vec::new(),
        lab_transform,
        scope_transform,
        scope: Default::default(),
        after_histogram: RgbHistogram::default(),
        preview_revision: 0,
        histogram_ms: 0.0,
        proxy: std::sync::Arc::new(proxy),
        selection: selection.map(std::sync::Arc::new),
        pw,
        ph,
        full_w: w,
        full_h: h,
        shadow_noise,
        preview_ppp: ctx.pixels_per_point(),
        detail: crate::camera_raw_detail_ui::DetailPreview::new(
            surf,
            area,
            match coverage {
                Coverage::Selection => {
                    st.doc.selection.clone().map(crate::camera_raw_detail_ui::Coverage::Selection).unwrap_or(crate::camera_raw_detail_ui::Coverage::None)
                }
                Coverage::Mask(mask) => crate::camera_raw_detail_ui::Coverage::Mask(mask),
                Coverage::None => crate::camera_raw_detail_ui::Coverage::None,
            },
        ),
        display,
        viewport: None,
        float,
        tex: None,
        before_tex: None,
        before_display_key: None,
        dirty: true,
        show_before: false,
        mixer_tab: 1,
        curve_state: Default::default(),
        curve_rect: None,
        render_ms: 0.0,
        raw_open: None,
        raw_curve: None,
        pending: None,
        preview_error: None,
        gpu,
        source_key: crate::camera_raw_gpu::source_key(),
        develop_ms: 0.0,
        develop_backend: "cpu",
        undo: Vec::new(),
        redo: Vec::new(),
        edit_origin: None,
        auto: Default::default(),
        main_focused: false,
    };
    d.render(ctx);
    app.camera_raw = Some(d);
    Ok(())
}
