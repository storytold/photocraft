use craft::behavior::{EditEvent, Mixed};
use craft::text::ui::DocUnit;
use craft::text::ui::character::{CharacterControls, ParagraphControls};
use craft::text::ui::type_bar::TypeBarChange;
use craft::text::ui::type_options::{TextAntiAlias, TextOrientation, TypeOptionWords};
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Env;
use craft::ui::sections::Section;

use super::defaults::{apply_defaults, formatting_params, target, target_text};
use super::{PhotocraftApp, fonts, json, values};

/// The Character panel's OpenType buttons, in Photoshop's order: (feature tag, command id,
/// tooltip). The tooltips are the Type › OpenType menu labels.
pub(crate) const OPENTYPE_BUTTONS: [(&str, &str, &str); 8] = [
    ("liga", "type.openType.standardLigatures", "Standard Ligatures"),
    ("calt", "type.openType.contextualAlternates", "Contextual Alternates"),
    ("dlig", "type.openType.discretionaryLigatures", "Discretionary Ligatures"),
    ("swsh", "type.openType.swash", "Swash"),
    ("salt", "type.openType.stylisticAlternates", "Stylistic Alternates"),
    ("titl", "type.openType.titlingAlternates", "Titling Alternates"),
    ("ordn", "type.openType.ordinals", "Ordinals"),
    ("frac", "type.openType.fractions", "Fractions"),
];

/// Runs an OpenType toggle command on the Character panel's scope: the edited layer's selected
/// characters (or its whole text) while typing, else every selected type layer. The new state
/// is explicit, so it follows the button the user saw rather than the layer's first run.
pub(crate) fn toggle_opentype(app: &mut PhotocraftApp, id: &str, shown_on: bool) {
    let Some(mut p) = formatting_params(app) else {
        if let Some((tag, _, _)) = OPENTYPE_BUTTONS.iter().find(|(_, command, _)| *command == id) {
            let props = match *tag {
                "liga" => json!({"ligatures": !shown_on}),
                "dlig" => json!({"discretionaryLigatures": !shown_on}),
                _ => json!({"features": { (*tag): !shown_on }}),
            };
            apply_defaults(app, &props);
        }
        return;
    };
    p["on"] = json!(!shown_on);
    let _ = app.run(id, p);
}

/// Properties panel sections for a type layer: Character, Paragraph and Type Options (#155).
pub fn type_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    type_sections(app, ui, true, true, false);
    type_options(app, ui);
}

/// Window › Character / Paragraph (#150): the same controls as Properties, in their own dock
/// group. Without a type target they edit the defaults for the next Type-tool layer.
pub fn character_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui, paragraph: bool) {
    type_sections(app, ui, !paragraph, paragraph, true);
}

/// The Character and/or Paragraph sections.
fn type_sections(app: &mut PhotocraftApp, ui: &mut egui::Ui, character: bool, paragraph: bool, show_defaults: bool) {
    if !show_defaults && target(app).is_none() {
        return;
    }
    let unit = values::doc_unit(app);
    if character {
        Section::new(tl!("Character")).show(ui, |ui, _| character_controls(app, ui, unit));
    }
    if paragraph {
        Section::new(tl!("Paragraph")).show(ui, |ui, _| paragraph_controls(app, ui, unit));
    }
}

fn character_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, unit: DocUnit) {
    let shown = values::character_values(app);
    let families = fonts::families();
    let styles = values::style_names(app, &shown);
    let face = values::face_axes(app);
    let mut font_state = fonts::font_state(app);
    let mut controls = CharacterControls::new(tl!("Character"), &shown, &families, &styles).unit(unit);
    controls = controls.font_picker(font_state.source());
    if let Some(face) = &face {
        controls = controls.axes(face.view());
    }
    let mut out = controls.show(ui);
    fonts::finish_font_state(app, font_state);
    values::apply_character_events(app, &out.edits);
    values::follow_family(app, std::mem::take(&mut out.preview_family), out.rested_family.as_deref());
    if let Some(request) = out.requests.take_color() {
        values::pick_color(app, request.color.map(|c| [c.r, c.g, c.b]));
    }
}

fn paragraph_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, unit: DocUnit) {
    let shown = values::paragraph_values(app);
    let vertical = values::orientation(app) == TextOrientation::Vertical;
    // PhotoCraft's layout does not hyphenate (#6710): the check box stays hidden.
    let out = ParagraphControls::new(tl!("Paragraph"), &shown).unit(unit).point_text(values::point_text(app)).vertical(vertical).hyphenate(false).show(ui);
    values::apply_paragraph_events(app, &out.edits);
}

/// Type Options: anti-aliasing and orientation of the whole layer (the Type menu's commands).
fn type_options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if target_text(app).is_none() {
        return;
    }
    let words = Env::of(ui).words::<TypeOptionWords>();
    let methods = values::anti_alias_methods();
    let method_entries: Vec<Entry<TextAntiAlias>> = methods.iter().map(|&m| Entry::new(m, m.label(&words).clone())).collect();
    let orientation_entries =
        [Entry::new(TextOrientation::Horizontal, words.horizontal.clone()), Entry::new(TextOrientation::Vertical, words.vertical.clone())];
    Section::new(tl!("Type Options")).show(ui, |ui, _| {
        let mut edits: Vec<EditEvent<TypeBarChange>> = Vec::new();
        let method = Select::new(words.anti_alias.clone(), Bind::given(values::shown(values::anti_alias(app))), &method_entries).show(ui);
        edits.extend(method.events.map(TypeBarChange::AntiAlias));
        let orientation = Segmented::new(words.orientation.clone(), Bind::given(Mixed::Same(values::orientation(app))), &orientation_entries).show(ui);
        edits.extend(orientation.events.map(TypeBarChange::Orientation));
        values::apply_bar_events(app, &edits);
    });
}
