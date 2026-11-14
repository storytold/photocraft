//! #13980: the batch Lens Correction dialog opens with the reference application's Correction
//! Options and File Type PSD, File Type offers JPG, PNG, PSD and TIFF, and the command keeps its
//! own defaults for scripts and the CLI.

use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::lens_cmds::{LENS, LENS_BATCH};
use serde_json::json;

use super::lens_batch_layout_tests::{H, entry, fields, fixture, harness, set, type_into};

/// Whether the checkbox `label` is ticked.
fn ticked(h: &H, label: &str) -> bool {
    h.get_by_role_and_label(Role::CheckBox, label).accesskit_node().toggled() == Some(Toggled::True)
}

/// The entry the dropdown `name` shows.
fn combo(h: &H, name: &str) -> Option<String> {
    h.get_by_role_and_label(Role::ComboBox, name).accesskit_node().value().map(|v| v.to_string())
}

#[test]
fn first_open_shows_the_reference_defaults() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let h = harness();
        let shown = ["Geometric Distortion", "Chromatic Aberration", "Vignette", "Auto Scale Image"].map(|l| ticked(&h, l));
        assert_eq!(shown, [true, false, false, true], "Geometric Distortion, Chromatic Aberration, Vignette, Auto Scale Image");
        assert_eq!(combo(&h, "Edge").as_deref(), Some("Transparency"));
        assert_eq!(combo(&h, "File Type").as_deref(), Some("PSD"));
        let params = crate::filter_dialog::params_of(fields(&h));
        assert_eq!(
            [&params["format"], &params["correctDistortion"], &params["correctCA"], &params["correctVignette"], &params["autoScale"], &params["edge"]],
            [&json!("psd"), &json!(true), &json!(false), &json!(false), &json!(true), &json!("transparency")],
            "OK sends what is shown"
        );
    });
}

#[test]
fn file_type_offers_jpg_png_psd_tiff_without_same() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        assert_eq!(fields(&h)["__choices"]["format"], json!(["jpg", "png", "psd", "tiff"]));
        h.get_by_role_and_label(Role::ComboBox, "File Type").click();
        h.run_steps(3);
        let tops = ["JPG", "PNG", "PSD", "TIFF"].map(|l| entry(&h, l).rect().top());
        assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "the entries run JPG, PNG, PSD, TIFF: {tops:?}");
        for gone in ["Same", "Png", "Jpg", "Psd", "Tiff"] {
            assert!(h.query_by_label(gone).is_none(), "no entry {gone}");
        }
    });
}

/// A field `ui.dialog.set` clears with `null` shows the new default, and OK sends it.
#[test]
fn a_null_field_shows_the_new_defaults() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        let dialog = h.state().ui.dialogs.last().expect("the dialog is open").id;
        for field in ["format", "correctCA", "correctVignette"] {
            let (req, _rx) = crate::ControlRequest::new("ui.dialog.set", json!({"dialog": dialog, "field": field, "value": null}));
            let ctx = h.ctx.clone();
            assert!(matches!(crate::control::handle(h.state_mut(), &ctx, &req), crate::control::Outcome::Done(_)));
        }
        h.run_steps(2);
        assert_eq!(combo(&h, "File Type").as_deref(), Some("PSD"));
        assert_eq!([ticked(&h, "Chromatic Aberration"), ticked(&h, "Vignette")], [false, false]);
        let params = crate::filter_dialog::params_of(fields(&h));
        assert_eq!(
            [&params["format"], &params["correctCA"], &params["correctVignette"]],
            [&json!("psd"), &json!(false), &json!(false)],
            "OK sends what is shown"
        );
    });
}

/// OK on the defaults writes PSDs into the destination folder.
#[test]
fn ok_on_the_defaults_saves_psd() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        let dir = fixture("defaults");
        set(&mut h, "input", json!([dir.join("a.png")]));
        type_into(&mut h, "Output", &dir.join("out").to_string_lossy());
        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(4);
        let written = dir.join("out").join("a.psd").exists();
        std::fs::remove_dir_all(&dir).ok();
        assert!(written, "the default File Type is PSD");
    });
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

    let dir = fixture("headless");
    let (input, output) = (dir.to_string_lossy().into_owned(), dir.join("out").to_string_lossy().into_owned());
    let r = photocraft_engine::Session::new().execute(LENS_BATCH, json!({"input": input, "output": output}));
    let written = dir.join("out").join("a.png").exists();
    std::fs::remove_dir_all(&dir).ok();
    assert!(r.is_ok(), "{r:?}");
    assert!(written, "without a format the batch keeps the source's format");
}
