//! Modeless Edit › Find and Replace Text. The engine owns matching and replacements.

use photocraft_doc::{DocId, LayerId};
use photocraft_engine::edit_menu_cmds::FindCursor;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{PhotocraftApp, theme::Tokens};

pub(crate) const COMMAND: &str = "edit.findAndReplaceText";
pub(crate) const CONTROL_ID: &str = "findAndReplaceText";
const TITLE: &str = "Find And Replace Text";
const SIZE: egui::Vec2 = egui::vec2(446.0, 230.0);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Fields {
    find: String,
    replace: String,
    all_layers: bool,
    forward: bool,
    case_sensitive: bool,
    #[serde(rename = "wholeWord")]
    whole_word: bool,
    ignore_accents: bool,
}

impl Default for Fields {
    fn default() -> Self {
        Self { find: String::new(), replace: String::new(), all_layers: true, forward: true, case_sensitive: false, whole_word: false, ignore_accents: false }
    }
}

impl Fields {
    fn same_search(&self, other: &Self) -> bool {
        self.find == other.find
            && self.all_layers == other.all_layers
            && self.forward == other.forward
            && self.case_sensitive == other.case_sensitive
            && self.whole_word == other.whole_word
            && self.ignore_accents == other.ignore_accents
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Done,
    Find,
    Change,
    ChangeAll,
    ChangeFind,
}

impl Action {
    fn command(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Find => "find",
            Self::Change => "change",
            Self::ChangeAll => "changeAll",
            Self::ChangeFind => "changeFind",
        }
    }

    fn needs_match(self) -> bool {
        matches!(self, Self::Change | Self::ChangeFind)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum FocusField {
    Find,
    Replace,
}

#[derive(Clone, Debug, PartialEq)]
struct CanvasMatch {
    revision: u64,
    session: String,
    anchor: usize,
    caret: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct Open {
    doc: DocId,
    layer: Option<LayerId>,
    selected: Vec<LayerId>,
    anchor: Option<LayerId>,
    cursor: Option<FindCursor>,
    start: Option<LayerId>,
    canvas_match: Option<CanvasMatch>,
    focus: Option<FocusField>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FindReplaceUi {
    fields: Fields,
    open: Option<Open>,
}

pub(crate) fn open(app: &mut PhotocraftApp) -> Result<Value, String> {
    if !app.session.is_enabled(COMMAND) {
        return Err(tl!("Find and Replace Text is not available right now").into());
    }
    if let Some(window) = app.ui.find_replace.open.as_mut() {
        window.focus = Some(FocusField::Find);
    } else {
        let st = app.session.active().ok_or("no document open")?;
        app.ui.find_replace.open = Some(Open {
            doc: st.doc.id,
            layer: st.active_layer,
            selected: st.selected_layers.clone(),
            anchor: st.layer_anchor,
            cursor: None,
            start: None,
            canvas_match: None,
            focus: Some(FocusField::Find),
        });
    }
    Ok(json!({"dialog": CONTROL_ID}))
}

fn owns_edit(app: &PhotocraftApp, window: &Open) -> bool {
    window.canvas_match.as_ref().zip(app.ui.text_edit.as_ref()).is_some_and(|(m, edit)| m.session == edit.session)
}

fn has_match(app: &PhotocraftApp, window: &Open) -> bool {
    owns_edit(app, window)
        && window.canvas_match.as_ref().zip(app.ui.text_edit.as_ref()).is_some_and(|(m, edit)| {
            m.anchor == edit.anchor
                && m.caret == edit.caret
                && app.session.active().is_some_and(|st| st.doc.id == window.doc && st.revision == m.revision && st.active_layer == Some(LayerId(edit.layer)))
        })
}

pub(crate) fn close(app: &mut PhotocraftApp) -> Result<Value, String> {
    let window = app.ui.find_replace.open.take().ok_or("Find and Replace Text is closed")?;
    if owns_edit(app, &window) {
        crate::type_tool::commit(app);
    }
    let current = app.session.active_index();
    if let Some(home) = app.session.documents().iter().position(|st| st.doc.id == window.doc) {
        app.session.set_active(home);
        if app.session.active().is_some_and(|st| (st.active_layer, &st.selected_layers, st.layer_anchor) != (window.layer, &window.selected, window.anchor)) {
            let _ = app.session.restore_layer_selection(window.layer, &window.selected, window.anchor);
        }
        if let Some(current) = current.filter(|&index| index != home) {
            app.session.set_active(current);
        }
    }
    Ok(Value::Null)
}

fn sync(app: &mut PhotocraftApp) {
    if app.ui.find_replace.open.as_ref().is_some_and(|window| app.session.active().is_none_or(|st| st.doc.id != window.doc)) {
        let _ = close(app);
    }
}

fn update_fields(app: &mut PhotocraftApp, fields: Fields) {
    if !app.ui.find_replace.fields.same_search(&fields)
        && let Some(mut window) = app.ui.find_replace.open.take()
    {
        if owns_edit(app, &window) {
            crate::type_tool::commit(app);
        }
        window.cursor = None;
        window.start = None;
        window.canvas_match = None;
        app.ui.find_replace.open = Some(window);
    }
    app.ui.find_replace.fields = fields;
}

fn apply(app: &mut PhotocraftApp, action: Action, from_control: bool) -> Result<Value, String> {
    if action == Action::Done {
        return close(app);
    }
    sync(app);
    let window = app.ui.find_replace.open.as_ref().ok_or("Find and Replace Text is closed")?;
    let fields = &app.ui.find_replace.fields;
    if fields.find.is_empty() {
        return Err(tl!("Enter text to find").into());
    }
    if action.needs_match() && !has_match(app, window) {
        return Err(tl!("Find a match before changing it").into());
    }
    let mut params = serde_json::to_value(fields).map_err(|err| err.to_string())?;
    params["action"] = json!(action.command());
    if !fields.all_layers {
        params["layer"] = json!(window.layer.ok_or("no layer was selected when Find and Replace Text opened")?.0);
    }
    if from_control && let Some(authorize) = app.services.automation_command.as_ref() {
        authorize(COMMAND, &params)?;
    }
    let cursor = window.cursor.clone();
    // The dialog has its own search position. A document edit can invalidate an outside match.
    // Finding a match advances the view's revision when it selects a layer, while
    // a text edit replaces the document snapshot.
    let before = app.session.active().map(|st| st.doc.clone());
    let outside_cursor = std::mem::replace(&mut app.session.edit_state.find_cursor, cursor);
    let outside_start = std::mem::replace(&mut app.session.edit_state.find_start, window.start);
    let result = app.run(COMMAND, params);
    let unchanged = before.as_ref().zip(app.session.active()).is_some_and(|(before, after)| std::sync::Arc::ptr_eq(before, &after.doc));
    let restore = if unchanged { outside_cursor } else { None };
    let cursor = std::mem::replace(&mut app.session.edit_state.find_cursor, restore);
    let start = std::mem::replace(&mut app.session.edit_state.find_start, if unchanged { outside_start } else { None });
    let reply = result?;
    let mut window = app.ui.find_replace.open.take().ok_or("Find and Replace Text is closed")?;
    if owns_edit(app, &window) {
        crate::type_tool::commit(app);
    }
    window.cursor = cursor;
    window.start = start;
    window.canvas_match = show_match(app, &reply["found"]);
    if window.canvas_match.is_some() {
        window.focus = Some(FocusField::Replace);
    }
    app.ui.find_replace.open = Some(window);
    Ok(reply)
}

fn show_match(app: &mut PhotocraftApp, found: &Value) -> Option<CanvasMatch> {
    let layer = LayerId(found.get("layer")?.as_u64()?);
    let start = usize::try_from(found.get("start")?.as_u64()?).ok()?;
    let end = usize::try_from(found.get("end")?.as_u64()?).ok()?;
    let (session, anchor, caret) = crate::type_tool::select_find_match(app, layer, start..end)?;
    Some(CanvasMatch { session, anchor, caret, revision: app.session.active()?.revision })
}

pub(crate) fn inspect(app: &PhotocraftApp) -> Option<Value> {
    let window = app.ui.find_replace.open.as_ref()?;
    Some(json!({"id": CONTROL_ID, "kind": CONTROL_ID, "title": TITLE, "fields": app.ui.find_replace.fields, "hasMatch": has_match(app, window)}))
}

pub(crate) fn set_field(app: &mut PhotocraftApp, key: &str, value: Value) -> Result<Value, String> {
    sync(app);
    let window = inspect(app).ok_or("Find and Replace Text is closed")?;
    let mut fields = window["fields"].clone();
    let previous = fields.get_mut(key).ok_or_else(|| format!("unknown field `{key}`"))?;
    *previous = value;
    update_fields(app, serde_json::from_value(fields).map_err(|err| err.to_string())?);
    Ok(Value::Null)
}

pub(crate) fn command(app: &mut PhotocraftApp, action: &str) -> Result<Value, String> {
    let action = match action {
        "done" => Action::Done,
        "find" => Action::Find,
        "change" => Action::Change,
        "changeAll" => Action::ChangeAll,
        "changeFind" => Action::ChangeFind,
        _ => return Err(format!("unknown Find and Replace Text action `{action}`")),
    };
    apply(app, action, true)
}

fn field(ui: &mut egui::Ui, label: &str, buffer: &mut String, focus: bool) -> egui::Response {
    let label = ui.label(label);
    let response = ui.add(egui::TextEdit::singleline(buffer).desired_width(f32::INFINITY)).labelled_by(label.id);
    if focus && !ui.is_sizing_pass() {
        response.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
            use egui::text::{CCursor, CCursorRange};
            state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(buffer.chars().count()))));
            egui::TextEdit::store_state(ui.ctx(), response.id, state);
        }
    }
    response
}

pub(crate) fn escape(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    if app.ui.find_replace.open.is_some()
        && app.framework.dialogs.stack().top().is_none()
        && !crate::prefs_ui::shortcuts_open(app)
        && !egui::Popup::is_any_open(ctx)
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        let _ = close(app);
        return true;
    }
    false
}

pub(super) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) -> Option<egui::Rect> {
    sync(app);
    let window = app.ui.find_replace.open.as_ref()?;
    let can_change = has_match(app, window);
    let focus = window.focus;
    let mut fields = app.ui.find_replace.fields.clone();
    let mut action = None;
    let mut focused = false;
    let t = Tokens::get(ctx);
    let window = egui::Window::new(tl!(TITLE))
        .id(egui::Id::unique(CONTROL_ID))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .default_pos(ctx.content_rect().center() - SIZE / 2.0)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.card).inner_margin(egui::Margin::same(14)));
    let response = window.show(ctx, |ui| {
        let width = (ctx.content_rect().width() - 40.0).clamp(260.0, SIZE.x - 28.0);
        ui.set_min_width(width);
        ui.horizontal_top(|ui| {
            let buttons = [tl!("Done"), tl!("Find Next"), tl!("Change"), tl!("Change All"), tl!("Change/Find")];
            let button_width = buttons.iter().map(|label| label.chars().count() as f32 * 8.0 + 16.0).fold(96.0, f32::max);
            ui.vertical(|ui| {
                ui.set_width((width - button_width - 22.0).max(155.0));
                let find = field(ui, tl!("Find What:"), &mut fields.find, matches!(focus, Some(FocusField::Find)));
                let replace = field(ui, tl!("Change To:"), &mut fields.replace, matches!(focus, Some(FocusField::Replace)));
                focused = !ui.is_sizing_pass();
                if find.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !fields.find.is_empty() {
                    action = Some(Action::Find);
                } else if replace.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    action = Some(Action::Done);
                }
                ui.add_space(8.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.checkbox(&mut fields.all_layers, tl!("Search All Layers"));
                        ui.checkbox(&mut fields.forward, tl!("Forward"));
                    });
                    ui.vertical(|ui| {
                        ui.checkbox(&mut fields.case_sensitive, tl!("Case Sensitive"));
                        ui.checkbox(&mut fields.whole_word, tl!("Whole Word Only"));
                        // The matching option is implemented by #14663.
                        ui.add_enabled_ui(false, |ui| ui.checkbox(&mut fields.ignore_accents, tl!("Ignore Accents")));
                    });
                });
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                if ui.add_sized([button_width, 24.0], egui::Button::new(buttons[0])).clicked() {
                    action = Some(Action::Done);
                }
                ui.add_space(8.0);
                for (button, label) in [Action::Find, Action::Change, Action::ChangeAll, Action::ChangeFind].into_iter().zip(buttons.into_iter().skip(1)) {
                    let enabled = !fields.find.is_empty() && (!button.needs_match() || (can_change && app.ui.find_replace.fields.same_search(&fields)));
                    if ui.add_enabled_ui(enabled, |ui| ui.add_sized([button_width, 24.0], egui::Button::new(label))).inner.clicked() {
                        action = Some(button);
                    }
                }
            });
        });
    });
    if focused && let Some(window) = app.ui.find_replace.open.as_mut() {
        window.focus = None;
    }
    update_fields(app, fields);
    if let Some(action) = action
        && let Err(error) = apply(app, action, false)
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
    response.map(|r| r.response.rect)
}

#[cfg(test)]
#[path = "find_replace_dialog_tests.rs"]
mod tests;
