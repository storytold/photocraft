//! The Brush, Pencil, Eraser and Mixer Brush members of the options bar: the host's preset picker
//! and settings toggle, the framework's `PaintBar`, and the symmetry menu.

use craft::draw::ui::{EraserMode, MixerValues, PaintBar, PaintTool, PaintValues, Smoothing};
use craft::theme::Srgba;
use craft::ui::contract::Entry;
use craft::ui::dialog::{Alert, HostedAlert};
use craft::ui::patterns::blend_keys::{BlendKeyEntry, blend_entries};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::RangeAlert;
use photocraft_doc::BlendMode;
use photocraft_paint::BrushSettings;
use serde_json::json;

use crate::PhotocraftApp;
use crate::brush_mode_menu;
use crate::state::Tool;

/// The blend key of the Brush's and Pencil's Mode list: a blend mode, or one of the two
/// painting-only modes the brush stores apart from its blend mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PaintMode {
    /// A blend mode.
    Blend(BlendMode),
    /// Paint behind the layer's pixels.
    Behind,
    /// Paint transparency.
    Clear,
}

impl PaintMode {
    /// The English text of the mode's entry.
    fn text(self) -> &'static str {
        match self {
            PaintMode::Blend(m) => m.label(),
            PaintMode::Behind => "Behind",
            PaintMode::Clear => "Clear",
        }
    }
}

/// The framework's tool for `tool`; `None` for a tool without a paint bar.
fn paint_tool(tool: Tool) -> Option<PaintTool> {
    Some(match tool {
        Tool::Brush => PaintTool::Brush,
        Tool::Pencil => PaintTool::Pencil,
        Tool::Eraser => PaintTool::Eraser,
        Tool::MixerBrush => PaintTool::MixerBrush,
        _ => return None,
    })
}

/// The Eraser's mode as the tool options store it ("brush", "pencil" or "block").
fn eraser_mode_of(name: &str) -> EraserMode {
    match name {
        "pencil" => EraserMode::Pencil,
        "block" => EraserMode::Block,
        _ => EraserMode::Brush,
    }
}

/// The stored name of an Eraser mode; the inverse of [`eraser_mode_of`].
fn eraser_mode_name(mode: EraserMode) -> &'static str {
    match mode {
        EraserMode::Pencil => "pencil",
        EraserMode::Block => "block",
        _ => "brush",
    }
}

/// The Mode list of the Brush and Pencil: the blend modes with Behind and Clear after Dissolve. Unlike the
/// Shift+right-click menu and the Pattern Stamp's list, it keeps both on the Background, so the wheel steps through
/// them there too.
fn mode_entries() -> Vec<Entry<PaintMode>> {
    static CACHE: crate::i18n::LangCache<(), Vec<Entry<PaintMode>>> = crate::i18n::LangCache::new();
    CACHE.get_or_build((), || {
        let keys: Vec<BlendKeyEntry<PaintMode>> = brush_mode_menu::choices()
            .into_iter()
            .map(|((mode, behind), _)| match (mode, behind) {
                (Some(m), _) => crate::panels::blend_entry(PaintMode::Blend(m), m),
                (None, true) => BlendKeyEntry::behind(PaintMode::Behind),
                (None, false) => BlendKeyEntry::clear(PaintMode::Clear),
            })
            .collect();
        blend_entries(&keys, |entry| Label::from(tl!(entry.key.text()).to_string()))
    })
}

/// The bar's values for `tool` from its brush `b` and the tool options. Opacity, Flow and Hardness
/// are fractions 0..1 in the brush and in the bar.
// The framework's value structs are `#[non_exhaustive]`, so they are built from their defaults.
#[allow(clippy::field_reassign_with_default)]
fn paint_values(app: &PhotocraftApp, tool: Tool, b: &BrushSettings) -> PaintValues<PaintMode> {
    let o = &app.ui.tool_options;
    let clear_flag = match tool {
        Tool::Brush => o.brush_clear,
        Tool::Pencil => o.pencil_clear,
        _ => false,
    };
    // `~` held makes the Brush and Pencil erase with the brush; the list shows Clear meanwhile.
    let clear = clear_flag || (matches!(tool, Tool::Brush | Tool::Pencil) && app.held_erase);
    let mode = if clear {
        PaintMode::Clear
    } else if b.behind {
        PaintMode::Behind
    } else {
        PaintMode::Blend(b.mode)
    };
    let mut v = PaintValues::new(mode);
    v.size = f64::from(b.size);
    v.hardness = f64::from(b.hardness);
    v.opacity = f64::from(b.opacity);
    v.flow = f64::from(b.flow);
    let mut smoothing = Smoothing::default();
    smoothing.amount = f64::from(b.smoothing.amount);
    smoothing.pulled_string = b.smoothing.pulled_string;
    smoothing.catch_up = b.smoothing.catch_up;
    smoothing.catch_up_on_end = b.smoothing.catch_up_on_end;
    smoothing.adjust_for_zoom = b.smoothing.adjust_for_zoom;
    v.smoothing = smoothing;
    v.pressure_opacity = b.pressure_opacity;
    v.pressure_size = b.pressure_size;
    v.build_up = b.build_up;
    v.erase_to_history = o.erase_to_history;
    v.auto_erase = o.pencil_auto_erase;
    let mut mixer = MixerValues::default();
    mixer.load_after_stroke = b.mixer.load_after_stroke;
    mixer.clean_after_stroke = b.mixer.clean_after_stroke;
    mixer.wet = f64::from(b.mixer.wet);
    mixer.load = f64::from(b.mixer.load);
    mixer.mix = f64::from(b.mixer.mix);
    mixer.flow = f64::from(b.mixer.flow);
    mixer.sample_all_layers = b.mixer.sample_all_layers;
    mixer.combination = o.mixer_combination;
    mixer.load_color = app.session.tools.mixer.next_load(app.session.tools.foreground).map(Srgba::from_array);
    mixer.solid_colors_only = b.mixer.solid_colors_only;
    v.mixer = mixer;
    v
}

/// Writes the bar's `after` values onto `brush` and the tool options. Only what differs from
/// `before` is written, so a Mode list that shows Clear because `~` is held does not store it.
fn apply_values(app: &mut PhotocraftApp, tool: Tool, brush: &mut BrushSettings, before: &PaintValues<PaintMode>, after: &PaintValues<PaintMode>) {
    // The brush keeps at least 1 % opacity and flow (0 % paints nothing, #11812).
    // PLAN: PaintBar's Opacity and Flow fields range over 0..=100, so a typed 0 on the Eraser reports no
    // `out_of_range` and shows no alert (the old bar's range started at 1); pn.C18's owner decides the range.
    const MIN_FRACTION: f32 = 0.01;
    brush.size = after.size as f32;
    brush.hardness = after.hardness as f32;
    brush.opacity = (after.opacity as f32).max(MIN_FRACTION);
    if tool != Tool::MixerBrush {
        brush.flow = (after.flow as f32).max(MIN_FRACTION);
    }
    if after.smoothing.amount != before.smoothing.amount {
        brush.smoothing.enabled = true;
    }
    brush.smoothing.amount = after.smoothing.amount as f32;
    brush.smoothing.pulled_string = after.smoothing.pulled_string;
    brush.smoothing.catch_up = after.smoothing.catch_up;
    brush.smoothing.catch_up_on_end = after.smoothing.catch_up_on_end;
    brush.smoothing.adjust_for_zoom = after.smoothing.adjust_for_zoom;
    brush.pressure_opacity = after.pressure_opacity;
    brush.pressure_size = after.pressure_size;
    brush.build_up = after.build_up;
    let m = &mut brush.mixer;
    m.load_after_stroke = after.mixer.load_after_stroke;
    m.clean_after_stroke = after.mixer.clean_after_stroke;
    m.wet = after.mixer.wet as f32;
    m.load = after.mixer.load as f32;
    m.mix = after.mixer.mix as f32;
    m.flow = after.mixer.flow as f32;
    m.sample_all_layers = after.mixer.sample_all_layers;
    m.solid_colors_only = after.mixer.solid_colors_only;
    let o = &mut app.ui.tool_options;
    o.erase_to_history = after.erase_to_history;
    o.pencil_auto_erase = after.auto_erase;
    o.mixer_combination = after.mixer.combination;
    if after.mode != before.mode {
        let clear = match tool {
            Tool::Pencil => &mut o.pencil_clear,
            _ => &mut o.brush_clear,
        };
        *clear = after.mode == PaintMode::Clear;
        match after.mode {
            PaintMode::Blend(m) => {
                brush.behind = false;
                brush.mode = m;
            }
            PaintMode::Behind => {
                brush.behind = true;
                brush.mode = BlendMode::Normal;
            }
            PaintMode::Clear => brush.behind = false,
        }
    }
}

/// The Eraser's range alert for a value typed out of range. One alert at a time: a second typed
/// value while one is open adds none.
fn open_range_alert(app: &mut PhotocraftApp, ui: &egui::Ui, range: &RangeAlert) {
    let alert = Alert::range(tl!("PhotoCraft").to_owned(), range, &Env::of(ui));
    // An `Err` is the alert that was not opened because one is already up.
    let _ = app.framework.dialogs.open_unless_open(HostedAlert::new(alert));
}

/// Shows the members of `tool`'s bar; false for a tool without a paint bar.
///
/// A pass applies only its own edits: the values are read from the app at the start of each pass
/// and `PaintBar` reports an edit only in the pass that read the input, so a second pass in the
/// same frame sees the stored result and writes nothing.
pub(super) fn members(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    let Some(paint) = paint_tool(tool) else { return false };
    // The picker and its settings toggle are the host's; a Block-mode Eraser has no brush behind them.
    let block = crate::eraser_ui::block_mode(app, tool);
    crate::brush_picker::show_field(ui, app, !block);
    crate::brush_picker::settings_toggle(ui, app);

    let brush_before = app.session.tools.brush.clone();
    let before = paint_values(app, tool, &brush_before);
    let mut values = before.clone();
    let mut eraser = eraser_mode_of(&app.ui.tool_options.eraser_mode);
    let modes = if matches!(tool, Tool::Brush | Tool::Pencil) { mode_entries() } else { Vec::new() };
    let mut bar = PaintBar::new(paint, &mut values).modes(&modes);
    if tool == Tool::Eraser {
        bar = bar.eraser_mode(&mut eraser);
    }
    let out = bar.show(ui);

    if out.eraser_mode.is_some() {
        app.ui.tool_options.eraser_mode = eraser_mode_name(eraser).into();
    }
    // One pass is one `tools.setBrush`, keyed by the pass's last edit event, so one gesture is one history step.
    // PLAN: assumed `crate::history_keys::coalesce(&str, &EditEvent<T>) -> Option<String>` (tl.P12's key rule).
    if let Some(last) = out.edits.iter().next_back() {
        let key = crate::history_keys::coalesce("brush", last);
        let mut brush = brush_before.clone();
        apply_values(app, tool, &mut brush, &before, &values);
        crate::brush_panel::commit_keyed(app, &brush_before, &brush, key);
    }
    if out.mixer_load {
        let result = app.run("brush.mixer.load", json!({}));
        report(app, result);
    }
    if out.mixer_clean {
        let result = app.run("brush.mixer.clean", json!({}));
        report(app, result);
    }
    // Only the Eraser alerts; the Brush and Pencil clamp silently (#11938).
    if tool == Tool::Eraser
        && let Some(range) = out.out_of_range
    {
        open_range_alert(app, ui, &range);
    }
    if matches!(tool, Tool::Brush | Tool::Pencil | Tool::Eraser) {
        crate::symmetry_ui::menu(ui, app);
    }
    true
}

/// Puts a command's error on the status line.
fn report(app: &mut PhotocraftApp, result: Result<serde_json::Value, String>) {
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
#[path = "../../mixer_blend_combination_tests.rs"]
mod mixer_blend_combination_tests;

#[cfg(test)]
#[path = "../../mixer_load_solid_colors_tests.rs"]
mod mixer_load_solid_colors_tests;
