//! The seam between PhotoCraft and the Craft suite's shared AI layer
//! ([craft-ai-integrations](https://github.com/storytold/craft-ai-integrations)).
//!
//! The engine and the UI build for wasm and never open a socket, so they talk to generative AI
//! only through [`GenerativeAi`], which the desktop app implements over the shared client
//! (`apps/photocraft/src/genai.rs`). The shared layer owns connections, keys, the choice of model
//! and consent records; PhotoCraft owns its commands, its pixels and its windows. Without an
//! implementation (the web build, the CLI, tests) generative commands are simply unavailable and
//! nothing is ever sent.
//!
//! The flow, for each fill: [`GenerativeAi::status`] says whether the tool can run (asked each
//! time it is about to be used), [`GenerativeAi::plan`] checks a request against the chosen model
//! and says what would leave the computer and where ([`Disclosure`]), the person agrees through
//! PhotoCraft's own prompt when [`FillPlan::needs_consent`], and [`FillPlan::start`] submits a
//! job that is polled and cancelled from an engine background job.
//!
//! Nothing here holds or prints a key: the layer reads it inside `start` and nowhere else.

use std::sync::Arc;

use serde::Serialize;

/// Whether Generative Fill can run now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum AiStatus {
    /// It can: the model the person chose, and where it runs.
    Ready {
        /// The model's name, such as "GPT Image 1".
        model: String,
        /// The connection it is reached through, such as "OpenAI".
        connection: String,
        /// Runs on this computer: nothing leaves it.
        local: bool,
        /// How the connection says it is paid for (each request, for a metered one).
        billing: AiBilling,
    },
    /// It can't. `reason` is a sentence for a person, shown as it is (not as an error).
    Unavailable { reason: String },
}

/// How a connection is paid for, as the shared layer records it. PhotoCraft never knows a price.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AiBilling {
    /// Free of charge.
    Free,
    /// Each request is billed by the service.
    Metered,
    /// Requests use a subscription's allowance.
    Subscription,
    /// Not recorded: the service's own terms say.
    #[default]
    Unknown,
}

impl AiStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, AiStatus::Ready { .. })
    }
}

/// Why a request failed, as the shared layer classifies it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AiErrorKind {
    /// This app or the chosen model can't do it.
    Unsupported,
    /// The request was wrong (a bug here, or bad input).
    InvalidRequest,
    /// The key was refused: fix it in the AI setup.
    Auth,
    /// Too many requests for now: try again shortly.
    RateLimited,
    /// Out of credit or allowance: no retry.
    Quota,
    /// The service could not be reached.
    Network,
    /// The service failed.
    Provider,
    /// The person stopped it. Not an error.
    Cancelled,
    /// The person must agree to send this first.
    ConsentRequired,
    /// Something went wrong inside the AI layer.
    Internal,
}

/// A failed AI call. `message` is safe to show (it never holds a key).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AiFailure {
    pub kind: AiErrorKind,
    pub message: String,
    /// For [`AiErrorKind::RateLimited`]: how long the service asked to wait, when it said.
    pub retry_after_secs: Option<u64>,
}

impl AiFailure {
    pub fn new(kind: AiErrorKind, message: impl Into<String>) -> AiFailure {
        AiFailure { kind, message: message.into(), retry_after_secs: None }
    }
}

impl std::fmt::Display for AiFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// What a Generative Fill sends: the area around the selection, the selection as a mask (both PNG,
/// the same size), the prompt and any reference pictures. These are the `generative-fill` tool's
/// inputs: source, mask, prompt and optional references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FillRequest {
    /// The area around the selection, sRGB PNG.
    pub source_png: Vec<u8>,
    /// The selection over that area, PNG with an alpha channel: transparent (and white) where the
    /// fill goes, opaque black where the picture is kept, partly transparent along a feathered
    /// edge. Transparent-means-edit is the convention mask-based image models read.
    pub mask_png: Vec<u8>,
    /// What to put there; may be empty (continue the picture through the selection).
    pub prompt: String,
    /// Pictures the person added to show what the fill should look like, sRGB PNG, in the order
    /// added. A guide only, never part of the document; usually none.
    pub references: Vec<Vec<u8>>,
}

/// Which input of the request a [`SentItem`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SentInput {
    /// The picture around the selection.
    Source,
    /// The selection's shape.
    Mask,
    /// The prompt.
    Prompt,
    /// A reference picture the person added (one item each).
    Reference,
    /// Anything else the layer reports.
    Other,
}

/// What kind of data a [`SentItem`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SentKind {
    Image,
    Text,
    Other,
}

/// One thing a request sends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SentItem {
    pub input: SentInput,
    pub kind: SentKind,
    /// Its size in bytes, when known.
    pub bytes: Option<u64>,
}

/// What a request would send, and where: what the consent prompt shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Disclosure {
    /// Where it goes: `https://host`, or the service's name when it has no address.
    pub destination: String,
    /// The connection's name, such as "OpenAI".
    pub connection: String,
    /// The model that will do the work.
    pub model: String,
    /// On this computer: nothing leaves it.
    pub local: bool,
    pub items: Vec<SentItem>,
}

/// How long a yes to sending lasts: the four choices every Craft app's consent prompt offers,
/// shortest first. Only `ThisTime` leaves nothing on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConsentTerm {
    ThisTime,
    OneHour,
    OneDay,
    UntilTurnedOff,
}

impl ConsentTerm {
    pub const ALL: [ConsentTerm; 4] = [ConsentTerm::ThisTime, ConsentTerm::OneHour, ConsentTerm::OneDay, ConsentTerm::UntilTurnedOff];

    /// The stable name commands use (`this_time`, `one_hour`, `one_day`, `until_revoked`), the
    /// shared layer's own ids.
    pub fn id(self) -> &'static str {
        match self {
            ConsentTerm::ThisTime => "this_time",
            ConsentTerm::OneHour => "one_hour",
            ConsentTerm::OneDay => "one_day",
            ConsentTerm::UntilTurnedOff => "until_revoked",
        }
    }

    /// The term with stable name `id`, if there is one.
    pub fn from_id(id: &str) -> Option<ConsentTerm> {
        ConsentTerm::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// How a started job stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobPoll {
    /// Queued or running, with a percentage when the service reports one.
    Working(Option<u8>),
    /// Finished: the images it returned (encoded PNG, JPEG or WebP), usually one.
    Done(Vec<Vec<u8>>),
    Failed(AiFailure),
    Cancelled,
}

/// A started job. Methods block briefly (a status request); they run on an engine worker.
pub trait FillJob: Send {
    fn poll(&mut self) -> Result<JobPoll, AiFailure>;
    /// Ask it to stop. Calling it twice, or on a finished job, is fine. `Unsupported` when the
    /// service can't stop an unfinished job: it keeps running and its result is thrown away.
    fn cancel(&mut self) -> Result<(), AiFailure>;
}

/// A checked request, waiting for consent and a start. Holds no key.
pub trait FillPlan: Send + Sync {
    /// What it sends, and where.
    fn disclosure(&self) -> &Disclosure;
    /// Must the person agree before it starts? Nothing on file covers it (or it lapsed), and no
    /// [`agree`](Self::agree) on this plan does.
    fn needs_consent(&self) -> bool;
    /// The person agreed, for `term`. A remembered term is saved in the app's consent record;
    /// `ThisTime` covers this plan's jobs only and writes nothing.
    fn agree(&self, term: ConsentTerm) -> Result<(), AiFailure>;
    /// Submit one job. Refuses with [`AiErrorKind::ConsentRequired`] when consent is missing.
    /// Blocks: reads the key and calls the service. Each call is a separate request.
    fn start(&self) -> Result<Box<dyn FillJob>, AiFailure>;
}

/// A service a person can connect in PhotoCraft's own setup (Preferences › Integrations).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceKind {
    /// OpenAI, with the person's own API key.
    OpenAi,
    /// Black Forest Labs, with the person's own API key.
    BlackForestLabs,
    /// Google Gemini (the Gemini API, not Vertex), with the person's own AI Studio API key.
    Gemini,
    /// A ComfyUI server, by address (on this computer, usually).
    ComfyUi,
}

impl ServiceKind {
    pub const ALL: [ServiceKind; 4] = [ServiceKind::OpenAi, ServiceKind::BlackForestLabs, ServiceKind::Gemini, ServiceKind::ComfyUi];

    /// Does connecting it take a key (else an address)?
    pub fn needs_key(self) -> bool {
        !matches!(self, ServiceKind::ComfyUi)
    }
}

/// A saved connection, as setup lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConnectionRow {
    pub id: String,
    pub label: String,
    /// Its address, when it has one.
    pub address: Option<String>,
    pub local: bool,
    /// What is wrong with it, or `None` when it is usable.
    pub problem: Option<ConnectionProblem>,
}

/// Why a saved connection can't be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionProblem {
    /// It needs a key and none is saved.
    NoKey,
    /// The key store could not be read (a locked keychain).
    KeyUnreadable,
    /// It is switched off.
    SwitchedOff,
}

/// A model that could do Generative Fill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelRow {
    pub connection: String,
    pub model: String,
    pub label: String,
    /// One line about it (where it runs, what it costs), from the shared layer.
    pub description: String,
    /// The one Generative Fill uses now.
    pub selected: bool,
}

/// What PhotoCraft's AI setup shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SetupView {
    pub connections: Vec<ConnectionRow>,
    /// The models Generative Fill could use, from every connection.
    pub models: Vec<ModelRow>,
    /// Where the shared settings live (shown so a person knows other Craft apps share them).
    pub store: Option<String>,
}

/// Generative AI for this session: the desktop app's implementation over the shared layer.
pub trait GenerativeAi: Send + Sync {
    /// Can Generative Fill run now? Reads settings only; sends nothing and reads no key.
    fn status(&self) -> AiStatus;
    /// Check `request` against the chosen model and say what would be sent. Sends nothing.
    fn plan(&self, request: FillRequest) -> Result<Arc<dyn FillPlan>, AiFailure>;
    /// The connections and models for PhotoCraft's setup. Reads settings only.
    fn setup(&self) -> Result<SetupView, AiFailure>;
    /// Connect a service: `value` is its API key, or for ComfyUI its address. Checks it with the
    /// service (a short request that lists its models, nothing from the document) before saving,
    /// so it blocks: run it off the interface thread. The key goes to the OS keychain through the
    /// shared layer, never to PhotoCraft's settings.
    fn connect(&self, kind: ServiceKind, value: &str) -> Result<(), AiFailure>;
    /// Remove a saved connection (and the choices that used it).
    fn disconnect(&self, connection: &str) -> Result<(), AiFailure>;
    /// Use `model` on `connection` for Generative Fill (the suite's choice, as the launcher's).
    fn choose(&self, connection: &str, model: &str) -> Result<(), AiFailure>;
}

/// A session's generative AI, shared with the background jobs that use it.
pub type SharedAi = Arc<dyn GenerativeAi>;
