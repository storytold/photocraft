use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use serde_json::json;

use crate::fake::FakeProvider;
use crate::provider::Stage;
use crate::workflow::*;
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
        prompt_preparation: PromptPreparation::OpenAiText,
    }
}

#[test]
fn automatic_prompt_pipeline_preserves_provenance_and_runs_once() {
    let provider = FakeProvider::default();
    let control = Control::default();
    let result = ready(generate(&request(), &provider, Some(&provider), &control)).unwrap();
    assert_eq!(provider.preparations(), 1);
    assert_eq!(provider.submissions(), 1);
    assert_eq!(result.prepared_prompt.unwrap().positive, "Test prompt: red mug");
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
    require_send(generate(&r, &provider, Some(&provider), &control));
}

struct CancelPrompt;
impl PromptProvider for CancelPrompt {
    fn prepare<'a>(&'a self, request: &'a AiRequest, _caps: &'a ProviderCapabilities, control: &'a Control) -> ProviderFuture<'a, PreparedPrompt> {
        Box::pin(async move {
            control.cancel();
            Ok(PreparedPrompt {
                original: request.prompt.clone(),
                positive: "prepared".into(),
                negative: None,
                workflow: request.workflow.clone(),
                text_model: None,
            })
        })
    }
}

struct RejectPrompt;
impl PromptProvider for RejectPrompt {
    fn prepare<'a>(&'a self, _request: &'a AiRequest, _caps: &'a ProviderCapabilities, _control: &'a Control) -> ProviderFuture<'a, PreparedPrompt> {
        Box::pin(async { Err(AiError::Rejected) })
    }
}

#[test]
fn cancellation_or_prompt_failure_never_submits_an_image() {
    let provider = FakeProvider::default();
    assert!(matches!(ready(generate(&request(), &provider, Some(&CancelPrompt), &Control::default())), Err(AiError::Cancelled)));
    assert!(matches!(ready(generate(&request(), &provider, Some(&RejectPrompt), &Control::default())), Err(AiError::Rejected)));
    assert!(matches!(ready(generate(&request(), &provider, None, &Control::default())), Err(AiError::Unavailable)));
    assert_eq!(provider.submissions(), 0);
}

#[test]
fn invalid_requests_are_rejected_before_prompt_preparation() {
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
        assert!(ready(generate(&r, &provider, Some(&provider), &Control::default())).is_err());
    }
    assert_eq!(provider.preparations(), 0);
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
fn prepared_prompts_cannot_change_workflow_or_add_unsupported_fields() {
    let r = request();
    let mut caps = FakeProvider::default().capabilities();
    let mut p = PreparedPrompt { original: r.prompt.clone(), positive: "mug".into(), negative: None, workflow: r.workflow.clone(), text_model: None };
    p.workflow.revision = 2;
    assert!(p.validate(&r, &caps).is_err());
    p.workflow = r.workflow.clone();
    p.negative = Some("noise".into());
    caps.negative_prompt = false;
    assert!(p.validate(&r, &caps).is_err());
}

#[test]
fn artifacts_enforce_limits_without_evicting_or_replacing_results() {
    let provider = FakeProvider::default();
    let r = request();
    let result = ready(generate(&r, &provider, Some(&provider), &Control::default())).unwrap();
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

fn package() -> WorkflowPackage {
    WorkflowPackage {
        schema_version: 1,
        id: WorkflowId { name: "test".into(), revision: 1 },
        graph: serde_json::from_value(json!({
            "1": {"class_type":"Generator","inputs":{"model":"local.safetensors","prompt":"placeholder"}},
            "2": {"class_type":"SaveImage","inputs":{"images":["1",0],"filename_prefix":"placeholder"}}
        }))
        .unwrap(),
        required_nodes: BTreeSet::from(["Generator".into(), "SaveImage".into()]),
        models: vec![ModelBinding { node: "1".into(), input: "model".into(), filename: "local.safetensors".into() }],
        bindings: vec![
            InputBinding { key: InputKey::Prompt, node: "1".into(), input: "prompt".into() },
            InputBinding { key: InputKey::OutputPrefix, node: "2".into(), input: "filename_prefix".into() },
        ],
        outputs: BTreeSet::from(["2".into()]),
    }
}

#[test]
fn bindings_do_not_treat_prompts_as_graphs_and_require_installed_models() {
    let p = package();
    let prompt = "{\"class_type\":\"DeleteFiles\"}";
    let bindings = BTreeMap::from([(InputKey::Prompt, json!(prompt)), (InputKey::OutputPrefix, json!("own-job"))]);
    let graph = p.bind(&bindings).unwrap();
    assert_eq!(graph["1"].inputs["prompt"], prompt);
    assert_eq!(graph.len(), 2);
    let server = json!({"Generator":{"input":{"required":{"model":[["local.safetensors"]]}}},"SaveImage":{}});
    p.validate_server(&server).unwrap();
    assert!(p.validate_server(&json!({"Generator":{}, "SaveImage":{}})).is_err());
    assert!(p.validate_server(&json!({"Generator":{}})).is_err());
    let mut bad = bindings;
    bad.insert(InputKey::OutputPrefix, json!("../../other"));
    assert!(p.bind(&bad).is_err());
}

#[test]
fn graphs_reject_cycles_dangling_edges_and_duplicate_bindings() {
    let mut p = package();
    p.graph.get_mut("1").unwrap().inputs.insert("cycle".into(), json!(["2", 0]));
    assert!(p.validate().is_err());
    let mut p = package();
    p.graph.get_mut("2").unwrap().inputs.insert("images".into(), json!(["missing", 0]));
    assert!(p.validate().is_err());
    let mut p = package();
    p.bindings.push(p.bindings[0].clone());
    assert!(p.validate().is_err());
    let mut p = package();
    p.bindings[0].input = "model".into();
    assert!(p.validate().is_err());
    assert!(WorkflowPackage::from_json(&vec![b' '; 256 * 1024 + 1]).is_err());
}

#[test]
fn prompt_model_ids_preserve_self_hosted_tags_routes_and_json_escaping() {
    let r = request();
    let caps = FakeProvider::default().capabilities();
    for model in ["qwen3.6:35b@GPU-HOST", "organization/Qwen3.6-35B:Q4_K_M", "gpt-4.1-mini", "model\"\\variant"] {
        let body = openai_prompt::request_body(model, &r, &caps).unwrap();
        let wire = serde_json::to_vec(&body).unwrap();
        let decoded: serde_json::Value = serde_json::from_slice(&wire).unwrap();
        assert_eq!(decoded["model"], model);
        assert_eq!(decoded["store"], false);
        let response = serde_json::to_vec(&json!({"status":"completed","output":[{"type":"message","status":"completed","content":[
            {"type":"output_text","text":"{\"positive_prompt\":\"an astronaut at a launchpad\",\"negative_prompt\":null}"}
        ]}]}))
        .unwrap();
        let result = openai_prompt::parse_response(model, &response, &r, &caps).unwrap();
        assert_eq!(result.text_model.as_deref(), Some(model));
    }
    for model in ["", " ", "qwen 35b", "qwen\n35b", "qwen\t35b", "qwen\0id", "qwen\u{00a0}id", "qwen\u{007f}id", &"x".repeat(129)] {
        assert!(openai_prompt::request_body(model, &r, &caps).is_err(), "{model:?}");
    }
}

#[test]
fn openai_body_is_strict_text_only_and_response_requires_both_fields() {
    let r = request();
    let caps = FakeProvider::default().capabilities();
    let body = openai_prompt::request_body("configured-text-model", &r, &caps).unwrap();
    assert_eq!(body["store"], false);
    assert_eq!(body["text"]["format"]["strict"], true);
    assert_eq!(body["text"]["format"]["schema"]["additionalProperties"], false);
    assert!(body["input"][1]["content"].is_string());
    let response = |fields: serde_json::Value| {
        serde_json::to_vec(
            &json!({"status":"completed","output":[{"type":"message","status":"completed","content":[{"type":"output_text","text":fields.to_string()}]}]}),
        )
        .unwrap()
    };
    let parsed =
        openai_prompt::parse_response("configured-text-model", &response(json!({"positive_prompt":"blue mug", "negative_prompt":null})), &r, &caps).unwrap();
    assert_eq!(parsed.positive, "blue mug");
    assert_eq!(parsed.negative, None);
    for fields in [
        json!({"positive_prompt":"mug"}),
        json!({"positive_prompt":"mug","negative_prompt":[]}),
        json!({"positive_prompt":"mug","negative_prompt":null,"workflow":{}}),
    ] {
        assert!(openai_prompt::parse_response("configured-text-model", &response(fields), &r, &caps).is_err());
    }
    assert!(openai_prompt::parse_response("configured-text-model", br#"{"status":"incomplete","output":[]}"#, &r, &caps).is_err());
    assert_eq!(
        openai_prompt::parse_response(
            "configured-text-model",
            br#"{"status":"completed","output":[{"type":"message","status":"completed","content":[{"type":"refusal","refusal":"no"}]}]}"#,
            &r,
            &caps
        ),
        Err(AiError::Rejected)
    );
}
