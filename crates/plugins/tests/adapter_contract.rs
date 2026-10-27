//! Synthetic, pure-Rust contract tests. Fake CPU/GPU/NPU labels prove protocol behavior only;
//! they neither enumerate hardware nor execute a proprietary filter/neural runtime.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use photocraft_plugins::adapter::{
    API_VERSION, Adapter, Context, Descriptor, Device, DeviceKind, Error, Execution, Image, Limits, MAX_ICC_BYTES, Operation, Output, OutputKind, Registry,
    Request, Response, Result,
};
use serde_json::{Value, json};

#[derive(Default)]
struct TestContext {
    canceled: AtomicBool,
    progress: Mutex<Vec<f32>>,
}

impl Context for TestContext {
    fn cancelled(&self) -> bool {
        self.canceled.load(Ordering::Relaxed)
    }

    fn progress(&self, fraction: f32) {
        self.progress.lock().unwrap().push(fraction);
    }
}

fn descriptor() -> Descriptor {
    Descriptor {
        api_version: API_VERSION,
        id: "org.example.synthetic".into(),
        name: "Synthetic community adapter".into(),
        version: "1.0".into(),
        operations: vec![
            Operation {
                id: "filter".into(),
                output_kind: OutputKind::Image,
                color_modes: ["rgb", "gray", "grayscale", "cmyk", "lab", "multichannel"].into_iter().map(String::from).collect(),
            },
            Operation { id: "mask".into(), output_kind: OutputKind::Mask, color_modes: vec!["rgb".into()] },
            Operation { id: "depth".into(), output_kind: OutputKind::Depth, color_modes: vec!["rgb".into()] },
        ],
        devices: vec![
            Device {
                id: "cpu.0".into(),
                kind: DeviceKind::Cpu,
                backend: "fake.cpu".into(),
                name: "Not real CPU discovery".into(),
                operations: vec!["filter".into(), "mask".into(), "depth".into()],
            },
            Device {
                id: "gpu.0".into(),
                kind: DeviceKind::Gpu,
                backend: "fake.gpu".into(),
                name: "Not real GPU execution".into(),
                operations: vec!["filter".into(), "mask".into(), "depth".into()],
            },
            Device {
                id: "npu.0".into(),
                kind: DeviceKind::Npu,
                backend: "fake.npu".into(),
                name: "Not real NPU execution".into(),
                operations: vec!["filter".into()],
            },
        ],
        resource_formats: vec!["original-filter".into(), "opaque-neural-package".into()],
        // Discovery metadata only: the host does not try to implement this invented dialect.
        params_schema: json!({"dialect": "example.invalid", "opaque": {"anything": [1, true]}}),
    }
}

fn image() -> Image {
    Image {
        x: -19,
        y: 37,
        width: 2,
        height: 1,
        channels: 4,
        color_mode: "rgb".into(),
        source_depth: 16,
        alpha: true,
        icc_profile: vec![0, 7, 255, 4],
        data: vec![0.2, 0.4, 0.6, 1.0, 0.7, 0.5, 0.3, 0.8],
    }
}

fn gray(input: &Image, depth: u8, samples: Vec<f32>) -> Image {
    Image {
        x: input.x,
        y: input.y,
        width: input.width,
        height: input.height,
        channels: 1,
        color_mode: "gray".into(),
        source_depth: depth,
        alpha: false,
        icc_profile: Vec::new(),
        data: samples,
    }
}

fn request() -> Request {
    Request {
        operation: "filter".into(),
        resource: None,
        params: json!({}),
        device: Some("cpu.0".into()),
        allow_fallback: false,
        image: image(),
        selection: None,
    }
}

fn response(request: &Request, devices: &[&str], backend: &str) -> Response {
    let (kind, image) = match request.operation.as_str() {
        "mask" => (OutputKind::Mask, gray(&request.image, 32, vec![0.0, 1.0])),
        "depth" => (OutputKind::Depth, gray(&request.image, 32, vec![-8.0, 900.0])),
        _ => (OutputKind::Image, request.image.clone()),
    };
    Response {
        output: Output { kind, image },
        execution: Execution { devices: devices.iter().map(|id| (*id).into()).collect(), backend: backend.into(), fallback_reason: None },
    }
}

struct FakeAdapter {
    descriptor: Descriptor,
    calls: Arc<AtomicUsize>,
    descriptor_calls: Arc<AtomicUsize>,
    output: Option<Response>,
    resource_seen: Arc<Mutex<Option<String>>>,
    cancel_during_run: bool,
    panic_during_run: bool,
    report_progress: bool,
}

impl Default for FakeAdapter {
    fn default() -> Self {
        Self {
            descriptor: descriptor(),
            calls: Arc::new(AtomicUsize::new(0)),
            descriptor_calls: Arc::new(AtomicUsize::new(0)),
            output: None,
            resource_seen: Arc::new(Mutex::new(None)),
            cancel_during_run: false,
            panic_during_run: false,
            report_progress: false,
        }
    }
}

impl Adapter for FakeAdapter {
    fn descriptor(&self) -> Descriptor {
        self.descriptor_calls.fetch_add(1, Ordering::Relaxed);
        self.descriptor.clone()
    }

    fn run(&self, request: &Request, ctx: &dyn Context) -> Result<Response> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.resource_seen.lock().unwrap() = request.resource.clone();
        if self.panic_during_run {
            panic!("synthetic adapter unwind");
        }
        if self.report_progress {
            ctx.progress(-1.0);
            ctx.progress(0.3);
            ctx.progress(f32::NAN);
            ctx.progress(f32::INFINITY);
            ctx.progress(2.0);
        }
        if self.cancel_during_run {
            ctx.progress(0.5);
        }
        if let Some(response) = &self.output {
            return Ok(response.clone());
        }
        let (device, backend) = match request.device.as_deref() {
            Some("gpu.0") => ("gpu.0", "fake.gpu"),
            Some("npu.0") => ("npu.0", "fake.npu"),
            _ => ("cpu.0", "fake.cpu"),
        };
        Ok(response(request, &[device], backend))
    }
}

fn register(adapter: FakeAdapter) -> Registry {
    let mut registry = Registry::default();
    registry.register(Arc::new(adapter)).unwrap();
    registry
}

fn run_with_output(request: &Request, output: Response) -> Result<Response> {
    register(FakeAdapter { output: Some(output), ..FakeAdapter::default() }).run("org.example.synthetic", request, &TestContext::default(), &Limits::default())
}

#[test]
fn registry_is_instance_owned_and_caches_descriptor_for_arc_snapshot() {
    let fake = FakeAdapter::default();
    let descriptor_calls = fake.descriptor_calls.clone();
    let calls = fake.calls.clone();
    let registry = register(fake);
    assert_eq!(registry.list(), vec![descriptor()]);
    assert_eq!(registry.get_descriptor("org.example.synthetic"), Some(&descriptor()));
    assert!(registry.get("missing").is_none());
    assert!(Registry::default().list().is_empty());
    let snapshot = registry.get("org.example.synthetic").unwrap();
    drop(registry);
    assert_eq!(snapshot.descriptor(), &descriptor());
    let output = snapshot.run(&request(), &TestContext::default(), &Limits::default()).unwrap();
    assert_eq!(output.output.image, image());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(descriptor_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn removal_preserves_frozen_snapshots_and_explicit_reregistration_is_allowed() {
    let adapter = Arc::new(FakeAdapter::default());
    let weak = Arc::downgrade(&adapter);
    let mut registry = Registry::default();
    registry.register(adapter.clone()).unwrap();
    drop(adapter);
    let snapshot = registry.get("org.example.synthetic").unwrap();
    assert!(!registry.remove("missing"));
    assert!(registry.remove("org.example.synthetic"));
    assert!(registry.get("org.example.synthetic").is_none());
    assert!(registry.get_descriptor("org.example.synthetic").is_none());
    assert!(registry.list().is_empty());
    assert!(weak.upgrade().is_some());
    snapshot.run(&request(), &TestContext::default(), &Limits::default()).unwrap();
    let mut replacement = FakeAdapter::default();
    replacement.descriptor.version = "2.0".into();
    registry.register(Arc::new(replacement)).unwrap();
    assert_eq!(registry.get_descriptor("org.example.synthetic").unwrap().version, "2.0");
    assert_eq!(snapshot.descriptor().version, "1.0");
    drop(snapshot);
    assert!(weak.upgrade().is_none());
}

#[test]
fn duplicate_and_missing_adapter_return_explicit_errors() {
    let mut registry = register(FakeAdapter::default());
    assert!(matches!(registry.register(Arc::new(FakeAdapter::default())), Err(Error::DuplicateAdapter(_))));
    assert!(matches!(registry.run("missing", &request(), &TestContext::default(), &Limits::default()), Err(Error::NotFound(_))));
    assert_eq!(registry.list().len(), 1);
}

#[test]
fn malformed_descriptors_are_rejected_without_replacement() {
    let mut cases = Vec::new();
    let mut value = descriptor();
    value.api_version = 2;
    cases.push(value);
    let mut value = descriptor();
    value.id.clear();
    cases.push(value);
    let mut value = descriptor();
    value.id = "x/y".into();
    cases.push(value);
    let mut value = descriptor();
    value.name = "\n".into();
    cases.push(value);
    let mut value = descriptor();
    value.version.clear();
    cases.push(value);
    let mut value = descriptor();
    value.operations.clear();
    cases.push(value);
    let mut value = descriptor();
    value.devices.clear();
    cases.push(value);
    let mut value = descriptor();
    value.operations.push(value.operations[0].clone());
    cases.push(value);
    let mut value = descriptor();
    value.operations[0].color_modes.clear();
    cases.push(value);
    let mut value = descriptor();
    value.operations[0].color_modes.push("rgb".into());
    cases.push(value);
    let mut value = descriptor();
    value.devices.push(value.devices[0].clone());
    cases.push(value);
    let mut value = descriptor();
    value.devices[0].operations.push("missing".into());
    cases.push(value);
    let mut value = descriptor();
    value.devices[0].operations.push("filter".into());
    cases.push(value);
    let mut value = descriptor();
    value.devices[0].backend = "a backend with spaces".into();
    cases.push(value);
    let mut value = descriptor();
    value.resource_formats.push("original-filter".into());
    cases.push(value);
    let mut value = descriptor();
    value.params_schema = json!([]);
    cases.push(value);
    let mut value = descriptor();
    value.operations.push(Operation { id: "unsupported".into(), output_kind: OutputKind::Image, color_modes: vec!["rgb".into()] });
    cases.push(value);
    for descriptor in cases {
        let mut registry = Registry::default();
        assert!(registry.register(Arc::new(FakeAdapter { descriptor, ..FakeAdapter::default() })).is_err());
        assert!(registry.list().is_empty());
    }
}

#[test]
fn unknown_operation_color_mode_device_or_unsupported_device_never_invokes_adapter() {
    let fake = FakeAdapter::default();
    let calls = fake.calls.clone();
    let registry = register(fake);
    let mut input = request();
    input.operation = "not-an-operation".into();
    assert!(matches!(registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()), Err(Error::UnsupportedOperation(_))));
    let mut input = request();
    input.image.color_mode = "unknown-format".into();
    assert!(matches!(registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()), Err(Error::UnsupportedColorMode(_))));
    let mut input = request();
    input.device = Some("gpu".into());
    assert!(matches!(registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()), Err(Error::UnknownDevice(_))));
    let mut input = request();
    input.device = Some("npu.0".into());
    input.operation = "mask".into();
    input.allow_fallback = true;
    assert!(matches!(registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()), Err(Error::Device(_))));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn checked_size_preflight_rejects_overflow_pixels_samples_channels_and_zero() {
    let limits = Limits::default();
    assert_eq!(limits.check_image_size(2, 3, 4).unwrap(), 24);
    for (width, height, channels) in [(0, 1, 4), (1, 0, 4), (1, 1, 0), (1, 1, 65), (u32::MAX, u32::MAX, 64)] {
        assert!(limits.check_image_size(width, height, channels).is_err());
    }
    let limits = Limits { max_pixels: 1, ..Limits::default() };
    assert!(limits.check_image_size(2, 1, 1).is_err());
    let limits = Limits { max_samples: 3, ..Limits::default() };
    assert!(limits.check_image_size(1, 1, 4).is_err());
    // Remove configured caps: multiplication/address-space overflow must still be rejected.
    let limits = Limits { max_pixels: usize::MAX, max_samples: usize::MAX, ..Limits::default() };
    assert!(limits.check_image_size(u32::MAX, u32::MAX, 64).is_err());
    assert!(limits.check_image_size(u32::MAX, u32::MAX, 1).is_err());
}

#[test]
fn malformed_image_layout_depth_and_nonfinite_samples_are_rejected_before_run() {
    let fake = FakeAdapter::default();
    let calls = fake.calls.clone();
    let registry = register(fake);
    let mut images = Vec::new();
    let mut value = image();
    value.data.pop();
    images.push(value);
    let mut value = image();
    value.data.push(0.0);
    images.push(value);
    let mut value = image();
    value.channels = 0;
    images.push(value);
    let mut value = image();
    value.channels = 65;
    images.push(value);
    let mut value = image();
    value.source_depth = 12;
    images.push(value);
    let mut value = image();
    value.width = u32::MAX;
    value.height = u32::MAX;
    images.push(value);
    let mut value = image();
    value.data[0] = f32::NAN;
    images.push(value);
    let mut value = image();
    value.source_depth = 32;
    value.data[0] = f32::INFINITY;
    images.push(value);
    let mut value = image();
    value.data[0] = -0.1;
    images.push(value);
    let mut value = image();
    value.data[0] = 1.1;
    images.push(value);
    let mut value = image();
    value.source_depth = 32;
    value.data[3] = 1.1;
    images.push(value);
    for image in images {
        let mut input = request();
        input.image = image;
        assert!(registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()).is_err());
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn geometry_coordinate_endpoints_are_checked_and_negative_origins_are_valid() {
    let limits = Limits::default();
    let mut valid = image();
    valid.x = i32::MIN;
    valid.y = -37;
    valid.validate(&limits).unwrap();
    let mut valid = image();
    valid.x = i32::MAX - 2;
    valid.y = i32::MAX - 1;
    valid.validate(&limits).unwrap();
    let mut invalid = valid.clone();
    invalid.x = i32::MAX - 1;
    assert!(matches!(invalid.validate(&limits), Err(Error::InvalidImage(_))));
    let mut invalid = valid;
    invalid.y = i32::MAX;
    assert!(matches!(invalid.validate(&limits), Err(Error::InvalidImage(_))));
    let mut invalid = image();
    invalid.width = i32::MAX as u32 + 1;
    let permissive = Limits { max_pixels: usize::MAX, max_samples: usize::MAX, ..limits };
    assert!(invalid.validate(&permissive).is_err());
}

#[test]
fn icc_limit_is_hard_even_when_caller_raises_it() {
    let mut input = request();
    input.image.icc_profile = vec![0; MAX_ICC_BYTES + 1];
    let limits = Limits { max_icc_bytes: usize::MAX, ..Limits::default() };
    assert!(matches!(input.image.validate(&limits), Err(Error::Limit(_))));
    input.image.icc_profile.truncate(MAX_ICC_BYTES);
    input.image.validate(&limits).unwrap();
    let limits = Limits { max_icc_bytes: 2, ..Limits::default() };
    assert!(matches!(image().validate(&limits), Err(Error::Limit(_))));
}

#[test]
fn rgb_gray_cmyk_lab_multichannel_and_8_16_32f_preserve_metadata() {
    let registry = register(FakeAdapter::default());
    for (mode, channels) in [("rgb", 4), ("gray", 2), ("grayscale", 2), ("cmyk", 5), ("lab", 4), ("multichannel", 64)] {
        for depth in [8, 16, 32] {
            let mut input = request();
            input.image.color_mode = mode.into();
            input.image.channels = channels;
            input.image.source_depth = depth;
            input.image.data = vec![0.5; 2 * usize::from(channels)];
            if depth == 32 {
                input.image.data[0] = -3.0;
                input.image.data[1] = if channels == 2 { 0.4 } else { 12.0 };
            }
            let result = registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()).unwrap();
            assert_eq!(result.output.image, input.image);
        }
    }
}

#[test]
fn selection_must_match_origin_extent_and_be_one_channel_gray_normalized() {
    let registry = register(FakeAdapter::default());
    let mut input = request();
    input.selection = Some(gray(&input.image, 32, vec![0.25, 0.8]));
    registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()).unwrap();
    let valid = input.selection.clone().unwrap();
    let mut selections = Vec::new();
    let mut value = valid.clone();
    value.x += 1;
    selections.push(value);
    let mut value = valid.clone();
    value.y += 1;
    selections.push(value);
    let mut value = valid.clone();
    value.width = 1;
    value.height = 2;
    selections.push(value);
    let mut value = valid.clone();
    value.color_mode = "rgb".into();
    selections.push(value);
    let mut value = valid.clone();
    value.channels = 2;
    value.data = vec![0.5; 4];
    selections.push(value);
    let mut value = valid.clone();
    value.alpha = true;
    selections.push(value);
    let mut value = valid.clone();
    value.icc_profile = vec![1];
    selections.push(value);
    let mut value = valid.clone();
    value.data[0] = -0.1;
    selections.push(value);
    let mut value = valid.clone();
    value.data[0] = 1.1;
    selections.push(value);
    let mut value = valid;
    value.data[0] = f32::NAN;
    selections.push(value);
    for selection in selections {
        input.selection = Some(selection);
        assert!(registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()).is_err());
    }
}

#[test]
fn image_output_must_preserve_kind_geometry_profile_and_all_format_metadata() {
    let input = request();
    let original = response(&input, &["cpu.0"], "fake.cpu");
    let mut cases = Vec::new();
    let mut value = original.clone();
    value.output.kind = OutputKind::Mask;
    cases.push(value);
    let mut value = original.clone();
    value.output.image.x += 1;
    cases.push(value);
    let mut value = original.clone();
    value.output.image.y -= 1;
    cases.push(value);
    let mut value = original.clone();
    value.output.image.width = 1;
    value.output.image.height = 2;
    cases.push(value);
    let mut value = original.clone();
    value.output.image.color_mode = "cmyk".into();
    cases.push(value);
    let mut value = original.clone();
    value.output.image.source_depth = 8;
    cases.push(value);
    let mut value = original.clone();
    value.output.image.channels = 8;
    value.output.image.width = 1;
    cases.push(value);
    let mut value = original.clone();
    value.output.image.alpha = false;
    cases.push(value);
    let mut value = original.clone();
    value.output.image.icc_profile.clear();
    cases.push(value);
    let mut value = original.clone();
    value.output.image.icc_profile[0] = 9;
    cases.push(value);
    let mut value = original.clone();
    value.output.image.data[0] = f32::NAN;
    cases.push(value);
    let mut value = original;
    value.output.image.data.pop();
    cases.push(value);
    for output in cases {
        assert!(run_with_output(&input, output).is_err());
    }
}

#[test]
fn mask_and_depth_output_contracts_are_distinct_from_image_output() {
    let registry = register(FakeAdapter::default());
    for operation in ["mask", "depth"] {
        let mut input = request();
        input.operation = operation.into();
        let valid = registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()).unwrap();
        assert_eq!(valid.output.image.source_depth, 32);
        assert_eq!(valid.output.image.channels, 1);
        assert_eq!(valid.output.image.icc_profile, Vec::<u8>::new());
        let mut invalid = valid.clone();
        invalid.output.image.icc_profile = vec![1];
        assert!(run_with_output(&input, invalid).is_err());
        let mut invalid = valid.clone();
        invalid.output.image.source_depth = 16;
        assert!(run_with_output(&input, invalid).is_err());
        let mut invalid = valid.clone();
        invalid.output.image.color_mode = "rgb".into();
        assert!(run_with_output(&input, invalid).is_err());
        let mut invalid = valid.clone();
        invalid.output.image.channels = 2;
        invalid.output.image.alpha = true;
        invalid.output.image.data = vec![0.5; 4];
        assert!(run_with_output(&input, invalid).is_err());
        let mut invalid = valid;
        invalid.output.kind = OutputKind::Image;
        assert!(run_with_output(&input, invalid).is_err());
    }
    let mut input = request();
    input.operation = "mask".into();
    for sample in [-0.001, 1.001, f32::NAN, f32::INFINITY] {
        let mut invalid = response(&input, &["cpu.0"], "fake.cpu");
        invalid.output.image.data[0] = sample;
        assert!(run_with_output(&input, invalid).is_err());
    }
    let mut input = request();
    input.operation = "depth".into();
    let result = registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()).unwrap();
    assert_eq!(result.output.image.data, vec![-8.0, 900.0]);
}

#[test]
fn resource_is_opaque_and_passed_exactly_without_path_or_format_interpretation() {
    let fake = FakeAdapter::default();
    let seen = fake.resource_seen.clone();
    let registry = register(fake);
    for resource in [
        None,
        Some(""),
        Some("  C:\\missing\\..\\Original.8BF\0?format=unknown  "),
        Some("affinity://未修改/包#原始"),
        Some("https://invalid.example/package?x=1"),
    ] {
        let mut input = request();
        input.resource = resource.map(str::to_owned);
        input.params = json!({"unknown.original.option": [true, 123, "preserved"]});
        registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()).unwrap();
        assert_eq!(*seen.lock().unwrap(), input.resource);
    }
}

#[test]
fn oversized_resource_is_rejected_without_calling_or_rewriting_it() {
    let fake = FakeAdapter::default();
    let calls = fake.calls.clone();
    let registry = register(fake);
    let mut input = request();
    input.resource = Some("12345".into());
    let limits = Limits { max_resource_bytes: 4, ..Limits::default() };
    assert!(matches!(registry.run("org.example.synthetic", &input, &TestContext::default(), &limits), Err(Error::Limit(_))));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert_eq!(input.resource.as_deref(), Some("12345"));
}

#[test]
fn fake_cpu_gpu_npu_devices_are_exact_ids_and_only_self_reported_metadata() {
    let registry = register(FakeAdapter::default());
    for (device, backend, kind) in [("cpu.0", "fake.cpu", DeviceKind::Cpu), ("gpu.0", "fake.gpu", DeviceKind::Gpu), ("npu.0", "fake.npu", DeviceKind::Npu)] {
        let mut input = request();
        input.device = Some(device.into());
        let result = registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()).unwrap();
        assert_eq!(result.execution.devices, vec![device.to_owned()]);
        assert_eq!(result.execution.backend, backend);
        assert_eq!(registry.get_descriptor("org.example.synthetic").unwrap().devices.iter().find(|entry| entry.id == device).unwrap().kind, kind);
    }
}

#[test]
fn strict_explicit_device_rejects_silent_or_explained_fallback_unless_allowed() {
    let mut input = request();
    input.device = Some("gpu.0".into());
    let mut cpu = response(&input, &["cpu.0"], "fake.cpu");
    assert!(matches!(run_with_output(&input, cpu.clone()), Err(Error::Device(_))));
    cpu.execution.fallback_reason = Some("fake GPU backend declined this request".into());
    assert!(matches!(run_with_output(&input, cpu.clone()), Err(Error::Device(_))));
    input.allow_fallback = true;
    let result = run_with_output(&input, cpu.clone()).unwrap();
    assert_eq!(result.execution.devices, vec!["cpu.0".to_owned()]);
    assert_eq!(result.execution.fallback_reason, cpu.execution.fallback_reason);
    cpu.execution.fallback_reason = None;
    assert!(matches!(run_with_output(&input, cpu.clone()), Err(Error::Device(_))));
    cpu.execution.fallback_reason = Some(" ".into());
    assert!(matches!(run_with_output(&input, cpu), Err(Error::Device(_))));
}

#[test]
fn actual_devices_support_operation_and_backend_reports_cannot_masquerade_as_validation() {
    let input = request();
    for (devices, backend) in [
        (vec![], "fake.cpu"),
        (vec!["invented"], "fake.cpu"),
        (vec!["cpu.0", "cpu.0"], "fake.cpu"),
        (vec!["cpu.0"], "real.physical.gpu.verified"),
        (vec!["cpu.0"], "mixed"),
    ] {
        assert!(run_with_output(&input, response(&input, &devices, backend)).is_err());
    }
    let mut input = request();
    input.operation = "mask".into();
    input.device = None;
    assert!(run_with_output(&input, response(&input, &["npu.0"], "fake.npu")).is_err());
}

#[test]
fn mixed_execution_requires_all_declared_devices_backend_and_explicit_fallback_permission() {
    let mut input = request();
    input.device = Some("gpu.0".into());
    let mut mixed = response(&input, &["gpu.0", "cpu.0"], "mixed");
    assert!(run_with_output(&input, mixed.clone()).is_err());
    input.allow_fallback = true;
    assert!(run_with_output(&input, mixed.clone()).is_err());
    mixed.execution.fallback_reason = Some("fake mixed execution".into());
    run_with_output(&input, mixed.clone()).unwrap();
    mixed.execution.backend = "fake.gpu".into();
    assert!(run_with_output(&input, mixed).is_err());
    // No explicit device means adapter scheduling is permitted, not a silent replacement.
    input.device = None;
    input.allow_fallback = false;
    run_with_output(&input, response(&input, &["gpu.0", "cpu.0"], "mixed")).unwrap();
    // A reason with an unchanged explicit device would be contradictory.
    input.device = Some("cpu.0".into());
    input.allow_fallback = true;
    let mut output = response(&input, &["cpu.0"], "fake.cpu");
    output.execution.fallback_reason = Some("no change occurred".into());
    assert!(run_with_output(&input, output).is_err());
}

#[test]
fn multiple_devices_with_a_common_declared_backend_report_that_backend() {
    let mut fake = FakeAdapter::default();
    let mut second = fake.descriptor.devices[0].clone();
    second.id = "cpu.1".into();
    fake.descriptor.devices.push(second);
    let mut input = request();
    input.device = None;
    fake.output = Some(response(&input, &["cpu.0", "cpu.1"], "fake.cpu"));
    let registry = register(fake);
    let output = registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()).unwrap();
    assert_eq!(output.execution.devices, vec!["cpu.0".to_owned(), "cpu.1".to_owned()]);
    assert_eq!(output.execution.backend, "fake.cpu");
}

#[test]
fn params_are_objects_and_json_depth_nodes_containers_bytes_are_bounded_iteratively() {
    let fake = FakeAdapter::default();
    let calls = fake.calls.clone();
    let registry = register(fake);
    let mut input = request();
    input.params = json!([1]);
    assert!(matches!(registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()), Err(Error::InvalidParams(_))));
    let mut cases = Vec::new();
    cases.push((json!({"a": 1, "b": 2}), Limits { max_json_object_entries: 1, ..Limits::default() }));
    cases.push((json!({"a": [1, 2]}), Limits { max_json_array_items: 1, ..Limits::default() }));
    cases.push((json!({"a": [1, 2]}), Limits { max_json_nodes: 3, ..Limits::default() }));
    cases.push((json!({"a": {"b": {}}}), Limits { max_json_depth: 2, ..Limits::default() }));
    cases.push((json!({"a": "long string"}), Limits { max_json_bytes: 4, ..Limits::default() }));
    for (params, limits) in cases {
        input.params = params;
        assert!(matches!(limits.check_params(&input.params), Err(Error::Limit(_))));
        assert!(matches!(registry.run("org.example.synthetic", &input, &TestContext::default(), &limits), Err(Error::Limit(_))));
    }
    // Much deeper than Serde's default recursion budget, built without recursive parsing.
    let mut nested = Value::Null;
    for _ in 0..512 {
        nested = Value::Array(vec![nested]);
    }
    let mut object = serde_json::Map::new();
    object.insert("deep".into(), nested);
    input.params = Value::Object(object);
    assert!(matches!(registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()), Err(Error::Limit(_))));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn json_byte_accounting_matches_compact_serde_for_escapes_unicode_and_scalars() {
    let registry = register(FakeAdapter::default());
    let mut input = request();
    input.params = json!({"\"key\\\n": [null, false, true, -1.25, 200, "\u{0000}\n\r\t\u{0008}\u{000c}\\\"汉字"]});
    let exact_bytes = serde_json::to_vec(&input.params).unwrap().len();
    let limits = Limits { max_json_bytes: exact_bytes, ..Limits::default() };
    limits.check_params(&input.params).unwrap();
    registry.run("org.example.synthetic", &input, &TestContext::default(), &limits).unwrap();
    let limits = Limits { max_json_bytes: exact_bytes - 1, ..Limits::default() };
    assert!(matches!(limits.check_params(&input.params), Err(Error::Limit(_))));
    assert!(matches!(registry.run("org.example.synthetic", &input, &TestContext::default(), &limits), Err(Error::Limit(_))));
}

#[test]
fn descriptor_schema_is_opaque_but_object_and_bounded_without_recursive_serialization() {
    let mut value = descriptor();
    value.params_schema = json!({"unknown-dialect": {"not-json-schema": 123}});
    let registry = register(FakeAdapter { descriptor: value, ..FakeAdapter::default() });
    registry.run("org.example.synthetic", &request(), &TestContext::default(), &Limits::default()).unwrap();
    struct DeepSchema;
    impl Adapter for DeepSchema {
        fn descriptor(&self) -> Descriptor {
            let mut value = Value::Null;
            for _ in 0..20_000 {
                value = Value::Array(vec![value]);
            }
            let mut result = descriptor();
            let mut schema = serde_json::Map::new();
            schema.insert("deep".into(), value);
            result.params_schema = Value::Object(schema);
            result
        }
        fn run(&self, _request: &Request, _ctx: &dyn Context) -> Result<Response> {
            Err(Error::Failed("must never be called".into()))
        }
    }
    let mut registry = Registry::default();
    assert!(matches!(registry.register(Arc::new(DeepSchema)), Err(Error::InvalidDescriptor(_))));
    assert!(registry.list().is_empty());
}

#[test]
fn cancellation_before_run_skips_adapter_and_after_run_discards_its_result() {
    let fake = FakeAdapter::default();
    let calls = fake.calls.clone();
    let registry = register(fake);
    let context = TestContext::default();
    context.canceled.store(true, Ordering::Relaxed);
    assert!(matches!(registry.run("org.example.synthetic", &request(), &context, &Limits::default()), Err(Error::Canceled)));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    struct CancelOnProgress {
        canceled: AtomicBool,
    }
    impl Context for CancelOnProgress {
        fn cancelled(&self) -> bool {
            self.canceled.load(Ordering::Relaxed)
        }
        fn progress(&self, _fraction: f32) {
            self.canceled.store(true, Ordering::Relaxed);
        }
    }
    let registry = register(FakeAdapter { cancel_during_run: true, ..FakeAdapter::default() });
    let context = CancelOnProgress { canceled: AtomicBool::new(false) };
    assert!(matches!(registry.run("org.example.synthetic", &request(), &context, &Limits::default()), Err(Error::Canceled)));
}

#[test]
fn unwinding_adapter_panic_is_an_error_and_registry_stays_usable() {
    let mut registry = register(FakeAdapter { panic_during_run: true, ..FakeAdapter::default() });
    assert!(matches!(registry.run("org.example.synthetic", &request(), &TestContext::default(), &Limits::default()), Err(Error::Panicked)));
    let mut healthy = FakeAdapter::default();
    healthy.descriptor.id = "org.example.healthy".into();
    registry.register(Arc::new(healthy)).unwrap();
    registry.run("org.example.healthy", &request(), &TestContext::default(), &Limits::default()).unwrap();
}

#[test]
fn unwinding_descriptor_panic_is_caught_too() {
    struct BadDescriptor;
    impl Adapter for BadDescriptor {
        fn descriptor(&self) -> Descriptor {
            panic!("synthetic descriptor unwind");
        }
        fn run(&self, _request: &Request, _ctx: &dyn Context) -> Result<Response> {
            Err(Error::Failed("not called".into()))
        }
    }
    let mut registry = Registry::default();
    assert_eq!(registry.register(Arc::new(BadDescriptor)), Err(Error::Panicked));
    assert!(registry.list().is_empty());
}

#[test]
fn progress_callback_receives_only_finite_clamped_fractions() {
    let registry = register(FakeAdapter { report_progress: true, ..FakeAdapter::default() });
    let context = TestContext::default();
    registry.run("org.example.synthetic", &request(), &context, &Limits::default()).unwrap();
    assert_eq!(*context.progress.lock().unwrap(), vec![0.0, 0.3, 1.0]);
}

#[test]
fn serde_round_trip_and_missing_allow_fallback_is_strict_by_default() {
    let original = request();
    let encoded = serde_json::to_value(&original).unwrap();
    assert_eq!(serde_json::from_value::<Request>(encoded.clone()).unwrap(), original);
    let mut legacy = encoded;
    legacy.as_object_mut().unwrap().remove("allow_fallback");
    assert!(!serde_json::from_value::<Request>(legacy).unwrap().allow_fallback);
    let mut minimal = serde_json::to_value(&original).unwrap();
    for field in ["params", "resource", "device", "selection", "allow_fallback"] {
        minimal.as_object_mut().unwrap().remove(field);
    }
    let defaults = serde_json::from_value::<Request>(minimal).unwrap();
    assert_eq!(defaults.params, json!({}));
    assert_eq!(defaults.device, None);
    assert!(!defaults.allow_fallback);
    let declaration = descriptor();
    let encoded = serde_json::to_string(&declaration).unwrap();
    assert!(encoded.contains("\"kind\":\"gpu\""));
    assert!(encoded.contains("\"output_kind\":\"mask\""));
    assert_eq!(serde_json::from_str::<Descriptor>(&encoded).unwrap(), declaration);
    let output = response(&original, &["cpu.0"], "fake.cpu");
    assert_eq!(serde_json::from_str::<Response>(&serde_json::to_string(&output).unwrap()).unwrap(), output);
    assert_eq!(serde_json::from_str::<Limits>(&serde_json::to_string(&Limits::default()).unwrap()).unwrap(), Limits::default());
}

#[test]
fn adapter_specific_failure_propagates_without_turning_into_success() {
    struct FailingAdapter;
    impl Adapter for FailingAdapter {
        fn descriptor(&self) -> Descriptor {
            descriptor()
        }
        fn run(&self, _request: &Request, _ctx: &dyn Context) -> Result<Response> {
            Err(Error::Failed("synthetic original package unsupported".into()))
        }
    }
    let mut registry = Registry::default();
    registry.register(Arc::new(FailingAdapter)).unwrap();
    assert_eq!(
        registry.run("org.example.synthetic", &request(), &TestContext::default(), &Limits::default()),
        Err(Error::Failed("synthetic original package unsupported".into()))
    );
}

#[test]
fn excessively_long_unknown_identifiers_produce_bounded_errors() {
    let registry = register(FakeAdapter::default());
    let long = "x".repeat(1024 * 1024);
    let error = registry.run(&long, &request(), &TestContext::default(), &Limits::default()).unwrap_err();
    assert!(error.to_string().len() < 256);
    let mut input = request();
    input.operation = long.clone();
    let error = registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()).unwrap_err();
    assert!(error.to_string().len() < 256);
    input.operation = "filter".into();
    input.device = Some(long);
    let error = registry.run("org.example.synthetic", &input, &TestContext::default(), &Limits::default()).unwrap_err();
    assert!(error.to_string().len() < 256);
}

#[test]
fn capability_list_and_schema_node_caps_are_enforced_at_registration() {
    let mut cases = Vec::new();
    let mut value = descriptor();
    value.resource_formats = (0..129).map(|index| format!("format-{index}")).collect();
    cases.push(value);
    let mut value = descriptor();
    value.name = "x".repeat(1025);
    cases.push(value);
    let mut value = descriptor();
    let mut object = serde_json::Map::new();
    for index in 0..1025 {
        object.insert(format!("key{index}"), Value::Null);
    }
    value.params_schema = Value::Object(object);
    cases.push(value);
    let mut value = descriptor();
    value.params_schema = json!({"bytes": "x".repeat(1024 * 1024)});
    cases.push(value);
    for descriptor in cases {
        let mut registry = Registry::default();
        assert!(matches!(registry.register(Arc::new(FakeAdapter { descriptor, ..FakeAdapter::default() })), Err(Error::InvalidDescriptor(_))));
    }
}

#[test]
fn registry_and_registration_are_send_sync_without_runtime_types() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Registry>();
    assert_send_sync::<photocraft_plugins::adapter::RegisteredAdapter>();
}
