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
use photocraft_engine::color_cmds::ColorExt;
use photocraft_engine::document_preset_cmds::DocumentSettings;
use photocraft_engine::presets::PresetsExt;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The SAVED-tier key of the last created settings (without their name): the next New Document opens on them.
pub(crate) const LAST_CREATED: SaveKey = SaveKey::new("photocraft.file.new");

/// The SAVED-tier key of the dialog's place and size.
const WINDOW: SaveKey = SaveKey::new("photocraft.file.new.window");

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
    .recent(app.session.presets().recent_documents.iter().map(to_options).collect());
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
    for preset in &app.session.presets().documents {
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
            ColorProfiles::new(*dialog_mode, app.session.color().working(*mode).description.clone(), others)
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
