//! What the dialog's shortcuts do. Which keys they are is the dialog's [`super::keymap::Keymap`]
//! (Camera Raw's table by default); this module only performs the actions it reports.
//!
//! The keys are read at the top of the dialog's frame, before its widgets, and never while a text
//! field has the keyboard (a slider's typed value, a mask's name): the keys type there instead.
//! A press that is a shortcut is consumed, and the application's own shortcuts don't run under
//! the dialog at all (`shortcuts::preflight_keys`), so `[` here is never the canvas brush's `[`.

use super::CameraRawDialog;
use super::controls::{remember_shortcut_edit, snapshot};
use super::keymap::Action;
use super::local::Draw;
use super::rail::Tool;

/// This frame's shortcuts. The classic panel has only Guided Upright's (⇧T and Delete); the
/// develop view (a raw the shared engine develops) has them all.
pub(super) fn handle(d: &mut CameraRawDialog, ctx: &egui::Context) {
    if ctx.text_edit_focused() {
        return;
    }
    let develop = d.develop.is_some();
    if develop {
        // the layout table and bracket fallback the app's own `[` `]` get (#3102, #3109): the
        // application's key handling, which applies them, doesn't run under the dialog
        crate::layout_keys::apply(ctx);
    }
    // Delete only when it has a guide or a local tool's selection to remove: otherwise it is the
    // point curve's (a selected point) and the widgets'
    let delete = guides_take_delete(d) || local_tool_live(d);
    // A only while the mask brush paints: elsewhere the key is left alone
    let brush = develop && d.tool == Tool::Masking && d.local.draw == Draw::Brush;
    let actions = d.keymap.take(ctx, |a| match a {
        Action::GuidedUpright => true,
        Action::DeleteSelected => delete,
        Action::AutoMask => brush,
        _ => develop,
    });
    for action in actions {
        perform(d, action);
    }
}

fn perform(d: &mut CameraRawDialog, action: Action) {
    match action {
        Action::Tool(t) => pick_tool(d, d.tool.after_picking(t)),
        Action::GuidedUpright => guided_upright(d),
        Action::Section(n) => section(d, n),
        Action::DeleteSelected => delete(d),
        Action::BrushSmaller | Action::BrushLarger | Action::FeatherLess | Action::FeatherMore => brush(d, action),
        // a tool setting, not an edit: saved as the brush's preference, not an undo step
        Action::AutoMask => d.local.brush.auto_mask = !d.local.brush.auto_mask,
    }
}

fn pick_tool(d: &mut CameraRawDialog, tool: Tool) {
    if d.develop.is_none() || d.tool == tool {
        return;
    }
    let was = d.tool;
    d.tool = tool;
    d.tool_changed(was);
}

/// Picks Guided Upright and its tool (a second press puts the tool down), one undo step; the
/// develop view goes to its Edit panel, where Geometry opens and scrolls into view.
fn guided_upright(d: &mut CameraRawDialog) {
    let was = snapshot(d);
    if crate::camera_raw_guides_ui::toggle(&mut d.guides, &mut d.params) {
        remember_shortcut_edit(d, was);
    }
    if d.guides.active {
        d.guides.reveal = true;
        pick_tool(d, Tool::Edit);
    }
}

/// ⌘N: the Edit panel's nth section (in the panel's own order) opened and scrolled to; again, when
/// it is open and its header in view, folded.
fn section(d: &mut CameraRawDialog, n: u8) {
    if d.develop.is_none() {
        return;
    }
    let sections = super::develop_panel::numbered_sections();
    let Some((id, _)) = usize::from(n).checked_sub(1).and_then(|i| sections.get(i)) else { return };
    let p = &mut d.develop_panel;
    if d.tool == Tool::Edit && p.is_open(id) && p.in_view(id) {
        p.fold(id);
    } else {
        p.reveal(id);
    }
    pick_tool(d, Tool::Edit);
}

/// Guided Upright's tool is up where its guides are shown (the classic panel, or the develop
/// view's Edit tool): Delete removes the selected guide.
fn guides_take_delete(d: &CameraRawDialog) -> bool {
    d.guides.active && (d.develop.is_none() || d.tool == Tool::Edit)
}

/// A local tool (Heal, Masking, Red Eye) is on the preview: Delete removes its selection.
fn local_tool_live(d: &CameraRawDialog) -> bool {
    d.develop.is_some() && d.tool != Tool::Edit && !d.show_before
}

/// Delete (or Backspace): the selected guide while Guided Upright's tool is up on the Edit panel,
/// else the local tool's selected mask, spot or red eye.
fn delete(d: &mut CameraRawDialog) {
    let was = snapshot(d);
    if guides_take_delete(d) {
        if crate::camera_raw_guides_ui::delete_selected(&mut d.guides, &mut d.params) {
            remember_shortcut_edit(d, was);
        }
        return;
    }
    if !local_tool_live(d) {
        return;
    }
    let Some(e) = d.develop.as_mut() else { return };
    if super::local::delete_selected(&mut d.local, d.tool, &mut e.settings) {
        d.params = e.camera_raw(&d.params);
        d.dirty = true;
        remember_shortcut_edit(d, was);
    }
}

/// A brush size after one press of `[` (`larger` false) or `]`, in its slider's units: a tenth of
/// the size, at least one unit, in `min..=max`.
pub(super) fn step_size(v: f64, larger: bool, min: f64, max: f64) -> f64 {
    let v = if v.is_finite() { v } else { min };
    let by = (v.abs() * 0.1).max(1.0);
    (if larger { v + by } else { v - by }).round().clamp(min, max)
}

/// A feather (0..100) after one press of ⇧`[` or ⇧`]`: a tenth of its range.
pub(super) fn step_feather(v: f64, more: bool) -> f64 {
    let v = if v.is_finite() { v } else { 50.0 };
    (if more { v + 10.0 } else { v - 10.0 }).round().clamp(0.0, 100.0)
}

/// `[` `]` and ⇧`[` ⇧`]` on the active brush: Masking's brush (while it paints), Heal's selected
/// spot or the next one, and Red Eye's selected eye (its pupil size; an eye has no feather). A
/// spot's or eye's change is an undo step; the brush's and the next spot's are tool settings.
fn brush(d: &mut CameraRawDialog, action: Action) {
    let (feather, up) = match action {
        Action::BrushSmaller => (false, false),
        Action::BrushLarger => (false, true),
        Action::FeatherLess => (true, false),
        _ => (true, true),
    };
    let was = snapshot(d);
    let Some(e) = d.develop.as_mut() else { return };
    let st = &mut d.local;
    let edited = match d.tool {
        Tool::Masking if st.draw == Draw::Brush => {
            // the panel's Size slider is 1..100 for a radius of 1/400 .. 1/4 of the long edge
            let b = &mut st.brush;
            if feather {
                b.feather = step_feather(b.feather, up);
            } else {
                b.size = step_size(b.size * 400.0, up, 1.0, 100.0) / 400.0;
            }
            false
        }
        Tool::Heal => {
            // Size is 1..250 for a radius of 1/1000 .. 1/4
            let spot = st.spot.and_then(|i| e.settings.spots.get_mut(i));
            let (size, fe) = spot.as_ref().map_or((st.heal.size, st.heal.feather), |sp| (sp.size, sp.feather));
            let (size, fe) = if feather { (size, step_feather(fe, up)) } else { (step_size(size * 1000.0, up, 1.0, 250.0) / 1000.0, fe) };
            (st.heal.size, st.heal.feather) = (size, fe);
            match spot {
                Some(sp) if (sp.size, sp.feather) != (size, fe) => {
                    (sp.size, sp.feather) = (size, fe);
                    true
                }
                _ => false,
            }
        }
        Tool::RedEye if !feather => {
            let Some(i) = st.eye.filter(|i| *i < e.settings.red_eye.len()) else { return };
            let id = format!("redEye.{i}.pupilSize");
            let Some(v) = craft_develop_model::controls::get(&e.settings, &id) else { return };
            let nv = step_size(v, up, 0.0, 100.0);
            nv != v && craft_develop_model::controls::set(&mut e.settings, &id, nv)
        }
        _ => false,
    };
    if edited {
        d.params = e.camera_raw(&d.params);
        d.dirty = true;
        remember_shortcut_edit(d, was);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[` `]` step a tenth of the size (at least a unit) and stay in the slider's range; the
    /// feather keys a tenth of 0..100.
    #[test]
    fn brush_steps_are_proportional_and_clamped() {
        assert_eq!(step_size(50.0, true, 1.0, 100.0), 55.0);
        assert_eq!(step_size(50.0, false, 1.0, 100.0), 45.0);
        assert_eq!(step_size(3.0, true, 1.0, 100.0), 4.0, "at least one unit");
        assert_eq!(step_size(1.0, false, 1.0, 100.0), 1.0, "the slider's minimum");
        assert_eq!(step_size(98.0, true, 1.0, 100.0), 100.0, "the slider's maximum");
        assert_eq!(step_size(f64::NAN, true, 1.0, 100.0), 2.0, "never a NaN");
        assert_eq!(step_feather(50.0, true), 60.0);
        assert_eq!(step_feather(5.0, false), 0.0);
        assert_eq!(step_feather(95.0, true), 100.0);
    }
}
