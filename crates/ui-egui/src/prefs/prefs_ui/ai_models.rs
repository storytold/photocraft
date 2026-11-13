//! Optional local models. Workers belong to the app runtime, never the preferences draft.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use egui::RichText;
use photocraft_ml::{MlError, Model};

use crate::{theme::Tokens, widgets};

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Idle,
    Downloading { done: u64, total: u64 },
    Installed,
}

#[derive(Clone, Debug, PartialEq)]
struct State {
    phase: Phase,
    error: Option<String>,
}

struct ModelJob {
    model: Model,
    state: Arc<Mutex<State>>,
    cancel: Arc<AtomicBool>,
    confirm_remove: bool,
}

impl ModelJob {
    fn new(model: Model) -> Self {
        Self { model, state: Arc::new(Mutex::new(State { phase: Phase::Idle, error: None })), cancel: Arc::new(AtomicBool::new(false)), confirm_remove: false }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn active(&self) -> bool {
        matches!(self.lock().phase, Phase::Downloading { .. })
    }

    /// Only local presence is probed. Opening Preferences never starts a network request.
    fn refresh(&mut self, installed: bool) {
        let mut state = self.lock();
        if !matches!(state.phase, Phase::Downloading { .. }) {
            let phase = if installed { Phase::Installed } else { Phase::Idle };
            let changed = state.phase != phase;
            state.phase = phase;
            drop(state);
            if changed {
                self.confirm_remove = false;
            }
        }
    }

    fn start_with<F>(&mut self, download: F) -> bool
    where
        F: FnOnce(Model, &dyn Fn(u64, u64), &dyn Fn() -> bool) -> Result<PathBuf, MlError> + Send + 'static,
    {
        {
            let mut state = self.lock();
            if !matches!(state.phase, Phase::Idle) {
                return false;
            }
            state.phase = Phase::Downloading { done: 0, total: self.model.size_bytes() };
            state.error = None;
            self.cancel.store(false, Ordering::Relaxed);
        }
        self.confirm_remove = false;
        let state = Arc::clone(&self.state);
        let cancel = Arc::clone(&self.cancel);
        let model = self.model;
        // Builder reports thread creation failure instead of panicking on resource exhaustion.
        if let Err(error) = std::thread::Builder::new().name("photocraft-model-download".into()).spawn(move || {
            let progress = |done, total| {
                state.lock().unwrap_or_else(PoisonError::into_inner).phase = Phase::Downloading { done, total };
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| download(model, &progress, &|| cancel.load(Ordering::Relaxed))));
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            match result {
                Ok(Ok(_)) => state.phase = Phase::Installed,
                Ok(Err(MlError::Cancelled)) => state.phase = Phase::Idle,
                Ok(Err(error)) => {
                    state.phase = Phase::Idle;
                    state.error = Some(error.to_string());
                }
                Err(_) => {
                    state.phase = Phase::Idle;
                    state.error = Some(tl!("Model download worker failed.").to_string());
                }
            }
        }) {
            let mut state = self.lock();
            state.phase = Phase::Idle;
            state.error = Some(error.to_string());
        }
        true
    }

    fn cancel(&self) {
        if self.active() {
            // Keep Downloading until the worker exits: a retry must not race the old install.
            self.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The first click arms confirmation. The second performs the local removal.
    fn remove_with(&mut self, remove: impl FnOnce(Model) -> Result<(), MlError>) {
        if !matches!(self.lock().phase, Phase::Installed) {
            return;
        }
        if !self.confirm_remove {
            self.confirm_remove = true;
            return;
        }
        self.confirm_remove = false;
        let result = remove(self.model);
        let mut state = self.lock();
        match result {
            Ok(()) => {
                state.phase = Phase::Idle;
                state.error = None;
            }
            Err(error) => state.error = Some(error.to_string()),
        }
    }
}

pub(super) struct Runtime {
    rows: Vec<ModelJob>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self { rows: Model::ALL.into_iter().map(ModelJob::new).collect() }
    }
}

impl Runtime {
    pub(super) fn repaint_while_active(&self, ctx: &egui::Context) {
        if self.rows.iter().any(ModelJob::active) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

pub(super) fn privacy(ui: &mut egui::Ui) {
    ui.label(tl!("Models are optional and run entirely on this computer. Downloading fetches the model file from Hugging Face; no images or documents are ever uploaded. Without a model, Select Subject, Remove Background and Remove Subject use the built-in (non-AI) methods."));
    ui.add_space(8.0);
}

fn powers(model: Model) -> &'static str {
    match model {
        Model::BirefnetLite => tl!("Select Subject, Remove Background"),
        Model::Lama => tl!("Remove Subject"),
    }
}

/// Megabytes with one decimal, as the size column shows them.
fn mb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_000_000.0)
}

fn progress_fraction(done: u64, total: u64) -> f32 {
    if total == 0 { 0.0 } else { (done as f64 / total as f64).clamp(0.0, 1.0) as f32 }
}

pub(super) fn show(ui: &mut egui::Ui, runtime: &mut Runtime) {
    show_with(ui, runtime, |model| photocraft_ml::model_path(model).is_some());
}

fn show_with(ui: &mut egui::Ui, runtime: &mut Runtime, installed: impl Fn(Model) -> bool) {
    let t = Tokens::get(ui.ctx());
    for row in &mut runtime.rows {
        row.refresh(installed(row.model));
        let state = row.lock().clone();
        ui.push_id(row.model, |ui| {
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tl!(row.model.label())).font(crate::theme::semibold(12.5)).color(t.text));
                if !matches!(state.phase, Phase::Downloading { .. }) {
                    ui.label(RichText::new(if state.phase == Phase::Installed { tl!("Installed") } else { tl!("Not installed") }).color(t.text_dim));
                }
            });
            ui.label(powers(row.model));
            let size = mb(row.model.size_bytes());
            ui.label(RichText::new(crate::i18n::fmt(tl!("{size} MB · {licence}"), &[("size", &size), ("licence", row.model.licence())])).color(t.text_dim));
            match state.phase {
                Phase::Idle => {
                    if widgets::secondary_button(ui, tl!("Download"), 0.0).clicked() {
                        row.start_with(photocraft_ml::download);
                    }
                }
                Phase::Downloading { done, total } => {
                    let fraction = progress_fraction(done, total);
                    let progress = crate::i18n::fmt(
                        tl!("{done} / {total} MB ({percent}%)"),
                        &[("done", &mb(done)), ("total", &mb(total)), ("percent", &format!("{:.0}", fraction * 100.0))],
                    );
                    ui.add(egui::ProgressBar::new(fraction).text(progress).fill(t.accent));
                    let cancelling = row.cancel.load(Ordering::Relaxed);
                    if ui
                        .add_enabled_ui(!cancelling, |ui| widgets::secondary_button(ui, if cancelling { tl!("Cancelling…") } else { tl!("Cancel") }, 0.0))
                        .inner
                        .clicked()
                    {
                        row.cancel();
                    }
                }
                Phase::Installed => {
                    ui.horizontal(|ui| {
                        if widgets::secondary_button(ui, if row.confirm_remove { tl!("Confirm remove") } else { tl!("Remove") }, 0.0).clicked() {
                            row.remove_with(photocraft_ml::remove);
                        }
                        if row.confirm_remove && widgets::secondary_button(ui, tl!("Cancel"), 0.0).clicked() {
                            row.confirm_remove = false;
                        }
                    });
                }
            }
            if let Some(error) = &state.error {
                ui.label(RichText::new(tl!(error.as_str())).color(t.danger));
            }
            ui.add_space(8.0);
        });
    }
    runtime.repaint_while_active(ui.ctx());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    fn wait_for(job: &ModelJob, predicate: impl Fn(&State) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !predicate(&job.lock()) {
            assert!(Instant::now() < deadline, "worker did not settle: {:?}", job.lock());
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn start_progress_double_start_and_success() {
        let mut job = ModelJob::new(Model::BirefnetLite);
        let (tx, rx) = mpsc::channel();
        let ui_thread = std::thread::current().id();
        assert!(job.start_with(move |model, progress, _| {
            assert_ne!(std::thread::current().id(), ui_thread);
            assert_eq!(model, Model::BirefnetLite);
            progress(25, 100);
            rx.recv_timeout(Duration::from_secs(10)).unwrap();
            Ok(PathBuf::from("fake-model"))
        }));
        wait_for(&job, |s| s.phase == Phase::Downloading { done: 25, total: 100 });
        assert!(!job.start_with(|_, _, _| panic!("duplicate worker")));
        job.refresh(false);
        assert!(job.active());
        tx.send(()).unwrap();
        wait_for(&job, |s| s.phase == Phase::Installed);
        assert!(!job.start_with(|_, _, _| panic!("already installed")));
    }

    #[test]
    fn cancellation_is_observed_and_retry_waits_for_worker_exit() {
        let mut job = ModelJob::new(Model::Lama);
        let (tx, rx) = mpsc::channel();
        assert!(job.start_with(move |_, _, cancel| {
            rx.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(cancel());
            Err(MlError::Cancelled)
        }));
        job.cancel();
        assert!(job.active());
        assert!(!job.start_with(|_, _, _| panic!("old worker still running")));
        tx.send(()).unwrap();
        wait_for(&job, |s| s.phase == Phase::Idle);
        assert!(job.lock().error.is_none());
        assert!(job.start_with(|_, _, cancel| {
            assert!(!cancel(), "new worker resets the cancellation flag");
            Ok(PathBuf::from("fake-model"))
        }));
        wait_for(&job, |s| s.phase == Phase::Installed);
    }

    #[test]
    fn errors_are_inline_state_and_retry_clears_them() {
        for message in ["network failure", "checksum mismatch"] {
            let mut job = ModelJob::new(Model::Lama);
            job.start_with(move |_, _, _| Err(MlError::Download(message.into())));
            wait_for(&job, |s| s.error.is_some());
            assert!(job.lock().error.as_ref().unwrap().contains(message));
            job.refresh(false);
            assert!(job.lock().error.is_some(), "presence refresh preserves the error");
            job.start_with(|_, _, _| Ok(PathBuf::from("fake-model")));
            wait_for(&job, |s| s.phase == Phase::Installed);
            assert!(job.lock().error.is_none());
        }
    }

    #[test]
    fn remove_requires_confirmation_and_reports_failure() {
        let mut job = ModelJob::new(Model::Lama);
        job.remove_with(|_| panic!("absent model"));
        job.refresh(true);
        job.remove_with(|_| panic!("first click must only ask"));
        assert!(job.confirm_remove);
        job.remove_with(|_| Err(MlError::Download("permission denied".into())));
        assert!(!job.confirm_remove);
        assert_eq!(job.lock().phase, Phase::Installed);
        assert!(job.lock().error.as_ref().unwrap().contains("permission denied"));
        job.remove_with(|_| panic!("confirmation must be requested again"));
        job.remove_with(|model| {
            assert_eq!(model, Model::Lama);
            Ok(())
        });
        assert_eq!(job.lock().phase, Phase::Idle);
        assert!(job.lock().error.is_none());
    }

    #[test]
    fn separate_models_run_concurrently_and_survive_closing_preferences() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let dialog = super::super::open_preferences(&mut app, "aiModels");
        let mut releases = Vec::new();
        for row in &mut app.prefs_rt.ai_models.rows {
            let (tx, rx) = mpsc::channel();
            releases.push(tx);
            assert!(row.start_with(move |_, _, cancel| {
                rx.recv_timeout(Duration::from_secs(10)).unwrap();
                assert!(!cancel());
                Ok(PathBuf::from("fake-model"))
            }));
        }
        app.ui.dialogs.retain(|d| d.id != dialog);
        assert!(app.prefs_rt.ai_models.rows.iter().all(ModelJob::active));
        super::super::open_preferences(&mut app, "aiModels");
        for tx in releases {
            tx.send(()).unwrap();
        }
        for row in &app.prefs_rt.ai_models.rows {
            wait_for(row, |s| s.phase == Phase::Installed);
        }
    }

    #[test]
    fn progress_handles_unknown_or_oversized_totals() {
        assert_eq!(progress_fraction(5, 0), 0.0);
        assert_eq!(progress_fraction(25, 100), 0.25);
        assert_eq!(progress_fraction(u64::MAX, 10), 1.0);
    }

    #[test]
    fn section_renders_without_downloads_or_model_files() {
        let mut runtime = Runtime::default();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            if ui.ctx().cumulative_pass_nr() == 0 {
                crate::PhotocraftApp::setup_context(ui.ctx(), Default::default());
                return;
            }
            privacy(ui);
            show_with(ui, &mut runtime, |_| false);
        });
        harness.run_steps(2);
        use egui_kittest::kittest::Queryable;
        assert_eq!(harness.get_all_by_label("Download").count(), Model::ALL.len());
        assert_eq!(harness.get_all_by_label("Not installed").count(), Model::ALL.len());
        for model in Model::ALL {
            harness.get_by_label(model.label());
        }
    }

    #[test]
    fn dynamic_labels_are_translated_in_complete_languages() {
        for lang in crate::i18n::Lang::all().filter(|lang| lang.complete_menus()) {
            for label in std::iter::once("AI Models").chain(Model::ALL.into_iter().map(Model::label)) {
                assert!(crate::i18n::has(lang, label), "{}: missing {label}", lang.code());
            }
        }
    }

    #[test]
    fn progress_and_failure_render_inline_without_network() {
        let mut runtime = Runtime::default();
        let first = runtime.rows.first_mut().unwrap();
        first.lock().phase = Phase::Downloading { done: 52_000_000, total: 208_000_000 };
        let second = runtime.rows.last_mut().unwrap();
        second.lock().error = Some("checksum mismatch".into());
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            if ui.ctx().cumulative_pass_nr() == 0 {
                crate::PhotocraftApp::setup_context(ui.ctx(), Default::default());
                return;
            }
            show_with(ui, &mut runtime, |_| false);
        });
        harness.run_steps(2);
        use egui_kittest::kittest::Queryable;
        harness.get_by_label("52.0 / 208.0 MB (25%)");
        harness.get_by_label("Cancel");
        harness.get_by_label("checksum mismatch");
    }
}
