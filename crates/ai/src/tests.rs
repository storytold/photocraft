use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use crate::fake::FakeProvider;
use crate::provider::Stage;
use crate::*;

struct Noop;
impl Wake for Noop {
    fn wake(self: Arc<Self>) {}
}

fn ready<T>(future: impl Future<Output = T>) -> T {
    let waker = Waker::from(Arc::new(Noop));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("fake future unexpectedly pending"),
    }
}

fn request() -> AiRequest {
    AiRequest {
        operation: Operation::Generate,
        workflow: WorkflowId { name: "test".into(), revision: 1 },
        prompt: "red mug".into(),
        negative_prompt: None,
        size: ImageSize { width: 4, height: 3 },
        variants: 1,
        seed: 42,
        source: None,
        edit_mask: None,
        references: Vec::new(),
    }
}

#[test]
fn pipeline_submits_once_and_reports_ready() {
    let provider = FakeProvider::default();
    let control = Control::default();
    let result = ready(generate(&request(), &provider, &control)).unwrap();
    assert_eq!(provider.submissions(), 1);
    assert_eq!(control.progress(), (Stage::Ready, 1.0));
    assert_eq!(result.artifacts[0].size(), ImageSize { width: 4, height: 3 });
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_pipeline_future_can_run_on_an_injected_send_runtime() {
    fn require_send(_: impl Send) {}
    let r = request();
    let provider = FakeProvider::default();
    let control = Control::default();
    require_send(generate(&r, &provider, &control));
}

#[test]
fn a_cancelled_job_never_submits_an_image() {
    let provider = FakeProvider::default();
    let control = Control::default();
    control.cancel();
    assert!(matches!(ready(generate(&request(), &provider, &control)), Err(AiError::Cancelled)));
    assert_eq!(provider.submissions(), 0);
}

#[test]
fn invalid_requests_are_rejected_before_submission() {
    let provider = FakeProvider::default();
    for variant in 0..7 {
        let mut r = request();
        match variant {
            0 => r.size.width = 0,
            1 => r.size.height = u32::MAX,
            2 => r.variants = u32::MAX,
            3 => r.variants = 0,
            4 => r.prompt = "x".repeat(MAX_PROMPT_BYTES + 1),
            5 => r.workflow.revision = 0,
            _ => r.operation = Operation::Edit,
        }
        assert!(ready(generate(&r, &provider, &Control::default())).is_err());
    }
    assert_eq!(provider.submissions(), 0);
}

#[test]
fn images_and_masks_reject_bad_lengths_nonfinite_values_and_empty_edits() {
    let size = ImageSize { width: 2, height: 1 };
    assert!(RgbaImage::new(size, vec![0.0; 4]).is_err());
    assert!(RgbaImage::new(size, vec![f32::INFINITY; 8]).is_err());
    assert!(Mask::new(size, vec![f32::NAN, 0.0]).is_err());
    assert!(Mask::new(size, vec![-0.1, 1.0]).is_err());
    let mut r = request();
    r.operation = Operation::Remove;
    r.size = size;
    r.source = Some(RgbaImage::new(size, vec![1.0; 8]).unwrap());
    let mut caps = FakeProvider::default().capabilities();
    caps.operations.push(Operation::Remove);
    caps.mask_mode = MaskMode::LocalComposite;
    assert!(r.validate(&caps).is_err());
    r.edit_mask = Some(Mask::new(size, vec![0.0; 2]).unwrap());
    assert!(r.validate(&caps).is_err());
    r.edit_mask = Some(Mask::new(size, vec![0.25, 1.0]).unwrap());
    assert!(r.validate(&caps).is_ok());
}

#[test]
fn artifacts_enforce_limits_without_evicting_or_replacing_results() {
    let provider = FakeProvider::default();
    let r = request();
    let result = ready(generate(&r, &provider, &Control::default())).unwrap();
    let bytes = result.bytes().unwrap();
    let mut store = ArtifactStore::new(bytes, 1);
    store.insert(1, result.clone()).unwrap();
    assert_eq!(store.bytes(), bytes);
    assert!(store.insert(1, result.clone()).is_err());
    assert_eq!(store.insert(2, result.clone()), Err(AiError::StorageFull));
    assert!(store.get(1).is_some());
    assert!(store.discard(1));
    assert!(!store.discard(1));
    assert_eq!(store.bytes(), 0);
    store.insert(2, result).unwrap();
    assert_eq!(store.ids().collect::<Vec<_>>(), vec![2]);
}
