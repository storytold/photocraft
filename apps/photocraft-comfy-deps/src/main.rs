//! Scoped model installer for an existing ComfyUI model directory.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use photocraft_ai::dependencies::{self, Install, Model};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;
struct Host {
    root: PathBuf,
    key: String,
    client: reqwest::Client,
    job: Mutex<Option<Arc<Job>>>,
}
struct Job {
    id: String,
    cancel: AtomicBool,
    bytes: AtomicU64,
    total: u64,
    result: Mutex<Option<Result<(), String>>>,
    seen: Mutex<Instant>,
}
type Failure = (StatusCode, String);
fn failure(message: impl ToString) -> Failure {
    (StatusCode::BAD_REQUEST, message.to_string())
}
#[tokio::main]
async fn main() {
    if let Err(error) = serve().await {
        eprintln!("PhotoCraft dependency service: {error}");
        std::process::exit(1);
    }
}
async fn serve() -> Result<(), String> {
    let key = std::env::var("PHOTOCRAFT_DEPS_TOKEN").map_err(|_| "PHOTOCRAFT_DEPS_TOKEN is required")?;
    if key.len() < 24 || key.len() > 4096 || key.chars().any(char::is_control) {
        return Err("Dependency service token must contain 24–4096 characters without control characters".into());
    }
    let root = PathBuf::from(std::env::var("PHOTOCRAFT_MODELS_DIR").unwrap_or_else(|_| "/models".into()));
    let root = std::fs::canonicalize(root).map_err(|_| "Model directory is unavailable")?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent("PhotoCraft")
        .build()
        .map_err(|e| e.to_string())?;
    let host = Arc::new(Host { root, key, client, job: Mutex::new(None) });
    let app = Router::new()
        .route("/models", get(models))
        .route("/jobs", post(install))
        .route("/jobs/{id}", get(status).delete(cancel))
        .layer(DefaultBodyLimit::max(4096))
        .route_layer(middleware::from_fn_with_state(host.clone(), authenticate))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(host);
    let address = std::env::var("PHOTOCRAFT_DEPS_LISTEN").unwrap_or_else(|_| "0.0.0.0:8190".into());
    let listener = tokio::net::TcpListener::bind(address).await.map_err(|e| e.to_string())?;
    axum::serve(listener, app).await.map_err(|e| e.to_string())
}
async fn authenticate(State(host): State<Arc<Host>>, request: Request, next: Next) -> Response {
    let key = request.headers().get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or_default();
    if !constant_time_eq::constant_time_eq(key.as_bytes(), host.key.as_bytes()) {
        return (StatusCode::UNAUTHORIZED, "Dependency service authentication failed").into_response();
    }
    next.run(request).await
}
// The catalog contains fixed relative paths. Reject symlink directories and existing links.
fn destination(root: &std::path::Path, model: &Model, create: bool) -> Result<PathBuf, String> {
    let relative = std::path::Path::new(&model.path);
    if relative.components().any(|p| !matches!(p, std::path::Component::Normal(_))) {
        return Err("Invalid catalog path".into());
    }
    let parent = relative.parent().ok_or("Missing model directory")?;
    let mut directory = root.to_path_buf();
    for part in parent.components() {
        directory.push(part);
        match std::fs::symlink_metadata(&directory) {
            Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => return Err("Model directory must not be a symlink".into()),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => std::fs::create_dir(&directory).map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    let path = root.join(relative);
    if let Ok(meta) = std::fs::symlink_metadata(&path)
        && (!meta.is_file() || meta.file_type().is_symlink())
    {
        return Err("Existing model is not a regular file".into());
    }
    Ok(path)
}
async fn models(State(host): State<Arc<Host>>) -> Result<Json<Value>, Failure> {
    let mut rows = Vec::new();
    for model in dependencies::catalog().map_err(failure)? {
        let path = destination(&host.root, &model, false).map_err(failure)?;
        let state = match std::fs::metadata(path) {
            Ok(meta) if meta.len() == model.size => "present",
            Ok(_) => "sizeMismatch",
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => "missing",
            Err(e) => return Err(failure(e)),
        };
        rows.push(json!({"model":model,"state":state,"hashVerified":false}));
    }
    Ok(Json(json!({"models":rows,"note":"Existing files are checked by size only. New downloads are SHA-256 verified; existing files are never replaced."})))
}
async fn install(State(host): State<Arc<Host>>, Json(input): Json<Install>) -> Result<Json<Value>, Failure> {
    let selected = input.selected().map_err(failure)?;
    let mut slot = host.job.lock().unwrap_or_else(PoisonError::into_inner);
    if slot.as_ref().is_some_and(|j| j.result.lock().unwrap_or_else(PoisonError::into_inner).is_none()) {
        return Err((StatusCode::CONFLICT, "A model download is already running".into()));
    }
    let total = selected.iter().try_fold(0u64, |n, m| n.checked_add(m.size)).ok_or_else(|| failure("Model size overflow"))?;
    let job = Arc::new(Job {
        id: uuid::Uuid::new_v4().to_string(),
        cancel: AtomicBool::new(false),
        bytes: AtomicU64::new(0),
        total,
        result: Mutex::new(None),
        seen: Mutex::new(Instant::now()),
    });
    *slot = Some(job.clone());
    let id = job.id.clone();
    drop(slot);
    tokio::spawn(async move {
        let result = download_all(&host, &job, selected).await;
        *job.result.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
    });
    Ok(Json(json!({"id":id})))
}
fn find_job(host: &Host, id: &str) -> Result<Arc<Job>, Failure> {
    let job = host
        .job
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .filter(|job| job.id == id)
        .cloned()
        .ok_or((StatusCode::NOT_FOUND, "No such download".into()))?;
    *job.seen.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    Ok(job)
}
async fn status(State(host): State<Arc<Host>>, Path(id): Path<String>) -> Result<Json<Value>, Failure> {
    let job = find_job(&host, &id)?;
    let result = job.result.lock().unwrap_or_else(PoisonError::into_inner);
    let (state, error) = match result.as_ref() {
        None => ("running", None),
        Some(Ok(())) => ("done", None),
        Some(Err(error)) => ("failed", Some(error.clone())),
    };
    Ok(Json(json!({"state":state,"bytes":job.bytes.load(Ordering::Relaxed),"total":job.total,"error":error})))
}
async fn cancel(State(host): State<Arc<Host>>, Path(id): Path<String>) -> Result<Json<Value>, Failure> {
    find_job(&host, &id)?.cancel.store(true, Ordering::Relaxed);
    Ok(Json(json!({"cancelled":true})))
}
fn check(job: &Job) -> Result<(), String> {
    if job.cancel.load(Ordering::Relaxed) || job.seen.lock().unwrap_or_else(PoisonError::into_inner).elapsed() > Duration::from_secs(180) {
        Err("Download cancelled or client disconnected".into())
    } else {
        Ok(())
    }
}
async fn download_all(host: &Host, job: &Job, selected: Vec<Model>) -> Result<(), String> {
    for model in selected {
        check(job)?;
        let path = destination(&host.root, &model, true)?;
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() == model.size => {
                job.bytes.fetch_add(model.size, Ordering::Relaxed);
                continue;
            }
            Ok(_) => return Err(format!("{} already exists with a different size; preserved existing file", model.id)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        let temp = path.with_extension(format!("photocraft-{}.part", uuid::Uuid::new_v4()));
        let result = download(host, job, &model, &temp, &path).await;
        // Only this job's unique temporary file is removed, even on hash/disk/network failures.
        let _ = tokio::fs::remove_file(&temp).await;
        result?;
    }
    Ok(())
}
async fn download(host: &Host, job: &Job, model: &Model, temp: &std::path::Path, path: &std::path::Path) -> Result<(), String> {
    let mut response = host
        .client
        .get(&model.url)
        .send()
        .await
        .map_err(|_| "Model download could not connect")?
        .error_for_status()
        .map_err(|_| "Model host rejected download")?;
    if response.content_length().is_some_and(|n| n != model.size) {
        return Err("Model download size does not match catalog".into());
    }
    let mut file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(temp).await.map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(|_| "Model download interrupted")? {
        check(job)?;
        bytes = bytes.checked_add(chunk.len() as u64).filter(|n| *n <= model.size).ok_or("Model download exceeds catalog size")?;
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        hash.update(&chunk);
        job.bytes.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    if bytes != model.size || format!("{:x}", hash.finalize()) != model.sha256 {
        return Err("Model download failed size or SHA-256 verification".into());
    }
    file.sync_all().await.map_err(|e| e.to_string())?;
    drop(file);
    check(job)?;
    // Atomic no-overwrite commit: a concurrently created existing model is preserved.
    tokio::fs::hard_link(temp, path).await.map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_reject_traversal_and_existing_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let mut model = dependencies::catalog().unwrap().remove(0);
        model.path = "../outside".into();
        assert!(destination(root.path(), &model, true).is_err());
        model.path = "diffusion_models/model.bin".into();
        let path = destination(root.path(), &model, true).unwrap();
        std::fs::write(&path, b"original").unwrap();
        assert_eq!(std::fs::read(destination(root.path(), &model, true).unwrap()).unwrap(), b"original");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path(), root.path().join("linked")).unwrap();
            model.path = "linked/model.bin".into();
            assert!(destination(root.path(), &model, true).is_err());
        }
    }
    #[tokio::test]
    async fn verified_download_commits_once_and_rejects_corruption() {
        let root = tempfile::tempdir().unwrap();
        let bytes = b"synthetic model bytes";
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/model", get(|| async { "synthetic model bytes" }))).await.unwrap();
        });
        let host = Host { root: root.path().into(), key: "test-only-token".into(), client: reqwest::Client::new(), job: Mutex::new(None) };
        let job = Job {
            id: "test".into(),
            cancel: AtomicBool::new(false),
            bytes: AtomicU64::new(0),
            total: bytes.len() as u64,
            result: Mutex::new(None),
            seen: Mutex::new(Instant::now()),
        };
        let mut model = dependencies::catalog().unwrap().remove(0);
        model.path = "diffusion_models/synthetic.bin".into();
        model.url = format!("http://{address}/model");
        model.size = bytes.len() as u64;
        model.sha256 = format!("{:x}", Sha256::digest(bytes));
        download_all(&host, &job, vec![model.clone()]).await.unwrap();
        let path = root.path().join(&model.path);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::write(&path, b"user existing file").unwrap();
        assert!(download_all(&host, &job, vec![model.clone()]).await.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"user existing file");
        model.path = "diffusion_models/corrupt.bin".into();
        model.sha256 = "0".repeat(64);
        assert!(download_all(&host, &job, vec![model.clone()]).await.is_err());
        assert!(!root.path().join(&model.path).exists());
        assert_eq!(std::fs::read_dir(root.path().join("diffusion_models")).unwrap().count(), 1);
        model.sha256 = format!("{:x}", Sha256::digest(bytes));
        job.cancel.store(true, Ordering::Relaxed);
        assert!(download_all(&host, &job, vec![model]).await.is_err());
        server.abort();
    }
}
