//! The host app's own menu items and its first say over commands ([`crate::Services::host_menu`]).

use serde_json::Value;

use crate::PhotocraftApp;
use crate::menus::MenuItem;

/// Inserts the host's items, each right after the item it names, in that item's submenu.
pub(super) fn insert_menu_items(app: &PhotocraftApp, items: &mut Vec<MenuItem>) {
    let Some(host) = app.services.host_menu.as_ref() else { return };
    for item in &host.items {
        let Some(at) = items.iter().position(|i| i.id == item.after) else { continue };
        let path = items.get(at).map(|i| i.path.clone()).unwrap_or_default();
        let row = MenuItem {
            id: item.id.clone(),
            label: item.label.clone(),
            path,
            shortcut: None,
            enabled: (host.enabled)(app, &item.id),
            checked: None,
            color: None,
        };
        items.insert(at + 1, row);
    }
}

/// Whether a host item can run (`None` for other ids).
pub(super) fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let host = app.services.host_menu.as_ref()?;
    host.items.iter().any(|i| i.id == id).then(|| (host.enabled)(app, id))
}

/// Lets the host run `id` first: `Some` when it did. While it runs, commands it invokes are
/// PhotoCraft's own.
pub(super) fn run(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let mut host = app.services.host_menu.take()?;
    let ran = (host.run)(app, ctx, id, params);
    app.services.host_menu = Some(host);
    ran
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde_json::json;

    use crate::{HostMenu, HostMenuItem, Services};

    use super::*;

    /// A host's item sits after the one it names, in its submenu, enabled as the host says; the
    /// host runs it, may take over any other command, and its own nested runs are PhotoCraft's.
    #[test]
    fn host_items_join_the_menus_and_the_host_runs_commands_first() {
        let seen = Rc::new(RefCell::new(Vec::<String>::new()));
        let log = Rc::clone(&seen);
        let host = HostMenu {
            items: vec![HostMenuItem { id: "host.share".into(), label: "Share…".into(), after: "file.saveACopy".into() }],
            enabled: Box::new(|app, _| app.session.active().is_some()),
            run: Box::new(move |app, ctx, id, _| {
                log.borrow_mut().push(id.to_string());
                match id {
                    "host.share" => Some(Ok(json!({"shared": true}))),
                    // Taken over, and running PhotoCraft's own command meanwhile.
                    "edit.paste" => Some(crate::menus::invoke(app, ctx, "file.new", json!({"width": 8, "height": 8}))),
                    _ => None,
                }
            }),
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services { host_menu: Some(host), ..Default::default() });
        let ctx = egui::Context::default();
        let items = crate::menus::menu_items(&app);
        let at = items.iter().position(|i| i.id == "file.saveACopy").expect("Save a Copy");
        let share = items.get(at + 1).expect("an item after Save a Copy");
        assert_eq!((share.id.as_str(), share.label.as_str(), share.path.as_slice()), ("host.share", "Share…", ["File".to_string()].as_slice()));
        assert!(!share.enabled, "no document");
        assert!(!crate::menus::is_enabled(&app, "host.share"));

        assert!(crate::menus::invoke(&mut app, &ctx, "edit.paste", json!({})).is_ok());
        assert_eq!(app.session.documents().len(), 1, "the host's nested run made a document");
        assert!(crate::menus::is_enabled(&app, "host.share"));
        assert!(crate::menus::menu_items(&app).iter().any(|i| i.id == "host.share" && i.enabled));
        assert_eq!(crate::menus::invoke(&mut app, &ctx, "host.share", json!({})), Ok(json!({"shared": true})));
        assert!(crate::menus::invoke(&mut app, &ctx, "select.all", json!({})).is_ok(), "not taken over: PhotoCraft runs it");
        assert_eq!(*seen.borrow(), ["edit.paste", "host.share", "select.all"], "file.new ran inside the host's run, as PhotoCraft's");
        assert!(app.services.host_menu.is_some(), "the host is back after each run");
    }
}
