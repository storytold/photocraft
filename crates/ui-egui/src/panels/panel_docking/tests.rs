//! The panel commands, the saved workspace and the legacy `ui.set` words, over the framework workspace.
//! Drags are the `DockArea`'s and its trials' own: these tests change the workspace through the
//! automation ids and `apply`, and assert layout membership, the front tab, minimized flags and
//! saved JSON keys, never sizes.

use super::*;
use craft::behavior::dock::{Action, Column, DockEdge, DockPlacement, Layout, Node, PanelPlace};
use craft::ui::docking::DockSizes;
use serde_json::{Value, json};

use crate::panels::registry;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
}

/// Runs `id` as the menus and the control protocol do.
fn invoke(app: &mut PhotocraftApp, id: &str, params: Value) -> Result<Value, String> {
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(app, &app_ctx, id, params)
}

/// The workspace as saved JSON: equal values mean an unchanged workspace.
fn saved(app: &PhotocraftApp) -> Value {
    serde_json::to_value(&app.ui.docking).unwrap()
}

fn floating(app: &PhotocraftApp, panel: &str) -> bool {
    app.ui.docking.layout.floating.iter().any(|window| window.contains(panel))
}

/// The panel's group is minimized.
fn minimized(app: &PhotocraftApp, panel: &str) -> bool {
    let layout = &app.ui.docking.layout;
    layout.place(&panel.to_owned()).is_some_and(|place| layout.group(&place).is_some_and(|group| group.minimized))
}

/// The panel's column is collapsed to icons.
fn iconic(app: &PhotocraftApp, panel: &str) -> bool {
    let layout = &app.ui.docking.layout;
    layout.place(&panel.to_owned()).and_then(|place| layout.column(&place)).is_some_and(|column| column.iconic)
}

/// The selected tab of the group holding `panel`.
fn active_tab(app: &PhotocraftApp, panel: &str) -> Option<String> {
    let layout = &app.ui.docking.layout;
    let place = layout.place(&panel.to_owned())?;
    layout.group(&place)?.active_panel().cloned()
}

fn control_set(app: &mut PhotocraftApp, ctx: &egui::Context, params: Value) -> Value {
    let (request, _) = crate::control::ControlRequest::new("ui.set", params);
    match crate::control::handle(app, ctx, &request) {
        crate::control::Reply::Done(value) => value,
        _ => panic!("ui.set must reply synchronously"),
    }
}

/// The columns of the end dock.
fn end_columns(layout: &Layout<String>) -> Option<&Vec<Column<String>>> {
    fn find(node: &Node<String>) -> Option<&Vec<Column<String>>> {
        match node {
            Node::Dock { side: DockEdge::End, columns, .. } => Some(columns),
            Node::Split { first, second, .. } => find(second).or_else(|| find(first)),
            _ => None,
        }
    }
    find(layout.root.as_ref()?)
}

type Check = fn(&PhotocraftApp) -> bool;

/// The `window.panel.*`, `window.toggle.*`, `type.panels.*` and `ui.dock` ids are the automation
/// contract (`docs/control-protocol.md`): each family changes the workspace as documented, and a
/// refused request leaves it equal.
#[test]
fn window_panel_ids_drive_the_workspace() {
    let mut app = app();
    assert!(!is_shown(&app, "character"), "Character starts closed in the default workspace");
    assert!(is_shown(&app, "navigator") && iconic(&app, "navigator") && !is_front(&app, "navigator"), "Navigator starts as an icon");

    // One row per request: id, params, and what holds afterwards. `row` makes each closure a `Check`.
    let row = |id: &'static str, params: Value, holds: Check| (id, params, holds);
    let rows = vec![
        // Without `show`: a hidden panel comes to the front, a front one closes (both id families).
        row("window.panel.character", json!({}), |a| is_front(a, "character")),
        row("window.panel.character", json!({}), |a| !is_shown(a, "character")),
        row("window.toggle.character", json!({}), |a| is_front(a, "character")),
        row("window.toggle.character", json!({}), |a| !is_shown(a, "character")),
        // With `show`: true never hides, even on the second call.
        row("window.toggle.character", json!({"show": true}), |a| is_front(a, "character")),
        row("window.toggle.character", json!({"show": true}), |a| is_front(a, "character")),
        row("window.toggle.character", json!({"show": false}), |a| !is_shown(a, "character")),
        // A panel in a column of icons opens its flyout and, toggled again, closes it; it stays in the layout.
        row("window.panel.navigator", json!({}), |a| is_front(a, "navigator") && iconic(a, "navigator")),
        row("window.panel.navigator", json!({}), |a| is_shown(a, "navigator") && !is_front(a, "navigator")),
        row("window.panel.brushes", json!({}), |a| is_front(a, "brushes")),
        row("window.panel.brushes", json!({"show": false}), |a| is_shown(a, "brushes") && !is_front(a, "brushes")),
        // A tab behind another one comes to the front.
        row("window.panel.channels", json!({}), |a| is_front(a, "channels") && !is_front(a, "layers")),
        row("window.toggle.layers", json!({"show": true}), |a| is_front(a, "layers")),
        // A minimized group opens again.
        row("window.panel.layout", json!({"action": {"SetMinimized": {"panel": "layers", "minimized": true}}}), |a| {
            minimized(a, "layers") && !is_front(a, "layers")
        }),
        row("window.panel.layers", json!({}), |a| is_front(a, "layers") && !minimized(a, "layers")),
        // The Type panels' ids: the style panels and Glyphs toggle; Character and Paragraph only show.
        row("type.panels.characterStyles", json!({}), |a| is_front(a, "characterStyles")),
        row("type.panels.characterStyles", json!({}), |a| !is_shown(a, "characterStyles")),
        row("type.panels.paragraphStyles", json!({}), |a| is_front(a, "paragraphStyles")),
        row("type.panels.paragraphStyles", json!({}), |a| !is_shown(a, "paragraphStyles")),
        row("type.panels.glyphs", json!({}), |a| is_front(a, "glyphs")),
        row("type.panels.glyphs", json!({}), |a| !is_shown(a, "glyphs")),
        row("type.panels.character", json!({}), |a| is_front(a, "character")),
        row("type.panels.character", json!({}), |a| is_front(a, "character")),
        row("type.panels.paragraph", json!({}), |a| is_front(a, "paragraph")),
        row("type.panels.paragraph", json!({}), |a| is_front(a, "paragraph")),
        // Windows: float, minimize and restore the window, the shared action form, and `ui.dock`.
        row("window.panel.float", json!({"panel": "properties", "x": 50, "y": 70, "width": 330, "height": 410}), |a| floating(a, "properties")),
        row("window.panel.minimize", json!({"panel": "properties", "minimized": true}), |a| minimized(a, "properties")),
        row("window.panel.minimize", json!({"panel": "properties", "minimized": false}), |a| !minimized(a, "properties")),
        // A column collapses to icons and expands again; a floating column does too, and keeps its icons floating.
        row("window.panel.iconic", json!({"panel": "properties"}), |a| iconic(a, "properties") && floating(a, "properties")),
        row("window.panel.iconic", json!({"panel": "properties", "iconic": false}), |a| !iconic(a, "properties")),
        row("window.panel.history", json!({"show": true}), |a| is_front(a, "history")),
        row("window.panel.layout", json!({"action": {"Float": {"panel": "history", "rect": [40, 60, 300, 300]}}}), |a| floating(a, "history")),
        row("ui.dock", json!({"action": {"Close": {"panel": "history"}}}), |a| !is_shown(a, "history")),
        // Moves: a tab joins another panel's group and goes before it.
        row("window.panel.move", json!({"panel": "swatches", "anchor": "layers"}), |a| probe::group_of(a, "layers").contains(&"swatches".to_owned())),
        row("window.panel.move", json!({"panel": "gradients", "anchor": "layers", "before": "layers"}), |a| {
            probe::group_of(a, "layers").first().map(String::as_str) == Some("gradients")
        }),
    ];
    for (id, params, holds) in rows {
        invoke(&mut app, id, params.clone()).unwrap_or_else(|e| panic!("{id} {params}: {e}"));
        assert!(holds(&app), "{id} {params}");
    }

    // Closing a group closes every panel of it.
    let members = probe::group_of(&app, "layers");
    assert!(members.len() > 1);
    invoke(&mut app, "window.panel.closeGroup", json!({"panel": "layers"})).unwrap();
    assert!(members.iter().all(|panel| !is_shown(&app, panel)), "{members:?}");
    invoke(&mut app, "window.panel.layers", json!({"show": true})).unwrap();
    assert!(is_front(&app, "layers"));
    assert!(invoke(&mut app, "window.panel.iconic", json!({"panel": "toolbar"})).is_err(), "the toolbar never collapses to icons");

    // A floated panel docks back to where it floated from; a floating group docks all of its panels.
    let mut app = self::app();
    let before = app.ui.docking.layout.clone();
    invoke(&mut app, "window.panel.float", json!({"panel": "properties", "x": 50, "y": 70, "width": 330, "height": 410})).unwrap();
    assert!(floating(&app, "properties"));
    assert!(app.ui.docking.floated.iter().any(|(panel, _)| panel == "properties"), "the workspace records where it floated from");
    invoke(&mut app, "window.panel.dock", json!({"panel": "properties"})).unwrap();
    assert_eq!(app.ui.docking.layout, before, "the panel is back where it was");

    invoke(&mut app, "window.panel.float", json!({"panel": "paths"})).unwrap();
    invoke(&mut app, "window.panel.move", json!({"panel": "history", "anchor": "paths"})).unwrap();
    assert!(app.ui.docking.layout.floating.iter().any(|w| w.contains("paths") && w.contains("history")));
    invoke(&mut app, "window.panel.dock", json!({"panel": "paths", "group": true})).unwrap();
    assert!(!floating(&app, "paths") && !floating(&app, "history"), "{:?}", app.ui.docking.layout.floating);
    assert!(is_shown(&app, "paths") && is_shown(&app, "history"));

    // A panel no workspace has placed opens in its own group, and the panels of that group meet there.
    let mut app = self::app();
    for panel in ["navigator", "info", "histogram"] {
        invoke(&mut app, &format!("window.panel.{panel}"), json!({"show": true})).unwrap();
    }
    let mut group = probe::group_of(&app, "info");
    group.sort();
    assert_eq!(group, ["histogram", "info", "navigator"]);

    // A refused request changes nothing, and Lock Workspace refuses moves but still opens panels.
    let mut app = self::app();
    for (id, params) in [
        ("window.panel.move", json!({"panel": "layers", "anchor": "unknown"})),
        ("window.panel.move", json!({"panel": "layers", "anchor": "properties", "before": "unknown"})),
        ("window.panel.move", json!({"panel": "layers", "anchor": "properties", "zone": 42})),
        ("window.panel.float", json!({"panel": "layers", "width": -10})),
        ("window.panel.float", json!({"panel": "unknown"})),
        ("window.panel.float", json!({"panel": "layers", "x": "invalid"})),
        ("window.panel.layers", json!({"show": "yes"})),
        ("window.panel.layout", json!({"action": {"Open": {"panel": "unknown", "anchor": null}}})),
    ] {
        let before = saved(&app);
        assert!(invoke(&mut app, id, params.clone()).is_err(), "{id} {params}");
        assert_eq!(saved(&app), before, "{id} {params}");
    }

    invoke(&mut app, "workspace.lock", json!({})).unwrap();
    let before = saved(&app);
    assert!(invoke(&mut app, "window.panel.float", json!({"panel": "layers"})).is_err());
    assert_eq!(saved(&app), before, "a locked workspace refuses the move");
    invoke(&mut app, "window.panel.channels", json!({"show": true})).unwrap();
    assert!(is_front(&app, "channels"), "opening a panel stays available");
}

/// A saved workspace keeps its custom tree and where a closed panel was; the save carries the
/// layout and the chrome flags.
#[test]
fn saved_workspace_restores_the_custom_tree_and_hidden_panel_placement() {
    let mut app = app();
    invoke(&mut app, "window.panel.move", json!({"panel": "swatches", "anchor": "layers", "zone": "top"})).unwrap();
    invoke(&mut app, "window.panel.float", json!({"panel": "properties", "x": 50, "y": 80})).unwrap();
    invoke(&mut app, "window.panel.close", json!({"panel": "swatches"})).unwrap();
    assert!(app.ui.docking.hidden.iter().any(|(panel, _)| panel == "swatches"));

    let written = crate::dock::snapshot(&app);
    let mut reloaded = self::app();
    crate::dock::apply(&mut reloaded, &written);
    assert_eq!(reloaded.ui.docking.layout, app.ui.docking.layout);
    assert_eq!(reloaded.ui.docking.hidden, app.ui.docking.hidden);
    invoke(&mut reloaded, "window.panel.swatches", json!({"show": true})).unwrap();
    assert!(is_front(&reloaded, "swatches"), "the closed panel reopens");
    assert!(!probe::group_of(&reloaded, "layers").contains(&"swatches".to_owned()), "at its own place, in a group above Layers");
}

/// A theme switch keeps the customised workspace: there is one arrangement in every theme.
#[test]
fn a_theme_switch_keeps_a_customised_workspace() {
    let mut app = app();
    let ctx = app.ctx().clone();
    apply(&mut app, vec![Action::move_beside("color".to_owned(), "layers".to_owned(), DockPlacement::tab(None))]).unwrap();
    let before = saved(&app);
    app.set_theme(&ctx, crate::theme::AppTheme::studio());
    assert_eq!(saved(&app), before);
    app.set_theme(&ctx, crate::theme::AppTheme::pro_medium());
    assert_eq!(saved(&app), before);
}

/// A save written before the dock was a workspace (group flags, tab picks, collapsed groups and
/// hidden tabs) loads once into the default workspace, and what it writes back loads unchanged.
#[test]
fn a_pre_docking_save_migrates_once() {
    let old = json!({
        "panels": {
            "layers": true, "history": true, "properties": true, "color": true, "navigator": false,
            "toolbar": true, "options_bar": true, "status_bar": true, "brush_settings": false, "brushes": false,
            "character": false, "toolbar_double": false, "dock": true, "rail": true, "menu_bar": true
        },
        "dockTabs": {"properties": 0, "color": 2, "layers": 0, "navigator": 0, "history": 1, "character": 0, "brushes": 0},
        "dock": {
            "order": [], "heights": {}, "collapsed": ["properties"],
            "hidden_tabs": {"color": ["Swatches"], "layers": ["Paths"], "history": ["Layer Comps"]},
            "tabOrder": {}
        },
        "timelineOpen": false
    });
    let mut app = app();
    crate::dock::apply(&mut app, &old);
    assert!(!is_shown(&app, "swatches") && !is_shown(&app, "paths") && !is_shown(&app, "layerComps"), "hidden tabs are closed");
    assert!(is_shown(&app, "history") && active_tab(&app, "history").as_deref() == Some("actions"), "the history group opens on its picked tab");
    assert!(is_front(&app, "gradients"), "the color group's picked tab is in front");
    assert!(minimized(&app, "properties"), "the collapsed group is minimized");

    let written = crate::dock::snapshot(&app);
    assert!(written["docking"].is_object());
    assert!(written.get("dockTabs").is_none() && written.get("dock").is_none(), "the old keys are not written back");
    let mut again = self::app();
    crate::dock::apply(&mut again, &written);
    assert_eq!((&again.ui.docking.layout, &again.ui.docking.hidden), (&app.ui.docking.layout, &app.ui.docking.hidden), "a second load changes nothing");
}

/// `ui.set` maps the old panel and dock words onto workspace changes, all fields or none.
#[test]
fn ui_set_dock_fields_map_onto_the_workspace_atomically() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let ok = |app: &mut PhotocraftApp, params: Value| control_set(app, &ctx, params)["ok"] == true;

    assert!(ok(&mut app, json!({"dockTabs": {"layers": 1}})));
    assert!(is_front(&app, "channels"), "a tab index picks the panel of the group's one tab order");
    assert!(ok(&mut app, json!({"panels": {"layers": false}})));
    assert!(["layers", "channels", "paths"].iter().all(|panel| !is_shown(&app, panel)));
    assert!(ok(&mut app, json!({"panels": {"layers": true}, "dockTabs": {"layers": 2}})), "a group opens and picks a tab in one request");
    assert!(is_front(&app, "paths"));
    assert!(ok(&mut app, json!({"dock": {"collapsed": ["color"]}})));
    assert!(minimized(&app, "color") && !minimized(&app, "properties"));
    assert!(ok(&mut app, json!({"dock": {"hiddenTabs": {"color": ["Swatches"]}}})));
    assert!(!is_shown(&app, "swatches"));
    assert!(ok(&mut app, json!({"panels": {"layers": false}})) && ok(&mut app, json!({"panels": {"layers": true}})));
    assert!(is_front(&app, "layers"), "a group the old words open shows its first tab");

    // The end dock's total after a `dockWidth` request.
    assert!(ok(&mut app, json!({"dockWidth": 400.0})));
    let sizes = DockSizes::of(&ctx);
    let total: f32 =
        end_columns(&app.ui.docking.layout).expect("the end dock").iter().map(|column| sizes.width(column, &|panel: &String| spec(&app, panel))).sum();
    assert!((total - 400.0).abs() < 1.0, "{total}");

    // One bad field applies nothing, whatever the good fields say.
    let before = saved(&app);
    for request in [
        json!({"panels": {"color": false}, "dockTabs": {"layers": 999}}),
        json!({"tool": "brush", "panels": {"navigator": true}, "dock": {"order": ["layers", "color"]}}),
        json!({"panels": {"navigator": true}, "dock": {"collapsed": ["nowhere"]}}),
        json!({"panels": {"navigator": true}, "dockWidth": "wide"}),
    ] {
        assert!(!ok(&mut app, request.clone()), "{request}");
        assert_eq!(saved(&app), before, "{request}");
    }
}

/// A save carries the Timeline as a panel of the layout and leaves the host's chrome flag to the host.
#[test]
fn a_saved_workspace_keeps_the_timeline_and_leaves_out_host_chrome() {
    let mut app = app();
    show_panel(&mut app, "timeline", Some(true)).unwrap();
    app.ui.panels.menu_bar = false;
    apply(&mut app, vec![Action::set_minimized("layers".to_owned(), true)]).unwrap();
    let written = crate::dock::snapshot(&app);

    let mut restored = self::app();
    assert!(restored.ui.panels.menu_bar);
    crate::dock::apply(&mut restored, &written);
    assert!(is_shown(&restored, "timeline"), "the Timeline's open state is in the layout");
    assert!(!restored.ui.timeline.playing());
    assert!(minimized(&restored, "layers"));
    assert!(restored.ui.panels.menu_bar, "the host keeps its own chrome flag");

    // A layout without the Timeline closes it.
    crate::dock::apply(&mut restored, &crate::dock::snapshot(&self::app()));
    assert!(!is_shown(&restored, "timeline"));
}

/// A craft-ui v2 save written by main must keep loading: every panel is shown or closed with a place, the toolbar it
/// predates docks at the start, and a change writes the versioned workspace.
#[test]
fn main_workspace_file_loads_as_the_default_and_saves_as_a_workspace() {
    let saved: Value = serde_json::from_str(include_str!("../../../tests/fixtures/main-docking-workspace.json")).unwrap();
    let mut app = app();
    crate::dock::apply(&mut app, &saved);
    app.ui.docking.validate().expect("the loaded workspace is valid");
    for entry in registry::PANELS {
        assert!(is_shown(&app, entry.id) || app.ui.docking.hidden.iter().any(|(panel, _)| panel == entry.id), "{} is shown or can be reopened", entry.id);
    }
    assert!(is_shown(&app, "layers"));
    // The save predates the toolbar panel and has no start dock: the toolbar comes back docked at the start.
    assert_eq!(toolbar_edge(&app), Some(DockEdge::Start), "the toolbar is docked at the start: {:?}", app.ui.docking.layout);

    invoke(&mut app, "window.panel.float", json!({"panel": "properties"})).unwrap();
    let written = crate::dock::snapshot(&app);
    assert!(written.get("dockingHidden").is_none());
    assert_eq!(written["docking"]["version"], json!(craft::behavior::dock::Workspace::<String>::VERSION));
    let mut reloaded = self::app();
    crate::dock::apply(&mut reloaded, &written);
    assert_eq!((&reloaded.ui.docking.layout, &reloaded.ui.docking.hidden), (&app.ui.docking.layout, &app.ui.docking.hidden));
}

/// The dock edge the toolbar's column is docked on.
fn toolbar_edge(app: &PhotocraftApp) -> Option<DockEdge> {
    let layout = &app.ui.docking.layout;
    let Some(PanelPlace::Dock { path, .. }) = layout.place(&registry::TOOLBAR.to_owned()) else { return None };
    [DockEdge::Start, DockEdge::End].into_iter().find(|edge| layout.dock_path(*edge).as_ref() == Some(&path))
}

/// Essentials shows Color, Properties and Layers in front, and Navigator, History and Brushes as the icons of a collapsed
/// column beside the canvas.
#[test]
fn essentials_keeps_navigator_history_and_brushes_as_icons() {
    let app = app();
    for panel in ["color", "properties", "layers"] {
        assert!(is_front(&app, panel) && !iconic(&app, panel), "{panel} shows in the expanded column");
    }
    for panel in ["navigator", "history", "brushes"] {
        assert!(iconic(&app, panel), "{panel} is an icon");
        assert_eq!(app.ui.docking.layout.panel_state(&panel.to_owned()), PanelState::Open, "{panel} is open but not showing");
    }
    assert_eq!(toolbar_edge(&app), Some(DockEdge::Start), "the toolbar has the start dock to itself");
}

/// The Window menu's row of a panel in a collapsed column opens its flyout and, picked again, closes it: the layout does
/// not change either time, and the row is checked while the flyout is open.
#[test]
fn the_window_menu_opens_and_closes_the_flyout_of_an_icon_panel() {
    let mut app = app();
    let before = saved(&app);
    let checked =
        |app: &PhotocraftApp| crate::menus::menu_rows(app).iter().find(|row| row.command.id == "window.panel.navigator").and_then(|row| row.kind.checked());
    assert_eq!(checked(&app), Some(false));
    invoke(&mut app, "window.panel.navigator", json!({})).unwrap();
    assert_eq!(checked(&app), Some(true), "the flyout is open");
    assert_eq!(app.ui.flyout.as_deref(), Some("navigator"));
    invoke(&mut app, "window.panel.navigator", json!({})).unwrap();
    assert_eq!(checked(&app), Some(false), "the flyout is closed");
    assert_eq!(saved(&app), before, "the layout never changed");
}

/// The toolbar takes no tab, group or column of icons, floats as its own column, and docks at either edge.
#[test]
fn the_toolbar_shares_nothing_floats_and_docks_at_either_edge() {
    let mut app = app();
    let before = saved(&app);
    for (id, params) in [
        ("window.panel.move", json!({"panel": "toolbar", "anchor": "layers"})),
        ("window.panel.move", json!({"panel": "toolbar", "anchor": "layers", "zone": "top"})),
        ("window.panel.move", json!({"panel": "layers", "anchor": "toolbar"})),
        ("window.panel.move", json!({"panel": "layers", "anchor": "toolbar", "zone": "bottom"})),
        ("window.panel.iconic", json!({"panel": "toolbar"})),
    ] {
        assert!(invoke(&mut app, id, params.clone()).is_err(), "{id} {params}");
        assert_eq!(saved(&app), before, "{id} {params}");
    }

    invoke(&mut app, "window.panel.float", json!({"panel": "toolbar", "column": true, "x": 80, "y": 90})).unwrap();
    assert!(floating(&app, "toolbar") && toolbar_edge(&app).is_none());
    invoke(&mut app, "window.panel.dock", json!({"panel": "toolbar", "column": true, "side": "end"})).unwrap();
    assert_eq!(toolbar_edge(&app), Some(DockEdge::End));
    invoke(&mut app, "window.panel.dock", json!({"panel": "toolbar", "column": true, "side": "start"})).unwrap();
    assert_eq!(toolbar_edge(&app), Some(DockEdge::Start));
}

/// `ui.set toolbarColumns` sets the toolbar column's width in tool columns, from one to the toolbar's cells.
#[test]
fn ui_set_toolbar_columns_sets_the_toolbar_columns_steps() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let steps = |app: &PhotocraftApp| {
        let layout = &app.ui.docking.layout;
        layout.place(&registry::TOOLBAR.to_owned()).and_then(|place| layout.column(&place)).and_then(|column| column.steps)
    };
    assert!(control_set(&mut app, &ctx, json!({"toolbarColumns": 3}))["ok"] == true);
    assert_eq!(steps(&app), Some(3));
    assert!(control_set(&mut app, &ctx, json!({"toolbarColumns": 1}))["ok"] == true);
    assert_eq!(steps(&app), Some(1));
    for bad in [json!(0), json!(10_000), json!("two")] {
        let before = saved(&app);
        assert!(control_set(&mut app, &ctx, json!({"toolbarColumns": bad.clone()}))["ok"] != true, "{bad}");
        assert_eq!(saved(&app), before, "{bad}");
    }
}
