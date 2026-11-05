//! Optional candidate-state captures shared by the existing Actions harnesses.
use egui_kittest::{Harness, HarnessBuilder};
use std::{fs::OpenOptions, path::PathBuf};

pub(crate) fn enabled() -> bool {
    std::env::var_os("PHOTOCRAFT_ACTIONS_SNAPSHOTS").is_some()
}

pub(crate) fn builder<State>(size: egui::Vec2) -> HarnessBuilder<State> {
    let builder = Harness::builder().with_size(size);
    if enabled() { builder.wgpu() } else { builder }
}

/// The caller supplies an existing project-local output directory for this run.
pub(crate) fn capture<State>(h: &mut Harness<'_, State>, name: &str) {
    let Some(dir) = std::env::var_os("PHOTOCRAFT_ACTIONS_SNAPSHOTS") else { return };
    assert!(name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'));
    let dir = PathBuf::from(dir);
    assert!(dir.is_absolute(), "snapshot output must be an absolute directory");
    let dir = dir.canonicalize().expect("snapshot output directory must already exist");
    let path = dir.join(format!("{name}.png"));
    // Reserve only a fresh child of the supplied directory; never replace a previous capture.
    let _reservation = OpenOptions::new().write(true).create_new(true).open(&path).expect("snapshot output must be new");
    h.render().expect("Actions state must render").save(&path).expect("Actions PNG must save");
    eprintln!("Actions candidate-state capture: {}", path.display());
}
