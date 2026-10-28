use super::{Arc, DialogKind, Map, PhotocraftApp, Tokens, Value, WebSettings, b, check, dropdown_str, json, n, number, params, s, web_cmds, web_preview};

const WEB_FORMATS: [(&str, &str); 5] = [("gif", "GIF"), ("png8", "PNG-8"), ("png24", "PNG-24"), ("jpeg", "JPEG"), ("wbmp", "WBMP")];

pub(super) const WEB_COMMAND: &str = "file.export.saveForWebLegacy";

/// Save for Web's preview tabs: the `__view` field's values and their labels.
const WEB_VIEWS: [(&str, &str); 4] = [("original", "Original"), ("optimized", "Optimized"), ("2up", "2-Up"), ("4up", "4-Up")];

pub fn open_web(app: &mut PhotocraftApp) -> u64 {
    let mut f = Map::new();
    f.insert("__web".into(), json!(true));
    f.insert("__command".into(), json!(WEB_COMMAND));
    f.insert("__label".into(), json!("Save for Web (Legacy)"));
    // The preview tab chosen last time, 2-Up at first (#9174).
    let view = app.ui.web_view.as_deref().filter(|v| WEB_VIEWS.iter().any(|(k, _)| k == v)).unwrap_or("2up");
    f.insert("__view".into(), json!(view));
    // Remember encoding choices, but each document starts at its own full size. This session's
    // last export lives in `file_menu`; the last one before a restart comes from the
    // preferences' remembered dialog settings (`web_cmds::remember_web_settings`).
    let last = app
        .session
        .file_menu
        .last_web
        .clone()
        .or_else(|| app.session.prefs().dialogs.get(WEB_COMMAND).cloned())
        .unwrap_or_else(|| json!({"format": "jpeg", "quality": 60}));
    if let Value::Object(m) = last {
        for (k, v) in m {
            if !matches!(k.as_str(), "path" | "dir" | "width" | "height" | "percent" | "colorTable" | "colorTableEdits") {
                f.insert(k, v);
            }
        }
    }
    for (k, v) in [
        ("format", json!("jpeg")),
        ("quality", json!(60)),
        ("palette", json!("selective")),
        ("colors", json!(128)),
        ("dither", json!("diffusion")),
        ("ditherAmount", json!(88)),
        ("transparency", json!(true)),
        ("matte", json!("#ffffff")),
        ("interlaced", json!(false)),
        ("progressive", json!(false)),
        ("optimized", json!(true)),
        ("embedIcc", json!(false)),
        ("convertToSrgb", json!(true)),
        ("metadata", json!("copyright")),
        ("percent", json!(100.0)),
        ("resample", json!("bicubic")),
        ("webSnap", json!(0)),
    ] {
        f.entry(k.to_string()).or_insert(v);
    }
    app.ui.open_dialog(DialogKind::Command, f)
}

/// Settings variants shown in 4-Up (current, then two smaller alternatives).
fn variants(p: &Value) -> Vec<Value> {
    let mut out = vec![p.clone()];
    let fmt = p["format"].as_str().unwrap_or("jpeg");
    for k in [2.0, 4.0] {
        let mut v = p.clone();
        if let Some(m) = v.as_object_mut() {
            m.remove("colorTable");
            m.remove("colorTableEdits");
        }
        if fmt == "jpeg" {
            v["quality"] = json!((p["quality"].as_f64().unwrap_or(60.0) / k).round().max(1.0));
        } else if fmt == "gif" || fmt == "png8" {
            v["colors"] = json!((p["colors"].as_f64().unwrap_or(128.0) / k).round().max(2.0));
        } else {
            v["format"] = json!(if k == 2.0 { "png8" } else { "gif" });
            v["colors"] = json!(256 / k as u32);
        }
        out.push(v);
    }
    out
}

/// The flattened preview image: pixels, width, height, full-size / proxy pixel ratio.
pub(super) type WebProxy = (Arc<Vec<[f32; 4]>>, u32, u32, f64, egui::Vec2);

pub(super) type WebCache = (String, Arc<egui::TextureHandle>, Option<(usize, Option<usize>, &'static str)>);

/// The longest side of the proxy a still image is previewed with, and an animation (each frame of which
/// is prepared as it plays, and sized while the dialog is open, on the UI thread).
const STILL_PROXY: f64 = 2048.0;
pub(super) const ANIMATION_PROXY: f64 = 1024.0;

/// The bounded proxy a preview works on: `frame` of `doc` at the export's size, scaled down to at most
/// `longest_side` px, flattened.
pub(super) fn web_proxy(doc: &photocraft_doc::Document, base: &Value, frame: Option<usize>, longest_side: f64) -> Result<WebProxy, String> {
    let (fw, fh) = web_preview::dimensions(doc, base)?;
    let k = (longest_side / fw.max(fh)).min(1.0);
    let st = WebSettings::from_params(base, "preview").unwrap_or_default();
    let shown = web_frame_document(doc, frame).map_err(|e| e.to_string())?;
    let resample = web_preview::proxy_resample(base, k);
    let (wd, _, _) =
        web_cmds::web_document(&shown, &json!({"width": (fw * k).round().max(1.0), "height": (fh * k).round().max(1.0), "resample": resample}), &st)
            .map_err(|e| e.to_string())?;
    let buf = photocraft_compose::flatten(&wd);
    Ok((
        Arc::new(buf.px),
        wd.size.width,
        wd.size.height,
        (fw * fh) / (f64::from(wd.size.width) * f64::from(wd.size.height)).max(1.0),
        egui::vec2(fw as f32, fh as f32),
    ))
}

/// The sizes of an animation's frames as GIFs, found one frame to a repaint so the dialog stays responsive.
#[derive(Clone)]
struct FrameSizes {
    signature: String,
    sizes: Vec<Option<usize>>,
}

#[allow(clippy::too_many_arguments)]
/// What an animated GIF of `count` frames comes to, whichever frame is on show: the sum of the frames' own
/// sizes. While frames remain to be sized it is their average times the count, and `false` says so.
fn animation_bytes(
    ui: &mut egui::Ui,
    doc: &Arc<photocraft_doc::Document>,
    rev: u64,
    p: &Value,
    key: egui::Id,
    count: usize,
    playing: bool,
    shown: usize,
) -> (usize, bool) {
    let mut base = p.clone();
    if let Some(m) = base.as_object_mut() {
        m.remove("colorTableEdits");
    }
    let signature = format!("{}|{rev}|{base}", doc.id.0);
    let mut found: FrameSizes = ui
        .data(|d| d.get_temp::<FrameSizes>(key))
        .filter(|c| c.signature == signature && c.sizes.len() == count)
        .unwrap_or(FrameSizes { signature, sizes: vec![None; count] });
    // Playing already prepares a frame at every step: sizing waits until it is paused.
    let next = found.sizes.iter().position(Option::is_none).filter(|_| !playing);
    if let Some(i) = next {
        let bytes = web_proxy(doc, &base, Some(i), ANIMATION_PROXY).ok().and_then(|(px, w, h, ratio, _)| {
            let st = WebSettings::from_params(&base, "preview").ok()?;
            let o = web_cmds::optimize(&px, w as usize, photocraft_geom::Rect::new(0, 0, w as i32, h as i32), &st, None, None, None, 72.0, false).ok()?;
            Some((o.bytes.len() as f64 * ratio) as usize)
        });
        found.sizes[i] = Some(bytes.unwrap_or(0));
        ui.ctx().request_repaint();
    }
    ui.data_mut(|d| d.insert_temp(key, found.clone()));
    let known: Vec<usize> = found.sizes.iter().flatten().copied().collect();
    let sum: usize = known.iter().sum();
    let done = known.len() == count;
    // Nothing sized yet (it was playing from the start): the shown frame's size for each stands in.
    let total = if done {
        sum
    } else if known.is_empty() {
        shown.saturating_mul(count)
    } else {
        sum / known.len() * count
    };
    (total, done)
}

/// Renders one preview pane: the original (`None`) or an optimised variant.
fn web_pane(app: &PhotocraftApp, ui: &mut egui::Ui, size: egui::Vec2, p: Option<&Value>, f: &mut Map<String, Value>, pane: usize, frame: Option<usize>) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else { return };
    let doc = st.doc.clone();
    let rev = st.revision;
    let playing = b(f, "__web_playing", false);
    // A bounded proxy; navigation never recomposites or re-encodes it.
    let base = p.cloned().unwrap_or_else(|| json!({}));
    let size_sig = format!("{}|{}|{}|{}|{}", base["percent"], base["width"], base["height"], base["resample"], base["convertToSrgb"]);
    let wkey = egui::Id::unique(("web-proxy", doc.id.0, pane));
    let proxy_sig = format!("{rev}|{frame:?}|{size_sig}");
    let cached_proxy: Option<(String, WebProxy)> = ui.data(|d| d.get_temp(wkey));
    let proxy = cached_proxy.filter(|(sig, _)| sig == &proxy_sig).map(|(_, proxy)| proxy);
    let (px, w, h, ratio, source_size) = match proxy {
        Some(v) => v,
        None => {
            let v = match web_proxy(&doc, &base, frame, if animation_frames(&doc).is_some() { ANIMATION_PROXY } else { STILL_PROXY }) {
                Ok(v) => v,
                Err(error) => {
                    ui.label(error);
                    return;
                }
            };
            ui.data_mut(|d| d.insert_temp(wkey, (proxy_sig, v.clone())));
            v
        }
    };
    let sig = format!("{}|{rev}|{frame:?}|{size_sig}|{}", doc.id.0, p.map(|v| v.to_string()).unwrap_or_else(|| "original".into()));
    let key = egui::Id::unique(("web-pane", doc.id.0, pane));
    let cached: Option<WebCache> = ui.data(|d| d.get_temp(key));
    let (tex, info) = match cached.filter(|c| c.0 == sig) {
        Some((_, tex, info)) => (tex, info),
        None => {
            let (rgba, info) = match p {
                None => (px.iter().flat_map(|q| q.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect::<Vec<u8>>(), None),
                Some(p) => match web_preview::optimized(ui, key, &format!("{}|{rev}|{frame:?}|{size_sig}", doc.id.0), p, &px, w, h) {
                    Ok((preview, bytes, colors, ext)) => (preview, Some(((bytes as f64 * ratio) as usize, colors, ext))),
                    Err(error) => {
                        ui.label(error);
                        (vec![0; (w * h * 4) as usize], None)
                    }
                },
            };
            let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            let tex = Arc::new(ui.ctx().load_texture("web-pane", img, egui::TextureOptions::LINEAR));
            ui.data_mut(|d| d.insert_temp(key, (sig, tex.clone(), info)));
            (tex, info)
        }
    };
    ui.vertical(|ui| {
        ui.set_width(size.x);
        let (r, response) = ui.allocate_exact_size((size - egui::vec2(0.0, 42.0)).max(egui::vec2(1.0, 1.0)), egui::Sense::drag());
        let response = response.on_hover_cursor(egui::CursorIcon::Grab);
        if let Some(rects) = f.get_mut("__previewRects").and_then(Value::as_array_mut) {
            rects.push(json!([r.left(), r.top(), r.right(), r.bottom()]));
        }
        crate::widgets::checker(ui.painter(), r, 8.0);
        web_preview::paint(ui, r, &response, &tex, source_size, f);
        ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
        let frames = animation_frames(&doc);
        let count_label = |n: usize| crate::i18n::fmt(tl!("{n} frames"), &[("n", &n.to_string())]);
        let (l1, l2) = match (p, info) {
            (None, _) => (
                "Original".to_string(),
                format!(
                    "{}  ·  {} × {} px{}",
                    doc.name,
                    doc.size.width,
                    doc.size.height,
                    frames.map(|n| format!("  ·  {}", count_label(n))).unwrap_or_default()
                ),
            ),
            (Some(_), Some((bytes, colors, ext))) => {
                // Only a GIF keeps the frames: its size is all of theirs, the same whichever frame is shown.
                let frames = frames.filter(|_| ext == "gif");
                let (bytes, whole) = match (frames, p) {
                    (Some(n), Some(p)) => animation_bytes(ui, &doc, rev, p, key.with("animation-size"), n, playing, bytes),
                    _ => (bytes, true),
                };
                // Photoshop's download estimate at 56.6 Kbps.
                let secs = (bytes as f64 * 8.0 / 56_600.0).ceil();
                (
                    format!("{}  {}{}", ext.to_uppercase(), if whole { "" } else { "~" }, crate::sizing::human_bytes(bytes as f64)),
                    format!(
                        "{secs} sec @ 56.6 Kbps{}{}",
                        colors.map(|c| format!("  ·  {c} colors")).unwrap_or_default(),
                        frames.map(|n| format!("  ·  {}", count_label(n))).unwrap_or_default()
                    ),
                )
            }
            (Some(_), None) => ("—".into(), String::new()),
        };
        let l1 = if ratio > 1.001 { format!("{l1}  ·  Proxy preview") } else { l1 };
        ui.add(egui::Label::new(egui::RichText::new(l1).color(t.text).size(11.5)).truncate());
        ui.add(egui::Label::new(egui::RichText::new(l2).color(t.text_dim).size(11.0)).truncate());
    });
}

/// Advance a preview clock by whole elapsed frames, retaining fractional time.
/// The caller resets `last` on Play so paused wall time never advances the preview.
pub(super) fn advance_preview(frame: usize, duration: usize, fps: f32, last: f64, now: f64) -> (usize, f64) {
    let duration = duration.max(1);
    let frame = frame.min(duration - 1);
    if !now.is_finite() {
        return (frame, last);
    }
    if !last.is_finite() || now < last {
        return (frame, now);
    }
    let fps = if fps.is_finite() && fps > 0.0 { f64::from(fps) } else { 30.0 };
    let elapsed_frames = (now - last) * fps;
    if !elapsed_frames.is_finite() {
        return (frame, now);
    }
    let steps = elapsed_frames.floor();
    if steps < 1.0 {
        return (frame, last);
    }
    let delta = (steps % duration as f64) as usize % duration;
    let remaining = duration - frame;
    let next = if delta >= remaining { delta - remaining } else { frame + delta };
    (next, now - (elapsed_frames - steps) / fps)
}

/// How many frames `doc`'s frame animation has, when it has more than one (a GIF of it is animated).
pub(super) fn animation_frames(doc: &photocraft_doc::Document) -> Option<usize> {
    doc.frame_animation.as_ref().map(|a| a.frames.len()).filter(|n| *n > 1)
}

/// The frame a frame animation is on `now` when it was on `frame` at `last`: each frame holds for its own
/// delay (a delay of 10 ms or less plays as 100 ms, as browsers do), and the animation loops. Returns the
/// time the frame began, keeping the fraction of a delay that has passed.
pub(super) fn advance_frames(frame: usize, delays_ms: &[u32], last: f64, now: f64) -> (usize, f64) {
    let n = delays_ms.len().max(1);
    let mut frame = frame.min(n - 1);
    if !now.is_finite() || !last.is_finite() || now < last {
        return (frame, if now.is_finite() { now } else { last });
    }
    let hold = |i: usize| match delays_ms.get(i).copied().unwrap_or(100) {
        0..=10 => 0.1,
        ms => f64::from(ms) / 1000.0,
    };
    let total: f64 = (0..n).map(hold).sum();
    let mut elapsed = (now - last) % total;
    // Under one pass of the animation: each step takes at least a hundredth of a second.
    while elapsed >= hold(frame) {
        elapsed -= hold(frame);
        frame = (frame + 1) % n;
    }
    (frame, now - elapsed)
}

/// What the playback bar below the previews shows.
struct PlaybackBar {
    /// The frame being previewed (0-based) and how many there are.
    frame: usize,
    count: usize,
    playing: bool,
    /// Names the frame count, as a frame animation (a GIF of it is animated) does.
    names_count: bool,
}

/// Moves a frame animation's preview on to the frame it is on now. Returns that frame and the bar to draw.
fn web_frame_playback(ui: &mut egui::Ui, f: &mut Map<String, Value>, doc: &photocraft_doc::Document) -> (Option<usize>, Option<PlaybackBar>) {
    let Some(a) = doc.frame_animation.as_ref().filter(|a| a.frames.len() > 1) else { return (None, None) };
    let delays: Vec<u32> = a.frames.iter().map(|fr| fr.delay_ms).collect();
    let count = delays.len();
    let now = ui.input(|i| i.time);
    let mut frame = f.get("__web_frame").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(a.current).min(count - 1);
    let mut last = n(f, "__web_time", now);
    let playing = b(f, "__web_playing", false);
    if playing {
        (frame, last) = advance_frames(frame, &delays, last, now);
        let hold = match delays.get(frame).copied().unwrap_or(100) {
            0..=10 => 0.1,
            ms => f64::from(ms) / 1000.0,
        };
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((hold - (now - last)).clamp(0.0, 1.0)));
    }
    f.insert("__web_frame".into(), json!(frame));
    f.insert("__web_time".into(), json!(last));
    f.insert("__web_playing".into(), json!(playing));
    (Some(frame), Some(PlaybackBar { frame, count, playing, names_count: true }))
}

/// Draws the playback bar: Play or Pause, and where the preview is. Pressing the button restarts the
/// preview's clock, so the time spent paused never counts.
fn web_playback_bar(ui: &mut egui::Ui, f: &mut Map<String, Value>, bar: &PlaybackBar) {
    ui.horizontal(|ui| {
        let label = if bar.playing { tl!("Pause") } else { tl!("Play") };
        if crate::icons::button(ui, if bar.playing { "pause" } else { "play" }, 22.0, bar.playing, label).clicked() {
            f.insert("__web_playing".into(), json!(!bar.playing));
            f.insert("__web_time".into(), json!(ui.input(|i| i.time)));
        }
        ui.label(format!("{} / {}", bar.frame + 1, bar.count));
        if bar.names_count {
            ui.label(crate::i18n::fmt(tl!("{n} frames"), &[("n", &bar.count.to_string())]));
        }
    });
}

fn web_playback(ui: &mut egui::Ui, f: &mut Map<String, Value>, doc: &photocraft_doc::Document) -> (Option<usize>, Option<PlaybackBar>) {
    if animation_frames(doc).is_some() {
        return web_frame_playback(ui, f, doc);
    }
    let Some(mut timeline) = doc.timeline.clone() else { return (None, None) };
    timeline.clamp();
    if timeline.duration < 2 {
        return (None, None);
    }
    let now = ui.input(|i| i.time);
    let mut frame = f.get("__web_frame").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(timeline.current).min(timeline.duration - 1);
    let mut last = n(f, "__web_time", now);
    let playing = b(f, "__web_playing", false);
    if playing {
        (frame, last) = advance_preview(frame, timeline.duration, timeline.fps, last, now);
        let remaining = (1.0 / f64::from(timeline.fps) - (now - last)).clamp(0.0, 1.0);
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(remaining));
    }
    f.insert("__web_frame".into(), json!(frame));
    f.insert("__web_time".into(), json!(last));
    f.insert("__web_playing".into(), json!(playing));
    (Some(frame), Some(PlaybackBar { frame, count: timeline.duration, playing, names_count: false }))
}

/// Seek a preview copy, preserving any edits to the source's currently displayed frame.
pub(super) fn web_frame_document(doc: &photocraft_doc::Document, frame: Option<usize>) -> Result<photocraft_doc::Document, photocraft_engine::EngineError> {
    let Some(frame) = frame else { return Ok(doc.clone()) };
    if animation_frames(doc).is_some() {
        let mut shown = doc.clone();
        shown.select_animation_frame(frame);
        return Ok(shown);
    }
    let mut preview = photocraft_engine::Session::new();
    preview.add_document(doc.clone(), None);
    preview.execute("timeline.setFrame", json!({"frame": frame}))?;
    Ok((*preview.active().ok_or(photocraft_engine::EngineError::NoDocument)?.doc).clone())
}

pub(super) fn web_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    ui.horizontal(|ui| {
        let mut view = s(f, "__view", "2up");
        for (k, l) in WEB_VIEWS {
            if ui.selectable_label(view == k, tl!(l)).clicked() {
                view = k.into();
                // The next Save for Web opens on this tab, as Photoshop's does.
                app.ui.web_view = Some(view.clone());
            }
        }
        f.insert("__view".into(), json!(view));
    });
    ui.add_space(4.0);
    let (frame, bar) = web_playback(ui, f, &doc);
    web_preview::controls(ui, f);
    if let Err(error) = web_preview::prepare_table(app, f) {
        ui.colored_label(t.text_dim, error);
    }
    let p = params(f);
    f.insert("__previewRects".into(), json!([]));
    let area = egui::vec2(
        (ui.available_width() - 270.0 - ui.spacing().item_spacing.x).max(260.0),
        (ui.available_height() - 48.0 - if bar.is_some() { 30.0 } else { 0.0 }).max(180.0),
    );
    ui.horizontal_top(|ui| {
        // Previews grow with the dialog; settings retain their compact width.
        ui.vertical(|ui| {
            ui.set_width(area.x);
            match s(f, "__view", "2up").as_str() {
                "original" => web_pane(app, ui, area, None, f, 0, frame),
                "optimized" => web_pane(app, ui, area, Some(&p), f, 1, frame),
                "2up" => {
                    ui.horizontal(|ui| {
                        web_pane(app, ui, egui::vec2(area.x / 2.0 - 4.0, area.y), None, f, 0, frame);
                        web_pane(app, ui, egui::vec2(area.x / 2.0 - 4.0, area.y), Some(&p), f, 1, frame);
                    });
                }
                _ => {
                    let v = variants(&p);
                    let cell = egui::vec2(area.x / 2.0 - 4.0, area.y / 2.0 - 4.0);
                    ui.horizontal(|ui| {
                        web_pane(app, ui, cell, None, f, 0, frame);
                        web_pane(app, ui, cell, Some(&v[0]), f, 1, frame);
                    });
                    ui.horizontal(|ui| {
                        web_pane(app, ui, cell, Some(&v[1]), f, 2, frame);
                        web_pane(app, ui, cell, Some(&v[2]), f, 3, frame);
                    });
                }
            }
            // Below the previews, as a player's controls are.
            if let Some(bar) = &bar {
                ui.add_space(4.0);
                web_playback_bar(ui, f, bar);
            }
        });
        ui.add_space(10.0);
        // Settings scroll independently on smaller windows.
        ui.vertical(|ui| {
            ui.set_width(250.0);
            egui::ScrollArea::vertical().id_salt("web-settings").max_height(area.y).show(ui, |ui| {
                ui.label(egui::RichText::new(tl!("Preset")).color(t.text_dim));
                let mut preset = s(f, "__preset", "");
                let mut opts: Vec<(String, &str)> = vec![("".into(), "[Unnamed]")];
                opts.extend(web_cmds::PRESETS.iter().map(|p| (p.to_string(), *p)));
                if crate::widgets::dropdown(ui, "web-preset", tl!("Preset"), &mut preset, &opts, 230.0) && !preset.is_empty() {
                    if let Some(Value::Object(m)) = web_cmds::preset_settings(&preset).map(|st| st.to_params()) {
                        f.extend(m);
                    }
                    f.insert("__preset".into(), json!(preset));
                }
                ui.add_space(4.0);
                dropdown_str(ui, "web-format", tl!("Format"), f, "format", &WEB_FORMATS, 230.0);
                ui.add_space(4.0);
                let fmt = s(f, "format", "jpeg");
                match fmt.as_str() {
                    "gif" | "png8" => {
                        dropdown_str(
                            ui,
                            "web-palette",
                            tl!("Color Table"),
                            f,
                            "palette",
                            &[
                                ("perceptual", tl!("Perceptual")),
                                ("selective", tl!("Selective")),
                                ("adaptive", tl!("Adaptive")),
                                ("restrictive", tl!("Restrictive (Web)")),
                                ("exact", tl!("Exact")),
                            ],
                            150.0,
                        );
                        number(ui, f, "colors", tl!("Colors"), 2.0..=256.0, "", 128.0);
                        dropdown_str(
                            ui,
                            "web-dither",
                            tl!("Dither"),
                            f,
                            "dither",
                            &[("none", tl!("No Dither")), ("diffusion", tl!("Diffusion")), ("pattern", tl!("Pattern")), ("noise", tl!("Noise"))],
                            150.0,
                        );
                        number(ui, f, "ditherAmount", tl!("Dither"), 0.0..=100.0, "%", 88.0);
                        check(ui, f, "transparency", tl!("Transparency"), true);
                        check(ui, f, "interlaced", tl!("Interlaced"), false);
                        number(ui, f, "webSnap", tl!("Web Snap"), 0.0..=100.0, "%", 0.0);
                    }
                    "jpeg" => {
                        number(ui, f, "quality", tl!("Quality"), 0.0..=100.0, "", 60.0);
                        check(ui, f, "progressive", tl!("Progressive"), false);
                        check(ui, f, "optimized", tl!("Optimized"), true);
                        check(ui, f, "embedIcc", tl!("Embed Color Profile"), false);
                    }
                    "wbmp" => {
                        dropdown_str(
                            ui,
                            "web-dither",
                            tl!("Dither"),
                            f,
                            "dither",
                            &[("none", tl!("No Dither")), ("diffusion", tl!("Diffusion")), ("pattern", tl!("Pattern"))],
                            150.0,
                        );
                    }
                    _ => {
                        check(ui, f, "transparency", tl!("Transparency"), true);
                        check(ui, f, "interlaced", tl!("Interlaced"), false);
                    }
                }
                if fmt != "png24" || !b(f, "transparency", true) {
                    ui.horizontal(|ui| {
                        let matte_label = ui.label(egui::RichText::new(tl!("Matte")).color(t.text_dim));
                        let mut m = s(f, "matte", "#ffffff");
                        if ui.add(egui::TextEdit::singleline(&mut m).desired_width(80.0)).labelled_by(matte_label.id).changed() {
                            f.insert("matte".into(), json!(m));
                        }
                    });
                }
                ui.add_space(6.0);
                check(ui, f, "convertToSrgb", tl!("Convert to sRGB"), true);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Metadata")).color(t.text_dim));
                    dropdown_str(
                        ui,
                        "web-meta",
                        tl!("Metadata"),
                        f,
                        "metadata",
                        &[
                            ("none", tl!("None")),
                            ("copyright", tl!("Copyright")),
                            ("copyrightAndContact", tl!("Copyright and Contact Info")),
                            ("all", tl!("All")),
                        ],
                        160.0,
                    );
                });
                if matches!(fmt.as_str(), "gif" | "png8") {
                    web_preview::table(ui, f);
                }
                ui.add_space(8.0);
                ui.label(egui::RichText::new(tl!("Image Size")).font(crate::theme::semibold(12.0)).color(t.text));
                web_size(ui, f, &doc);
                let nslices = photocraft_doc::slices::resolve(&doc).len();
                if !doc.slices.is_empty() {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(crate::i18n::trn(crate::i18n::current(), nslices as u64, "{n} slice", "{n} slices")).color(t.text_dim));
                    check(ui, f, "html", tl!("Save HTML and Images"), true);
                    dropdown_str(ui, "web-slices", tl!("Slices"), f, "slices", &[("all", tl!("All Slices")), ("user", tl!("All User Slices"))], 150.0);
                }
            });
        });
    });
}

/// A size row in a two-column grid: the label, then a field showing `shown`. Returns the typed
/// value when it changed, compared with the value the widget was given so a field that only
/// clamps does not count as an edit.
fn size_field(ui: &mut egui::Ui, label: &str, shown: f64, range: std::ops::RangeInclusive<f32>, unit: &str, decimals: u32) -> Option<f64> {
    let t = Tokens::get(ui.ctx());
    let passed = (shown as f32).clamp(*range.start(), *range.end());
    let mut v = passed;
    ui.label(egui::RichText::new(label).color(t.text_dim));
    crate::widgets::value_field_prec(ui, &mut v, range, unit, 90.0, decimals).accessible_name(tl!(&label));
    ui.end_row();
    (v != passed).then_some(f64::from(v))
}

/// Image Size, as in Photoshop's Save for Web: Width and Height linked in proportion (on at
/// each opening), Percent and the resampling method. A side edit sets exact pixels and drops
/// `percent`, which the engine would otherwise apply first; a Percent edit sets both sides.
fn web_size(ui: &mut egui::Ui, f: &mut Map<String, Value>, doc: &photocraft_doc::Document) {
    use crate::export_dialog::{MAX_PX, resize_from_scale, resize_from_side};
    let (w0, h0) = (f64::from(doc.size.width).max(1.0), f64::from(doc.size.height).max(1.0));
    let (w, h) = web_preview::dimensions(doc, &params(f)).unwrap_or((w0, h0));
    let lock = b(f, "__constrain", true);
    let pct = n(f, "percent", w / w0 * 100.0);
    let write = |f: &mut Map<String, Value>, width: f64, height: f64| {
        f.insert("width".into(), json!(width));
        f.insert("height".into(), json!(height));
    };
    let max = MAX_PX as f32;
    let (width, height, percent) = egui::Grid::new("web-size")
        .num_columns(2)
        .show(ui, |ui| {
            (
                size_field(ui, tl!("Width"), w, 1.0..=max, "px", 0),
                size_field(ui, tl!("Height"), h, 1.0..=max, "px", 0),
                size_field(ui, tl!("Percent"), pct, 1.0..=1000.0, "%", 2),
            )
        })
        .inner;
    let side = match (width, height) {
        (Some(v), _) => resize_from_side(w0, h0, v.round(), h, lock, true).ok(),
        (_, Some(v)) => resize_from_side(w0, h0, w, v.round(), lock, false).ok(),
        _ => None,
    };
    if let Some(size) = side {
        f.remove("percent");
        write(f, size.width, size.height);
    } else if let Some(v) = percent
        && let Ok(size) = resize_from_scale(w0, h0, v)
    {
        f.insert("percent".into(), json!(size.scale));
        write(f, size.width, size.height);
    }
    let mut linked = lock;
    if crate::widgets::checkbox(ui, &mut linked, tl!("Constrain proportions")).changed() {
        f.insert("__constrain".into(), json!(linked));
        // Linked again, the height follows the width in the document's proportions.
        if linked && let Ok(size) = resize_from_side(w0, h0, w, h, true, true) {
            f.remove("percent");
            write(f, size.width, size.height);
        }
    }
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Resample")).color(Tokens::get(ui.ctx()).text_dim));
        // Photoshop's Save for Web list, in its order.
        dropdown_str(
            ui,
            "web-resample",
            tl!("Resample"),
            f,
            "resample",
            &[
                ("nearest", tl!("Nearest Neighbor (hard edges)")),
                ("bilinear", tl!("Bilinear")),
                ("bicubic", tl!("Bicubic (smooth gradients)")),
                ("bicubicSmoother", tl!("Bicubic Smoother (enlargement)")),
                ("bicubicSharper", tl!("Bicubic Sharper (reduction)")),
            ],
            170.0,
        );
    });
}

pub(super) fn web_confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    // Check while the control/input policy is still installed, before an asynchronous picker
    // can outlive the request. A browser download uses the interactive platform writer.
    let authorize = |p: &Value| -> Result<(), String> {
        if let Some(authorize) = app.session.authorize.or(if app.automation_input { app.services.automation_authorize } else { None }) {
            authorize(WEB_COMMAND, p).map_err(|e| e.to_string())?;
        }
        if app.automation_input
            && let Some(authorize) = app.services.automation_command.as_ref()
        {
            authorize(WEB_COMMAND, p)?;
        }
        Ok(())
    };
    let mut p = params(f);
    authorize(&p)?;
    // Confirmation may follow a field edit without an intervening frame (Enter or automation).
    let mut fields = f.clone();
    web_preview::prepare_table(app, &mut fields)?;
    let prepared = params(&fields);
    if prepared != p {
        // The policy must also approve the actual palette-bearing export parameters.
        authorize(&prepared)?;
        p = prepared;
    }
    if p.get("path").is_some() || p.get("dir").is_some() {
        return save_for_web(app, p);
    }
    let doc = app.session.active().ok_or("no document")?.doc.clone();
    let has_slices = !doc.slices.is_empty();
    let html = b(f, "html", true);
    let st = WebSettings::from_params(&p, "file.export.saveForWebLegacy").map_err(|e| e.to_string())?;
    let base = photocraft_doc::slices::base_name(&doc);
    let suggested = if has_slices && html { format!("{base}.html") } else { format!("{base}.{}", st.format.ext()) };
    app.pick_save(&suggested, move |app, path| {
        app.refocus(doc.id)?;
        if has_slices {
            let dir = path.rfind(['/', '\\']).map(|i| path[..i].to_string()).unwrap_or_else(|| ".".into());
            p["dir"] = json!(dir);
            p["html"] = json!(html);
        } else {
            p["path"] = json!(path);
        }
        save_for_web(app, p)
    })
}

/// Runs Save for Web with `p`, which says where to write.
fn save_for_web(app: &mut PhotocraftApp, p: Value) -> Result<Value, String> {
    #[cfg(target_arch = "wasm32")]
    let result = download_for_web(app, p);
    #[cfg(not(target_arch = "wasm32"))]
    let result = app.run(WEB_COMMAND, p);
    let r = result.inspect_err(|error| {
        app.ui.status = error.clone();
        app.ui.status_error = true;
    })?;
    app.sync_views();
    app.ui.status =
        crate::i18n::trn(crate::i18n::current(), r["files"].as_array().map_or(0, Vec::len) as u64, "Saved for Web: {n} file", "Saved for Web: {n} files");
    app.ui.status_error = false;
    Ok(r)
}

#[cfg(any(test, target_arch = "wasm32"))]
pub(super) fn download_for_web(app: &mut PhotocraftApp, mut p: Value) -> Result<Value, String> {
    // Browser downloads have no directory hierarchy. Keep the HTML's image and spacer URLs
    // beside the downloaded page, matching the names handed to the platform service.
    p["imagesFolder"] = json!("");
    let write = app.services.write.as_mut().ok_or("no download service")?;
    web_cmds::save_for_web_with_writer(&mut app.session, &p, &mut |path, bytes| write(path, bytes).map_err(photocraft_engine::EngineError::Other))
        .map_err(|e| e.to_string())
}
