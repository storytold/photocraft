//! Authenticated same-origin web host and bounded native AI workers.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo, clippy::unimplemented, clippy::unreachable)]
use axum::{
    Extension, Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::{Path, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine as _;
use photocraft_ai::gateway;
use photocraft_engine::{Session, jobs::JobCtx, prefs::Integrations};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

type Failure = (StatusCode, Json<Value>);
fn failure(status: StatusCode, message: &str) -> Failure {
    (status, Json(json!({"error":message})))
}
fn setting(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.into())
}
fn allowed(name: &str, default: &str) -> Vec<String> {
    setting(name, default).split(',').map(|s| s.trim().trim_end_matches('/').to_owned()).filter(|s| !s.is_empty()).collect()
}
fn key_from_values(service: Option<String>, common: Option<String>) -> std::result::Result<Option<photocraft_ai::secret::Secret>, String> {
    service
        .filter(|s| !s.is_empty())
        .or_else(|| common.filter(|s| !s.is_empty()))
        .map(photocraft_ai::secret::Secret::new)
        .transpose()
        .map_err(|_| "Invalid API key in Docker environment".into())
}
fn environment_key(service: &str) -> std::result::Result<Option<photocraft_ai::secret::Secret>, String> {
    key_from_values(std::env::var(service).ok(), std::env::var("OPENAI_API_KEY").ok())
}

struct Host {
    authorization: String,
    providers: Mutex<Integrations>,
    config_path: std::path::PathBuf,
    environment_prompt_key: Option<photocraft_ai::secret::Secret>,
    environment_image_key: Option<photocraft_ai::secret::Secret>,
    prompt_model: String,
    language: String,
    comfy: Vec<String>,
    prompt: Vec<String>,
    images: Vec<String>,
    dependency: Vec<String>,
    jobs: Mutex<BTreeMap<String, Arc<Job>>>,
    worker: Arc<Semaphore>,
}
struct Job {
    owner: String,
    ctx: JobCtx,
    seen: Mutex<Instant>,
    result: Mutex<Option<std::result::Result<Arc<[u8]>, String>>>,
}
#[derive(Clone)]
struct Caller(String);
impl Host {
    fn from_environment() -> std::result::Result<Self, String> {
        let password = std::env::var("PHOTOCRAFT_WEB_PASSWORD").map_err(|_| "Set PHOTOCRAFT_WEB_PASSWORD before starting the web host")?;
        if password.len() < 12 || password.len() > 256 || password.chars().any(char::is_control) {
            return Err("The web password must contain 12–256 bytes without control characters".into());
        }
        let user = setting("PHOTOCRAFT_WEB_USER", "photocraft");
        if user.is_empty() || user.len() > 128 || user.contains(':') || user.chars().any(char::is_control) {
            return Err("Invalid web username".into());
        }
        let mut providers = Integrations {
            comfyui_endpoint: setting("PHOTOCRAFT_COMFYUI_ENDPOINT", "http://127.0.0.1:8188"),
            comfyui_api_key_env: setting("PHOTOCRAFT_COMFYUI_KEY_ENV", ""),
            openai_prompt_endpoint: setting("PHOTOCRAFT_OPENAI_PROMPT_ENDPOINT", "https://api.openai.com/v1"),
            openai_prompt_api_key_env: setting("PHOTOCRAFT_OPENAI_PROMPT_KEY_ENV", ""),
            openai_image_endpoint: setting("PHOTOCRAFT_OPENAI_IMAGE_ENDPOINT", "https://api.openai.com/v1"),
            openai_image_api_key_env: setting("PHOTOCRAFT_OPENAI_IMAGE_KEY_ENV", ""),
            allow_agent_control: false,
            control_port: 0,
            ..Integrations::default()
        };
        let config_path = std::path::PathBuf::from(setting("PHOTOCRAFT_AI_CONFIG", "/data/ai-config.json"));
        match std::fs::read(&config_path) {
            Ok(bytes) if bytes.len() <= 64 * 1024 => providers = serde_json::from_slice(&bytes).map_err(|_| "Invalid private AI configuration")?,
            Ok(_) => return Err("Private AI configuration is too large".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot read private AI configuration".into()),
        }
        providers.validate()?;
        let prompt_model = setting("PHOTOCRAFT_OPENAI_PROMPT_MODEL", "gpt-4.1-mini");
        photocraft_ai::openai_prompt::validate_model(&prompt_model).map_err(|e| e.to_string())?;
        let language = setting("PHOTOCRAFT_WEB_LANGUAGE", "en");
        let mut validation = photocraft_engine::prefs::Preferences::default();
        validation.set("interface.language", json!(language))?;
        let environment_prompt_key = environment_key("PHOTOCRAFT_OPENAI_PROMPT_API_KEY")?;
        let environment_image_key = environment_key("PHOTOCRAFT_OPENAI_IMAGE_API_KEY")?;
        let comfy = allowed("PHOTOCRAFT_ALLOWED_COMFYUI_ENDPOINTS", &providers.comfyui_endpoint);
        let prompt = allowed("PHOTOCRAFT_ALLOWED_PROMPT_ENDPOINTS", &providers.openai_prompt_endpoint);
        let images = allowed("PHOTOCRAFT_ALLOWED_IMAGE_ENDPOINTS", &providers.openai_image_endpoint);
        // Empty by default: web users cannot point the dependency key at a new service.
        let dependency = allowed("PHOTOCRAFT_ALLOWED_DEPENDENCY_ENDPOINTS", "");
        for endpoint in comfy.iter().chain(&prompt).chain(&images).chain(&dependency) {
            photocraft_ai::config::validate_endpoint(endpoint).map_err(|e| e.to_string())?;
        }
        Ok(Self {
            authorization: format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))),
            providers: Mutex::new(providers),
            config_path,
            environment_prompt_key,
            environment_image_key,
            prompt_model,
            language,
            comfy,
            prompt,
            images,
            dependency,
            jobs: Mutex::new(BTreeMap::new()),
            worker: Arc::new(Semaphore::new(1)),
        })
    }
    fn effective_providers(&self, requested: &Integrations) -> std::result::Result<Integrations, String> {
        requested.validate()?;
        if requested.comfyui_dependency_api_key.is_some()
            || requested.comfyui_api_key.is_some()
            || requested.openai_prompt_api_key.is_some()
            || requested.openai_image_api_key.is_some()
        {
            return Err("Save API keys through AI Settings before starting a job".into());
        }
        let stored = self.providers.lock().unwrap_or_else(PoisonError::into_inner);
        for (url, configured, choices) in [
            (&requested.comfyui_endpoint, &stored.comfyui_endpoint, &self.comfy),
            (&requested.openai_prompt_endpoint, &stored.openai_prompt_endpoint, &self.prompt),
            (&requested.openai_image_endpoint, &stored.openai_image_endpoint, &self.images),
        ] {
            if url.trim_end_matches('/') != configured.trim_end_matches('/') && !choices.iter().any(|choice| choice == url.trim_end_matches('/')) {
                return Err("Save the API endpoint through AI Settings before starting a job".into());
            }
        }
        if requested.comfyui_api_key_env != stored.comfyui_api_key_env
            || requested.openai_prompt_api_key_env != stored.openai_prompt_api_key_env
            || requested.openai_image_api_key_env != stored.openai_image_api_key_env
        {
            return Err("Save API key references through AI Settings before starting a job".into());
        }
        if requested.comfyui_dependency_endpoint != stored.comfyui_dependency_endpoint {
            return Err("Save the dependency service endpoint through AI Settings before starting a job".into());
        }
        let mut providers = requested.clone();
        providers.comfyui_dependency_api_key = stored.comfyui_dependency_api_key.clone();
        // Never forward stored credentials to an alternative endpoint.
        providers.comfyui_api_key = (requested.comfyui_endpoint == stored.comfyui_endpoint).then(|| stored.comfyui_api_key.clone()).flatten();
        providers.openai_prompt_api_key =
            (requested.openai_prompt_endpoint == stored.openai_prompt_endpoint).then(|| stored.openai_prompt_api_key.clone()).flatten();
        providers.openai_image_api_key =
            (requested.openai_image_endpoint == stored.openai_image_endpoint).then(|| stored.openai_image_api_key.clone()).flatten();
        if requested.openai_prompt_endpoint == stored.openai_prompt_endpoint && providers.openai_prompt_api_key.is_none() {
            providers.openai_prompt_api_key = self.environment_prompt_key.clone();
        }
        if requested.openai_image_endpoint == stored.openai_image_endpoint && providers.openai_image_api_key.is_none() {
            providers.openai_image_api_key = self.environment_image_key.clone();
        }
        Ok(providers)
    }
    fn public_configuration(&self) -> Value {
        let mut providers = self.providers.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if providers.openai_prompt_api_key.is_none() {
            providers.openai_prompt_api_key = self.environment_prompt_key.clone();
        }
        if providers.openai_image_api_key.is_none() {
            providers.openai_image_api_key = self.environment_image_key.clone();
        }
        let mut public = public_configuration(&providers);
        public["webDefaults"] = json!({"language":self.language,"promptModel":self.prompt_model});
        public
    }
    /// Saved endpoints receive the operator's keys, so they obey the same allowlists as jobs.
    /// Key-variable names are operator settings, not web-user input.
    fn check_saved_configuration(&self, incoming: &Integrations, stored: &Integrations) -> std::result::Result<(), String> {
        for (url, kept, choices) in [
            (&incoming.comfyui_endpoint, &stored.comfyui_endpoint, &self.comfy),
            (&incoming.openai_prompt_endpoint, &stored.openai_prompt_endpoint, &self.prompt),
            (&incoming.openai_image_endpoint, &stored.openai_image_endpoint, &self.images),
        ] {
            let url = url.trim_end_matches('/');
            if url != kept.trim_end_matches('/') && !choices.iter().any(|choice| choice.trim_end_matches('/') == url) {
                return Err("This API endpoint is not in the operator's allowlist".into());
            }
        }
        if incoming.comfyui_api_key_env != stored.comfyui_api_key_env
            || incoming.openai_prompt_api_key_env != stored.openai_prompt_api_key_env
            || incoming.openai_image_api_key_env != stored.openai_image_api_key_env
        {
            return Err("API key environment variables are set by the operator and cannot be changed here".into());
        }
        let dependency = incoming.comfyui_dependency_endpoint.trim_end_matches('/');
        if dependency != stored.comfyui_dependency_endpoint.trim_end_matches('/') && !self.dependency.iter().any(|choice| choice.trim_end_matches('/') == dependency) {
            return Err("This dependency service endpoint is not in the operator's allowlist".into());
        }
        Ok(())
    }
    fn save_configuration(&self, mut incoming: Integrations) -> std::result::Result<Value, String> {
        incoming.validate()?;
        let mut stored = self.providers.lock().unwrap_or_else(PoisonError::into_inner);
        self.check_saved_configuration(&incoming, &stored)?;
        for (new, old) in [
            (&mut incoming.comfyui_dependency_api_key, &stored.comfyui_dependency_api_key),
            (&mut incoming.comfyui_api_key, &stored.comfyui_api_key),
            (&mut incoming.openai_prompt_api_key, &stored.openai_prompt_api_key),
            (&mut incoming.openai_image_api_key, &stored.openai_image_api_key),
        ] {
            if new.is_none() {
                *new = old.clone();
            }
            if new.as_ref().is_some_and(|key| key.expose().is_empty()) {
                *new = None;
            }
        }
        let bytes = serde_json::to_vec_pretty(&incoming).map_err(|_| "Cannot serialize AI configuration")?;
        photocraft_ai::config::write_private_file(&self.config_path, &bytes).map_err(|_| "Cannot save the private server AI configuration")?;
        *stored = incoming;
        drop(stored);
        Ok(self.public_configuration())
    }
    fn job(&self, id: &str, caller: &Caller) -> std::result::Result<Arc<Job>, Failure> {
        let job = self
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .filter(|job| job.owner == caller.0)
            .cloned()
            .ok_or_else(|| failure(StatusCode::NOT_FOUND, "AI job not found"))?;
        *job.seen.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
        Ok(job)
    }
}
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get("origin") else {
        return true;
    };
    let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) else {
        return false;
    };
    origin.to_str().is_ok_and(|origin| origin == format!("http://{host}") || origin == format!("https://{host}"))
}
async fn authenticate(State(host): State<Arc<Host>>, mut request: Request, next: Next) -> Response {
    if request.uri().path() == "/healthz" {
        return next.run(request).await;
    }
    let supplied = request.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !constant_time_eq::constant_time_eq(supplied.as_bytes(), host.authorization.as_bytes()) {
        let mut response = failure(StatusCode::UNAUTHORIZED, "PhotoCraft web authentication required").into_response();
        response.headers_mut().insert("www-authenticate", HeaderValue::from_static("Basic realm=\"PhotoCraft\", charset=\"UTF-8\""));
        return response;
    }
    if !origin_allowed(request.headers()) {
        return failure(StatusCode::FORBIDDEN, "Cross-origin requests are disabled").into_response();
    }
    let old = request
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() <= 8192)
        .and_then(|cookies| cookies.split(';').find_map(|cookie| cookie.trim().strip_prefix("photocraft_session=")))
        .filter(|value| value.len() == 32 && value.bytes().all(|c| c.is_ascii_hexdigit()));
    let owner = old.map(str::to_owned).unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    let cookie = if old.is_none() { Some(format!("photocraft_session={owner}; HttpOnly; SameSite=Strict; Path=/")) } else { None };
    request.extensions_mut().insert(Caller(owner));
    let api = request.uri().path().starts_with("/api/");
    let mut response = next.run(request).await;
    if let Some(cookie) = cookie.and_then(|value| HeaderValue::from_str(&value).ok()) {
        response.headers_mut().insert("set-cookie", cookie);
    }
    response.headers_mut().insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    response.headers_mut().insert("content-security-policy",HeaderValue::from_static("default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' blob: data:; worker-src 'self' blob:; frame-ancestors 'self'; base-uri 'self'"));
    if api {
        response.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
    }
    response
}
fn public_configuration(providers: &Integrations) -> Value {
    let mut public = json!({"providers":providers,"commands":gateway::COMMANDS,"documentUploadBytes":gateway::MAX_DOCUMENT_BYTES,
        "apiKeysConfigured":{"comfyui":providers.comfyui_api_key.is_some(),"prompt":providers.openai_prompt_api_key.is_some(),"images":providers.openai_image_api_key.is_some()}});
    photocraft_ai::secret::redact(&mut public);
    public
}
async fn configuration(State(host): State<Arc<Host>>) -> Json<Value> {
    Json(host.public_configuration())
}
async fn save_configuration(State(host): State<Arc<Host>>, request: Request) -> std::result::Result<Json<Value>, Failure> {
    if !content_type(request.headers(), "application/json") {
        return Err(failure(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Expected JSON configuration"));
    }
    let bytes = to_bytes(request.into_body(), 64 * 1024).await.map_err(|_| failure(StatusCode::PAYLOAD_TOO_LARGE, "AI configuration is too large"))?;
    let providers: Integrations = serde_json::from_slice(&bytes).map_err(|_| failure(StatusCode::BAD_REQUEST, "Invalid AI configuration"))?;
    host.save_configuration(providers).map(Json).map_err(|e| failure(StatusCode::BAD_REQUEST, &e))
}

fn content_type(headers: &HeaderMap, expected: &str) -> bool {
    headers.get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|v| v.split(';').next() == Some(expected))
}
async fn submit(State(host): State<Arc<Host>>, Extension(caller): Extension<Caller>, request: Request) -> std::result::Result<Json<Value>, Failure> {
    if !content_type(request.headers(), "application/x-photocraft-ai") {
        return Err(failure(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Expected a PhotoCraft AI envelope"));
    }
    let permit =
        host.worker.clone().try_acquire_owned().map_err(|_| failure(StatusCode::TOO_MANY_REQUESTS, "The web AI worker is busy; wait for the current job"))?;
    {
        let jobs = host.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        let retained: usize = jobs
            .values()
            .filter_map(|job| job.result.lock().unwrap_or_else(PoisonError::into_inner).as_ref().and_then(|result| result.as_ref().ok().map(|data| data.len())))
            .sum();
        if jobs.len() >= 16 || retained > 256 * 1024 * 1024 {
            return Err(failure(StatusCode::TOO_MANY_REQUESTS, "Web AI result storage is full; finish or discard previous jobs"));
        }
    }
    let bytes = to_bytes(request.into_body(), gateway::MAX_PACKET_BYTES)
        .await
        .map_err(|_| failure(StatusCode::PAYLOAD_TOO_LARGE, "AI gateway upload limit exceeded"))?;
    let (input, document): (gateway::Request, _) = gateway::decode(&bytes).map_err(|_| failure(StatusCode::BAD_REQUEST, "Invalid AI gateway envelope"))?;
    if document.len() > gateway::MAX_DOCUMENT_BYTES
        || !gateway::COMMANDS.contains(&input.command.as_str())
        || input.id.len() != 32
        || uuid::Uuid::parse_str(&input.id).is_err()
    {
        return Err(failure(StatusCode::BAD_REQUEST, "Unsupported AI command, identifier or document size"));
    }
    let providers: Integrations =
        serde_json::from_value(input.providers.clone()).map_err(|_| failure(StatusCode::BAD_REQUEST, "Invalid AI provider settings"))?;
    let providers = host.effective_providers(&providers).map_err(|e| failure(StatusCode::BAD_REQUEST, &e))?;
    let job = Arc::new(Job { owner: caller.0, ctx: JobCtx::new(), seen: Mutex::new(Instant::now()), result: Mutex::new(None) });
    let id = input.id.clone();
    {
        let mut jobs = host.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        if jobs.contains_key(&id) {
            return Err(failure(StatusCode::CONFLICT, "AI job identifier already exists; do not resubmit"));
        }
        jobs.insert(id.clone(), job.clone());
    }
    let prompt_model = host.prompt_model.clone();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (_, document): (gateway::Request, _) = gateway::decode(&bytes).map_err(|e| e.to_string())?;
            process(&input, &providers, &prompt_model, document, &job.ctx)
        }))
        .unwrap_or_else(|_| Err("AI worker failed internally; the browser document is unchanged".into()));
        *job.result.lock().unwrap_or_else(PoisonError::into_inner) = Some(result.map(Arc::from));
    });
    Ok(Json(json!({"job":id})))
}
fn process(input: &gateway::Request, providers: &Integrations, prompt_model: &str, document: &[u8], ctx: &JobCtx) -> std::result::Result<Vec<u8>, String> {
    ctx.check().map_err(|e| e.to_string())?;
    ctx.progress(0.01, "Loading captured browser document");
    let mut session = Session::new();
    if !document.is_empty() {
        let doc = photocraft_engine::ai_cmds::remote::load_document(document).map_err(|e| e.to_string())?;
        session.add_document(doc, None);
    }
    session.configure_ai_prompt_model(prompt_model).map_err(|e| e.to_string())?;
    session.load_prefs_json(&json!({"integrations":providers}).to_string())?;
    session.set_inline_job_ctx(Some(ctx.clone()));
    let ready = session.execute(&input.command, input.params.clone()).map_err(|e| e.to_string())?;
    if gateway::JSON_COMMANDS.contains(&input.command.as_str()) {
        ctx.check().map_err(|e| e.to_string())?;
        return gateway::encode(&ready, &[]).map_err(|e| e.to_string());
    }
    let id = ready.get("result").and_then(Value::as_u64).ok_or("AI workflow returned no result")?;
    ctx.check().map_err(|e| e.to_string())?;
    session.export_ai_gateway_result(id).map_err(|e| e.to_string())
}
async fn status(
    State(host): State<Arc<Host>>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
) -> std::result::Result<Json<gateway::Status>, Failure> {
    let job = host.job(&id, &caller)?;
    let result = job.result.lock().unwrap_or_else(PoisonError::into_inner);
    let (state, error) = if job.ctx.cancelled() {
        ("cancelled", None)
    } else {
        match &*result {
            None => ("running", None),
            Some(Ok(_)) => ("done", None),
            Some(Err(error)) => ("failed", Some(error.clone())),
        }
    };
    Ok(Json(gateway::Status { state: state.into(), progress: job.ctx.fraction(), message: job.ctx.message(), error }))
}
async fn result(State(host): State<Arc<Host>>, Extension(caller): Extension<Caller>, Path(id): Path<String>) -> std::result::Result<Response, Failure> {
    let job = host.job(&id, &caller)?;
    let data = job
        .result
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .ok_or_else(|| failure(StatusCode::CONFLICT, "AI result is not ready"))?;
    let length = data.len();
    let mut response = Body::from(Bytes::from_owner(data)).into_response();
    response.headers_mut().insert("content-type", HeaderValue::from_static("application/x-photocraft-ai"));
    if let Ok(value) = HeaderValue::from_str(&length.to_string()) {
        response.headers_mut().insert("content-length", value);
    }
    Ok(response)
}
async fn cancel(
    State(host): State<Arc<Host>>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> std::result::Result<Json<Value>, Failure> {
    if !content_type(&headers, "application/json") {
        return Err(failure(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Expected JSON cancellation"));
    }
    host.job(&id, &caller)?.ctx.cancel();
    Ok(Json(json!({"cancelled":true})))
}
async fn discard(State(host): State<Arc<Host>>, Extension(caller): Extension<Caller>, Path(id): Path<String>) -> std::result::Result<Json<Value>, Failure> {
    host.job(&id, &caller)?.ctx.cancel();
    host.jobs.lock().unwrap_or_else(PoisonError::into_inner).remove(&id);
    Ok(Json(json!({"discarded":true})))
}
async fn cleanup(host: Arc<Host>) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let mut jobs = host.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        jobs.retain(|_, job| {
            let age = job.seen.lock().unwrap_or_else(PoisonError::into_inner).elapsed();
            if age > Duration::from_secs(180) && job.result.lock().unwrap_or_else(PoisonError::into_inner).is_none() {
                job.ctx.cancel();
            }
            age < Duration::from_secs(600)
        });
    }
}
#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--health-check") {
        let address = setting("PHOTOCRAFT_WEB_BIND", "0.0.0.0:8080");
        let port = address.rsplit(':').next().ok_or("Invalid web bind address")?;
        let response = reqwest::Client::new().get(format!("http://127.0.0.1:{port}/healthz")).timeout(Duration::from_secs(5)).send().await?;
        if !response.status().is_success() {
            return Err("Web host health check failed".into());
        }
        return Ok(());
    }
    let host = Arc::new(Host::from_environment()?);
    tokio::spawn(cleanup(host.clone()));
    let site = setting("PHOTOCRAFT_WEB_ROOT", "/opt/photocraft/web");
    let router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/ai/config", get(configuration).put(save_configuration))
        .route("/api/ai/jobs", post(submit))
        .route("/api/ai/jobs/{id}", get(status).delete(discard))
        .route("/api/ai/jobs/{id}/result", get(result))
        .route("/api/ai/jobs/{id}/cancel", post(cancel))
        .fallback_service(tower_http::services::ServeDir::new(site))
        .layer(middleware::from_fn_with_state(host.clone(), authenticate))
        .with_state(host);
    let bind = setting("PHOTOCRAFT_WEB_BIND", "0.0.0.0:8080");
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("PhotoCraft web host listening on {bind}");
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csrf_checks_exact_origin_and_not_host_suffixes() {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("photocraft.local:18888"));
        headers.insert("origin", HeaderValue::from_static("http://photocraft.local:18888"));
        assert!(origin_allowed(&headers));
        for origin in ["http://evil.test", "http://photocraft.local:18888.evil.test", "null"] {
            headers.insert("origin", HeaderValue::from_str(origin).unwrap());
            assert!(!origin_allowed(&headers));
        }
    }
    fn host() -> Host {
        let defaults = Integrations::default();
        Host {
            authorization: "test".into(),
            comfy: vec![defaults.comfyui_endpoint.clone()],
            prompt: vec![defaults.openai_prompt_endpoint.clone()],
            images: vec![defaults.openai_image_endpoint.clone()],
            dependency: vec!["http://localhost:8190".into()],
            providers: Mutex::new(defaults),
            config_path: std::env::temp_dir().join(format!("photocraft-config-test-{}.json", uuid::Uuid::new_v4())),
            environment_prompt_key: None,
            environment_image_key: None,
            prompt_model: "gpt-4.1-mini".into(),
            language: "auto".into(),
            jobs: Mutex::new(BTreeMap::new()),
            worker: Arc::new(Semaphore::new(1)),
        }
    }
    #[test]
    fn saving_configuration_obeys_the_operator_allowlists() {
        let host = Host { prompt: vec!["https://api.openai.com/v1".into(), "https://gateway.example/v1".into()], ..host() };
        let mut incoming = Integrations { openai_prompt_endpoint: "https://attacker.example/v1".into(), ..Default::default() };
        let error = host.save_configuration(incoming.clone()).unwrap_err();
        assert!(error.contains("allowlist"), "{error}");
        assert_eq!(host.providers.lock().unwrap().openai_prompt_endpoint, "https://api.openai.com/v1", "nothing was stored");
        incoming.openai_prompt_endpoint = "https://gateway.example/v1/".into();
        assert!(host.save_configuration(incoming.clone()).is_ok());
        // Key variables and the dependency service stay with the operator.
        let env = Integrations { openai_prompt_api_key_env: "PHOTOCRAFT_WEB_PASSWORD".into(), ..Default::default() };
        assert!(host.save_configuration(env).is_err());
        let dependency = Integrations {
            openai_prompt_endpoint: incoming.openai_prompt_endpoint.clone(),
            comfyui_dependency_endpoint: "https://attacker.example".into(),
            ..Default::default()
        };
        assert!(host.save_configuration(dependency).is_err());
    }
    #[test]
    fn docker_defaults_are_redacted_and_menu_credentials_take_precedence() {
        assert!(key_from_values(None, None).unwrap().is_none());
        assert_eq!(key_from_values(Some(String::new()), Some("synthetic-common".into())).unwrap().unwrap().expose(), "synthetic-common");
        assert_eq!(key_from_values(Some("synthetic-service".into()), Some("synthetic-common".into())).unwrap().unwrap().expose(), "synthetic-service");
        assert!(key_from_values(Some("bad\nkey".into()), None).is_err());
        let host = Host {
            environment_prompt_key: Some(photocraft_ai::secret::Secret::new("synthetic-docker-key".into()).unwrap()),
            prompt_model: "qwen3.6:35b@GPU-HOST".into(),
            language: "de".into(),
            ..host()
        };
        let public = host.public_configuration();
        assert_eq!(public["webDefaults"]["language"], "de");
        assert_eq!(public["webDefaults"]["promptModel"], "qwen3.6:35b@GPU-HOST");
        assert!(!public.to_string().contains("synthetic-docker-key"));
        assert_eq!(host.effective_providers(&Integrations::default()).unwrap().openai_prompt_api_key.unwrap().expose(), "synthetic-docker-key");
        host.save_configuration(Integrations::default()).unwrap();
        assert!(!std::fs::read_to_string(&host.config_path).unwrap().contains("synthetic-docker-key"));
        host.save_configuration(Integrations {
            openai_prompt_api_key: Some(photocraft_ai::secret::Secret::new("synthetic-menu-key".into()).unwrap()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(host.effective_providers(&Integrations::default()).unwrap().openai_prompt_api_key.unwrap().expose(), "synthetic-menu-key");
        std::fs::remove_file(&host.config_path).unwrap();
    }
    #[test]
    fn jobs_are_owned_by_the_browser_session() {
        let host = host();
        host.jobs
            .lock()
            .unwrap()
            .insert("own".into(), Arc::new(Job { owner: "browser-a".into(), ctx: JobCtx::new(), seen: Mutex::new(Instant::now()), result: Mutex::new(None) }));
        assert!(host.job("own", &Caller("browser-a".into())).is_ok());
        assert!(host.job("own", &Caller("browser-b".into())).is_err());
        assert!(host.job("missing", &Caller("browser-a".into())).is_err());
    }
    #[test]
    fn private_keys_persist_and_public_configuration_cannot_reveal_them() {
        let host = host();
        let incoming =
            Integrations { openai_prompt_api_key: Some(photocraft_ai::secret::Secret::new("synthetic-stored-key".into()).unwrap()), ..Default::default() };
        let public = host.save_configuration(incoming).unwrap();
        assert!(!public.to_string().contains("synthetic-stored-key"));
        assert_eq!(public["apiKeysConfigured"]["prompt"], true);
        let persisted: Integrations = serde_json::from_slice(&std::fs::read(&host.config_path).unwrap()).unwrap();
        assert_eq!(persisted.openai_prompt_api_key.as_ref().unwrap().expose(), "synthetic-stored-key");
        let effective = host.effective_providers(&Integrations::default()).unwrap();
        assert_eq!(effective.openai_prompt_api_key.as_ref().unwrap().expose(), "synthetic-stored-key");
        host.save_configuration(Integrations::default()).unwrap();
        assert!(host.providers.lock().unwrap().openai_prompt_api_key.is_some());
        let clear = Integrations { openai_prompt_api_key: Some(photocraft_ai::secret::Secret::new(String::new()).unwrap()), ..Default::default() };
        assert_eq!(host.save_configuration(clear).unwrap()["apiKeysConfigured"]["prompt"], false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&host.config_path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_file(&host.config_path).unwrap();
    }
    #[test]
    fn provider_allowlist_does_not_grant_other_environment_variables() {
        let host = host();
        let defaults = host.providers.lock().unwrap().clone();
        assert!(host.effective_providers(&defaults).is_ok());
        let mut bad = defaults.clone();
        bad.openai_prompt_api_key_env = "PATH".into();
        assert!(host.effective_providers(&bad).is_err());
        bad = defaults;
        bad.comfyui_endpoint = "http://169.254.169.254".into();
        assert!(host.effective_providers(&bad).is_err());
    }
    #[test]
    fn dependency_credentials_are_private_and_bound_to_saved_endpoint() {
        let host = host();
        let incoming = Integrations {
            comfyui_dependency_endpoint: "http://localhost:8190".into(),
            comfyui_dependency_api_key: Some(photocraft_ai::secret::Secret::new("synthetic-dependency-key".into()).unwrap()),
            ..Default::default()
        };
        let public = host.save_configuration(incoming).unwrap();
        assert!(!public.to_string().contains("synthetic-dependency-key"));
        let mut request: Integrations = serde_json::from_value(public["providers"].clone()).unwrap();
        assert_eq!(host.effective_providers(&request).unwrap().comfyui_dependency_api_key.unwrap().expose(), "synthetic-dependency-key");
        request.comfyui_dependency_endpoint = "http://localhost:8191".into();
        assert!(host.effective_providers(&request).is_err());
        std::fs::remove_file(&host.config_path).unwrap();
    }
}
