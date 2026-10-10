//! A nonblocking, egui-native picker for the app's private Documents directory.
//!
//! Android's Storage Access Framework is deliberately not imitated here:
//! selecting a document in another app requires a content URI and a Java/NDK
//! permission-aware bridge. This picker only sees our own files.

use std::collections::BTreeSet;
use std::path::Path;

use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest};

pub struct Picker {
    request: FileDialogRequest,
    reply: Option<FileDialogReply>,
    selected: BTreeSet<String>,
    name: String,
}

impl Picker {
    pub fn new(request: FileDialogRequest, reply: FileDialogReply) -> Self {
        let name = match &request {
            FileDialogRequest::Save { suggested } => suggested
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("Untitled.pcraft")
                .to_owned(),
            FileDialogRequest::Open { .. } => String::new(),
        };
        Self { request, reply: Some(reply), selected: BTreeSet::new(), name }
    }

    /// True when a result or cancellation has been handed back to PhotoCraft.
    pub fn show(&mut self, ctx: &egui::Context, directory: Option<&Path>) -> bool {
        let mut finished: Option<Option<FileDialogAnswer>> = None;
        let saving = matches!(self.request, FileDialogRequest::Save { .. });

        egui::Window::new(if saving { "Save document" } else { "Open document" })
            .collapsible(false)
            .resizable(true)
            .default_width(440.0)
            .show(ctx, |ui| {
                let Some(dir) = directory else {
                    ui.label("Android's private data directory is not available.");
                    if ui.button("Cancel").clicked() {
                        finished = Some(None);
                    }
                    return;
                };
                ui.small(format!("PhotoCraft Documents: {}", dir.display()));
                ui.separator();
                match &self.request {
                    FileDialogRequest::Open { extensions, multiple, .. } => {
                        let names = readable_files(dir, extensions.as_deref());
                        if names.is_empty() {
                            ui.label("No compatible documents in the application's Documents directory.");
                        }
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            for name in names {
                                let mut selected = self.selected.contains(&name);
                                if ui.checkbox(&mut selected, &name).changed() {
                                    if !*multiple {
                                        self.selected.clear();
                                    }
                                    if selected {
                                        self.selected.insert(name.clone());
                                    } else {
                                        self.selected.remove(&name);
                                    }
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.add_enabled(!self.selected.is_empty(), egui::Button::new("Open")).clicked() {
                                finished = Some(Some(FileDialogAnswer::Paths(
                                    self.selected.iter().map(|name| dir.join(name).to_string_lossy().into_owned()).collect()
                                )));
                            }
                            if ui.button("Cancel").clicked() {
                                finished = Some(None);
                            }
                        });
                    }
                    FileDialogRequest::Save { .. } => {
                        ui.label("File name");
                        ui.text_edit_singleline(&mut self.name);
                        ui.horizontal(|ui| {
                            let leaf = self.name.rsplit(['/', '\\']).next().unwrap_or_default();
                            let valid = !leaf.is_empty() && leaf != "." && leaf != ".." && !leaf.contains('\0');
                            if ui.add_enabled(valid, egui::Button::new("Save")).clicked() {
                                finished = Some(Some(FileDialogAnswer::SaveTo(
                                    dir.join(leaf).to_string_lossy().into_owned()
                                )));
                            }
                            if ui.button("Cancel").clicked() {
                                finished = Some(None);
                            }
                        });
                    }
                }
            });

        if let Some(answer) = finished {
            if let Some(reply) = self.reply.take() {
                reply.send(answer);
            }
            return true;
        }
        false
    }
}

fn readable_files(dir: &Path, allowed: Option<&[String]>) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_file() {
                return None;
            }
            let filename = entry.file_name().into_string().ok()?;
            if let Some(allowed) = allowed {
                let ext = Path::new(&filename).extension()?.to_str()?;
                if !allowed.iter().any(|v| v.trim_start_matches('.').eq_ignore_ascii_case(ext)) {
                    return None;
                }
            }
            Some(filename)
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|name| name.to_ascii_lowercase());
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_directory_returns_no_files() {
        assert!(readable_files(Path::new("/path/that/does/not/exist"), None).is_empty());
    }
}
