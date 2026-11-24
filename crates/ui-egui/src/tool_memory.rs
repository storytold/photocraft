//! Selected tool, colours and tool options remembered between launches, as in Photoshop (#2814).
//!
//! `Preferences::tool_options` holds the [`ToolOptions`] fields that differ from their defaults,
//! so a default changed in a later release still reaches users who never touched that option.
//! Saved once the pointer is up (not on every slider step), restored at launch. A hand-edited or
//! old file can't break the options: an unknown key is ignored and a field that doesn't read back
//! (wrong type, a number too large for its field) keeps its default, and numbers are clamped to
//! the ranges their options-bar fields allow ([`sanitize`]).
//!
//! Each painting tool's own brush (`UiState::tool_brushes`, swapped by
//! `paint_mouse::sync_tool_brush`) is kept the same way in `Preferences::tool_brushes` (#6212),
//! with the tool that owns the session brush (that brush itself is `Preferences::last_brush`).
//! Reset Tool and Reset All Tools change both, so the next save forgets what they reset.
//!
//! The tools used last (`UiState::group_tools`) are kept the same way in
//! `Preferences::group_tools` (#3569), so a tool picked from a flyout stays on its toolbar slot
//! and its key after a restart. A name this version doesn't know is skipped, as with the options.

use craft::color_ui::ui::{SampleSize, SampleSource};
use craft::draw::ui::crop::{FIELD_MAX, RATIO_MAX};
use craft::draw::ui::{LINE_WEIGHT_RANGE, RADIUS_RANGE};
use craft::imaging::ui::select::MagneticSettings;
use craft::ui::bars::ConstraintFields;
use photocraft_engine::BrushSettings;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

/// The tool whose brush the session brush is, and the other painting tools' brushes.
pub(crate) type ToolBrushes = (Tool, Vec<(Tool, BrushSettings)>);

/// The sRGB stand-ins and exact inks of recently picked CMYK colours, oldest first.
pub(crate) type CmykPicks = Vec<([f32; 3], [f32; 4])>;

/// Explicit choices override temporary pen and held-key tools, even when selecting the same tool.
pub(crate) fn select(app: &mut PhotocraftApp, tool: Tool) {
    app.stylus.tool_before_eraser = None;
    app.ui.spring = None;
    if app.ui.tool == Tool::Frame && tool != Tool::Frame {
        crate::frame_tool::clear(app);
    }
    // PLAN: R95 puts the end of a Type on a Path proposal (`type_path_placement::propose(app, None)`) in `tools::tool_changed` (canvas/tools/mod.rs), not here.
    app.ui.tool = tool;
    ended_by_tool(app);
}

/// Add New Artboard's placing mode ends with the Artboard tool (#11386): every place that sets
/// `ui.tool` calls this right after, so there is one rule.
pub(crate) fn ended_by_tool(app: &mut PhotocraftApp) {
    if app.ui.tool != Tool::Artboard {
        app.ui.artboard_tool.adding = false;
    }
}

/// Temporary canvas overrides do not change `ui.tool`. Spring-loaded letters and the pen's
/// eraser end do, so remember the tool they will return to instead. A different explicit choice
/// (or beginning a type edit) owns the tool and stands, just as in `spring_tools::tick`.
pub(crate) fn selected(app: &PhotocraftApp) -> Tool {
    let tool = if app.ui.tool == Tool::Eraser { app.stylus.tool_before_eraser.unwrap_or(app.ui.tool) } else { app.ui.tool };
    if let Some(s) = &app.ui.spring
        && tool == s.tool
        && app.ui.text_edit.is_none()
    {
        return s.previous;
    }
    tool
}

/// The options that differ from the defaults, as JSON (`Null` when none do).
fn changed(o: &ToolOptions) -> Value {
    let (Ok(Value::Object(now)), Ok(Value::Object(def))) = (serde_json::to_value(o), serde_json::to_value(ToolOptions::default())) else {
        return Value::Null;
    };
    let diff: Map<String, Value> = now.into_iter().filter(|(k, v)| def.get(k) != Some(v)).collect();
    if diff.is_empty() { Value::Null } else { Value::Object(diff) }
}

/// Remember the selected tool, the tools used last and the options once the user lets go of the
/// pointer. Run every frame, so choices made through the control channel or a tool preset are
/// remembered too.
pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    let tool = selected(app);
    if app.prefs_rt.selected_tool != Some(tool) {
        app.prefs_rt.selected_tool = Some(tool);
        if let Ok(now) = serde_json::to_value(tool)
            && app.session.prefs().selected_tool != now
        {
            app.session.prefs.edit(|p| p.selected_tool = now);
            // prefs_ui::tick ran before the panels; ensure it can flush this new revision.
            ctx.request_repaint();
        }
    }
    persist_colors(app, ctx);
    persist_group_tools(app, ctx, tool);
    persist_brushes(app);
    if app.prefs_rt.tool_options.as_ref() == Some(&app.ui.tool_options) {
        return;
    }
    app.prefs_rt.tool_options = Some(app.ui.tool_options.clone());
    let now = changed(&app.ui.tool_options);
    if app.session.prefs().tool_options != now {
        app.session.prefs.edit(|p| p.tool_options = now);
    }
}

/// Keep pixel and mask swatches separately, even when the app exits with a mask targeted.
fn colors(app: &PhotocraftApp) -> [[[f32; 4]; 2]; 2] {
    let t = &app.session.tools;
    let current = [t.foreground, t.background];
    if t.mask_targeted { [t.other_colors, current] } else { [current, t.other_colors] }
}

fn persist_colors(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let now = colors(app);
    if app.prefs_rt.tool_colors == Some(now) && app.prefs_rt.tool_cmyk_picks == app.session.tools.cmyk_picks {
        return;
    }
    app.prefs_rt.tool_colors = Some(now);
    app.prefs_rt.tool_cmyk_picks = app.session.tools.cmyk_picks.clone();
    let saved = photocraft_engine::ToolState::saved_colors(now, &app.prefs_rt.tool_cmyk_picks);
    app.session.prefs.edit(|p| p.tool_colors = photocraft_engine::prefs::ToolColors(saved));
    // Panels and shortcuts can change colours after prefs_ui::tick already ran this frame.
    ctx.request_repaint();
}

fn restore_colors(app: &mut PhotocraftApp) {
    let saved = &app.session.prefs().tool_colors;
    let mut pairs = colors(app);
    let [[foreground, background], [mask_foreground, mask_background]] = &mut pairs;
    // Each colour under its preference path (Preferences › Tool Colors).
    let paths = [
        ("toolColors.foreground", foreground),
        ("toolColors.background", background),
        ("toolColors.maskForeground", mask_foreground),
        ("toolColors.maskBackground", mask_background),
    ];
    for (path, color) in paths {
        let key = path.strip_prefix("toolColors.").unwrap_or(path);
        if let Some(v) = saved.get(key) {
            match serde_json::from_value::<[f32; 4]>(v.clone()) {
                Ok(c) if c.iter().all(|x| x.is_finite()) => *color = c,
                _ => log::warn!("Preferences: ignoring the remembered `{key}` colour"),
            }
        }
    }
    // Parse only a bounded number of valid entries, newest first, before mutating the session.
    // `remember_cmyk` deduplicates entries and enforces the same bound as during a live session.
    let picks: Option<CmykPicks> = saved.get("cmykPicks").and_then(Value::as_array).map(|entries| {
        entries
            .iter()
            .rev()
            .filter_map(|v| match serde_json::from_value::<([f32; 3], [f32; 4])>(v.clone()) {
                Ok((rgb, inks)) if rgb.iter().all(|x| x.is_finite()) && inks.iter().all(|x| x.is_finite() && (0.0..=1.0).contains(x)) => Some((rgb, inks)),
                _ => {
                    log::warn!("Preferences: ignoring a malformed remembered CMYK colour");
                    None
                }
            })
            .take(photocraft_engine::ToolState::CMYK_PICKS)
            .collect()
    });
    let [pixels, mask] = pairs;
    let (current, other) = if app.session.tools.mask_targeted { (mask, pixels) } else { (pixels, mask) };
    [app.session.tools.foreground, app.session.tools.background] = current;
    app.session.tools.other_colors = other;
    if let Some(picks) = picks {
        app.session.tools.cmyk_picks.clear();
        for (rgb, inks) in picks.into_iter().rev() {
            app.session.tools.remember_cmyk(rgb, inks);
        }
    }
    app.prefs_rt.tool_cmyk_picks = app.session.tools.cmyk_picks.clone();
    app.prefs_rt.tool_colors = Some(pairs);
}

/// Remember the tools used last when the list changes, whether or not the selected tool did: a
/// tool picked during a drag that ends on the tool it began with changes only the list.
fn persist_group_tools(app: &mut PhotocraftApp, ctx: &egui::Context, tool: Tool) {
    // Noted here as well: no frame follows a tool picked right before quitting.
    app.ui.remember_group_tool(tool);
    if app.prefs_rt.group_tools.as_ref() == Some(&app.ui.group_tools) {
        return;
    }
    app.prefs_rt.group_tools = Some(app.ui.group_tools.clone());
    if let Ok(now) = serde_json::to_value(&app.ui.group_tools)
        && app.session.prefs().group_tools != now
    {
        app.session.prefs.edit(|p| p.group_tools = now);
        // As for the selected tool: prefs_ui::tick ran before the panels.
        ctx.request_repaint();
    }
}

/// Remember the painting tools' own brushes when they change (a compare per frame otherwise:
/// tip bitmaps are shared, so equal brushes compare by pointer). A brush that can't be remembered
/// yet (a sampled tip too big to embed, before the preset store takes it) keeps what was saved.
fn persist_brushes(app: &mut PhotocraftApp) {
    let (owner, brushes) = (saved_owner(app), &app.ui.tool_brushes);
    if app.prefs_rt.tool_brushes.as_ref().is_some_and(|(o, b)| *o == owner && b == brushes) {
        return;
    }
    let old = app.session.prefs().tool_brushes.get("brushes").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut saved = Map::new();
    for (tool, brush) in brushes {
        // The owner's brush is the session brush, remembered as `last_brush`.
        if *tool == owner {
            continue;
        }
        let Ok(Value::String(key)) = serde_json::to_value(tool) else { continue };
        if let Some(v) = photocraft_engine::brush_cmds::remembered_settings(brush).or_else(|| old.get(&key).cloned()) {
            saved.insert(key, v);
        }
    }
    // Nothing to remember while the Brush owns the session brush and no other tool has its own.
    let now = match serde_json::to_value(owner) {
        Ok(o) if !saved.is_empty() || owner != Tool::Brush => json!({"owner": o, "brushes": saved}),
        _ => Value::Null,
    };
    app.prefs_rt.tool_brushes = Some((owner, brushes.clone()));
    if app.session.prefs().tool_brushes != now {
        app.session.prefs.edit(|p| p.tool_brushes = now);
    }
}

/// The tool whose brush `last_brush` holds: the one that owns the session brush, once
/// `last_brush::persist` has saved that brush. Until then (a sampled tip too big to embed, before
/// the preset store takes it) `last_brush` still holds the previously saved owner's brush, so
/// that owner stands; with none saved it is the Brush, as `toolBrushes` is null then.
fn saved_owner(app: &PhotocraftApp) -> Tool {
    let t = &app.session.tools;
    if t.brush_rev == t.brush_saved_rev {
        return app.ui.brush_tool;
    }
    app.session.prefs().tool_brushes.get("owner").and_then(|v| serde_json::from_value::<Tool>(v.clone()).ok()).unwrap_or(Tool::Brush)
}

/// Restore the painting tools' own brushes and which tool the session brush (`last_brush`)
/// belongs to. Unknown tools, tools without a brush and brushes that don't read back are skipped
/// with a warning: such a tool starts from the current brush, as on a first launch.
fn restore_brushes(app: &mut PhotocraftApp) {
    let saved = app.session.prefs().tool_brushes.clone();
    if !saved.is_null() && !saved.is_object() {
        log::warn!("Preferences: the remembered tool brushes are not an object; using the defaults");
    }
    let tool = |v: Value| serde_json::from_value::<Tool>(v).ok().filter(|t| crate::paint_mouse::has_brush_picker(*t));
    if let Some(v) = saved.get("owner") {
        match tool(v.clone()) {
            Some(owner) => app.ui.brush_tool = owner,
            None => log::warn!("Preferences: ignoring the remembered brush owner: {v}"),
        }
    }
    let owner = app.ui.brush_tool;
    let mut brushes: Vec<(Tool, BrushSettings)> = Vec::new();
    for (k, v) in saved.get("brushes").and_then(Value::as_object).into_iter().flatten() {
        let Some(t) = tool(Value::String(k.clone())).filter(|t| *t != owner) else {
            log::warn!("Preferences: ignoring the remembered brush of `{k}`");
            continue;
        };
        match photocraft_engine::brush_cmds::restored_settings(&app.session, v) {
            Some(b) => brushes.push((t, b)),
            None => log::warn!("Preferences: ignoring the remembered brush of `{k}`: it doesn't read back"),
        }
    }
    app.ui.tool_brushes = brushes;
    app.prefs_rt.tool_brushes = Some((owner, app.ui.tool_brushes.clone()));
}

/// Flush the final tool state on normal exit, even if no further frame ran after a change.
/// The brush first, as in a frame, so the saved brush owner matches the saved brush.
pub(crate) fn save_on_exit(app: &mut PhotocraftApp) -> Result<(), String> {
    let ctx = app.ctx().clone();
    crate::last_brush::persist(app, &ctx);
    persist(app, &ctx);
    crate::prefs_ui::save_preferences(app)
}

/// Restore the tools used last, keeping their order. Anything that isn't a tool is skipped with a
/// warning: the rest of the list still holds.
fn restore_group_tools(app: &mut PhotocraftApp) {
    let saved = app.session.prefs().group_tools.clone();
    if !saved.is_null() && !saved.is_array() {
        log::warn!("Preferences: the remembered group tools are not a list; using the defaults");
    }
    for v in saved.as_array().into_iter().flatten().rev() {
        match serde_json::from_value::<Tool>(v.clone()) {
            Ok(tool) => app.ui.remember_group_tool(tool),
            Err(_) => log::warn!("Preferences: ignoring the remembered group tool {v}"),
        }
    }
}

/// Restore the selected tool before the first frame and its options field by field.
pub fn restore(app: &mut PhotocraftApp) {
    restore_group_tools(app);
    let saved = app.session.prefs().selected_tool.clone();
    if !saved.is_null() {
        match serde_json::from_value::<Tool>(saved) {
            Ok(tool) => app.ui.tool = tool,
            Err(err) => log::warn!("Preferences: ignoring the remembered selected tool: {err}"),
        }
    }
    // Here rather than on the first frame, so a launch alone has nothing to save.
    app.ui.remember_group_tool(app.ui.tool);
    app.prefs_rt.selected_tool = Some(app.ui.tool);
    app.prefs_rt.group_tools = Some(app.ui.group_tools.clone());
    let mut saved = app.session.prefs().tool_options.clone();
    upgrade(&mut saved);
    if let Some(saved) = saved.as_object()
        && let Ok(Value::Object(mut cur)) = serde_json::to_value(&app.ui.tool_options)
    {
        for (k, v) in saved {
            let Some(old) = cur.get(k).cloned() else { continue };
            cur.insert(k.clone(), v.clone());
            if !reads_back(&cur, k) {
                log::warn!("Preferences: ignoring the remembered tool option `{k}`: {v}");
                cur.insert(k.clone(), old);
            }
        }
        if let Ok(mut o) = serde_json::from_value(Value::Object(cur)) {
            sanitize(&mut o);
            app.ui.tool_options = o;
        }
    } else if !saved.is_null() {
        log::warn!("Preferences: the remembered tool options are not an object; using the defaults");
    }
    app.prefs_rt.tool_options = Some(app.ui.tool_options.clone());
    restore_colors(app);
    restore_brushes(app);
}

/// Moves the options an older build saved under their old keys into the sections that replaced
/// them, in an object of tool options: the saved `toolOptions` at launch (`restore`) and a tool
/// preset's (`select_tool_preset`). Each old key is read once and removed. A key the new section
/// already holds wins, and only the keys that were read are written, so a preset that stored
/// `crop_ratio` alone merges into `crop.ratio` without resetting the other crop fields. A value
/// that does not read is dropped and the field keeps its default, as `restore` treats any bad
/// field.
pub(crate) fn upgrade(value: &mut Value) {
    let Value::Object(o) = value else { return };
    upgrade_crop(o);
    upgrade_eyedropper(o);
    upgrade_magnetic(o);
    upgrade_transform(o);
    if let Some(v) = o.remove("pen_rubber_band").filter(Value::is_boolean) {
        put(o, &["path_options"], "rubberBand", v);
    }
}

/// Moves the brush picker's old view keys into `parts` and `search` (`BrushPickerSettings`), for
/// the saved `Preferences::brush_picker` and any picker view stored beside a preset. `scale`
/// keeps its name.
pub(crate) fn upgrade_picker_view(value: &mut Value) {
    let Value::Object(o) = value else { return };
    for (old, section, key) in
        [("showName", &["parts"][..], "name"), ("showStroke", &["parts"][..], "stroke"), ("showTip", &["parts"][..], "tip"), ("showSearch", &[][..], "search")]
    {
        if let Some(v) = o.remove(old).filter(Value::is_boolean) {
            put(o, section, key, v);
        }
    }
}

/// Writes `value` at `path`/`key` of `o` unless that key is already there. A section that exists
/// but is not an object stays as it is.
fn put(o: &mut Map<String, Value>, path: &[&str], key: &str, value: Value) {
    let mut section = o;
    for name in path {
        let Value::Object(next) = section.entry(*name).or_insert_with(|| Value::Object(Map::new())) else { return };
        section = next;
    }
    section.entry(key).or_insert(value);
}

fn upgrade_crop(o: &mut Map<String, Value>) {
    if let Some(Value::String(text)) = o.remove("crop_ratio") {
        put(o, &["crop"], "ratio", ratio_from_text(&text));
    }
    let (width, height) = (o.remove("crop_custom_ratio_width"), o.remove("crop_custom_ratio_height"));
    if width.is_some() || height.is_some() {
        let side = |v: Option<Value>| v.and_then(|v| v.as_f64()).filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(0.0);
        put(o, &["crop"], "ratioFields", json!([side(width), side(height)]));
    }
    if let Some(v) = o.remove("crop_delete").filter(Value::is_boolean) {
        put(o, &["crop"], "deletePixels", v);
    }
    if let Some(v) = o.remove("crop_content_aware").and_then(|v| v.as_bool()) {
        put(o, &["crop"], "fill", json!(if v { "contentAware" } else { "background" }));
    }
    // The guide, its show rule, orientation and the shield kept their keys and values.
    for (old, key) in
        [("crop_overlay", "guide"), ("crop_overlay_show", "guideShow"), ("crop_overlay_orientation", "guideOrientation"), ("crop_shield", "shield")]
    {
        if let Some(v) = o.remove(old).filter(|v| !v.is_null()) {
            put(o, &["crop"], key, v);
        }
    }
    // The Crop tool and Perspective Crop shared one set of size fields; each has its own now.
    let size = [
        ("width", o.remove("crop_width").and_then(|v| v.as_str().and_then(length_from_text))),
        ("height", o.remove("crop_height").and_then(|v| v.as_str().and_then(length_from_text))),
        ("resolution", o.remove("crop_resolution").and_then(|v| v.as_str().map(str::trim).filter(|t| !t.is_empty()).map(Value::from))),
        (
            "resolutionUnit",
            o.remove("crop_resolution_unit").and_then(|v| match v.as_str()? {
                "px/in" => Some(json!("pixelsPerInch")),
                "px/cm" => Some(json!("pixelsPerCentimetre")),
                _ => None,
            }),
        ),
    ];
    for (key, value) in size {
        if let Some(value) = value {
            put(o, &["crop", "size"], key, value.clone());
            put(o, &["perspective_crop"], key, value);
        }
    }
}

/// `crop_ratio`'s saved text as a `CropRatio` value: "" Free, "original", "whr" (W x H x
/// Resolution) and "w:h" with two positive numbers; anything else is Free.
fn ratio_from_text(text: &str) -> Value {
    match text {
        "original" => json!("original"),
        "whr" => json!("size"),
        _ => {
            let sides = text.split_once(':').and_then(|(w, h)| {
                let side = |s: &str| s.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0 && *v <= RATIO_MAX);
                Some((side(w)?, side(h)?))
            });
            match sides {
                Some((width, height)) => json!({"sides": {"width": width, "height": height}}),
                None => json!("free"),
            }
        }
    }
}

/// A saved length ("4 in", "10,5 cm", "1024") as a `CropLength` value: a positive number up to
/// `FIELD_MAX` and an optional unit suffix, pixels when there is none. `None` for empty text, a
/// number out of range and an unknown unit.
fn length_from_text(text: &str) -> Option<Value> {
    let text = text.trim();
    if text.is_empty() || text.len() > 64 {
        return None;
    }
    let split = text.find(|c: char| c.is_alphabetic() || c == '"').unwrap_or(text.len());
    let (number, suffix) = (text.get(..split)?.trim(), text.get(split..)?.trim());
    let value: f64 = number.replace(',', ".").parse().ok().filter(|v: &f64| v.is_finite() && *v > 0.0 && *v <= FIELD_MAX)?;
    let unit = match suffix.to_ascii_lowercase().as_str() {
        "" | "px" | "pixel" | "pixels" => "pixels",
        "in" | "\"" | "inch" | "inches" => "inches",
        "cm" | "centimeter" | "centimeters" | "centimetre" | "centimetres" => "centimetres",
        "mm" | "millimeter" | "millimeters" | "millimetre" | "millimetres" => "millimetres",
        "pt" | "point" | "points" => "points",
        "pica" | "picas" => "picas",
        _ => return None,
    };
    Some(json!({"value": value, "unit": unit}))
}

fn upgrade_eyedropper(o: &mut Map<String, Value>) {
    let size = o.remove("eyedropper_size").and_then(|v| v.as_u64()).and_then(|px| u32::try_from(px).ok()).and_then(SampleSize::from_pixels);
    if let Some(size) = size.and_then(|s| serde_json::to_value(s).ok()) {
        put(o, &["eyedropper"], "size", size);
    }
    let source = o.remove("eyedropper_sample").and_then(|v| v.as_str().and_then(SampleSource::from_id));
    if let Some(source) = source.and_then(|s| serde_json::to_value(s).ok()) {
        put(o, &["eyedropper"], "source", source);
    }
    if let Some(v) = o.remove("eyedropper_ring").filter(Value::is_boolean) {
        put(o, &["eyedropper"], "showRing", v);
    }
}

fn upgrade_magnetic(o: &mut Map<String, Value>) {
    for (old, key) in [("magnetic_width", "width"), ("magnetic_contrast", "contrast"), ("magnetic_frequency", "frequency")] {
        if let Some(v) = o.remove(old).filter(Value::is_number) {
            put(o, &["magnetic"], key, v);
        }
    }
    if let Some(v) = o.remove("magnetic_pressure").filter(Value::is_boolean) {
        put(o, &["magnetic"], "pressureWidth", v);
    }
}

fn upgrade_transform(o: &mut Map<String, Value>) {
    for (old, key) in [("transform_reference_point", "showReferencePoint"), ("transform_relative_position", "relativePosition")] {
        if let Some(v) = o.remove(old).filter(Value::is_boolean) {
            put(o, &["transform"], key, v);
        }
    }
    // The saved name was one of the General preference's seven; the Transform bar offers three,
    // so every bicubic variant reads as Bicubic.
    let interpolation = o.remove("transform_interpolation").and_then(|v| {
        Some(match v.as_str()? {
            "nearestNeighbor" => "nearestNeighbor",
            "bilinear" => "bilinear",
            "bicubic" | "bicubicAutomatic" | "bicubicSmoother" | "bicubicSharper" | "preserveDetails" => "bicubic",
            _ => return None,
        })
    });
    if let Some(name) = interpolation {
        put(o, &["transform"], "interpolation", json!(name));
    }
}

/// The values Width and Height take under a marquee or Slice tool Style: a size in px for Fixed
/// Size, else a ratio.
pub(crate) fn style_range(style: &str) -> std::ops::RangeInclusive<f32> {
    if style == "fixedSize" { 1.0..=ConstraintFields::SIZE_MAX as f32 } else { 0.001..=999.0 }
}

/// Bring remembered options into the ranges their options-bar fields allow, so a hand-edited or
/// damaged file can't start a tool with a value its own controls could never set (a 1e30 px
/// feather, a 0-sided polygon). A NaN (JSON can't hold one, but it costs nothing to rule out)
/// takes the default.
pub(crate) fn sanitize(o: &mut ToolOptions) {
    let d = ToolOptions::default();
    let fit = |v: &mut f32, lo: f32, hi: f32, def: f32| *v = if v.is_nan() { def } else { v.clamp(lo, hi) };
    fit(&mut o.tolerance, 0.0, 255.0, d.tolerance);
    fit(&mut o.feather, 0.0, 1000.0, d.feather);
    fit(&mut o.fill_opacity, 1.0, 100.0, d.fill_opacity);
    fit(&mut o.type_size, 0.1, 1296.0, d.type_size);
    fit(&mut o.pattern_stamp_scale, 1.0, 1000.0, d.pattern_stamp_scale);
    fit(&mut o.pattern_stamp_angle, -180.0, 180.0, d.pattern_stamp_angle);
    fit(&mut o.patch_structure, 1.0, 7.0, d.patch_structure);
    fit(&mut o.patch_color, 0.0, 10.0, d.patch_color);
    fit(&mut o.cam_structure, 1.0, 7.0, d.cam_structure);
    fit(&mut o.cam_color, 0.0, 10.0, d.cam_color);
    fit(&mut o.exposure, 1.0, 100.0, d.exposure);
    fit(&mut o.strength, 1.0, 100.0, d.strength);
    fit(&mut o.stroke_width, 0.0, 288.0, d.stroke_width);
    fit(&mut o.corner_radius, *RADIUS_RANGE.start() as f32, *RADIUS_RANGE.end() as f32, d.corner_radius);
    // The framework's `LINE_WEIGHT_RANGE` is the one source of the weight bounds.
    fit(&mut o.line_weight, *LINE_WEIGHT_RANGE.start() as f32, *LINE_WEIGHT_RANGE.end() as f32, d.line_weight);
    fit(&mut o.curve_fit, 0.5, 10.0, d.curve_fit);
    // The marquee's and the Slice tool's Width and Height are a ratio or a size in px, as their
    // Style says.
    let (lo, hi) = style_range(&o.marquee_style).into_inner();
    fit(&mut o.marquee_width, lo, hi, d.marquee_width);
    fit(&mut o.marquee_height, lo, hi, d.marquee_height);
    if !matches!(o.slice_style.as_str(), "normal" | "fixedRatio" | "fixedSize") {
        o.slice_style = d.slice_style.clone();
    }
    let (lo, hi) = style_range(&o.slice_style).into_inner();
    fit(&mut o.slice_width, lo, hi, d.slice_width);
    fit(&mut o.slice_height, lo, hi, d.slice_height);
    if o.slice_style == "fixedSize" {
        (o.slice_width, o.slice_height) = (o.slice_width.round(), o.slice_height.round());
    }
    for side in &mut o.crop.ratio_fields {
        *side = if side.is_nan() { 0.0 } else { side.clamp(0.0, f64::MAX) };
    }
    fit(&mut o.magic_eraser_opacity, 1.0, 100.0, d.magic_eraser_opacity);
    fit(&mut o.bg_tolerance, 0.0, 100.0, d.bg_tolerance);
    fit(&mut o.cr_tolerance, 0.0, 100.0, d.cr_tolerance);
    let (width_min, width_max) = MagneticSettings::WIDTH_RANGE;
    fit(&mut o.magnetic.width, width_min, width_max, d.magnetic.width);
    fit(&mut o.magnetic.contrast, 1.0, 100.0, d.magnetic.contrast);
    fit(&mut o.magnetic.frequency, 0.0, 100.0, d.magnetic.frequency);
    fit(&mut o.red_eye_pupil_size, 1.0, 100.0, d.red_eye_pupil_size);
    fit(&mut o.red_eye_darken, 0.0, 100.0, d.red_eye_darken);
    o.polygon_sides = o.polygon_sides.clamp(3, 100);
    if o.crop.guide_orientation >= o.crop.guide.orientations() {
        o.crop.guide_orientation = 0;
    }
    // PLAN: `StrokeOptions::valid` is gone; the stroke built from the options is checked through `validate`, as `valid` did (owner of StrokeOptions).
    if o.shape_stroke.stroke(3.0, photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK)).validate().is_err() {
        o.shape_stroke = d.shape_stroke;
    }
}

/// Does `opts` deserialize, with field `k` coming back as a value? A number out of an `f32`'s
/// range reads as infinity, which writes back as `null`. A section saved as a partial object (an
/// upgraded old key) comes back with the nulls of its unset `Option` fields, which the default
/// has too.
fn reads_back(opts: &Map<String, Value>, k: &str) -> bool {
    let Ok(o) = serde_json::from_value::<ToolOptions>(Value::Object(opts.clone())) else { return false };
    let Ok(Value::Object(back)) = serde_json::to_value(&o) else { return false };
    let default_nulls = serde_json::to_value(ToolOptions::default()).ok().and_then(|d| d.get(k).map(nulls)).unwrap_or(0);
    back.get(k).is_some_and(|v| nulls(v) <= opts.get(k).map_or(0, nulls).max(default_nulls))
}

fn nulls(v: &Value) -> usize {
    match v {
        Value::Null => 1,
        Value::Array(a) => a.iter().map(nulls).sum(),
        Value::Object(o) => o.values().map(nulls).sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_tool_and_non_default_options_roundtrip() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.tool = Tool::Lasso;
        app.ui.tool_options.tolerance = 42.5;
        let ctx = app.ctx().clone();
        persist(&mut app, &ctx);

        let saved = app.session.prefs();
        assert_eq!(saved.selected_tool, serde_json::to_value(Tool::Lasso).unwrap());
        assert_eq!(saved.tool_options["tolerance"], Value::from(42.5));
        assert!(saved.tool_options.get("contiguous").is_none(), "unchanged defaults are not written");

        let mut restored = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        restored.session.prefs.edit(|p| {
            p.selected_tool = saved.selected_tool.clone();
            p.tool_options = saved.tool_options.clone();
        });
        restore(&mut restored);
        assert_eq!(restored.ui.tool, Tool::Lasso);
        assert_eq!(restored.ui.tool_options.tolerance, 42.5);
    }

    /// Preferences and tool presets saved before the options moved into the framework's types
    /// still load: each old key lands in its new section, a new key beside an old one wins, a
    /// preset's partial object stays partial and the next save writes no old key.
    #[test]
    fn old_tool_option_keys_load_into_the_new_types() {
        use craft::behavior::geometry::crop::{CropGuide, GuideShow};
        use craft::draw::ui::crop::{CropFill, CropLength, CropRatio, ResolutionUnit};
        use craft::ui::runtime::env::UnitKind;

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.prefs.edit(|p| {
            p.tool_options = json!({
                "crop_ratio": "2:3",
                "crop": {"ratio": "original", "deletePixels": false},
                "crop_content_aware": true,
                "crop_overlay": "triangle",
                "crop_overlay_show": "never",
                "crop_overlay_orientation": 1,
                "crop_width": "4 in",
                "crop_height": "600",
                "crop_resolution": "300",
                "crop_resolution_unit": "px/cm",
                "eyedropper_size": 5,
                "eyedropper_sample": "current",
                "magnetic_width": 30.0,
                "transform_reference_point": true,
                "pen_rubber_band": true,
            })
        });
        restore(&mut app);
        let o = &app.ui.tool_options;
        assert_eq!(o.crop.ratio, CropRatio::Original, "the new key wins over the old one");
        assert!(!o.crop.delete_pixels);
        assert_eq!(o.crop.fill, CropFill::ContentAware);
        assert_eq!((o.crop.guide, o.crop.guide_show, o.crop.guide_orientation), (CropGuide::Triangle, GuideShow::Never, 1));
        assert_eq!(o.crop.size.width, Some(CropLength::new(4.0, UnitKind::Inches)));
        assert_eq!(o.crop.size.height, Some(CropLength::new(600.0, UnitKind::Pixels)));
        assert_eq!((o.crop.size.resolution.as_str(), o.crop.size.resolution_unit), ("300", ResolutionUnit::PixelsPerCentimetre));
        assert_eq!(o.perspective_crop, o.crop.size, "Perspective Crop took the fields the two tools shared");
        assert_eq!((o.eyedropper.size.pixels(), o.eyedropper.source), (5, SampleSource::Current));
        assert_eq!(o.magnetic.width, 30.0);
        assert!(o.transform.show_reference_point);
        assert!(o.path_options.rubber_band);

        // A save after any change writes the new keys only.
        app.ui.tool_options.tolerance = 1.0;
        let ctx = app.ctx().clone();
        persist(&mut app, &ctx);
        let saved = app.session.prefs().tool_options.clone();
        for old in ["crop_ratio", "crop_width", "crop_overlay", "eyedropper_size", "magnetic_width", "transform_reference_point", "pen_rubber_band"] {
            assert!(saved.get(old).is_none(), "{old} is written back");
        }
        assert!(saved.get("crop").is_some() && saved.get("eyedropper").is_some());

        // A preset stores only what it changed: `crop_ratio` alone becomes `crop.ratio` alone.
        let mut preset = json!({"crop_ratio": "2:3"});
        upgrade(&mut preset);
        assert_eq!(preset, json!({"crop": {"ratio": {"sides": {"width": 2.0, "height": 3.0}}}}));
        let mut preset = json!({"crop_ratio": "whr", "crop_width": "1 furlong"});
        upgrade(&mut preset);
        assert_eq!(preset, json!({"crop": {"ratio": "size"}}), "a length that does not read is dropped");
    }

    #[test]
    fn restore_ignores_unknown_and_malformed_tool_options() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let original_tool = app.ui.tool;
        let default_contiguous = ToolOptions::default().contiguous;
        app.session.prefs.edit(|p| {
            p.selected_tool = Value::from("not-a-tool");
            p.tool_options = serde_json::json!({
                "tolerance": 42.5,
                "contiguous": "not a boolean",
                "future_option": true
            });
        });

        restore(&mut app);
        assert_eq!(app.ui.tool, original_tool, "an invalid selected tool leaves the current tool intact");
        assert_eq!(app.ui.tool_options.tolerance, 42.5, "valid fields still restore");
        assert_eq!(app.ui.tool_options.contiguous, default_contiguous, "a malformed field falls back to its default");
    }
}
