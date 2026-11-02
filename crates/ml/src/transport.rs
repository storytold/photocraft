//! HTTP behind a trait, so providers are testable without a network and the wasm build has no
//! client at all (upstream issue #41, rule 1).

use std::sync::Mutex;

use crate::Error;

/// A received HTTP reply. `status` is kept even for errors so callers can explain them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// Minimal blocking HTTP. Implementations map connection failures to [`Error::Unreachable`] and
/// never turn an HTTP error status into an `Err`: the status comes back in the response.
pub trait Transport: Send + Sync {
    fn get(&self, url: &str) -> Result<HttpResponse, Error>;
    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<HttpResponse, Error>;
}

/// Real transport on top of `ureq`. One agent per transport; connections are reused.
#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
pub struct UreqTransport {
    agent: ureq::Agent,
}

#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
impl UreqTransport {
    /// `timeout` bounds one whole request (connect + send + receive). A CPU-only model on a big
    /// image can take minutes, so callers pass something generous for `post` paths.
    pub fn new(timeout: std::time::Duration) -> Self {
        let config =
            ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).user_agent("PhotoCraft generative (photocraft-ml)").build();
        Self { agent: config.new_agent() }
    }

    fn finish(mut resp: ureq::http::Response<ureq::Body>, url: &str) -> Result<HttpResponse, Error> {
        let status = resp.status().as_u16();
        let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_owned);
        // Results are PNGs of the user's canvas; 256 MiB is far beyond any sane reply.
        let body = resp
            .body_mut()
            .with_config()
            .limit(256 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| Error::Unreachable { url: url.to_owned(), detail: e.to_string() })?;
        Ok(HttpResponse { status, content_type, body })
    }
}

#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
impl Transport for UreqTransport {
    fn get(&self, url: &str) -> Result<HttpResponse, Error> {
        let resp = self.agent.get(url).call().map_err(|e| Error::Unreachable { url: url.to_owned(), detail: e.to_string() })?;
        Self::finish(resp, url)
    }

    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<HttpResponse, Error> {
        let resp = self
            .agent
            .post(url)
            .header("Content-Type", content_type)
            .send(body)
            .map_err(|e| Error::Unreachable { url: url.to_owned(), detail: e.to_string() })?;
        Self::finish(resp, url)
    }
}

/// One recorded request made through a [`FakeTransport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub method: &'static str,
    pub url: String,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// Test transport: answers from a queue of canned responses and records every request.
/// An empty queue answers like an unreachable server.
#[derive(Default)]
pub struct FakeTransport {
    responses: Mutex<Vec<Result<HttpResponse, Error>>>,
    requests: Mutex<Vec<Recorded>>,
}

impl FakeTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a reply; replies are consumed in the order they were queued.
    pub fn push(&self, r: Result<HttpResponse, Error>) {
        if let Ok(mut q) = self.responses.lock() {
            q.insert(0, r);
        }
    }

    /// Queue a `200 OK` reply with this body and content type.
    pub fn push_ok(&self, content_type: &str, body: Vec<u8>) {
        self.push(Ok(HttpResponse { status: 200, content_type: Some(content_type.to_owned()), body }));
    }

    /// Everything that was asked so far, oldest first.
    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }

    fn answer(&self, rec: Recorded) -> Result<HttpResponse, Error> {
        let url = rec.url.clone();
        if let Ok(mut r) = self.requests.lock() {
            r.push(rec);
        }
        let next = self.responses.lock().ok().and_then(|mut q| q.pop());
        next.unwrap_or(Err(Error::Unreachable { url, detail: "fake transport: no reply queued".into() }))
    }
}

impl Transport for FakeTransport {
    fn get(&self, url: &str) -> Result<HttpResponse, Error> {
        self.answer(Recorded { method: "GET", url: url.to_owned(), content_type: None, body: Vec::new() })
    }

    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<HttpResponse, Error> {
        self.answer(Recorded { method: "POST", url: url.to_owned(), content_type: Some(content_type.to_owned()), body: body.to_vec() })
    }
}
