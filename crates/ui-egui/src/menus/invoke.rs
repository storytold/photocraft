use craft::keymap::CommandRef;
use craft::menu::{LANGUAGE_COMMAND, LanguageChoice, THEME_COMMAND, ThemeChoice};
// PLAN: assumed facade paths `craft::keymap::CommandRef` and `craft::ui::dialog::DialogId`.
use craft::ui::dialog::DialogId;

use super::{DOCUMENT_ITEM, PhotocraftApp, Value, json};

/// The reply of a command that opened a dialog: the id in the form `ui.dialog.open` and `ui.inspect` use.
fn dialog_reply(id: DialogId) -> Value {
    json!({"dialog": id.value().to_string()})
}

/// The open Free Transform as the control protocol reports it, `null` without one.
fn transform_reply(app: &PhotocraftApp) -> Value {
    json!({"transform": app.ui.transform.as_ref().map(crate::state::FreeTransform::record)})
}

/// Photoshop's Window › <panel> ids for the panels the shell already has, as `window.toggle.*`.
pub(crate) fn panel_alias(id: &str) -> Option<&'static str> {
    Some(match id.strip_prefix("window.panel.")? {
        "layers" => "window.toggle.layers",
        "history" => "window.toggle.history",
        "properties" | "adjustments" => "window.toggle.properties",
        "color" | "swatches" => "window.toggle.color",
        "navigator" | "info" | "histogram" => "window.toggle.navigator",
        "tools" => "window.toggle.toolbar",
        "options" => "window.toggle.options",
        "contextualTaskBar" => "window.toggle.contextualTaskBar",
        "brushSettings" => "window.toggle.brushSettings",
        "brushes" => "window.toggle.brushes",
        _ => return None,
    })
}

/// View › Proof Setup presets we can simulate, as `view.proofSetup` profiles.
pub(super) fn proof_preset(id: &str) -> Option<&'static str> {
    Some(match id.strip_prefix("view.proofSetup.")? {
        "workingCmyk" => "working-cmyk",
        "internetStandardRgb" | "monitorRgb" => "srgb",
        _ => return None,
    })
}

/// The workspace id of a `window.workspace.<id>` command; `select` is the command that takes the id as a parameter.
pub(super) fn workspace_id(id: &str) -> Option<&str> {
    id.strip_prefix("window.workspace.").filter(|workspace| *workspace != "select")
}

/// Whether command `id` leaves a Type box-handle drag going: only the ones that change what is
/// on screen and nothing else. `UI_COMMANDS` is every command the shell carries out itself, so it
/// can't tell them apart: it holds `edit.freeTransform` and `file.save`, which must see the dragged
/// box, `view.clearSelectedGuides`, which edits the document, and `window.nextDocument`, which
/// switches to another one mid-drag. So the commands that cannot touch either are listed one by
/// one, and the panel ones (open, close, arrange: `window.panel.*`, `window.toggle.*` and
/// `type.panels.*`, the Character panel's Cmd+T among them) by prefix, since they only lay out
/// windows. Everything else applies the box first, which is harmless when no drag is going.
fn keeps_box_drag(id: &str) -> bool {
    const VIEW: &[&str] = &[
        "view.zoomIn",
        "view.zoomOut",
        "view.fitOnScreen",
        "view.actualPixels",
        "view.rotateView",
        "view.resetView",
        "view.darkenInterface",
        "view.lightenInterface",
    ];
    VIEW.contains(&id) || ["window.panel.", "window.toggle.", "type.panels."].iter().any(|prefix| id.starts_with(prefix))
}

/// Run a command id from any source (menu, shortcut, palette, automation).
pub fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: Value) -> Result<Value, String> {
    // The regions that ran before it in this pass show its change in the next one. A view command changes only what
    // the view's regions hold in their versions.
    if !crate::panels::region_versions::changes_only_the_view(app, id) {
        app.commands_run += 1;
    }
    ctx.request_repaint();
    if app.automation_input
        && let Some(authorize) = app.services.automation_command.as_ref()
    {
        authorize(id, &params)?;
    }
    // A held Type box handle applies its box before any command that may read or edit the
    // document runs; a command that edits it first would make the drag's box stale and lose it.
    if !keeps_box_drag(id) {
        crate::type_box_preview::end_drag(app);
    }
    if crate::discard_ui::intercept(app, id, &params) {
        return Ok(Value::Null);
    }
    let revealed = crate::layer_reveal::mark(app);
    let history = crate::panels::history::mark(app);
    let result = invoke_unguarded(app, ctx, id, params);
    crate::layer_reveal::file(app, revealed);
    crate::panels::history::file(app, history);
    let r = result?;
    // The palette's empty-query list (UI-217-7); opening the palette isn't a command to recall.
    if id != "edit.search" {
        crate::palette::note_recent(app, id);
    }
    Ok(r)
}

/// Layer › New › Convert to Frame without params: asks for the frame's name and size first.
fn frame_convert_dialog(app: &mut PhotocraftApp, id: &str) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let layer = st.active_layer.and_then(|id| st.doc.layer(id)).ok_or("select a shape or type layer")?;
    let b = photocraft_engine::transform_cmds::transform_frame(&st.doc, layer);
    let mut fixed = serde_json::Map::new();
    fixed.insert("layer".into(), json!(layer.id.0));
    let dialog = crate::filter_dialog::open_with_spec(app, id, tl!("New Frame"), r#"{"name":text,"width":1..300000=100,"height":1..300000=100}"#, fixed);
    let ctx = app.ctx().clone();
    for (field, value) in [("__preview", json!(false)), ("name", json!("Frame")), ("width", json!(b.width().max(1))), ("height", json!(b.height().max(1)))] {
        app.framework.dialogs.set_json_field(&ctx, dialog, field, value).map_err(|e| e.to_string())?;
    }
    Ok(dialog_reply(dialog))
}

/// [`invoke`] without the unsaved-changes prompt, for once the user has already answered it.
fn invoke_early_commands(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if let Some(tool) = crate::tool_shortcuts::tool(id) {
        crate::tool_memory::select(app, tool);
        // The tool's own brush now, not at the next frame, so a paint command right after uses
        // it: a first Pencil pick its 1 px tip (#12200), any other tool its remembered brush.
        crate::paint_mouse::sync_tool_brush(app);
        return Some(Ok(json!({"tool": tool})));
    }
    if id == "layer.frame.convert" && params.as_object().is_none_or(|o| o.is_empty()) {
        return Some(frame_convert_dialog(app, id));
    }
    if id == "help.checkForUpdates" {
        return Some(crate::updates_ui::check(app, ctx, params.clone()));
    }
    // Layer › Copy CSS: the engine writes the CSS, the clipboard is the shell's (#9372).
    if id == photocraft_engine::layer_css_cmds::ID {
        return Some(app.run(id, params.clone()).inspect(|r| {
            if let Some(css) = r.get("css").and_then(Value::as_str) {
                ctx.copy_text(css.to_string());
            }
        }));
    }
    if id == "ui.symmetryTransform" {
        return Some(crate::symmetry_ui::begin(app).map(|_| Value::Null));
    }
    if let Some(r) = crate::tool_reset::invoke(app, id, params) {
        return Some(r);
    }
    // A filter on a type layer asks to rasterize it first (#4303).
    if let Some(r) = crate::rasterize_prompt::intercept_filter(app, id, params) {
        return Some(r);
    }
    // A pending Pen anchor is gesture state; Edit › Undo must match the keyboard. Undo back to
    // where a Pen path was finished (closed, ↩/Esc, another tool) gives the path back to the Pen
    // rather than deleting it; Redo finishes it again. Step Backward/Forward step the same way.
    // An open Free Transform or Crop box steps itself first.
    if let Some(r) = crate::vector_ui::pen_history_step(app, id) {
        return Some(r.map(|_| Value::Null));
    }
    if id == "edit.undo" && crate::actions::library::edit_undo_label(app).is_some() {
        return Some(crate::actions::library::undo(app));
    }
    // Image › Crop while the Crop tool has a pending frame commits that frame (#1918).
    if id == "image.crop" && crate::crop_ui::pending(app) {
        let committed = crate::crop_ui::apply(app);
        return Some(Ok(json!({"committed": committed})));
    }
    if let Some(result) = crate::panel_docking::command(app, id, params) {
        return Some(result);
    }
    // Help › Discord, website, GitHub, Report an Issue.
    if let Some(url) = crate::links::url_for(id) {
        return Some(Ok(crate::links::open(app, ctx, url)));
    }
    if id == "view.proofSetup.custom" {
        return Some(crate::filter_dialog::open(app, "view.proofSetup").map(dialog_reply));
    }
    // Image › Analysis tools/dialogs, Measurement Log and Notes panels, File › Import › Notes.
    if let Some(r) = crate::analysis::menu(app, id, params) {
        return Some(r);
    }
    // Workspace Select, Modifier Keys, custom pixel aspect, Extras/32-bit options.
    if let Some(r) = crate::workspace_ui::menu(app, id, params) {
        return Some(r);
    }
    // Character/Paragraph Styles, Glyphs, Edit › Check Spelling dialog.
    if let Some(r) = crate::type_panels_ui::menu(app, id, params) {
        return Some(r);
    }
    // Image > Variables (Define / Data Sets), Apply Data Set.
    if let Some(r) = crate::variables_ui::menu(app, id, params) {
        return Some(r);
    }

    // Filter › Plug-ins (installed WebAssembly plug-ins, Install Plug-in…).
    if let Some(r) = crate::plugin_ui::menu(app, id, params) {
        return Some(r);
    }
    // History panel menu › History Options… (without params: its dialog).
    if let Some(r) = crate::history_options_ui::menu(app, id, params) {
        return Some(r);
    }
    None
}

fn invoke_shell_commands(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    // View/Window/Type shell items, and dialogs/pickers in front of File commands.
    // Edit › Preferences, Keyboard Shortcuts, Color Settings and other Edit dialogs.
    if let Some(r) = crate::prefs_ui::invoke(app, ctx, id, params) {
        return Some(r);
    }
    // Save for Web's dialog would export the committed document, not the open Free Transform (#6737).
    if id == crate::transform_tool::SAVE_FOR_WEB
        && let Some(r) = crate::transform_tool::intercept(app, id)
    {
        return Some(r);
    }
    // Save for Web, Print and the other File-menu dialogs added with slices.
    if let Some(r) = crate::file_ui::invoke(app, ctx, id, params) {
        return Some(r);
    }
    if let Some(r) = crate::view_cmds::invoke(app, ctx, id, params) {
        return Some(r);
    }
    // Liquify dialog, Puppet Warp and Perspective Warp modes, Filter Gallery and Vanishing Point (and their control params).
    if let Some(r) = crate::distort_ui::menu(app, ctx, id, params) {
        return Some(r);
    }
    // Camera Raw Filter dialog (and its control params).
    if let Some(r) = crate::camera_raw_ui::menu(app, ctx, id, params) {
        return Some(r);
    }
    if let Some(r) = crate::wide_angle_ui::menu(app, ctx, id, params) {
        return Some(r);
    }
    if let Some(profile) = proof_preset(id) {
        // View › Proof Setup presets: set the proof profile and turn Proof Colors on.
        return Some(app.run("view.proofSetup", json!({"profile": profile})).and_then(|_| app.run("view.proofColors", json!({"on": true}))));
    }
    if let Some(alias) = panel_alias(id) {
        return Some(invoke(app, ctx, alias, params.clone()));
    }
    // The four commands the framework's workspace set owns keep their menu ids on this path.
    let owned = photocraft_engine::prefs::theme_ids::command_id(id);
    if owned != id
        && let Some(result) = crate::workspace_ui::run_command(app, ctx, &CommandRef::with_params(owned, params.clone()))
    {
        return Some(result);
    }
    if let Some(workspace) = workspace_id(id) {
        return Some(crate::workspace_ui::select(app, workspace).map(|()| json!({"workspace": app.workspaces.set.current})));
    }
    None
}

/// The commands the shell answers itself before the engine (File › Open and Save with the
/// dialogs they ask, loading a path as a selection, …); `Ok(None)` for any other.
fn invoke_file_commands(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Result<Option<Value>, String> {
    let result = match id {
        // ⌘↩ / Ctrl+Enter (#306): load the path selected in the Paths panel (or the one being
        // drawn) as a selection.
        "path.toSelection" if params.get("name").is_none() => crate::vector_ui::path_to_selection(app, params.clone()),
        // ⌘⇧↩ / ⌘⌥↩ / ⇧⌥⌘↩ load the same path, adding it to, subtracting it from or intersecting
        // it with the selection.
        "path.addToSelection" | "path.subtractFromSelection" | "path.intersectWithSelection" if params.get("name").is_none() => {
            let mut p = params.as_object().cloned().unwrap_or_default();
            let mode = match id {
                "path.addToSelection" => "add",
                "path.subtractFromSelection" => "subtract",
                _ => "intersect",
            };
            p.insert("mode".into(), json!(mode));
            crate::vector_ui::path_to_selection(app, Value::Object(p))
        }
        // Layer › Rename Layer from a menu starts in-place renaming in the Layers panel.
        "layer.renameLayer" if params.get("name").is_none() => {
            let st = app.session.active().ok_or("no document")?;
            let id = params.get("layer").and_then(Value::as_u64).or(st.active_layer.map(|l| l.0)).ok_or("no active layer")?;
            st.doc.layer(photocraft_doc::LayerId(id)).ok_or("no such layer")?;
            app.layers_rename = Some(photocraft_doc::LayerId(id));
            Ok(Value::Null)
        }
        "layer.duplicate" if params.as_object().is_none_or(|o| o.is_empty()) => crate::duplicate_layer_ui::open(app).map(dialog_reply),
        "file.new" if params.as_object().is_none_or(|o| o.is_empty()) => Ok(dialog_reply(crate::new_doc_ui::open(app))),
        "file.open" => {
            if let Some(path) = params.get("path").and_then(Value::as_str) {
                open_path(app, path)
            } else {
                app.open_dialog_file()
            }
        }
        "file.import.videoFramesToLayers" if params.get("path").and_then(Value::as_str).is_none() => {
            app.ask_folder(|app, path| {
                let r = app.run("file.import.videoFramesToLayers", json!({"path": &path}));
                if let Ok(v) = &r {
                    // #11113: a truncated GIF or WebP in the folder salvages its early frames;
                    // the command's warning says the animation arrived shortened.
                    let what = crate::i18n::fmt(crate::i18n::t("Imported {name}"), &[("name", &crate::file_open::display_name(&path))]);
                    crate::notices::footage_warnings(app, v, &what);
                }
                r.map_err(|e| e.to_string())
            })
        }
        // File > Import > Variable Data Sets… asks for the CSV (#8816); with a `path` the engine reads it itself.
        "file.import.variableDataSets" if params.get("path").and_then(Value::as_str).is_none() => {
            let doc = app.active_doc_id()?;
            let params = params.clone();
            app.pick_file_bytes_filtered(&["csv", "txt"], move |app, _name, bytes| {
                app.refocus(doc)?;
                let text = data_sets_text(bytes)?;
                let r = photocraft_engine::import_data_sets_text(&mut app.session, &text, &params).map_err(|e| e.to_string());
                app.sync_views();
                r
            })
        }
        "file.save" => {
            // Writes back only to a layered file that holds the whole document; anything else
            // goes through Save As, so the original is never overwritten with less (#3806).
            let choices = crate::FormatChoices::from_params(params, id)?;
            match params.get("path").and_then(Value::as_str) {
                Some(path) => app.save_as_with(Some(path.to_string()), choices),
                None => {
                    let own = app.session.active().and_then(|d| d.path.clone().map(|p| (photocraft_engine::file_cmds::holds_at(&d.doc, &p), p, d)));
                    match own {
                        Some((true, path, d)) => {
                            // Another app changed the file since it was opened or saved: ask first (#6596).
                            let doc = d.doc.id;
                            crate::disk_change_prompt::intercept(app, doc, &path, false, crate::FormatChoices::default())
                                .unwrap_or_else(|| app.save_in_place(path, crate::FormatChoices::default()))
                        }
                        Some((false, path, d)) if photocraft_engine::file_cmds::saves_in_place(&path) => {
                            // Its own file is layered but can't hold what the document has now.
                            let losses = photocraft_engine::file_cmds::losses_at(&d.doc, &path);
                            let name = crate::file_open::display_name(&path);
                            let title = crate::i18n::fmt(tl!("{name} can't keep everything in this document"), &[("name", name.as_str())]);
                            let mut lines = losses;
                            lines.push(tl!("Choose a format that keeps it, such as Photoshop (PSD), or save a copy.").to_string());
                            crate::notices::post(app, title, lines, craft::theme::variant::Tone::Neutral, None);
                            app.save_as_with(None, choices)
                        }
                        // Its own flat file, while it still keeps the whole document (one plain
                        // layer it can store): written back, as Photoshop's Save does (#5733),
                        // asking first if another app changed it (#6596), then JPEG Options for a
                        // JPEG (#2223), or TIFF, Targa or DDS Options for those. Cancel in either writes
                        // nothing.
                        Some((false, path, d)) if crate::saves_flat_in_place(&d.doc, &path) => {
                            let doc = d.doc.id;
                            if app.save_options_open() {
                                return Err(crate::SAVE_OPTIONS_OPEN.into());
                            }
                            crate::disk_change_prompt::intercept(app, doc, &path, false, choices).unwrap_or_else(|| app.save_flat_in_place(path, choices))
                        }
                        // Any other flat file asks for its name; confirming its own file records a
                        // plain Save that writes the file an action is played on (#3531).
                        _ => app.save_dialog_flow(None, choices, true),
                    }
                }
            }
        }
        "file.exit" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            Ok(Value::Null)
        }
        "file.clearRecent" => {
            app.clear_recent();
            Ok(Value::Null)
        }
        "file.removeRecent" => {
            let path = params.get("path").and_then(Value::as_str).ok_or("file.removeRecent needs a \"path\"")?;
            app.remove_recent(path);
            Ok(Value::Null)
        }
        id if id.starts_with("file.openRecent.") => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                match id.rsplit('.').next().and_then(|s| s.parse::<usize>().ok()).and_then(|i| app.ui.recent_files.get(i).cloned()) {
                    Some(p) => open_path(app, &p),
                    None => Err("no such recent file".to_string()),
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = app;
                let message = "Open Recent is unavailable on the web";
                Err(tl!(message).to_string())
            }
        }
        "file.saveAs" => {
            let choices = crate::FormatChoices::from_params(params, id)?;
            app.save_as_with(params.get("path").and_then(Value::as_str).map(str::to_string), choices)
        }
        _ => return Ok(None),
    };
    result.map(Some)
}

/// Dispatch UI actions and dialogs, falling back to the engine for other commands.
pub(crate) fn invoke_unguarded(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: Value) -> Result<Value, String> {
    // File › Save and the unsaved-changes prompt's Save write from here without `run`: they must
    // not write Apply Data Set's Preview either. The dialog shows it again on the next frame.
    crate::panels::apply_data_set_ui::end_preview(app);
    if let Some(result) = crate::smart_svg_ui::intercept(app, id, &params) {
        return result;
    }
    if id == "layer.selectUnderCursor" {
        return select_layer_under_cursor(app, ctx);
    }
    if let Some(result) = invoke_early_commands(app, ctx, id, &params) {
        return result;
    }
    if let Some(result) = invoke_shell_commands(app, ctx, id, &params) {
        return result;
    }
    if let Some(result) = invoke_file_commands(app, ctx, id, &params)? {
        return Ok(result);
    }
    if let Some(result) = invoke_window_command(app, ctx, id, &params) {
        return result;
    }
    if let Some(result) = invoke_export_command(app, id, &params)? {
        return result;
    }
    if let Some(result) = invoke_type_command(app, ctx, id, &params) {
        return result;
    }
    if let Some(result) = invoke_view_command(app, ctx, id, &params) {
        return result;
    }
    match id {
        "edit.search" => {
            app.ui.palette.open = !app.ui.palette.open;
            Ok(Value::Null)
        }
        "help.about" => Ok(dialog_reply(crate::credits::open_about(app, Default::default()))),
        "help.systemInfo" => {
            let dialog = crate::credits::open_system_info(app);
            Ok(json!({"dialog": dialog.value().to_string(), "info": crate::gpu_status::system_info_json(app)}))
        }
        "layer.smartFilter.blendingOptions" if params.get("blend").is_none() && params.get("opacity").is_none() => {
            let st = app.session.active().ok_or("no document open")?;
            let layer = params.get("layer").and_then(Value::as_u64).map(photocraft_doc::LayerId).or(st.active_layer).ok_or("no layer selected")?;
            let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(layer).map(|l| &l.content) else {
                return Err("the layer is not a Smart Object".into());
            };
            let index = match params.get("index") {
                Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("invalid Smart Filter index")?,
                None => sm.smart_filters.len().checked_sub(1).ok_or("the Smart Object has no Smart Filters")?,
            };
            crate::smart_filter_dialog::open(app, layer, index).map(dialog_reply)
        }
        "layer.layerStyle.blendingOptions" if params.as_object().is_none_or(|o| o.is_empty()) => {
            crate::layer_style::open(app, Some(crate::layer_style::BLENDING)).map(dialog_reply).ok_or_else(|| "no active layer".to_string())
        }
        // Only interactive creation opens the Color Picker; it creates the layer on OK,
        // so Cancel leaves neither an orphan layer nor an undo/redo history entry.
        // Explicit parameters continue straight to the engine.
        "layer.newFillLayer.solidColor" if params.as_object().is_none_or(|o| o.is_empty()) => {
            let dialog = crate::solid_fill_ui::open_new(app).ok_or("no document is open")?;
            Ok(dialog_reply(dialog))
        }
        // Solid Color fills and solid-colour shapes use the Color Picker, Pattern fills the Pattern
        // Fill dialog; other adjustment / fill / shape controls use Properties.
        "layer.layerContentOptions" => {
            if let Some(dialog) = crate::solid_fill_ui::open(app).or_else(|| crate::shape_fill_ui::open(app)) {
                return Ok(dialog_reply(dialog));
            }
            if let Some(dialog) = crate::pattern_fill_ui::open(app) {
                return Ok(dialog_reply(dialog));
            }
            let r = app.run(id, params)?;
            crate::panel_docking::show_panel(app, "properties", Some(true))?;
            Ok(r)
        }
        // Edit › Stroke…: open its full options dialog when called from menus/context
        // menus. Explicit params from Actions, CLI, MCP and control channel execute directly.
        crate::stroke_ui::COMMAND if params.as_object().is_none_or(|o| o.is_empty()) => {
            // A targeted smart-filter mask can admit Stroke without a raster layer.
            if let Some(why) = app.session.disabled_reason_with(id, &app.with_mask_target(id, params)) {
                return Err(why);
            }
            Ok(dialog_reply(crate::stroke_ui::open(app)))
        }
        // Layer › New › Group… / Group from Layers… (and the Layers panel's menu, ⌥-click on its
        // New Group button): Photoshop's New Group dialog. Explicit params run the engine directly,
        // and Group Layers (⌘G) never asks.
        _ if crate::new_group_ui::fronts(id) && params.as_object().is_none_or(|o| o.is_empty()) => {
            if let Some(Err(why)) = photocraft_engine::commands::find(id).map(|c| (c.enabled)(&app.session)) {
                return Err(why);
            }
            Ok(dialog_reply(crate::new_group_ui::open(app, id)))
        }
        // Channels › Channel Options… without params: the dialog on the targeted alpha/spot
        // channel (also in Quick Mask mode, where its row is the selected one), Quick Mask Options
        // in Quick Mask otherwise, or Layer Mask Display Options on a targeted layer mask (with
        // params: the engine directly).
        crate::channel_options_ui::COMMAND if params.as_object().is_none_or(|o| o.is_empty()) => {
            let mask = app.layer_mask_targeted();
            let st = app.session.active().ok_or("no document is open")?;
            let channel = match st.channel_view.target {
                photocraft_engine::channel_cmds::ChannelTarget::Alpha(i) => json!(i),
                _ if st.doc.quick_mask.is_some() => json!("quickMask"),
                _ if mask => json!("mask"),
                _ => return Err("target an alpha or spot channel, or a layer mask, first".into()),
            };
            let dialog = crate::channel_options_ui::open(app, &channel).ok_or("no such channel")?;
            Ok(dialog_reply(dialog))
        }
        // Channels › New Channel… without params: its dialog (with params: the engine). The
        // panel's New button makes one at once, as Photoshop's does.
        crate::channel_options_ui::NEW_CHANNEL if params.as_object().is_none_or(|o| o.is_empty()) => {
            let dialog = crate::channel_options_ui::open_new_channel(app).ok_or("no document is open")?;
            Ok(dialog_reply(dialog))
        }
        // Channels › Merge Channels… without params: its dialog, which picks the mode and the
        // documents (with params: the engine).
        crate::dialogs::merge_channels_ui::COMMAND if params.as_object().is_none_or(|o| o.is_empty()) => {
            if let Some(Err(why)) = photocraft_engine::commands::find(id).map(|c| (c.enabled)(&app.session)) {
                return Err(why);
            }
            let dialog = crate::dialogs::merge_channels_ui::open(app).ok_or("no document is open")?;
            Ok(dialog_reply(dialog))
        }
        // Channels › New Spot Channel… without params: its dialog (with params: the engine).
        crate::channel_options_ui::NEW_SPOT if params.as_object().is_none_or(|o| o.is_empty()) => {
            let dialog = crate::channel_options_ui::open_new_spot(app).ok_or("no document is open")?;
            Ok(dialog_reply(dialog))
        }
        // Select › Find Layers: the Layers panel's Name filter (#10884).
        "select.findLayers" => crate::layer_filter_ui::find::invoke(app, params),
        // Photoshop's "Select and Mask…" is the engine's select.refineEdge.
        // Select › Color Range… from the menu: the dialog (with params: the engine directly).
        "select.colorRange" if params.as_object().is_none_or(|o| o.is_empty()) => crate::color_range_ui::open(app).map(dialog_reply),
        // Layer › New Fill Layer › Pattern… from the menu: the Pattern Fill dialog (with params:
        // the engine directly).
        photocraft_engine::pattern_fill_cmds::CREATE if params.as_object().is_none_or(|o| o.is_empty()) => {
            Ok(dialog_reply(crate::pattern_fill_ui::open_new(app)?))
        }
        // Edit › Fill… from the menu or its shortcuts: the Fill dialog (with params: the engine).
        crate::fill_ui::COMMAND if params.as_object().is_none_or(|o| o.is_empty()) => {
            // Admission follows the effective mask/channel target, just like enablement and run.
            if let Some(why) = app.session.disabled_reason_with(id, &app.with_mask_target(id, params)) {
                return Err(why);
            }
            Ok(dialog_reply(crate::fill_ui::open(app)))
        }
        // Photoshop's workspace (select_mask_ui); `distort_ui::menu` routes it first.
        "select.selectAndMask" => {
            crate::select_mask_ui::open(app).map(|_| json!({"selectAndMask": crate::distort_ui::workspace_fields(app, "select.selectAndMask")}))
        }
        // Select › Transform Selection from the menu: the interactive box (with params: the engine).
        "select.transformSelection" if params.as_object().is_none_or(|o| o.is_empty()) => {
            crate::transform_tool::begin_selection(app, ctx).map(|_| transform_reply(app))
        }
        "edit.paste" if params.as_object().is_none_or(|o| o.is_empty()) => {
            // Photoshop: paste in place when the copied area is visible, else centred in the view;
            // images from other apps are always centred.
            app.import_os_clipboard()?;
            app.clip_read_for_paste = true;
            let external = app.clip_external;
            let visible = !external
                && app.session.active_index().zip(app.session.clipboard.as_ref()).is_some_and(|(i, clip)| {
                    let v = &app.ui.views[i];
                    let pz = app.point_zoom();
                    let (hw, hh) = (app.last_canvas_rect.width() / 2.0 / pz / app.ui.view.display_aspect(), app.last_canvas_rect.height() / 2.0 / pz);
                    let r =
                        photocraft_geom::Rect::new((v.center[0] - hw) as i32, (v.center[1] - hh) as i32, (v.center[0] + hw) as i32, (v.center[1] + hh) as i32);
                    !clip.bounds.intersect(&r).is_empty()
                });
            let p = match app.session.active_index() {
                Some(i) if !visible => json!({"center": app.ui.views[i].center}),
                _ => json!({}),
            };
            app.run("edit.paste", p)
        }
        // Layer › New › Layer… (⇧⌘N) asks first, as in Photoshop; with params it runs as is.
        crate::new_layer_ui::COMMAND if params.as_object().is_none_or(|o| o.is_empty()) => Ok(dialog_reply(crate::new_layer_ui::open(app)?)),
        // Layer › Layer Style › <effect>… opens the Layer Style dialog on that effect.
        ls if params.as_object().is_none_or(|o| o.is_empty())
            && ls.strip_prefix("layer.layerStyle.").is_some_and(|k| crate::layer_style::KINDS.iter().any(|(kind, _)| *kind == k)) =>
        {
            let kind = &ls["layer.layerStyle.".len()..];
            crate::layer_style::open(app, Some(kind)).map(dialog_reply).ok_or_else(|| "no active layer".to_string())
        }
        "layer.videoLayers.reloadFrame" => {
            let r = app.run(id, params)?;
            // #11113: reloading a clip salvaged from a truncated GIF or WebP discloses the loss,
            // as the import and replace-footage paths do.
            crate::notices::footage_warnings(app, &r, crate::i18n::t("Reloaded the footage"));
            Ok(r)
        }
        // An id no arm took and the engine does not know belongs to a framework command set (panel commands,
        // workspace commands, `actionsPanel.*`).
        _ => {
            if photocraft_engine::commands::find(id).is_none() && crate::app::framework::run_command(app, ctx, &CommandRef::with_params(id, params.clone())) {
                return Ok(Value::Null);
            }
            app.run(id, params)
        }
    }
}

/// The text of a picked Variable Data Sets file. Invalid UTF-8 is refused, as the engine's
/// path-based import refuses it, so a failed read never replaces the document's data sets with
/// U+FFFD-substituted values.
pub(super) fn data_sets_text(bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|e| format!("the data set file is not valid UTF-8: {e}"))
}

fn open_path(app: &mut PhotocraftApp, path: &str) -> Result<Value, String> {
    app.open_path(path).map(|w| json!({"warnings": w}))
}

fn invoke_export_command(app: &mut PhotocraftApp, id: &str, params: &Value) -> Result<Option<Result<Value, String>>, String> {
    let result = match id {
        "file.export.exportAs" => Some(Ok(json!({"dialog": crate::export_dialog::open(app)?.value()}))),
        "file.export.quickExportAsPng" => Some(crate::export_dialog::quick_export_png(app)),
        "file.export.renderVideo" if params.get("dir").is_none() => {
            #[cfg(target_arch = "wasm32")]
            {
                Some(Err("Render Video to a local directory is unavailable on the web".into()))
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let document = app.session.active().ok_or("no document")?.doc.id;
                let mut render_params = params.as_object().cloned().unwrap_or_default();
                Some(app.ask_folder(move |app, dir| {
                    render_params.insert("dir".into(), json!(dir));
                    app.with_document(document, |app| app.run("file.export.renderVideo", Value::Object(render_params)))
                }))
            }
        }
        layer if matches!(layer, "layer.exportAs" | "layer.quickExportAsPng") && params.as_object().is_none_or(|object| object.is_empty()) => {
            let selected = app.session.active().and_then(|document| document.active_layer).ok_or("no active layer")?;
            if layer == "layer.exportAs" {
                // Several layers (artboards, say) selected: one file each (#3629).
                let layers = app.session.active().map(|document| document.selected_layers()).unwrap_or_default();
                if layers.len() > 1 {
                    let layers = layers.to_vec();
                    Some(Ok(json!({"dialog": crate::export_dialog::open_layers(app, &layers)?.value()})))
                } else {
                    Some(Ok(json!({"dialog": crate::export_dialog::open_layer(app, selected)?.value()})))
                }
            } else {
                Some(crate::export_dialog::quick_export_selected_layers_png(app))
            }
        }
        _ => None,
    };
    Ok(result)
}

fn invoke_type_command(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        "type.increaseSize" | "type.decreaseSize" | "type.increaseSizeMore" | "type.decreaseSizeMore"
            if params.as_object().is_none_or(|object| object.is_empty()) =>
        {
            Some((|| {
                let target = type_command_target(app).ok_or("the active layer is not a type layer")?;
                app.run(id, target)
            })())
        }
        command
            if (command.starts_with("type.antiAlias.")
                || command.starts_with("type.orientation.")
                || command.starts_with("type.openType.")
                || matches!(command, "type.warpText" | "type.loadDefaultTypeStyles"))
                && params.get("layer").is_none()
                && params.get("layers").is_none()
                && params.get("range").is_none() =>
        {
            Some((|| {
                let Some(mut target) = crate::type_tool::formatting_params(app) else { return app.run(id, params.clone()) };
                if let Some(properties) = params.as_object() {
                    for (key, value) in properties {
                        target[key] = value.clone();
                    }
                }
                app.run(id, target)
            })())
        }
        "type.editText" => Some((|| {
            crate::type_tool::edit_active(app, ctx)?;
            if let Some(focus) = ctx.memory(|memory| memory.focused()) {
                ctx.memory_mut(|memory| memory.surrender_focus(focus));
            }
            Ok(Value::Null)
        })()),
        // While editing type, Select > All works on the text (#1355, #2893); the clipboard rows are the framework's
        // `EditCommands`, which the focused text editor takes before they reach the app.
        "select.all" if app.ui.text_edit.is_some() => {
            use craft::ui::runtime::commands::{CommandSet, EditCommands, FireSource, fire};
            fire(ctx, CommandRef::new(EditCommands::SelectAll.id()), FireSource::Menu);
            Some(Ok(Value::Null))
        }
        _ => None,
    }
}

fn type_command_target(app: &PhotocraftApp) -> Option<Value> {
    let (layer, range) = crate::type_tool::target(app)?;
    let mut params = json!({"layer": layer});
    if let Some(range) = range {
        params["range"] = json!(range);
    }
    if let Some(session) = app.ui.text_edit.as_ref().map(|edit| edit.session.clone()) {
        params["coalesce"] = json!(session);
    }
    Some(params)
}

fn invoke_view_command(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        // The canvas's view commands (zoom, fits, 100%, 200%, Print Size, the rotate rows): asked of the active
        // document's canvas, which runs them in its next pass as it runs their chords.
        v if crate::shortcut_dispatch::is_view_command(v) => Some(crate::panels::actions::runner::run_view(app, v, params.clone())),
        // A menu row's Alternate: the adjustment's dialog with its last settings. `invoke` notes the palette recent.
        a if params.get("lastSettings").and_then(Value::as_bool) == Some(true) && crate::adjust_dialog::has_dialog(a) => {
            Some(crate::adjust_dialog::open_last(app, a).map(dialog_reply))
        }
        "view.editSelectedGuides" => Some(crate::new_guide_ui::open_edit(app).map(|_| Value::Null)),
        "view.clearSelectedGuides" => {
            Some(crate::guide_select::delete_selected(app).and_then(|done| if done { Ok(Value::Null) } else { Err("No guide is selected".to_string()) }))
        }
        "view.rulers" | "view.show.grid" | "view.show.guides" | "view.snap" | "view.lockGuides" => {
            let extras = &mut app.ui.extras;
            let slot = match id {
                "view.rulers" => &mut extras.rulers,
                "view.show.grid" => &mut extras.grid,
                "view.show.guides" => &mut extras.guides,
                "view.snap" => &mut extras.snap,
                _ => &mut extras.lock_guides,
            };
            *slot = !*slot;
            let enabled = *slot;
            if id == "view.show.grid" && enabled {
                app.ui.view.extras = true;
            }
            if id == "view.rulers" {
                crate::prefs_ui::remember_rulers(app, enabled);
            }
            Some(Ok(json!(enabled)))
        }
        "edit.freeTransform"
        | "edit.transform.scale"
        | "edit.transform.rotate"
        | "edit.transform.skew"
        | "edit.transform.distort"
        | "edit.transform.perspective" => Some((|| {
            if app.ui.transform.as_ref().is_some_and(|transform| transform.content_aware.is_some()) {
                return Err(crate::transform_tool::CAS_ONLY_SCALES.into());
            }
            if app.ui.transform.is_none() {
                crate::transform_tool::begin(app, ctx)?;
            }
            let now = ctx.input(|input| input.time);
            if let Some(transform) = app.ui.transform.as_mut() {
                let _ = transform.session.apply(craft::draw::ui::transform::TransformEdit::Mode(crate::transform_tool::mode_for_command(id)), now);
            }
            Ok(transform_reply(app))
        })()),
        "edit.freeTransformCopy" => Some(crate::transform_tool::begin_copy(app, ctx).map(|_| transform_reply(app))),
        "edit.transform.warp" | "layer.smartObjects.warp" if params.as_object().is_none_or(|object| object.is_empty()) => {
            Some(crate::transform_tool::begin_warp(app, ctx).map(|_| transform_reply(app)))
        }
        "edit.transform.splitWarpCrosswise"
        | "edit.transform.splitWarpHorizontally"
        | "edit.transform.splitWarpVertically"
        | "edit.transform.removeWarpSplit"
            if app.ui.transform.as_ref().is_some_and(|transform| transform.session.warp().is_some()) && params.get("warp").is_none() =>
        {
            let at = params.get("at").and_then(Value::as_array).and_then(|array| Some([array.first()?.as_f64()?, array.get(1)?.as_f64()?]));
            Some(crate::transform_tool::split(app, id, at).map(|_| transform_reply(app)))
        }
        "edit.transform.warpGrid" if app.ui.transform.as_ref().is_some_and(|transform| transform.session.warp().is_some()) && params.get("warp").is_none() => {
            Some(crate::transform_tool::edit_session_warp(app, id, params).map(|_| transform_reply(app)))
        }
        sz if crate::sizing::is_sizing(sz) && params.as_object().is_none_or(|object| object.is_empty()) => {
            Some(crate::sizing::open(app, sz).map(dialog_reply).ok_or_else(|| "no document".to_string()))
        }
        "view.newGuide" if params.as_object().is_none_or(|object| object.is_empty()) => Some(Ok(dialog_reply(crate::new_guide_ui::open(app)))),
        // Image › Adjustments › Equalize with a selection asks what to equalize, as in Photoshop.
        crate::equalize_ui::COMMAND if params.as_object().is_none_or(|object| object.is_empty()) && crate::equalize_ui::asks(app) => {
            Some(Ok(dialog_reply(crate::equalize_ui::open(app))))
        }
        // Edit › Assign Profile on a document with layers asks first (#14235).
        "edit.assignProfile" if params.as_object().is_none_or(|object| object.is_empty()) && crate::dialogs::assign_profile_layers_alert::asks(app) => {
            Some(Ok(dialog_reply(crate::dialogs::assign_profile_layers_alert::open(app))))
        }
        fl if crate::filter_dialog::has_dialog(fl) && params.as_object().is_none_or(|object| object.is_empty()) => {
            Some(crate::filter_dialog::open(app, fl).map(dialog_reply))
        }
        a if crate::adjust_dialog::has_dialog(a) && params.as_object().is_none_or(|object| object.is_empty()) => {
            Some(crate::adjust_dialog::open(app, a).map(dialog_reply))
        }
        _ => None,
    }
}

/// Appearance › Sync with System, a theme in a mode, or the next mode.
fn theme_command(app: &mut PhotocraftApp, ctx: &egui::Context, params: &Value) -> Result<Value, String> {
    match ThemeChoice::from_command(&CommandRef::with_params(THEME_COMMAND, params.clone())) {
        Some(ThemeChoice::System) => {
            app.run("prefs.set", json!({"path": "interface.appearanceMode", "value": "auto"}))?;
            crate::prefs_ui::sync_appearance(app, ctx);
            Ok(Value::Null)
        }
        Some(ThemeChoice::Theme { id, dark, .. }) => {
            app.check_theme(&id)?;
            Ok(app.pick_theme(ctx, &id, dark).map_or(Value::Null, |note| json!({"note": note})))
        }
        Some(ThemeChoice::NextMode) => {
            crate::prefs_ui::cycle_appearance(app, ctx);
            Ok(Value::Null)
        }
        _ => Err(format!("unknown theme parameters {params}")),
    }
}

/// A Language row: Automatic or a language code.
fn language_command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let code = match LanguageChoice::from_command(&CommandRef::with_params(LANGUAGE_COMMAND, params.clone())) {
        Some(LanguageChoice::Automatic) => "auto".to_string(),
        // Serbian was `sr` before the catalogs moved to craft-i18n; old scripts still switch.
        Some(LanguageChoice::Code(code)) if code.eq_ignore_ascii_case("sr") => "sr-latn".to_string(),
        Some(LanguageChoice::Code(code)) => code,
        _ => return Err(format!("unknown language parameters {params}")),
    };
    if code != "auto" && !crate::i18n::Lang::all().any(|lang| lang.code() == code) {
        return Err(format!("unknown UI language `{code}`"));
    }
    app.run("prefs.set", json!({"values": {"interface.language": code}}))
}

fn invoke_window_command(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        "window.newWindowForDocument" => Some((|| {
            let doc = app.session.active_index().ok_or("no document")?;
            let wid = app.ui.alloc_id();
            let view = app.ui.views[doc].clone();
            app.ui.windows.push(crate::state::DocWindow { id: wid, document: doc, view, open: true, floating: false, focused: false, position: None });
            Ok(json!({"window": wid}))
        })()),
        "view.darkenInterface" | "view.lightenInterface" => {
            Some(crate::prefs_ui::step_brightness(app, ctx, id == "view.lightenInterface").map(|()| json!({"theme": app.ui.theme.id().as_str()})))
        }
        THEME_COMMAND => Some(theme_command(app, ctx, params)),
        LANGUAGE_COMMAND => Some(language_command(app, params)),
        "window.home" => {
            crate::canvas::start_screen::toggle_home(app);
            Some(Ok(Value::Null))
        }
        "window.nextDocument" | "window.previousDocument" => Some((|| {
            let count = app.session.documents().len();
            let active = app.session.active_index().ok_or("no document open")?;
            let next = if id == "window.nextDocument" { (active + 1) % count } else { (active + count - 1) % count };
            if next != active {
                app.run("document.activate", json!({"document": next}))?;
            }
            app.jobs.focus = None;
            Ok(json!({"document": next}))
        })()),
        _ if id.starts_with(DOCUMENT_ITEM) => Some((|| {
            let count = app.session.documents().len();
            let next =
                id.strip_prefix(DOCUMENT_ITEM).ok_or("no such document")?.parse::<usize>().ok().filter(|index| *index < count).ok_or("no such document")?;
            app.session.set_active(next);
            app.jobs.focus = None;
            Ok(json!({"document": next}))
        })()),
        _ => None,
    }
}

/// Pointer-dependent shell command; pixel picking and selection remain engine commands. A gesture in progress needs
/// no clause here: the tool host owns presses while it runs.
fn select_layer_under_cursor(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<Value, String> {
    if app.brush_resize.active() {
        return Ok(json!({"layer": null}));
    }
    let Some(view) = crate::canvas::document_display::active_view(app) else { return Ok(json!({"layer": null})) };
    let Some(pointer) = ctx.pointer_hover_pos().filter(|p| view.rect.contains(*p)) else { return Ok(json!({"layer": null})) };
    let doc = view.to_doc(pointer);
    let (x, y) = (doc.x, doc.y);
    let inside = app
        .session
        .active()
        .is_some_and(|st| x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 && x < f64::from(st.doc.size.width) && y < f64::from(st.doc.size.height));
    if !inside {
        return Ok(json!({"layer": null}));
    }
    let picked = app.run("layer.pickAt", json!({"x": x, "y": y, "target": "layer", "select": false}))?;
    if let Some(layer) = picked.get("layer").and_then(Value::as_u64) {
        app.run("layer.select", json!({"layer": layer, "mode": "replace"}))?;
    }
    Ok(picked)
}
