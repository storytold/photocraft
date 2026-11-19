//! A dialog's title is its command's name without the menu item's ellipsis (#15541): Edit › Stroke…
//! opens "Stroke", as in Photoshop. Covers the dialogs that name themselves after their menu item.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;

/// Opens one dialog.
type Open = fn(&mut PhotocraftApp);

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app
    });
    h.run_steps(6);
    h
}

#[test]
fn dialog_titles_drop_the_menu_ellipsis() {
    let cases: [(&str, Open); 4] = [
        ("Stroke", |app| {
            crate::stroke_ui::open(app);
        }),
        ("New Group", |app| {
            crate::new_group_ui::open(app, crate::new_group_ui::NEW_GROUP);
        }),
        ("New Group from Layers", |app| {
            crate::new_group_ui::open(app, crate::new_group_ui::FROM_LAYERS);
        }),
        ("New Brush Preset", |app| {
            crate::brush_preset_dialog::open(app, None);
        }),
    ];
    for (title, open) in cases {
        let mut h = harness();
        open(h.state_mut());
        h.run_steps(4);
        assert!(h.query_all_by_label(&format!("{title}…")).next().is_none(), "the {title} dialog's title has no ellipsis");
        assert!(h.query_all_by_label(title).next().is_some(), "the dialog is titled {title}");
    }
}
