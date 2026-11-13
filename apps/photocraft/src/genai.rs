//! Generative Fill over the Craft suite's shared AI layer
//! ([craft-ai-integrations](https://github.com/storytold/craft-ai-integrations)): the desktop
//! app's [`GenerativeAi`]. Connections, keys, the chosen model and consent records are the
//! suite's, shared with the launcher and the other Craft apps; PhotoCraft only asks for the
//! catalogue's `generative-fill` tool and draws its own windows.
//!
//! Nothing here reads, stores or prints a key: the shared layer reads it inside `start` and keeps
//! it in the OS keychain. No model, service or address is written into PhotoCraft; they come from
//! the person's choice. Nothing is sent until a fill starts, and nothing leaves the computer
//! without the person's agreement (PhotoCraft's own prompt, recorded in its consent file).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use craft_ai_client::{AiClient, ConnectionManager, GroupState, KeyState, RoleStatus, SaveError, ToolDesk};
use craft_ai_core::{
    AiError, AppId, Billing, ConnectionDraft, ConnectionId, ConsentLedger, ErrorKind, Event, InputRole, JobState, Locality, ModelId, PRESETS, Payload,
    PayloadKind, Request, Role,
};
use craft_ai_native::ConsentFile;
use craft_ai_native::secrets::{EnvStore, KeychainStore, LayeredStore, SecretStore};
use craft_ai_native::shared::SharedStore;
use photocraft_engine::genai::{
    AiBilling, AiErrorKind, AiFailure, AiStatus, ConnectionProblem, ConnectionRow, ConsentTerm, Disclosure, FillJob, FillPlan, FillRequest, GenerativeAi,
    JobPoll, ModelRow, SentInput, SentItem, SentKind, ServiceKind, SetupView, SharedAi,
};

/// The app id the launcher knows PhotoCraft by: consent and per-app choices are kept under it.
const APP: &str = "photocraft";
/// The catalogue's tool PhotoCraft's Generative Fill is.
const TOOL: &str = "generative-fill";
/// How long a status answer is used before it is read again. The menus and the options bar ask
/// every frame they are drawn; this keeps that from reading the shared settings (and the
/// keychain) dozens of times a second, while a change made in another Craft app shows within a
/// second or so. An answer past it is still given while a worker reads the new one.
#[cfg(not(test))]
const STATUS_TTL: Duration = Duration::from_secs(1);
#[cfg(test)]
const STATUS_TTL: Duration = Duration::from_millis(50);

/// The app's generative AI: the shared layer if it can be reached, else one that says why not.
pub fn shared() -> SharedAi {
    match CraftAi::standard() {
        Ok(ai) => Arc::new(ai),
        Err(e) => Arc::new(Unreachable(e.message)),
    }
}

/// The shared layer through PhotoCraft's client.
pub struct CraftAi {
    client: Arc<AiClient>,
    shared: SharedStore,
    secrets: Arc<dyn SecretStore>,
    manager: ConnectionManager,
    role: Role,
    consent: Option<ConsentFile>,
    status: Arc<StatusCache>,
}

/// The tool's status as last read, shared with the worker that reads it again. Reading it
/// reaches the OS keychain, which can block while an unlock prompt is open, so once there is an
/// answer it is never read again on the interface thread.
#[derive(Default)]
struct StatusCache {
    value: Mutex<Option<(Instant, AiStatus)>>,
    /// A worker is reading it.
    refreshing: AtomicBool,
    /// Bumped when the answer is forgotten, so a read that started before doesn't bring it back.
    generation: AtomicU64,
}

impl StatusCache {
    /// The answer on hand and when it was read, if any.
    fn get(&self) -> Option<(Instant, AiStatus)> {
        self.value.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Keep `s`, read when the cache was at `generation`, unless it was forgotten since.
    fn put(&self, generation: u64, s: AiStatus) {
        let mut v = self.value.lock().unwrap_or_else(PoisonError::into_inner);
        if self.generation.load(Ordering::SeqCst) == generation {
            *v = Some((Instant::now(), s));
        }
    }
}

/// Whether the tool can run, read from the shared layer (the settings and the keychain).
fn read_status(client: &AiClient, role: &Role) -> AiStatus {
    match client.status(role) {
        RoleStatus::Ready(r) => AiStatus::Ready {
            model: r.model_label,
            connection: r.connection_label,
            local: r.locality == Locality::Local,
            billing: match r.billing {
                Billing::Free => AiBilling::Free,
                Billing::Metered => AiBilling::Metered,
                Billing::SubscriptionAllowance => AiBilling::Subscription,
                Billing::Unknown => AiBilling::Unknown,
            },
        },
        RoleStatus::Unavailable { reason } => AiStatus::Unavailable { reason },
    }
}

/// The `generative-fill` tool, from the shared catalogue.
fn fill_role() -> Result<Role, AiError> {
    craft_ai_features::builtin()
        .into_iter()
        .find(|f| f.id == TOOL)
        .and_then(|f| f.role())
        .ok_or_else(|| AiError::new(ErrorKind::Internal, "The shared AI catalogue has no Generative Fill tool."))
}

/// PhotoCraft's app id in the shared layer.
fn app_id() -> Result<AppId, AiError> {
    AppId::new(APP).map_err(|e| AiError::new(ErrorKind::Internal, e.to_string()))
}

impl CraftAi {
    /// The usual setup: the shared settings folder (or `CRAFT_AI_DIR`), keys from the
    /// environment and then the OS keychain, the standard adapters, and PhotoCraft's consent file.
    pub fn standard() -> Result<CraftAi, AiError> {
        let client = AiClient::standard(app_id()?)?;
        let shared = SharedStore::at_default_location()?;
        let secrets: Arc<dyn SecretStore> = Arc::new(LayeredStore::new(vec![Box::new(EnvStore::new()), Box::new(KeychainStore::new())]));
        let manager = ConnectionManager::new(shared.clone(), Arc::clone(&secrets));
        let consent = client.consent_file();
        let ai = CraftAi { client: Arc::new(client), shared, secrets, manager, role: fill_role()?, consent, status: Arc::default() };
        // Read it off the interface thread before the first menu asks.
        ai.refresh_in_background();
        Ok(ai)
    }

    /// Built from parts (tests: a temporary store, an in-memory key store, scripted adapters).
    #[cfg(test)]
    fn with_parts(
        shared: SharedStore,
        secrets: Arc<dyn SecretStore>,
        factory: craft_ai_client::ProviderFactory,
        probe: craft_ai_client::ProviderFactory,
        consent: ConsentFile,
    ) -> CraftAi {
        #[allow(clippy::expect_used)]
        let app = app_id().expect("a valid literal id");
        let client = AiClient::new(app, shared.clone(), Arc::clone(&secrets)).with_factory(factory);
        let manager = ConnectionManager::new(shared.clone(), Arc::clone(&secrets)).with_probe_factory(probe);
        #[allow(clippy::expect_used)]
        let role = fill_role().expect("the catalogue has generative-fill");
        CraftAi { client: Arc::new(client), shared, secrets, manager, role, consent: Some(consent), status: Arc::default() }
    }

    /// Forget the status after a change made here (a connection, a choice): the next ask reads it.
    fn forget_status(&self) {
        let mut v = self.status.value.lock().unwrap_or_else(PoisonError::into_inner);
        self.status.generation.fetch_add(1, Ordering::SeqCst);
        *v = None;
    }

    /// Read the status again on a worker, unless one is already reading it.
    fn refresh_in_background(&self) {
        if self.status.refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        let (client, role, cache) = (Arc::clone(&self.client), self.role.clone(), Arc::clone(&self.status));
        let generation = cache.generation.load(Ordering::SeqCst);
        let worker = Arc::clone(&cache);
        let spawned = std::thread::Builder::new().name("photocraft-ai-status".into()).spawn(move || {
            worker.put(generation, read_status(&client, &role));
            worker.refreshing.store(false, Ordering::SeqCst);
        });
        // No thread: the answer on hand stays until the next try.
        if spawned.is_err() {
            cache.refreshing.store(false, Ordering::SeqCst);
        }
    }

    /// The launcher's side, for choosing the tool's model for the whole suite.
    fn desk(&self) -> Result<ToolDesk, AiError> {
        let launcher = AiClient::new(ToolDesk::app()?, self.shared.clone(), Arc::clone(&self.secrets));
        Ok(ToolDesk::new(launcher, self.shared.clone(), None))
    }
}

/// A shared-layer error as PhotoCraft's seam carries it. The message is the layer's, safe to
/// show (it never holds a key).
fn failure(e: AiError) -> AiFailure {
    let kind = match e.kind {
        ErrorKind::Unsupported => AiErrorKind::Unsupported,
        ErrorKind::InvalidRequest => AiErrorKind::InvalidRequest,
        ErrorKind::Auth => AiErrorKind::Auth,
        ErrorKind::RateLimited => AiErrorKind::RateLimited,
        ErrorKind::Quota => AiErrorKind::Quota,
        ErrorKind::Network => AiErrorKind::Network,
        ErrorKind::Provider => AiErrorKind::Provider,
        ErrorKind::Cancelled => AiErrorKind::Cancelled,
        ErrorKind::ConsentRequired => AiErrorKind::ConsentRequired,
        ErrorKind::Internal => AiErrorKind::Internal,
    };
    AiFailure { kind, message: e.message, retry_after_secs: e.retry_after_secs }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A consent term as the shared layer names it.
fn core_term(t: ConsentTerm) -> craft_ai_core::ConsentTerm {
    match t {
        ConsentTerm::ThisTime => craft_ai_core::ConsentTerm::ThisTime,
        ConsentTerm::OneHour => craft_ai_core::ConsentTerm::OneHour,
        ConsentTerm::OneDay => craft_ai_core::ConsentTerm::OneDay,
        ConsentTerm::UntilTurnedOff => craft_ai_core::ConsentTerm::UntilRevoked,
    }
}

impl GenerativeAi for CraftAi {
    /// The last answer, read again on a worker once it is older than [`STATUS_TTL`]. Only with
    /// no answer yet (before the first read finished, or right after a change made here) is it
    /// read on the caller's thread.
    fn status(&self) -> AiStatus {
        match self.status.get() {
            Some((at, s)) if at.elapsed() < STATUS_TTL => s,
            Some((_, s)) => {
                self.refresh_in_background();
                s
            }
            None => {
                let generation = self.status.generation.load(Ordering::SeqCst);
                let s = read_status(&self.client, &self.role);
                self.status.put(generation, s.clone());
                s
            }
        }
    }

    /// Check the request against the chosen model and say what it would send, and where.
    fn plan(&self, request: FillRequest) -> Result<Arc<dyn FillPlan>, AiFailure> {
        let mut req = Request::new(self.role.capability.clone())
            .with_input(InputRole::Source, Payload::inline("image/png", request.source_png).map_err(failure)?)
            .with_input(InputRole::Mask, Payload::inline("image/png", request.mask_png).map_err(failure)?)
            .with_input(InputRole::Prompt, Payload::text(request.prompt));
        // After the source, mask and prompt, in the order added: the shared layer numbers them
        // from there, and a route that takes none refuses the plan here, before consent.
        for png in request.references {
            req = req.with_input(InputRole::Reference, Payload::inline("image/png", png).map_err(failure)?);
        }
        let mut plan = self.client.plan(&self.role, req.clone()).map_err(failure)?;
        // OpenAI picks its own quality unless told, and fills came back soft; other adapters
        // refuse params they don't know, so this is asked only of OpenAI.
        if plan.provider() == "openai" {
            plan = self.client.plan(&self.role, req.with_param("quality", serde_json::json!("high"))).map_err(failure)?;
        }
        let d = plan.disclosure();
        let disclosure = Disclosure {
            destination: d.destination.clone(),
            connection: plan.connection_label().to_owned(),
            model: model_label(&self.shared, plan.model()),
            local: d.locality == Locality::Local,
            items: d
                .items
                .iter()
                .map(|i| SentItem {
                    input: match i.role {
                        InputRole::Source => SentInput::Source,
                        InputRole::Mask => SentInput::Mask,
                        InputRole::Prompt => SentInput::Prompt,
                        InputRole::Reference => SentInput::Reference,
                        _ => SentInput::Other,
                    },
                    kind: match i.kind {
                        PayloadKind::Image => SentKind::Image,
                        PayloadKind::Text => SentKind::Text,
                        _ => SentKind::Other,
                    },
                    bytes: i.bytes,
                })
                .collect(),
        };
        Ok(Arc::new(Planned { client: Arc::clone(&self.client), plan, disclosure, consent: self.consent.clone(), this_time: Mutex::new(None) }))
    }

    /// The saved connections and the models Generative Fill could use, from the shared settings.
    fn setup(&self) -> Result<SetupView, AiFailure> {
        let connections = self
            .manager
            .views()
            .map_err(failure)?
            .into_iter()
            .map(|v| ConnectionRow {
                id: v.connection.id.as_str().to_owned(),
                label: v.connection.label.clone(),
                address: v.connection.endpoint.clone(),
                local: v.connection.locality == Locality::Local,
                problem: if !v.connection.enabled {
                    Some(ConnectionProblem::SwitchedOff)
                } else {
                    match v.key {
                        KeyState::Missing => Some(ConnectionProblem::NoKey),
                        KeyState::Unreadable => Some(ConnectionProblem::KeyUnreadable),
                        KeyState::Saved | KeyState::NotNeeded => None,
                    }
                },
            })
            .collect();
        let picker = self.client.picker(&self.role, &craft_ai_features::setup_options(&craft_ai_features::builtin())).map_err(failure)?;
        let models = picker
            .groups
            .into_iter()
            .filter(|g| g.state == GroupState::Ready)
            .flat_map(|g| g.models)
            .map(|m| ModelRow {
                connection: m.connection.as_str().to_owned(),
                model: m.model.as_str().to_owned(),
                label: m.label,
                description: m.description,
                selected: m.selected,
            })
            .collect();
        Ok(SetupView { connections, models, store: Some(self.manager.store_path().display().to_string()) })
    }

    /// Check a new connection with the service, then save it (its key to the keychain).
    fn connect(&self, kind: ServiceKind, value: &str) -> Result<(), AiFailure> {
        let key = preset_key(kind);
        let preset = PRESETS
            .iter()
            .find(|p| p.key == key)
            .ok_or_else(|| AiFailure::new(AiErrorKind::Unsupported, "This service isn't offered by the shared AI layer."))?;
        let mut draft = ConnectionDraft::from_preset(preset);
        let value = value.trim();
        if kind.needs_key() {
            draft.key = value.to_owned();
        } else if !value.is_empty() {
            draft.endpoint = value.to_owned();
        }
        // Ask the service for its models before keeping anything, so a wrong key or address is
        // found now, and the models it really has are what PhotoCraft offers.
        let address = draft.endpoint.trim().to_owned();
        let report = self.manager.test_draft(&draft).map_err(failure)?;
        if !report.models.is_empty() {
            let _ = draft.set_models(&address, report.models);
        }
        let saved = self.manager.save(&draft);
        self.forget_status();
        match saved {
            Ok(_) => Ok(()),
            Err(SaveError::Invalid(e)) => Err(AiFailure::new(AiErrorKind::InvalidRequest, e.message)),
            Err(SaveError::Failed(e)) => Err(failure(e)),
        }
    }

    /// Remove a saved connection and the choices that used it.
    fn disconnect(&self, connection: &str) -> Result<(), AiFailure> {
        let id = ConnectionId::new(connection).map_err(|e| AiFailure::new(AiErrorKind::InvalidRequest, e.to_string()))?;
        let r = self.manager.delete(&id).map(|_| ()).map_err(failure);
        self.forget_status();
        r
    }

    /// Make `model` on `connection` the suite's choice for Generative Fill.
    fn choose(&self, connection: &str, model: &str) -> Result<(), AiFailure> {
        let bad = |e: craft_ai_core::ids::IdError| AiFailure::new(AiErrorKind::InvalidRequest, e.to_string());
        let (connection, model) = (ConnectionId::new(connection).map_err(bad)?, ModelId::new(model).map_err(bad)?);
        let r = self.desk().and_then(|d| d.choose(&self.role, &connection, &model)).map_err(failure);
        self.forget_status();
        r
    }
}

/// The shared layer's connection preset for a service PhotoCraft's setup offers.
fn preset_key(kind: ServiceKind) -> &'static str {
    match kind {
        ServiceKind::OpenAi => "openai",
        ServiceKind::BlackForestLabs => "bfl",
        ServiceKind::Gemini => "gemini",
        ServiceKind::ComfyUi => "comfyui",
    }
}

/// A model's name as the person's connections call it.
fn model_label(shared: &SharedStore, model: &ModelId) -> String {
    shared
        .load()
        .ok()
        .and_then(|s| s.connections.iter().flat_map(|c| &c.models).find(|m| &m.id == model).map(|m| m.label.clone()))
        .unwrap_or_else(|| model.as_str().to_owned())
}

/// A checked Generative Fill request waiting for consent and a start.
struct Planned {
    client: Arc<AiClient>,
    plan: craft_ai_client::Plan,
    disclosure: Disclosure,
    consent: Option<ConsentFile>,
    /// The ledger a "this time" yes made: it covers this plan's jobs and is never saved.
    this_time: Mutex<Option<ConsentLedger>>,
}

impl Planned {
    /// The ledger to check and start with: this plan's own "this time" yes, or what is on file.
    fn ledger(&self) -> ConsentLedger {
        if let Some(l) = self.this_time.lock().unwrap_or_else(PoisonError::into_inner).clone() {
            return l;
        }
        self.consent.as_ref().map(ConsentFile::load_or_empty).unwrap_or_default()
    }
}

impl FillPlan for Planned {
    fn disclosure(&self) -> &Disclosure {
        &self.disclosure
    }

    /// Does nothing on file, and no yes on this plan, cover it?
    fn needs_consent(&self) -> bool {
        self.plan.needs_consent(&self.ledger())
    }

    /// Record the person's yes for `term`; only a remembered term is written to the consent file.
    fn agree(&self, term: ConsentTerm) -> Result<(), AiFailure> {
        let term = core_term(term);
        let mut saved = self.consent.as_ref().map(ConsentFile::load_or_empty).unwrap_or_default();
        let for_job = saved.agree(self.plan.disclosure(), term, now_secs());
        // Only a remembered yes is written; "this time" leaves nothing on disk.
        if term.remembers()
            && let Some(file) = &self.consent
        {
            saved.forget_lapsed(now_secs());
            file.save(&saved).map_err(failure)?;
        }
        *self.this_time.lock().unwrap_or_else(PoisonError::into_inner) = Some(for_job);
        Ok(())
    }

    /// Submit one job with this plan's consent (reads the key and calls the service).
    fn start(&self) -> Result<Box<dyn FillJob>, AiFailure> {
        let job = self.client.start(&self.plan, &self.ledger()).map_err(failure)?;
        Ok(Box::new(Running { job, images: Vec::new() }))
    }
}

/// A started job, polled from the engine's background job.
struct Running {
    job: craft_ai_client::Job,
    /// Pictures that arrived so far (events come once each).
    images: Vec<Vec<u8>>,
}

impl FillJob for Running {
    /// Where the job stands, keeping the pictures that arrived.
    fn poll(&mut self) -> Result<JobPoll, AiFailure> {
        let snap = self.job.poll().map_err(failure)?;
        for e in snap.events {
            if let Event::Output(o) = e
                && let Payload::Inline { media_type, data } = o.payload
                && media_type.as_str().starts_with("image/")
            {
                self.images.push(data);
            }
        }
        Ok(match snap.state {
            JobState::Queued => JobPoll::Working(None),
            JobState::Running { percent } => JobPoll::Working(percent),
            JobState::Succeeded if self.images.is_empty() => {
                JobPoll::Failed(AiFailure::new(AiErrorKind::Provider, "The AI service finished without returning a picture PhotoCraft can read."))
            }
            JobState::Succeeded => JobPoll::Done(std::mem::take(&mut self.images)),
            JobState::Failed { error } => JobPoll::Failed(failure(error)),
            JobState::Cancelled => JobPoll::Cancelled,
        })
    }

    /// Ask the service to stop the job.
    fn cancel(&mut self) -> Result<(), AiFailure> {
        self.job.cancel().map_err(failure)
    }
}

/// The shared AI folder can't be found (no home folder, no `CRAFT_AI_DIR`): the tool is
/// unavailable and says why.
struct Unreachable(String);

impl GenerativeAi for Unreachable {
    fn status(&self) -> AiStatus {
        AiStatus::Unavailable { reason: self.0.clone() }
    }
    fn plan(&self, _: FillRequest) -> Result<Arc<dyn FillPlan>, AiFailure> {
        Err(AiFailure::new(AiErrorKind::Unsupported, self.0.clone()))
    }
    fn setup(&self) -> Result<SetupView, AiFailure> {
        Err(AiFailure::new(AiErrorKind::Unsupported, self.0.clone()))
    }
    fn connect(&self, _: ServiceKind, _: &str) -> Result<(), AiFailure> {
        Err(AiFailure::new(AiErrorKind::Unsupported, self.0.clone()))
    }
    fn disconnect(&self, _: &str) -> Result<(), AiFailure> {
        Err(AiFailure::new(AiErrorKind::Unsupported, self.0.clone()))
    }
    fn choose(&self, _: &str, _: &str) -> Result<(), AiFailure> {
        Err(AiFailure::new(AiErrorKind::Unsupported, self.0.clone()))
    }
}

#[cfg(test)]
mod tests;
