//! What a mouse button does with a painting tool. The left button paints. The right button never
//! paints with the tool unless it is told to: with a painting tool it opens the Brush Preset picker
//! at the pointer (Photoshop), or, with Preferences › Tools › Right-click with painting tools set
//! to Erase, a right drag with the Brush erases with the current brush (Krita, Paint). Each right
//! stroke is one `paint.stroke` with `"erase": true`, so one undo step, with the pen pressure and
//! tilt of a normal stroke.
//!
//! [`secondary_takes`] says what a secondary press does for the paint tool; [`context_popup`] is the
//! one router that says which popup a press the tool refused opens, for every canvas tool.

use craft::keymap::Mods;
use photocraft_engine::BrushSettings;
use photocraft_engine::prefs::RightClickPaint;

use crate::PhotocraftApp;
use crate::canvas_tool_menu::MenuKind;
use crate::state::Tool;

/// The brush is a tool option, kept per tool like Photoshop's options bar: switching from the
/// Brush to the Eraser saves the session brush for the old tool and loads the new tool's, so each
/// keeps its own size, hardness, mode, opacity, dynamics and smoothing. A tool seen for the
/// first time takes what the brush already has, at Photoshop's 10 % smoothing. Other tools leave
/// the brush alone.
///
/// The foreground and background colours, the erase flag and the stroke seed belong to the
/// session rather than to the tool, so a swap never carries them between tools: picking a colour
/// or painting with the Eraser works with any tool's brush.
pub fn sync_tool_brush(app: &mut PhotocraftApp) {
    let tool = app.ui.tool;
    if !has_brush_picker(tool) || app.ui.brush_tool == tool {
        return;
    }
    // The live brush is the outgoing tool's, so save it there before the new one takes over: any
    // edit it made since it was last active is in it, so its copy never goes stale.
    let current = (*app.session.tools.brush).clone();
    save_brush(app, app.ui.brush_tool, current.clone());
    *app.session.tools.brush = match app.ui.tool_brushes.iter().find(|(t, _)| *t == tool) {
        Some((_, b)) => {
            // A brush remembered from the previous run holds its stored tips as references:
            // load their full bitmaps (a missing one keeps painting with its preview).
            let mut b = session_fields(&current, b);
            let _ = app.session.load_brush_tips(&mut b);
            b
        }
        // A tool seen for the first time takes what the brush has, at Photoshop's smoothing, but
        // in Normal mode: each tool's Mode starts at Normal, so a Multiply Brush doesn't make the
        // first Pattern Stamp stroke multiply (or give Healing a mode its menu doesn't have).
        // The Pattern Stamp also starts at full opacity and flow (#12616): those fields are its own.
        // The Background Eraser, the Healing Brush, the Burn, Dodge and Sponge tools, the Clone
        // Stamp, the Smudge tool and the Art History Brush start with their own measured tips
        // instead, as the whole brush Reset Tool gives them (#11911, #12099, #12671, #12684,
        // #13029, #12601, #12964, #12631); the Art History Brush's is at full opacity (#10263).
        // The Art History Brush's Angle and pressure toggles are its own bar controls, so they
        // start at 0° and off too, whatever the outgoing brush has (#12629).
        None if matches!(
            tool,
            Tool::BackgroundEraser | Tool::Healing | Tool::Burn | Tool::Dodge | Tool::Sponge | Tool::CloneStamp | Tool::Smudge | Tool::ArtHistoryBrush
        ) =>
        {
            fresh_brush(tool, &current)
        }
        // The Pencil starts with its own 1 px hard round (#12200) regardless of the Brush, keeping
        // the outgoing opacity.
        None if tool == Tool::Pencil => BrushSettings { opacity: current.opacity, ..fresh_brush(tool, &current) },
        // The Eraser starts with its Reset Tool tip and no smoothing (#11936), keeping the outgoing
        // opacity and flow as before (a first use's are not measured).
        None if tool == Tool::Eraser => BrushSettings { opacity: current.opacity, flow: current.flow, ..fresh_brush(tool, &current) },
        None => {
            let fresh = BrushSettings::default();
            let (opacity, flow) = match tool {
                Tool::PatternStamp => (fresh.opacity, fresh.flow),
                _ => (current.opacity, current.flow),
            };
            // The Remove tool starts at its measured 19 px; the rest is the current brush (#12548).
            let size = if tool == Tool::Remove { REMOVE_TOOL_SIZE } else { current.size };
            BrushSettings { smoothing: first_smoothing(), mode: photocraft_color::BlendMode::Normal, opacity, flow, size, ..current }
        }
    };
    // The picked preset was the outgoing tool's brush: forget it, so Update Brush can't overwrite
    // that preset with this tool's brush.
    app.session.tools.current_preset = None;
    app.session.tools.brush_changed();
    app.ui.brush_tool = tool;
}

/// The brush `tool` had when it was last active.
fn save_brush(app: &mut PhotocraftApp, tool: Tool, brush: BrushSettings) {
    match app.ui.tool_brushes.iter_mut().find(|(t, _)| *t == tool) {
        Some(e) => e.1 = brush,
        None => app.ui.tool_brushes.push((tool, brush)),
    }
}

/// The session's own brush fields, which a tool swap never carries: the colours, the erase flag
/// and the stroke seed (a stroke's replay seed is the command's, not the tool's).
fn session_fields(current: &BrushSettings, brush: &BrushSettings) -> BrushSettings {
    BrushSettings { color: current.color, background: current.background, erase: current.erase, seed: current.seed, ..brush.clone() }
}

/// The Brush tool's tip after Reset Tool, as the reference measures it (#11810): size, hardness
/// and spacing of a 13 px soft round at 25 % spacing.
const BRUSH_TOOL_TIP: (f32, f32, f32) = (13.0, 0.0, 0.25);
/// The Mixer Brush's tip after Reset Tool (#11945): a 35 px soft round at 25 % spacing.
const MIXER_BRUSH_TIP: (f32, f32, f32) = (35.0, 0.0, 0.25);
/// The Eraser's tip after Reset Tool (#11936): a 13 px hard round at 25 % spacing, with no
/// smoothing.
const ERASER_TOOL_TIP: (f32, f32, f32) = (13.0, 1.0, 0.25);
/// The Background Eraser's tip after Reset Tool (#11911): a 13 px hard round at 25 % spacing.
const BACKGROUND_ERASER_TIP: (f32, f32, f32) = (13.0, 1.0, 0.25);
/// The Healing Brush's tip after Reset Tool (#12099): a 19 px hard round at 25 % spacing.
const HEALING_BRUSH_TIP: (f32, f32, f32) = (19.0, 1.0, 0.25);
/// The Pencil tool's starting and Reset Tool tip, as the reference measures it (#12200): size,
/// hardness and spacing of a 1 px hard round at 25 % spacing.
const PENCIL_TOOL_TIP: (f32, f32, f32) = (1.0, 1.0, 0.25);
/// The Burn, Dodge and Sponge tools' tip after Reset Tool (#12671, #12684, #13029): a 65 px soft
/// round at 25 % spacing.
const TONING_TOOL_TIP: (f32, f32, f32) = (65.0, 0.0, 0.25);
/// The Clone Stamp's tip after Reset Tool (#12601): a 21 px soft round at 25 % spacing.
const CLONE_STAMP_TIP: (f32, f32, f32) = (21.0, 0.0, 0.25);
/// The Art History Brush's tip after Reset Tool (#12631): a 21 px soft round at 25 % spacing.
const ART_HISTORY_BRUSH_TIP: (f32, f32, f32) = (21.0, 0.0, 0.25);
/// The Remove tool's size after Reset Tool (#12548): 19 px. Its hardness and spacing are not
/// measured, so they stay the default brush's.
const REMOVE_TOOL_SIZE: f32 = 19.0;
/// The Smudge tool's tip after Reset Tool (#12964): a 13 px soft round at 25 % spacing.
const SMUDGE_TOOL_TIP: (f32, f32, f32) = (13.0, 0.0, 0.25);

/// `tool`'s brush as it starts out (Reset Tool, #2825): the default brush at Photoshop's
/// smoothing, with the session's own fields of `current`. The Brush tool's tip is
/// [`BRUSH_TOOL_TIP`], the Mixer Brush's [`MIXER_BRUSH_TIP`] and measured mixer settings, the
/// Eraser's [`ERASER_TOOL_TIP`] with no smoothing, the Background Eraser's [`BACKGROUND_ERASER_TIP`], the Healing Brush's
/// [`HEALING_BRUSH_TIP`], the Burn, Dodge and Sponge tools' [`TONING_TOOL_TIP`], the Clone Stamp's
/// [`CLONE_STAMP_TIP`] and the Art History Brush's [`ART_HISTORY_BRUSH_TIP`], both with no
/// smoothing, the Remove tool's size [`REMOVE_TOOL_SIZE`], the Smudge
/// tool's [`SMUDGE_TOOL_TIP`] and the Pencil's [`PENCIL_TOOL_TIP`]; the other tools keep the
/// default tip.
pub(crate) fn fresh_brush(tool: Tool, current: &BrushSettings) -> BrushSettings {
    let mut brush = BrushSettings { smoothing: first_smoothing(), ..BrushSettings::default() };
    match tool {
        Tool::Brush => (brush.size, brush.hardness, brush.spacing) = BRUSH_TOOL_TIP,
        Tool::MixerBrush => {
            (brush.size, brush.hardness, brush.spacing) = MIXER_BRUSH_TIP;
            brush.mixer.wet = 0.8;
            brush.mixer.load = 0.75;
            brush.mixer.mix = 0.9;
            brush.mixer.flow = 1.0;
            brush.mixer.load_after_stroke = true;
            brush.mixer.clean_after_stroke = true;
            brush.mixer.sample_all_layers = false;
        }
        Tool::Eraser => {
            (brush.size, brush.hardness, brush.spacing) = ERASER_TOOL_TIP;
            brush.smoothing.amount = 0.0;
        }
        Tool::BackgroundEraser => (brush.size, brush.hardness, brush.spacing) = BACKGROUND_ERASER_TIP,
        Tool::Healing => (brush.size, brush.hardness, brush.spacing) = HEALING_BRUSH_TIP,
        Tool::Pencil => (brush.size, brush.hardness, brush.spacing) = PENCIL_TOOL_TIP,
        Tool::Burn | Tool::Dodge | Tool::Sponge => (brush.size, brush.hardness, brush.spacing) = TONING_TOOL_TIP,
        Tool::Smudge => (brush.size, brush.hardness, brush.spacing) = SMUDGE_TOOL_TIP,
        Tool::CloneStamp => {
            (brush.size, brush.hardness, brush.spacing) = CLONE_STAMP_TIP;
            brush.smoothing.amount = 0.0;
        }
        Tool::ArtHistoryBrush => {
            (brush.size, brush.hardness, brush.spacing) = ART_HISTORY_BRUSH_TIP;
            brush.smoothing.amount = 0.0;
        }
        Tool::Remove => brush.size = REMOVE_TOOL_SIZE,
        _ => {}
    }
    session_fields(current, &brush)
}

/// Photoshop's 10 % stroke smoothing, what a tool's brush starts at.
fn first_smoothing() -> photocraft_engine::paint::brush::Smoothing {
    photocraft_engine::paint::brush::Smoothing { amount: 0.1, ..Default::default() }
}

/// Does a stroke with `tool` starting now paint in Photoshop's Clear mode, erasing with its own
/// brush (#3439)? Clear chosen in its Mode dropdown, or `~` held (`hold_keys::erase_key`).
pub fn clears(app: &PhotocraftApp, tool: Tool) -> bool {
    let chosen = match tool {
        Tool::Brush => app.ui.tool_options.brush_clear,
        Tool::Pencil => app.ui.tool_options.pencil_clear,
        _ => false,
    };
    crate::hold_keys::erases_while_held(tool) && (chosen || app.held_erase)
}

/// Tools with the options-bar brush chip, which opens the Brush Preset picker.
pub fn has_brush_picker(tool: Tool) -> bool {
    tool.is_brushlike() || tool == Tool::QuickSelection
}

/// Tools whose canvas right-click opens the Brush Preset picker: the painting tools. The Quick
/// Selection tool has the picker too, but its right-click opens the selection menu (#7268).
pub fn right_click_picker(tool: Tool) -> bool {
    has_brush_picker(tool) && !crate::canvas_tool_menu::applies(tool)
}

/// Does a right-button drag with `tool` erase (rather than open the picker)?
pub fn right_erases(app: &PhotocraftApp, tool: Tool) -> bool {
    matches!(tool, Tool::Brush | Tool::Eraser) && app.session.prefs().tools.right_click_with_painting_tools == RightClickPaint::Erase
}

/// What a secondary press does for the paint tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SecondaryUse {
    /// A right stroke that erases with the current brush (the Erase preference).
    Erase,
    /// The Alt+right brush resize drag.
    Resize,
    /// Shift+Alt+right: Photoshop's HUD Color Picker, which stays up until the button is released.
    Hud,
    /// The press is not the paint tool's: it reaches the canvas menu as a context press.
    None,
}

/// What a secondary press of `tool` with `mods` held (the press's, latched modifiers included)
/// does for the paint tool. The order is the rule's: a brush mode menu press and a layer list press
/// (Cmd, or the Move tool) are menus, so neither erases nor resizes; a resize beats the erase; the
/// HUD is the Shift+Alt press of a tool whose right click is the picker, unless the right button
/// erases.
pub(crate) fn secondary_takes(app: &PhotocraftApp, tool: Tool, mods: Mods) -> SecondaryUse {
    // The press has no document point here: a plain bucket press over the pasteboard is no paint
    // press either way (`context_popup` tells the two menus apart).
    if crate::brush_mode_menu::opens_on(app, tool, mods, None) {
        return SecondaryUse::None;
    }
    let layer_menu = crate::layer_pick_ui::is_gesture(tool, mods);
    if crate::brush_resize::PaintResize::takes(tool, craft::behavior::PointerButton::Secondary, mods) {
        return SecondaryUse::Resize;
    }
    // Cmd beats Erase: the layer list is `layer_pick_ui::is_gesture`'s.
    let erase = right_erases(app, tool) && !layer_menu;
    if erase {
        return SecondaryUse::Erase;
    }
    if right_click_picker(tool) && !layer_menu && mods.shift && mods.alt {
        return SecondaryUse::Hud;
    }
    SecondaryUse::None
}

/// The popup a context press opens.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ContextPopup {
    /// A canvas menu of this kind.
    Menu(MenuKind),
    /// The Brush Preset picker.
    BrushPicker,
}

/// The popup a context press (a press its tool refused, or a press with no tool use) of `tool` with
/// `mods` held at document point `doc` opens, in the order the canvas menus have: the brush mode
/// menu; the layer list of Cmd or the Move tool (the pasteboard colour menu off the image without
/// Cmd); nothing over a Magnetic Lasso border being drawn; a pending crop's menu, also outside the
/// image; the pasteboard menu; the view menu of the Zoom and Hand tools; the Brush Preset picker of
/// the painting tools; and the tool's own menu, the type edit menu or the type layer menu. Guide
/// menus come from the guides' own output and transform menus from the transform box.
pub(crate) fn context_popup(app: &mut PhotocraftApp, tool: Tool, mods: Mods, doc: [f64; 2]) -> Option<ContextPopup> {
    use crate::canvas_tool_menu as menu;
    if crate::brush_mode_menu::opens_on(app, tool, mods, Some(doc)) {
        return Some(ContextPopup::Menu(MenuKind::BrushMode { plain: !crate::brush_mode_menu::is_gesture(tool, mods) }));
    }
    if crate::layer_pick_ui::is_gesture(tool, mods) {
        let layers = crate::layer_pick_ui::layers_at(app, doc[0], doc[1]);
        if !layers.is_empty() {
            return Some(ContextPopup::Menu(MenuKind::Layers { layers }));
        }
        return (!mods.cmd && menu::pasteboard_at(app, tool, doc)).then_some(ContextPopup::Menu(MenuKind::Pasteboard));
    }
    if menu::magnetic_drawing(app) {
        return None;
    }
    if tool == Tool::Crop && crate::crop_ui::pending(app) {
        let presets = crate::crop_presets::user_presets(app).into_iter().map(|preset| preset.name).collect();
        return Some(ContextPopup::Menu(MenuKind::Crop { presets }));
    }
    if menu::pasteboard_at(app, tool, doc) {
        return Some(ContextPopup::Menu(MenuKind::Pasteboard));
    }
    if menu::is_view_tool(tool) {
        return Some(ContextPopup::Menu(MenuKind::View));
    }
    // A waiting Remove stroke takes no right-click: nothing opens over its tint (#12551).
    if right_click_picker(tool) && !crate::retouch_ui::remove_pending_live(app) {
        return Some(ContextPopup::BrushPicker);
    }
    if menu::editing_text(app, tool) {
        return Some(ContextPopup::Menu(MenuKind::TypeEdit { spelling: None }));
    }
    if menu::type_layer_at(app, tool, doc).is_some() {
        return Some(ContextPopup::Menu(MenuKind::TypeLayer { shape: menu::TypeShape::Other }));
    }
    menu::applies(tool).then_some(ContextPopup::Menu(MenuKind::Tool))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../pencil_reset_defaults_tests.rs"]
mod pencil_reset_defaults_tests;

#[cfg(test)]
#[path = "remove_right_click_tests.rs"]
mod remove_right_click_tests;
