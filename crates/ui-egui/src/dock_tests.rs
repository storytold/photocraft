//! Dock layout tests (#88): panes of modules, heights, splitters, collapse, reorder, tab drag,
//! the Studio rail and picker, persistence and migration of pre-module layouts.

use egui::{Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::*;
use crate::theme::ThemeKind;

fn fronts(l: &DockLayout) -> Vec<&str> {
    l.panes.iter().map(Pane::front).collect()
}

fn tabs(l: &DockLayout, id: &str) -> Vec<String> {
    l.pane_of(id).and_then(|i| l.panes.get(i)).map(|s| s.tabs.clone()).unwrap_or_default()
}

fn pane<'a>(l: &'a DockLayout, id: &str) -> &'a Pane {
    &l.panes[l.pane_of(id).unwrap()]
}

fn pane_mut<'a>(l: &'a mut DockLayout, id: &str) -> &'a mut Pane {
    let i = l.pane_of(id).unwrap();
    &mut l.panes[i]
}

#[test]
fn last_expanded_pane_fills_the_column() {
    let l = DockLayout::default();
    let hs = l.heights_for(1200.0, 28.0);
    assert_eq!(hs[0], l.panes[0].default_height());
    assert_eq!(hs[1], l.panes[1].default_height());
    let total: f32 = hs.iter().sum::<f32>() + 2.0 * GAP;
    assert!((total - 1200.0).abs() < 1e-3, "{hs:?}");
    // Collapsed panes shrink to the tab strip; the one above Layers keeps its height.
    let mut l = DockLayout::default();
    l.set_collapsed(0, true);
    let hs = l.heights_for(1000.0, 28.0);
    assert_eq!(hs[0], 28.0);
    assert_eq!(hs[1], l.panes[1].default_height());
    // Layers collapsed: Properties becomes the filler.
    l.set_collapsed(2, true);
    let hs = l.heights_for(800.0, 28.0);
    assert_eq!(hs[2], 28.0);
    assert!((hs[1] - (800.0 - 56.0 - 2.0 * GAP)).abs() < 1e-3);
}

#[test]
fn short_columns_squeeze_panes_down_to_their_minimum_and_never_go_negative() {
    let l = DockLayout::default();
    // Leave enough room for the fixed controls and a partial squeeze of Properties.
    let avail = l.panes[0].compact_height() + l.panes[1].min_height() + l.panes[2].min_height() + 2.0 * GAP + 10.0;
    let hs = l.heights_for(avail, 28.0);
    assert_eq!(hs[2], l.panes[2].min_height(), "Layers keeps its minimum: {hs:?}");
    assert_eq!(hs[0], l.panes[0].compact_height(), "the pane farthest from Layers gives way last");
    assert!(hs[1] < l.panes[1].compact_height());
    for avail in [0.0, -50.0, 1.0, f32::NAN, f32::INFINITY, 1e9] {
        for h in l.heights_for(avail, 28.0) {
            assert!(h.is_finite() && h >= 0.0, "{avail}: {h}");
        }
    }
    assert!(DockLayout { panes: Vec::new(), ..Default::default() }.heights_for(500.0, 28.0).is_empty());
}

#[test]
fn bad_stored_values_are_sanitised() {
    let mut l: DockLayout = serde_json::from_value(json!({
        "panes": [
            {"tabs": ["layers", "layers", "bogus"], "active": "nope", "height": -10.0},
            {"tabs": ["layers", "color"], "height": 1e12},
            {"tabs": []},
            {"tabs": ["gone-plugin"]}
        ],
        "bogus": 1
    }))
    .unwrap();
    l.sanitize();
    assert_eq!(l.panes.len(), 2);
    assert_eq!(l.panes[0].tabs, ["layers"]);
    assert_eq!(l.panes[0].active, "layers");
    assert_eq!(l.panes[0].height, None);
    assert_eq!(l.panes[1].tabs, ["color"], "a module appears once");
    assert!(l.panes[1].height() <= MAX_HEIGHT);
    for bad in [f32::NAN, -10.0, f32::INFINITY, 1e12] {
        l.panes[1].height = Some(bad);
        let h = l.panes[1].height();
        assert!(h.is_finite() && h >= l.panes[1].min_height() && h <= MAX_HEIGHT, "{bad} -> {h}");
    }
    let many = DockLayout { panes: (0..500).map(|_| Pane::new(&["layers"])).collect(), ..Default::default() };
    let mut many = many;
    many.sanitize();
    assert_eq!(many.panes.len(), 1);
    // Old UI state without the field loads the default.
    let mut ui = serde_json::to_value(crate::state::UiState::default()).unwrap();
    ui.as_object_mut().unwrap().remove("dock");
    let ui: crate::state::UiState = serde_json::from_value(ui).unwrap();
    assert_eq!(ui.dock, DockLayout::default());
}

#[test]
fn move_pane_reorders() {
    let mut l = DockLayout::default();
    l.move_pane(2, Some(0));
    assert_eq!(fronts(&l), ["layers", "color", "properties"]);
    l.move_pane(0, None);
    assert_eq!(fronts(&l), ["color", "properties", "layers"]);
    l.move_pane(0, Some(0));
    assert_eq!(fronts(&l), ["color", "properties", "layers"]);
    l.move_pane(99, Some(0));
    l.move_pane(0, Some(99));
    assert_eq!(l.panes.len(), 3);
}

#[test]
fn tabs_join_other_panes_or_get_their_own() {
    let mut l = DockLayout::default();
    // Channels onto the Properties pane: it joins as the front tab.
    l.drop_tab("channels", Drop::Join(1));
    assert_eq!(tabs(&l, "properties"), ["properties", "adjustments", "channels"]);
    assert!(l.is_front("channels"));
    assert_eq!(tabs(&l, "layers"), ["layers", "paths"]);
    // History isn't docked; Paths gets a pane of its own at the top.
    l.drop_tab("paths", Drop::NewAt(0));
    assert_eq!(fronts(&l), ["paths", "color", "channels", "layers"]);
    // Dropping a lone tab where it already is changes nothing; neither does joining itself.
    let before = l.clone();
    l.drop_tab("paths", Drop::NewAt(0));
    l.drop_tab("paths", Drop::NewAt(1));
    l.drop_tab("paths", Drop::Join(0));
    l.drop_tab("paths", Drop::Join(99));
    l.drop_tab("history", Drop::Join(0));
    assert_eq!(l, before);
    // The last tab out empties its pane, which goes.
    l.drop_tab("paths", Drop::Join(3));
    assert_eq!(fronts(&l), ["color", "channels", "paths"]);
    assert_eq!(tabs(&l, "layers"), ["layers", "paths"]);
}

#[test]
fn add_panel_puts_a_module_in_the_pane_asked() {
    let mut l = DockLayout::default();
    l.drop_tab_or_add("history", 0);
    assert_eq!(tabs(&l, "color"), ["color", "swatches", "gradients", "patterns", "history"]);
    assert!(l.is_front("history"));
    l.drop_tab_or_add("channels", 0);
    assert_eq!(tabs(&l, "layers"), ["layers", "paths"]);
    assert!(l.is_front("channels"));
    let before = l.clone();
    l.drop_tab_or_add("bogus", 0);
    l.drop_tab_or_add("info", 99);
    assert_eq!(l, before);
}

#[test]
fn add_pane_is_always_a_pane_of_its_own() {
    let mut l = DockLayout::essentials(true);
    // A sibling (Character) is docked, but the + picker still makes a pane (show would join it).
    l.add_pane("character");
    l.add_pane("paragraph");
    assert_eq!(tabs(&l, "character"), ["character"]);
    assert_eq!(tabs(&l, "paragraph"), ["paragraph"]);
    // No group comes back with it, and the filler stays last.
    l.add_pane("actions");
    assert_eq!(tabs(&l, "actions"), ["actions"]);
    assert_eq!(l.panes.last().unwrap().tabs, ["layers", "channels", "paths"]);
    // Already docked: brought forward, nothing added; unknown ids do nothing.
    let n = l.panes.len();
    l.add_pane("layers");
    l.add_pane("bogus");
    assert_eq!(l.panes.len(), n);
}

#[test]
fn show_brings_back_a_whole_group_in_its_place_and_hide_closes_a_pane() {
    let mut l = DockLayout::essentials(true);
    l.show("info");
    assert_eq!(fronts(&l), ["color", "properties", "info", "layers"]);
    assert_eq!(tabs(&l, "info"), ["navigator", "histogram", "info"]);
    // A hidden module whose group-mate is docked joins it.
    l.close("histogram");
    l.show("histogram");
    assert_eq!(tabs(&l, "info"), ["navigator", "info", "histogram"]);
    // Collapsed panes expand when shown.
    let i = l.pane_of("layers").unwrap();
    l.set_collapsed(i, true);
    l.show("paths");
    assert!(!pane(&l, "paths").collapsed && l.is_front("paths"));
    l.hide("paths");
    assert!(!l.visible("layers") && !l.visible("channels"));
    // Layers comes back last, with its tabs.
    l.show("layers");
    assert_eq!(l.panes.last().unwrap().tabs, ["layers", "channels", "paths"]);
    assert!(!l.toggle("layers") && !l.visible("layers"));
    assert!(l.toggle("layers"));
}

#[test]
fn the_color_group_follows_the_theme_until_changed() {
    let mut l = DockLayout::essentials(true);
    l.follow_theme(false);
    assert_eq!(l, DockLayout::essentials(false));
    assert!(l.is_front("swatches"));
    l.follow_theme(true);
    assert_eq!(l, DockLayout::essentials(true));
    // Rearranged by the user: left alone.
    l.drop_tab("patterns", Drop::NewAt(0));
    let mine = l.clone();
    l.follow_theme(false);
    assert_eq!(l, mine);
}

#[test]
fn layouts_saved_before_modules_are_migrated() {
    let v = json!({
        "panels": {"layers": true, "history": true, "properties": false, "color": true, "navigator": false},
        "dockTabs": {"color": 2, "layers": 1, "history": 99},
        "dock": {"order": ["layers", "color"], "heights": {"color": 222.0}, "collapsed": ["history"], "hiddenTabs": {"layers": ["Paths"]}}
    });
    let l = from_legacy(&v, true);
    assert_eq!(fronts(&l), ["channels", "gradients", "history"]);
    assert_eq!(l.panes[0].tabs, ["layers", "channels"]);
    assert_eq!(pane(&l, "gradients").height, Some(222.0));
    assert!(pane(&l, "history").collapsed);
    // Studio's Color group started with Swatches: index 2 is still Gradients.
    assert!(from_legacy(&v, false).is_front("gradients"));
    // Nothing saved: the pre-module default (Color, Properties, Layers).
    assert_eq!(fronts(&from_legacy(&json!({}), true)), ["color", "properties", "layers"]);
    // Through `apply`, as a remembered `panelLayout` or a saved workspace.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    apply(&mut app, &v);
    assert_eq!(app.ui.dock, l);
    // Garbage never panics.
    for bad in [json!(null), json!({"dock": 3}), json!({"dock": {"order": [1, null], "heights": "x"}, "dockTabs": [1]}), json!({"panels": {"layers": "yes"}})] {
        let _ = from_legacy(&bad, true);
        apply(&mut app, &bad);
    }
}

#[test]
fn ui_set_takes_new_and_legacy_dock_keys() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let set = |app: &mut PhotocraftApp, p: Option<Value>, t: Option<Value>, d: Option<Value>| -> Result<(), String> {
        if let Some(l) = layout_from_control(app, p.as_ref(), t.as_ref(), d.as_ref())? {
            app.ui.dock = l;
        }
        Ok(())
    };
    set(&mut app, Some(json!({"history": true, "color": false, "toolbar": true})), None, None).unwrap();
    assert_eq!(fronts(&app.ui.dock), ["properties", "history", "layers"]);
    set(&mut app, None, Some(json!({"layers": 2})), None).unwrap();
    assert!(app.ui.dock.is_front("paths"));
    // A legacy `dock` keeps what is showing.
    set(&mut app, None, None, Some(json!({"order": ["layers"]}))).unwrap();
    assert_eq!(fronts(&app.ui.dock), ["paths", "properties", "history"]);
    let new = json!({"panes": [{"tabs": ["info", "layers"], "active": "layers"}]});
    set(&mut app, None, None, Some(new)).unwrap();
    assert_eq!(fronts(&app.ui.dock), ["layers"]);
    assert_eq!(layout_from_control(&app, Some(&json!({"toolbar": false})), None, None), Ok(None));
    for (p, t, d) in [
        (Some(json!({"layers": 3})), None, None),
        (None, Some(json!({"layers": 9})), None),
        (None, Some(json!({"bogus": 0})), None),
        (None, Some(json!([0])), None),
        (None, None, Some(json!({"panes": 4}))),
        (None, None, Some(json!({"nope": 1}))),
        (None, None, Some(json!(7))),
    ] {
        assert!(layout_from_control(&app, p.as_ref(), t.as_ref(), d.as_ref()).is_err(), "{p:?} {t:?} {d:?}");
    }
}

fn app_with_layers() -> (PhotocraftApp, photocraft_doc::LayerId, photocraft_doc::LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let pixel = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    let adj = app.session.active().unwrap().active_layer.unwrap();
    assert_ne!(pixel, adj);
    app.sync_views();
    (app, pixel, adj)
}

fn harness(app: PhotocraftApp, size: egui::Vec2, theme: ThemeKind) -> Harness<'static, PhotocraftApp> {
    // 60 fps steps, so two clicks a frame apart count as a double-click.
    let mut h = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::right_dock(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.state_mut().ui.theme = theme;
    h.run_steps(4);
    h
}

/// The rect of the pane holding module `id`.
fn rect_of(h: &Harness<'static, PhotocraftApp>, id: &str) -> Rect {
    module_rect(h.state(), &h.ctx, id).unwrap_or_else(|| panic!("{id} not drawn: {:?}", last_rects(&h.ctx)))
}

fn tab_rect(h: &Harness<'static, PhotocraftApp>, id: &str) -> Rect {
    last_strips(&h.ctx).into_iter().flat_map(|s| s.tabs).find(|(t, _)| t == id).map(|(_, r)| r).unwrap_or_else(|| panic!("tab {id} not on a strip"))
}

fn strip_of(h: &Harness<'static, PhotocraftApp>, id: &str) -> StripRects {
    let i = h.state().ui.dock.pane_of(id).unwrap();
    last_strips(&h.ctx).into_iter().find(|s| s.pane == i).unwrap()
}

fn drag(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, to: Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for i in 1..=6 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 6.0)));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, button: PointerButton) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(egui::Event::PointerButton { pos: p, button, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn is_pro(theme: ThemeKind) -> bool {
    matches!(theme, ThemeKind::Pro | ThemeKind::ProMedium)
}

#[test]
fn switching_layer_kinds_keeps_the_layers_panel_still() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        let (app, pixel, adj) = app_with_layers();
        let mut h = harness(app, vec2(1200.0, 800.0), theme);
        let rects = |h: &Harness<'static, PhotocraftApp>| last_rects(&h.ctx);
        let with_adj = rects(&h);
        assert!(with_adj.iter().any(|(g, _)| g == "layers"));
        h.state_mut().run("layer.select", json!({"layer": pixel.0})).unwrap();
        h.run_steps(4);
        assert_eq!(rects(&h), with_adj, "{theme:?}: selecting a pixel layer moved the dock panes");
        h.state_mut().run("layer.select", json!({"layer": adj.0})).unwrap();
        h.run_steps(4);
        assert_eq!(rects(&h), with_adj, "{theme:?}: selecting an adjustment layer moved the dock panes");
        // Properties ↔ Adjustments tabs don't move anything either.
        pane_mut(&mut h.state_mut().ui.dock, "properties").active = "adjustments".into();
        h.run_steps(3);
        assert_eq!(rects(&h).iter().map(|(_, r)| *r).collect::<Vec<_>>(), with_adj.iter().map(|(_, r)| *r).collect::<Vec<_>>());
    }
}

#[test]
fn dragging_the_splitter_resizes_and_survives_a_ui_state_round_trip() {
    let (app, _, _) = app_with_layers();
    let mut h = harness(app, vec2(1200.0, 800.0), ThemeKind::ProMedium);
    let props = rect_of(&h, "properties");
    let layers = rect_of(&h, "layers");
    let split = Pos2::new(props.center().x, props.bottom() + GAP / 2.0);
    let color = rect_of(&h, "color");
    drag(&mut h, split, split - vec2(0.0, 60.0));
    let props2 = rect_of(&h, "properties");
    let layers2 = rect_of(&h, "layers");
    assert!((props2.height() - (props.height() - 60.0)).abs() < 2.0, "{props:?} -> {props2:?}");
    assert!((layers2.top() - (layers.top() - 60.0)).abs() < 2.0, "{layers:?} -> {layers2:?}");
    assert_eq!(rect_of(&h, "color"), color, "the pane above keeps its place");
    assert_eq!(layers2.bottom(), layers.bottom());
    // Dragging past the minimum pushes the group above down to its minimum too (#2573), then stops.
    let split = Pos2::new(props2.center().x, props2.bottom() + GAP / 2.0);
    drag(&mut h, split, split - vec2(0.0, 600.0));
    assert_eq!(rect_of(&h, "properties").height(), Pane::new(&["properties"]).min_height());
    assert_eq!(rect_of(&h, "color").height(), Pane::new(&["color"]).min_height());
    assert_eq!(rect_of(&h, "layers").bottom(), layers.bottom());
    // Dragging back down grows Properties alone; Color stays where it was pushed.
    let from = Pos2::new(split.x, rect_of(&h, "properties").bottom() + GAP / 2.0);
    drag(&mut h, from, split);
    assert!((rect_of(&h, "properties").bottom() - props2.bottom()).abs() < 2.0);
    assert_eq!(rect_of(&h, "color").height(), Pane::new(&["color"]).min_height());

    // Save and restore the UI state (what `ui.inspect` / `ui.set` and workspaces carry).
    let saved = serde_json::to_value(&h.state().ui).unwrap();
    let (mut app2, _, _) = app_with_layers();
    app2.ui = serde_json::from_value(saved).unwrap();
    let h2 = harness(app2, vec2(1200.0, 800.0), ThemeKind::ProMedium);
    assert_eq!(last_rects(&h2.ctx), last_rects(&h.ctx));

    // Remembered in the preferences once the mouse is up, and restored at the next launch.
    let prefs = h.state().session.prefs_to_json();
    let mut s2 = photocraft_engine::Session::new();
    s2.load_prefs_json(&prefs).unwrap();
    let mut app3 = PhotocraftApp::new(s2, crate::Services::default());
    restore(&mut app3);
    assert_eq!(app3.ui.dock, h.state().ui.dock);
    // …unless Remember Workspace Changes is off.
    let mut s3 = photocraft_engine::Session::new();
    s3.load_prefs_json(&prefs).unwrap();
    s3.prefs.edit(|p| p.workspace.remember_workspace_changes = false);
    let mut app4 = PhotocraftApp::new(s3, crate::Services::default());
    restore(&mut app4);
    assert_eq!(app4.ui.dock, DockLayout::default());
}

/// `resize` on `shown` laid out in a column `avail` tall, then the new layout.
fn resized(l: &mut DockLayout, shown: &[&str], avail: f32, i: usize, dy: f32) -> Vec<f32> {
    assert_eq!(fronts(l), shown);
    let hs = l.heights_for(avail, 28.0);
    l.resize(&hs, i, dy);
    l.heights_for(avail, 28.0)
}

fn total(hs: &[f32]) -> f32 {
    hs.iter().sum::<f32>() + GAP * hs.len().saturating_sub(1) as f32
}

#[test]
fn dragging_a_splitter_down_pushes_every_group_below_in_turn() {
    let shown = ["color", "properties", "navigator", "layers"];
    let mut l = DockLayout { panes: shown.iter().map(|id| Pane::new(&[*id])).collect(), ..Default::default() };
    let before = l.heights_for(1200.0, 28.0);
    // Properties gives way first, then Navigator, then the filler; the column stays 1200.
    let hs = resized(&mut l, &shown, 1200.0, 0, 300.0);
    assert_eq!(hs[0], before[0] + 300.0, "{hs:?}");
    assert_eq!(hs[1], Pane::new(&["properties"]).min_height());
    assert_eq!(hs[2], before[2] - (300.0 - (before[1] - Pane::new(&["properties"]).min_height())));
    assert_eq!(hs[3], before[3], "the filler is untouched until the groups above it are at minimum");
    assert!((total(&hs) - 1200.0).abs() < 1e-3);
    // Far past the end: everything below is at its minimum and the drag stops there.
    let hs = resized(&mut l, &shown, 1200.0, 0, 5000.0);
    assert_eq!(hs[1], Pane::new(&["properties"]).min_height());
    assert_eq!(hs[2], Pane::new(&["navigator"]).min_height());
    assert_eq!(hs[3], Pane::new(&["layers"]).min_height());
    assert!((total(&hs) - 1200.0).abs() < 1e-3, "{hs:?}");
    let full = hs[0];
    assert_eq!(resized(&mut l, &shown, 1200.0, 0, 50.0)[0], full);
}

#[test]
fn dragging_a_splitter_up_pushes_every_group_above_in_turn() {
    let shown = ["color", "properties", "navigator", "layers"];
    let mut l = DockLayout { panes: shown.iter().map(|id| Pane::new(&[*id])).collect(), ..Default::default() };
    let before = l.heights_for(1200.0, 28.0);
    // The splitter under Navigator: Navigator shrinks first, then Properties, then Color.
    let hs = resized(&mut l, &shown, 1200.0, 2, -200.0);
    assert_eq!(hs[2], Pane::new(&["navigator"]).min_height());
    assert_eq!(hs[1], before[1] - (200.0 - (before[2] - Pane::new(&["navigator"]).min_height())));
    assert_eq!(hs[0], before[0]);
    assert_eq!(hs[3], before[3] + 200.0, "the filler takes the room");
    let hs = resized(&mut l, &shown, 1200.0, 2, -5000.0);
    assert!(hs[..3].iter().zip(&l.panes).all(|(h, pane)| *h == pane.min_height()), "{hs:?}");
    assert!((total(&hs) - 1200.0).abs() < 1e-3);
    // The group grown when the filler isn't next: the splitter under Color grows Properties.
    let mut l = DockLayout { panes: shown.iter().map(|id| Pane::new(&[*id])).collect(), ..Default::default() };
    let hs = resized(&mut l, &shown, 1200.0, 0, -500.0);
    assert_eq!(hs[0], Pane::new(&["color"]).min_height());
    assert_eq!(hs[1], before[1] + (before[0] - Pane::new(&["color"]).min_height()));
    assert_eq!(hs[2], before[2]);
}

#[test]
fn pushing_skips_collapsed_groups_and_respects_the_filler() {
    let shown = ["color", "properties", "navigator", "history", "layers"];
    let mut l = DockLayout { panes: shown.iter().map(|id| Pane::new(&[*id])).collect(), ..Default::default() };
    l.set_collapsed(1, true);
    l.set_collapsed(4, true);
    let before = l.heights_for(1200.0, 28.0);
    // History is the filler now; Properties keeps its strip and Layers stays collapsed.
    let hs = resized(&mut l, &shown, 1200.0, 0, 5000.0);
    assert_eq!(hs[1], 28.0);
    assert_eq!(hs[4], 28.0);
    assert_eq!(hs[2], Pane::new(&["navigator"]).min_height());
    assert_eq!(hs[3], Pane::new(&["history"]).min_height());
    assert_eq!(hs[0], before[0] + (before[2] - Pane::new(&["navigator"]).min_height()) + (before[3] - Pane::new(&["history"]).min_height()));
    assert!((total(&hs) - 1200.0).abs() < 1e-3);
    assert!(pane(&l, "history").height.is_none(), "the filler's height is never stored");
    // Dragging up from Navigator over the collapsed Properties shrinks Color.
    let hs = resized(&mut l, &shown, 1200.0, 2, -5000.0);
    assert_eq!(hs[0], Pane::new(&["color"]).min_height());
    assert_eq!(hs[1], 28.0);
    assert_eq!(hs[2], Pane::new(&["navigator"]).min_height());
    assert!((total(&hs) - 1200.0).abs() < 1e-3);
    // Nothing to resize against, bad deltas and out-of-range splitters are ignored.
    let snapshot = l.clone();
    for (i, dy) in [(3, 50.0), (4, 50.0), (9, 50.0), (0, f32::NAN), (0, f32::INFINITY), (0, 0.0)] {
        let hs = l.heights_for(1200.0, 28.0);
        l.resize(&hs, i, dy);
        assert_eq!(l, snapshot, "splitter {i} by {dy}");
    }
}

#[test]
fn dragging_swatches_down_pushes_properties_then_layers() {
    let (app, _, _) = app_with_layers();
    let mut h = harness(app, vec2(1200.0, 800.0), ThemeKind::ProMedium);
    let color = rect_of(&h, "color");
    let props = rect_of(&h, "properties");
    let layers = rect_of(&h, "layers");
    let room = (props.height() - Pane::new(&["properties"]).min_height()) + (layers.height() - Pane::new(&["layers"]).min_height());
    let split = Pos2::new(color.center().x, color.bottom() + GAP / 2.0);
    // Past Properties' minimum: Layers gives way too instead of the drag hard-stopping.
    let dy = props.height() - Pane::new(&["properties"]).min_height() + 40.0;
    drag(&mut h, split, split + vec2(0.0, dy));
    assert!((rect_of(&h, "color").height() - (color.height() + dy)).abs() < 2.0, "{color:?} -> {:?}", rect_of(&h, "color"));
    assert_eq!(rect_of(&h, "properties").height(), Pane::new(&["properties"]).min_height());
    assert!((rect_of(&h, "layers").height() - (layers.height() - 40.0)).abs() < 2.0);
    assert_eq!(rect_of(&h, "layers").bottom(), layers.bottom());
    // All the way: everything below at its minimum.
    let split = Pos2::new(color.center().x, rect_of(&h, "color").bottom() + GAP / 2.0);
    drag(&mut h, split, split + vec2(0.0, 2000.0));
    assert!((rect_of(&h, "color").height() - (color.height() + room)).abs() < 2.0);
    assert_eq!(rect_of(&h, "layers").height(), Pane::new(&["layers"]).min_height());
    assert_eq!(rect_of(&h, "layers").bottom(), layers.bottom());
}

#[test]
fn reset_workspace_restores_the_default_layout_and_new_workspaces_keep_theirs() {
    let (app, _, _) = app_with_layers();
    let mut h = harness(app, vec2(1200.0, 800.0), ThemeKind::ProMedium);
    let ctx = h.ctx.clone();
    let default = last_rects(&h.ctx);
    pane_mut(&mut h.state_mut().ui.dock, "properties").height = Some(120.0);
    h.state_mut().ui.dock.set_collapsed(0, true);
    h.state_mut().ui.dock.drop_tab("channels", Drop::NewAt(0));
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.newWorkspace", json!({"name": "Tall Layers"})).unwrap();
    let mine = h.state().ui.dock.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.essentials", json!({})).unwrap();
    assert_eq!(h.state().ui.dock, DockLayout::default());
    pane_mut(&mut h.state_mut().ui.dock, "color").height = Some(300.0);
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.resetWorkspace", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(last_rects(&h.ctx), default);
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.select", json!({"name": "Tall Layers"})).unwrap();
    assert_eq!(h.state().ui.dock, mine);
}

#[test]
fn a_single_tab_click_expands_a_collapsed_pane() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        for tab in ["swatches", "color"] {
            let (mut app, _, _) = app_with_layers();
            app.ui.dock.set_collapsed(0, true);
            // Locking prevents rearrangement, not opening an existing panel.
            app.session.prefs.edit(|p| p.workspace_locked = true);
            let mut h = harness(app, vec2(1200.0, 800.0), theme);
            let collapsed_height = rect_of(&h, "color").height();
            let p = tab_rect(&h, tab).center();
            click(&mut h, p, PointerButton::Primary);
            assert!(!h.state().ui.dock.panes[0].collapsed, "{theme:?}: tab {tab}");
            assert!(h.state().ui.dock.is_front(tab), "{theme:?}: tab {tab}");
            assert!(rect_of(&h, "color").height() > collapsed_height);
        }
    }
}

#[test]
fn double_clicking_a_tab_collapses_and_dragging_a_strip_reorders() {
    let (app, _, _) = app_with_layers();
    let mut h = harness(app, vec2(1200.0, 800.0), ThemeKind::ProMedium);
    let tab = tab_rect(&h, "color").center();
    h.event(egui::Event::PointerMoved(tab));
    h.run_steps(1);
    for _ in 0..2 {
        h.event(egui::Event::PointerButton { pos: tab, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        h.event(egui::Event::PointerButton { pos: tab, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
    assert!(h.state().ui.dock.panes[0].collapsed);
    assert!(rect_of(&h, "color").height() < 40.0);
    h.state_mut().ui.dock.set_collapsed(0, false);
    h.run_steps(3);
    // Drag the Layers tab strip (right of its tabs) above Color.
    let menu = strip_of(&h, "layers").menu;
    let strip = Pos2::new(menu.left() - 30.0, menu.center().y);
    let to = rect_of(&h, "color").left_top() + vec2(120.0, 10.0);
    drag(&mut h, strip, to);
    assert_eq!(fronts(&h.state().ui.dock).first(), Some(&"layers"));
    // A locked workspace keeps the order.
    h.state_mut().session.prefs.edit(|p| p.workspace_locked = true);
    let menu = strip_of(&h, "layers").menu;
    drag(&mut h, Pos2::new(menu.left() - 30.0, menu.center().y), Pos2::new(menu.left() - 30.0, 790.0));
    assert_eq!(fronts(&h.state().ui.dock).first(), Some(&"layers"));
}

/// Tabs drag between panes in both themes: onto a strip to join it, below the last pane
/// for one of their own. A locked workspace keeps them.
#[test]
fn dragging_a_tab_moves_it_between_panes() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        let (app, _, _) = app_with_layers();
        let mut h = harness(app, vec2(1200.0, 900.0), theme);
        let to = strip_of(&h, "properties").menu.left_center() - vec2(40.0, 0.0);
        let from = tab_rect(&h, "channels").center();
        drag(&mut h, from, to);
        let dock = &h.state().ui.dock;
        assert_eq!(tabs(dock, "properties"), ["properties", "adjustments", "channels"], "{theme:?}");
        assert!(dock.is_front("channels"), "{theme:?}");
        // To the very bottom: a pane of its own below Layers.
        let layers = rect_of(&h, "layers");
        let from = tab_rect(&h, "gradients").center();
        drag(&mut h, from, Pos2::new(layers.center().x, layers.bottom() - 4.0));
        assert_eq!(h.state().ui.dock.panes.last().unwrap().tabs, ["gradients"], "{theme:?}: {:?}", h.state().ui.dock);
        h.state_mut().session.prefs.edit(|p| p.workspace_locked = true);
        let before = h.state().ui.dock.clone();
        let (from, to) = (tab_rect(&h, "paths").center(), rect_of(&h, "color").center());
        drag(&mut h, from, to);
        assert_eq!(h.state().ui.dock, before, "{theme:?}: locked");
    }
}

#[test]
fn a_dropped_tab_lands_at_the_slot_asked() {
    let mut l = DockLayout::default();
    // Into another pane, between Color and Swatches.
    l.drop_tab("channels", Drop::Insert(0, 1));
    assert_eq!(tabs(&l, "color"), ["color", "channels", "swatches", "gradients", "patterns"]);
    assert!(l.is_front("channels"));
    // Reordering within a pane: the slot is counted before the tab leaves.
    l.drop_tab("patterns", Drop::Insert(0, 0));
    assert_eq!(tabs(&l, "color"), ["patterns", "color", "channels", "swatches", "gradients"]);
    l.drop_tab("patterns", Drop::Insert(0, 3));
    assert_eq!(tabs(&l, "color"), ["color", "channels", "patterns", "swatches", "gradients"]);
    // Next to itself nothing moves, but it comes forward; out of range changes nothing.
    l.drop_tab("swatches", Drop::Insert(0, 4));
    assert_eq!(tabs(&l, "color"), ["color", "channels", "patterns", "swatches", "gradients"]);
    assert!(l.is_front("swatches"));
    let before = l.clone();
    l.drop_tab("swatches", Drop::Insert(99, 0));
    l.drop_tab("bogus", Drop::Insert(0, 0));
    assert_eq!(l, before);
    l.drop_tab("layers", Drop::Insert(0, 999));
    assert_eq!(tabs(&l, "color").last().map(String::as_str), Some("layers"));
}

/// Dropping a tab between two tabs of another strip puts it there, not at the end.
#[test]
fn dragging_a_tab_between_two_tabs_inserts_it_there() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        let (app, _, _) = app_with_layers();
        let mut h = harness(app, vec2(1200.0, 900.0), theme);
        let (color, swatches) = (tab_rect(&h, "color"), tab_rect(&h, "swatches"));
        let first = if color.left() < swatches.left() { "color" } else { "swatches" };
        let between = Pos2::new((color.right().min(swatches.right()) + color.left().max(swatches.left())) / 2.0, color.center().y);
        let from = tab_rect(&h, "channels").center();
        drag(&mut h, from, between);
        let got = tabs(&h.state().ui.dock, "color");
        assert_eq!(got.get(1).map(String::as_str), Some("channels"), "{theme:?}: {got:?}");
        assert_eq!(got.first().map(String::as_str), Some(first), "{theme:?}: {got:?}");
        // Within the strip: the first tab dragged to just before the last one shown.
        let shown = strip_of(&h, "color").tabs;
        let (last, r) = shown.last().cloned().unwrap();
        let from = tab_rect(&h, first).center();
        drag(&mut h, from, Pos2::new(r.left() + 2.0, r.center().y));
        let got = tabs(&h.state().ui.dock, "color");
        let i = got.iter().position(|t| *t == last).unwrap();
        assert_eq!(got.get(i - 1).map(String::as_str), Some(first), "{theme:?}: {got:?}");
    }
}

#[test]
fn tiny_windows_do_not_panic() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        for (w, ht) in [(40.0, 30.0), (300.0, 80.0), (600.0, 200.0), (2000.0, 120.0)] {
            for rail in [false, true] {
                let (mut app, _, _) = app_with_layers();
                app.ui.dock.show("history");
                app.ui.dock.show("navigator");
                app.ui.dock.rail = rail;
                app.ui.dock.flyout = rail.then(|| "layers".to_owned());
                let mut h = harness(app, vec2(w, ht), theme);
                for (_, r) in last_rects(&h.ctx) {
                    assert!(r.height() >= 0.0 && r.is_finite());
                }
                // Drag a splitter way off-screen.
                if let Some((_, r)) = last_rects(&h.ctx).first().cloned() {
                    let p = Pos2::new(r.center().x, r.bottom() + GAP / 2.0);
                    drag(&mut h, p, p + vec2(0.0, 5000.0));
                    drag(&mut h, p, p - vec2(0.0, 5000.0));
                }
            }
        }
    }
}

#[test]
fn the_rail_and_window_menu_never_lose_a_panel() {
    let (mut app, _, _) = app_with_layers();
    let ctx = egui::Context::default();
    let collapsed = |app: &PhotocraftApp, id: &str| pane(&app.ui.dock, id).collapsed;
    // The rail collapses and expands a docked pane; it never hides it (#129).
    rail_click(&mut app, "layers");
    assert!(app.ui.dock.visible("layers") && collapsed(&app, "layers"));
    rail_click(&mut app, "layers");
    assert!(app.ui.dock.visible("layers") && !collapsed(&app, "layers"));
    // …and shows a hidden one.
    rail_click(&mut app, "history");
    assert!(app.ui.dock.is_front("history") && !collapsed(&app, "history"));
    // A background tab comes forward rather than collapsing its pane.
    rail_click(&mut app, "channels");
    assert!(app.ui.dock.is_front("channels") && !collapsed(&app, "channels"));
    // Window › Layers on a collapsed pane expands it instead of hiding it.
    let i = app.ui.dock.pane_of("layers").unwrap();
    app.ui.dock.set_collapsed(i, true);
    crate::menus::invoke(&mut app, &ctx, "window.panel.layers", json!({})).unwrap();
    assert!(app.ui.dock.is_front("layers") && !collapsed(&app, "layers"));
    app.ui.dock.set_collapsed(0, true);
    crate::menus::invoke(&mut app, &ctx, "window.toggle.color", json!({})).unwrap();
    assert!(app.ui.dock.visible("color") && !collapsed(&app, "color"));
    app.ui.dock.set_collapsed(0, true);
    crate::menus::invoke(&mut app, &ctx, "window.panel.gradients", json!({})).unwrap();
    assert!(app.ui.dock.is_front("gradients") && !collapsed(&app, "gradients"));
    // Hidden panels come back from the Window menu, expanded, in front.
    app.ui.dock.hide("history");
    crate::menus::invoke(&mut app, &ctx, "window.panel.actions", json!({})).unwrap();
    assert!(app.ui.dock.is_front("actions") && !collapsed(&app, "actions"));
    // The dock hidden (⇧Tab) comes back with the panel asked for.
    app.ui.panels.dock = false;
    crate::menus::invoke(&mut app, &ctx, "window.panel.navigator", json!({})).unwrap();
    assert!(app.ui.panels.dock);
    // Reset Workspace brings back everything the preset shows.
    app.ui.dock.hide("layers");
    app.ui.dock.hide("properties");
    app.ui.dock.set_collapsed(0, true);
    crate::menus::invoke(&mut app, &ctx, "window.workspace.resetWorkspace", json!({})).unwrap();
    assert_eq!(app.ui.dock, DockLayout::default());
}

/// #129: clicking around the whole UI (layers, canvas, tools, options bar) never switches,
/// hides or moves a dock panel.
#[test]
fn clicking_around_the_ui_keeps_the_panels_put() {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let (app, _, _) = app_with_layers();
        app
    });
    h.run_steps(8);
    let dock = h.state().ui.dock.clone();
    let rects = last_rects(&h.ctx);
    assert!(rects.iter().any(|(g, _)| g == "layers"), "{rects:?}");
    let layers = rect_of_full(&h, "layers");
    let canvas = h.state().last_canvas_rect;
    let mut points: Vec<Pos2> = Vec::new();
    // Layer rows (eyes, thumbnails, names) and the Layers footer.
    for dy in [118.0, 140.0, 150.0, 170.0, 182.0, 200.0] {
        for x in [layers.left() + 22.0, layers.left() + 50.0, layers.center().x] {
            points.push(Pos2::new(x, layers.top() + dy));
        }
    }
    // The canvas, the toolbar column and the options bar.
    points.extend([canvas.center(), canvas.left_top() + vec2(40.0, 40.0), canvas.right_bottom() - vec2(30.0, 30.0)]);
    for y in (110..460).step_by(33) {
        points.push(Pos2::new(20.0, y as f32));
    }
    for x in (40..1100).step_by(70) {
        points.push(Pos2::new(x as f32, 50.0));
    }
    for p in points {
        h.hover_at(p);
        h.run_steps(1);
        h.drag_at(p);
        h.run_steps(1);
        h.drop_at(p);
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        // Popups and dialogs opened by a click are fine; panels changing aren't.
        let app = h.state();
        assert_eq!(app.ui.dock, dock, "click at {p:?} changed the dock");
        if app.ui.dialogs.is_empty() && app.ui.shell.dialog.is_none() {
            assert_eq!(last_rects(&h.ctx), rects, "click at {p:?} moved the dock panes");
        }
        h.state_mut().ui.dialogs.clear();
    }
}

fn rect_of_full(h: &Harness<'static, PhotocraftApp>, id: &str) -> Rect {
    module_rect(h.state(), &h.ctx, id).unwrap()
}

fn right_click_tab(h: &mut Harness<'static, PhotocraftApp>, id: &str) {
    let p = tab_rect(h, id).center();
    click(h, p, PointerButton::Secondary);
}

/// #1753: both themes offer Close for the clicked tab and Close Tab Group. Closing the last tab
/// takes its pane away; the Window menu brings a module back with its group.
#[test]
fn panel_tab_context_menu_closes_one_tab_or_its_pane() {
    use egui_kittest::kittest::Queryable;

    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        let (app, _, _) = app_with_layers();
        let mut h = harness(app, vec2(1300.0, 850.0), theme);
        // Close the first tab: the pane stays, showing the next one.
        right_click_tab(&mut h, "properties");
        h.get_by_label("Close").click();
        h.run_steps(3);
        assert!(!h.state().ui.dock.visible("properties"), "{theme:?}");
        assert!(h.state().ui.dock.is_front("adjustments"), "{theme:?}: closing one tab keeps the pane");
        assert_eq!(strip_of(&h, "adjustments").tabs.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["adjustments"]);
        // Close the last one: its pane goes, the others stay.
        right_click_tab(&mut h, "adjustments");
        h.get_by_label("Close").click();
        h.run_steps(3);
        assert_eq!(fronts(&h.state().ui.dock), ["color", "layers"], "{theme:?}");
        // The Window menu brings the module back where its group was.
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, "window.panel.properties", json!({})).unwrap();
        h.run_steps(3);
        assert_eq!(fronts(&h.state().ui.dock), ["color", "properties", "layers"], "{theme:?}");
        // Close Tab Group takes the whole pane.
        right_click_tab(&mut h, "properties");
        h.get_by_label("Close Tab Group").click();
        h.run_steps(3);
        assert!(!h.state().ui.dock.visible("properties") && !h.state().ui.dock.visible("adjustments"), "{theme:?}");
    }
}

/// Studio's card header: ⌄ collapses the pane, ✕ closes it.
#[test]
fn studio_pane_buttons_collapse_and_close() {
    let (app, _, _) = app_with_layers();
    let mut h = harness(app, vec2(1300.0, 850.0), ThemeKind::Studio);
    let s = strip_of(&h, "properties");
    let (collapse, close) = (s.collapse.expect("collapse button"), s.close.expect("close button"));
    for (_, r) in &s.tabs {
        assert!(r.intersect(collapse).width() <= 0.5 && r.intersect(close).width() <= 0.5 && r.intersect(s.menu).width() <= 0.5);
    }
    click(&mut h, collapse.center(), PointerButton::Primary);
    assert!(pane(&h.state().ui.dock, "properties").collapsed);
    let collapse = strip_of(&h, "properties").collapse.unwrap();
    click(&mut h, collapse.center(), PointerButton::Primary);
    assert!(!pane(&h.state().ui.dock, "properties").collapsed);
    let close = strip_of(&h, "properties").close.unwrap();
    click(&mut h, close.center(), PointerButton::Primary);
    assert!(!h.state().ui.dock.visible("properties"));
    // Pro strips stay Photoshop's: no extra buttons.
    let (app, _, _) = app_with_layers();
    let h = harness(app, vec2(1300.0, 850.0), ThemeKind::ProMedium);
    assert!(last_strips(&h.ctx).iter().all(|s| s.collapse.is_none() && s.close.is_none()));
}

/// Studio: » collapses the inspector to icons, an icon opens its module as a flyout (a click
/// outside closes it), « expands again; + lists every module and adds one.
#[test]
fn studio_rail_flyouts_and_the_panel_picker() {
    use egui_kittest::kittest::Queryable;

    let (app, _, _) = app_with_layers();
    let mut h = harness(app, vec2(1300.0, 850.0), ThemeKind::Studio);
    h.get_by_label("Collapse to Icons").click();
    h.run_steps(3);
    assert!(h.state().ui.dock.rail);
    assert!(h.query_by_label("Collapse to Icons").is_none(), "the inspector is folded away");
    assert!(h.query_by_label("History").is_none(), "History isn't docked, so it has no rail icon");
    h.get_by_label("Layers").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dock.flyout.as_deref(), Some("layers"));
    click(&mut h, Pos2::new(300.0, 400.0), PointerButton::Primary);
    assert_eq!(h.state().ui.dock.flyout, None, "a click outside closes the flyout");
    h.get_by_label("Expand Panels").click();
    h.run_steps(3);
    assert!(!h.state().ui.dock.rail);
    // The picker (the header's +; each strip has its own too): search, then pick.
    h.get_all_by_label("Add Panel").next().unwrap().click();
    h.run_steps(3);
    h.event(egui::Event::Text("hist".into()));
    h.run_steps(3);
    assert!(h.query_by_label("Swatches").is_none(), "the search filters the list");
    h.get_by_label("History").click();
    h.run_steps(3);
    assert!(h.state().ui.dock.is_front("history"));
    // The rail persists with the layout.
    h.state_mut().ui.dock.rail = true;
    let v = serde_json::to_value(&h.state().ui.dock).unwrap();
    assert!(serde_json::from_value::<DockLayout>(v).unwrap().rail);
}

/// Full app at `size` (1× scale) on a document with `n` layers named "Row 00", "Row 01", …
fn app_harness_rows(size: egui::Vec2, theme: ThemeKind, n: usize) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(size).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.theme = theme;
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        for i in 0..n {
            app.run("layer.new.layer", json!({"name": format!("Row {i:02}")})).unwrap();
        }
        app.sync_views();
        app
    });
    h.run_steps(8);
    h
}

/// Layer rows fully inside the Layers rows viewport.
fn rows_in_view(h: &Harness<'static, PhotocraftApp>, n: usize) -> usize {
    use egui_kittest::kittest::Queryable;
    let layers = rect_of(h, "layers");
    (0..n)
        .filter(|i| {
            h.query_all_by_label(&format!("Row {i:02}"))
                .map(|q| q.rect())
                .any(|r| r.left() >= layers.left() - 1.0 && r.top() >= layers.top() + 26.0 && r.bottom() <= layers.bottom() - 36.0)
        })
        .count()
}

/// #147: the default workspace gives Layers the column's spare height: at least ten rows on a
/// 900 pt window, and a usable list on a 720 pt one.
#[test]
fn default_layout_shows_ten_layer_rows_at_900pt() {
    for (size, want) in [(vec2(1440.0, 900.0), 10), (vec2(1280.0, 720.0), 4)] {
        let h = app_harness_rows(size, ThemeKind::ProMedium, 40);
        let rows = rows_in_view(&h, 40);
        let drawn = last_rects(&h.ctx);
        assert!(rows >= want, "{size:?}: {rows} rows visible, want ≥ {want}: {drawn:?}");
        // Color and Properties stay open, just not at the expense of Layers.
        for id in ["color", "properties"] {
            assert!(rect_of(&h, id).height() >= pane(&h.state().ui.dock, id).compact_height() - 0.5, "{id} at {size:?}: {drawn:?}");
        }
    }
}

#[test]
fn defaults_give_way_to_layers_but_user_sizes_stay() {
    let l = DockLayout::default();
    // A tall column: everyone at their defaults, Layers takes the rest.
    let hs = l.heights_for(1200.0, 28.0);
    assert_eq!(hs[0], l.panes[0].default_height());
    assert_eq!(hs[1], l.panes[1].default_height());
    // A short one: default-sized panes shrink toward their compact heights so Layers keeps
    // its preferred height, the one nearest Layers first.
    let hs = l.heights_for(820.0, 28.0);
    assert!(hs[2] >= l.panes[2].preferred_fill() - 0.5, "{hs:?}");
    assert!(hs[1] < l.panes[1].default_height() && hs[1] >= l.panes[1].compact_height(), "{hs:?}");
    // Heights the user dragged to (saved in prefs) are kept as they are.
    let mut mine = DockLayout::default();
    mine.panes[1].height = Some(340.0);
    mine.panes[0].height = Some(200.0);
    let hs = mine.heights_for(800.0, 28.0);
    assert_eq!((hs[0], hs[1]), (200.0, 340.0), "{hs:?}");
}

/// #150: Window › Character opens a Character | Paragraph pane next to Properties (which
/// stays), and toggles closed again.
#[test]
fn window_character_opens_its_own_pane_and_keeps_properties() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        let (app, _, _) = app_with_layers();
        let mut h = harness(app, vec2(1440.0, 900.0), theme);
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, "window.panel.character", json!({})).unwrap();
        h.run_steps(3);
        assert_eq!(fronts(&h.state().ui.dock), ["color", "properties", "character", "layers"], "{theme:?}");
        let ch = rect_of(&h, "character");
        assert!(ch.height() > 60.0, "{theme:?}: Character expanded: {ch:?}");
        let props = rect_of(&h, "properties");
        assert!(props.height() > 60.0 && props.bottom() <= ch.top(), "{theme:?}: Properties visible above Character");
        assert!(rect_of(&h, "layers").top() > ch.bottom(), "{theme:?}: Layers stays the filler at the bottom");
        assert_eq!(crate::view_cmds::checked(h.state(), "window.panel.character"), Some(true));
        // Paragraph is the pane's second tab; choosing it again on its tab closes the pane.
        crate::menus::invoke(h.state_mut(), &ctx, "window.panel.paragraph", json!({})).unwrap();
        assert!(h.state().ui.dock.is_front("paragraph"));
        crate::menus::invoke(h.state_mut(), &ctx, "window.panel.paragraph", json!({})).unwrap();
        h.run_steps(3);
        assert!(!h.state().ui.dock.visible("character") && h.state().ui.dock.visible("properties"), "{theme:?}");
        assert!(!last_rects(&h.ctx).iter().any(|(g, _)| g == "character" || g == "paragraph"));
        // Type › Panels › Character Panel always shows it; Reset Workspace closes it.
        crate::menus::invoke(h.state_mut(), &ctx, "type.panels.character", json!({})).unwrap();
        crate::menus::invoke(h.state_mut(), &ctx, "type.panels.character", json!({})).unwrap();
        assert!(h.state().ui.dock.visible("character"));
        crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.resetWorkspace", json!({})).unwrap();
        assert!(!h.state().ui.dock.visible("character") && h.state().ui.dock.visible("properties"));
    }
}

type StripProbe = Option<(Vec<(usize, Rect)>, Rect, Option<Rect>, Rect)>;

/// #151: at narrow widths and 2× scale, in every theme, tabs elide or overflow into a chevron
/// and never run under the panel menu button.
#[test]
fn tab_strips_never_overlap_the_menu_button() {
    let overlap = |a: Rect, b: Rect| {
        let i = a.intersect(b);
        i.width() > 0.5 && i.height() > 0.5
    };
    for theme in ThemeKind::ALL {
        for scale in [1.0, 2.0] {
            for width in [180.0, 250.0, 290.0, 420.0] {
                for g in GROUPS {
                    let tabs: Vec<&'static str> = group_tabs(g, is_pro(theme)).iter().filter_map(|id| modules::get(id)).map(|m| m.title).collect();
                    for sel in 0..tabs.len() {
                        let want = tabs.len();
                        let tabs = tabs.clone();
                        let mut h = Harness::builder().with_size(vec2(width, 200.0)).with_pixels_per_point(scale).build_ui_state(
                            move |ui, out: &mut StripProbe| {
                                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                                    return;
                                }
                                let mut s = sel;
                                let r = crate::widgets::card_ex(ui, "t", &tabs, &mut s, false, |ui, _| {
                                    ui.label("body");
                                });
                                let mut menu = r.menu.rect;
                                for b in [&r.collapse, &r.close].into_iter().flatten() {
                                    menu = menu.union(b.rect);
                                }
                                *out = Some((r.tabs, menu, r.chevron, ui.max_rect()));
                            },
                            None,
                        );
                        PhotocraftApp::setup_context(&h.ctx, theme);
                        h.run_steps(3);
                        let (shown, menu, chevron, card) = h.state().clone().unwrap();
                        let what = format!("{theme:?} {scale}x {width}pt {g} selected {sel}");
                        assert!(shown.iter().any(|(i, _)| *i == sel), "{what}: the selected tab is on the strip: {shown:?}");
                        for (i, r) in &shown {
                            assert!(!overlap(*r, menu), "{what}: tab {i} {r:?} runs under the buttons {menu:?}");
                            assert!(r.left() >= card.left() - 0.5 && r.right() <= card.right() + 0.5, "{what}: tab {i} outside the card");
                        }
                        for w in shown.windows(2) {
                            assert!(!overlap(w[0].1, w[1].1), "{what}: tabs overlap");
                        }
                        if let Some(c) = chevron {
                            assert!(!overlap(c, menu) && shown.iter().all(|(_, r)| !overlap(*r, c)), "{what}: chevron overlaps");
                        } else {
                            assert_eq!(shown.len(), want, "{what}: a tab went missing without a chevron");
                        }
                    }
                }
            }
        }
    }
}

/// #151 in the real dock: every strip at 2× scale fits beside its menu button, and a tab in the
/// chevron menu can still be chosen.
#[test]
fn dock_strips_fit_and_the_chevron_menu_switches_tabs() {
    for theme in ThemeKind::ALL {
        let (mut app, _, _) = app_with_layers();
        app.ui.dock = DockLayout::preset(&GROUPS, is_pro(theme));
        let mut h = Harness::builder().with_size(vec2(900.0, 1000.0)).with_pixels_per_point(2.0).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                crate::panels::right_dock(app, ui);
                egui::CentralPanel::default().show(ui, |_| {});
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, theme);
        h.state_mut().ui.theme = theme;
        h.run_steps(4);
        let strips = last_strips(&h.ctx);
        assert!(strips.len() >= 5, "{theme:?}: {strips:?}");
        for s in &strips {
            for (id, r) in &s.tabs {
                assert!(r.intersect(s.menu).width() <= 0.5, "{theme:?} pane {}: tab {id} under the menu", s.pane);
            }
        }
    }
    // A strip too narrow for its tabs: pick a hidden tab from the chevron menu.
    let mut h = Harness::builder().with_size(vec2(150.0, 200.0)).build_ui_state(
        |ui, sel: &mut usize| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let _ = crate::widgets::card_ex(ui, "color", &["Color", "Swatches", "Gradients", "Patterns"], sel, false, |ui, _| {
                ui.label("body");
            });
        },
        0usize,
    );
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.run_steps(3);
    use egui_kittest::kittest::Queryable;
    h.get_by_label("More panels").click();
    h.run_steps(3);
    h.get_by_label("Patterns").click();
    h.run_steps(3);
    assert_eq!(*h.state(), 3, "Patterns chosen from the chevron menu");
}

#[test]
fn tab_hides_all_panels_and_shift_tab_only_the_dock() {
    // #1313: Photoshop's Tab hides the Tools panel, the options bar and the panel dock (Tab again
    // brings back what it hid); ⇧Tab hides and shows the dock alone.
    let (mut app, _, _) = app_with_layers();
    let ctx = egui::Context::default();
    let run = |app: &mut PhotocraftApp, id: &str| crate::menus::invoke(app, &ctx, id, json!({})).unwrap();
    app.ui.panels.options_bar = false;
    run(&mut app, "window.togglePanels");
    let p = &app.ui.panels;
    assert!(!p.toolbar && !p.options_bar && !p.dock);
    run(&mut app, "window.togglePanels");
    let p = &app.ui.panels;
    assert!(p.toolbar && !p.options_bar && p.dock, "Tab brings back only what it hid");
    run(&mut app, "window.toggle.dock");
    assert!(!app.ui.panels.dock && app.ui.panels.toolbar, "⇧Tab hides only the dock");
    run(&mut app, "window.toggle.dock");
    assert!(app.ui.panels.dock);
    // Everything hidden by hand: Tab shows it all.
    (app.ui.panels.toolbar, app.ui.panels.options_bar, app.ui.panels.dock) = (false, false, false);
    run(&mut app, "window.togglePanels");
    assert!(app.ui.panels.toolbar && app.ui.panels.options_bar && app.ui.panels.dock);
    // The keys are Photoshop's.
    let bound = |key: &str| crate::shortcut_dispatch::bindings(&app).into_iter().find(|(_, sc)| Some(*sc) == crate::shortcuts::parse(key)).map(|(id, _)| id);
    assert_eq!(bound("Tab").as_deref(), Some("window.togglePanels"));
    assert_eq!(bound("Shift+Tab").as_deref(), Some("window.toggle.dock"));
    // An older saved UI state without the field shows the dock.
    let mut v = serde_json::to_value(crate::state::Panels::default()).unwrap();
    v.as_object_mut().unwrap().remove("dock");
    assert!(serde_json::from_value::<crate::state::Panels>(v).unwrap().dock);
}

/// Every theme but Pro paints the hovered tab × in the danger colour.
#[test]
fn only_pro_keeps_a_grey_tab_close() {
    for kind in ThemeKind::ALL {
        let t = crate::theme::Tokens::for_kind(kind);
        assert!(t.tab_close && t.tab_add, "{kind:?}");
        assert_eq!(t.tab_close_danger, !is_pro(kind), "{kind:?}");
    }
}

/// Like a browser: hovering a tab shows its ×, which sits beside the label, never on it; a
/// middle click closes the tab too; the round + after the last tab adds a module to that pane.
#[test]
fn tabs_close_on_hover_or_middle_click_and_the_strip_plus_adds_a_tab() {
    use egui_kittest::kittest::Queryable;

    for theme in [ThemeKind::ProMedium, ThemeKind::Studio, ThemeKind::Adwaita] {
        let (app, _, _) = app_with_layers();
        let mut h = harness(app, vec2(1300.0, 850.0), theme);
        assert!(h.query_by_label("Close Tab").is_none(), "{theme:?}: no × until a tab is hovered");
        let tab = tab_rect(&h, "properties");
        let centred = h.query_all_by_label("Properties").map(|n| n.rect()).find(|r| tab.contains(r.center()) && r.width() < tab.width() - 1.0);
        if let Some(c) = centred {
            assert!((c.center().x - tab.center().x).abs() < 1.0, "{theme:?}: unhovered labels are centred");
        }
        h.event(egui::Event::PointerMoved(tab.center()));
        h.run_steps(12);
        let x = h.get_by_label("Close Tab").rect();
        assert!(tab.contains_rect(x), "{theme:?}: the × belongs to the tab");
        let label = h.query_all_by_label("Properties").map(|n| n.rect()).find(|r| tab.contains(r.center()) && r.width() < tab.width() - 1.0);
        if let Some(label) = label {
            assert!(label.right() <= x.left() + 0.5, "{theme:?}: the × must not cover the label ({label:?} vs {x:?})");
        }
        click(&mut h, x.center(), PointerButton::Primary);
        assert!(!h.state().ui.dock.visible("properties"), "{theme:?}: the × closes the tab");
        // Middle click.
        let tab = tab_rect(&h, "layers");
        click(&mut h, tab.center(), PointerButton::Middle);
        assert!(!h.state().ui.dock.visible("layers"), "{theme:?}: a middle click closes the tab");
        // The strip's +: it adds into its own pane.
        let strip = strip_of(&h, "color");
        let band = Rect::from_x_y_ranges(strip.menu.left() - 2000.0..=strip.menu.right() + 50.0, strip.menu.y_range());
        let plus =
            h.query_all_by_label("Add Tab").map(|n| n.rect()).find(|r| band.contains(r.center())).unwrap_or_else(|| panic!("{theme:?}: no + on the strip"));
        let last = strip.tabs.iter().map(|(_, r)| r.right()).fold(f32::MIN, f32::max);
        assert!(plus.left() >= last - 0.5, "{theme:?}: the + follows the last tab");
        click(&mut h, plus.center(), PointerButton::Primary);
        h.event(egui::Event::Text("hist".into()));
        h.run_steps(3);
        // Pro's icon column has a History button too; the picker's row is the wide one, the icon square.
        let row = h.query_all_by_label("History").find(|n| n.rect().width() > n.rect().height() + 4.0).expect("History in the picker");
        row.click();
        h.run_steps(3);
        assert!(tabs(&h.state().ui.dock, "color").iter().any(|t| t == "history"), "{theme:?}: {:?}", h.state().ui.dock.panes);
    }
}

/// Studio's icon rail: middle click closes a module; the right-click menu offers Close and Close
/// Tab Group, like every other panel menu, and Expand Panels.
#[test]
fn studio_rail_icons_have_a_context_menu() {
    use egui_kittest::kittest::Queryable;

    let (app, _, _) = app_with_layers();
    let mut h = harness(app, vec2(1300.0, 850.0), ThemeKind::Studio);
    h.state_mut().ui.dock.rail = true;
    h.run_steps(3);
    let icon = |h: &Harness<'static, PhotocraftApp>, l: &str| h.get_by_label(l).rect().center();
    let p = icon(&h, "Layers");
    click(&mut h, p, PointerButton::Secondary);
    h.get_by_label("Close").click();
    h.run_steps(3);
    assert!(!h.state().ui.dock.visible("layers"));
    let p = icon(&h, "Properties");
    click(&mut h, p, PointerButton::Secondary);
    h.get_by_label("Close Tab Group").click();
    h.run_steps(3);
    assert!(!h.state().ui.dock.visible("properties") && !h.state().ui.dock.visible("adjustments"));
    let front = h.state().ui.dock.panes[0].front().to_owned();
    let label = crate::modules::title(&front).to_owned();
    let p = icon(&h, &label);
    click(&mut h, p, PointerButton::Middle);
    assert!(!h.state().ui.dock.visible(&front), "middle click closes {front}");
    h.state_mut().ui.dock.show("history");
    h.run_steps(3);
    let p = icon(&h, "History");
    click(&mut h, p, PointerButton::Secondary);
    // The rail's own « also says Expand Panels; the menu's entry comes last.
    h.get_all_by_label("Expand Panels").last().unwrap().click();
    h.run_steps(3);
    assert!(!h.state().ui.dock.rail);
}

/// Pro: a + at the foot of Photoshop's icon column adds a panel without changing the dock's look.
#[test]
fn the_pro_rail_plus_adds_a_panel() {
    use egui_kittest::kittest::Queryable;

    let (app, _, _) = app_with_layers();
    let mut h = harness(app, vec2(1300.0, 850.0), ThemeKind::Pro);
    assert!(h.state().ui.panels.rail);
    // Photoshop's icon column; the + is its last button.
    let plus = h.query_all_by_label("Add Panel").map(|n| n.rect()).max_by(|a, b| a.bottom().total_cmp(&b.bottom())).expect("a + on the rail");
    assert!(plus.bottom() > 700.0, "the + sits at the foot of the rail: {plus:?}");
    click(&mut h, plus.center(), PointerButton::Primary);
    h.event(egui::Event::Text("hist".into()));
    h.run_steps(3);
    h.query_all_by_label("History").find(|n| n.rect().width() > n.rect().height() + 4.0).expect("History in the picker").click();
    h.run_steps(3);
    assert!(h.state().ui.dock.is_front("history"));
}

/// Pro: Photoshop's icon column lists the docked panels, not a fixed set, so one added with
/// the + shows there and one closed goes.
#[test]
fn the_pro_rail_lists_the_docked_panels() {
    use egui_kittest::kittest::Queryable;

    let (app, _, _) = app_with_layers();
    let mut h = harness(app, vec2(1300.0, 850.0), ThemeKind::Pro);
    let rail =
        |h: &Harness<'static, PhotocraftApp>| egui::containers::panel::PanelState::load(&h.ctx, egui::Id::new("rail")).expect("the Pro rail").outer_rect.left();
    let on_rail = |h: &Harness<'static, PhotocraftApp>, label: &str| h.query_all_by_label(label).any(|n| n.rect().left() >= rail(h));
    assert!(on_rail(&h, "Layers") && on_rail(&h, "Channels") && on_rail(&h, "Swatches"));
    assert!(!on_rail(&h, "Histogram"));
    h.state_mut().ui.dock.add_pane("histogram");
    h.state_mut().ui.dock.close("channels");
    h.run_steps(3);
    assert!(on_rail(&h, "Histogram") && !on_rail(&h, "Channels"));
}

/// The pane terminology keeps layouts from the original panel PR loadable.
#[test]
fn section_layouts_still_load_as_panes() {
    let old = json!({"sections": [{"tabs": ["history", "actions"], "active": "actions", "height": 240.0, "collapsed": true}], "rail": true});
    let layout: DockLayout = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(layout.panes.len(), 1);
    assert_eq!(layout.panes[0].front(), "actions");
    assert_eq!(layout.panes[0].height, Some(240.0));
    assert!(layout.panes[0].collapsed && layout.rail);
    let saved = serde_json::to_value(&layout).unwrap();
    assert!(saved.get("panes").is_some() && saved.get("sections").is_none());
    assert_eq!(serde_json::from_value::<DockLayout>(saved).unwrap(), layout);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    apply(&mut app, &json!({"dock": old}));
    assert_eq!(app.ui.dock, layout);
    assert_eq!(layout_from_control(&app, None, None, Some(&old)).unwrap(), Some(layout));
}

/// Shrinking a pane must keep its footer and bottom boundary inside its allocation.
#[test]
fn short_layer_panes_keep_the_footer_inside() {
    use egui_kittest::kittest::Queryable;
    for theme in ThemeKind::ALL {
        let (mut app, _, _) = app_with_layers();
        app.ui.dock = DockLayout { panes: vec![Pane::new(&["layers"]), Pane::new(&["history"])], ..Default::default() };
        app.ui.dock.panes[0].height = Some(140.0);
        let h = harness(app, vec2(1000.0, 700.0), theme);
        let pane = rect_of(&h, "layers");
        let footer = h.get_by_label("Create a new layer").rect();
        assert!(footer.top() >= pane.top() && footer.bottom() <= pane.bottom() + 0.5, "{theme:?}: footer {footer:?} outside pane {pane:?}");
        assert!(footer.height() >= 25.0, "{theme:?}: footer clipped: {footer:?}");
    }
}
