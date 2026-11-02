//! Opt-in pointer replays for the complete PR gallery; no product UI or captions.
use super::*;
use craft_ui::docking::{Placement, Target, Zone};

struct Take {
    clip: Clip,
    focus: Vec<&'static str>,
    frames: Vec<Value>,
}

impl Take {
    fn new(root: &Path, name: &str, focus: &[&'static str], h: &mut H) -> Self {
        let mut take = Self { clip: Clip::new(root, name), focus: focus.to_vec(), frames: Vec::new() };
        take.snap(h, 1000);
        take
    }
    fn snap(&mut self, h: &mut H, duration: u32) {
        let mut rects: Vec<[f32; 4]> = h
            .ctx
            .data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects")))
            .unwrap_or_default()
            .into_iter()
            .filter(|(id, _)| self.focus.contains(&id.as_str()))
            .map(|(_, r)| [r.left(), r.top(), r.right(), r.bottom()])
            .collect();
        if self.focus.contains(&"tools") {
            let r = tools_rect(h);
            rects.push([r.left(), r.top(), r.right(), r.bottom()]);
        }
        if self.focus.contains(&"documents")
            && let Some(tabs) = &h.state().tab_strip
        {
            let r = tabs.rect;
            rects.push([r.left(), r.top(), r.right(), r.bottom()]);
        }
        self.clip.capture(h);
        self.frames.push(json!({"duration_ms":duration,"focus_rects":rects}));
    }
    fn pause(&mut self, h: &mut H) {
        h.run_steps(8);
        self.snap(h, 1600);
    }
    fn tap(&mut self, h: &mut H, at: Pos2) {
        h.event(Event::PointerMoved(at));
        h.run_steps(2);
        self.snap(h, 450);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        self.snap(h, 180);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        self.pause(h);
    }
    fn drag(&mut self, h: &mut H, from: Pos2, to: Pos2) {
        h.event(Event::PointerMoved(from));
        h.run_steps(2);
        self.snap(h, 500);
        h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        self.snap(h, 180);
        for step in 1..=20 {
            h.event(Event::PointerMoved(from + (to - from) * (step as f32 / 20.0)));
            h.run_steps(1);
            self.snap(h, 70);
        }
        self.snap(h, 500);
        h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        self.pause(h);
    }
    fn finish(&self) {
        assert_eq!(self.frames.len(), self.clip.frame);
        std::fs::write(self.clip.directory.join("frames.json"), serde_json::to_string_pretty(&self.frames).unwrap()).unwrap();
    }
}

fn scene() -> H {
    let mut h = harness();
    for panel in ["properties", "actions"] {
        run(&mut h, "window.panel.close", json!({"panel":panel}));
    }
    h
}
fn floating(h: &mut H, panel: &str, x: f32, compact: bool) {
    run(h, "window.panel.edge", json!({"panel":panel,"edge":"floating","individual":true,"collapsed":compact,"x":x,"y":180,"width":320,"height":400}));
}
fn group(h: &H, panel: &str) -> Rect {
    h.ctx.data(|d| d.get_temp::<Vec<(String, Rect)>>(key("group-rects"))).unwrap().into_iter().find(|(id, _)| id == panel).unwrap().1
}
fn tab(h: &H, panel: &str) -> Rect {
    DockArea::<String>::hit_rects(&h.ctx, area_id()).into_iter().find(|(hit, _)| matches!(hit, Hit::Tab(id) if id==panel)).unwrap().1
}
fn expand(h: &H, panel: &str) -> Pos2 {
    h.ctx.read_response(key("expand").with(panel)).unwrap().rect.center()
}
fn collapse(h: &H, panel: &str) -> Pos2 {
    h.ctx.read_response(area_id().with(("full-chevron", panel))).unwrap().rect.center()
}
fn pair(compact: bool, zone: Zone) -> H {
    let mut h = scene();
    floating(&mut h, "color", 330.0, compact);
    floating(&mut h, "brushes", 730.0, compact);
    run(
        &mut h,
        "window.panel.group",
        json!({"panel":"brushes","operation":"drop","whole":true,"unit":"group","to":Target::At{anchor:"color".to_owned(),placement:Placement::Split(zone)},"sourceWidth":if compact{42}else{320},"sourceHeight":if compact{68}else{400}}),
    );
    h
}

fn documents(root: &Path) {
    let mut h = scene();
    floating(&mut h, "brushes", 700.0, false);
    let mut take = Take::new(root, "11-document-row", &["brushes", "documents"], &mut h);
    let row = h.state().tab_strip.as_ref().unwrap().rect;
    let from = tab(&h, "brushes").center();
    take.drag(&mut h, from, pos2(row.center().x, row.bottom() + 1.0));
    assert!(group(&h, "brushes").top() >= h.state().tab_strip.as_ref().unwrap().rect.bottom() - 1.1);
    let at = collapse(&h, "brushes");
    take.tap(&mut h, at);
    take.tap(&mut h, at);
    take.finish();
}
fn stack_height(root: &Path) {
    let mut h = pair(true, Zone::Top);
    let mut take = Take::new(root, "12-compact-height", &["color", "brushes"], &mut h);
    let at = expand(&h, "brushes");
    take.tap(&mut h, at);
    for panel in ["color", "brushes"] {
        assert!(group(&h, panel).height() >= 300.0);
    }
    let height = group(&h, "brushes").height();
    let at = collapse(&h, "brushes");
    take.tap(&mut h, at);
    let at = expand(&h, "brushes");
    take.tap(&mut h, at);
    assert!((group(&h, "brushes").height() - height).abs() < 1.1);
    take.finish();
}
fn bottom_height(root: &Path) {
    let mut h = pair(false, Zone::Right);
    let mut take = Take::new(root, "13-bottom-height", &["color", "brushes"], &mut h);
    let before = group(&h, "color").union(group(&h, "brushes"));
    let widths = [group(&h, "color").width(), group(&h, "brushes").width()];
    let from = before.right_bottom() - vec2(2.0, 0.25);
    take.drag(&mut h, from, from + vec2(0.0, 80.0));
    assert!((group(&h, "color").union(group(&h, "brushes")).height() - before.height() - 80.0).abs() < 1.1);
    for (i, panel) in ["color", "brushes"].iter().enumerate() {
        assert!((group(&h, panel).width() - widths[i]).abs() < 1.1);
    }
    let r = group(&h, "color").union(group(&h, "brushes"));
    let from = r.center_bottom() - vec2(0.0, 0.25);
    take.drag(&mut h, from, from - vec2(0.0, 55.0));
    take.finish();
}
fn independent_parts(root: &Path) {
    let mut h = scene();
    for (panel, x) in [("navigator", 140.0), ("brushes", 490.0), ("properties", 830.0)] {
        floating(&mut h, panel, x, false);
    }
    for (panel, anchor) in [("brushes", "navigator"), ("properties", "brushes")] {
        run(
            &mut h,
            "window.panel.group",
            json!({"panel":panel,"operation":"drop","whole":true,"unit":"group","to":Target::At{anchor:anchor.to_owned(),placement:Placement::Split(Zone::Right)},"sourceWidth":320,"sourceHeight":400}),
        );
    }
    let mut take = Take::new(root, "14-middle-width", &["navigator", "brushes", "properties"], &mut h);
    let widths = [group(&h, "navigator").width(), group(&h, "brushes").width(), group(&h, "properties").width()];
    let from = group(&h, "brushes").right_center() - vec2(0.25, 0.0);
    take.drag(&mut h, from, from + vec2(55.0, 0.0));
    assert!((group(&h, "brushes").width() - widths[1] - 55.0).abs() < 1.1);
    assert!((group(&h, "navigator").width() - widths[0]).abs() < 1.1);
    assert!((group(&h, "properties").width() - widths[2]).abs() < 1.1);
    take.finish();
    drop(take);
    let middle_width = group(&h, "brushes").width();
    let mut take = Take::new(root, "15-independent-parts", &["navigator", "brushes", "properties"], &mut h);
    for panel in ["navigator", "properties"] {
        let at = collapse(&h, panel);
        take.tap(&mut h, at);
        assert!((group(&h, "brushes").width() - middle_width).abs() < 1.1);
    }
    for panel in ["navigator", "properties"] {
        let at = expand(&h, panel);
        take.tap(&mut h, at);
        assert!((group(&h, "brushes").width() - middle_width).abs() < 1.1);
    }
    take.finish();
}
fn tools_resize(root: &Path) {
    let mut h = scene();
    let mut take = Take::new(root, "16-whole-tools", &["tools"], &mut h);
    let r = tools_rect(&h);
    let from = pos2(r.center().x, r.top() + 3.0 + HEADER + GRIP * 0.5);
    take.drag(&mut h, from, pos2(650.0, 160.0));
    assert!(h.state().ui.toolbar_floating.is_some());
    let before = tools_rect(&h);
    let from = before.center_bottom() - vec2(0.0, 0.25);
    take.drag(&mut h, from, from - vec2(0.0, 100.0));
    assert!((tools_rect(&h).height() - before.height() + 100.0).abs() < 1.1);
    let before = tools_rect(&h);
    let from = before.right_center() - vec2(0.25, 0.0);
    take.drag(&mut h, from, from + vec2(50.0, 0.0));
    assert!((tools_rect(&h).width() - before.width() - 50.0).abs() < 1.1);
    let r = tools_rect(&h);
    let from = pos2(r.center().x, r.top() + 3.0 + HEADER + GRIP * 0.5);
    take.drag(&mut h, from, pos2(1.0, 300.0));
    assert!(h.state().ui.toolbar_floating.is_none());
    take.finish();
}
fn full_join(root: &Path) {
    let mut h = scene();
    floating(&mut h, "color", 180.0, false);
    floating(&mut h, "brushes", 720.0, true);
    let mut take = Take::new(root, "17-full-join-detach", &["color", "brushes"], &mut h);
    let height = group(&h, "color").height();
    let from = tab(&h, "color").center();
    let to = strip(&h, "brushes").rect.right_center() - vec2(1.0, 0.0);
    take.drag(&mut h, from, to);
    assert!((group(&h, "color").height() - height).abs() < 1.1);
    let from = tab(&h, "color").center();
    take.drag(&mut h, from, pos2(220.0, 240.0));
    assert!((group(&h, "color").height() - height).abs() < 1.1);
    take.finish();
}
fn attached_tabs(root: &Path) {
    let mut h = scene();
    floating(&mut h, "history", 720.0, true);
    floating(&mut h, "color", 200.0, false);
    floating(&mut h, "brushes", 620.0, true);
    let mut take = Take::new(root, "18-attached-tab-insertion", &["history", "color", "brushes", "documents"], &mut h);
    let canvas = last_bodies(&h.ctx).into_iter().find(|(id, _)| id == "canvas").unwrap().1;
    let from = grip(&strip(&h, "history"));
    take.drag(&mut h, from, pos2(canvas.center().x, canvas.top() + 1.0));
    for _ in 0..2 {
        let at = collapse(&h, "history");
        take.tap(&mut h, at);
    }
    let from = tab(&h, "color").center();
    let to = tab(&h, "history").right_center() + vec2(2.0, 0.0);
    take.drag(&mut h, from, to);
    let from = cell(&h, "brushes").center();
    let to = tab(&h, "history").right_center();
    take.drag(&mut h, from, to);
    assert_eq!(crate::panel_docking::group_members(&h.state().ui.docking.as_ref().unwrap().layout, "history"), ["history", "brushes", "color"]);
    let from = tab(&h, "brushes").center();
    let to = tab(&h, "history").left_center() + vec2(2.0, 0.0);
    take.drag(&mut h, from, to);
    assert_eq!(crate::panel_docking::group_members(&h.state().ui.docking.as_ref().unwrap().layout, "history"), ["brushes", "history", "color"]);
    let at = collapse(&h, "brushes");
    take.tap(&mut h, at);
    let at = collapse(&h, "brushes");
    take.tap(&mut h, at);
    take.finish();
}

#[test]
fn pr_followup_pointer_replay() {
    let Some(root) = std::env::var_os("PHOTOCRAFT_PR_FOLLOWUP_REPLAY").map(PathBuf::from) else { return };
    let _gpu = photocraft_testkit::gpu_lock();
    std::fs::create_dir_all(&root).unwrap();
    documents(&root);
    stack_height(&root);
    bottom_height(&root);
    independent_parts(&root);
    tools_resize(&root);
    full_join(&root);
    attached_tabs(&root);
}
