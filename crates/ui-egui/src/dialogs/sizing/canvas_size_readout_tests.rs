//! #14582: Canvas Size uses flat image bytes, not the first layer's storage with alpha.

use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use super::*;

#[test]
fn canvas_size_readouts_count_flat_image_bytes_and_update_with_new_dimensions() {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(8).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("prefs.set", json!({"path": "interface.language", "value": "en"})).unwrap();
        app.session.execute("file.new", json!({"width": 400, "height": 300, "resolution": 72})).unwrap();
        app
    });
    h.run_steps(4);
    let id = open(h.state_mut(), CANVAS_COMMAND).expect("a document is open");
    h.run_steps(6);
    // The fixture pins English unit words. The host still uses the regional number format,
    // so render the independently measured quantities with that format before querying the UI.
    let format = crate::i18n::framework_i18n().format();
    let initial = format.number_ungrouped(351.6, 1);
    assert_eq!(h.query_all_by_label_contains(&initial).count(), 2, "Current and New Size both count three RGB channels");

    for (width, height, value, decimals) in [(1, 300, 900.0, 0), (133, 300, 116.9, 1), (500, 400, 585.9, 1), (567, 300, 498.3, 1)] {
        let app = h.state_mut();
        let ctx = app.ctx().clone();
        app.framework.dialogs.set_json_field(&ctx, id, "width", json!(width)).unwrap();
        app.framework.dialogs.set_json_field(&ctx, id, "height", json!(height)).unwrap();
        h.run_steps(4);
        let number = format.number_ungrouped(value, decimals);
        let want = if decimals == 0 { format!("{number} bytes") } else { number };
        assert_eq!(h.query_all_by_label_contains(&initial).count(), 1, "the original size is unchanged");
        assert_eq!(h.query_all_by_label_contains(&want).count(), 1, "{width}×{height} shows the new flat size");
    }
}
