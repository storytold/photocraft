//! Modern Actions list and runtime editor integration. Definitions remain engine commands.
use super::{
    Action, Frame, Gate, NEW_SET, PausedPlayback, PhotocraftApp, RESUME, Rect, Sense, Source, Tokens, Value, abandoned, actions_cmds, editor, editor_glyphs,
    icons, json, load_dialog, paused_token, run_with_parents, set_label, suspended, vec2, waiting_dialog,
};
use crate::state::{ActionsLayout, ActionsListSize};
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct Runtime {
    page: Option<editor::Page>,
    page_drawn: Option<u64>,
    pending: Option<(String, Value, Option<Gate>)>,
    task: Option<Task>,
    progress: Option<Progress>,
    pub(super) quantum: Option<usize>,
    pub(super) yielded: Option<Frame>,
    pub(super) parents: Vec<Frame>,
    pub(super) history: Option<actions_cmds::ActionHistoryContinuation>,
    delete: Option<(u64, Delete, Option<Gate>)>,
}

struct Task {
    frame: Frame,
    parents: Vec<Frame>,
    owner: Option<actions_cmds::playback::Owner>,
    gate: Option<Gate>,
    used: u64,
    ran: u64,
    then: Vec<Action>,
    history: Option<actions_cmds::ActionHistoryContinuation>,
}

struct Progress {
    root: Arc<Action>,
    completed: usize,
    done: bool,
}

enum Delete {
    Action(Box<Action>),
    Group(String, Vec<Action>, u64),
}

/// Include backend drafts and suspended playback so layout/navigation guards cannot bypass hidden owners.
pub(crate) fn editor_busy(app: &PhotocraftApp) -> bool {
    app.actions_runtime.page.is_some() || app.session.actions.editor.is_some() || app.actions_runtime.task.is_some() || suspended(app)
}

pub(crate) fn layout_checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    match id {
        "actions.layout.modern" => Some(app.ui.actions_layout == ActionsLayout::Modern),
        "actions.layout.classic" => Some(app.ui.actions_layout == ActionsLayout::Classic),
        _ => None,
    }
}

/// Route layout changes through the current editor's leave flow before replacing its presentation.
pub(crate) fn layout_command(app: &mut PhotocraftApp, id: &str) -> Option<Result<Value, String>> {
    let layout = match id {
        "actions.layout.modern" => ActionsLayout::Modern,
        "actions.layout.classic" => ActionsLayout::Classic,
        _ => return None,
    };
    if editor_busy(app) {
        return Some(leave_for(app, id, json!({})));
    }
    app.ui.actions_layout = layout;
    Some(Ok(json!({"layout":layout})))
}

/// Persist layout and row size after pointer interaction ends, and only when the saved value differs.
pub(crate) fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|input| input.pointer.any_down()) {
        return;
    }
    let saved = json!({"layout":app.ui.actions_layout,"size":app.ui.actions_list_size});
    if app.session.prefs().actions_panel != saved {
        app.session.prefs.edit(|prefs| prefs.actions_panel = saved);
    }
}

/// Restore known presentation settings, using Modern and large rows for absent or unknown values.
pub(crate) fn restore(app: &mut PhotocraftApp) {
    let saved = &app.session.prefs().actions_panel;
    app.ui.actions_layout = match saved.get("layout").and_then(Value::as_str) {
        Some("classic") => ActionsLayout::Classic,
        _ => ActionsLayout::Modern,
    };
    app.ui.actions_list_size = match saved.get("size").and_then(Value::as_str) {
        Some("small") => ActionsListSize::Small,
        _ => ActionsListSize::Large,
    };
}

/// Expose the live page and playback frontier without creating a new draft or serializing runtime owners.
pub(crate) fn inspect(app: &PhotocraftApp) -> Value {
    let page = app.actions_runtime.page.as_ref();
    json!({
        "layout":app.ui.actions_layout,"size":app.ui.actions_list_size,"search":app.ui.actions.search,
        "selected":app.ui.actions.selected,"collapsedGroups":app.ui.actions.collapsed_sets,
        "editor":page.map(|page| json!({"draft":page.draft(),"modal":page.has_modal(),
            "value":actions_cmds::editor::inspect(&app.session,page.draft()).ok()})),
        "playing":app.actions_runtime.progress.as_ref().map(|progress| json!({
            "name":progress.root.name,"progress":fraction(progress),"done":progress.done,
            "paused":app.actions_runtime.task.is_none() && !progress.done,
        })),
    })
}

fn fraction(progress: &Progress) -> f32 {
    if progress.done {
        return 1.0;
    }
    let total = progress.root.enabled_steps().count();
    if total == 0 { 0.0 } else { (progress.completed as f32 / total as f32).min(0.99) }
}

fn activity(app: &PhotocraftApp) -> editor::Activity {
    editor::Activity { playback_busy: app.actions_runtime.task.is_some() || suspended(app), finishing_step: app.session.has_recording_job() }
}

/// Prefer the current command gate, inheriting the automation gate only for automation input.
fn effective_gate(app: &PhotocraftApp) -> Option<Gate> {
    app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None })
}

/// Attach a visible page to an existing backend draft without allocating or replacing that draft.
fn attach(app: &mut PhotocraftApp) -> Result<(), String> {
    if app.actions_runtime.page.is_none()
        && let Some(draft) = app.session.actions.editor.as_ref()
    {
        let view = actions_cmds::editor::inspect(&app.session, draft.id()).map_err(|error| error.to_string())?;
        app.actions_runtime.page = Some(editor::Page::attach(&view)?);
    }
    Ok(())
}

/// An explicit successful backend Discard also closes the matching visible control-channel
/// consumer. A page executing its own leave is extracted already and handles its own event.
pub(crate) fn discarded(app: &mut PhotocraftApp, draft: u64) {
    if app.actions_runtime.page.as_ref().is_some_and(|page| page.draft() == draft) {
        app.actions_runtime.page = None;
        app.actions_runtime.pending = None;
    }
}

/// Open one owned editor page only when no draft or playback already occupies the Actions flow.
fn open(app: &mut PhotocraftApp, params: Value) -> Result<Value, String> {
    if editor_busy(app) {
        return Err(tl!("Close the open Actions editor first").into());
    }
    let page = editor::Page::open(app, params)?;
    let id = page.draft();
    app.actions_runtime.page = Some(page);
    Ok(json!({"draft":id}))
}

/// Apply page outcomes to selection, pending navigation or preview without publishing definitions directly.
fn event(app: &mut PhotocraftApp, event: editor::Event) -> Result<(), String> {
    match event {
        editor::Event::Saved { source } | editor::Event::Closed { saved: true, source } => {
            if let Some(index) = source {
                app.ui.actions.select(index, None);
            }
        }
        editor::Event::CancelledLeave => {
            app.actions_runtime.pending = None;
        }
        editor::Event::Preview { draft, from } => {
            start_preview(app, draft, from)?;
        }
        editor::Event::Closed { saved: false, .. } => {}
    }
    Ok(())
}

/// Defer a navigation effect until the owned page has saved or discarded successfully.
pub(super) fn leave_for(app: &mut PhotocraftApp, id: &str, params: Value) -> Result<Value, String> {
    if app.actions_runtime.pending.is_some() {
        return Err(tl!("Answer the open Actions editor question first").into());
    }
    attach(app)?;
    let active = activity(app);
    if active.playback_busy || active.finishing_step {
        return Err(tl!("Finish or cancel action playback and the pending step first").into());
    }
    let Some(mut page) = app.actions_runtime.page.take() else { return Err(tl!("Close the open Actions editor first").into()) };
    let previous = app.actions_runtime.pending.take();
    app.actions_runtime.pending = Some((id.to_string(), params, app.session.authorize));
    match page.request_leave(app, active) {
        Ok(Some(closed @ editor::Event::Closed { .. })) => {
            event(app, closed)?;
            finish_navigation(app)?;
        }
        Ok(Some(other)) => {
            event(app, other)?;
            app.actions_runtime.page = Some(page);
        }
        Ok(None) => {
            app.actions_runtime.page = Some(page);
        }
        Err(error) => {
            app.actions_runtime.page = Some(page);
            app.actions_runtime.pending = previous;
            return Err(error);
        }
    }
    Ok(json!({"pending":app.actions_runtime.pending.is_some()}))
}

/// Resume the parked destination with its original authorization gate; defer actual Exit to the close flow.
fn finish_navigation(app: &mut PhotocraftApp) -> Result<(), String> {
    let Some((id, params, gate)) = app.actions_runtime.pending.take() else { return Ok(()) };
    let saved = app.session.authorize;
    app.session.authorize = gate.or(saved);
    let result = if id == "__actionsExit" {
        app.actions_runtime.pending = Some((id, params, gate));
        Ok(Value::Null)
    } else {
        app.run(&id, params)
    };
    app.session.authorize = saved;
    result.map(|_| ())
}

pub(super) fn layout_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    for (id, label) in [("actions.layout.modern", "Actions"), ("actions.layout.classic", "Classic Actions")] {
        if ui.add_enabled(!editor_busy(app), egui::RadioButton::new(layout_checked(app, id) == Some(true), tl!(label))).clicked() {
            ui.close();
            let result = app_run_layout(app, id);
            report(app, result);
        }
    }
}

fn app_run_layout(app: &mut PhotocraftApp, id: &str) -> Result<Value, String> {
    app.run(id, json!({}))
}
fn report(app: &mut PhotocraftApp, result: Result<Value, String>) {
    if let Err(error) = result
        && error != crate::file_dialog::CANCELLED
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

pub(super) fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.add_enabled(false, egui::Button::new(tl!("Preview on hover")));
    let mut command = None;
    if ui.button(tl!("Import Actions…")).clicked() {
        command = Some("import");
    }
    let selected = app.ui.actions.selected.is_some_and(|index| index < app.session.actions.list.len());
    if ui.add_enabled(selected, egui::Button::new(tl!("Export Action…"))).clicked() {
        command = Some("export");
    }
    for label in ["Clear All Actions", "Playback Options…", "Allow Tool Recording"] {
        ui.add_enabled(false, egui::Button::new(tl!(label)));
    }
    layout_menu(app, ui);
    ui.separator();
    if ui.button(tl!("Close")).clicked() {
        command = Some("close");
    }
    if ui.button(tl!("Close Tab Group")).clicked() {
        command = Some("closeGroup");
    }
    if let Some(command) = command {
        ui.close();
        let result = match command {
            "import" => load_dialog(app),
            "export" => export(app),
            "close" => app.run("window.panel.close", json!({"panel":"actions"})),
            _ => app.run("window.panel.closeGroup", json!({"panel":"actions"})),
        };
        report(app, result);
    }
}

/// Export the selected action through the Photoshop-only projection and sanitize its suggested filename.
fn export(app: &mut PhotocraftApp) -> Result<Value, String> {
    let index = app.ui.actions.selected.ok_or("Select an action to export")?;
    let action = app.session.actions.list.get(index).ok_or("The selected action no longer exists")?;
    let stem = action.name.chars().map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let mut stem = photocraft_engine::file_cmds::portable_device_name(stem);
    while stem.len() > 251 {
        stem.pop();
    }
    let result = app.run("actions.save", json!({"action":index,"skipUnsupported":true}))?;
    let bytes = result["data"].as_str().and_then(photocraft_engine::paint::tile::b64_decode).ok_or("The action export returned no data")?;
    app.pick_save(&format!("{stem}.ATN"), move |app, path| {
        app.services.write.as_mut().ok_or("No writer configured")?(&path, &bytes)?;
        Ok(json!({"path":path}))
    })
}

#[derive(Clone)]
struct DragAction {
    index: usize,
    source: Action,
}

enum Operation {
    New,
    Edit(usize),
    Play(Vec<usize>),
    Duplicate(Value),
    DeleteAction(usize),
    DeleteGroup(String),
    Assign(Box<DragAction>, String),
    NewGroup,
}

fn context(ui: &mut egui::Ui, action: Option<usize>, group: Option<&str>, play: Vec<usize>, group_mutable: bool) -> Option<Operation> {
    let mut operation = None;
    if ui.add_enabled(action.is_some(), egui::Button::new(tl!("Edit Action"))).clicked() {
        operation = action.map(Operation::Edit);
    }
    if ui.button(if group.is_some() { tl!("Play Actions") } else { tl!("Play Action") }).clicked() {
        operation = Some(Operation::Play(play));
    }
    for label in ["Apply to subject", "Apply to background"] {
        ui.add_enabled(false, egui::Button::new(tl!(label)));
    }
    let duplicate = if group.is_some() { tl!("Duplicate group") } else { tl!("Duplicate action") };
    if ui.add_enabled(group_mutable, egui::Button::new(duplicate)).clicked() {
        operation = Some(Operation::Duplicate(match group {
            Some(group) => json!({"set":group}),
            None => json!({"action":action}),
        }));
    }
    let delete = if group.is_some() { tl!("Delete group") } else { tl!("Delete action") };
    if ui.add_enabled(group_mutable, egui::Button::new(delete)).clicked() {
        operation = match group {
            Some(group) => Some(Operation::DeleteGroup(group.into())),
            None => action.map(Operation::DeleteAction),
        };
    }
    if operation.is_some() {
        ui.close();
    }
    operation
}

fn thumbnail(ui: &mut egui::Ui, action: &Action, size: f32, tokens: &Tokens) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let color = actions_cmds::BUTTON_COLORS.iter().position(|name| *name == action.color).filter(|index| *index > 0);
    let fill = color.and_then(|index| tokens.action_swatches().get(index).copied()).unwrap_or(tokens.card);
    ui.painter().rect_filled(rect, tokens.radius_sm, fill);
    let glyph = if action.icon.is_empty() { "action-editor-click" } else { editor_glyphs::display_glyph(&action.icon) };
    icons::paint(ui, rect, glyph, size * 0.6, if color.is_some() { Tokens::ACTION_THUMBNAIL_GLYPH } else { tokens.text });
    response
}

fn action_row(app: &mut PhotocraftApp, ui: &mut egui::Ui, index: usize, action: &Action, indented: bool) -> Option<Operation> {
    let tokens = Tokens::get(ui.ctx());
    let size = if app.ui.actions_list_size == ActionsListSize::Large { 40.0 } else { 28.0 };
    let hovered = ui.rect_contains_pointer(Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), size)));
    let mut operation = None;
    let row = ui.horizontal(|ui| {
        if indented {
            ui.add_space(16.0);
        }
        let tile = thumbnail(ui, action, size, &tokens);
        if tile.clicked() {
            app.ui.actions.select(index, None);
            operation = Some(Operation::Play(vec![index]));
        }
        if tile.drag_started() {
            tile.dnd_set_drag_payload(DragAction { index, source: action.clone() });
        }
        let name = ui
            .add_sized(vec2((ui.available_width() - 60.0).max(20.0), size), egui::Button::new(&action.name).frame(false).sense(Sense::click_and_drag()))
            .on_hover_text(&action.name);
        if name.clicked() {
            app.ui.actions.select(index, None);
            operation = Some(Operation::Play(vec![index]));
        }
        if name.drag_started() {
            name.dnd_set_drag_payload(DragAction { index, source: action.clone() });
        }
        if (hovered || app.ui.actions.selected == Some(index)) && icons::button(ui, "settings", 24.0, false, tl!("Edit action")).clicked() {
            operation = Some(Operation::Edit(index));
        }
        let menu = icons::button(ui, "ellipsis", 24.0, false, tl!("Actions"));
        egui::Popup::menu(&menu).show(|ui| {
            operation = context(ui, Some(index), None, vec![index], true).or(operation.take());
        });
    });
    row.response.context_menu(|ui| {
        operation = context(ui, Some(index), None, vec![index], true).or(operation.take());
    });
    operation
}

pub(super) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if let Err(error) = attach(app) {
        report(app, Err(error));
    }
    if let Some(progress) = &app.actions_runtime.progress {
        ui.heading(&progress.root.name);
        ui.add(egui::ProgressBar::new(fraction(progress)).show_percentage());
        if !progress.done && ui.button(tl!("Stop")).clicked() {
            cancel(app);
        }
        return;
    }
    if let Some(mut page) = app.actions_runtime.page.take() {
        app.actions_runtime.page_drawn = Some(app.frame);
        let active = activity(app);
        let swatches = Tokens::get(ui.ctx()).action_swatches();
        let answer = page.show(app, ui, &swatches, active);
        let closed = matches!(answer, Some(editor::Event::Closed { .. }));
        if !closed {
            app.actions_runtime.page = Some(page);
        }
        if let Some(answer) = answer {
            let result = event(app, answer).and_then(|()| if closed { finish_navigation(app) } else { Ok(()) });
            report(app, result.map(|()| Value::Null));
        }
        return;
    }
    let mut operation = None;
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.ui.actions.search).hint_text(tl!("Search actions")).desired_width((ui.available_width() - 28.0).max(40.0)))
            .accessible_name(tl!("Search actions"));
        if ui.button("×").accessible_name(tl!("Clear search")).on_hover_text(tl!("Clear search")).clicked() {
            app.ui.actions.search.clear();
        }
    });
    ui.horizontal(|ui| {
        ui.strong(tl!("Yours"));
        let view = icons::button(ui, "ellipsis", 24.0, false, tl!("Sort & View options"));
        egui::Popup::menu(&view).show(|ui| {
            ui.label(tl!("List size"));
            ui.radio_value(&mut app.ui.actions_list_size, ActionsListSize::Large, tl!("Large"));
            ui.radio_value(&mut app.ui.actions_list_size, ActionsListSize::Small, tl!("Small"));
        });
        if icons::button(ui, "folder-plus", 24.0, false, tl!("Create new group")).clicked() {
            operation = Some(Operation::NewGroup);
        }
        if icons::button(ui, "plus", 24.0, false, tl!("Create new action")).clicked() {
            operation = Some(Operation::New);
        }
    });
    let list = app.session.actions.list.clone();
    let query = app.ui.actions.search.to_lowercase();
    let matches = |action: &Action| action.name.to_lowercase().contains(&query);
    let mut folders = match photocraft_engine::actions_edit_cmds::set_occurrences(&app.session) {
        Ok(folders) => folders,
        Err(error) => { app.ui.status = error; app.ui.status_error = true; return; }
    };
    // Preserve the original logical order until physical rows exist; then use their slots,
    // including empty occurrences and repeated captions.
    if !folders.iter().any(|folder| folder.origin.is_some()) {
        let names = photocraft_engine::actions_edit_cmds::set_names(&app.session);
        let ranks: std::collections::HashMap<_, _> = names.iter().enumerate().map(|(rank, name)| (name.as_str(), rank)).collect();
        folders.sort_by_key(|folder| (
            folder.name == photocraft_engine::atn_cmds::UNGROUPED_SET,
            ranks.get(folder.name.as_str()).copied().unwrap_or(usize::MAX),
        ));
    }
    let mut occurrences = std::collections::HashMap::new();
    for folder in &folders { *occurrences.entry(folder.name.as_str()).or_insert(0usize) += 1; }
    app.ui.actions.collapsed_folders.retain(|key| folders.iter().any(|row| key.matches(row)));
    if app.ui.actions.selected_folder.as_ref().is_some_and(|key| !folders.iter().any(|row| key.matches(row))) {
        app.ui.actions.selected_folder = None;
        app.ui.actions.selected_set = None;
    }
    egui::ScrollArea::vertical().id_salt("modern-actions-list").show(ui, |ui| {
        for folder in &folders {
            let group = &folder.name;
            let members = &folder.actions;
            if !query.is_empty() && !members.iter().any(|&index| matches(&list[index])) {
                continue;
            }
            let ungrouped = group == photocraft_engine::atn_cmds::UNGROUPED_SET && folder.origin.is_none() && !members.is_empty();
            if ungrouped {
                ui.separator();
                for &index in members {
                    if matches(&list[index]) {
                        operation = action_row(app, ui, index, &list[index], false).or(operation.take());
                    }
                }
                continue;
            }
            let key = super::FolderSelection::from_row(folder);
            let collapsed = app.ui.actions.folder_collapsed(&key);
            // The existing group mutation commands accept a logical caption. Until the
            // serialized slot-selector extension lands, they cannot mutate one namesake.
            let group_mutable = occurrences.get(group.as_str()) == Some(&1);
            ui.push_id(("modern-physical-folder", key.origin, key.slot, group), |ui| {
                let header = ui.horizontal(|ui| {
                    if icons::button(ui, if collapsed { "chevron-right" } else { "chevron-down" }, 20.0, false, group).clicked() {
                        app.ui.actions.toggle_folder(key.clone());
                    }
                    let selected = app.ui.actions.selected.is_none() && app.ui.actions.selected_folder.as_ref().is_some_and(|selected| selected.matches(folder));
                    if ui.selectable_label(selected, set_label(group)).clicked() {
                        app.ui.actions.select_folder(key.clone());
                    }
                    let menu = icons::button(ui, "ellipsis", 24.0, false, tl!("Actions"));
                    egui::Popup::menu(&menu).show(|ui| {
                        operation = context(ui, None, Some(group), members.clone(), group_mutable).or(operation.take());
                    });
                });
                if group_mutable && let Some(drag) = header.response.dnd_release_payload::<DragAction>() {
                    operation = Some(Operation::Assign(Box::new((*drag).clone()), group.clone()));
                }
                header.response.context_menu(|ui| {
                    operation = context(ui, None, Some(group), members.clone(), group_mutable).or(operation.take());
                });
                if !collapsed || !query.is_empty() {
                    for &index in members {
                        if matches(&list[index]) {
                            operation = action_row(app, ui, index, &list[index], true).or(operation.take());
                        }
                    }
                }
            });
        }
        if list.is_empty() {
            let tokens = Tokens::get(ui.ctx());
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 70.0), Sense::hover());
            icons::paint(ui, rect, "action-editor-click", 36.0, tokens.text_faint);
            ui.label(tl!("You haven't created any actions"));
            ui.label(tl!("Import actions or create a new action to get started."));
        }
    });
    if let Some(operation) = operation {
        let result = match operation {
            Operation::New => {
                let set = app.ui.actions.selected_set.clone();
                open(app, json!({"set":set}))
            }
            Operation::Edit(index) => open(app, json!({"action":index})),
            Operation::Play(indices) => start_library(app, &indices),
            Operation::Duplicate(params) => app.run("actions.duplicate", params),
            Operation::DeleteAction(index) => ask_delete(app, Some(index), None),
            Operation::DeleteGroup(group) => ask_delete(app, None, Some(group)),
            Operation::Assign(drag, group) => {
                if app.session.actions.list.get(drag.index) != Some(&drag.source) {
                    Err(tl!("The dragged action changed; drag it again").into())
                } else {
                    app.run("actions.assignSet", json!({"action":drag.index,"set":group}))
                }
            }
            Operation::NewGroup => new_group(app),
        };
        report(app, result);
    }
}

fn new_group(app: &mut PhotocraftApp) -> Result<Value, String> {
    let names = photocraft_engine::actions_edit_cmds::set_names(&app.session);
    let name = (1u32..).map(|number| format!("Group {number}")).find(|name| !names.contains(name)).ok_or("No free group name")?;
    let fields = json!({NEW_SET:true,"__command":"actions.newSet","__label":"New group","name":name});
    let id = app.ui.open_dialog(crate::state::DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    Ok(json!({"dialog":id}))
}

/// Queue an immutable draft snapshot with the backend owner that suspends capture for its entire preview.
fn start_preview(app: &mut PhotocraftApp, draft: u64, from: usize) -> Result<Value, String> {
    let (action, owner) = actions_cmds::playback::preview(&mut app.session, draft, from).map_err(|error| error.to_string())?;
    let frame = Frame { action, from, source: Source::Draft(draft), take: app.session.actions.take };
    queue(app, frame, owner, Vec::new())
}

/// Snapshot remaining group actions and acquire a frame-spanning owner for the first resolved definition.
fn start_library(app: &mut PhotocraftApp, indices: &[usize]) -> Result<Value, String> {
    let Some((&index, rest)) = indices.split_first() else { return Ok(json!({"ran":0})) };
    let then = rest
        .iter()
        .map(|&index| app.session.actions.list.get(index).cloned().ok_or("The selected action no longer exists".to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let (action, from, owner) = actions_cmds::playback::Owner::library(&mut app.session, &json!({"action":index})).map_err(|error| error.to_string())?;
    let frame = Frame { action, from, source: Source::Library, take: app.session.actions.take };
    queue(app, frame, owner, then)
}

/// Install one incremental task and its authorization gate, releasing the supplied owner on admission failure.
fn queue(app: &mut PhotocraftApp, frame: Frame, owner: actions_cmds::playback::Owner, then: Vec<Action>) -> Result<Value, String> {
    if app.actions_runtime.task.is_some() {
        owner.finish(&mut app.session);
        return Err("Another action is already playing".into());
    }
    if let Err(error) = owner.yielding(&mut app.session, true) {
        owner.finish(&mut app.session);
        return Err(error.to_string());
    }
    app.actions_runtime.progress = Some(Progress { root: frame.action.clone(), completed: 0, done: false });
    app.actions_runtime.task = Some(Task { frame, parents: Vec::new(), owner: Some(owner), gate: effective_gate(app), used: 0, ran: 0, then, history: None });
    Ok(json!({"playing":true}))
}

/// Transfer a confirmed dialog continuation back into the tick task without replacing its owner or budgets.
pub(super) fn resume_tick(app: &mut PhotocraftApp, paused: PausedPlayback) -> Result<(), String> {
    if let Some(owner) = &paused.record
        && let Err(error) = owner.yielding(&mut app.session, true)
    {
        if let Some(owner) = paused.record {
            owner.finish(&mut app.session);
        }
        app.actions_runtime.progress = None;
        return Err(error.to_string());
    }
    app.actions_runtime.task = Some(Task {
        frame: paused.frame,
        parents: paused.parents,
        owner: paused.record,
        gate: paused.gate,
        used: paused.used,
        ran: paused.ran.saturating_add(1),
        then: paused.then,
        history: paused.history,
    });
    Ok(())
}

pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    tick_with_quantum(app, ctx, 8);
}

/// Advance bounded work under the captured gate, transferring ownership across nested frames and dialogs.
/// Completion, cancellation and failure must release the same capture owner.
fn tick_with_quantum(app: &mut PhotocraftApp, ctx: &egui::Context, quantum: usize) {
    if app.actions_runtime.page.is_none() && app.actions_runtime.pending.as_ref().is_some_and(|(id, _, _)| id == "__actionsExit") {
        app.actions_runtime.pending = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    let orphaned: Vec<_> = app
        .paused_playbacks
        .paused
        .iter()
        .filter_map(|(token, paused)| {
            let open = if paused.dialog == 0 {
                app.ui.type_panels.spell.as_ref().is_some_and(|dialog| dialog.resume == Some(*token))
            } else {
                app.ui.dialogs.iter().any(|dialog| dialog.id == paused.dialog)
            };
            (paused.incremental && !open).then_some(*token)
        })
        .collect();
    for token in orphaned {
        abandoned(app, &serde_json::Map::from_iter([(RESUME.into(), json!(token))]));
    }
    let Some(mut task) = app.actions_runtime.task.take() else {
        if app.actions_runtime.progress.as_ref().is_some_and(|progress| progress.done) {
            app.actions_runtime.progress = None;
        }
        return;
    };
    let saved = app.session.authorize;
    app.actions_runtime.quantum = Some(quantum.max(1));
    app.actions_runtime.yielded = None;
    app.actions_runtime.parents.clear();
    app.actions_runtime.history = None;
    let result = (|| {
        let service = if app.automation_input { app.services.automation_authorize } else { None };
        let gate = super::continuation_gate(task.gate, super::continuation_gate(saved, service)?)?;
        let owner = task.owner.as_ref().ok_or("The playback lost its owner")?;
        owner.yielding(&mut app.session, false).map_err(|error| error.to_string())?;
        if let Source::Draft(id) = task.frame.source
            && owner.draft_id() != Some(id)
        {
            return Err("The draft preview lost its owner".into());
        }
        task.frame.validate(&app.session)?;
        if let Some(history) = task.history.take() {
            history.resume(&mut app.session).map_err(|error| error.to_string())?;
        }
        app.session.authorize = gate;
        run_with_parents(app, task.frame.clone(), &task.parents, task.used, true)
    })();
    app.actions_runtime.quantum = None;
    app.session.authorize = saved;
    task.history = app.actions_runtime.history.take();
    task.used = app.session.actions.playback_steps;
    let result = result.and_then(|value| {
        task.ran = task.ran.saturating_add(value.get("ran").and_then(Value::as_u64).unwrap_or(0));
        if let Some(failed) = value.get("failed") {
            return Err(failed.get("error").and_then(Value::as_str).unwrap_or("Action playback failed").to_string());
        }
        if let Some(dialog) = value.get("paused").and_then(waiting_dialog) {
            let token = paused_token(app, dialog);
            let entry = token.and_then(|token| app.paused_playbacks.paused.get_mut(&token)).ok_or("The action step dialog lost its continuation")?;
            entry.parents.extend(std::mem::take(&mut task.parents));
            entry.used = task.used;
            entry.ran = task.ran;
            entry.record = task.owner.take();
            entry.incremental = true;
            entry.then = std::mem::take(&mut task.then);
            return Ok(false);
        }
        if let Some(frame) = app.actions_runtime.yielded.take() {
            let mut parents = std::mem::take(&mut app.actions_runtime.parents);
            parents.append(&mut task.parents);
            task.parents = parents;
            task.frame = frame;
        } else if !task.parents.is_empty() {
            task.frame = task.parents.remove(0);
        } else {
            return Ok(true);
        }
        if let Some(progress) = &mut app.actions_runtime.progress {
            let root_frame = task.parents.last().unwrap_or(&task.frame);
            let frontier = if task.parents.is_empty() { root_frame.from } else { root_frame.from.saturating_sub(1) };
            progress.completed = progress.root.enabled_steps().filter(|(index, _)| *index < frontier).count();
        }
        task.owner.as_ref().ok_or("The playback lost its owner")?.yielding(&mut app.session, true).map_err(|error| error.to_string())?;
        Ok(false)
    });
    match result {
        Ok(false) if task.owner.is_some() => {
            app.actions_runtime.task = Some(task);
            ctx.request_repaint();
        }
        Ok(false) => {
            ctx.request_repaint();
        }
        Ok(true) => {
            if let Some(owner) = task.owner.take() {
                owner.finish(&mut app.session);
            }
            if task.then.is_empty() {
                if let Some(progress) = &mut app.actions_runtime.progress {
                    progress.done = true;
                }
                ctx.request_repaint();
            } else {
                let indices: Option<Vec<_>> = task.then.iter().map(|action| app.session.actions.list.iter().position(|current| current == action)).collect();
                let result = indices.ok_or("A group action changed during playback".to_string()).and_then(|indices| start_library(app, &indices));
                if result.is_err() {
                    app.actions_runtime.progress = None;
                }
                report(app, result);
                ctx.request_repaint();
            }
        }
        Err(error) => {
            actions_cmds::finish_pending_history(&mut app.session);
            if let Some(owner) = task.owner.take() {
                owner.finish(&mut app.session);
            }
            app.actions_runtime.progress = None;
            report(app, Err(error));
            ctx.request_repaint();
        }
    }
}

/// Release incremental task and dialog owners, closing only their own Spelling panel or step dialogs.
pub(super) fn cancel(app: &mut PhotocraftApp) {
    if let Some(mut task) = app.actions_runtime.task.take()
        && let Some(owner) = task.owner.take()
    {
        owner.finish(&mut app.session);
    }
    actions_cmds::finish_pending_history(&mut app.session);
    let tokens: Vec<_> = app.paused_playbacks.paused.iter().filter_map(|(token, paused)| paused.incremental.then_some(*token)).collect();
    for token in tokens {
        if let Some(paused) = app.paused_playbacks.paused.remove(&token) {
            if paused.dialog == 0 {
                // Only this continuation owns the Spelling panel; a manual/unrelated one stays.
                if app.ui.type_panels.spell.as_ref().is_some_and(|dialog| dialog.resume == Some(token)) {
                    app.ui.type_panels.spell = None;
                }
            } else {
                app.ui.close_dialog(paused.dialog);
            }
            if let Some(owner) = paused.record {
                owner.finish(&mut app.session);
            }
        }
    }
    app.actions_runtime.progress = None;
    app.actions_runtime.yielded = None;
    app.actions_runtime.parents.clear();
}

/// Release all remaining playback captures during application exit, including non-incremental continuations.
pub(crate) fn shutdown(app: &mut PhotocraftApp) {
    cancel(app);
    for (_, paused) in std::mem::take(&mut app.paused_playbacks.paused) {
        if let Some(owner) = paused.record {
            owner.finish(&mut app.session);
        }
    }
}

pub(crate) fn guard_navigation(app: &mut PhotocraftApp, id: &str, params: Value) -> Option<Result<Value, String>> {
    editor_busy(app).then(|| leave_for(app, id, params))
}

/// Hold normal window Close behind the current editor leave question, preserving any pending destination.
pub(crate) fn guard_exit(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    if !editor_busy(app) {
        return false;
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    if app.actions_runtime.pending.is_none() {
        let result = leave_for(app, "__actionsExit", Value::Null);
        report(app, result);
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    true
}

/// Keep owned questions visible off-tab and resume parked navigation only after a successful page close.
pub(crate) fn modals(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.actions_runtime.page_drawn == Some(app.frame) {
        return;
    }
    let Some(mut page) = app.actions_runtime.page.take() else { return };
    let active = activity(app);
    let answer = page.show_modals(app, ctx, active);
    let closed = matches!(answer, Some(editor::Event::Closed { .. }));
    if !closed {
        app.actions_runtime.page = Some(page);
    }
    if let Some(answer) = answer {
        let result = event(app, answer).and_then(|()| if closed { finish_navigation(app) } else { Ok(()) });
        report(app, result.map(|()| Value::Null));
    }
}

const DELETE: &str = "__actionsModernDelete";

pub(crate) fn owns_delete(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(DELETE)
}

/// Capture the selected definition or group revision and authorization gate before opening its delete question.
fn ask_delete(app: &mut PhotocraftApp, index: Option<usize>, group: Option<String>) -> Result<Value, String> {
    if app.actions_runtime.delete.is_some() {
        return Err("Answer the open delete question first".into());
    }
    if editor_busy(app) || app.session.actions.recording_active() {
        return Err("Stop recording and close the Actions editor first".into());
    }
    let (target, message) = match group {
        Some(group) => {
            if !photocraft_engine::actions_edit_cmds::set_names(&app.session).contains(&group) {
                return Err("The selected group no longer exists".into());
            }
            let members = app
                .session
                .actions
                .list
                .iter()
                .filter(|action| photocraft_engine::atn_cmds::logical_set_name(action.set.as_deref()) == group)
                .cloned()
                .collect();
            let message = crate::i18n::fmt(tl!("Delete the group \"{name}\"?"), &[("name", &group)]);
            (Delete::Group(group, members, app.session.actions.rev), message)
        }
        None => {
            let action = index.and_then(|index| app.session.actions.list.get(index)).cloned().ok_or("The selected action no longer exists")?;
            let message = crate::i18n::fmt(tl!("Delete the action \"{name}\"?"), &[("name", &action.name)]);
            (Delete::Action(Box::new(action)), message)
        }
    };
    let fields = json!({"__label":"Delete action","message":message});
    let id = app.ui.open_dialog(crate::state::DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    if let Some(dialog) = app.ui.dialog_mut(id) {
        dialog.fields.insert(DELETE.into(), json!(id));
    }
    app.actions_runtime.delete = Some((id, target, effective_gate(app)));
    Ok(json!({"dialog":id}))
}

pub(crate) fn delete_body(ui: &mut egui::Ui, fields: &mut serde_json::Map<String, Value>) {
    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or_default());
}

/// Clear only the delete owner named by this dialog, leaving a later question intact.
pub(crate) fn delete_cancel(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>) {
    if app.actions_runtime.delete.as_ref().is_some_and(|(id, _, _)| fields.get(DELETE).and_then(Value::as_u64) == Some(*id)) {
        app.actions_runtime.delete = None;
    }
}

/// Reject a stale or ambiguous captured target before deleting it under the question's original gate.
pub(crate) fn delete_confirm(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>) -> Result<Value, String> {
    let (id, target, gate) = app.actions_runtime.delete.take().ok_or("The delete question is no longer current")?;
    if fields.get(DELETE).and_then(Value::as_u64) != Some(id) {
        return Err("The delete question has changed".into());
    }
    let params = match target {
        Delete::Action(action) => {
            let mut matches = app.session.actions.list.iter().enumerate().filter(|(_, current)| *current == action.as_ref()).map(|(index, _)| index);
            let index = matches.next().ok_or("The selected action changed; select it again")?;
            if matches.next().is_some() {
                return Err("The selected action is ambiguous".into());
            }
            json!({"action":index})
        }
        Delete::Group(group, members, revision) => {
            let current: Vec<_> = app
                .session
                .actions
                .list
                .iter()
                .filter(|action| photocraft_engine::atn_cmds::logical_set_name(action.set.as_deref()) == group)
                .cloned()
                .collect();
            if app.session.actions.rev != revision || current != members || !photocraft_engine::actions_edit_cmds::set_names(&app.session).contains(&group) {
                return Err("The selected group changed; select it again".into());
            }
            json!({"set":group})
        }
    };
    let saved = app.session.authorize;
    app.session.authorize = gate.or(saved);
    let result = app.run("actions.delete", params);
    app.session.authorize = saved;
    if result.is_ok() {
        app.ui.actions.selected = None;
        app.ui.actions.selected_step = None;
        app.ui.actions.selected_set = None;
    }
    result
}

#[cfg(test)]
#[path = "new_layout_tests.rs"]
mod tests;
