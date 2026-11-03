//! Original synthetic PhotoCraft pixels and embedded native resource, MIT OR Apache-2.0.
//! Separate Rust-only delivery fixture: real Window API, publisher, native codec and app consumer.
//! Browser mode is configured for this binary by the existing boundary fixture.
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use photocraft_ui_egui::{
    FileDialogAnswer, FileDialogRequest, PhotocraftApp, Services,
    file_providers::{self, Host},
};
use serde_json::json;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_test::wasm_bindgen_test;

type Captures = Rc<RefCell<Vec<Vec<u8>>>>;

fn error(message: impl ToString) -> JsValue {
    JsValue::from_str(&message.to_string())
}
fn field(object: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    Reflect::get(object, &name.into())
}
fn text(object: &JsValue, name: &str) -> Result<String, JsValue> {
    field(object, name)?.as_string().ok_or_else(|| error(format!("missing string field {name}")))
}
fn call(object: &JsValue, name: &str, arguments: &[JsValue]) -> Result<JsValue, JsValue> {
    let function = field(object, name)?.dyn_into::<Function>()?;
    let array = Array::new();
    for argument in arguments {
        array.push(argument);
    }
    function.apply(object, &array)
}

struct Page {
    window: web_sys::Window,
    api: JsValue,
    host: Host,
    ctx: egui::Context,
    events: Rc<RefCell<Vec<JsValue>>>,
    listener: Closure<dyn FnMut(web_sys::Event)>,
}
impl Page {
    fn install(host: Host) -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or_else(|| error("real browser Window required"))?;
        if Reflect::has(&window, &super::API.into())? {
            return Err(error("delivery fixture requires its own fresh page capability"));
        }
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(event) = event.dyn_ref::<web_sys::CustomEvent>() {
                observed.borrow_mut().push(event.detail());
            }
        });
        window.add_event_listener_with_callback(super::EVENT, listener.as_ref().unchecked_ref())?;
        let mut page = Self { window, api: JsValue::UNDEFINED, host, ctx: egui::Context::default(), events, listener };
        let installed = super::install(page.host.clone(), page.ctx.clone());
        page.api = field(&page.window, super::API)?;
        installed?;
        Ok(page)
    }
    fn api(&self, method: &str, arguments: &[JsValue]) -> Result<JsValue, JsValue> {
        call(&self.api, method, arguments)
    }
    fn matching(&self, kind: &str, request: &str) -> Vec<JsValue> {
        self.events
            .borrow()
            .iter()
            .filter(|event| text(event, "type").is_ok_and(|value| value == kind) && text(event, "requestId").is_ok_and(|value| value == request))
            .cloned()
            .collect()
    }
    fn one(&self, kind: &str, request: &str) -> Result<JsValue, JsValue> {
        let events = self.matching(kind, request);
        if events.len() != 1 {
            return Err(error(format!("expected one real window {kind} for {request}, got {}", events.len())));
        }
        events.into_iter().next().ok_or_else(|| error("missing real event"))
    }
    fn tick(&self, app: &mut PhotocraftApp) {
        // This is the actual public app-thread consumer, not a simulated ACK/checkpoint helper.
        file_providers::tick(app, &self.ctx);
        app.poll_file_dialog(&self.ctx, None);
    }
    fn queue_dialog(&self, app: &mut PhotocraftApp) -> Result<(), JsValue> {
        app.save_as(None).map_err(error)?;
        app.poll_file_dialog(&self.ctx, None);
        assert!(app.file_dialog_open());
        Ok(())
    }
    fn capture(&self, app: &mut PhotocraftApp) -> Result<String, JsValue> {
        let before = self.events.borrow().len();
        self.queue_dialog(app)?;
        super::publish(&self.host);
        let dialogs: Vec<_> =
            self.events.borrow().iter().skip(before).filter(|event| text(event, "type").is_ok_and(|kind| kind == "saveDialog")).cloned().collect();
        assert_eq!(dialogs.len(), 1, "the actual dialog must reach Window once");
        let dialog = dialogs.first().ok_or_else(|| error("missing real dialog"))?;
        self.api("resolveSaveDialog", &[text(dialog, "requestId")?.into(), "provider".into(), "delivery.pcraft".into()])?;
        app.poll_file_dialog(&self.ctx, None);
        file_providers::capture(app);
        let snapshot = self.api("snapshot", &[])?;
        let documents = field(&snapshot, "documents")?.dyn_into::<Array>()?;
        let active = app.session.active().ok_or_else(|| error("missing active document"))?.doc.id.0.to_string();
        let document = documents
            .iter()
            .find(|document| text(document, "documentId").is_ok_and(|id| id == active))
            .ok_or_else(|| error("missing native capture snapshot"))?;
        text(&document, "pendingRequestId")
    }
    // Function::call0 catches a regression that escapes the production copy import. Restore
    // the exact property descriptor before returning its result or asserting the outcome.
    fn with_replacement(&self, target: &JsValue, key: &str, replacement: &JsValue) -> Result<Result<JsValue, JsValue>, JsValue> {
        let object = field(&js_sys::global(), "Object")?;
        let descriptor = call(&object, "getOwnPropertyDescriptor", &[target.clone(), key.into()])?;
        let original = field(target, key)?;
        let host = self.host.clone();
        let operation = Closure::<dyn FnMut()>::new(move || super::publish(&host));
        let outcome = match Reflect::set(target, &key.into(), replacement) {
            Ok(true) => operation.as_ref().unchecked_ref::<Function>().call0(&JsValue::UNDEFINED),
            Ok(false) => Err(error("injected method is not writable")),
            Err(error) => Err(error),
        };
        if descriptor.is_undefined() {
            if !Reflect::delete_property(target.unchecked_ref::<Object>(), &key.into())? {
                return Err(error("could not remove injected method"));
            }
        } else {
            call(&object, "defineProperty", &[target.clone(), key.into(), descriptor.clone()])?;
        }
        let restored = call(&object, "getOwnPropertyDescriptor", &[target.clone(), key.into()])?;
        if field(target, key)? != original || restored.is_undefined() != descriptor.is_undefined() {
            return Err(error("injected property was not restored exactly"));
        }
        if !descriptor.is_undefined() {
            for attribute in ["value", "get", "set", "writable", "enumerable", "configurable"] {
                if field(&restored, attribute)? != field(&descriptor, attribute)? {
                    return Err(error("injected property descriptor changed"));
                }
            }
        }
        Ok(outcome)
    }
    fn fail_copy(&self) -> Result<u32, JsValue> {
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let fault = Closure::<dyn FnMut(JsValue) -> Result<JsValue, JsValue>>::new(move |_| {
            observed.set(observed.get() + 1);
            Err(js_sys::RangeError::new("synthetic PhotoCraft byte-copy failure").into())
        });
        let array = field(&js_sys::global(), "Uint8Array")?;
        self.with_replacement(&array, "from", fault.as_ref())??;
        Ok(calls.get())
    }
    fn assert_taken_terminal_connection(&self) -> Result<(), JsValue> {
        assert!(!self.host.busy());
        let request = self.api("requestOpen", &["cancelled-before-publication".into()])?;
        let request_id = request.as_string().ok_or_else(|| error("read request must be a string"))?;
        self.api("cancelOpen", &[request, "".into()])?;
        // No outstanding read remains. Only a terminal later in the CURRENT taken batch can
        // prevent this real, synchronous openRequest listener from unregistering its account.
        let rejected = Rc::new(Cell::new(false));
        let observed = rejected.clone();
        let api = self.api.clone();
        let wanted = request_id.clone();
        let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(event) = event.dyn_ref::<web_sys::CustomEvent>() {
                let detail = event.detail();
                if text(&detail, "type").is_ok_and(|kind| kind == "openRequest") && text(&detail, "requestId").is_ok_and(|request| request == wanted) {
                    observed.set(call(&api, "unregisterProvider", &[]).is_err());
                }
            }
        });
        self.window.add_event_listener_with_callback(super::EVENT, listener.as_ref().unchecked_ref())?;
        super::publish(&self.host);
        self.window.remove_event_listener_with_callback(super::EVENT, listener.as_ref().unchecked_ref())?;
        assert!(rejected.get(), "a real Window listener must not disconnect before the later taken terminal");
        self.one("openCancelled", &request_id)?;
        assert!(self.host.connected());
        assert!(!self.host.busy());
        Ok(())
    }

    fn fail_dispatch(&self, kind: &str, accept_save_first: bool) -> Result<Vec<String>, JsValue> {
        let original = field(&self.window, "dispatchEvent")?.dyn_into::<Function>()?;
        let window: JsValue = self.window.clone().into();
        let api = self.api.clone();
        let wanted = kind.to_string();
        let requests = Rc::new(RefCell::new(Vec::new()));
        let observed = requests.clone();
        let fault = Closure::<dyn FnMut(JsValue) -> Result<JsValue, JsValue>>::new(move |event: JsValue| {
            let detail = field(&event, "detail")?;
            if text(&detail, "type")? != wanted {
                return original.call1(&window, &event);
            }
            let request = text(&detail, "requestId")?;
            observed.borrow_mut().push(request.clone());
            if accept_save_first {
                original.call1(&window, &event)?;
                call(&api, "saveResult", &[request.into(), "saved".into(), "remote".into()])?;
            }
            Err(js_sys::RangeError::new("synthetic PhotoCraft dispatch failure").into())
        });
        self.with_replacement(&self.window, "dispatchEvent", fault.as_ref())??;
        let observed = requests.borrow().clone();
        Ok(observed)
    }
}
impl Drop for Page {
    fn drop(&mut self) {
        let _ = self.window.remove_event_listener_with_callback(super::EVENT, self.listener.as_ref().unchecked_ref());
        if field(&self.window, super::API).is_ok_and(|current| Object::is(&current, &self.api)) {
            let _ = Reflect::delete_property(self.window.unchecked_ref::<Object>(), &super::API.into());
            super::CURRENT_CONTEXT.with(|current| *current.borrow_mut() = None);
        }
    }
}

fn application() -> Result<(PhotocraftApp, Host, Captures), JsValue> {
    let host = Host::default();
    let dialog = host.clone();
    let captures = Captures::default();
    let recorded = captures.clone();
    let services = Services {
        file_provider: Some(host.clone()),
        import: Some(Box::new(|name, bytes, options| {
            photocraft_io::import_with_options(name, bytes, options).map(|result| (result.document, result.warnings)).map_err(|error| error.to_string())
        })),
        export: Some(Box::new(move |doc, name, settings| {
            let result = photocraft_io::export(doc, name, &settings.io_options()).map_err(|error| error.to_string())?;
            recorded.borrow_mut().push(result.bytes.clone());
            Ok((result.bytes, result.warnings))
        })),
        file_dialog: Some(Box::new(move |request, _, reply| match request {
            FileDialogRequest::Save { suggested } if dialog.wants_dialog() => dialog.dialog(suggested, reply),
            _ => reply.send(Some(FileDialogAnswer::Unavailable("fixture must not use a local file dialog".into()))),
        })),
        write: Some(Box::new(|_, _| Err("fixture must not use a device writer".into()))),
        write_unconfirmed: true,
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width":2,"height":2,"name":"Delivery"})).map_err(error)?;
    let image =
        photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgba, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 70, 90, 110, 255])
            .map_err(error)?;
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).map_err(error)?;
    let raster = photocraft_io::import("original.png", &png).map_err(error)?.document;
    let state = app.session.active_mut().ok_or_else(|| error("no fixture document"))?;
    let doc = Arc::make_mut(&mut state.doc);
    doc.layers = raster.layers;
    doc.metadata.text.push(("Original".into(), "Native delivery fixture".into()));
    doc.metadata.psd_resources.push((4000, "Original embedded bytes".into(), Arc::new(png)));
    state.saved_revision = 0;
    Ok((app, host, captures))
}

fn decoded(bytes: &[u8]) -> Result<photocraft_doc::Document, JsValue> {
    photocraft_io::import("delivery.pcraft", bytes).map(|result| result.document).map_err(error)
}

fn assert_native_payload(bytes: &[u8], source: &photocraft_doc::Document) -> Result<(), JsValue> {
    let document = decoded(bytes)?;
    assert_eq!(document.layers, source.layers, "native tile data must survive provider capture");
    assert_eq!(document.metadata, source.metadata, "embedded original resource bytes must survive");
    assert_eq!(document.size, source.size);
    Ok(())
}

#[wasm_bindgen_test]
pub fn real_window_delivery_recovery_preserves_photo_native_resources_and_exact_app_ownership() -> Result<(), JsValue> {
    let (mut app, host, captures) = application()?;
    let page = Page::install(host.clone())?;
    assert!(!host.connected());
    assert!(!host.enabled());
    let connection = page.api("registerProvider", &["delivery-provider".into(), "Delivery files".into(), "account-one".into()])?;
    let source = app.session.active().ok_or_else(|| error("no active source"))?.doc.clone();
    let doc = source.id;
    let saved_before = app.session.active().ok_or_else(|| error("no active source"))?.saved_revision;
    let first = page.capture(&mut app)?;
    let captured_revision = app.session.active().ok_or_else(|| error("no active capture"))?.revision;
    let independent = page.api("requestOpen", &["independent".into()])?;
    assert_eq!(page.fail_copy()?, 1, "the actual catch-enabled static copy must execute");
    assert!(page.matching("save", &first).is_empty());
    page.one("openRequest", &independent.as_string().ok_or_else(|| error("read request must be a string"))?)?;
    assert!(host.pending_for(doc), "the app must consume its own failure on a later tick");
    assert_native_payload(captures.borrow().first().ok_or_else(|| error("native exporter was not called"))?, &source)?;
    app.run("layer.new.layer", json!({"name":"Later edit"})).map_err(error)?;
    let later = app.session.active().ok_or_else(|| error("no later state"))?.doc.clone();
    let revision = app.session.active().ok_or_else(|| error("no later state"))?.revision;
    let journal = app.session.journal.clone();
    page.tick(&mut app);
    assert!(Arc::ptr_eq(&app.session.active().ok_or_else(|| error("no failed source"))?.doc, &later));
    assert_eq!(app.session.active().ok_or_else(|| error("no failed source"))?.saved_revision, saved_before);
    assert_eq!(app.session.active().ok_or_else(|| error("no failed source"))?.revision, revision);
    assert!(app.session.active().ok_or_else(|| error("no failed source"))?.is_dirty());
    assert_eq!(app.session.journal, journal);
    assert!(!host.pending_for(doc));
    assert!(!host.bound(doc));
    assert!(page.api("saveResult", &[first.clone().into(), "saved".into(), "remote".into()]).is_err());
    super::publish(&host);
    let failed = page.one("saveFailed", &first)?;
    assert_eq!(text(&failed, "revision")?, captured_revision.to_string());
    assert_eq!(field(&failed, "connectionId")?, connection);
    page.api("cancelOpen", &[independent, "".into()])?;
    super::publish(&host);

    let retry = page.capture(&mut app)?;
    assert_ne!(retry, first);
    let retry_revision = app.session.active().ok_or_else(|| error("no retry source"))?.revision;
    let retry_doc = app.session.active().ok_or_else(|| error("no retry source"))?.doc.clone();
    super::publish(&host);
    let save = page.one("save", &retry)?;
    let payload = field(&save, "bytes")?.dyn_into::<Uint8Array>()?.to_vec();
    assert_eq!(captures.borrow().last(), Some(&payload));
    assert_native_payload(&payload, &retry_doc)?;
    page.api("saveResult", &[retry.clone().into(), "saved".into(), "remote".into()])?;
    app.run("layer.new.layer", json!({"name":"After capture"})).map_err(error)?;
    page.tick(&mut app);
    assert_eq!(app.session.active().ok_or_else(|| error("no ACK source"))?.saved_revision, retry_revision);
    assert!(app.session.active().ok_or_else(|| error("no ACK source"))?.is_dirty());
    assert!(host.bound(doc));
    assert!(page.api("saveResult", &[retry.into(), "saved".into(), "remote".into()]).is_err());

    let dispatch_failed = page.capture(&mut app)?;
    let checkpoint = app.session.active().ok_or_else(|| error("no dispatch source"))?.saved_revision;
    assert_eq!(page.fail_dispatch("save", false)?, [dispatch_failed.clone()]);
    page.tick(&mut app);
    super::publish(&host);
    page.one("saveFailed", &dispatch_failed)?;
    assert_eq!(app.session.active().ok_or_else(|| error("no dispatch source"))?.saved_revision, checkpoint);
    assert!(!host.pending_for(doc));

    let accepted = page.capture(&mut app)?;
    let accepted_revision = app.session.active().ok_or_else(|| error("no accepted source"))?.revision;
    assert_eq!(page.fail_dispatch("save", true)?, [accepted.clone()]);
    page.tick(&mut app);
    super::publish(&host);
    assert_eq!(app.session.active().ok_or_else(|| error("no accepted source"))?.saved_revision, accepted_revision);
    assert!(page.matching("saveFailed", &accepted).is_empty(), "an accepted synchronous ACK wins over a later throw");

    // Force a real chooser error, then prove its original exported callback is retired.
    page.queue_dialog(&mut app)?;
    let failed_dialogs = page.fail_dispatch("saveDialog", false)?;
    assert_eq!(failed_dialogs.len(), 1);
    page.tick(&mut app);
    assert!(!app.file_dialog_open());
    assert!(!host.saving());
    let failed_dialog = failed_dialogs.first().ok_or_else(|| error("missing failed dialog"))?;
    assert!(page.api("resolveSaveDialog", &[failed_dialog.clone().into(), "provider".into(), "late.pcraft".into()]).is_err());

    let read = page.api("requestOpen", &["failed-read".into()])?;
    let read_id = read.as_string().ok_or_else(|| error("read request must be a string"))?;
    assert_eq!(page.fail_dispatch("openRequest", false)?, [read_id.clone()]);
    page.one("openFailed", &read_id)?;
    assert!(page.api("openResource", &[read, "late.pcraft".into(), Uint8Array::from(payload.as_slice()).into()]).is_err());

    page.assert_taken_terminal_connection()?;

    // Reopen the actual published native payload through the real exported callback and decoder.
    let read = page.api("requestOpen", &["remote".into()])?;
    let read_id = read.as_string().ok_or_else(|| error("reopen request must be a string"))?;
    super::publish(&host);
    page.api("openResource", &[read, "delivery.pcraft".into(), Uint8Array::from(payload.as_slice()).into()])?;
    page.tick(&mut app);
    super::publish(&host);
    let opened = page.one("opened", &read_id)?;
    assert_eq!(field(&opened, "connectionId")?, connection);
    let reopened = app.session.active().ok_or_else(|| error("native project was not admitted"))?;
    assert_eq!(reopened.doc.layers, retry_doc.layers);
    assert_eq!(reopened.doc.metadata, retry_doc.metadata);
    assert_ne!(reopened.doc.id, doc, "an admitted native ID collision must be remapped");
    let original = app.session.documents().iter().find(|state| state.doc.id == doc).ok_or_else(|| error("original tab was lost"))?;
    assert_eq!(original.saved_revision, accepted_revision);
    assert_eq!(original.doc.layers.len(), retry_doc.layers.len() + 1);
    assert!(original.path.is_none());
    assert!(app.ui.recent_files.is_empty());

    page.api("unregisterProvider", &[])?;
    let next = page.api("registerProvider", &["delivery-provider".into(), "Delivery files".into(), "account-two".into()])?;
    assert_ne!(next, connection);
    assert!(page.api("saveResult", &[accepted.into(), "saved".into(), "remote".into()]).is_err());
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(page.api("snapshot", &[]).and_then(|snapshot| text(&snapshot, "namespace"))?, "account-two");
    Ok(())
}
