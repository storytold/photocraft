use craft::color_ui::ui::{InfoColor, InfoMeasure, InfoPanel, InfoReadings, SamplerReadout};
use craft::ui::runtime::env::Label;

use super::{PhotocraftApp, Value, json};
use crate::canvas::document_display::CanvasHost;

/// The active document's pixel under the pointer, if the pointer is over the image.
pub(crate) fn hover_pixel(app: &PhotocraftApp) -> Option<(i32, i32)> {
    let size = app.session.active()?.doc.size;
    app.hover_doc.map(|p| (p[0].floor() as i32, p[1].floor() as i32)).filter(|(x, y)| *x >= 0 && *y >= 0 && *x < size.width as i32 && *y < size.height as i32)
}

/// The colours the Info panel reads at document pixel `(x, y)`: the composite RGB (straight RGBA
/// 0..1), averaged over the Eyedropper's Sample Size over the box it reads at the view zoom, as in
/// Photoshop (#1649, #11732, #11751), and its CMYK percentages (the document's own inks for a CMYK
/// document, #11640). Both are cached per document, pixel, revision, Sample Size and zoom, so
/// neither the composite nor the ink composite runs again on a frame that shows the same pixel of
/// the same document (#4528) — and a switch to another document re-reads (#11681 review).
pub(crate) fn pointer_readout(app: &mut PhotocraftApp, (x, y): (i32, i32)) -> Option<crate::InfoSample> {
    let st = app.session.active()?;
    let (doc, rev) = (st.doc.id, st.revision);
    let sample_size = app.ui.tool_options.eyedropper.size.pixels();
    let zoom = app.current_zoom();
    let key = (doc, x, y, rev, sample_size, zoom.to_bits());
    if let Some((k, v)) = app.info_sample
        && k == key
    {
        return Some(v);
    }
    let at = [f64::from(x) + 0.5, f64::from(y) + 0.5];
    let params = json!({"x": at[0], "y": at[1], "size": sample_size, "zoom": f64::from(zoom)});
    let v: Vec<f32> = serde_json::from_value(app.session.execute("document.sampleColor", params).ok()?).ok()?;
    let rgba = [*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?];
    // The CMYK row through the engine, as the Color Sampler points do: the document's own inks for
    // a CMYK document, so a pure ink reads 100 % (#11640), over the same box the RGB row just
    // averaged (#11751); else the colour-managed conversion of its RGB composite, the same one the
    // Color Picker uses (#8101).
    let cmyk = app.session.active().map(|st| photocraft_engine::sampler_cmds::cmyk_readout(&st.doc, at, sample_size, Some(f64::from(zoom)), rgba))?;
    app.info_sample = Some((key, (rgba, cmyk)));
    Some((rgba, cmyk))
}

/// The Info panel's W and H for a size of `wh` px (the selection's or the pending crop frame's):
/// both go through the ruler unit's formatter at `dpi`, percent of the document's `extent`.
pub(crate) fn wh_readout(ur: &photocraft_engine::prefs::UnitsAndRulers, dpi: f64, extent: [f64; 2], wh: [f64; 2]) -> [String; 2] {
    [ur.format(wh[0], dpi, extent[0]), ur.format(wh[1], dpi, extent[1])]
}

/// The Info panel's ruler cells (#11991), shown while the Ruler tool is active and a ruler is
/// drawn: the angle and length(s), and the point and size the X/Y and W/H cells show instead of
/// the pointer's and the selection's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RulerRows {
    /// The angle and length (a plain ruler) or the angle and both arms' lengths (a protractor), as value strings;
    /// the panel words their names.
    pub(crate) measure: InfoMeasure,
    /// The start point (a plain ruler) or the second arm's end (a protractor), in document pixels.
    pub(crate) point: [f64; 2],
    /// End minus start, signed and y down, for a plain ruler. A protractor's is not measured, so
    /// W/H keep showing what they show today.
    pub(crate) size: Option<[f64; 2]>,
}

/// [`RulerRows`] for the active document, or `None` with another tool or no ruler. Lengths are in
/// the ruler unit with 2 decimals and the angle has 1 decimal and "°"; the options bar's Use
/// Measurement Scale never changes them, as measured.
pub(crate) fn ruler_rows(app: &PhotocraftApp) -> Option<RulerRows> {
    if app.ui.tool != crate::state::Tool::Ruler {
        return None;
    }
    let doc = &app.session.active()?.doc;
    let r = doc.measurement.ruler?;
    // A unit scale: the Info panel reads pixels converted to the ruler unit, never the scale.
    let info = photocraft_engine::analysis_cmds::ruler_info(&r, &Default::default());
    let g = |k: &str| info[k].as_f64();
    let units = &app.session.prefs().units_and_rulers;
    let dpi = f64::from(doc.resolution_dpi);
    let length = |px: f64| {
        let v = units.rulers.from_px(px, dpi, f64::from(doc.size.width), units.point_size.per_inch());
        format!("{v:.2}")
    };
    let angle = format!("{:.1}°", g("angle")?);
    match r.protractor {
        None => Some(RulerRows { measure: InfoMeasure::line(angle, length(g("l1")?)), point: r.start, size: Some([g("w")?, g("h")?]) }),
        Some(q) => Some(RulerRows { measure: InfoMeasure::protractor(angle, length(g("l1")?), length(g("l2")?)), point: q, size: None }),
    }
}

/// The 8-bit value of a channel read 0..1.
fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// What the Info panel shows now: the colour under the pointer (RGB and CMYK), its position, the selection's
/// or pending crop frame's size (with the Ruler tool, a drawn ruler's point and size), the document size and the Color Sampler `samplers`. A guide being dragged
/// (#5728) shows its position on its own axis only, even when the pointer is outside the canvas. Without a
/// document the readings are empty, which the panel shows as its no-document notice.
pub(crate) fn readings<'a>(app: &mut PhotocraftApp, samplers: &'a [(SamplerReadout, [Label; 2])]) -> InfoReadings<'a> {
    let Some(st) = app.session.active() else { return InfoReadings::new() };
    let (size, origin, dpi, idx) = (st.doc.size, st.doc.ruler_origin, f64::from(st.doc.resolution_dpi), app.session.active_index());
    let selection = st.doc.selection.as_ref().map(photocraft_compose::bounds::content_bounds).map(|r| [r.width() as f64, r.height() as f64]);
    // Positions and sizes in the ruler unit (Preferences › Units & Rulers).
    let units = app.session.prefs().units_and_rulers.clone();
    let extent = [f64::from(size.width), f64::from(size.height)];
    // With the Crop tool, W and H are the pending frame's, as in Photoshop (#3145).
    // With a drawn ruler and the Ruler tool, X/Y are its point and W/H its size (#11991).
    let ruler = ruler_rows(app);
    let wh = ruler.as_ref().and_then(|r| r.size).or(crate::crop_ui::info_size(app)).or(selection);
    let guide = idx.and_then(|idx| craft::ui::canvas_controls::reading::running(app.ctx(), CanvasHost::primary(app, idx).canvas_id()));
    let pointer = hover_pixel(app);
    let sample = pointer.and_then(|p| pointer_readout(app, p));
    let color = sample.map(|(c, cmyk)| {
        // CMYK in percent like Photoshop's readout, through the engine's readout: the document's own inks
        // for a CMYK document (#11640), else the same colour-managed conversion as the Color Picker
        // (#8101) and the Color Sampler readout (`sampler_cmds::cmyk_readout`).
        InfoColor::from_parts([byte(c[0]), byte(c[1]), byte(c[2])], cmyk.map(|v| v.clamp(0, 100) as u8))
    });
    // Measured from the rulers' zero point (#2955).
    let at = |axis: usize, px: f64| Label::from(units.format(px - origin[axis], dpi, extent[axis]));
    let position = match (&ruler, guide) {
        (Some(r), _) => Some([at(0, r.point[0]), at(1, r.point[1])]),
        (None, Some(g)) if g.along == craft::behavior::Axis::Y => Some([at(0, g.position), Label::Empty]),
        (None, Some(g)) => Some([Label::Empty, at(1, g.position)]),
        (None, None) => pointer.map(|p| [at(0, f64::from(p.0)), at(1, f64::from(p.1))]),
    };
    let extent_labels = wh.map(|s| wh_readout(&units, dpi, extent, s).map(Label::from));
    InfoReadings::new()
        .color(color)
        .position(position)
        .extent(extent_labels)
        .measure(ruler.as_ref().map(|r| r.measure.clone()))
        .document(Some([size.width, size.height]))
        .samplers(samplers)
}

/// The first `N` entries of the JSON array `v` as bytes; `None` when one is missing or not a byte.
fn channels<const N: usize>(v: &Value) -> Option<[u8; N]> {
    let list = v.as_array()?;
    let mut out = [0u8; N];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::try_from(list.get(i)?.as_i64()?).ok()?;
    }
    Some(out)
}

/// The Color Sampler points (#1046) as the panel shows them: each with its position in the current ruler
/// unit and the same RGB/CMYK readout as the pointer (one composite pixel each). A point with no colour
/// to read is left out.
pub(crate) fn sampler_rows(app: &mut PhotocraftApp) -> Vec<(SamplerReadout, [Label; 2])> {
    let readouts = sampler_readouts(app);
    let Some(st) = app.session.active() else { return Vec::new() };
    let (size, dpi) = (st.doc.size, f64::from(st.doc.resolution_dpi));
    let units = &app.session.prefs().units_and_rulers;
    readouts
        .iter()
        .filter_map(|s| {
            let at = s["position"]
                .as_array()
                .map_or((0.0, 0.0), |a| (a.first().and_then(Value::as_f64).unwrap_or(0.0), a.get(1).and_then(Value::as_f64).unwrap_or(0.0)));
            let (rgb, cmyk) = (channels::<3>(&s["rgb"])?, channels::<4>(&s["cmyk"])?);
            let number = usize::try_from(s["number"].as_i64().unwrap_or(0)).unwrap_or(0);
            // The point sits on its pixel's centre (#11725); the row names the pixel, as the pointer row does.
            let (x, y) = (at.0.floor(), at.1.floor());
            let labels = [Label::from(units.format(x, dpi, f64::from(size.width))), Label::from(units.format(y, dpi, f64::from(size.height)))];
            Some((SamplerReadout::from_values(number, [x as i64, y as i64], rgb, cmyk), labels))
        })
        .collect()
}

/// Photoshop's Info panel: colour under the pointer (RGB and CMYK), position, selection size.
pub(super) fn info_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let samplers = sampler_rows(app);
    let readings = readings(app, &samplers);
    InfoPanel::new(tl!("Info"), &readings).show(ui);
}

/// The Info panel's Color Sampler readouts and the (document, revision, Sample Size) they were
/// composited for.
pub(crate) type SamplerReadouts = ((photocraft_doc::DocId, u64, u32), std::sync::Arc<Vec<Value>>);

/// The Color Sampler readouts of the active document, composited once per revision (#4528):
/// adding, moving or deleting a sampler and every pixel edit bump the revision, so between them
/// the Info panel reuses them instead of compositing each point through the stack every frame.
/// They average over the Sample Size shared with the Eyedropper (#11732), which is part of the
/// key, so changing it re-reads every sampler at once.
pub(crate) fn sampler_readouts(app: &mut PhotocraftApp) -> std::sync::Arc<Vec<Value>> {
    let size = app.ui.tool_options.eyedropper.size.pixels();
    let Some(key) = app.session.active().map(|s| (s.doc.id, s.revision, size)) else { return std::sync::Arc::default() };
    if let Some((k, v)) = &app.info_samplers
        && *k == key
    {
        return v.clone();
    }
    let list = app.session.execute("view.colorSamplers.list", json!({"size": size})).ok();
    let v = std::sync::Arc::new(list.as_ref().and_then(|v| v["samplers"].as_array()).cloned().unwrap_or_default());
    app.info_samplers = Some((key, v.clone()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Composited once per revision: repeated frames share the readouts; adding a sampler or an
    /// edit recomputes them (#4528).
    #[test]
    fn sampler_readouts_are_reused_until_the_document_changes() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
        app.run("view.colorSamplers.add", json!({"x": 4, "y": 4})).unwrap();
        let a = sampler_readouts(&mut app);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0]["rgb"], json!([255, 255, 255]));
        assert!(std::sync::Arc::ptr_eq(&a, &sampler_readouts(&mut app)), "the next frame reuses them");
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let b = sampler_readouts(&mut app);
        assert!(!std::sync::Arc::ptr_eq(&a, &b));
        assert_eq!(b[0]["rgb"], json!([255, 0, 0]), "an edit shows");
        app.run("view.colorSamplers.add", json!({"x": 8, "y": 8})).unwrap();
        assert_eq!(sampler_readouts(&mut app).len(), 2, "so does a new sampler");
    }

    /// #11681 review: the pointer sample cache is keyed by the document as well as the pixel, the
    /// revision and the Sample Size. Two open documents can hold the same revision, so switching
    /// back to the first must re-read its own inks instead of repeating the second one's row.
    #[test]
    fn pointer_readout_is_keyed_by_the_document() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        // Two CMYK documents at the same revision (one edit each), the first pure cyan, the second
        // pure magenta, both inked under pixel (3, 3).
        for ink in [[1.0f32, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]] {
            app.run("file.new", json!({"width": 8, "height": 8, "mode": "cmyk", "background": "white"})).unwrap();
            app.session
                .edit("paint", move |doc, _| {
                    let surf = doc.layers[0].surface_mut().unwrap();
                    surf.fill_rect(photocraft_geom::Rect::new(2, 2, 4, 4), &[ink[0], ink[1], ink[2], ink[3], 1.0]);
                    Ok(())
                })
                .unwrap();
        }
        let read = |app: &mut PhotocraftApp| pointer_readout(app, (3, 3)).map(|(_, cmyk)| cmyk);
        assert_eq!(read(&mut app), Some([0, 100, 0, 0]), "the open document reads its magenta ink");
        app.run("document.activate", json!({"document": 0})).unwrap();
        assert_eq!(read(&mut app), Some([100, 0, 0, 0]), "the first document re-reads its own cyan");
        // And back to the second, still its own values.
        app.run("document.activate", json!({"document": 1})).unwrap();
        assert_eq!(read(&mut app), Some([0, 100, 0, 0]), "and so does the second");
    }
}

#[cfg(test)]
#[path = "info_ruler_tests.rs"]
mod info_ruler_tests;
