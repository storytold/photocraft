use std::sync::Arc;

use egui::{Area, Color32, Pos2, Rect, TextureHandle, pos2, vec2};
use egui_kittest::Harness;

use super::*;

/// What the test frame draws: a big button under everything, small foreground windows (menus or
/// dialogs), a full-window workspace, and a window with GPU content (a custom callback, a user
/// texture and a texture of its own).
#[derive(Default)]
struct Scene {
    open: bool,
    other: bool,
    third: bool,
    workspace: bool,
    gpu: bool,
    /// The GPU window's texture: dropped (freed) when it closes.
    texture: Option<TextureHandle>,
    /// What the GPU window's callback holds: a ghost must not keep it alive.
    resource: Arc<()>,
    clicks: u32,
}

/// A user texture (one the renderer owns, like the GPU canvas).
const USER_TEXTURE: TextureId = TextureId::User(7);

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
                Area::new(Id::unique(id)).order(Order::Foreground).fixed_pos(at).show(&ctx, |ui| {
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
            if s.third {
                window("third", pos2(300.0, 40.0), "Third", None);
            }
            if s.workspace {
                window("workspace", Pos2::ZERO, "Workspace", Some(vec2(400.0, 300.0)));
            }
            if s.gpu {
                let texture = s.texture.get_or_insert_with(|| ctx.load_texture("ghost", egui::ColorImage::filled([2, 2], Color32::RED), Default::default())).id();
                let resource = Arc::clone(&s.resource);
                Area::new(Id::unique("gpu")).order(Order::Foreground).fixed_pos(pos2(40.0, 150.0)).show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label("Gpu text");
                        let (rect, _) = ui.allocate_exact_size(vec2(60.0, 20.0), egui::Sense::hover());
                        let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
                        ui.painter().add(Shape::Callback(egui::PaintCallback { rect, callback: resource }));
                        ui.painter().image(USER_TEXTURE, rect, uv, Color32::WHITE);
                        ui.painter().image(texture, rect, uv, Color32::WHITE);
                    });
                });
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

#[test]
fn with_animations_off_no_window_is_snapshotted() {
    let mut h = harness(0.0);
    h.state_mut().open = true;
    h.run_steps(5);
    assert!(h.ctx.data(|d| d.get_temp::<State>(state_id())).is_none(), "no per-frame copy of an open window's shapes");
}

/// Moving along a menu bar or down a chain of submenus: each window replaces the last, and none
/// leaves a trail, in that frame or later.
#[test]
fn a_chain_of_windows_replacing_each_other_leaves_no_trail() {
    let mut h = harness(0.2);
    h.state_mut().open = true;
    h.run_steps(30);
    for (from, to) in [(0, 1), (1, 2), (2, 0), (0, 1)] {
        let s = h.state_mut();
        for (i, on) in [&mut s.open, &mut s.other, &mut s.third].into_iter().enumerate() {
            *on = i == to;
        }
        // A new window shows after egui's sizing pass; the one it replaced never paints again.
        for _ in 0..30 {
            h.step();
            for (i, text) in ["Ghost me", "Other", "Third"].into_iter().enumerate() {
                assert!(i == to || painted(&h, text).is_empty(), "{text} trails after {from} -> {to}");
            }
            assert_eq!(ghosts(&h), 0, "after {from} -> {to}");
        }
        assert_eq!(painted(&h, ["Ghost me", "Other", "Third"][to]).len(), 1);
    }
}

/// A dialog that opened from another (Image Size's Resample options, Save As' format options) closes
/// as its parent comes back in the same frame: the parent is just there, no trail of the child.
#[test]
fn a_dialog_returning_to_its_parent_leaves_no_trail() {
    let mut h = harness(0.2);
    h.state_mut().other = true;
    h.run_steps(30);
    h.state_mut().other = false;
    h.state_mut().open = true;
    for _ in 0..30 {
        h.step();
        assert!(painted(&h, "Other").is_empty());
        assert_eq!(ghosts(&h), 0);
    }
    assert_eq!(painted(&h, "Ghost me").len(), 1);
}

/// A closed window's ghost paints its own text and frame but none of its GPU content: no custom
/// callback (whose resources may be gone), no user texture, and no texture freed with the window.
#[test]
fn a_ghost_paints_no_stale_textures_or_callbacks() {
    let mut h = harness(0.2);
    h.state_mut().gpu = true;
    h.run_steps(30);
    assert!(Arc::strong_count(&h.state().resource) > 1, "the open window's callback holds it");
    let freed = h.state().texture.as_ref().map(TextureHandle::id).expect("the window made its texture");
    h.state_mut().gpu = false;
    h.state_mut().texture = None;
    h.step();
    assert_eq!(ghosts(&h), 1);
    assert_eq!(painted(&h, "Gpu text").len(), 1, "the ghost fades its text");
    for c in &h.output().shapes {
        assert!(!matches!(c.shape, Shape::Callback(_)), "no callback painted");
        let id = c.shape.texture_id();
        assert!(id != USER_TEXTURE && id != freed, "no stale texture painted: {id:?}");
    }
    assert_eq!(Arc::strong_count(&h.state().resource), 1, "the ghost keeps no callback alive");
}
