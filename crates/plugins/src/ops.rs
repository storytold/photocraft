//! The ABI v2 vocabulary: the events the host sends and the operations a panel returns.
//!
//! A panel plug-in is a state machine. The host sends one JSON event and gets back a JSON array of
//! **operations** to carry out (`ui.view`, `doc.commit`, `http.request`, …). Everything a plug-in
//! says is untrusted input, so each operation is validated here — ranges, lengths, ids, blob names
//! and loopback URLs — before the engine sees it.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::view::{self, Widget};
use crate::{Error, Result};

/// Most operations one `pc_event` may return.
pub const MAX_OPS: usize = 64;
/// Largest operations array accepted from one `pc_event` (and the largest event sent to it).
pub const MAX_OPS_BYTES: usize = 1 << 20;
/// Longest text in a `ui.notice` or `ui.progress`.
pub const MAX_TEXT: usize = 4_000;
/// Most layers one `doc.commit` may add.
pub const MAX_COMMIT_LAYERS: usize = 32;
/// Shortest and longest `timer.set` delay, in milliseconds.
pub const MIN_TIMER_MS: u64 = 16;
pub const MAX_TIMER_MS: u64 = 600_000;
/// Largest `timeoutMs` an `http.request` may ask for.
pub const MAX_HTTP_TIMEOUT_MS: u64 = 600_000;
/// Largest side a `doc.capture` may ask for.
pub const MAX_CAPTURE_SIDE: u32 = 16_384;
/// Largest side a committed layer may be resampled to.
pub const MAX_COMMIT_SIDE: u32 = 32_768;
/// Largest `params` one `doc.command` may carry, serialized.
pub const MAX_COMMAND_PARAMS_BYTES: usize = 64 << 10;

/// An operation id: whatever the plug-in used, echoed back in the `result` event. Operations
/// without one are fire-and-forget.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OpId {
    Num(i64),
    Text(String),
}

impl std::fmt::Display for OpId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpId::Num(n) => write!(f, "{n}"),
            OpId::Text(s) => f.write_str(s),
        }
    }
}

/// How serious a `ui.notice` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// `doc.capture.source`: the whole composite, the active layer, the selection, or one layer by id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CaptureSource {
    Named(CaptureSourceName),
    Layer { layer: u64 },
}

/// The named capture sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureSourceName {
    /// Merged visible pixels.
    Composite,
    /// The active layer's own pixels.
    Layer,
    /// The selection as grayscale coverage.
    Selection,
}

/// `doc.capture.rect`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CaptureRect {
    /// `"canvas"` or `"selection"` (the selection's bounds).
    Whole(WholeRect),
    /// The selection's bounds grown by `pad`, for inpaint context.
    Padded { selection: Padded },
    /// An explicit rectangle, clamped to the canvas by the host.
    Rect { x: f64, y: f64, width: f64, height: f64 },
}

/// The named rectangles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WholeRect {
    Canvas,
    Selection,
}

/// `{"pad": px}`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Padded {
    #[serde(default)]
    pub pad: f64,
}

/// `doc.capture.alpha`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureAlpha {
    /// The source's own alpha.
    #[default]
    Source,
    /// Fully opaque.
    None,
    /// The selection coverage.
    Selection,
    /// One minus the selection coverage.
    SelectionInverted,
}

/// A layer in a `doc.commit`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitLayer {
    /// The PNG or JPEG to decode, already held by the host.
    pub blob: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    /// Resample to this size first (results come back at the model's size).
    #[serde(default)]
    pub scale_to: Option<Size>,
    #[serde(default)]
    pub mask: Option<CommitMask>,
    #[serde(default)]
    pub above: Above,
}

/// A width and height in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// Where a committed layer's mask comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommitMask {
    /// The current selection, as a single string.
    Selection(MaskSource),
    /// A grayscale PNG the same size as the layer after `scaleTo`.
    Blob { blob: String },
}

/// The one named mask source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaskSource {
    Selection,
}

/// Where a committed layer goes in the stack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Above {
    /// Directly above the active layer.
    #[default]
    Active,
    /// At the top of the stack.
    Top,
}

/// The methods an `http.request` may use. Spelled as HTTP spells them: the spec's example says `POST`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

/// What an `http.request` sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HttpBody {
    Json { json: Value },
    Text { text: String },
    Blob { blob: String },
    Multipart { multipart: Vec<MultipartPart> },
}

/// One part of a multipart body. The host builds the boundary, filenames and content types.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultipartPart {
    pub name: String,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub blob: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

/// How an `http.request` wants its response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HttpResponse {
    None,
    Json,
    Text,
    Blob,
}

/// One thing a panel asks the host to do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all_fields = "camelCase")]
pub enum Operation {
    /// Draw this widget tree; it replaces the previous one.
    #[serde(rename = "ui.view")]
    UiView { view: Widget },
    /// Change input values without replacing the view.
    #[serde(rename = "ui.set")]
    UiSet { values: Map<String, Value> },
    /// Drive the status-bar job UI and its Cancel button.
    #[serde(rename = "ui.progress")]
    UiProgress {
        #[serde(default)]
        fraction: Option<f64>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        cancellable: bool,
        #[serde(default)]
        done: bool,
    },
    /// A host notice, as `notices` in `ui.inspect`.
    #[serde(rename = "ui.notice")]
    UiNotice { level: NoticeLevel, text: String },
    /// Get a `result` event `ms` from now.
    #[serde(rename = "timer.set")]
    TimerSet { id: String, ms: u64 },
    #[serde(rename = "timer.clear")]
    TimerClear { id: String },
    /// Write to the plug-in's own key/value store; `null` deletes.
    #[serde(rename = "storage.set")]
    StorageSet { key: String, value: Value },
    /// Ask about the active document. Needs `document: ["read"]`.
    #[serde(rename = "doc.info")]
    DocInfo { id: OpId },
    /// Run one engine command, by id. The host runs only the commands its own panel policy allows,
    /// and only when the manifest asks for the document capability that command needs; the manifest
    /// is a request, never a grant. Needs `document: ["read"]` or `["read","addLayers"]`.
    #[serde(rename = "doc.command")]
    DocCommand {
        id: OpId,
        command: String,
        #[serde(default)]
        params: Value,
    },
    /// Capture pixels as a PNG blob. Needs `document: ["read"]`.
    #[serde(rename = "doc.capture")]
    DocCapture {
        id: OpId,
        source: CaptureSource,
        rect: CaptureRect,
        #[serde(default)]
        alpha: CaptureAlpha,
        #[serde(default)]
        max_side: Option<u32>,
        #[serde(default)]
        bits: Option<u8>,
    },
    /// Add layers in one undo step. Needs `document: ["addLayers"]`.
    #[serde(rename = "doc.commit")]
    DocCommit {
        id: OpId,
        doc: u64,
        label: String,
        layers: Vec<CommitLayer>,
        #[serde(default)]
        if_revision: Option<u64>,
    },
    /// A loopback HTTP request. Needs `capabilities.network`.
    #[serde(rename = "http.request")]
    HttpRequest {
        id: OpId,
        method: HttpMethod,
        url: String,
        #[serde(default)]
        headers: Map<String, Value>,
        #[serde(default)]
        body: Option<HttpBody>,
        response: HttpResponse,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Release a blob.
    #[serde(rename = "blob.free")]
    BlobFree { blob: String },
}

impl Operation {
    /// The id this operation's `result` event will carry, if it gets one.
    ///
    /// Timers are absent on purpose: their id names the `timer` event they later fire, not a result.
    pub fn id(&self) -> Option<&OpId> {
        match self {
            Operation::DocInfo { id }
            | Operation::DocCommand { id, .. }
            | Operation::DocCapture { id, .. }
            | Operation::DocCommit { id, .. }
            | Operation::HttpRequest { id, .. } => Some(id),
            _ => None,
        }
    }

    /// The operation's name, as it appears in the manifest and in error messages.
    pub fn name(&self) -> &'static str {
        match self {
            Operation::UiView { .. } => "ui.view",
            Operation::UiSet { .. } => "ui.set",
            Operation::UiProgress { .. } => "ui.progress",
            Operation::UiNotice { .. } => "ui.notice",
            Operation::TimerSet { .. } => "timer.set",
            Operation::TimerClear { .. } => "timer.clear",
            Operation::StorageSet { .. } => "storage.set",
            Operation::DocInfo { .. } => "doc.info",
            Operation::DocCommand { .. } => "doc.command",
            Operation::DocCapture { .. } => "doc.capture",
            Operation::DocCommit { .. } => "doc.commit",
            Operation::HttpRequest { .. } => "http.request",
            Operation::BlobFree { .. } => "blob.free",
        }
    }

    /// Checks what the types cannot express: ranges, lengths, ids, blob names, loopback URLs.
    pub fn validate(&self) -> Result<()> {
        match self {
            Operation::UiView { view } => view::check(view),
            Operation::UiSet { values } => {
                for key in values.keys() {
                    if !view::valid_id(key) {
                        return Err(Error::Ops(format!("`ui.set` names {key:?}, which is not a widget id")));
                    }
                }
                Ok(())
            }
            Operation::UiProgress { fraction, text, .. } => {
                if fraction.is_some_and(|f| !f.is_finite() || !(0.0..=1.0).contains(&f)) {
                    return Err(Error::Ops("`ui.progress` needs a `fraction` between 0 and 1, or null".into()));
                }
                bounded(text.as_deref(), "`ui.progress` text")
            }
            Operation::UiNotice { text, .. } => {
                if text.trim().is_empty() {
                    return Err(Error::Ops("`ui.notice` needs text".into()));
                }
                bounded(Some(text), "`ui.notice` text")
            }
            Operation::TimerSet { id, ms } => {
                named(id, "timer")?;
                if !(MIN_TIMER_MS..=MAX_TIMER_MS).contains(ms) {
                    return Err(Error::Ops(format!("`timer.set` asks for {ms} ms, outside {MIN_TIMER_MS}..={MAX_TIMER_MS}")));
                }
                Ok(())
            }
            Operation::TimerClear { id } => named(id, "timer"),
            Operation::StorageSet { key, .. } => named(key, "storage key"),
            Operation::DocInfo { .. } => Ok(()),
            Operation::DocCommand { command, params, .. } => {
                command_id(command)?;
                if !(params.is_object() || params.is_null()) {
                    return Err(Error::Ops(format!("`doc.command` params for {command} must be a JSON object")));
                }
                // The reply itself is bounded (`MAX_OPS_BYTES`); this caps what one command is told
                // to do independently of that, and the host sends the same bytes on.
                if serde_json::to_vec(params).map(|b| b.len()).unwrap_or(usize::MAX) > MAX_COMMAND_PARAMS_BYTES {
                    return Err(Error::Limit(format!("`doc.command` params for {command} are larger than {MAX_COMMAND_PARAMS_BYTES} bytes")));
                }
                Ok(())
            }
            Operation::DocCapture { source, rect, max_side, bits, .. } => {
                if max_side.is_some_and(|s| s == 0 || s > MAX_CAPTURE_SIDE) {
                    return Err(Error::Ops(format!("`doc.capture` `maxSide` must be 1..={MAX_CAPTURE_SIDE}")));
                }
                if bits.is_some_and(|b| b != 8 && b != 16) {
                    return Err(Error::Ops("`doc.capture` `bits` must be 8 or 16".into()));
                }
                match rect {
                    CaptureRect::Rect { x, y, width, height } => {
                        if ![x, y, width, height].iter().all(|v| v.is_finite()) || *width <= 0.0 || *height <= 0.0 {
                            return Err(Error::Ops("`doc.capture` needs a finite rect with a positive width and height".into()));
                        }
                    }
                    CaptureRect::Padded { selection } => {
                        if !selection.pad.is_finite() || selection.pad < 0.0 || selection.pad > MAX_CAPTURE_SIDE as f64 {
                            return Err(Error::Ops(format!("`doc.capture` `pad` must be between 0 and {MAX_CAPTURE_SIDE}")));
                        }
                    }
                    CaptureRect::Whole(_) => {}
                }
                if let CaptureSource::Layer { layer } = source
                    && *layer == 0
                {
                    return Err(Error::Ops("`doc.capture` `{\"layer\": 0}` is not a layer id".into()));
                }
                Ok(())
            }
            Operation::DocCommit { doc, label, layers, .. } => {
                if *doc == 0 {
                    return Err(Error::Ops("`doc.commit` needs the document id it started from".into()));
                }
                if label.trim().is_empty() || label.chars().count() > 128 {
                    return Err(Error::Ops("`doc.commit` needs a `label` of 1-128 characters".into()));
                }
                if layers.is_empty() || layers.len() > MAX_COMMIT_LAYERS {
                    return Err(Error::Ops(format!("`doc.commit` takes 1-{MAX_COMMIT_LAYERS} layers")));
                }
                for layer in layers {
                    if !crate::blobs::valid_id(&layer.blob) {
                        return Err(Error::Ops(format!("`doc.commit` names {:?}, which is not a blob id", layer.blob)));
                    }
                    if !layer.x.is_finite() || !layer.y.is_finite() {
                        return Err(Error::Ops("`doc.commit` needs finite layer positions".into()));
                    }
                    if layer.name.chars().count() > 64 {
                        return Err(Error::Ops("`doc.commit` layer names are at most 64 characters".into()));
                    }
                    if layer.scale_to.is_some_and(|s| s.width == 0 || s.height == 0 || s.width > MAX_COMMIT_SIDE || s.height > MAX_COMMIT_SIDE) {
                        return Err(Error::Ops(format!("`doc.commit` `scaleTo` must be 1..={MAX_COMMIT_SIDE} on each side")));
                    }
                    if let Some(CommitMask::Blob { blob }) = &layer.mask
                        && !crate::blobs::valid_id(blob)
                    {
                        return Err(Error::Ops(format!("`doc.commit` mask names {blob:?}, which is not a blob id")));
                    }
                }
                Ok(())
            }
            Operation::HttpRequest { url, headers, body, timeout_ms, .. } => {
                if !loopback_url(url) {
                    return Err(Error::Ops(format!("`http.request` url {url:?} must be an http:// loopback address")));
                }
                for (name, value) in headers {
                    let ok = matches!(name.to_ascii_lowercase().as_str(), "content-type" | "accept" | "authorization");
                    if !ok || !value.is_string() {
                        return Err(Error::Ops(format!("`http.request` may only set Content-Type, Accept and Authorization (not {name:?})")));
                    }
                }
                if timeout_ms.is_some_and(|t| t == 0 || t > MAX_HTTP_TIMEOUT_MS) {
                    return Err(Error::Ops(format!("`http.request` `timeoutMs` must be 1..={MAX_HTTP_TIMEOUT_MS}")));
                }
                match body {
                    Some(HttpBody::Blob { blob }) if !crate::blobs::valid_id(blob) => {
                        return Err(Error::Ops(format!("`http.request` body names {blob:?}, which is not a blob id")));
                    }
                    Some(HttpBody::Multipart { multipart }) => {
                        if multipart.is_empty() || multipart.len() > 64 {
                            return Err(Error::Ops("`http.request` multipart takes 1-64 parts".into()));
                        }
                        for part in multipart {
                            if part.name.is_empty() || part.name.len() > 256 {
                                return Err(Error::Ops("a multipart part needs a name of 1-256 bytes".into()));
                            }
                            match (&part.blob, &part.text) {
                                (Some(id), None) if crate::blobs::valid_id(id) => {}
                                (None, Some(_)) => {}
                                _ => return Err(Error::Ops(format!("multipart part {:?} needs exactly one of `blob` or `text`", part.name))),
                            }
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
            Operation::BlobFree { blob } => {
                if !crate::blobs::valid_id(blob) {
                    return Err(Error::Ops(format!("`blob.free` names {blob:?}, which is not a blob id")));
                }
                Ok(())
            }
        }
    }
}

/// Parses the operations array a `pc_event` returned and checks every operation. An empty (or
/// whitespace-only) reply means "no operations".
pub fn parse(bytes: &[u8]) -> Result<Vec<Operation>> {
    if bytes.len() > MAX_OPS_BYTES {
        return Err(Error::Limit(format!("operations are {} bytes (limit {MAX_OPS_BYTES})", bytes.len())));
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    let ops: Vec<Operation> = serde_json::from_slice(bytes).map_err(|e| Error::Ops(format!("operations: {e} in {:?}", preview(bytes))))?;
    if ops.len() > MAX_OPS {
        return Err(Error::Limit(format!("{} operations (limit {MAX_OPS})", ops.len())));
    }
    for op in &ops {
        op.validate()?;
    }
    Ok(ops)
}

/// A short, printable preview of a reply, so the author of a panel that returns bad JSON has
/// something to look at. Bytes are shown as they decode, controls dropped.
fn preview(bytes: &[u8]) -> String {
    const SHOWN: usize = 200;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(SHOWN)]);
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    if bytes.len() > SHOWN { format!("{text}…") } else { text }
}

/// `http://` and a loopback authority, with or without a port. The granted-host check is the host's.
fn loopback_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    crate::manifest::valid_network_host(host)
}

/// A timer or storage key.
fn named(id: &str, what: &str) -> Result<()> {
    if view::valid_id(id) { Ok(()) } else { Err(Error::Ops(format!("`{what}` {id:?} must be 1-64 characters from A-Z a-z 0-9 . _ -"))) }
}

/// A command id, as the engine's registry spells them: `type.create`, `layer.align.leftEdges`.
///
/// Checked here so a typo is a clear error rather than an "unknown command" from the host, and so
/// the host only ever matches against ids of this shape.
fn command_id(id: &str) -> Result<()> {
    let ok = (1..=64).contains(&id.len())
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && !id.ends_with('.')
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if ok { Ok(()) } else { Err(Error::Ops(format!("`doc.command` names {id:?}, which is not a command id (see `plugin.list` for ids)"))) }
}

fn bounded(text: Option<&str>, what: &str) -> Result<()> {
    if text.is_some_and(|t| t.chars().count() > MAX_TEXT) {
        return Err(Error::Ops(format!("{what} is longer than {MAX_TEXT} characters")));
    }
    Ok(())
}

/// An event, as the host sends it: `{"type": …, "now": <ms since the Unix epoch>, …}`.
///
/// `now` is the only clock a plug-in gets, and `entropy` in `init` the only randomness (seed it
/// with a PRNG): v2 has no clock or randomness import, as v1.
pub fn event(kind: &str, now_ms: u64, fields: Value) -> Value {
    let mut map = match fields {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    map.insert("type".into(), json!(kind));
    map.insert("now".into(), json!(now_ms));
    Value::Object(map)
}

/// The `init` event: host facts, locale, the plug-in's stored values, 32 bytes of hex entropy, and
/// the capabilities the user granted.
pub fn init(now_ms: u64, host: Value, locale: &str, storage: &Map<String, Value>, entropy: &str, grants: &Value) -> Value {
    event("init", now_ms, json!({"host": host, "locale": locale, "storage": storage, "entropy": entropy, "grants": grants}))
}

/// A button press: the pressed id plus every input's current value.
pub fn ui_click(now_ms: u64, id: &str, values: &Map<String, Value>) -> Value {
    event("ui.click", now_ms, json!({"id": id, "values": values}))
}

/// A committed input: Enter, focus lost, slider released, choice picked. Not sent per keystroke.
pub fn ui_change(now_ms: u64, id: &str, value: &Value, values: &Map<String, Value>) -> Value {
    event("ui.change", now_ms, json!({"id": id, "value": value, "values": values}))
}

/// The panel was shown or hidden; a plug-in may pause polling while hidden.
pub fn panel_visible(now_ms: u64, visible: bool) -> Value {
    event("panel.visible", now_ms, json!({"visible": visible}))
}

/// The active document changed or was closed (`doc` is `null` then).
pub fn doc_changed(now_ms: u64, doc: &Value, revision: u64) -> Value {
    event("doc.changed", now_ms, json!({"doc": doc, "revision": revision}))
}

/// An operation with an `id` finished. `fields` carries its result, or `error`.
pub fn result(now_ms: u64, id: &OpId, ok: bool, fields: Value) -> Value {
    let mut map = match event("result", now_ms, fields) {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    map.insert("id".into(), json!(id));
    map.insert("ok".into(), json!(ok));
    Value::Object(map)
}

/// The user pressed Cancel on the plug-in's progress.
pub fn cancel(now_ms: u64) -> Value {
    event("cancel", now_ms, json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(s: &str) -> Result<Vec<Operation>> {
        parse(s.as_bytes())
    }

    #[test]
    fn parses_the_spec_commit_example() {
        // `doc.commit` from `docs/plugins-v2.md`.
        let ops = parse_str(
            r#"[{"op": "doc.commit", "id": "c1", "doc": 3, "label": "OpenLayer: Inpaint",
                "layers": [{"blob": "b:21", "name": "OpenLayer_Inpaint", "x": 812, "y": 440,
                            "scaleTo": {"width": 512, "height": 512},
                            "mask": {"blob": "b:19"}, "above": "active"}]}]"#,
        )
        .unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].name(), "doc.commit");
        assert_eq!(ops[0].id(), Some(&OpId::Text("c1".into())));
        let Operation::DocCommit { doc, layers, if_revision, .. } = &ops[0] else { return };
        assert_eq!((*doc, layers.len(), *if_revision), (3, 1, None));
        assert_eq!(layers[0].scale_to, Some(Size { width: 512, height: 512 }));
        assert_eq!(layers[0].mask, Some(CommitMask::Blob { blob: "b:19".into() }));
        assert_eq!(layers[0].above, Above::Active);
    }

    #[test]
    fn parses_the_spec_request_example() {
        let ops = parse_str(
            r#"[{"op": "http.request", "id": "up1", "method": "POST", "url": "http://127.0.0.1:8188/upload/image",
                "body": {"multipart": [
                  {"name": "image", "filename": "ol_src_3f2a.png", "contentType": "image/png", "blob": "b:18"},
                  {"name": "type", "text": "input"}, {"name": "overwrite", "text": "true"}]},
                "response": "json", "timeoutMs": 60000}]"#,
        )
        .unwrap();
        let Some(HttpBody::Multipart { multipart }) = (match &ops[0] {
            Operation::HttpRequest { body, .. } => body.clone(),
            _ => None,
        }) else {
            return;
        };
        assert_eq!(multipart.len(), 3);
        assert_eq!(multipart[0].content_type.as_deref(), Some("image/png"));
        assert_eq!(multipart[1].text.as_deref(), Some("input"));
    }

    #[test]
    fn an_empty_reply_means_no_operations() {
        assert!(parse(b"").unwrap().is_empty());
        assert!(parse(b"  \n ").unwrap().is_empty());
        assert!(parse(b"[]").unwrap().is_empty());
    }

    #[test]
    fn rejects_malformed_operations() {
        // An empty reply is not malformed: it means "no operations", and is asserted below.
        for (i, bad) in [
            "not json",
            "{}",
            r#"{"op": "teleport"}"#,
            // Unknown op tag.
            r#"[{"op": "ui.viewx"}]"#,
            // Notice without text, or an unknown level.
            r#"[{"op": "ui.notice", "level": "info", "text": "  "}]"#,
            r#"[{"op": "ui.notice", "level": "shout", "text": "x"}]"#,
            // Timers out of range, bad ids.
            r#"[{"op": "timer.set", "id": "t", "ms": 5}]"#,
            r#"[{"op": "timer.set", "id": "t", "ms": 700000}]"#,
            r#"[{"op": "timer.clear", "id": "no spaces"}]"#,
            r#"[{"op": "storage.set", "key": "a b", "value": 1}]"#,
            // Progress outside 0-1.
            r#"[{"op": "ui.progress", "fraction": -0.5}]"#,
            // Blob names that are not blobs.
            r#"[{"op": "blob.free", "blob": "17"}]"#,
            r#"[{"op": "doc.commit", "id": "c", "doc": 1, "label": "L", "layers": [{"blob": "17"}]}]"#,
            r#"[{"op": "doc.commit", "id": "c", "doc": 0, "label": "L", "layers": [{"blob": "b:1"}]}]"#,
            r#"[{"op": "doc.commit", "id": "c", "doc": 1, "label": "", "layers": [{"blob": "b:1"}]}]"#,
            r#"[{"op": "doc.commit", "id": "c", "doc": 1, "label": "L", "layers": []}]"#,
            r#"[{"op": "doc.commit", "id": "c", "doc": 1, "label": "L", "layers": [{"blob": "b:1", "scaleTo": {"width": 0, "height": 8}}]}]"#,
            // A capture rect with no area, or a bad size.
            r#"[{"op": "doc.capture", "id": "s", "source": "composite", "rect": {"x": 0, "y": 0, "width": 0, "height": 10}}]"#,
            r#"[{"op": "doc.capture", "id": "s", "source": "composite", "rect": "canvas", "maxSide": 20000}]"#,
            r#"[{"op": "doc.capture", "id": "s", "source": "composite", "rect": "canvas", "bits": 12}]"#,
            r#"[{"op": "doc.capture", "id": "s", "source": {"layer": 0}, "rect": "canvas"}]"#,
            // The network rules: not loopback, or a header the host owns.
            r#"[{"op": "http.request", "id": "h", "method": "GET", "url": "http://example.com/x", "response": "json"}]"#,
            r#"[{"op": "http.request", "id": "h", "method": "GET", "url": "https://127.0.0.1/x", "response": "json"}]"#,
            r#"[{"op": "http.request", "id": "h", "method": "GET", "url": "http://127.0.0.1/x", "response": "json", "headers": {"Host": "evil"}}]"#,
            r#"[{"op": "http.request", "id": "h", "method": "GET", "url": "http://127.0.0.1/x", "response": "json", "timeoutMs": 9999999}]"#,
            r#"[{"op": "http.request", "id": "h", "method": "PATCH", "url": "http://127.0.0.1/x", "response": "json"}]"#,
            // Multipart parts need exactly one of blob/text.
            r#"[{"op": "http.request", "id": "h", "method": "POST", "url": "http://127.0.0.1/x", "response": "json",
                "body": {"multipart": [{"name": "a", "blob": "b:1", "text": "both"}]}}]"#,
            r#"[{"op": "http.request", "id": "h", "method": "POST", "url": "http://127.0.0.1/x", "response": "json",
                "body": {"multipart": [{"name": "a"}]}}]"#,
        ]
        .into_iter()
        .enumerate()
        {
            assert!(parse_str(bad).is_err(), "case {i}: {bad}");
        }
        assert!(parse(&vec![b'x'; MAX_OPS_BYTES + 1]).is_err());
        // One operation past the cap.
        let many = format!("[{}]", vec![r#"{"op":"ui.notice","level":"info","text":"x"}"#; MAX_OPS + 1].join(","));
        assert!(matches!(parse_str(&many), Err(Error::Limit(_))));
        // An empty reply is "no operations", not an error, however much whitespace it holds.
        assert_eq!(parse(b"").unwrap(), Vec::new());
        assert_eq!(parse(b"  \n\t ").unwrap(), Vec::new());
    }

    #[test]
    fn a_capture_rect_is_a_name_or_a_rectangle() {
        let rect = |v: &str| {
            let json = format!(r#"[{{"op": "doc.capture", "id": "s", "source": "selection", "rect": {v}}}]"#);
            let ops = parse_str(&json).unwrap();
            match &ops[0] {
                Operation::DocCapture { rect, .. } => rect.clone(),
                other => panic!("expected `doc.capture`, got {other:?}"),
            }
        };
        assert_eq!(rect(r#""canvas""#), CaptureRect::Whole(WholeRect::Canvas));
        assert_eq!(rect(r#""selection""#), CaptureRect::Whole(WholeRect::Selection));
        assert_eq!(rect(r#"{"selection": {"pad": 8}}"#), CaptureRect::Padded { selection: Padded { pad: 8.0 } });
        assert_eq!(rect(r#"{"selection": {}}"#), CaptureRect::Padded { selection: Padded { pad: 0.0 } });
        assert_eq!(rect(r#"{"x": 1, "y": 2, "width": 3, "height": 4}"#), CaptureRect::Rect { x: 1.0, y: 2.0, width: 3.0, height: 4.0 });
        // Alpha defaults to the source's own, and `bits` to 8.
        let ops = parse_str(r#"[{"op": "doc.capture", "id": 7, "source": "composite", "rect": "canvas"}]"#).unwrap();
        let Operation::DocCapture { alpha, bits, id, .. } = &ops[0] else { return };
        assert_eq!((*alpha, *bits, id), (CaptureAlpha::Source, None, &OpId::Num(7)));
    }

    #[test]
    fn an_id_may_be_a_string_or_a_number() {
        let ops = parse_str(r#"[{"op": "doc.info", "id": 12}, {"op": "doc.info", "id": "twelve"}]"#).unwrap();
        assert_eq!(ops[0].id(), Some(&OpId::Num(12)));
        assert_eq!(ops[1].id(), Some(&OpId::Text("twelve".into())));
        // Operations without an id get no `result` event.
        let ops = parse_str(r#"[{"op": "ui.progress", "fraction": 0.5}, {"op": "blob.free", "blob": "b:1"}]"#).unwrap();
        assert!(ops.iter().all(|op| op.id().is_none()));
    }

    #[test]
    fn events_carry_the_type_and_the_only_clock() {
        let e = init(
            1_700_000_000_000,
            json!({"app": "PhotoCraft", "version": "0.9.0", "abi": 2, "platform": "linux"}),
            "en",
            &Map::new(),
            "ab",
            &json!({"document": ["read"]}),
        );
        assert_eq!(e["type"], "init");
        assert_eq!(e["now"], 1_700_000_000_000u64);
        assert_eq!(e["host"]["app"], "PhotoCraft");
        assert_eq!(e["locale"], "en");
        assert_eq!(e["grants"]["document"][0], "read");

        let mut values = Map::new();
        values.insert("prefix".into(), json!("/A"));
        let click = ui_click(9, "insert", &values);
        assert_eq!((click["type"].clone(), click["id"].clone(), click["now"].clone()), (json!("ui.click"), json!("insert"), json!(9)));
        assert_eq!(click["values"]["prefix"], "/A");

        let change = ui_change(9, "size", &json!(12), &values);
        assert_eq!(change["value"], 12);
        assert_eq!(panel_visible(9, false)["visible"], false);
        assert_eq!(doc_changed(9, &Value::Null, 4)["doc"], Value::Null);
        assert_eq!(doc_changed(9, &json!(3), 4)["revision"], 4);
        assert_eq!(cancel(9)["type"], "cancel");

        let r = result(9, &OpId::Text("c1".into()), false, json!({"error": "no document"}));
        assert_eq!(r["type"], "result");
        assert_eq!((r["id"].clone(), r["ok"].clone(), r["error"].clone()), (json!("c1"), json!(false), json!("no document")));
    }

    #[test]
    fn only_loopback_urls_pass() {
        for good in ["http://127.0.0.1:8188/x", "http://localhost/", "http://[::1]:3000/a?b=c", "http://127.0.0.1:*"] {
            assert!(loopback_url(good), "{good}");
        }
        for bad in ["https://127.0.0.1/x", "http://example.com", "http://127.0.0.1.evil.com/x", "ftp://127.0.0.1", "http://10.0.0.1:80/x", "127.0.0.1/x"] {
            assert!(!loopback_url(bad), "{bad}");
        }
    }
}
