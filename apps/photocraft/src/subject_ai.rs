//! Select Subject and Remove Background over the Craft suite's shared AI layer
//! ([craft-ai-integrations](https://github.com/storytold/craft-ai-integrations)): the desktop
//! app's [`SubjectModel`]. The catalogue's `remove-background` tool picks the model, and the
//! layer's built-in Burn engine runs it on this computer. Connections and model folders are the
//! suite's, shared with the launcher and the other Craft apps.
//!
//! Only a model on this computer is used: a picture is never sent anywhere from here.

use std::sync::Arc;
use std::time::Duration;

use craft_ai_client::{AiClient, FolderDesk, ModelDownloadConfig, ModelDownloader, RoleStatus, SetupPhase, SetupSession, ToolDesk};
use craft_ai_core::{AiError, AppId, ConsentLedger, ErrorKind, Event, InputRole, JobState, Locality, Payload, Request, Role};
use craft_ai_features::{Feature, NativeRoute, SetupRoute, agreement_for};
use craft_ai_native::secrets::{EnvStore, KeychainStore, LayeredStore, SecretStore};
use craft_ai_native::shared::SharedStore;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
use photocraft_engine::subject_model::{ModelError, ModelOffer, ModelStatus, SharedSubjectModel, SubjectImage, SubjectModel};

/// The app id the launcher knows PhotoCraft by.
const APP: &str = "photocraft";
/// The catalogue's tool: its model answers `image.matte` from the picture alone.
const TOOL: &str = "remove-background";
/// The built-in engine PhotoCraft ships.
const ENGINE: &str = "burn";
/// How often a running job or download is looked at.
const POLL: Duration = Duration::from_millis(50);
/// The part of an install's progress bar the download takes; the check takes the rest.
const DOWNLOAD_SHARE: f32 = 0.95;
const DOWNLOADING: &str = "Downloading subject model";

/// The connection the shared layer saves for this build's model (`ToolDesk::enable_native`).
fn native_connection() -> String {
    format!("local-{TOOL}-{ENGINE}")
}

/// The app's subject model, or `None` if the shared layer can't be reached: Select Subject and
/// Remove Background then keep their built-in method.
pub fn shared() -> Option<SharedSubjectModel> {
    match CraftSubject::standard() {
        Ok(model) => Some(Arc::new(model)),
        Err(e) => {
            eprintln!("subject model unavailable: {}", e.message);
            None
        }
    }
}

/// The tool from the shared catalogue.
fn tool() -> Result<(Feature, Role), AiError> {
    let feature = craft_ai_features::builtin().into_iter().find(|f| f.id == TOOL);
    let found = feature.and_then(|f| f.role().map(|role| (f, role)));
    found.ok_or_else(|| AiError::new(ErrorKind::Internal, "The shared AI catalogue has no Remove background tool."))
}

/// PhotoCraft's id in the shared settings.
fn app_id() -> Result<AppId, AiError> {
    AppId::new(APP).map_err(|e| AiError::new(ErrorKind::Internal, e.to_string()))
}

/// The shared layer through PhotoCraft's client.
pub struct CraftSubject {
    client: AiClient,
    shared: SharedStore,
    secrets: Arc<dyn SecretStore>,
    feature: Feature,
    role: Role,
}

impl CraftSubject {
    /// The usual setup: the shared settings folder (or `CRAFT_AI_DIR`) and the standard adapters.
    pub fn standard() -> Result<Self, AiError> {
        let (feature, role) = tool()?;
        let client = AiClient::standard(app_id()?)?;
        let shared = SharedStore::at_default_location()?;
        let secrets: Arc<dyn SecretStore> = Arc::new(LayeredStore::new(vec![Box::new(EnvStore::new()), Box::new(KeychainStore::new())]));
        Ok(Self { client, shared, secrets, feature, role })
    }

    /// The route this build runs.
    fn route(&self) -> Option<&NativeRoute> {
        self.feature.engine_recipe(ENGINE)?.native.iter().find(|n| n.provider == ENGINE)
    }

    /// What a person is asked to agree to: the model, its size, its folder and the terms of the
    /// built-in route and its files. `None` when this build has no route for the tool.
    fn offer(&self) -> Option<ModelOffer> {
        let recipe = self.feature.engine_recipe(ENGINE)?;
        let route = self.route()?;
        let download_bytes = route.models.iter().fold(0u64, |sum, file| sum.saturating_add(file.size_bytes));
        let licences: Vec<String> = route.models.iter().map(|file| format!("{}: {}", file.filename, file.licence.name)).collect();
        let terms = format!("{}\n\n{}", agreement_for(SetupRoute::BuiltIn).full_text, licences.join("\n"));
        Some(ModelOffer { model: recipe.label.clone(), download_bytes, folder: self.download_folder(route), terms })
    }

    /// Where a download would go: the suite's model folder, or the folder it would make for a
    /// person who has none yet. Reading this creates nothing.
    fn download_folder(&self, route: &NativeRoute) -> String {
        let chosen = FolderDesk::new(self.shared.clone()).native_download_dir(&self.feature, ENGINE).ok();
        let folder = chosen.or_else(|| Some(craft_ai_native::paths::suggested_models_dir()?.join(&route.models.first()?.folder)));
        folder.map_or_else(String::new, |path| path.display().to_string())
    }

    /// Whether the tool has a model chosen that is not this build's: PhotoCraft's own choice, else
    /// the launcher's. A download would leave that choice in place.
    fn keeps_another_choice(&self) -> bool {
        let Ok(store) = self.shared.load() else { return false };
        let Ok(app) = app_id() else { return false };
        let mine = store.apps.get(&app).and_then(|settings| settings.overrides.iter().find(|b| b.role == self.role.id));
        let chosen = mine.or_else(|| store.defaults.iter().find(|b| b.role == self.role.id));
        chosen.is_some_and(|b| b.connection.as_str() != native_connection())
    }

    /// Makes `checked` the suite's model for this tool, as the launcher would.
    fn adopt(&self, checked: &craft_ai_client::CheckedNative) -> Result<(), AiError> {
        let launcher = AiClient::new(ToolDesk::app()?, self.shared.clone(), Arc::clone(&self.secrets));
        ToolDesk::new(launcher, self.shared.clone(), None).adopt(checked).map(|_| ())
    }
}

/// The shared layer's error for the engine: a cancel stays a cancel, anything else keeps its words.
fn model_error(e: AiError) -> ModelError {
    if e.kind == ErrorKind::Cancelled { ModelError::Cancelled } else { ModelError::Failed(e.message) }
}

/// A failure with `message` as its words.
fn failed(message: impl ToString) -> ModelError {
    ModelError::Failed(message.to_string())
}

/// The model's input: `image` as a file the engine reads, within `limit`, the shared layer's limit
/// for one payload (the engine reads only inline pictures). PNG, or a good JPEG when a PNG would
/// not fit: the engine reads the whole picture and only a mask comes back. A picture too big even
/// as a JPEG goes at half size, as often as it takes, and its mask is scaled back up
/// ([`fit_mask`]). Returns the payload and the size of the picture in it.
fn encode(image: &SubjectImage, limit: u64) -> Result<(Payload, u32, u32), ModelError> {
    let mut smaller: Option<SubjectImage> = None;
    loop {
        let current = smaller.as_ref().unwrap_or(image);
        if let Some(payload) = encode_within(current, limit)? {
            return Ok((payload, current.width, current.height));
        }
        if current.width <= 1 && current.height <= 1 {
            return Err(failed("The picture is too big to hand to the model."));
        }
        let next = half(current);
        smaller = Some(next);
    }
}

/// `image` as PNG, else as a good JPEG, whichever fits `limit` first; `None` when neither does.
fn encode_within(image: &SubjectImage, limit: u64) -> Result<Option<Payload>, ModelError> {
    let img = Image::from_u8(image.width, image.height, ChannelLayout::Rgb, image.rgb.clone()).map_err(failed)?;
    let png = photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).map_err(failed)?;
    if png.len() as u64 <= limit {
        return Payload::inline("image/png", png).map(Some).map_err(failed);
    }
    let options = EncodeOptions { jpeg_quality: 95, jpeg_chroma_subsampling: false, ..EncodeOptions::default() };
    let jpeg = photocraft_codecs::encode(&img, Format::Jpeg, &options).map_err(failed)?;
    if jpeg.len() as u64 <= limit {
        return Payload::inline("image/jpeg", jpeg).map(Some).map_err(failed);
    }
    Ok(None)
}

/// `image` at half size (rounded up), each pixel the mean of the up to four it covers.
fn half(image: &SubjectImage) -> SubjectImage {
    let (w, h) = (image.width as usize, image.height as usize);
    let (half_w, half_h) = (w.div_ceil(2), h.div_ceil(2));
    let mut rgb = Vec::with_capacity(half_w.saturating_mul(half_h).saturating_mul(3));
    for y in 0..half_h {
        for x in 0..half_w {
            let mut sum = [0u32; 3];
            let mut n = 0u32;
            for (sx, sy) in [(2 * x, 2 * y), (2 * x + 1, 2 * y), (2 * x, 2 * y + 1), (2 * x + 1, 2 * y + 1)] {
                let start = (sy * w + sx) * 3;
                if sx < w
                    && sy < h
                    && let Some(px) = image.rgb.get(start..start + 3)
                {
                    for (total, value) in sum.iter_mut().zip(px) {
                        *total += u32::from(*value);
                    }
                    n += 1;
                }
            }
            let n = n.max(1);
            rgb.extend(sum.map(|total| ((total + n / 2) / n) as u8));
        }
    }
    SubjectImage { width: half_w as u32, height: half_h as u32, rgb }
}

/// `mask`, `from` = (width, height), scaled to `to` with bilinear sampling; as it is when the sizes
/// match.
fn fit_mask(mask: Vec<u8>, from: (u32, u32), to: (u32, u32)) -> Vec<u8> {
    if from == to {
        return mask;
    }
    let (from_w, from_h) = (from.0 as usize, from.1 as usize);
    let (to_w, to_h) = (to.0 as usize, to.1 as usize);
    if from_w == 0 || from_h == 0 {
        return vec![0; to_w.saturating_mul(to_h)];
    }
    let at = |x: usize, y: usize| f32::from(mask.get(y.min(from_h - 1) * from_w + x.min(from_w - 1)).copied().unwrap_or(0));
    // The source position of a target pixel's centre, split into a pixel and the way to the next.
    let source = |t: usize, from: usize, to: usize| {
        let s = ((t as f32 + 0.5) * from as f32 / to as f32 - 0.5).max(0.0);
        (s.floor() as usize, s - s.floor())
    };
    let mut out = Vec::with_capacity(to_w.saturating_mul(to_h));
    for y in 0..to_h {
        let (y0, fy) = source(y, from_h, to_h);
        for x in 0..to_w {
            let (x0, fx) = source(x, from_w, to_w);
            let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
            let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
            out.push((top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8);
        }
    }
    out
}

/// The mask in `png`, one byte per pixel, checked against the size of the picture it answers.
fn decode_mask(png: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ModelError> {
    let mask = photocraft_codecs::decode(png).map_err(failed)?;
    if mask.width() != width || mask.height() != height {
        return Err(failed(format!("The model answered {} × {} for a {} × {} picture.", mask.width(), mask.height(), width, height)));
    }
    if mask.layout() != ChannelLayout::Gray || mask.sample_type() != SampleType::U8 {
        return Err(failed("The model's mask is not an 8-bit grayscale picture."));
    }
    Ok(mask.data().to_vec())
}

impl SubjectModel for CraftSubject {
    /// Ready only for a model on this computer; otherwise the download when it would help.
    fn status(&self) -> ModelStatus {
        match self.client.status(&self.role) {
            RoleStatus::Ready(ready) if ready.locality == Locality::Local => ModelStatus::Ready { model: ready.model_label },
            RoleStatus::Ready(ready) => {
                ModelStatus::Unavailable { reason: format!("{} runs online, and PhotoCraft only uses a model on this computer here.", ready.model_label) }
            }
            // A download only helps when it becomes the tool's model: the shared layer keeps a
            // choice a person made, even one that does not work right now.
            RoleStatus::Unavailable { reason } if self.keeps_another_choice() => {
                ModelStatus::Unavailable { reason: format!("{reason} Choose another model for Remove background in the Craft launcher.") }
            }
            RoleStatus::Unavailable { reason } => self.offer().map_or(ModelStatus::Unavailable { reason }, ModelStatus::Offer),
        }
    }

    /// Runs the chosen model on `image` through the shared client, refusing anything that would
    /// send it off the computer.
    fn matte(&self, image: &SubjectImage, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>, ModelError> {
        let (payload, sent_width, sent_height) = encode(image, craft_ai_core::Limits::default().max_inline_bytes)?;
        let request = Request::new(self.role.capability.clone()).with_input(InputRole::Source, payload);
        let plan = self.client.plan(&self.role, request).map_err(model_error)?;
        // Nothing is agreed to here, so anything that would leave the computer is refused by
        // `start` too; this says so in words.
        let nothing_agreed = ConsentLedger::default();
        if plan.needs_consent(&nothing_agreed) {
            return Err(failed("The chosen model runs online, and PhotoCraft only uses a model on this computer here."));
        }
        let job = self.client.start(&plan, &nothing_agreed).map_err(model_error)?;
        let mut answer = None;
        let mut stopping = false;
        loop {
            let snapshot = job.poll().map_err(model_error)?;
            for event in snapshot.events {
                if let Event::Output(output) = event
                    && let Payload::Inline { data, .. } = output.payload
                {
                    answer = Some(data);
                }
            }
            match snapshot.state {
                JobState::Succeeded => break,
                JobState::Failed { error } => return Err(model_error(error)),
                JobState::Cancelled => return Err(ModelError::Cancelled),
                _ => {}
            }
            if !stopping && cancelled() {
                stopping = true;
                // An engine that can't stop now runs to its end, and the answer is dropped below.
                let _ = job.cancel();
            }
            std::thread::sleep(POLL);
        }
        if stopping {
            return Err(ModelError::Cancelled);
        }
        let mask = decode_mask(&answer.ok_or_else(|| failed("The model finished without a mask."))?, sent_width, sent_height)?;
        Ok(fit_mask(mask, (sent_width, sent_height), (image.width, image.height)))
    }

    /// Downloads and checks the built-in route's files in the suite's model folder, then makes
    /// them the tool's model.
    fn install(&self, progress: &dyn Fn(f32, &str), cancelled: &dyn Fn() -> bool) -> Result<(), ModelError> {
        progress(0.0, DOWNLOADING);
        let folders = FolderDesk::new(self.shared.clone());
        folders.ensure_default().map_err(model_error)?;
        let plan = folders.prepare_native_setup(&self.feature, ENGINE, &self.client).map_err(model_error)?;
        let mut session = SetupSession::new(plan);
        session.accept_licences(true);
        let request = session.start().ok_or_else(|| failed("The model can't be set up right now."))?;
        let downloader = ModelDownloader::new(ModelDownloadConfig::default());
        let (tx, rx) = std::sync::mpsc::channel();
        let checked = std::thread::scope(|scope| {
            let request = &request;
            let worker = scope.spawn(move || {
                request.run(&downloader, None, &mut |_, step| {
                    let _ = tx.send(step);
                })
            });
            while !worker.is_finished() {
                for step in rx.try_iter() {
                    let done = step.done as f32 / step.total.max(1) as f32;
                    match step.phase {
                        SetupPhase::Downloading => progress(done * DOWNLOAD_SHARE, DOWNLOADING),
                        SetupPhase::Verifying => progress(DOWNLOAD_SHARE + done * (1.0 - DOWNLOAD_SHARE), "Checking subject model"),
                    }
                }
                if cancelled() {
                    request.cancel();
                }
                std::thread::sleep(POLL);
            }
            worker.join().unwrap_or_else(|_| Err(AiError::new(ErrorKind::Internal, "The model download stopped unexpectedly.")))
        })
        .map_err(model_error)?;
        self.adopt(&checked).map_err(model_error)
    }
}

#[cfg(test)]
mod tests;
