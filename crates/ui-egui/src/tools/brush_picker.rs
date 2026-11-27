//! Photoshop's Brush Preset picker host. The framework's `BrushPresetPicker` draws the popover and the closed field;
//! this file gives it the app's rows, brush values and gear rows and maps what it reports back to the engine.
//!
//! Two places show the same picker through one build path ([`PickerInput`]): the canvas, where a right-click opens
//! it at a point ([`show_at`], opened by [`open`]), and the options bar, where the closed field opens it
//! ([`show_field`]). Every brush edit is a journaled `tools.setBrush` command, one per gesture.

use std::sync::Arc;

use craft::behavior::EditEvents;
use craft::draw::ui::brush_presets::{
    BrushCardPart, BrushCardParts, BrushPick, BrushPickerSettings, BrushPickerState, BrushPickerValues, BrushPresetPicker, BrushPresetPickerOutput, GearSection,
};
use craft::draw::ui::brush_settings::TipAngle;
use craft::ui::buttons::{Button, ToggleButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, MemoProps, MemoView, Outbox, Props as _, SaveKey, Show as _, TooltipSpec, Tracked, bind_tracked, tooltip};
use craft::ui::patterns::presets::PresetSource;
use craft::ui::runtime::env::Env;
use photocraft_engine::BrushSettings;
use photocraft_engine::paint::{BrushPreset, MAX_BRUSH_SIZE, TipShape};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::brush_panel::brushes::{BrushKey, brush_source, current_key, run_picks, tip_paint};
use crate::brush_panel::commit_keyed;
use crate::state::Tool;

/// Everything one pass of the picker reads from the app, built once so the canvas popup and the bar field show the same
/// thing.
pub(crate) struct PickerInput<'a> {
    source: PresetSource<'a, BrushKey>,
    current: Option<BrushKey>,
    values: BrushPickerValues,
    tip: &'a TipShape,
    face: BrushFace,
}

/// What the picker shows of the brush besides its tip: the head's values and the tip picture's shape.
#[derive(Clone, Copy, MemoProps)]
struct BrushFace {
    size: f32,
    hardness: f32,
    angle: f32,
    roundness: f32,
}

impl BrushFace {
    fn of(brush: &BrushSettings) -> Self {
        Self { size: brush.size, hardness: brush.hardness, angle: brush.angle, roundness: brush.roundness }
    }
}

impl<'a> PickerInput<'a> {
    /// The presets of `presets` with the card `parts`, `current` picked, and the brush's `tip` and `face`.
    fn new(presets: &'a Tracked<Arc<Vec<BrushPreset>>>, tip: &'a TipShape, face: BrushFace, current: Option<BrushKey>, parts: BrushCardParts) -> Self {
        let round = *tip == TipShape::Round;
        let values = BrushPickerValues::new(f64::from(face.size), round.then_some(f64::from(face.hardness)))
            .with_tip(TipAngle::new(f64::from(face.angle), f64::from(face.roundness)));
        Self { source: brush_source(presets, parts), current, values, tip, face }
    }

    /// Every row's key, in list order, in the scope of `ui`.
    fn keys(&self, ui: &egui::Ui) -> Vec<BrushKey> {
        self.source.rows(ui.ctx(), &Env::of(ui)).iter().map(|r| r.key.clone()).collect()
    }

    /// The framework picker over this input and the picker's view `state`.
    pub(crate) fn picker<'p>(&'p mut self, state: Bind<'p, BrushPickerState<BrushKey>>) -> BrushPresetPicker<'p, BrushKey> {
        BrushPresetPicker::new(tl!("Brush Preset"), Bind::bound(&mut self.current), &self.source, state, self.values)
            .max_diameter(f64::from(MAX_BRUSH_SIZE))
            .import()
            .gear_rows(gear_rows)
            .resizable(SaveKey::new("photocraft.brush_picker"))
            .tip(tip_paint(self.tip, (self.face.hardness, self.face.angle, self.face.roundness)))
    }
}

/// The Background Eraser's own rows under the compact picker's head: the brush and the tolerance they edit, and the
/// brush as it was before the pass.
struct EraserRows {
    before: BrushSettings,
    brush: BrushSettings,
    tolerance: String,
    key: Option<String>,
}

/// Sends each event of a head edit as a `tools.setBrush`; the events of one gesture share the coalesce key, so the
/// gesture is one command.
fn commit_edits<T>(app: &mut PhotocraftApp, events: &EditEvents<T>, set: impl Fn(&mut BrushSettings, &T)) {
    for event in events.iter() {
        let Some(value) = event.landed() else { continue };
        let before = (*app.session.tools.brush).clone();
        let mut after = before.clone();
        set(&mut after, value);
        // A `Set` has no key and is its own command.
        commit_keyed(app, &before, &after, crate::history_keys::coalesce("brush", event));
    }
}

/// Runs what the picker reported: the head's edits, Escape's revert, the saved view and the picks.
/// `keys` are every row's key in list order (a reorder pick is placed by them).
pub(crate) fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, keys: &[BrushKey], out: BrushPresetPickerOutput<BrushKey>) {
    commit_edits(app, &out.diameter, |b, v| b.size = *v as f32);
    commit_edits(app, &out.hardness, |b, v| b.hardness = *v as f32);
    commit_edits(app, &out.tip, |b, v| {
        b.angle = v.degrees as f32;
        b.roundness = v.roundness as f32;
    });
    if let Some(back) = out.reverted {
        let now = (*app.session.tools.brush).clone();
        let mut after = now.clone();
        after.size = back.diameter as f32;
        if let Some(h) = back.hardness {
            after.hardness = h as f32;
        }
        if let Some(tip) = back.tip {
            after.angle = tip.degrees as f32;
            after.roundness = tip.roundness as f32;
        }
        // One command and no history step: `tools.setBrush` without a coalesce key is a tool setting.
        commit_keyed(app, &now, &after, None);
    }
    if let Some(settings) = &out.settings {
        save(app, settings);
    }
    for pick in out.picks {
        match pick {
            BrushPick::OpenSettings => open_settings(app, ctx),
            other => run_picks(app, vec![other], keys),
        }
    }
}

/// The picker at the point `app.ui.brush_picker` holds (a right-click on the canvas), while it is open. A tool without
/// a picker closes it. The Background Eraser shows the compact form with its own rows under the head. A memoized view:
/// it runs when the presets, the brush's tip or face, the current preset, the picker's state or the eraser's rows
/// change, or when input reaches its popover.
pub(crate) fn show_at(ui: &mut egui::Ui, app: &mut PhotocraftApp) {
    let Some(at) = app.ui.brush_picker else { return };
    if !crate::paint_mouse::has_brush_picker(app.ui.tool) {
        close(app);
        return;
    }
    let mut sent = Vec::new();
    let tools = &app.session.tools;
    let eraser = app.ui.tool == Tool::BackgroundEraser;
    let props = CanvasProps {
        presets: &tools.presets,
        tip: &tools.brush.tip,
        face: BrushFace::of(&tools.brush),
        current: current_key(tools),
        state: &mut app.ui.brush_picker_state,
        at,
        eraser: eraser.then_some((&tools.brush, &app.ui.tool_options.bg_tolerance_control)),
        out: Outbox::new(&mut sent),
    };
    let _ = MemoView::new("brush-preset-canvas", props, canvas_view).show(ui);
    for CanvasEvent { out, keys, eraser, closed } in sent {
        apply(app, ui.ctx(), &keys, out);
        if let Some(rows) = eraser {
            commit_keyed(app, &rows.before, &rows.brush, rows.key);
            if app.ui.tool_options.bg_tolerance_control != rows.tolerance {
                app.ui.tool_options.bg_tolerance_control = rows.tolerance;
            }
        }
        if closed {
            close(app);
        }
    }
}

/// Every input of the canvas picker: the presets, what it shows of the brush, the picker's view state, the point it
/// opens at and, for the Background Eraser, the brush and tolerance its rows edit.
#[derive(MemoProps)]
struct CanvasProps<'a> {
    presets: &'a Tracked<Arc<Vec<BrushPreset>>>,
    tip: &'a Tracked<TipShape>,
    face: BrushFace,
    current: Option<BrushKey>,
    state: &'a mut Tracked<BrushPickerState<BrushKey>>,
    at: [f32; 2],
    eraser: Option<(&'a Tracked<BrushSettings>, &'a String)>,
    out: Outbox<'a, CanvasEvent>,
}

/// What a run of the canvas picker reports: the picker's output with every row's key when it holds picks, the
/// eraser's rows as the run left them, and whether the picker closed.
struct CanvasEvent {
    out: BrushPresetPickerOutput<BrushKey>,
    keys: Vec<BrushKey>,
    eraser: Option<EraserRows>,
    closed: bool,
}

/// The canvas picker's view.
fn canvas_view(props: &mut CanvasProps<'_>, ui: &mut egui::Ui) {
    let mut open = true;
    let mut eraser = props.eraser.map(|(brush, tolerance)| {
        let brush = (**brush).clone();
        EraserRows { before: brush.clone(), brush, tolerance: tolerance.clone(), key: None }
    });
    let mut input = PickerInput::new(props.presets, props.tip, props.face, props.current.clone(), props.state.parts);
    let picker = input.picker(bind_tracked!(props.state)).open_at(egui::pos2(props.at[0], props.at[1]), Bind::bound(&mut open));
    let picker = match eraser.as_mut() {
        Some(rows) => picker.compact(true).rows(|ui| rows.key = crate::bg_eraser_popup::rows(ui, &mut rows.brush, &mut rows.tolerance)),
        None => picker,
    };
    let out = picker.show(ui);
    let keys = if out.picks.is_empty() { Vec::new() } else { input.keys(ui) };
    // The popover writes its binding when it closes. Its output reports it closed in a run that shows no layer (the
    // view's measuring run) too, so that is no close.
    let closed = !open;
    props.out.send(CanvasEvent { out, keys, eraser, closed });
}

/// Every input of the options bar's picker field: the presets, what it shows of the brush (its tip and face), and the
/// picker's view state.
#[derive(MemoProps)]
struct FieldProps<'a> {
    presets: &'a Tracked<Arc<Vec<BrushPreset>>>,
    tip: &'a Tracked<TipShape>,
    face: BrushFace,
    current: Option<BrushKey>,
    enabled: bool,
    clone_stamp: bool,
    state: &'a mut Tracked<BrushPickerState<BrushKey>>,
    out: Outbox<'a, FieldEvent>,
}

/// What a run of the picker field reports: the picker's output, with every row's key when it holds picks.
struct FieldEvent(BrushPresetPickerOutput<BrushKey>, Vec<BrushKey>);

/// The closed picker field of the options bar, which opens the popover. `enabled` is false for the Eraser in Block
/// mode, which paints with no brush. A memoized view: it runs when the presets, the brush's tip or face, the current
/// preset or the picker's state change, while its popover is open, or when input reaches it.
pub(crate) fn show_field(ui: &mut egui::Ui, app: &mut PhotocraftApp, enabled: bool) {
    let mut sent = Vec::new();
    let tools = &app.session.tools;
    let props = FieldProps {
        presets: &tools.presets,
        tip: &tools.brush.tip,
        face: BrushFace::of(&tools.brush),
        current: current_key(tools),
        enabled,
        clone_stamp: app.ui.tool == Tool::CloneStamp,
        state: &mut app.ui.brush_picker_state,
        out: Outbox::new(&mut sent),
    };
    let _ = MemoView::new("brush-preset-field", props, field_view).show(ui);
    for FieldEvent(out, keys) in sent {
        apply(app, ui.ctx(), &keys, out);
    }
}

/// The picker field's view.
fn field_view(props: &mut FieldProps<'_>, ui: &mut egui::Ui) {
    let mut input = PickerInput::new(props.presets, props.tip, props.face, props.current.clone(), props.state.parts);
    let out = input.picker(bind_tracked!(props.state)).enabled(props.enabled).show(ui);
    if props.clone_stamp && !out.open {
        let face = &out.response;
        let at = face.hover_pos().or_else(|| face.has_focus().then(|| face.rect.center()));
        // The picker already owns a tooltip stage for its response id.
        let tip_id = face.id.with("clone-stamp-tip");
        tooltip::at_pointer(ui, tip_id, Some(&TooltipSpec::new(tl!("Open the Brush Preset picker")).into()), at, face.is_pointer_button_down_on());
    }
    let keys = if out.picks.is_empty() { Vec::new() } else { input.keys(ui) };
    props.out.send(FieldEvent(out, keys));
}

#[cfg(test)]
/// Where the canvas picker is open, in screen points; `None` while it is closed.
pub(crate) fn opened_at(app: &PhotocraftApp) -> Option<egui::Pos2> {
    app.ui.brush_picker.map(|at| egui::pos2(at[0], at[1]))
}

/// Opens the canvas picker at `at`. The framework popover closes itself on an outside press and keeps its own opening
/// baseline for Escape.
pub(crate) fn open(app: &mut PhotocraftApp, at: [f32; 2]) {
    app.ui.brush_picker = Some(at);
}

/// Closes the canvas picker.
pub(crate) fn close(app: &mut PhotocraftApp) {
    app.ui.brush_picker = None;
}

/// PhotoCraft's view before the user changes it: name and stroke shown, tip hidden, search bar on, scale 1.
fn default_settings() -> BrushPickerSettings {
    BrushPickerSettings::new(BrushCardParts::ALL.set(BrushCardPart::Tip, false), 1.0)
}

/// Restores the picker's remembered view at launch. A missing or malformed key keeps PhotoCraft's default; the
/// framework clamps the scale and turns the parts on when none is.
pub fn restore(app: &mut PhotocraftApp) {
    let mut saved = app.session.prefs().brush_picker.clone();
    crate::tool_memory::upgrade_picker_view(&mut saved);
    let mut merged = serde_json::to_value(default_settings()).unwrap_or(Value::Null);
    if let (Some(base), Some(over)) = (merged.as_object_mut(), saved.as_object()) {
        for (k, v) in over {
            base.insert(k.clone(), v.clone());
        }
    }
    let settings = serde_json::from_value::<BrushPickerSettings>(merged).unwrap_or_else(|_| default_settings());
    app.ui.brush_picker_state.restore(&settings);
}

/// Remembers the picker's view in the preferences when it differs from what is stored.
fn save(app: &mut PhotocraftApp, settings: &BrushPickerSettings) {
    let Ok(now) = serde_json::to_value(settings) else { return };
    if app.session.prefs().brush_picker != now {
        app.session.prefs.edit(|p| p.brush_picker = now);
    }
}

/// A row of the gear menu whose function the app does not have yet: disabled, with the reason as its tooltip. `checked`
/// makes it a check box.
fn pending_row(ui: &mut egui::Ui, label: &str, checked: Option<bool>) {
    let reason = tl!("Not available yet");
    match checked {
        Some(on) => {
            Checkbox::new(Bind::given(on)).label(tl!(label).to_owned()).enabled(false).tooltip(reason).show(ui);
        }
        None => {
            Button::new(tl!(label).to_owned()).enabled(false).tooltip(reason).show(ui);
        }
    }
}

/// The app's rows of the picker's gear menu, by section.
fn gear_rows(ui: &mut egui::Ui, section: GearSection) {
    match section {
        GearSection::Create => pending_row(ui, "New Brush Group…", None),
        GearSection::Parts => pending_row(ui, "Show Additional Preset Info", Some(true)),
        GearSection::View => pending_row(ui, "Show Recent Brushes", Some(true)),
        GearSection::Library => {
            pending_row(ui, "Append Default Brushes…", None);
            pending_row(ui, "Export Selected Brushes…", None);
        }
        GearSection::End => {
            pending_row(ui, "Get More Brushes…", None);
            pending_row(ui, "Converted Legacy Tool Presets", None);
            pending_row(ui, "Legacy Brushes", None);
        }
        _ => {}
    }
}

/// Shows the Brush Settings panel on its settings tab (what F5 does when it is hidden).
pub fn open_settings(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if crate::panel_docking::is_shown(app, "brushSettings") {
        // The panel is already showing; there is no tab to switch.
        return;
    }
    if let Err(e) = crate::menus::invoke(app, ctx, "window.panel.brushSettings", json!({})) {
        app.ui.status = e;
    }
}

/// The options-bar button beside the brush field that shows and hides the Brush Settings panel (Photoshop's "Toggle
/// the Brush Settings panel"). It runs Window › Brush Settings (F5).
pub(crate) fn settings_toggle(ui: &mut egui::Ui, app: &mut PhotocraftApp) {
    let mut toggled = Vec::new();
    let props = ToggleProps { shown: crate::panel_docking::is_shown(app, "brushSettings"), toggled: Outbox::new(&mut toggled) };
    let _ = MemoView::new("brush-settings-toggle", props, toggle_view).show(ui);
    if !toggled.is_empty()
        && let Err(e) = crate::menus::invoke(app, ui.ctx(), "window.panel.brushSettings", json!({}))
    {
        app.ui.status = e;
    }
}

/// Every input of the Brush Settings toggle.
#[derive(MemoProps)]
struct ToggleProps<'a> {
    shown: bool,
    toggled: Outbox<'a, ()>,
}

/// The Brush Settings toggle's view.
fn toggle_view(props: &mut ToggleProps<'_>, ui: &mut egui::Ui) {
    let mut on = props.shown;
    ToggleButton::new(tl!("Toggle the Brush Settings panel"), Bind::bound(&mut on))
        .icon(craft::behavior::IconId::new("adjustments"))
        .icon_only()
        .tooltip(tl!("Toggle the Brush Settings panel  (F5)"))
        .show(ui);
    if on != props.shown {
        props.toggled.send(());
    }
}

#[cfg(test)]
#[path = "../brush_picker_gear_menu_tests.rs"]
mod gear_menu_tests;
