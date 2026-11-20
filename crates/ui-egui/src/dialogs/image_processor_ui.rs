//! The File Type section of the Image Processor dialog. Its rows (`<format>/<option>`, one group per format) are
//! built with the rest of the form by `view_cmds::dialogs::params_for`; this module folds their values into the
//! per-format objects the command takes and refuses an OK with no format ticked.

use craft::host::{PickRequest, Picked};
use craft::theme::Variant;
use craft::ui::buttons::Button;
use craft::ui::contract::{LeastWidth, Props as _, Show as _, TooltipSpec, clicked};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, ParamDialog, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Map, Value};

use crate::PhotocraftApp;

/// The command id of the dialog.
pub(crate) const COMMAND: &str = "file.scripts.imageProcessor";

/// The formats the Image Processor saves.
const FORMATS: [&str; 3] = ["jpeg", "psd", "tiff"];

/// Folds the `<format>/<option>` values into the `{jpeg: {..}, psd: {..}, tiff: {..}}` objects the command takes.
pub(crate) fn nest(map: &mut Map<String, Value>) {
    for format in FORMATS {
        let prefix = format!("{format}/");
        let keys: Vec<String> = map.keys().filter(|key| key.starts_with(&prefix)).cloned().collect();
        let mut options = Map::new();
        for key in keys {
            if let Some(value) = map.remove(&key) {
                options.insert(key[prefix.len()..].to_owned(), value);
            }
        }
        if !options.is_empty() {
            map.insert(format.to_owned(), Value::Object(options));
        }
    }
}

/// Whether the Image Processor on the source `source` waits for the open Free Transform: document sources read
/// committed pixels, not the pending preview; Folder processing stays usable. The command (`run_command_at_depth`)
/// and the dialog's OK ([`check`]) both refuse by it.
pub(crate) fn waits_for_transform(app: &PhotocraftApp, source: Option<&str>) -> bool {
    app.ui.transform.is_some()
        && match source {
            Some("open") => true,
            Some("current") => app.transform_preview.as_ref().is_none_or(|preview| app.session.active().is_some_and(|st| st.doc.id == preview.doc.id)),
            _ => false,
        }
}

/// The `ParamFamily::check` of the dialog: OK is refused, and the dialog stays open, while a document source waits for
/// the open Free Transform or no file type is ticked.
pub(crate) fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    if waits_for_transform(app, result.fields.text("source")) {
        return Err(Label::from(tl!("Commit or cancel the transform first")));
    }
    if FORMATS.iter().any(|format| result.fields.bool(&format!("{format}/enabled")) == Some(true)) {
        Ok(())
    } else {
        Err(Label::from(tl!("You must save to at least one file type.")))
    }
}

#[derive(Clone, Copy)]
enum SettingsAction {
    Load,
    Save,
}

fn settings_action_id() -> egui::Id {
    egui::Id::unique("pc-image-processor-settings-action")
}

fn generation_id() -> egui::Id {
    egui::Id::unique("pc-image-processor-settings-generation")
}

pub(crate) fn settings_footer(dialog: ParamDialog<'_>) -> ParamDialog<'_> {
    dialog.ok_label(Label::from(tl!("Run"))).shaped(|dialog| {
        dialog.footer(|ui, _| {
            for (action, label, tip) in [
                (SettingsAction::Load, tl!("Load…"), tl!("Load a settings file from disk")),
                (SettingsAction::Save, tl!("Save…"), tl!("Save the current dialog settings to disk")),
            ] {
                let button = Button::new(label).variant(Variant::SECONDARY).least_width(LeastWidth::DialogButton).tooltip(TooltipSpec::new(tip));
                if clicked(&button.show(ui)) {
                    ui.ctx().data_mut(|data| data.insert_temp(settings_action_id(), action));
                }
            }
        })
    })
}

/// The dialog's id is reused when it reopens. A picker left over from a previous opening must not touch the new form.
pub(crate) fn settings_opened(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        let generation = data.get_temp_mut_or_default::<u64>(generation_id());
        *generation = generation.wrapping_add(1);
        data.remove::<SettingsAction>(settings_action_id());
    });
}

pub(crate) fn take_settings_action(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId) {
    let action = ctx.data_mut(|data| {
        let id = settings_action_id();
        let action = data.get_temp::<SettingsAction>(id)?;
        data.remove::<SettingsAction>(id);
        Some(action)
    });
    let Some(action) = action else { return };
    let result = pick_settings(app, id, action);
    if let Err(error) = result
        && error != crate::file_dialog::CANCELLED
    {
        crate::dialogs::alerts::alert(app, error);
    }
}

fn pick_settings(app: &mut PhotocraftApp, id: DialogId, action: SettingsAction) -> Result<Value, String> {
    const PURPOSE: &str = "imageProcessorSettings";
    let generation = app.ctx().data(|data| data.get_temp::<u64>(generation_id()));
    let (request, bytes) = match action {
        SettingsAction::Load => (PickRequest::open(PURPOSE).title(tl!("Pick an XML file to load")), None),
        SettingsAction::Save => {
            let Some(dialog) = app.framework.dialogs.get_by_id::<HostedParamDialog>(id) else { return Ok(Value::Null) };
            let request = PickRequest::save(PURPOSE).title(tl!("Pick an XML file to save")).file_name("Image Processor.xml").suggested_extension("xml");
            (request, Some(settings::encode(dialog.fields())?))
        }
    };
    app.ask_file(request.filter(tl!("XML Files (*.xml)"), &["xml"]), move |app, picked| {
        if app.ctx().data(|data| data.get_temp::<u64>(generation_id())) != generation || app.framework.dialogs.get_by_id::<HostedParamDialog>(id).is_none() {
            return Ok(Value::Null);
        }
        let result = match bytes {
            Some(bytes) => write_settings(app, picked, &bytes),
            None => load_settings(app, id, picked),
        };
        result.inspect_err(|error| {
            if error != crate::file_dialog::CANCELLED {
                crate::dialogs::alerts::alert(app, error.clone());
            }
        })
    })
}

fn load_settings(app: &mut PhotocraftApp, id: DialogId, picked: Picked) -> Result<Value, String> {
    let bytes = match picked {
        Picked::Contents(files) => files.into_iter().next().map(|(_, bytes)| bytes).ok_or(crate::file_dialog::CANCELLED)?,
        #[cfg(not(target_arch = "wasm32"))]
        Picked::Paths(paths) => {
            let path = paths.first().ok_or(crate::file_dialog::CANCELLED)?;
            photocraft_format::read_file_limited(path, settings::MAX_BYTES as u64).map_err(|error| format!("{}: {error}", path.display()))?
        }
        _ => return Err(tl!("Could not open the Image Processor settings file.").into()),
    };
    let fields = settings::decode(&bytes)?;
    if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id) {
        for (key, value) in fields.iter() {
            dialog.set_field(key, value.clone());
        }
    }
    Ok(Value::Null)
}

fn write_settings(app: &mut PhotocraftApp, picked: Picked, bytes: &[u8]) -> Result<Value, String> {
    let path = picked.first_path().ok_or(crate::file_dialog::CANCELLED)?;
    let path = path.to_str().ok_or(tl!("The settings file name is not valid text."))?;
    let write = app.services.write.as_mut().ok_or(tl!("Saving Image Processor settings is unavailable."))?;
    write(path, bytes)?;
    Ok(Value::Null)
}

mod settings;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "image_processor_settings_tests.rs"]
mod settings_tests;
