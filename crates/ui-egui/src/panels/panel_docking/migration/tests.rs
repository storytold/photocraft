//! Legacy conversion tests, independent of compact rendering and Tools input.
use super::super::group_members;
use super::*;
use serde_json::json;
fn columns(layout: &Layout<String>) -> Vec<(Side, &Column<String>)> {
    let mut pending: Vec<_> = layout.root.iter().collect();
    let mut out = Vec::new();
    while let Some(node) = pending.pop() {
        match node {
            Node::Dock { side, columns } => out.extend(columns.iter().map(|c| (*side, c))),
            Node::Split { first, second, .. } => pending.extend([second.as_ref(), first.as_ref()]),
            _ => {}
        }
    }
    out
}

#[test]
fn legacy_column_migration_keeps_tabs_active_row_weights_and_presentation() {
    let saved = json!({
        "docking":{"root":{"Tabs":{"panels":["layers"],"active":0}},"floating":[{"panels":["properties","actions","channels"],"active":2,"rect":[100,100,320,400]}]},
        "dockingColumns":[{"members":["properties","actions","channels"],"rows":[{"panels":["properties","channels"],"active":1,"weight":3},{"panels":["actions"],"active":0,"weight":1}],"height":600,"attachment":{"side":"rightInner","order":0,"width":320,"expanded":true}}],
        "dockingStripWidths":{"properties":120},"dockingCollapsed":["properties","actions","channels"]
    });
    let workspace = migrate(&saved).unwrap();
    assert!(valid_workspace(&workspace));
    let c = columns(&workspace.layout).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == "properties"))).unwrap().1;
    assert_eq!(c.groups[0].panels, ["properties", "channels"]);
    assert_eq!(c.groups[0].active, 1);
    assert_eq!(c.groups[0].height, Some(450.0));
    assert_eq!(c.groups[1].height, Some(150.0));
    let p = presentation(&saved);
    assert_eq!(p.widths.get("properties"), Some(&120.0));
    assert_eq!(p.heights.get("actions"), Some(&600.0));
}

#[test]
fn legacy_horizontal_root_keeps_its_split_geometry() {
    let root = Node::Split {
        axis: super::super::SplitAxis::Horizontal,
        size: super::super::SplitSize::Ratio(0.37),
        first: Box::new(Node::Tabs { panels: vec!["layers".into()], active: 0 }),
        second: Box::new(Node::Tabs { panels: vec!["color".into(), "swatches".into()], active: 1 }),
    };
    let saved = json!({"docking":{"root":root,"floating":[]},"dockingColumns":[]});
    let workspace = migrate(&saved).unwrap();
    let Node::Split { second, .. } = workspace.layout.root.as_ref().unwrap() else { panic!("canvas split") };
    assert_eq!(second.as_ref(), &root);
    assert!(valid_workspace(&workspace));
}

#[test]
fn migrating_multiple_hidden_panels_keeps_their_return_anchor_without_duplicates() {
    let at = json!({"floating":null,"anchor":"layers","placement":{"Tab":{"before":null}},"size":null,"stack":null});
    let saved =
        json!({"docking":{"root":{"Tabs":{"panels":["layers"],"active":0}},"floating":[]},"dockingColumns":[],"dockingHidden":{"properties":at,"actions":at}});
    let mut workspace = migrate(&saved).unwrap();
    assert_eq!(workspace.hidden.len(), 2);
    for id in ["properties", "actions"] {
        let at = workspace.hidden.iter().find(|(p, _)| p == id).unwrap().1.clone();
        assert_eq!(at.anchor.as_deref(), Some("layers"));
        workspace.set_visible(id.into(), true, None, permissions).unwrap();
    }
    assert!(valid_workspace(&workspace));
    assert_eq!(group_members(&workspace.layout, "layers").len(), 3);
}

#[test]
fn migration_survives_snapshot_and_reload_without_losing_view_metadata() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let saved = json!({"docking":{"root":{"Tabs":{"panels":["layers"],"active":0}},"floating":[]},"dockingColumns":[],"panelPresentation":{"migrated":true},"toolbarFloating":[80,120]});
    crate::dock::apply(&mut app, &saved);
    let before = app.ui.docking.clone().unwrap();
    let written = crate::dock::snapshot(&app);
    crate::dock::apply(&mut app, &written);
    assert_eq!(app.ui.docking.as_ref(), Some(&before));
    assert!(app.ui.panel_presentation.migrated);
    assert_eq!(app.ui.toolbar_floating, Some([80.0, 120.0]));
}
#[test]
fn optional_view_corruption_and_metric_upgrade_keep_other_personal_values() {
    let saved = json!({"panelPresentation":{"widths":{"properties":44,"color":49.6},"tools_size":{"future":true},"migrated":true}});
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::dock::apply(&mut app, &saved);
    assert_eq!(app.ui.panel_presentation.widths["properties"], super::super::COMPACT_COLUMN_WIDTH);
    assert_eq!(app.ui.panel_presentation.widths["color"], 49.6);
    assert!(app.ui.panel_presentation.migrated);
    assert!(app.ui.panel_presentation.tools_size.is_none());
    app.ui.panel_presentation.widths.insert("properties".into(), 44.0);
    let written = crate::dock::snapshot(&app);
    crate::dock::apply(&mut app, &written);
    assert_eq!(app.ui.panel_presentation.widths["properties"], 44.0);
}
#[test]
fn unsupported_attachment_does_not_silently_discard_its_panels() {
    let saved = json!({"docking":{"root":{"Tabs":{"panels":["layers"],"active":0}},"floating":[{"panels":["properties"],"active":0,"rect":[100,100,320,400]}]},"dockingColumns":[{"members":["properties"],"attachment":{"side":"futureSide","width":300}}]});
    assert!(migrate(&saved).is_none());
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::dock::apply(&mut app, &saved);
    let written = crate::dock::snapshot(&app);
    assert_eq!(written["docking"], saved["docking"]);
    assert_eq!(written["dockingColumns"], saved["dockingColumns"]);
}
#[test]
fn old_width_default_is_upgraded_once_even_in_an_otherwise_valid_view_object() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::dock::apply(&mut app, &json!({"panelPresentation":{"widths":{"properties":44}}}));
    assert_eq!(app.ui.panel_presentation.widths["properties"], super::super::COMPACT_COLUMN_WIDTH);
}

#[test]
fn named_workspace_preserves_migrated_view_preferences() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::dock::apply(&mut app, &json!({"panelPresentation":{"migrated":true,"metrics_revision":1,"widths":{"properties":117}},"toolbarFloating":[80,120]}));
    crate::workspace_ui::menu(&mut app, "window.workspace.newWorkspace", &json!({"name":"Migrated panels"})).unwrap().unwrap();
    let saved = &app.session.prefs().workspaces["Migrated panels"];
    assert_eq!(saved["panelPresentation"]["widths"]["properties"], 117.0);
    assert_eq!(saved["toolbarFloating"], json!([80.0, 120.0]));
}
#[test]
fn migrated_layout_renders_through_the_stock_workspace_renderer() {
    let _gpu = photocraft_testkit::gpu_lock();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0,900.0)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::Pro);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let saved=json!({"docking":{"root":{"Tabs":{"panels":["layers"],"active":0}},"floating":[{"panels":["properties","navigator"],"active":0,"rect":[100,100,320,400]}]},"dockingColumns":[{"members":["properties","navigator"],"rows":[{"panels":["properties"],"weight":1},{"panels":["navigator"],"weight":1}],"attachment":{"side":"left","width":300,"expanded":true}}]});
        crate::dock::apply(&mut app,&saved);
        app
    });
    h.run_steps(8);
    assert!(valid_workspace(h.state().ui.docking.as_ref().unwrap()));
    let image = h.render().unwrap();
    if let Some(path) = std::env::var_os("PHOTOCRAFT_MIGRATION_CAPTURE") {
        image.save(path).unwrap();
    }
}
