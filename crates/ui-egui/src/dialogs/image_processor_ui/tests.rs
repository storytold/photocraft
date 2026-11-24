use craft::ui::dialog::DialogId;
use serde_json::{Value, json};

use crate::{PhotocraftApp, Services};

/// The app with one document and the Image Processor open.
fn open() -> (Box<PhotocraftApp>, egui::Context, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 40, "height": 20})).unwrap();
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(&mut app, &ctx, super::COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().expect("the reply names a dialog").to_owned();
    let id = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|id| id.value().to_string() == want).expect("the dialog is open");
    (app, ctx, id)
}

fn set(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, key: &str, value: Value) {
    app.framework.dialogs.set_json_field(ctx, id, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
}

#[test]
fn values_nest_per_format() {
    let mut map = json!({"output": "o", "jpeg/enabled": true, "jpeg/quality": 5.0, "psd/enabled": false, "tiff/lzw": true}).as_object().unwrap().clone();
    super::nest(&mut map);
    assert_eq!(Value::Object(map), json!({"output": "o", "jpeg": {"enabled": true, "quality": 5.0}, "psd": {"enabled": false}, "tiff": {"lzw": true}}));
}

#[test]
fn ok_with_no_file_type_ticked_keeps_the_dialog() {
    let (mut app, ctx, id) = open();
    set(&mut app, &ctx, id, "jpeg/enabled", json!(false));
    assert!(crate::dialogs::confirm(&mut app, &ctx, id).is_err(), "OK is refused");
    assert!(app.framework.dialogs.open_dialogs().iter().any(|d| d.id == id), "the dialog stays open");
}

#[test]
fn run_dispatches_all_enabled_formats_with_their_own_dimensions() {
    let (mut app, ctx, id) = open();
    let directory = tempfile::tempdir().unwrap();
    set(&mut app, &ctx, id, "output", json!(directory.path()));
    for (key, value) in [
        ("jpeg/resize", json!(true)),
        ("jpeg/width", json!(20)),
        ("jpeg/height", json!(20)),
        ("psd/enabled", json!(true)),
        ("psd/resize", json!(true)),
        ("psd/width", json!(10)),
        ("psd/height", json!(10)),
        ("tiff/enabled", json!(true)),
    ] {
        set(&mut app, &ctx, id, key, value);
    }
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let mut sizes: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let ext = path.extension().unwrap().to_str().unwrap().to_owned();
            let bytes = std::fs::read(&path).unwrap();
            let doc = photocraft_io::import(path.to_str().unwrap(), &bytes).unwrap().document;
            (ext, doc.size.width, doc.size.height)
        })
        .collect();
    sizes.sort();
    assert_eq!(sizes, [("jpg".into(), 20, 10), ("psd".into(), 10, 5), ("tif".into(), 40, 20)]);
}

#[test]
fn processor_dialog_offers_jpeg_psd_and_tiff() {
    let (mut app, ctx, id) = open();
    ctx.run_ui(Default::default(), |ui| crate::dialogs::run(&mut app, ui)).textures_delta.clear();
    let fields = app.framework.dialogs.open_dialogs().into_iter().find(|dialog| dialog.id == id).unwrap().fields.unwrap();
    let formats: Vec<_> = fields.as_object().unwrap().keys().filter_map(|key| key.strip_suffix("/enabled")).collect();
    assert_eq!(formats, ["jpeg", "psd", "tiff"]);
}

#[test]
fn dimensions_need_both_switches_for_every_format() {
    use craft::ui::patterns::form_dialogs::DialogFields;
    for (format, _) in super::IMAGE_PROCESSOR_FORMATS {
        let mut values = DialogFields::new();
        for (enabled, resize) in [(false, true), (true, false), (true, true), (false, false)] {
            values.set(format!("{format}/enabled"), enabled);
            values.set(format!("{format}/resize"), resize);
            let mut rows = super::resize_rows(format);
            super::apply_resize_gates(&mut rows, &values);
            assert_eq!(rows[0].enabled(&values), enabled, "{format} resize");
            for row in &rows[1..] {
                assert_eq!(row.enabled(&values), enabled && resize, "{format} {}", row.key);
            }
            assert_eq!(values.bool(&format!("{format}/resize")), Some(resize));
        }
    }
}
