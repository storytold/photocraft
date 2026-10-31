//! The few S3 requests a bundle needs (GET, PUT, DELETE of one object), signed with SigV4.

use std::fmt;
use std::time::{Duration, SystemTime};

use photocraft_format::{FormatError, Result};

use crate::sign::{EMPTY_SHA256, Signer, encode_path, sha256_hex};

/// An S3-compatible bucket and how to sign in to it.
#[derive(Debug, Clone)]
pub struct S3Config {
    /// The service URL without the bucket: `https://s3.us-east-1.amazonaws.com`,
    /// `https://<account>.r2.cloudflarestorage.com`, `http://127.0.0.1:9000`.
    pub endpoint: String,
    /// Signing region: the bucket's region on AWS, `auto` on R2, `us-east-1` on MinIO.
    pub region: String,
    pub bucket: String,
    pub credentials: Credentials,
    /// Address the bucket as `bucket.host/key` rather than `host/bucket/key`. Path style works on
    /// AWS, R2 and MinIO; some providers want virtual-hosted.
    pub virtual_hosted: bool,
}

/// An access key. `Debug` leaves the secret and the session token out.
#[derive(Clone)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    /// For temporary credentials (AWS STS).
    pub session_token: Option<String>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"<redacted>")
            .field("session_token", &self.session_token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// When a PUT may replace what is stored.
#[derive(Clone, Copy)]
pub(crate) enum Condition<'a> {
    Always,
    /// Only if nothing is stored under the key (`If-None-Match: *`).
    IfAbsent,
    /// Only if the stored object still has this ETag (`If-Match`).
    IfMatch(&'a str),
}

pub(crate) enum Put {
    Done {
        etag: Option<String>,
    },
    /// 412: the condition did not hold.
    PreconditionFailed,
    /// 409: another conditional write to the key was in flight.
    Conflicting,
}

#[derive(Clone, Copy)]
enum Method {
    Get,
    Put,
    Delete,
}

impl Method {
    fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}

type Response = ureq::http::Response<ureq::Body>;

pub(crate) struct Client {
    agent: ureq::Agent,
    /// `scheme://authority` requests go to.
    origin: String,
    /// The `Host` header ureq sends for `origin` (no default port), which the signature covers.
    host: String,
    /// `/bucket` (path style) or empty (virtual-hosted).
    bucket_path: String,
    region: String,
    credentials: Credentials,
}

fn unsupported(msg: String) -> FormatError {
    FormatError::Unsupported(msg)
}

pub(crate) fn io_error(msg: String) -> FormatError {
    FormatError::Io(std::io::Error::other(msg))
}

impl Client {
    pub fn new(config: S3Config) -> Result<Self> {
        let (scheme, rest) =
            config.endpoint.split_once("://").ok_or_else(|| unsupported(format!("S3 endpoint `{}` has no http:// or https://", config.endpoint)))?;
        let scheme = scheme.to_ascii_lowercase();
        let default_port = match scheme.as_str() {
            "https" => ":443",
            "http" => ":80",
            _ => return Err(unsupported(format!("S3 endpoint `{}`: only http and https", config.endpoint))),
        };
        let authority = rest.strip_suffix('/').unwrap_or(rest).to_ascii_lowercase();
        if authority.is_empty() || authority.contains(['/', '?', '#', '@']) {
            return Err(unsupported(format!("S3 endpoint `{}` must be scheme://host[:port], without a path", config.endpoint)));
        }
        let authority = authority.strip_suffix(default_port).unwrap_or(&authority).to_string();
        // S3 bucket naming rules (R2 and MinIO follow them): nothing that could change the path.
        let bucket = &config.bucket;
        if bucket.is_empty() || !bucket.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.') {
            return Err(unsupported(format!("S3 bucket name `{bucket}`: only a-z, 0-9, `-` and `.`")));
        }
        let (host, bucket_path) = if config.virtual_hosted { (format!("{bucket}.{authority}"), String::new()) } else { (authority, format!("/{bucket}")) };
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            // A signed request is never re-sent somewhere else.
            .max_redirects(0)
            .max_redirects_will_error(false)
            .user_agent(concat!("PhotoCraft/", env!("CARGO_PKG_VERSION")))
            .build()
            .new_agent();
        Ok(Client { agent, origin: format!("{scheme}://{host}"), host, bucket_path, region: config.region, credentials: config.credentials })
    }

    fn call(&self, method: Method, key: &str, body: &[u8], condition: Condition<'_>) -> Result<Response> {
        let path = format!("{}/{}", self.bucket_path, encode_path(key));
        let url = format!("{}{path}", self.origin);
        let payload = if body.is_empty() { EMPTY_SHA256.to_string() } else { sha256_hex(body) };
        let amz_date = chrono::DateTime::<chrono::Utc>::from(SystemTime::now()).format("%Y%m%dT%H%M%SZ").to_string();
        let mut headers: Vec<(&str, &str)> = vec![("x-amz-content-sha256", &payload), ("x-amz-date", &amz_date)];
        if let Some(token) = &self.credentials.session_token {
            headers.push(("x-amz-security-token", token));
        }
        match condition {
            Condition::Always => {}
            Condition::IfAbsent => headers.push(("if-none-match", "*")),
            Condition::IfMatch(etag) => headers.push(("if-match", etag)),
        }
        let signer = Signer { access_key_id: &self.credentials.access_key_id, secret_access_key: &self.credentials.secret_access_key, region: &self.region };
        let auth = signer.authorization(method.as_str(), &self.host, &path, &amz_date, &headers, &payload);
        let response = match method {
            Method::Get => signed(self.agent.get(&url), &headers, &auth).call(),
            Method::Delete => signed(self.agent.delete(&url), &headers, &auth).call(),
            Method::Put => signed(self.agent.put(&url), &headers, &auth).send(body),
        };
        response.map_err(|e| io_error(format!("S3 {} `{key}`: {e}", method.as_str())))
    }

    /// The object and its ETag, or `None` when there is no such key. Refuses objects over `max`
    /// bytes.
    pub fn get(&self, key: &str, max: usize) -> Result<Option<(Vec<u8>, Option<String>)>> {
        let mut response = self.call(Method::Get, key, &[], Condition::Always)?;
        match response.status().as_u16() {
            200 => {}
            404 => return Ok(None),
            status => return Err(failure(Method::Get, key, status, &mut response)),
        }
        let too_big = || FormatError::LimitExceeded(format!("S3 object `{key}` is larger than {max} bytes"));
        let limit = u64::try_from(max).unwrap_or(u64::MAX);
        if response.body().content_length().is_some_and(|len| len > limit) {
            return Err(too_big());
        }
        let etag = etag(&response);
        // ureq refuses a body that reaches its limit, so allow one byte more and check here.
        let bytes = response.body_mut().with_config().limit(limit.saturating_add(1)).read_to_vec().map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => too_big(),
            e => io_error(format!("S3 GET `{key}`: {e}")),
        })?;
        if bytes.len() > max {
            return Err(too_big());
        }
        Ok(Some((bytes, etag)))
    }

    pub fn put(&self, key: &str, body: &[u8], condition: Condition<'_>) -> Result<Put> {
        let mut response = self.call(Method::Put, key, body, condition)?;
        match response.status().as_u16() {
            200 | 201 | 204 => Ok(Put::Done { etag: etag(&response) }),
            412 => Ok(Put::PreconditionFailed),
            409 => Ok(Put::Conflicting),
            status => Err(failure(Method::Put, key, status, &mut response)),
        }
    }

    /// Removes `key`; a key that is not there is not an error.
    pub fn delete(&self, key: &str) -> Result<()> {
        let mut response = self.call(Method::Delete, key, &[], Condition::Always)?;
        match response.status().as_u16() {
            200 | 204 | 404 => Ok(()),
            status => Err(failure(Method::Delete, key, status, &mut response)),
        }
    }
}

fn signed<B>(mut request: ureq::RequestBuilder<B>, headers: &[(&str, &str)], authorization: &str) -> ureq::RequestBuilder<B> {
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request.header("authorization", authorization)
}

fn etag(response: &Response) -> Option<String> {
    response.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// An unexpected status, with the `<Code>` and `<Message>` of S3's XML error body when it has them.
fn failure(method: Method, key: &str, status: u16, response: &mut Response) -> FormatError {
    let body = response.body_mut().with_config().limit(64 << 10).read_to_string().unwrap_or_default();
    let field = |tag: &str| {
        let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
        let start = body.find(&open)? + open.len();
        let len = body.get(start..)?.find(&close)?;
        body.get(start..start + len)
    };
    let detail = match (field("Code"), field("Message")) {
        (Some(code), Some(message)) => format!(": {code}: {message}"),
        (Some(code), None) => format!(": {code}"),
        _ => String::new(),
    };
    io_error(format!("S3 {} `{key}`: HTTP {status}{detail}", method.as_str()))
}
