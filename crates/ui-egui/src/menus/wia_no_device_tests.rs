//! #13938: File > Import > WIA Support… with no WIA device answers the menu item with the
//! "WIA Support" error alert instead of doing nothing. The active document stays unchanged, and
//! with no document open none is created. The cases the reference application was not measured
//! with — off Windows, and a result that reports devices — keep today's silent path.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::menus::invoke;

/// The alert's message: our own words for the reference's no-device notice.
const MESSAGE: &str = "No WIA-compatible device was found.";

/// An app without a document, or with a 60 x 40 white one.
fn app_with(document: bool) -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    if document {
        app.run("file.new", json!({"width": 60, "height": 40, "background": "white"})).unwrap();
    }
    app.sync_views();
    let ctx = app.ctx().clone();
    (app, ctx)
}

/// The messages of the plain alerts open on the host: the house `HostedAlert` keys as "alert".
fn alert_messages(app: &PhotocraftApp) -> Vec<String> {
    app.framework
        .dialogs
        .open_dialogs()
        .into_iter()
        .filter(|d| d.key.as_deref() == Some("alert"))
        .filter_map(|d| d.fields.as_ref()?.get("message").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// History states and layer count of the active document.
fn doc_state(app: &PhotocraftApp) -> (usize, usize) {
    let st = app.session.active().expect("a document");
    (st.history.past_len(), st.doc.layers.len())
}

/// Without a document: the alert answers the menu item, and no document is created.
#[cfg(target_os = "windows")]
#[test]
fn no_device_without_a_document_shows_the_alert_and_creates_no_document() {
    let (mut app, ctx) = app_with(false);
    let r = invoke(&mut app, &ctx, "file.import.wiaSupport", json!({})).unwrap();
    assert_eq!(r["available"], json!(true), "the command still answers the click");
    assert_eq!(alert_messages(&app), vec![MESSAGE.to_string()], "the alert answers the menu item");
    assert!(app.session.active().is_none(), "no document is created");
}

/// With a document: the alert answers the menu item; same document, no History state,
/// the layers unchanged.
#[cfg(target_os = "windows")]
#[test]
fn no_device_with_a_document_shows_the_alert_and_leaves_it_unchanged() {
    let (mut app, ctx) = app_with(true);
    let before = doc_state(&app);
    let id = app.session.active().expect("a document").doc.id;
    invoke(&mut app, &ctx, "file.import.wiaSupport", json!({})).unwrap();
    assert_eq!(alert_messages(&app), vec![MESSAGE.to_string()], "the alert answers the menu item");
    let st = app.session.active().expect("the document stays open");
    assert_eq!(st.doc.id, id, "the same document");
    assert_eq!((st.history.past_len(), st.doc.layers.len()), before, "no History state, the layers unchanged");
}

/// OK closes the alert and leaves the document unchanged.
#[cfg(target_os = "windows")]
#[test]
fn ok_closes_the_alert_and_the_document_stays_unchanged() {
    let (mut app, ctx) = app_with(true);
    let before = doc_state(&app);
    invoke(&mut app, &ctx, "file.import.wiaSupport", json!({})).unwrap();
    let alert = app.framework.dialogs.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some("alert")).expect("the alert is open").id;
    crate::dialogs::confirm(&mut app, &ctx, alert).expect("OK answers the alert");
    assert!(alert_messages(&app).is_empty(), "OK closes the alert");
    assert_eq!(doc_state(&app), before, "the document is unchanged");
}

/// Off Windows the menu item keeps today's behaviour: the command answers that the feature is
/// unavailable, with no alert and no document (not measured in the reference application).
#[cfg(not(target_os = "windows"))]
#[test]
fn off_windows_the_menu_item_keeps_todays_silent_path() {
    let (mut app, ctx) = app_with(false);
    let r = invoke(&mut app, &ctx, "file.import.wiaSupport", json!({})).unwrap();
    assert_eq!(r["available"], json!(false), "the engine reports the feature unavailable");
    assert!(alert_messages(&app).is_empty(), "no alert off Windows");
    assert!(app.session.active().is_none(), "no document is created");
}

/// Only a result that reports an empty device list opens the alert: once real enumeration names
/// a device, the menu item is silent as it is today, and a result without the list changes nothing.
#[test]
fn only_an_empty_device_list_reads_as_no_device() {
    assert!(invoke::lists_no_wia_device(&json!({"devices": []})), "an empty list reports no device");
    assert!(!invoke::lists_no_wia_device(&json!({"devices": ["WIA Imaging Device"]})), "a reported device stays silent");
    assert!(!invoke::lists_no_wia_device(&json!({})), "a result without the list changes nothing");
}
