//! Opt-in, current-tab automation. No network listener or server document access.
use photocraft_ui_egui::ControlRequest;
use std::sync::{Mutex, OnceLock, mpsc::Sender};
use wasm_bindgen::prelude::*;

static CHANNEL: OnceLock<Mutex<Option<(Sender<ControlRequest>, egui::Context)>>> = OnceLock::new();
pub fn attach(sender: Sender<ControlRequest>, context: egui::Context) {
    *CHANNEL.get_or_init(Mutex::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((sender, context));
}
#[wasm_bindgen]
pub async fn photocraft_control(request: &str) -> std::result::Result<String, JsValue> {
    if request.len() > 128 * 1024 {
        return Err(JsValue::from_str("Control request exceeds its budget"));
    }
    let value: serde_json::Value = serde_json::from_str(request).map_err(|_| JsValue::from_str("Invalid control JSON"))?;
    let method = value.get("method").and_then(|v| v.as_str()).ok_or_else(|| JsValue::from_str("Missing control method"))?;
    let (sender, context) = CHANNEL
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .ok_or_else(|| JsValue::from_str("Browser control is disabled; open this tab with ?control"))?;
    let (request, reply) = ControlRequest::new(method, value.get("params").cloned().unwrap_or_else(|| serde_json::json!({})));
    sender.send(request).map_err(|_| JsValue::from_str("Browser control is unavailable"))?;
    context.request_repaint();
    let start = js_sys::Date::now();
    loop {
        match reply.try_recv() {
            Ok(value) => return serde_json::to_string(&value).map_err(|_| JsValue::from_str("Invalid control response")),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return Err(JsValue::from_str("Browser control closed")),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        if js_sys::Date::now() - start > 3_600_000.0 {
            return Err(JsValue::from_str("Browser control timed out"));
        }
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("No browser window"))?;
        let mut failed = false;
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            failed = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 10).is_err();
        });
        if failed {
            return Err(JsValue::from_str("Browser timer unavailable"));
        }
        wasm_bindgen_futures::JsFuture::from(promise).await?;
    }
}
