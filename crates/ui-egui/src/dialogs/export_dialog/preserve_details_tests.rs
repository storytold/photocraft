//! #13737: Export As's Resample lists seven methods in the reference order, Preserve Details
//! last, with Bicubic Automatic still the default. Preserve Details is Image Size's
//! `preserveDetails` resampler; presets and recorded steps accept it.

use super::*;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::{Color, SampleType, Size};
use photocraft_engine::export_preset_cmds::RESAMPLE;
use photocraft_geom::Rect;

/// 12 x 8 RGB/8 with a one-pixel checker, so resamplers differ when it is enlarged.
fn checker() -> Document {
    let mut doc = Document::with_background("Checker", Size::new(12, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let surface = doc.layers[0].surface_mut().unwrap();
    let black = photocraft_raster::from_rgba(&surface.format(), [0.0, 0.0, 0.0, 1.0]);
    for y in 0..8 {
        for x in (y % 2..12).step_by(2) {
            surface.fill_rect(Rect::new(x, y, x + 1, y + 1), &black);
        }
    }
    doc
}

fn pixels(doc: &Document) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(doc).into_px()
}

fn export_with(resample: &str) -> Document {
    let doc = checker();
    let f = json!({"scale": 300, "resample": resample, "profile": "document"}).as_object().unwrap().clone();
    export_document(&doc, &f, None, &photocraft_engine::color_cmds::document_profile(&doc)).unwrap()
}

#[test]
fn seven_methods_in_the_reference_order_with_bicubic_automatic_the_default() {
    let keys: Vec<&str> = RESAMPLE.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, ["bicubic", "bicubicAutomatic", "bicubicSharper", "bicubicSmoother", "bilinear", "nearest", "preserveDetails"]);
    let labels: Vec<&str> = RESAMPLE.iter().map(|(_, l)| *l).collect();
    assert_eq!(labels, ["Bicubic", "Bicubic Automatic", "Bicubic Sharper", "Bicubic Smoother", "Bilinear", "Nearest Neighbor", "Preserve Details"]);
    assert_eq!(DEFAULT_RESAMPLE, "bicubicAutomatic");
    assert_eq!(resample_method(&Map::new(), None).unwrap(), "bicubicAutomatic");
    // The dialog lists the methods in the engine's order, Automatic included.
    let offered: Vec<String> = RESAMPLE_CHOICES
        .iter()
        .map(|resample| {
            let mut options = ExportOptions::default();
            options.resample = *resample;
            settings_from(&options, PixelSize::new(12, 8))["resample"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(offered, keys);
}

#[test]
fn preserve_details_is_image_sizes_resampler_and_differs_from_bicubic() {
    let preserve = export_with("preserveDetails");
    assert_eq!((preserve.size.width, preserve.size.height), (36, 24));
    let mut s = photocraft_engine::Session::new();
    s.add_document(checker(), None);
    s.execute("image.imageSize", json!({"width": 36, "height": 24, "resample": "preserveDetails"})).unwrap();
    assert_eq!(pixels(&preserve), pixels(&s.active().unwrap().doc), "the same pixels as Image Size's Preserve Details");
    assert_ne!(pixels(&preserve), pixels(&export_with("bicubic")), "an enlarged export differs from Bicubic");
}

#[test]
fn presets_and_recorded_steps_accept_preserve_details_and_unknown_names_still_fail() {
    assert!(valid_setting("resample", &json!("preserveDetails")));
    let mut s = photocraft_engine::Session::new();
    s.execute("file.export.presets.save", json!({"name": "Detail", "settings": {"format": "png", "resample": "preserveDetails"}})).unwrap();
    let err = photocraft_engine::export_preset_cmds::export_resample("lanczos").unwrap_err();
    assert!(err.contains("preserveDetails") && err.contains("bicubicAutomatic"), "{err}");
    assert!(!valid_setting("resample", &json!("lanczos")));
}

/// The framework's Export As dialog offers Preserve Details (the host hands it `RESAMPLE_CHOICES`), and picking it
/// sets `resample` to `preserveDetails` in the settings the export, a preset and a recorded step read.
#[test]
fn the_dialog_offers_preserve_details_and_it_sets_preserve_details() {
    let services =
        crate::Services { export: Some(Box::new(|_, _, _| Ok((Vec::new(), Vec::new())))), write: Some(Box::new(|_, _| Ok(()))), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.session.add_document(checker(), None);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    open(h.state_mut()).unwrap();
    h.run_steps(4);
    h.get_by_role_and_label(Role::ComboBox, "Resample").click();
    h.run_steps(4);
    h.get_by_label("Preserve Details").click();
    h.run_steps(4);
    let options = h.state().framework.dialogs.get::<HostedExportAs>().expect("Export As is open").options().clone();
    assert_eq!(options.resample, Resample::PreserveDetails);
    assert_eq!(settings_from(&options, PixelSize::new(12, 8))["resample"], "preserveDetails");
    assert_eq!(options_of(&settings_from(&options, PixelSize::new(12, 8))).resample, Resample::PreserveDetails, "and reads back");
}
