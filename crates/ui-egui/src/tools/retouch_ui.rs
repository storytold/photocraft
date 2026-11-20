//! UI for the retouching tools (healing, patch, clone, history brush, art history brush,
//! blur/sharpen/smudge, dodge/burn/sponge, Mixer Brush, Color Replacement) and the smart
//! selection tools (Quick Selection, Object Selection): gesture → engine command, options bars,
//! and the clone-source marker point. The options bars are in [`bar`].

use craft::keymap::Mods;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

/// The tool a stroke uses with ⌥ held: Dodge and Burn swap, as do Blur and Sharpen.
pub(crate) fn alt_flipped(tool: Tool, alt: bool) -> Tool {
    match (tool, alt) {
        (Tool::Dodge, true) => Tool::Burn,
        (Tool::Burn, true) => Tool::Dodge,
        (Tool::Blur, true) => Tool::Sharpen,
        (Tool::Sharpen, true) => Tool::Blur,
        _ => tool,
    }
}

/// Clone Stamp / Healing Brush params for the options bar and the clone source (without
/// `points` and `target`): shared by the live preview and the commit. `None` when no source
/// point is set yet.
pub(crate) fn clone_params(app: &PhotocraftApp) -> Option<Value> {
    // Refused there (`healing_refused`): no live preview either.
    if healing_in_indexed(app, app.ui.tool) {
        return None;
    }
    let o = &app.ui.tool_options;
    // The options bar's Mode is the tool's own brush's.
    let mode = crate::blend_keys::brush_tool_mode(app.ui.tool, app.session.tools.brush.mode);
    // View › Pattern Preview: both tools carry a stroke on across the canvas edges (#8883).
    let sample = if clone_stamp_indexed(app) { "current" } else { o.clone_sample.as_str() };
    let mut p = json!({"aligned": o.clone_aligned, "sampleLayer": sample, "mode": mode.label(), "wrap": app.ui.view.pattern_preview});
    if app.ui.tool == Tool::CloneStamp {
        p["opacity"] = json!(app.session.tools.brush.opacity * 100.0);
        p["flow"] = json!(app.session.tools.brush.flow * 100.0);
    }
    // The Clone Source panel's active slot (set by ⌥-click) drives the stroke: the engine keeps
    // the aligned pairing and applies the slot's scale/rotation/flip. An empty slot refuses to
    // paint, as with no source at all: `ui.clone_source` / `ui.clone_offset` carry no document,
    // so falling back to them would read another document's coordinates from this one's pixels.
    app.session.presets.clone.active().source?;
    Some(p)
}

/// The Clone Stamp in an Indexed Color document, where the reference greys Sample at Current Layer
/// and names the Normal Mode "Threshold" (#12613).
pub(crate) fn clone_stamp_indexed(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::CloneStamp && app.session.active().is_some_and(|st| st.doc.mode == photocraft_doc::ColorMode::Indexed)
}

/// History Brush params for the live preview and the commit (without `points` and `target`): the
/// options bar's Mode and the tool's own brush. For an Eraser with Erase to History they also
/// carry `eraseToHistory` and the Eraser's Pencil or Block tip.
pub(crate) fn history_brush_params(app: &PhotocraftApp) -> Value {
    history_brush_params_with(app, app.ui.tool_options.erase_to_history)
}

/// [`history_brush_params`], with Erase to History on whatever the option says: ⌥ reverses the
/// option for one stroke.
fn history_brush_params_with(app: &PhotocraftApp, erase_to_history: bool) -> Value {
    // The tool's tip goes in the params: the session's brush changes with the tool, if a tool key
    // is pressed mid-stroke.
    let brush = &app.session.tools.brush;
    let mut p = json!({
        "mode": brush.mode.label(),
        "opacity": brush.opacity * 100.0,
        "flow": brush.flow * 100.0,
        "size": brush.size,
        "hardness": brush.hardness * 100.0,
        "spacing": brush.spacing * 100.0,
    });
    // The whole brush too (tip shape, sampled tip, dynamics, smoothing), so the commit and any
    // replay paint with the brush the stroke was pressed with. Stored tips go as references
    // (`remembered_settings`, as each tool's remembered brush); only with a sampled tip or pattern
    // too big for that, not yet in the preset store, does the brush go in full. `square` is
    // written out even when off.
    let mut whole = photocraft_engine::brush_cmds::remembered_settings(brush).or_else(|| serde_json::to_value(brush).ok()).unwrap_or(Value::Null);
    if let Some(o) = whole.as_object_mut() {
        o.insert("square".into(), json!(brush.square));
        p["brush"] = whole;
    }
    if app.ui.tool == Tool::Eraser && erase_to_history {
        p["eraseToHistory"] = json!(true);
        // The Eraser's Pencil and Block tips (as `strokes::stroke_params`).
        if crate::eraser_ui::pencil_mode(app, Tool::Eraser) {
            p["pencil"] = json!(true);
            // Pencil mode has no Flow control: the Brush mode's setting must not carry over.
            p["flow"] = json!(100.0);
        }
        if crate::eraser_ui::block_mode(app, Tool::Eraser) {
            p["block"] = json!(true);
            p["zoom"] = json!(app.point_zoom());
        }
    }
    p
}

/// The options a stroke of `tool` keeps from its press (`canvas::Drag::history_params`).
pub(crate) fn history_params_at_press(app: &PhotocraftApp, tool: Tool, alt: bool) -> Option<Value> {
    (tool == Tool::HistoryBrush).then(|| history_brush_params_with(app, alt != app.ui.tool_options.erase_to_history))
}

/// The command and options-bar params (without `points` and `target`) of a sequential-dab tool:
/// Blur, Sharpen, Smudge, Dodge, Burn, Sponge. Shared by the live preview and the commit.
pub(crate) fn dab_params(app: &PhotocraftApp, tool: Tool) -> Option<(&'static str, Value)> {
    let o = &app.ui.tool_options;
    Some(match tool {
        Tool::Blur => ("paint.blur", json!({"strength": o.strength, "sampleAllLayers": o.sample_all.blur})),
        Tool::Sharpen => ("paint.sharpen", json!({"strength": o.strength, "protectDetail": o.protect_detail, "sampleAllLayers": o.sample_all.sharpen})),
        Tool::Smudge => (
            "paint.smudge",
            json!({"strength": o.strength, "fingerPainting": o.finger_painting, "sampleAllLayers": o.sample_all.smudge, "mode": o.smudge_mode.label()}),
        ),
        Tool::Dodge | Tool::Burn => (
            if tool == Tool::Dodge { "paint.dodge" } else { "paint.burn" },
            json!({"range": o.tone_range, "exposure": o.exposure, "protectTones": o.protect_tones}),
        ),
        // Flow as the bar's field stores it (#12976), so a restored fraction paints what the bar shows.
        Tool::Sponge => ("paint.sponge", json!({"mode": o.sponge_mode, "flow": crate::eraser_ui::stored_tolerance(o.sponge_flow), "vibrance": o.vibrance})),
        _ => return None,
    })
}

/// `paint.colorReplacement` params from the options bar (without `points`): shared by the live
/// preview and the commit. The replacement `color` is the foreground, recorded so a replayed
/// stroke paints the same colour; Background Swatch sampling matches the background colour.
pub(crate) fn color_replacement_params(app: &PhotocraftApp) -> Value {
    let o = &app.ui.tool_options;
    json!({
        "color": app.session.tools.foreground,
        "mode": o.cr_mode,
        "sampling": o.cr_sampling,
        "limits": o.cr_limits,
        // As the field stores it (#11930), so a restored fraction replaces the same pixels before
        // and after the options bar shows it.
        "tolerance": crate::eraser_ui::stored_tolerance(o.cr_tolerance),
        "antiAlias": o.cr_anti_alias,
        // A mask or channel target is refused: the tool recolours layer pixels only.
        "target": crate::canvas::paint_target(app),
    })
}

/// `paint.patternStamp` params from the options bar and the Patterns panel (without `points` and
/// `target`): shared by the live preview and the commit. The jitter `seed` comes from the stroke's
/// first point, so the live stroke (which starts with only that point) and the commit agree.
pub(crate) fn pattern_stamp_params(app: &PhotocraftApp, points: &[[f64; 3]]) -> Value {
    let o = &app.ui.tool_options;
    let mut p = json!({
        "aligned": o.pattern_stamp_aligned,
        "impressionist": o.pattern_stamp_impressionist,
        "scale": o.pattern_stamp_scale,
        "angle": o.pattern_stamp_angle,
        "seed": stroke_seed(points),
        // The options bar's Mode is the tool's own brush's; Behind is a mode of that list.
        "mode": if app.session.tools.brush.behind { "Behind" } else { app.session.tools.brush.mode.label() },
        "opacity": app.session.tools.brush.opacity * 100.0,
        "flow": app.session.tools.brush.flow * 100.0,
    });
    if o.pattern_stamp_aligned
        && let Some(ph) = app.ui.pattern_stamp_phase
    {
        p["phase"] = json!(ph);
    }
    if let Some(id) = photocraft_engine::presets::patterns::current(&app.session).or(app.session.patterns.items.first()).map(|x| x.id.clone()) {
        p["pattern"] = json!(id);
    }
    p
}

/// The jitter seed of a stroke drawn live and committed on release: from its first position, which
/// the live stroke and the commit share (the engine's default hashes every point, which the live
/// stroke doesn't have yet).
pub(crate) fn stroke_seed(points: &[[f64; 3]]) -> u64 {
    // Position only: a pen tap raises the first point's pressure on release (#1798).
    let first = points.first().map(|p| json!([p[0], p[1]]).to_string()).unwrap_or_default();
    photocraft_engine::paint::rng::seed_from_bytes(first.as_bytes())
}

/// A Clone Stamp press with no source point yet: the reference application's alert, opened at
/// the press (#12609) and once while it is up. Returns whether it refused the stroke.
pub(crate) fn clone_stamp_without_source(app: &mut PhotocraftApp, tool: Tool) -> bool {
    if tool != Tool::CloneStamp || clone_params(app).is_some() {
        return false;
    }
    let message = if cfg!(target_os = "macos") {
        tl!("Could not use the clone stamp tool because the area to clone has not been defined (Option-click to define a source point).")
    } else {
        tl!("Could not use the clone stamp tool because the area to clone has not been defined (Alt-click to define a source point).")
    };
    let _ = crate::dialogs::alerts::alert(app, message);
    true
}

/// Finish a stroke with a retouching tool. Returns false if `tool` isn't one.
pub fn finish_stroke(app: &mut PhotocraftApp, tool: Tool, points: &[[f64; 3]], mods: Mods, line: bool) -> bool {
    finish_stroke_with(app, tool, points, mods, line, None)
}

/// [`finish_stroke`], with a History Brush stroke's options as they were at the press.
pub(crate) fn finish_stroke_with(app: &mut PhotocraftApp, tool: Tool, points: &[[f64; 3]], mods: Mods, line: bool, history: Option<Value>) -> bool {
    let o = app.ui.tool_options.clone();
    let pts = json!(points);
    let (cmd, mut p): (&str, Value) = match tool {
        Tool::SpotHealing => ("paint.spotHealing", json!({"type": o.spot_type, "sampleAllLayers": o.sample_all.spot_healing})),
        Tool::Remove => {
            if !o.remove_after_each_stroke {
                remove_pending_add(app, points);
                return true;
            }
            ("paint.remove", remove_params(&o, crate::canvas::paint_target(app).as_str() == Some("pixels")))
        }
        Tool::MixerBrush => ("paint.mixerBrush", json!({})),
        Tool::ColorReplacement => ("paint.colorReplacement", color_replacement_params(app)),
        Tool::PatternStamp => ("paint.patternStamp", pattern_stamp_params(app, points)),
        Tool::Healing | Tool::CloneStamp => {
            if healing_refused(app, tool) {
                return true;
            }
            let Some(p) = clone_params(app) else {
                if clone_stamp_without_source(app, tool) {
                    return true;
                }
                // Option-click on the Mac, Alt-click on Windows.
                app.ui.status = if cfg!(target_os = "macos") {
                    tl!("Option-click to define a source point to clone from")
                } else {
                    tl!("Alt-click to define a source point to clone from")
                }
                .into();
                app.ui.status_error = true;
                return true;
            };
            (if tool == Tool::Healing { "paint.healingBrush" } else { "paint.cloneStamp" }, p)
        }
        Tool::HistoryBrush => ("paint.historyBrush", history.unwrap_or_else(|| history_brush_params(app))),
        Tool::ArtHistoryBrush => (
            "paint.artHistoryBrush",
            json!({
                "style": o.art_history_style,
                "area": o.art_history_area,
                "tolerance": o.art_history_tolerance,
                "opacity": app.session.tools.brush.opacity * 100.0,
            }),
        ),
        Tool::Blur | Tool::Sharpen | Tool::Smudge | Tool::Dodge | Tool::Burn | Tool::Sponge => match dab_params(app, tool) {
            Some(cp) => cp,
            None => return false,
        },
        Tool::QuickSelection => {
            let size = app.session.tools.brush.size;
            let mode = if mods.alt { "subtract" } else { "add" };
            let xy: Vec<[f64; 2]> = points.iter().map(|q| [q[0], q[1]]).collect();
            let (sample_all, enhance) = (o.sample_all.quick_selection, o.enhance_edge);
            let r = app.run("select.quick", json!({"points": xy, "size": size, "mode": mode, "enhanceEdge": enhance, "sampleAllLayers": sample_all}));
            report_smart_result(app, r, tl!("Quick Selection found nothing there"));
            return true;
        }
        _ => return false,
    };
    p["points"] = pts;
    // The layer mask, an alpha channel or the Quick Mask when targeted, as the Brush paints.
    p["target"] = crate::canvas::paint_target(app);
    crate::canvas::unsmoothed_line(&mut p, line);
    let result = app.run(cmd, p);
    app.retouch_from_live = matches!(&result, Ok(r) if r.get("fromLive").and_then(Value::as_bool) == Some(true));
    match result {
        Ok(r) if matches!(tool, Tool::Healing | Tool::CloneStamp) => {
            // Aligned: keep the offset for later strokes; non-aligned: every stroke restarts at the source.
            app.ui.clone_offset = r.get("offset").and_then(|v| serde_json::from_value(v.clone()).ok()).filter(|_| o.clone_aligned);
        }
        Ok(r) if tool == Tool::PatternStamp => {
            app.ui.pattern_stamp_phase = r.get("phase").and_then(|v| serde_json::from_value(v.clone()).ok()).filter(|_| o.pattern_stamp_aligned);
        }
        Ok(_) => {}
        // The Art History Brush's source refusals are alerts with OK, as the press-time ones, and
        // not in the status bar as well (#12638).
        Err(e) if tool == Tool::ArtHistoryBrush && e.starts_with("Could not use the art history brush because") => {
            app.ui.status.clear();
            app.ui.status_error = false;
            let _ = crate::dialogs::alerts::alert(app, tl!(&e).to_owned());
        }
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    true
}

/// `paint.remove` params from the options bar (without the strokes).
fn remove_params(o: &crate::state::ToolOptions, composite: bool) -> Value {
    json!({"sampleAllLayers": o.sample_all.remove, "newLayer": o.remove_new_layer && composite})
}

/// The selection a queued Remove stroke was drawn against. A later selection must not
/// redirect the queued strokes to a different layer or colour component.
#[derive(Clone, PartialEq)]
pub(crate) struct RemoveTarget {
    doc: photocraft_doc::DocId,
    layer: Option<photocraft_doc::LayerId>,
    channel: photocraft_engine::channel_cmds::ChannelTarget,
    paint: Value,
}

fn remove_target(app: &PhotocraftApp) -> Option<RemoveTarget> {
    let st = app.session.active()?;
    Some(RemoveTarget { doc: st.doc.id, layer: st.active_layer, channel: st.channel_view.target, paint: crate::canvas::paint_target(app) })
}

/// Remove after each stroke off: a finished stroke waits with the others, tinted on the canvas,
/// until ↩ or the options bar's ✓ applies them all as one removal (Esc or ⊘ drops them).
pub fn remove_pending_add(app: &mut PhotocraftApp, points: &[[f64; 3]]) {
    let Some(target) = remove_target(app) else { return };
    let Some(st) = app.session.active() else { return };
    let size = [st.doc.size.width, st.doc.size.height];
    if app.remove_pending_target.as_ref() != Some(&target) {
        app.remove_pending.clear();
        app.remove_trail = None;
        app.remove_pending_target = Some(target);
    }
    // The brush it was drawn with goes with it: the tint shows it, the removal uses it, however
    // the brush changes before ↩.
    let b = &app.session.tools.brush;
    let entry = json!({"points": points, "size": b.size, "hardness": b.hardness * 100.0, "spacing": b.spacing * 100.0});
    let brush = || photocraft_engine::retouch_cmds::stroke_brush(&app.session.tools.brush, &entry);
    app.remove_trail.get_or_insert_with(|| crate::stroke_trail::Trail::new(size)).feed_stroke_reach(points, brush);
    app.remove_pending.push(entry);
    app.ui.status = crate::i18n::fmt(tl!("Strokes to remove: {n} (Enter applies, Esc cancels)"), &[("n", &app.remove_pending.len().to_string())]);
    app.ui.status_error = false;
}

/// Apply the waiting strokes as one removal (one job, one undo step). False when none wait.
pub fn remove_pending_commit(app: &mut PhotocraftApp) -> bool {
    // Enter can arrive before the next canvas frame checks pending-stroke lifetime.
    if !remove_pending_live(app) {
        return false;
    }
    let strokes = std::mem::take(&mut app.remove_pending);
    app.remove_trail = None;
    app.remove_pending_target = None;
    let mut p = remove_params(&app.ui.tool_options, crate::canvas::paint_target(app).as_str() == Some("pixels"));
    p["strokes"] = json!(strokes);
    p["target"] = crate::canvas::paint_target(app);
    app.ui.status.clear();
    if let Err(e) = app.run("paint.remove", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

/// Drop the waiting strokes. False when none wait.
pub fn remove_pending_cancel(app: &mut PhotocraftApp) -> bool {
    if app.remove_pending.is_empty() {
        return false;
    }
    app.remove_pending.clear();
    app.remove_trail = None;
    app.remove_pending_target = None;
    app.ui.status.clear();
    app.ui.status_error = false;
    true
}

/// Whether Remove strokes are still waiting: they are dropped when the tool or the document
/// changed under them (checked every frame).
pub fn remove_pending_live(app: &mut PhotocraftApp) -> bool {
    if app.remove_pending.is_empty() {
        return false;
    }
    if app.ui.tool == Tool::Remove && remove_target(app).as_ref() == app.remove_pending_target.as_ref() {
        return true;
    }
    remove_pending_cancel(app);
    false
}

/// Patch Tool and Content-Aware Move Tool: a drag that starts inside the selection (without ⇧ or
/// ⌥) drags the selection; any other drag draws a lasso selection.
pub fn patch_drags_selection(app: &PhotocraftApp, at: [f64; 2], mods: Mods) -> bool {
    if mods.shift || mods.alt {
        return false;
    }
    let Some(sel) = app.session.active().and_then(|st| st.doc.selection.as_ref()) else { return false };
    sel.sample_channel(at[0].floor() as i32, at[1].floor() as i32, 0) > 0.0
}

/// The Patch or Content-Aware Move press.
#[derive(Clone, Debug)]
pub(crate) struct PatchPress {
    /// The tool pressed.
    pub tool: Tool,
    /// The press point, in document pixels.
    pub start: [f64; 2],
    /// The modifiers held at the press.
    pub modifiers: Mods,
}

impl PatchPress {
    /// The press of `tool` at `start` with `modifiers` held.
    pub(crate) fn new(tool: Tool, start: [f64; 2], modifiers: Mods) -> Self {
        Self { tool, start, modifiers }
    }
}

/// Where a Patch drag ends: with ⇧ pressed after the press (⇧ at the press adds to the selection
/// instead, so this never sees it) the end is held to the dominant axis from the start, 0 or 90
/// degrees only (#12129). Other tools, and a drag without ⇧, end at `end`; so does a drag that
/// never moved.
pub(crate) fn patch_end(press: &PatchPress, end: [f64; 2], shift: bool) -> [f64; 2] {
    if press.tool != Tool::Patch || !shift {
        return end;
    }
    let (dx, dy) = (end[0] - press.start[0], end[1] - press.start[1]);
    if dx.abs() >= dy.abs() { [end[0], press.start[1]] } else { [press.start[0], end[1]] }
}

/// Whole-pixel offset for a drag from `start` to `end`, limited so the dragged outline stays on the
/// canvas (the engine rejects a patch or move that leaves it).
pub fn patch_offset(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) -> [i32; 2] {
    let (dx, dy) = ((end[0] - start[0]).round(), (end[1] - start[1]).round());
    let (dx, dy) = if dx.is_finite() && dy.is_finite() { (dx.clamp(-1e7, 1e7) as i32, dy.clamp(-1e7, 1e7) as i32) } else { (0, 0) };
    let Some((canvas, sel)) = app.session.active().and_then(|st| Some((st.doc.bounds(), st.doc.selection.clone()?))) else { return [dx, dy] };
    let key = u64::MAX - app.session.active().map_or(0, |st| st.doc.id.0);
    let b = app.cached_bounds(key, &sel).intersect(&canvas);
    if b.is_empty() {
        return [dx, dy];
    }
    let clamp = |d: i32, lo: i32, hi: i32| if lo > hi { 0 } else { d.clamp(lo, hi) };
    [clamp(dx, canvas.x0 - b.x0, canvas.x1 - b.x1), clamp(dy, canvas.y0 - b.y0, canvas.y1 - b.y1)]
}

/// Status-bar hint for the Patch and Content-Aware Move tools: the selection is the patch and it
/// has to be dragged somewhere. Without it a lasso, or a click inside the selection, changes
/// nothing on the canvas and the tool looks dead (#1715).
pub fn patch_hint(app: &mut PhotocraftApp) {
    app.ui.status = tl!("Now drag the selection onto another area").into();
    app.ui.status_error = false;
}

/// Patch Tool: the patch was dragged from `start` to `end`.
pub fn finish_patch(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    let off = patch_offset(app, start, end);
    if off == [0, 0] {
        patch_hint(app);
        return;
    }
    let preview = crate::patch_preview::take(app);
    let o = &app.ui.tool_options;
    if o.patch_content_aware {
        // A background job (progress dialog, Esc cancels), like the Content-Aware Move.
        let p = json!({
            "offset": off,
            "contentAware": true,
            "structure": o.patch_structure.round().clamp(1.0, 7.0),
            "color": o.patch_color.round().clamp(0.0, 10.0),
            "sampleAllLayers": o.sample_all.patch,
            "target": crate::canvas::paint_target(app),
        });
        if let Err(e) = app.run("paint.patch", p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    let p = json!({"offset": off, "mode": app.ui.tool_options.patch_mode, "target": crate::canvas::paint_target(app)});
    match app.run("paint.patch", p) {
        Ok(_) => crate::patch_preview::committed(app, preview, off),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Content-Aware Move Tool: the selection was dragged from `start` to `end`. The engine runs it as
/// a background job (progress dialog, Esc cancels); the selection follows the content. With
/// Transform On Drop a transform box comes up on the dropped patch first ([`crate::cam_transform`]).
pub fn finish_content_aware_move(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    let off = patch_offset(app, start, end);
    if off == [0, 0] {
        patch_hint(app);
        return;
    }
    if app.ui.tool_options.cam_transform_on_drop && crate::cam_transform::begin(app, off) {
        return;
    }
    let p = content_aware_move_params(app, [i64::from(off[0]), i64::from(off[1])]);
    if let Err(e) = app.run("paint.contentAwareMove", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The Content-Aware Move tool's "Don't show again" for [`content_aware_move_no_pixels`].
pub(crate) const CAM_NO_PIXELS_PREF: &str = "contentAwareMove.noPixelsSelected";

/// Content-Aware Move Tool with no selection: a drag whose lasso would select no pixel (a click,
/// a straight line) selects nothing and adds no History state, and says so in the alert "No pixels
/// are selected." with "Don't show again" (#12192). Returns whether this drag was such a one. With
/// a selection, or a lasso that selects pixels, it is the tool's lasso as before.
pub fn content_aware_move_no_pixels(app: &mut PhotocraftApp, points: &[[f64; 2]]) -> bool {
    let Some(canvas) = app.session.active().filter(|s| s.doc.selection.is_none()).map(|s| s.doc.bounds()) else { return false };
    if lasso_selects_pixels(points, canvas, app.ui.tool_options.anti_alias) {
        return false;
    }
    let _ = crate::dialogs::alerts::alert_once(app, tl!("No pixels are selected."), CAM_NO_PIXELS_PREF);
    true
}

/// Whether `select.lasso` through `points` would select any pixel of `canvas`: the same
/// rasterization (its coverage over the canvas's coordinates), kept where it rounds to a nonzero
/// 8-bit value as the selection stores it. Only the lasso's bounds are rasterized, a strip of rows
/// at a time, stopping at the first selected pixel. The strips start at 64 rows, so a lasso that
/// selects is answered after the first, and double up to about 4 MP (16 MB), so a lasso that
/// selects nothing (a long line) is rasterized and scanned in parallel: a corner-to-corner line
/// on 7360 x 4912 takes 25-32 ms, about what the `select.lasso` it replaces took (23-27 ms).
fn lasso_selects_pixels(points: &[[f64; 2]], canvas: photocraft_geom::Rect, anti_alias: bool) -> bool {
    const FIRST_STRIP: u32 = 64;
    const MAX_STRIP_PIXELS: u32 = 1 << 22;
    // `coverage8` of the selection combine: the 8-bit value a pixel's coverage is stored as. It
    // only grows with the coverage, so a pixel is kept from the least coverage it keeps (found
    // once over the f32 bits in [0, 1]), and the scan is a single compare per pixel.
    let stored = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let (mut below, mut kept) = (0u32, 1.0f32.to_bits());
    while kept - below > 1 {
        let mid = below + (kept - below) / 2;
        if stored(f32::from_bits(mid)) > 0 { kept = mid } else { below = mid }
    }
    let least = f32::from_bits(kept);
    // As `select.lasso` reads the points.
    let pts: Vec<(f32, f32)> = points.iter().map(|p| (p[0] as f32, p[1] as f32)).collect();
    if pts.len() < 3 {
        return false;
    }
    let (mut lo, mut hi) = ((f32::INFINITY, f32::INFINITY), (f32::NEG_INFINITY, f32::NEG_INFINITY));
    for &(x, y) in pts.iter().filter(|(x, y)| x.is_finite() && y.is_finite()) {
        lo = (lo.0.min(x), lo.1.min(y));
        hi = (hi.0.max(x), hi.1.max(y));
    }
    if lo.0 > hi.0 || lo.1 > hi.1 {
        return false;
    }
    // Saturating float-to-int casts; the canvas clips them.
    let bounds =
        photocraft_geom::Rect::new(lo.0.floor() as i32, lo.1.floor() as i32, (hi.0.ceil() as i32).saturating_add(1), (hi.1.ceil() as i32).saturating_add(1));
    let window = bounds.intersect(&canvas);
    let max_rows = (MAX_STRIP_PIXELS / window.width().max(1)).max(FIRST_STRIP);
    let (mut y, mut rows) = (window.y0, FIRST_STRIP);
    while y < window.y1 {
        let strip = photocraft_geom::Rect::new(window.x0, y, window.x1, y.saturating_add_unsigned(rows).min(window.y1));
        if any_at_least(&photocraft_algo::selection::polygon_window(&pts, canvas, strip, anti_alias), least) {
            return true;
        }
        y = strip.y1;
        rows = rows.saturating_mul(2).min(max_rows);
    }
    false
}

/// Whether any of `mask` is at least `least`: in parallel blocks, each one a branch-free pass the
/// compiler vectorizes.
fn any_at_least(mask: &[f32], least: f32) -> bool {
    let block = |b: &[f32]| b.iter().fold(false, |found, v| found | (*v >= least));
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        mask.par_chunks(1 << 16).any(block)
    }
    #[cfg(target_arch = "wasm32")]
    mask.chunks(1 << 16).any(block)
}

/// `paint.contentAwareMove`'s params for a drag by `offset`, from the options bar.
pub fn content_aware_move_params(app: &PhotocraftApp, offset: [i64; 2]) -> Value {
    let o = &app.ui.tool_options;
    json!({
        "offset": offset,
        "mode": o.cam_mode,
        "structure": o.cam_structure.round().clamp(1.0, 7.0),
        "color": o.cam_color.round().clamp(0.0, 10.0),
        "sampleAllLayers": o.sample_all.content_aware_move,
        "target": crate::canvas::paint_target(app),
    })
}

/// Object Selection: the dragged rectangle. A click (or a sub-2 px nudge) cannot be a
/// rectangle: with Object Finder on it selects the object under the pointer, the one hovering
/// highlighted (#4087). With it off it falls back to a Quick Selection dab at the brush size
/// instead of looking dead (⌥ or Subtract mode subtracts, otherwise it adds). Larger drags
/// run the rectangle GrabCut. Both combine as the options bar's New / Add / Subtract /
/// Intersect buttons say, with ⇧ adding, ⌥ subtracting and ⇧⌥ intersecting (#12853).
pub fn finish_object_selection(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2], mods: Mods) {
    // Indexed Color and Bitmap: the reference application's alert with OK (#12867), and nothing runs.
    if let Some(why) = app.session.active().and_then(|st| photocraft_engine::smartselect_cmds::object_mode_refusal(st.doc.mode)) {
        crate::object_finder::clear(app);
        let _ = crate::dialogs::alerts::alert(app, tl!(why));
        return;
    }
    let (x, y) = (start[0].min(end[0]), start[1].min(end[1]));
    let (w, h) = ((end[0] - start[0]).abs(), (end[1] - start[1]).abs());
    let sample_all = app.ui.tool_options.sample_all.object_selection;
    let hard_edge = app.ui.tool_options.object_hard_edge;
    let mode = crate::tool_feedback::mode_name(crate::tool_feedback::selection_op(
        Tool::ObjectSelection,
        app.ui.selection_mode,
        mods,
        crate::tool_feedback::has_selection(app),
    ));
    if (w < 2.0 || h < 2.0) && app.ui.tool_options.object_finder {
        // Object Finder (#4087): the object under the click, as hovering highlighted it.
        let r = app.run("select.object", json!({"point": [end[0], end[1]], "mode": mode, "sampleAllLayers": sample_all, "hardEdge": hard_edge}));
        crate::object_finder::clear(app);
        report_smart_result(app, r, tl!("No object found there — drag a rectangle around it"));
        return;
    }
    if w < 2.0 || h < 2.0 {
        let size = app.session.tools.brush.size;
        let mode = if mods.alt || mode == "subtract" { "subtract" } else { "add" };
        let r = app.run("select.quick", json!({"points": [[end[0], end[1]]], "size": size, "mode": mode, "sampleAllLayers": sample_all}));
        report_smart_result(app, r, tl!("Quick Selection found nothing there"));
        return;
    }
    let r = app.run(
        "select.object",
        json!({"rect": [x.round(), y.round(), w.round(), h.round()], "mode": mode, "sampleAllLayers": sample_all, "hardEdge": hard_edge}),
    );
    report_smart_result(app, r, tl!("Object Selection found nothing there — try a tighter rectangle"));
}

/// Reports a smart-selection command result in the status bar. Engine errors are already
/// reported by [`PhotocraftApp::run`], but an empty result (`changed: false`) is a silent
/// success that looks like a dead button (Select Subject finding nothing on a busy photo,
/// Object Selection missing its rectangle). `empty_msg` says what to try next.
pub fn report_smart_result(app: &mut PhotocraftApp, res: Result<Value, String>, empty_msg: &str) {
    if matches!(res, Ok(ref v) if v.get("changed").and_then(Value::as_bool) == Some(false)) {
        app.ui.status = empty_msg.into();
        app.ui.status_error = false;
    }
}

/// The Healing Brush in an Indexed Color document: the reference's alert with OK at the ⌥-click
/// and at a stroke, and nothing changes (#12112). The engine refuses the stroke with the same words.
fn healing_refused(app: &mut PhotocraftApp, tool: Tool) -> bool {
    if !healing_in_indexed(app, tool) {
        return false;
    }
    let _ = crate::dialogs::alerts::alert(
        app,
        tl!("Could not set the clone source because it does not work with Indexed Color images (convert image to RGB Color mode to edit)."),
    );
    true
}

fn healing_in_indexed(app: &PhotocraftApp, tool: Tool) -> bool {
    tool == Tool::Healing && app.session.active().is_some_and(|st| st.doc.mode == photocraft_doc::ColorMode::Indexed)
}

/// ⌥-click with Clone Stamp / Healing Brush sets the source.
pub fn set_source(app: &mut PhotocraftApp, x: f64, y: f64) {
    if healing_refused(app, app.ui.tool) {
        return;
    }
    app.ui.clone_source = Some([x.round(), y.round()]);
    app.ui.clone_offset = None;
    let _ = app.run("cloneSource.set", json!({"source": [x.round(), y.round()]}));
    let template = "Clone source set at {x}, {y}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("x", &format!("{x:.0}")), ("y", &format!("{y:.0}"))]);
    app.ui.status_error = false;
}

/// Where the clone source is sampled from, shown only while a Clone Stamp or Healing Brush stroke
/// is painted, as in Photoshop: setting the source (⌥-click) leaves nothing on the canvas (#668).
/// `at` is the last point of the running stroke, `None` while none runs. A source in another open
/// document has no point on this canvas.
pub fn source_marker_point(app: &PhotocraftApp, at: Option<[f64; 2]>) -> Option<[f64; 2]> {
    if !matches!(app.ui.tool, Tool::CloneStamp | Tool::Healing) {
        return None;
    }
    let slot = app.session.presets.clone.active();
    if slot.document_closed || slot.document.is_some_and(|id| app.session.active().is_none_or(|st| st.doc.id != id)) {
        return None;
    }
    crate::preset_panels::clone_sample_point(app, Some(at?))
}

pub(crate) mod bar;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../healing_indexed_alert_tests.rs"]
mod healing_indexed_alert_tests;

#[cfg(test)]
#[path = "../clone_stamp_no_source_alert_tests.rs"]
mod clone_stamp_no_source_alert_tests;

#[cfg(test)]
#[path = "remove_options_bar_labels_tests.rs"]
mod remove_options_bar_labels_tests;
