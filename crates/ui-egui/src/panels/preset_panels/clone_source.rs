//! The Clone Source panel: five source slots, the slot's offset, scale, rotation and flips, and the overlay options.
//! Every edit is a `cloneSource.*` engine command.

use craft::behavior::{EditEvent, GestureId, IconId, Mixed};
use craft::ui::buttons::{Button, IconButton, ToggleButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Outcome as _, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::form::{Field, FieldGroup, SliderField};
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::text::{LinkedFields, NumberField};
use photocraft_engine::presets::PresetsExt;
use serde_json::{Value, json};

use super::run;
use crate::PhotocraftApp;

/// Runs `command`; a value edit of one gesture shares one undo step through the gesture's id.
fn edit(app: &mut PhotocraftApp, command: &str, mut params: Value, gesture: Option<GestureId>) {
    if let Some(id) = gesture {
        params["coalesce"] = json!(format!("clone-source-{}", id.0));
    }
    run(app, command, params);
}

/// A number field of `value`: the value an edit landed on with its gesture, `None` when nothing changed. With
/// `name`, the field sits in a `Field` labelled `label` and keeps `name` as its own accessible name, so the label
/// describes it.
fn number(
    ui: &mut egui::Ui,
    label: &str,
    name: Option<&str>,
    value: f64,
    range: std::ops::RangeInclusive<f64>,
    unit: UnitKind,
) -> Option<(f64, Option<GestureId>)> {
    let mut shown = Some(value);
    let out = match name {
        Some(name) => {
            Field::new(label.to_owned())
                .show(ui, |ui, _| NumberField::new(name.to_owned(), Bind::bound(&mut shown)).keeps_name(true).range(range.clone()).unit_kind(unit).show(ui))
                .inner?
        }
        None => NumberField::new(label.to_owned(), Bind::bound(&mut shown)).range(range).unit_kind(unit).show(ui),
    };
    let landed = (*out.events.landed()?)?;
    ((landed - value).abs() > 1e-9).then(|| (landed, out.events.last().and_then(EditEvent::id)))
}

/// A toggle button of `value`: the new state when a click changed it.
fn toggle(ui: &mut egui::Ui, label: &str, value: bool) -> Option<bool> {
    let mut on = value;
    let out = ToggleButton::new(label.to_owned(), Bind::bound(&mut on)).show(ui);
    out.events.landed().copied().filter(|&now| now != value)
}

/// The `cloneSource` registry body.
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let cs = app.session.presets().clone.clone();
    let slot = cs.active().clone();
    let mut select = None;
    Row::new().show(ui, |ui, _| {
        for i in 0..photocraft_engine::presets::clone_source::SLOTS {
            let mut on = cs.active == i;
            let out = ToggleButton::new(format!("{}", i + 1), Bind::bound(&mut on)).show(ui);
            if out.events.landed() == Some(&true) && cs.active != i {
                select = Some(i);
            }
        }
    });
    if let Some(index) = select {
        run(app, "cloneSource.select", json!({"index": index}));
    }
    // The document the source was set in (the active one for slots set before slots remembered it).
    let source_doc = match slot.document {
        Some(id) => slot.open_document().and_then(|_| app.session.documents().iter().find(|d| d.doc.id == id).map(|st| &**st)),
        None => app.session.active(),
    };
    let source_line = match (slot.source, source_doc) {
        (Some(_), None) => tl!("Source: document closed").to_string(),
        (Some(_), Some(d)) => match slot.layer.and_then(|l| d.doc.layer(photocraft_doc::LayerId(l))) {
            Some(l) => format!("Source: {} : {}", d.doc.name, l.name),
            None => "Source: set".to_string(),
        },
        _ => crate::i18n::fmt(tl!("Source: not set ({key}-click with Clone Stamp)"), &[("key", &crate::shortcuts::pretty("Alt"))]),
    };
    Text::new(source_line).show(ui);

    let mut set: Option<(Value, Option<GestureId>)> = None;
    // X/Y show the pairing at the last hover point (or the last committed one), in document pixels;
    // an edit moves exactly that pairing, before the first stroke or with Aligned off too (#13124).
    let off = super::clone_offset_readout(app).unwrap_or([0.0, 0.0]);
    let mut offset_edit = None;
    FieldGroup::new(tl!("Transform")).show(ui, |ui, _| {
        if slot.source.is_some() {
            if let Some((x, id)) = number(ui, "X", Some("Clone Source X Offset"), off[0], -30000.0..=30000.0, UnitKind::Pixels) {
                offset_edit = Some(([x, off[1]], id));
            }
            if let Some((y, id)) = number(ui, "Y", Some("Clone Source Y Offset"), off[1], -30000.0..=30000.0, UnitKind::Pixels) {
                offset_edit = Some(([off[0], y], id));
            }
        }
        // The W/H chain: with it on, a new W or H scales the other by the same factor.
        let mut size = Mixed::each([Some(slot.scale[0]), Some(slot.scale[1])]);
        let mut linked = slot.linked;
        let out = LinkedFields::new(tl!("Size"), Bind::bound(&mut size), Bind::bound(&mut linked)).captions(["W", "H"]).range(-1000.0..=1000.0).show(ui);
        if let Some(new) = out.edit.events.landed() {
            let gesture = out.edit.events.last().and_then(EditEvent::id);
            if let Some(w) = new[0].same().copied().flatten().filter(|w| (w - slot.scale[0]).abs() > 1e-9) {
                set = Some((json!({"width": w}), gesture));
            } else if let Some(h) = new[1].same().copied().flatten().filter(|h| (h - slot.scale[1]).abs() > 1e-9) {
                set = Some((json!({"height": h}), gesture));
            }
        }
        if let Some(&now) = out.link.landed().filter(|&&now| now != slot.linked) {
            set = Some((json!({"linked": now}), None));
        }
        if let Some((angle, id)) = number(ui, "A", None, slot.rotation, -180.0..=180.0, UnitKind::Degrees) {
            set = Some((json!({"rotation": angle}), id));
        }
        if let Some(now) = toggle(ui, tl!("Flip Horizontal"), slot.flip_h) {
            set = Some((json!({"flipH": now}), None));
        }
        if let Some(now) = toggle(ui, tl!("Flip Vertical"), slot.flip_v) {
            set = Some((json!({"flipV": now}), None));
        }
    });
    if let Some((offset, gesture)) = offset_edit {
        set = super::clone_offset_edit(app, offset).map(|params| (params, gesture));
    }
    if let Some((params, gesture)) = set {
        edit(app, "cloneSource.set", params, gesture);
    }
    if Button::new(tl!("Reset Transform")).show(ui).response().clicked() {
        run(app, "cloneSource.resetTransform", json!({}));
    }

    // On a video timeline: which frame a stroke samples, relative to the frame painted.
    if app.session.active().is_some_and(|d| d.doc.timeline.is_some()) {
        let mut frame_set: Option<(Value, Option<GestureId>)> = None;
        FieldGroup::new(tl!("Frame")).show(ui, |ui, _| {
            let mut shown = Some(slot.frame_offset as f64);
            // A visible label, as X and Y have, and − / + buttons that step the offset by one frame.
            let (field, step) = Row::new()
                .show(ui, |ui, _| {
                    let field = Field::new(tl!("Frame Offset").to_string()).show(ui, |ui, _| {
                        NumberField::new(tl!("Frame Offset").to_string(), Bind::bound(&mut shown))
                            .keeps_name(true)
                            .range(-9999.0..=9999.0)
                            .commit_decimals(0)
                            .show(ui)
                    });
                    let earlier = IconButton::new(IconId::new("minus")).label(Label::from(tl!("One Frame Earlier").to_string())).show(ui).clicked();
                    let later = IconButton::new(IconId::new("plus")).label(Label::from(tl!("One Frame Later").to_string())).show(ui).clicked();
                    (field.inner, if earlier { -1 } else if later { 1 } else { 0 })
                })
                .inner
                .unwrap_or((None, 0));
            if let Some(out) = field
                && let Some(Some(landed)) = out.events.landed().copied()
                && landed.round() as i64 != slot.frame_offset
            {
                frame_set = Some((json!({"frameOffset": landed.round() as i64}), out.events.last().and_then(EditEvent::id)));
            }
            if step != 0 {
                frame_set = Some((json!({"frameOffset": (slot.frame_offset + step).clamp(-9999, 9999)}), None));
            }
            let mut hold = slot.hold_frame;
            Checkbox::new(Bind::bound(&mut hold)).label(tl!("Hold Source Frame")).show(ui);
            if hold != slot.hold_frame {
                frame_set = Some((json!({"holdFrame": hold}), None));
            }
            let line = match (slot.hold_frame, slot.frame, slot.frame_offset) {
                (true, Some(f), _) => crate::i18n::fmt(tl!("Samples frame {frame}"), &[("frame", &(f + 1).to_string())]),
                (true, None, _) => tl!("Samples the frame on screen until a source is set").to_string(),
                (false, _, 0) => tl!("Samples the frame being painted").to_string(),
                (false, _, -1) => tl!("Samples the previous frame").to_string(),
                (false, _, n) => crate::i18n::fmt(tl!("Samples {count} frames from the one painted"), &[("count", &format!("{n:+}"))]),
            };
            Text::new(line).show(ui);
        });
        if let Some((params, gesture)) = frame_set {
            edit(app, "cloneSource.set", params, gesture);
        }
    }

    // Overlay options.
    let mut o = cs.overlay.clone();
    let before = o.clone();
    let mut opacity = o.opacity as f64;
    let mut gesture = None;
    Checkbox::new(Bind::bound(&mut o.show)).label(tl!("Show Overlay")).show(ui);
    let slider = SliderField::new(tl!("Opacity"), Bind::bound(&mut opacity), 0.0..=100.0).unit_kind(UnitKind::Percent).show(ui);
    if slider.events.landed().is_some() {
        gesture = slider.events.last().and_then(EditEvent::id);
        o.opacity = opacity as f32;
    }
    let modes = [("normal", tl!("Normal")), ("darken", tl!("Darken")), ("lighten", tl!("Lighten")), ("difference", tl!("Difference"))]
        .map(|(value, label)| Entry::new(value.to_string(), label));
    Select::new(tl!("Blend Mode"), Bind::bound(&mut o.blend), &modes).show(ui);
    Checkbox::new(Bind::bound(&mut o.clipped)).label(tl!("Clipped")).show(ui);
    Checkbox::new(Bind::bound(&mut o.auto_hide)).label(tl!("Auto Hide")).show(ui);
    Checkbox::new(Bind::bound(&mut o.invert)).label(tl!("Invert")).show(ui);
    if o != before {
        let params = json!({"show": o.show, "opacity": o.opacity, "clipped": o.clipped, "autoHide": o.auto_hide, "invert": o.invert, "blend": o.blend});
        edit(app, "cloneSource.overlay", params, gesture);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Modifiers, vec2};
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};

    type H = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

    fn harness(timeline: bool) -> H {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        if timeline {
            app.run("timeline.create", json!({"duration": 10, "fps": 24.0})).unwrap();
        }
        let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(vec2(400.0, 900.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(2);
        h
    }

    fn offset_field(h: &H) -> Option<egui_kittest::Node<'_>> {
        h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == "Frame Offset"))
    }

    /// On a video timeline the panel sets the slot's frame offset and hold through
    /// `cloneSource.set`; without a timeline it shows neither.
    #[test]
    fn the_frame_controls_set_the_slot_on_a_timeline_only() {
        assert!(offset_field(&harness(false)).is_none(), "no timeline, no frame controls");
        let mut h = harness(true);
        let shown = h.query_all_by_label_contains("Frame Offset").filter(|n| n.accesskit_node().role() != Role::SpinButton).count();
        assert!(shown > 0, "the field has a visible label beside it");
        offset_field(&h).expect("the Frame Offset field").click();
        h.run_steps(1);
        h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text("-1".into()));
        h.run_steps(1);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert_eq!(h.state().session.presets().clone.active().frame_offset, -1);
        h.get_by_role_and_label(Role::Button, "One Frame Earlier").click();
        h.run_steps(2);
        assert!(h.state().session.presets().clone.active().frame_offset < -1, "minus steps to an earlier frame");
        h.get_by_role_and_label(Role::Button, "One Frame Later").click();
        h.run_steps(2);
        h.get_by_role_and_label(Role::Button, "One Frame Later").click();
        h.run_steps(2);
        assert!(h.state().session.presets().clone.active().frame_offset > -1, "plus steps to a later frame");
        h.get_by_label("Hold Source Frame").click();
        h.run_steps(2);
        assert!(h.state().session.presets().clone.active().hold_frame);
    }
}
