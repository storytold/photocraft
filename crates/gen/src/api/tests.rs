use super::*;
use crate::api_config::preset;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

pub(super) fn png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    bytes.extend_from_slice(&1024u32.to_be_bytes());
    bytes.extend_from_slice(&768u32.to_be_bytes());
    bytes
}

fn server(status: u16, body: String, pause: bool) -> (String, std::thread::JoinHandle<String>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/edit", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicBool::new(false));
    let signal = accepted.clone();
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut bytes = Vec::new();
        let mut length = None;
        loop {
            let mut chunk = [0; 2048];
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
            if length.is_none()
                && let Some(index) = bytes.windows(4).position(|s| s == b"\r\n\r\n")
            {
                let header = String::from_utf8_lossy(&bytes[..index]);
                let content = header
                    .lines()
                    .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|n| n.trim().parse::<usize>().ok()))
                    .unwrap();
                length = Some(index + 4 + content);
            }
            if length.is_some_and(|n| bytes.len() >= n) {
                break;
            }
        }
        signal.store(true, Ordering::Release);
        if pause {
            std::thread::sleep(Duration::from_millis(350));
        }
        let _ = write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        String::from_utf8(bytes).unwrap()
    });
    (endpoint, thread, accepted)
}

#[test]
fn all_protocols_send_canvas_prompt_and_exactly_one_request() {
    for protocol in [Protocol::GeminiInteractions, Protocol::GeminiGenerateContent, Protocol::Seedream] {
        let image = png();
        let data = STANDARD.encode(&image);
        let response = match protocol {
            Protocol::GeminiInteractions => json!({"status":"completed","steps":[{"type":"model_output","content":[{"type":"image","data":data}]}]}),
            Protocol::GeminiGenerateContent => json!({"candidates":[{"finishReason":"STOP","content":{"parts":[{"inlineData":{"data":data}}]}}]}),
            Protocol::Seedream => json!({"data":[{"b64_json":data}]}),
            Protocol::Flux => Value::Null, // The asynchronous workflow has separate tests.
        };
        let (mut endpoint, thread, _) = server(200, response.to_string(), false);
        if protocol == Protocol::GeminiGenerateContent {
            endpoint.push_str("/{model}:generateContent");
        }
        let provider = ApiProvider::new(Profile { protocol, endpoint, model: "test-image".into() }, "secret-test-key".into()).unwrap();
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("canvas.png");
        std::fs::write(&input, &image).unwrap();
        let result = provider.edit(&EditRequest { image: &input, prompt: "Make it yellow", workspace: work.path() }, &|| false).unwrap();
        assert_eq!(std::fs::read(result.saved_path).unwrap(), image);
        let request = thread.join().unwrap();
        let (header, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(header.contains("secret-test-key"));
        assert!(!header.lines().next().unwrap().contains("secret-test-key"));
        let body: Value = serde_json::from_str(body).unwrap();
        assert!(body.to_string().contains("Make it yellow"));
        assert!(body.to_string().contains(&STANDARD.encode(&image)));
        if protocol == Protocol::Seedream {
            assert!(header.to_ascii_lowercase().contains("authorization: bearer secret-test-key"));
            assert_eq!(body["size"], "2309x1732");
            assert!(body.get("stream").is_none(), "the synchronous request omits model-dependent stream options");
        } else {
            assert!(header.to_ascii_lowercase().contains("x-goog-api-key: secret-test-key"));
            if protocol == Protocol::GeminiInteractions {
                assert_eq!(body["response_format"]["aspect_ratio"], "4:3");
            } else {
                assert_eq!(body["generationConfig"]["imageConfig"]["aspectRatio"], "4:3");
            }
        }
    }
}

#[test]
fn cancellation_drops_transport_promptly_without_detached_worker_or_output() {
    let (endpoint, thread, accepted) = server(200, "{}".into(), true);
    let profile = Profile { endpoint, ..preset("gemini").unwrap() };
    let provider = ApiProvider::new(profile, "secret".into()).unwrap();
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("canvas.png");
    std::fs::write(&input, png()).unwrap();
    let start = Instant::now();
    assert!(matches!(
        provider.edit(&EditRequest { image: &input, prompt: "edit", workspace: work.path() }, &|| accepted.load(Ordering::Acquire)),
        Err(Error::Cancelled)
    ));
    assert!(start.elapsed() < Duration::from_millis(300));
    assert!(!work.path().join("api-result.png").exists());
    thread.join().unwrap();
}

#[test]
fn http_failures_and_redirects_never_echo_provider_secrets_or_retry() {
    for status in [302, 400, 401, 403, 404, 429, 500] {
        let (endpoint, thread, _) = server(status, "private-credential and prompt".into(), false);
        let provider = ApiProvider::new(Profile { endpoint, ..preset("gemini").unwrap() }, "private-credential".into()).unwrap();
        let error = cancellable(provider.request(json!({})), &|| false).unwrap_err().to_string();
        assert!(error.contains(&status.to_string()));
        assert!(!error.contains("private-credential"));
        thread.join().unwrap();
    }
}

#[test]
fn seedream_http_rejection_keeps_safe_code_and_field_without_output_or_private_message() {
    let private = "private-credential PRIVATE_PROMPT https://private.example/signed?token=secret data:image/png;base64,PRIVATE_IMAGE";
    let response = json!({"error":{"code":"InvalidParameter.UnsupportedParameter","param":"stream","message":private}});
    let (endpoint, thread, _) = server(400, response.to_string(), false);
    let provider = ApiProvider::new(Profile { endpoint, ..preset("seedream").unwrap() }, "private-credential".into()).unwrap();
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("canvas.png");
    std::fs::write(&input, png()).unwrap();
    let rejected =
        provider.edit(&EditRequest { image: &input, prompt: "PRIVATE_PROMPT", workspace: work.path() }, &|| false).err().expect("HTTP 400").to_string();
    assert!(rejected.contains("HTTP 400"));
    assert!(rejected.contains("InvalidParameter.UnsupportedParameter; parameter: stream"));
    assert!(rejected.contains("does not support"));
    for value in ["private-credential", "PRIVATE_PROMPT", "private.example", "PRIVATE_IMAGE", "data:image"] {
        assert!(!rejected.contains(value), "{rejected}");
    }
    assert!(!work.path().join("api-result.png").exists());
    let request = thread.join().unwrap();
    assert_eq!(request.matches("POST /edit HTTP/").count(), 1);
}

#[test]
fn seedream_diagnostics_allowlist_fields_codes_and_content_categories() {
    for (value, expected) in [
        (
            json!({"error":{"code":"InvalidParameter","message":"The parameter `size` specified in the request is not valid: PRIVATE_PROMPT"}}),
            "parameter: size",
        ),
        (json!({"error":{"code":"MissingParameter","param":"image"}}), "parameter: image"),
        (json!({"error":{"code":"InvalidImageURL.InvalidFormat","message":"PRIVATE_IMAGE"}}), "could not decode"),
        (json!({"error":{"code":"InvalidSubscription","message":"PRIVATE_ACCOUNT"}}), "account eligibility"),
        (json!({"error":{"code":"InputImageSensitiveContentDetected.PrivacyInformation","message":"PRIVATE_PROMPT"}}), "privacy check"),
        (json!({"error":{"code":"InvalidParameter.PRIVATE_KEY","param":"PRIVATE_PROMPT","message":"PRIVATE_IMAGE"}}), "InvalidParameter"),
    ] {
        let detail = rejection::detail(&value).unwrap();
        assert!(detail.contains(expected), "{detail}");
        assert!(!detail.contains("PRIVATE"), "{detail}");
        let error = parse_result(Protocol::Seedream, &value).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
        let item = json!({"data":[value]});
        assert!(parse_result(Protocol::Seedream, &item).unwrap_err().to_string().contains(expected));
    }
    for value in [json!({"error":{"code":"PRIVATE_KEY","param":"size","message":"PRIVATE_PROMPT"}}), json!({"error":{"code":null}}), json!({"error":true})] {
        assert!(rejection::detail(&value).is_none());
        assert!(!parse_result(Protocol::Seedream, &value).unwrap_err().to_string().contains("PRIVATE"));
    }
}

#[test]
fn seedream_content_diagnostics_preserve_stage_and_only_known_policy_suffixes() {
    for (code, expected_code, stage) in [
        ("SensitiveContentDetected", "SensitiveContentDetected", "input text"),
        ("InputTextSensitiveContentDetected", "InputTextSensitiveContentDetected", "input text"),
        ("InputImageSensitiveContentDetected", "InputImageSensitiveContentDetected", "reference image"),
        ("InputTextRiskDetection", "InputTextRiskDetection", "input text"),
        ("InputImageRiskDetection", "InputImageRiskDetection", "reference image"),
        ("OutputImageSensitiveContentDetected", "OutputImageSensitiveContentDetected", "generated image"),
        ("OutputImageRiskDetection", "OutputImageRiskDetection", "generated image"),
        ("InputTextSensitiveContentDetected.PolicyViolation", "InputTextSensitiveContentDetected.PolicyViolation", "policy check rejected the input text"),
        (
            "InputImageSensitiveContentDetected.PolicyViolation",
            "InputImageSensitiveContentDetected.PolicyViolation",
            "policy check rejected the reference image",
        ),
        (
            "InputImageSensitiveContentDetected.PrivacyInformation",
            "InputImageSensitiveContentDetected.PrivacyInformation",
            "privacy check rejected the reference image",
        ),
        (
            "OutputImageSensitiveContentDetected.PolicyViolation",
            "OutputImageSensitiveContentDetected.PolicyViolation",
            "policy check rejected the generated image",
        ),
        ("OutputImageSensitiveContentDetected.DeepFake", "OutputImageSensitiveContentDetected.DeepFake", "policy check rejected the generated image"),
        ("InputImageSensitiveContentDetected.PRIVATE_KEY", "InputImageSensitiveContentDetected", "reference image"),
        ("OutputImageRiskDetection.PRIVATE_KEY", "OutputImageRiskDetection", "generated image"),
        ("ContentSecurityDetectionError", "ContentSecurityDetectionError", "review service failed"),
    ] {
        let value = json!({"error":{"code":code,"param":"PRIVATE_PARAM","message":"PRIVATE_KEY PRIVATE_PROMPT data:image/png;base64,PRIVATE_IMAGE"}});
        let detail = rejection::detail(&value).unwrap();
        assert!(detail.starts_with(&format!("{expected_code}.")), "{detail}");
        assert!(detail.contains(stage), "{detail}");
        assert!(!detail.contains("PRIVATE"), "{detail}");
        assert!(!detail.contains("data:image"), "{detail}");
        assert!(!detail.contains("ContentRejected"), "{detail}");
        for response in [value.clone(), json!({"data":[value]})] {
            let error = parse_result(Protocol::Seedream, &response).unwrap_err().to_string();
            assert!(error.contains(expected_code) && error.contains(stage), "{error}");
            assert!(!error.contains("PRIVATE"), "{error}");
        }
    }
}

#[test]
fn seedream_error_body_limits_keep_status_and_do_not_echo_html_or_oversized_data() {
    for body in ["<html>PRIVATE_KEY PRIVATE_PROMPT</html>".into(), "PRIVATE_KEY".repeat(7000)] {
        let (endpoint, thread, _) = server(400, body, false);
        let provider = ApiProvider::new(Profile { endpoint, ..preset("seedream").unwrap() }, "PRIVATE_KEY".into()).unwrap();
        let error = cancellable(provider.request(json!({})), &|| false).unwrap_err().to_string();
        assert!(error.contains("HTTP 400"));
        assert!(!error.contains("PRIVATE"));
        thread.join().unwrap();
    }
}

#[test]
fn seedream_synchronous_payload_is_minimal_for_flash_pro_and_legacy_models() {
    for model in ["doubao-seedream-5-0-flash-260915", "doubao-seedream-5-0-pro-260628", "doubao-seedream-5-0-lite-260128", "doubao-seedream-4-5-251128"] {
        let profile = Profile { model: model.into(), ..preset("seedream").unwrap() };
        let body = request_body(&profile, "Edit the background", &png(), 1024, 768).unwrap();
        assert_eq!(body["model"], model);
        assert_eq!(body["size"], "2309x1732");
        assert_eq!(body["response_format"], "b64_json");
        assert!(body["image"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert_eq!(body.as_object().unwrap().len(), 5);
        assert!(body.get("stream").is_none());
        assert!(body.get("sequential_image_generation").is_none());
    }
}

#[test]
fn stalled_service_times_out_without_output() {
    let (endpoint, thread, _) = server(200, "{}".into(), true);
    let mut provider = ApiProvider::new(Profile { endpoint, ..preset("gemini").unwrap() }, "test-key".into()).unwrap();
    provider.timeout = Duration::from_millis(100);
    assert!(matches!(cancellable(provider.request(json!({})), &|| false), Err(Error::Timeout)));
    thread.join().unwrap();
}

#[test]
fn validates_endpoints_models_keys_and_output_completion() {
    for endpoint in [
        "http://example.com/edit",
        "https://user:pass@example.com/edit",
        "https://example.com/?key=secret",
        "https://example.com/#secret",
        "file:///edit",
        "nonsense",
    ] {
        assert!(ApiProvider::new(Profile { endpoint: endpoint.into(), ..preset("gemini").unwrap() }, "test".into()).is_err());
    }
    for model in ["", "../other", "model?key=x", "model\r\n"] {
        assert!(ApiProvider::new(Profile { model: model.into(), ..preset("gemini").unwrap() }, "test".into()).is_err());
    }
    assert!(
        ApiProvider::new(
            Profile { protocol: Protocol::GeminiGenerateContent, endpoint: "https://{model}.example.com/edit".into(), model: "target".into() },
            "test".into()
        )
        .is_err()
    );
    for key in ["", "x\ny", "key with spaces"] {
        assert!(validate_key(key).is_err());
    }
    for value in [
        json!({}),
        json!({"status":"in_progress","output_image":{"data":"eA=="}}),
        json!({"status":"failed","output_image":{"data":"eA=="}}),
        json!({"error":{"message":"private-credential"}}),
    ] {
        let error = parse_result(Protocol::GeminiInteractions, &value).unwrap_err().to_string();
        assert!(!error.contains("private-credential"));
    }
    for value in [
        json!({"data":[{"url":"https://example.com/image"}]}),
        json!({"data":[{"b64_json":"eA=="},{"b64_json":"eQ=="}]}),
        json!({"data":[{"b64_json":"not base64"}]}),
        json!({"data":[{"error":{},"b64_json":"eA=="}]}),
    ] {
        assert!(parse_result(Protocol::Seedream, &value).is_err());
    }
    assert!(
        parse_result(Protocol::GeminiGenerateContent, &json!({"candidates":[{"finishReason":"SAFETY","content":{"parts":[{"inlineData":{"data":"eA=="}}]}}]}))
            .is_err()
    );
    assert!(
        parse_result(Protocol::GeminiGenerateContent, &json!({"candidates":[{"content":{"parts":[{"thought":true,"inlineData":{"data":"eA=="}}]}}]})).is_err()
    );
}
