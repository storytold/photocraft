//! #16795: Window › Options shows and hides the options bar, its check mark follows the bar, and the
//! control channel drives it like the other Window toggles.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::control::{ControlRequest, Reply, handle};

/// An app with a document open, so the options bar has a tool to show.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app
}

/// The control channel's whole reply to `method` with `params`.
fn control(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let ctx = app.ctx().clone();
    let (request, _reply) = ControlRequest::new(method, params);
    let Reply::Done(reply) = handle(app, &ctx, &request) else { panic!("{method} should reply at once") };
    reply
}

/// Chooses menu item `id` through the control channel with the command's `params`, as a click on it does; its
/// result.
fn invoke(app: &mut PhotocraftApp, id: &str, params: Value) -> Value {
    let reply = control(app, "ui.menu.invoke", json!({"id": id, "params": params}));
    assert_eq!(reply["ok"], true, "{id}: {reply}");
    reply["result"].clone()
}

/// Whether the menu shows `id` checked, as `ui.menu.list` reports it.
fn checked(app: &mut PhotocraftApp, id: &str) -> Value {
    let list = control(app, "ui.menu.list", json!({}));
    let rows = list["result"].as_array().cloned().unwrap_or_default();
    rows.into_iter().find(|row| row["id"] == id).map_or(Value::Null, |row| row["checked"].clone())
}

/// Whether the options bar is on (Window › Options), as `ui.inspect` reports it.
fn bar_shown(app: &mut PhotocraftApp) -> Value {
    control(app, "ui.inspect", json!({}))["result"]["optionsBar"]["enabled"].clone()
}

#[test]
fn window_options_hides_and_shows_the_options_bar_with_its_check_mark() {
    let mut app = app();
    assert_eq!((checked(&mut app, "window.panel.options"), bar_shown(&mut app)), (json!(true), json!(true)), "checked while the bar shows");
    invoke(&mut app, "window.panel.options", json!({}));
    assert_eq!((checked(&mut app, "window.panel.options"), bar_shown(&mut app)), (json!(false), json!(false)), "choosing it hides the bar and unchecks it");
    assert!(!app.ui.status.contains("unknown command"), "no error in the status bar: {}", app.ui.status);
    invoke(&mut app, "window.panel.options", json!({}));
    assert_eq!((checked(&mut app, "window.panel.options"), bar_shown(&mut app)), (json!(true), json!(true)), "choosing it again brings the bar back");
}

#[test]
fn the_control_channel_shows_or_hides_the_options_bar_with_show() {
    let mut app = app();
    for (show, want) in [(false, false), (false, false), (true, true), (true, true)] {
        invoke(&mut app, "window.panel.options", json!({"show": show}));
        assert_eq!(bar_shown(&mut app), json!(want), "show: {show} sets the bar rather than toggling it");
    }
    let reply = control(&mut app, "ui.menu.invoke", json!({"id": "window.panel.options", "params": {"show": "no"}}));
    assert_eq!(reply["ok"], false, "a show that isn't true or false is refused: {reply}");
    assert_eq!(bar_shown(&mut app), json!(true), "and changes nothing");
}

/// Neighbours: the other Window bars beside Options keep toggling on their own.
#[test]
fn window_tools_and_contextual_task_bar_keep_their_own_toggles() {
    let mut app = app();
    let tools = checked(&mut app, "window.panel.tools");
    invoke(&mut app, "window.panel.tools", json!({}));
    assert_ne!(checked(&mut app, "window.panel.tools"), tools, "Window › Tools toggles the toolbar");
    invoke(&mut app, "window.panel.contextualTaskBar", json!({}));
    assert_eq!(checked(&mut app, "window.panel.contextualTaskBar"), json!(false), "Window › Contextual Task Bar toggles its bar");
    assert_eq!(bar_shown(&mut app), json!(true), "neither touches the options bar");
}
