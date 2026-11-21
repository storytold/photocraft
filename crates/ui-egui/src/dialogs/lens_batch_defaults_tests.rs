//! #13980: the batch Lens Correction dialog opens with the reference defaults (Chromatic
//! Aberration and Vignette off, File Type PSD) and File Type offers JPG, PNG, PSD and TIFF. The
//! command's own defaults for scripts and the CLI are unchanged.

use craft::ui::patterns::form_dialogs::ParamKind;
use serde_json::json;

use super::lens_batch_layout_tests::{field, opened, set, source_folder};

const LENS: &str = photocraft_engine::lens_cmds::LENS;
const LENS_BATCH: &str = photocraft_engine::lens_cmds::LENS_BATCH;

#[test]
fn first_open_shows_the_reference_defaults() {
    let (h, id) = opened();
    let shown = ["correctDistortion", "correctCA", "correctVignette", "autoScale"].map(|key| field(&h, id, key));
    assert_eq!(shown, [json!(true), json!(false), json!(false), json!(true)], "Geometric Distortion, Chromatic Aberration, Vignette, Auto Scale Image");
    assert_eq!(field(&h, id, "edge"), json!("transparency"));
    assert_eq!(field(&h, id, "format"), json!("psd"));
}

#[test]
fn file_type_offers_jpg_png_psd_tiff_without_same() {
    let rows = super::rows(".".into());
    let row = rows.iter().find(|p| p.key == "format").expect("a File Type row");
    let ParamKind::Choice { options, .. } = &row.kind else { panic!("File Type is a list") };
    let entries: Vec<(&str, &str)> = options.iter().map(|(id, label)| (id.as_str(), label.as_str())).collect();
    assert_eq!(entries, [("jpg", "JPG"), ("png", "PNG"), ("psd", "PSD"), ("tiff", "TIFF")]);
}

/// OK on the defaults writes PSDs into the destination folder.
#[test]
fn ok_on_the_defaults_saves_psd() {
    let (mut h, id) = opened();
    let dir = source_folder();
    let out = dir.path().join("out");
    set(&mut h, id, "input", json!(dir.path().to_string_lossy()));
    set(&mut h, id, "output", json!(out.to_string_lossy()));
    crate::dialogs::testing::confirm(h.state_mut(), id).expect("OK is accepted");
    h.run_steps(4);
    assert!(out.join("a.psd").exists(), "the default File Type is PSD");
}

/// Scripts and the CLI keep the command's own defaults: Chromatic Aberration and Vignette on, the
/// source's own format.
#[test]
fn the_command_keeps_its_own_defaults() {
    let lc = photocraft_engine::lens_cmds::lens_params(LENS, &json!({}), photocraft_geom::Rect::new(0, 0, 40, 20), None).unwrap();
    assert!(lc.correct_distortion && lc.correct_ca && lc.correct_vignette, "corrections default to on");
    let doc = photocraft_engine::commands::find(LENS).unwrap().params;
    for key in [r#""correctVignette":bool=true"#, r#""correctCA":bool=true"#] {
        assert!(doc.contains(key), "{key} in {doc}");
    }

    let dir = source_folder();
    let out = dir.path().join("out");
    let r = photocraft_engine::Session::new().execute(LENS_BATCH, json!({"input": dir.path().to_string_lossy(), "output": out.to_string_lossy()}));
    assert!(r.is_ok(), "{r:?}");
    assert!(out.join("a.png").exists(), "without a format the batch keeps the source's format");
}
