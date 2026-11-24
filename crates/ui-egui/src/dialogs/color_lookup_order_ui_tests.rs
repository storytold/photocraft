//! #14423: Image › Adjustments › Color Lookup shows Data Order and Table Order only while a `.cube`
//! file is the LUT, and OK applies the chosen orders.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::filter_dialog::open;

const COLOR_LOOKUP: &str = "image.adjustments.colorLookup";

/// `swap2.cube`: out = (b, g, r), red index fastest.
const SWAP2: &str = "LUT_3D_SIZE 2\n0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";

fn app_with_doc() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 4, "height": 4, "background": "#c83232"})).unwrap();
    app
}

fn fields_of(app: &PhotocraftApp, id: DialogId) -> DialogFields {
    app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("open dialog").fields().clone()
}

fn try_set(app: &mut PhotocraftApp, id: DialogId, key: &str, value: Value) -> Result<(), String> {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, key, value)
}

fn passes(app: &mut PhotocraftApp, n: usize) {
    let ctx = app.ctx().clone();
    for _ in 0..n {
        crate::testing::run_ui(&ctx, Default::default(), |ui| crate::dialogs::run(app, ui));
    }
}

fn ok(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    let _ = crate::dialogs::confirm(app, &ctx, id);
    passes(app, 3);
    assert!(app.framework.dialogs.get_by_id::<HostedParamDialog>(id).is_none(), "OK closes the dialog");
}

fn shown(app: &PhotocraftApp, id: DialogId) -> bool {
    fields_of(app, id).bool("lutCube") == Some(true)
}

fn pixel(app: &PhotocraftApp) -> [u8; 3] {
    let px = app.session.active().unwrap().doc.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 1, 1));
    [0, 1, 2].map(|c| (px[c] * 255.0).round() as u8)
}

/// Hidden with no LUT, a built-in look or a `.3dl` from the LUT folder; shown for a `.cube` from the folder and
/// for a loaded LUT; and while hidden, the rows refuse values.
#[test]
fn the_orders_show_only_for_a_cube() {
    let tmp = tempfile::tempdir().unwrap();
    let cube = tmp.path().join("swap2.cube");
    std::fs::write(&cube, SWAP2).unwrap();
    let threedl = tmp.path().join("ramp.3dl");
    std::fs::write(&threedl, "0 1023\n0 0 0\n0 0 1023\n0 1023 0\n0 1023 1023\n1023 0 0\n1023 0 1023\n1023 1023 0\n1023 1023 1023\n").unwrap();
    let mut app = app_with_doc();
    let id = open(&mut app, COLOR_LOOKUP).unwrap();
    passes(&mut app, 2);
    assert!(!shown(&app, id), "no LUT");
    assert!(try_set(&mut app, id, "dataOrder", json!("bgr")).is_err(), "a hidden row takes no value");
    let fields = fields_of(&app, id);
    assert_eq!((fields.text("dataOrder"), fields.text("tableOrder")), (Some("rgb"), Some("bgr")), "the defaults");

    try_set(&mut app, id, "lut", json!("warm")).unwrap();
    passes(&mut app, 2);
    assert!(!shown(&app, id), "a built-in look");

    try_set(&mut app, id, "file", json!(threedl.to_str().unwrap())).unwrap();
    passes(&mut app, 2);
    assert!(!shown(&app, id), "a .3dl file");

    try_set(&mut app, id, "file", json!(cube.to_str().unwrap())).unwrap();
    passes(&mut app, 2);
    assert!(shown(&app, id), "a .cube file");
    try_set(&mut app, id, "dataOrder", json!("bgr")).unwrap();
    assert!(try_set(&mut app, id, "tableOrder", json!("grb")).is_err(), "only RGB and BGR");
}

/// OK with a `.cube` and Data Order BGR (Table Order left at BGR) applies the file's table read as B G R: the
/// swap2 table becomes the identity, as measured. With the defaults the file's swap applies.
#[test]
fn ok_applies_the_chosen_orders() {
    let tmp = tempfile::tempdir().unwrap();
    let cube = tmp.path().join("swap2.cube");
    std::fs::write(&cube, SWAP2).unwrap();
    for (data_order, want) in [("rgb", [50, 50, 200]), ("bgr", [200, 50, 50])] {
        let mut app = app_with_doc();
        let id = open(&mut app, COLOR_LOOKUP).unwrap();
        try_set(&mut app, id, "file", json!(cube.to_str().unwrap())).unwrap();
        passes(&mut app, 2);
        try_set(&mut app, id, "dataOrder", json!(data_order)).unwrap();
        try_set(&mut app, id, "dither", json!(false)).unwrap();
        ok(&mut app, id);
        assert_eq!(pixel(&app), want, "data order {data_order}");
    }
}
