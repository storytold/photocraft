//! Dock splitters through the whole app (#295): the gaps between the right dock's panel groups
//! resize them, with the Layers panel and the canvas around them, at the window sizes people use.

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_ui_egui::dock::{GAP, Group, last_rects};
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::json;

type H = Harness<'static, PhotocraftApp>;

fn app(w: f32, h: f32) -> H {
    let mut h = Harness::builder().with_size(vec2(w, h)).with_pixels_per_point(1.0).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).expect("new");
        for _ in 0..6 {
            app.run("layer.new.layer", json!({})).expect("layer");
        }
        app.sync_views();
        app
    });
    h.run_steps(6);
    h
}

fn rect(h: &H, g: Group) -> egui::Rect {
    last_rects(&h.ctx).into_iter().find(|(x, _)| *x == g).map(|(_, r)| r).unwrap_or_else(|| panic!("{g:?} not drawn"))
}

fn press(h: &mut H, p: Pos2) {
    h.event(Event::PointerMoved(p));
    h.step();
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
}

fn release(h: &mut H, p: Pos2) {
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn drag(h: &mut H, from: Pos2, to: Pos2) {
    press(h, from);
    for i in 1..=6 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 6.0)));
        h.step();
    }
    release(h, to);
}

#[test]
fn the_splitter_above_layers_gives_layers_room_in_the_real_app() {
    for (w, hgt) in [(1440.0, 900.0), (1280.0, 720.0), (1920.0, 1080.0)] {
        let mut h = app(w, hgt);
        let layers = rect(&h, Group::Layers);
        // Grab anywhere along the gap, not only its exact middle.
        for dy in [-4.0, 0.0, 4.0] {
            let props = rect(&h, Group::Properties);
            let at = Pos2::new(props.center().x + 40.0, props.bottom() + GAP / 2.0 + dy);
            let before = rect(&h, Group::Layers);
            let room = (props.height() - Group::Properties.min_height()).min(15.0);
            drag(&mut h, at, at - vec2(0.0, 15.0));
            let after = rect(&h, Group::Layers);
            assert!((after.height() - (before.height() + room)).abs() < 1.0, "{w}×{hgt} dy {dy}: {before:?} -> {after:?}");
        }
        assert!(rect(&h, Group::Layers).height() > layers.height(), "{w}×{hgt}");
        assert!(h.state().ui.dock.heights.contains_key(&Group::Properties), "the new size is in the layout");
        // The Properties group never goes below its minimum.
        let props = rect(&h, Group::Properties);
        let at = Pos2::new(props.center().x, props.bottom() + GAP / 2.0);
        drag(&mut h, at, at - vec2(0.0, 900.0));
        assert_eq!(rect(&h, Group::Properties).height(), Group::Properties.min_height(), "{w}×{hgt}");
    }
}

#[test]
fn double_clicking_a_splitter_resets_both_groups() {
    let mut h = app(1440.0, 900.0);
    let props0 = rect(&h, Group::Properties);
    let at = Pos2::new(props0.center().x, props0.bottom() + GAP / 2.0);
    drag(&mut h, at, at - vec2(0.0, 60.0));
    assert!(rect(&h, Group::Properties).height() < props0.height() - 50.0);
    let at = Pos2::new(props0.center().x, rect(&h, Group::Properties).bottom() + GAP / 2.0);
    for _ in 0..2 {
        press(&mut h, at);
        release(&mut h, at);
    }
    h.run_steps(3);
    assert_eq!(rect(&h, Group::Properties), props0, "back to the default height");
    assert!(!h.state().ui.dock.heights.contains_key(&Group::Properties));
}
