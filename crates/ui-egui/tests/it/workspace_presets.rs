//! #1120: workspace selection/reset must apply useful layouts, including saved layouts.

use photocraft_engine::Session;
use photocraft_ui_egui::dock;
use photocraft_ui_egui::{PhotocraftApp, menus};
use serde_json::{Value, json};

/// The panels the presets differ in, in the order [`shown`] reports them.
const PANELS: [&str; 6] = ["color", "properties", "navigator", "history", "character", "layers"];

const PRESETS: [(&str, &str, &[&str], bool); 6] = [
    ("essentials", "Essentials", &["color", "properties", "navigator", "history", "layers"], false),
    ("photography", "Photography", &["properties", "navigator", "history", "layers"], false),
    ("painting", "Painting", &["color", "layers"], false),
    ("graphicAndWeb", "Graphic and Web", &["properties", "character", "layers"], false),
    ("pixelArt", "Pixel Art", &["color", "navigator", "layers"], false),
    ("motion", "Motion", &["properties", "layers"], true),
];

fn app() -> PhotocraftApp {
    PhotocraftApp::new(Session::new(), Default::default())
}

fn invoke(app: &mut PhotocraftApp, id: &str, params: Value) {
    let ctx = app.ctx().clone();
    menus::invoke(app, &ctx, id, params).unwrap();
}

fn select(app: &mut PhotocraftApp, suffix: &str) {
    invoke(app, &format!("window.workspace.{suffix}"), json!({}));
}

/// Whether the workspace holds `panel`, in a column of icons or not.
fn is_open(app: &PhotocraftApp, panel: &str) -> bool {
    app.ui.docking.layout.contains(&panel.to_owned())
}

fn shown(app: &PhotocraftApp) -> Vec<&'static str> {
    PANELS.into_iter().filter(|panel| is_open(app, panel)).collect()
}

fn layout(app: &PhotocraftApp) -> Value {
    json!({"current": app.workspaces.set.current, "shown": shown(app), "timeline": is_open(app, "timeline"), "dock": app.ui.docking})
}

#[test]
fn reported_presets_have_distinct_useful_layouts() {
    let mut app = app();
    let mut failures = Vec::new();
    for (suffix, name, panels, timeline) in PRESETS.into_iter().skip(3) {
        select(&mut app, suffix);
        if shown(&app) != panels || is_open(&app, "timeline") != timeline {
            failures.push(format!("{name}: {:?}, timeline={}", shown(&app), is_open(&app, "timeline")));
        }
    }
    assert!(failures.is_empty(), "incorrect layouts: {failures:?}");
}

#[test]
fn presets_select_expected_panels_and_reset_every_dock_group() {
    let mut app = app();
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    for (suffix, name, panels, timeline) in PRESETS {
        select(&mut app, suffix);
        assert_eq!(app.workspaces.set.current, suffix);
        assert_eq!(shown(&app), panels, "{name}");
        assert_eq!(is_open(&app, "timeline"), timeline, "{name}");
        let id = format!("window.workspace.{suffix}");
        assert_eq!(menus::menu_rows(&app).iter().find(|item| item.command.id == id).unwrap().kind.checked(), Some(true));
        let expected = layout(&app);
        invoke(&mut app, "window.panel.character", json!({}));
        invoke(&mut app, "window.panel.navigator", json!({}));
        invoke(&mut app, "window.panel.timeline", json!({}));
        assert_ne!(layout(&app), expected, "the edits change the layout of {name}");
        invoke(&mut app, "workspace.reset", json!({}));
        assert_eq!(layout(&app), expected, "reset {name}");
        assert_eq!(app.session.active().unwrap().doc, doc, "layout must not create/edit a timeline");
    }
}

#[test]
fn presets_are_deterministic_from_every_other_preset_and_select_route() {
    for (suffix, name, _, _) in PRESETS {
        let mut baseline = app();
        select(&mut baseline, suffix);
        let expected = layout(&baseline);
        for (prior, _, _, _) in PRESETS {
            let mut app = app();
            select(&mut app, prior);
            invoke(&mut app, "window.workspace.select", json!({"name": name}));
            assert_eq!(shown(&app), shown(&baseline), "{prior} -> {name}");
            assert_eq!(app.workspaces.set.current, expected["current"], "{prior} -> {name}");
        }
    }
}

#[test]
fn preset_layouts_round_trip_through_remembered_preferences() {
    for (suffix, _, _, _) in PRESETS {
        let mut app = app();
        select(&mut app, suffix);
        let expected = layout(&app);
        dock::persist(&mut app);
        let mut session = Session::new();
        session.load_prefs_json(&app.session.prefs_to_json()).unwrap();
        let mut restored = PhotocraftApp::new(session, Default::default());
        dock::restore(&mut restored);
        assert_eq!(layout(&restored), expected, "restore {suffix}");
    }
}

#[test]
fn open_timeline_is_saved_without_saving_playback() {
    let mut app = app();
    invoke(&mut app, "window.panel.timeline", json!({}));
    assert!(is_open(&app, "timeline"));
    invoke(&mut app, "workspace.new", json!({"name": "Open timeline"}));
    dock::persist(&mut app);
    let prefs = app.session.prefs_to_json();
    let mut session = Session::new();
    session.load_prefs_json(&prefs).unwrap();
    let mut restored = PhotocraftApp::new(session, Default::default());
    dock::restore(&mut restored);
    assert!(is_open(&restored, "timeline"));
    assert!(!restored.ui.timeline.playing());
    select(&mut restored, "essentials");
    invoke(&mut restored, "window.workspace.select", json!({"name": "Open timeline"}));
    assert!(is_open(&restored, "timeline"));
    assert!(!restored.ui.timeline.playing());
}

#[test]
fn custom_workspaces_restore_timeline_visibility_and_dock_after_presets() {
    let mut app = app();
    for (name, suffix) in [("Custom still", "pixelArt"), ("Custom motion", "motion")] {
        select(&mut app, suffix);
        invoke(&mut app, "window.panel.history", json!({}));
        let expected = layout(&app);
        invoke(&mut app, "workspace.new", json!({"name": name}));
        let mut session = Session::new();
        session.load_prefs_json(&app.session.prefs_to_json()).unwrap();
        let mut restored = PhotocraftApp::new(session, Default::default());
        select(&mut restored, if suffix == "motion" { "essentials" } else { "motion" });
        invoke(&mut restored, "window.workspace.select", json!({"name": name}));
        assert_eq!(shown(&restored), shown(&app), "custom select {name}");
        assert_eq!(is_open(&restored, "timeline"), expected["timeline"], "custom select {name}");
        invoke(&mut restored, "window.panel.timeline", json!({}));
        invoke(&mut restored, "window.panel.color", json!({}));
        invoke(&mut restored, "workspace.reset", json!({}));
        assert_eq!(shown(&restored), shown(&app), "custom reset {name}");
        assert_eq!(is_open(&restored, "timeline"), expected["timeline"], "custom reset {name}");
    }
}

#[test]
fn legacy_workspace_without_timeline_visibility_still_loads() {
    let mut app = app();
    select(&mut app, "painting");
    let painting = app.ui.docking.clone();
    // PLAN: assumed the integration crate reaches `craft` (the facade) as the app crate does.
    app.workspaces.set.saved.push(craft::behavior::dock::NamedWorkspace::new("legacy", "Legacy", painting));
    select(&mut app, "motion");
    invoke(&mut app, "window.workspace.select", json!({"name": "Legacy"}));
    assert_eq!(shown(&app), ["color", "layers"]);
    assert!(!is_open(&app, "timeline"));
}

/// A preset replaces the arrangement: the shell preferences and the lock stay, and the toolbar, a panel of the
/// arrangement, is in every preset.
#[test]
fn presets_preserve_shell_preferences_and_workspace_lock() {
    let mut app = app();
    app.ui.panels.options_bar = false;
    app.ui.panels.status_bar = false;
    app.workspaces.set.locked = true;
    for (suffix, _, _, _) in PRESETS {
        select(&mut app, suffix);
        assert!(is_open(&app, "toolbar"), "{suffix} has the toolbar");
        assert!(!app.ui.panels.options_bar);
        assert!(!app.ui.panels.status_bar);
        assert!(app.workspaces.set.locked);
    }
}
