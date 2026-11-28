//! #16731: a box over a group doesn't warp, as one over several layers doesn't; a box over one
//! layer of any other kind still does.

use super::*;
use crate::transform_tool::begin::WARP_GROUP;

/// The Edit › Transform rows of the Warp block.
const WARP_BLOCK: [&str; 5] = [
    "edit.transform.warp",
    "edit.transform.splitWarpCrosswise",
    "edit.transform.splitWarpHorizontally",
    "edit.transform.splitWarpVertically",
    "edit.transform.removeWarpSplit",
];

/// Is the Warp row of the open box's right-click menu enabled?
fn warp_row(app: &PhotocraftApp) -> bool {
    let words = TransformMenuWords::english();
    open(app).session.menu(&words).iter().find(|e| matches!(e.value(), Some(TransformChoice::Warp))).map(|e| e.is_enabled()).expect("the menu has a Warp row")
}

#[test]
fn a_group_box_greys_warp_in_its_menu_and_in_edit_transform() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = app.ctx().clone();
    app.session.execute("layer.new.groupFromLayers", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert!(st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| matches!(l.content, LayerContent::Group(_))), "the group is active");
    begin(&mut app, &ctx).unwrap();
    assert!(open(&app).layers.is_empty(), "a box over the group as one layer");
    assert!(!warp_row(&app), "Warp is greyed in the box's menu");
    for id in WARP_BLOCK {
        assert!(!crate::menus::is_enabled(&app, id), "{id} is greyed while the group box is open");
    }
    // Asked anyway (automation), Warp refuses and leaves the box as it was.
    assert_eq!(begin_warp(&mut app, &ctx), Err(WARP_GROUP.into()));
    assert!(open(&app).session.warp().is_none(), "the box doesn't switch to Warp");
    cancel(&mut app);
    // With no box open, Warp refuses without leaving one behind.
    assert_eq!(begin_warp(&mut app, &ctx), Err(WARP_GROUP.into()));
    assert!(app.ui.transform.is_none(), "no box is left open");
}

#[test]
fn single_pixel_type_and_smart_object_layers_keep_warp() {
    let ctx = egui::Context::default();
    let pixel = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let mut typed = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    typed.run("type.create", json!({"x": 10, "y": 40, "text": "Hi"})).unwrap();
    let mut smart = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    smart.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    for (name, mut app) in [("pixel", pixel), ("type", typed), ("smart object", smart)] {
        begin(&mut app, &ctx).unwrap();
        assert!(warp_row(&app), "a {name} layer's box offers Warp");
        assert!(crate::menus::is_enabled(&app, "edit.transform.warp"), "Edit › Transform › Warp is live over a {name} layer");
        cancel(&mut app);
    }
}
