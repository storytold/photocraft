//! Menu model generated from the engine command registry plus UI-level commands.

use std::sync::Arc;

use craft::keymap::CommandRef;
use craft::menu::{Gate, ModalRules, PendingRule};
use craft::ui::contract::MemoProps;
use craft::ui::runtime::commands::CommandGate;
use serde_json::{Value, json};

use crate::PhotocraftApp;

mod bar;
pub mod enable_rules;
pub(crate) mod facts;
mod invoke;
mod items;
pub mod native_menu;
pub mod parity;
pub mod plugin_ui;
mod state;
pub mod view_cmds;
pub mod workspace_ui;

pub(crate) use bar::help_entries;
pub use bar::menu_bar;
pub use invoke::invoke;
pub(crate) use invoke::{invoke_unguarded, panel_alias};
pub use items::{MenuModel, base_rows, is_live, menu_rows, model, row};
#[cfg(test)]
pub(crate) use items::{fresh_bar, model_id};
pub use state::is_enabled;
pub(crate) use state::reveal_label;
#[cfg(test)]
use state::translated_menu_label;

/// What may run from the menus now: the gate (a dialog, the shortcut recorder) and the per-command
/// rules (a full-window workspace, a pending crop). Both bars, the runtime's command gate and
/// [`allows`] read this one function.
pub(crate) fn menu_gate(app: &PhotocraftApp) -> (Gate, ModalRules) {
    let gate = Gate::default().modal(app.dialog_open() || crate::prefs_ui::shortcuts_open(app)).recording_shortcut(crate::prefs_ui::recording(app));
    let workspace = app.smart_svg.modal_open() || crate::distort_ui::modal_workspace(app).is_some();
    let pending = crate::crop_ui::pending(app);
    let rule = pending.then(|| Arc::new(|c: &CommandRef| crate::crop_ui::available_while_pending(&c.id)) as PendingRule);
    // The revision is the exact value of the rules' variable parts; a new variable part joins it.
    let revision = u64::from(workspace) | u64::from(pending) << 1;
    let rules = ModalRules::default().in_dialogs(crate::shortcuts::NAV_COMMANDS.iter().copied()).workspace(workspace).pending(rule).with_revision(revision);
    (gate, rules)
}

/// The runtime's gate over commands. Only PhotoCraft's catalogue commands are gated here: framework
/// command sets and dialog-scoped commands pass, and the runtime's own modal rule decides them. The shell's
/// screen-mode commands are judged as their View › Screen Mode rows, and the rows' own rules
/// ([`refused`]) apply to them: their keys and menu items are one command.
pub(crate) fn command_gate(app: &PhotocraftApp) -> CommandGate {
    let (gate, rules) = menu_gate(app);
    // The refusals follow the rows' rules, so they are derived from the facts the menu model is built from.
    let refused = craft::ui::runtime::derived::derived(app.ctx(), egui::Id::unique("photocraft-refused-commands"), &facts::MenuFacts::of(app), || refused(app));
    CommandGate::new(GateProps { gate, rules, refused }, |props, command| gated(&props.gate, &props.rules, &props.refused, command))
}

/// What the runtime's gate reads: the runtime asks it again only when these change.
#[derive(MemoProps)]
struct GateProps {
    gate: Gate,
    rules: ModalRules,
    refused: Arc<Vec<CommandRef>>,
}

/// [`command_gate`]'s test for one command, for callers that hold no gate.
pub(crate) fn allows(app: &PhotocraftApp, command: &CommandRef) -> bool {
    let (gate, rules) = menu_gate(app);
    gated(&gate, &rules, &refused(app), command)
}

fn gated(gate: &Gate, rules: &ModalRules, refused: &[CommandRef], command: &CommandRef) -> bool {
    if refused.contains(command) {
        return false;
    }
    let row = crate::shortcut_dispatch::screen_mode_row(command).map(CommandRef::new);
    let command = row.as_ref().unwrap_or(command);
    !is_live(&command.id) || gate.allows_under(rules, command)
}

/// The framework commands PhotoCraft's state turns off now: the shell's screen modes its View › Screen Mode rows
/// grey (no full screen without a document, #9349; none while type is edited), and the canvas colour keys of the
/// held Hand while type is edited.
fn refused(app: &PhotocraftApp) -> Vec<CommandRef> {
    let screen_modes = crate::shortcut_dispatch::RUNTIME_COMMANDS
        .iter()
        .filter(|(id, _)| crate::shortcut_dispatch::is_screen_mode(id) && crate::view_cmds::is_enabled(app, id) == Some(false))
        .map(|(_, bridged)| bridged.command());
    let colors = crate::canvas_tool_menu::CanvasColorCommands::ALL.into_iter().filter(|_| app.ui.text_edit.is_some()).map(CommandRef::from);
    screen_modes.chain(colors).collect()
}

/// Top-level menus in Photoshop order.
pub const TOP_MENUS: [&str; 10] = ["File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help"];

/// UI-level commands (handled by the shell rather than the engine): id, label, menu, shortcut.
/// The table is the engine's ([`photocraft_engine::shell_commands::SHELL_COMMANDS`]), so the
/// commands and their keys exist without this UI (headless shortcut sets, lists, conflicts).
pub const UI_COMMANDS: &[(&str, &str, &[&str], Option<&str>)] = photocraft_engine::shell_commands::SHELL_COMMANDS;

/// Window › <document>: the open documents, listed at the end of the Window menu as in Photoshop.
/// The id ends in the document's index, which is its tab's position.
pub(crate) const DOCUMENT_ITEM: &str = "window.document.";

#[cfg(test)]
mod tests;

#[cfg(test)]
mod appearance_menu_trials;

#[cfg(test)]
mod open_recent_position_tests;

#[cfg(test)]
mod export_submenu_separator_tests;

#[cfg(test)]
mod open_recent_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tracks_dedupes_caps_and_lists() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent("/tmp/a.png");
        app.push_recent("/tmp/b.psd");
        app.push_recent("/tmp/a.png"); // de-dupe → moves to front
        assert_eq!(*app.ui.recent_files, vec!["/tmp/a.png".to_string(), "/tmp/b.psd".to_string()]);
        for i in 0..15 {
            app.push_recent(&format!("/tmp/f{i}.png"));
        }
        assert_eq!(app.ui.recent_files.len(), 17, "all kept below the default cap of 20");
        for i in 15..30 {
            app.push_recent(&format!("/tmp/f{i}.png"));
        }
        assert_eq!(app.ui.recent_files.len(), 20, "capped at Recent File List Contains (20)");
        assert_eq!(app.session.prefs().file_handling.recent_files, *app.ui.recent_files, "stored in the preferences");

        // Menu lists them under File › Open Recent, with basenames, plus Clear Recent Files.
        *app.ui.recent_files = vec!["/tmp/a.png".into(), "/dir/b.psd".into()];
        let rows = menu_rows(&app);
        let labels: Vec<&str> = rows.iter().filter(|r| r.command.id.starts_with("file.openRecent.")).map(|r| r.label.as_str()).collect();
        assert_eq!(labels, vec!["a.png", "b.psd"]);
        assert!(rows.iter().any(|r| r.command.id == "file.clearRecent" && r.path == ["File", "Open Recent"]));

        // Clear Recent Files empties the list.
        let ctx = app.ctx().clone();
        invoke(&mut app, &ctx, "file.clearRecent", json!({})).unwrap();
        assert!(app.ui.recent_files.is_empty());
        assert!(app.session.prefs().file_handling.recent_files.is_empty(), "cleared in the preferences too");
        assert!(!menu_rows(&app).iter().any(|r| r.command.id.starts_with("file.openRecent.")));
        // A bad recent index errors gracefully (no panic).
        assert!(invoke(&mut app, &ctx, "file.openRecent.5", json!({})).is_err());

        // Remove one entry (the Home screen's ×, #2691): the others keep their order.
        for p in ["/tmp/c.tif", "/tmp/b.psd", "/tmp/a.png"] {
            app.push_recent(p);
        }
        invoke(&mut app, &ctx, "file.removeRecent", json!({"path": "/tmp/b.psd"})).unwrap();
        assert_eq!(*app.ui.recent_files, vec!["/tmp/a.png".to_string(), "/tmp/c.tif".to_string()]);
        assert_eq!(app.session.prefs().file_handling.recent_files, *app.ui.recent_files, "removed in the preferences too");
        invoke(&mut app, &ctx, "file.removeRecent", json!({"path": "/tmp/not-listed.png"})).unwrap();
        assert_eq!(app.ui.recent_files.len(), 2, "a path that isn't listed changes nothing");
        assert!(invoke(&mut app, &ctx, "file.removeRecent", json!({})).is_err(), "needs a path");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn opening_a_file_records_it_as_recent() {
        use photocraft_color::{ColorMode, SampleType};
        use photocraft_geom::Size;
        let services = crate::Services {
            import: Some(Box::new(|_n: &str, _b: &[u8], _opts: &photocraft_io::ImportOptions| {
                Ok((photocraft_doc::Document::new("t", Size::new(4, 4), ColorMode::Rgb, SampleType::U8), Vec::new()))
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let path = std::env::temp_dir().join("photocraft_recent_test.pcraft");
        std::fs::write(&path, b"x").unwrap();
        let p = path.to_string_lossy().to_string();
        let ctx = app.ctx().clone();
        invoke(&mut app, &ctx, "file.open", json!({ "path": p })).unwrap();
        assert_eq!(app.ui.recent_files.first().map(String::as_str), Some(p.as_str()));
        // Re-open via the recent entry once closed (an open file shows its tab instead, #6598).
        app.session.close(0);
        invoke(&mut app, &ctx, "file.openRecent.0", json!({})).unwrap();
        assert_eq!(app.session.documents().len(), 1);
        assert_eq!(app.session.active().and_then(|d| d.path.as_deref()), Some(p.as_str()));
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod quick_export_label_tests {
    use super::*;

    #[test]
    fn file_quick_export_menu_reflects_selected_format_without_renaming_layer_export() {
        if !crate::i18n::needs_catalogs() {
            return;
        }
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for (value, expected) in [("png", "PNG"), ("jpg", "JPG"), ("gif", "GIF"), ("webp", "WebP"), ("exr", "EXR")] {
            app.run("prefs.set", json!({"values": {"export.quickExportFormat": value}})).unwrap();
            let rows = menu_rows(&app);
            let quick: Vec<_> = rows.iter().filter(|r| r.command.id == "file.export.quickExportAsPng").collect();
            assert_eq!(quick.len(), 1, "the catalogue and UI command must not create duplicate items");
            let shown_format = if cfg!(target_arch = "wasm32") { "PNG" } else { expected };
            assert_eq!(quick[0].label, format!("Quick Export as {shown_format}"));
            let id = crate::i18n::Lang::from_code("id").unwrap();
            let localized = translated_menu_label(id, &quick[0].command.id, &quick[0].label);
            assert_eq!(localized, crate::i18n::tr_id(id, "file.export.quickExportAsPng", "Quick Export as PNG").replace("PNG", shown_format));
            let layer = row(&rows, "layer.quickExportAsPng").unwrap();
            assert_eq!(layer.label, "Quick Export as PNG", "layer export always creates PNG");
        }
    }
}

#[cfg(test)]
mod reveal_label_tests {
    use super::*;

    #[test]
    fn smart_objects_reveal_item_names_the_platform_file_manager() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let id = "layer.smartObjects.revealInFinder";
        let rows = menu_rows(&app);
        let reveal: Vec<_> = rows.iter().filter(|r| r.command.id == id).collect();
        assert_eq!(reveal.len(), 1);
        assert_eq!(reveal[0].path, ["Layer", "Smart Objects"]);
        assert_eq!(reveal[0].label, reveal_label());
        let catalog = crate::shortcut_dispatch::command_rows(&app);
        assert!(catalog.iter().any(|d| d.command.id == id && d.label == reveal_label()), "the shortcut list shows the menu label");
        // The menu catalogue (and docs/parity-checklist.md) keeps Photoshop's macOS name.
        assert!(crate::menu_catalog::CATALOG.iter().any(|(_, label, _, i)| *i == id && *label == "Reveal in Finder"));
    }
}

#[cfg(test)]
mod window_documents_tests {
    use super::*;
    use serde_json::json;

    fn app(docs: usize) -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for _ in 0..docs {
            app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        }
        app.sync_views();
        app
    }

    /// The rows after Window's last own item: a separator and the documents.
    fn listed(app: &PhotocraftApp) -> Vec<(String, String, bool)> {
        let rows = menu_rows(app);
        let window: Vec<_> = rows.iter().filter(|r| r.path == ["Window"]).collect();
        let at = window.iter().position(|r| r.command.id.starts_with(DOCUMENT_ITEM)).unwrap_or(window.len());
        if at > 0 && at < window.len() {
            assert_eq!(window[at - 1].label, "---", "a separator comes before the documents");
        }
        window[at..].iter().map(|r| (r.command.id.clone(), r.label.clone(), matches!(r.kind, craft::menu::ItemKind::Check(true)))).collect()
    }

    /// #2991: Window ends with the open documents in tab order, the front one checked, as in
    /// Photoshop; choosing one brings it to the front, through `invoke` and through the control
    /// channel's `ui.menu.invoke`.
    #[test]
    fn window_menu_lists_documents_and_switches() {
        for docs in [0usize, 1, 3] {
            let mut app = app(docs);
            let ctx = app.ctx().clone();
            let names: Vec<String> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
            let front = docs.checked_sub(1);
            let want: Vec<_> = (0..docs).map(|i| (format!("window.document.{i}"), names[i].clone(), Some(i) == front)).collect();
            assert_eq!(listed(&app), want, "{docs} documents");
            if docs < 2 {
                continue;
            }
            // It stays last in Window, after the panels.
            let rows = menu_rows(&app);
            let last = rows.iter().rposition(|r| r.path.first().map(String::as_str) == Some("Window")).unwrap();
            assert_eq!(rows[last].command.id, format!("window.document.{}", docs - 1));

            assert_eq!(invoke(&mut app, &ctx, "window.document.0", json!({})).unwrap(), json!({"document": 0}));
            assert_eq!(app.session.active_index(), Some(0));
            assert!(listed(&app)[0].2);
            invoke(&mut app, &ctx, &format!("window.document.{}", docs - 1), json!({})).unwrap();
            let (req, _rx) = crate::control::ControlRequest::new("ui.menu.invoke", json!({"id": "window.document.0"}));
            let crate::control::Reply::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("expected an immediate reply") };
            assert_eq!(r["ok"], true, "{r}");
            assert_eq!(app.session.active_index(), Some(0), "the control channel chooses like a click");

            // A stale or malformed id (a document closed since the menu was built) is an error.
            for id in ["window.document.3", "window.document.", "window.document.x", "window.document.-1"] {
                assert!(is_enabled(&app, id), "{id}: rows are always enabled");
                assert!(invoke(&mut app, &ctx, id, json!({})).is_err(), "{id}");
            }
            assert_eq!(app.session.active_index(), Some(0));
        }

        // A rename shows the new name, never a translation of it.
        if crate::i18n::needs_catalogs() {
            let mut app = app(1);
            let ctx = app.ctx().clone();
            app.session.saved_to("/tmp/Layers".into());
            let de = crate::i18n::Lang::from_code("de").unwrap();
            let _lang = crate::i18n::language_scope(de);
            let shown = model(&mut app, &ctx);
            assert_eq!(row(shown.rows(), "window.document.0").map(|r| r.label.as_str()), Some("Layers"));
        }
    }
}
