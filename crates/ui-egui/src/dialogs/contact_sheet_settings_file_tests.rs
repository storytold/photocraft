//! #13698: Contact Sheet II's Load…, Save… and Reset…, and the settings file they read and write.

use craft::ui::dialog::{DialogId, StackDialog, StandardPrompt};
use serde_json::{Map, Value, json};

use super::super::contact_sheet_layout_tests::{H, field, harness, set};
use super::{RES_UNIT, Stamp, defaults, read, spacing_px, write};
use crate::PhotocraftApp;

/// The PostScript name of the default caption font's regular face.
const INTER: &str = "Inter-Regular";

fn stamp() -> Stamp {
    Stamp { date: "2026-11-10T22:30:47".into(), version: "1.0".into(), locale: "en_GB".into() }
}

fn map(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

/// The dialog's fields as a fresh dialog has them.
fn fresh() -> Map<String, Value> {
    map(defaults())
}

/// The reference application's file for the measured Load… example.
fn reference_load_file() -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<ContactSheetIISettings>
  <source imageSource="Folder" path="C:\some\folder\in" includeSubfolders="false" groupImages="false"/>
  <document units="pixels" width="500" height="600" resolution="100" resUnits="pixels/inch" mode="Grayscale" bitDepth="16-bit" colorProfile="Working Gray" flattenLayers="false"/>
  <thumbnail place="down first" cols="3" rows="4" bestFit="true" useAutospacing="false" horz="10 px" vert="20 px"/>
  <caption enabled="true"><font name="{INTER}" size="20"/></caption>
</ContactSheetIISettings>
"#
    )
}

/// What the reference load example sets, as the dialog keeps it.
fn reference_load_values() -> Map<String, Value> {
    map(json!({
        "input": r"C:\some\folder\in", "includeSubfolders": false, "groupByFolder": false, "units": "pixels", "width": 500, "height": 600, "resolution": 100, RES_UNIT: "in",
        "mode": "gray", "depth": 16, "flatten": false, "placeAcrossFirst": false, "columns": 3, "rows": 4,
        "rotateForBestFit": true, "autoSpacing": false, "caption": true, "font": photocraft_text::fonts::DEFAULT_FAMILY, "fontSize": 20,
    }))
}

/// The values of `f` the dialog has a row for: the spacing (#13689) and other keys are left out.
fn settings(f: &Map<String, Value>) -> Map<String, Value> {
    const KEYS: [&str; 19] = [
        "input",
        "includeSubfolders",
        "groupByFolder",
        "units",
        "width",
        "height",
        "resolution",
        RES_UNIT,
        "mode",
        "depth",
        "flatten",
        "placeAcrossFirst",
        "columns",
        "rows",
        "rotateForBestFit",
        "autoSpacing",
        "caption",
        "font",
        "fontSize",
    ];
    f.iter().filter(|(k, _)| KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// The numbers of `a` and `b` equal within float noise, the rest exactly.
fn same(a: &Map<String, Value>, b: &Map<String, Value>) -> bool {
    a.len() == b.len()
        && a.iter().all(|(k, v)| match (v.as_f64(), b.get(k).and_then(Value::as_f64)) {
            (Some(x), Some(y)) => (x - y).abs() < 1e-9,
            _ => b.get(k) == Some(v),
        })
}

#[test]
fn the_measured_document_and_caption_are_written_as_the_reference_writes_them() {
    // The reference's measured file: 96.67 x 10 in at 472.44 pixels/cm, 100 x 100, 72 pt.
    let f = map(json!({
        "input": "", "units": "inches", "width": 290.0 / 3.0, "height": 10, "resolution": 1200, RES_UNIT: "cm", "mode": "rgb", "depth": 8,
        "flatten": true, "placeAcrossFirst": true, "columns": 100, "rows": 100, "rotateForBestFit": false, "autoSpacing": false,
        "caption": true, "font": photocraft_text::fonts::DEFAULT_FAMILY, "fontSize": 72,
    }));
    let want = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<ContactSheetIISettings>
  <source imageSource="Folder" path="" includeSubfolders="true" groupImages="false"/>
  <document units="inches" width="96.6666666666667" height="10" resolution="472.44094488189" resUnits="pixels/cm" mode="RGB Color" bitDepth="8-bit" colorProfile="sRGB IEC61966-2.1" flattenLayers="true"/>
  <thumbnail place="across first" cols="100" rows="100" bestFit="false" useAutospacing="false" horz="0.014 in" vert="0.014 in"/>
  <caption enabled="true">
    <font name="{INTER}" size="72"/>
  </caption>
  <date>2026-11-10T22:30:47</date>
  <version>1.0</version>
  <locale>en_GB</locale>
</ContactSheetIISettings>
"#
    );
    assert_eq!(write(&f, &stamp()), want);
}

#[test]
fn what_is_written_reads_back_as_the_same_values() {
    let defaults = settings(&fresh());
    let edited = [
        reference_load_values(),
        map(json!({
            "input": "/tmp/a & b/\"sheets\" <1>\nline\r2\tend", "includeSubfolders": true, "groupByFolder": true, "units": "cm", "width": 20.32, "height": 25.4, "resolution": 300, RES_UNIT: "cm",
            "mode": "lab", "depth": 8, "flatten": true, "placeAcrossFirst": true, "columns": 100, "rows": 1,
            "rotateForBestFit": false, "autoSpacing": true, "caption": false, "font": photocraft_text::fonts::DEFAULT_FAMILY, "fontSize": 12.5,
        })),
    ];
    for f in edited {
        let read = read(&write(&f, &stamp()), &fresh()).expect("a settings file");
        assert!(same(&settings(&read), &f), "read back {:?}, wrote {f:?}", settings(&read));
        assert_ne!(settings(&read), defaults);
    }
}

#[test]
fn the_reference_load_example_sets_every_field_and_reads_the_spacing_in_pixels() {
    let read = read(&reference_load_file(), &fresh()).expect("a settings file");
    assert!(same(&settings(&read), &reference_load_values()), "{read:?}");
    // In the sheet's units (pixels here), for the spacing fields of #13689.
    assert_eq!((read["horizontal"].as_f64(), read["vertical"].as_f64()), (Some(10.0), Some(20.0)));
}

/// The reference application's measured Save… file, whole.
#[test]
fn the_reference_save_example_loads_back() {
    let xml = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<ContactSheetIISettings>
  <source imageSource="Files" path="" includeSubfolders="true" groupImages="false"/>
  <document units="inches" width="96.6666666666667" height="10" resolution="472.44094488189" resUnits="pixels/cm" mode="RGB Color" bitDepth="8-bit" colorProfile="sRGB IEC61966-2.1" flattenLayers="true"/>
  <thumbnail place="across first" cols="100" rows="100" bestFit="false" useAutospacing="false" horz="0.014 in" vert="0.111 in"/>
  <caption enabled="true">
    <font name="{INTER}" size="72"/>
  </caption>
  <date>2026-11-10T22:30:47</date>
  <version>(script version string)</version>
  <locale>en_GB</locale>
</ContactSheetIISettings>
"#
    );
    let mut current = reference_load_values();
    current.insert("input".into(), json!("/current/folder"));
    let read = read(&xml, &current).expect("a settings file");
    let kept = settings(&read);
    let n = |key: &str| kept[key].as_f64().unwrap_or(f64::NAN);
    assert!((n("resolution") - 1200.0).abs() < 1e-6 && (n("width") - 96.6666666666667).abs() < 1e-9, "{kept:?}");
    let rest = [
        "input",
        "includeSubfolders",
        "groupByFolder",
        RES_UNIT,
        "units",
        "height",
        "columns",
        "rows",
        "flatten",
        "placeAcrossFirst",
        "rotateForBestFit",
        "autoSpacing",
        "caption",
        "fontSize",
        "mode",
        "depth",
    ]
    .map(|k| kept[k].clone());
    let want = [
        json!("/current/folder"),
        json!(true),
        json!(false),
        json!("cm"),
        json!("inches"),
        json!(10.0),
        json!(100.0),
        json!(100.0),
        json!(true),
        json!(true),
        json!(false),
        json!(false),
        json!(true),
        json!(72.0),
        json!("rgb"),
        json!(8),
    ];
    assert_eq!(rest, want, "Use = Files keeps the current folder; the profile is not read");
    // The spacing is read (8 px at 1200 ppi, in inches) but has no field yet (#13689).
    assert!((read["vertical"].as_f64().unwrap_or(f64::NAN) - 8.0 / 1200.0).abs() < 1e-12, "{read:?}");
    assert!(!kept.contains_key("vertical") && !kept.contains_key("horizontal"));
}

#[test]
fn units_the_dropdown_does_not_list_are_written_as_inches() {
    let mut f = fresh();
    f.extend(map(json!({"units": "mm", "width": 203.2, "height": 254, "resolution": 300})));
    let xml = write(&f, &stamp());
    assert!(xml.contains(r#"units="inches" width="8" height="10""#), "{xml}");
}

#[test]
fn write_fills_what_the_fields_lack_with_the_defaults() {
    assert_eq!(write(&Map::new(), &stamp()), write(&fresh(), &stamp()));
}

#[test]
fn the_stamp_has_a_date_and_a_locale() {
    let stamp = Stamp::now();
    let d = stamp.date.as_bytes();
    let shape = d.len() == 19
        && d.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            _ => c.is_ascii_digit(),
        });
    assert!(shape, "{}", stamp.date);
    assert!(!stamp.locale.is_empty() && !stamp.version.is_empty());
}

#[test]
fn spacing_is_read_in_inches_at_72_px_per_inch_or_in_pixels() {
    assert_eq!(["0.111 in", "0.014 in", "20 px", " 7.5 px "].map(spacing_px), [Some(8.0), Some(1.0), Some(20.0), Some(8.0)]);
    assert_eq!(["20", "0,111 in", "1 cm", "abc px", ""].map(spacing_px), [None; 5]);
    // `vert="20 px"` on an 8 x 10 in sheet at 300 ppi: 20 px in inches, not 20 / 72 in.
    let xml = r#"<ContactSheetIISettings><document units="inches" width="8" height="10"/><thumbnail vert="20 px" horz="0.111 in"/></ContactSheetIISettings>"#;
    let read = read(xml, &fresh()).expect("a settings file");
    let inches = |key: &str| read[key].as_f64().unwrap_or(f64::NAN);
    assert!((inches("vertical") - 20.0 / 300.0).abs() < 1e-12 && (inches("horizontal") - 8.0 / 300.0).abs() < 1e-12, "{read:?}");
}

#[test]
fn a_missing_unknown_or_broken_value_keeps_the_current_one() {
    let current = reference_load_values();
    let xml = r#"<ContactSheetIISettings>
  <source imageSource="Files" path="/elsewhere" includeSubfolders="maybe" groupImages="1"/>
  <document units="furlongs" width="1" height="2" resolution="1,5" resUnits="pixels/mile" mode="Purple" bitDepth="32-bit" flattenLayers="yes"/>
  <thumbnail place="sideways" cols="abc" rows="NaN" bestFit="1" useAutospacing=""/>
  <caption><font name="NoSuchFont-Bold" size="inf"/></caption>
</ContactSheetIISettings>"#;
    let read = read(xml, &current).expect("a settings file");
    assert_eq!(read, current, "nothing in the file is a value the dialog takes");
    // Units with a broken width keep all three; an empty file keeps everything.
    let half = r#"<ContactSheetIISettings><document units="cm" width="x" height="3"/></ContactSheetIISettings>"#;
    assert_eq!(super::read(half, &current), Ok(current.clone()));
    assert_eq!(super::read("<ContactSheetIISettings/>", &current), Ok(current.clone()));
    // A resolution the dialog does not take gives no spacing, not an infinite one.
    let zero = r#"<ContactSheetIISettings><document resolution="0"/><thumbnail vert="20 px"/></ContactSheetIISettings>"#;
    let read = super::read(zero, &current).expect("a settings file");
    assert!(!read.contains_key("vertical") && read["resolution"] == json!(0.0), "{read:?}");
}

#[test]
fn a_file_that_is_not_a_settings_file_is_an_error() {
    for xml in ["", "not xml at all", "<ContactSheetIISettings>", "<OtherSettings mode=\"Grayscale\"/>"] {
        assert!(read(xml, &fresh()).is_err(), "{xml:?}");
    }
}

/// The dialog open from the menu, on a fresh app.
fn opened() -> (H, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(&mut app, &ctx, super::super::COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().map(str::to_owned).or_else(|| reply["dialog"].as_u64().map(|v| v.to_string())).expect("the reply names a dialog");
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|id| id.value().to_string() == want).expect("the dialog is open");
    (harness(app), id)
}

/// Loads `xml` into dialog `id`, as Load…'s picked file does.
fn load(h: &mut H, id: DialogId, xml: &str) {
    let ctx = h.ctx.clone();
    super::super::load(h.state_mut(), &ctx, id, "Contact Sheet II.xml", xml.as_bytes());
    h.run_steps(3);
}

/// The open alerts' messages.
fn alerts(h: &H) -> Vec<String> {
    let dialogs = &h.state().framework.dialogs;
    dialogs
        .open_dialogs()
        .into_iter()
        .filter_map(|d| dialogs.get_by_id::<craft::ui::dialog::HostedAlert>(d.id)?.json_fields()?.get("message")?.as_str().map(str::to_owned))
        .collect()
}

#[test]
fn the_dialog_has_load_save_and_reset_buttons() {
    let labels: Vec<String> = super::super::footer_buttons().into_iter().map(|(id, label)| format!("{id}:{}", label.as_str())).collect();
    assert_eq!(labels, ["load:Load…", "save:Save…", "reset:Reset…"]);
    let (h, _) = opened();
    for label in ["Load…", "Save…", "Reset…"] {
        assert!(egui_kittest::kittest::Queryable::query_all_by_label(&h, label).next().is_some(), "{label} in the footer");
    }
}

#[test]
fn save_writes_the_dialog_values_and_load_reads_them_back() {
    let (mut h, id) = opened();
    for (key, value) in [("input", json!("/tmp/sheets")), ("mode", json!("lab")), ("depth", json!("16")), ("place", json!("down")), ("columns", json!(3))] {
        set(&mut h, id, key, value);
    }
    let xml = super::super::settings_text(h.state(), id).expect("the dialog is open");
    assert!(xml.contains(r#"path="/tmp/sheets""#) && xml.contains(r#"mode="Lab Color""#) && xml.contains(r#"bitDepth="16-bit""#), "{xml}");
    assert!(xml.contains(r#"place="down first""#) && xml.contains(r#"cols="3""#), "{xml}");
    let (mut h, id) = opened();
    load(&mut h, id, &xml);
    let shown = ["input", "mode", "depth", "place", "columns"].map(|key| field(&h, id, key));
    assert_eq!(shown, [json!("/tmp/sheets"), json!("lab"), json!("16"), json!("down"), json!(3)]);
}

#[test]
fn loading_the_reference_example_shows_every_value() {
    let (mut h, id) = opened();
    load(&mut h, id, &reference_load_file());
    let want = [
        ("input", json!(r"C:\some\folder\in")),
        ("includeSubfolders", json!(false)),
        ("units", json!("pixels")),
        ("width", json!(500.0)),
        ("height", json!(600.0)),
        ("resolution", json!(100.0)),
        ("mode", json!("gray")),
        ("depth", json!("16")),
        ("place", json!("down")),
        ("columns", json!(3)),
        ("rows", json!(4)),
        ("rotateForBestFit", json!(true)),
        ("autoSpacing", json!(false)),
        ("fontSize", json!(20.0)),
    ];
    for (key, value) in want {
        assert_eq!(field(&h, id, key), value, "{key}");
    }
}

#[test]
fn a_file_that_is_not_xml_shows_an_alert_and_changes_nothing() {
    let (mut h, id) = opened();
    let before = super::super::settings_text(h.state(), id);
    load(&mut h, id, "not xml at all");
    assert_eq!(alerts(&h).len(), 1, "{:?}", alerts(&h));
    let after = super::super::settings_text(h.state(), id);
    let strip = |t: Option<String>| t.map(|t| t.split("<date>").next().unwrap_or_default().to_owned());
    assert_eq!(strip(after), strip(before));
}

#[test]
fn reset_asks_first_and_yes_restores_the_defaults_with_no_folder() {
    for yes in [false, true] {
        let (mut h, id) = opened();
        set(&mut h, id, "input", json!("/tmp/sheets"));
        set(&mut h, id, "columns", json!(3));
        set(&mut h, id, "units", json!("pixels"));
        let ctx = h.ctx.clone();
        super::super::footer(h.state_mut(), &ctx, id, "reset");
        let prompt = h.state().framework.dialogs.get::<StandardPrompt>().map(StackDialog::id).expect("Reset… asks first");
        if yes {
            crate::dialogs::testing::confirm(h.state_mut(), prompt).expect("Yes");
        } else {
            crate::dialogs::testing::cancel(h.state_mut(), prompt).expect("No");
        }
        h.run_steps(3);
        let shown = ["input", "columns", "units"].map(|key| field(&h, id, key));
        if yes {
            assert_eq!(shown, [json!(""), json!(5), json!("inches")], "Yes restores the defaults");
            assert_eq!(field(&h, id, "width"), json!(8.0), "the size is not converted");
        } else {
            assert_eq!(shown, [json!("/tmp/sheets"), json!(3), json!("pixels")], "No changes nothing");
        }
    }
}
