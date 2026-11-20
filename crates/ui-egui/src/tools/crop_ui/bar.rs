//! The Crop tool's options as the framework `CropBar`, and the preset list of the task bar. One handler turns the cells'
//! outputs into app calls.

use craft::draw::ui::crop::{CropBar, CropPresetList, CropRatioChange};
use craft::ui::dialog::{Alert, HostedAlert};
use craft::ui::runtime::env::Env;

use super::{document, handle_event, refit, reset, toggle_straighten, words};
use crate::PhotocraftApp;
use crate::state::Tool;

/// The Crop options inside the options bar's `Bar`.
pub(crate) fn bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(document) = document(app) else { return };
    let mut settings = app.ui.tool_options.crop.clone();
    let users = crate::crop_presets::user_presets(app);
    let (armed, modified, entry) = {
        let tool = &app.canvas_tools.crop.tool;
        (tool.straighten_armed(), tool.modified(), tool.shows_selection_entry(&settings))
    };
    let words = words(app);
    let out = CropBar::new(&mut settings, &document)
        .user_presets(&users)
        .straighten(armed)
        .selection_entry(entry)
        .selection_chosen(entry)
        .can_reset(modified)
        .help(words.fill_help.clone())
        .show(ui);
    if out.settings_changed || out.ratio.is_some() || out.selection {
        app.ui.tool_options.crop = settings.clone();
    }
    preset_outputs(app, out.ratio, out.new_preset, out.delete_preset, out.selection);
    if out.straighten {
        toggle_straighten(app);
    }
    if out.pick_color.is_some() {
        let rgb = settings.shield.custom_color.map(|v| f32::from(v) / 255.0);
        if let Err(message) = crate::color_picker_ui::open_for_target(app, "cropShield", "Color Picker (Crop Shield Color)", rgb) {
            crate::notices::error(app, message);
        }
    }
    if out.reset {
        reset(app);
    }
    if out.help {
        let message = format!(
            "{} {}\n{} {}",
            words.fill_help_background.as_str(),
            words.fill_help_background_text.as_str(),
            words.fill_help_content_aware.as_str(),
            words.fill_help_content_aware_text.as_str()
        );
        let alert = Alert::new(words.fill_help_title.clone(), message);
        // The alert takes the Escape of the pass it opens in, as an open dialog takes the one that closes it: the
        // canvas, drawn after the bar, would otherwise cancel the pending crop with it.
        if app.framework.dialogs.open_unless_open_in(ui.ctx(), HostedAlert::new(alert)).is_ok() {
            crate::window_keys::take_escape(ui.ctx());
        }
    }
    if let Some(range) = out.out_of_range {
        let alert = Alert::range(tl!("PhotoCraft").to_owned(), &range, &Env::of(ui));
        // An `Err` is the alert that was not opened because one is already up.
        let _ = app.framework.dialogs.open_unless_open(HostedAlert::new(alert));
    }
}

/// The task bar's preset list.
pub(crate) fn preset_list(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(document) = document(app) else { return };
    let mut settings = app.ui.tool_options.crop.clone();
    let users = crate::crop_presets::user_presets(app);
    let entry = app.canvas_tools.crop.tool.shows_selection_entry(&settings);
    let out = CropPresetList::new(&mut settings, &document).user_presets(&users).selection_entry(entry).selection_chosen(entry).show(ui);
    if out.ratio.is_some() || out.selection {
        app.ui.tool_options.crop = settings;
    }
    preset_outputs(app, out.ratio, out.new_preset, out.delete_preset, out.selection);
}

/// What a preset pick asks of the app, for the bar and the task bar alike.
fn preset_outputs(app: &mut PhotocraftApp, ratio: Option<CropRatioChange>, new_preset: bool, delete_preset: bool, selection: bool) {
    if let Some(change) = ratio {
        refit(app, change.editing);
    }
    if selection {
        let event = app.canvas_tools.crop.tool.take_selection_frame();
        handle_event(app, event);
    }
    if new_preset {
        crate::crop_presets::open_new(app);
    }
    if delete_preset {
        crate::crop_presets::open_delete(app);
    }
}

/// A key of what the bar shows of the Crop tool: the options, Straighten, Reset, the Selection entry and the ✓ and ⊘
/// cells. The task bar's preset list, Straighten and Cancel, the tool's keys and a drag of the frame change them
/// without a command, so the options bar's content version holds this key. The options are hashed through their
/// serialized form, which holds every stored field.
pub(crate) fn bar_key(app: &PhotocraftApp) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    if app.ui.tool != Tool::Crop {
        return None;
    }
    struct HashWriter<'a>(&'a mut std::collections::hash_map::DefaultHasher);
    impl std::io::Write for HashWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.write(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let settings = &app.ui.tool_options.crop;
    serde_json::to_writer(HashWriter(&mut hasher), settings).ok()?;
    let tool = &app.canvas_tools.crop.tool;
    (tool.straighten_armed(), tool.modified(), tool.shows_selection_entry(settings), commits_shown(app)).hash(&mut hasher);
    Some(hasher.finish())
}

/// The ✓ and ⊘ cells show while a crop is pending or the frame was edited.
pub(crate) fn commits_shown(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && (super::pending(app) || app.canvas_tools.crop.tool.modified())
}

/// ✓ crops, ⊘ discards.
pub(crate) fn committed(app: &mut PhotocraftApp, apply: bool) {
    if apply {
        super::apply(app);
    } else {
        super::discard(app);
    }
}
