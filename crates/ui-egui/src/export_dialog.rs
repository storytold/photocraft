//! File › Export › Export As… (and Quick Export as PNG): format, quality, transparency and scale,
//! with a preview and an estimated file size. The estimate encodes a small proxy and scales by
//! pixel count, so the dialog stays instant on 36 MP documents.

use std::sync::Arc;

use photocraft_doc::Document;
use serde_json::{Map, Value, json};

use crate::state::DialogKind;
use crate::theme::Tokens;
use crate::{ExportSettings, PhotocraftApp};

const FORMATS: [(&str, &str); 6] = [("png", "PNG"), ("jpg", "JPG"), ("webp", "WebP"), ("avif", "AVIF"), ("tif", "TIFF"), ("tga", "TGA")];

pub fn open(app: &mut PhotocraftApp) -> Result<u64, String> {
    let st = app.session.active().ok_or("no document")?;
    let mut f = Map::new();
    f.insert("__export".into(), json!(true));
    f.insert("__label".into(), json!("Export As"));
    f.insert("format".into(), json!("png"));
    f.insert("quality".into(), json!(app.session.prefs().export.jpeg_quality));
    f.insert("lossless".into(), json!(app.session.prefs().export.webp_lossless));
    f.insert("transparency".into(), json!(true));
    f.insert("scale".into(), json!(100));
    f.insert("metadata".into(), json!("none"));
    f.insert("__w".into(), json!(st.doc.size.width));
    f.insert("__h".into(), json!(st.doc.size.height));
    Ok(app.ui.open_dialog(DialogKind::Command, f))
}

/// Layer › Export As…: the same dialog for just the active layer (trimmed to its pixels).
pub fn open_layer(app: &mut PhotocraftApp, layer: photocraft_doc::LayerId) -> Result<u64, String> {
    let st = app.session.active().ok_or("no document")?;
    let ldoc = photocraft_engine::layer_menu_cmds::layer_document(&st.doc, layer).map_err(|e| e.to_string())?;
    let id = open(app)?;
    if let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == id) {
        d.fields.insert("__layer".into(), json!(layer.0));
        d.fields.insert("__label".into(), json!(format!("Export As: {}", ldoc.name)));
        d.fields.insert("__w".into(), json!(ldoc.size.width));
        d.fields.insert("__h".into(), json!(ldoc.size.height));
    }
    Ok(id)
}

/// Layer › Quick Export as PNG.
pub fn quick_export_layer_png(app: &mut PhotocraftApp, layer: photocraft_doc::LayerId) -> Result<Value, String> {
    let mut f = Map::new();
    f.insert("format".into(), json!("png"));
    f.insert("transparency".into(), json!(true));
    f.insert("scale".into(), json!(100));
    f.insert("__layer".into(), json!(layer.0));
    confirm(app, &f)
}

/// The document a dialog exports: the whole image, or one layer (`__layer`).
fn source_document(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<Arc<Document>, String> {
    let st = app.session.active().ok_or("no document")?;
    match f.get("__layer").and_then(Value::as_u64) {
        Some(id) => photocraft_engine::layer_menu_cmds::layer_document(&st.doc, photocraft_doc::LayerId(id)).map(Arc::new).map_err(|e| e.to_string()),
        None => Ok(st.doc.clone()),
    }
}

fn s(f: &Map<String, Value>, k: &str) -> String {
    f.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}
fn n(f: &Map<String, Value>, k: &str, d: f64) -> f64 {
    f.get(k).and_then(Value::as_f64).unwrap_or(d)
}

/// The document as exported: scaled (engine Image Size, bicubic) and flattened over white when
/// transparency is off or the format has no alpha.
fn export_document(doc: &Document, f: &Map<String, Value>, max_side: Option<u32>) -> Result<Document, String> {
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc.clone(), None);
    let mut scale = n(f, "scale", 100.0) / 100.0;
    if let Some(m) = max_side {
        let long = doc.size.width.max(doc.size.height) as f64 * scale;
        if long > m as f64 {
            scale *= m as f64 / long;
        }
    }
    // Both dimensions are sent: a scale can leave a 1 px width unchanged while halving the
    // height, and the engine would otherwise keep the aspect ratio from the width alone.
    let (w, h) = scaled_size(doc.size.width as f64, doc.size.height as f64, scale);
    if (w - doc.size.width as f64).abs() >= 1.0 || (h - doc.size.height as f64).abs() >= 1.0 {
        s.execute("image.imageSize", json!({"width": w, "height": h, "resample": if max_side.is_some() { "bilinear" } else { "bicubic" }}))
            .map_err(|e| e.to_string())?;
    }
    let fmt = s_fmt(f);
    if !f.get("transparency").and_then(Value::as_bool).unwrap_or(true) || fmt == "jpg" {
        s.execute("layer.flattenImage", json!({})).map_err(|e| e.to_string())?;
    }
    s.active().map(|d| (*d.doc).clone()).ok_or_else(|| "export failed".into())
}

/// The exported pixel size: each side scaled and rounded on its own, never below 1 px. The
/// dialog's size label and `export_document` share it so the label names the file's real size.
fn scaled_size(w: f64, h: f64, scale: f64) -> (f64, f64) {
    ((w * scale).round().max(1.0), (h * scale).round().max(1.0))
}

/// Export As already supports lossy WebP; reuse the Quick Export defaults when
/// choosing that format so both flows expose the same quality and lossless options.
fn set_format_defaults(f: &mut Map<String, Value>, fmt: &str, prefs: &photocraft_engine::prefs::Export) {
    f.insert("format".into(), json!(fmt));
    match fmt {
        "jpg" => {
            f.insert("quality".into(), json!(prefs.jpeg_quality));
        }
        "webp" => {
            f.insert("quality".into(), json!(prefs.webp_quality));
            f.insert("lossless".into(), json!(prefs.webp_lossless));
        }
        "avif" => {
            f.insert("quality".into(), json!(90));
            f.insert("avifSpeed".into(), json!(8));
            f.insert("avifDepth".into(), json!(0));
            f.insert("avifAlphaQuality".into(), json!(100));
        }
        _ => {}
    }
}

fn s_fmt(f: &Map<String, Value>) -> String {
    let v = s(f, "format");
    if v.is_empty() { "png".into() } else { v }
}

fn lossless(f: &Map<String, Value>) -> bool {
    f.get("lossless").and_then(Value::as_bool).unwrap_or(false)
}

fn settings(f: &Map<String, Value>) -> ExportSettings {
    let fmt = s_fmt(f);
    let quality = n(f, "quality", 85.0).clamp(1.0, 100.0) as u8;
    ExportSettings {
        avif_quality: if fmt == "avif" { n(f, "quality", 90.0) as u8 } else { 90 },
        avif_speed: n(f, "avifSpeed", 8.0) as u8,
        avif_depth: n(f, "avifDepth", 0.0) as u8,
        avif_alpha_quality: n(f, "avifAlphaQuality", 100.0) as u8,
        jpeg_quality: (fmt == "jpg").then_some(quality),
        webp_lossless: fmt != "webp" || lossless(f),
        webp_quality: (fmt == "webp" && !lossless(f)).then_some(quality),
        xmp_all: s(f, "metadata") == "all",
        ..Default::default()
    }
}

/// Estimated size (bytes) from a ≤512 px proxy encode, scaled by pixel count.
fn estimate(app: &PhotocraftApp, doc: &Document, f: &Map<String, Value>) -> Option<u64> {
    // AV1 encoding is too expensive to run synchronously during dialog drawing.
    if s_fmt(f) == "avif" {
        return None;
    }
    let export = app.services.export.as_ref()?;
    let proxy = export_document(doc, f, Some(512)).ok()?;
    let (bytes, _) = export(&proxy, &format!("estimate.{}", s_fmt(f)), &settings(f)).ok()?;
    let scale = n(f, "scale", 100.0) / 100.0;
    let full = doc.size.width as f64 * scale * doc.size.height as f64 * scale;
    let small = (proxy.size.width as f64 * proxy.size.height as f64).max(1.0);
    Some((bytes.len() as f64 * full / small) as u64)
}

pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let Some(doc) = app.session.active().map(|s| s.doc.clone()) else { return };
    ui.horizontal_top(|ui| {
        // Left: settings.
        ui.vertical(|ui| {
            ui.set_width(220.0);
            ui.label(egui::RichText::new(tl!("File Settings")).font(crate::theme::semibold(12.0)).color(t.text));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Format")).color(t.text_dim));
                let mut fmt = s_fmt(f);
                let opts: Vec<(String, &str)> = FORMATS
                    .iter()
                    .filter(|(k, _)| *k != "avif" || photocraft_codecs::caps(photocraft_codecs::Format::Avif).write)
                    .map(|(k, l)| (k.to_string(), *l))
                    .collect();
                if crate::widgets::dropdown(ui, "export-format", &mut fmt, &opts, 130.0) {
                    set_format_defaults(f, &fmt, &app.session.prefs().export);
                }
            });
            let fmt = s_fmt(f);
            if fmt == "webp" {
                let mut ll = lossless(f);
                crate::widgets::checkbox(ui, &mut ll, tl!("Lossless"));
                f.insert("lossless".into(), json!(ll));
            }
            if fmt == "jpg" || fmt == "avif" || (fmt == "webp" && !lossless(f)) {
                let mut q = n(f, "quality", 85.0) as f32;
                crate::widgets::slider_row(ui, tl!("Quality"), &mut q, 1.0..=100.0, "%", None);
                f.insert("quality".into(), json!(q.round() as u8));
            }
            if fmt == "avif" {
                let mut speed = n(f, "avifSpeed", 8.0) as f32;
                crate::widgets::slider_row(ui, "Speed", &mut speed, 1.0..=10.0, "", None);
                f.insert("avifSpeed".into(), json!(speed.round() as u8));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Bit depth")).color(t.text_dim));
                    let mut depth = n(f, "avifDepth", 0.0) as u8;
                    if crate::widgets::dropdown(ui, "avif-depth", &mut depth, &[(0, "Automatic"), (8, "8 bit"), (10, "10 bit")], 130.0) {
                        f.insert("avifDepth".into(), json!(depth));
                    }
                });
                let mut aq = n(f, "avifAlphaQuality", 100.0) as f32;
                crate::widgets::slider_row(ui, "Alpha quality", &mut aq, 1.0..=100.0, "%", None);
                f.insert("avifAlphaQuality".into(), json!(aq.round() as u8));
                ui.label(egui::RichText::new(tl!("Lossy RGB 4:4:4; ICC profile retained.")).color(t.text_dim).size(11.5));
            }
            if fmt != "jpg" {
                let mut tr = f.get("transparency").and_then(Value::as_bool).unwrap_or(true);
                crate::widgets::checkbox(ui, &mut tr, tl!("Transparency"));
                f.insert("transparency".into(), json!(tr));
            }
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Metadata")).color(t.text_dim));
                let mut m = s(f, "metadata");
                let opts: Vec<(String, &str)> = vec![("none".into(), tl!("None")), ("all".into(), tl!("All"))];
                if crate::widgets::dropdown(ui, "export-metadata", &mut m, &opts, 130.0) {
                    f.insert("metadata".into(), json!(m));
                }
            });
            ui.add_space(8.0);
            ui.label(egui::RichText::new(tl!("Image Size")).font(crate::theme::semibold(12.0)).color(t.text));
            let mut sc = n(f, "scale", 100.0) as f32;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Scale")).color(t.text_dim));
                crate::widgets::value_field(ui, &mut sc, 1.0..=1000.0, "%", 70.0);
            });
            f.insert("scale".into(), json!(sc.round()));
            let (bw, bh) = (n(f, "__w", f64::from(doc.size.width)), n(f, "__h", f64::from(doc.size.height)));
            let (w, h) = scaled_size(bw, bh, f64::from(sc) / 100.0);
            ui.label(egui::RichText::new(format!("{w} × {h} px")).color(t.text_dim).size(11.5));
        });
        ui.add_space(12.0);
        // Right: preview + estimated size.
        ui.vertical(|ui| {
            let layer = f.get("__layer").and_then(Value::as_u64);
            let key = egui::Id::new(("export-preview", doc.id.0, app.session.active().map_or(0, |s| s.revision), layer));
            let sig = format!(
                "{}{}{}{}{}{}",
                s_fmt(f),
                n(f, "quality", 85.0),
                lossless(f),
                f.get("transparency").map(|v| v.to_string()).unwrap_or_default(),
                n(f, "scale", 100.0),
                s(f, "metadata")
            );
            let cached: Option<(String, Option<u64>, Arc<egui::TextureHandle>)> = ui.data(|d| d.get_temp(key));
            let (size, tex) = match cached.filter(|c| c.0 == sig) {
                Some((_, size, tex)) => (size, tex),
                None => {
                    let src = source_document(app, f).unwrap_or_else(|_| doc.clone());
                    let size = estimate(app, &src, f);
                    let img = photocraft_compose::thumbnail(&export_document(&src, f, Some(360)).unwrap_or_else(|_| (*src).clone()), 360);
                    let color = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.pixels);
                    let tex = Arc::new(ui.ctx().load_texture("export-preview", color, egui::TextureOptions::LINEAR));
                    ui.data_mut(|d| d.insert_temp(key, (sig, size, tex.clone())));
                    (size, tex)
                }
            };
            let side = 300.0;
            let (r, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
            crate::widgets::checker(ui.painter(), r, 8.0);
            let ts = tex.size_vec2();
            let k = (side / ts.x.max(ts.y)).min(1.0e3);
            let ir = egui::Rect::from_center_size(r.center(), ts * k);
            ui.painter().image(tex.id(), ir, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.separator), egui::StrokeKind::Outside);
            let est = size.map_or("—".to_string(), |b| crate::sizing::human_bytes(b as f64));
            ui.label(egui::RichText::new(format!("{}  ≈ {est}", s_fmt(f).to_uppercase())).color(t.text_dim).size(11.5));
        });
    });
}

/// Export with the dialog's settings: choose a path, render, encode, write.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    if s_fmt(f) == "avif" {
        let mut params = f.clone();
        if let Some(q) = f.get("quality") {
            params.insert("avifQuality".into(), q.clone());
        }
        photocraft_engine::file_cmds::apply_avif_params(&Value::Object(params), &mut photocraft_codecs::EncodeOptions::default()).map_err(|e| e.to_string())?;
        if !photocraft_codecs::caps(photocraft_codecs::Format::Avif).write {
            return Err("AVIF support isn't included in this build of PhotoCraft".into());
        }
    }
    let doc = source_document(app, f)?;
    let stem = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(a, _)| a).to_string();
    let ext = s_fmt(f);
    let suggested = format!("{stem}.{ext}");
    let (f, settings) = (f.clone(), settings(f));
    app.pick_save(&suggested, move |app, path| {
        let out = export_document(&doc, &f, None)?;
        let export = app.services.export.as_ref().ok_or("no exporter configured")?;
        let (bytes, warnings) = export(&out, &path, &settings)?;
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        app.ui.status = format!("Exported {path} ({})", crate::sizing::human_bytes(bytes.len() as f64));
        app.ui.status_error = false;
        crate::notices::io_warnings(app, &format!("Exported {}", crate::file_open::display_name(&path)), &warnings);
        Ok(json!({"path": path, "bytes": bytes.len(), "warnings": warnings}))
    })
}

/// File › Export › Quick Export as PNG: the format, quality, metadata, colour space and location
/// from File › Export › Export Preferences (engine `file.export.quickExport`). On the web (no
/// file system) it falls back to a PNG download through the export service.
pub fn quick_export_png(app: &mut PhotocraftApp) -> Result<Value, String> {
    if !cfg!(target_arch = "wasm32") {
        let prefs = app.session.prefs().export.clone();
        let fmt = serde_json::to_value(prefs.quick_export_format).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "png".into());
        let same = serde_json::to_value(prefs.quick_export_location).ok().is_some_and(|v| v == "sameFolder");
        let saved = app.session.active().is_some_and(|d| d.path.is_some());
        if same && saved {
            return quick_export(app, json!({}));
        }
        let st = app.session.active().ok_or("no document")?;
        let stem = st.doc.name.rsplit_once('.').map_or(st.doc.name.as_str(), |(a, _)| a);
        let (suggested, doc) = (format!("{stem}.{fmt}"), st.doc.id);
        return app.pick_save(&suggested, move |app, path| {
            app.refocus(doc)?;
            quick_export(app, json!({"path": path}))
        });
    }
    let mut f = Map::new();
    f.insert("format".into(), json!("png"));
    f.insert("transparency".into(), json!(true));
    f.insert("scale".into(), json!(100));
    confirm(app, &f)
}

/// Runs Quick Export with `p` (an explicit `path`, or none for the document's folder).
fn quick_export(app: &mut PhotocraftApp, p: Value) -> Result<Value, String> {
    let r = app.run("file.export.quickExport", p)?;
    app.ui.status = format!("Exported {}", r["path"].as_str().unwrap_or_default());
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_document_scales_height_when_rounded_width_is_unchanged() {
        use photocraft_doc::{Color, ColorMode, SampleType, Size};

        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for (width, height, scale, expected) in [
                (1, 100, 50, (1, 50)),
                (1, 200, 50, (1, 100)),
                (10, 100, 99, (10, 99)),
                (10, 100, 101, (10, 101)),
                (400, 300, 50, (200, 150)),
                (400, 300, 100, (400, 300)),
            ] {
                let doc = Document::with_background("x", Size::new(width, height), ColorMode::Rgb, depth, Color::WHITE);
                let before = doc.clone();
                let mut f = Map::new();
                f.insert("format".into(), json!("png"));
                f.insert("scale".into(), json!(scale));
                let out = export_document(&doc, &f, None).unwrap();
                assert_eq!((out.size.width, out.size.height), expected, "{width} × {height} at {scale}%, {depth:?}");
                assert_eq!(out.depth, depth, "export preserves the document's depth");
                assert_eq!(doc, before, "export leaves the source document unchanged");
            }
        }
    }

    #[test]
    fn export_document_proxy_limits_both_dimensions_before_rounding() {
        use photocraft_doc::{Color, ColorMode, SampleType, Size};

        for (width, height, scale, max_side, expected) in
            [(1, 1000, 100, 360, (1, 360)), (1, 1000, 50, 360, (1, 360)), (1, 1000, 25, 360, (1, 250)), (13, 20, 50, 8, (5, 8)), (20, 13, 50, 8, (8, 5))]
        {
            let doc = Document::with_background("x", Size::new(width, height), ColorMode::Rgb, SampleType::U8, Color::WHITE);
            let before = doc.clone();
            let mut f = Map::new();
            f.insert("format".into(), json!("png"));
            f.insert("scale".into(), json!(scale));
            let out = export_document(&doc, &f, Some(max_side)).unwrap();
            assert_eq!((out.size.width, out.size.height), expected, "{width} × {height} at {scale}%, proxy limit {max_side}");
            assert!(out.size.width.max(out.size.height) <= max_side, "proxy respects its longest-side limit");
            assert_eq!(doc, before, "preview leaves the source document unchanged");
        }
    }

    #[test]
    fn avif_export_controls_defaults_and_confirmation_validation() {
        let mut fields = Map::new();
        set_format_defaults(&mut fields, "avif", &Default::default());
        let s = settings(&fields);
        assert_eq!((s.avif_quality, s.avif_speed, s.avif_depth, s.avif_alpha_quality), (90, 8, 0, 100));
        fields.insert("avifSpeed".into(), json!(99));
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        assert!(confirm(&mut app, &fields).unwrap_err().contains("avifSpeed"));
    }

    #[test]
    fn unavailable_avif_export_does_not_open_a_save_dialog() {
        if photocraft_codecs::caps(photocraft_codecs::Format::Avif).write {
            return;
        }
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let mut fields = Map::new();
        set_format_defaults(&mut fields, "avif", &Default::default());
        assert!(confirm(&mut app, &fields).unwrap_err().contains("isn't included"));
        assert!(!app.file_dialog_open());
    }

    #[test]
    fn drawing_avif_controls_keeps_options_valid_for_confirmation() {
        if !photocraft_codecs::caps(photocraft_codecs::Format::Avif).write {
            return;
        }
        use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
        use std::{cell::RefCell, rc::Rc};
        let calls = Rc::new(RefCell::new(Vec::new()));
        let picker_calls = calls.clone();
        let writer_calls = calls.clone();
        let services = crate::Services {
            file_dialog: Some(Box::new(move |request, _parent, reply| {
                let FileDialogRequest::Save { suggested } = request else { panic!("expected a save dialog") };
                picker_calls.borrow_mut().push(format!("pick {suggested}"));
                reply.send(Some(FileDialogAnswer::SaveTo("chosen-destination.avif".into())));
            })),
            export: Some(Box::new(|doc, path, settings| {
                let options = photocraft_io::ExportOptions {
                    encode: photocraft_codecs::EncodeOptions {
                        avif_quality: settings.avif_quality,
                        avif_speed: settings.avif_speed,
                        avif_depth: settings.avif_depth,
                        avif_alpha_quality: settings.avif_alpha_quality,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                photocraft_io::export(doc, path, &options).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(move |path, bytes| {
                assert_eq!(photocraft_codecs::decode(bytes).unwrap().dimensions(), (16, 8));
                writer_calls.borrow_mut().push(format!("write {path}"));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.session.execute("file.new", json!({"width": 16, "height": 8})).unwrap();
        let mut fields = Map::new();
        set_format_defaults(&mut fields, "avif", &Default::default());
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let mut output = ctx.run_ui(Default::default(), |ui| body(&mut app, ui, &mut fields));
        output.textures_delta.clear();
        for name in ["quality", "avifSpeed", "avifAlphaQuality"] {
            assert!(fields[name].as_u64().is_some(), "{name} must be an integer after drawing");
        }
        let result = confirm(&mut app, &fields).unwrap();
        assert_eq!(result, json!({"fileDialog": "save"}));
        assert!(app.file_dialog_open());
        assert!(calls.borrow().is_empty(), "confirmation queues the dialog before exporting");
        app.poll_file_dialog(&ctx, None);
        assert!(!app.file_dialog_open());
        assert_eq!(&*calls.borrow(), &["pick Untitled.avif", "write chosen-destination.avif"]);
        assert!(app.ui.status.starts_with("Exported chosen-destination.avif"));
    }

    #[test]
    fn export_document_scales_and_flattens() {
        let doc = Document::with_background(
            "x",
            photocraft_doc::Size::new(200, 100),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let mut f = Map::new();
        f.insert("format".into(), json!("jpg"));
        f.insert("scale".into(), json!(50));
        let out = export_document(&doc, &f, None).unwrap();
        assert_eq!((out.size.width, out.size.height), (100, 50));
        assert_eq!(out.layers.len(), 1);
        let proxy = export_document(&doc, &f, Some(40)).unwrap();
        assert_eq!(proxy.size.width, 40);
        assert_eq!(settings(&f).jpeg_quality, Some(85));
    }

    /// #1353: a scale that changes only the height (the width rounds back to the original)
    /// used to be skipped, so the exported file kept the original size while the dialog
    /// labelled it as scaled.
    #[test]
    fn export_document_scales_when_only_the_height_changes() {
        let mut f = Map::new();
        f.insert("format".into(), json!("png"));
        f.insert("scale".into(), json!(50));
        for (src_h, want_h) in [(100u32, 50u32), (200, 100)] {
            let doc = Document::with_background(
                "x",
                photocraft_doc::Size::new(1, src_h),
                photocraft_doc::ColorMode::Rgb,
                photocraft_doc::SampleType::U8,
                photocraft_doc::Color::WHITE,
            );
            let out = export_document(&doc, &f, None).unwrap();
            assert_eq!((out.size.width, out.size.height), (1, want_h), "1 × {src_h} at 50%");
            // The preview proxy takes the same path.
            let proxy = export_document(&doc, &f, Some(40)).unwrap();
            assert_eq!((proxy.size.width, proxy.size.height), (1, 40), "proxy of 1 × {src_h}");
        }
        // The label and the export agree, and neither side drops below 1 px.
        assert_eq!(scaled_size(1.0, 100.0, 0.5), (1.0, 50.0));
        assert_eq!(scaled_size(10.0, 300.0, 0.01), (1.0, 3.0));
    }

    #[test]
    fn switching_export_formats_uses_independent_quality_preferences() {
        let prefs = photocraft_engine::prefs::Export { jpeg_quality: 43, webp_quality: 72, webp_lossless: false, ..Default::default() };
        let mut fields = Map::new();
        set_format_defaults(&mut fields, "webp", &prefs);
        assert_eq!(fields["quality"], 72);
        assert_eq!(fields["lossless"], false);
        let s = settings(&fields);
        assert!(!s.webp_lossless);
        assert_eq!(s.webp_quality, Some(72));
        assert_eq!(s.jpeg_quality, None);

        set_format_defaults(&mut fields, "jpg", &prefs);
        assert_eq!(fields["quality"], 43);
        assert_eq!(settings(&fields).jpeg_quality, Some(43));
        assert_eq!(settings(&fields).webp_quality, None);

        set_format_defaults(&mut fields, "webp", &photocraft_engine::prefs::Export::default());
        assert!(settings(&fields).webp_lossless, "default WebP Quick Export mode is lossless");
        assert_eq!(settings(&fields).webp_quality, None);
    }

    #[test]
    fn webp_settings_follow_the_lossless_switch() {
        let mut f = Map::new();
        f.insert("format".into(), json!("webp"));
        f.insert("quality".into(), json!(70));
        let s = settings(&f);
        assert!(!s.webp_lossless, "Export As writes lossy WebP unless asked");
        assert_eq!(s.webp_quality, Some(70));
        assert_eq!(s.jpeg_quality, None);
        f.insert("lossless".into(), json!(true));
        let s = settings(&f);
        assert!(s.webp_lossless);
        assert_eq!(s.webp_quality, None);
        f.insert("format".into(), json!("png"));
        assert!(settings(&f).webp_lossless, "other formats leave the WebP default alone");
    }
}
