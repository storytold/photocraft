//! Camera Raw textures use the same ICC display transform as the main canvas.
//! Filtering, sampling and histograms stay in the document's original working space.
use egui::{Color32, ColorImage};
use photocraft_cms::Transform;
use std::sync::Arc;

/// The proxy contains document values, not the main canvas's pre-encoded texture values.
/// In particular, linear RGB must be transformed directly rather than using the canvas's
/// sRGB-curve twin as the source profile. Monitor and soft-proof policy remain shared.
pub(crate) fn transform(session: &photocraft_engine::Session, doc: &photocraft_doc::Document) -> Result<Arc<Transform>, String> {
    use photocraft_engine::display_color::{DISPLAY_BPC, DISPLAY_INTENT};
    let source = photocraft_engine::color_cmds::composite_profile(doc);
    let monitor = session.color.monitor();
    let proof = session.color.proof(doc.id);
    let transform = if proof.enabled {
        Transform::proof(&source, &proof.setup.profile, &monitor, proof.setup.intent, proof.setup.bpc, proof.setup.simulate_paper)
    } else {
        Transform::new(&source, &monitor, DISPLAY_INTENT, DISPLAY_BPC)
    };
    transform.map(Arc::new).map_err(|e| e.to_string())
}

pub(crate) fn display_pixel(pixel: &[f32; 4], transform: &Transform) -> Color32 {
    let [r, g, b, alpha] = *pixel;
    let mut display = [0.0; 3];
    transform.eval_fast(&[r, g, b], &mut display);
    let enc = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let [r, g, b] = display.map(enc);
    Color32::from_rgba_unmultiplied(r, g, b, enc(alpha))
}

pub(crate) fn image(pixels: &[[f32; 4]], width: usize, height: usize, transform: &Transform) -> ColorImage {
    #[cfg(not(target_arch = "wasm32"))]
    let converted = {
        use rayon::prelude::*;
        pixels.par_iter().with_min_len(4096).map(|p| display_pixel(p, transform)).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let converted = pixels.iter().map(|p| display_pixel(p, transform)).collect();
    ColorImage::new([width, height], converted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_cms::{Builtin, cached};

    #[test]
    fn prophoto_middle_grey_is_not_misread_as_srgb() {
        let transform = cached(Builtin::ProPhotoCompat.profile(), Builtin::Srgb.profile(), Default::default()).unwrap();
        let v = 0.18f32.powf(1.0 / 1.8);
        let source = [[v, v, v, 1.0]];
        let result = image(&source, 1, 1, &transform);
        let grey = result.pixels[0];
        // 18% linear grey is approximately 118/255 in sRGB, not ProPhoto's 98/255.
        for c in [grey.r(), grey.g(), grey.b()] {
            assert!((i32::from(c) - 118).abs() <= 1);
        }
        assert_eq!(source[0], [v, v, v, 1.0], "display conversion never edits the source");
    }

    #[test]
    fn srgb_preview_and_alpha_keep_their_values() {
        let transform = cached(Builtin::Srgb.profile(), Builtin::Srgb.profile(), Default::default()).unwrap();
        let color = display_pixel(&[0.2, 0.4, 0.8, 0.5], &transform);
        let expected = Color32::from_rgba_unmultiplied(51, 102, 204, 128);
        assert_eq!(color, expected);
    }

    #[test]
    fn linear_float_document_is_encoded_exactly_once() {
        use photocraft_color::{Color, ColorMode, SampleType};
        let session = photocraft_engine::Session::new();
        let mut doc = photocraft_doc::Document::with_background(
            "linear",
            photocraft_doc::Size::new(1, 1),
            ColorMode::Rgb,
            SampleType::F32,
            Color::rgba(0.18, 0.18, 0.18, 1.0),
        );
        doc.icc_profile = Some(Arc::new(Builtin::LinearSrgb.profile().to_bytes().to_vec()));
        let transform = transform(&session, &doc).unwrap();
        let grey = display_pixel(&[0.18, 0.18, 0.18, 1.0], &transform);
        assert!((i32::from(grey.r()) - 118).abs() <= 1);
    }

    #[test]
    fn dialog_keeps_working_pixels_and_refreshes_its_monitor_transform() {
        use crate::{PhotocraftApp, camera_raw_ui};
        use photocraft_engine::Session;
        use serde_json::json;
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let mut app = PhotocraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width":4,"height":4,"depth":16})).unwrap();
        app.run("edit.fill", json!({"color":"#626262"})).unwrap();
        app.run("edit.assignProfile", json!({"profile":"prophoto-compat"})).unwrap();
        app.run("edit.colorSettings", json!({"monitorProfile":"srgb"})).unwrap();
        camera_raw_ui::open(&mut app, &ctx).unwrap();
        let d = app.camera_raw.as_ref().unwrap();
        let source = d.proxy.clone();
        let params = d.params.clone();
        let display = image(&source, d.pw, d.ph, &d.display_transform);
        assert!(display.pixels[0].r() > 110, "ProPhoto midtones must be converted for display");
        assert_eq!(d.processed, source, "display conversion is never baked into document pixels");
        let signature = d.display_signature;
        app.run("edit.colorSettings", json!({"monitorProfile":"display-p3"})).unwrap();
        ctx.run_ui(Default::default(), |ui| camera_raw_ui::show(&mut app, ui.ctx())).textures_delta.clear();
        let d = app.camera_raw.as_ref().unwrap();
        assert_ne!(d.display_signature, signature);
        assert_eq!(d.params, params);
        assert_eq!(d.proxy, source);
    }
}
