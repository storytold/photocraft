//! #13685: Contact Sheet II flattens the sheet unless `flatten: false` is given, and the dialog's
//! defaults (20.32 x 25.4 cm at 300 pixels/inch) make the reference 2400 x 3000 px sheet.

use super::tests::{images, tmp};
use super::*;

const COMMAND: &str = "file.automate.contactSheetII";

/// Runs the command on `dir` with `params` and returns the first sheet.
fn sheet(s: &mut Session, dir: &str, mut params: Value) -> std::sync::Arc<photocraft_doc::Document> {
    params["input"] = json!(dir);
    let r = s.execute(COMMAND, params).unwrap();
    s.documents()[r["documents"][0].as_u64().unwrap() as usize].doc.clone()
}

#[test]
fn the_sheet_is_flattened_unless_flatten_is_false() {
    let dir = tmp("defaults-flatten");
    images(&dir, 3, 60, 30);
    let mut s = Session::new();
    let grid = json!({"units": "pixels", "width": 400, "height": 300, "columns": 2, "rows": 2});
    assert_eq!(sheet(&mut s, &dir, grid.clone()).layers.len(), 1, "flattened when not given");
    let mut layered = grid;
    layered["flatten"] = json!(false);
    let names: Vec<String> = sheet(&mut s, &dir, layered).layers.iter().map(|l| l.name.clone()).collect();
    assert!(names.contains(&"img0".to_string()) && names.contains(&"img2".to_string()), "flatten: false keeps the layers: {names:?}");
}

#[test]
fn the_dialog_defaults_make_a_flat_2400_by_3000_sheet_at_300_ppi() {
    let dir = tmp("defaults-size");
    images(&dir, 1, 60, 30);
    let mut s = Session::new();
    let d = sheet(&mut s, &dir, json!({"units": "cm", "width": 20.32, "height": 25.4, "resolution": 300.0}));
    assert_eq!((d.size.width, d.size.height, d.resolution_dpi), (2400, 3000, 300.0));
    assert_eq!(d.layers.len(), 1);
}

#[test]
fn the_params_doc_says_flatten_defaults_to_true() {
    let spec = specs().into_iter().find(|s| s.id == COMMAND).unwrap();
    assert!(spec.params.contains(r#""flatten":bool=true"#), "{}", spec.params);
}
