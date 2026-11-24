//! The frame's stroke cells: the check box, colour well, width and alignment shared by the options bar and the
//! Properties section, and the stroke's JSON for the engine.

use craft::behavior::EditEvents;
use craft::color_ui::ui::{ColorPopover, PickedColor};
use craft::theme::Srgba;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::form::Field;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::UnitKind;
use craft::ui::text::NumberField;
use egui::Ui;
use photocraft_doc::{Color, Fill, ShapeStroke, StrokeAlign};
use serde_json::{Value, json};

/// A pass in which a stroke cell changed the value.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Change {
    /// The history coalesce key of the edit's gesture; `None` for a one-step `Set`, which is its own engine step.
    pub coalesce: Option<String>,
}

/// The value an edit pass lands on and its change, from the pass's last event; `None` when nothing landed (a
/// `Begin`, or no edit). A `Cancel` lands on the value from before the gesture under the gesture's key.
pub(super) fn land<T: Clone>(key: &str, events: &EditEvents<T>) -> Option<(T, Change)> {
    let last = events.last()?;
    let value = last.landed()?.clone();

    Some((value, Change { coalesce: crate::history_keys::coalesce(key, last) }))
}

/// The engine's `stroke` parameter: `Null` removes the stroke. Opacity is in percent. The colour is left out for a
/// non-solid paint, so the engine keeps the paint.
pub(super) fn param(stroke: &Option<ShapeStroke>) -> Value {
    stroke.as_ref().map_or(Value::Null, json)
}

/// A stroke as the engine's `stroke` object.
pub(super) fn json(s: &ShapeStroke) -> Value {
    let align = match s.align {
        StrokeAlign::Inside => "inside",
        StrokeAlign::Outside => "outside",
        _ => "center",
    };
    let mut v = json!({"width": s.width, "opacity": s.opacity * 100., "align": align});
    if let Fill::Solid(c) = &s.paint {
        let [r, g, b] = c.to_rgb();
        v["color"] = json!([r, g, b, c.alpha]);
    }
    v
}

/// The stroke check box and, with a stroke, its colour (with alpha), width and alignment. `key` prefixes the
/// coalesce key of each gesture. Returns the change made in this pass; a pass applies only its own input.
pub(super) fn cells(ui: &mut Ui, value: &mut Option<ShapeStroke>, key: &str) -> Option<Change> {
    let mut change = None;
    let on = Checkbox::new(Bind::given(value.is_some())).label(tl!("Stroke")).show(ui).events;
    if let Some((on, c)) = land(key, &on) {
        *value = on.then(ShapeStroke::default);
        change = Some(c);
    }
    let Some(s) = value else { return change };

    let color = match s.paint {
        Fill::Solid(c) => c,
        _ => Color::BLACK,
    };
    let [r, g, b] = color.to_rgb();
    let well =
        ColorPopover::new(tl!("Stroke Color"), Bind::given(PickedColor::rgb(Srgba::new(r, g, b, color.alpha)))).picker(|p| p.alpha(true)).show(ui).events;
    if let Some((picked, c)) = land(key, &well) {
        let p = picked.color;
        s.paint = Fill::Solid(Color::rgba(p.r, p.g, p.b, p.a));
        change = Some(c);
    }

    let width = Field::new(tl!("Stroke:"))
        .show(ui, |ui, _| {
            NumberField::new(tl!("Stroke:"), Bind::given(Some(f64::from(s.width)))).range(0.0..=1000.0).unit_kind(UnitKind::Pixels).show(ui).edit.events
        })
        .inner
        .unwrap_or_default();
    if let Some((Some(w), c)) = land(key, &width) {
        s.width = w as f32;
        change = Some(c);
    }

    let aligns =
        [Entry::new(StrokeAlign::Inside, tl!("Inside")), Entry::new(StrokeAlign::Center, tl!("Center")), Entry::new(StrokeAlign::Outside, tl!("Outside"))];
    let picked = Select::new(tl!("Align"), Bind::given(s.align), &aligns).show(ui).events;
    if let Some((align, c)) = land(key, &picked) {
        s.align = align;
        change = Some(c);
    }
    change
}
