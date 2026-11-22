//! The hosted-dialog glue: one pass of the framework's dialog host per frame ([`run`]), the
//! results it hands back ([`take_results`]: parameter dialogs through [`params`], prompts through
//! [`PROMPTS`], every other family through [`TAKERS`]), the command opener
//! ([`open_command_dialog`]) and the one-time move of the dialogs' remembered values
//! ([`move_memories`]). Each dialog family owns its module below; this file only routes to it.

use craft::host::Host;
use craft::settings::Remembered;
use craft::ui::dialog::{DialogId, DialogResult, HeldOk, PromptAnswer, StandardPrompt};
use photocraft_doc::DocId;
use photocraft_engine::prefs::Preferences;
use serde_json::Value;

use crate::PhotocraftApp;

/// What a family of the hosted dialogs does with each result, and who owns which dialog key.
pub(crate) mod params {
    use std::path::{Path, PathBuf};

    use craft::host::{FileFilter, PickId, PickRequest};
    use craft::ui::contract::Bind;
    use craft::ui::dialog::{DialogId, DialogResult, StackDialog, dialog_id};
    use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamResult};
    use craft::ui::runtime::env::Label;
    use craft::ui::runtime::memory::SaveKey;
    use craft::ui::runtime::requests::{PathKind, PathRequest};
    use photocraft_doc::DocId;
    use serde_json::Value;

    use super::{
        PhotocraftApp, alerts, brush_preset_dialog, canvas_input, channel_options_ui, color_picker_ui, color_table_ui, count_group_name_ui, duplicate_layer_ui,
        equalize_ui, file_forms, fill_ui, filter_dialog, history_options_ui, image_processor_ui, merge_channels_ui, new_group_ui, new_layer_ui,
        smart_filter_dialog, snapshot_dialog, stack_dialog, stroke_ui,
    };
    use crate::canvas::document_display::CanvasHost;

    /// Judges a held OK: a refusal's label, or `Ok`.
    type OkCheck = fn(&mut PhotocraftApp, &ParamResult) -> Result<(), Label>;

    /// One family of parameter dialogs: the dialogs whose key it owns, and what it does with their answers.
    #[derive(Clone, Copy)]
    pub(crate) struct ParamFamily {
        /// Whether the family opens the dialog with this key. The first family that owns a key wins.
        pub owns: fn(&str) -> bool,
        /// What OK does: runs the family's command with the confirmed values. The family reads its per-open state with
        /// [`captured`].
        pub run: fn(&mut PhotocraftApp, &egui::Context, ParamResult),
        /// Judges a held OK before the dialog closes; a refusal shows under the rows and keeps the dialog open.
        pub check: Option<OkCheck>,
        /// The files of a folder, for a files row of the family's dialogs.
        pub listing: Option<fn(&PhotocraftApp, &Path) -> Vec<PathBuf>>,
        /// A footer button the dialog reported (the button's id comes last).
        pub footer: Option<fn(&mut PhotocraftApp, &egui::Context, DialogId, &str)>,
        /// What a cancelled dialog of the family leaves to undo (a live preview, a guide edit).
        pub cancel: Option<fn(&mut PhotocraftApp, &egui::Context, DialogId)>,
        /// The document a dialog is about, read from its captured value: the dialog closes with the document.
        pub document: Option<fn(&Value) -> Option<DocId>>,
    }

    impl ParamFamily {
        /// A family with only the two required hooks.
        pub(crate) const fn new(owns: fn(&str) -> bool, run: fn(&mut PhotocraftApp, &egui::Context, ParamResult)) -> Self {
            ParamFamily { owns, run, check: None, listing: None, footer: None, cancel: None, document: None }
        }
    }

    /// Every family of parameter dialogs, first match wins; the generic filter family owns what is left.
    pub(crate) const PARAM_FAMILIES: &[ParamFamily] = &[
        ParamFamily { check: Some(fill_ui::check), ..ParamFamily::new(fill_ui::owns, fill_ui::run) },
        ParamFamily { check: Some(stroke_ui::check), ..ParamFamily::new(stroke_ui::owns, stroke_ui::run) },
        ParamFamily {
            check: Some(smart_filter_dialog::check),
            cancel: Some(filter_dialog::cancel),
            document: Some(smart_filter_dialog::document),
            ..ParamFamily::new(smart_filter_dialog::owns, smart_filter_dialog::run)
        },
        ParamFamily::new(new_layer_ui::owns, new_layer_ui::run),
        ParamFamily::new(new_group_ui::owns, new_group_ui::run),
        ParamFamily { check: Some(count_group_name_ui::check), ..ParamFamily::new(count_group_name_ui::owns, count_group_name_ui::run) },
        ParamFamily { check: Some(duplicate_layer_ui::check), ..ParamFamily::new(duplicate_layer_ui::owns, duplicate_layer_ui::run) },
        ParamFamily::new(snapshot_dialog::owns, snapshot_dialog::run),
        ParamFamily::new(history_options_ui::owns, history_options_ui::run),
        ParamFamily::new(brush_preset_dialog::owns, brush_preset_dialog::run),
        ParamFamily { check: Some(equalize_ui::check), ..ParamFamily::new(equalize_ui::owns, equalize_ui::run) },
        ParamFamily::new(channel_options_ui::owns, channel_options_ui::run),
        ParamFamily { check: Some(merge_channels_ui::check), ..ParamFamily::new(merge_channels_ui::owns, merge_channels_ui::run) },
        ParamFamily {
            check: Some(filter_dialog::check),
            footer: Some(color_table_ui::footer_action),
            cancel: Some(filter_dialog::cancel),
            ..ParamFamily::new(|key| key == "image.mode.colorTable", filter_dialog::run)
        },
        ParamFamily { cancel: Some(filter_dialog::cancel), ..ParamFamily::new(file_forms::owns, filter_dialog::run) },
        ParamFamily { listing: Some(stack_dialog::listing), ..ParamFamily::new(stack_dialog::owns, stack_dialog::run) },
        ParamFamily {
            check: Some(crate::canvas_tool_menu::pen_dialogs::check),
            ..ParamFamily::new(crate::canvas_tool_menu::pen_dialogs::owns, crate::canvas_tool_menu::pen_dialogs::run)
        },
        ParamFamily::new(crate::analysis::dialogs::owns, crate::analysis::dialogs::run),
        ParamFamily::new(crate::layers_panel_ui::owns, crate::layers_panel_ui::run),
        ParamFamily {
            check: Some(image_processor_ui::check),
            cancel: Some(filter_dialog::cancel),
            ..ParamFamily::new(|key| key == image_processor_ui::COMMAND, filter_dialog::run)
        },
        ParamFamily { cancel: Some(filter_dialog::cancel), ..ParamFamily::new(crate::view_cmds::dialogs::is_form, filter_dialog::run) },
        crate::prefs_ui::PRESETS_FAMILY,
        ParamFamily { check: Some(crate::crop_presets::check), ..ParamFamily::new(crate::crop_presets::owns, crate::crop_presets::run) },
        crate::actions::dialogs::NEW_SET_FAMILY,
        crate::actions::dialogs::INSERT_STOP_FAMILY,
        crate::actions::playback::FAMILY,
        crate::actions::menu_item::FAMILY,
        crate::smart_svg_ui::family(),
        ParamFamily { check: Some(crate::workspace_ui::check_param), ..ParamFamily::new(crate::workspace_ui::owns_param, crate::workspace_ui::run_param) },
        ParamFamily { check: Some(filter_dialog::check), cancel: Some(filter_dialog::cancel), ..ParamFamily::new(filter_dialog::owns, filter_dialog::run) },
    ];

    /// The family that owns the dialog `key`.
    fn family_for(key: &str) -> Option<&'static ParamFamily> {
        PARAM_FAMILIES.iter().find(|family| (family.owns)(key))
    }

    /// The scope a family's dialog remembers its values under, and the scope [`move_memories`] moves the old value to.
    pub(super) fn memory_scope(key: &str) -> String {
        format!("photocraft.{key}")
    }

    /// Opens the parameter dialog `key` over `fields`, built by `build`. The dialog edits its own history (Cmd+Z steps
    /// its values), remembers its values under [`memory_scope`] and lets its colour rows sample the active document's
    /// canvas. `captured` is the per-open state the family needs when the answer comes back (Smart Filter's target,
    /// Duplicate's layer; `Value::Null` for none): it stays in the app until [`take`] has run the family on the result or
    /// the dialog was cancelled, and the family reads it with [`captured`].
    pub(crate) fn open_param(
        app: &mut PhotocraftApp,
        key: &str,
        fields: DialogFields,
        captured: Value,
        build: impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static,
    ) -> DialogId {
        open_with(app, key, fields, captured, Some(SaveKey::named(memory_scope(key))), build)
    }

    /// [`open_param`] for a dialog that remembers nothing (a spec dialog, a smart edit).
    pub(crate) fn open_param_unsaved(
        app: &mut PhotocraftApp,
        key: &str,
        fields: DialogFields,
        captured: Value,
        build: impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static,
    ) -> DialogId {
        open_with(app, key, fields, captured, None, build)
    }

    /// The one body of the two openers: `save` is the remember key, when the dialog has one.
    fn open_with(
        app: &mut PhotocraftApp,
        key: &str,
        fields: DialogFields,
        captured: Value,
        save: Option<SaveKey>,
        build: impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static,
    ) -> DialogId {
        let canvas = app.session.active_index().map(|idx| CanvasHost::primary(app, idx).canvas_id());
        let dialog = HostedParamDialog::new(app.ctx(), key, fields, move |fields| {
            let mut dialog = build(fields).history(true);
            if let Some(save) = &save {
                dialog = dialog.remember(save.clone());
            }
            match canvas {
                Some(canvas) => dialog.canvas(canvas),
                None => dialog,
            }
        });
        // A family without a check closes at OK; one with a check holds OK until `answer_held` judged it.
        let dialog = if family_for(key).is_some_and(|family| family.check.is_some()) { dialog.held_ok() } else { dialog };
        app.framework.captured.insert(key.to_owned(), captured);
        app.framework.dialogs.open(dialog)
    }

    /// The per-open state the dialog `key` was opened with, `Value::Null` when it had none.
    pub(crate) fn captured(app: &PhotocraftApp, key: &str) -> Value {
        app.framework.captured.get(key).cloned().unwrap_or(Value::Null)
    }

    /// Runs the families on the parameter dialogs that finished, then answers the held OKs and the requests the open
    /// dialogs filed. A result is taken once, so a second pass of a frame finds nothing to run.
    pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
        while let Some((result, key)) = app.framework.dialogs.take_keyed::<HostedParamDialog>() {
            let key = key.unwrap_or_default();
            let family = family_for(&key);
            let id = match &result {
                DialogResult::Ok(result) => result.id,
                _ => dialog_id(key.as_str()),
            };
            match result {
                DialogResult::Ok(result) => {
                    if let Some(family) = family {
                        (family.run)(app, ctx, result);
                    }
                }
                _ => {
                    if let Some(cancel) = family.and_then(|family| family.cancel) {
                        cancel(app, ctx, id);
                    }
                }
            }
            // The dialog is gone: its eyedropper and its file pickers have nobody to answer.
            canvas_input::disarm_dropper(app);
            drop_picks(app, ctx, id);
            app.framework.captured.remove(&key);
        }
        answer_held(app);
        answer_requests(app, ctx);
    }

    /// The ids of the open hosted parameter dialogs.
    fn hosted(app: &PhotocraftApp) -> Vec<DialogId> {
        let dialogs = &app.framework.dialogs;
        dialogs.stack().ids().filter(|id| dialogs.get_by_id::<HostedParamDialog>(*id).is_some()).collect()
    }

    fn dialog_mut(app: &mut PhotocraftApp, id: DialogId) -> Option<&mut HostedParamDialog> {
        app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(id)
    }

    fn key_of(app: &PhotocraftApp, id: DialogId) -> Option<String> {
        app.framework.dialogs.get_by_id::<HostedParamDialog>(id).and_then(|dialog| StackDialog::key(dialog).map(str::to_owned))
    }

    /// Judges the OK each held dialog filed with its family's `check` (no check accepts) and answers the dialog.
    fn answer_held(app: &mut PhotocraftApp) {
        for id in hosted(app) {
            let Some(result) = dialog_mut(app, id).and_then(HostedParamDialog::take_ok) else { continue };
            let answer = if crate::actions::is_editor(app, id) {
                // An editor of Record Again or of a paused menu step: its answer is judged as the step requires, with the
                // values OK runs.
                let params = crate::filter_dialog::command_params(&result.key, &result.fields);
                crate::actions::check_editor(app, id, &result.key, &params).map_err(Label::from)
            } else {
                match family_for(&result.key).and_then(|family| family.check) {
                    Some(check) => check(app, &result),
                    None => Ok(()),
                }
            };
            if let Some(dialog) = dialog_mut(app, id) {
                dialog.answer_ok(answer);
            }
        }
    }

    /// The file pickers started for a dialog and not answered yet, in the frame's memory: a pick answers on a later pass.
    #[derive(Clone, Default)]
    struct Picks(Vec<(PickId, DialogId, String)>);

    fn picks_id() -> egui::Id {
        egui::Id::unique("pc-param-picks")
    }

    /// Answers the dialogs' requests of this pass: colours through a Color Picker child, paths through the host's file
    /// service, the eyedropper through the canvas, listings and footer buttons through the family. Each request is filed
    /// by the pass that took the input, so a second pass of the frame answers nothing twice.
    pub(crate) fn answer_requests(app: &mut PhotocraftApp, ctx: &egui::Context) {
        settle_picks(app, ctx);
        for id in hosted(app) {
            let key = key_of(app, id);
            let Some(dialog) = dialog_mut(app, id) else { continue };
            let (colour, path, dropper, listing, footer, range) = (
                dialog.pick_color(),
                dialog.pick_path(),
                dialog.take_eyedropper(),
                dialog.take_listing(),
                dialog.take_footer_action(),
                dialog.take_range_alert(),
            );
            if let Some(alert) = range {
                let _ = alerts::show(app, alert);
            }
            if let Some(request) = colour {
                color_picker_ui::open_for_request(app, id, request);
            }
            if let Some((row, colour)) = color_picker_ui::answer_for(app, id)
                && let Some(dialog) = dialog_mut(app, id)
            {
                dialog.answer_color(row, colour);
            }
            if let Some(request) = path {
                start_pick(app, ctx, id, request);
            }
            if let Some(request) = dropper {
                canvas_input::arm_dropper(app, id, request.key);
            }
            let family = key.as_deref().and_then(family_for);
            if let (Some(request), Some(listing)) = (listing, family.and_then(|family| family.listing)) {
                let files = listing(app, &request.folder);
                if let Some(dialog) = dialog_mut(app, id) {
                    dialog.answer_listing(request.key, files);
                }
            }
            if let (Some(action), Some(footer)) = (footer, family.and_then(|family| family.footer)) {
                footer(app, ctx, id, &action);
            }
        }
    }

    /// Starts the picker a path row asked for, in the folder the row's current path names.
    fn start_pick(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, request: PathRequest) {
        const PURPOSE: &str = "dialog";
        let current = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).and_then(|dialog| match dialog.fields().get(&request.key) {
            Some(FieldValue::Text(text)) if !text.is_empty() => Some(PathBuf::from(text)),
            _ => None,
        });
        let start = current.and_then(|path| if path.is_dir() { Some(path) } else { path.parent().map(Path::to_path_buf) }).filter(|dir| dir.is_dir());
        let mut pick = match &request.kind {
            PathKind::SaveFile => {
                let mut pick = PickRequest::save(PURPOSE);
                for filter in &request.filters {
                    let extensions: Vec<&str> = filter.extensions.iter().map(String::as_str).collect();
                    pick.filters.push(FileFilter::new(filter.name.clone(), &extensions));
                }
                if let Some(name) = &request.file_name {
                    pick = pick.file_name(name.clone());
                }
                pick
            }
            PathKind::OpenFile { extensions, .. } => {
                let mut pick = PickRequest::open(PURPOSE);
                if !extensions.is_empty() {
                    let refs: Vec<&str> = extensions.iter().map(String::as_str).collect();
                    pick = pick.filter(extensions.join(", "), &refs);
                }
                pick
            }
            _ => PickRequest::folder(PURPOSE),
        };
        if let Some(dir) = start {
            pick = pick.directory(dir);
        }
        let ticket = app.framework.host().files().pick(pick);
        ctx.data_mut(|data| data.get_temp_mut_or_default::<Picks>(picks_id()).0.push((ticket, id, request.key)));
    }

    /// Hands each answered pick to its dialog. A cancelled or unavailable picker answers nothing; a pick of a dialog that
    /// closed is forgotten.
    fn settle_picks(app: &mut PhotocraftApp, ctx: &egui::Context) {
        let waiting = ctx.data_mut(|data| std::mem::take(&mut data.get_temp_mut_or_default::<Picks>(picks_id()).0));
        if waiting.is_empty() {
            return;
        }
        let host = app.framework.host().clone();
        let mut still = Vec::new();
        for (ticket, id, key) in waiting {
            if dialog_mut(app, id).is_none() {
                host.files().forget(ticket);
                continue;
            }
            match host.files().take_answer(ticket) {
                None => still.push((ticket, id, key)),
                Some(picked) => {
                    if let (Some(path), Some(dialog)) = (picked.first_path(), dialog_mut(app, id)) {
                        dialog.answer_path(key, path);
                    }
                }
            }
        }
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<Picks>(picks_id()).0.splice(0..0, still);
        });
    }

    /// Forgets the picks of the dialog `id`, which closed.
    fn drop_picks(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId) {
        let waiting = ctx.data_mut(|data| std::mem::take(&mut data.get_temp_mut_or_default::<Picks>(picks_id()).0));
        if waiting.is_empty() {
            return;
        }
        let host = app.framework.host().clone();
        let (closed, kept): (Vec<_>, Vec<_>) = waiting.into_iter().partition(|(_, dialog, _)| *dialog == id);
        for (ticket, ..) in closed {
            host.files().forget(ticket);
        }
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<Picks>(picks_id()).0.splice(0..0, kept);
        });
    }

    /// A bare command form: a dialog without rows that the generic filter family runs.
    pub(super) fn open_bare(app: &mut PhotocraftApp, command: &str, label: &str) -> DialogId {
        let (key, title) = (command.to_owned(), label.to_owned());
        open_param(app, command, DialogFields::new(), Value::Null, move |fields| {
            ParamDialog::command(key.clone(), title.clone(), Vec::<Param>::new()).values(Bind::bound(fields))
        })
    }
}

/// One entry of [`PROMPTS`]: answers the prompt when the answer's tag is its own, and says whether it did.
pub(crate) type PromptRow = fn(&mut PhotocraftApp, &egui::Context, &mut PromptAnswer) -> bool;

/// The prompts' answers, one row per family: the first row whose tag type matches takes the answer (A11). A prompt
/// family gives an `answer` function, never a `take`: the host keeps one plain result per dialog type, so a second taker
/// would take another family's answer.
pub(crate) const PROMPTS: &[PromptRow] = &[
    crate::tools::tool_reset::answer,
    crate::analysis::note_alerts::answer,
    delete_layer_prompt::answer,
    mask_trash_prompt::answer,
    disk_change_prompt::answer,
    rasterize_prompt::answer,
    live_shape_prompt::answer,
    preset_group_prompt::answer,
    open_prompt::answer_prompt,
    alerts::answer,
    crate::jobs_ui::answer,
    crate::gpu_status::answer,
    crate::actions::library::answer,
];

/// [`prompts_take`] over the rows `rows`.
fn take_prompts_from(app: &mut PhotocraftApp, ctx: &egui::Context, rows: &[PromptRow]) {
    while let Some(result) = app.framework.dialogs.take::<StandardPrompt>() {
        // Only an `Ok` carries an answer: a host Cancel (a replaced prompt, a parent closed over it) reaches no row.
        let DialogResult::Ok(mut answer) = result else { continue };
        for row in rows {
            if row(app, ctx, &mut answer) {
                break;
            }
        }
    }
}

/// Hands each prompt answer to the family it is for.
pub(crate) fn prompts_take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    take_prompts_from(app, ctx, PROMPTS);
}

/// The families that take their own results (one `take(app, ctx)` each), in order.
pub(crate) const TAKERS: &[fn(&mut PhotocraftApp, &egui::Context)] = &[
    alerts::take,
    discard_ui::take,
    open_prompt::take,
    credits::take,
    adjust_dialog::take,
    layer_style::take,
    color_picker_ui::take,
    crate::type_panels_ui::color_picker::take,
    crate::type_panels_ui::spelling::take,
    color_range_ui::take,
    select_mask_ui::take,
    caf_ui::take,
    liquify_ui::take,
    gallery_ui::take,
    wide_angle_ui::take,
    vp_ui::take,
    camera_raw_ui::take,
    new_doc_ui::take,
    sizing::take,
    shape_dialog::take,
    guide_layout_ui::take,
    new_guide_ui::take,
    export_dialog::take,
    file_ui::take,
    format_prompts::take,
    pattern_fill_ui::take,
    crate::workspace_ui::take,
    crate::kys_import::take,
    crate::prefs_ui::take_presets,
    crate::prefs_ui::take_window,
    crate::layers_panel_ui::take,
    crate::slice_ui::take,
    crate::crop_presets::take,
    crate::panels::apply_data_set_ui::take,
    crate::variables_ui::take,
    crate::actions::dialogs::take_stops,
];

/// Runs the answers of every dialog that finished: parameter dialogs, prompts, then each family's own results. It runs
/// after every host pass and at once after [`confirm`], [`cancel`] and an automation edit, so an answer acts inside the
/// request that caused it.
pub(crate) fn take_results(app: &mut PhotocraftApp, ctx: &egui::Context) {
    params::take(app, ctx);
    prompts_take(app, ctx);
    for taker in TAKERS {
        taker(app, ctx);
    }
}

/// One pass of the dialogs: the guide edit follows its dialog, the palette shows, the filter preview and the Preferences window are fed, the
/// framework's host runs every open dialog, and the results are taken. A frame that runs a second pass runs this again;
/// the host applies only a pass's own input and a result is taken once, so nothing acts twice.
pub fn run(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if crate::guide_edit::sync(app) {
        ui.ctx().request_repaint();
    }
    crate::type_panels_ui::color_picker::check(app, ui.ctx());
    palette::show(app, ui);
    crate::filter_dialog::reconcile_match_color(app);
    filter_preview_ui::feed(app, ui.ctx());
    crate::prefs_ui::feed(app);
    app.framework.dialogs.run(ui);
    take_results(app, &ui.ctx().clone());
}

/// Answers dialog `id` with OK, as its default button would, and runs what OK does. A held OK is answered at once by the
/// family that holds it: an accepted one closes the dialog in this call, a refusal comes back as the error text, and one
/// that waits on a pick (Export's save dialog) is answered by the pick.
pub fn confirm(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId) -> Result<(), String> {
    if app.framework.dialogs.get_dyn(id).is_none() {
        return Err(format!("no dialog {}", id.value()));
    }
    // Automation confirming a dialog that stands for a command is judged as that command, whichever way the answer
    // arrives (a control request or synthetic input).
    #[cfg(not(target_arch = "wasm32"))]
    if app.automation_input
        && let Some(authorize) = app.services.automation_command.as_ref()
        && let Some((command, params)) = crate::control::dialogs::hosted_command(app, id)
    {
        authorize(&command, &params)?;
    }
    // Synthetic input answering Save on the unsaved-changes prompt is judged as `file.save`, as a control answer is.
    #[cfg(not(target_arch = "wasm32"))]
    if app.automation_input && crate::control::dialogs::is_save_answer(app, id) {
        crate::discard_ui::authorize_save(app, false)?;
    }
    if !app.framework.dialogs.confirm_by_id(ctx, id) {
        if app.framework.dialogs.held_state(id) != Some(HeldOk::Waiting) {
            take_results(app, ctx);
            return Err(format!("dialog {} cannot be confirmed now", id.value()));
        }
        // The family that holds the OK answers it among the results.
        take_results(app, ctx);
        match app.framework.dialogs.held_state(id) {
            Some(HeldOk::Accepted) => {
                app.framework.dialogs.confirm_by_id(ctx, id);
            }
            Some(HeldOk::Refused(reason)) => return Err(reason.as_str().to_owned()),
            _ => {}
        }
    }
    take_results(app, ctx);
    Ok(())
}

/// Cancels dialog `id`, as Cancel or Escape would, and runs what a cancel leaves to do.
pub fn cancel(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId) -> Result<(), String> {
    // A prompt answers Escape with its escape button, as an answer its family takes; the host's own cancel is a result
    // that reaches no family (it stands for a replaced prompt).
    let escape = app.framework.dialogs.get_dyn_mut(id).and_then(|dialog| dialog.downcast_mut::<StandardPrompt>()).and_then(|prompt| {
        let choice = prompt.escape_choice()?;
        prompt.set_choice(choice);
        Some(())
    });
    if escape.is_some() && app.framework.dialogs.confirm_by_id(ctx, id) {
        take_results(app, ctx);
        return Ok(());
    }
    if !app.framework.dialogs.cancel_by_id(ctx, id) {
        return Err(format!("no dialog {}", id.value()));
    }
    take_results(app, ctx);
    Ok(())
}

/// Cancels every open parameter dialog whose family reports one of the `closed` documents from the dialog's captured
/// value: a dialog about a document does not outlive it.
pub(crate) fn cancel_for_closed(app: &mut PhotocraftApp, ctx: &egui::Context, closed: &[DocId]) {
    let doomed: Vec<DialogId> = app
        .framework
        .dialogs
        .open_dialogs()
        .into_iter()
        .filter_map(|open| {
            let key = open.key?;
            let document = params::PARAM_FAMILIES.iter().find(|family| (family.owns)(&key))?.document?;
            let doc = document(app.framework.captured.get(&key)?)?;
            closed.contains(&doc).then_some(open.id)
        })
        .collect();
    if doomed.is_empty() {
        return;
    }
    for id in doomed {
        app.framework.dialogs.cancel_by_id(ctx, id);
    }
    take_results(app, ctx);
}

/// Open the parameter dialog of `command`: the guide layout, Stroke, Equalize (when it asks), New Group, Color Range,
/// Slice options and Divide, the adjustment editor for `image.adjustments.*`, the schema dialog for filters, otherwise a
/// bare command form. The error is an opener's refusal text.
pub fn open_command_dialog(app: &mut PhotocraftApp, command: &str, label: &str) -> Result<DialogId, String> {
    if command == "view.newGuideLayout" {
        return guide_layout_ui::open(app).ok_or_else(|| "no document".to_string());
    }
    if command == stroke_ui::COMMAND {
        return Ok(stroke_ui::open(app));
    }
    if command == equalize_ui::COMMAND && equalize_ui::asks(app) {
        return Ok(equalize_ui::open(app));
    }
    if new_group_ui::fronts(command) {
        return Ok(new_group_ui::open(app, command));
    }
    if command == color_range_ui::COMMAND {
        return color_range_ui::open(app);
    }
    if command == "slice.set" {
        return crate::slice_ui::open_options(app);
    }
    if command == "slice.divide" {
        return crate::slice_ui::open_divide(app);
    }
    if adjust_dialog::has_dialog(command) {
        return adjust_dialog::open(app, command);
    }
    if filter_dialog::has_dialog(command) {
        return filter_dialog::open(app, command);
    }
    Ok(params::open_bare(app, command, label))
}

/// Preference entries the engine reads itself, by key: the engine has no settings store, so they cannot move.
const ENGINE_READ: &[&str] = &["ui.layersPanelOptions", "file.print", "file.export.saveForWebLegacy"];

/// The "Don't show again" entries (`{"skip": true}`), by key, as their prompt modules read them. Told apart by key, not
/// by the shape of the value.
fn dont_show_again_keys() -> [&'static str; 4] {
    [delete_layer_prompt::PREF, live_shape_prompt::PREF, crate::analysis::note_alerts::DELETE_NOTE_PREF, crate::retouch_ui::CAM_NO_PIXELS_PREF]
}

/// Where the remembered value of the preference entry `key` goes: `None` keeps it in `Preferences.dialogs`, else the
/// scope it moves to. Order: keep, then route, then the generic `photocraft.<key>`.
fn memory_destination(key: &str) -> Option<String> {
    if ENGINE_READ.contains(&key) || dont_show_again_keys().contains(&key) {
        return None;
    }
    // The keys the app passes to a workspace or dialog (`select.refineEdge.workspace`, `edit.contentAwareFill.workspace`,
    // `filter.liquify`, `view.newGuide`, `file.export.exportAs`, `file.jpegOptions`) follow the generic naming, so only a
    // key a library crate names differently routes.
    let routed = match key {
        "filter.cameraRaw.scope" => Some("craft_imaging.raw_develop.scope".to_owned()),
        _ => key
            .strip_prefix("image.adjustments.")
            .map(|kind| format!("photocraft.adjust.last.{kind}"))
            .or_else(|| key.strip_prefix("shape.create.").map(|kind| format!("craft_draw.shape.create.{kind}"))),
    };
    Some(routed.unwrap_or_else(|| params::memory_scope(key)))
}

/// `value` without its nulls: a missing key reads as the dialog's default, and TOML has no null.
fn without_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(map.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, without_nulls(v))).collect()),
        Value::Array(items) => Value::Array(items.into_iter().filter(|v| !v.is_null()).map(without_nulls).collect()),
        other => other,
    }
}

/// Fill and Stroke remembered their blend mode as a `mode_key` string ("colorBurn"); the dialogs keep the layers panel's key.
fn convert_blend_mode(key: &str, value: &mut Value) {
    if !matches!(key, "edit.fill" | "edit.stroke") {
        return;
    }
    let Some(mode) = value.get_mut("mode") else { return };
    if let Some(blend) = mode.as_str().and_then(crate::panels::layers_tree::blend_of) {
        *mode = Value::String(blend.key().to_string());
    }
}

/// Moves the dialogs' last-used values out of `Preferences.dialogs` into the remembered section of `host`, once, before
/// the first frame. Entries the engine reads and the don't-show-again entries stay; a second run finds nothing to move
/// because every moved entry left `Preferences.dialogs`. An entry that has no table form stays too.
pub(crate) fn move_memories(prefs: &mut Preferences, host: &Host) {
    let keys: Vec<String> = prefs.dialogs.keys().cloned().collect();
    for key in keys {
        let Some(scope) = memory_destination(&key) else { continue };
        let Some(old) = prefs.dialogs.get(&key) else { continue };
        let mut value = without_nulls(old.clone());
        convert_blend_mode(&key, &mut value);
        let Ok(table) = toml::Table::try_from(&value) else { continue };
        Remembered::<toml::Table>::new(scope).store(host, &table);
        prefs.dialogs.remove(&key);
    }
}

pub mod adjust_dialog;
pub mod adjust_editors;
pub mod adjust_preview;
pub mod alerts;
#[cfg(test)]
mod arbitrary_ruler_prefill_tests;
pub(crate) mod bitmap_dialog;
pub mod blend_options;
pub mod blur_pins;
pub mod brush_preset_dialog;
pub mod caf_ui;
pub(super) mod camera_raw_detail_ui;
pub(super) mod camera_raw_gpu;
pub(super) mod camera_raw_scope_ui;
pub mod camera_raw_ui;
pub(crate) mod canvas_input;
pub mod channel_options_ui;
#[cfg(test)]
mod color_lookup_no_remember_tests;
pub(crate) mod color_picker_preview;
pub mod color_picker_ui;
pub mod color_range_ui;
pub mod color_table_ui;
pub(crate) mod conditional_mode_change_dialog;
pub mod count_group_name_ui;
pub mod credits;
pub mod dds_options_ui;
pub mod delete_layer_prompt;
pub mod discard_ui;
pub mod disk_change_prompt;
pub mod distort_ui;
pub mod duplicate_layer_ui;
pub mod equalize_ui;
pub mod export_dialog;
pub(crate) mod fade_modes;
pub mod file_forms;
pub mod file_ui;
pub mod fill_ui;
pub mod filter_dialog;
pub(super) mod filter_preview_ui;
#[cfg(not(target_arch = "wasm32"))]
pub(super) mod filter_preview_worker;
pub(crate) mod format_prompts;
pub mod gallery_ui;
pub(crate) mod guide_edit;
pub mod guide_layout_ui;
pub mod history_options_ui;
pub(crate) mod image_processor_ui;
pub mod jpeg_options_ui;
pub mod layer_picker;
pub mod layer_style;
pub(crate) mod lens_batch_ui;
pub mod liquify_ui;
pub mod live_shape_prompt;
pub mod mask_trash_prompt;
pub mod merge_channels_ui;
pub mod new_doc_ui;
pub mod new_group_ui;
pub mod new_guide_ui;
pub mod new_layer_ui;
pub mod open_prompt;
pub mod palette;
#[cfg(test)]
mod paths_export_default_tests;
#[cfg(test)]
mod paths_export_document_bounds_tests;
pub(crate) mod pattern_fill_ui;
pub mod preset_group_prompt;
pub mod rasterize_prompt;
#[cfg(test)]
mod rotate_canvas_layout_tests;
pub(crate) mod rotate_canvas_ui;
#[cfg(test)]
mod scale_effects_layout_tests;
pub mod screen_picker;
pub mod select_mask_ui;
pub mod shape_dialog;
pub(crate) mod shape_fill_ui;
pub(crate) mod shared_picture;
pub(super) mod sizing;
pub mod smart_filter_dialog;
pub mod snapshot_dialog;
pub(crate) mod solid_fill_ui;
pub mod stack_dialog;
pub mod stroke_ui;
pub mod targa_options_ui;
#[cfg(test)]
mod tests;
pub mod tiff_options_ui;
pub mod vp_ui;
pub mod wide_angle_ui;

/// Confirm and cancel a dialog from a test, on the app's own context.
#[cfg(test)]
pub(crate) mod testing {
    use craft::ui::dialog::DialogId;

    use crate::PhotocraftApp;

    /// Answers dialog `id` with OK and takes the results.
    pub(crate) fn confirm(app: &mut PhotocraftApp, id: DialogId) -> Result<(), String> {
        let ctx = app.ctx().clone();
        super::confirm(app, &ctx, id)
    }

    /// Cancels dialog `id` and takes the results.
    pub(crate) fn cancel(app: &mut PhotocraftApp, id: DialogId) -> Result<(), String> {
        let ctx = app.ctx().clone();
        super::cancel(app, &ctx, id)
    }

    /// Runs one pass of the dialog host, as a frame does. A hosted parameter form loads its remembered values and
    /// defaults when it first shows, so its fields read empty until a pass has run.
    pub(crate) fn show(app: &mut PhotocraftApp) {
        let ctx = app.ctx().clone();
        crate::testing::run_ui(&ctx, Default::default(), |ui| super::run(app, ui));
    }
}
