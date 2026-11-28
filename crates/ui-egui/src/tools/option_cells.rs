//! The options bar's number cell and hint, shared by every PhotoCraft tool bar: a captioned
//! `NumberField` whose out-of-range commit comes back as a [`RangeAlert`] for the caller to show.

use std::ops::RangeInclusive;
use std::sync::Arc;

use craft::behavior::{EditEvent, EditEvents, Mixed, NumberCodec};
use craft::ui::contract::{Bind, FieldLayout, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing};
use craft::ui::dialog::Alert;
use craft::ui::display::{Elide, Shade, Text};
use craft::ui::form::Field;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::{NumberField, RangeAlert};

use crate::PhotocraftApp;

/// One options-bar number cell over an `f32` option. The bound value changes only when the field
/// commits, so showing a bar never rounds a fraction another tool shares.
pub(crate) struct NumberCell<'a> {
    caption: Label,
    value: Bind<'a, f32>,
    range: RangeInclusive<f64>,
    unit: Option<Label>,
    whole: bool,
    alert_fractions: bool,
    popup_slider: bool,
    tooltip: Option<Label>,
    enabled: bool,
    icon: Option<IconSpec<'static>>,
}

/// A number cell named `caption` over `value`, held to `range`.
pub(crate) fn number<'a>(caption: impl Into<Label>, value: Bind<'a, f32>, range: RangeInclusive<f64>) -> NumberCell<'a> {
    NumberCell {
        caption: caption.into(),
        value,
        range,
        unit: None,
        whole: false,
        alert_fractions: false,
        popup_slider: false,
        tooltip: None,
        enabled: true,
        icon: None,
    }
}

impl<'a> NumberCell<'a> {
    /// A unit shown after the number.
    #[must_use]
    pub(crate) fn unit(mut self, unit: impl Into<Label>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// The cell holds whole numbers and shows no decimals.
    #[must_use]
    pub(crate) fn whole(mut self) -> Self {
        self.whole = true;
        self
    }

    /// The cell holds whole numbers, and a typed fraction is rounded to the closest one after it
    /// commits and reported in [`NumberCellOutput::rounded`] for the caller's alert.
    #[must_use]
    pub(crate) fn whole_alerting_fractions(mut self) -> Self {
        self.whole = true;
        self.alert_fractions = true;
        self
    }

    /// A chevron after the field opens a slider over the same range.
    #[must_use]
    pub(crate) fn popup_slider(mut self) -> Self {
        self.popup_slider = true;
        self
    }

    /// The field's tooltip.
    #[must_use]
    pub(crate) fn tooltip(mut self, tooltip: impl Into<Label>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Greys the cell when `enabled` is false.
    #[must_use]
    pub(crate) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// An icon side label in place of the caption; the field keeps the caption as its accessible name.
    #[must_use]
    pub(crate) fn icon(mut self, icon: impl Into<IconSpec<'static>>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Shows the cell. Returns the pass's edit events, which a caller keys its history step by
    /// (`history_keys::coalesce`), and the alert of a typed value that was clamped to the range.
    pub(crate) fn show(self, ui: &mut egui::Ui) -> NumberCellOutput {
        let Self { caption, mut value, range, unit, whole, alert_fractions, popup_slider, tooltip, enabled, icon } = self;
        let mut side = SideLabel::new().scrub(true);
        if let Some(icon) = icon {
            side = side.icon(icon).layout(FieldLayout::Inside);
        }
        let mut out = NumberCellOutput { events: EditEvents::default(), out_of_range: None, rounded: false };
        Field::new(caption.clone()).enabled(enabled).side(side).sizing(Sizing::Content).show(ui, |ui, _| {
            let mut field = NumberField::named(caption.clone(), value.reborrowed()).range(range.clone()).scrub_by_label();
            if whole {
                let codec = NumberCodec::new(Env::of(ui).units().reader(None), 0);
                field = field.step(1.0).codec(codec);
                // A cell that alerts on fractions commits the fraction as typed, so it can tell one
                // was typed; it rounds the value below.
                if !alert_fractions {
                    field = field.commit_decimals(0);
                }
            }
            if let Some(unit) = unit.clone() {
                field = field.unit(unit);
            }
            if popup_slider {
                field = field.popup_slider(range.clone());
            }
            if let Some(tooltip) = tooltip.clone() {
                field = field.tooltip(tooltip);
            }
            let shown = field.show(ui);
            out = NumberCellOutput { events: shown.edit.events, out_of_range: shown.out_of_range, rounded: false };
        });
        if alert_fractions && out.events.committed().is_some_and(|v| v.is_finite() && v.fract() != 0.0) {
            if let Some(&typed) = value.value() {
                let rounded = typed.round().clamp(*range.start() as f32, *range.end() as f32);
                let _ = value.reborrow().apply(&EditEvent::set(Mixed::Same(typed), rounded, Arc::from(""), None));
            }
            out.rounded = true;
        }
        out
    }
}

/// What a [`NumberCell`] reported this pass.
pub(crate) struct NumberCellOutput {
    /// The field's edit events: a scrub or arrow run is one gesture, a typed commit a `Set`.
    pub events: EditEvents<f32>,
    /// A typed value that was clamped to the range, in the pass it committed.
    pub out_of_range: Option<RangeAlert>,
    /// A typed fraction was rounded to a whole number, in the pass it committed (only for a cell
    /// built with [`NumberCell::whole_alerting_fractions`]).
    pub rounded: bool,
}

/// Opens the out-of-range alert for `alert`; `ui` supplies the env that words its range. One alert
/// is open per message (the helper's rule).
pub(crate) fn range_alert(app: &mut PhotocraftApp, ui: &egui::Ui, alert: &RangeAlert) {
    let _ = crate::dialogs::alerts::show(app, Alert::range(tl!("PhotoCraft"), alert, &Env::of(ui)));
}

/// A faint one-line hint after a bar's members.
pub(crate) fn hint(ui: &mut egui::Ui, text: impl Into<Label>) {
    Text::new(text).shade(Shade::Faint).elide(Elide::End).show(ui);
}
