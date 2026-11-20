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
use craft::ui::contract::{Bind, MemoProps, MemoView, Outbox, Props as _, SaveKey, Show as _, Tracked, bind_tracked};
use craft::ui::runtime::env::Env;
use photocraft_engine::BrushSettings;
use photocraft_engine::paint::{BrushPreset, MAX_BRUSH_SIZE, TipShape};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::brush_panel::brushes::{BrushKey, current_key, preset_rows, run_picks, tip_paint};
use crate::brush_panel::commit_keyed;
use crate::state::Tool;

/// Everything one pass of the picker reads from the app, built once so the canvas popup and the bar field show the same
/// thing.
pub(crate) struct PickerInput {
    rows: Vec<craft::ui::patterns::presets::PresetRow<BrushKey>>,
    current: Option<BrushKey>,
    values: BrushPickerValues,
    brush: BrushSettings,
}

impl PickerInput {
    /// The rows, the current preset and the brush values as the app holds them now.
    pub(crate) fn of(app: &PhotocraftApp, env: &Env) -> Self {
        let tools = &app.session.tools;
        Self::new(tools.presets.as_slice(), &tools.brush, current_key(tools), app.ui.brush_picker_state.parts, env)
    }

    /// The rows of `presets` with the card `parts`, `current` picked, and the values of `brush`.
    fn new(presets: &[BrushPreset], brush: &BrushSettings, current: Option<BrushKey>, parts: BrushCardParts, env: &Env) -> Self {
        let brush = brush.clone();
        let round = brush.tip == TipShape::Round;
        let values = BrushPickerValues::new(f64::from(brush.size), round.then_some(f64::from(brush.hardness)))
            .with_tip(TipAngle::new(f64::from(brush.angle), f64::from(brush.roundness)));
        Self { rows: preset_rows(presets, env, parts), current, values, brush }
    }

    /// Every row's key, in list order.
    fn keys(&self) -> Vec<BrushKey> {
        self.rows.iter().map(|r| r.key.clone()).collect()
    }

    /// The framework picker over this input and the picker's view `state`.
    pub(crate) fn picker<'a>(&'a mut self, state: Bind<'a, BrushPickerState<BrushKey>>) -> BrushPresetPicker<'a, BrushKey> {
        BrushPresetPicker::new(tl!("Brush Preset"), Bind::bound(&mut self.current), &self.rows, state, self.values)
            .max_diameter(f64::from(MAX_BRUSH_SIZE))
            .import()
            .gear_rows(gear_rows)
            .resizable(SaveKey::new("photocraft.brush_picker"))
            .tip(std::sync::Arc::new(tip_paint(&self.brush)))
    }
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
/// a picker closes it. The Background Eraser shows the compact form with its own rows under the head.
pub(crate) fn show_at(ui: &mut egui::Ui, app: &mut PhotocraftApp) {
    let Some(at) = app.ui.brush_picker else { return };
    if !crate::paint_mouse::has_brush_picker(app.ui.tool) {
        close(app);
        return;
    }
    let env = Env::of(ui);
    let mut input = PickerInput::of(app, &env);
    let mut open = true;
    let background_eraser = app.ui.tool == Tool::BackgroundEraser;
    let row_before = input.brush.clone();
    let mut row_brush = input.brush.clone();
    let mut tolerance = app.ui.tool_options.bg_tolerance_control.clone();
    let mut key = None;
    let out = {
        let picker = input.picker(bind_tracked!(app.ui.brush_picker_state)).open_at(egui::pos2(at[0], at[1]), Bind::bound(&mut open));
        let picker =
            if background_eraser { picker.compact(true).rows(|ui| key = crate::bg_eraser_popup::rows(ui, &mut row_brush, &mut tolerance)) } else { picker };
        picker.show(ui)
    };
    let closed = !open || !out.open;
    apply(app, ui.ctx(), &input.keys(), out);
    if background_eraser {
        commit_keyed(app, &row_before, &row_brush, key);
        if app.ui.tool_options.bg_tolerance_control != tolerance {
            app.ui.tool_options.bg_tolerance_control = tolerance;
        }
    }
    if closed {
        close(app);
    }
}

/// Every input of the options bar's picker field: the presets and the brush it shows, and the picker's view state.
#[derive(MemoProps)]
struct FieldProps<'a> {
    presets: &'a Tracked<Arc<Vec<BrushPreset>>>,
    brush: &'a Tracked<BrushSettings>,
    current: Option<BrushKey>,
    enabled: bool,
    state: &'a mut Tracked<BrushPickerState<BrushKey>>,
    out: Outbox<'a, FieldEvent>,
}

/// What a run of the picker field reports: the picker's output, with every row's key when it holds picks.
struct FieldEvent(BrushPresetPickerOutput<BrushKey>, Vec<BrushKey>);

/// The closed picker field of the options bar, which opens the popover. `enabled` is false for the Eraser in Block
/// mode, which paints with no brush. A memoized view: it runs when the presets, the brush, the current preset or the
/// picker's state change, while its popover is open, or when input reaches it.
pub(crate) fn show_field(ui: &mut egui::Ui, app: &mut PhotocraftApp, enabled: bool) {
    let mut sent = Vec::new();
    let tools = &app.session.tools;
    let props = FieldProps {
        presets: &tools.presets,
        brush: &tools.brush,
        current: current_key(tools),
        enabled,
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
    let env = Env::of(ui);
    let mut input = PickerInput::new(props.presets.as_slice(), props.brush, props.current.clone(), props.state.parts, &env);
    let out = input.picker(bind_tracked!(props.state)).enabled(props.enabled).show(ui);
    let keys = if out.picks.is_empty() { Vec::new() } else { input.keys() };
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
