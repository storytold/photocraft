use craft::ui::dialog::{DialogId, StackDialog as _};
use craft::ui::patterns::form_dialogs::HostedImageSize;
use craft::ui::runtime::env::UnitKind;
use serde_json::{Value, json};

use super::*;

fn app_with(width: u32, height: u32, resolution: u32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": width, "height": height, "resolution": resolution})).unwrap();
    app
}

/// Opens `command` on the host and returns its id.
fn open_hosted(app: &mut PhotocraftApp, command: &str) -> DialogId {
    open(app, command).expect("a document is open")
}

/// Writes a field as `ui.dialog.set` does.
fn set(app: &mut PhotocraftApp, id: DialogId, field: &str, value: Value) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, field, value).unwrap_or_else(|e| panic!("{field}: {e}"));
}

/// Answers OK, which runs the command through `take`.
fn confirm_by_id(app: &mut PhotocraftApp, id: DialogId) {
    crate::dialogs::testing::confirm(app, id).unwrap();
}

fn doc_size(app: &PhotocraftApp) -> photocraft_doc::Size {
    app.session.active().unwrap().doc.size
}

/// The fields of the open Image Size `id`.
fn image_fields(app: &PhotocraftApp, id: DialogId) -> Value {
    app.framework.dialogs.get_by_id::<HostedImageSize>(id).expect("Image Size is open").json_fields().expect("it has fields")
}

fn unit(kind: UnitKind) -> Value {
    serde_json::to_value(kind).unwrap()
}

/// #6214: 2 columns on the Width at 300 ppi are 1550 px (the default 180 pt columns with 12 pt
/// gutters). Image Size's Height follows the proportions; Canvas Size keeps its Height.
#[test]
fn typed_columns_resize_the_document() {
    for command in ["image.imageSize", "image.canvasSize"] {
        let mut app = app_with(1000, 500, 300);
        let id = open_hosted(&mut app, command);
        set(&mut app, id, "widthUnit", unit(UnitKind::Columns));
        set(&mut app, id, "width", json!(2));
        confirm_by_id(&mut app, id);
        let want = if command == "image.imageSize" { photocraft_doc::Size::new(1550, 775) } else { photocraft_doc::Size::new(1550, 500) };
        assert_eq!(doc_size(&app), want, "{command}");
    }
}

#[test]
fn dialog_maps_to_engine_command() {
    let mut app = app_with(200, 100, 72);
    let id = open_hosted(&mut app, "image.imageSize");
    set(&mut app, id, "width", json!(100));
    set(&mut app, id, "height", json!(50));
    confirm_by_id(&mut app, id);
    assert_eq!(doc_size(&app), photocraft_doc::Size::new(100, 50));
    let id = open_hosted(&mut app, "image.canvasSize");
    set(&mut app, id, "width", json!(120));
    set(&mut app, id, "anchor", serde_json::to_value(craft::behavior::Anchor9::TopLeft).unwrap());
    confirm_by_id(&mut app, id);
    assert_eq!(doc_size(&app), photocraft_doc::Size::new(120, 50));
}

/// #9260: OK with a new Width resizes a document whose own resolution is outside the range the
/// dialog accepts (an imported PNG's pHYs of 1 px/m is 0.0254 ppi); the document keeps it.
#[test]
fn ok_resizes_a_document_with_an_imported_resolution_out_of_range() {
    for own in [0.0254_f32, 40_000.0] {
        let mut app = app_with(200, 100, 72);
        app.session
            .edit("ppi", |doc, _| {
                doc.resolution_dpi = own;
                Ok(())
            })
            .unwrap();
        let id = open_hosted(&mut app, "image.imageSize");
        set(&mut app, id, "width", json!(100));
        set(&mut app, id, "height", json!(50));
        confirm_by_id(&mut app, id);
        let doc = &app.session.active().unwrap().doc;
        assert_eq!((doc.size, doc.resolution_dpi), (photocraft_doc::Size::new(100, 50), own), "{own}");
    }
}

/// #6019 and the Image Interpolation preference: the dialog opens on the preference's method; an
/// Indexed Color document keeps Automatic, turns any other into Nearest Neighbor and offers only those two.
#[test]
fn indexed_color_resamples_with_nearest_neighbor_only() {
    let method = |m: Resample| serde_json::to_value(m).unwrap();
    let ctx_of = |app: &PhotocraftApp| app.ctx().clone();

    let mut app = app_with(200, 100, 72);
    for (pref, want) in [
        ("bicubicAutomatic", Resample::Automatic),
        ("bicubicSharper", Resample::BicubicSharper),
        ("bicubicSmoother", Resample::BicubicSmoother),
        ("nearestNeighbor", Resample::NearestNeighbor),
    ] {
        app.session.execute("prefs.set", json!({"path": "general.imageInterpolation", "value": pref})).unwrap();
        let id = open_hosted(&mut app, "image.imageSize");
        assert_eq!(image_fields(&app, id)["resample"], method(want), "{pref}");
        let ctx = ctx_of(&app);
        app.framework.dialogs.cancel_by_id(&ctx, id);
    }

    app.session.execute("image.mode.indexedColor", json!({})).unwrap();
    for (pref, want) in [("bicubic", Resample::NearestNeighbor), ("bicubicAutomatic", Resample::Automatic), ("nearestNeighbor", Resample::NearestNeighbor)] {
        app.session.execute("prefs.set", json!({"path": "general.imageInterpolation", "value": pref})).unwrap();
        let id = open_hosted(&mut app, "image.imageSize");
        assert_eq!(image_fields(&app, id)["resample"], method(want), "{pref}");
        let ctx = ctx_of(&app);
        assert!(app.framework.dialogs.set_json_field(&ctx, id, "resample", method(Resample::Bicubic)).is_err(), "{pref}: Bicubic is not offered");
        assert!(app.framework.dialogs.set_json_field(&ctx, id, "resample", method(Resample::Automatic)).is_ok(), "{pref}: Automatic is offered");
        app.framework.dialogs.cancel_by_id(&ctx, id);
    }
}

/// #5756: points follow Preferences › Units & Rulers › Point/Pica Size. With Traditional (72.27 pt/in)
/// and Resample off, 100 px typed as 72.27 pt wide is 100 ppi.
#[test]
fn traditional_point_size_drives_points_and_picas() {
    let mut app = app_with(100, 50, 72);
    app.session.execute("prefs.set", json!({"path": "unitsAndRulers.pointSize", "value": "traditional"})).unwrap();
    let id = open_hosted(&mut app, "image.imageSize");
    set(&mut app, id, "resample", Value::Null);
    set(&mut app, id, "unit", unit(UnitKind::Points));
    set(&mut app, id, "width", json!(72.27));
    confirm_by_id(&mut app, id);
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.size, photocraft_doc::Size::new(100, 50));
    assert!((doc.resolution_dpi - 100.0).abs() < 1e-2, "{}", doc.resolution_dpi);
}

/// #4162: a document wider than 30000 units in a physical unit (40000 px at 72 ppi is 40000 pt)
/// keeps its size on OK with no edit.
#[test]
fn large_documents_in_physical_units_open_unchanged() {
    for command in ["image.imageSize", "image.canvasSize"] {
        let mut app = app_with(40000, 4, 72);
        let id = open_hosted(&mut app, command);
        set(&mut app, id, "unit", unit(UnitKind::Points));
        confirm_by_id(&mut app, id);
        assert_eq!(doc_size(&app), photocraft_doc::Size::new(40000, 4), "{command}");
    }
}

/// #14768: Image Size's readout counts the flat image: the mode's colour channels at the document's depth, no alpha;
/// Indexed one index byte and Bitmap one bit, though both are stored expanded.
#[test]
fn the_image_size_readout_counts_the_flat_image() {
    let bpp = |new: Value, commands: &[&str]| {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", new).unwrap();
        for c in commands {
            s.execute(c, json!({})).unwrap();
        }
        flat_bytes_per_pixel(&s.active().unwrap().doc)
    };
    assert_eq!(bpp(json!({"width": 4, "height": 4}), &[]), 3.0, "RGB 8 bit, no alpha");
    assert_eq!(bpp(json!({"width": 4, "height": 4, "depth": 16}), &[]), 6.0);
    assert_eq!(bpp(json!({"width": 4, "height": 4, "depth": 32}), &[]), 12.0);
    assert_eq!(bpp(json!({"width": 4, "height": 4, "mode": "cmyk"}), &[]), 4.0);
    assert_eq!(bpp(json!({"width": 4, "height": 4, "mode": "gray"}), &[]), 1.0);
    assert_eq!(bpp(json!({"width": 4, "height": 4, "mode": "lab"}), &[]), 3.0);
    assert_eq!(bpp(json!({"width": 4, "height": 4}), &["image.mode.indexedColor"]), 1.0);
    assert_eq!(bpp(json!({"width": 4, "height": 4}), &["image.mode.grayscale", "image.mode.bitmap"]), 1.0 / 8.0);
}
