//! Edit › Color Settings, built as the reference application's dialog (#14227): the four groups
//! Working Spaces, Color Management Policies, Conversion Options and Advanced Controls, in two
//! columns, each with the reference's control names and in the reference's order. The controls
//! PhotoCraft's command has no parameter for (an Engine, scene-referred compensation) are left out,
//! not faked.
//!
//! The rows are built here rather than parsed from the command's params doc, so the captions are the
//! dialog's own words and the profile menus name their profiles by description, never by internal id.
//! The command's parameter ids stay what they are, and the dialog keeps the command as its key, so the
//! generic family still runs OK, `ui.dialog.set` and `ui.dialog.confirm` reach the same rows, the
//! values are remembered under the same scope, and the Settings menu's bookkeeping (#14229) still runs.
//!
//! Controls the reference dialog has no place for stay below its own: the Reset check box, the Monitor
//! Profile menu and the line naming the profile in use (#569). Engine and Settings-menu values the
//! reference measures in its own way keep today's controls.

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamLayout, set_param_field};
use craft::ui::runtime::env::Label;
use craft::ui::value_drag::SliderScale;
use photocraft_engine::color_cmds::ColorExt;
use photocraft_engine::installed_profiles::InstalledProfile;
use serde_json::Value;

use crate::PhotocraftApp;
use crate::filter_dialog::{COLOR_SETTINGS_COMMAND as COMMAND, color_settings_preset_row, color_settings_text_gamma_rows, installed_profile_choices};

/// The built-in working spaces each menu lists, in the params doc's order. An installed profile of the
/// same space follows them, by description (#6323).
const WORKING_RGB: [&str; 6] = ["srgb", "display-p3", "adobe-rgb-compat", "prophoto-compat", "linear-srgb", "rec2020"];
const WORKING_CMYK: [&str; 2] = ["coated-cmyk", "coated-fogra39"];
const WORKING_GRAY: [&str; 3] = ["dot-gain-20-compat", "sgray", "gray-gamma-2.2"];
/// The monitor profile menu: `auto` is the main display's profile, else a built-in RGB profile.
const MONITOR_PROFILES: [&str; 6] = ["auto", "srgb", "display-p3", "adobe-rgb-compat", "prophoto-compat", "rec2020"];
/// The rendering intents, in the reference dialog's order; the ids are the command's.
const INTENTS: [(&str, &str); 4] =
    [("perceptual", "Perceptual"), ("saturation", "Saturation"), ("relative", "Relative Colorimetric"), ("absolute", "Absolute Colorimetric")];
/// Blend Text Colors Using Gamma's range, as the params doc gives it (#14243); the rows replace it.
const GAMMA_MIN: f64 = 1.0;
const GAMMA_MAX: f64 = 3.0;
const GAMMA_DEFAULT: f64 = 1.45;
/// The Spot working space's dot gain, in percent at a 50 % dot, as the params doc gives it (#6787).
const SPOT_MIN: f64 = 0.0;
const SPOT_MAX: f64 = 49.0;
const SPOT_DEFAULT: f64 = 20.0;

/// A choice list from `(id, English label)` pairs; the label goes through the catalog.
fn opts(list: &[(&'static str, &'static str)]) -> Vec<(String, Label)> {
    list.iter().map(|(id, label)| ((*id).to_owned(), Label::from(tl!(label)))).collect()
}

/// The menu of a colour management policy, in the reference dialog's order. Only the last entry names
/// the space the policy converts to.
fn policy_options(space: &'static str) -> Vec<(String, Label)> {
    let mut options = vec![("off".to_owned(), Label::from(tl!("Off"))), ("preserve".to_owned(), Label::from(tl!("Preserve Embedded Profiles")))];
    options.push(("convert".to_owned(), Label::from(tl!("Convert to Working").to_string() + " " + space)));
    options
}

/// A built-in profile id as the dialog names it: the profile's description, never the id (#14227).
/// An id no built-in has is a path the command also takes, and its own description came from
/// [`installed_profile_choices`]; a path that is gone shows as it is, so the row keeps the value.
fn profile_label(id: &str) -> Label {
    match craft::cms::Builtin::from_id(id) {
        Some(builtin) => Label::from(builtin.description()),
        None => Label::from(id.to_owned()),
    }
}

/// A working-space menu: the built-in `ids` by description, then the profiles installed in the system
/// for that space, by their descriptions too.
fn space_options(ids: &[&str], key: &str, installed: &[InstalledProfile]) -> Vec<(String, Label)> {
    let mut options: Vec<(String, Label)> = ids.iter().map(|id| ((*id).to_owned(), profile_label(id))).collect();
    options.extend(installed_profile_choices(COMMAND, key, installed).into_iter().map(|(path, description)| (path, Label::from(description))));
    options
}

/// The Monitor Profile menu: `auto` reads as the reference's automatic choice, the rest by description.
fn monitor_options() -> Vec<(String, Label)> {
    MONITOR_PROFILES.iter().map(|id| ((*id).to_owned(), if *id == "auto" { Label::from(tl!("Auto")) } else { profile_label(id) })).collect()
}

/// The rows of the dialog: the Settings menu on top, then the reference's four groups in its order,
/// then the controls that are PhotoCraft's own. `app` supplies the monitor line's cell.
pub(crate) fn rows(app: &PhotocraftApp) -> Vec<Param> {
    let installed = photocraft_engine::installed_profiles::installed();
    let choice = |key: &'static str, label: &'static str, options: Vec<(String, Label)>| Param::new(key, tl!(label), ParamKind::choice(options, None));
    let mut list = vec![
        // The Settings menu (#14229): where the values came from, above the groups, as today.
        color_settings_preset_row(Label::from(tl!("Settings"))),
        Param::section(tl!("Working Spaces")),
        choice("workingRgb", "RGB", space_options(&WORKING_RGB, "workingRgb", &installed)),
        choice("workingCmyk", "CMYK", space_options(&WORKING_CMYK, "workingCmyk", &installed)),
        choice("workingGray", "Gray", space_options(&WORKING_GRAY, "workingGray", &installed)),
        // The reference's Spot entry is a menu of dot gains, which PhotoCraft has no list of yet; its
        // dot-gain field stays, under the reference's name for the row.
        Param::new("spotDotGain", tl!("Spot"), ParamKind::range(SPOT_MIN, SPOT_MAX, SPOT_DEFAULT, SliderScale::Linear)),
        Param::section(tl!("Color Management Policies")).beside(),
        choice("policyRgb", "RGB", policy_options("RGB")),
        choice("policyCmyk", "CMYK", policy_options("CMYK")),
        choice("policyGray", "Gray", policy_options("Gray")),
        Param::read_only_text("profileMismatches", Label::Empty, tl!("Profile Mismatches:")).full_width(),
        choice_bool("askOnMismatch", "Ask When Opening", true),
        choice_bool("askOnPaste", "Ask When Pasting", true),
        Param::read_only_text("missingProfiles", Label::Empty, tl!("Missing Profiles:")).full_width(),
        choice_bool("askOnMissing", "Ask When Opening", false),
        Param::section(tl!("Conversion Options")),
        choice("intent", "Intent", opts(&INTENTS)),
        choice_bool("bpc", "Use Black Point Compensation", true),
        choice_bool("dither", "Use Dither (8-bit/channel images)", true),
        Param::section(tl!("Advanced Controls")).beside(),
        Param::new("blendTextGamma", tl!("Blend Text Colors Using Gamma"), ParamKind::range(GAMMA_MIN, GAMMA_MAX, GAMMA_DEFAULT, SliderScale::Linear)),
        // PhotoCraft's own controls, below the reference's.
        Param::new("reset", tl!("Reset"), ParamKind::bool(false)),
        choice("monitorProfile", "Monitor Profile", monitor_options()),
    ];
    // The checkbox beside the gamma field, in place of the range row above (#14243).
    color_settings_text_gamma_rows(&mut list);
    // The monitor line: the cell holds the text each frame refreshes, which the build closure cannot read.
    let note = crate::monitor_status::note_cell(app);
    list.push(Param::custom("monitorNote", Label::Empty, move |ui, _, _| {
        let text = note.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
        ui.label(text)
    }));
    list
}

/// A check box row with the command's default ticked state.
fn choice_bool(key: &'static str, label: &'static str, default: bool) -> Param {
    Param::new(key, tl!(label), ParamKind::bool(default))
}

/// The values the dialog opens with: the stored Color Settings, each into the row that holds it.
/// A value its row will not take keeps the row's own.
fn opening_fields(app: &PhotocraftApp, params: &[Param]) -> DialogFields {
    let mut fields = DialogFields::new();
    let Ok(stored) = serde_json::to_value(&app.session.color().settings) else { return fields };
    for (key, value) in stored.as_object().into_iter().flatten() {
        let _ = set_param_field(params, &mut fields, key, value.clone());
    }
    fields
}

/// The values Alt+Cancel restores: the rows' defaults, then what the dialog opened with.
fn reset_fields(params: &[Param], opening: &DialogFields) -> DialogFields {
    let mut reset = DialogFields::new();
    for p in params {
        if let Some(value) = p.kind.default_value() {
            reset.set(p.key.clone(), value);
        }
    }
    for (key, value) in opening.iter() {
        reset.set(key, value.clone());
    }
    reset
}

/// Opens Edit › Color Settings on the current settings and the monitor profile read just now.
pub(crate) fn open(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    let spec = photocraft_engine::commands::find(COMMAND).ok_or_else(|| format!("{COMMAND} has no dialog"))?;
    let title = crate::filter_dialog::title_of(spec.label);
    let params = rows(app);
    let fields = opening_fields(app, &params);
    let reset = reset_fields(&params, &fields);
    let key = COMMAND.to_owned();
    Ok(crate::dialogs::params::open_param(app, COMMAND, fields, Value::Null, move |fields| {
        ParamDialog::command(key.clone(), title.clone(), params.clone())
            .values(Bind::bound(fields))
            .reset_to(reset.clone())
            // The reference dialog's shape: its option groups as groups, captions before their controls.
            .layout(ParamLayout::Document)
            .alt_resets(true)
            .alert_title(tl!("PhotoCraft"))
    }))
}

#[cfg(test)]
#[path = "color_settings_layout_tests.rs"]
mod layout_tests;
