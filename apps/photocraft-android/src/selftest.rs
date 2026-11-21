#![cfg(target_os = "android")]

//! Startup self-test: PhotoCraft's own new → fill → save → open round trip, on the device, through
//! the very services this shell installs ([`crate::services::services`]).
//!
//! The FilmCraft pilot proved its MediaCodec backend by decoding an embedded clip through the
//! engine's factory; PhotoCraft has no video pipeline, so its equivalent is the file pipeline that
//! Android changes most: `services.import`, `services.export`, `services.write` and
//! `open_path`. The self-test builds a document with the app's own commands, saves it as PSD and
//! PNG (the encoders the shell installed), opens each file back through the app's own importer and
//! compares a pixel — logcat then carries the sizes and the result.
//!
//! No test asset is embedded: the document is synthesized by the engine, so there is nothing to
//! license and nothing to ship. It runs on a background thread and never touches the UI.

use photocraft_engine::Session;
use photocraft_ui_egui::PhotocraftApp;

use crate::services::{Incoming, Paths, Saf};

/// Run the round trip on its own thread and log the result (never blocks or fails the app).
pub fn spawn(paths: Paths, saf: Saf, incoming: Incoming) {
    std::thread::spawn(move || match round_trip(&paths, &saf, &incoming) {
        Ok(line) => log::info!("photocraft-android: {line}"),
        Err(e) => log::warn!("photocraft-android: self-test failed: {e}"),
    });
}

/// The red the document is filled with and the tolerance a round trip may shift it by (the PSD and
/// PNG paths both go through the document's colour management).
fn is_red(px: [f32; 4]) -> bool {
    px[0] > 0.9 && px[1] < 0.1 && px[2] < 0.1 && px[3] > 0.9
}

fn round_trip(paths: &Paths, saf: &Saf, incoming: &Incoming) -> Result<String, String> {
    let dir = paths.data.join("selftest");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // The app is headless here: the same `PhotocraftApp` the window runs, with the same services,
    // so a passing round trip means the shell's seams work (the window adds only the painting).
    let mut app = PhotocraftApp::new(Session::new(), crate::services::services(paths, saf, incoming));
    app.run("file.new", serde_json::json!({"width": 16, "height": 16})).map_err(|e| format!("file.new: {e}"))?;
    app.run("edit.fill", serde_json::json!({"color": "#ff0000"})).map_err(|e| format!("edit.fill: {e}"))?;
    app.session.documents().first().ok_or("file.new opened no document")?;

    let mut steps = Vec::new();
    for (file, format) in [("selftest.psd", "PSD"), ("selftest.png", "PNG")] {
        let path = dir.join(file);
        let target = path.to_string_lossy().into_owned();
        // `save_as` encodes through `services.export` and writes through `services.write` (the
        // crash-safe atomic write the desktop uses).
        app.save_as(Some(target.clone())).map_err(|e| format!("save {file}: {e}"))?;
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if bytes == 0 {
            return Err(format!("{file} was written empty"));
        }
        // Open the file back through `services.import` and check the pixel survived.
        app.open_path(&target).map_err(|e| format!("open {file}: {e}"))?;
        let doc = app.session.documents().last().ok_or_else(|| format!("{file} did not open"))?;
        let px = photocraft_compose::flatten(&doc.doc).px.first().copied().unwrap_or_default();
        if !is_red(px) {
            return Err(format!("{file} round trip changed the pixel: {px:?}"));
        }
        steps.push(format!("{format} {bytes} B → {}×{} px #ff0000 ✓", doc.doc.size.width, doc.doc.size.height));
    }
    Ok(format!("self-test: {} document(s), {}", app.session.documents().len(), steps.join(" | ")))
}
