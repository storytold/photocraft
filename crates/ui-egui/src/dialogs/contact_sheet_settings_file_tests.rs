//! #13698: Contact Sheet II's Load…, Save… and Reset…, and the settings file they read and write.

use std::cell::RefCell;
use std::rc::Rc;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use serde_json::{Map, Value, json};

use super::super::contact_sheet_layout_tests::{H, fields, harness_with, pick, shown, type_into};
use super::super::contact_sheet_ok_validation_tests::{alerts, sheet_dialog};
use super::super::contact_sheet_sticky_tests::{click, text_of, ticked, values};
use super::super::{RES_UNIT, SETTINGS_FILE_MAX, defaults, settings};
use super::{Stamp, read, spacing_px, write};
use crate::PhotocraftApp;
use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};

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
        json!(100),
        json!(100),
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

/// Every file the Save… service wrote: path and text.
type Written = Rc<RefCell<Vec<(String, String)>>>;
type Asked = Rc<RefCell<Vec<FileDialogRequest>>>;

/// The dialog on an app whose file dialog answers `answers` in turn and whose writes are kept.
fn dialog_with(answers: Vec<Option<FileDialogAnswer>>) -> (H, Asked, Written) {
    let (show, asked) = crate::file_dialog::fake(answers);
    let written = Written::default();
    let log = written.clone();
    let services = crate::Services {
        file_dialog: Some(show),
        write: Some(Box::new(move |path, bytes| {
            log.borrow_mut().push((path.to_string(), String::from_utf8_lossy(bytes).into_owned()));
            Ok(())
        })),
        ..Default::default()
    };
    (harness_with(PhotocraftApp::new(photocraft_engine::Session::new(), services)), asked, written)
}

/// Clicks the dialog's button `label`, and lets the file dialog it asked for answer.
fn press(h: &mut H, label: &str) {
    click(h, Role::Button, label);
    let ctx = h.ctx.clone();
    h.state_mut().poll_file_dialog(&ctx, None);
    h.run_steps(3);
}

fn load(xml: &str) -> Option<FileDialogAnswer> {
    Some(FileDialogAnswer::Contents("Contact Sheet II.xml".into(), xml.as_bytes().to_vec()))
}

#[test]
fn the_dialog_has_load_save_and_reset_buttons() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (h, _, _) = dialog_with(vec![]);
        for label in ["Load…", "Save…", "Reset…"] {
            assert!(h.query_by_role_and_label(Role::Button, label).is_some(), "{label}");
        }
    });
}

#[test]
fn save_writes_the_dialog_values_and_load_reads_them_back() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (mut h, asked, written) = dialog_with(vec![Some(FileDialogAnswer::SaveTo("/settings/Contact Sheet II.xml".into()))]);
        pick(&mut h, "Units", "pixels");
        pick(&mut h, "Resolution Unit", "pixels/cm");
        pick(&mut h, "Mode", "Lab Color");
        pick(&mut h, "Place", "down first");
        type_into(&mut h, "Columns", "7");
        click(&mut h, Role::CheckBox, "Rotate For Best Fit");
        let saved = values(&h);
        press(&mut h, "Save…");
        assert!(matches!(&asked.borrow()[..], [FileDialogRequest::Save { suggested }] if suggested.ends_with("Contact Sheet II.xml")), "{:?}", asked.borrow());
        let (path, xml) = written.borrow().first().cloned().expect("Save… wrote the file");
        assert_eq!(path, "/settings/Contact Sheet II.xml");
        for attr in [r#"units="pixels""#, r#"mode="Lab Color""#, r#"place="down first""#, r#"cols="7""#, r#"bestFit="true""#] {
            assert!(xml.contains(attr), "{attr} in {xml}");
        }
        assert_eq!(values(&h), saved, "Save… changes nothing");

        // Into a fresh dialog: every value comes back.
        let (mut h, _, _) = dialog_with(vec![load(&xml)]);
        press(&mut h, "Load…");
        assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
        assert!(same(&values(&h), &saved), "loaded {:?}, saved {saved:?}", values(&h));
    });
}

#[test]
fn loading_the_reference_example_shows_every_value() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (mut h, asked, _) = dialog_with(vec![load(&reference_load_file())]);
        let sheet = h.state().ui.dialogs.last().map(|d| d.id).expect("the dialog is open");
        if let Some(d) = h.state_mut().ui.dialog_mut(sheet) {
            d.fields.insert("font".into(), json!("Serif"));
        }
        h.run_steps(2);
        press(&mut h, "Load…");
        assert!(matches!(&asked.borrow()[..], [FileDialogRequest::Open { multiple: false, extensions: Some(e), .. }] if e == &["xml"]), "{:?}", asked.borrow());
        let combos = ["Units", "Resolution Unit", "Mode", "Bit Depth", "Place"].map(|n| text_of(&h, Role::ComboBox, n));
        assert_eq!(combos, ["pixels", "pixels/inch", "Grayscale", "16-bit", "down first"]);
        assert_eq!(["Width", "Height", "Resolution", "Columns", "Rows", "Font size"].map(|n| shown(&h, n)), [500.0, 600.0, 100.0, 3.0, 4.0, 20.0]);
        assert_eq!(text_of(&h, Role::TextInput, "Input"), r"C:\some\folder\in");
        assert_eq!(text_of(&h, Role::TextInput, "Font"), photocraft_text::fonts::DEFAULT_FAMILY, "from its PostScript name");
        let boxes =
            ["Include Subfolders", "Group Images by Folder", "Flatten All Layers", "Rotate For Best Fit", "Use Auto-Spacing", "Use Filename as Caption"]
                .map(|n| ticked(&h, n));
        assert_eq!(boxes, [false, false, false, true, false, true]);
        // No spacing fields yet (#13689): the run keeps today's gaps.
        assert!(!fields(&h).contains_key("horizontal") && !fields(&h).contains_key("vertical"));
    });
}

#[test]
fn a_broken_value_keeps_its_field_and_a_file_that_is_not_xml_changes_nothing() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let broken = r#"<ContactSheetIISettings><document mode="Purple" resolution="0"/><thumbnail cols="101" rows="4"/></ContactSheetIISettings>"#;
        let large = format!("<ContactSheetIISettings>{}</ContactSheetIISettings>", " ".repeat(SETTINGS_FILE_MAX));
        let (mut h, _, _) = dialog_with(vec![load(broken), load("this is not XML"), load(&large)]);
        let before = values(&h);
        press(&mut h, "Load…");
        let mut want = before.clone();
        want.insert("rows".into(), json!(4));
        assert_eq!(values(&h), want, "only Rows is a value the dialog takes");
        assert!(alerts(&h).is_empty());

        press(&mut h, "Load…");
        // The alert is on top of the dialog.
        let sheet = |h: &H| -> Map<String, Value> {
            sheet_dialog(h).expect("the dialog stays open").iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect()
        };
        assert_eq!(sheet(&h), want, "a file that is not XML changes nothing");
        assert!(matches!(&alerts(&h)[..], [m] if m.starts_with("Contact Sheet II.xml: ")), "{:?}", alerts(&h));
        assert!(!h.state().ui.status_error, "the alert alone reports it: {}", h.state().ui.status);

        h.state_mut().ui.dialogs.retain(|d| d.kind != crate::state::DialogKind::Error);
        press(&mut h, "Load…");
        assert_eq!(sheet(&h), want, "a file over 1 MiB is not read");
        assert_eq!(alerts(&h), ["Contact Sheet II.xml: too large"]);
    });
}

#[test]
fn reset_asks_first_and_yes_restores_the_defaults_with_no_folder() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (mut h, _, _) = dialog_with(vec![]);
        let defaults = values(&h);
        let dir = std::env::temp_dir().to_string_lossy().into_owned();
        let sheet = h.state().ui.dialogs.last().map(|d| d.id).expect("the dialog is open");
        if let Some(d) = h.state_mut().ui.dialog_mut(sheet) {
            d.fields.insert("input".into(), json!(dir));
            d.fields.insert("files".into(), json!(["/a.png", "/b.png"]));
        }
        h.run_steps(2);
        pick(&mut h, "Units", "pixels");
        pick(&mut h, "Mode", "Grayscale");
        pick(&mut h, "Resolution Unit", "pixels/cm");
        // Group first: unticking Include Subfolders greys it.
        click(&mut h, Role::CheckBox, "Group Images by Folder");
        click(&mut h, Role::CheckBox, "Include Subfolders");
        let edited = values(&h);
        assert_eq!((edited["includeSubfolders"].clone(), edited["groupByFolder"].clone()), (json!(false), json!(true)));

        click(&mut h, Role::Button, "Reset…");
        let question = h.state().ui.dialogs.last().cloned().expect("Reset… asks");
        assert_eq!(crate::dialogs::title(&question), "Script Alert");
        assert!(h.query_by_label("Restore default settings?").is_some());
        assert!(h.query_by_role_and_label(Role::Button, "Yes").is_some());
        click(&mut h, Role::Button, "No");
        assert_eq!(h.state().ui.dialogs.len(), 1, "No closes the question");
        assert_eq!(values(&h), edited, "No changes nothing");

        click(&mut h, Role::Button, "Reset…");
        click(&mut h, Role::Button, "Yes");
        assert_eq!(h.state().ui.dialogs.len(), 1, "Yes closes the question, not the dialog");
        assert_eq!(values(&h), defaults, "the defaults, no folder and no Files list");
        assert_eq!(text_of(&h, Role::ComboBox, "Units"), "inches");
        assert_eq!(text_of(&h, Role::ComboBox, "Resolution Unit"), "pixels/inch");
        assert_eq!([ticked(&h, "Include Subfolders"), ticked(&h, "Group Images by Folder")], [true, false], "#13892's defaults");
        assert_eq!(text_of(&h, Role::TextInput, "Input"), "");
    });
}

#[test]
fn without_a_file_dialog_load_and_save_do_nothing() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness_with(PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()));
        let before = values(&h);
        press(&mut h, "Load…");
        press(&mut h, "Save…");
        assert_eq!(values(&h), before);
        assert!(alerts(&h).is_empty() && !h.state().ui.status_error, "{}", h.state().ui.status);
    });
}
