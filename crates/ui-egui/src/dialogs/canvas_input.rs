//! The picks a hosted dialog arms on the canvas: the table the dialog tool (`canvas/tools/dialog.rs`) reads.
//!
//! [`armed`] says which pick applies from the dialog host's stack, and one function per pick does what a press (or a
//! drag step) on the image does for it. All points are document points. Nothing here routes presses: the tool maps
//! the host's pointer calls onto these functions. The precise crosshair under Caps Lock is the host's
//! (`ToolContext::precise`), so [`cursor`] has no argument for it.
//!
//! Each pick runs on the pass that received the press or drag event: the dialog tool gets those events from the tool
//! host once, so a second pass in the same frame gets none and runs no pick. The functions read the stack's top dialog,
//! the dialog's own value and the point they are given.

use craft::behavior::{Cursor, CursorIcon, IconId, PaintedCursor};
use craft::color_ui::ui::ColorPickerDialog;
use craft::draw::ui::pattern::PatternFillDialog;
use craft::imaging::layer_style::EffectSpec;
use craft::imaging::ui::adjust::TonePicker;
use craft::imaging::ui::color_range::HostedColorRange;
use craft::imaging::ui::layer_style::HostedLayerStyle;
use craft::keymap::Mods;
use craft::theme::Srgba;
use craft::ui::dialog::{DialogId, StackDialog as _};
use craft::ui::patterns::form_dialogs::HostedParamDialog;

use crate::PhotocraftApp;

/// A pick a hosted dialog (or the Properties panel) arms on the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DialogPick {
    /// A Levels or Curves point picker, or the Properties panel's armed one.
    TonePicker(TonePicker),
    /// A ⌘-click on the image with Curves open adds a point.
    CurvesPoint,
    /// The Color Picker's eyedropper.
    Color,
    /// A parameter dialog's eyedropper toggle.
    ParamDropper,
    /// Color Range's sampling.
    ColorRange,
    /// A drag on the image moves the selected shadow's angle and distance.
    ShadowDrag,
    /// A drag on the image moves the selected Gradient Overlay's offset.
    GradientDrag,
    /// A drag on the image moves the Pattern Fill phase (`pattern_fill_ui`'s gesture).
    PatternPhase,
    /// A click on the image moves a filter preview's view to the point.
    Locator,
}

/// The pipette's tip, at the bottom left of its 24-unit icon box.
const PIPETTE_HOTSPOT: egui::Vec2 = egui::vec2(2.0, 22.0);

/// The pick the stack's top dialog arms now, or the Properties panel's armed picker when no dialog is open.
///
/// Reads the host's stack only. A dialog below the top never arms a pick: the dialog over it takes the input.
pub(crate) fn armed(app: &PhotocraftApp) -> Option<DialogPick> {
    let dialogs = &app.framework.dialogs;
    let Some(top) = dialogs.stack().top() else {
        return super::layer_picker::armed(app).map(|(_, picker)| DialogPick::TonePicker(picker));
    };
    if let Some(open) = super::adjust_dialog::open_value(app).filter(|open| open.id == top) {
        let dialog = super::adjust_dialog::hosted(app, top)?;
        if let Some(picker) = dialog.picker() {
            return super::adjust_dialog::picker_applies(app, open, picker).then_some(DialogPick::TonePicker(picker));
        }
        // ⌘/Ctrl held on a press: the Curves dialog's own point. The modifier is the pass's state, not an event.
        return (open.kind == "curves" && app.ctx().input(|i| i.modifiers.command)).then_some(DialogPick::CurvesPoint);
    }
    if dialogs.get_by_id::<HostedColorRange>(top).is_some() {
        return Some(DialogPick::ColorRange);
    }
    if let Some(style) = dialogs.get_by_id::<HostedLayerStyle>(top) {
        let draft = style.draft();
        if draft.drags_shadow() {
            return Some(DialogPick::ShadowDrag);
        }
        return draft.drags_gradient().then_some(DialogPick::GradientDrag);
    }
    if let Some((dropper, _)) = app.dialog_dropper.as_ref().filter(|(dropper, _)| *dropper == top) {
        if dialogs.get_by_id::<ColorPickerDialog>(*dropper).is_some() {
            return Some(DialogPick::Color);
        }
        if dialogs.get_by_id::<HostedParamDialog>(*dropper).is_some() {
            return Some(DialogPick::ParamDropper);
        }
    }
    // PLAN: `pattern_fill_ui::drag_target(app)` takes the hosted dialog (dialogs-w60), not the base's field map.
    // The phase drag belongs to the Pattern Fill dialog while it is the top: a dialog opened above it takes the canvas.
    if dialogs.get_by_id::<PatternFillDialog>(top).is_some() && super::pattern_fill_ui::drag_target(app).is_some() {
        return Some(DialogPick::PatternPhase);
    }
    super::filter_preview_ui::locator(app).is_some().then_some(DialogPick::Locator)
}

/// Records that the dialog `dialog` asked for a screen sample under the request `key`: the next presses on the image
/// answer it, while `dialog` is the stack's top. The takers of the eyedropper requests call this (`take_eyedropper`
/// hands a request once, so something has to remember it).
pub(crate) fn arm_dropper(app: &mut PhotocraftApp, dialog: DialogId, key: String) {
    app.dialog_dropper = Some((dialog, key));
}

/// Forgets the armed dropper (the dialog closed or its toggle went off).
pub(crate) fn disarm_dropper(app: &mut PhotocraftApp) {
    app.dialog_dropper = None;
}

/// Whether `pick` keeps sampling while the primary button stays held and the pointer moves: the one rule of the
/// dialogs' eyedroppers (the Levels and Curves picker, the Color Picker's, a parameter dialog's, Color Range's), Add and
/// Subtract samples included: a held drag keeps adding or subtracting. The Properties panel's armed picker, a Curves
/// point and the preview locator act on the press alone.
pub(crate) fn follows_drag(app: &PhotocraftApp, pick: DialogPick) -> bool {
    match pick {
        DialogPick::TonePicker(_) => app.framework.dialogs.stack().top().is_some(),
        DialogPick::Color | DialogPick::ParamDropper | DialogPick::ColorRange => true,
        _ => false,
    }
}

/// A drag step at `at` of a pick that [`follows_drag`]: the pick runs again on the press's modifiers.
pub(crate) fn resample(app: &mut PhotocraftApp, pick: DialogPick, at: [f64; 2], mods: Mods) {
    match pick {
        DialogPick::TonePicker(_) => {
            pick_tone(app, at);
        }
        DialogPick::Color => {
            pick_color(app, at);
        }
        DialogPick::ParamDropper => {
            pick_dropper(app, at, mods);
        }
        DialogPick::ColorRange => {
            pick_color_range(app, at, mods);
        }
        _ => {}
    }
}

/// A press at `at` with the armed Levels or Curves point picker (the dialog's, or the Properties panel's when no dialog
/// is open). Consumes the press even when nothing changes: a refused mode or a point off the image.
pub(crate) fn pick_tone(app: &mut PhotocraftApp, at: [f64; 2]) -> bool {
    if app.framework.dialogs.stack().top().is_some() { super::adjust_dialog::pick(app, at) } else { super::layer_picker::sample_at(app, at[0], at[1]) }
}

/// A ⌘-click at `at` on the image with Curves open: a point on the shown curve (one per colour channel with
/// `every_channel`).
pub(crate) fn add_curve_point(app: &mut PhotocraftApp, at: [f64; 2], every_channel: bool) -> bool {
    super::adjust_dialog::add_point(app, at, every_channel)
}

/// The colour of the image at `at` for the armed dropper of the top dialog. `None` off the image or over transparency.
fn sampled(app: &mut PhotocraftApp, at: [f64; 2]) -> Option<Srgba> {
    crate::canvas::composite_color(app, at[0], at[1]).map(Srgba::from_rgb_array)
}

/// A press at `at` with the Color Picker's eyedropper armed: the colour of the image answers the request. A point off
/// the image changes nothing but is still the dialog's press.
pub(crate) fn pick_color(app: &mut PhotocraftApp, at: [f64; 2]) -> bool {
    let Some((id, key)) = app.dialog_dropper.clone() else { return false };
    let Some(color) = sampled(app, at) else { return true };
    let Some(picker) = app.framework.dialogs.get_mut_by_id::<ColorPickerDialog>(id) else { return false };
    picker.requests().answer_eyedropper(key, color);
    true
}

/// A press at `at` with a parameter dialog's eyedropper armed. Replace Color keeps its click order in the dialog
/// (`mods` add or subtract a sample), every other dialog takes the colour under its request key.
pub(crate) fn pick_dropper(app: &mut PhotocraftApp, at: [f64; 2], mods: Mods) -> bool {
    let Some((id, key)) = app.dialog_dropper.clone() else { return false };
    let replace_color = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).and_then(|dialog| dialog.key()) == Some(crate::filter_dialog::REPLACE_COLOR);
    if replace_color {
        return crate::filter_dialog::replace_color_sample(app, id, at, mods);
    }
    let Some(color) = sampled(app, at) else { return true };
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) else { return false };
    dialog.answer_eyedropper(key, color);
    true
}

/// A press at `at` with Color Range open: samples the image under the dialog's sample tool and `mods`. `false` when
/// the select takes no samples or the point is off the document.
pub(crate) fn pick_color_range(app: &mut PhotocraftApp, at: [f64; 2], mods: Mods) -> bool {
    app.framework.dialogs.get_mut::<HostedColorRange>().is_some_and(|dialog| dialog.sample_canvas(at, mods))
}

/// A drag step with a shadow page selected in Layer Style. `total` is the travel since the press, which the press itself
/// passes as zero; the draft starts its drag from the shown angle and distance on that zero.
// PLAN: the draft takes `start: bool`; the dialog tool (canvas-widgets-w22) calls with a zero `total` on the press, so a zero travel is the start here.
pub(crate) fn drag_shadow(app: &mut PhotocraftApp, _start: [f64; 2], total: [f64; 2]) -> bool {
    let Some(dialog) = app.framework.dialogs.get_mut::<HostedLayerStyle>() else { return false };
    let draft = dialog.draft_mut();
    let Some(kind) = draft.selected_kind().filter(|_| draft.drags_shadow()) else { return false };
    let defaults = EffectSpec::photoshop(kind).defaults;
    draft.drag_shadow(&defaults, total == [0.0, 0.0], total);
    true
}

/// A drag step with a Gradient Overlay page selected in Layer Style: the offset moves by the travel over the layer's size.
/// Nothing for an empty or removed layer.
pub(crate) fn drag_gradient(app: &mut PhotocraftApp, _start: [f64; 2], total: [f64; 2]) -> bool {
    let Some(layer) = super::layer_style::open_layer(app) else { return false };
    let Some(size) = super::layer_style::gradient_layer_bounds(&app.session, layer) else { return false };
    let Some(dialog) = app.framework.dialogs.get_mut::<HostedLayerStyle>() else { return false };
    let draft = dialog.draft_mut();
    if !draft.drags_gradient() {
        return false;
    }
    draft.drag_gradient(total == [0.0, 0.0], total, size);
    true
}

/// A press at `at` with a filter preview's locator armed: the preview's view centres on the point.
pub(crate) fn locate(app: &mut PhotocraftApp, at: [f64; 2]) -> bool {
    super::filter_preview_ui::centre_on(app, at);
    true
}

/// The end of a gesture of `pick`: a Layer Style drag ends. The other picks have nothing to end.
pub(crate) fn release(app: &mut PhotocraftApp, pick: DialogPick) {
    if matches!(pick, DialogPick::ShadowDrag | DialogPick::GradientDrag)
        && let Some(dialog) = app.framework.dialogs.get_mut::<HostedLayerStyle>()
    {
        dialog.draft_mut().end_drag();
    }
}

/// The modifiers the pass holds that change what a Color Range press does.
fn held(app: &PhotocraftApp) -> Mods {
    let held = app.ctx().input(|i| i.modifiers);
    Mods { shift: held.shift, alt: held.alt, ..Mods::default() }
}

/// The cursor over the image while `pick` is armed: the pipette for the samplers (Color Range's with the badge of the
/// tool the held modifiers select), the move cursor for the drags, the locator mark for the locator.
pub(crate) fn cursor(app: &PhotocraftApp, pick: DialogPick) -> Cursor {
    let pipette = |badge| Cursor::Painted(PaintedCursor::tool(IconId::new("eyedropper"), PIPETTE_HOTSPOT, badge));
    match pick {
        DialogPick::TonePicker(_) | DialogPick::Color | DialogPick::ParamDropper => pipette(None),
        DialogPick::ColorRange => {
            let badge = app.framework.dialogs.get::<HostedColorRange>().and_then(|dialog| dialog.tool().with_mods(held(app)).badge());
            pipette(badge)
        }
        DialogPick::CurvesPoint => pipette(None),
        DialogPick::ShadowDrag | DialogPick::GradientDrag | DialogPick::PatternPhase => Cursor::System(CursorIcon::Move),
        DialogPick::Locator => Cursor::Painted(PaintedCursor::Locator),
    }
}
