//! Host-neutral, versioned data contract for community compatibility adapters.
//!
//! This module is **separate from the no-imports WebAssembly ABI v1**. An adapter is trusted
//! in-process Rust code, not a sandbox, a stable FFI ABI, or an implemented IPC protocol. This
//! module loads no executables, libraries or SDKs and performs no filesystem or shell access.
//! A community adapter can interpret an original resource without the host rewriting it; that
//! does not mean that PhotoCraft can itself host Photoshop/Affinity filters or unwrap neural
//! packages. No proprietary SDK is included.
//!
//! Register an `Arc<dyn Adapter>` in an instance of [`Registry`]. Registration reads and validates
//! [`Adapter::descriptor`] once and caches it. [`Registry::get`] returns an owned, immutable
//! registration snapshot suitable for a background task. [`RegisteredAdapter::run`] validates
//! both sides of the call, applies device policy, and catches unwinding adapter panics. It cannot
//! stop hung code, recover from an abort/OOM, or verify physical GPU/NPU execution. Device and
//! backend metadata are adapter assertions, **not hardware attestation**. Cancellation is
//! cooperative and checked before and after the call; adapters must poll [`Context::cancelled`].
//!
//! All public data types use Serde's field names as written here; enums use `snake_case`.
//! Validation must precede serialization of caller-built values. In particular, JSON validation
//! is iterative and bounded, and does not recursively serialize an unchecked `Value`. Serde
//! derives are a data convenience, not an untrusted-byte decoder with allocation limits: a
//! transport must bound input bytes and deserialize before invoking this validated Rust API.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Version of this Rust data protocol, independent of the WebAssembly `ABI_VERSION`.
pub const API_VERSION: u32 = 1;
/// An ICC blob may never exceed 4 MiB, even with custom [`Limits`].
pub const MAX_ICC_BYTES: usize = 4 * 1024 * 1024;
const MAX_CAPABILITIES: usize = 128;
const MAX_TEXT_BYTES: usize = 1024;
const MAX_ID_BYTES: usize = 128;
const MAX_JSON_DEPTH: usize = 64;

/// Errors are local to the adapter protocol and do not change the WebAssembly plug-in errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("adapter protocol version {actual} is unsupported (expected {expected})")]
    ApiVersion { expected: u32, actual: u32 },
    #[error("invalid adapter descriptor: {0}")]
    InvalidDescriptor(String),
    #[error("adapter {0:?} is already registered")]
    DuplicateAdapter(String),
    #[error("adapter {0:?} is not registered")]
    NotFound(String),
    #[error("unsupported adapter operation {0:?}")]
    UnsupportedOperation(String),
    #[error("unsupported adapter image color mode {0:?}")]
    UnsupportedColorMode(String),
    #[error("unknown adapter device {0:?}")]
    UnknownDevice(String),
    #[error("adapter device policy: {0}")]
    Device(String),
    #[error("invalid adapter image: {0}")]
    InvalidImage(String),
    #[error("invalid adapter selection: {0}")]
    InvalidSelection(String),
    #[error("invalid adapter parameters: {0}")]
    InvalidParams(String),
    #[error("invalid adapter response: {0}")]
    InvalidResponse(String),
    #[error("adapter limit exceeded: {0}")]
    Limit(String),
    #[error("adapter run canceled")]
    Canceled,
    #[error("adapter panicked while unwinding")]
    Panicked,
    #[error("adapter failed: {0}")]
    Failed(String),
}

/// The adapter protocol's result type.
pub type Result<T> = std::result::Result<T, Error>;

/// The only output kinds in protocol v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Image,
    Mask,
    Depth,
}

/// Adapter-declared device category, not a probe of the host's physical hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Cpu,
    Gpu,
    Npu,
}

/// One operation; accepted color modes are exact, case-sensitive string identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub output_kind: OutputKind,
    /// For example `rgb`, `grayscale`, `cmyk`, `lab`, or an adapter-defined mode.
    pub color_modes: Vec<String>,
}

/// A stable adapter-local device id and its claimed operation/backend support.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub kind: DeviceKind,
    pub backend: String,
    pub name: String,
    /// Operation ids from the enclosing descriptor; must not be empty.
    pub operations: Vec<String>,
}

/// Immutable capabilities, validated and cached at registration.
///
/// Adapter/operation/device/backend/color-mode ids are 1..=128 ASCII bytes from
/// `[A-Za-z0-9._-]`. Names, versions and resource-format labels are nonblank, contain no
/// control characters, and occupy at most 1024 UTF-8 bytes. Lists have at most 128 entries;
/// operations/devices, each operation's modes and each device's operations are nonempty.
/// Ids/modes/formats cannot repeat, all device operation references must exist, and every
/// operation must have at least one supporting device. Schema limits are [`Limits::default`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Descriptor {
    pub api_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub operations: Vec<Operation>,
    pub devices: Vec<Device>,
    /// Original-format labels for discovery only. No path, suffix or resource parsing occurs.
    #[serde(default)]
    pub resource_formats: Vec<String>,
    /// Opaque JSON object: bounded for safety, not interpreted as a particular schema dialect.
    #[serde(default = "empty_object")]
    pub params_schema: Value,
}

/// Interleaved, row-major `f32` samples with straight (not premultiplied) alpha last.
///
/// `x`/`y` are the buffer's document-space origin, including negative coordinates. Width and
/// height must fit `i32`, and the exclusive `x + width`/`y + height` endpoints must not overflow.
/// Depth is source metadata: samples are always floats. At depth 8/16 all samples are normalized to
/// `0..=1`; at depth 32 finite color samples may be negative or HDR. Alpha is always `0..=1`.
/// No color conversion, profile substitution, channel inference or premultiplication occurs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub color_mode: String,
    pub source_depth: u8,
    pub alpha: bool,
    pub icc_profile: Vec<u8>,
    pub data: Vec<f32>,
}

/// One operation invocation. `resource` is passed byte-for-byte to the adapter, never resolved,
/// opened, normalized, trimmed, inspected for a format, or rewritten by this module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub operation: String,
    #[serde(default)]
    pub resource: Option<String>,
    /// Must be an object; validated for bounded size/depth, not against `params_schema`.
    #[serde(default = "empty_object")]
    pub params: Value,
    /// Explicit adapter-local device id. No alias such as `gpu` is inferred by the host.
    #[serde(default)]
    pub device: Option<String>,
    /// False by default. A changed/mixed explicit device set requires true and a reason.
    #[serde(default)]
    pub allow_fallback: bool,
    pub image: Image,
    /// Optional same-geometry, alpha-free, profile-free one-channel gray `0..=1` selection.
    /// This is analysis/seed context, not permission to preblend an image result with its input:
    /// an image-output host applies this coverage once. Return the uncomposited candidate image.
    /// Mask results are authoritative scalar data, not automatically multiplied by this selection.
    /// This protocol module validates the data but does not perform either host writeback.
    #[serde(default)]
    pub selection: Option<Image>,
}

/// Declared output kind and its image data. Mask/depth outputs must be alpha-free,
/// one-channel `gray`/`grayscale`, empty ICC profile, depth 32, and have the input geometry.
/// Masks are `0..=1`; depth values may be any finite floats. Image outputs preserve all input
/// format metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Output {
    pub kind: OutputKind,
    pub image: Image,
}

/// Adapter-reported execution, not independent verification of physical execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    /// Nonempty, unique ids from the descriptor, each supporting the requested operation.
    pub devices: Vec<String>,
    /// Must equal the declared backend when all devices share it; otherwise exactly `mixed`.
    pub backend: String,
    /// Required if an explicit requested device differs from the actual set. A reason is
    /// forbidden when fallback was not allowed, or when that explicit set did not change.
    #[serde(default)]
    pub fallback_reason: Option<String>,
}

/// A successfully validated output plus adapter-reported execution metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub output: Output,
    pub execution: Execution,
}

/// Cooperative callbacks. Implementations should be cheap and non-panicking; a background
/// caller should provide a thread-safe implementation. No session/document/runtime type leaks
/// through this interface. Adapter progress is forwarded only if finite and clamped to `0..=1`.
pub trait Context {
    fn cancelled(&self) -> bool;
    fn progress(&self, fraction: f32);
}

/// Trusted in-process extension point. `descriptor` is read exactly once per registration.
/// Return [`Error::Failed`] for adapter-specific failures and poll cancellation during work.
pub trait Adapter: Send + Sync {
    fn descriptor(&self) -> Descriptor;
    fn run(&self, request: &Request, ctx: &dyn Context) -> Result<Response>;
}

/// Per-invocation budgets. These are validation/allocation-preflight limits, not accounting
/// for memory allocated internally by trusted adapter code. The ICC hard cap and a maximum
/// JSON depth of 64 still apply if a caller raises the corresponding fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub max_pixels: usize,
    pub max_samples: usize,
    pub max_icc_bytes: usize,
    pub max_resource_bytes: usize,
    pub max_json_depth: usize,
    pub max_json_nodes: usize,
    pub max_json_bytes: usize,
    pub max_json_object_entries: usize,
    pub max_json_array_items: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_pixels: 32 * 1024 * 1024,
            max_samples: 128 * 1024 * 1024,
            max_icc_bytes: MAX_ICC_BYTES,
            max_resource_bytes: 64 * 1024,
            max_json_depth: 32,
            max_json_nodes: 16 * 1024,
            max_json_bytes: 1024 * 1024,
            max_json_object_entries: 1024,
            max_json_array_items: 16 * 1024,
        }
    }
}

impl Limits {
    /// Validate a parameter object using the same bounded, iterative JSON checks as execution.
    /// Hosts should call this before cloning caller-built metadata into a worker snapshot.
    pub fn check_params(&self, params: &Value) -> Result<()> {
        validate_json_object(params, self)
    }

    /// Check dimensions and all pixel/sample/byte multiplications **before allocating** an
    /// image buffer. Returns the exact sample count; zero-sized buffers and channels outside
    /// `1..=64` are invalid. This helper works on both 32-bit and 64-bit hosts.
    pub fn check_image_size(&self, width: u32, height: u32, channels: u8) -> Result<usize> {
        if width == 0 || height == 0 || !(1..=64).contains(&channels) {
            return Err(Error::InvalidImage("nonzero dimensions and 1..=64 channels required".into()));
        }
        let width = usize::try_from(width).map_err(|_| Error::Limit("image width".into()))?;
        let height = usize::try_from(height).map_err(|_| Error::Limit("image height".into()))?;
        let pixels = width.checked_mul(height).ok_or_else(|| Error::Limit("pixel count overflow".into()))?;
        if pixels > self.max_pixels {
            return Err(Error::Limit("pixel count".into()));
        }
        let samples = pixels.checked_mul(usize::from(channels)).ok_or_else(|| Error::Limit("sample count overflow".into()))?;
        if samples > self.max_samples {
            return Err(Error::Limit("sample count".into()));
        }
        let bytes = samples.checked_mul(std::mem::size_of::<f32>()).ok_or_else(|| Error::Limit("sample byte count overflow".into()))?;
        if bytes > isize::MAX as usize {
            return Err(Error::Limit("image exceeds addressable allocation".into()));
        }
        Ok(samples)
    }
}

impl Image {
    /// Validate layout, finite/normalized samples, source-depth metadata and the ICC cap.
    /// Call [`Limits::check_image_size`] first when constructing a new allocation.
    pub fn validate(&self, limits: &Limits) -> Result<()> {
        let samples = limits.check_image_size(self.width, self.height, self.channels)?;
        let width = i32::try_from(self.width).map_err(|_| Error::InvalidImage("width exceeds coordinate range".into()))?;
        let height = i32::try_from(self.height).map_err(|_| Error::InvalidImage("height exceeds coordinate range".into()))?;
        self.x.checked_add(width).ok_or_else(|| Error::InvalidImage("x + width overflows coordinate range".into()))?;
        self.y.checked_add(height).ok_or_else(|| Error::InvalidImage("y + height overflows coordinate range".into()))?;
        if self.data.len() != samples {
            return Err(Error::InvalidImage("sample count does not match dimensions/channels".into()));
        }
        if !matches!(self.source_depth, 8 | 16 | 32) {
            return Err(Error::InvalidImage("source_depth must be 8, 16 or 32".into()));
        }
        if !valid_id(&self.color_mode) {
            return Err(Error::InvalidImage("invalid color_mode identifier".into()));
        }
        if self.alpha && self.channels < 2 {
            return Err(Error::InvalidImage("alpha requires a color channel".into()));
        }
        if self.icc_profile.len() > limits.max_icc_bytes.min(MAX_ICC_BYTES) {
            return Err(Error::Limit("ICC profile bytes".into()));
        }
        let channels = usize::from(self.channels);
        for (index, value) in self.data.iter().enumerate() {
            if !value.is_finite() {
                return Err(Error::InvalidImage("non-finite sample".into()));
            }
            let is_alpha = self.alpha && index % channels == channels - 1;
            if (self.source_depth != 32 || is_alpha) && !(0.0..=1.0).contains(value) {
                return Err(Error::InvalidImage("normalized or alpha sample outside 0..=1".into()));
            }
        }
        Ok(())
    }
}

/// Instance-owned registry. Clone cheaply snapshots the `Arc` registrations; there is no
/// process-global state and no hidden locking or executable discovery.
#[derive(Default, Clone, Debug)]
pub struct Registry {
    adapters: BTreeMap<String, Arc<RegisteredAdapter>>,
}

impl Registry {
    /// Validate and cache capabilities, rejecting duplicate ids instead of replacing them.
    /// Descriptor/schema validation uses the default JSON limits.
    pub fn register(&mut self, adapter: Arc<dyn Adapter>) -> Result<()> {
        let mut descriptor = catch_adapter_unwind(|| adapter.descriptor())?;
        if let Err(error) = validate_descriptor(&descriptor) {
            // An invalid descriptor can contain a caller-built, extremely deep Value. Do not
            // recursively drop it after the iterative depth check rejects it.
            discard_json_iterative(std::mem::take(&mut descriptor.params_schema));
            return Err(error);
        }
        if self.adapters.contains_key(&descriptor.id) {
            return Err(Error::DuplicateAdapter(descriptor.id.clone()));
        }
        self.adapters.insert(descriptor.id.clone(), Arc::new(RegisteredAdapter { descriptor, adapter }));
        Ok(())
    }

    /// Remove an entry, returning whether it existed. Existing [`Arc`] snapshots remain
    /// valid and in-flight calls are not terminated. The adapter is dropped when its final
    /// registration snapshot is dropped. The removed id may immediately be registered again.
    pub fn remove(&mut self, id: &str) -> bool {
        self.adapters.remove(id).is_some()
    }

    /// Owned descriptors, in stable lexicographic adapter-id order.
    pub fn list(&self) -> Vec<Descriptor> {
        self.adapters.values().map(|entry| entry.descriptor.clone()).collect()
    }

    /// Cached, validated descriptor. This never calls adapter code again.
    pub fn get_descriptor(&self, id: &str) -> Option<&Descriptor> {
        self.adapters.get(id).map(|entry| &entry.descriptor)
    }

    /// Owned registration snapshot for work that outlives this registry borrow.
    pub fn get(&self, id: &str) -> Option<Arc<RegisteredAdapter>> {
        self.adapters.get(id).cloned()
    }

    /// Run through the same validation path as an owned registration snapshot.
    pub fn run(&self, id: &str, request: &Request, ctx: &dyn Context, limits: &Limits) -> Result<Response> {
        let registered = self.get(id).ok_or_else(|| Error::NotFound(error_id(id)))?;
        registered.run(request, ctx, limits)
    }
}

/// A cached descriptor paired with its trusted adapter; fields cannot be replaced by callers.
pub struct RegisteredAdapter {
    descriptor: Descriptor,
    adapter: Arc<dyn Adapter>,
}

impl std::fmt::Debug for RegisteredAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("RegisteredAdapter").field("descriptor", &self.descriptor).finish_non_exhaustive()
    }
}

impl RegisteredAdapter {
    pub fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }

    /// Validate inputs before invoking code and validate outputs before returning success.
    /// Cancellation takes precedence over an adapter result after its call returns. Unwinding
    /// panics become `Panicked`; an abort or a hung adapter cannot be contained in-process.
    pub fn run(&self, request: &Request, ctx: &dyn Context, limits: &Limits) -> Result<Response> {
        if ctx.cancelled() {
            return Err(Error::Canceled);
        }
        let operation = validate_request(&self.descriptor, request, limits)?;
        if ctx.cancelled() {
            return Err(Error::Canceled);
        }
        let progress = CheckedContext { inner: ctx };
        let result = catch_adapter_unwind(|| self.adapter.run(request, &progress));
        if ctx.cancelled() {
            return Err(Error::Canceled);
        }
        let response = result??;
        validate_response(&self.descriptor, operation, request, &response, limits)?;
        if ctx.cancelled() {
            return Err(Error::Canceled);
        }
        Ok(response)
    }
}

struct CheckedContext<'a> {
    inner: &'a dyn Context,
}

impl Context for CheckedContext<'_> {
    fn cancelled(&self) -> bool {
        self.inner.cancelled()
    }

    fn progress(&self, fraction: f32) {
        if fraction.is_finite() {
            self.inner.progress(fraction.clamp(0.0, 1.0));
        }
    }
}

fn catch_adapter_unwind<T>(call: impl FnOnce() -> T) -> Result<T> {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(result) => Ok(result),
        Err(payload) => {
            // Even a panic_any payload can have a panicking destructor. Quarantine a second
            // payload instead of letting its destructor escape this last-resort guard.
            if let Err(secondary) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
                std::mem::forget(secondary);
            }
            Err(Error::Panicked)
        }
    }
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_ID_BYTES && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn error_id(value: &str) -> String {
    if valid_id(value) { value.into() } else { "<invalid identifier>".into() }
}

fn valid_text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= MAX_TEXT_BYTES && !value.chars().any(char::is_control)
}

fn descriptor_error(message: &str) -> Error {
    Error::InvalidDescriptor(message.into())
}

fn validate_descriptor(descriptor: &Descriptor) -> Result<()> {
    if descriptor.api_version != API_VERSION {
        return Err(Error::ApiVersion { expected: API_VERSION, actual: descriptor.api_version });
    }
    if !valid_id(&descriptor.id) || !valid_text(&descriptor.name) || !valid_text(&descriptor.version) {
        return Err(descriptor_error("invalid id, name or version"));
    }
    if descriptor.operations.is_empty()
        || descriptor.operations.len() > MAX_CAPABILITIES
        || descriptor.devices.is_empty()
        || descriptor.devices.len() > MAX_CAPABILITIES
        || descriptor.resource_formats.len() > MAX_CAPABILITIES
    {
        return Err(descriptor_error("capability lists must be nonempty and bounded (resource_formats may be empty)"));
    }
    validate_json_object(&descriptor.params_schema, &Limits::default()).map_err(|error| descriptor_error(&format!("params_schema: {error}")))?;
    let mut operation_ids = BTreeSet::new();
    for operation in &descriptor.operations {
        if !valid_id(&operation.id) || !operation_ids.insert(operation.id.as_str()) {
            return Err(descriptor_error("invalid or duplicate operation id"));
        }
        if operation.color_modes.is_empty() || operation.color_modes.len() > MAX_CAPABILITIES {
            return Err(descriptor_error("operation color modes must be nonempty and bounded"));
        }
        let mut modes = BTreeSet::new();
        for mode in &operation.color_modes {
            if !valid_id(mode) || !modes.insert(mode.as_str()) {
                return Err(descriptor_error("invalid or duplicate color mode"));
            }
        }
    }
    let mut device_ids = BTreeSet::new();
    let mut supported_operations = BTreeSet::new();
    for device in &descriptor.devices {
        if !valid_id(&device.id)
            || !device_ids.insert(device.id.as_str())
            || !valid_id(&device.backend)
            || !valid_text(&device.name)
            || device.operations.is_empty()
            || device.operations.len() > MAX_CAPABILITIES
        {
            return Err(descriptor_error("invalid/duplicate device or invalid device capabilities"));
        }
        let mut operations = BTreeSet::new();
        for operation in &device.operations {
            if !operation_ids.contains(operation.as_str()) || !operations.insert(operation.as_str()) {
                return Err(descriptor_error("device refers to unknown or duplicate operation"));
            }
            supported_operations.insert(operation.as_str());
        }
    }
    if operation_ids != supported_operations {
        return Err(descriptor_error("every operation must be supported by a declared device"));
    }
    let mut formats = BTreeSet::new();
    for format in &descriptor.resource_formats {
        if !valid_text(format) || !formats.insert(format.as_str()) {
            return Err(descriptor_error("invalid or duplicate resource format label"));
        }
    }
    Ok(())
}

fn validate_request<'a>(descriptor: &'a Descriptor, request: &Request, limits: &Limits) -> Result<&'a Operation> {
    let operation = descriptor
        .operations
        .iter()
        .find(|operation| operation.id == request.operation)
        .ok_or_else(|| Error::UnsupportedOperation(error_id(&request.operation)))?;
    validate_json_object(&request.params, limits)?;
    if request.resource.as_ref().is_some_and(|resource| resource.len() > limits.max_resource_bytes) {
        return Err(Error::Limit("opaque resource bytes".into()));
    }
    request.image.validate(limits)?;
    if !operation.color_modes.contains(&request.image.color_mode) {
        return Err(Error::UnsupportedColorMode(request.image.color_mode.clone()));
    }
    if let Some(device_id) = &request.device {
        let device = find_device(descriptor, device_id)?;
        if !device.operations.contains(&operation.id) {
            return Err(Error::Device("requested device does not support this operation".into()));
        }
    }
    if let Some(selection) = &request.selection {
        selection.validate(limits)?;
        if !same_geometry(&request.image, selection) || !is_gray(selection) || selection.data.iter().any(|value| !(0.0..=1.0).contains(value)) {
            return Err(Error::InvalidSelection("must be same-geometry, alpha-free/profile-free one-channel gray in 0..=1".into()));
        }
    }
    Ok(operation)
}

fn find_device<'a>(descriptor: &'a Descriptor, id: &str) -> Result<&'a Device> {
    descriptor.devices.iter().find(|device| device.id == id).ok_or_else(|| Error::UnknownDevice(error_id(id)))
}

fn same_geometry(left: &Image, right: &Image) -> bool {
    left.x == right.x && left.y == right.y && left.width == right.width && left.height == right.height
}

fn is_gray(image: &Image) -> bool {
    image.channels == 1 && !image.alpha && image.icc_profile.is_empty() && matches!(image.color_mode.as_str(), "gray" | "grayscale")
}

fn validate_response(descriptor: &Descriptor, operation: &Operation, request: &Request, response: &Response, limits: &Limits) -> Result<()> {
    if response.output.kind != operation.output_kind {
        return Err(Error::InvalidResponse("output kind differs from operation declaration".into()));
    }
    let output = &response.output.image;
    output.validate(limits)?;
    if !same_geometry(&request.image, output) {
        return Err(Error::InvalidResponse("output geometry differs from input".into()));
    }
    match operation.output_kind {
        OutputKind::Image => {
            let input = &request.image;
            if output.color_mode != input.color_mode
                || output.source_depth != input.source_depth
                || output.channels != input.channels
                || output.alpha != input.alpha
                || output.icc_profile != input.icc_profile
            {
                return Err(Error::InvalidResponse("image output changed mode/depth/channels/alpha/ICC profile".into()));
            }
        }
        OutputKind::Mask | OutputKind::Depth => {
            if !is_gray(output) || output.source_depth != 32 {
                return Err(Error::InvalidResponse("mask/depth output must be alpha-free/profile-free, single-channel gray F32".into()));
            }
            if operation.output_kind == OutputKind::Mask && output.data.iter().any(|value| !(0.0..=1.0).contains(value)) {
                return Err(Error::InvalidResponse("mask sample outside 0..=1".into()));
            }
        }
    }
    validate_execution(descriptor, operation, request, &response.execution)
}

fn validate_execution(descriptor: &Descriptor, operation: &Operation, request: &Request, execution: &Execution) -> Result<()> {
    if execution.devices.is_empty() || execution.devices.len() > descriptor.devices.len() {
        return Err(Error::Device("actual device list must be nonempty and bounded".into()));
    }
    let mut seen = BTreeSet::new();
    let mut backend: Option<&str> = None;
    let mut mixed_backends = false;
    for id in &execution.devices {
        if !seen.insert(id.as_str()) {
            return Err(Error::Device("duplicate actual device id".into()));
        }
        let device = find_device(descriptor, id)?;
        if !device.operations.contains(&operation.id) {
            return Err(Error::Device("actual device does not support this operation".into()));
        }
        match backend {
            None => backend = Some(device.backend.as_str()),
            Some(previous) if previous != device.backend.as_str() => mixed_backends = true,
            Some(_) => {}
        }
    }
    let expected_backend = if mixed_backends { "mixed" } else { backend.ok_or_else(|| Error::Device("missing actual backend".into()))? };
    if execution.backend != expected_backend {
        return Err(Error::Device("reported backend differs from declared actual-device backend(s)".into()));
    }
    if let Some(reason) = &execution.fallback_reason
        && (!request.allow_fallback || !valid_text(reason))
    {
        return Err(Error::Device("fallback reason requires permission and nonempty bounded text".into()));
    }
    if let Some(requested) = &request.device {
        let unchanged = execution.devices.len() == 1 && execution.devices.first() == Some(requested);
        if !unchanged && (!request.allow_fallback || execution.fallback_reason.is_none()) {
            return Err(Error::Device("explicit device changed without allowed, explained fallback".into()));
        }
        if unchanged && execution.fallback_reason.is_some() {
            return Err(Error::Device("fallback reported but explicit device did not change".into()));
        }
    }
    Ok(())
}

fn budget_add(total: &mut usize, amount: usize, limit: usize, name: &str) -> Result<()> {
    let next = total.checked_add(amount).ok_or_else(|| Error::Limit(name.into()))?;
    if next > limit {
        return Err(Error::Limit(name.into()));
    }
    *total = next;
    Ok(())
}

// Exact compact JSON string byte length, without allocating or serializing nested values.
fn json_string_bytes(value: &str, total: &mut usize, limit: usize) -> Result<()> {
    budget_add(total, 2, limit, "JSON bytes")?;
    for byte in value.bytes() {
        let amount = match byte {
            b'"' | b'\\' | b'\x08' | b'\t' | b'\n' | b'\x0c' | b'\r' => 2,
            0..=31 => 6,
            _ => 1,
        };
        budget_add(total, amount, limit, "JSON bytes")?;
    }
    Ok(())
}

fn validate_json_object(root: &Value, limits: &Limits) -> Result<()> {
    if !root.is_object() {
        return Err(Error::InvalidParams("JSON root must be an object".into()));
    }
    let mut stack = vec![(root, 1usize)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        if depth > limits.max_json_depth.min(MAX_JSON_DEPTH) {
            return Err(Error::Limit("JSON depth".into()));
        }
        budget_add(&mut nodes, 1, limits.max_json_nodes, "JSON nodes")?;
        match value {
            Value::Null => budget_add(&mut bytes, 4, limits.max_json_bytes, "JSON bytes")?,
            Value::Bool(value) => budget_add(&mut bytes, if *value { 4 } else { 5 }, limits.max_json_bytes, "JSON bytes")?,
            // Number formatting is scalar and bounded; never call to_string on a container.
            Value::Number(value) => budget_add(&mut bytes, value.to_string().len(), limits.max_json_bytes, "JSON bytes")?,
            Value::String(value) => json_string_bytes(value, &mut bytes, limits.max_json_bytes)?,
            Value::Array(values) => {
                if values.len() > limits.max_json_array_items {
                    return Err(Error::Limit("JSON array items".into()));
                }
                check_json_children(nodes, stack.len(), values.len(), depth, limits)?;
                budget_add(&mut bytes, 2, limits.max_json_bytes, "JSON bytes")?;
                budget_add(&mut bytes, values.len().saturating_sub(1), limits.max_json_bytes, "JSON bytes")?;
                stack.try_reserve(values.len()).map_err(|_| Error::Limit("JSON traversal allocation".into()))?;
                for value in values {
                    stack.push((value, depth + 1));
                }
            }
            Value::Object(values) => {
                if values.len() > limits.max_json_object_entries {
                    return Err(Error::Limit("JSON object entries".into()));
                }
                check_json_children(nodes, stack.len(), values.len(), depth, limits)?;
                budget_add(&mut bytes, 2, limits.max_json_bytes, "JSON bytes")?;
                budget_add(&mut bytes, values.len().saturating_sub(1), limits.max_json_bytes, "JSON bytes")?;
                stack.try_reserve(values.len()).map_err(|_| Error::Limit("JSON traversal allocation".into()))?;
                for (key, value) in values {
                    json_string_bytes(key, &mut bytes, limits.max_json_bytes)?;
                    budget_add(&mut bytes, 1, limits.max_json_bytes, "JSON bytes")?;
                    stack.push((value, depth + 1));
                }
            }
        }
    }
    Ok(())
}

fn check_json_children(nodes: usize, pending: usize, children: usize, depth: usize, limits: &Limits) -> Result<()> {
    let scheduled = nodes.checked_add(pending).and_then(|value| value.checked_add(children)).ok_or_else(|| Error::Limit("JSON nodes overflow".into()))?;
    if scheduled > limits.max_json_nodes {
        return Err(Error::Limit("JSON nodes".into()));
    }
    if children != 0 && depth >= limits.max_json_depth.min(MAX_JSON_DEPTH) {
        return Err(Error::Limit("JSON depth".into()));
    }
    Ok(())
}

enum JsonChildren {
    Array(std::vec::IntoIter<Value>),
    Object(serde_json::map::IntoIter),
}

impl JsonChildren {
    fn next(&mut self) -> Option<Value> {
        match self {
            Self::Array(values) => values.next(),
            Self::Object(values) => values.next().map(|(_, value)| value),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::Array(values) => values.len() == 0,
            Self::Object(values) => values.len() == 0,
        }
    }
}

fn discard_json_iterative(root: Value) {
    // Keep owned iterators rather than copying a wide container into a second allocation.
    // Single-child chains require no traversal stack at all, even when extremely deep.
    let mut stack = Vec::new();
    let mut current = Some(root);
    loop {
        if let Some(value) = current.take() {
            let children = match value {
                Value::Array(values) => Some(JsonChildren::Array(values.into_iter())),
                Value::Object(values) => Some(JsonChildren::Object(values.into_iter())),
                _ => None,
            };
            if let Some(mut children) = children {
                current = children.next();
                if !children.is_empty() {
                    stack.push(children);
                }
            }
        } else if let Some(children) = stack.last_mut() {
            current = children.next();
            if children.is_empty() {
                stack.pop();
            }
        } else {
            break;
        }
    }
}
