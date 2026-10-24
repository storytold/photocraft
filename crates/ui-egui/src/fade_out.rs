//! Dialogs, menus and popups fade out when they close, as they fade in when they open. egui fades
//! an area in over the style's animation time but drops a closed one at once, so a dialog or menu
//! used to vanish in one frame; Photoshop's fade out with the system's window animations
//! (Windows). A foreground window that is no longer drawn is painted from its last frame for one
//! animation time, fading, and takes no input: clicks go to whatever is under it.
//!
//! Interface › Enable Animations off (an animation time of 0) turns this off with the fades in.
//! A window replaced in the same frame (moving between menus or submenus, a dialog that opens
//! another) and a full-window workspace (Camera Raw, Liquify, the command palette's scrim) just
//! go, as before.

use egui::epaint::ClippedShape;
use egui::{Context, Id, LayerId, Order, Shape, TextureId};

/// A closed window's last frame, fading out.
#[derive(Clone)]
struct Ghost {
    /// The window it was: drawn again, it stops fading.
    from: LayerId,
    shapes: Vec<ClippedShape>,
    /// When it was last drawn (`InputState::time`): it fades from there.
    start: f64,
}

/// A foreground window drawn last frame.
#[derive(Clone)]
struct Shown {
    layer: LayerId,
    shapes: Vec<ClippedShape>,
    /// Frames drawn in a row: a window drawn for one frame only doesn't fade.
    frames: u32,
    /// When it was last drawn.
    at: f64,
}

#[derive(Clone, Default)]
struct State {
    shown: Vec<Shown>,
    ghosts: Vec<Ghost>,
}

fn state_id() -> Id {
    Id::new("pc-fade-out")
}

/// A window this much of the screen or more is a workspace, not a dialog or menu.
const WORKSPACE: f32 = 0.9;

/// After every window of the frame is drawn: notice the foreground windows that closed and paint
/// the ones still fading.
pub fn finish_frame(ctx: &Context) {
    // A pass egui will redo (a sizing pass) is not the frame shown: judge the one that is.
    if ctx.will_discard() {
        return;
    }
    let mut st: State = ctx.data_mut(|d| d.remove_temp(state_id())).unwrap_or_default();
    let time = ctx.global_style().animation_time;
    let now = ctx.input(|i| i.time);
    let screen = ctx.content_rect().area().max(1.0);
    let mut layers: Vec<LayerId> = ctx.memory(|m| m.areas().visible_layer_ids()).into_iter().filter(|l| l.order == Order::Foreground).collect();
    layers.sort_by_key(|l| l.id.value());
    let mut drawn = Vec::new();
    for layer in layers {
        let workspace = ctx.memory(|m| m.area_rect(layer.id)).is_some_and(|r| r.area() >= WORKSPACE * screen);
        let shapes: Vec<ClippedShape> =
            if workspace { Vec::new() } else { ctx.graphics(|g| g.get(layer).map(|list| list.all_entries().cloned().collect()).unwrap_or_default()) };
        if workspace || !shapes.is_empty() {
            let frames = st.shown.iter().find(|s| s.layer == layer).map_or(1, |s| s.frames.saturating_add(1));
            drawn.push(Shown { layer, shapes, frames, at: now });
        }
    }
    // Drawn again (it reopened, or a pass left it out): no longer fading.
    st.ghosts.retain(|g| !drawn.iter().any(|s| s.layer == g.from));
    let replaced = drawn.iter().any(|s| !st.shown.iter().any(|p| p.layer == s.layer));
    let gone = std::mem::take(&mut st.shown).into_iter().filter(|p| !drawn.iter().any(|s| s.layer == p.layer));
    if time > 0.0 && !replaced {
        st.ghosts.extend(gone.filter(|p| p.frames >= 2 && !p.shapes.is_empty()).map(|p| Ghost { from: p.layer, shapes: p.shapes, start: p.at }));
    }
    st.shown = drawn;
    st.ghosts.retain(|g| now - g.start < f64::from(time));
    for g in &st.ghosts {
        let opacity = (1.0 - (now - g.start) / f64::from(time).max(1e-6)).clamp(0.0, 1.0) as f32;
        paint(ctx, g, opacity);
    }
    if !st.ghosts.is_empty() {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(state_id(), st));
}

/// Paints a ghost at `opacity` on a layer of its own, above the foreground windows. Not an area,
/// so nothing there takes input. Custom GPU callbacks and images whose texture is gone with the
/// window are left out.
fn paint(ctx: &Context, g: &Ghost, opacity: f32) {
    let layer = LayerId::new(Order::Foreground, state_id().with(g.from.id));
    let textures = ctx.tex_manager();
    let alive = |id: TextureId| matches!(id, TextureId::Managed(_)) && textures.read().meta(id).is_some();
    for c in &g.shapes {
        if matches!(c.shape, Shape::Callback(_)) || !alive(c.shape.texture_id()) {
            continue;
        }
        let mut painter = ctx.layer_painter(layer).with_clip_rect(c.clip_rect);
        painter.set_opacity(opacity);
        painter.add(c.shape.clone());
    }
}

#[cfg(test)]
mod tests;
