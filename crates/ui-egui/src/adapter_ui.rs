//! Filter › Plug-ins › Compatibility Adapters…: trusted native package management.
//!
//! This is not the sandboxed `.wasm` plug-in installer. All package operations go through
//! [`PhotocraftApp::run`], including probes and model preparation; the shell never reads files,
//! launches a worker, downloads a model or starts its own command thread. Engine commands may
//! return a job, which this view observes without consuming the shell's job events.

use std::collections::BTreeSet;

use egui::{RichText, Ui};
use photocraft_engine::jobs::{JobId, JobInfo};
use photocraft_plugins::adapter::{Descriptor, DeviceKind};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::menus::MenuItem;
use crate::theme::Tokens;
use crate::widgets;

pub const MENU_ID: &str = "plugin.adapter.manage";
pub const MENU_LABEL: &str = "Compatibility Adapters…";
pub const MENU_PATH: &[&str] = &["Filter", "Plug-ins"];

const NATIVE_WARNING: &str = "Native adapters run programs with the same access as your user account. They are not sandboxed. Install and enable only packages and publishers you trust.";
const DEVICE_WARNING: &str = "Device and backend details, including managed OS NPU execution, are adapter-reported assertions, not hardware attestation.";
const DOWNLOAD_WARNING: &str = "Preparing a system AI model may download components. Checking availability does not grant download consent.";

#[derive(Clone, Debug, Deserialize)]
struct Inspection {
    descriptor: Descriptor,
    launch: String,
    warning: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Availability {
    NotChecked,
    Ready,
    NeedsModel,
    Unavailable,
    Error,
}

#[derive(Clone, Debug, Deserialize)]
struct PackageStatus {
    state: Availability,
    message: String,
}

#[derive(Clone, Debug, Deserialize)]
struct Package {
    id: String,
    name: String,
    version: String,
    enabled: bool,
    descriptor: Descriptor,
    launch: String,
    status: PackageStatus,
}

#[derive(Debug, Deserialize)]
struct Packages {
    available: bool,
    packages: Vec<Package>,
    warnings: Vec<String>,
}

#[derive(Clone, Debug)]
enum Action {
    Refresh,
    Inspect,
    Install,
    Enable(String, bool),
    Check(String),
    Prepare(String),
    Remove(String),
}

impl Action {
    fn command(&self) -> &'static str {
        match self {
            Self::Refresh => "plugin.adapter.packages",
            Self::Inspect => "plugin.adapter.inspectPackage",
            Self::Install => "plugin.adapter.install",
            Self::Enable(..) => "plugin.adapter.enable",
            Self::Check(..) => "plugin.adapter.status",
            Self::Prepare(..) => "plugin.adapter.prepare",
            Self::Remove(..) => "plugin.adapter.remove",
        }
    }
}

#[derive(Debug)]
struct Pending {
    job: JobId,
    action: Action,
}

/// Transient view state. Trust and model-download consent are deliberately not persisted.
#[derive(Debug, Default)]
pub struct State {
    pub open: bool,
    install_path: String,
    inspected_path: String,
    inspected: Option<Inspection>,
    trust: bool,
    available: Option<bool>,
    packages: Vec<Package>,
    warnings: Vec<String>,
    status: String,
    error: Option<String>,
    refresh: bool,
    pending: Option<Pending>,
    prepare_consent: BTreeSet<String>,
    remove_confirmation: Option<String>,
}

/// Keep the manager available without a document, also on WASM where it explains the limitation.
pub fn is_enabled(id: &str) -> Option<bool> {
    (id == MENU_ID).then_some(true)
}

/// Add the manager without modifying or reusing `plugin.install` (the `.wasm` installer).
/// Existing engine/shell menu registration may already supply this row; never duplicate it.
pub fn insert_menu_items(items: &mut Vec<MenuItem>) {
    if items.iter().any(|item| item.id == MENU_ID) {
        return;
    }
    let at = items.iter().position(|item| item.id == "plugin.install").unwrap_or(items.len());
    items.insert(
        at,
        MenuItem {
            id: MENU_ID.into(),
            label: MENU_LABEL.into(),
            path: MENU_PATH.iter().map(|part| (*part).into()).collect(),
            shortcut: None,
            enabled: true,
            checked: None,
            color: None,
        },
    );
}

/// Pure window command; no package is inspected, enabled, probed or prepared by opening it.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != MENU_ID {
        return None;
    }
    if !params.as_object().is_some_and(|p| p.is_empty()) {
        return Some(Err("plugin.adapter.manage takes an empty parameter object".into()));
    }
    let state = &mut app.adapter_ui;
    state.open = true;
    state.refresh = true;
    state.trust = false;
    state.prepare_consent.clear();
    state.remove_confirmation = None;
    Some(Ok(json!({"dialog": MENU_ID})))
}

impl State {
    fn invalidate_inspection(&mut self) {
        self.inspected = None;
        self.inspected_path.clear();
        self.trust = false;
    }

    fn can_install(&self) -> bool {
        self.available == Some(true)
            && self.pending.is_none()
            && self.trust
            && self.inspected.is_some()
            && !self.install_path.trim().is_empty()
            && self.inspected_path == self.install_path.trim()
    }

    /// Build only the documented engine params, checking consent again at dispatch time.
    fn params(&self, action: &Action) -> Result<Value, String> {
        match action {
            Action::Refresh => Ok(json!({})),
            Action::Inspect => {
                let path = self.install_path.trim();
                if path.is_empty() {
                    return Err("Choose an adapter package manifest to inspect.".into());
                }
                Ok(json!({"path": path}))
            }
            Action::Install => {
                if !self.can_install() {
                    return Err("Inspect this package and explicitly confirm that you trust it before installing.".into());
                }
                Ok(json!({"path": self.install_path.trim(), "trust": true}))
            }
            Action::Enable(provider, enabled) => Ok(json!({"provider": provider, "enabled": enabled})),
            Action::Check(provider) | Action::Remove(provider) => Ok(json!({"provider": provider})),
            Action::Prepare(provider) => {
                if !self.prepare_consent.contains(provider) {
                    return Err("Explicit consent is required before preparing or downloading a model.".into());
                }
                Ok(json!({"provider": provider, "consent": true}))
            }
        }
    }

    fn apply_packages(&mut self, value: Value) -> Result<(), String> {
        let result: Packages = serde_json::from_value(value).map_err(|e| format!("Invalid adapter package response: {e}"))?;
        self.available = Some(result.available);
        self.packages = result.packages;
        self.warnings = result.warnings;
        self.prepare_consent.retain(|id| self.packages.iter().any(|p| &p.id == id));
        Ok(())
    }

    fn finish(&mut self, action: Action, value: Value) -> Result<(), String> {
        match action {
            Action::Refresh => self.apply_packages(value)?,
            Action::Inspect => {
                let inspected: Inspection = serde_json::from_value(value).map_err(|e| format!("Invalid adapter inspection response: {e}"))?;
                self.inspected_path = self.install_path.trim().to_string();
                self.inspected = Some(inspected);
                self.trust = false;
                self.status = "Package inspected. Review its details and trust warning before installing.".into();
            }
            Action::Install => {
                self.trust = false;
                self.apply_packages(value)?;
                self.status = "Package installed disabled. Enable it, then check availability.".into();
            }
            Action::Enable(_, enabled) => {
                self.apply_packages(value)?;
                self.status = if enabled { "Adapter enabled. Check availability before using Select Subject." } else { "Adapter disabled." }.into();
            }
            Action::Remove(_) => {
                self.remove_confirmation = None;
                self.apply_packages(value)?;
                self.status = "Adapter package removed. OS-managed models are not uninstalled.".into();
            }
            Action::Check(provider) | Action::Prepare(provider) => {
                let status: PackageStatus = serde_json::from_value(value.get("status").cloned().unwrap_or(Value::Null))
                    .map_err(|e| format!("Invalid adapter availability response: {e}"))?;
                self.status = status.message.clone();
                if let Some(package) = self.packages.iter_mut().find(|p| p.id == provider) {
                    package.status = status;
                }
                // Authoritative metadata refresh only; this does not probe or download.
                self.refresh = true;
            }
        }
        Ok(())
    }

    fn run(&mut self, app: &mut PhotocraftApp, action: Action) -> Result<(), String> {
        if self.pending.is_some() {
            return Err("Wait for the current adapter operation to finish.".into());
        }
        let params = self.params(&action)?;
        if matches!(action, Action::Inspect) {
            self.invalidate_inspection();
        }
        if let Action::Prepare(provider) = &action {
            // Each attempt needs a fresh affirmative download consent, including a failed attempt.
            self.prepare_consent.remove(provider);
        }
        self.error = None;
        let value = app.run(action.command(), params)?;
        if value.get("pending").and_then(Value::as_bool) == Some(true) {
            let job = value.get("job").and_then(Value::as_u64).ok_or("Adapter command returned a pending job without an id.")?;
            self.status = "Adapter operation running…".into();
            self.pending = Some(Pending { job: JobId(job), action });
            Ok(())
        } else {
            self.finish(action, value)
        }
    }

    fn observe_job(&mut self, jobs: &[JobInfo]) -> Result<(), String> {
        let Some(pending) = self.pending.as_ref() else { return Ok(()) };
        let Some(job) = jobs.iter().find(|job| job.id == pending.job) else {
            self.pending = None;
            self.refresh = true;
            return Err("The adapter job is no longer available. Refresh the package list to see its current state.".into());
        };
        if job.state == "running" {
            return Ok(());
        }
        let Some(pending) = self.pending.take() else { return Ok(()) };
        match job.state {
            "done" => self.finish(pending.action, job.result.clone().unwrap_or(Value::Null)),
            "cancelled" => Err("Adapter operation cancelled. No new download consent is retained.".into()),
            _ => Err(job.error.clone().unwrap_or_else(|| "Adapter operation failed.".into())),
        }
    }
}

/// Draw every frame, including while closed so a pending engine job is still observed.
/// Taking the slot lets widget actions use `app.run` without holding a mutable field borrow.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let mut state = std::mem::take(&mut app.adapter_ui);
    if state.pending.is_some()
        && let Err(error) = state.observe_job(&app.session.jobs_with_recent())
    {
        state.error = Some(error);
    }
    if state.open && !cfg!(target_arch = "wasm32") && state.refresh && state.pending.is_none() {
        state.refresh = false; // A failed refresh is not retried every frame.
        if let Err(error) = state.run(app, Action::Refresh) {
            state.error = Some(error);
        }
    }
    if state.open {
        let mut open = true;
        let mut action = None;
        let mut browse = false;
        egui::Window::new(MENU_LABEL).id(egui::Id::new(MENU_ID)).open(&mut open).default_width(660.0).min_width(300.0).vscroll(true).show(ctx, |ui| {
            if cfg!(target_arch = "wasm32") {
                ui.label(
                    "Native compatibility adapters are unsupported in the browser. Use the native PhotoCraft application to install and enable an adapter.",
                );
                ui.label("Install Plug-in… remains the separate sandboxed WebAssembly plug-in installer.");
                return;
            }
            body(&mut state, ui, app.services.file_dialog.is_some() && !app.file_dialog_open(), &mut action, &mut browse);
        });
        state.open = open;
        if !open {
            state.trust = false;
            state.prepare_consent.clear();
        }
        if let Some(action) = action
            && let Err(error) = state.run(app, action)
        {
            state.error = Some(error);
        }
        if browse && let Err(error) = browse_package(app) {
            state.error = Some(error);
        }
    }
    app.adapter_ui = state;
}

fn body(state: &mut State, ui: &mut Ui, can_browse: bool, action: &mut Option<Action>, browse: &mut bool) {
    let t = Tokens::get(ui.ctx());
    ui.label("Install → Enable → Check availability → Select › Subject");
    ui.label(RichText::new(NATIVE_WARNING).color(t.warning));
    ui.label(RichText::new(DEVICE_WARNING).color(t.text_dim));
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(state.pending.is_none(), |ui| {
            if widgets::secondary_button(ui, "Refresh packages", 0.0).clicked() {
                *action = Some(Action::Refresh);
            }
        });
        if state.pending.is_some() {
            ui.spinner();
            ui.label("Adapter operation running…");
        }
    });
    if let Some(error) = &state.error {
        ui.label(RichText::new(error).color(t.danger));
    }
    if !state.status.is_empty() {
        ui.label(&state.status);
    }
    for warning in &state.warnings {
        ui.label(RichText::new(warning).color(t.warning));
    }
    if state.available != Some(true) {
        ui.label("Native adapter package storage is unavailable in this application. No adapter programs have been started.");
        return;
    }
    widgets::hairline(ui);
    widgets::section_label(ui, "Install native adapter package");
    ui.label("Choose the package manifest (.json), not a .wasm filter or an arbitrary executable.");
    ui.add_enabled_ui(state.pending.is_none(), |ui| {
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 92.0).max(80.0);
            if ui.add_sized([width, 24.0], egui::TextEdit::singleline(&mut state.install_path).hint_text("Adapter package manifest path")).changed() {
                state.invalidate_inspection();
            }
            ui.add_enabled_ui(can_browse, |ui| {
                *browse = widgets::secondary_button(ui, "Browse…", 80.0).clicked();
            });
        });
        if widgets::secondary_button(ui, "Inspect", 80.0).clicked() {
            *action = Some(Action::Inspect);
        }
        if let Some(inspected) = &state.inspected {
            descriptor(ui, &inspected.descriptor, &inspected.launch);
            if !inspected.warning.is_empty() {
                ui.label(RichText::new(&inspected.warning).color(t.warning));
            }
            widgets::checkbox(ui, &mut state.trust, "I trust this package and understand it is not sandboxed");
        }
        ui.add_enabled_ui(state.can_install(), |ui| {
            if widgets::primary_button(ui, "Install disabled", 120.0).clicked() {
                *action = Some(Action::Install);
            }
        });
    });
    widgets::hairline(ui);
    widgets::section_label(ui, "Installed adapters");
    if state.packages.is_empty() {
        ui.label("No native adapter packages are installed.");
    }
    for package in &state.packages {
        ui.push_id(&package.id, |ui| {
            egui::Frame::new().fill(t.card).stroke(egui::Stroke::new(1.0, t.card_border)).corner_radius(t.radius).inner_margin(10.0).show(ui, |ui| {
                ui.label(RichText::new(format!("{} {}", package.name, package.version)).font(crate::theme::semibold(13.0)));
                let color = match package.status.state {
                    Availability::Error => t.danger,
                    Availability::NeedsModel | Availability::Unavailable => t.warning,
                    _ => t.text_dim,
                };
                ui.label(RichText::new(format!("{}: {}", status_label(package.status.state), package.status.message)).color(color));
                egui::CollapsingHeader::new("Package details").show(ui, |ui| descriptor(ui, &package.descriptor, &package.launch));
                ui.add_enabled_ui(state.pending.is_none(), |ui| {
                    let mut enabled = package.enabled;
                    if widgets::checkbox(ui, &mut enabled, "Enabled").changed() {
                        *action = Some(Action::Enable(package.id.clone(), enabled));
                    }
                    ui.horizontal_wrapped(|ui| {
                        if widgets::secondary_button(ui, "Check availability", 0.0).clicked() {
                            *action = Some(Action::Check(package.id.clone()));
                        }
                        if widgets::secondary_button(ui, "Remove…", 0.0).clicked() {
                            state.remove_confirmation = Some(package.id.clone());
                        }
                    });
                    ui.label(RichText::new(DOWNLOAD_WARNING).color(t.text_dim));
                    let mut consent = state.prepare_consent.contains(&package.id);
                    if widgets::checkbox(ui, &mut consent, "I consent to preparing this model, including possible system downloads").changed() {
                        if consent {
                            state.prepare_consent.insert(package.id.clone());
                        } else {
                            state.prepare_consent.remove(&package.id);
                        }
                    }
                    ui.add_enabled_ui(consent, |ui| {
                        if widgets::secondary_button(ui, "Prepare model", 0.0).clicked() {
                            *action = Some(Action::Prepare(package.id.clone()));
                        }
                    });
                    if state.remove_confirmation.as_deref() == Some(package.id.as_str()) {
                        ui.label(
                            RichText::new("Remove this adapter package? This disables it and removes its managed package files, not OS-managed models.")
                                .color(t.warning),
                        );
                        ui.horizontal_wrapped(|ui| {
                            if widgets::secondary_button(ui, "Remove package", 0.0).clicked() {
                                *action = Some(Action::Remove(package.id.clone()));
                            }
                            if widgets::secondary_button(ui, "Cancel", 0.0).clicked() {
                                state.remove_confirmation = None;
                            }
                        });
                    }
                });
            });
        });
        ui.add_space(6.0);
    }
    ui.label("When an enabled adapter reports Ready, open an image and use Select › Subject. If it needs a model, explicitly consent above and choose Prepare model first.");
}

fn status_label(status: Availability) -> &'static str {
    match status {
        Availability::NotChecked => "Not checked",
        Availability::Ready => "Ready",
        Availability::NeedsModel => "Needs model",
        Availability::Unavailable => "Unavailable",
        Availability::Error => "Error",
    }
}

fn descriptor(ui: &mut Ui, descriptor: &Descriptor, launch: &str) {
    ui.label(format!("{} {} ({})", descriptor.name, descriptor.version, descriptor.id));
    ui.label(format!("Launch: {launch}"));
    ui.label(format!("Operations: {}", descriptor.operations.iter().map(|o| o.id.as_str()).collect::<Vec<_>>().join(", ")));
    for device in &descriptor.devices {
        let kind = match device.kind {
            DeviceKind::Cpu => "CPU",
            DeviceKind::Gpu => "GPU",
            DeviceKind::Npu => "NPU",
        };
        ui.label(format!("Reported device: {} ({kind}, {}, {})", device.name, device.id, device.backend));
    }
}

fn browse_package(app: &mut PhotocraftApp) -> Result<Value, String> {
    use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
    app.ask_file(FileDialogRequest::Open { multiple: false, initial_dir: None, extensions: Some(vec!["json".into()]) }, |app, answer| {
        let FileDialogAnswer::Paths(paths) = answer else { return Err("Choose a native adapter package manifest by its local path.".into()) };
        let path = paths.into_iter().next().ok_or("No adapter package manifest was selected.")?;
        app.adapter_ui.install_path = path;
        app.adapter_ui.invalidate_inspection();
        Ok(Value::Null)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inspection() -> Inspection {
        serde_json::from_value(json!({
            "descriptor": {"api_version":1,"id":"example.adapter","name":"Example Adapter","version":"1","operations":[],"devices":[]},
            "launch":"example.exe","warning":"not sandboxed"
        }))
        .unwrap()
    }

    #[test]
    fn installation_requires_inspection_matching_path_and_fresh_trust() {
        let mut state = State { available: Some(true), install_path: " package.json ".into(), ..Default::default() };
        assert!(!state.trust);
        assert!(state.params(&Action::Install).is_err());
        state.inspected = Some(inspection());
        state.inspected_path = "package.json".into();
        assert!(state.params(&Action::Install).is_err());
        state.trust = true;
        assert_eq!(state.params(&Action::Install).unwrap(), json!({"path":"package.json","trust":true}));
        state.install_path = "different.json".into();
        assert!(!state.can_install());
        state.invalidate_inspection();
        assert!(!state.trust && state.inspected.is_none());
    }

    #[test]
    fn prepare_requires_separate_provider_specific_consent() {
        let mut state = State::default();
        let action = Action::Prepare("example.adapter".into());
        state.trust = true;
        assert!(state.params(&action).is_err(), "install trust is not download consent");
        state.prepare_consent.insert("different.adapter".into());
        assert!(state.params(&action).is_err());
        state.prepare_consent.insert("example.adapter".into());
        assert_eq!(state.params(&action).unwrap(), json!({"provider":"example.adapter","consent":true}));
        assert_eq!(state.params(&Action::Check("example.adapter".into())).unwrap(), json!({"provider":"example.adapter"}));
    }

    #[test]
    fn package_list_and_invalid_responses_do_not_probe_or_invent_ready() {
        let mut state = State::default();
        state.apply_packages(json!({"available":true,"packages":[],"warnings":["example warning"]})).unwrap();
        assert_eq!(state.available, Some(true));
        assert_eq!(state.warnings, ["example warning"]);
        assert!(state.apply_packages(json!({"available":"true","packages":[],"warnings":[]})).is_err());
        assert!(serde_json::from_value::<PackageStatus>(json!({"state":"verified_npu","message":"claim"})).is_err());
        state.finish(Action::Inspect, serde_json::to_value(json!({"descriptor":null,"launch":"x","warning":""})).unwrap()).unwrap_err();
        assert!(state.inspected.is_none() && !state.trust);
    }

    #[test]
    fn manager_has_a_distinct_untranslated_menu_route_without_a_document() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let items = crate::menus::menu_items(&app);
        let manager = items.iter().find(|item| item.id == MENU_ID).expect("parent wired manager menu");
        assert_eq!(manager.label, MENU_LABEL);
        assert_eq!(manager.path, MENU_PATH);
        assert!(manager.enabled && crate::menus::is_enabled(&app, MENU_ID));
        assert_eq!(items.iter().filter(|item| item.id == MENU_ID).count(), 1);
        let result = crate::menus::invoke(&mut app, &egui::Context::default(), MENU_ID, json!({})).unwrap();
        assert_eq!(result, json!({"dialog":MENU_ID}));
        assert!(app.adapter_ui.open);
        assert!(!app.adapter_ui.trust && app.adapter_ui.pending.is_none());
        assert!(menu(&mut app, "plugin.install", &json!({})).is_none(), "never intercept the WASM installer");
        assert!(menu(&mut app, MENU_ID, &json!({"trust":true})).unwrap().is_err());
    }

    #[test]
    fn adapter_actions_use_app_run_authorization_and_revoke_download_consent() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.automation_input = true;
        app.services.automation_command = Some(Box::new(|id, _| Err(format!("blocked {id}"))));
        let mut state = State { install_path: "not-read.json".into(), ..Default::default() };
        let error = state.run(&mut app, Action::Inspect).unwrap_err();
        assert_eq!(error, "blocked plugin.adapter.inspectPackage");
        state.prepare_consent.insert("example.adapter".into());
        let error = state.run(&mut app, Action::Prepare("example.adapter".into())).unwrap_err();
        assert_eq!(error, "blocked plugin.adapter.prepare");
        assert!(state.prepare_consent.is_empty());
    }

    #[test]
    fn idle_frames_do_not_probe_refresh_or_download() {
        use std::cell::Cell;
        use std::rc::Rc;
        let calls = Rc::new(Cell::new(0usize));
        let recorded = Rc::clone(&calls);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.automation_input = true;
        app.services.automation_command = Some(Box::new(move |_, _| {
            recorded.set(recorded.get() + 1);
            Ok(())
        }));
        app.adapter_ui.open = true;
        app.adapter_ui.available = Some(true);
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        for _ in 0..3 {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                show(&mut app, ui.ctx());
            })
            .textures_delta
            .clear();
        }
        assert_eq!(calls.get(), 0);
    }
}
