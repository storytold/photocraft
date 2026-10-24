//! Toolbar tools in Edit › Keyboard Shortcuts (#4086). Tools sharing a key keep the current
//! member and cycle with Shift, including custom assignments (#8056).

use crate::{PhotocraftApp, state::Tool};

pub(crate) fn id(tool: Tool) -> String {
    let mut name = format!("{tool:?}");
    if let Some(first) = name.get_mut(..1) {
        first.make_ascii_lowercase();
    }
    format!("tools.select.{name}")
}

pub(crate) fn tool(id: &str) -> Option<Tool> {
    let name = id.strip_prefix("tools.select.")?;
    let tool = Tool::from_name(name)?;
    (id == self::id(tool)).then_some(tool)
}

pub(crate) fn default(tool: Tool) -> Option<String> {
    (tool.key() != '\0').then(|| tool.key().to_string())
}

pub(crate) fn shortcut(app: &PhotocraftApp, tool: Tool) -> Option<String> {
    crate::shortcuts::effective_shortcut(app, &id(tool), default(tool).as_deref())
}

pub(crate) fn uses_default(app: &PhotocraftApp, tool: Tool) -> bool {
    !app.session.prefs().shortcuts.contains_key(&id(tool))
}

/// Resolve a dispatched tool binding against its live group. The general shortcut table has
/// one owner per key, but tools may share it; choosing that first owner on every press would
/// discard the active group member. Rebuild from preferences so removed keys cannot stick.
pub(crate) fn for_key(app: &PhotocraftApp, first: Tool, shortcut: egui::KeyboardShortcut) -> Tool {
    let group_for = |shortcut| {
        Tool::ALL
            .into_iter()
            .filter(|t| {
                let id = id(*t);
                let default = default(*t);
                let prefs = app.session.prefs();
                let main = prefs.shortcut(&id, default.as_deref());
                main.and_then(crate::shortcuts::parse) == Some(shortcut)
                    || prefs.alternates(&id, main).iter().any(|key| crate::shortcuts::parse(key) == Some(shortcut))
            })
            .collect::<Vec<_>>()
    };
    let shift_switch = app.session.prefs().tools.use_shift_key_for_tool_switch;
    let mut group = group_for(shortcut);
    let cycle = if group.is_empty() && shift_switch && shortcut.modifiers.shift {
        let mut plain = shortcut;
        plain.modifiers.shift = false;
        group = group_for(plain);
        true
    } else {
        !shift_switch
    };
    match group.iter().position(|t| *t == app.ui.tool) {
        Some(i) if cycle => group.get(i + 1).or_else(|| group.first()).copied().unwrap_or(first),
        Some(_) => app.ui.tool,
        None => app.ui.group_tool(first.key()).filter(|t| group.contains(t)).or_else(|| group.first().copied()).unwrap_or(first),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_toolbar_tool_has_a_unique_live_shortcut_row() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        let items = crate::prefs_ui::shortcut_items(&app);
        let mut ids = std::collections::HashSet::new();
        for t in Tool::ALL {
            let id = id(t);
            assert!(ids.insert(id.clone()));
            assert_eq!(tool(&id), Some(t));
            assert!(crate::menus::is_live(&id));
            assert!(crate::menus::is_enabled(&app, &id));
            let rows: Vec<_> = items.iter().filter(|i| i.0 == id).collect();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].1, t.label());
            assert_eq!(rows[0].2, ["Tools"]);
            assert_eq!(rows[0].3, default(t));
            assert_eq!(crate::shortcuts::default_shortcut(&id), default(t));
            crate::menus::invoke(&mut app, &ctx, &id, json!({})).unwrap();
            assert_eq!(app.ui.tool, t);
        }
        for id in ["tools.select.", "tools.select.missing", "tools.select.Brush"] {
            assert!(!crate::menus::is_live(id));
            assert!(crate::menus::invoke(&mut app, &ctx, id, json!({})).is_err());
        }
    }

    #[test]
    fn toolbar_hints_follow_assignments_removal_and_reset() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        assert_eq!(shortcut(&app, Tool::Pencil).as_deref(), Some("B"));
        assert_eq!(shortcut(&app, Tool::SingleRowMarquee), None);
        app.run("prefs.set", json!({"path": "shortcuts.tools.select.pencil", "value": "F6"})).unwrap();
        assert_eq!(shortcut(&app, Tool::Pencil).as_deref(), Some("F6"));
        app.run("prefs.set", json!({"path": "shortcuts.tools.select.pencil", "value": ""})).unwrap();
        assert_eq!(shortcut(&app, Tool::Pencil), None);
        app.run("prefs.reset", json!({"path": "shortcuts.tools.select.pencil"})).unwrap();
        assert_eq!(shortcut(&app, Tool::Pencil).as_deref(), Some("B"));
    }

    #[test]
    fn a_remapped_tool_letter_does_not_steal_text_input() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("prefs.set", json!({"path": "shortcuts.tools.select.pencil", "value": "Q"})).unwrap();
        let mut h = egui_kittest::Harness::builder().build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                ui.text_edit_singleline(&mut app.ui.status).request_focus();
                crate::shortcuts::handle(app, ui.ctx());
            },
            app,
        );
        h.run_steps(3);
        assert!(h.ctx.text_edit_focused());
        h.key_press(egui::Key::Q);
        h.event(egui::Event::Text("q".into()));
        h.run_steps(2);
        assert_eq!(h.state().ui.tool, Tool::Brush);
        assert_eq!(h.state().ui.status, "q");
    }
}

#[cfg(test)]
mod engine_table_tests {
    use super::*;

    /// The engine knows the tools as data (headless shortcut sets); it must match the toolbar.
    #[test]
    fn the_engines_tool_table_matches_the_tools() {
        let ours: Vec<(String, &str, Option<String>)> = Tool::ALL.into_iter().map(|t| (id(t), t.label(), default(t))).collect();
        let engine: Vec<(String, &str, Option<String>)> =
            photocraft_engine::shell_commands::TOOLS.iter().map(|(i, n, k)| (i.to_string(), *n, k.map(str::to_string))).collect();
        assert_eq!(ours, engine, "update photocraft_engine::shell_commands::TOOLS");
    }
}
