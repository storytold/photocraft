use egui::{Area, Pos2, pos2, vec2};
use egui_kittest::Harness;

use super::*;

/// What the test frame draws: a big button under everything, a small foreground window (a menu
/// or dialog), another one, and a full-window workspace.
#[derive(Default)]
struct Scene {
    open: bool,
    other: bool,
    workspace: bool,
    clicks: u32,
}

const AT: Pos2 = pos2(40.0, 40.0);

fn harness(animation_time: f32) -> Harness<'static, Scene> {
    Harness::builder().with_size(vec2(400.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, s: &mut Scene| {
            let ctx = ui.ctx().clone();
            ctx.all_styles_mut(|st| st.animation_time = animation_time);
            if ui.add_sized(vec2(380.0, 280.0), egui::Button::new("Under")).clicked() {
                s.clicks += 1;
            }
            let window = |id: &str, at: Pos2, text: &str, size: Option<egui::Vec2>| {
                Area::new(Id::new(id)).order(Order::Foreground).fixed_pos(at).show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        if let Some(size) = size {
                            ui.set_min_size(size);
                        }
                        ui.label(text);
                    });
                });
            };
            if s.open {
                window("window", AT, "Ghost me", None);
            }
            if s.other {
                window("other", pos2(200.0, 40.0), "Other", None);
            }
            if s.workspace {
                window("workspace", Pos2::ZERO, "Workspace", Some(vec2(400.0, 300.0)));
            }
            finish_frame(&ctx);
        },
        Scene::default(),
    )
}

/// The opacity of each painted `text` in the last frame: its colour's alpha (a painter's opacity
/// fades the colours; a label's glyphs take the shape's fallback colour) times the shape's own.
fn painted(h: &Harness<'_, Scene>, text: &str) -> Vec<f32> {
    h.output()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Text(t) if t.galley.text() == text => Some(f32::from(t.fallback_color.a()) / 255.0 * t.opacity_factor),
            _ => None,
        })
        .collect()
}

fn ghosts(h: &Harness<'_, Scene>) -> usize {
    h.ctx.data(|d| d.get_temp::<State>(state_id())).map_or(0, |s| s.ghosts.len())
}

fn open_then_close(h: &mut Harness<'_, Scene>, set: impl Fn(&mut Scene, bool)) {
    set(h.state_mut(), true);
    h.run_steps(30);
    set(h.state_mut(), false);
    h.step();
}

#[test]
fn a_closed_window_fades_out_and_takes_no_clicks() {
    let mut h = harness(0.2);
    open_then_close(&mut h, |s, on| s.open = on);
    let first = painted(&h, "Ghost me");
    assert!(matches!(first.as_slice(), [o] if *o > 0.5 && *o < 1.0), "fading from its last frame: {first:?}");
    h.step();
    let next = painted(&h, "Ghost me");
    assert!(matches!(next.as_slice(), [o] if *o < first[0]), "fainter each frame: {next:?}");
    // A click where it was goes to what is under it.
    let p = AT + vec2(12.0, 12.0);
    h.hover_at(p);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    assert!(!painted(&h, "Ghost me").is_empty(), "still fading while clicked through");
    assert_eq!(h.state().clicks, 1);
    h.run_steps(15);
    assert!(painted(&h, "Ghost me").is_empty(), "gone after the animation time");
    assert_eq!(ghosts(&h), 0);
}

#[test]
fn with_animations_off_a_window_closes_at_once() {
    let mut h = harness(0.0);
    open_then_close(&mut h, |s, on| s.open = on);
    assert!(painted(&h, "Ghost me").is_empty());
    assert_eq!(ghosts(&h), 0);
}

/// Moving between menus or submenus closes one window as another opens: no trail.
#[test]
fn a_window_replaced_in_the_same_frame_does_not_fade() {
    let mut h = harness(0.2);
    h.state_mut().open = true;
    h.run_steps(30);
    h.state_mut().open = false;
    h.state_mut().other = true;
    h.step();
    assert!(painted(&h, "Ghost me").is_empty());
    assert_eq!(ghosts(&h), 0);
}

#[test]
fn a_full_window_workspace_does_not_fade() {
    let mut h = harness(0.2);
    open_then_close(&mut h, |s, on| s.workspace = on);
    assert!(painted(&h, "Workspace").is_empty());
    assert_eq!(ghosts(&h), 0);
}

#[test]
fn reopening_a_fading_window_stops_the_fade() {
    let mut h = harness(0.2);
    open_then_close(&mut h, |s, on| s.open = on);
    assert_eq!(ghosts(&h), 1);
    h.state_mut().open = true;
    h.step();
    assert_eq!(ghosts(&h), 0);
    assert_eq!(painted(&h, "Ghost me").len(), 1, "only the window itself");
}
