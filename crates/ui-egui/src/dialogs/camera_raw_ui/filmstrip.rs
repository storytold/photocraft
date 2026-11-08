//! The open-time dialog's filmstrip: raws opened together wait side by side in one Camera Raw
//! dialog, as in Photoshop's. Each keeps its own settings and demosaic choice when another is
//! shown, Open develops every one and Cancel closes every one.
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

/// A raw waiting in the open-time dialog: opened (developed with the defaults) and reviewed
/// before Open keeps it.
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
    source: Option<ThumbSource>,
}

/// A filmstrip raw's pixels at thumbnail size and its camera tone curve, to develop its
/// thumbnail with settings it was given without being shown.
struct ThumbSource {
    px: std::sync::Arc<Vec<[f32; 4]>>,
    size: (usize, usize),
    full_w: usize,
    float: bool,
    curve: Option<std::sync::Arc<Vec<[f32; 2]>>>,
    encoding_gamma: f32,
}

impl HeldRaw {
    pub(super) fn new(app: &PhotocraftApp, raw: RawOpen) -> Self {
        let thumb = super::open::document_index(app, raw.document).and_then(|i| thumbnail(app, i));
        HeldRaw { raw, kept: None, thumb, tex: None, selected: false, anchor: false, stale: false, source: None }
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

/// Camera Raw's Auto for a filmstrip raw, from the proxy the dialog would use, so showing a raw
/// doesn't change what Auto gives it.
pub(super) fn auto_tone(app: &PhotocraftApp, raw: &RawOpen, p: &mut CameraRaw) {
    if let Some(st) = super::open::document_index(app, raw.document).and_then(|i| app.session.documents().get(i))
        && let Some(surf) = st.doc.layers.iter().find_map(|l| l.surface())
        && let Ok((area, w, h)) = super::convert::preview_area(st.doc.bounds(), surf)
    {
        let (proxy, ..) = super::convert::box_proxy(surf, area, w, h);
        photocraft_algo::camera_raw::auto_tone(&proxy, p);
    }
}

/// Open with raws in the filmstrip: the shown one as the dialog has it, then every other with
/// its own settings. One reply for one raw, `{"opened": [...]}` for several. A raw leaves the
/// filmstrip once it has opened or its background develop has started: one that fails stays or
/// comes back there with its settings, so the dialog returns for it (or Cancel still closes it).
pub(super) fn open_all(app: &mut PhotocraftApp, params: &CameraRaw, raw: &RawOpen, original: &CameraRaw) -> Result<Value, String> {
    keep_settings(app);
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
/// them (they close again).
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
    for (raw, _) in &raws {
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
    match (film.get("click"), flag("selectAll")) {
        (Some(_), true) => Err("Camera Raw filmstrip takes click or selectAll, not both".into()),
        (None, true) => {
            select_all(app);
            Ok(())
        }
        (Some(i), false) => {
            let i = i
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .filter(|i| *i < app.raw_shelf.len())
                .ok_or("filmstrip click must be the index of a raw in it")?;
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
/// pattern's seed, and the preview scale and transfer curve PhotoCraft sets from each document.
/// Camera Raw's Geometry (Upright, guides), crop, masks and spot removal would join them here;
/// these settings have none of them yet.
const PER_IMAGE: [&str; 3] = ["seed", "pixelScale", "encodingGamma"];

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

/// Redraws up to `limit` thumbnails that an edit synced from the shown raw left out of date.
/// `true` when one was redrawn or more are left. Each develops a thumbnail-sized copy of its raw,
/// as the preview does, on this thread.
pub(super) fn refresh_thumbnails(app: &mut PhotocraftApp, limit: usize) -> bool {
    let mut redrawn = 0;
    for i in 0..app.raw_shelf.len() {
        let Some(held) = app.raw_shelf.get(i).filter(|h| h.stale) else { continue };
        if redrawn >= limit {
            return true;
        }
        redrawn += 1;
        let source = if held.source.is_none() { thumb_source(app, &held.raw) } else { None };
        let display = super::open::document_index(app, held.raw.document)
            .and_then(|k| app.session.documents().get(k))
            .and_then(|st| app.session.color.canvas_display(&st.doc).ok());
        let Some(held) = app.raw_shelf.get_mut(i) else { continue };
        held.stale = false;
        if source.is_some() {
            held.source = source;
        }
        let (Some(src), Some(display), Some((params, _))) = (&held.source, display, &held.kept) else { continue };
        let mut params = params.clone();
        if params.encoding_gamma == 0.0 {
            params.encoding_gamma = src.encoding_gamma;
        }
        let job = super::ProxyJob {
            proxy: src.px.clone(),
            selection: None,
            raw_curve: src.curve.clone(),
            params,
            float: src.float,
            size: src.size,
            full_w: src.full_w,
            display,
            gpu: None,
            source_key: 0,
        };
        held.thumb = Some(job.run().image);
        held.tex = None;
    }
    // A redrawn thumbnail's texture loads on the next frame.
    redrawn > 0
}

/// The filmstrip as the control channel describes it: each raw's name, whether it is selected,
/// shown, edited, and whether its thumbnail still waits for an edit synced to it.
pub(crate) fn describe(app: &PhotocraftApp) -> Value {
    let shown = shown_index(app);
    // The shown raw's settings are the dialog's until it is left.
    let live = app.camera_raw.as_ref().map(|d| &d.params);
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
                "thumbnailPending": h.stale,
            }))
            .collect::<Vec<_>>()
    )
}

/// [`ThumbSource`] of a filmstrip raw: its developed pixels at thumbnail size, and the camera tone
/// curve the dialog would read from its file.
fn thumb_source(app: &PhotocraftApp, raw: &RawOpen) -> Option<ThumbSource> {
    let st = app.session.documents().get(super::open::document_index(app, raw.document)?)?;
    let surf = st.doc.layers.iter().find_map(|l| l.surface())?;
    let (area, w, h) = super::convert::preview_area(st.doc.bounds(), surf).ok()?;
    let (proxy, pw, ph, _) = super::convert::box_proxy(surf, area, w, h);
    let (px, tw, th) = shrink(&proxy, pw, ph);
    Some(ThumbSource {
        px: std::sync::Arc::new(px),
        size: (tw, th),
        full_w: w,
        float: surf.format().sample == photocraft_color::SampleType::F32,
        curve: super::open::raw_tone_curve(raw).map(std::sync::Arc::new),
        encoding_gamma: photocraft_engine::lens_cmds::encoding_gamma(&st.doc),
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
