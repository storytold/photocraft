//! #12077: with a text layer active, the Horizontal Type Mask starts a new mask in that layer's
//! type settings - the font, style and size the options bar shows for the layer - and not in the
//! tool's fresh defaults. The measured document (800x600 RGB/8, an 80 pt "TXT" active) gives the
//! same selection size as the same text typed with the tool itself set to 80 pt and a pixel layer
//! active; the text layer is not changed and stays active after the commit.
//!
//! The issue's *not measured* rows keep today's behaviour, pinned here: the Vertical Type Mask, a
//! paragraph (box) drag, a mixed orientation (a vertical text layer active), a non-text active
//! layer, and the tool options after the mask. Clicks and the box drag go through the canvas's
//! tool events.

use egui::Modifiers;
use photocraft_doc::LayerId;
use serde_json::json;

use super::super::commit;
use super::super::mask_replace_selection_tests::{bounds, click, new_app, selection};
use super::super::{current_text, text_layer};
use crate::PhotocraftApp;
use crate::canvas::testing::{ToolEvent, tool_event};
use crate::state::Tool;

/// The layer's first character run, which is what `bar_character_values` names the layer by.
fn char_style(app: &PhotocraftApp, id: LayerId) -> photocraft_doc::text::CharStyle {
    let t = text_layer(&app.session.active().unwrap().doc, id).expect("a type layer");
    t.char_runs().into_iter().next().expect("a first character run").style
}

/// What the options bar shows for the layer: its first character run's family, style name and size.
fn style(app: &PhotocraftApp, id: LayerId) -> (String, String, f32) {
    let c = char_style(app, id);
    (c.font_family, c.font_style, c.size_pt)
}

/// The working layer the open mask session is typing into.
fn mask_style(app: &PhotocraftApp) -> (String, String, f32) {
    let ed = app.ui.text_edit.as_ref().expect("an open mask session");
    style(app, LayerId(ed.layer))
}

fn active_layer(app: &PhotocraftApp) -> Option<LayerId> {
    app.session.active().and_then(|st| st.active_layer)
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

/// [`new_app`]'s document, whose active layer is a type layer named `TXT` with the family
/// `"Inter"`, the style `"Bold"` and `pt` points - the issue's 80 pt text layer.
fn txt_active(pt: f32) -> (Box<PhotocraftApp>, LayerId) {
    let mut app = new_app();
    let id =
        app.run("type.create", json!({"text": "TXT", "x": 60, "y": 120, "font": "Inter", "fontStyle": "Bold", "size": pt})).unwrap()["layer"].as_u64().unwrap();
    let id = LayerId(id);
    // The tool's own options, deliberately different from the layer's.
    app.ui.tool_options.type_font = "Tool Font".into();
    app.ui.tool_options.type_style = "Regular".into();
    app.ui.tool_options.type_size = 48.0;
    (app, id)
}

/// Start a mask with a click at `at` and type `s`.
fn mask(app: &mut PhotocraftApp, at: (f64, f64), s: &str) {
    click(app, at.0, at.1, Modifiers::NONE);
    assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask && ed.created), "a new mask session at {at:?}");
    super::super::host::type_text(app, s);
}

/// A paragraph mask dragged from `from` to `to` (both ways over 4 screen pixels).
fn box_mask(app: &mut PhotocraftApp, from: (f64, f64), to: (f64, f64), s: &str) {
    tool_event(app, ToolEvent::Down { x: from.0, y: from.1, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: to.0, y: to.1 }, Modifiers::NONE);
    assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask && ed.created), "a new mask box {from:?} to {to:?}");
    super::super::host::type_text(app, s);
}

/// The size of the committed mask's selection, in whole pixels.
fn selection_size(app: &PhotocraftApp) -> (u32, u32) {
    let r = bounds(&selection(app)).expect("a Type Mask selection");
    (r.width(), r.height())
}

/// The measured row: an 80 pt text layer active and a click on empty canvas starts the mask at
/// 80 pt, not at the tool's 48 pt.
#[test]
fn a_mask_click_with_an_active_text_layer_starts_at_that_layers_size() {
    let (mut app, _) = txt_active(80.0);
    mask(&mut app, (60.0, 300.0), "MASK");
    assert_eq!(mask_style(&app).2, 80.0, "the active layer's size, not the tool's 48");
}

/// The measured acceptance "font, size and style as the options bar shows them for that layer".
#[test]
fn a_mask_click_takes_the_active_layers_font_and_style() {
    let (mut app, _) = txt_active(80.0);
    mask(&mut app, (60.0, 300.0), "MASK");
    assert_eq!(mask_style(&app), ("Inter".into(), "Bold".into(), 80.0), "the layer's font, style and size");
}

/// A 12 pt layer scaled 200 % shows 24 pt in the options bar and the Character panel, so that is
/// the size the mask takes.
#[test]
fn a_mask_click_takes_the_size_the_options_bar_shows_for_the_layer() {
    let (mut app, id) = txt_active(12.0);
    app.run("type.edit", json!({"layer": id.0, "transform": [2.0, 0.0, 0.0, 2.0, 60.0, 120.0]})).unwrap();
    mask(&mut app, (60.0, 300.0), "MASK");
    assert_eq!(mask_style(&app).2, 24.0, "12 pt at 200 % reads 24 pt, as the bar shows it");
}

/// The issue's comparison rows: the mask started by an active 80 pt text layer and the mask the
/// same text makes with a pixel layer active and the tool set to what the bar shows for that layer
/// (font, style and size) have the same selection size, and both differ from today's 48 pt result
/// made at the same point.
#[test]
fn the_active_layer_mask_selects_the_same_glyphs_as_the_tool_at_the_same_size() {
    let (mut from_layer, _) = txt_active(80.0);
    mask(&mut from_layer, (60.0, 300.0), "MASK");
    commit(&mut from_layer);
    let from_layer = selection_size(&from_layer);

    let (mut from_tool, _) = txt_active(80.0);
    from_tool.run("layer.new.layer", json!({})).unwrap();
    // The bar shows "Inter" "Bold" 80 pt for the layer, so the comparison tool is set to all three.
    from_tool.ui.tool_options.type_font = "Inter".into();
    from_tool.ui.tool_options.type_style = "Bold".into();
    from_tool.ui.tool_options.type_size = 80.0;
    mask(&mut from_tool, (60.0, 300.0), "MASK");
    commit(&mut from_tool);
    let from_tool = selection_size(&from_tool);

    let (mut at_48, _) = txt_active(80.0);
    at_48.run("layer.new.layer", json!({})).unwrap();
    mask(&mut at_48, (60.0, 300.0), "MASK");
    commit(&mut at_48);
    let at_48 = selection_size(&at_48);

    assert_eq!(from_layer, from_tool, "the same size as the tool set to the layer's size");
    assert_ne!(from_layer, at_48, "and not today's 48 pt outline {at_48:?}");
}

/// The second measured row: the text layer is not changed and stays active after the commit, and
/// the session still leaves only the selection behind.
#[test]
fn the_text_layer_is_unchanged_and_stays_active_after_the_commit() {
    let (mut app, id) = txt_active(80.0);
    let (before_style, before_text, count) = (style(&app, id), current_text(&app, id), layers(&app));
    mask(&mut app, (60.0, 300.0), "MASK");
    commit(&mut app);
    assert!(app.ui.text_edit.is_none(), "the session ended");
    assert_eq!(current_text(&app, id), before_text, "\"TXT\" is not changed");
    assert_eq!(style(&app, id), before_style, "nor its font, style or size");
    assert_eq!(active_layer(&app), Some(id), "and it is the active layer again after the commit");
    assert_eq!(layers(&app), count, "no layer is left behind");
    assert!(selection(&app).is_some(), "the selection is what the commit made");
}

/// The third measured row, and the case with no active layer at all: today's tool options still
/// apply. Guard test: passes before and after the change.
#[test]
fn a_non_text_active_layer_keeps_todays_tool_options() {
    // A document whose only layer is the Background pixel layer.
    let mut background = new_app();
    background.ui.tool_options.type_font = "Tool Font".into();
    background.ui.tool_options.type_style = "Regular".into();
    background.ui.tool_options.type_size = 48.0;
    // The empty pixel layer above "TXT" is active.
    let (mut pixel, _) = txt_active(80.0);
    pixel.run("layer.new.layer", json!({})).unwrap();
    // "TXT" is present but nothing is the active layer.
    let (mut none_active, _) = txt_active(80.0);
    none_active.run("select.deselectLayers", json!({})).unwrap();
    assert_eq!(active_layer(&none_active), None, "no active layer");
    let cases: Vec<(&str, Box<PhotocraftApp>)> = vec![("Background", background), ("pixel layer", pixel), ("no active layer", none_active)];
    for (what, mut app) in cases {
        mask(&mut app, (60.0, 300.0), "MASK");
        assert_eq!(mask_style(&app), ("Tool Font".into(), "Regular".into(), 48.0), "{what}: the tool options as today");
    }
}

/// Not measured and keeping today's behaviour, as a whole: the Vertical Type Mask, and a click
/// whose active text layer is vertical; a paragraph drag with a horizontal text layer active.
#[test]
fn the_vertical_mask_and_a_paragraph_drag_keep_todays_tool_options() {
    // The Vertical Type Mask with a horizontal text layer active.
    let (mut vertical, _) = txt_active(80.0);
    vertical.ui.tool = Tool::VerticalTypeMask;
    mask(&mut vertical, (600.0, 100.0), "MASK");
    assert_eq!(mask_style(&vertical), ("Tool Font".into(), "Regular".into(), 48.0), "the vertical mask keeps today's defaults");
    // The Horizontal Type Mask with a vertical text layer active.
    let (mut mixed, _) = new_vertical();
    mixed.ui.tool = Tool::HorizontalTypeMask;
    mask(&mut mixed, (600.0, 300.0), "MASK");
    assert_eq!(mask_style(&mixed), ("Tool Font".into(), "Regular".into(), 48.0), "a mixed orientation keeps today's defaults");
    // A paragraph (box) drag with a horizontal text layer active.
    let (mut dragged, _) = txt_active(80.0);
    box_mask(&mut dragged, (60.0, 250.0), (400.0, 340.0), "MASK");
    assert_eq!(mask_style(&dragged), ("Tool Font".into(), "Regular".into(), 48.0), "a box drag keeps today's defaults");
}

/// [`txt_active`] with a vertical type layer active instead.
fn new_vertical() -> (Box<PhotocraftApp>, LayerId) {
    let mut app = new_app();
    let id = app
        .run("type.create", json!({"text": "TXT", "x": 60, "y": 120, "font": "Inter", "fontStyle": "Bold", "size": 80.0, "orientation": "vertical"}))
        .unwrap()["layer"]
        .as_u64()
        .unwrap();
    app.ui.tool_options.type_font = "Tool Font".into();
    app.ui.tool_options.type_style = "Regular".into();
    app.ui.tool_options.type_size = 48.0;
    (app, LayerId(id))
}

/// The plain Type tool is not a Type Mask: with the same 80 pt layer active its click still uses
/// the tool options (#10824 covers the settings of a type target, not this). Guard test: passes
/// before and after the change.
#[test]
fn the_plain_type_tool_still_starts_at_the_tool_options() {
    let (mut app, _) = txt_active(80.0);
    app.ui.tool = Tool::Type;
    click(&mut app, 60.0, 300.0, Modifiers::NONE);
    let ed = app.ui.text_edit.clone().expect("a new type session");
    assert!(!ed.mask, "a type layer, not a mask");
    assert_eq!(mask_style(&app), ("Tool Font".into(), "Regular".into(), 48.0), "the Type tool's own defaults");
}

/// The zero case: a text layer with no characters still shows its font, style and size in the bar
/// (its one run), so the mask starts at them - the takeover only moves create parameters, it
/// cannot knock anything out.
#[test]
fn an_empty_active_text_layer_gives_its_style_too() {
    let mut app = new_app();
    app.run("type.create", json!({"text": "", "x": 60, "y": 120, "font": "Inter", "fontStyle": "Bold", "size": 80})).unwrap();
    app.ui.tool_options.type_font = "Tool Font".into();
    app.ui.tool_options.type_style = "Regular".into();
    app.ui.tool_options.type_size = 48.0;
    mask(&mut app, (60.0, 300.0), "MASK");
    assert_eq!(mask_style(&app), ("Inter".into(), "Bold".into(), 80.0), "the empty layer's run, as the bar shows it");
}

/// Not measured: which settings besides the size are taken over. Only the bar's font, style and
/// size move to the mask - the active layer's leading, tracking, baseline shift and underline do
/// not. Red before the change like the takeover itself: its last line names the three keys that
/// move over.
#[test]
fn a_mask_takes_no_other_character_settings_from_the_layer() {
    let mut app = new_app();
    app.run("type.create", json!({"text": "TXT", "x": 60, "y": 120, "font": "Inter", "fontStyle": "Bold", "size": 80, "leading": 100, "tracking": 200, "baselineShift": 5, "underline": true}))
        .unwrap();
    app.ui.tool_options.type_font = "Tool Font".into();
    app.ui.tool_options.type_style = "Regular".into();
    app.ui.tool_options.type_size = 48.0;
    mask(&mut app, (60.0, 300.0), "MASK");
    let id = LayerId(app.ui.text_edit.as_ref().expect("an open mask session").layer);
    let c = char_style(&app, id);
    assert_eq!((c.leading_pt, c.tracking, c.baseline_shift_pt, c.underline), (None, 0.0, 0.0, false), "only font, style and size move over");
    assert_eq!((c.font_family.as_str(), c.font_style.as_str(), c.size_pt), ("Inter", "Bold", 80.0), "those three do");
}

/// A layer that names no family or style (their placeholder for "the default"): the mask takes its
/// size but keeps the tool's own font and style, so no empty name reaches `type.create`.
#[test]
fn a_layer_that_names_no_family_or_style_gives_only_its_size() {
    let mut app = new_app();
    app.run("type.create", json!({"text": "TXT", "x": 60, "y": 120, "font": "", "fontStyle": "", "size": 80})).unwrap();
    app.ui.tool_options.type_font = "Tool Font".into();
    app.ui.tool_options.type_style = "Regular".into();
    app.ui.tool_options.type_size = 48.0;
    mask(&mut app, (60.0, 300.0), "MASK");
    assert_eq!(mask_style(&app), ("Tool Font".into(), "Regular".into(), 80.0), "the size is taken, the empty names are not");
}

/// A document as a corrupt file can leave it: the active layer's runs carry `bad` as their size, a
/// value no `type` command writes (the commands keep the size within 0.1-1296 pt).
fn corrupt_layer_size(app: &mut PhotocraftApp, id: LayerId, bad: f32) {
    let st = app.session.active_mut().unwrap();
    let mut doc = (*st.doc).clone();
    if let Some(photocraft_doc::LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
        t.size_pt = bad;
        for r in &mut t.runs {
            r.style.size_pt = bad;
        }
    }
    *st.doc_mut() = std::sync::Arc::new(doc);
    st.revision += 1;
}

/// The corrupt-document case: a layer size that is not a finite number. It stays out of the create
/// parameters, so the mask still starts - at the tool's own size, with the layer's font and style
/// taken as measured. Red without the gate on the finite product: the size becomes JSON `null`,
/// `type.create` refuses it, and the click does nothing at all.
#[test]
fn a_non_finite_layer_size_keeps_the_tool_size() {
    for bad in [f32::NAN, f32::INFINITY] {
        let (mut app, id) = txt_active(80.0);
        corrupt_layer_size(&mut app, id, bad);
        mask(&mut app, (60.0, 300.0), "MASK");
        assert_eq!(mask_style(&app), ("Inter".into(), "Bold".into(), 48.0), "the tool's size, not a null the engine rejects ({bad})");
    }
}

/// A shown size can also leave the range `type.create` accepts while staying finite: an 800 pt
/// layer under a 4x/0.7x transform shows sqrt(2.8) * 800 = 1338 pt, above the engine's 1296 pt
/// maximum, and a corrupt document can carry a 0 pt run, below its 0.1 pt minimum. Inheriting a
/// refused size fails the whole create, so the click would make no mask at all. The size stays
/// out of the parameters there too, and the mask still starts - at the tool's own size, the
/// layer's font and style taken. Red before the range gate: no mask session opens.
#[test]
fn a_shown_layer_size_outside_the_create_range_keeps_the_tool_size() {
    // A plain document reaches the too-large case: 800 pt is a size the commands write, only the
    // transform pushes the shown product past the maximum. The transformed glyph band then sits
    // entirely right of x = 189 and above y = 120, so the same click this file's other cases use
    // does not land on the layer's glyphs (which would edit the layer, #12075) but starts a
    // fresh mask.
    let (mut app, id) = txt_active(800.0);
    app.run("type.edit", json!({"layer": id.0, "transform": [4.0, 0.0, 0.0, 0.7, 60.0, 120.0]})).unwrap();
    mask(&mut app, (60.0, 300.0), "MASK");
    assert_eq!(mask_style(&app), ("Inter".into(), "Bold".into(), 48.0), "the tool's size, not the 1338 pt the create refuses");
    // And the corrupt-document case below the accepted minimum.
    let (mut app, id) = txt_active(80.0);
    corrupt_layer_size(&mut app, id, 0.0);
    mask(&mut app, (60.0, 300.0), "MASK");
    assert_eq!(mask_style(&app), ("Inter".into(), "Bold".into(), 48.0), "the tool's size, not the 0 pt the create refuses");
}

/// Not measured: whether the tool options themselves change afterwards. Today they do not, and
/// they still do not. Guard test: passes before and after the change.
#[test]
fn taking_the_active_layers_style_leaves_the_tool_options_as_they_are() {
    let (mut app, _) = txt_active(80.0);
    mask(&mut app, (60.0, 300.0), "MASK");
    commit(&mut app);
    let o = &app.ui.tool_options;
    assert_eq!((o.type_font.as_str(), o.type_style.as_str(), o.type_size), ("Tool Font", "Regular", 48.0), "the tool options are untouched");
}
