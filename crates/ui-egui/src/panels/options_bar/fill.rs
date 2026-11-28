//! The Paint Bucket's members of the options bar, and the Mode list of the Paint Bucket and the stamp tools.

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, bind_tracked};
use craft::ui::patterns::blend_keys::{BlendKeyEntry, blend_entries};
use craft::ui::pickers::Select;
use photocraft_doc::BlendMode;

use super::labelled;
use super::paint::PaintMode;
use crate::PhotocraftApp;
use crate::state::Tool;
use crate::tools::option_cells::number;

/// The text of a Mode entry.
fn label_of(mode: PaintMode) -> String {
    match mode {
        PaintMode::Blend(blend) => tl!(blend.label()).to_owned(),
        PaintMode::Behind => tl!("Behind").to_owned(),
        PaintMode::Clear => tl!("Clear").to_owned(),
        PaintMode::HealingReplace => tl!("Replace").to_owned(),
    }
}

/// The Mode list of `tool` in Photoshop's order, with a rule above each blend group and none first. Behind (Pattern
/// Stamp) is listed always and enabled by `behind_enabled` (it is greyed on the Background). `threshold` names Normal
/// "Threshold", as the reference does for the stamps in an Indexed Color document. Empty for a tool without a blend
/// mode control.
pub(crate) fn mode_entries(tool: Tool, behind_enabled: bool, threshold: bool) -> Vec<Entry<PaintMode>> {
    static CACHE: crate::i18n::LangCache<(Tool, bool, bool), Vec<Entry<PaintMode>>> = crate::i18n::LangCache::new();
    CACHE.get_or_build((tool, behind_enabled, threshold), || {
        if tool == Tool::Healing {
            // The Healing Brush's measured list (#12101): Normal, Replace, a rule, Multiply,
            // Screen, Darken, Lighten, a rule, Color, Luminosity. Not the blend-group rules: the
            // reference starts its groups elsewhere in this short list. Replace has no letter: the
            // ⇧⌥ keys and the ⇧+ / ⇧− cycle keep the seven blend modes.
            let blend = |b: BlendMode| Entry::new(PaintMode::Blend(b), label_of(PaintMode::Blend(b)));
            return vec![
                blend(BlendMode::Normal),
                Entry::new(PaintMode::HealingReplace, label_of(PaintMode::HealingReplace)),
                Entry::separator(),
                blend(BlendMode::Multiply),
                blend(BlendMode::Screen),
                blend(BlendMode::Darken),
                blend(BlendMode::Lighten),
                Entry::separator(),
                blend(BlendMode::Color),
                blend(BlendMode::Luminosity),
            ];
        }
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
    })
}

/// The Mode select over `entries` (from [`mode_entries`]) and the bound `mode`.
pub(crate) fn mode_select<'a>(mode: Bind<'a, PaintMode>, entries: &'a [Entry<PaintMode>]) -> Select<'a, PaintMode> {
    Select::new(tl!("Mode").to_owned(), mode, entries)
}

/// The reference's alert for a Paint Bucket Opacity typed past 1..100 % (#12533). The framework's
/// whole-number alert would write "1 %" with a space; the reference writes "1%".
pub(crate) const PAINT_BUCKET_OPACITY_ALERT: &str = "An integer between 1% and 100% is required. Closest value inserted.";

/// The reference's alert for a Paint Bucket Tolerance typed past 0..255 or as a fraction (#12533),
/// the framework's wording of a whole-number range without a unit.
pub(crate) const PAINT_BUCKET_TOLERANCE_ALERT: &str = "An integer between 0 and 255 is required. Closest value inserted.";

/// The Paint Bucket's members: Fill source, Mode, Opacity, Tolerance, Anti-alias, Contiguous and All Layers. Opacity
/// takes whole percents 1..100 and Tolerance whole numbers 0..255; a value typed past either range becomes the
/// closest one with the reference's alert, as does a fraction typed in Tolerance; a fraction typed in Opacity rounds
/// without one (#12533).
pub(super) fn members(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.ui.tool != Tool::PaintBucket {
        return;
    }
    let entries = mode_entries(Tool::PaintBucket, true, false);
    let sources = [Entry::new(false, tl!("Foreground").to_owned()), Entry::new(true, tl!("Pattern").to_owned())];
    // The Source dropdown stands alone, without a caption, as in the reference.
    Select::new(tl!("Source").to_owned(), bind_tracked!(app.ui.tool_options => bucket_fill_pattern), &sources)
        .tooltip(tl!("Set source for fill area").to_owned())
        .show(ui);
    let o = &app.ui.tool_options;
    let shown = if o.bucket_clear { PaintMode::Clear } else { PaintMode::Blend(o.bucket_blend_mode) };
    let mut mode = shown;
    labelled(ui, tl!("Mode"), |ui| mode_select(Bind::bound(&mut mode), &entries).tooltip(tl!("Fill Mode").to_owned()).show(ui));
    if mode != shown {
        let o = &mut app.ui.tool_options;
        match mode {
            PaintMode::Clear => o.bucket_clear = true,
            PaintMode::Blend(blend) => {
                o.bucket_clear = false;
                o.bucket_blend_mode = blend;
            }
            PaintMode::Behind => {}
            // Replace belongs to the Healing Brush's list, never the Paint Bucket's.
            PaintMode::HealingReplace => {}
        }
    }
    let opacity = number(tl!("Opacity"), bind_tracked!(app.ui.tool_options => fill_opacity), 1.0..=100.0)
        .whole()
        .unit("%")
        .tooltip(tl!("Set opacity for fill"))
        .show(ui)
        .out_of_range;
    let tolerance = number(tl!("Tolerance"), bind_tracked!(app.ui.tool_options => tolerance), 0.0..=255.0)
        .whole_alerting_fractions()
        .tooltip(tl!("Range of colors to fill"))
        .show(ui);
    let (anti_alias, contiguous, all_layers) = bind_tracked!(app.ui.tool_options => anti_alias, contiguous, sample_all.paint_bucket);
    Checkbox::new(anti_alias).label(tl!("Anti-alias").to_owned()).tooltip(tl!("Smoothes edge transition").to_owned()).show(ui);
    Checkbox::new(contiguous).label(tl!("Contiguous").to_owned()).tooltip(tl!("Fill only contiguous pixels").to_owned()).show(ui);
    Checkbox::new(all_layers).label(tl!("All Layers").to_owned()).tooltip(tl!("Fill composite image").to_owned()).show(ui);
    if opacity.is_some() {
        let _ = crate::dialogs::alerts::alert(app, tl!(PAINT_BUCKET_OPACITY_ALERT));
    } else if let Some(alert) = tolerance.out_of_range {
        // The framework words a whole-number field without a unit as the reference does:
        // "An integer between 0 and 255 is required. Closest value inserted."
        crate::tools::option_cells::range_alert(app, ui, &alert);
    } else if tolerance.rounded {
        let _ = crate::dialogs::alerts::alert(app, tl!(PAINT_BUCKET_TOLERANCE_ALERT));
    }
}
