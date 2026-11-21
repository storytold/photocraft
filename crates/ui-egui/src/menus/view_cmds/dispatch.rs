use craft::ui::shell::ScreenMode;
use photocraft_engine::prefs::FontPreview;

use super::arrange::{arrange, command as arrange_command, layout_checked};
use super::dialogs::front;
use super::options::{pixel_aspect_ratio, show_slot, snap_slot};
use super::screen_mode_names;
use super::{LANGUAGE, PIXEL_ASPECTS, PhotocraftApp, SCREEN_MODES, Show, SnapTo, Value, json};

/// The ids `panel_docking::command` runs for Window › panels and the Type menu's Character and Paragraph.
const PANEL_IDS: [&str; 17] = [
    "window.panel.layerComps",
    "window.panel.layers",
    "window.panel.history",
    "window.panel.navigator",
    "window.panel.properties",
    "window.panel.color",
    "window.panel.actions",
    "window.panel.channels",
    "window.panel.paths",
    "window.panel.character",
    "window.panel.paragraph",
    "window.panel.info",
    "window.panel.histogram",
    "window.panel.swatches",
    "window.panel.adjustments",
    "type.panels.character",
    "type.panels.paragraph",
];

/// The Window item and its shell command for the contextual task bar, which is a flag of the canvas tools, not a
/// dock panel.
fn is_context_task_bar(id: &str) -> bool {
    matches!(id, "window.panel.contextualTaskBar" | "window.toggle.contextualTaskBar")
}

/// Ids handled here (live menu items).
pub fn handles(id: &str) -> bool {
    if let Some(k) = id.strip_prefix("view.show.") {
        return show_slot(&mut Show::default(), k).is_some() || k == "all";
    }
    if let Some(k) = id.strip_prefix("view.snapTo.") {
        return snap_slot(&mut SnapTo::default(), k).is_some() || k == "all" || k == "none";
    }
    if let Some(k) = id.strip_prefix("view.screenMode.") {
        return SCREEN_MODES.contains(&k) || matches!(k, "cycle" | "cycleBack");
    }
    if let Some(k) = id.strip_prefix("view.pixelAspectRatio.") {
        return PIXEL_ASPECTS.contains(&k);
    }
    if let Some(k) = id.strip_prefix("type.fontPreviewSize.") {
        return FontPreview::parse(k).is_some();
    }
    if let Some(k) = id.strip_prefix("type.languageOptions.") {
        return LANGUAGE.contains(&k) || k == "middleEasternAndSouthAsianComposer";
    }
    if let Some(k) = id.strip_prefix("window.arrange.") {
        return arrange_command(k).is_some();
    }
    PANEL_IDS.contains(&id)
        || is_context_task_bar(id)
        || matches!(
            id,
            "view.extras" | "view.flipHorizontal" | "view.pixelAspectRatioCorrection" | "view.patternPreview" | "view.pixelArtPreview" | "view.rotateView"
        )
}

pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    is_enabled_with_engine(app, id, || app.session.is_enabled(id))
}

/// Keep picker enablement testable with the engine's native filesystem check disabled.
pub(super) fn is_enabled_with_engine(app: &PhotocraftApp, id: &str, engine_enabled: impl FnOnce() -> bool) -> Option<bool> {
    if !handles(id) && !wraps(id) {
        return None;
    }
    let doc = app.session.active().is_some();
    Some(match id {
        // Full Screen Mode shows a document, so without one it is unavailable, as in Photoshop
        // (#9349), and so is the cycle through it: a full screen mode left without a document is
        // Standard again before the next pass (`leave_full_screen_without_document`).
        "view.screenMode.cycle" | "view.screenMode.cycleBack" => app.ui.text_edit.is_none() && doc,
        "view.screenMode.fullScreen" | "view.screenMode.fullScreenWithMenuBar" => doc,
        "view.rotateView" => doc,
        i if i.starts_with("window.arrange.") => arrange_command(&i["window.arrange.".len()..]).is_some_and(|command| {
            let windows = &app.ui.document_windows;
            command.arrangement().enabled(app.session.documents().len(), windows.docked.len(), windows.floating.len())
        }),
        // Embedded placement reads the picker's bytes on every platform. Linked placement keeps
        // the engine's native filesystem guard because its source must remain reachable.
        "file.placeEmbedded" => doc && app.services.files.is_some(),
        // Batch's dialog is built from an Actions-panel action. Keep it disabled until one
        // exists instead of opening a menu item that can only report "record an action".
        "file.automate.batch" => crate::actions::selected_action(app).is_some() && engine_enabled(),
        #[cfg(target_arch = "wasm32")]
        "file.export.dataSetsAsFiles" => false,
        i if wraps(i) => engine_enabled(),
        _ => true,
    })
}

/// A full screen mode with no document left (the last one closed) goes back to Standard Screen
/// Mode, rather than showing the start screen on the full screen pasteboard (#9349). The shell
/// brings the window to the mode it binds.
pub fn leave_full_screen_without_document(app: &mut PhotocraftApp) {
    if app.ui.view.screen_mode != ScreenMode::Standard && app.session.documents().is_empty() {
        app.ui.view.screen_mode = ScreenMode::Standard;
    }
}

pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let o = &app.ui.view;
    if let Some(k) = id.strip_prefix("view.show.") {
        let mut s = o.show;
        return show_slot(&mut s, k).map(|b| *b);
    }
    if let Some(k) = id.strip_prefix("view.snapTo.") {
        let mut s = o.snap_to;
        return snap_slot(&mut s, k).map(|b| *b);
    }
    if let Some(k) = id.strip_prefix("view.screenMode.") {
        return SCREEN_MODES.contains(&k).then(|| screen_mode_names::name(o.screen_mode) == k);
    }
    if let Some(k) = id.strip_prefix("view.pixelAspectRatio.") {
        return PIXEL_ASPECTS.contains(&k).then(|| o.pixel_aspect == k);
    }
    if let Some(k) = id.strip_prefix("type.fontPreviewSize.") {
        return FontPreview::parse(k).map(|size| app.session.prefs().type_.font_preview == size);
    }
    if let Some(k) = id.strip_prefix("type.languageOptions.") {
        return if k == "middleEasternAndSouthAsianComposer" {
            Some(o.middle_eastern_composer)
        } else {
            LANGUAGE.contains(&k).then(|| o.language_features == k)
        };
    }
    if let Some(k) = id.strip_prefix("window.arrange.")
        && let Some(on) = layout_checked(app, k)
    {
        return Some(on);
    }
    if is_context_task_bar(id) {
        return Some(app.ui.panels.context_task_bar);
    }
    if PANEL_IDS.contains(&id) {
        return Some(crate::panel_docking::toggle_panel(id).is_some_and(|panel| crate::panel_docking::is_checked(app, panel)));
    }
    if let Some(c) = type_checked(app, id) {
        return Some(c);
    }
    Some(match id {
        "view.extras" => o.extras,
        "view.flipHorizontal" => o.flip_horizontal,
        "view.pixelAspectRatioCorrection" => o.pixel_aspect_correction,
        "view.patternPreview" => o.pattern_preview,
        "view.pixelArtPreview" => o.pixel_art_preview,
        _ => return None,
    })
}

/// Checkmarks of the Type menu's anti-aliasing, orientation and OpenType items, for the layer the
/// options bar and Properties show: the one being edited, else the active or a selected type layer.
/// Anti-aliasing and orientation stay check items (`Some(false)`) without a type layer: a native
/// menu can't change an item's kind in place.
fn type_checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let t = crate::type_tool::target_text(app);
    if let Some(k) = id.strip_prefix("type.antiAlias.") {
        return Some(t.is_some_and(|t| photocraft_engine::type_extra_cmds::aa_name(t.antialias) == k));
    }
    if let Some(k) = id.strip_prefix("type.orientation.") {
        return Some(t.is_some_and(|t| (t.orientation == photocraft_doc::text::Orientation::Vertical) == (k == "vertical")));
    }
    let t = t?;
    // The style where the menu reads: the selection start while editing a range (as the Character
    // panel shows it), else the layer's first run.
    let start = crate::type_tool::target(app).and_then(|(_, range)| range).map_or(0, |r| r[0]);
    if let Some(key) = id.strip_prefix("type.openType.numerals.") {
        let want = photocraft_engine::type_extra_cmds::NumeralsStyle::from_id(key)?;
        return Some(photocraft_engine::type_extra_cmds::numerals_style_at(t, start) == Some(want));
    }
    let tag = match id.strip_prefix("type.openType.")? {
        "standardLigatures" => "liga",
        "contextualAlternates" => "calt",
        "discretionaryLigatures" => "dlig",
        "swash" => "swsh",
        "oldstyle" => "onum",
        "stylisticAlternates" => "salt",
        "titlingAlternates" => "titl",
        "ornaments" => "ornm",
        "ordinals" => "ordn",
        "fractions" => "frac",
        set => stylistic_set_tag(set.strip_prefix("stylisticSets.set")?)?,
    };
    Some(photocraft_engine::type_extra_cmds::feature_on_at(t, start, tag))
}

/// `ss01`..`ss20` for the digits of a Stylistic Sets item id (`1`..`20`, no leading zeros).
fn stylistic_set_tag(n: &str) -> Option<&'static str> {
    if n.starts_with('0') || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    photocraft_engine::type_extra_cmds::STYLISTIC_SET_TAGS.get(n.parse::<usize>().ok()?.checked_sub(1)?).copied()
}

/// Engine commands that get a dialog or file picker here when invoked without parameters.
fn wraps(id: &str) -> bool {
    matches!(
        id,
        "file.openAs"
            | "file.openAsSmartObject"
            | "file.openExrParts"
            | "file.saveACopy"
            | "file.placeEmbedded"
            | "file.placeLinked"
            | "layer.smartObjects.replaceContents"
            | "layer.smartObjects.exportContents"
            | "layer.smartObjects.relinkToFile"
            | "layer.smartObjects.convertToLinked"
            | "file.closeAll"
            | "file.closeOthers"
            | "file.fileInfo"
            | "file.automate.fitImage"
            | "file.automate.conditionalModeChange"
            | "file.automate.batch"
            | "file.scripts.imageProcessor"
            | "file.scripts.loadFilesIntoStack"
            | "file.automate.photomerge"
            | "file.automate.mergeToHdrPro"
            | "file.automate.lensCorrection"
            | "file.export.layersToFiles"
            | "file.export.layerCompsToFiles"
            | "file.export.artboardsToFiles"
            | "file.export.artboardsToPdf"
            | "file.export.dataSetsAsFiles"
            | "layer.new.artboard"
            | "file.export.colorLookupTables"
            | "view.newGuideLayout"
            | "type.warpText"
            | "type.pasteLoremIpsum"
            | "image.applyImage"
            | "image.calculations"
            | "select.saveSelection"
            | "select.loadSelection"
            | "layer.videoLayers.newVideoLayerFromFile"
            | "layer.videoLayers.replaceFootage"
            | "layer.videoLayers.interpretFootage"
    )
}

pub(super) fn no_params(p: &Value) -> bool {
    p.as_object().is_none_or(|o| o.is_empty())
}

/// Run a View/Window/Type shell command, or front an engine command with its dialog. `None` when
/// `id` isn't ours.
pub fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if let Some(result) = crate::panel_docking::command(app, id, params) {
        return Some(result);
    }
    if wraps(id) {
        return front(app, ctx, id, params);
    }
    if !handles(id) {
        return None;
    }
    if crate::shortcut_dispatch::is_screen_mode(id) {
        return Some(screen_mode(app, ctx, id));
    }
    Some(run(app, id, params))
}

/// A View › Screen Mode command from a menu, the toolbar's list or automation: fired to the window's shell, as its
/// key fires it, which applies it in its next pass. Refused where the menu greys it: Full Screen Mode shows a
/// document (#9349).
fn screen_mode(app: &PhotocraftApp, ctx: &egui::Context, id: &str) -> Result<Value, String> {
    if is_enabled(app, id) == Some(false) {
        return Err(if app.session.documents().is_empty() { "Full Screen Mode needs an open document".into() } else { format!("{id} is not available now") });
    }
    match crate::shortcut_dispatch::fire_bridged(ctx, id) {
        Some(true) => Ok(Value::Null),
        _ => Err(format!("{id} is unavailable: no window shell runs it")),
    }
}

fn flag_param(p: &Value, cur: bool) -> bool {
    p.get("on").and_then(Value::as_bool).unwrap_or(!cur)
}

fn run(app: &mut PhotocraftApp, id: &str, p: &Value) -> Result<Value, String> {
    if id == "view.rotateView" {
        return rotate_command(app, p);
    }
    // One store: the font picker reads the same preference.
    if let Some(k) = id.strip_prefix("type.fontPreviewSize.") {
        app.run("prefs.set", json!({"values": {"type.fontPreview": k}}))?;
        return Ok(json!(k));
    }
    if is_context_task_bar(id) {
        let bar = &mut app.ui.panels.context_task_bar;
        *bar = flag_param(p, *bar);
        return Ok(json!(*bar));
    }
    let o = &mut app.ui.view;
    if let Some(k) = id.strip_prefix("view.show.") {
        if k == "all" {
            o.show = Show { layer_edges: true, mesh: true, ..Show::default() };
            o.extras = true;
            app.ui.extras.grid = true;
            app.ui.extras.guides = true;
            return Ok(json!(true));
        }
        let slot = show_slot(&mut o.show, k).ok_or("unknown Show item")?;
        *slot = flag_param(p, *slot);
        let v = *slot;
        if v {
            // Showing an item turns Extras back on, as in Photoshop.
            o.extras = true;
        }
        return Ok(json!(v));
    }
    if let Some(k) = id.strip_prefix("view.snapTo.") {
        if k == "all" || k == "none" {
            let on = k == "all";
            o.snap_to = SnapTo { guides: on, grid: on, layers: on, slices: on, document_bounds: on };
            return Ok(json!(on));
        }
        let slot = snap_slot(&mut o.snap_to, k).ok_or("unknown Snap To item")?;
        *slot = flag_param(p, *slot);
        return Ok(json!(*slot));
    }
    if let Some(k) = id.strip_prefix("view.pixelAspectRatio.") {
        o.pixel_aspect = k.to_string();
        return Ok(json!({"pixelAspectRatio": pixel_aspect_ratio(k)}));
    }
    if let Some(k) = id.strip_prefix("type.languageOptions.") {
        if k == "middleEasternAndSouthAsianComposer" {
            o.middle_eastern_composer = flag_param(p, o.middle_eastern_composer);
            return Ok(json!(o.middle_eastern_composer));
        }
        o.language_features = k.to_string();
        return Ok(json!(k));
    }
    if let Some(k) = id.strip_prefix("window.arrange.") {
        return arrange(app, k, p);
    }
    match id {
        "view.extras" => {
            o.extras = flag_param(p, o.extras);
            Ok(json!(o.extras))
        }
        "view.flipHorizontal" => {
            o.flip_horizontal = flag_param(p, o.flip_horizontal);
            Ok(json!(o.flip_horizontal))
        }
        "view.pixelAspectRatioCorrection" => {
            o.pixel_aspect_correction = flag_param(p, o.pixel_aspect_correction);
            Ok(json!(o.pixel_aspect_correction))
        }
        "view.patternPreview" => {
            o.pattern_preview = flag_param(p, o.pattern_preview);
            Ok(json!(o.pattern_preview))
        }
        "view.pixelArtPreview" => {
            o.pixel_art_preview = flag_param(p, o.pixel_art_preview);
            Ok(json!(o.pixel_art_preview))
        }
        _ => Err(format!("unhandled {id}")),
    }
}

/// `view.rotateView`: set or add degrees on the active document's camera.
fn rotate_command(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let i = app.session.active_index().ok_or("no document open")?;
    let current = app.ui.views.get(i).map(|v| v.rotation).ok_or("no document open")?;
    let rotation = apply_params(current, p)?;
    let v = app.ui.views.get_mut(i).ok_or("no document open")?;
    v.rotation = rotation;
    Ok(json!({"rotation": v.rotation}))
}

/// `a` in degrees wrapped to (-180, 180]; a number that is not finite is 0.
fn wrap_deg(a: f32) -> f32 {
    if !a.is_finite() {
        return 0.0;
    }
    let mut x = f64::from(a) % 360.0;
    if x > 180.0 {
        x -= 360.0;
    } else if x <= -180.0 {
        x += 360.0;
    }
    x as f32
}

fn finite_number(p: &Value, key: &str) -> Result<Option<f32>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite() && n.abs() <= f64::from(f32::MAX))
            .map(|n| Some(n as f32))
            .ok_or_else(|| format!("{key} must be a finite number representable as f32")),
    }
}

/// Apply `view.rotateView` params to `current`. Empty params, a non-object, or a missing
/// `angle`/`delta`/`reset` are errors (never a silent no-op).
fn apply_params(current: f32, p: &Value) -> Result<f32, String> {
    let obj = p.as_object().ok_or("params must be an object")?;
    for k in obj.keys() {
        if !matches!(k.as_str(), "angle" | "delta" | "reset") {
            return Err(format!("unknown field `{k}`"));
        }
    }
    let reset = match obj.get("reset") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| "reset must be a boolean".to_string())?,
    };
    let angle = finite_number(p, "angle")?;
    let delta = finite_number(p, "delta")?;
    if !reset && angle.is_none() && delta.is_none() {
        return Err("expected angle, delta, or reset".into());
    }
    if reset {
        return Ok(0.0);
    }
    let mut r = angle.unwrap_or(current);
    if let Some(d) = delta {
        r += d;
        if !r.is_finite() {
            return Err("delta overflowed the angle".into());
        }
    }
    Ok(wrap_deg(r))
}
