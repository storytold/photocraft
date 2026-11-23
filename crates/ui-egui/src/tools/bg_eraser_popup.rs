//! The Background Eraser's compact brush popup (#11910): the Brush Preset picker's compact form
//! (no preset list, search or gear) that its options-bar chip and a canvas right-click open, as in
//! Photoshop. The picker's head shows Size, Hardness and the tip widget; [`rows`] adds Spacing and
//! the "Size:" and "Tolerance:" pen selects (Off · Pen Pressure · Stylus Wheel), the latter with the
//! reference's tablet warning.
//!
//! The rows edit the caller's copy of the brush; the Tolerance control is a tool option
//! (`ToolOptions::bg_tolerance_control`).

use craft::behavior::IconId;
use craft::theme::variant::Tone;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Icon;
use craft::ui::form::SliderField;
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
use craft::ui::primitives::IconSpec;
use photocraft_engine::BrushSettings;
use photocraft_engine::paint::Control;

/// The pen selects' entries, as stored.
pub const OFF: &str = "off";
pub const PEN_PRESSURE: &str = "penPressure";
pub const STYLUS_WHEEL: &str = "stylusWheel";
/// The tooltip of the warning triangle next to "Tolerance:".
pub const PRESSURE_WARNING: &str = "Control by pen pressure requires the use of a pressure sensitive tablet";

fn pen_entries() -> [Entry<String>; 3] {
    [
        Entry::new(OFF.to_string(), tl!("Off")),
        Entry::new(PEN_PRESSURE.to_string(), tl!("Pen Pressure")),
        Entry::new(STYLUS_WHEEL.to_string(), tl!("Stylus Wheel")),
    ]
}

/// What the brush's "Size:" control is: Pen Pressure is the brush's pressure-for-size switch,
/// Stylus Wheel its Shape Dynamics size control.
pub fn size_control(b: &BrushSettings) -> &'static str {
    if b.pressure_size {
        PEN_PRESSURE
    } else if b.shape_dynamics.enabled && b.shape_dynamics.size.control == Control::StylusWheel {
        STYLUS_WHEEL
    } else {
        OFF
    }
}

/// Set the brush's "Size:" control.
pub fn set_size_control(b: &mut BrushSettings, control: &str) {
    b.pressure_size = control == PEN_PRESSURE;
    if control == STYLUS_WHEEL {
        b.shape_dynamics.enabled = true;
        b.shape_dynamics.size.control = Control::StylusWheel;
    } else if b.shape_dynamics.size.control == Control::StylusWheel {
        b.shape_dynamics.size.control = Control::Off;
    }
}

/// The app rows of the compact picker, under its head: Spacing, the "Size:" and "Tolerance:" selects
/// and the tablet warning. They edit `brush` and the tool's Tolerance control. Returns the coalesce
/// key of a Spacing edit, so the caller commits a scrub as one history step.
pub fn rows(ui: &mut egui::Ui, brush: &mut BrushSettings, tolerance_control: &mut String) -> Option<String> {
    let mut key = None;
    let mut spacing = f64::from(brush.spacing) * 100.0;
    let shown = spacing;
    let out = SliderField::new(tl!("Spacing"), Bind::bound(&mut spacing), 1.0..=1000.0).unit("%").show(ui);
    if spacing != shown {
        brush.spacing = (spacing / 100.0).clamp(0.01, 10.0) as _;
        key = out.events.iter().find_map(|e| crate::history_keys::coalesce("brush", e));
    }
    let entries = pen_entries();
    let size = Select::new(tl!("Size:"), Bind::given(size_control(brush).to_string()), &entries).show(ui).events.committed().cloned();
    if let Some(control) = size {
        set_size_control(brush, &control);
    }
    Row::new().show(ui, |ui, _| {
        let picked = Select::new(tl!("Tolerance:"), Bind::given(tolerance_control.clone()), &entries).show(ui).events.committed().cloned();
        if let Some(control) = picked {
            *tolerance_control = control;
        }
        Icon::new(IconSpec::svg(IconId::new("warning"))).tone(Tone::Warn).label(tl!(PRESSURE_WARNING).to_owned()).show(ui);
    });
    key
}
