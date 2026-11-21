//! The Brush Settings panel's section bodies (Photoshop's right-hand side): Brush Tip Shape with
//! its tip grid and angle/roundness widget, Shape Dynamics, Scattering, Texture, Dual Brush, Color
//! Dynamics, Transfer and Brush Pose, all on the framework's rows. Each edits a working copy of the
//! brush; the panel turns the difference into one `tools.setBrush` call.

use std::ops::RangeInclusive;

use craft::behavior::EditEvents;
use craft::draw::ui::brush_presets::{BrushCardPart, BrushCardParts};
use craft::draw::ui::brush_settings::{BrushSizeField, DynamicControl, DynamicControlRow, TipAngle, TipEllipse};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, EditResponse, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::form::{Field, SliderField, SliderFieldLayout};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::presets::{PresetBrowser, PresetEvent, PresetState, PresetView};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::UnitKind;
use egui::Ui;
use photocraft_engine::BrushSettings;
use photocraft_engine::paint::{self, BrushPreset, Control, Dynamic, MAX_BRUSH_SIZE, MaskMode, Pattern, PatternStyle, TipShape};

use super::brush_panel::brushes::{BrushKey, brush_source};
use super::preset_browser::pattern_picker;
use crate::PhotocraftApp;
use crate::history_keys;
use crate::pressure_curve_ui;
use crate::state::Tool;

pub const CONTROLS: [(Control, &str); 8] = [
    (Control::Off, "Off"),
    (Control::Fade, "Fade"),
    (Control::PenPressure, "Pen Pressure"),
    (Control::PenTilt, "Pen Tilt"),
    (Control::StylusWheel, "Stylus Wheel"),
    (Control::Rotation, "Rotation"),
    (Control::InitialDirection, "Initial Direction"),
    (Control::Direction, "Direction"),
];

pub const MASK_MODES: [(MaskMode, &str); 10] = [
    (MaskMode::Multiply, "Multiply"),
    (MaskMode::Subtract, "Subtract"),
    (MaskMode::Darken, "Darken"),
    (MaskMode::Overlay, "Overlay"),
    (MaskMode::ColorDodge, "Color Dodge"),
    (MaskMode::ColorBurn, "Color Burn"),
    (MaskMode::LinearBurn, "Linear Burn"),
    (MaskMode::HardMix, "Hard Mix"),
    (MaskMode::LinearHeight, "Linear Height"),
    (MaskMode::Height, "Height"),
];

const PATTERNS: [(PatternStyle, &str); 4] =
    [(PatternStyle::Paper, "Paper"), (PatternStyle::Canvas, "Canvas"), (PatternStyle::Noise, "Noise"), (PatternStyle::Dots, "Dots")];

/// The largest dual brush tip, in pixels.
const MAX_DUAL_SIZE: f64 = 2500.0;
/// The tip grids' height before they scroll.
const GRID_HEIGHT: f32 = 104.0;
/// The longest side of a tile made from a library pattern.
const LIBRARY_TILE_SIDE: u32 = 256;

/// What a pane reports for its pass.
#[derive(Debug, Default)]
pub(crate) struct SectionOut {
    /// The name of the preset whose tip was picked in a tip grid, for `brush.presets.setCurrent`.
    pub picked: Option<String>,
    /// The history key of the last edit of the pass (`history_keys::coalesce`), for `commit_keyed`.
    pub key: Option<String>,
}

impl SectionOut {
    /// Adds the report of a later pass of the same frame: its own picks and edits replace this one's, and a pass
    /// with none (egui gives a frame's input events to its first pass only) leaves them.
    pub(crate) fn merge(&mut self, later: SectionOut) {
        self.picked = later.picked.or(self.picked.take());
        self.key = later.key.or(self.key.take());
    }

    /// Takes the edit events of `edit` into the pass's key; whether it held any.
    fn note<T>(&mut self, edit: &EditResponse<T>) -> bool {
        self.note_events(&edit.events)
    }

    /// Takes `events` into the pass's key; whether it held any.
    fn note_events<T>(&mut self, events: &EditEvents<T>) -> bool {
        let mut any = false;
        for event in events.iter() {
            self.key = history_keys::coalesce("brush", event);
            any = true;
        }
        any
    }
}

/// One jitter row of a dynamics section: its control id, label, whether the Control row offers the direction
/// controls, and the Minimum row's label when the block has one.
struct DynamicRow<'a> {
    id: &'static str,
    label: &'a str,
    angle: bool,
    minimum: Option<&'a str>,
}

/// How a stored number shows: `shown = stored × factor`, rounded to whole units within `min..=max`.
#[derive(Clone, Copy)]
struct Shown {
    factor: f64,
    min: f64,
    max: f64,
    unit: Option<UnitKind>,
}

impl Shown {
    /// A fraction as whole percent.
    const fn percent(min: f64, max: f64) -> Self {
        Self { factor: 100.0, min, max, unit: Some(UnitKind::Percent) }
    }

    /// A number shown as stored.
    const fn whole(min: f64, max: f64, unit: Option<UnitKind>) -> Self {
        Self { factor: 1.0, min, max, unit }
    }
}

/// A 0 to 100 % amount.
const AMOUNT: Shown = Shown::percent(0.0, 100.0);
/// Tilt is stored in degrees (±90, W3C Pointer Events); Photoshop shows it as ±100 %.
const TILT: Shown = Shown { factor: 100.0 / 90.0, min: -100.0, max: 100.0, unit: Some(UnitKind::Percent) };

/// The tip a preset contributes to a tip grid: (shape, size, hardness, angle, roundness, spacing).
pub fn preset_tip(p: &BrushPreset) -> (&TipShape, f32, f32, f32, f32, f32) {
    let b = &p.brush;
    (&b.tip, b.size, b.hardness, b.angle, b.roundness, b.spacing)
}

/// One pane's drawing state: the app (tip grids, pattern library, open curves) and the pass's report.
struct Pane<'a> {
    app: &'a mut PhotocraftApp,
    out: SectionOut,
}

impl Pane<'_> {
    /// A number row (caption, field, slider) showing `shown`; whether it was edited. Typed and dragged values
    /// land on whole units.
    fn number(&mut self, ui: &mut Ui, label: &str, shown: &mut f64, reset: f64, s: Shown, enabled: bool) -> bool {
        let mut field = SliderField::new(label.to_owned(), Bind::bound(shown).reset(reset), s.min..=s.max).step(1.0).enabled(enabled);
        if let Some(unit) = s.unit {
            field = field.unit_kind(unit);
        }
        let edit = field.show(ui);
        self.out.note(&edit)
    }

    /// A row over the stored `v`, shown as `s` says. Only an edit writes `v`, so a fractional import stays as it is
    /// until the user changes it.
    fn field(&mut self, ui: &mut Ui, label: &str, v: &mut f32, reset: f32, s: Shown, enabled: bool) -> bool {
        let mut shown = (f64::from(*v) * s.factor).round().clamp(s.min, s.max);
        let reset = (f64::from(reset) * s.factor).round().clamp(s.min, s.max);
        if !self.number(ui, label, &mut shown, reset, s, enabled) {
            return false;
        }
        *v = ((shown / s.factor) as f32).clamp((s.min / s.factor) as f32, (s.max / s.factor) as f32);
        true
    }

    /// A whole-number row over a count or size.
    fn whole(&mut self, ui: &mut Ui, label: &str, v: &mut u32, reset: u32, range: RangeInclusive<f64>, unit: Option<UnitKind>) -> bool {
        let mut shown = f64::from(*v).clamp(*range.start(), *range.end());
        let s = Shown::whole(*range.start(), *range.end(), unit);
        if !self.number(ui, label, &mut shown, f64::from(reset), s, true) {
            return false;
        }
        *v = shown as u32;
        true
    }

    fn check(&mut self, ui: &mut Ui, label: &str, v: &mut bool) -> bool {
        let edit = Checkbox::new(Bind::bound(v)).label(label.to_owned()).show(ui);
        self.out.note(&edit)
    }

    /// `label`'s select over the mask modes.
    fn mode(&mut self, ui: &mut Ui, label: &str, mode: &mut MaskMode) {
        let entries: Vec<Entry<MaskMode>> = MASK_MODES.iter().map(|(m, l)| Entry::new(*m, tl!(*l).to_string())).collect();
        Field::new(label.to_owned()).show(ui, |ui, _| {
            let edit = Select::new(label.to_owned(), Bind::bound(&mut *mode), &entries).show(ui);
            self.out.note(&edit);
        });
    }

    /// The tip grid: the presets' tips as a locked grid of the Brushes panel's rows. Returns the clicked preset's
    /// name.
    fn tip_grid(&mut self, ui: &mut Ui, id: &'static str, selected: Option<BrushKey>) -> Option<String> {
        let parts = BrushCardParts::ALL.set(BrushCardPart::Name, false).set(BrushCardPart::Stroke, false);
        let source = brush_source(&self.app.session.tools.presets, parts);
        let mut state = PresetState::<BrushKey>::new();
        let mut browser = PresetBrowser::new(tl!("Brush tips"), Bind::bound(&mut state), &source)
            .browser_id(egui::Id::unique(id))
            .view_lock(PresetView::Grid)
            .toolbar(false)
            .room_height(GRID_HEIGHT);
        if let Some(key) = selected {
            browser = browser.selected(key);
        }
        browser.show(ui).events.into_iter().find_map(|event| match event {
            PresetEvent::Select { key: BrushKey::Preset(name), .. } => Some(name),
            _ => None,
        })
    }

    /// The tip of the preset called `name`.
    fn preset_tip(&self, name: &str) -> Option<(TipShape, f32, f32, f32, f32, f32)> {
        let p = self.app.session.tools.presets.iter().find(|p| p.name == name)?;
        let (tip, size, hardness, angle, roundness, spacing) = preset_tip(p);
        Some((tip.clone(), size, hardness, angle, roundness, spacing))
    }

    /// A Size row over `size` pixels, 1 to `max`; `tip` supplies the original size of a sampled tip.
    fn size(&mut self, ui: &mut Ui, size: &mut f32, max: f64, tip: &TipShape) {
        let mut diameter = f64::from(*size);
        let mut row = BrushSizeField::new(tl!("Size"), Bind::bound(&mut diameter), max);
        if let Some((w, h)) = tip.bitmap_size() {
            row = row.restore(w.max(h) as f64);
        }
        let out = row.show(ui);
        if self.out.note(&out.edit) {
            *size = (diameter as f32).clamp(1.0, max as f32);
        }
    }

    /// `Control: [Off ▾] [25] [~]`, with this dynamic's response curve editor under it while it is open.
    /// `angle` offers the direction controls (Photoshop only has them for Angle Jitter).
    fn control_row(&mut self, ui: &mut Ui, id: &'static str, d: &mut Dynamic, angle: bool) {
        let n = if angle || matches!(d.control, Control::InitialDirection | Control::Direction) { 8 } else { 6 };
        let controls: Vec<DynamicControl<Control>> = CONTROLS[..n]
            .iter()
            .map(|(control, label)| {
                let entry = DynamicControl::new(*control, tl!(*label));
                match control {
                    Control::Fade if !angle => entry.fade().curve(),
                    Control::PenPressure | Control::PenTilt | Control::StylusWheel | Control::Rotation if !angle => entry.curve(),
                    _ => entry,
                }
            })
            .collect();
        let open = self.app.ui.brush_curves_open.contains(id);
        let shaped = !d.curve.is_linear();
        let mut steps = f64::from(d.fade_steps);
        let row = DynamicControlRow::new(&controls, Bind::bound(&mut d.control), Bind::bound(&mut steps)).curve(open, shaped).show(ui);
        if self.out.note_events(&row.steps) {
            d.fade_steps = steps.round().clamp(1.0, 9999.0) as u32;
        }
        let open = if row.curve_toggled {
            if !self.app.ui.brush_curves_open.remove(id) {
                self.app.ui.brush_curves_open.insert(id);
            }
            !open
        } else {
            open
        };
        let shows_curve = controls.iter().any(|entry| entry.value == d.control && entry.curve);
        if open && shows_curve {
            let mut points = d.curve.points();
            if self.out.note_events(&pressure_curve_ui::editor(ui, tl!("Response curve"), &mut points)) {
                d.curve = paint::ResponseCurve::new(&points);
            }
        }
    }

    /// A jitter row, its Control row and (optionally) the Minimum row: one Photoshop block. Minimum belongs to the
    /// control: Off disables editing without discarding its saved value.
    fn dynamic(&mut self, ui: &mut Ui, row: DynamicRow<'_>, d: &mut Dynamic, reset: &Dynamic) {
        let DynamicRow { id, label, angle, minimum } = row;
        self.field(ui, label, &mut d.jitter, reset.jitter, AMOUNT, true);
        self.control_row(ui, id, d, angle);
        if let Some(label) = minimum {
            self.field(ui, label, &mut d.minimum, reset.minimum, AMOUNT, d.control != Control::Off);
        }
    }

    /// The Scatter row with its Both Axes switch under it.
    fn scatter(&mut self, ui: &mut Ui, amount: &mut f32, both_axes: &mut bool, reset: f32) {
        self.field(ui, tl!("Scatter"), amount, reset, Shown::percent(0.0, 1000.0), true);
        self.check(ui, tl!("Both Axes"), both_axes);
    }

    /// The Spacing switch with its percent field (1 to 1000 %) beside it, enabled with the switch.
    fn spacing(&mut self, ui: &mut Ui, b: &mut BrushSettings, reset: f32) {
        Row::new().show(ui, |ui, _| {
            self.check(ui, tl!("Spacing"), &mut b.spacing_enabled);
            let s = Shown::percent(1.0, 1000.0);
            let mut shown = (f64::from(b.spacing) * s.factor).round().clamp(s.min, s.max);
            let reset = (f64::from(reset) * s.factor).round().clamp(s.min, s.max);
            let edit = SliderField::new(tl!("Spacing"), Bind::bound(&mut shown).reset(reset), s.min..=s.max)
                .layout(SliderFieldLayout::Inline)
                .step(1.0)
                .unit_kind(UnitKind::Percent)
                .enabled(b.spacing_enabled)
                .show(ui);
            if self.out.note(&edit) {
                b.spacing = ((shown / s.factor) as f32).clamp(0.01, 10.0);
            }
        });
    }

    /// Brush Tip Shape: the tip grid and the tip's own controls. Picking a tip copies the tip fields only (the
    /// dynamics sections stay as they are, as in Photoshop) and reports the preset's name so the caller can mark it
    /// current; the fields are also copied into the working brush so this frame's controls start from them.
    fn tip_shape(&mut self, ui: &mut Ui, b: &mut BrushSettings, d: &BrushSettings) {
        let selected = {
            let tools = &self.app.session.tools;
            tools
                .current_preset
                .as_deref()
                .and_then(|name| tools.presets.iter().find(|p| p.name.eq_ignore_ascii_case(name)))
                .map(|p| BrushKey::Preset(p.name.clone()))
        };
        if let Some(name) = self.tip_grid(ui, "brush-tip-grid", selected)
            && let Some((tip, size, hardness, angle, roundness, spacing)) = self.preset_tip(&name)
        {
            (b.tip, b.size, b.hardness, b.angle, b.roundness, b.spacing) = (tip.into(), size, hardness, angle, roundness, spacing);
            self.out.picked = Some(name);
            return;
        }
        self.size(ui, &mut b.size, f64::from(MAX_BRUSH_SIZE), &b.tip.clone());
        Row::new().show(ui, |ui, _| {
            self.check(ui, tl!("Flip X"), &mut b.flip_x);
            self.check(ui, tl!("Flip Y"), &mut b.flip_y);
        });
        let degrees = Shown::whole(-180.0, 180.0, Some(UnitKind::Degrees));
        self.field(ui, tl!("Angle:"), &mut b.angle, d.angle, degrees, true);
        self.field(ui, tl!("Roundness:"), &mut b.roundness, d.roundness, Shown::percent(1.0, 100.0), true);
        let mut tip = TipAngle::new(f64::from(b.angle), f64::from(b.roundness));
        let edit = TipEllipse::new(tl!("Angle and roundness"), Bind::bound(&mut tip)).show(ui);
        if self.out.note(&edit) {
            b.angle = tip.degrees as f32;
            b.roundness = tip.roundness as f32;
        }
        let round = matches!(*b.tip, TipShape::Round);
        self.field(ui, tl!("Hardness"), &mut b.hardness, d.hardness, AMOUNT, round);
        self.spacing(ui, b, d.spacing);
    }

    fn shape_dynamics(&mut self, ui: &mut Ui, b: &mut BrushSettings, d: &BrushSettings) {
        let (sd, dd) = (&mut b.shape_dynamics, &d.shape_dynamics);
        self.field(ui, tl!("Size Jitter"), &mut sd.size.jitter, dd.size.jitter, AMOUNT, true);
        self.control_row(ui, "brush-size-ctl", &mut sd.size, false);
        self.field(ui, tl!("Minimum Diameter"), &mut sd.size.minimum, dd.size.minimum, AMOUNT, sd.size.control != Control::Off);
        // Tilt Scale applies with the size control on Pen Tilt (greyed out otherwise, like Photoshop).
        let tilt = sd.size.control == Control::PenTilt;
        self.field(ui, tl!("Tilt Scale"), &mut sd.tilt_scale, dd.tilt_scale, Shown::percent(0.0, 200.0), tilt);
        self.dynamic(ui, DynamicRow { id: "brush-angle-ctl", label: tl!("Angle Jitter"), angle: true, minimum: None }, &mut sd.angle, &dd.angle);
        self.dynamic(
            ui,
            DynamicRow { id: "brush-round-ctl", label: tl!("Roundness Jitter"), angle: false, minimum: Some(tl!("Minimum Roundness")) },
            &mut sd.roundness,
            &dd.roundness,
        );
        Row::new().show(ui, |ui, _| {
            self.check(ui, tl!("Flip X Jitter"), &mut sd.flip_x_jitter);
            self.check(ui, tl!("Flip Y Jitter"), &mut sd.flip_y_jitter);
        });
        self.check(ui, tl!("Brush Projection"), &mut sd.brush_projection);
    }

    fn scattering(&mut self, ui: &mut Ui, b: &mut BrushSettings, d: &BrushSettings) {
        let (sc, dd) = (&mut b.scattering, &d.scattering);
        self.scatter(ui, &mut sc.scatter.jitter, &mut sc.both_axes, dd.scatter.jitter);
        self.control_row(ui, "brush-scatter-ctl", &mut sc.scatter, false);
        self.whole(ui, tl!("Count"), &mut sc.count, dd.count, 1.0..=16.0, None);
        self.dynamic(ui, DynamicRow { id: "brush-count-ctl", label: tl!("Count Jitter"), angle: false, minimum: None }, &mut sc.count_jitter, &dd.count_jitter);
    }

    /// The pick of a library pattern as the brush's tile.
    fn texture_pattern(&mut self, ui: &mut Ui, tx: &mut paint::Texture) {
        let selected = tx.pattern_id.clone().unwrap_or_default();
        if let Some(id) = pattern_picker(self.app, ui, &selected)
            && let Some(tile) = self.app.session.patterns.items.iter().find(|p| p.id == id).map(library_tile)
        {
            tx.pattern = Pattern::Tile(tile);
            tx.pattern_id = Some(id);
        }
    }

    fn texture(&mut self, ui: &mut Ui, b: &mut BrushSettings, d: &BrushSettings) {
        let (tx, dd) = (&mut b.texture, &d.texture);
        self.texture_pattern(ui, tx);
        let mut style = match &tx.pattern {
            Pattern::Procedural { style, .. } => Some(*style),
            Pattern::Tile(_) | Pattern::Stored(_) => None,
        };
        let held = style;
        let mut entries: Vec<Entry<Option<PatternStyle>>> = PATTERNS.iter().map(|(s, l)| Entry::new(Some(*s), tl!(*l).to_string())).collect();
        if style.is_none() {
            entries.push(Entry::new(None, tl!("Custom").to_string()));
        }
        Field::new(tl!("Pattern")).show(ui, |ui, _| {
            let edit = Select::new(tl!("Pattern"), Bind::bound(&mut style), &entries).show(ui);
            self.out.note(&edit);
        });
        if style != held
            && let Some(s) = style
        {
            let size = match &tx.pattern {
                Pattern::Procedural { size, .. } => *size,
                Pattern::Tile(_) | Pattern::Stored(_) => 128,
            };
            tx.pattern = Pattern::Procedural { style: s, size, seed: 1 };
            tx.pattern_id = None;
        }
        self.check(ui, tl!("Invert"), &mut tx.invert);
        if let Pattern::Procedural { size, .. } = &mut tx.pattern {
            self.whole(ui, tl!("Pattern Size"), size, 128, 16.0..=1024.0, Some(UnitKind::Pixels));
        }
        self.field(ui, tl!("Scale"), &mut tx.scale, dd.scale, Shown::percent(0.0, 1000.0), true);
        let brightness = Shown { factor: 150.0, min: -150.0, max: 150.0, unit: None };
        self.field(ui, tl!("Brightness"), &mut tx.brightness, dd.brightness, brightness, true);
        // Photoshop's -50..100 contrast onto the model's -1..1.
        let shown_contrast = |c: f32| f64::from(if c < 0.0 { c * 50.0 } else { c * 100.0 }.round());
        let mut contrast = shown_contrast(tx.contrast);
        if self.number(ui, tl!("Contrast"), &mut contrast, shown_contrast(dd.contrast), Shown::whole(-50.0, 100.0, None), true) {
            tx.contrast = (if contrast < 0.0 { contrast / 50.0 } else { contrast / 100.0 }) as f32;
        }
        self.check(ui, tl!("Texture Each Tip"), &mut tx.each_tip);
        self.mode(ui, tl!("Mode:"), &mut tx.mode);
        self.field(ui, tl!("Depth"), &mut tx.depth, dd.depth, AMOUNT, true);
        let each = tx.each_tip;
        Stack::new().enabled(each).show(ui, |ui, _| {
            self.field(ui, tl!("Minimum Depth"), &mut tx.depth_jitter.minimum, dd.depth_jitter.minimum, AMOUNT, true);
            self.field(ui, tl!("Depth Jitter"), &mut tx.depth_jitter.jitter, dd.depth_jitter.jitter, AMOUNT, true);
            self.control_row(ui, "brush-depth-ctl", &mut tx.depth_jitter, false);
        });
    }

    fn dual_brush(&mut self, ui: &mut Ui, b: &mut BrushSettings, d: &BrushSettings) {
        let (db, dd) = (&mut b.dual_brush, &d.dual_brush);
        self.mode(ui, tl!("Mode:"), &mut db.mode);
        self.check(ui, tl!("Flip"), &mut db.flip);
        // The dual tip is a sub-setting, not a selection: no persistent highlight (a look-alike match would highlight
        // an arbitrary duplicate).
        if let Some(name) = self.tip_grid(ui, "brush-dual-grid", None)
            && let Some((tip, size, hardness, angle, roundness, spacing)) = self.preset_tip(&name)
        {
            (db.tip, db.size, db.hardness, db.angle, db.roundness, db.spacing) = (tip, size, hardness, angle, roundness, spacing);
        }
        self.size(ui, &mut db.size, MAX_DUAL_SIZE, &db.tip.clone());
        self.field(ui, tl!("Spacing"), &mut db.spacing, dd.spacing, Shown::percent(1.0, 1000.0), true);
        self.scatter(ui, &mut db.scatter, &mut db.both_axes, dd.scatter);
        self.whole(ui, tl!("Count"), &mut db.count, dd.count, 1.0..=16.0, None);
    }

    fn color_dynamics(&mut self, ui: &mut Ui, b: &mut BrushSettings, d: &BrushSettings) {
        let (c, dd) = (&mut b.color_dynamics, &d.color_dynamics);
        self.check(ui, tl!("Apply Per Tip"), &mut c.per_tip);
        self.dynamic(ui, DynamicRow { id: "brush-fgbg-ctl", label: tl!("Foreground/Background Jitter"), angle: false, minimum: None }, &mut c.fg_bg, &dd.fg_bg);
        self.field(ui, tl!("Hue Jitter"), &mut c.hue_jitter, dd.hue_jitter, AMOUNT, true);
        self.field(ui, tl!("Saturation Jitter"), &mut c.saturation_jitter, dd.saturation_jitter, AMOUNT, true);
        self.field(ui, tl!("Brightness Jitter"), &mut c.brightness_jitter, dd.brightness_jitter, AMOUNT, true);
        self.field(ui, tl!("Purity"), &mut c.purity, dd.purity, Shown::percent(-100.0, 100.0), true);
    }

    fn transfer(&mut self, ui: &mut Ui, b: &mut BrushSettings, d: &BrushSettings, tool: Tool) {
        let (tr, dd) = (&mut b.transfer, &d.transfer);
        let minimum = Some(tl!("Minimum"));
        self.dynamic(ui, DynamicRow { id: "brush-opacity-ctl", label: tl!("Opacity Jitter"), angle: false, minimum }, &mut tr.opacity, &dd.opacity);
        self.dynamic(ui, DynamicRow { id: "brush-flow-ctl", label: tl!("Flow Jitter"), angle: false, minimum }, &mut tr.flow, &dd.flow);
        // Wetness and mix only affect the Mixer Brush. Keep their settings visible and saved for other tools, but
        // disable the whole blocks, including controllers and minimums.
        Stack::new().enabled(tool == Tool::MixerBrush).show(ui, |ui, _| {
            self.dynamic(ui, DynamicRow { id: "brush-wet-ctl", label: tl!("Wetness Jitter"), angle: false, minimum }, &mut tr.wetness, &dd.wetness);
            self.dynamic(ui, DynamicRow { id: "brush-mix-ctl", label: tl!("Mix Jitter"), angle: false, minimum }, &mut tr.mix, &dd.mix);
        });
        self.mixer_options(ui, &mut b.mixer, &d.mixer);
    }

    /// The Mixer Brush options the brush carries (Photoshop shows them in the Mixer Brush's options bar;
    /// `paint.mixerBrush` uses them unless its params override them).
    fn mixer_options(&mut self, ui: &mut Ui, m: &mut paint::mixer::MixerSettings, d: &paint::mixer::MixerSettings) {
        Divider::horizontal().show(ui);
        Text::new(tl!("Mixer Brush")).shade(Shade::Muted).show(ui);
        self.field(ui, tl!("Wet"), &mut m.wet, d.wet, AMOUNT, true);
        self.field(ui, tl!("Load"), &mut m.load, d.load, AMOUNT, true);
        self.field(ui, tl!("Mix"), &mut m.mix, d.mix, AMOUNT, true);
        self.field(ui, tl!("Flow"), &mut m.flow, d.flow, AMOUNT, true);
        self.check(ui, tl!("Sample All Layers"), &mut m.sample_all_layers);
        self.check(ui, tl!("Load the brush after each stroke"), &mut m.load_after_stroke);
        self.check(ui, tl!("Clean the brush after each stroke"), &mut m.clean_after_stroke);
    }

    fn pose(&mut self, ui: &mut Ui, b: &mut BrushSettings, d: &BrushSettings) {
        let (p, dd) = (&mut b.pose, &d.pose);
        self.field(ui, tl!("Tilt X"), &mut p.tilt_x, dd.tilt_x, TILT, true);
        self.check(ui, tl!("Override Tilt X"), &mut p.override_tilt_x);
        self.field(ui, tl!("Tilt Y"), &mut p.tilt_y, dd.tilt_y, TILT, true);
        self.check(ui, tl!("Override Tilt Y"), &mut p.override_tilt_y);
        self.field(ui, tl!("Rotation"), &mut p.rotation, dd.rotation, Shown::whole(0.0, 360.0, Some(UnitKind::Degrees)), true);
        self.check(ui, tl!("Override Rotation"), &mut p.override_rotation);
        self.field(ui, tl!("Pressure"), &mut p.pressure, dd.pressure, AMOUNT, true);
        self.check(ui, tl!("Override Pressure"), &mut p.override_pressure);
    }
}

/// A grayscale tile of the library pattern `p` for the brush's texture: luminance, sampled so the longer side is at
/// most `LIBRARY_TILE_SIDE` (a pattern file can declare a padded source of any size).
fn library_tile(p: &photocraft_doc::Pattern) -> paint::tile::GrayTile {
    let (w, h) = (p.width.max(1), p.height.max(1));
    let format = p.surface.format();
    let step = (w.max(h) as f32 / LIBRARY_TILE_SIDE as f32).max(1.0);
    let (tw, th) = (((w as f32 / step) as u32).max(1), ((h as f32 / step) as u32).max(1));
    paint::tile::GrayTile::from_fn(tw, th, |x, y| {
        let (sx, sy) = (((x as f32 * step) as u32).min(w - 1), ((y as f32 * step) as u32).min(h - 1));
        let px = p.surface.read_region(photocraft_geom::Rect::from_xywh(sx as i32, sy as i32, 1, 1));
        let [r, g, b, _] = photocraft_raster::to_rgba(&format, &px);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    })
}

/// Controls for section `i` (see [`crate::brush_panel::SECTIONS`]; the panel sends any other index back to 0).
/// The tip grids read the preset library from `app`; the active `tool` determines which Transfer controls apply.
/// Reports the name of a preset whose tip was picked in the Brush Tip Shape grid and the history key of the pass's
/// last edit.
pub(crate) fn section_body(app: &mut PhotocraftApp, ui: &mut Ui, b: &mut BrushSettings, i: usize, tool: Tool) -> SectionOut {
    let defaults = BrushSettings::default();
    let mut pane = Pane { app, out: SectionOut::default() };
    match i {
        0 => pane.tip_shape(ui, b, &defaults),
        1 => pane.shape_dynamics(ui, b, &defaults),
        2 => pane.scattering(ui, b, &defaults),
        3 => pane.texture(ui, b, &defaults),
        4 => pane.dual_brush(ui, b, &defaults),
        5 => pane.color_dynamics(ui, b, &defaults),
        6 => pane.transfer(ui, b, &defaults, tool),
        _ => pane.pose(ui, b, &defaults),
    }
    pane.out
}

#[cfg(test)]
mod tests;
