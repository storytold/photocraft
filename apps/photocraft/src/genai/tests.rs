//! The desktop link to the shared AI layer, with no network and no real settings: a temporary
//! shared folder, an in-memory key store, and a scripted adapter registered on a
//! `ProviderFactory` (the adopting-scene pattern from craft-ai-integrations).

use std::sync::Arc;

use craft_ai_client::{AiClient, ProviderFactory, ToolDesk};
use craft_ai_core::mock::{MockProvider, Outcome, Script};
use craft_ai_core::{
    AiError, AppId, CapabilitySpec, Connection, ConnectionId, ErrorKind, InputRole, ModelId, ModelInfo, Output, Payload, Provider, SecretRef, SecretString,
};
use craft_ai_native::ConsentFile;
use craft_ai_native::secrets::{MemoryStore, SecretStore};
use craft_ai_native::shared::SharedStore;
use photocraft_codecs::{ChannelLayout, Format, Image};
use photocraft_engine::Session;
use photocraft_engine::genai::{AiBilling, AiErrorKind, AiStatus, ConnectionProblem, ConsentTerm, FillRequest, GenerativeAi, JobPoll, SentInput, ServiceKind};
use photocraft_engine::genai_cmds::{AGREE, FILL, FILLS};
use serde_json::json;

use super::{CraftAi, failure};

/// A solid-colour PNG.
fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let px: Vec<u8> = (0..w * h).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect();
    let img = Image::from_u8(w, h, ChannelLayout::Rgba, px).unwrap();
    photocraft_codecs::encode(&img, Format::Png, &Default::default()).unwrap()
}

/// A model that can do Generative Fill: inpainting from a source, a mask and a prompt.
fn inpaint_model() -> ModelInfo {
    ModelInfo::new(ModelId::new("fill-1").unwrap(), "Fill One").with(
        CapabilitySpec::new(craft_ai_core::CapabilityId::parse("image.inpaint").unwrap())
            .accepting(InputRole::Source, 1)
            .accepting(InputRole::Mask, 1)
            .accepting(InputRole::Prompt, 1)
            .requiring(InputRole::Source)
            .requiring(InputRole::Mask)
            .requiring(InputRole::Prompt),
    )
}

/// What the scripted service does with each job.
#[derive(Clone)]
enum Answer {
    Picture(Vec<u8>),
    Fail(AiError),
    /// Runs for many polls (to be cancelled).
    Slow,
}

fn script(answer: &Answer) -> Script {
    match answer {
        Answer::Picture(png) => Script {
            running_polls: 1,
            text_chunks: vec![],
            outcome: Outcome::Succeed(vec![Output { payload: Payload::inline("image/png", png.clone()).unwrap(), label: None }]),
        },
        Answer::Fail(e) => Script { running_polls: 0, text_chunks: vec![], outcome: Outcome::Fail(e.clone()) },
        Answer::Slow => Script { running_polls: 100_000, text_chunks: vec![], outcome: Outcome::Succeed(vec![]) },
    }
}

/// The launcher's side, done: a temporary shared folder with a "fake" service (online, with a
/// saved key, or on this computer) offering an inpainting model, optionally chosen for
/// Generative Fill.
struct Scene {
    dir: tempfile::TempDir,
    shared: SharedStore,
    keys: Arc<MemoryStore>,
}

impl Scene {
    fn new(online: bool, chosen: bool) -> Scene {
        let dir = tempfile::tempdir().unwrap();
        let shared = SharedStore::new(dir.path().join("connections.json"));
        let keys = Arc::new(MemoryStore::new());
        let connection = if online {
            keys.set(&SecretRef::new("fake-key").unwrap(), &SecretString::new("not-a-real-key")).unwrap();
            json!({
                "id": "fake", "label": "Fake (online)", "provider": "fake",
                "endpoint": "https://fake.example.com/v1", "locality": "remote",
                "auth": {"kind": "api_key", "secret": "fake-key"},
                "billing": "metered", "models": [inpaint_model()],
            })
        } else {
            json!({
                "id": "fake", "label": "Fake (on this computer)", "provider": "fake",
                "locality": "local", "auth": {"kind": "none"}, "billing": "free", "models": [inpaint_model()],
            })
        };
        let connection: Connection = serde_json::from_value(connection).unwrap();
        shared.update(|store| store.connections.push(connection)).unwrap();
        let scene = Scene { dir, shared, keys };
        if chosen {
            scene.launcher().choose(&super::fill_role().unwrap(), &ConnectionId::new("fake").unwrap(), &ModelId::new("fill-1").unwrap()).unwrap();
        }
        scene
    }

    fn launcher(&self) -> ToolDesk {
        ToolDesk::new(AiClient::new(AppId::new("launcher").unwrap(), self.shared.clone(), self.keys.clone()), self.shared.clone(), None)
    }

    fn consent_path(&self) -> std::path::PathBuf {
        self.dir.path().join("consent-photocraft.json")
    }

    /// PhotoCraft's link, shipping an adapter for the "fake" service that answers `answer`
    /// (or none at all).
    fn ai(&self, answer: Option<Answer>) -> CraftAi {
        let mut factory = ProviderFactory::standard();
        let mut probe = ProviderFactory::standard();
        if let Some(answer) = answer {
            factory.register("fake", move |c: &Connection, _key| Ok(Arc::new(MockProvider::new(c.models.clone(), script(&answer))) as Arc<dyn Provider>));
        } else {
            factory.unregister("fake");
        }
        // "Test" for a new OpenAI connection answers from here, not from OpenAI.
        probe.register("openai", |_c: &Connection, key| {
            if key.is_none() {
                return Err(AiError::new(ErrorKind::Auth, "no key"));
            }
            Ok(Arc::new(MockProvider::new(vec![inpaint_model()], Script::default())) as Arc<dyn Provider>)
        });
        CraftAi::with_parts(self.shared.clone(), self.keys.clone(), factory, probe, ConsentFile::new(self.consent_path()))
    }
}

fn request() -> FillRequest {
    FillRequest { source_png: png(8, 8, [10, 20, 30]), mask_png: png(8, 8, [255, 255, 255]), prompt: "a red balloon".into() }
}

fn wait(job: &mut Box<dyn photocraft_engine::genai::FillJob>) -> JobPoll {
    for _ in 0..1000 {
        match job.poll().unwrap() {
            JobPoll::Working(_) => std::thread::sleep(std::time::Duration::from_millis(1)),
            done => return done,
        }
    }
    panic!("the job never ended");
}

#[test]
fn nothing_chosen_is_unavailable_and_nothing_is_planned() {
    let scene = Scene::new(true, false);
    let ai = scene.ai(Some(Answer::Picture(png(4, 4, [255, 0, 0]))));
    let AiStatus::Unavailable { reason } = ai.status() else { panic!("ready with nothing chosen") };
    assert!(!reason.is_empty());
    assert!(ai.plan(request()).is_err());
}

#[test]
fn the_suite_default_is_ready_and_a_remote_model_needs_consent_first() {
    let scene = Scene::new(true, true);
    let ai = scene.ai(Some(Answer::Picture(png(4, 4, [255, 0, 0]))));
    assert_eq!(ai.status(), AiStatus::Ready { model: "Fill One".into(), connection: "Fake (online)".into(), local: false, billing: AiBilling::Metered });
    let plan = ai.plan(request()).unwrap();
    // What the consent prompt shows: where, and what (kinds and sizes).
    let d = plan.disclosure();
    assert_eq!(d.destination, "https://fake.example.com");
    assert_eq!(d.connection, "Fake (online)");
    assert_eq!(d.model, "Fill One");
    assert!(!d.local);
    assert_eq!(d.items.iter().map(|i| i.input).collect::<Vec<_>>(), [SentInput::Source, SentInput::Mask, SentInput::Prompt]);
    assert_eq!(d.items[0].bytes, Some(request().source_png.len() as u64));
    // Without a yes, nothing starts.
    assert!(plan.needs_consent());
    let refused = plan.start().err().unwrap();
    assert_eq!(refused.kind, AiErrorKind::ConsentRequired);
    // "This time": it starts, and nothing is written.
    plan.agree(ConsentTerm::ThisTime).unwrap();
    assert!(!plan.needs_consent());
    let mut job = plan.start().unwrap();
    assert_eq!(wait(&mut job), JobPoll::Done(vec![png(4, 4, [255, 0, 0])]));
    assert!(!scene.consent_path().exists(), "\"this time\" leaves nothing on disk");
    assert!(ai.plan(request()).unwrap().needs_consent(), "and the next request asks again");
}

#[test]
fn a_remembered_yes_is_saved_in_photocrafts_consent_file() {
    let scene = Scene::new(true, true);
    let ai = scene.ai(Some(Answer::Picture(png(4, 4, [255, 0, 0]))));
    let plan = ai.plan(request()).unwrap();
    plan.agree(ConsentTerm::OneHour).unwrap();
    assert!(scene.consent_path().exists());
    let next = ai.plan(request()).unwrap();
    assert!(!next.needs_consent(), "covered for the hour");
    assert!(next.start().is_ok());
    // It is PhotoCraft's: another app's client isn't covered by it.
    let text = std::fs::read_to_string(scene.consent_path()).unwrap();
    assert!(text.contains("photocraft"), "{text}");
}

#[test]
fn a_model_on_this_computer_needs_no_consent() {
    let scene = Scene::new(false, true);
    let ai = scene.ai(Some(Answer::Picture(png(4, 4, [0, 255, 0]))));
    let AiStatus::Ready { local, .. } = ai.status() else { panic!("not ready") };
    assert!(local);
    let plan = ai.plan(request()).unwrap();
    assert!(plan.disclosure().local);
    assert!(!plan.needs_consent());
    let mut job = plan.start().unwrap();
    assert!(matches!(wait(&mut job), JobPoll::Done(_)));
}

#[test]
fn an_app_without_the_adapter_is_unavailable_and_reads_no_key() {
    let scene = Scene::new(true, true);
    let ai = scene.ai(None);
    let AiStatus::Unavailable { reason } = ai.status() else { panic!("ready without an adapter") };
    assert!(reason.contains("can't run the chosen model"), "{reason}");
    let e = ai.plan(request()).err().unwrap();
    assert_eq!(e.kind, AiErrorKind::Unsupported);
}

#[test]
fn errors_keep_their_kind_and_message() {
    let scene = Scene::new(false, true);
    let ai = scene.ai(Some(Answer::Fail(AiError::new(ErrorKind::Auth, "Fake refused the key."))));
    let mut job = ai.plan(request()).unwrap().start().unwrap();
    let JobPoll::Failed(f) = wait(&mut job) else { panic!("didn't fail") };
    assert_eq!((f.kind, f.message.as_str()), (AiErrorKind::Auth, "Fake refused the key."));
    let pairs = [
        (ErrorKind::Unsupported, AiErrorKind::Unsupported),
        (ErrorKind::InvalidRequest, AiErrorKind::InvalidRequest),
        (ErrorKind::Auth, AiErrorKind::Auth),
        (ErrorKind::RateLimited, AiErrorKind::RateLimited),
        (ErrorKind::Quota, AiErrorKind::Quota),
        (ErrorKind::Network, AiErrorKind::Network),
        (ErrorKind::Provider, AiErrorKind::Provider),
        (ErrorKind::Cancelled, AiErrorKind::Cancelled),
        (ErrorKind::ConsentRequired, AiErrorKind::ConsentRequired),
        (ErrorKind::Internal, AiErrorKind::Internal),
    ];
    for (theirs, ours) in pairs {
        assert_eq!(failure(AiError::new(theirs, "m")).kind, ours);
    }
    let busy = failure(AiError::new(ErrorKind::RateLimited, "slow down").with_retry_after(7));
    assert_eq!(busy.retry_after_secs, Some(7));
}

#[test]
fn a_cancelled_job_ends_cancelled() {
    let scene = Scene::new(false, true);
    let ai = scene.ai(Some(Answer::Slow));
    let mut job = ai.plan(request()).unwrap().start().unwrap();
    assert!(matches!(job.poll().unwrap(), JobPoll::Working(_)));
    job.cancel().unwrap();
    job.cancel().unwrap();
    assert_eq!(wait(&mut job), JobPoll::Cancelled);
}

#[test]
fn a_whole_fill_lands_as_one_undo_step_inside_the_mask_after_consent() {
    let scene = Scene::new(true, true);
    let ai = Arc::new(scene.ai(Some(Answer::Picture(png(16, 16, [255, 0, 0])))));
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 120, "height": 80, "background": "#ffffff"})).unwrap();
    s.execute("select.rect", json!({"x": 40, "y": 20, "width": 30, "height": 30})).unwrap();
    s.set_generative_ai(Some(ai));
    assert!(s.is_enabled(FILL));
    let history = s.active().unwrap().history.past_len();
    // Nothing is sent before the person agrees.
    let e = s.execute(FILL, json!({"prompt": "a red square", "variations": 2})).unwrap_err().to_string();
    assert!(e.contains("agreement"), "{e}");
    let fills = s.execute(FILLS, json!({})).unwrap();
    assert_eq!(fills["fills"][0]["state"], "consent");
    let id = fills["fills"][0]["id"].as_u64().unwrap();
    assert_eq!(s.generative_fill_disclosure(id).unwrap().destination, "https://fake.example.com");
    let r = s.execute(AGREE, json!({"fill": id, "term": "this_time"})).unwrap();
    assert_eq!(r["variations"].as_array().unwrap().len(), 2);
    assert_eq!(s.active().unwrap().history.past_len(), history + 1, "one undo step");
    let flat = photocraft_compose::flatten(&s.active().unwrap().doc);
    let rgb = |x, y| {
        let p = flat.get(x, y);
        [0, 1, 2].map(|c| (p[c] * 255.0).round() as u8)
    };
    assert_eq!(rgb(55, 35), [255, 0, 0], "inside the selection: the model's picture");
    assert_eq!(rgb(30, 35), [255, 255, 255], "outside it: unchanged");
    assert_eq!(rgb(5, 5), [255, 255, 255]);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
}

#[test]
fn setup_lists_connections_and_models_and_choosing_is_the_suites_choice() {
    let scene = Scene::new(true, false);
    let ai = scene.ai(Some(Answer::Picture(png(4, 4, [255, 0, 0]))));
    let view = ai.setup().unwrap();
    assert_eq!(view.connections.len(), 1);
    assert_eq!(view.connections[0].label, "Fake (online)");
    assert_eq!(view.connections[0].problem, None);
    assert!(!view.connections[0].local);
    assert_eq!(view.models.len(), 1);
    assert_eq!((view.models[0].label.as_str(), view.models[0].selected), ("Fill One", false));
    ai.choose("fake", "fill-1").unwrap();
    assert!(ai.status().is_ready(), "the status isn't kept stale after a choice");
    assert!(ai.setup().unwrap().models[0].selected);
    // The launcher sees the same choice: it's the suite's default.
    let launcher = scene.launcher().view(&super::fill_role().unwrap()).unwrap();
    assert_eq!(launcher.bound, Some((ConnectionId::new("fake").unwrap(), ModelId::new("fill-1").unwrap())));
    // A model that can't do it is refused, and nothing changes.
    assert!(ai.choose("fake", "no-such-model").is_err());
    // A missing key shows on the connection.
    scene.keys.delete(&SecretRef::new("fake-key").unwrap()).unwrap();
    assert_eq!(ai.setup().unwrap().connections[0].problem, Some(ConnectionProblem::NoKey));
    // Removing it takes the choice with it.
    ai.disconnect("fake").unwrap();
    assert!(ai.setup().unwrap().connections.is_empty());
    assert!(!ai.status().is_ready());
}

#[test]
fn connecting_a_service_checks_it_and_keeps_the_key_out_of_the_settings_file() {
    let scene = Scene::new(false, false);
    let ai = scene.ai(Some(Answer::Picture(png(4, 4, [255, 0, 0]))));
    // A blank key is refused before anything is saved.
    assert!(ai.connect(ServiceKind::OpenAi, "  ").is_err());
    assert_eq!(ai.setup().unwrap().connections.len(), 1);
    ai.connect(ServiceKind::OpenAi, "sk-test-not-real\n").unwrap();
    let view = ai.setup().unwrap();
    let openai = view.connections.iter().find(|c| c.label == "OpenAI").unwrap();
    assert_eq!(openai.address.as_deref(), Some("https://api.openai.com/v1"));
    assert_eq!(openai.problem, None, "its key is saved");
    // The model the test found is offered for Generative Fill.
    assert!(view.models.iter().any(|m| m.connection == openai.id && m.label == "Fill One"));
    let settings = std::fs::read_to_string(scene.shared.path()).unwrap();
    assert!(!settings.contains("sk-test-not-real"), "the key lives in the key store only");
}

#[test]
fn every_service_setup_offers_is_a_preset_of_the_shared_layer() {
    for kind in ServiceKind::ALL {
        let preset = craft_ai_core::PRESETS.iter().find(|p| p.key == super::preset_key(kind));
        let preset = preset.unwrap_or_else(|| panic!("{kind:?} has no preset"));
        assert_eq!(preset.needs_key, kind.needs_key(), "{kind:?}");
    }
    let gemini = craft_ai_core::PRESETS.iter().find(|p| p.key == "gemini").unwrap();
    assert_eq!(gemini.endpoint, "https://generativelanguage.googleapis.com/v1beta");
}

#[test]
fn gemini_is_offered_for_generative_fill() {
    let fill = craft_ai_features::builtin().into_iter().find(|f| f.id == "generative-fill").unwrap();
    assert!(fill.cloud.iter().any(|c| c.preset == "gemini"));
    assert!(fill.cloud.iter().any(|c| c.preset == "bfl"));
}
