//! Toolbar tools in Edit › Keyboard Shortcuts (#4086). Explicit assignments select an
//! individual tool; unchanged tools keep Photoshop's shared keys and Shift cycling.

use craft::keymap::{Binding, CommandDef, CommandKind, Scope};
use craft::ui::runtime::commands::CommandRef;

use crate::{PhotocraftApp, state::Tool};

/// The id of the command that selects `tool`.
pub(crate) fn id(tool: Tool) -> String {
    format!("tools.select.{}", tool.id())
}

pub(crate) fn tool(id: &str) -> Option<Tool> {
    let name = id.strip_prefix("tools.select.")?;
    Tool::from_id(name)
}

pub(crate) fn default(tool: Tool) -> Option<String> {
    (tool.key() != '\0').then(|| tool.key().to_string())
}

#[cfg(test)]
pub(crate) fn shortcut(app: &PhotocraftApp, tool: Tool) -> Option<String> {
    crate::shortcuts::effective_shortcut(app, &id(tool), default(tool).as_deref())
}

/// Whether the user left `tool`'s key alone: a tool the user gave its own key leaves its letter's group.
pub(crate) fn uses_default(app: &PhotocraftApp, tool: Tool) -> bool {
    !app.session.prefs().shortcuts.contains_key(&id(tool))
}

/// The letter that picks `tool` through the tool keys (group memory, Shift cycling, spring-loading): its
/// default while unchanged, or a single letter the user gave it and another tool as well, either assigned or
/// as the default of a tool of its group (#14208). A key only `tool` has stays an ordinary shortcut that
/// selects just that tool (#4086).
pub(crate) fn letter(app: &PhotocraftApp, tool: Tool) -> Option<char> {
    if uses_default(app, tool) {
        return (tool.key() != '\0').then_some(tool.key());
    }
    let own = user_letter(app, tool)?;
    let shares = |t: Tool| user_letter(app, t) == Some(own) || (uses_default(app, t) && t.key() == own && tool.key() == own);
    Tool::ALL.into_iter().any(|t| t != tool && shares(t)).then_some(own)
}

/// A single letter the user assigned to `tool`, upper case.
fn user_letter(app: &PhotocraftApp, tool: Tool) -> Option<char> {
    let mut chars = app.session.prefs().shortcuts.get(&id(tool))?.chars();
    let c = chars.next().filter(char::is_ascii_alphabetic)?;
    chars.next().is_none().then(|| c.to_ascii_uppercase())
}

/// The first tool, in toolbar order, that the tool keys give `letter` ([`letter`]): the one its Shift+letter row names.
pub(crate) fn first_with_letter(app: &PhotocraftApp, letter: char) -> Option<Tool> {
    Tool::ALL.into_iter().filter(|t| self::letter(app, *t) == Some(letter)).min_by_key(|t| crate::panels::toolbar::toolbar_position(*t))
}

/// The letters users gave to several tools that are no tool's default, each with its first tool in toolbar
/// order: each needs its own Shift+letter cycle row (#14208).
pub(crate) fn shared_letters(app: &PhotocraftApp) -> Vec<(char, Tool)> {
    let defaults = group_letters();
    let mut groups: Vec<(char, Tool)> = Vec::new();
    for t in Tool::ALL {
        let Some(letter) = letter(app, t).filter(|l| !defaults.iter().any(|(d, _)| d == l)) else { continue };
        match groups.iter_mut().find(|(l, _)| *l == letter) {
            Some(group) if crate::panels::toolbar::toolbar_position(t) < crate::panels::toolbar::toolbar_position(group.1) => group.1 = t,
            Some(_) => {}
            None => groups.push((letter, t)),
        }
    }
    groups
}

/// One catalog row per tool: kind `Tool` in the group of its letter, so the tools that share a letter cycle, with the
/// letter as the default key.
// The runtime's own definition of each tool (`panels::tool_commands::ToolCommand`) carries the same letter and group, which is what binds the key
// and shows in tooltips; these rows keep the Keyboard Shortcuts dialog's tool pages.
pub(crate) fn defs(_app: &PhotocraftApp) -> Vec<CommandDef> {
    Tool::ALL
        .into_iter()
        .map(|t| {
            let letter = default(t);
            let mut def = CommandDef::new(CommandRef::new(id(t)), t.label(), Scope::app())
                .with_kind(CommandKind::Tool { group: letter.clone() })
                .with_path(vec!["Tools".into(), t.label().into()]);
            def.defaults.extend(letter.and_then(|l| Binding::parse(&l).ok()));
            def
        })
        .collect()
}

/// Each tool letter with the first tool of its group in toolbar order, in the order the letters first appear.
pub(crate) fn group_letters() -> Vec<(char, Tool)> {
    let mut groups: Vec<(char, Tool)> = Vec::new();
    for t in Tool::ALL {
        let letter = t.key();
        if letter == '\0' {
            continue;
        }
        match groups.iter_mut().find(|(l, _)| *l == letter) {
            Some(group) if crate::panels::toolbar::toolbar_position(t) < crate::panels::toolbar::toolbar_position(group.1) => group.1 = t,
            Some(_) => {}
            None => groups.push((letter, t)),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_toolbar_tool_has_a_unique_live_shortcut_row() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = app.ctx().clone();
        let rows = crate::shortcut_dispatch::command_rows(&app);
        let mut ids = std::collections::HashSet::new();
        for t in Tool::ALL {
            let id = id(t);
            assert!(ids.insert(id.clone()));
            assert_eq!(tool(&id), Some(t));
            assert!(crate::menus::is_live(&id));
            assert!(crate::menus::is_enabled(&app, &id));
            let matching: Vec<_> = rows.iter().filter(|r| r.command.id == id).collect();
            assert_eq!(matching.len(), 1);
            assert_eq!(matching[0].label, t.label());
            assert_eq!(matching[0].paths, [vec!["Tools".to_string(), t.label().to_string()]]);
            assert_eq!(matching[0].kind, CommandKind::Tool { group: default(t) });
            let bound: Vec<Binding> = default(t).and_then(|l| Binding::parse(&l).ok()).into_iter().collect();
            assert_eq!(matching[0].defaults, bound);
            assert_eq!(crate::shortcuts::default_shortcut(&id), default(t));
            crate::menus::invoke(&mut app, &ctx, &id, json!({})).unwrap();
            assert_eq!(app.ui.tool, t);
        }
        for id in ["tools.select.", "tools.select.missing", "tools.select.Brush"] {
            assert!(!crate::menus::is_live(id));
            assert!(crate::menus::invoke(&mut app, &ctx, id, json!({})).is_err());
        }
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

#[cfg(test)]
mod id_tests {
    use super::*;

    #[test]
    fn selection_ids_preserve_debug_names_and_resolve_exactly() {
        for tool_value in Tool::ALL {
            let mut old_name = format!("{tool_value:?}");
            old_name.get_mut(..1).unwrap().make_ascii_lowercase();
            let old_id = format!("tools.select.{old_name}");
            assert_eq!(id(tool_value), old_id);
            assert_eq!(tool(&old_id), Tool::from_name(&old_name).filter(|found| *found == tool_value));
        }
        assert_eq!(tool("tools.select.Brush"), None);
        assert_eq!(tool("tools.select.notATool"), None);
    }
}
