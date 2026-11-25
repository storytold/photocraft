//! Rust-only opaque-child role for the existing generated wasm-bindgen-test page.
//! No generated glue is edited and no parent-realm callback impersonates child execution.
use std::cell::Cell;
use std::rc::Rc;

use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;

use super::{API, EVENT, call, field};

const ROLE: &str = "#photocraft-origin-child=";
const CHILD_TIMEOUT_MS: i32 = 600_000;

fn error(message: impl ToString) -> JsValue {
    JsValue::from_str(&message.to_string())
}

fn get(value: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    Reflect::get(value, &name.into())
}

fn text(value: &JsValue, name: &str) -> Result<String, JsValue> {
    get(value, name)?.as_string().ok_or_else(|| error(format!("missing child {name}")))
}

fn endpoint() -> Result<String, JsValue> {
    let endpoint =
        option_env!("PHOTOCRAFT_TEST_ORIGIN_RELAY").ok_or_else(|| error("run cargo xtask browser-test-provider-origin with bound artifact evidence"))?;
    let url = web_sys::Url::new(endpoint)?;
    let path = url.pathname();
    let token = path.strip_prefix('/').and_then(|path| path.strip_suffix('/')).ok_or_else(|| error("bad relay capability path"))?;
    if url.protocol() != "http:"
        || url.hostname() != "127.0.0.1"
        || url.port().parse::<u16>().ok().is_none_or(|port| port == 0)
        || !url.username().is_empty()
        || !url.password().is_empty()
        || !url.search().is_empty()
        || !url.hash().is_empty()
        || token.len() != 64
        || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
        || url.href() != endpoint
    {
        return Err(error("test artifact endpoint must be one exact IPv4 loopback capability URL"));
    }
    Ok(endpoint.to_string())
}

fn parent_origin(value: &str) -> Result<(), JsValue> {
    let url = web_sys::Url::new(value)?;
    if url.protocol() != "http:" || url.hostname() != "127.0.0.1" || url.origin() != value || url.port().parse::<u16>().ok().is_none_or(|port| port == 0) {
        return Err(error("exact loopback runner parent origin required"));
    }
    Ok(())
}

/// Called before the normal fixture installs any Host/API. The generated runner invokes the
/// same selected Rust test in this real sandboxed browsing context, with an independent WASM instance.
pub(super) fn child_role() -> Result<bool, JsValue> {
    let window = web_sys::window().ok_or_else(|| error("browser window required"))?;
    let fragment = window.location().hash()?;
    let Some(encoded) = fragment.strip_prefix(ROLE) else {
        return Ok(false);
    };
    if encoded.len() > 2048 {
        return Err(error("opaque child role too large"));
    }
    let configured = endpoint()?;
    let actual = web_sys::Url::new(&window.location().href()?)?;
    actual.set_hash("");
    if actual.href() != configured {
        return Err(error("child role must execute on the owned artifact page"));
    }
    let role = js_sys::JSON::parse(&String::from(js_sys::decode_uri_component(encoded)?))?;
    let origin = text(&role, "parentOrigin")?;
    parent_origin(&origin)?;
    let marker = text(&role, "marker")?;
    let nonce = text(&role, "nonce")?;
    if marker != "photocraft-provider-opaque-probe" || nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(error("invalid opaque-child role correlation"));
    }
    let parent = window.parent()?.ok_or_else(|| error("child has no parent WindowProxy"))?;
    if Object::is(parent.as_ref(), window.as_ref()) {
        return Err(error("opaque role cannot run in the top-level window"));
    }
    // This actual child's lookup must throw SecurityError. If a future browser/fixture change
    // exposes the object, the genuine exported registerProvider call is attempted and the parent
    // denial/snapshot assertions fail. No access failure is inferred from the reverse direction.
    let access =
        get(parent.as_ref(), API).and_then(|api| call(&api, "registerProvider", &["untrusted".into(), "Untrusted storage".into(), "other-account".into()]));
    let (denied, name) = match access {
        Ok(_) => (false, String::new()),
        Err(exception) => (true, text(&exception, "name")?),
    };
    let forged = Object::new();
    field(&forged, "type", &EVENT.into())?;
    field(&forged, "method", &"registerProvider".into())?;
    field(&forged, "args", &Array::of3(&"untrusted".into(), &"Untrusted storage".into(), &"other-account".into()))?;
    field(&forged, "marker", &marker.clone().into())?;
    field(&forged, "nonce", &nonce.clone().into())?;
    parent.post_message(&forged, &origin)?;
    let report = Object::new();
    field(&report, "marker", &marker.into())?;
    field(&report, "nonce", &nonce.into())?;
    field(&report, "denied", &denied.into())?;
    field(&report, "errorName", &name.into())?;
    field(&report, "attemptedMessage", &true.into())?;
    // Only true after the actual forged postMessage call above returned successfully.
    parent.post_message(&report, &origin)?;
    Ok(true)
}

fn message(frame: &web_sys::Element, event: &web_sys::Event, marker: &str, nonce: &str, forged: &Cell<u32>) -> Result<Option<JsValue>, JsValue> {
    if !event.is_trusted() {
        return Ok(None);
    }
    let source = get(event.as_ref(), "source")?;
    let child = get(frame.as_ref(), "contentWindow")?;
    if child.is_null() || !Object::is(&source, &child) || text(event.as_ref(), "origin")? != "null" {
        return Ok(None);
    }
    if frame.get_attribute("sandbox").as_deref() != Some("allow-scripts") {
        return Err(error("opaque child's actual sandbox flags changed"));
    }
    let data = get(event.as_ref(), "data")?;
    if text(&data, "marker")? != marker || text(&data, "nonce")? != nonce {
        return Ok(None);
    }
    if get(&data, "type")?.as_string().as_deref() == Some(EVENT) {
        let args = get(&data, "args")?.dyn_into::<Array>()?;
        if text(&data, "method")? != "registerProvider"
            || args.length() != 3
            || args.get(0).as_string().as_deref() != Some("untrusted")
            || args.get(1).as_string().as_deref() != Some("Untrusted storage")
            || args.get(2).as_string().as_deref() != Some("other-account")
        {
            return Err(error("real child's forged request did not match the original scenario"));
        }
        forged.set(forged.get().saturating_add(1));
        return Ok(None);
    }
    if forged.get() != 1
        || get(&data, "denied")?.as_bool() != Some(true)
        || text(&data, "errorName")? != "SecurityError"
        || get(&data, "attemptedMessage")?.as_bool() != Some(true)
    {
        return Err(error("real opaque child did not prove the exact denied API/forged-message boundary"));
    }
    let report = Object::new();
    for key in ["marker", "nonce", "denied", "errorName", "attemptedMessage"] {
        field(&report, key, &get(&data, key)?)?;
    }
    field(&report, "senderOrigin", &"null".into())?;
    field(&report, "realSourceMatched", &true.into())?;
    field(&report, "trustedMessage", &true.into())?;
    field(&report, "sandbox", &"allow-scripts".into())?;
    field(&report, "forgedMessages", &forged.get().into())?;
    Ok(Some(report.into()))
}

struct Probe {
    window: web_sys::Window,
    frame: web_sys::Element,
    listener: Closure<dyn FnMut(web_sys::Event)>,
    timeout: Closure<dyn FnMut()>,
    timer: Option<i32>,
}
impl Drop for Probe {
    fn drop(&mut self) {
        if let Some(timer) = self.timer {
            self.window.clear_timeout_with_handle(timer);
        }
        let _ = self.window.remove_event_listener_with_callback("message", self.listener.as_ref().unchecked_ref());
        self.frame.remove();
    }
}

/// Receives only real MessageEvents from this exact child WindowProxy, then removes our
/// frame/listener/timer on success, rejected JS operations, timeout or any later assertion.
pub(super) async fn probe(window: &web_sys::Window, marker: &str) -> Result<JsValue, JsValue> {
    let endpoint = endpoint()?;
    let origin = window.location().origin()?;
    parent_origin(&origin)?;
    let bytes = Uint8Array::new_with_length(16);
    let crypto = get(window.as_ref(), "crypto")?;
    get(&crypto, "getRandomValues")?.dyn_into::<Function>()?.call1(&crypto, &bytes)?;
    let nonce: String = bytes.to_vec().iter().map(|byte| format!("{byte:02x}")).collect();
    let role = Object::new();
    field(&role, "parentOrigin", &origin.clone().into())?;
    field(&role, "marker", &marker.into())?;
    field(&role, "nonce", &nonce.clone().into())?;
    let fragment = js_sys::encode_uri_component(&String::from(js_sys::JSON::stringify(&role)?));
    let document = window.document().ok_or_else(|| error("test document required"))?;
    let frame = document.create_element("iframe")?;
    frame.set_attribute("sandbox", "allow-scripts")?;
    frame.set_attribute("src", &format!("{endpoint}{ROLE}{}", String::from(fragment)))?;
    let mut resolve = None;
    let mut reject = None;
    let promise = Promise::new(&mut |yes, no| {
        resolve = Some(yes);
        reject = Some(no);
    });
    let resolve = resolve.ok_or_else(|| error("Promise resolver unavailable"))?;
    let reject = reject.ok_or_else(|| error("Promise rejector unavailable"))?;
    let finished = Rc::new(Cell::new(false));
    let forged = Rc::new(Cell::new(0_u32));
    let done = finished.clone();
    let observed = frame.clone();
    let expected_nonce = nonce.clone();
    let expected_marker = marker.to_string();
    let reject_event = reject.clone();
    let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        if done.get() {
            return;
        }
        match message(&observed, &event, &expected_marker, &expected_nonce, &forged) {
            Ok(None) => {}
            Ok(Some(report)) => {
                done.set(true);
                if let Err(error) = resolve.call1(&JsValue::UNDEFINED, &report) {
                    let _ = reject_event.call1(&JsValue::UNDEFINED, &error);
                }
            }
            Err(error) => {
                done.set(true);
                let _ = reject_event.call1(&JsValue::UNDEFINED, &error);
            }
        }
    });
    let timeout = Closure::<dyn FnMut()>::new(move || {
        if !finished.replace(true) {
            let _ = reject.call1(&JsValue::UNDEFINED, &error("opaque Rust child did not report its real access check"));
        }
    });
    let mut owned = Probe { window: window.clone(), frame, listener, timeout, timer: None };
    window.add_event_listener_with_callback("message", owned.listener.as_ref().unchecked_ref())?;
    owned.timer = Some(window.set_timeout_with_callback_and_timeout_and_arguments_0(owned.timeout.as_ref().unchecked_ref(), CHILD_TIMEOUT_MS)?);
    document
        .body()
        .map(|body| body.unchecked_into::<web_sys::Element>())
        .or_else(|| document.document_element())
        .ok_or_else(|| error("document has no attachment point"))?
        .append_child(&owned.frame)?;
    let report = JsFuture::from(promise).await?;
    field(report.unchecked_ref::<Object>(), "parentOrigin", &origin.into())?;
    field(report.unchecked_ref::<Object>(), "relayOrigin", &web_sys::Url::new(&endpoint)?.origin().into())?;
    let proof = String::from(js_sys::JSON::stringify(&report)?);
    wasm_bindgen_test::console_log!("PHOTOCRAFT_OPAQUE_CHILD_PROOF {proof}");
    Ok(report)
}
