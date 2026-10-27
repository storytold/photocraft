//! Native transport for an explicitly authorized, **trusted** adapter executable.
//!
//! This is reliability isolation, not an OS sandbox: workers retain the user's permissions and
//! can start descendants. Only the direct child is killed/reaped. The caller supplies the
//! executable and manifest descriptor; discovery never executes it. Register with
//! [`adapter::Registry`] for authoritative capability/device/fallback checks. Execution reports
//! are worker assertions, not physical-device attestation. No implicit CPU fallback occurs.
//!
//! Each invocation has a private temporary directory and exactly six arguments:
//! `--action <probe|prepare|run> --request <absolute request.bin> --response <absolute response.bin>`.
//! No shell or additional arguments are used. Standard streams are null. The timeout covers
//! copying, the child, and decoding. Cancellation is checked during copies and every 10 ms
//! while polling. Symlink response rejection is best effort, not a portable race-free sandbox.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::{self, Adapter, Context, Descriptor, Error, Execution, Image, Limits, Output, OutputKind, Request, Response, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Ready,
    NeedsModel,
    Unavailable,
    Error,
}

/// Readiness reported by an authorized worker, not a host hardware probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerStatus {
    pub state: WorkerState,
    pub message: String,
}

/// A control/status response or a successful run response.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkerReply {
    Status(WorkerStatus),
    Ok(Response),
}

#[derive(Debug)]
pub struct WorkerAdapter {
    executable: PathBuf,
    descriptor: Descriptor,
    timeout: Duration,
}

impl WorkerAdapter {
    /// Construct without starting or requiring an installed executable. The path must be
    /// absolute and the timeout 1000..=300000 ms. Windows requires an explicit `.exe`, excluding
    /// scripts that `Command` could run via cmd. Executable aliases are not canonicalized.
    /// Register before running: only the registry performs full descriptor validation.
    pub fn new(executable: PathBuf, mut descriptor: Descriptor, timeout: Duration) -> Result<Self> {
        if let Err(error) = Limits::default().check_params(&descriptor.params_schema) {
            discard_json(std::mem::take(&mut descriptor.params_schema));
            return Err(error);
        }
        if descriptor.api_version != adapter::API_VERSION {
            return Err(Error::ApiVersion { expected: adapter::API_VERSION, actual: descriptor.api_version });
        }
        // Bound caller-built capabilities before descriptor() can clone them.
        protocol::json_header(&descriptor, &Budget { ctx: &NoContext, deadline: None })?;
        if !(Duration::from_millis(1000)..=Duration::from_millis(300000)).contains(&timeout) {
            return Err(Error::Failed("worker timeout must be 1000..=300000 ms".into()));
        }
        if !executable.is_absolute() {
            return Err(Error::Failed("worker executable must be an absolute path".into()));
        }
        #[cfg(windows)]
        if !executable.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("exe")) {
            return Err(Error::Failed("worker executable must be an explicit .exe, not a shell script".into()));
        }
        Ok(Self { executable, descriptor, timeout })
    }

    pub fn probe(&self, ctx: &dyn Context) -> Result<WorkerStatus> {
        self.control("probe", ctx)
    }

    pub fn prepare(&self, ctx: &dyn Context) -> Result<WorkerStatus> {
        self.control("prepare", ctx)
    }

    fn control(&self, action: &'static str, ctx: &dyn Context) -> Result<WorkerStatus> {
        match self.invoke(action, None, ctx)? {
            WorkerReply::Status(status) => Ok(status),
            WorkerReply::Ok(_) => Err(Error::InvalidResponse("worker returned a run result for a control action".into())),
        }
    }

    fn invoke(&self, action: &'static str, request: Option<&Request>, ctx: &dyn Context) -> Result<WorkerReply> {
        let deadline = Instant::now().checked_add(self.timeout).ok_or_else(|| Error::Limit("worker deadline overflow".into()))?;
        let budget = Budget { ctx, deadline: Some(deadline) };
        budget.check()?;
        let directory = tempfile::Builder::new().prefix("photocraft-worker-").tempdir().map_err(|error| io_error("create worker directory", error))?;
        let working_dir = fs::canonicalize(directory.path()).map_err(|error| io_error("resolve worker directory", error))?;
        let request_path = working_dir.join("request.bin");
        let response_path = working_dir.join("response.bin");
        let mut file = OpenOptions::new().write(true).create_new(true).open(&request_path).map_err(|error| io_error("create worker request", error))?;
        protocol::write_request_inner(&mut file, request, &budget)?;
        file.flush().map_err(|error| io_error("flush worker request", error))?;
        drop(file);
        budget.check()?;
        let child = Command::new(&self.executable)
            .current_dir(&working_dir)
            .arg("--action")
            .arg(action)
            .arg("--request")
            .arg(&request_path)
            .arg("--response")
            .arg(&response_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| io_error("start worker", error))?;
        // Drop order kills/reaps the child before removing its temporary directory.
        let mut child = ChildGuard { child, reaped: false };
        let exit = loop {
            budget.check()?;
            if let Some(status) = child.child.try_wait().map_err(|error| io_error("poll worker", error))? {
                child.reaped = true;
                break status;
            }
            thread::sleep(Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())));
        };
        budget.check()?;
        let mut file = open_response(&response_path)?;
        let reply = protocol::read_response_inner(&mut file, &budget)?;
        budget.check()?;
        if !exit.success() {
            if let WorkerReply::Status(status) = &reply {
                return Err(Error::Failed(format!("worker exited with {exit}: {}", status.message)));
            }
            return Err(Error::Failed(format!("worker exited with {exit}")));
        }
        Ok(reply)
    }
}

impl Adapter for WorkerAdapter {
    fn descriptor(&self) -> Descriptor {
        self.descriptor.clone()
    }
    fn run(&self, request: &Request, ctx: &dyn Context) -> Result<Response> {
        match self.invoke("run", Some(request), ctx)? {
            WorkerReply::Ok(response) => Ok(response),
            WorkerReply::Status(status) => Err(Error::Failed(format!("worker {:?}: {}", status.state, status.message))),
        }
    }
}

struct ChildGuard {
    child: Child,
    reaped: bool,
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

struct Budget<'a> {
    ctx: &'a dyn Context,
    deadline: Option<Instant>,
}
impl Budget<'_> {
    fn check(&self) -> Result<()> {
        if self.ctx.cancelled() {
            return Err(Error::Canceled);
        }
        if self.deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(Error::Failed("worker timed out".into()));
        }
        Ok(())
    }
}

struct NoContext;
impl Context for NoContext {
    fn cancelled(&self) -> bool {
        false
    }
    fn progress(&self, _fraction: f32) {}
}

fn io_error(operation: &str, error: std::io::Error) -> Error {
    Error::Failed(format!("{operation}: {error}"))
}

fn open_response(path: &Path) -> Result<File> {
    let before = fs::symlink_metadata(path).map_err(|error| io_error("inspect worker response", error))?;
    check_response_metadata(&before)?;
    let file = File::open(path).map_err(|error| io_error("open worker response", error))?;
    let opened = file.metadata().map_err(|error| io_error("inspect opened worker response", error))?;
    check_response_metadata(&opened)?;
    let after = fs::symlink_metadata(path).map_err(|error| io_error("recheck worker response", error))?;
    check_response_metadata(&after)?;
    // Safe Rust has no portable O_NOFOLLOW open. These checks are deliberately best effort.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() || after.dev() != opened.dev() || after.ino() != opened.ino() {
            return Err(Error::InvalidResponse("worker response file changed while opening".into()));
        }
    }
    Ok(file)
}
fn check_response_metadata(metadata: &fs::Metadata) -> Result<()> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(Error::InvalidResponse("worker response must be a regular, non-symlink file".into()));
    }
    if metadata.len() > protocol::max_frame_bytes(false)? {
        return Err(Error::Limit("worker response file bytes".into()));
    }
    Ok(())
}

// Avoid recursively dropping an arbitrarily deep caller-built schema after rejecting it.
fn discard_json(root: Value) {
    enum Children {
        Array(std::vec::IntoIter<Value>),
        Object(serde_json::map::IntoIter),
    }
    impl Children {
        fn next(&mut self) -> Option<Value> {
            match self {
                Self::Array(values) => values.next(),
                Self::Object(values) => values.next().map(|(_, value)| value),
            }
        }
        fn remaining(&self) -> usize {
            match self {
                Self::Array(values) => values.len(),
                Self::Object(values) => values.len(),
            }
        }
    }
    let mut stack = Vec::new();
    let mut current = Some(root);
    loop {
        if let Some(value) = current.take() {
            let children = match value {
                Value::Array(values) => Some(Children::Array(values.into_iter())),
                Value::Object(values) => Some(Children::Object(values.into_iter())),
                _ => None,
            };
            if let Some(mut children) = children {
                current = children.next();
                if children.remaining() != 0 {
                    stack.push(children);
                }
            }
        } else if let Some(children) = stack.last_mut() {
            current = children.next();
            if children.remaining() == 0 {
                stack.pop();
            }
        } else {
            break;
        }
    }
}

/// Fixed cross-language protocol, independent of any vendor runtime.
///
/// A frame starts with eight ASCII bytes `PCADREQ1` / `PCADRES1`, little-endian u32 header
/// length (<=1 MiB), UTF-8 JSON, then interleaved LE f32 samples. Run headers contain
/// `protocol:1, operation, resource, params, device, allow_fallback, image, selection`.
/// Metadata has every [`Image`] field except `data`. Image samples precede selection samples.
/// Control requests are exactly `{"protocol":1}`. Response headers contain
/// `protocol:1, status, message, output, execution`. Only `ok` has non-null output/execution;
/// `output` is `{kind,image:<metadata>}`. Other statuses have no samples. Trailing bytes fail.
///
/// Decoders require seekable input to compare actual frame length with metadata **before**
/// allocating sample buffers. Byte/sample vectors use fallible reservations; Serde's internal
/// allocations are bounded by the header cap and default recursion limit. Default adapter
/// limits apply even if registration uses higher limits. Successful messages are discarded.
/// Descriptor authority and device/fallback policy remain the registry's responsibility.
pub mod protocol {
    use super::*;

    pub const REQUEST_MAGIC: &[u8; 8] = b"PCADREQ1";
    pub const RESPONSE_MAGIC: &[u8; 8] = b"PCADRES1";
    pub const MAX_HEADER_BYTES: usize = 1024 * 1024;
    const COPY_BYTES: usize = 16 * 1024;
    const COPY_SAMPLES: usize = COPY_BYTES / 4;

    /// `None` encodes a probe/prepare request, `Some` a run request.
    pub fn write_request(writer: &mut impl Write, request: Option<&Request>, ctx: &dyn Context) -> Result<()> {
        write_request_inner(writer, request, &Budget { ctx, deadline: None })
    }
    pub fn read_request(reader: &mut (impl Read + Seek), ctx: &dyn Context) -> Result<Option<Request>> {
        read_request_inner(reader, &Budget { ctx, deadline: None })
    }
    pub fn write_response(writer: &mut impl Write, reply: &WorkerReply, ctx: &dyn Context) -> Result<()> {
        write_response_inner(writer, reply, &Budget { ctx, deadline: None })
    }
    pub fn read_response(reader: &mut (impl Read + Seek), ctx: &dyn Context) -> Result<WorkerReply> {
        read_response_inner(reader, &Budget { ctx, deadline: None })
    }

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ControlHeader {
        protocol: u32,
    }
    #[derive(Serialize)]
    struct RunHeaderRef<'a> {
        protocol: u32,
        operation: &'a str,
        resource: &'a Option<String>,
        params: &'a Value,
        device: &'a Option<String>,
        allow_fallback: bool,
        image: ImageInfo<'a>,
        selection: Option<ImageInfo<'a>>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RunHeader {
        protocol: u32,
        operation: String,
        resource: Option<String>,
        params: Value,
        device: Option<String>,
        allow_fallback: bool,
        image: ImageMetadata,
        selection: Option<ImageMetadata>,
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RequestHeader {
        Run(Box<RunHeader>),
        Control(ControlHeader),
    }
    #[derive(Clone, Copy, Serialize)]
    struct ImageInfo<'a> {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        channels: u8,
        color_mode: &'a str,
        source_depth: u8,
        alpha: bool,
        icc_profile: &'a [u8],
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ImageMetadata {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        channels: u8,
        color_mode: String,
        source_depth: u8,
        alpha: bool,
        icc_profile: Vec<u8>,
    }
    impl<'a> From<&'a Image> for ImageInfo<'a> {
        fn from(image: &'a Image) -> Self {
            Self {
                x: image.x,
                y: image.y,
                width: image.width,
                height: image.height,
                channels: image.channels,
                color_mode: &image.color_mode,
                source_depth: image.source_depth,
                alpha: image.alpha,
                icc_profile: &image.icc_profile,
            }
        }
    }
    impl ImageMetadata {
        fn info(&self) -> ImageInfo<'_> {
            ImageInfo {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
                channels: self.channels,
                color_mode: &self.color_mode,
                source_depth: self.source_depth,
                alpha: self.alpha,
                icc_profile: &self.icc_profile,
            }
        }
        fn into_image(self, data: Vec<f32>) -> Image {
            Image {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
                channels: self.channels,
                color_mode: self.color_mode,
                source_depth: self.source_depth,
                alpha: self.alpha,
                icc_profile: self.icc_profile,
                data,
            }
        }
    }
    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum ReplyState {
        Ready,
        NeedsModel,
        Unavailable,
        Error,
        Ok,
    }
    impl From<WorkerState> for ReplyState {
        fn from(state: WorkerState) -> Self {
            match state {
                WorkerState::Ready => Self::Ready,
                WorkerState::NeedsModel => Self::NeedsModel,
                WorkerState::Unavailable => Self::Unavailable,
                WorkerState::Error => Self::Error,
            }
        }
    }
    #[derive(Serialize)]
    struct ResponseHeaderRef<'a> {
        protocol: u32,
        status: ReplyState,
        message: &'a str,
        output: Option<OutputInfo<'a>>,
        execution: Option<&'a Execution>,
    }
    #[derive(Serialize)]
    struct OutputInfo<'a> {
        kind: OutputKind,
        image: ImageInfo<'a>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ResponseHeader {
        protocol: u32,
        status: ReplyState,
        message: String,
        output: Option<OutputMetadata>,
        execution: Option<Execution>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct OutputMetadata {
        kind: OutputKind,
        image: ImageMetadata,
    }

    pub(super) fn write_request_inner(writer: &mut impl Write, request: Option<&Request>, budget: &Budget<'_>) -> Result<()> {
        budget.check()?;
        if let Some(request) = request {
            check_request_fields(&request.operation, &request.resource, &request.params, &request.device)?;
            let image = ImageInfo::from(&request.image);
            check_image_data(image, &request.image.data)?;
            if let Some(selection) = &request.selection {
                check_selection(image, ImageInfo::from(selection))?;
                check_image_data(ImageInfo::from(selection), &selection.data)?;
            }
            let header = RunHeaderRef {
                protocol: 1,
                operation: &request.operation,
                resource: &request.resource,
                params: &request.params,
                device: &request.device,
                allow_fallback: request.allow_fallback,
                image,
                selection: request.selection.as_ref().map(ImageInfo::from),
            };
            write_header(writer, REQUEST_MAGIC, &header, budget)?;
            write_samples(writer, image, &request.image.data, false, budget)?;
            if let Some(selection) = &request.selection {
                write_samples(writer, ImageInfo::from(selection), &selection.data, true, budget)?;
            }
        } else {
            write_header(writer, REQUEST_MAGIC, &ControlHeader { protocol: 1 }, budget)?;
        }
        budget.check()
    }

    fn read_request_inner(reader: &mut (impl Read + Seek), budget: &Budget<'_>) -> Result<Option<Request>> {
        let frame_len = frame_length(reader, true, budget)?;
        let (header, header_len): (RequestHeader, _) = read_header(reader, REQUEST_MAGIC, budget)?;
        match header {
            RequestHeader::Control(header) => {
                check_version(header.protocol)?;
                check_frame_length(frame_len, header_len, 0)?;
                check_eof(reader, budget)?;
                Ok(None)
            }
            RequestHeader::Run(header) => {
                let header = *header;
                check_version(header.protocol)?;
                check_request_fields(&header.operation, &header.resource, &header.params, &header.device)?;
                let count = check_image_info(header.image.info())?;
                let selection_count = if let Some(selection) = &header.selection {
                    check_selection(header.image.info(), selection.info())?;
                    check_image_info(selection.info())?
                } else {
                    0
                };
                check_frame_length(frame_len, header_len, count.checked_add(selection_count).ok_or_else(|| Error::Limit("worker request samples".into()))?)?;
                let data = read_samples(reader, header.image.info(), false, budget)?;
                let selection = if let Some(selection) = header.selection {
                    let data = read_samples(reader, selection.info(), true, budget)?;
                    Some(selection.into_image(data))
                } else {
                    None
                };
                check_eof(reader, budget)?;
                Ok(Some(Request {
                    operation: header.operation,
                    resource: header.resource,
                    params: header.params,
                    device: header.device,
                    allow_fallback: header.allow_fallback,
                    image: header.image.into_image(data),
                    selection,
                }))
            }
        }
    }

    fn write_response_inner(writer: &mut impl Write, reply: &WorkerReply, budget: &Budget<'_>) -> Result<()> {
        budget.check()?;
        match reply {
            WorkerReply::Status(status) => {
                check_message(&status.message)?;
                let header = ResponseHeaderRef { protocol: 1, status: status.state.into(), message: &status.message, output: None, execution: None };
                write_header(writer, RESPONSE_MAGIC, &header, budget)?;
            }
            WorkerReply::Ok(response) => {
                check_execution(&response.execution)?;
                let image = ImageInfo::from(&response.output.image);
                check_output(response.output.kind, image)?;
                check_image_data(image, &response.output.image.data)?;
                let header = ResponseHeaderRef {
                    protocol: 1,
                    status: ReplyState::Ok,
                    message: "",
                    output: Some(OutputInfo { kind: response.output.kind, image }),
                    execution: Some(&response.execution),
                };
                write_header(writer, RESPONSE_MAGIC, &header, budget)?;
                write_samples(writer, image, &response.output.image.data, response.output.kind == OutputKind::Mask, budget)?;
            }
        }
        budget.check()
    }

    pub(super) fn read_response_inner(reader: &mut (impl Read + Seek), budget: &Budget<'_>) -> Result<WorkerReply> {
        let frame_len = frame_length(reader, false, budget)?;
        let (header, header_len): (ResponseHeader, _) = read_header(reader, RESPONSE_MAGIC, budget)?;
        check_version(header.protocol)?;
        check_message(&header.message)?;
        if matches!(header.status, ReplyState::Ok) {
            let output = header.output.ok_or_else(|| Error::InvalidResponse("ok worker response requires output".into()))?;
            let execution = header.execution.ok_or_else(|| Error::InvalidResponse("ok worker response requires execution".into()))?;
            check_execution(&execution)?;
            check_output(output.kind, output.image.info())?;
            let samples = check_image_info(output.image.info())?;
            check_frame_length(frame_len, header_len, samples)?;
            let data = read_samples(reader, output.image.info(), output.kind == OutputKind::Mask, budget)?;
            check_eof(reader, budget)?;
            return Ok(WorkerReply::Ok(Response { output: Output { kind: output.kind, image: output.image.into_image(data) }, execution }));
        }
        if header.output.is_some() || header.execution.is_some() {
            return Err(Error::InvalidResponse("non-ok worker response must not contain output or execution".into()));
        }
        check_frame_length(frame_len, header_len, 0)?;
        check_eof(reader, budget)?;
        let state = match header.status {
            ReplyState::Ready => WorkerState::Ready,
            ReplyState::NeedsModel => WorkerState::NeedsModel,
            ReplyState::Unavailable => WorkerState::Unavailable,
            ReplyState::Error => WorkerState::Error,
            ReplyState::Ok => return Err(Error::InvalidResponse("unexpected worker ok state".into())),
        };
        Ok(WorkerReply::Status(WorkerStatus { state, message: header.message }))
    }

    fn check_version(version: u32) -> Result<()> {
        if version != 1 {
            return Err(Error::ApiVersion { expected: 1, actual: version });
        }
        Ok(())
    }
    fn valid_id(value: &str) -> bool {
        !value.is_empty() && value.len() <= 128 && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    }
    fn check_request_fields(operation: &str, resource: &Option<String>, params: &Value, device: &Option<String>) -> Result<()> {
        let limits = Limits::default();
        limits.check_params(params)?;
        if !valid_id(operation) || device.as_ref().is_some_and(|device| !valid_id(device)) {
            return Err(Error::InvalidParams("invalid worker operation or device identifier".into()));
        }
        if resource.as_ref().is_some_and(|resource| resource.len() > limits.max_resource_bytes) {
            return Err(Error::Limit("worker resource bytes".into()));
        }
        Ok(())
    }
    fn check_image_info(image: ImageInfo<'_>) -> Result<usize> {
        let limits = Limits::default();
        let samples = limits.check_image_size(image.width, image.height, image.channels)?;
        let width = i32::try_from(image.width).map_err(|_| Error::InvalidImage("worker width exceeds coordinate range".into()))?;
        let height = i32::try_from(image.height).map_err(|_| Error::InvalidImage("worker height exceeds coordinate range".into()))?;
        image.x.checked_add(width).ok_or_else(|| Error::InvalidImage("worker x + width overflows".into()))?;
        image.y.checked_add(height).ok_or_else(|| Error::InvalidImage("worker y + height overflows".into()))?;
        if !matches!(image.source_depth, 8 | 16 | 32) || !valid_id(image.color_mode) || (image.alpha && image.channels < 2) {
            return Err(Error::InvalidImage("invalid worker depth, color mode, or alpha metadata".into()));
        }
        if image.icc_profile.len() > limits.max_icc_bytes.min(adapter::MAX_ICC_BYTES) {
            return Err(Error::Limit("worker ICC profile bytes".into()));
        }
        Ok(samples)
    }
    fn check_image_data(image: ImageInfo<'_>, data: &[f32]) -> Result<()> {
        if check_image_info(image)? != data.len() {
            return Err(Error::InvalidImage("worker sample count does not match image metadata".into()));
        }
        Ok(())
    }
    fn is_gray(image: ImageInfo<'_>) -> bool {
        image.channels == 1 && !image.alpha && image.icc_profile.is_empty() && matches!(image.color_mode, "gray" | "grayscale")
    }
    fn check_selection(image: ImageInfo<'_>, selection: ImageInfo<'_>) -> Result<()> {
        check_image_info(selection)?;
        if !is_gray(selection) || image.x != selection.x || image.y != selection.y || image.width != selection.width || image.height != selection.height {
            return Err(Error::InvalidSelection("worker selection must be same-geometry, alpha-free/profile-free one-channel gray".into()));
        }
        Ok(())
    }
    fn check_output(kind: OutputKind, image: ImageInfo<'_>) -> Result<()> {
        check_image_info(image)?;
        if kind != OutputKind::Image && (!is_gray(image) || image.source_depth != 32) {
            return Err(Error::InvalidResponse("worker mask/depth must be alpha-free/profile-free one-channel gray F32".into()));
        }
        Ok(())
    }
    fn check_execution(execution: &Execution) -> Result<()> {
        if execution.devices.is_empty() || execution.devices.len() > 128 || !valid_id(&execution.backend) {
            return Err(Error::InvalidResponse("invalid worker execution devices/backend".into()));
        }
        for (index, device) in execution.devices.iter().enumerate() {
            if !valid_id(device) || execution.devices.iter().take(index).any(|previous| previous == device) {
                return Err(Error::InvalidResponse("invalid or duplicate worker execution device".into()));
            }
        }
        if let Some(reason) = &execution.fallback_reason
            && (reason.trim().is_empty() || reason.len() > 1024 || reason.chars().any(char::is_control))
        {
            return Err(Error::InvalidResponse("invalid worker fallback explanation".into()));
        }
        Ok(())
    }
    fn check_message(message: &str) -> Result<()> {
        if message.len() > 64 * 1024 {
            return Err(Error::Limit("worker response message bytes".into()));
        }
        Ok(())
    }
    fn check_sample(image: ImageInfo<'_>, index: usize, value: f32, normalized: bool) -> Result<()> {
        let channels = usize::from(image.channels);
        let alpha = image.alpha && index % channels == channels - 1;
        if !value.is_finite() || ((normalized || image.source_depth != 32 || alpha) && !(0.0..=1.0).contains(&value)) {
            return Err(Error::InvalidImage("worker sample is non-finite or outside its normalized range".into()));
        }
        Ok(())
    }
    fn write_samples(writer: &mut impl Write, image: ImageInfo<'_>, samples: &[f32], normalized: bool, budget: &Budget<'_>) -> Result<()> {
        let mut bytes = [0u8; COPY_BYTES];
        let mut index = 0usize;
        for chunk in samples.chunks(COPY_SAMPLES) {
            budget.check()?;
            let length = chunk.len().checked_mul(4).ok_or_else(|| Error::Limit("worker sample bytes".into()))?;
            let target = bytes.get_mut(..length).ok_or_else(|| Error::Limit("worker copy buffer".into()))?;
            for (value, target) in chunk.iter().zip(target.as_chunks_mut::<4>().0) {
                check_sample(image, index, *value, normalized)?;
                target.copy_from_slice(&value.to_le_bytes());
                index = index.checked_add(1).ok_or_else(|| Error::Limit("worker sample index".into()))?;
            }
            writer.write_all(target).map_err(|error| io_error("write worker samples", error))?;
        }
        budget.check()
    }
    fn read_samples(reader: &mut impl Read, image: ImageInfo<'_>, normalized: bool, budget: &Budget<'_>) -> Result<Vec<f32>> {
        budget.check()?;
        let count = check_image_info(image)?;
        let mut samples = Vec::new();
        samples.try_reserve_exact(count).map_err(|_| Error::Limit("worker sample allocation".into()))?;
        let mut bytes = [0u8; COPY_BYTES];
        while samples.len() < count {
            budget.check()?;
            let length = count.saturating_sub(samples.len()).min(COPY_SAMPLES).checked_mul(4).ok_or_else(|| Error::Limit("worker sample bytes".into()))?;
            let source = bytes.get_mut(..length).ok_or_else(|| Error::Limit("worker copy buffer".into()))?;
            reader.read_exact(source).map_err(|error| io_error("read worker samples", error))?;
            for source in source.as_chunks::<4>().0 {
                let value = f32::from_le_bytes(*source);
                check_sample(image, samples.len(), value, normalized)?;
                samples.push(value);
            }
        }
        budget.check()?;
        Ok(samples)
    }
    pub(super) fn max_frame_bytes(request: bool) -> Result<u64> {
        let payload = Limits::default()
            .max_samples
            .checked_mul(4)
            .and_then(|bytes| bytes.checked_mul(if request { 2 } else { 1 }))
            .ok_or_else(|| Error::Limit("worker frame limit".into()))?;
        let bytes = payload.checked_add(MAX_HEADER_BYTES).and_then(|bytes| bytes.checked_add(12)).ok_or_else(|| Error::Limit("worker frame limit".into()))?;
        u64::try_from(bytes).map_err(|_| Error::Limit("worker frame limit".into()))
    }
    fn frame_length(reader: &mut (impl Read + Seek), request: bool, budget: &Budget<'_>) -> Result<u64> {
        budget.check()?;
        let start = reader.stream_position().map_err(|error| io_error("locate worker frame", error))?;
        let end = reader.seek(SeekFrom::End(0)).map_err(|error| io_error("measure worker frame", error))?;
        reader.seek(SeekFrom::Start(start)).map_err(|error| io_error("rewind worker frame", error))?;
        let length = end.checked_sub(start).ok_or_else(|| Error::InvalidResponse("invalid worker frame position".into()))?;
        if length < 12 || length > max_frame_bytes(request)? {
            return Err(Error::Limit("worker frame bytes".into()));
        }
        budget.check()?;
        Ok(length)
    }
    fn check_frame_length(actual: u64, header_len: usize, samples: usize) -> Result<()> {
        let payload = samples.checked_mul(4).ok_or_else(|| Error::Limit("worker payload length".into()))?;
        let expected = header_len.checked_add(payload).and_then(|bytes| bytes.checked_add(12)).ok_or_else(|| Error::Limit("worker frame length".into()))?;
        if actual != u64::try_from(expected).map_err(|_| Error::Limit("worker frame length".into()))? {
            return Err(Error::InvalidResponse("worker frame length differs from metadata (truncated or trailing bytes)".into()));
        }
        Ok(())
    }

    struct HeaderBuffer<'a, 'b> {
        bytes: Vec<u8>,
        budget: &'a Budget<'b>,
        failure: Option<Error>,
    }
    impl Write for HeaderBuffer<'_, '_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let result = (|| {
                self.budget.check()?;
                let length = self.bytes.len().checked_add(bytes.len()).ok_or_else(|| Error::Limit("worker JSON header bytes".into()))?;
                if length > MAX_HEADER_BYTES {
                    return Err(Error::Limit("worker JSON header bytes".into()));
                }
                self.bytes.try_reserve(bytes.len()).map_err(|_| Error::Limit("worker JSON header allocation".into()))?;
                self.bytes.extend_from_slice(bytes);
                Ok(())
            })();
            if let Err(error) = result {
                self.failure = Some(error);
                return Err(std::io::Error::other("worker header budget exceeded"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    pub(super) fn json_header(header: &impl Serialize, budget: &Budget<'_>) -> Result<Vec<u8>> {
        let mut writer = HeaderBuffer { bytes: Vec::new(), budget, failure: None };
        let result = serde_json::to_writer(&mut writer, header);
        if let Some(error) = writer.failure {
            return Err(error);
        }
        result.map_err(|error| Error::InvalidResponse(format!("serialize worker header: {error}")))?;
        Ok(writer.bytes)
    }
    fn write_header(writer: &mut impl Write, magic: &[u8; 8], header: &impl Serialize, budget: &Budget<'_>) -> Result<()> {
        let bytes = json_header(header, budget)?;
        let length = u32::try_from(bytes.len()).map_err(|_| Error::Limit("worker JSON header length".into()))?;
        budget.check()?;
        writer.write_all(magic).map_err(|error| io_error("write worker magic", error))?;
        writer.write_all(&length.to_le_bytes()).map_err(|error| io_error("write worker header length", error))?;
        for chunk in bytes.chunks(COPY_BYTES) {
            budget.check()?;
            writer.write_all(chunk).map_err(|error| io_error("write worker header", error))?;
        }
        budget.check()
    }
    fn read_header<T: serde::de::DeserializeOwned>(reader: &mut impl Read, magic: &[u8; 8], budget: &Budget<'_>) -> Result<(T, usize)> {
        budget.check()?;
        let mut actual_magic = [0u8; 8];
        reader.read_exact(&mut actual_magic).map_err(|error| io_error("read worker magic", error))?;
        if &actual_magic != magic {
            return Err(Error::InvalidResponse("incorrect worker frame magic".into()));
        }
        let mut word = [0u8; 4];
        reader.read_exact(&mut word).map_err(|error| io_error("read worker header length", error))?;
        let length = usize::try_from(u32::from_le_bytes(word)).map_err(|_| Error::Limit("worker JSON header length".into()))?;
        if length == 0 || length > MAX_HEADER_BYTES {
            return Err(Error::Limit("worker JSON header length".into()));
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| Error::Limit("worker JSON header allocation".into()))?;
        bytes.resize(length, 0);
        for chunk in bytes.chunks_mut(COPY_BYTES) {
            budget.check()?;
            reader.read_exact(chunk).map_err(|error| io_error("read worker JSON header", error))?;
        }
        budget.check()?;
        let header = serde_json::from_slice(&bytes).map_err(|error| Error::InvalidResponse(format!("decode worker JSON header: {error}")))?;
        budget.check()?;
        Ok((header, length))
    }
    fn check_eof(reader: &mut impl Read, budget: &Budget<'_>) -> Result<()> {
        budget.check()?;
        let mut byte = [0u8; 1];
        if reader.read(&mut byte).map_err(|error| io_error("check worker frame end", error))? != 0 {
            return Err(Error::InvalidResponse("trailing worker frame bytes".into()));
        }
        budget.check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    struct TestContext;
    impl Context for TestContext {
        fn cancelled(&self) -> bool {
            false
        }
        fn progress(&self, _: f32) {}
    }

    fn sample_image() -> Image {
        Image {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            channels: 3,
            color_mode: "rgb".into(),
            source_depth: 8,
            alpha: false,
            icc_profile: vec![1, 2, 3, 4],
            data: vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.0, 0.5, 1.0],
        }
    }

    #[test]
    fn protocol_probe_roundtrip() {
        let ctx = TestContext;
        let mut buffer = Vec::new();
        protocol::write_request(&mut buffer, None, &ctx).unwrap();
        let mut cursor = Cursor::new(buffer);
        let decoded = protocol::read_request(&mut cursor, &ctx).unwrap();
        assert!(decoded.is_none());

        let reply = WorkerReply::Status(WorkerStatus { state: WorkerState::Ready, message: "Hexagon NPU available".into() });
        let mut resp_buf = Vec::new();
        protocol::write_response(&mut resp_buf, &reply, &ctx).unwrap();
        let mut resp_cursor = Cursor::new(resp_buf);
        let read_reply = protocol::read_response(&mut resp_cursor, &ctx).unwrap();
        assert_eq!(read_reply, reply);
    }

    #[test]
    fn protocol_run_roundtrip() {
        let ctx = TestContext;
        let image = sample_image();
        let request = Request {
            operation: "select-subject".into(),
            resource: Some("model.onnx".into()),
            params: json!({"threshold": 0.5}),
            device: Some("npu.0".into()),
            allow_fallback: true,
            image: image.clone(),
            selection: None,
        };
        let mut req_buf = Vec::new();
        protocol::write_request(&mut req_buf, Some(&request), &ctx).unwrap();
        let mut cursor = Cursor::new(req_buf);
        let decoded = protocol::read_request(&mut cursor, &ctx).unwrap().unwrap();
        assert_eq!(decoded.operation, request.operation);
        assert_eq!(decoded.resource, request.resource);
        assert_eq!(decoded.params, request.params);
        assert_eq!(decoded.device, request.device);
        assert_eq!(decoded.allow_fallback, request.allow_fallback);
        assert_eq!(decoded.image.data, request.image.data);

        let mask = Image {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
            channels: 1,
            color_mode: "gray".into(),
            source_depth: 32,
            alpha: false,
            icc_profile: Vec::new(),
            data: vec![0.0, 1.0, 1.0, 0.0],
        };
        let response = Response {
            output: Output { kind: OutputKind::Mask, image: mask },
            execution: Execution { devices: vec!["npu.0".into()], backend: "windows-ai".into(), fallback_reason: None },
        };
        let reply = WorkerReply::Ok(response.clone());
        let mut resp_buf = Vec::new();
        protocol::write_response(&mut resp_buf, &reply, &ctx).unwrap();
        let mut resp_cursor = Cursor::new(resp_buf);
        let read_reply = protocol::read_response(&mut resp_cursor, &ctx).unwrap();
        let WorkerReply::Ok(read_resp) = read_reply else {
            panic!("expected Ok");
        };
        assert_eq!(read_resp.output.kind, response.output.kind);
        assert_eq!(read_resp.output.image.data, response.output.image.data);
        assert_eq!(read_resp.execution.devices, response.execution.devices);
    }

    #[test]
    fn protocol_rejects_invalid_magic_or_trailing_bytes() {
        let ctx = TestContext;
        let mut bad_magic = b"WRONGMAG\x04\0\0\0{}".to_vec();
        assert!(protocol::read_request(&mut Cursor::new(&mut bad_magic), &ctx).is_err());

        let mut good_probe = Vec::new();
        protocol::write_request(&mut good_probe, None, &ctx).unwrap();
        good_probe.push(0xFF);
        assert!(protocol::read_request(&mut Cursor::new(good_probe), &ctx).is_err());
    }

    #[test]
    fn worker_adapter_constructor_validates_path_and_timeout() {
        let descriptor = Descriptor {
            api_version: adapter::API_VERSION,
            id: "org.photocraft.test".into(),
            name: "Test".into(),
            version: "1.0".into(),
            operations: vec![],
            devices: vec![],
            resource_formats: vec![],
            params_schema: json!({}),
        };

        // Relative path rejected
        let rel = PathBuf::from("worker.exe");
        assert!(WorkerAdapter::new(rel, descriptor.clone(), Duration::from_secs(5)).is_err());

        // Timeout too low or too high
        #[cfg(windows)]
        let valid_path = PathBuf::from("C:\\Program Files\\Worker\\worker.exe");
        #[cfg(not(windows))]
        let valid_path = PathBuf::from("/usr/bin/worker");

        assert!(WorkerAdapter::new(valid_path.clone(), descriptor.clone(), Duration::from_millis(500)).is_err());
        assert!(WorkerAdapter::new(valid_path.clone(), descriptor.clone(), Duration::from_millis(400_000)).is_err());

        // Valid constructor succeeds without spawning
        assert!(WorkerAdapter::new(valid_path, descriptor, Duration::from_secs(5)).is_ok());
    }
}
