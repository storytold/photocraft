//! #13683: Contact Sheet II shows its fields in the reference application's four sections, with
//! Mode, Bit Depth, Place and the resolution unit as lists, and Units converting the size.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, ParamKind};
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::PhotocraftApp;

type H = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

/// The open dialogs drawn over `app`, as the window shows them.
fn harness(app: Box<PhotocraftApp>) -> H {
    let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(900.0, 1000.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::dialogs::run(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// The app's dialogs with Contact Sheet II open from the menu, without a document (the command
/// needs none).
fn opened() -> (H, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(&mut app, &ctx, super::COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().map(str::to_owned).or_else(|| reply["dialog"].as_u64().map(|v| v.to_string())).expect("the reply names a dialog");
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|id| id.value().to_string() == want).expect("the dialog is open");
    (harness(app), id)
}

fn field(h: &H, id: DialogId, key: &str) -> Value {
    h.state()
        .framework
        .dialogs
        .get_by_id::<HostedParamDialog>(id)
        .and_then(|d| d.fields().get(key).and_then(|v| serde_json::to_value(v).ok()))
        .unwrap_or(Value::Null)
}

fn num(h: &H, id: DialogId, key: &str) -> f64 {
    field(h, id, key).as_f64().unwrap_or_else(|| panic!("{key} is a number"))
}

fn set(h: &mut H, id: DialogId, key: &str, value: Value) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
    h.run_steps(3);
}

/// The options of the list row `key`, as `(id, label)`.
fn options(key: &str) -> Vec<(String, String)> {
    let rows = super::rows(".".into());
    let row = rows.iter().find(|p| p.key == key).unwrap_or_else(|| panic!("a {key} row"));
    let ParamKind::Choice { options, .. } = &row.kind else { panic!("{key} is a list") };
    options.iter().map(|(id, label)| (id.clone(), label.as_str().to_owned())).collect()
}

#[test]
fn the_four_sections_and_their_rows_follow_the_reference_order() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (h, _) = opened();
        let top = |label: &str| h.query_all_by_label(label).map(|n| n.rect().top()).reduce(f32::min).unwrap_or_else(|| panic!("no {label}"));
        // Each section's title, then its rows, top to bottom. "Use Filename as Caption" is the last section's title and
        // its checkbox.
        let order = [
            "Source Images",
            "Input",
            "Include Subfolders",
            "Group Images by Folder",
            "Document",
            "Units",
            "Width",
            "Height",
            "Resolution",
            "Flatten All Layers",
            "Mode",
            "Bit Depth",
            "Thumbnails",
            "Place",
            "Columns",
            "Rows",
            "Rotate For Best Fit",
            "Use Auto-Spacing",
            "Use Filename as Caption",
            "Font",
            "Font size",
        ];
        for pair in order.windows(2) {
            assert!(top(pair[0]) < top(pair[1]), "order: {pair:?}");
        }
    });
}

#[test]
fn mode_bit_depth_place_and_units_list_the_reference_entries() {
    let labels = |key: &str| options(key).into_iter().map(|(_, label)| label).collect::<Vec<_>>();
    assert_eq!(labels("units"), ["inches", "cm", "pixels"]);
    assert_eq!(labels("mode"), ["Grayscale", "RGB Color", "CMYK Color", "Lab Color"]);
    assert_eq!(labels("depth"), ["8-bit", "16-bit"]);
    assert_eq!(labels("place"), ["across first", "down first"]);
    let rows = super::rows(".".into());
    let resolution = rows.iter().find(|p| p.key == "resolution").expect("a Resolution row");
    assert_eq!(resolution.unit_key(), Some("resolution.unit"), "Resolution has its own unit list");
}

#[test]
fn switching_units_converts_width_and_height() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (mut h, id) = opened();
        let size = |h: &H| (num(h, id, "width"), num(h, id, "height"));
        let close = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
        assert!(close(size(&h), (8.0, 10.0)));
        for (unit, want) in [("cm", (20.32, 25.4)), ("pixels", (2400.0, 3000.0)), ("inches", (8.0, 10.0)), ("pixels", (2400.0, 3000.0)), ("cm", (20.32, 25.4))]
        {
            pick_unit(&mut h, id, unit);
            assert_eq!(field(&h, id, "units"), json!(unit));
            assert!(close(size(&h), want), "{unit}: {:?}", size(&h));
        }
        assert_eq!(num(&h, id, "resolution"), 300.0, "the resolution stays");
    });
}

/// A pixel size too small for two decimals in inches survives the switch to inches and back.
#[test]
fn switching_units_keeps_a_small_pixel_size() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (mut h, id) = opened();
        pick_unit(&mut h, id, "pixels");
        set(&mut h, id, "resolution", json!(10_000.0));
        set(&mut h, id, "width", json!(16.0));
        set(&mut h, id, "height", json!(16.0));
        pick_unit(&mut h, id, "inches");
        assert!(num(&h, id, "width") > 0.0 && num(&h, id, "height") > 0.0, "the size is not lost in inches");
        pick_unit(&mut h, id, "pixels");
        assert_eq!((num(&h, id, "width"), num(&h, id, "height")), (16.0, 16.0));
    });
}

/// Picks `unit` in the Units list, as the list's pick does.
fn pick_unit(h: &mut H, id: DialogId, unit: &str) {
    set(h, id, "units", json!(unit));
}

/// OK sends the params the form sent before the layout: the resolution in pixels/inch whatever unit it is shown in,
/// and `placeAcrossFirst` and `depth` as before.
#[test]
fn ok_runs_the_command_with_the_same_params() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (mut h, id) = opened();
        let params = |h: &H| {
            let fields = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields().clone();
            crate::filter_dialog::command_params(super::COMMAND, &fields)
        };
        let before = params(&h);
        for (key, want) in [
            ("input", json!(".")),
            ("units", json!("inches")),
            ("width", json!(8.0)),
            ("height", json!(10.0)),
            ("resolution", json!(300.0)),
            ("mode", json!("rgb")),
            ("depth", json!(8)),
            ("columns", json!(5)),
            ("rows", json!(6)),
            ("placeAcrossFirst", json!(true)),
            ("autoSpacing", json!(true)),
            ("rotateForBestFit", json!(false)),
            ("caption", json!(true)),
            ("fontSize", json!(12.0)),
            ("flatten", json!(false)),
            ("includeSubfolders", json!(true)),
            ("groupByFolder", json!(false)),
        ] {
            assert_eq!(before.get(key), Some(&want), "{key}: {before}");
        }
        assert!(before.get("resolution.unit").is_none() && before.get("place").is_none(), "only the command's keys: {before}");
        set(&mut h, id, "resolution.unit", json!("cm"));
        assert_eq!(params(&h), before, "the resolution unit is view state");
        set(&mut h, id, "depth", json!("16"));
        set(&mut h, id, "place", json!("down"));
        let p = params(&h);
        assert_eq!((&p["depth"], &p["placeAcrossFirst"]), (&json!(16), &json!(false)));

        let dir = tempfile::tempdir().unwrap();
        let mut image = photocraft_engine::Session::new();
        image.execute("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
        let png = photocraft_io::export(&image.active().unwrap().doc, "a.png", &Default::default()).unwrap().bytes;
        std::fs::write(dir.path().join("a.png"), png).unwrap();
        set(&mut h, id, "input", json!(dir.path().to_string_lossy()));
        set(&mut h, id, "caption", json!(false));
        set(&mut h, id, "depth", json!("8"));
        crate::dialogs::testing::confirm(h.state_mut(), id).expect("OK is accepted");
        h.run_steps(4);
        let app = h.state();
        assert!(app.framework.dialogs.is_empty(), "OK closes the dialog");
        assert_eq!(app.session.documents().len(), 1, "one sheet");
        let sheet = &app.session.active().unwrap().doc;
        assert_eq!((sheet.size.width, sheet.size.height, sheet.resolution_dpi), (2400, 3000, 300.0));
    });
}
