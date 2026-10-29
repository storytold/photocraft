//! Opt-in, synthetic-document captures of the current app, never a desktop recording.
use super::{GRIP, HEADER, INSET, PhotocraftApp, Strip, key, strips};
use crate::panel_docking::{PANELS, area_id, last_bodies};
use craft_ui::docking::{Divider, DockArea, Hit};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

type H = Harness<'static, PhotocraftApp>;

struct Clip {
    directory: PathBuf,
    frame: usize,
}
impl Drop for Clip {
    fn drop(&mut self) {
        // Repeated captures may leave older numbered files: the encoder reads only this run.
        if let Err(error) = std::fs::write(self.directory.join("frame-count.txt"), self.frame.to_string()) {
            eprintln!("cannot write capture frame count: {error}");
        }
    }
}
impl Clip {
    fn new(root: &Path, name: &str) -> Self {
        let directory = root.join(name);
        std::fs::create_dir_all(&directory).unwrap();
        Self { directory, frame: 0 }
    }
    fn capture(&mut self, h: &mut H) {
        h.render().unwrap().save(self.directory.join(format!("{:04}.png", self.frame))).unwrap();
        self.frame += 1;
    }
    fn settle(&mut self, h: &mut H) {
        for _ in 0..6 {
            h.run_steps(1);
            self.capture(h);
        }
    }
    fn click(&mut self, h: &mut H, at: Pos2) {
        h.event(Event::PointerMoved(at));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run_steps(1);
            self.capture(h);
        }
        self.settle(h);
    }
    fn drag(&mut self, h: &mut H, from: Pos2, to: Pos2, modifiers: Modifiers, cancel: bool) {
        h.event(Event::ModifiersChanged(modifiers));
        h.event(Event::PointerMoved(from));
        h.run_steps(1);
        h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers });
        h.run_steps(1);
        self.capture(h);
        for step in 1..=8 {
            h.event(Event::PointerMoved(from + (to - from) * (step as f32 / 8.0)));
            h.run_steps(1);
            self.capture(h);
        }
        if cancel {
            h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers });
            h.run_steps(1);
            self.capture(h);
            h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: false, repeat: false, modifiers });
        }
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers });
        h.run_steps(1);
        self.capture(h);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        self.settle(h);
    }
}

fn run(h: &mut H, id: &str, params: Value) {
    h.state_mut().run(id, params).unwrap();
    h.run_steps(6);
}
fn invoke(h: &mut H, id: &str, params: Value) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, id, params).unwrap();
    h.run_steps(6);
}
fn strip(h: &H, panel: &str) -> Strip {
    let layout = &h.state().ui.docking.as_ref().unwrap().layout;
    strips(h.state(), &h.ctx, layout).into_iter().find(|s| s.panels().any(|p| p == panel)).unwrap()
}
fn cell(h: &H, panel: &str) -> Rect {
    strip(h, panel).cells().into_iter().find(|(id, _)| id == panel).unwrap().1
}
fn grip(s: &Strip) -> Pos2 {
    pos2(s.rect.center().x, s.rect.top() + INSET + HEADER + GRIP * 0.5)
}
fn harness() -> H {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_step_dt(1.0 / 30.0).with_max_steps(64).wgpu().build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ProMedium);
        // GPU readbacks omit the OS cursor: the ring explicitly marks replayed input.
        cc.egui_ctx.on_end_pass(
            "panel-demo-pointer",
            Arc::new(|ui| {
                let ctx = ui.ctx();
                if let Some(at) = ctx.pointer_hover_pos() {
                    let t = crate::theme::Tokens::get(ctx);
                    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("panel-demo-pointer")));
                    painter.circle_stroke(at, 5.0, egui::Stroke::new(2.0, t.accent));
                }
            }),
        );
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.theme = crate::theme::ThemeKind::ProMedium;
        app.run("prefs.set", json!({"path":"interface.language","value":"en"})).unwrap();
        app.run("file.new", json!({"width":640,"height":480,"name":"Panel demo"})).unwrap();
        for (panel, x) in [("properties", 120), ("actions", 330)] {
            app.run("window.panel.edge", json!({"panel":panel,"edge":"floating","individual":true,"collapsed":true,"x":x,"y":160})).unwrap();
        }
        for &(id, _, _) in PANELS {
            if !["layers", "channels", "paths", "properties", "actions"].contains(&id)
                && app.ui.docking.as_ref().is_some_and(|w| w.layout.contains(&id.to_owned()))
            {
                app.run("window.panel.close", json!({"panel":id})).unwrap();
            }
        }
        app.sync_views();
        app
    });
    h.run_steps(8);
    h
}
fn joined() -> H {
    let mut h = harness();
    run(&mut h, "window.panel.move", json!({"panel":"actions","anchor":"properties","zone":"bottom"}));
    h
}
fn group_and_tear_off(root: &Path) {
    let mut h = harness();
    let mut clip = Clip::new(root, "01-group-flyout-tearoff");
    clip.capture(&mut h);
    let from = cell(&h, "actions").center();
    let to = cell(&h, "properties").center_bottom() - vec2(0.0, 2.0);
    clip.drag(&mut h, from, to, Modifiers::NONE, false);
    assert_eq!(h.state().ui.docking.as_ref().unwrap().layout.floating.len(), 1);
    let at = cell(&h, "actions").center();
    clip.click(&mut h, at);
    assert_eq!(h.state().ui.docking_flyout.as_deref(), Some("actions"));
    h.render().unwrap().save(root.join("adjacent-flyout.png")).unwrap();
    let from = cell(&h, "actions").center();
    clip.drag(&mut h, from, pos2(440.0, 390.0), Modifiers::NONE, false);
    assert_eq!(h.state().ui.docking.as_ref().unwrap().layout.floating.len(), 2);
}
fn insert_and_redock(root: &Path) {
    let mut h = joined();
    run(&mut h, "window.panel.edge", json!({"panel":"navigator","edge":"floating","individual":true,"collapsed":true,"x":430,"y":180}));
    let mut clip = Clip::new(root, "02-insert-redock");
    clip.capture(&mut h);
    let from = cell(&h, "navigator").center();
    let to = cell(&h, "properties").center_bottom() - vec2(0.0, 2.0);
    clip.drag(&mut h, from, to, Modifiers::NONE, false);
    assert_eq!(strip(&h, "properties").panels().map(String::as_str).collect::<Vec<_>>(), ["properties", "navigator", "actions"]);
    let from = cell(&h, "actions").center();
    let to = pos2(strip(&h, "properties").rect.center().x, cell(&h, "properties").top() + 1.0);
    clip.drag(&mut h, from, to, Modifiers::NONE, false);
    assert_eq!(strip(&h, "actions").panels().next().map(String::as_str), Some("actions"));
    let bounds = h.ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap();
    let from = grip(&strip(&h, "actions"));
    clip.drag(&mut h, from, pos2(bounds.left() + 2.0, 300.0), Modifiers::NONE, false);
    assert!(strip(&h, "actions").attached);
    let from = grip(&strip(&h, "actions"));
    clip.drag(&mut h, from, pos2(bounds.right() - 2.0, 300.0), Modifiers::NONE, false);
    assert!(strip(&h, "actions").attached && !strip(&h, "actions").left);
    let at = cell(&h, "navigator").center();
    clip.click(&mut h, at);
    h.render().unwrap().save(root.join("right-attached-flyout.png")).unwrap();
}
fn divider(h: &H, wanted: Divider<String>) -> Pos2 {
    DockArea::<String>::hit_rects(&h.ctx, area_id()).into_iter().find(|(hit, _)| *hit == Hit::Divider(wanted.clone())).unwrap().1.center()
}
fn full_stack(root: &Path) {
    let mut h = joined();
    let mut clip = Clip::new(root, "03-expanded-stack-resize");
    clip.capture(&mut h);
    let s = strip(&h, "properties");
    let at = pos2(s.rect.right() - 33.0, s.rect.top() + INSET + 10.0);
    clip.click(&mut h, at);
    assert!(h.state().ui.panel_presentation.compact.iter().all(|p| p != "properties" && p != "actions"));
    run(&mut h, "window.panel.edge", json!({"panel":"properties","edge":"left"}));
    clip.settle(&mut h);
    let from = divider(&h, Divider::Group("properties".into()));
    clip.drag(&mut h, from, from + vec2(0.0, 80.0), Modifiers::NONE, false);
    let from = divider(&h, Divider::Column("properties".into()));
    clip.drag(&mut h, from, from + vec2(75.0, 0.0), Modifiers::NONE, false);
    let body = last_bodies(&h.ctx).into_iter().filter(|(id, _)| ["properties", "actions"].contains(&id.as_str())).map(|(_, r)| r).reduce(Rect::union).unwrap();
    let from = pos2(body.center().x, body.bottom() - 2.0);
    clip.drag(&mut h, from, from - vec2(0.0, 100.0), Modifiers::NONE, false);
    h.render().unwrap().save(root.join("expanded-stack.png")).unwrap();
}
fn tools_rect(h: &H) -> Rect {
    h.ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique("hidden-panels-hover").with("tools"))).unwrap()
}
fn tools(root: &Path) {
    let mut h = harness();
    let mut clip = Clip::new(root, "04-whole-tools");
    clip.capture(&mut h);
    let r = tools_rect(&h);
    let from = pos2(r.center().x, r.top() + 3.0 + HEADER + GRIP * 0.5);
    clip.drag(&mut h, from, pos2(650.0, 160.0), Modifiers::NONE, false);
    assert!(h.state().ui.toolbar_floating.is_some());
    h.render().unwrap().save(root.join("floating-tools.png")).unwrap();
    let r = tools_rect(&h);
    clip.click(&mut h, pos2(r.center().x, r.top() + 13.0));
    assert!(h.state().ui.panels.toolbar_double);
    let r = tools_rect(&h);
    clip.click(&mut h, pos2(r.center().x, r.top() + 13.0));
    let r = tools_rect(&h);
    let from = pos2(r.center().x, r.top() + 3.0 + HEADER + GRIP * 0.5);
    clip.drag(&mut h, from, pos2(1.0, 300.0), Modifiers::NONE, false);
    assert!(h.state().ui.toolbar_floating.is_none());
}
fn compact_width(root: &Path) {
    let mut h = joined();
    let mut clip = Clip::new(root, "05-compact-width");
    clip.capture(&mut h);
    for delta in [35.0, 50.0, 75.0, -250.0] {
        let s = strip(&h, "properties");
        let from = pos2(s.rect.right() - 1.0, s.rect.center().y);
        clip.drag(&mut h, from, from + vec2(delta, 0.0), Modifiers::NONE, false);
    }
    assert!((strip(&h, "properties").rect.width() - 44.0).abs() < 0.5);
}
fn cancellation(root: &Path) {
    let mut h = joined();
    let mut clip = Clip::new(root, "06-cancel-lock");
    clip.capture(&mut h);
    let before = crate::dock::snapshot(h.state());
    let from = grip(&strip(&h, "properties"));
    let bounds = h.ctx.data(|d| d.get_temp::<Rect>(key("bounds"))).unwrap();
    clip.drag(&mut h, from, pos2(bounds.left() + 2.0, 300.0), Modifiers::CTRL, false);
    assert!(!strip(&h, "properties").attached);
    let before_escape = crate::dock::snapshot(h.state());
    let from = grip(&strip(&h, "properties"));
    clip.drag(&mut h, from, pos2(420.0, 280.0), Modifiers::NONE, true);
    assert_eq!(crate::dock::snapshot(h.state()), before_escape);
    invoke(&mut h, "window.workspace.lockWorkspace", json!({"on":true}));
    let locked = crate::dock::snapshot(h.state());
    let from = grip(&strip(&h, "properties"));
    clip.drag(&mut h, from, pos2(540.0, 340.0), Modifiers::NONE, false);
    assert_eq!(crate::dock::snapshot(h.state()), locked);
    assert_ne!(crate::dock::snapshot(h.state()), before);
}
fn restore(root: &Path) {
    let mut h = joined();
    run(&mut h, "window.panel.move", json!({"panel":"channels","anchor":"properties","zone":"tab"}));
    run(&mut h, "window.panel.group", json!({"panel":"properties","operation":"expand"}));
    run(&mut h, "window.panel.edge", json!({"panel":"properties","edge":"left"}));
    run(&mut h, "window.panel.group", json!({"panel":"properties","operation":"edge-size","width":350,"height":560,"heights":{"properties":3,"actions":2}}));
    invoke(&mut h, "window.workspace.newWorkspace", json!({"name":"Panel demo"}));
    let saved = h.state().ui.docking.clone();
    let mut clip = Clip::new(root, "07-workspace-restore");
    clip.capture(&mut h);
    run(&mut h, "window.panel.group", json!({"panel":"properties","operation":"collapse"}));
    clip.settle(&mut h);
    run(&mut h, "window.panel.edge", json!({"panel":"properties","edge":"floating","x":450,"y":240}));
    clip.settle(&mut h);
    invoke(&mut h, "window.workspace.select", json!({"name":"Panel demo"}));
    clip.settle(&mut h);
    assert_eq!(h.state().ui.docking, saved);
    h.render().unwrap().save(root.join("restored-tabs.png")).unwrap();
}

fn preferences(root: &Path) {
    let mut h = joined();
    run(&mut h, "window.panel.edge", json!({"panel":"properties","edge":"rightInner"}));
    let mut clip = Clip::new(root, "08-visibility-preferences");
    run(&mut h, "prefs.set", json!({"path":"workspace.autoCollapseIconPanels","value":true}));
    let at = cell(&h, "properties").center();
    clip.click(&mut h, at);
    clip.click(&mut h, pos2(850.0, 15.0));
    assert!(h.state().ui.docking_flyout.is_none());
    run(&mut h, "prefs.set", json!({"path":"workspace.autoCollapseIconPanels","value":false}));
    let at = cell(&h, "properties").center();
    clip.click(&mut h, at);
    clip.click(&mut h, pos2(850.0, 15.0));
    assert!(h.state().ui.docking_flyout.is_some());
    invoke(&mut h, "window.togglePanels", json!({}));
    clip.settle(&mut h);
    assert!(!h.state().ui.panels.dock);
    h.event(Event::PointerMoved(pos2(1279.0, 300.0)));
    clip.settle(&mut h);
    assert!(h.query_by_label("Layers").is_some());
    assert!(!h.state().ui.panels.dock, "edge reveal must not rewrite visibility");
    h.event(Event::PointerMoved(pos2(600.0, 500.0)));
    h.run_steps(15);
    clip.settle(&mut h);
    assert!(h.query_by_label("Layers").is_none());
    invoke(&mut h, "window.togglePanels", json!({}));
    clip.settle(&mut h);
}

#[test]
fn current_version_showcase() {
    let Some(root) = std::env::var_os("PHOTOCRAFT_PANEL_SHOWCASE").map(PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("build.txt"),
        format!(
            "PhotoCraft {}\nNative egui/wgpu offscreen UI; synthetic document; no desktop recording.\nRing marks replayed input (test-only overlay).\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
    group_and_tear_off(&root);
    insert_and_redock(&root);
    full_stack(&root);
    tools(&root);
    compact_width(&root);
    cancellation(&root);
    restore(&root);
    preferences(&root);
    let mut h = harness();
    invoke(&mut h, "edit.preferences.workspace", json!({}));
    h.render().unwrap().save(root.join("workspace-preferences.png")).unwrap();
    let mut h = harness();
    invoke(&mut h, "help.about", json!({}));
    h.render().unwrap().save(root.join("about-version.png")).unwrap();
}
