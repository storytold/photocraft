//! Display names for the measured Actions editor glyph identifiers (#10946).
//!
//! Source preparation: include this module in the real editor when its consumer is added.
//! Lookup is exact and display-only. Keep the original metadata identifier when saving.

/// The picker uses six columns, with entries ordered row by row.
pub const GLYPH_COLUMNS: usize = 6;

/// Measured identifiers and PhotoCraft's original asset names, in picker order.
pub const GLYPHS: [(&str, &str); 30] = [
    ("ClickAction", "action-editor-click"),
    ("ActionsCrop", "action-editor-crop"),
    ("LayerMerge", "action-editor-merge"),
    ("AdjustmentGeneral", "action-editor-adjust"),
    ("HueAndSat", "action-editor-hue"),
    ("ActionsExposure", "action-editor-exposure"),
    ("ColorLookUpTables", "action-editor-lut"),
    ("BlackAndWhite", "action-editor-black-white"),
    ("SmartRemove", "action-editor-smart-remove"),
    ("ActionsDodge", "action-editor-dodge"),
    ("ActionsBurn", "action-editor-burn"),
    ("ActionsErase", "action-editor-erase"),
    ("LayerEffects", "action-editor-layer-effects"),
    ("AIGenEffects", "action-editor-generative-effects"),
    ("Sharpen", "action-editor-sharpen"),
    ("ObjectSelectionLasso", "action-editor-object-select"),
    ("MagicWand", "action-editor-wand"),
    ("AIGenerate", "action-editor-generate"),
    ("TextColor", "action-editor-text-color"),
    ("ActionsText", "action-editor-text"),
    ("ActionsDownload", "action-editor-download"),
    ("ActionsLayers", "action-editor-layers"),
    ("Brush", "action-editor-brush"),
    ("SelectSubject", "action-editor-subject"),
    ("RemoveBackground", "action-editor-remove-background"),
    ("GaussianBlur", "action-editor-blur"),
    ("LayerHarmonize", "action-editor-harmonize"),
    ("P_AdjustClarity_32x32", "action-editor-clarity"),
    ("AIStyles", "action-editor-styles"),
    ("ShowGrid", "action-editor-grid"),
];

/// Resolve a display glyph without normalizing or replacing the stored identifier.
pub fn display_glyph(identifier: &str) -> &'static str {
    GLYPHS.iter().find_map(|&(key, asset)| (key == identifier).then_some(asset)).unwrap_or("square")
}
