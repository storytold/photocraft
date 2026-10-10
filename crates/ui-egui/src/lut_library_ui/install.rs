//! Installing LUT packs from the UI: the folder dialog, drops, and the status line, plus the
//! housekeeping that ends a canvas preview when the browser stops being drawn.

use std::path::Path;

use egui::Id;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The status line after `lut.installPack` finished (`v` is its result).
pub fn report(app: &mut PhotocraftApp, v: &Value) {
    let n = v.get("count").and_then(Value::as_u64).unwrap_or(0);
    let pack = v.get("pack").and_then(Value::as_str).unwrap_or_default();
    let skipped = v.get("skipped").and_then(Value::as_array).map_or(0, Vec::len);
    let dupes = v.get("duplicates").and_then(Value::as_array).map_or(0, Vec::len);
    let mut line = crate::i18n::fmt(tl!("LUTs installed into {pack}: {n}"), &[("pack", pack), ("n", &n.to_string())]);
    let notes: Vec<String> = [(skipped, tl!("{n} skipped")), (dupes, tl!("{n} already installed"))]
        .iter()
        .filter(|(c, _)| *c > 0)
        .map(|(c, what)| crate::i18n::fmt(what, &[("n", &c.to_string())]))
        .collect();
    if !notes.is_empty() {
        line.push_str(&format!(" ({})", notes.join(", ")));
    }
    app.ui.status = line;
    app.ui.status_error = false;
}

/// Install the folder or `.zip` at `path` as a pack, reporting the result in the status bar.
pub fn install_path(app: &mut PhotocraftApp, path: &str) {
    if let Ok(v) = app.run("lut.installPack", json!({"path": path}))
        && v.get("count").is_some()
    {
        report(app, &v);
    }
}

/// The frame the LUT browser was last drawn in, kept in egui memory.
const SHOWN: &str = "lutlib-shown";

/// Called by the browser every frame it is drawn, so a drop can tell whether it landed on it.
pub(super) fn mark_shown(ctx: &egui::Context) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(Id::new(SHOWN), frame));
}

/// Whether the LUT browser was drawn in the previous frame or this one. A drop is read at the start
/// of a frame, before the panel is drawn, so "the previous frame" is the one the user was looking at.
fn browser_visible(ctx: &egui::Context) -> bool {
    let seen: Option<u64> = ctx.data(|d| d.get_temp(Id::new(SHOWN)));
    seen.is_some_and(|seen| ctx.cumulative_frame_nr() <= seen.saturating_add(1))
}

/// Whether a dropped file is a LUT pack: a folder or a `.zip`, dropped while Color Lookup's LUT
/// browser is showing, in a session that has a library. Anywhere else a folder or archive is not
/// ours to take (a folder of photos with one stray `.cube` must not install silently), so it opens
/// like any other drop.
pub fn is_pack_drop(app: &PhotocraftApp, ctx: &egui::Context, path: &Path) -> bool {
    app.session.lut_library.is_some()
        && browser_visible(ctx)
        && path.is_absolute()
        && (path.is_dir() || path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")))
}

pub(super) fn install_dialog(app: &mut PhotocraftApp) {
    if app.services.file_dialog.is_none() {
        app.ui.status = tl!("Choosing a folder is not available here; use the lut.installPack command").into();
        app.ui.status_error = true;
        return;
    }
    // The dialog never blocks the frame (`file_dialog`); the install starts once it answers.
    if let Err(e) = app.pick_folder(|app, dir| {
        install_path(app, &dir);
        Ok(Value::Null)
    }) && e != crate::file_dialog::CANCELLED
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The preview of a LUT stops when the editor is no longer drawn (another layer, a closed panel).
pub fn end_stale_preview(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let (flag, seen): (u64, u64) = ctx.data(|d| d.get_temp(Id::new("lutlib-previewing"))).unwrap_or_default();
    if flag != 0 && ctx.cumulative_frame_nr() > seen + 1 {
        if app.live_adjust.as_ref().is_some_and(|(l, _)| l.0 == flag) {
            app.live_adjust = None;
        }
        ctx.data_mut(|d| d.insert_temp(Id::new("lutlib-previewing"), (0u64, 0u64)));
    }
}
