//! Linux global menu: exports the in-window menu bar over the Canonical AppMenu (dbusmenu)
//! protocol, so global-menu environments host PhotoCraft's menus natively. One exporter per
//! app, started lazily on the first frame; a 1 Hz publish cadence keeps the global menu in
//! step with document state (checked states, enablement, recent files) nearly as fast as
//! opening the menu. Activations come back as
//! [`craft_appmenu::MenuEvent`]s and dispatch exactly like an in-window menu click
//! ([`crate::menus::invoke`]), so an item behaves identically in both hosts.
//!
//! Nothing runs outside Linux/X11/BSD with a D-Bus session bus:
//! [`craft_appmenu::AppMenu::start`] fails gracefully there and every call below
//! becomes a no-op (the web build has no menus at all). `PHOTOCRAFT_NO_GLOBAL_MENU=1`
//! skips the export entirely (debugging).

use crate::menus::{MenuItem, TOP_MENUS};
use craft_appmenu::MenuModel;
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
use serde_json::json;
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
use std::time::{Duration, Instant};

/// Publish cadence: the export reflects document state, so it is refreshed like a background
/// poller — cheap next to opening a real menu, since the exporter diffs models on its own
/// thread and only emits what changed. Only used by [`tick`], which only exists where the
/// exporter lives (its duty cycle would be wasted work with no exporter, and the `Instant`
/// clock would panic on wasm).
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
const REFRESH: Duration = Duration::from_secs(1);

/// The app's name on the bus (machine names are lowercase; AGENTS.md).
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
const APP_NAME: &str = "photocraft";

/// Starts (once), publishes (on a duty cycle) and drains menu clicks. Called from the frame
/// loop, right after the control channel is drained. Exists only where the exporter can:
/// Linux/BSD with a D-Bus session (`craft-appmenu`'s live side — macOS and Windows get its
/// stub and always fail to start). Not alone a size matter: `Instant::now()` panics on
/// wasm32-unknown-unknown, so the no-op variant below is what the web build runs (a frame
/// loop tick must never panic; review note).
#[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
pub fn tick(app: &mut crate::PhotocraftApp, ctx: &egui::Context) {
    // The flag follows the registrar's registration state (cheap atomic) while an exporter
    // runs: while a host serves the menus, the in-window menu bar is hidden; it comes back at
    // the same second the host (or its service) disappears. With no exporter the field is
    // never touched (it is only ever flipped true here), so the in-window bar stays.
    if let Some(menu) = &app.global_menu {
        app.global_menu_hosted = menu.hosted();
    }
    if app.global_menu.is_none() && !app.global_menu_unavailable {
        if std::env::var_os("PHOTOCRAFT_NO_GLOBAL_MENU").is_some() {
            app.global_menu_unavailable = true;
            log::debug!("AppMenu: disabled by PHOTOCRAFT_NO_GLOBAL_MENU");
        } else {
            match craft_appmenu::AppMenu::start(APP_NAME) {
                Ok(menu) => {
                    log::debug!("AppMenu: global menu exporting");
                    app.global_menu = Some(menu);
                }
                Err(e) => {
                    app.global_menu_unavailable = true;
                    log::debug!("AppMenu: global menu stays off ({e})");
                }
            }
        }
    }
    // Publish at most once per cycle; the very first publish happens right away.
    if app.global_menu_last.is_none_or(|last| last.elapsed() >= REFRESH) {
        let items = crate::menus::menu_items(app);
        if let Some(menu) = &app.global_menu {
            menu.replace(&model(&items));
        }
        app.global_menu_last = Some(Instant::now());
    }
    // The activations come in a batch (the exporter queues while the frame runs); collect one
    // before dispatching, because dispatch takes the app mutably.
    let events: Vec<craft_appmenu::MenuEvent> = match app.global_menu.as_ref() {
        Some(menu) => std::iter::from_fn(|| menu.try_event()).collect(),
        None => return,
    };
    for ev in events {
        let craft_appmenu::MenuEvent::Activated { action, .. } = ev;
        // The behaviour path is the one an in-window click takes, so a global-menu
        // activation is identical to the same item in the in-window menu bar.
        if let Err(e) = crate::menus::invoke(app, ctx, &action, json!({})) {
            log::debug!("AppMenu: activating {action} failed: {e}");
        }
    }
}

/// No exporter can exist on this platform: the in-window menu bar owns the menus and
/// there is nothing to start, publish or drain. Deliberately not touching any field here,
/// so the app's global-menu state stays untouched (and its `Instant`-based duty cycle
/// never runs on a target whose clock would panic).
#[cfg(not(all(unix, not(target_os = "macos"), not(target_arch = "wasm32"))))]
pub fn tick(_app: &mut crate::PhotocraftApp, _ctx: &egui::Context) {}

/// Builds the global-menu model from the same item list the in-window menu bar renders, so
/// both menus present the same structure, order, labels (translated), shortcuts, enablement
/// and check marks. The structural rules (top grouping, skip-empty, separator collapsing,
/// submenu nesting) live in the shared `craft-appmenu` crate (`MenuModel::from_flat`, tested
/// there); this only maps PhotoCraft's items onto them, translating on the way.
pub fn model(items: &[MenuItem]) -> MenuModel {
    let lang = crate::i18n::current();
    // Iterate the known top menus in bar order; the flat builder restores the same order
    // from first-seen, and a top outside the catalog (or with no items) simply never shows.
    let flat: Vec<craft_appmenu::FlatItem> = TOP_MENUS
        .iter()
        .flat_map(|&top| items.iter().filter(move |i| i.path.first().is_some_and(|p| p == top)))
        .map(|it| {
            let translated: Vec<String> =
                it.path.iter().map(|s| crate::i18n::tr(lang, s).to_string()).collect();
            if it.label == "---" {
                return craft_appmenu::FlatItem {
                    label: "---".to_string(),
                    path: translated,
                    action: String::new(),
                    shortcut: None,
                    enabled: true,
                    checked: None,
                };
            }
            craft_appmenu::FlatItem {
                label: crate::i18n::tr_id(lang, &it.id, &it.label).to_string(),
                path: translated,
                action: it.id.clone(),
                shortcut: it.shortcut.clone(),
                enabled: it.enabled,
                checked: it.checked,
            }
        })
        .collect();
    MenuModel::from_flat(&flat)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, label: &str, path: &[&str]) -> MenuItem {
        MenuItem {
            id: id.into(),
            label: label.into(),
            path: path.iter().map(|s| s.to_string()).collect(),
            shortcut: None,
            enabled: true,
            checked: None,
            color: None,
        }
    }

    fn tree(entries: &[craft_appmenu::MenuEntry], depth: usize) -> String {
        use craft_appmenu::MenuEntry;
        let mut out = String::new();
        for entry in entries {
            match entry {
                MenuEntry::Separator => {                    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{}|\n", "  ".repeat(depth)));
                }
                MenuEntry::Command(c) => {
                    let _ = std::fmt::Write::write_fmt(
                        &mut out,
                        format_args!(
                            "{}{}({}){}\n",
                            "  ".repeat(depth),
                            c.label,
                            c.action,
                            c.checked.map(|c| if c { "[✓]" } else { "[ ]]" }).unwrap_or_default(),
                        ),
                    );
                    if let Some(children) = &c.children {
                        out.push_str(&tree(children, depth + 1));
                    }
                }
            }
        }
        out
    }

    fn flat_items() -> Vec<MenuItem> {
        vec![
            item("file.new", "New…", &["File"]),
            item("", "---", &["File"]),
            item("file.save", "Save", &["File"]),
            item("edit.undo", "Undo", &["Edit"]),
            item("view.zoomIn", "Zoom In", &["View"]),
            item("view.grid", "Show Grid", &["View", "Extras"]),
            item("view.rulers", "Show Rulers", &["View", "Extras"]),
        ]
    }

    #[test]
    fn a_hosted_global_menu_hides_the_in_window_titles() {
        use crate::PhotocraftApp;
        use egui::vec2;
        use egui_kittest::{Harness, kittest::Queryable};

        for (hosted, expect_titles) in [(false, true), (true, false)] {
            // Fonts land on the context inside `build_eframe`, before the app's first frame.
            let mut harness = Harness::builder()
                .with_size(vec2(1200.0, 800.0))
                .with_max_steps(16)
                .build_eframe(move |cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ALL[0]);
                    let mut app =
                        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                    // Opt out of really starting an exporter (and the field is then never
                    // touched): the hosted state is what the test drives.
                    app.global_menu_unavailable = true;
                    app.global_menu_hosted = hosted;
                    app
                });
            harness.run_steps(3);
            // "File" appears in the app only as a menu title, in-window or global-hosted.
            let has_titles = harness.query_by_label_contains("File").is_some();
            assert_eq!(has_titles, expect_titles, "hosted={hosted}");
        }
    }

    #[test]
    fn the_flat_item_list_becomes_the_same_tree_the_menu_bar_renders() {
        let items = flat_items();
        let printed = tree(&model(&items).children, 0);
        let expected = "File()\n  New…(file.new)\n  |\n  Save(file.save)\n\
Edit()\n  Undo(edit.undo)\nView()\n  Zoom In(view.zoomIn)\n  Extras()\n    Show Grid(view.grid)\n    Show Rulers(view.rulers)\n";
        assert_eq!(printed, expected, "{printed}");
    }

    #[test]
    fn adjacent_separators_collapse_and_empty_tops_are_skipped() {
        let items = vec![
            item("file.new", "New…", &["File"]),
            item("", "---", &["File"]),
            item("", "---", &["File"]),
            item("", "---", &["Edit"]), // nothing before it: suppressed
            item("edit.undo", "Undo", &["Edit"]),
            // No tops but File/Edit exist: the other eight top menus stay away.
        ];
        let printed = tree(&model(&items).children, 0);
        // Two adjacent "---" become one separator (doubled collapse), and a separator with
        // nothing printed before it within the level is suppressed.
        assert_eq!(printed, "File()\n  New…(file.new)\n  |\nEdit()\n  Undo(edit.undo)\n", "{printed}");
    }
}
