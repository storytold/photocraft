//! Plugins › Browse Plugins… and Manage Plugins…: a non-modal window over the plug-in catalog
//! (`plugin.store.*` engine commands). Browse lists every catalog plug-in by its GitHub project,
//! with Install / Update per row and Install Selected for several at once; Manage lists the
//! installed ones with Uninstall. Downloads run in the background; their rows show a progress bar
//! and Cancel.

use std::collections::BTreeSet;

use photocraft_engine::plugin_store_cmds::{Entry, Job};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The window's state (in `UiState`, so `ui.inspect`/`ui.set` can read and drive it).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StoreView {
    pub open: bool,
    /// Manage Plugins… (installed only) rather than Browse.
    pub installed_only: bool,
    pub query: String,
    /// Ids ticked for Install Selected.
    pub selected: BTreeSet<String>,
}

pub fn open(app: &mut PhotocraftApp, installed_only: bool) -> Value {
    let v = &mut app.ui.plugin_store;
    v.open = true;
    v.installed_only = installed_only;
    json!({"open": true, "installedOnly": installed_only})
}

/// Finishes downloads and keeps the window repainting while they run.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    app.session.plugin_store.poll();
    if app.session.plugin_store.busy() {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// What a row can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    NotInstalled,
    Installed,
    Update,
    Busy,
}

fn installed_version(id: &str) -> Option<String> {
    photocraft_plugins::registry::get(id).map(|p| p.manifest().version.clone())
}

fn state_of(app: &PhotocraftApp, e: &Entry) -> State {
    if matches!(app.session.plugin_store.job(&e.id), Some(Job::Queued | Job::Downloading)) {
        return State::Busy;
    }
    match installed_version(&e.id) {
        None => State::NotInstalled,
        Some(v) if v != e.version => State::Update,
        Some(_) => State::Installed,
    }
}

fn matches(e: &Entry, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || [&e.name, &e.description, &e.author, &e.collection].iter().any(|f| f.to_lowercase().contains(&q))
}

/// The listings the window shows, in catalog order (grouped by collection).
fn visible(app: &PhotocraftApp) -> Vec<Entry> {
    let v = &app.ui.plugin_store;
    app.session.plugin_store.catalog().iter().filter(|e| matches(e, &v.query) && (!v.installed_only || installed_version(&e.id).is_some())).cloned().collect()
}

/// What a click asked for, run after drawing (commands need `&mut app`).
enum Action {
    Install(Vec<String>),
    Uninstall(String),
    Cancel(String),
    Link(String),
}

fn size_text(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.0} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.plugin_store.open {
        return;
    }
    let mut open = true;
    let mut action: Option<Action> = None;
    let entries = visible(app);
    let available = app.session.plugin_store.available();
    let states: Vec<State> = entries.iter().map(|e| state_of(app, e)).collect();
    let jobs: Vec<Option<Job>> = entries.iter().map(|e| app.session.plugin_store.job(&e.id).cloned()).collect();
    let progress: Vec<Option<(u64, u64)>> = entries.iter().map(|e| app.session.plugin_store.progress(&e.id)).collect();
    let title = if app.ui.plugin_store.installed_only { tl!("Manage Plugins") } else { tl!("Browse Plugins") };
    egui::Window::new(title)
        .id(egui::Id::unique("photocraft_plugin_store"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([560.0, 520.0])
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .min_width(420.0)
        .show(ctx, |ui| {
            let t = crate::theme::Tokens::get(ctx);
            let view = &mut app.ui.plugin_store;
            ui.horizontal(|ui| {
                if crate::widgets::pill_tab(ui, "Browse", !view.installed_only).clicked() {
                    view.installed_only = false;
                }
                if crate::widgets::pill_tab(ui, "Installed", view.installed_only).clicked() {
                    view.installed_only = true;
                }
                ui.add_space(8.0);
                let w = (ui.available_width() - 4.0).max(120.0);
                crate::widgets::text_field(ui, egui::Id::unique("plugin_store_query"), &mut view.query, tl!("Search plug-ins"), w, false);
            });
            ui.add_space(6.0);
            if !available {
                ui.colored_label(t.warning, tl!("Downloading plug-ins is not available in this build."));
                ui.add_space(4.0);
            }
            let footer = crate::widgets::button_height(ctx) + 44.0;
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height((ctx.content_rect().height() - 220.0 - footer).clamp(120.0, 440.0)).show(ui, |ui| {
                if entries.is_empty() {
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(if view.installed_only { tl!("No catalog plug-ins are installed.") } else { tl!("No plug-ins match the search.") }).color(t.text_dim));
                }
                let mut collection = "";
                for (((e, state), job), progress) in entries.iter().zip(&states).zip(&jobs).zip(&progress) {
                    if e.collection != collection {
                        collection = &e.collection;
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            crate::widgets::section_label(ui, &e.collection);
                            if ui.link(tl!("Source on GitHub")).on_hover_text(&e.repo).clicked() {
                                action = Some(Action::Link(e.repo.clone()));
                            }
                        });
                        if let Some(why) = &e.deprecated {
                            ui.add(egui::Label::new(egui::RichText::new(why).color(t.warning)).wrap());
                        }
                        crate::widgets::hairline(ui);
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let mut ticked = view.selected.contains(&e.id);
                        let can_tick = available && matches!(state, State::NotInstalled | State::Update);
                        ui.add_enabled_ui(can_tick, |ui| {
                            let r = crate::widgets::checkbox(ui, &mut ticked, "");
                            // The name is drawn beside the box, not as its label: give the box it too.
                            r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, can_tick, ticked, &e.name));
                            if r.changed() {
                                if ticked {
                                    view.selected.insert(e.id.clone());
                                } else {
                                    view.selected.remove(&e.id);
                                }
                            }
                        });
                        ui.vertical(|ui| {
                            // Room for the row's buttons: an outdated plug-in has Update and Uninstall.
                            let buttons = if matches!(state, State::Update | State::Busy) { 196.0 } else { 110.0 };
                            ui.set_width((ui.available_width() - buttons).max(160.0));
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&e.name).font(crate::theme::medium(12.5)).color(t.text));
                                ui.label(egui::RichText::new(format!("{} · {} · {}", e.version, e.author, size_text(e.size))).color(t.text_faint));
                            });
                            ui.add(egui::Label::new(egui::RichText::new(&e.description).color(t.text_dim)).wrap());
                            if let Some(Job::Failed { message }) = job {
                                ui.add(egui::Label::new(egui::RichText::new(message).color(t.warning)).wrap());
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match state {
                            State::Busy => {
                                if crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked() {
                                    action = Some(Action::Cancel(e.id.clone()));
                                }
                                match progress {
                                    Some((received, total)) if *total > 0 => {
                                        let bar = egui::ProgressBar::new(*received as f32 / *total as f32).desired_width(96.0).show_percentage();
                                        ui.add(bar).on_hover_text(format!("{} / {}", size_text(*received as usize), size_text(*total as usize)));
                                    }
                                    _ => {
                                        ui.label(egui::RichText::new(tl!("Queued")).color(t.text_dim));
                                    }
                                }
                            }
                            State::Installed => {
                                if crate::widgets::secondary_button(ui, tl!("Uninstall"), 84.0).clicked() {
                                    action = Some(Action::Uninstall(e.id.clone()));
                                }
                            }
                            State::NotInstalled | State::Update => {
                                let label = if *state == State::Update { tl!("Update") } else if matches!(job, Some(Job::Failed { .. })) { tl!("Retry") } else { tl!("Install") };
                                let clicked = ui.add_enabled_ui(available, |ui| crate::widgets::secondary_button(ui, label, 84.0).clicked()).inner;
                                if clicked {
                                    action = Some(Action::Install(vec![e.id.clone()]));
                                }
                                // An outdated plug-in can still be removed when updating it fails or can't run.
                                if *state == State::Update && crate::widgets::secondary_button(ui, tl!("Uninstall"), 84.0).clicked() {
                                    action = Some(Action::Uninstall(e.id.clone()));
                                }
                            }
                        });
                    });
                }
            });
            ui.add_space(6.0);
            crate::widgets::hairline(ui);
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(tl!(
                        "Community plug-ins from GitHub. Each download is checked against the catalog's checksum, and plug-ins run sandboxed, without access to files, network or the system."
                    ))
                    .color(t.text_faint),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            // Ticks on rows that are no longer installable (installed meanwhile) don't count.
            view.selected.retain(|id| entries.iter().zip(&states).any(|(e, s)| e.id == *id && matches!(s, State::NotInstalled | State::Update)) || !entries.iter().any(|e| e.id == *id));
            // Only the ticks this view shows: one made in Browse, or hidden by the search, waits.
            let ticked: Vec<String> = view.selected.iter().filter(|id| entries.iter().any(|e| e.id == **id)).cloned().collect();
            ui.horizontal(|ui| {
                let label = crate::i18n::fmt(tl!("Install Selected ({count})"), &[("count", &ticked.len().to_string())]);
                let enabled = available && !ticked.is_empty();
                if ui.add_enabled_ui(enabled, |ui| crate::widgets::primary_button(ui, &label, 160.0).clicked()).inner {
                    action = Some(Action::Install(ticked.clone()));
                }
            });
        });
    app.ui.plugin_store.open = open;
    match action {
        Some(Action::Install(ids)) => {
            match app.run("plugin.store.install", json!({"ids": ids})) {
                Ok(_) => app.ui.plugin_store.selected.retain(|id| !ids.contains(id)),
                Err(e) => app.ui.status = e,
            }
        }
        Some(Action::Uninstall(id)) => {
            if let Err(e) = app.run("plugin.store.uninstall", json!({"id": id})) {
                app.ui.status = e;
            }
        }
        Some(Action::Cancel(id)) => {
            if let Err(e) = app.run("plugin.store.cancel", json!({"id": id})) {
                app.ui.status = e;
            }
        }
        Some(Action::Link(url)) => {
            crate::links::open(app, ctx, &url);
        }
        None => {}
    }
}

#[cfg(test)]
#[path = "plugin_store_ui_tests.rs"]
mod tests;
