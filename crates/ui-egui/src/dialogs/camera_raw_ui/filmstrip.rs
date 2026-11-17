//! The open-time dialog's filmstrip: raws opened together wait side by side in one Camera Raw
//! dialog, as in Photoshop's. Each keeps its own settings and demosaic choice when another is
//! shown, Open develops every one and Cancel closes every one. The framework's `RawFilmstrip`
//! draws the tiles; this module holds the raws and their thumbnails.

use super::open::{Target, commit_open_raw, open_raw_with, return_on_failure};
use super::sidecar::read_sidecar;
use super::{CameraRaw, PhotocraftApp, RawOpen, Rect, Value, hosted, image, json, workspace_demosaic};

/// Long side of a filmstrip thumbnail, in pixels.
const THUMB_SIDE: usize = 240;

/// A raw waiting in the open-time dialog: opened (developed with the defaults) and reviewed
/// before Open keeps it.
pub struct HeldRaw {
    pub raw: RawOpen,
    /// Its settings and the settings it started from (its sidecar's, or the defaults) once it
    /// has been shown and left for another (`None`: not edited yet).
    pub(super) kept: Option<(CameraRaw, CameraRaw)>,
    /// The filmstrip thumbnail until it is first handed to the workspace, then its texture.
    pub(super) thumb: Option<egui::ColorImage>,
    // PLAN: the framework tile takes a `SizedTexture`, which does not own its texture: the handle stays here so the texture lives as long as the raw is held.
    tex: Option<egui::TextureHandle>,
}

impl HeldRaw {
    pub(super) fn new(app: &PhotocraftApp, raw: RawOpen) -> Self {
        let thumb = super::open::document_index(app, raw.document).and_then(|i| thumbnail(app, i));
        HeldRaw { raw, kept: None, thumb, tex: None }
    }
}

/// A filmstrip thumbnail of document `index` (a just-developed raw: its first pixel layer).
fn thumbnail(app: &PhotocraftApp, index: usize) -> Option<egui::ColorImage> {
    let doc = &app.session.documents().get(index)?.doc;
    let surf = doc.layers.iter().find_map(|l| l.surface())?;
    let area = doc.bounds();
    let (w, h) = (area.width() as usize, area.height() as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let k = w.max(h).div_ceil(THUMB_SIDE).max(1);
    let (tw, th) = (w.div_ceil(k), h.div_ceil(k));
    let mut acc = vec![[0.0f32; 5]; tw * th];
    let mut row = vec![[0.0f32; 4]; w];
    for y in (0..h).step_by(k.div_ceil(2).max(1)) {
        surf.read_rgba_into(Rect::new(area.x0, area.y0 + y as i32, area.x1, area.y0 + y as i32 + 1), &mut row);
        for (x, q) in row.iter().enumerate().step_by(k.div_ceil(2).max(1)) {
            let a = &mut acc[(y / k) * tw + x / k];
            for c in 0..4 {
                a[c] += q[c];
            }
            a[4] += 1.0;
        }
    }
    let px: Vec<[f32; 4]> = acc.iter().map(|a| if a[4] > 0.0 { [a[0] / a[4], a[1] / a[4], a[2] / a[4], a[3] / a[4]] } else { [0.0; 4] }).collect();
    let display = app.session.color.canvas_display(doc).ok()?;
    Some(image(&px, tw, th, &display))
}

/// `px` (`w`×`h`) box-averaged down to at most [`THUMB_SIDE`] pixels on its long side.
fn shrink(px: &[[f32; 4]], w: usize, h: usize) -> (Vec<[f32; 4]>, usize, usize) {
    let k = w.max(h).div_ceil(THUMB_SIDE).max(1);
    let (tw, th) = (w.div_ceil(k), h.div_ceil(k));
    let mut acc = vec![[0.0f32; 5]; tw * th];
    for (i, q) in px.iter().enumerate().take(w * h) {
        if let Some(a) = acc.get_mut((i / w / k) * tw + (i % w) / k) {
            for c in 0..4 {
                a[c] += q[c];
            }
            a[4] += 1.0;
        }
    }
    (acc.iter().map(|a| if a[4] > 0.0 { [a[0] / a[4], a[1] / a[4], a[2] / a[4], a[3] / a[4]] } else { [0.0; 4] }).collect(), tw, th)
}

/// Hands each held raw's thumbnail to the workspace's tile, with whether the raw is edited.
pub(super) fn sync_pictures(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let current = app.raw.as_ref().and_then(|s| s.raw_open.as_ref()).map(|r| r.document);
    let Some(shown) = hosted(app) else { return };
    let current_edited = !shown.workspace().session().is_identity();
    // The shelf is a field beside the dialog host, so both are borrowed at once.
    let shelf = &mut app.raw_shelf;
    let Some(workspace) = app.framework.dialogs.get_mut::<super::Hosted>() else { return };
    for (i, held) in shelf.iter_mut().enumerate() {
        if let Some(img) = held.thumb.take() {
            held.tex = Some(ctx.load_texture(format!("camera-raw-thumb-{}", held.raw.document.0), img, egui::TextureOptions::LINEAR));
        }
        let Some(tex) = &held.tex else { continue };
        let edited = if Some(held.raw.document) == current { current_edited } else { held.kept.as_ref().is_some_and(|(p, _)| !p.is_identity()) };
        workspace.workspace_mut().set_item_picture(i, egui::load::SizedTexture::from_handle(tex), edited);
    }
}

/// Shows raw `i` of the filmstrip, with the settings it was left with (its sidecar's, the
/// Camera Raw Defaults' Auto, or the defaults the first time).
pub(super) fn open_held(app: &mut PhotocraftApp, ctx: &egui::Context, i: usize) -> Result<(), String> {
    let held = app.raw_shelf.get(i).ok_or("no raw file at that place in the filmstrip")?;
    let (raw, kept) = (held.raw.clone(), held.kept.clone());
    open_raw_with(app, ctx, raw, kept)
}

/// Shows raw `to` of the filmstrip; the one being left keeps its settings, and its thumbnail
/// shows them.
pub(super) fn show_raw(app: &mut PhotocraftApp, ctx: &egui::Context, to: usize) -> Result<(), String> {
    let Some(session) = app.raw.as_ref() else { return Ok(()) };
    let current = session.raw_open.as_ref().map(|r| r.document);
    if session.target != Target::OpenRaw || app.raw_shelf.get(to).is_none_or(|h| Some(h.raw.document) == current) {
        return Ok(());
    }
    keep_settings(app);
    sync_pictures(app, ctx);
    // On success this shows the raw in the workspace. On failure the workspace stays on the raw being left, and
    // the other keeps its place in the filmstrip.
    let active = app.session.active_index();
    let shown = open_held(app, ctx, to);
    if shown.is_err()
        && let Some(i) = active
    {
        app.session.set_active(i);
    }
    shown
}

/// Stores the workspace's settings (and demosaic choice) on its filmstrip raw, and refreshes the
/// raw's thumbnail from the preview.
pub(super) fn keep_settings(app: &mut PhotocraftApp) {
    let Some(workspace) = hosted(app) else { return };
    let settings = workspace.workspace().session().clone();
    let Some(demosaic) = workspace_demosaic(workspace) else { return };
    keep(app, &settings, demosaic, true);
}

/// Stores `settings` and `demosaic` on the held raw of the shown one; with `thumbnail`, refreshes its thumbnail from the
/// develop of those settings.
pub(super) fn keep(app: &mut PhotocraftApp, settings: &CameraRaw, demosaic: photocraft_io::raw::Demosaic, thumbnail: bool) {
    let Some(session) = app.raw.as_ref() else { return };
    let Some(raw) = &session.raw_open else { return };
    let (document, original, source) = (raw.document, session.original.clone(), session.source.clone());
    let Some(held) = app.raw_shelf.iter_mut().find(|h| h.raw.document == document) else { return };
    held.kept = Some((settings.clone(), original));
    held.raw.demosaic = demosaic;
    if !thumbnail {
        return;
    }
    // The preview may not have caught up with the last edit: the thumbnail shows what was kept.
    let developed = source.developed().filter(|d| &d.settings == settings).map(|d| d.processed.clone()).unwrap_or_else(|| source.proxy_job(settings).run().px);
    if !developed.is_empty() {
        let (px, w, h) = shrink(&developed, source.pw, source.ph);
        held.thumb = Some(image(&px, w, h, &source.display));
        held.tex = None;
    }
}

/// The settings Open develops a never-shown filmstrip raw with, and what they started from:
/// its sidecar's, else the Camera Raw Defaults' Auto when that is on, else the defaults.
pub(super) fn first_settings(app: &PhotocraftApp, raw: &RawOpen) -> (CameraRaw, Option<CameraRaw>) {
    if let Some((loaded, _, _)) = raw.path.as_deref().and_then(read_sidecar) {
        return (loaded.clone(), Some(loaded));
    }
    let mut p = CameraRaw::default();
    // From the proxy the dialog would use, so showing a raw doesn't change what Auto gives it.
    if app.session.prefs().raw_defaults.apply_auto_tone
        && let Some(st) = super::open::document_index(app, raw.document).and_then(|i| app.session.documents().get(i))
        && let Some(surf) = st.doc.layers.iter().find_map(|l| l.surface())
        && let Ok((area, w, h)) = super::convert::preview_area(st.doc.bounds(), surf)
    {
        let (proxy, ..) = super::convert::box_proxy(surf, area, w, h);
        photocraft_algo::camera_raw::auto_tone(&proxy, &mut p);
    }
    (p, None)
}

/// Open with raws in the filmstrip: the shown one as the dialog has it, then every other with
/// its own settings. One reply for one raw, `{"opened": [...]}` for several. A raw leaves the
/// filmstrip once it has opened or its background develop has started: one that fails stays or
/// comes back there with its settings, so the dialog returns for it (or Cancel still closes it).
pub(super) fn open_all(app: &mut PhotocraftApp, params: &CameraRaw, raw: &RawOpen, original: &CameraRaw) -> Result<Value, String> {
    let others: Vec<photocraft_doc::DocId> = app.raw_shelf.iter().map(|h| h.raw.document).filter(|id| *id != raw.document).collect();
    let first = commit_open_raw(app, params, raw, Some(original));
    if let Ok(v) = &first {
        return_on_failure(app, v, original);
        app.raw_shelf.retain(|h| h.raw.document != raw.document);
    }
    if others.is_empty() {
        return first;
    }
    let mut opened = Vec::new();
    let mut failed = Vec::new();
    match first {
        Ok(v) => opened.push(v),
        Err(e) => failed.push(format!("{}: {e}", raw.name)),
    }
    for id in others {
        let Some(held) = app.raw_shelf.iter().find(|h| h.raw.document == id) else { continue };
        let (held_raw, kept) = (held.raw.clone(), held.kept.clone());
        let (p, loaded) = match kept {
            Some((p, original)) => (p, Some(original)),
            None => first_settings(app, &held_raw),
        };
        match commit_open_raw(app, &p, &held_raw, loaded.as_ref()) {
            Ok(v) => {
                return_on_failure(app, &v, loaded.as_ref().unwrap_or(&CameraRaw::default()));
                opened.push(v);
                app.raw_shelf.retain(|h| h.raw.document != id);
            }
            Err(e) => failed.push(format!("{}: {e}", held_raw.name)),
        }
    }
    if !failed.is_empty() {
        let e = failed.join("; ");
        if opened.is_empty() {
            return Err(e);
        }
        crate::notices::error(app, e);
    }
    Ok(json!({"opened": opened}))
}

/// Done: saves each raw's settings to its sidecar, as Camera Raw's Done does, and opens none of
/// them (they close again). `settings` are the shown raw's.
pub(super) fn done(app: &mut PhotocraftApp, settings: &CameraRaw) -> Value {
    use super::sidecar::Stored;
    let Some(raw) = app.raw.as_ref().and_then(|s| s.raw_open.clone()) else { return Value::Null };
    let mut raws = vec![(raw.clone(), settings.clone())];
    for held in app.raw_shelf.iter().filter(|h| h.raw.document != raw.document) {
        let p = match &held.kept {
            Some((p, _)) => p.clone(),
            None => first_settings(app, &held.raw).0,
        };
        raws.push((held.raw.clone(), p));
    }
    let stored: Vec<Stored> = raws.iter().map(|(raw, p)| super::sidecar::store_sidecar(app, raw, p)).collect();
    let saved: Vec<&String> = stored.iter().filter_map(|s| if let Stored::Written(name) = s { Some(name) } else { None }).collect();
    // A failed write (already in a notice) keeps the dialog and every raw open, so no edit is lost.
    if stored.contains(&Stored::Failed) {
        app.ui.status = tl!("Some settings could not be saved; the raws stay open").to_string();
        app.ui.status_error = true;
        return json!({"done": false, "saved": saved});
    }
    app.ui.status = if !saved.is_empty() {
        crate::i18n::fmt(tl!("Saved RAW Develop settings to {file}"), &[("file", &saved.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))])
    } else if stored.iter().all(|s| *s == Stored::Defaults) {
        tl!("Nothing to save: these raws are at their default settings").to_string()
    } else {
        tl!("No settings saved: these raws can't keep a sidecar here").to_string()
    };
    app.ui.status_error = saved.is_empty() && stored.contains(&Stored::Skipped);
    let saved = json!(saved);
    app.raw_shelf.clear();
    for (raw, _) in &raws {
        if let Some(index) = super::open::document_index(app, raw.document) {
            app.session.close(index);
        }
    }
    app.sync_views();
    json!({"done": true, "saved": saved})
}
