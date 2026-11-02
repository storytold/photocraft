//! Real browser checks for the page-local file-provider capability, without starting eframe.

use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use photocraft_ui_egui::file_providers::Host;
use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

const API: &str = "photocraftFileProviders";
const LIMIT: u32 = 80 * 1024 * 1024;

// This helper is test-only: the product has no handwritten script or postMessage listener.
// It needs an actual opaque browsing context, which a Rust-only Host fixture cannot model.
#[wasm_bindgen(inline_js = r#"
export function probeOpaqueParentApi(parentOrigin, marker) {
    if (parentOrigin !== window.location.origin || parentOrigin === 'null') {
        return Promise.reject(new Error('an exact non-opaque parent origin is required'));
    }
    return new Promise((resolve, reject) => {
        const frame = document.createElement('iframe');
        frame.setAttribute('sandbox', 'allow-scripts');
        let timer;
        const cleanup = () => {
            clearTimeout(timer);
            window.removeEventListener('message', receive);
            frame.remove();
        };
        const receive = event => {
            if (event.source !== frame.contentWindow || event.origin !== 'null' ||
                !event.data || event.data.marker !== marker) {
                return;
            }
            const report = { denied: event.data.denied, errorName: event.data.errorName,
                senderOrigin: event.origin, attemptedMessage: event.data.attemptedMessage };
            cleanup();
            resolve(report);
        };
        window.addEventListener('message', receive);
        const originLiteral = JSON.stringify(parentOrigin);
        const child = 'let denied = false; let errorName = ""; ' +
            'try { void parent.photocraftFileProviders; } ' +
            'catch (error) { denied = true; errorName = error.name; } ' +
            'parent.postMessage({type:"photocraft-file-provider", method:"registerProvider", ' +
            'args:["untrusted", "Untrusted storage", "other-account"]}, ' + originLiteral + '); ' +
            'parent.postMessage({marker:' + JSON.stringify(marker) +
            ', denied, errorName, attemptedMessage:true}, ' + originLiteral + ');';
        frame.srcdoc = '<script>' + child + '</script>';
        timer = setTimeout(() => {
            cleanup();
            reject(new Error('opaque iframe did not report its access check'));
        }, 5000);
        (document.body || document.documentElement).appendChild(frame);
    });
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = probeOpaqueParentApi)]
    fn opaque_parent_probe(parent_origin: &str, marker: &str) -> js_sys::Promise;
}

struct InstalledApi {
    window: web_sys::Window,
    api: JsValue,
}

impl Drop for InstalledApi {
    fn drop(&mut self) {
        if let Ok(current) = Reflect::get(&self.window, &API.into())
            && Object::is(&current, &self.api)
        {
            // Remove only the object installed by this fixture; never another caller's API.
            let _ = Reflect::delete_property(self.window.unchecked_ref::<Object>(), &API.into());
        }
    }
}

fn call(api: &JsValue, method: &str, arguments: &[JsValue]) -> Result<JsValue, JsValue> {
    let function = Reflect::get(api, &method.into())?.dyn_into::<Function>()?;
    let args = Array::new();
    for argument in arguments {
        args.push(argument);
    }
    function.apply(api, &args)
}

fn field(object: &Object, key: &str, value: &JsValue) -> Result<(), JsValue> {
    if Reflect::set(object, &key.into(), value)? { Ok(()) } else { Err(JsValue::from_str("fixture object rejected its field")) }
}

fn resource(id: &str, name: &str, updated_at: &str) -> Result<Object, JsValue> {
    let object = Object::new();
    field(&object, "id", &id.into())?;
    field(&object, "name", &name.into())?;
    field(&object, "updatedAt", &updated_at.into())?;
    Ok(object)
}

fn list(entry: &Object) -> JsValue {
    let array = Array::new();
    array.push(entry);
    array.into()
}

fn rejected_without_snapshot_change(host: &Host, api: &JsValue, method: &str, arguments: &[JsValue]) {
    let before = host.snapshot();
    assert!(call(api, method, arguments).is_err(), "{method} accepted malformed browser arguments");
    assert_eq!(host.snapshot(), before, "{method} mutated the host after rejecting browser arguments");
}

/// One fixture owns the window capability across all phases, including the asynchronous iframe.
/// It does not depend on ordering between tests or install a GPU/editor instance.
#[wasm_bindgen_test]
pub async fn page_local_provider_boundary_checks_default_off_inputs_limits_and_opaque_frames() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("browser window required"))?;
    assert!(!Reflect::has(&window, &API.into())?, "the test page must not auto-start the editor");
    let host = Host::default();
    crate::file_providers::install(host.clone(), egui::Context::default())?;
    let api = Reflect::get(&window, &API.into())?;
    let _installed = InstalledApi { window: window.clone(), api: api.clone() };
    assert_eq!(Reflect::get(&api, &"protocolVersion".into())?.as_f64(), Some(1.0));
    assert_eq!(Reflect::get(&api, &"app".into())?.as_string().as_deref(), Some("photocraft"));
    assert!(Object::is_frozen(api.unchecked_ref::<Object>()));

    // Exposing the API does not opt in. Exercise the actual exported callbacks, not only Host.
    let initial = host.snapshot();
    assert!(!host.connected());
    assert!(!host.enabled());
    assert!(!host.busy());
    assert_eq!(initial["providerEnabled"], false);
    rejected_without_snapshot_change(&host, &api, "requestOpen", &["file-one".into()]);
    rejected_without_snapshot_change(&host, &api, "setEnabled", &[true.into()]);
    rejected_without_snapshot_change(&host, &api, "setEnabled", &["true".into()]);
    rejected_without_snapshot_change(&host, &api, "registerProvider", &[1.into(), "Files".into(), "account".into()]);
    rejected_without_snapshot_change(&host, &api, "registerProvider", &["provider".into(), "x".repeat(101).into(), "account".into()]);
    rejected_without_snapshot_change(&host, &api, "registerProvider", &["provider".into(), "Files".into(), "account/email".into()]);
    assert!(host.take_events().is_empty());

    // The sandbox deliberately omits allow-same-origin. Its origin is opaque, not the parent's.
    // It both tries the existing parent API and sends a forged request to the exact parent origin.
    let before_probe = host.snapshot();
    let origin = window.location().origin()?;
    let report = JsFuture::from(opaque_parent_probe(&origin, "photocraft-provider-opaque-probe")).await?;
    assert_eq!(Reflect::get(&report, &"denied".into())?.as_bool(), Some(true));
    assert_eq!(Reflect::get(&report, &"errorName".into())?.as_string().as_deref(), Some("SecurityError"));
    assert_eq!(Reflect::get(&report, &"senderOrigin".into())?.as_string().as_deref(), Some("null"));
    assert_eq!(Reflect::get(&report, &"attemptedMessage".into())?.as_bool(), Some(true));
    assert_eq!(host.snapshot(), before_probe);
    assert!(!host.connected());
    assert!(!host.busy());
    assert!(host.take_events().is_empty());

    // Positive registration and metadata cases ensure rejections above are not a disconnected stub.
    let connection = call(&api, "registerProvider", &["provider".into(), "Files".into(), "account-one".into()])?;
    assert!(connection.is_string());
    assert!(host.connected());
    assert!(host.enabled());
    let entry = resource("file-one", "Drawing.psd", "today")?;
    // Cyclic extra data cannot be JSON-serialized, but extra resource properties must be ignored.
    field(&entry, "ignored", &entry)?;
    call(&api, "setResources", &[connection.clone(), list(&entry)])?;
    let before_bad_lists = host.snapshot();
    rejected_without_snapshot_change(&host, &api, "setResources", &[connection.clone(), Object::new().into()]);
    let malformed = Object::new();
    field(&malformed, "id", &1.into())?;
    field(&malformed, "name", &"Drawing.psd".into())?;
    rejected_without_snapshot_change(&host, &api, "setResources", &[connection.clone(), list(&malformed)]);
    let duplicate = Array::new();
    duplicate.push(&entry);
    duplicate.push(&entry);
    rejected_without_snapshot_change(&host, &api, "setResources", &[connection.clone(), duplicate.into()]);
    for bad in [
        resource(&"x".repeat(161), "Drawing.psd", "today")?,
        resource("file-one", &"x".repeat(256), "today")?,
        resource("file-one", "../Drawing.psd", "today")?,
        resource("file-one", "Drawing.psd", &"x".repeat(101))?,
    ] {
        rejected_without_snapshot_change(&host, &api, "setResources", &[connection.clone(), list(&bad)]);
    }
    let too_many = Array::new();
    for index in 0..101 {
        too_many.push(&resource(&format!("file-{index}"), "Drawing.psd", "today")?.into());
    }
    rejected_without_snapshot_change(&host, &api, "setResources", &[connection.clone(), too_many.into()]);
    assert_eq!(host.snapshot(), before_bad_lists);
    // The snapshot does not expose resource metadata: check it through a real read event as well.
    let retained = call(&api, "requestOpen", &["file-one".into()])?;
    let retained_events = host.take_events();
    assert_eq!(retained_events.len(), 1);
    assert_eq!(retained_events[0].metadata["filename"], "Drawing.psd");
    call(&api, "cancelOpen", &[retained, "".into()])?;
    assert_eq!(host.take_events().len(), 1);
    assert!(!host.busy());
    let maximum = Array::new();
    for index in 0..100 {
        maximum.push(&resource(&format!("file-{index}"), "Drawing.psd", "today")?.into());
    }
    call(&api, "setResources", &[connection.clone(), maximum.into()])?;
    call(&api, "setResources", &[connection.clone(), list(&entry)])?;

    // Keep a valid live read throughout the rejection cases: none may consume or admit it.
    let request = call(&api, "requestOpen", &["file-one".into()])?;
    let events = host.take_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].metadata["type"], "openRequest");
    assert_eq!(events[0].metadata["requestId"].as_str(), request.as_string().as_deref());
    assert!(host.busy());
    rejected_without_snapshot_change(&host, &api, "openResource", &[request.clone(), "Drawing.psd".into(), Array::new().into()]);
    rejected_without_snapshot_change(&host, &api, "openResource", &[request.clone(), "Drawing.psd".into(), Uint8Array::new_with_length(0).into()]);
    rejected_without_snapshot_change(&host, &api, "openResource", &[request.clone(), "../Drawing.psd".into(), Uint8Array::new_with_length(1).into()]);
    let oversized = Uint8Array::new_with_length(LIMIT + 1);
    assert_eq!(oversized.length(), LIMIT + 1);
    let error = call(&api, "openResource", &[request.clone(), "Drawing.psd".into(), oversized.into()]);
    assert_eq!(error.err().and_then(|error| error.as_string()).as_deref(), Some("file exceeds the provider limit"));
    // This error is from the JS/WASM adapter before to_vec, not the core's separate size check.
    assert!(host.busy());
    assert!(host.take_events().is_empty());
    rejected_without_snapshot_change(&host, &api, "cancelOpen", &[request.clone(), "x".repeat(1025).into()]);
    call(&api, "cancelOpen", &[request.clone(), "".into()])?;
    let cancelled = host.take_events();
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled[0].metadata["type"], "openCancelled");
    assert_eq!(cancelled[0].metadata["requestId"].as_str(), request.as_string().as_deref());
    assert!(!host.busy());
    rejected_without_snapshot_change(&host, &api, "openResource", &[request, "Drawing.psd".into(), Uint8Array::new_with_length(1).into()]);
    rejected_without_snapshot_change(&host, &api, "saveResult", &["unknown".into(), "invented".into(), "".into()]);
    rejected_without_snapshot_change(&host, &api, "resolveSaveDialog", &["unknown".into(), "invalid".into(), "Drawing.psd".into()]);
    call(&api, "unregisterProvider", &[])?;
    assert!(!host.connected());
    assert!(!host.enabled());
    Ok(())
}
