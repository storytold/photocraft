use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::{FileFilter, PickKind, Picked, ScriptedFiles};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Value, json};

use super::{COMMAND, settings};
use crate::{PhotocraftApp, Services};

#[test]
fn settings_xml_follows_the_reference_order_and_formats() {
    let mut fields = DialogFields::from_pairs([("source", "folder"), ("input", "C:\\é & <source>\r\n"), ("output", "C:\\résultats")]);
    fields.set("jpeg/quality", 5.25);
    fields.set("jpeg/resize", true);
    fields.set("jpeg/width", 100_i64);
    fields.set("jpeg/height", 80_i64);
    let xml = settings::encode(&fields).unwrap();
    let text = std::str::from_utf8(&xml).unwrap();
    assert_eq!(
        text,
        concat!(
            "\u{feff}<ImageProcessor>\n",
            "\t<version>1</version>\n",
            "\t<useopen>false</useopen>\n",
            "\t<includesub>false</includesub>\n",
            "\t<source>C:\\é &amp; &lt;source&gt;&#13;&#10;</source>\n",
            "\t<open>false</open>\n",
            "\t<saveinsame>false</saveinsame>\n",
            "\t<dest>C:\\résultats</dest>\n",
            "\t<jpeg>true</jpeg>\n",
            "\t<psd>false</psd>\n",
            "\t<tiff>false</tiff>\n",
            "\t<lzw>false</lzw>\n",
            "\t<converticc>false</converticc>\n",
            "\t<q>5.25</q>\n",
            "\t<max>true</max>\n",
            "\t<jpegresize>true</jpegresize>\n",
            "\t<jpegw>100</jpegw>\n",
            "\t<jpegh>80</jpegh>\n",
            "\t<psdresize>false</psdresize>\n",
            "\t<psdw>1</psdw>\n",
            "\t<psdh>1</psdh>\n",
            "\t<tiffresize>false</tiffresize>\n",
            "\t<tiffw>1</tiffw>\n",
            "\t<tiffh>1</tiffh>\n",
            "\t<runaction>false</runaction>\n",
            "\t<actionset></actionset>\n",
            "\t<action></action>\n",
            "\t<info></info>\n",
            "\t<icc>true</icc>\n",
            "\t<keepstructure>true</keepstructure>\n",
            "</ImageProcessor>\n",
        )
    );
    let decoded = settings::decode(&xml).unwrap();
    assert_eq!(decoded.text("input"), fields.text("input"));
    assert_eq!(decoded.text("output"), fields.text("output"));
    assert_eq!(decoded.number("jpeg/quality"), Some(5.25));
}

#[test]
fn partial_xml_keeps_missing_fields_and_ignores_unknown_elements() {
    let fields = settings::decode(
        br#"<ImageProcessor><useopen>true</useopen><source/><future><q>invalid</q></future><actionset>Missing set  </actionset></ImageProcessor>"#,
    )
    .unwrap();
    assert_eq!(fields, DialogFields::from_pairs([("source", "open"), ("input", "")]));
    for source in ["current", "open", "folder"] {
        let current = DialogFields::from_pairs([("source", source)]);
        let encoded = settings::encode(&current).unwrap();
        assert_eq!(settings::decode(&encoded).unwrap().text("source"), Some(source));
    }
}

#[test]
fn invalid_settings_are_rejected() {
    for invalid in [
        "",
        "<Other/>",
        "<ImageProcessor xmlns='urn:other'/>",
        "<!DOCTYPE ImageProcessor><ImageProcessor/>",
        "<ImageProcessor>",
        "<ImageProcessor><q>abc</q></ImageProcessor>",
        "<ImageProcessor><q>NaN</q></ImageProcessor>",
        "<ImageProcessor><q>12.01</q></ImageProcessor>",
        "<ImageProcessor><jpegw>30001</jpegw></ImageProcessor>",
        "<ImageProcessor><psdw>0</psdw></ImageProcessor>",
        "<ImageProcessor><jpeg>maybe</jpeg></ImageProcessor>",
        "<ImageProcessor><tiff>tr<something/>ue</tiff></ImageProcessor>",
        "<ImageProcessor><icc>false</icc><icc>true</icc></ImageProcessor>",
        "<ImageProcessor><source>&bad;</source></ImageProcessor>",
        "<ImageProcessor><dest>A & B</dest></ImageProcessor>",
        "<ImageProcessor><dest>A &amp</dest></ImageProcessor>",
        "<ImageProcessor><future label='A & B'/></ImageProcessor>",
        "<ImageProcessor/><Another/>",
    ] {
        assert!(settings::decode(invalid.as_bytes()).is_err(), "{invalid}");
    }
    assert!(settings::decode(&[0xff]).is_err());
    assert!(settings::decode(&vec![b' '; settings::MAX_BYTES + 1]).is_err());
    assert!(settings::encode(&DialogFields::from_pairs([("input", "bad\0path")])).is_err());
}

#[test]
fn escaped_ampersands_and_literal_cdata_remain_valid() {
    let xml = concat!(
        "\u{feff}<ImageProcessor><!-- A & B --><?note A & B?>",
        "<future label='A &amp; B'/>",
        "<dest>A &amp; B &#38; C &amp;amp;</dest>",
        "<source><![CDATA[A & B]]></source></ImageProcessor>",
    );
    let fields = settings::decode(xml.as_bytes()).unwrap();
    assert_eq!(fields.text("output"), Some("A & B & C &amp;"));
    assert_eq!(fields.text("input"), Some("A & B"));
}

type AppHarness = Harness<'static, Box<PhotocraftApp>>;
type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

fn open(app: &mut PhotocraftApp) -> DialogId {
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, COMMAND, json!({})).unwrap();
    app.framework.dialogs.stack().top().unwrap()
}

fn harness() -> (AppHarness, DialogId, Arc<ScriptedFiles>, Written) {
    let files = Arc::new(ScriptedFiles::held());
    let written: Written = Default::default();
    let log = written.clone();
    let services = Services {
        files: Some(files.clone()),
        write: Some(Box::new(move |path, bytes| {
            log.borrow_mut().push((path.to_owned(), bytes.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 40, "height": 20})).unwrap();
    let id = open(&mut app);
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 1000.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            app.poll_file_dialog();
            crate::dialogs::run(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    (h, id, files, written)
}

fn fields(h: &AppHarness, id: DialogId) -> &DialogFields {
    h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields()
}

fn set(h: &mut AppHarness, id: DialogId, key: &str, value: Value) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, key, value).unwrap_or_else(|error| panic!("{key}: {error}"));
}

fn click(h: &mut AppHarness, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
    h.run_steps(4);
}

#[test]
fn the_footer_saves_and_loads_pending_values_without_running_image_processor() {
    let (mut h, id, files, written) = harness();
    for (key, value) in [
        ("source", json!("folder")),
        ("input", json!("pictures & sketches")),
        ("output", json!("exports")),
        ("includeSubfolders", json!(true)),
        ("jpeg/quality", json!(9.75)),
        ("jpeg/convertToSrgb", json!(true)),
        ("jpeg/resize", json!(true)),
        ("jpeg/width", json!(2048)),
        ("jpeg/height", json!(1536)),
        ("psd/enabled", json!(true)),
        ("psd/maximizeCompatibility", json!(false)),
        ("psd/resize", json!(true)),
        ("psd/width", json!(1024)),
        ("psd/height", json!(768)),
        ("tiff/enabled", json!(true)),
        ("tiff/lzw", json!(true)),
        ("tiff/resize", json!(true)),
        ("tiff/width", json!(4096)),
        ("tiff/height", json!(3072)),
    ] {
        set(&mut h, id, key, value);
    }
    h.run_steps(3);
    let buttons = ["Load…", "Save…", "Cancel", "Run"].map(|label| h.get_by_role_and_label(egui::accesskit::Role::Button, label).rect());
    for (index, button) in buttons.iter().enumerate() {
        assert!(button.is_positive(), "button {index} is visible");
        assert!(buttons.iter().skip(index + 1).all(|other| !button.intersects(*other)), "footer buttons must not overlap");
    }
    let original = fields(&h, id).clone();
    click(&mut h, "Save…");
    let request = files.requests().pop().unwrap();
    assert_eq!(request.kind, PickKind::Save);
    assert_eq!(request.title.as_deref(), Some("Pick an XML file to save"));
    assert_eq!(request.filters, [FileFilter::new("XML Files (*.xml)", &["xml"])]);
    files.answer(Picked::Paths(vec![PathBuf::from("preset.xml")])).unwrap();
    h.run_steps(3);
    assert_eq!(fields(&h, id), &original);
    let (path, saved) = written.borrow_mut().pop().unwrap();
    assert_eq!(path, "preset.xml");

    set(&mut h, id, "source", json!("open"));
    set(&mut h, id, "jpeg/quality", json!(2.0));
    set(&mut h, id, "psd/enabled", json!(false));
    set(&mut h, id, "tiff/width", json!(16));
    h.run_steps(3);
    click(&mut h, "Load…");
    let request = files.requests().pop().unwrap();
    assert_eq!(request.kind, PickKind::Open);
    assert_eq!(request.title.as_deref(), Some("Pick an XML file to load"));
    assert_eq!(request.filters, [FileFilter::new("XML Files (*.xml)", &["xml"])]);
    files.answer(Picked::Contents(vec![(path, saved)])).unwrap();
    h.run_steps(3);
    assert_eq!(fields(&h, id), &original);
    assert!(written.borrow().is_empty(), "Load does not write output or close the dialog");
}

#[test]
fn malformed_and_cancelled_picks_leave_the_form_unchanged() {
    let (mut h, id, files, written) = harness();
    let original = fields(&h, id).clone();

    click(&mut h, "Save…");
    files.answer(Picked::Cancelled).unwrap();
    h.run_steps(3);
    assert!(written.borrow().is_empty());
    click(&mut h, "Load…");
    files.answer(Picked::Cancelled).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().framework.dialogs.stack().top(), Some(id));

    let path = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(path.path(), b"<ImageProcessor><useopen>true</useopen><dest>A & B</dest></ImageProcessor>").unwrap();
    click(&mut h, "Load…");
    files.answer(Picked::Paths(vec![path.path().to_path_buf()])).unwrap();
    h.run_steps(3);
    assert_eq!(fields(&h, id), &original);
    let alert = h.state().framework.dialogs.stack().top().unwrap();
    assert_ne!(alert, id, "an error is shown over the form");
    assert!(h.state().ui.status.contains("unescaped ampersand"));
}

#[test]
fn a_load_answer_for_a_closed_dialog_does_not_edit_a_reopened_dialog() {
    let (mut h, id, files, _) = harness();
    click(&mut h, "Load…");
    let ctx = h.ctx.clone();
    crate::dialogs::cancel(h.state_mut(), &ctx, id).unwrap();
    let reopened = open(h.state_mut());
    let original = fields(&h, reopened).clone();
    files.answer(Picked::Contents(vec![("preset.xml".into(), b"<ImageProcessor><q>12</q></ImageProcessor>".to_vec())])).unwrap();
    h.run_steps(4);
    assert_eq!(fields(&h, reopened), &original);
}
