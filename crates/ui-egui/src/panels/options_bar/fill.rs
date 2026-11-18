//! The Paint Bucket's members of the options bar, and the Mode list of the Paint Bucket and the stamp tools.

use craft::ui::contract::{Bind, Entry, Show as _};
use craft::ui::patterns::blend_keys::{BlendKeyEntry, blend_entries};
use craft::ui::pickers::Select;
use photocraft_doc::BlendMode;

use super::paint::PaintMode;
use super::{check, labelled, number_field};
use crate::PhotocraftApp;
use crate::state::Tool;

/// The text of a Mode entry.
fn label_of(mode: PaintMode) -> String {
    match mode {
        PaintMode::Blend(blend) => tl!(blend.label()).to_owned(),
        PaintMode::Behind => tl!("Behind").to_owned(),
        PaintMode::Clear => tl!("Clear").to_owned(),
    }
}

/// The Mode list of `tool` in Photoshop's order, with a rule above each blend group and none first. Behind (Pattern
/// Stamp) is listed always and enabled by `behind_enabled` (it is greyed on the Background). `threshold` names Normal
/// "Threshold", as the reference does for the stamps in an Indexed Color document. Empty for a tool without a blend
/// mode control.
pub(crate) fn mode_entries(tool: Tool, behind_enabled: bool, threshold: bool) -> Vec<Entry<PaintMode>> {
    let keyed: Vec<BlendKeyEntry<PaintMode>> = if tool == Tool::PatternStamp {
        crate::brush_mode_menu::pattern_stamp_choices()
            .into_iter()
            .filter_map(|((mode, behind), _)| match (mode, behind) {
                (Some(blend), false) => Some(crate::panels::blend_entry(PaintMode::Blend(blend), blend)),
                (None, true) => Some(BlendKeyEntry::behind(PaintMode::Behind)),
                _ => None,
            })
            .collect()
    } else {
        crate::blend_keys::tool_modes(tool)
            .unwrap_or_default()
            .into_iter()
            .map(|mode| match mode {
                Some(blend) => crate::panels::blend_entry(PaintMode::Blend(blend), blend),
                None => BlendKeyEntry::clear(PaintMode::Clear),
            })
            .collect()
    };
    blend_entries(&keyed, |entry| match entry.key {
        PaintMode::Blend(BlendMode::Normal) if threshold => tl!("Threshold").to_owned().into(),
        key => label_of(key).into(),
    })
    .into_iter()
    .map(|entry| if entry.value() == Some(&PaintMode::Behind) { entry.enabled(behind_enabled) } else { entry })
    .collect()
}

/// The Mode select over `entries` (from [`mode_entries`]) and the bound `mode`.
pub(crate) fn mode_select<'a>(mode: Bind<'a, PaintMode>, entries: &'a [Entry<PaintMode>]) -> Select<'a, PaintMode> {
    Select::new(tl!("Mode").to_owned(), mode, entries)
}

/// The Paint Bucket's members: Fill source, Mode, Opacity, Tolerance, Anti-alias, Contiguous and All Layers.
pub(super) fn members(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.ui.tool != Tool::PaintBucket {
        return;
    }
    let entries = mode_entries(Tool::PaintBucket, true, false);
    let sources = [Entry::new(false, tl!("Foreground").to_owned()), Entry::new(true, tl!("Pattern").to_owned())];
    let o = &mut app.ui.tool_options;
    labelled(ui, tl!("Fill"), |ui| Select::new(tl!("Source").to_owned(), Bind::bound(&mut o.bucket_fill_pattern), &sources).show(ui));
    let mut mode = if o.bucket_clear { PaintMode::Clear } else { PaintMode::Blend(o.bucket_blend_mode) };
    labelled(ui, tl!("Mode"), |ui| mode_select(Bind::bound(&mut mode), &entries).show(ui));
    match mode {
        PaintMode::Clear => o.bucket_clear = true,
        PaintMode::Blend(blend) => {
            o.bucket_clear = false;
            o.bucket_blend_mode = blend;
        }
        PaintMode::Behind => {}
    }
    if let Some(opacity) = number_field(ui, tl!("Opacity"), o.fill_opacity, 1.0..=100.0, "%").landed() {
        o.fill_opacity = *opacity;
    }
    if let Some(tolerance) = number_field(ui, tl!("Tolerance"), o.tolerance, 0.0..=255.0, "").landed() {
        o.tolerance = *tolerance;
    }
    check(ui, tl!("Anti-alias"), &mut o.anti_alias);
    check(ui, tl!("Contiguous"), &mut o.contiguous);
    check(ui, tl!("All Layers"), &mut o.sample_all.paint_bucket);
}
