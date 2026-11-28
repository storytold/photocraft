//! Character Styles and Paragraph Styles panels: the framework's `TextStylesPanel` over the
//! engine's `type.{character,paragraph}Style.*` commands.

use craft::text::ui::style_panel::{TextStyleAction, TextStyleKind, TextStyleRow, TextStylesPanel};
use craft::ui::contract::Bind;
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::CommandRef;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The panel menu's rows.
pub(crate) fn panel_rows(_app: &PhotocraftApp) -> Vec<PanelRow> {
    TextStylesPanel::panel_rows()
}

/// The panel's commands come back to it as actions, scoped to the focused panel.
// PLAN: the lead's ruling maps `TextStyleCommand` here into the panel's action handling, but the command carries no
// panel kind and the panel's `Facts` (picked row, selection) are private, so the panel's own scope delivers it.
pub(crate) fn run_fired(_app: &mut PhotocraftApp, _ctx: &egui::Context, _command: &CommandRef) -> bool {
    false
}

/// The styles panel of one kind, with the Style Options editor of the open style under it.
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui, paragraph: bool) {
    let prefix = if paragraph { "type.paragraphStyle" } else { "type.characterStyle" };
    let kind_index = usize::from(paragraph);
    let tgt = super::target(app);
    let list = if app.session.active().is_some() { app.session.execute(&format!("{prefix}.list"), tgt.clone()).ok() } else { None };
    let styles: Vec<Value> = list.as_ref().and_then(|l| l["styles"].as_array().cloned()).unwrap_or_default();
    let cur = list.as_ref().map(|l| l["current"].clone()).unwrap_or(Value::Null);
    let (cur_id, overridden) = if paragraph {
        (cur["paragraph"].as_u64(), cur["paragraphOverride"].as_bool().unwrap_or(false))
    } else {
        (cur["character"].as_u64(), cur["characterOverride"].as_bool().unwrap_or(false))
    };
    let has_target = !cur.is_null();
    let text_selected = !tgt.get("range").and_then(Value::as_array).is_some_and(|r| r[0] == r[1]);
    let rows: Vec<TextStyleRow> = styles.iter().map(|s| TextStyleRow::new(s["id"].as_u64().unwrap_or(0), s["name"].as_str().unwrap_or(""))).collect();
    let kind = if paragraph { TextStyleKind::Paragraph } else { TextStyleKind::Character };
    let label = if paragraph { tl!("Paragraph Styles") } else { tl!("Character Styles") };

    let mut selection = app.ui.type_panels.style_selection[kind_index].clone();
    let output = TextStylesPanel::new(label, kind, &rows, Bind::bound(&mut selection))
        .current(cur_id, overridden)
        .target(has_target)
        .text_selected(text_selected)
        .empty(tl!("No document"))
        .show(ui);
    let highlighted: Vec<u64> = rows.iter().map(|r| r.id).filter(|id| selection.contains(id)).collect();
    app.ui.type_panels.style_selection[kind_index] = selection;

    if let Some(message) = &output.status {
        app.ui.status = message.text.as_str().to_owned();
        app.ui.status_error = matches!(message.tone, craft::theme::variant::Tone::Bad);
    }
    match output.action {
        Some(TextStyleAction::Apply { id, clear_overrides, .. }) => {
            let mut p = tgt.clone();
            p["id"] = json!(id);
            p["clearOverrides"] = json!(clear_overrides);
            super::run(app, &format!("{prefix}.apply"), p);
        }
        Some(TextStyleAction::ClearOverride) => {
            super::run(app, &format!("{prefix}.clearOverride"), tgt.clone());
        }
        Some(TextStyleAction::Redefine(id)) => {
            let mut p = tgt.clone();
            p["id"] = json!(id);
            super::run(app, &format!("{prefix}.redefine"), p);
        }
        Some(TextStyleAction::New { from_selection, .. }) => {
            let mut p = tgt.clone();
            p["fromSelection"] = json!(from_selection);
            if let Some(r) = super::run(app, &format!("{prefix}.new"), p)
                && let Some(nid) = r["id"].as_u64()
            {
                app.ui.type_panels.options = Some((paragraph, nid as u32));
            }
        }
        Some(TextStyleAction::Duplicate(id)) => {
            super::run(app, &format!("{prefix}.duplicate"), json!({ "id": id }));
        }
        Some(TextStyleAction::Delete(id)) => {
            super::run(app, &format!("{prefix}.delete"), json!({ "id": id }));
            app.ui.type_panels.options = None;
        }
        Some(TextStyleAction::Options(id)) => app.ui.type_panels.options = Some((paragraph, id as u32)),
        Some(TextStyleAction::DuplicateHighlighted) => {
            // "No character style" (id 0) is never duplicated; the base paragraph style is.
            for id in highlighted.iter().filter(|id| **id != 0 || paragraph) {
                super::run(app, &format!("{prefix}.duplicate"), json!({ "id": id }));
            }
        }
        Some(TextStyleAction::DeleteHighlighted) => {
            for id in highlighted.iter().filter(|id| **id != 0) {
                super::run(app, &format!("{prefix}.delete"), json!({ "id": id }));
            }
            app.ui.type_panels.options = None;
        }
        // PLAN: SelectUnused needs an engine query of the unused styles (`{prefix}.unused` returning their ids,
        // for `style_selection`); `type_styles_cmds.rs` has none yet.
        _ => {}
    }

    if let Some((para, id)) = app.ui.type_panels.options
        && para == paragraph
    {
        if let Some(s) = styles.iter().find(|s| s["id"].as_u64() == Some(u64::from(id))) {
            super::style_options::show(app, ui, paragraph, s);
        } else {
            app.ui.type_panels.options = None;
        }
    }
}
