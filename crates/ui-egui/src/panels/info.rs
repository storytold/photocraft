use photocraft_color::ColorMode;
use photocraft_engine::sampler_cmds::{ChannelReadout, Readouts};

use super::{PhotocraftApp, RichText, Tokens, Value, empty, json, theme, widgets};
use crate::info_panel_options::Readout;

/// The active document's pixel under the pointer, if the pointer is over the image.
pub(crate) fn hover_pixel(app: &PhotocraftApp) -> Option<(i32, i32)> {
    let size = app.session.active()?.doc.size;
    app.hover_doc.map(|p| (p[0].floor() as i32, p[1].floor() as i32)).filter(|(x, y)| *x >= 0 && *y >= 0 && *x < size.width as i32 && *y < size.height as i32)
}

/// The colour the Info panel reads at document pixel `(x, y)`, straight RGBA 0..1: averaged over
/// the Eyedropper's Sample Size, as in Photoshop (#1649), cached per document activation, pixel and revision.
pub(crate) fn pointer_sample(app: &mut PhotocraftApp, (x, y): (i32, i32)) -> Option<[f32; 4]> {
    let st = app.session.active()?;
    let sample_size = app.ui.tool_options.eyedropper_size;
    let key = (st.doc.id, app.session.active_generation(), x, y, st.revision, sample_size);
    if let Some((cached, v)) = app.info_sample
        && cached == key
    {
        return Some(v);
    }
    let params = json!({"x": f64::from(x) + 0.5, "y": f64::from(y) + 0.5, "size": sample_size});
    let v: Vec<f32> = serde_json::from_value(app.session.execute("document.sampleColor", params).ok()?).ok()?;
    let v = [*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?];
    app.info_sample = Some((key, v));
    Some(v)
}

/// Extra-channel readings under the pointer, keyed by document ID, activation, pixel and revision.
pub(crate) type PointerChannels = ((photocraft_doc::DocId, u64, i32, i32, u64), std::sync::Arc<Vec<ChannelReadout>>);

/// Every alpha and spot channel's reading at document pixel `(x, y)` (one pixel, as Photoshop
/// reads a channel), cached per document activation, pixel and revision.
pub(crate) fn pointer_channels(app: &mut PhotocraftApp, (x, y): (i32, i32)) -> std::sync::Arc<Vec<ChannelReadout>> {
    let Some(key) = app.session.active().map(|s| (s.doc.id, app.session.active_generation(), x, y, s.revision)) else { return std::sync::Arc::default() };
    if let Some((k, v)) = &app.info_channels
        && *k == key
    {
        return v.clone();
    }
    let v = std::sync::Arc::new(app.session.active().map(|s| photocraft_engine::sampler_cmds::channel_readouts(&s.doc, x, y)).unwrap_or_default());
    app.info_channels = Some((key, v.clone()));
    v
}

/// The rows of readout `r` as (key, value) pairs: the document's `mode` decides what Actual Color
/// shows; `channels` are the document's extra channels (name, is spot) so the Spot Channels and
/// Alpha Channels readouts keep their rows when the pointer is off the canvas; `v` and `read` are
/// the sample's values, "—" without one.
fn readout_rows(mode: ColorMode, r: Readout, v: Option<&Readouts>, channels: &[(String, bool)], read: Option<&[ChannelReadout]>) -> Vec<(String, String)> {
    let dash = || "—".to_string();
    let n = |f: fn(&Readouts) -> i32| v.map_or_else(dash, |r| f(r).to_string());
    let pct = |f: fn(&Readouts) -> i32| v.map_or_else(dash, |r| format!("{}%", f(r)));
    let deg = |f: fn(&Readouts) -> i32| v.map_or_else(dash, |r| format!("{}°", f(r)));
    let rows = |pairs: Vec<(&str, String)>| pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect::<Vec<_>>();
    let rgb = || rows(vec![("R:", n(|r| r.rgb[0])), ("G:", n(|r| r.rgb[1])), ("B:", n(|r| r.rgb[2]))]);
    let cmyk = || rows(vec![("C:", pct(|r| r.cmyk[0])), ("M:", pct(|r| r.cmyk[1])), ("Y:", pct(|r| r.cmyk[2])), ("K:", pct(|r| r.cmyk[3]))]);
    let lab = || rows(vec![("L:", n(|r| r.lab[0])), ("a:", n(|r| r.lab[1])), ("b:", n(|r| r.lab[2]))]);
    let gray = || rows(vec![("K:", pct(|r| r.gray))]);
    // Each extra channel of the kind, in Channels-panel order, with its reading when there is one.
    let extra = |spot: bool| {
        channels
            .iter()
            .enumerate()
            .filter(|(_, (_, s))| *s == spot)
            .map(|(i, (name, _))| (format!("{name}:"), read.and_then(|r| r.get(i)).map_or_else(dash, |c| format!("{}%", c.percent))))
            .collect::<Vec<_>>()
    };
    match r {
        Readout::ActualColor => match mode {
            ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone => gray(),
            ColorMode::Cmyk => cmyk(),
            ColorMode::Lab => lab(),
            ColorMode::Multichannel => [extra(true), extra(false)].concat(),
            ColorMode::Rgb | ColorMode::Indexed => rgb(),
        },
        Readout::Grayscale => gray(),
        Readout::Rgb => rgb(),
        Readout::Web => {
            let hex = |i: usize| v.map_or_else(dash, |r| r.web.get(2 * i..2 * i + 2).unwrap_or("—").to_string());
            rows(vec![("R:", hex(0)), ("G:", hex(1)), ("B:", hex(2))])
        }
        Readout::Hsb => rows(vec![("H:", deg(|r| r.hsb[0])), ("S:", pct(|r| r.hsb[1])), ("B:", pct(|r| r.hsb[2]))]),
        Readout::Hsl => rows(vec![("H:", deg(|r| r.hsl[0])), ("S:", pct(|r| r.hsl[1])), ("L:", pct(|r| r.hsl[2]))]),
        Readout::Cmyk => cmyk(),
        Readout::Lab => lab(),
        Readout::TotalInk => rows(vec![("CMYK:", pct(|r| r.total_ink))]),
        // Only with a sample: `read` is the channels under the pointer or at the point.
        Readout::TotalSpotInk => rows(vec![("Spot:", read.map_or_else(dash, |c| format!("{}%", photocraft_engine::sampler_cmds::total_spot_ink(c))))]),
        Readout::Opacity => vec![(format!("{}:", tl!("Composite opacity")), pct(|r| r.opacity))],
        Readout::SpotChannels => extra(true),
        Readout::AlphaChannels => extra(false),
    }
}

/// Readouts with a descriptive target or channel name use a full-width row, leaving the value clear.
fn wide_readout(mode: ColorMode, r: Readout) -> bool {
    matches!(r, Readout::Opacity | Readout::SpotChannels | Readout::AlphaChannels) || (r == Readout::ActualColor && mode == ColorMode::Multichannel)
}

/// The Color Sampler lines of readout `r` for one point: its rows on one line, except the channel
/// readouts, one channel per line.
fn sampler_lines(mode: ColorMode, r: Readout, v: Option<&Readouts>, channels: &[(String, bool)], read: Option<&[ChannelReadout]>) -> Vec<String> {
    let rows = readout_rows(mode, r, v, channels, read);
    if matches!(r, Readout::SpotChannels | Readout::AlphaChannels) {
        rows.into_iter().map(|(k, v)| format!("{k}{v}")).collect()
    } else if rows.is_empty() {
        Vec::new()
    } else {
        vec![rows.iter().map(|(k, v)| format!("{k}{v}")).collect::<Vec<_>>().join(" ")]
    }
}

/// Photoshop's Info panel: the colour under the pointer in the readouts the panel menu picks
/// (`info_panel_options`), position, selection size, Color Sampler points.
pub(super) fn info_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No document"));
        return;
    };
    let (size, origin, mode) = (st.doc.size, st.doc.ruler_origin, st.doc.mode);
    let channels: Vec<(String, bool)> = st.doc.channels.iter().map(|c| (c.name.clone(), c.spot.is_some())).collect();
    // Positions and sizes in the ruler unit (Preferences › Units & Rulers).
    let units = app.session.prefs().units_and_rulers.clone();
    let dpi = st.doc.resolution_dpi as f64;
    let fx = move |px: f64| units.format(px, dpi, size.width as f64);
    let units_y = app.session.prefs().units_and_rulers.clone();
    let fy = move |px: f64| units_y.format(px, dpi, size.height as f64);
    let sel = st.doc.selection.as_ref().map(photocraft_compose::bounds::content_bounds);
    let pos = hover_pixel(app);
    // While a guide is dragged (#5728), the Info panel shows its position in one axis only, even
    // when the pointer is outside the canvas (ruler drags do not update `hover_doc` there).
    let (guide_x, guide_y) = match app.guide_drag {
        Some(d) if d.vertical => (Some(d.pos), None),
        Some(d) => (None, Some(d.pos)),
        None => (None, None),
    };
    let rgba = pos.and_then(|p| pointer_sample(app, p));
    // The readouts of the sample, through the same conversions as the Color Sampler points
    // (`sampler_cmds::readouts`: colour-managed CMYK as the Color Picker, #8101; Grayscale and
    // Opacity as Photoshop reads them), and the extra channels under the pointer.
    let pointer = rgba.and_then(|c| app.session.active().map(|s| photocraft_engine::sampler_cmds::readouts(&s.doc, c)));
    let read = pos.map(|p| pointer_channels(app, p));
    let shown = app.ui.info_panel.shown();
    let mono = theme::mono(11.5);
    // A column row: a short key ("R:", "Ink:") in a fixed slot, then the value.
    let row = |ui: &mut egui::Ui, k: &str, v: String| {
        ui.horizontal(|ui| {
            let key = egui::Label::new(RichText::new(k).color(t.text_dim).size(11.5));
            if k.chars().count() <= 2 {
                ui.add_sized([18.0, 16.0], key);
            } else {
                ui.add(key);
            }
            ui.label(RichText::new(v).font(mono.clone()).color(t.text));
        });
    };
    // A full-width row for a channel: the value sits at the right edge, the name takes what is
    // left and is cut with an ellipsis, so a long spot name never runs under the value.
    let wide_row = |ui: &mut egui::Ui, k: &str, v: String| {
        ui.horizontal(|ui| {
            ui.set_min_height(16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(v).font(mono.clone()).color(t.text));
                // The name starts at the left of what is left, not against the value.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(k).color(t.text_dim).size(11.5)).truncate());
                });
            });
        });
    };
    // Each readout is a block of rows. The colour readouts' rows are short, so their blocks fill
    // the panel's two columns in turn; the channel readouts list names, so each takes the full
    // width below them.
    let (narrow, wide): (Vec<_>, Vec<_>) = shown
        .iter()
        .map(|r| (wide_readout(mode, *r), readout_rows(mode, *r, pointer.as_ref(), &channels, read.as_deref().map(Vec::as_slice))))
        .filter(|(_, b)| !b.is_empty())
        .partition(|(wide, _)| !*wide);
    if !narrow.is_empty() {
        ui.columns(2, |cols| {
            for (i, (_, block)) in narrow.iter().enumerate() {
                let col = &mut cols[i % 2];
                if i >= 2 {
                    col.add_space(6.0);
                }
                for (k, v) in block {
                    row(col, k, v.clone());
                }
            }
        });
    }
    for (i, (_, block)) in wide.iter().enumerate() {
        if i > 0 || !narrow.is_empty() {
            ui.add_space(6.0);
        }
        for (k, v) in block {
            wide_row(ui, k, v.clone());
        }
    }
    if !narrow.is_empty() || !wide.is_empty() {
        widgets::hairline(ui);
    }
    ui.columns(2, |cols| {
        let dash = || "—".to_string();
        // Measured from the rulers' zero point (#2955).
        row(&mut cols[0], "X:", guide_x.map(|p| fx(p - origin[0])).or_else(|| pos.map(|p| fx(p.0 as f64 - origin[0]))).unwrap_or_else(dash));
        row(&mut cols[0], "Y:", guide_y.map(|p| fy(p - origin[1])).or_else(|| pos.map(|p| fy(p.1 as f64 - origin[1]))).unwrap_or_else(dash));
        row(&mut cols[1], "W:", sel.map_or_else(dash, |r| fx(r.width() as f64)));
        row(&mut cols[1], "H:", sel.map_or_else(dash, |r| fy(r.height() as f64)));
    });
    widgets::hairline(ui);
    ui.label(
        RichText::new(crate::i18n::fmt(tl!("Doc: {w} × {h} px"), &[("w", &size.width.to_string()), ("h", &size.height.to_string())]))
            .color(t.text_dim)
            .size(11.5),
    );
    // Color Sampler points (#1046): each under its number, with its position in the current ruler
    // unit and the same readouts as the pointer (one composite pixel each).
    let samplers = sampler_readouts(app);
    if !samplers.is_empty() {
        widgets::hairline(ui);
        let cell = |ui: &mut egui::Ui, s: String| {
            ui.label(RichText::new(s).font(mono.clone()).color(t.text));
        };
        for s in samplers.iter() {
            let n = s["number"].as_i64().unwrap_or(0);
            let at = s["position"]
                .as_array()
                .map_or((0.0, 0.0), |a| (a.first().and_then(Value::as_f64).unwrap_or(0.0), a.get(1).and_then(Value::as_f64).unwrap_or(0.0)));
            let values: Option<Readouts> = serde_json::from_value(s.clone()).ok();
            let read: Option<Vec<ChannelReadout>> = serde_json::from_value(s["channels"].clone()).ok();
            ui.horizontal(|ui| {
                ui.add_sized([24.0, 16.0], egui::Label::new(RichText::new(format!("#{n}")).color(t.text_dim).size(11.5)));
                cell(ui, format!("X: {}  Y: {}", fx(at.0), fy(at.1)));
            });
            for line in shown.iter().flat_map(|r| sampler_lines(mode, *r, values.as_ref(), &channels, read.as_deref())) {
                ui.horizontal(|ui| {
                    ui.add_sized([24.0, 16.0], egui::Label::new(RichText::new("").size(11.5)));
                    cell(ui, line);
                });
            }
        }
    }
}

#[cfg(test)]
mod readout_tests;

#[cfg(test)]
mod cache_tests;

/// Color Sampler readouts and the (document ID, activation, revision) they were composited for.
pub(crate) type SamplerReadouts = ((photocraft_doc::DocId, u64, u64), std::sync::Arc<Vec<Value>>);

/// The Color Sampler readouts of the active document, composited once per revision (#4528):
/// adding, moving or deleting a sampler and every pixel edit bump the revision, so between them
/// the Info panel reuses them instead of compositing each point through the stack every frame.
/// Activation also matters: reopened documents can reuse an ID and reach the same revision.
pub(crate) fn sampler_readouts(app: &mut PhotocraftApp) -> std::sync::Arc<Vec<Value>> {
    let Some(key) = app.session.active().map(|s| (s.doc.id, app.session.active_generation(), s.revision)) else { return std::sync::Arc::default() };
    if let Some((k, v)) = &app.info_samplers
        && *k == key
    {
        return v.clone();
    }
    let list = app.session.execute("view.colorSamplers.list", json!({})).ok();
    let v = std::sync::Arc::new(list.as_ref().and_then(|v| v["samplers"].as_array()).cloned().unwrap_or_default());
    app.info_samplers = Some((key, v.clone()));
    v
}

#[cfg(test)]
mod opacity_tests;
