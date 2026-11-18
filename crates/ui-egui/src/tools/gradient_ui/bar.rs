//! The Gradient tool's options-bar cells: Mode (live Gradient or Classic gradient), the gradient swatch, the style,
//! and for the Classic gradient the blend mode and Opacity, then Reverse and Dither.

use craft::draw::ui::gradient::{GradientFillWords, GradientStyle, GradientSwatch};
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, LabelKind, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::patterns::blend_keys::{BlendKeyEntry, blend_entries};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::NumberField;
use photocraft_doc::BlendMode;

use super::{fill_layer_active, options_changed, stops};
use crate::state::Tool;
use crate::{PhotocraftApp, panel_docking};

/// The blend modes the Classic gradient lists, with a rule above each group.
fn blend_modes() -> Vec<Entry<BlendMode>> {
    let keys: Vec<BlendKeyEntry<BlendMode>> =
        crate::blend_keys::tool_modes(Tool::Gradient).unwrap_or_default().into_iter().flatten().map(|mode| crate::panels::blend_entry(mode, mode)).collect();
    blend_entries(&keys, |entry| Label::from(tl!(entry.key.label()).to_string()))
}

/// `add` captioned `label` as a field of the bar.
fn labelled(ui: &mut egui::Ui, label: &str, mut add: impl FnMut(&mut egui::Ui)) {
    Field::new(label.to_owned()).show(ui, |ui, _| add(ui));
}

/// The Gradient options cells over the tool options. Every change that also edits a selected gradient fill layer
/// (style, Reverse, Dither) goes through [`options_changed`], once, after the cells.
pub(crate) fn bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let before = app.ui.tool_options.clone();
    // A selected gradient fill layer is edited live whatever the stored choice: the Mode cell reads Gradient and is greyed.
    let locked_live = fill_layer_active(app);
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let tool_stops = stops::of_preset(&app.session.presets.gradient, fg, bg);
    let sampler = stops::sampler(fg, bg);
    let words = Env::of(ui).words::<GradientFillWords>();
    let styles: Vec<Entry<GradientStyle>> = GradientStyle::ALL.iter().map(|style| Entry::new(*style, style.label(&words)).icon(style.icon())).collect();
    let modes = [Entry::new(false, tl!("Gradient").to_owned()), Entry::new(true, tl!("Classic gradient").to_owned())];
    let blends = blend_modes();
    let open_editor;
    {
        let o = &mut app.ui.tool_options;
        let mut classic = o.gradient_classic && !locked_live;
        Field::new(tl!("Gradient mode").to_owned()).side(SideLabel::new().kind(LabelKind::Hidden)).show(ui, |ui, _| {
            Select::new(tl!("Gradient mode").to_owned(), Bind::bound(&mut classic), &modes).enabled(!locked_live).show(ui);
        });
        if !locked_live {
            o.gradient_classic = classic;
        }
        open_editor = GradientSwatch::new(tl!("Gradient picker").to_string(), &tool_stops).sampler(&sampler).show(ui).clicked;
        Divider::vertical().show(ui);
        let style = stops::style_named(&o.gradient_style);
        if let Some(next) = Segmented::new(Label::from(tl!("Gradient style").to_string()), Bind::given(style), &styles).show(ui).value().copied() {
            o.gradient_style = stops::style_name(next).to_owned();
        }
        if classic {
            labelled(ui, tl!("Mode"), |ui| {
                Select::new(tl!("Blend Mode").to_owned(), Bind::bound(&mut o.gradient_blend_mode), &blends).show(ui);
            });
            labelled(ui, tl!("Opacity"), |ui| {
                let field =
                    NumberField::new(tl!("Opacity").to_owned(), Bind::given(Some(f64::from(o.fill_opacity)))).range(1.0..=100.0).unit("%").scrub_by_label();
                if let Some(next) = field.show(ui).value().copied().flatten() {
                    o.fill_opacity = next as f32;
                }
            });
        }
        if let Some(on) = Checkbox::new(Bind::given(o.gradient_reverse)).label(tl!("Reverse").to_owned()).show(ui).value().copied() {
            o.gradient_reverse = on;
        }
        if let Some(on) = Checkbox::new(Bind::given(o.gradient_dither)).label(tl!("Dither").to_owned()).show(ui).value().copied() {
            o.gradient_dither = on;
        }
    }
    if open_editor {
        let _ = panel_docking::show_panel(app, "gradientEditor", None);
    }
    options_changed(app, &before);
}
