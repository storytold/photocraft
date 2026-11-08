//! The Mixer Brush's Current Brush Load swatch shows what the reservoir holds (#11953): a solid
//! colour after Load Brush, the sampled 64 × 64 tip image after an ⌥-click (an edge between green
//! and blue shows green on the left and blue on the right), and a checkerboard after Clean Brush.

use egui::{Modifiers, Rect, Shape, TextureId, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::mixer_load_image;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// A pixel layer green left of x 100 and blue from there on, with the Mixer Brush active.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": GREEN})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 0, "width": 100, "height": 100})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": BLUE})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#ffffff"})).unwrap();
    app.ui.tool = Tool::MixerBrush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 1.0}})).unwrap();
    app
}

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1600.0, 120.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

fn flatten<'a>(shape: &'a Shape, out: &mut Vec<&'a Shape>) {
    if let Shape::Vec(v) = shape {
        v.iter().for_each(|s| flatten(s, out));
    } else {
        out.push(shape);
    }
}

/// What the swatch draws this frame: the fill colours of its rectangles and the textures of its
/// images, inside the swatch's chip.
fn swatch(h: &Harness<'static, PhotocraftApp>) -> (Vec<egui::Color32>, Vec<TextureId>) {
    let r = h.get_by_label("Current Brush Load").rect();
    let chip = Rect::from_min_size(egui::pos2(r.left() + 4.0, r.center().y - 8.0), vec2(16.0, 16.0)).expand(0.5);
    let (mut fills, mut textures) = (Vec::new(), Vec::new());
    for clipped in &h.output().shapes {
        let mut all = Vec::new();
        flatten(&clipped.shape, &mut all);
        for s in all {
            match s {
                Shape::Rect(rs) if chip.contains_rect(rs.rect) => fills.push(rs.fill),
                Shape::Mesh(m) if m.texture_id != TextureId::default() && chip.contains_rect(m.calc_bounds()) => textures.push(m.texture_id),
                _ => {}
            }
        }
    }
    (fills, textures)
}

fn has_fonts(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into())))
}

#[test]
fn the_sampled_image_keeps_its_left_and_right() {
    let load: Vec<[f32; 4]> = (0..64 * 64).map(|i| if i % 64 < 32 { GREEN } else { BLUE }).collect();
    let image = mixer_load_image(&load);
    assert_eq!(image.size, [64, 64]);
    assert_eq!(image[(10, 32)], egui::Color32::from_rgb(0, 255, 0));
    assert_eq!(image[(54, 32)], egui::Color32::from_rgb(0, 0, 255));
}

#[test]
fn the_swatch_shows_the_load_the_tip_image_or_a_checkerboard() {
    let mut h = harness();
    if !has_fonts(&h) {
        return;
    }
    // Load Brush: one solid red, no image.
    h.state_mut().run("brush.mixer.load", json!({})).unwrap();
    h.run_steps(3);
    let (fills, textures) = swatch(&h);
    assert!(fills.contains(&egui::Color32::from_rgb(255, 0, 0)) && textures.is_empty(), "{fills:?} {textures:?}");

    // ⌥-click on the green/blue edge: the sampled tip image, green left and blue right.
    tool_event(h.state_mut(), ToolEvent::Down { x: 100.0, y: 50.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(h.state_mut(), ToolEvent::Up { x: 100.0, y: 50.0 }, Modifiers::ALT);
    h.run_steps(3);
    let load = h.state().session.tools.mixer.spatial.clone().expect("the ⌥-click loads the tip image");
    let (_, textures) = swatch(&h);
    assert_eq!(textures.len(), 1, "the swatch draws the tip image");
    let image = mixer_load_image(&load);
    let (left, right) = (image[(16, 32)], image[(48, 32)]);
    assert!(left.g() > 200 && left.b() < 50, "left of the image {left:?}");
    assert!(right.b() > 200 && right.g() < 50, "right of the image {right:?}");

    // Clean Brush: a checkerboard of two greys, no image.
    h.state_mut().run("brush.mixer.clean", json!({})).unwrap();
    h.run_steps(3);
    let (fills, textures) = swatch(&h);
    let greys: std::collections::BTreeSet<_> = fills.iter().filter(|c| c.r() == c.g() && c.g() == c.b() && c.r() > 150).map(|c| c.r()).collect();
    assert!(greys.len() >= 2 && textures.is_empty(), "checkerboard {fills:?} {textures:?}");
}
