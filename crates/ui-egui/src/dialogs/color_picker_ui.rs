//! Photoshop's Color Picker (Foreground / Background Color) on the framework's `ColorPickerDialog`.
//!
//! Every opener builds the dialog, opens it on the dialog host and records what it is for in
//! `app.color_targets` ([`PickTarget`]). [`take`] runs once per pass from `dialogs::TAKERS`: it
//! serves the dialog's screen pick, Add to Swatches and canvas eyedropper, previews the picked
//! colour in its parent ([`live_preview`]) and commits an OK to the target ([`commit`]). A hosted
//! dialog's colour row opens a child with [`open_for_request`] and pulls the answer with
//! [`answer_for`]; a window the app holds itself opens one with [`open_for_window`] and pulls the
//! answer with [`answer_for_window`].

use craft::cms::convert::CmykConverter;
use craft::color_ui::ui::{ColorPickerDialog, ColorPickerResult, PickedColor};
use craft::theme::{ModelValues, Srgba};
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::runtime::requests::ColorRequest;
use photocraft_doc::{DocId, LayerId};
use serde_json::{Value, json};
use std::hash::Hash;

use crate::PhotocraftApp;
use crate::shape_fill_ui::ShapeTarget;

/// What a picker's OK sets.
#[derive(Clone, Debug)]
pub(crate) enum PickTarget {
    /// The foreground colour.
    Foreground,
    /// The background colour.
    Background,
    /// Runs `command` with `params` plus `"color"`; `document` is the document the picker opened on.
    Command { command: String, params: Value, document: Option<DocId> },
    /// A shape layer's fill or stroke, or the next shape's colours.
    Shape(ShapeTarget),
    /// The colour of a Solid Color fill layer.
    SolidFill { document: DocId, layer: LayerId },
    /// A Solid Color fill layer created on OK.
    NewSolidFill { document: DocId },
    /// A colour row `key` of the hosted dialog `parent`; the answer waits in [`answer_for`].
    Request { parent: DialogId, key: String },
    /// The Crop shield's custom colour.
    CropShield,
    /// The Artboard tool's custom background.
    ArtboardBackground,
    /// The Note tool's colour for new notes.
    NoteColor,
    /// The pasteboard's custom colour.
    Pasteboard,
    /// A colour row `key` of `window`, which the app holds itself; the answer waits in [`answer_for_window`].
    Window { window: PickWindow, key: String },
}

/// The windows the app holds itself (no parent dialog) that open a Color Picker for a colour of theirs. Each one's
/// picker and answers are its own: one window closing never takes another's picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PickWindow {
    /// The Preferences window's colour rows.
    Preferences,
    /// The Properties panel's colour wells (an adjustment layer's colours).
    Properties,
}

impl PickWindow {
    /// The id under which this window's answers wait in `app.color_answers`.
    fn answers(self) -> DialogId {
        egui::Id::unique(("pc-color-picker-window", self))
    }
}

impl PickTarget {
    /// The dialog key: the result key and the key of an eyedropper answer.
    fn key(&self) -> String {
        match self {
            PickTarget::Foreground => "foreground".into(),
            PickTarget::Background => "background".into(),
            PickTarget::Command { .. } => "command".into(),
            PickTarget::Shape(_) => "shape".into(),
            PickTarget::SolidFill { .. } => "solidFill".into(),
            PickTarget::NewSolidFill { .. } => "newSolidFill".into(),
            PickTarget::Request { key, .. } | PickTarget::Window { key, .. } => key.clone(),
            PickTarget::CropShield => "cropShield".into(),
            PickTarget::ArtboardBackground => "artboardBackground".into(),
            PickTarget::NoteColor => "noteColor".into(),
            PickTarget::Pasteboard => "pasteboard".into(),
        }
    }
}

/// What makes two pickers the same target: the kind and its keys, never the command's params or the
/// colour. The host replaces an open picker only for an equal identity.
#[derive(Hash, Debug)]
enum Identity<'a> {
    Plain(&'static str),
    Request(DialogId, &'a str),
    Window(PickWindow, &'a str),
    Command(&'a str, Option<u64>),
    Shape(bool, Option<u64>),
    SolidFill(u64, u64),
    NewSolidFill(u64),
}

impl PickTarget {
    fn identity(&self) -> Identity<'_> {
        match self {
            PickTarget::Foreground => Identity::Plain("foreground"),
            PickTarget::Background => Identity::Plain("background"),
            PickTarget::CropShield => Identity::Plain("cropShield"),
            PickTarget::ArtboardBackground => Identity::Plain("artboardBackground"),
            PickTarget::NoteColor => Identity::Plain("noteColor"),
            PickTarget::Pasteboard => Identity::Plain("pasteboard"),
            PickTarget::Request { parent, key } => Identity::Request(*parent, key),
            PickTarget::Window { window, key } => Identity::Window(*window, key),
            PickTarget::Command { command, document, .. } => Identity::Command(command, document.map(|d| d.0)),
            PickTarget::Shape(t) => Identity::Shape(t.which == crate::shape_fill_ui::Paint::Stroke, t.layer.map(|l| l.0)),
            PickTarget::SolidFill { document, layer } => Identity::SolidFill(document.0, layer.0),
            PickTarget::NewSolidFill { document } => Identity::NewSolidFill(document.0),
        }
    }
}

/// `"#rrggbb"` of channels in 0..1.
pub fn hex(c: [f32; 3]) -> String {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(c[0]), q(c[1]), q(c[2]))
}

/// The channels of `"#rrggbb"`.
pub fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let h = s.trim().trim_start_matches('#');
    // `from_str_radix` accepts a leading '+', so require plain hex digits first.
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    // `get`, not `[..]`: typed text may be multi-byte, and a byte range can split a character.
    let c = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|v| f32::from(v) / 255.0);
    Some([c(0)?, c(2)?, c(4)?])
}

/// `"#rrggbb"` of a colour.
pub(crate) fn rgb_hex(c: Srgba) -> String {
    hex(c.to_rgb_array())
}

/// The picker's title for the colour called `name`: "Color Picker (Vine Color)".
pub fn title_for(name: &str) -> String {
    crate::i18n::fmt(tl!("Color Picker ({name})"), &[("name", name)])
}

/// CMYK inks (0..1) as the picker's model values (percent).
pub(crate) fn inks_model(inks: [f32; 4]) -> ModelValues {
    ModelValues::new("cmyk", &inks.map(|v| v * 100.0))
}

/// The CMYK inks (0..1) a picked colour was entered as; `None` when it was not entered in CMYK.
pub(crate) fn inks_of(picked: &PickedColor) -> Option<[f32; 4]> {
    let m = picked.model.as_ref().filter(|m| m.model == "cmyk")?;
    let v = m.values;
    Some([v[0] / 100.0, v[1] / 100.0, v[2] / 100.0, v[3] / 100.0])
}

/// Whether two ink sets are the same after the round trip through percent.
pub(crate) fn same_inks(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
}

/// The CMYK converter of the document colour settings; the built-in coated one when the profile is unusable.
fn converter(app: &PhotocraftApp) -> CmykConverter {
    CmykConverter::new(app.session.color.working(photocraft_doc::ColorMode::Cmyk)).ok().unwrap_or_else(|| CmykConverter::coated().clone())
}

/// Opens `dialog` for `target` with the app's picker features, under `parent` when it has one.
fn launch(app: &mut PhotocraftApp, dialog: ColorPickerDialog, parent: Option<DialogId>, target: PickTarget) -> DialogId {
    let mut dialog = dialog.cmyk(&converter(app)).canvas_samples(true).eyedropper(true).add_to_swatches(true).instance(target.identity());
    if let Some(idx) = app.session.active_index() {
        dialog = dialog.canvas(crate::canvas::document_display::CanvasHost::primary(app, idx).canvas_id());
    }
    let id = match parent {
        Some(parent) => app.framework.dialogs.open_child(dialog, parent),
        None => app.framework.dialogs.open(dialog),
    };
    live_preview::begin(app, id, &target);
    app.color_targets.insert(id, target);
    id
}

/// Opens the picker for the foreground or background colour.
pub fn open(app: &mut PhotocraftApp, target: &str) -> DialogId {
    let background = target == "background";
    let c = if background { app.session.tools.background } else { app.session.tools.foreground };
    let rgb = [c[0], c[1], c[2]];
    let color = Srgba::from_rgb_array(rgb);
    let mut dialog = if background { ColorPickerDialog::background(color) } else { ColorPickerDialog::foreground(color) };
    // A colour that was picked as CMYK shows the inks it was picked as, not what its sRGB stand-in converts to (#4196).
    if let Some(inks) = app.session.tools.picked_cmyk(rgb) {
        dialog = dialog.kept(inks_model(inks));
    }
    launch(app, dialog, None, if background { PickTarget::Background } else { PickTarget::Foreground })
}

/// Opens the picker on `rgb` titled `label` for `target`, with `kept` inks as the model values it shows.
pub(crate) fn open_for(app: &mut PhotocraftApp, label: &str, color: Srgba, kept: Option<ModelValues>, target: PickTarget) -> DialogId {
    let mut dialog = ColorPickerDialog::new(target.key(), label.to_owned(), color);
    if let Some(kept) = kept {
        dialog = dialog.kept(kept);
    }
    launch(app, dialog, None, target)
}

/// Opens the picker on `rgb` for something other than the tool colours: OK runs `command` with
/// `params` plus `"color": "#rrggbb"` (e.g. a gradient stop's colour).
pub(crate) fn open_for_command(app: &mut PhotocraftApp, label: &str, rgb: [f32; 3], command: &str, params: Value) -> DialogId {
    // The document the picker was opened on: its live canvas preview never shows on another (`color_picker_preview`).
    let document = app.session.active().map(|st| st.doc.id);
    let target = PickTarget::Command { command: command.into(), params, document };
    open_for(app, label, Srgba::from_rgb_array(rgb), None, target)
}

/// Opens the picker on `rgb` for a colour the shell keeps itself: `cropShield`, `artboardBackground`,
/// `noteColor` or `pasteboard` (or `foreground`/`background`). Any other name is refused.
pub(crate) fn open_for_target(app: &mut PhotocraftApp, target: &str, label: &str, rgb: [f32; 3]) -> Result<DialogId, String> {
    let which = match target {
        "foreground" | "background" => return Ok(open(app, target)),
        "cropShield" => PickTarget::CropShield,
        "artboardBackground" => PickTarget::ArtboardBackground,
        "noteColor" => PickTarget::NoteColor,
        "pasteboard" => PickTarget::Pasteboard,
        other => return Err(format!("unknown colour target: {other}")),
    };
    Ok(open_for(app, label, Srgba::from_rgb_array(rgb), None, which))
}

/// `request` with its title in the app's form, "Color Picker (Vine Color)"; a request with no title keeps the dialog's own.
fn titled(request: ColorRequest) -> ColorRequest {
    match request.title.as_ref().map(|name| title_for(name.as_str())) {
        Some(title) => request.title(title),
        None => request,
    }
}

/// Opens the colour child of `parent`'s `request`; its OK waits for [`answer_for`].
pub(crate) fn open_for_request(app: &mut PhotocraftApp, parent: DialogId, request: ColorRequest) -> DialogId {
    let target = PickTarget::Request { parent, key: request.key.clone() };
    launch(app, ColorPickerDialog::from_request(titled(request)), Some(parent), target)
}

/// The confirmed colour of the child opened for `parent`, once, with the request's key. The first answer
/// in push order; a parent has one picker child at a time.
pub(crate) fn answer_for(app: &mut PhotocraftApp, parent: DialogId) -> Option<(String, Srgba)> {
    let i = app.color_answers.iter().position(|(p, ..)| *p == parent)?;
    let (_, key, color) = app.color_answers.remove(i);
    Some((key, color))
}

/// Opens the picker for a colour row of `window`, which the app holds itself (no parent dialog).
pub(crate) fn open_for_window(app: &mut PhotocraftApp, window: PickWindow, request: ColorRequest) -> DialogId {
    let target = PickTarget::Window { window, key: request.key.clone() };
    launch(app, ColorPickerDialog::from_request(titled(request)), None, target)
}

/// The oldest confirmed colour of `window`'s picker, once.
pub(crate) fn answer_for_window(app: &mut PhotocraftApp, window: PickWindow) -> Option<(String, Srgba)> {
    answer_for(app, window.answers())
}

/// Cancels the picker `window` opened, when it is still open (the window closed over it).
pub(crate) fn cancel_window_picker(app: &mut PhotocraftApp, ctx: &egui::Context, window: PickWindow) {
    if let Some((id, _)) = find(app, |t| matches!(t, PickTarget::Window { window: w, .. } if *w == window)) {
        app.framework.dialogs.cancel_by_id(ctx, id);
    }
}

/// Whether `window`'s picker is open, as the dialog host knows it.
pub(crate) fn window_picker_open(app: &PhotocraftApp, window: PickWindow) -> bool {
    app.color_targets
        .iter()
        .any(|(id, t)| matches!(t, PickTarget::Window { window: w, .. } if *w == window) && app.framework.dialogs.get_by_id::<ColorPickerDialog>(*id).is_some())
}

/// The Color Picker, when it is the top dialog (another dialog opened over it takes the input).
pub(crate) fn top(app: &PhotocraftApp) -> Option<DialogId> {
    let id = app.framework.dialogs.stack().top()?;
    app.framework.dialogs.get_by_id::<ColorPickerDialog>(id).map(|_| id)
}

/// What picker `id` is for, while it is one of this module's.
pub(crate) fn target(app: &PhotocraftApp, id: DialogId) -> Option<&PickTarget> {
    app.color_targets.get(&id)
}

/// The topmost open picker of this module whose target `matches`.
pub(crate) fn find(app: &PhotocraftApp, matches: impl Fn(&PickTarget) -> bool) -> Option<(DialogId, PickTarget)> {
    app.framework
        .dialogs
        .stack()
        .ids()
        .filter(|id| app.framework.dialogs.get_by_id::<ColorPickerDialog>(*id).is_some())
        .filter_map(|id| app.color_targets.get(&id).filter(|t| matches(t)).map(|t| (id, t.clone())))
        .last()
}

/// Once per pass: serves each open picker's screen pick, Add to Swatches and canvas eyedropper request,
/// previews the picked colours in their parents, and commits the pickers that finished. A second pass in
/// one frame finds nothing new: every request is taken from the dialog once.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.color_targets.is_empty() {
        return;
    }
    let ids: Vec<DialogId> = app.color_targets.keys().copied().collect();
    // PLAN: assumed `screen_picker::busy(app: &PhotocraftApp) -> bool` and `hold(app, ctx, id, gesture)` (dialogs-w39).
    let busy = crate::screen_picker::busy(app);
    for &id in &ids {
        let Some(d) = app.framework.dialogs.get_mut_by_id::<ColorPickerDialog>(id) else { continue };
        d.screen_busy(busy);
        let gesture = d.take_screen_pick();
        let swatch = d.take_add_to_swatches();
        let dropper = d.requests().take_eyedropper();
        if let Some(gesture) = gesture {
            crate::screen_picker::hold(app, ctx, id, gesture);
        }
        // PLAN: assumed `screen_picker::take(app, id) -> Option<Srgba>` (dialogs-w39).
        if let Some(color) = crate::screen_picker::take(app, id)
            && let Some(d) = app.framework.dialogs.get_mut_by_id::<ColorPickerDialog>(id)
        {
            d.set_color(PickedColor::rgb(color));
        }
        if let Some(picked) = swatch {
            crate::swatches_ui::add_from_picker(app, picked);
        }
        if let Some(request) = dropper {
            crate::dialogs::canvas_input::arm_dropper(app, id, request.key);
        }
    }
    live_preview::sync(app);
    for id in ids {
        match app.framework.dialogs.take_by_id::<ColorPickerDialog>(id) {
            Some(DialogResult::Ok(ColorPickerResult { picked, .. })) => close(app, id, Some(picked)),
            Some(DialogResult::Cancel) => close(app, id, None),
            Some(_) => close(app, id, None),
            None => {
                if app.framework.dialogs.get_by_id::<ColorPickerDialog>(id).is_none() {
                    close(app, id, None);
                }
            }
        }
    }
}

/// Picker `id` is closed: stops its preview and dropper, and commits `picked` (OK) to its target.
fn close(app: &mut PhotocraftApp, id: DialogId, picked: Option<PickedColor>) {
    live_preview::end(app, id);
    // The dropper is one app-wide arm; a closing picker releases it.
    crate::dialogs::canvas_input::disarm_dropper(app);
    let Some(target) = app.color_targets.remove(&id) else { return };
    let Some(picked) = picked else { return };
    match commit(app, id, target, &picked) {
        // A colour picked in the Color Picker joins the Recently Used Colors, as in Photoshop.
        Ok(_) => crate::paint_picker::remember(app, picked.color.to_rgb_array()),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

fn set_tool_color(app: &mut PhotocraftApp, which: &str, picked: &PickedColor) -> Result<Value, String> {
    let mut params = json!({ which: rgb_hex(picked.color) });
    // The CMYK values the person entered stay the colour's inks: a CMYK document is filled with them (#4196).
    if let Some(inks) = inks_of(picked) {
        params[format!("{which}Cmyk")] = json!(inks);
    }
    let r = app.run("tools.setColors", params)?;
    if which == "foreground" {
        crate::apply_color::foreground_changed(app, None);
    }
    Ok(r)
}

/// OK: apply `picked` to what picker `id` was opened for.
fn commit(app: &mut PhotocraftApp, _id: DialogId, target: PickTarget, picked: &PickedColor) -> Result<Value, String> {
    let color = rgb_hex(picked.color);
    match &target {
        PickTarget::Foreground => set_tool_color(app, "foreground", picked),
        PickTarget::Background => set_tool_color(app, "background", picked),
        PickTarget::Command { command, params, .. } => {
            let shown = crate::color_picker_preview::shown_for(app, &target, picked);
            let mut p = if params.is_object() { params.clone() } else { json!({}) };
            p["color"] = json!(color);
            let r = app.run(command, p)?;
            // The canvas already shows this edit (the live preview): refresh only what it changed.
            if let Some((doc, key)) = shown {
                crate::canvas::shown_as_document(app, doc, |k| k == key);
            }
            Ok(r)
        }
        PickTarget::Shape(t) => crate::shape_fill_ui::confirm(app, t, picked),
        PickTarget::SolidFill { document, layer } => crate::solid_fill_ui::confirm(app, *document, *layer, picked),
        PickTarget::NewSolidFill { document } => crate::solid_fill_ui::confirm_new(app, *document, picked),
        PickTarget::Request { parent, key } => {
            app.color_answers.push((*parent, key.clone(), picked.color));
            Ok(json!({ "color": color }))
        }
        PickTarget::Window { window, key } => {
            app.color_answers.push((window.answers(), key.clone(), picked.color));
            Ok(json!({ "color": color }))
        }
        PickTarget::CropShield => {
            crate::crop_ui::set_shield_color(app, &color)?;
            Ok(json!({ "color": color }))
        }
        // The Artboard tool's custom background for the boards it draws (#11385).
        PickTarget::ArtboardBackground => {
            let shown = crate::artboard_tool::bar::shown_background(app).1;
            let unchanged = parse_hex(&shown).map(hex).as_deref() == Some(color.as_str());
            crate::artboard_tool::bar::set_custom_background_from_picker(app, &color, unchanged)
        }
        // The Note tool's colour for new notes (#2144).
        PickTarget::NoteColor => {
            app.ui.analysis.note_color = picked.color.to_rgb_array();
            Ok(json!({ "color": color }))
        }
        // The pasteboard menu's Select Custom Color…: the custom colour, and use it.
        PickTarget::Pasteboard => app.run("prefs.set", json!({"values": {"interface.canvasColor": "custom", "interface.canvasCustomColor": color}})),
    }
}

/// Helpers for tests that drive a picker; OK and Cancel go through `dialogs::testing`.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// Sets picker `id`'s colour to `hex` (`#rrggbb`), as an edit would; the opacity stays what the picker shows.
    pub fn set_color(app: &mut PhotocraftApp, id: DialogId, hex: &str) {
        let rgb = parse_hex(hex).expect("a colour");
        let picker = app.framework.dialogs.get_mut_by_id::<ColorPickerDialog>(id).expect("the picker is open");
        let alpha = picker.current().map_or(1.0, |c| c.color.a);
        picker.set_color(PickedColor::rgb(Srgba { a: alpha, ..Srgba::from_rgb_array(rgb) }));
    }

    /// The colour picker `id` shows now.
    pub fn current(app: &PhotocraftApp, id: DialogId) -> Option<PickedColor> {
        app.framework.dialogs.get_by_id::<ColorPickerDialog>(id)?.current()
    }
}

pub(crate) mod live_preview;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod screen_integration_tests {
    use super::*;
    use craft::color_ui::ui::StillCapture;
    use egui::vec2;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn text_swatch_opens_full_dialog_and_screen_pick_waits_for_ok() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":160,"height":120})).unwrap();
        let id = session.execute("type.create", json!({"text":"Hello world","color":"#000000","size":24,"x":10,"y":60})).unwrap()["layer"].as_u64().unwrap();
        let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), session, Default::default());
            app.ui.tool = crate::state::Tool::Type;
            app.services.screen_capture = Some(std::sync::Arc::new(|| Box::new(StillCapture::color(Srgba::rgb(1.0, 136.0 / 255.0, 0.0)))));
            app
        });
        h.run_steps(8);
        let before = h.state().session.active().unwrap().history.entries().len();
        let fg = h.state().session.tools.foreground;
        h.get_all_by_role_and_label(egui::accesskit::Role::ColorWell, "Text color").next().expect("the text swatch").click();
        h.run_steps(6);
        let picker = top(h.state()).expect("the text swatch opens the Color Picker");
        assert!(matches!(target(h.state(), picker), Some(PickTarget::Command { command, .. }) if command == "type.setStyle"));
        h.get_by_label("Sample color from the screen").click();
        h.run_steps(6);
        assert_eq!(rgb_hex(testing::current(h.state(), picker).unwrap().color), "#ff8800");
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), before, "sampling only changes the picker's colour");
        assert_eq!(h.state().session.tools.foreground, fg);
        h.get_by_label("OK").click();
        h.run_steps(4);
        let doc = &h.state().session.active().unwrap().doc;
        let layer = doc.layer(photocraft_doc::LayerId(id)).unwrap();
        let photocraft_doc::LayerContent::Text(text) = &layer.content else { panic!("not text") };
        assert_eq!(text.color.to_rgba8(), [255, 136, 0, 255]);
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), before + 1);
    }
}
