//! Shared picker dialogs for delete confirmation and group creation.

use crate::pickers::browser::asset_browser::{BrowserAction, GroupView, ItemPaintState, PaintItem};

const PENDING: &str = "picker-delete-confirmation";
const NEW_GROUP: &str = "picker-new-group-dialog";
const RENAME_GROUP: &str = "picker-rename-group-dialog";
const RENAME_CARD: &str = "picker-rename-card-dialog";
const RENAME_HEADERS_AFTER_ITEMS: &str = "picker-rename-headers-after-items";
const RENAME_TARGET: &str = "target";
const RENAME_QUEUE: &str = "queue";

/// Queue the shared New Group dialog from a picker panel menu.
pub(crate) fn request_new_group(ctx: &egui::Context, panel: &str) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new((NEW_GROUP, panel)), true));
}

/// Queue one unified selection rename for the shared card/header workflow.
pub(crate) fn request_rename_selection(ctx: &egui::Context, panel: &str, action: BrowserAction) {
    let request_id = egui::Id::new((RENAME_CARD, panel, "request"));
    ctx.data_mut(|data| {
        data.insert_temp(request_id, Some(action));
        data.insert_temp(request_id.with("defer"), true);
    });
}

/// Turn New Group browser actions into a named group creation only after the dialog is accepted.
pub(crate) fn show_new_group_dialog(ui: &mut egui::Ui, panel: &str, groups: &[GroupView], actions: &mut Vec<BrowserAction>) {
    let ctx = ui.ctx().clone();
    let request_id = egui::Id::new((NEW_GROUP, panel));
    if actions.iter().any(|action| matches!(action, BrowserAction::NewGroup)) {
        actions.retain(|action| !matches!(action, BrowserAction::NewGroup));
        request_new_group(&ctx, panel);
    }
    if !ctx.data(|data| data.get_temp::<bool>(request_id).unwrap_or(false)) {
        return;
    }

    let name_id = request_id.with("name");
    let mut name = ctx.data(|data| data.get_temp::<String>(name_id)).unwrap_or_else(|| next_group_name(groups));
    let mut decision = None;
    show_group_name_editor(&ctx, request_id, "Group Name", &mut name, &mut decision);

    match decision {
        Some(true) if !name.trim().is_empty() => {
            actions.push(BrowserAction::CreateGroup(name.trim().to_owned()));
            clear_group_name_dialog(&ctx, request_id, name_id);
        }
        Some(false) => {
            clear_group_name_dialog(&ctx, request_id, name_id);
        }
        _ => {
            ctx.data_mut(|data| data.insert_temp(name_id, name));
        }
    }
}

/// Render the Rename Group dialogue and translate accepted names into rename actions.
fn show_next_group_rename(ui: &mut egui::Ui, panel: &str, actions: &mut Vec<BrowserAction>) {
    let ctx = ui.ctx().clone();
    let request_id = egui::Id::new((RENAME_GROUP, panel));
    let defer = ctx.data_mut(|data| data.remove_temp::<bool>(request_id.with("defer")).unwrap_or(false));
    let queued =
        if defer { None } else { ctx.data_mut(|data| data.remove_temp::<Option<BrowserAction>>(egui::Id::new((RENAME_GROUP, panel, "request")))).flatten() };
    if let Some(BrowserAction::BeginRenameSelection { targets }) = queued {
        let mut headers = targets.into_iter();
        let (first, remaining) = (headers.next(), headers.collect::<Vec<_>>());
        if let Some(first) = first {
            ctx.data_mut(|data| {
                let name = match &first {
                    super::selection::SelectionTarget::Group(group) => group.clone(),
                    super::selection::SelectionTarget::Folder { path, .. } => path.last().cloned().unwrap_or_default(),
                    super::selection::SelectionTarget::Item { .. } => String::new(),
                };
                data.insert_temp(request_id.with(RENAME_TARGET), first.clone());
                data.insert_temp(request_id.with(RENAME_QUEUE), remaining);
                data.insert_temp(request_id.with("name"), name);
                data.insert_temp(request_id, true);
            });
        }
    }
    if !ctx.data(|data| data.get_temp::<bool>(request_id).unwrap_or(false)) {
        return;
    }

    let name_id = request_id.with("name");
    let target = ctx.data(|data| data.get_temp::<super::selection::SelectionTarget>(request_id.with(RENAME_TARGET)));
    let Some(target) = target else { return };
    let target_name = match &target {
        super::selection::SelectionTarget::Group(group) => group.clone(),
        super::selection::SelectionTarget::Folder { path, .. } => path.last().cloned().unwrap_or_default(),
        super::selection::SelectionTarget::Item { .. } => return,
    };
    let mut name = ctx.data(|data| data.get_temp::<String>(name_id)).unwrap_or_else(|| target_name.clone());
    let mut decision = None;
    let title = if matches!(&target, super::selection::SelectionTarget::Folder { .. }) { tl!("Rename Folder") } else { tl!("Rename Group") };
    show_group_name_editor(&ctx, request_id, title, &mut name, &mut decision);

    match decision {
        Some(true) if !name.trim().is_empty() => {
            actions.push(match target {
                super::selection::SelectionTarget::Group(group) => BrowserAction::RenameGroup(group, name.trim().to_owned()),
                super::selection::SelectionTarget::Folder { group, path } => {
                    BrowserAction::RenameNestedGroup { group, group_path: path, text: name.trim().to_owned() }
                }
                super::selection::SelectionTarget::Item { .. } => return,
            });
            advance_group_rename(&ctx, request_id, name_id, true);
        }
        Some(false) => advance_group_rename(&ctx, request_id, name_id, false),
        _ => {
            ctx.data_mut(|data| data.insert_temp(name_id, name));
        }
    }
}

/// Render the card rename popup, including the adapter's real preset preview in its first column.
fn show_next_item_rename(ui: &mut egui::Ui, panel: &str, groups: &[GroupView], actions: &mut Vec<BrowserAction>, paint_item: &mut PaintItem<'_>) {
    let ctx = ui.ctx().clone();
    let request_id = egui::Id::new((RENAME_CARD, panel, "request"));
    let defer = ctx.data_mut(|data| data.remove_temp::<bool>(request_id.with("defer")).unwrap_or(false));
    let queued = if defer { None } else { ctx.data_mut(|data| data.remove_temp::<Option<BrowserAction>>(request_id)).flatten() };
    let action = if defer {
        None
    } else {
        queued.or_else(|| actions.iter().position(|action| matches!(action, BrowserAction::BeginRenameSelection { .. })).map(|index| actions.remove(index)))
    };
    if let Some(BrowserAction::BeginRenameSelection { targets }) = action {
        let headers = targets.iter().filter(|target| !matches!(target, super::selection::SelectionTarget::Item { .. })).cloned().collect::<Vec<_>>();
        let mut items = targets.into_iter().filter_map(|target| match target {
            super::selection::SelectionTarget::Item { group, path, key } => groups
                .iter()
                .find(|view| view.name == group)
                .and_then(|view| view.items.iter().find(|item| item.key == key && item.group_path == path))
                .map(|item| (group, item.clone())),
            super::selection::SelectionTarget::Group(_) | super::selection::SelectionTarget::Folder { .. } => None,
        });
        if let Some((group, item)) = items.next() {
            let name = item.name.clone();
            ctx.data_mut(|data| {
                if !headers.is_empty() {
                    data.insert_temp(egui::Id::new((RENAME_HEADERS_AFTER_ITEMS, panel)), headers);
                }
                data.insert_temp(request_id.with("targets"), items.collect::<Vec<_>>());
                data.insert_temp(request_id.with("group"), group);
                data.insert_temp(request_id.with("item"), item);
                data.insert_temp(request_id.with("name"), name);
                data.insert_temp(request_id, true);
            });
        } else if !headers.is_empty() {
            let group_request = egui::Id::new((RENAME_GROUP, panel, "request"));
            ctx.data_mut(|data| {
                data.insert_temp(group_request, Some(BrowserAction::BeginRenameSelection { targets: headers }));
                data.insert_temp(egui::Id::new((RENAME_GROUP, panel)).with("defer"), true);
            });
        }
    }
    if !ctx.data(|data| data.get_temp::<bool>(request_id).unwrap_or(false)) {
        return;
    }

    let group = ctx.data(|data| data.get_temp::<String>(request_id.with("group"))).unwrap_or_default();
    let item = ctx.data(|data| data.get_temp::<crate::pickers::browser::asset_browser::ItemView>(request_id.with("item")));
    let Some(item) = item else { return };
    let name_id = request_id.with("name");
    let mut name = ctx.data(|data| data.get_temp::<String>(name_id)).unwrap_or_else(|| item.name.clone());
    let mut decision = None;
    let title = format!("{} Name", card_type_name(panel));
    show_card_name_editor(&ctx, request_id, &title, &group, &item, groups, paint_item, &mut name, &mut decision);

    match decision {
        Some(true) if !name.trim().is_empty() => {
            actions.push(BrowserAction::RenameItem {
                group: group.clone(),
                group_path: item.group_path.clone(),
                key: item.key.clone(),
                text: name.trim().to_owned(),
            });
            advance_card_rename(&ctx, panel, request_id, name_id, true);
        }
        Some(false) => advance_card_rename(&ctx, panel, request_id, name_id, false),
        _ => {
            ctx.data_mut(|data| data.insert_temp(name_id, name));
        }
    }
}

/// Run the rename workflow for the complete browser selection, regardless of its composition.
/// Group and card editors are merely target-specific presentations within this one batch.
pub(crate) fn show_rename_selection_dialog(
    ui: &mut egui::Ui,
    panel: &str,
    groups: &[GroupView],
    actions: &mut Vec<BrowserAction>,
    paint_item: &mut PaintItem<'_>,
) {
    show_next_item_rename(ui, panel, groups, actions, paint_item);
    show_next_group_rename(ui, panel, actions);
}

fn show_group_name_editor(ctx: &egui::Context, request_id: egui::Id, title: &str, name: &mut String, decision: &mut Option<bool>) {
    let name_id = request_id.with("name");
    show_popup_dialog(ctx, request_id.with("modal"), title, egui::vec2(340.0, 63.0), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let button_width = 72.0;
            let left_width = (ui.available_width() - button_width - 12.0).max(80.0);
            ui.allocate_ui_with_layout(egui::vec2(left_width, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.label("Name");
                let output = egui::TextEdit::singleline(name).desired_width(left_width).cursor_at_end(true).show(ui);
                if !ctx.data(|data| data.get_temp::<bool>(name_id.with("focused")).unwrap_or(false)) {
                    output.response.request_focus();
                    let mut state = output.state;
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(name.chars().count()))));
                    state.store(ctx, output.response.id);
                    ctx.data_mut(|data| data.insert_temp(name_id.with("focused"), true));
                }
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                let ok = ui.add_sized(egui::vec2(button_width, 24.0), egui::Button::new(tl!("OK")));
                if ok.clicked() {
                    *decision = Some(true);
                }
                let cancel = ui.add_sized(egui::vec2(button_width, 24.0), egui::Button::new(tl!("Cancel")));
                if cancel.clicked() {
                    *decision = Some(false);
                }
                apply_dialog_keyboard(ctx, decision, &ok, &cancel);
            });
        });
    });
}

fn show_card_name_editor(
    ctx: &egui::Context,
    request_id: egui::Id,
    title: &str,
    group: &str,
    item: &crate::pickers::browser::asset_browser::ItemView,
    groups: &[GroupView],
    paint_item: &mut PaintItem<'_>,
    name: &mut String,
    decision: &mut Option<bool>,
) {
    let name_id = request_id.with("name");
    show_popup_dialog(ctx, request_id.with("modal"), title, egui::vec2(420.0, 88.0), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let preview_size = egui::vec2(88.0, 88.0);
            let (preview_rect, _) = ui.allocate_exact_size(preview_size, egui::Sense::hover());
            if let Some((group_index, item_index)) = groups.iter().enumerate().find_map(|(group_index, view)| {
                (view.name == group)
                    .then(|| {
                        view.items
                            .iter()
                            .position(|candidate| candidate.key == item.key && candidate.group_path == item.group_path)
                            .map(|item_index| (group_index, item_index))
                    })
                    .flatten()
            }) {
                paint_item(ui, preview_rect, group_index, item_index, ItemPaintState::Item { selected: false });
            }
            let middle_width = 220.0;
            ui.allocate_ui_with_layout(egui::vec2(middle_width, ui.available_height()), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.label("Name");
                let input_width = (ui.available_width() - 6.0).max(40.0);
                let output = egui::TextEdit::singleline(name).desired_width(input_width).cursor_at_end(true).show(ui);
                if !ctx.data(|data| data.get_temp::<bool>(name_id.with("focused")).unwrap_or(false)) {
                    output.response.request_focus();
                    let mut state = output.state;
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(name.chars().count()))));
                    state.store(ctx, output.response.id);
                    ctx.data_mut(|data| data.insert_temp(name_id.with("focused"), true));
                }
            });
            ui.vertical(|ui| {
                let ok = ui.add_sized(egui::vec2(72.0, 24.0), egui::Button::new(tl!("OK")));
                if ok.clicked() {
                    *decision = Some(true);
                }
                let cancel = ui.add_sized(egui::vec2(72.0, 24.0), egui::Button::new(tl!("Cancel")));
                if cancel.clicked() {
                    *decision = Some(false);
                }
                apply_dialog_keyboard(ctx, decision, &ok, &cancel);
            });
        });
    });
}

fn card_type_name(panel: &str) -> &'static str {
    match panel {
        "gradients" => tl!("Gradient"),
        "patterns" => tl!("Pattern"),
        "swatches" => tl!("Swatch"),
        "styles" => tl!("Style"),
        "shapes" => tl!("Shape"),
        "toolPresets" => tl!("Tool Preset"),
        "cloneSource" => tl!("Clone Source"),
        "brushes" => tl!("Brush"),
        _ => tl!("Preset"),
    }
}

fn advance_card_rename(ctx: &egui::Context, panel: &str, request_id: egui::Id, name_id: egui::Id, confirmed: bool) {
    ctx.data_mut(|data| {
        data.remove::<String>(name_id);
        data.remove::<bool>(name_id.with("focused"));
        let mut queue = data.get_temp::<Vec<(String, crate::pickers::browser::asset_browser::ItemView)>>(request_id.with("targets")).unwrap_or_default();
        if confirmed && !queue.is_empty() {
            let (group, item) = queue.remove(0);
            data.insert_temp(request_id.with("group"), group);
            data.insert_temp(name_id, item.name.clone());
            data.insert_temp(request_id.with("item"), item);
            data.insert_temp(request_id.with("targets"), queue);
            data.insert_temp(request_id, true);
        } else {
            data.remove::<Vec<(String, crate::pickers::browser::asset_browser::ItemView)>>(request_id.with("targets"));
            data.remove::<String>(request_id.with("group"));
            data.remove::<crate::pickers::browser::asset_browser::ItemView>(request_id.with("item"));
            data.remove::<bool>(request_id);
            let pending_headers_id = egui::Id::new((RENAME_HEADERS_AFTER_ITEMS, panel));
            let pending_headers = data.get_temp::<Vec<super::selection::SelectionTarget>>(pending_headers_id);
            data.remove::<Vec<super::selection::SelectionTarget>>(pending_headers_id);
            if confirmed && let Some(headers) = pending_headers.filter(|headers| !headers.is_empty()) {
                let group_request = egui::Id::new((RENAME_GROUP, panel, "request"));
                data.insert_temp(group_request, Some(BrowserAction::BeginRenameSelection { targets: headers }));
                data.insert_temp(egui::Id::new((RENAME_GROUP, panel)).with("defer"), true);
            }
        }
    });
}

fn clear_group_name_dialog(ctx: &egui::Context, request_id: egui::Id, name_id: egui::Id) {
    ctx.data_mut(|data| {
        data.remove::<String>(name_id);
        data.remove::<bool>(name_id.with("focused"));
        data.remove::<bool>(request_id);
    });
}

fn advance_group_rename(ctx: &egui::Context, request_id: egui::Id, name_id: egui::Id, confirmed: bool) {
    ctx.data_mut(|data| {
        data.remove::<String>(name_id);
        data.remove::<bool>(name_id.with("focused"));
        data.remove::<super::selection::SelectionTarget>(request_id.with(RENAME_TARGET));
        let mut queue = data.get_temp::<Vec<super::selection::SelectionTarget>>(request_id.with(RENAME_QUEUE)).unwrap_or_default();
        if confirmed && !queue.is_empty() {
            let next = queue.remove(0);
            let name = match &next {
                super::selection::SelectionTarget::Group(group) => group.clone(),
                super::selection::SelectionTarget::Folder { path, .. } => path.last().cloned().unwrap_or_default(),
                super::selection::SelectionTarget::Item { .. } => String::new(),
            };
            data.insert_temp(request_id.with(RENAME_TARGET), next.clone());
            data.insert_temp(name_id, name);
            data.insert_temp(request_id.with(RENAME_QUEUE), queue);
            data.insert_temp(request_id, true);
        } else {
            data.remove::<Vec<super::selection::SelectionTarget>>(request_id.with(RENAME_QUEUE));
            data.remove::<bool>(request_id);
        }
    });
}

/// Enter confirms the active picker dialog unless a dialog button owns keyboard focus; in that
/// case egui's focused button handles Enter itself. Escape always aborts the dialog.
fn apply_dialog_keyboard(ctx: &egui::Context, decision: &mut Option<bool>, confirm: &egui::Response, cancel: &egui::Response) {
    let (enter, escape) = ctx.input(|input| (input.key_pressed(egui::Key::Enter), input.key_pressed(egui::Key::Escape)));
    if escape {
        *decision = Some(false);
    } else if enter && !confirm.has_focus() && !cancel.has_focus() {
        *decision = Some(true);
    }
}

fn next_group_name(groups: &[GroupView]) -> String {
    let next =
        groups.iter().filter_map(|group| group.name.strip_prefix("Group ").and_then(|suffix| suffix.parse::<u64>().ok())).max().unwrap_or(0).saturating_add(1);
    format!("Group {next}")
}

/// Hold delete actions until the user confirms them. The popup is shared by browser context menus,
/// panel menus, and footer controls so none of those entry points can bypass confirmation.
pub(crate) fn confirm_deletions(ui: &mut egui::Ui, panel: &str, groups: &[GroupView], actions: Vec<BrowserAction>) -> Vec<BrowserAction> {
    let ctx = ui.ctx().clone();
    let id = egui::Id::new((PENDING, panel));
    let has_pending = ctx.data(|data| data.get_temp::<Vec<BrowserAction>>(id).is_some());
    let mut passthrough = Vec::new();
    if !has_pending && actions.iter().any(is_delete) {
        let deletions = actions.iter().filter(|action| is_delete(action)).cloned().collect::<Vec<_>>();
        passthrough.extend(actions.into_iter().filter(|action| !is_delete(action)));
        ctx.data_mut(|data| data.insert_temp(id, deletions));
    } else if !has_pending {
        return actions;
    }

    let Some(pending) = ctx.data(|data| data.get_temp::<Vec<BrowserAction>>(id)) else { return passthrough };
    let (kind, name) = describe(&pending, groups, panel);
    let lang = crate::i18n::current();
    let (one, other) = delete_confirmation_messages(&kind);
    let message = crate::i18n::fmt(&crate::i18n::trn(lang, pending.len() as u64, one, other), &[("name", &name)]);

    let decision = show_confirmation_dialog(&ctx, id.with("modal"), "PhotoCraft", &message, &tl!("OK"), &tl!("Cancel"));
    match decision {
        Some(true) => {
            let mut confirmed = ctx.data_mut(|data| data.remove_temp::<Vec<BrowserAction>>(id).unwrap_or_default());
            confirmed.extend(passthrough);
            confirmed
        }
        Some(false) => {
            ctx.data_mut(|data| data.remove_temp::<Vec<BrowserAction>>(id));
            Vec::new()
        }
        None => passthrough,
    }
}

/// Show a compact, draggable popup with the standard card frame and title band.
fn show_popup_dialog<T>(ctx: &egui::Context, id: egui::Id, title: &str, content_size: egui::Vec2, contents: impl FnOnce(&mut egui::Ui) -> T) -> T {
    const MARGIN: f32 = 12.0;
    const BORDER: f32 = 1.0;
    const TITLE_HEIGHT: f32 = 32.0;
    const TITLE_INSET: f32 = 10.0;

    let position_id = id.with("position");
    let frame_size = content_size + egui::vec2(2.0 * (MARGIN + BORDER), 2.0 * (MARGIN + BORDER));
    let screen = ctx.content_rect();
    let default_pos = (screen.size() - frame_size) * 0.5;
    let position = ctx.data_mut(|data| data.get_persisted::<egui::Vec2>(position_id)).unwrap_or(default_pos);
    let theme = crate::theme::Tokens::get(ctx);
    let frame = egui::Frame::NONE.fill(theme.card).stroke(egui::Stroke::new(BORDER, theme.card_border)).inner_margin(egui::Margin::same(MARGIN as i8));
    let area = egui::Modal::default_area(id).anchor(egui::Align2::LEFT_TOP, position);
    let mut drag_delta = egui::Vec2::ZERO;
    // Reserve a full title band plus a 12 px gap before laying out dialog contents.
    // The frame's own inner margin supplies the matching left/right/bottom inset.
    let layout_size = content_size + egui::vec2(0.0, TITLE_HEIGHT - BORDER);
    let response = egui::Modal::new(id).area(area).frame(frame).backdrop_color(egui::Color32::TRANSPARENT).show(ctx, |ui| {
        ui.set_min_size(layout_size);
        ui.set_max_size(layout_size);
        let content_rect = ui.max_rect();
        let original_clip = ui.clip_rect();
        // The title band intentionally extends through the frame margin to the outer top-left edge.
        ui.set_clip_rect(original_clip.expand(MARGIN + BORDER));
        let title_bar_rect = egui::Rect::from_min_size(
            content_rect.min - egui::vec2(MARGIN + BORDER, MARGIN + BORDER),
            egui::vec2(content_rect.width() + 2.0 * (MARGIN + BORDER), TITLE_HEIGHT),
        );
        ui.painter().rect_filled(title_bar_rect, 0.0, theme.dock);
        ui.painter().text(
            title_bar_rect.left_center() + egui::vec2(TITLE_INSET, 0.0),
            egui::Align2::LEFT_CENTER,
            title,
            egui::FontId::proportional(14.0),
            theme.text,
        );
        // A separate child keeps the full title band draggable without expanding the frame's
        // content min_rect. Interacting directly on the content UI would shift the frame itself.
        let title_ui = ui.new_child(egui::UiBuilder::new().max_rect(title_bar_rect));
        drag_delta = title_ui.interact(title_bar_rect, id.with("title-bar"), egui::Sense::drag()).drag_delta();
        ui.set_clip_rect(original_clip);
        ui.add_space(TITLE_HEIGHT - BORDER);
        contents(ui)
    });
    if drag_delta != egui::Vec2::ZERO {
        ctx.data_mut(|data| data.insert_persisted(position_id, position + drag_delta));
    }
    response.inner
}

/// Returns `Some(true)` for OK, `Some(false)` for Cancel, and `None` while still open.
fn show_confirmation_dialog(ctx: &egui::Context, id: egui::Id, title: &str, message: &str, ok_label: &str, cancel_label: &str) -> Option<bool> {
    let mut decision = None;
    show_popup_dialog(ctx, id, title, egui::vec2(256.0, 64.0), |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(egui::RichText::new("⚠").size(20.0));
                ui.add_space(8.0);
                let message_width = ui.available_width();
                ui.add_sized(egui::vec2(message_width, ui.available_height()), egui::Label::new(message).wrap());
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.add_space((ui.available_width() - 162.0) * 0.5);
                let ok = ui.add_sized(egui::vec2(75.0, 20.0), egui::Button::new(ok_label));
                if ok.clicked() {
                    decision = Some(true);
                }
                let cancel = ui.add_sized(egui::vec2(75.0, 20.0), egui::Button::new(cancel_label));
                if cancel.clicked() {
                    decision = Some(false);
                }
                apply_dialog_keyboard(ctx, &mut decision, &ok, &cancel);
            });
        });
    });
    decision
}

fn is_delete(action: &BrowserAction) -> bool {
    matches!(action, BrowserAction::DeleteItem { .. } | BrowserAction::DeleteGroup(_) | BrowserAction::DeleteNestedGroup { .. })
}

fn describe(actions: &[BrowserAction], groups: &[GroupView], panel: &str) -> (String, String) {
    let contains_group = actions.iter().any(|action| matches!(action, BrowserAction::DeleteGroup(_) | BrowserAction::DeleteNestedGroup { .. }));
    let contains_item = actions.iter().any(|action| matches!(action, BrowserAction::DeleteItem { .. }));
    if contains_group && contains_item {
        let kind = match panel {
            "gradients" => "gradient",
            "swatches" => "swatch",
            "patterns" => "pattern",
            "styles" => "style",
            "shapes" => "shape",
            "brushes" => "brush",
            "toolPresets" => "tool preset",
            "cloneSource" => "clone source",
            _ => "item",
        };
        return (format!("{kind} and group"), String::new());
    }
    if groups_only(actions) {
        let name = match actions.first() {
            Some(BrowserAction::DeleteGroup(group)) => group.clone(),
            Some(BrowserAction::DeleteNestedGroup { group_path, .. }) => group_path.last().cloned().unwrap_or_default(),
            _ => String::new(),
        };
        return ("group".into(), name);
    }
    let kind = match panel {
        "gradients" => "gradient",
        "swatches" => "swatch",
        "patterns" => "pattern",
        "styles" => "style",
        "shapes" => "shape",
        "brushes" => "brush",
        "toolPresets" => "tool preset",
        "cloneSource" => "clone source",
        _ => "item",
    };
    let name = match actions.first() {
        Some(BrowserAction::DeleteItem { group, key, .. }) => groups
            .iter()
            .find(|g| &g.name == group)
            .and_then(|g| g.items.iter().find(|item| &item.key == key))
            .map(|item| item.name.clone())
            .unwrap_or_else(|| key.clone()),
        _ => String::new(),
    };
    (kind.into(), name)
}

fn groups_only(actions: &[BrowserAction]) -> bool {
    !actions.is_empty() && actions.iter().all(|action| matches!(action, BrowserAction::DeleteGroup(_) | BrowserAction::DeleteNestedGroup { .. }))
}

fn delete_confirmation_messages(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "gradient and group" => ("Delete the selected gradient and group?", "Delete the selected gradients and groups?"),
        kind if kind.ends_with(" and group") => ("Delete the selected item and group?", "Delete the selected items and groups?"),
        "group" => ("Delete group \"{name}\" and its gradients?", "Delete the selected groups and their gradients?"),
        "gradient" => ("Delete gradient \"{name}\"?", "Delete the selected gradients?"),
        "swatch" => ("Delete swatch \"{name}\"?", "Delete the selected swatches?"),
        "pattern" => ("Delete pattern \"{name}\"?", "Delete the selected patterns?"),
        "style" => ("Delete style \"{name}\"?", "Delete the selected styles?"),
        "shape" => ("Delete shape \"{name}\"?", "Delete the selected shapes?"),
        "brush" => ("Delete brush \"{name}\"?", "Delete the selected brushes?"),
        "tool preset" => ("Delete tool preset \"{name}\"?", "Delete the selected tool presets?"),
        "clone source" => ("Delete clone source \"{name}\"?", "Delete the selected clone sources?"),
        _ => ("Delete item \"{name}\"?", "Delete the selected items?"),
    }
}

#[cfg(test)]
mod tests {
    use egui::vec2;
    use egui_kittest::{Harness, kittest::Queryable};

    use super::{next_group_name, request_new_group, show_new_group_dialog, show_popup_dialog, show_rename_selection_dialog};
    use crate::pickers::browser::asset_browser::GroupView;

    #[test]
    fn new_group_name_increments_the_highest_exact_group_number() {
        let groups = [
            GroupView { name: "Group 2".into(), root_only: false, items: Vec::new() },
            GroupView { name: "Group 10".into(), root_only: false, items: Vec::new() },
            GroupView { name: "Group 3 backup".into(), root_only: false, items: Vec::new() },
        ];
        assert_eq!(next_group_name(&groups), "Group 11");
        assert_eq!(next_group_name(&[]), "Group 1");
    }

    #[test]
    fn new_group_dialog_shows_name_field_and_stacked_confirmation_buttons() {
        let groups = [GroupView { name: "Group 4".into(), root_only: false, items: Vec::new() }];
        let mut harness = Harness::builder().with_size(vec2(640.0, 480.0)).build_ui(|ui| {
            request_new_group(ui.ctx(), "test-panel");
            let mut actions = Vec::new();
            show_new_group_dialog(ui, "test-panel", &groups, &mut actions);
        });
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);

        let name = harness.get_by_label("Name").rect();
        let ok = harness.get_by_label("OK").rect();
        let cancel = harness.get_by_label("Cancel").rect();
        assert!(cancel.top() >= ok.bottom(), "Cancel is below OK: OK={ok:?}, Cancel={cancel:?}");
        assert!(name.left() < ok.left(), "the name field occupies the left column: name={name:?}, buttons={ok:?}");
        let tokens = crate::theme::Tokens::get(&harness.ctx);
        let card = harness
            .output()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect) if rect.fill == tokens.card && rect.rect.width() > 300.0 => Some(rect.rect),
                _ => None,
            })
            .min_by(|a, b| a.width().total_cmp(&b.width()))
            .expect("new group popup frame");
        let title = harness
            .output()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect) if rect.fill == tokens.dock && rect.rect.width() > 300.0 => Some(rect.rect),
                _ => None,
            })
            .max_by(|a, b| a.width().total_cmp(&b.width()))
            .expect("new group title bar");
        assert!((card.height() - 120.0).abs() < 1.0, "new group popup is 120 px high: {card:?}");
        assert!(name.top() - title.bottom() >= 11.0, "new group content is inset below its title bar: title={title:?}, name={name:?}");
    }

    #[test]
    fn rename_group_opens_the_shared_dialog_with_the_existing_name() {
        let mut harness = Harness::builder().with_size(vec2(640.0, 480.0)).build_ui(|ui| {
            let mut actions =
                vec![super::BrowserAction::BeginRenameSelection { targets: vec![super::super::selection::SelectionTarget::Group("Existing Group".into())] }];
            let mut paint = |_: &egui::Ui, _: egui::Rect, _: usize, _: usize, _: super::ItemPaintState| {};
            show_rename_selection_dialog(ui, "rename-test", &[], &mut actions, &mut paint);
            assert!(actions.is_empty(), "the rename request is consumed by the dialog");
        });
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);

        assert!(harness.query_by_label("Name").is_some(), "the shared name editor is displayed");
        assert!(
            harness.output().shapes.iter().any(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => text.galley.text() == "Rename Group",
                _ => false,
            }),
            "the popup title is Rename Group"
        );
    }

    #[test]
    fn renaming_a_selected_folder_header_targets_only_that_header() {
        let initial = vec![super::BrowserAction::BeginRenameSelection {
            targets: vec![super::super::selection::SelectionTarget::Folder { group: "Parent".into(), path: vec!["Nested".into()] }],
        }];
        let mut harness = Harness::builder().with_size(vec2(640.0, 480.0)).build_ui_state(
            |ui, actions: &mut Vec<super::BrowserAction>| {
                let mut paint = |_: &egui::Ui, _: egui::Rect, _: usize, _: usize, _: super::ItemPaintState| {};
                show_rename_selection_dialog(ui, "folder-rename-test", &[], actions, &mut paint);
            },
            initial,
        );
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);
        assert!(harness.query_by_label("Name").is_some());
        assert!(harness.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text() == "Rename Folder")));

        harness.get_by_label("OK").click();
        harness.run_steps(3);

        assert!(harness.state().iter().any(|action| matches!(
            action,
            super::BrowserAction::RenameNestedGroup { group, group_path, text }
                if group == "Parent" && group_path == &["Nested"] && text == "Nested"
        )));
    }

    #[test]
    fn mixed_selection_rename_runs_card_before_parent_group_dialog() {
        use crate::pickers::browser::asset_browser::{BrowserAction, ItemPaintState, ItemView};

        let groups = vec![GroupView {
            name: "A".into(),
            root_only: false,
            items: vec![ItemView { key: "gradient-a".into(), name: "Gradient A".into(), group_path: Vec::new() }],
        }];
        let initial = vec![BrowserAction::BeginRenameSelection {
            targets: vec![
                super::super::selection::SelectionTarget::Group("B".into()),
                super::super::selection::SelectionTarget::Item { group: "A".into(), path: Vec::new(), key: "gradient-a".into() },
            ],
        }];
        let mut harness = Harness::builder().with_size(vec2(640.0, 480.0)).build_ui_state(
            |ui, actions: &mut Vec<BrowserAction>| {
                let mut paint_item = |_: &egui::Ui, _: egui::Rect, _: usize, _: usize, _: ItemPaintState| {};
                show_rename_selection_dialog(ui, "gradients", &groups, actions, &mut paint_item);
            },
            initial,
        );
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);

        assert!(harness.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text().contains("Name"))));
        harness.get_by_label("OK").click();
        harness.run_steps(5);
        assert!(harness.state().iter().any(|action| matches!(action, BrowserAction::RenameItem { group, key, .. } if group == "A" && key == "gradient-a")));
        assert!(harness.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text() == "Rename Group")));
        harness.get_by_label("OK").click();
        harness.run_steps(4);
        assert!(harness.state().iter().any(|action| matches!(action, BrowserAction::RenameGroup(group, _) if group == "B")));
    }

    #[test]
    fn delete_popup_title_bar_covers_the_full_frame_when_the_message_wraps() {
        let mut harness = Harness::builder().with_size(vec2(640.0, 480.0)).build_ui(|ui| {
            super::show_confirmation_dialog(
                ui.ctx(),
                egui::Id::new("delete-title-width-test"),
                "PhotoCraft",
                "Delete the selected groups and all of their gradients? This message is intentionally long enough to wrap onto multiple lines.",
                "OK",
                "Cancel",
            );
        });
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);

        let tokens = crate::theme::Tokens::get(&harness.ctx);
        let card = harness
            .output()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect) if rect.fill == tokens.card && rect.rect.width() > 250.0 => Some(rect.rect),
                _ => None,
            })
            .min_by(|a, b| a.width().total_cmp(&b.width()))
            .expect("delete popup frame");
        let title_bar = harness
            .output()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect) if rect.fill == tokens.dock && rect.rect.width() > 250.0 => Some(rect.rect),
                _ => None,
            })
            .max_by(|a, b| a.width().total_cmp(&b.width()))
            .expect("delete popup title bar");
        assert!((title_bar.right() - card.right()).abs() < 1.0, "title bar reaches the popup's right edge: title={title_bar:?}, frame={card:?}");
    }

    #[test]
    fn title_bar_is_flush_with_the_popup_frame_top_and_left() {
        let mut h = Harness::builder().with_size(vec2(640.0, 480.0)).build_ui(|ui| {
            show_popup_dialog(ui.ctx(), egui::Id::new("popup-title-alignment-test"), "PhotoCraft", vec2(256.0, 96.0), |ui| {
                ui.label("Dialog content");
            });
        });
        crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);

        let tokens = crate::theme::Tokens::get(&h.ctx);
        let card = h
            .output()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect) if rect.fill == tokens.card && rect.rect.width() > 250.0 => Some(rect.rect),
                _ => None,
            })
            .min_by(|a, b| a.width().total_cmp(&b.width()))
            .expect("popup card frame");
        let title_band = h
            .output()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Rect(rect) if rect.fill == tokens.dock && rect.rect.width() > 250.0 => Some(rect.rect),
                _ => None,
            })
            .max_by(|a, b| a.width().total_cmp(&b.width()))
            .expect("popup title band");
        let content = h.get_by_label("Dialog content").rect();

        assert!((title_band.left() - card.left()).abs() < 1.0, "title band left {:?} != card left {:?}", title_band, card);
        assert!((title_band.top() - card.top()).abs() < 1.0, "title band top {:?} != card top {:?}", title_band, card);
        assert!((title_band.right() - card.right()).abs() < 1.0, "title band right {:?} != card right {:?}", title_band, card);
        assert!(content.top() - title_band.bottom() >= 11.0, "delete dialog content is separated from title bar: title={title_band:?}, content={content:?}");
    }
}
