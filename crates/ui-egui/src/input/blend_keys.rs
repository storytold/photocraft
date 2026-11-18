//! Blend mode keys, as in Photoshop. ⇧+ and ⇧− step through the modes (#3102), and ⇧⌥ plus a
//! letter picks one (⇧⌥M Multiply, ⇧⌥S Screen…). With a painting tool that has a Mode of its own
//! (`tool_modes`: Brush, Pencil, Paint Bucket, Gradient, the stamps, History Brush, Healing) they
//! set that tool's mode within its menu and never the layer's. With a tool that has no blend mode
//! (Move, the selection tools, the Erasers…) they set every selected layer's mode. With the Color
//! Replacement tool the ⇧⌥ letters for Normal, Multiply, Hue and Luminosity set the active layer's
//! mode too and its own Mode stays, as measured in the reference application (#11928); its other
//! letters, the letters with several layers selected, and ⇧+ / ⇧− do nothing.
//! The step list wraps at either end. Each key is one call of an engine command
//! (`tools.setBrush`, `layer.setProps` with every target layer), so it is one undo step.
//!
//! The keys arrive as fired `BlendStep` and `BlendShortcut` commands; the framework's `BlendKeys` reads them over
//! the list of modes the target accepts.

use craft::ui::patterns::blend_keys::{BlendKeyEntry, BlendKeyPick, BlendKeys, BlendShortcut, BlendStep};
use craft::ui::runtime::commands::{CommandSet as _, Fired};
use photocraft_color::BlendMode;
use serde_json::json;

use crate::brush_mode_menu::Item;
use crate::state::Tool;
use crate::{PhotocraftApp, brush_mode_menu};

/// Photoshop's Healing Brush modes, less Replace (not implemented yet).
pub const HEALING_MODES: [BlendMode; 7] =
    [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen, BlendMode::Darken, BlendMode::Lighten, BlendMode::Color, BlendMode::Luminosity];

/// The mode a brush-based tool paints with: its brush's, or Normal for a Healing Brush whose brush
/// holds a mode its short menu lacks (the Brush's Overlay remembered from before it had a menu, a
/// preset with tool settings, `tools.setBrush`). The stroke, the dropdown and the keys share it.
pub fn brush_tool_mode(tool: Tool, mode: BlendMode) -> BlendMode {
    if tool == Tool::Healing && !HEALING_MODES.contains(&mode) { BlendMode::Normal } else { mode }
}

/// A painting tool's options-bar Mode menu in order, `None` being Clear (after Dissolve, on the
/// tools Photoshop offers it on). `None` for a tool without a blend mode control.
pub fn tool_modes(tool: Tool) -> Option<Vec<Option<BlendMode>>> {
    let layer = || BlendMode::LAYER_MODES.iter().copied().map(Some).collect::<Vec<_>>();
    Some(match tool {
        Tool::Brush | Tool::Pencil | Tool::PaintBucket => {
            let mut modes = layer();
            let at = modes.iter().position(|m| *m == Some(BlendMode::Dissolve)).map_or(modes.len(), |i| i + 1);
            modes.insert(at, None);
            modes
        }
        Tool::Gradient | Tool::CloneStamp | Tool::PatternStamp | Tool::HistoryBrush => layer(),
        Tool::Healing => HEALING_MODES.iter().copied().map(Some).collect(),
        _ => return None,
    })
}

/// Where the active tool's blend mode keys go.
enum Target {
    /// The tool's own Mode menu (`brush_mode_menu::items`).
    Tool,
    /// A painting tool with a blend mode in Photoshop but no Mode control here yet: the keys do
    /// nothing, and never reach the layer.
    Nothing,
    Layers,
    /// The ⇧⌥ letters for these modes set the active layer's mode while it is the only one
    /// selected; the other letters, several selected layers and ⇧+ / ⇧− do nothing (the Color
    /// Replacement tool, #11928: only these letters on one layer were measured).
    LayerLetters(&'static [BlendMode]),
}

fn target(tool: Tool) -> Target {
    if tool_modes(tool).is_some() {
        return Target::Tool;
    }
    match tool {
        Tool::SpotHealing | Tool::Blur | Tool::Sharpen | Tool::Smudge | Tool::ArtHistoryBrush => Target::Nothing,
        // Its ⇧⌥ letters set the active layer's mode and leave the tool's own Mode (#11928).
        Tool::ColorReplacement => Target::LayerLetters(&[BlendMode::Normal, BlendMode::Multiply, BlendMode::Hue, BlendMode::Luminosity]),
        // Photoshop's ⇧⌥S/M/H pick the Dodge and Burn Range and ⇧⌥D/S the Sponge's mode: never
        // the layer's.
        Tool::Dodge | Tool::Burn | Tool::Sponge => Target::Nothing,
        // No blend mode of their own (the Erasers, the Mixer Brush…): the layer's, as in Photoshop.
        _ => Target::Layers,
    }
}

#[cfg(test)]
/// The active tool's mode, `None` being Clear. A brush-based tool's lives in its own brush.
pub fn tool_mode(app: &mut PhotocraftApp) -> Option<BlendMode> {
    let o = &app.ui.tool_options;
    match app.ui.tool {
        Tool::Brush if o.brush_clear => None,
        Tool::Pencil if o.pencil_clear => None,
        Tool::PaintBucket if o.bucket_clear => None,
        Tool::PaintBucket => Some(o.bucket_blend_mode),
        Tool::Gradient => Some(o.gradient_blend_mode),
        _ => {
            crate::paint_mouse::sync_tool_brush(app);
            Some(brush_tool_mode(app.ui.tool, app.session.tools.brush.mode))
        }
    }
}

/// Sets the active tool's mode; `None` is Clear.
fn set_tool_mode(app: &mut PhotocraftApp, mode: Option<BlendMode>) {
    match app.ui.tool {
        Tool::Gradient => {
            if let Some(m) = mode {
                set_gradient_mode(app, m);
            }
            return;
        }
        Tool::PaintBucket => {
            app.ui.tool_options.bucket_clear = mode.is_none();
            if let Some(m) = mode {
                app.ui.tool_options.bucket_blend_mode = m;
            }
            return;
        }
        Tool::Brush => app.ui.tool_options.brush_clear = mode.is_none(),
        Tool::Pencil => app.ui.tool_options.pencil_clear = mode.is_none(),
        _ => {}
    }
    crate::paint_mouse::sync_tool_brush(app);
    // Behind is one of the painting modes in the reference application: picking any other mode
    // (Clear too) leaves it, in the same one command (#11896). With Behind off, as before.
    let mut brush = serde_json::Map::new();
    if let Some(m) = mode
        && let Ok(m) = serde_json::to_value(m)
    {
        brush.insert("mode".into(), m);
    }
    if app.session.tools.brush.behind {
        brush.insert("behind".into(), json!(false));
    }
    if !brush.is_empty() {
        let _ = app.run("tools.setBrush", json!({ "brush": brush }));
    }
}

/// Selects Behind on the Brush, Pencil or Pattern Stamp, through the menu's own engine call.
fn set_behind(app: &mut PhotocraftApp) {
    let tool = app.ui.tool;
    if app.session.active().is_none() {
        return;
    }
    let Some((id, params)) = brush_mode_menu::engine_call(app, tool, Item::Behind) else { return };
    crate::paint_mouse::sync_tool_brush(app);
    if app.run(id, params).is_ok() {
        match tool {
            Tool::Brush => app.ui.tool_options.brush_clear = false,
            Tool::Pencil => app.ui.tool_options.pencil_clear = false,
            _ => {}
        }
    }
}

pub(crate) fn set_gradient_mode(app: &mut PhotocraftApp, mode: BlendMode) {
    let before = app.ui.tool_options.clone();
    app.ui.tool_options.gradient_blend_mode = mode;
    // Live mode: a selected gradient fill layer follows, as with the dropdown.
    crate::gradient_ui::options_changed(app, &before);
}

/// Sets `mode` on every selected layer that can take it, in one undo step: the locked Background
/// has no blend mode, and only groups pass through.
fn set_layers_mode(app: &mut PhotocraftApp, mode: BlendMode) {
    let Some(st) = app.session.active() else { return };
    let ids: Vec<u64> = st
        .selected_layers()
        .into_iter()
        .filter_map(|id| st.doc.layer(id))
        .filter(|l| !crate::doc_props_ui::is_background(&st.doc, l) && (mode != BlendMode::PassThrough || l.is_group()) && l.blend != mode)
        .map(|l| l.id.0)
        .collect();
    if !ids.is_empty() {
        let _ = app.run("layer.setProps", json!({ "layers": ids, "blend": mode.label() }));
    }
}

/// The entries of the active tool's own Mode menu, in the menu's order; Behind and Clear only where the menu offers
/// them and the active layer takes them.
fn tool_entries(app: &PhotocraftApp) -> Vec<BlendKeyEntry<Item>> {
    brush_mode_menu::items(app.ui.tool)
        .into_iter()
        .filter(|item| matches!(item, Item::Blend(_) | Item::Behind | Item::Clear) && brush_mode_menu::enabled(app, *item))
        .map(|item| match item {
            Item::Blend(m) => crate::panels::blend_entry(item, m),
            Item::Behind => BlendKeyEntry::behind(item),
            _ => BlendKeyEntry::clear(item),
        })
        .collect()
}

/// The entries of the layer modes the active layer takes: Pass Through first for a group.
fn layer_entries(is_group: bool) -> Vec<BlendKeyEntry<BlendMode>> {
    std::iter::once(BlendMode::PassThrough).filter(|_| is_group).chain(BlendMode::LAYER_MODES).map(|m| crate::panels::blend_entry(m, m)).collect()
}

/// Applies a fired `BlendStep` or `BlendShortcut` command to the active tool's target. Returns whether the command is
/// one of these: it is taken even when it picks nothing, so its letter never reaches the single-letter tool keys.
pub(crate) fn fired(app: &mut PhotocraftApp, f: &Fired) -> bool {
    let step = BlendStep::from_ref(&f.command).is_some();
    if !step && BlendShortcut::from_ref(&f.command).is_none() {
        return false;
    }
    match target(app.ui.tool) {
        Target::Tool => {
            // The brush the keys read is the tool's own.
            if !matches!(app.ui.tool, Tool::Gradient | Tool::PaintBucket) {
                crate::paint_mouse::sync_tool_brush(app);
            }
            let entries = tool_entries(app);
            let current = brush_mode_menu::current(app, app.ui.tool);
            let keys = BlendKeys::new(&entries);
            let pick = match &current {
                Some(c) => keys.current(c),
                None => keys,
            }
            .fired(f);
            match pick.map(BlendKeyPick::into_key) {
                Some(Item::Blend(m)) => set_tool_mode(app, Some(m)),
                Some(Item::Clear) => set_tool_mode(app, None),
                Some(Item::Behind) => set_behind(app),
                _ => {}
            }
        }
        Target::Nothing => {}
        Target::Layers => {
            let Some(st) = app.session.active() else { return true };
            let Some(layer) = st.active_layer.and_then(|id| st.doc.layer(id)) else { return true };
            // The locked Background has no blend mode of its own to step from.
            if step && crate::doc_props_ui::is_background(&st.doc, layer) {
                return true;
            }
            let entries = layer_entries(layer.is_group());
            let current = layer.blend;
            if let Some(pick) = BlendKeys::new(&entries).current(&current).fired(f) {
                set_layers_mode(app, pick.into_key());
            }
        }
        Target::LayerLetters(modes) => {
            // Only one selected layer was measured; several keep today's (nothing).
            let one = app.session.active().is_some_and(|st| st.selected_layers().len() == 1);
            let entries: Vec<_> = modes.iter().map(|m| crate::panels::blend_entry(*m, *m)).collect();
            if let Some(BlendKeyPick::Lettered(m)) = BlendKeys::new(&entries).fired(f)
                && one
            {
                set_layers_mode(app, m);
            }
        }
    }
    true
}

/// A key-press `Fired` for `command`, for the trials of the modules that read fired commands.
#[cfg(test)]
pub(crate) fn test_fired(command: craft::ui::runtime::commands::CommandRef) -> Fired {
    use craft::ui::runtime::commands::{FirePhase, FireSource};
    Fired::new(command, FireSource::Key, FirePhase::Press, 0.0)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../brush_behind_key_tests.rs"]
mod brush_behind_key_tests;

#[cfg(test)]
#[path = "../brush_blend_keys_behind_tests.rs"]
mod brush_blend_keys_behind_tests;
