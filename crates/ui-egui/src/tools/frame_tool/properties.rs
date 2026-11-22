//! The Frame section of Properties: size and position of the container, its stroke, the target and the image
//! commands.

use super::{Target, report, select, stroke, target};
use crate::PhotocraftApp;
use craft::ui::buttons::Button;
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, GlyphLabel, LabelKind, LabelScheme, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft::ui::display::Divider;
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::UnitKind;
use craft::ui::sections::Section;
use craft::ui::text::NumberField;
use egui::Ui;
use photocraft_doc::Layer;
use serde_json::json;
use std::ops::RangeInclusive;

/// The range of W and H, in pixels.
const SIZE: RangeInclusive<f64> = 1.0..=300_000.0;
/// The range of X and Y, in pixels.
const OFFSET: RangeInclusive<f64> = -300_000.0..=300_000.0;

/// A glyph-labelled pixel field whose scrubbing label edits the value. Returns the landed value and its change.
fn field(ui: &mut Ui, glyph: &'static str, name: &'static str, value: f64, range: &RangeInclusive<f64>, key: &str) -> Option<(f64, stroke::Change)> {
    let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text(glyph))).scrub(true);
    let events = Field::new(name)
        .side(side)
        .show(ui, |ui, _| NumberField::new(name, Bind::given(Some(value))).range(range.clone()).unit_kind(UnitKind::Pixels).show(ui).edit.events)
        .inner
        .unwrap_or_default();
    let (v, change) = stroke::land(key, &events)?;
    Some((v?, change))
}

/// Frame-aware Properties: W/H/X/Y edit the container; the content can be selected, fitted or replaced.
pub(crate) fn properties(app: &mut PhotocraftApp, ui: &mut Ui, l: &Layer) {
    let Some(f) = &l.frame else { return };
    let locked = app.session.active().is_some_and(|st| {
        let locks = st.doc.effective_locks(l.id);
        locks.all || locks.position
    });
    let id = l.id;
    let [a, b, c, d, x, y] = f.transform.m;
    let (w, h) = (a.hypot(b), c.hypot(d));
    let filled = l.children().is_some_and(|c| !c.is_empty());
    let current = target(app, l);
    let mut stroke_value = f.stroke.clone();
    let targets: Vec<Entry<Target>> = if filled {
        vec![Entry::new(Target::Frame, tl!("Frame")), Entry::new(Target::Content, tl!("Image")), Entry::new(Target::Both, tl!("Both"))]
    } else {
        vec![Entry::new(Target::Frame, tl!("Frame"))]
    };

    let mut geometry: Option<([f64; 4], stroke::Change)> = None;
    let mut stroke_change = None;
    let mut picked_target = None;
    let mut import = false;
    let mut fit: Vec<&'static str> = Vec::new();

    Section::fixed(tl!("Frame")).enabled(!locked).show(ui, |ui, _| {
        FieldGroup::new(tl!("Frame")).labels(LabelScheme::Inspector).show(ui, |ui, _| {
            let mut values = [w, h, x, y];
            let mut edited = None;
            Row::new().fill_width().show(ui, |ui, _| {
                for (i, glyph, name, range, key) in [(0, "W", tl!("Width"), &SIZE, "w"), (1, "H", tl!("Height"), &SIZE, "h")] {
                    if let Some((v, ch)) = field(ui, glyph, name, values[i], range, &format!("frame:{}:{key}", id.0)) {
                        values[i] = v;
                        edited = Some(ch);
                    }
                }
            });
            Row::new().fill_width().show(ui, |ui, _| {
                for (i, glyph, key) in [(2, "X", "x"), (3, "Y", "y")] {
                    if let Some((v, ch)) = field(ui, glyph, glyph, values[i], &OFFSET, &format!("frame:{}:{key}", id.0)) {
                        values[i] = v;
                        edited = Some(ch);
                    }
                }
            });
            if let Some(ch) = edited {
                geometry = Some((values, ch));
            }
            if let Some(ch) = stroke::cells(ui, &mut stroke_value, &format!("frame:{}:stroke", id.0)) {
                stroke_change = Some(ch);
            }
            Divider::horizontal().show(ui);
            let pick = Segmented::new(tl!("Target"), Bind::given(current), &targets).show(ui).events;
            if let Some(t) = pick.landed() {
                picked_target = Some(*t);
            }
            import |= Button::new(if filled { tl!("Replace Image…") } else { tl!("Import Image…") }).show(ui).clicks > 0;
            if filled {
                for (mode, label) in [("contentToFrame", tl!("Fit Content to Frame")), ("frameToContent", tl!("Fit Frame to Content"))] {
                    if Button::new(label).show(ui).clicks > 0 {
                        fit.push(mode);
                    }
                }
            }
        });
    });

    if let Some(([nw, nh, nx, ny], ch)) = geometry
        && w > 0.
        && h > 0.
    {
        let r = app.run("layer.frame.set", json!({"layer":id.0,"transform":[a*nw/w,b*nw/w,c*nh/h,d*nh/h,nx,ny],"coalesce":ch.coalesce}));
        report(app, r);
    }
    if let Some(ch) = stroke_change {
        let r = app.run("layer.frame.set", json!({"layer":id.0,"stroke":stroke::param(&stroke_value),"coalesce":ch.coalesce}));
        report(app, r);
    }
    if let Some(t) = picked_target {
        select(app, id, t);
    }
    if import {
        select(app, id, Target::Frame);
        let ctx = ui.ctx().clone();
        let r = crate::menus::invoke(app, &ctx, "file.placeEmbedded", json!({}));
        report(app, r);
    }
    for mode in fit {
        let r = app.run("layer.frame.fit", json!({"layer":id.0,"mode":mode}));
        report(app, r);
    }
}
