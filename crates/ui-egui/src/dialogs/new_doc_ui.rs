//! File › New: the framework's New Document dialog, hosted. This file is the app side: it opens the dialog on the
//! last created settings, carries the colour picker, the saved presets and the recent list between the dialog and the
//! engine, and runs `file.new` with what Create answers.

use craft::behavior::PixelSize;
use craft::theme::Srgba;
use craft::ui::contract::{Bind, SaveKey};
use craft::ui::dialog::{DialogId, DialogResult, StackDialog};
use craft::ui::patterns::form_dialogs::{
    BackgroundContents, BitDepth, ColorMode as DialogMode, ColorProfiles, DocumentOptions, HostedNewDocument, NewDocumentDialog, PIXEL_ASPECTS, SQUARE_PIXELS,
};
use craft::ui::patterns::presets::{GroupPath, NamedPresetEvent, PresetLibrary};
use craft::ui::runtime::env::UnitKind;
use photocraft_color::ColorMode;
use photocraft_engine::document_preset_cmds::DocumentSettings;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The SAVED-tier key of the last created settings (without their name): the next New Document opens on them.
pub(crate) const LAST_CREATED: SaveKey = SaveKey::new("photocraft.file.new");

/// The SAVED-tier key of the dialog's place and size.
const WINDOW: SaveKey = SaveKey::new("photocraft.file.new.window");

/// Units of `unit` per inch, 0 for pixels: points and picas at `pt_per_in` (Preferences › Point/Pica
/// Size, 72 PostScript or 72.27 traditional), as Image Size and the Crop tool convert them.
fn per_inch(unit: &str, pt_per_in: f32) -> f32 {
    let pt = if pt_per_in.is_finite() && pt_per_in > 0.0 { pt_per_in } else { 72.0 };
    match unit {
        "pt" => pt,
        "pica" => pt / 12.0,
        _ => UNITS.iter().find(|u| u.0 == unit).map_or(0.0, |u| u.2),
    }
}

/// Pixels to the display unit at `ppi`, with `pt_per_in` points per inch.
pub fn to_unit(px: f32, unit: &str, ppi: f32, pt_per_in: f32) -> f32 {
    match per_inch(unit, pt_per_in) {
        per_in if per_in > 0.0 => px / ppi.max(1.0) * per_in,
        _ => px,
    }
}

/// Display unit back to pixels at `ppi`, with `pt_per_in` points per inch.
pub fn from_unit(v: f32, unit: &str, ppi: f32, pt_per_in: f32) -> f32 {
    match per_inch(unit, pt_per_in) {
        per_in if per_in > 0.0 => (v / per_in * ppi.max(1.0)).round(),
        _ => v.round(),
/// The colour modes the dialog offers, with the engine's mode of each.
const MODES: [(ColorMode, DialogMode); 5] = [
    (ColorMode::Bitmap, DialogMode::Bitmap),
    (ColorMode::Grayscale, DialogMode::Grayscale),
    (ColorMode::Rgb, DialogMode::Rgb),
    (ColorMode::Cmyk, DialogMode::Cmyk),
    (ColorMode::Lab, DialogMode::Lab),
];

/// Opens File › New on the last created settings and returns the dialog's id.
pub(crate) fn open(app: &mut PhotocraftApp) -> DialogId {
    let options = app.new_document_options();
    // An image on the clipboard is offered as the Clipboard preset; the dialog opens on it, at its size.
    let clipboard = app.new_document_clipboard();
    let background = app.session.tools.background;
    let background = Srgba::from_rgb_array([background[0], background[1], background[2]]);
    let units = app.session.prefs().units_and_rulers.clone();
    // A resolution that is not a positive finite number leaves the dialog's own default.
    let usable = |ppi: f64| Some(ppi).filter(|ppi| ppi.is_finite() && *ppi > 0.0);
    let (print, screen) = (usable(units.print_resolution), usable(units.screen_resolution));
    let profiles = profiles(app);
    let custom = custom_pixel_aspects(app);
    let dialog = HostedNewDocument::new(app.ctx(), options, move |options| {
        let mut dialog = NewDocumentDialog::new(Bind::bound(options))
            .background_color(background)
            .profiles(profiles.clone())
            .custom_pixel_aspects(custom.clone())
            .app_name("PhotoCraft")
            .remember(WINDOW);
        if let Some(size) = clipboard {
            dialog = dialog.clipboard(size);
        }
        if let Some(ppi) = print {
            dialog = dialog.print_resolution(ppi);
        }
        if let Some(ppi) = screen {
            dialog = dialog.screen_resolution(ppi);
        }
        dialog
    })
    .saved(saved_library(app))
    .recent(app.session.presets.recent_documents.iter().map(to_options).collect());
    app.framework.dialogs.open(dialog)
}

/// The dialog's pump (a `TAKERS` row). Each call carries only what the dialog queued since the last one: a colour
/// request, the saved-preset edits and the answer of a closed dialog are each taken once, so a second call in the
/// same frame finds nothing to do.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    carry_color(app);
    store_preset_events(app);
    if let Some(DialogResult::Ok(result)) = app.framework.dialogs.take::<HostedNewDocument>() {
        create(app, &result.options);
    }
}

/// Opens the colour picker for the dialog's background colour request, and hands the picked colour back.
/// A cancelled picker needs no answer: Background Contents only changes when `set_color` answers.
fn carry_color(app: &mut PhotocraftApp) {
    let Some((id, request)) = app.framework.dialogs.get_mut::<HostedNewDocument>().map(|dialog| (dialog.id(), dialog.pick_color())) else { return };
    if let Some(request) = request {
        crate::color_picker_ui::open_for_request(app, id, request);
    }
    f.insert("__clipboard".into(), json!([w, h]));
    apply_preset(f, &(CLIPBOARD, w, h, 72.0), &UnitsAndRulers::default());
}

/// The clipboard image's size, when the dialog offers the Clipboard preset.
fn clipboard_preset(f: &Map<String, Value>) -> Option<Preset> {
    let size = f.get("__clipboard")?.as_array()?;
    let dim = |i: usize| size.get(i)?.as_u64().and_then(|v| u32::try_from(v).ok()).filter(|v| *v > 0);
    Some((CLIPBOARD, dim(0)?, dim(1)?, 72.0))
}

/// Set Width or Height (`key`) from a value typed in `unit`: whole pixels for `file.new`, plus the
/// typed value so the field keeps showing it (see [`shown_size`]).
pub fn set_size(f: &mut Map<String, Value>, key: &str, v: f32, unit: &str, ppi: f32, pt_per_in: f32) {
    f.insert(key.into(), px_value(from_unit(v, unit, ppi, pt_per_in)));
    f.insert(typed_key(key), json!({"value": v, "unit": unit}));
    f.remove("__preset");
    f.remove("__savedPreset");
}

/// What Width or Height (`key`, `d` pixels when unset) shows in `unit`: the value the user typed
/// while it still gives the field's pixel count, else the pixels converted. Converting the rounded
/// pixels back on every keystroke turned a typed `5` mm into 14 px and the text into `4.9` (#1147).
/// A size over the 300000 px limit shows the limit, not the typed value the document won't have.
pub fn shown_size(f: &Map<String, Value>, key: &str, d: f32, unit: &str, ppi: f32, pt_per_in: f32) -> f32 {
    let px = get_f(f, key, d);
    let typed = f.get(&typed_key(key)).and_then(Value::as_object).filter(|t| unit != "px" && t.get("unit").and_then(Value::as_str) == Some(unit));
    match typed.and_then(|t| t.get("value")).and_then(Value::as_f64) {
        Some(v) if from_unit(v as f32, unit, ppi, pt_per_in).max(1.0) == px => v as f32,
        _ => to_unit(px, unit, ppi, pt_per_in),
    if let Some((key, color)) = crate::color_picker_ui::answer_for(app, id)
        && let Some(dialog) = app.framework.dialogs.get_mut::<HostedNewDocument>()
    {
        dialog.set_color(&key, color);
    }
}

/// Runs the dialog's saved-preset edits on the engine's preset store. The dialog has already applied each to its own
/// library, so a created preset's value is read from it by name.
fn store_preset_events(app: &mut PhotocraftApp) {
    let Some(dialog) = app.framework.dialogs.get_mut::<HostedNewDocument>() else { return };
    let commands: Vec<(&str, Value)> = dialog
        .take_preset_events()
        .into_iter()
        .filter_map(|event| match event {
            NamedPresetEvent::Create { name, .. } => {
                let item = dialog.saved_presets().items().into_iter().find(|item| item.name == name)?;
                Some(("document.presets.save", json!({"name": name, "settings": to_settings(&item.value)})))
            }
            NamedPresetEvent::Delete(name) => Some(("document.presets.delete", json!({"name": name}))),
            NamedPresetEvent::Rename { key, text, .. } => Some(("document.presets.rename", json!({"name": key, "newName": text}))),
            _ => None,
        })
        .collect();
    for (command, params) in commands {
        if let Err(error) = app.run(command, params) {
            app.ui.status = error;
        }
    }
}

/// Creates the document from what Create answered. Only a created document updates the last created settings, the
/// recent list and the pixel aspect ratio, so a failed Create changes none of them.
fn create(app: &mut PhotocraftApp, options: &DocumentOptions) {
    if let Err(error) = app.run("file.new", create_params(options)) {
        app.ui.status = error;
        return;
    }
    if let Some(i) = app.session.active_index() {
        app.ui.views[i].place(craft::ui::canvas::ViewCommands::FitOnScreen);
    }
    // The new document gets the picked ratio, Square Pixels included, whatever View › Pixel Aspect Ratio held before
    // (#14034). The ratio is app-wide, so this sets the view's.
    app.ui.view.pixel_aspect = listed_pixel_aspect(app, &options.pixel_aspect);
    let mut kept = options.clone();
    kept.name.clear();
    craft::ui::runtime::memory::set_saved_value(app.ctx(), &LAST_CREATED, Some(kept));
    // Best effort: the document exists either way.
    let _ = app.run("document.recent.add", json!({"settings": to_settings(options)}));
}

/// View › Pixel Aspect Ratio's saved custom ratios as the dialog's own entries: `(id, name)` with the view's
/// `custom:<ratio>:<name>` ids, in the view menu's order. The names are the user's.
fn custom_pixel_aspects(app: &PhotocraftApp) -> Vec<(String, String)> {
    app.ui.shell.custom_pars.iter().map(|c| (format!("custom:{}:{}", c.ratio, c.name), c.name.clone())).collect()
}

/// `id` while the dialog lists it, else Square Pixels: the dialog shows an id it no longer lists (a custom ratio
/// deleted since it was remembered) as Square Pixels, and Create gives what it showed.
fn listed_pixel_aspect(app: &PhotocraftApp, id: &str) -> String {
    let listed = PIXEL_ASPECTS.iter().any(|(known, _)| *known == id) || custom_pixel_aspects(app).iter().any(|(custom, _)| custom == id);
    if listed { id.to_owned() } else { SQUARE_PIXELS.to_owned() }
}

/// The engine's saved document presets as the dialog's library, in the engine's order.
fn saved_library(app: &PhotocraftApp) -> PresetLibrary<DocumentOptions> {
    let mut library = PresetLibrary::new();
    for preset in &app.session.presets.documents {
        let mut options = to_options(&preset.settings);
        options.name.clone_from(&preset.name);
        if library.add(&GroupPath::root(), &preset.name, options).is_err() {
            break;
        }
    }
    library
}

/// The Color Profile lists of the dialog: per mode, the working space (with its profile's description), then the
/// built-in and installed profiles of that mode's colour space. A profile is named by what `file.new`'s `profile`
/// takes: a built-in id or a path.
pub(crate) fn profiles(app: &PhotocraftApp) -> Vec<ColorProfiles> {
    let installed = photocraft_engine::installed_profiles::installed();
    MODES
        .iter()
        .map(|(mode, dialog_mode)| {
            let space = photocraft_engine::color_cmds::mode_space(*mode);
            let mut others: Vec<String> = craft::cms::Builtin::ALL.iter().filter(|b| b.profile().color_space == space).map(|b| b.id().to_string()).collect();
            others.extend(installed.iter().filter(|i| i.is_destination() && i.color_space == space).map(|i| i.path.clone()));
            ColorProfiles::new(*dialog_mode, app.session.color.working(*mode).description.clone(), others)
        })
        .collect()
}

/// The params of `file.new` for what the dialog answered. The display units are not among them.
pub(crate) fn create_params(options: &DocumentOptions) -> Value {
    let mode = MODES.iter().find(|(_, shown)| *shown == options.color_mode).map_or("rgb", |(mode, _)| mode_key(*mode));
    let mut params = json!({
        "name": options.name,
        "width": options.size.width,
        "height": options.size.height,
        "resolution": options.pixels_per_inch,
        "mode": mode,
        "depth": match options.depth {
            BitDepth::One => 1,
            BitDepth::Sixteen => 16,
            BitDepth::ThirtyTwo => 32,
            _ => 8,
        },
        "background": match options.background {
            BackgroundContents::Black => "black".to_owned(),
            BackgroundContents::BackgroundColor => "backgroundColor".to_owned(),
            BackgroundContents::Transparent => "transparent".to_owned(),
            BackgroundContents::Custom(color) => crate::color_picker_ui::hex(color),
            _ => "white".to_owned(),
        },
        "artboard": options.artboard,
    });
    if let Some(profile) = &options.profile {
        params["profile"] = json!(profile);
    }
    params
}

/// The key of a colour mode in the engine's settings.
fn mode_key(mode: ColorMode) -> &'static str {
    match mode {
        ColorMode::Bitmap => "bitmap",
        ColorMode::Grayscale => "gray",
        ColorMode::Cmyk => "cmyk",
        ColorMode::Lab => "lab",
        _ => "rgb",
    }
}

/// The key of a size unit in the engine's settings.
fn unit_key(unit: UnitKind) -> &'static str {
    match unit {
        UnitKind::Inches => "in",
        UnitKind::Centimetres => "cm",
        UnitKind::Millimetres => "mm",
        UnitKind::Points => "pt",
        UnitKind::Picas => "pica",
        _ => "px",
    }
}

/// The settings as `document.presets.save` and `document.recent.add` take them: what `file.new` takes (but the name and
/// the artboard, which the engine's settings do not hold) plus the display units.
pub(crate) fn to_settings(options: &DocumentOptions) -> Value {
    let mut settings = create_params(options);
    if let Some(fields) = settings.as_object_mut() {
        fields.remove("name");
        fields.remove("artboard");
    }
    settings["unit"] = json!(unit_key(options.unit));
    settings["resolutionUnit"] = json!(if options.resolution_unit == UnitKind::PixelsPerCentimetre { "cm" } else { "in" });
    settings
}

/// Whether the dialog currently holds `settings` (so its Recent card shows selected).
fn holds(f: &Map<String, Value>, settings: &DocumentSettings) -> bool {
    // Unset fields hold the dialog's defaults, as the details pane shows them.
    let (w, h) = (get_f(f, "width", 1920.0), get_f(f, "height", 1080.0));
    (w, h, get_f(f, "resolution", 72.0)) == (settings.width as f32, settings.height as f32, settings.resolution)
        && f.get("depth").and_then(Value::as_u64).unwrap_or(8) == u64::from(settings.depth)
        && get_s(f, "mode", "rgb") == settings.mode
        && get_s(f, "background", "white") == settings.background
}

/// A card in the preset grid: a built-in preset or a recently created document.
enum Card {
    Builtin(Preset),
    Recent(DocumentSettings),
}

impl Card {
    /// Pixel size and resolution; a built-in preset at the Units & Rulers preset resolutions.
    fn size(&self, units: &UnitsAndRulers) -> (u32, u32, f32) {
        match self {
            Card::Builtin(listed) => {
                let p = at_pref_resolutions(listed, units);
                (p.1, p.2, p.3)
            }
            Card::Recent(s) => (s.width, s.height, s.resolution),
        }
    }

    /// The preset's name; a recent document takes the name of the built-in preset it matches.
    fn title(&self) -> &'static str {
        match self {
            Card::Builtin(p) => p.0,
            Card::Recent(s) => {
                CATEGORIES.iter().flat_map(|c| c.1.iter()).find(|p| (p.1, p.2, p.3) == (s.width, s.height, s.resolution)).map_or("Custom", |p| p.0)
            }
        }
    }

    /// The size line, in the unit the preset applies (a recent one: the unit it was made in).
    fn size_line(&self, units: &UnitsAndRulers, f: &Map<String, Value>) -> String {
        let (w, h, ppi) = self.size(units);
        let unit = match self {
            Card::Builtin(p) => preset_unit(p, f),
            Card::Recent(s) => s.unit.clone(),
        };
        if unit == "px" {
            format!("{w} x {h} px @ {} ppi", widgets::fmt_num(f64::from(ppi)))
        } else {
            let pt_per_in = units.point_size.per_inch() as f32;
            let v = |n: u32| widgets::fmt_num(f64::from(to_unit(n as f32, &unit, ppi, pt_per_in)));
            format!("{} x {} {unit} @ {} ppi", v(w), v(h), widgets::fmt_num(f64::from(ppi)))
        }
    }
}

/// The Recent category: the Clipboard preset, the documents recently created here (newest
/// first), then the built-in defaults they don't already cover.
fn recent_cards(f: &Map<String, Value>, recent: &[DocumentSettings], units: &UnitsAndRulers) -> Vec<Card> {
    let mut cards: Vec<Card> = clipboard_preset(f).map(Card::Builtin).into_iter().collect();
    cards.extend(recent.iter().cloned().map(Card::Recent));
    let defaults = CATEGORIES[0].1.iter().filter(|p| {
        let (_, w, h, ppi) = at_pref_resolutions(p, units);
        !recent.iter().any(|s| (s.width, s.height, s.resolution) == (w, h, ppi))
    });
    cards.extend(defaults.copied().map(Card::Builtin));
    cards
}

fn saved_presets(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> bool {
    let presets = app.session.presets.documents.clone();
    if presets.is_empty() {
        ui.label(tl!("No saved presets yet."));
        return false;
    }
    let mut create = false;
    egui::ScrollArea::vertical().id_salt("saved-document-presets").max_height(320.0).show(ui, |ui| {
        for preset in &presets {
            ui.push_id(&preset.name, |ui| {
                ui.horizontal(|ui| {
                    let selected = get_s(f, "__savedPreset", "") == preset.name;
                    let label = egui::Button::new(&preset.name).selected(selected).truncate();
                    let response = ui.add_sized(vec2(422.0, 32.0), label).on_hover_text(&preset.name);
                    if response.clicked() {
                        apply_saved_preset(f, preset);
                        // Touch-tolerant double-tap (finger jitter > egui's 6 px); see `widgets::second_click`.
                        create |= widgets::second_click(ui, &response);
                    }
                    if widgets::secondary_button(ui, tl!("Delete"), 76.0).clicked() {
                        match app.run("document.presets.delete", json!({"name":preset.name})) {
                            Ok(_) => {
                                f.remove("__presetError");
                                if selected {
                                    f.remove("__savedPreset");
                                }
                            }
                            Err(e) => {
                                f.insert("__presetError".into(), json!(e.to_string()));
                            }
                        }
                    }
                });
                let s = &preset.settings;
                small_label(ui, &format!("{} × {} px @ {} ppi · {}/{}", s.width, s.height, s.resolution, s.mode.to_uppercase(), s.depth));
                ui.add_space(8.0);
            });
        }
    });
    create
}

fn save_preset(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.add_space(12.0);
    if f.get("__savingPreset").and_then(Value::as_bool) != Some(true) {
        if widgets::secondary_button(ui, tl!("Save Preset…"), 240.0).clicked() {
            f.insert("__savingPreset".into(), json!(true));
            f.remove("__presetError");
        }
    } else {
        // Consume before TextEdit sees Enter, so naming can never confirm the outer dialog.
        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let label = ui.label(tl!("Preset Name"));
        let mut name = get_s(f, "__presetName", "");
        ui.add(egui::TextEdit::singleline(&mut name).id_salt("document-preset-name").desired_width(240.0).char_limit(255)).labelled_by(label.id);
        f.insert("__presetName".into(), json!(name));
        ui.horizontal(|ui| {
            if widgets::secondary_button(ui, tl!("Save"), 110.0).clicked() || enter {
                match app.run("document.presets.save", json!({"name":name,"settings":document_settings(f)})) {
                    Ok(_) => {
                        if let Some(preset) = app.session.presets.documents.last() {
                            apply_saved_preset(f, preset);
                        }
                        f.insert("__category".into(), json!("Saved"));
                        f.remove("__savingPreset");
                        f.remove("__presetName");
                        f.remove("__presetError");
                        ui.ctx().request_repaint();
                    }
                    Err(e) => {
                        f.insert("__presetError".into(), json!(e.to_string()));
                    }
                }
            }
            if widgets::secondary_button(ui, tl!("Cancel"), 110.0).clicked() {
                f.remove("__savingPreset");
                f.remove("__presetError");
            }
        });
    }
    if let Some(error) = f.get("__presetError").and_then(Value::as_str) {
        // The engine's message stays copyable, unlike the static labels around it.
        ui.add(egui::Label::new(error).wrap().selectable(true));
    }
}

/// Returns whether a preset double-click requested creation, after applying its settings.
/// A recent category click can make egui count the preset double-click as a triple-click.
pub fn body(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> bool {
    let t = Tokens::get(ui.ctx());
    // Headings and field labels are static, as in Photoshop: no text cursor over them, no
    // drag-selecting them (#8331). Text fields keep theirs.
    ui.style_mut().interaction.selectable_labels = false;
    let cat = get_s(f, "__category", "Recent");
    let mut create = false;
    // Category tabs.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for name in CATEGORIES.iter().map(|c| c.0).chain(["Saved"]) {
            let on = cat == name;
            let label = if name == "Saved" { tl!("Saved") } else { tl!(name) };
            let r = ui
                .add(egui::Label::new(RichText::new(label).size(13.0).color(if on { t.text } else { t.text_dim })).selectable(false).sense(Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if on {
                ui.painter().line_segment([r.rect.left_bottom() + vec2(0.0, 3.0), r.rect.right_bottom() + vec2(0.0, 3.0)], Stroke::new(2.0, t.text));
            }
            if r.clicked() {
                f.insert("__category".into(), json!(name));
            }
        }
    });
    ui.add_space(6.0);
    widgets::hairline(ui);
    ui.add_space(8.0);
    let units = app.session.prefs().units_and_rulers.clone();
    let presets: Vec<Card> = match CATEGORIES.iter().find(|c| c.0 == cat) {
        Some(c) if c.0 != CATEGORIES[0].0 => c.1.iter().copied().map(Card::Builtin).collect(),
        _ => recent_cards(f, &app.session.presets.recent_documents, &units),
    };
    let presets: Vec<Card> = presets
        .into_iter()
        .map(|c| match c {
            Card::Builtin(p) => Card::Builtin(oriented(&p, f)),
            recent => recent,
        })
        .collect();
    let chosen = get_s(f, "__preset", "");
    ui.horizontal_top(|ui| {
        // Left: preset grid.
        ui.vertical(|ui| {
            ui.set_width(520.0);
            if cat == "Saved" {
                create |= saved_presets(app, ui, f);
                return;
            }
            ui.label(RichText::new(crate::i18n::fmt(tl!("BLANK DOCUMENT PRESETS ({n})"), &[("n", &presets.len().to_string())])).size(11.0).color(t.text_faint));
            ui.add_space(6.0);
            let card = vec2(164.0, 112.0);
            // Up to 20 recent documents: scroll past three rows rather than grow the dialog.
            egui::ScrollArea::vertical().id_salt("new-document-presets").max_height(3.0 * (card.y + 8.0)).show(ui, |ui| {
                for row in presets.chunks(3) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        for c in row {
                            let (r, resp) = ui.allocate_exact_size(card, Sense::click());
                            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                            let (w, h, _) = c.size(&units);
                            let on = match c {
                                Card::Builtin(p) => chosen == p.0,
                                Card::Recent(settings) => holds(f, settings),
                            };
                            ui.painter().rect_filled(
                                r,
                                t.radius,
                                if on {
                                    t.row_selected
                                } else if resp.hovered() {
                                    t.hover
                                } else {
                                    t.field
                                },
                            );
                            if on {
                                ui.painter().rect_stroke(r, t.radius, Stroke::new(1.5, t.accent), StrokeKind::Inside);
                            }
                            page_icon(ui, Rect::from_center_size(pos2(r.center().x, r.top() + 34.0), vec2(40.0, 40.0)), w, h, &t);
                            let title = card_title(ui.painter(), tl!(c.title()), card.x - 12.0, t.text);
                            let elided = title.elided;
                            ui.painter().galley(pos2(r.center().x, r.top() + 70.0 - title.size().y / 2.0), title, t.text);
                            let size = c.size_line(&units, f);
                            ui.painter().text(pos2(r.center().x, r.top() + 97.0), Align2::CENTER_CENTER, &size, egui::FontId::proportional(10.5), t.text_faint);
                            let resp = match c {
                                // A recent document's tooltip names everything it restores.
                                Card::Recent(s) => {
                                    let background = match s.background.as_str() {
                                        "white" => tl!("White"),
                                        "black" => tl!("Black"),
                                        "transparent" => tl!("Transparent"),
                                        "backgroundColor" => tl!("Background Color"),
                                        hex => hex,
                                    };
                                    resp.on_hover_text(format!("{} · {size} · {}/{} · {background}", tl!(c.title()), s.mode.to_uppercase(), s.depth))
                                }
                                Card::Builtin(_) if elided => resp.on_hover_text(tl!(c.title())),
                                Card::Builtin(_) => resp,
                            };
                            if resp.clicked() {
                                match c {
                                    Card::Builtin(p) => apply_preset(f, p, &units),
                                    Card::Recent(settings) => apply_settings(f, settings),
                                }
                                // Touch-tolerant double-tap (finger jitter > egui's 6 px); see `widgets::second_click`.
                                create |= widgets::second_click(ui, &resp);
                            }
                        }
                    });
                    ui.add_space(8.0);
                }
            });
        });
        ui.add_space(10.0);
        // Right: Preset Details.
        ui.vertical(|ui| {
            ui.set_width(260.0);
            ui.label(RichText::new(tl!("PRESET DETAILS")).size(11.0).color(t.text_faint));
            ui.add_space(4.0);
            // Keep the canonical English default in state; only the TextEdit shows a translation.
            let raw = get_s(f, "name", "Untitled-1");
            let mut name = if raw == "Untitled-1" { tl!("Untitled-1").to_string() } else { raw };
            let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(250.0).font(egui::FontId::proportional(15.0))).accessible_name(tl!("Name"));
            if r.changed() {
                f.insert("name".into(), json!(name));
            }
            crate::field_tab::register(ui.ctx(), r.id);
            // Photoshop opens the dialog with the name selected, so typing replaces it and Tab goes
            // straight on to Width. (`__focused` marks that this instance has had its turn.)
            if !f.contains_key("__focused") {
                f.insert("__focused".into(), json!(true));
                crate::field_tab::focus(ui.ctx(), r.id);
            }
            if r.gained_focus() {
                crate::field_tab::select_all(ui.ctx(), r.id, &name);
            }
            ui.add_space(8.0);
            let ppi = get_f(f, "resolution", 72.0);
            let mut unit = get_s(f, "__unit", "px");
            // A typed unit (`210mm` while in pixels) converts at the new document's resolution.
            // Points and picas per Preferences › Point/Pica Size, as Image Size converts them.
            let pt_per_in = units.point_size.per_inch();
            let units = widgets::FieldUnits { unit: widgets::unit_of_key(&unit), dpi: f64::from(ppi), ppi: pt_per_in };
            let pt_per_in = pt_per_in as f32;
            small_label(ui, tl!("Width"));
            ui.horizontal(|ui| {
                let mut w = shown_size(f, "width", 1920.0, &unit, ppi, pt_per_in);
                if widgets::value_field_units(ui, &mut w, 0.0..=300_000.0, 110.0, widgets::unit_decimals(&unit), units).accessible_name(tl!("Width")).changed()
                {
                    set_size(f, "width", w, &unit, ppi, pt_per_in);
                }
                let opts: Vec<(String, &str)> = UNITS.iter().map(|u| (u.0.to_string(), u.1)).collect();
                if widgets::dropdown(ui, "nd-unit", tl!("Units"), &mut unit, &opts, 120.0) {
                    f.insert("__unit".into(), json!(unit));
                    f.remove("__savedPreset");
                }
            });
            small_label(ui, tl!("Height"));
            ui.horizontal(|ui| {
                let mut h = shown_size(f, "height", 1080.0, &unit, ppi, pt_per_in);
                if widgets::value_field_units(ui, &mut h, 0.0..=300_000.0, 110.0, widgets::unit_decimals(&unit), units).accessible_name(tl!("Height")).changed()
                {
                    set_size(f, "height", h, &unit, ppi, pt_per_in);
                }
                ui.add_space(6.0);
                // Page orientation (≠ Type › Orientation → Hướng chữ).
                small_label(ui, crate::i18n::tr_ctx(crate::i18n::current(), "document", "Orientation"));
                let (w, h) = (get_f(f, "width", 1920.0), get_f(f, "height", 1080.0));
                for (icon, portrait) in [("rectangle-vertical", true), ("rectangle-horizontal", false)] {
                    // Tooltip strings are looked up via `icons::button` → `tl!`; enum/state is aspect swap only.
                    if icons::button(ui, icon, 24.0, (h > w) == portrait, if portrait { "Portrait" } else { "Landscape" }).clicked() {
                        // The pick also turns the presets (`oriented`), even when the size already has it.
                        f.insert("__orientation".into(), json!(if portrait { "portrait" } else { "landscape" }));
                        if (h > w) != portrait {
                            swap_size(f);
                        }
                    }
                }
            });
            // Under Orientation, as in Photoshop: the document starts as one artboard (#9086).
            let mut artboard = f.get("artboard").and_then(Value::as_bool).unwrap_or(false);
            if widgets::checkbox(ui, &mut artboard, tl!("Artboards")).changed() {
                f.insert("artboard".into(), json!(artboard));
            }
            ui.add_space(4.0);
            small_label(ui, tl!("Resolution"));
            ui.horizontal(|ui| {
                let per_cm = get_s(f, "__resUnit", "in") == "cm";
                let mut r = if per_cm { ppi / 2.54 } else { ppi };
                if widgets::value_field(ui, &mut r, 1.0..=30_000.0, "", 110.0).accessible_name(tl!("Resolution")).changed() {
                    set_resolution(f, if per_cm { r * 2.54 } else { r });
                }
                let mut ru = get_s(f, "__resUnit", "in");
                if widgets::dropdown(
                    ui,
                    "nd-resunit",
                    tl!("Resolution"),
                    &mut ru,
                    &[("in".to_string(), tl!("Pixels/Inch")), ("cm".to_string(), tl!("Pixels/Centimeter"))],
                    120.0,
                ) {
                    f.insert("__resUnit".into(), json!(ru));
                    f.remove("__savedPreset");
                }
            });
            ui.add_space(4.0);
            small_label(ui, tl!("Color Mode"));
            ui.horizontal(|ui| {
                let mut mode = get_s(f, "mode", "rgb");
                if widgets::dropdown(
                    ui,
                    "nd-mode",
                    tl!("Color Mode"),
                    &mut mode,
                    &[
                        ("gray".to_string(), tl!("Grayscale")),
                        ("rgb".to_string(), tl!("RGB Color")),
                        ("cmyk".to_string(), tl!("CMYK Color")),
                        ("lab".to_string(), tl!("Lab Color")),
                    ],
                    110.0,
                ) {
                    f.insert("mode".into(), json!(mode));
                    // A profile of the old mode's colour space doesn't suit the new one.
                    f.remove("profile");
                    f.remove("__savedPreset");
                }
                let mut depth = f.get("depth").and_then(Value::as_u64).unwrap_or(8);
                let depth_options: Vec<(u64, &str, &str)> = DEPTH_OPTIONS.iter().map(|(bits, label, tooltip)| (*bits, *label, *tooltip)).collect();
                if widgets::dropdown_with_tooltips(ui, "nd-depth", tl!("Depth"), &mut depth, &depth_options, 120.0) {
                    f.insert("depth".into(), json!(depth));
                    f.remove("__savedPreset");
                }
            });
            ui.add_space(4.0);
            // Photoshop's Advanced Options › Color Profile (#321): the working space by default,
            // or another profile of the mode's colour space, built in or installed in the system.
            small_label(ui, tl!("Color Profile"));
            {
                let mode = get_s(f, "mode", "rgb");
                let space = color_mode_of(&mode).map(photocraft_engine::color_cmds::mode_space);
                let working = color_mode_of(&mode).map(|m| app.session.color.working(m).description.clone()).unwrap_or_default();
                let installed = photocraft_engine::installed_profiles::installed();
                let opts = profile_options(space, &working, &installed);
                let mut profile = get_s(f, "profile", "working");
                if !opts.iter().any(|(v, _)| *v == profile) {
                    // Not a profile of this mode (or no longer installed): Create must not send it.
                    profile = "working".into();
                    f.remove("profile");
                }
                let shown: Vec<(String, &str)> = opts.iter().map(|(v, l)| (v.clone(), l.as_str())).collect();
                if widgets::dropdown(ui, "nd-profile", tl!("Color Profile"), &mut profile, &shown, 240.0) {
                    if profile == "working" {
                        f.remove("profile");
                    } else {
                        f.insert("profile".into(), json!(profile));
                    }
                    f.remove("__savedPreset");
                }
            }
            ui.add_space(4.0);
            small_label(ui, tl!("Background Contents"));
            // A custom colour is kept as its "#rrggbb", which `file.new` takes as is (#4789).
            let current = get_s(f, "background", "white");
            let mut bg = if crate::color_picker_ui::parse_hex(&current).is_some() { CUSTOM.to_string() } else { current.clone() };
            let opts = [
                ("white".to_string(), tl!("White")),
                ("black".to_string(), tl!("Black")),
                ("backgroundColor".to_string(), tl!("Background Color")),
                ("transparent".to_string(), tl!("Transparent")),
                (CUSTOM.to_string(), tl!("Custom…")),
            ];
            let shown = background_rgb(app, f);
            let mut pick = false;
            ui.horizontal(|ui| {
                if widgets::dropdown(ui, "nd-bg", tl!("Background"), &mut bg, &opts, 240.0 - SWATCH - 6.0) {
                    if bg == CUSTOM {
                        // Custom… opens the Color Picker on the colour shown so far.
                        pick = true;
                    } else {
                        f.insert("background".into(), json!(bg));
                    }
                    f.remove("__savedPreset");
                    // A fresh choice of Background Color uses today's toolbox colour. Selecting a
                    // saved preset instead restores the colour captured when it was saved.
                    f.remove("backgroundColor");
                }
                // Photoshop's swatch beside the menu: the colour the canvas starts with; a click
                // opens the Color Picker, as Custom… does.
                let (r, resp) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
                match shown {
                    Some(c) => {
                        ui.painter().rect_filled(
                            r,
                            2.0,
                            egui::Color32::from_rgb((c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8),
                        );
                    }
                    None => widgets::checker(ui.painter(), r, 4.0),
                }
                ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, Tokens::get(ui.ctx()).field_border), StrokeKind::Inside);
                let resp = resp.on_hover_text(tl!("Custom background color")).on_hover_cursor(egui::CursorIcon::PointingHand);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Custom background color")));
                if resp.clicked() {
                    pick = true;
                    f.remove("__savedPreset");
                }
            });
            if pick {
                let start = shown.unwrap_or([1.0; 3]);
                f.insert("background".into(), json!(crate::color_picker_ui::hex(start)));
                f.remove("backgroundColor");
                crate::color_picker_ui::request_field(f, "background", &crate::color_picker_ui::title_for(tl!("Background Contents")));
            }
            save_preset(app, ui, f);
        });
    });
    create
/// The engine's stored settings as the dialog's options, without a name. A stored Background Color that carries its
/// captured colour (a preset saved by an earlier version) becomes that custom colour, so it keeps the colour it had.
#[allow(clippy::field_reassign_with_default)] // `DocumentOptions` is `#[non_exhaustive]`: it cannot be built with a struct expression here.
pub(crate) fn to_options(settings: &DocumentSettings) -> DocumentOptions {
    let mut options = DocumentOptions::default();
    options.size = PixelSize::new(settings.width, settings.height);
    options.pixels_per_inch = f64::from(settings.resolution);
    options.unit = match settings.unit.as_str() {
        "in" => UnitKind::Inches,
        "cm" => UnitKind::Centimetres,
        "mm" => UnitKind::Millimetres,
        "pt" => UnitKind::Points,
        "pica" => UnitKind::Picas,
        _ => UnitKind::Pixels,
    };
    options.resolution_unit = if settings.resolution_unit == "cm" { UnitKind::PixelsPerCentimetre } else { UnitKind::PixelsPerInch };
    options.color_mode = match settings.mode.as_str() {
        "bitmap" => DialogMode::Bitmap,
        "gray" | "grayscale" => DialogMode::Grayscale,
        "cmyk" => DialogMode::Cmyk,
        "lab" => DialogMode::Lab,
        _ => DialogMode::Rgb,
    };
    options.depth = match settings.depth {
        1 => BitDepth::One,
        16 => BitDepth::Sixteen,
        32 => BitDepth::ThirtyTwo,
        _ => BitDepth::Eight,
    };
    options.profile.clone_from(&settings.profile);
    options.background = match settings.background.as_str() {
        "black" => BackgroundContents::Black,
        "transparent" => BackgroundContents::Transparent,
        "backgroundColor" => settings.background_color.map_or(BackgroundContents::BackgroundColor, BackgroundContents::Custom),
        "white" => BackgroundContents::White,
        hex => crate::color_picker_ui::parse_hex(hex).map_or(BackgroundContents::White, BackgroundContents::Custom),
    };
    options
}

#[cfg(test)]
mod tests;
