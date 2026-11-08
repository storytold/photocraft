//! The open-time dialog's filmstrip: raws opened together wait side by side in one Camera Raw
//! dialog, as in Photoshop's. None is a document yet. Each keeps its own settings and demosaic
//! choice when another is shown, Open develops every one into a document and Cancel drops every
//! one.
//!
//! Several can be selected, as in Camera Raw's filmstrip: Shift-click selects a range, ⌘-click
//! (Ctrl elsewhere) adds or removes one, ⌘A selects every raw. The shown raw is always one of
//! them, and an edit to it changes the same settings on every other selected raw
//! ([`sync_selected`]).

use super::open::{Target, commit_open_raw, open_raw_with, return_on_failure};
use super::sidecar::read_sidecar;
use super::{
    Align2, CameraRaw, CameraRawDialog, Color32, ERect, FontId, PhotocraftApp, RawOpen, Rect, Sense, Stroke, Tokens, Value, json, pos2, vec2, widgets,
};

/// The filmstrip column, shown when more than one raw is being opened.
pub(super) const FILM_W: f32 = 150.0;
/// Long side of a filmstrip thumbnail, in pixels.
const THUMB_SIDE: usize = 240;

/// A raw waiting in the open-time dialog: developed with the defaults and reviewed before Open
/// makes it a document (until then it is not one: no tab, nothing on the canvas).
pub struct HeldRaw {
    pub raw: RawOpen,
    /// Its settings and the settings it started from (its sidecar's, or the defaults) once it
    /// has been shown and left for another (`None`: not edited yet).
    pub(super) kept: Option<(CameraRaw, CameraRaw)>,
    /// The filmstrip thumbnail until it is first drawn, then its texture.
    pub(super) thumb: Option<egui::ColorImage>,
    tex: Option<egui::TextureHandle>,
    /// In the filmstrip's selection. The shown raw always is, whether this is set or not.
    pub(super) selected: bool,
    /// Where the next Shift-click's range starts: the raw last clicked without Shift.
    anchor: bool,
    /// The thumbnail is older than `kept` (an edit synced from the shown raw): redrawn once the
    /// edit ends.
    pub(super) stale: bool,
    /// What the thumbnail is developed from, built the first time it has to be redrawn.
    source: Option<std::sync::Arc<ThumbSource>>,
    /// The thumbnail being redrawn on a worker: its source and image, `None` when it failed.
    redraw: Option<ThumbRedraw>,
    /// The thumbnail was taken from the dialog's preview while `redraw` ran: its result only
    /// fills the source cache. The worker still counts towards [`MAX_SOURCE_BUILDS`] until it
    /// is done.
    discard_redraw: bool,
}

/// A thumbnail redraw on a worker: its source and image, `None` when it failed.
type ThumbRedraw = std::sync::mpsc::Receiver<Option<(std::sync::Arc<ThumbSource>, egui::ColorImage)>>;

/// A filmstrip raw's pixels at thumbnail size and its camera tone curve, to develop its
/// thumbnail with settings it was given without being shown.
struct ThumbSource {
    px: std::sync::Arc<Vec<[f32; 4]>>,
    size: (usize, usize),
    full_w: usize,
    float: bool,
    curve: Option<std::sync::Arc<Vec<[f32; 2]>>>,
    /// The raw file binned to the thumbnail's size: synced raw-stage settings develop from the
    /// sensor data, as Open does ([`super::ProxyJob`]). `None`: the file can't be read again.
    raw: Option<crate::camera_raw_detail_ui::RawDetail>,
    encoding_gamma: f32,
}

impl HeldRaw {
    pub(super) fn new(app: &PhotocraftApp, raw: RawOpen) -> Self {
        let thumb = super::open::raw_document(app, &raw).and_then(|doc| thumbnail(app, &doc));
        HeldRaw { raw, kept: None, thumb, tex: None, selected: false, anchor: false, stale: false, source: None, redraw: None, discard_redraw: false }
    }
}

/// A filmstrip thumbnail of `doc` (a just-developed raw: its first pixel layer).
fn thumbnail(app: &PhotocraftApp, doc: &photocraft_doc::Document) -> Option<egui::ColorImage> {
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
    Some(CameraRawDialog::image(&px, tw, th, &display))
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
    let Some(d) = app.camera_raw.as_ref() else { return Ok(()) };
    let current = d.raw_open.as_ref().map(|r| r.document);
    if d.target != Target::OpenRaw || app.raw_shelf.get(to).is_none_or(|h| Some(h.raw.document) == current) {
        return Ok(());
    }
    keep_settings(app);
    // On success this replaces the dialog. On failure the dialog stays on the raw being left, and
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

/// Stores the dialog's settings (and demosaic choice) on its filmstrip raw, and refreshes the
/// raw's thumbnail from the preview.
pub(super) fn keep_settings(app: &mut PhotocraftApp) {
    let Some(d) = app.camera_raw.as_ref() else { return };
    let Some(raw) = &d.raw_open else { return };
    let Some(held) = app.raw_shelf.iter_mut().find(|h| h.raw.document == raw.document) else { return };
    held.kept = Some((d.params.clone(), d.original.clone()));
    held.raw.demosaic = raw.demosaic;
    // The preview may not have caught up with the last edit: the thumbnail shows what was kept.
    let frame = (d.dirty || d.render_pending()).then(|| d.proxy_job().run().px);
    let developed = frame.as_deref().unwrap_or(&d.processed);
    if !developed.is_empty() {
        let (px, w, h) = shrink(developed, d.pw, d.ph);
        held.thumb = Some(CameraRawDialog::image(&px, w, h, &d.display));
        // An older redraw still on a worker must not land over it.
        (held.discard_redraw, held.stale) = (held.redraw.is_some(), false);
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
    if app.session.prefs().raw_defaults.apply_auto_tone {
        auto_tone(app, raw, &mut p);
    }
    (p, None)
}

/// Camera Raw's Auto for a filmstrip raw, from the pixels the dialog's Auto reads
/// ([`default_proxy`]), so showing a raw doesn't change what Auto gives it.
pub(super) fn auto_tone(app: &PhotocraftApp, raw: &RawOpen, p: &mut CameraRaw) {
    if let Some(proxy) = super::open::raw_document(app, raw).and_then(|doc| default_proxy(&doc)) {
        photocraft_algo::camera_raw::auto_tone(&proxy, p);
    }
}

/// The default develop of a raw box-averaged to the preview's size: what Camera Raw Defaults ›
/// Apply auto tone reads, for a shown raw and one Open develops without showing it alike.
pub(super) fn default_proxy(doc: &photocraft_doc::Document) -> Option<Vec<[f32; 4]>> {
    let surf = doc.layers.iter().find_map(|l| l.surface())?;
    let (area, w, h) = super::convert::preview_area(doc.bounds(), surf).ok()?;
    Some(super::convert::box_proxy(surf, area, w, h).0)
}

/// Open with raws in the filmstrip: every selected one in filmstrip order, the shown one as the
/// dialog has it and the others with their own settings, as Camera Raw's Open opens the selected
/// images. The raws not selected are let go, as Cancel lets them go. One reply for one raw,
/// `{"opened": [...]}` for several. A raw leaves the filmstrip once it has opened or its
/// background develop has started: one that fails stays or comes back there with its settings,
/// so the dialog returns for it (or Cancel lets it go).
pub(super) fn open_all(app: &mut PhotocraftApp, params: &CameraRaw, raw: &RawOpen, original: &CameraRaw) -> Result<Value, String> {
    keep_settings(app);
    let selected = selection(app);
    // In filmstrip order, the shown raw in its place, so the tabs follow the filmstrip.
    let mut order = Vec::new();
    let mut dropped = Vec::new();
    for (held, selected) in app.raw_shelf.iter().zip(selected) {
        if selected || held.raw.document == raw.document {
            order.push(held.raw.document);
        } else {
            dropped.push(held.raw.clone());
        }
    }
    if !order.contains(&raw.document) {
        order.insert(0, raw.document);
    }
    app.raw_shelf.retain(|h| !dropped.iter().any(|r| r.document == h.raw.document));
    // Held raws were never documents; one shown from the session closes, as Cancel closes it.
    for r in dropped.iter().filter(|r| r.held.is_none()) {
        if let Some(index) = super::open::document_index(app, r.document) {
            app.session.close(index);
        }
    }
    let mut opened = Vec::new();
    let mut failed = Vec::new();
    let mut last = None;
    for id in &order {
        let (this, p, loaded) = if *id == raw.document {
            (raw.clone(), params.clone(), Some(original.clone()))
        } else {
            let Some(held) = app.raw_shelf.iter().find(|h| h.raw.document == *id) else { continue };
            let (held_raw, kept) = (held.raw.clone(), held.kept.clone());
            let (p, loaded) = match kept {
                Some((p, original)) => (p, Some(original)),
                None => first_settings(app, &held_raw),
            };
            (held_raw, p, loaded)
        };
        let r = commit_open_raw(app, &p, &this, loaded.as_ref());
        match &r {
            Ok(v) => {
                return_on_failure(app, v, &p, loaded.as_ref().unwrap_or(&CameraRaw::default()));
                opened.push(v.clone());
                app.raw_shelf.retain(|h| h.raw.document != *id);
            }
            Err(e) => failed.push(format!("{}: {e}", this.name)),
        }
        last = Some(r);
    }
    if order.len() == 1 {
        return last.unwrap_or_else(|| Err("no raw file to open".into()));
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
/// them.
pub(super) fn done(app: &mut PhotocraftApp) -> Value {
    use super::sidecar::Stored;
    keep_settings(app);
    let Some(d) = app.camera_raw.as_ref() else { return Value::Null };
    let Some(raw) = d.raw_open.clone() else { return Value::Null };
    let mut raws = vec![(raw.clone(), d.params.clone())];
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
        crate::i18n::fmt(tl!("Saved Camera Raw settings to {file}"), &[("file", &saved.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))])
    } else if stored.iter().all(|s| *s == Stored::Defaults) {
        tl!("Nothing to save: these raws are at their default settings").to_string()
    } else {
        tl!("No settings saved: these raws can't keep a sidecar here").to_string()
    };
    app.ui.status_error = saved.is_empty() && stored.contains(&Stored::Skipped);
    let saved = json!(saved);
    app.camera_raw = None;
    app.raw_shelf.clear();
    // Held raws were never documents; one shown from the session closes, as before it was kept.
    for (raw, _) in raws.iter().filter(|(r, _)| r.held.is_none()) {
        if let Some(index) = super::open::document_index(app, raw.document) {
            app.session.close(index);
        }
    }
    app.sync_views();
    json!({"done": true, "saved": saved})
}

/// The index of the raw the dialog shows in the filmstrip, when it shows one of them.
fn shown_index(app: &PhotocraftApp) -> Option<usize> {
    let d = app.camera_raw.as_ref().filter(|d| d.target == Target::OpenRaw)?;
    let shown = d.raw_open.as_ref()?.document;
    app.raw_shelf.iter().position(|h| h.raw.document == shown)
}

/// Which filmstrip raws are selected: the ones marked, and the shown one.
pub(super) fn selection(app: &PhotocraftApp) -> Vec<bool> {
    let shown = shown_index(app);
    app.raw_shelf.iter().enumerate().map(|(i, h)| h.selected || Some(i) == shown).collect()
}

/// Forgets the selection: a dialog that opens afresh shows one raw, selected alone.
pub(super) fn clear_selection(shelf: &mut [HeldRaw]) {
    for h in shelf {
        (h.selected, h.anchor) = (false, false);
    }
}

fn set_anchor(shelf: &mut [HeldRaw], at: usize) {
    for (i, h) in shelf.iter_mut().enumerate() {
        h.anchor = i == at;
    }
}

/// A click on raw `i` of the filmstrip, as in Camera Raw's: alone it selects only that raw and
/// shows it; with Shift it selects the range from the last raw clicked without Shift (with ⌘ as
/// well, adds the range); with ⌘ (Ctrl on Windows and Linux) it adds the raw and shows it, or
/// takes it out of the selection. The selection is never empty: the last selected raw stays,
/// and when the shown raw is taken out another selected one is shown.
pub(super) fn click(app: &mut PhotocraftApp, ctx: &egui::Context, i: usize, shift: bool, command: bool) -> Result<(), String> {
    if i >= app.raw_shelf.len() {
        return Err("no raw file at that place in the filmstrip".into());
    }
    let Some(shown) = shown_index(app) else { return Ok(()) };
    let selected = selection(app);
    // The shown raw is selected without a mark; marked now, it stays selected when another is shown.
    for (h, s) in app.raw_shelf.iter_mut().zip(&selected) {
        h.selected = *s;
    }
    if shift {
        let anchor = app.raw_shelf.iter().position(|h| h.anchor).unwrap_or(shown);
        let range = anchor.min(i)..=anchor.max(i);
        for (k, h) in app.raw_shelf.iter_mut().enumerate() {
            h.selected = range.contains(&k) || (command && h.selected);
        }
        set_anchor(&mut app.raw_shelf, anchor);
        return show_raw(app, ctx, i);
    }
    set_anchor(&mut app.raw_shelf, i);
    if command && selected.get(i) == Some(&true) {
        if selected.iter().filter(|s| **s).count() < 2 {
            return Ok(());
        }
        if i == shown {
            // The next selected raw after it, else the nearest before it.
            let next = (i + 1..selected.len()).chain((0..i).rev()).find(|k| selected.get(*k) == Some(&true));
            if let Some(next) = next {
                show_raw(app, ctx, next)?;
            }
        }
        if let Some(h) = app.raw_shelf.get_mut(i) {
            h.selected = false;
        }
        return Ok(());
    }
    for (k, h) in app.raw_shelf.iter_mut().enumerate() {
        h.selected = k == i || (command && h.selected);
    }
    show_raw(app, ctx, i)
}

/// The control channel's filmstrip request: `{"click": index, "shift": bool, "command": bool}`
/// clicks a raw as the pointer would, `{"selectAll": true}` is ⌘A.
pub(super) fn request(app: &mut PhotocraftApp, ctx: &egui::Context, film: &Value) -> Result<(), String> {
    let fields = film.as_object().ok_or("Camera Raw filmstrip must be an object")?;
    if shown_index(app).is_none() {
        return Err("the filmstrip is for raws being opened".into());
    }
    for (key, value) in fields {
        match key.as_str() {
            "click" => {}
            "shift" | "command" | "selectAll" if value.is_boolean() => {}
            "shift" | "command" | "selectAll" => return Err(format!("Camera Raw filmstrip {key} must be a boolean")),
            _ => return Err(format!("unknown Camera Raw filmstrip property {key}")),
        }
    }
    let flag = |k: &str| film.get(k).and_then(Value::as_bool) == Some(true);
    // Once the request is known to be good, an edit not yet synced goes to the raws selected when
    // it was made, not to the new selection.
    match (film.get("click"), flag("selectAll")) {
        (Some(_), true) => Err("Camera Raw filmstrip takes click or selectAll, not both".into()),
        (None, true) => {
            sync_selected(app);
            select_all(app);
            Ok(())
        }
        (Some(i), false) => {
            let i = i
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .filter(|i| *i < app.raw_shelf.len())
                .ok_or("filmstrip click must be the index of a raw in it")?;
            sync_selected(app);
            click(app, ctx, i, flag("shift"), flag("command"))
        }
        (None, false) => Err("Camera Raw filmstrip needs click or selectAll".into()),
    }
}

/// ⌘A in the filmstrip: every raw selected; the shown one stays shown.
pub(super) fn select_all(app: &mut PhotocraftApp) {
    for h in &mut app.raw_shelf {
        h.selected = true;
    }
}

/// Settings that belong to one photograph and never sync to the other selected raws: the grain
/// pattern's seed, the preview scale and transfer curve PhotoCraft sets from each document, and
/// Guided Upright's guides (`uprightGuides`, once Geometry lands), which are lines drawn on one
/// picture. The Upright mode itself syncs, so each raw finds its own lines. Crop, masks and spot
/// removal would join them here.
const PER_IMAGE: [&str; 4] = ["seed", "pixelScale", "encodingGamma", "uprightGuides"];

/// The settings Auto sets. With several raws selected Auto sets them on each from its own
/// pixels, as Camera Raw's does, rather than copying the shown raw's.
const AUTO_TONE: [&str; 6] = ["exposure", "contrast", "highlights", "shadows", "whites", "blacks"];

/// `target` with what changed from `old` to `new`: a slider (one band of the Color Mixer, one
/// part of a grading wheel) at a time; a point curve whole.
fn merge_changes(old: &Value, new: &Value, target: &mut Value) {
    if old == new {
        return;
    }
    match (old, new, &mut *target) {
        (Value::Object(o), Value::Object(n), Value::Object(t)) => {
            for (k, v) in n {
                if let (Some(was), Some(slot)) = (o.get(k), t.get_mut(k)) {
                    merge_changes(was, v, slot);
                }
            }
        }
        (Value::Array(o), Value::Array(n), Value::Array(t))
            if o.len() == n.len() && t.len() == n.len() && n.iter().all(Value::is_number) && o.iter().all(Value::is_number) =>
        {
            for ((was, v), slot) in o.iter().zip(n).zip(t.iter_mut()) {
                if was != v {
                    *slot = v.clone();
                }
            }
        }
        _ => *target = new.clone(),
    }
}

/// Applies the shown raw's latest edit to every other selected raw: only the settings that changed
/// since the last call, on top of each raw's own (the ones it was left with, else the ones Open
/// would give it), so its other settings, what it started from and its demosaic choice stay. A
/// raw changed so gets the edited dot, and its thumbnail is redrawn once the edit ends
/// ([`refresh_thumbnails`]). Cheap when nothing changed: one comparison.
pub(super) fn sync_selected(app: &mut PhotocraftApp) {
    let Some(d) = app.camera_raw.as_mut() else { return };
    if d.params == d.synced && !d.auto_all {
        return;
    }
    let old = std::mem::replace(&mut d.synced, d.params.clone());
    let auto = std::mem::take(&mut d.auto_all);
    let new = d.params.clone();
    let Some(shown) = shown_index(app) else { return };
    let others: Vec<usize> = selection(app).iter().enumerate().filter(|(i, s)| **s && *i != shown).map(|(i, _)| i).collect();
    if others.is_empty() {
        return;
    }
    let (Ok(Value::Object(mut old)), Ok(Value::Object(mut new))) = (serde_json::to_value(&old), serde_json::to_value(&new)) else { return };
    for k in PER_IMAGE.iter().chain(if auto { AUTO_TONE.as_slice() } else { &[] }) {
        old.remove(*k);
        new.remove(*k);
    }
    let (old, new) = (Value::Object(old), Value::Object(new));
    if old == new && !auto {
        return;
    }
    for i in others {
        let Some(held) = app.raw_shelf.get(i) else { continue };
        let raw = held.raw.clone();
        let (base, original) = match held.kept.clone() {
            Some(kept) => kept,
            None => {
                let (p, loaded) = first_settings(app, &raw);
                (p, loaded.unwrap_or_default())
            }
        };
        let Ok(mut merged) = serde_json::to_value(&base) else { continue };
        merge_changes(&old, &new, &mut merged);
        let Ok(mut next) = serde_json::from_value::<CameraRaw>(merged) else { continue };
        if auto {
            auto_tone(app, &raw, &mut next);
        }
        // A curve the shown raw takes is valid for every raw; this only guards the merge.
        if next.validate_changes(&original).is_err() {
            continue;
        }
        if next != base
            && let Some(held) = app.raw_shelf.get_mut(i)
        {
            held.stale = true;
            held.kept = Some((next, original));
        }
    }
}

/// Thumbnail redraws that build their source at once: the first redraw of a raw decodes and bins
/// the whole raw (tens of megabytes for a while), so a Select All over many raws doesn't hold them
/// all in memory together. The others wait their turn, still marked stale.
const MAX_SOURCE_BUILDS: usize = 2;

/// Redraws the thumbnails an edit synced from the shown raw left out of date, each on a worker:
/// it develops the raw binned to thumbnail size as Open develops it, which the first time decodes
/// the raw from its file (a few hundred milliseconds for a large raw). Takes in the ones that are
/// done and starts the next. `wait`: finish them here (the control channel, so its reply shows
/// them). `true` while one is still being drawn or waits, or one was just taken in, so the dialog
/// draws again until they are all shown.
pub(super) fn refresh_thumbnails(app: &mut PhotocraftApp, wait: bool) -> bool {
    let mut busy = false;
    // Twice: a raw edited again while its redraw ran starts the next one once that is in.
    for i in (0..app.raw_shelf.len()).flat_map(|i| [i, i]) {
        let Some(held) = app.raw_shelf.get(i) else { continue };
        let building = app.raw_shelf.iter().filter(|h| h.redraw.is_some() && h.source.is_none()).count();
        if held.stale && held.redraw.is_none() && held.source.is_none() && building >= MAX_SOURCE_BUILDS {
            busy = true;
        } else if held.stale && held.redraw.is_none() {
            let job = thumb_job(app, held);
            let Some(held) = app.raw_shelf.get_mut(i) else { continue };
            held.stale = false;
            match job {
                Some(job) => held.redraw = Some(spawn_thumb(job)),
                None => log::warn!("{}: no thumbnail for its synced settings", held.raw.name),
            }
        }
        let Some(held) = app.raw_shelf.get_mut(i) else { continue };
        let Some(rx) = &held.redraw else { continue };
        let got = if wait { rx.recv().map_err(|_| std::sync::mpsc::TryRecvError::Disconnected) } else { rx.try_recv() };
        match got {
            Ok(done) => {
                held.redraw = None;
                busy = true;
                let discard = std::mem::take(&mut held.discard_redraw);
                if let Some((source, image)) = done {
                    held.source = Some(source);
                    if !discard {
                        held.thumb = Some(image);
                        held.tex = None;
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => busy = true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => (held.redraw, held.discard_redraw) = (None, false),
        }
    }
    busy
}

/// Whether a filmstrip thumbnail still waits to be redrawn: another raw's, or the shown raw's
/// while its preview hasn't caught up with an edit.
pub(super) fn thumbnails_pending(app: &PhotocraftApp) -> bool {
    let shown_behind =
        shown_index(app).is_some() && app.camera_raw.as_ref().is_some_and(|d| d.thumb_revision != d.preview_revision || d.dirty || d.render_pending());
    shown_behind || app.raw_shelf.iter().any(|h| h.stale || h.redraw.is_some())
}

/// The shown raw's thumbnail follows its edits, as Camera Raw's does: from the dialog's own
/// preview once it has caught up with the edit (no develop of its own), and only when the
/// preview has changed since. `true` when it was redrawn (its texture loads on the next frame).
pub(super) fn refresh_shown_thumbnail(app: &mut PhotocraftApp) -> bool {
    let Some(i) = shown_index(app) else { return false };
    let Some(d) = app.camera_raw.as_mut() else { return false };
    if d.dirty || d.render_pending() || d.thumb_revision == d.preview_revision || d.processed.is_empty() {
        return false;
    }
    d.thumb_revision = d.preview_revision;
    let Some(held) = app.raw_shelf.get_mut(i) else { return false };
    let (px, w, h) = shrink(&d.processed, d.pw, d.ph);
    held.thumb = Some(CameraRawDialog::image(&px, w, h, &d.display));
    // An older redraw still on a worker (from before it was shown) must not land over it.
    (held.discard_redraw, held.stale) = (held.redraw.is_some(), false);
    held.tex = None;
    true
}

/// Everything one thumbnail redraw reads, owned, so it can run off the UI thread.
#[derive(Clone)]
struct ThumbJob {
    params: CameraRaw,
    source: Result<std::sync::Arc<ThumbSource>, ThumbBuild>,
    display: std::sync::Arc<photocraft_engine::display_color::CanvasDisplay>,
}

/// What a thumbnail's first redraw builds its [`ThumbSource`] from.
#[derive(Clone)]
struct ThumbBuild {
    surf: photocraft_raster::Surface,
    bounds: photocraft_geom::Rect,
    raw: RawOpen,
    encoding_gamma: f32,
    /// ProPhoto to the document's colours ([`crate::camera_raw_detail_ui::RawDetail::to_document`]).
    to_document: Option<std::sync::Arc<photocraft_cms::Transform>>,
}

fn thumb_job(app: &PhotocraftApp, held: &HeldRaw) -> Option<ThumbJob> {
    let (params, _) = held.kept.clone()?;
    // A held raw: not a document, so not in the session.
    let doc = super::open::raw_document(app, &held.raw)?;
    let display = app.session.color.canvas_display(&doc).ok()?;
    let source = match &held.source {
        Some(source) => Ok(source.clone()),
        None => {
            let surf = doc.layers.iter().find_map(|l| l.surface())?.clone();
            Err(ThumbBuild {
                surf,
                bounds: doc.bounds(),
                raw: held.raw.clone(),
                encoding_gamma: photocraft_engine::lens_cmds::encoding_gamma(&doc),
                to_document: super::convert::raw_to_document(app, &doc),
            })
        }
    };
    Some(ThumbJob { params, source, display })
}

fn spawn_thumb(job: ThumbJob) -> ThumbRedraw {
    let (tx, rx) = std::sync::mpsc::channel();
    let draw = |job: ThumbJob, tx: std::sync::mpsc::Sender<_>| {
        let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| draw_thumb(job))).ok().flatten();
        let _ = tx.send(drawn);
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (worker_job, worker_tx) = (job.clone(), tx.clone());
        if let Err(e) = std::thread::Builder::new().name("camera-raw-thumbnail".into()).spawn(move || draw(worker_job, worker_tx)) {
            // No thread to spare: drawn here rather than left out of date.
            log::warn!("no thread for a Camera Raw thumbnail, drawing it here: {e}");
            draw(job, tx);
        }
    }
    #[cfg(target_arch = "wasm32")]
    draw(job, tx);
    rx
}

fn draw_thumb(job: ThumbJob) -> Option<(std::sync::Arc<ThumbSource>, egui::ColorImage)> {
    let source = match job.source {
        Ok(source) => source,
        Err(build) => std::sync::Arc::new(thumb_source(&build)?),
    };
    let mut params = job.params;
    if params.encoding_gamma == 0.0 {
        params.encoding_gamma = source.encoding_gamma;
    }
    let frame = super::ProxyJob {
        proxy: source.px.clone(),
        selection: None,
        raw_curve: source.curve.clone(),
        raw_source: source.raw.clone(),
        params,
        float: source.float,
        size: source.size,
        full_w: source.full_w,
        display: job.display,
        gpu: None,
        source_key: 0,
    }
    .run();
    Some((source, frame.image))
}

/// The filmstrip as the control channel describes it: each raw's name, whether it is selected,
/// shown, edited, and whether its thumbnail still waits for an edit synced to it.
pub(crate) fn describe(app: &PhotocraftApp) -> Value {
    let shown = shown_index(app);
    // The shown raw's settings are the dialog's until it is left.
    let live = app.camera_raw.as_ref().map(|d| &d.params);
    let shown_behind = app.camera_raw.as_ref().is_some_and(|d| d.thumb_revision != d.preview_revision || d.dirty || d.render_pending());
    json!(
        app.raw_shelf
            .iter()
            .zip(selection(app))
            .enumerate()
            .map(|(i, (h, selected))| json!({
                "name": h.raw.name,
                "selected": selected,
                "shown": Some(i) == shown,
                "edited": if Some(i) == shown { live.is_some_and(|p| !p.is_identity()) } else { h.kept.as_ref().is_some_and(|(p, _)| !p.is_identity()) },
                "thumbnailPending": h.stale || (h.redraw.is_some() && !h.discard_redraw) || (Some(i) == shown && shown_behind),
            }))
            .collect::<Vec<_>>()
    )
}

/// [`ThumbSource`] of a filmstrip raw: its developed pixels at thumbnail size, and the camera tone
/// curve the dialog would read from its file.
fn thumb_source(build: &ThumbBuild) -> Option<ThumbSource> {
    let ThumbBuild { surf, bounds, raw, encoding_gamma, to_document } = build;
    let (bounds, encoding_gamma) = (*bounds, *encoding_gamma);
    let (area, w, h) = super::convert::preview_area(bounds, surf).ok()?;
    // The raw binned straight to the thumbnail's size, as the dialog's proxy is to its own.
    let k = w.max(h).div_ceil(THUMB_SIDE).max(1);
    let (tw, th) = (w.div_ceil(k), h.div_ceil(k));
    let bytes = match (&raw.bytes, &raw.path) {
        (Some(b), _) => Some(b.as_ref().clone()),
        #[cfg(not(target_arch = "wasm32"))]
        (None, Some(p)) => std::fs::read(p).ok(),
        _ => None,
    };
    let source = bytes.and_then(|b| photocraft_io::raw::RawSource::proxy_only(&b, k).ok()).map(|s| crate::camera_raw_detail_ui::RawDetail {
        source: std::sync::Arc::new(s),
        demosaic: raw.developed_with,
        to_document: to_document.clone(),
    });
    let as_opened = photocraft_io::raw::RawTuning { demosaic: raw.developed_with, ..Default::default() };
    let developed = source.as_ref().and_then(|s| super::convert::raw_proxy(s, &as_opened, tw, th));
    let (px, size, raw_source, curve) = match (developed, source) {
        (Some((px, _)), Some(s)) => {
            let curve = std::sync::Arc::new(s.source.tone_curve().to_vec());
            (px, (tw, th), Some(s), Some(curve))
        }
        _ => {
            let (proxy, pw, ph, _) = super::convert::box_proxy(surf, area, w, h);
            let (px, tw, th) = shrink(&proxy, pw, ph);
            (px, (tw, th), None, None)
        }
    };
    Some(ThumbSource {
        px: std::sync::Arc::new(px),
        size,
        full_w: w,
        float: surf.format().sample == photocraft_color::SampleType::F32,
        curve,
        raw: raw_source,
        encoding_gamma,
    })
}

/// What a click in the filmstrip asks for.
pub(super) enum FilmClick {
    /// Raw `.0`, with Shift (`.1`) or ⌘ (`.2`) held.
    Tile(usize, bool, bool),
    /// Select All (the filmstrip's context menu).
    SelectAll,
}

/// The filmstrip of raws being opened: a thumbnail and name each, the shown one outlined, the
/// other selected ones highlighted, a dot on edited ones. Returns what was clicked.
pub(super) fn filmstrip(ui: &mut egui::Ui, rect: ERect, shelf: &mut [HeldRaw], current: Option<photocraft_doc::DocId>, t: &Tokens) -> Option<FilmClick> {
    let mut clicked = None;
    let mut film = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 6.0))));
    film.set_clip_rect(rect.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical().id_salt("camera-raw-filmstrip").auto_shrink([false, false]).show(&mut film, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let w = ui.available_width();
        for (i, held) in shelf.iter_mut().enumerate() {
            if held.tex.is_none()
                && let Some(img) = held.thumb.take()
            {
                held.tex = Some(ui.ctx().load_texture(format!("camera-raw-thumb-{}", held.raw.document.0), img, egui::TextureOptions::LINEAR));
            }
            let (tile, response) = ui.allocate_exact_size(vec2(w, w * 0.75 + 18.0), Sense::click());
            let frame = ERect::from_min_size(tile.min, vec2(w, w * 0.75));
            let shown = Some(held.raw.document) == current;
            let selected = held.selected || shown;
            let p = ui.painter();
            // Selected: a highlighted tile (thumbnail and name); the shown raw is outlined as well.
            p.rect_filled(tile, 2.0, if selected { t.row_selected } else { Color32::TRANSPARENT });
            p.rect_filled(frame, 2.0, if response.hovered() { t.hover } else { t.canvas });
            if let Some(tex) = &held.tex {
                let size = tex.size_vec2();
                let fit = ((frame.width() - 8.0) / size.x.max(1.0)).min((frame.height() - 8.0) / size.y.max(1.0));
                let r = ERect::from_center_size(frame.center(), size * fit);
                widgets::checker(p, r, 6.0);
                p.image(tex.id(), r, ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            if shown {
                p.rect_stroke(frame, 2.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
            }
            if held.kept.as_ref().is_some_and(|(p, _)| !p.is_identity()) {
                p.circle_filled(frame.right_top() + vec2(-7.0, 7.0), 3.0, t.text);
            }
            let name = ERect::from_min_max(pos2(tile.left(), frame.bottom()), tile.max);
            let font = FontId::proportional(10.5);
            let label = tail_fitting(p, &held.raw.name, font.clone(), name.width());
            p.with_clip_rect(name).text(name.center(), Align2::CENTER_CENTER, label, font, t.text_dim);
            let response = response.on_hover_text(&held.raw.name);
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &held.raw.name));
            if response.clicked() {
                let m = ui.input(|i| i.modifiers);
                clicked = Some(FilmClick::Tile(i, m.shift, m.command));
            }
            response.context_menu(|ui| {
                if ui.button(tl!("Select All")).clicked() {
                    clicked = Some(FilmClick::SelectAll);
                    ui.close();
                }
            });
        }
    });
    clicked
}

/// `name`, or "…" and as much of its end as fits in `width`: raws from one shoot differ at the
/// end of their names (the frame number).
fn tail_fitting(p: &egui::Painter, name: &str, font: FontId, width: f32) -> String {
    let fits = |s: &str| p.layout_no_wrap(s.to_owned(), font.clone(), Color32::WHITE).size().x <= width;
    if fits(name) {
        return name.to_owned();
    }
    let chars: Vec<char> = name.chars().collect();
    (1..chars.len()).map(|start| format!("…{}", chars[start..].iter().collect::<String>())).find(|s| fits(s)).unwrap_or_default()
}
