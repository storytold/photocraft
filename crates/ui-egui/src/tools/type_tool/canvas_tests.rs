//! Type tool through the real canvas (#123, #124): clicks and drags at known glyph positions
//! place the caret and selection on the glyph under the pointer at any zoom, HiDPI scale, pan,
//! view flip and layer transform; size edits apply at layer and selection scope, for native and
//! PSD-round-tripped text, as one history step per drag.

use crate::app::framework::BuildApp;
use craft::keymap::Mods;
use craft::text::ui::character::{CharacterControls, ParagraphControls};
use craft::text::ui::type_bar::TypeBar;
use craft::text::ui::type_options::{TextAntiAlias, TextOrientation};
use craft::ui::runtime::requests::ColorRequest;
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{LayerContent, LayerId, TextLayer};
use photocraft_engine::color_cmds::ColorExt;
use photocraft_geom::{Affine, Point};
use serde_json::json;

use super::{PathClick, layout, path_target, text_layer};
use crate::PhotocraftApp;
use crate::canvas::testing::{ToolEvent, tool_event, tool_event_response};

const ANTI_ALIAS_METHODS: [TextAntiAlias; 5] = [TextAntiAlias::None, TextAntiAlias::Sharp, TextAntiAlias::Crisp, TextAntiAlias::Strong, TextAntiAlias::Smooth];

/// A text edit session on layer `layer` with the given selection, as the editor holds it.
fn edit_of(layer: u64, caret: usize, anchor: usize, session: &str) -> crate::state::TextEdit {
    crate::state::TextEdit { layer, document: None, caret, anchor, session: session.into(), created: false, mask: false }
}

/// Starts the session on the active type layer.
fn begin_edit(app: &mut PhotocraftApp) -> Result<(), String> {
    let ctx = app.ctx().clone();
    super::edit_active(app, &ctx)
}

fn commit_edit(app: &mut PhotocraftApp) {
    super::commit(app);
}

fn cancel_edit(app: &mut PhotocraftApp) {
    super::cancel(app);
}

fn type_text(app: &mut PhotocraftApp, s: &str) {
    super::host::type_text(app, s);
}

/// The panel and options-bar edit path: defaults, or the target's selection or layers.
fn defaults_apply(app: &mut PhotocraftApp, props: serde_json::Value) {
    super::apply(app, props, None);
}

/// Screen position of document point (`x`, `y`) in the active canvas.
fn canvas_at(app: &PhotocraftApp, x: f64, y: f64) -> Pos2 {
    crate::canvas::testing::to_screen(app, [x, y])
}

fn canvas_rect(app: &PhotocraftApp) -> egui::Rect {
    crate::canvas::testing::view(app).expect("a canvas view").rect
}

/// Sets the main view's zoom and centre through the `ui.set` view command, then runs the passes
/// that show it.
fn set_view(h: &mut Harness<'static, Box<PhotocraftApp>>, zoom: f32, center: [f32; 2]) {
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"zoom": zoom, "center": center}));
    let app = h.state_mut();
    assert!(matches!(crate::control::handle(app, &ctx, &req), crate::control::Reply::Done(_)), "ui.set sets the view");
    h.run_steps(2);
}

/// Click (or drag out a box to `to`) with the current tool through the canvas pass.
fn drag_create(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
    if from != to {
        tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, Modifiers::NONE);
    }
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::NONE);
}

fn open_swatch_picker(app: &mut PhotocraftApp, request: Option<ColorRequest>) {
    if let Some(request) = request {
        let rgb = request.color.map_or([0.0; 3], |c| [c.r, c.g, c.b]);
        super::open_color_picker(app, rgb);
    }
}

fn dialog_set(app: &mut PhotocraftApp, dialog: craft::ui::dialog::DialogId, field: &str, value: serde_json::Value) {
    let ctx = app.ctx().clone();
    for (field, value) in crate::control::dialogs::map_request(app, dialog, field, value) {
        app.framework.dialogs.set_json_field(&ctx, dialog, &field, value).unwrap();
    }
}

fn dialog_confirm(app: &mut PhotocraftApp, dialog: craft::ui::dialog::DialogId) {
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, dialog).unwrap();
}

fn dialog_cancel(app: &mut PhotocraftApp, dialog: craft::ui::dialog::DialogId) {
    let ctx = app.ctx().clone();
    crate::dialogs::cancel(app, &ctx, dialog).unwrap();
}

/// The Character and Paragraph controls as the Character panel shows them: values of the type
/// target in, edit events applied through the type tool's value rules.
// PLAN: the production panel's entry belongs to panels-w42; this host drives the same controls and rules
fn show_panels(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let character = super::values::character_values(app);
    let paragraph = super::values::paragraph_values(app);
    let unit = super::values::doc_unit(app);
    let families = super::fonts::families();
    let family = character.family.clone().unwrap_or_else(|| app.ui.tool_options.type_font.clone());
    let styles = super::fonts::styles(&family);
    let mut out = CharacterControls::new("Character", &character, &families, &styles).unit(unit).show(ui);
    open_swatch_picker(app, out.requests.take_color());
    for edit in &out.edits {
        super::values::apply_character(app, edit);
    }
    let point = super::values::point_text(app);
    let out = ParagraphControls::new("Paragraph", &paragraph).unit(unit).point_text(point).show(ui);
    for edit in &out.edits {
        super::values::apply_paragraph(app, edit);
    }
}

/// The options bar's type strip. Orientation and anti-aliasing show fixed values: these tests
/// assert the edits the bar makes, not what it shows for them.
// PLAN: the "Toggle text orientation" button and the colour well's "Text color" label are assumed (TypeBar words)
fn show_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let character = super::values::bar_character_values(app);
    let paragraph = super::values::paragraph_values(app);
    let unit = super::values::doc_unit(app);
    let families = super::fonts::families();
    let styles = super::values::style_names(app, &character);
    let mut fonts = super::font_state(app);
    let mut bar = TypeBar::new(&character, &paragraph, &families, &styles)
        .unit(unit)
        .orientation(TextOrientation::Horizontal)
        .anti_alias(TextAntiAlias::Smooth)
        .anti_alias_methods(&ANTI_ALIAS_METHODS)
        .can_warp(true);
    bar = bar.font_picker(fonts.source());
    let mut out = bar.show(ui);
    super::finish_font_state(app, fonts);
    open_swatch_picker(app, out.requests.take_color());
    for edit in &out.edits {
        super::values::apply_bar(app, edit);
    }
}

/// Clicks the presets chevron of the number field named `menu` minus " presets" ("Font size presets"): the button
/// inside that field's spin button.
fn open_presets(h: &Harness<'static, Box<PhotocraftApp>>, menu: &str) {
    let field = menu.strip_suffix(" presets").unwrap_or(menu);
    h.get_by_role_and_label(egui::accesskit::Role::SpinButton, field).get_by_role_and_label(egui::accesskit::Role::Button, "Presets").click();
}

/// The accessible name of the field `label` of the Properties panel, or of the options bar's own name for it
/// ("Font size" is "Set font size" there).
fn field_name(h: &Harness<'static, Box<PhotocraftApp>>, role: egui::accesskit::Role, label: &str) -> String {
    // The Character panel names its style field "Style"; the options bar names it "Set font style".
    let lower = label.to_lowercase();
    [label.to_owned(), format!("Set {lower}"), format!("Set font {lower}")]
        .into_iter()
        .find(|name| h.query_by_role_and_label(role, name).is_some())
        .unwrap_or_else(|| format!("Set {lower}"))
}

/// Scrolls the open popup list until the row `label` is shown: lists draw only the rows in view, so a row near the end
/// of a long one is not in the access tree until the wheel has brought it into view.
pub(super) fn reveal(h: &mut Harness<'static, Box<PhotocraftApp>>, label: &str) {
    for _ in 0..40 {
        if h.query_by_label(label).is_some() {
            return;
        }
        let list = h.query_all_by_role(egui::accesskit::Role::ListBox).last().expect("an open list").rect();
        h.hover_at(list.center());
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -3.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run();
    }
    panic!("the row {label} never came into view");
}

fn panel_harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    // Wait for the font scan: while it runs the panels repaint every 100 ms and `Harness::run` gives up.
    let _ = photocraft_text::shared();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(760.0, 800.0)).with_max_steps(16).build_app(
        |ui, app: &mut PhotocraftApp| {
            ui.set_width(340.0);
            show_panels(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// The Properties panel without the app's frame logic, which ends a text edit whose layer is not the
/// active one: for a session on a layer other than the primary.
fn panel_harness_without_frame_logic(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let _ = photocraft_text::shared();
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(760.0, 800.0)).with_max_steps(16).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            ui.set_width(340.0);
            show_panels(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// Frames the options bar may take to settle. Opening the font list loads the rows' sample faces on
/// worker threads, and each pass that runs before a face has loaded is one more frame. The rows are
/// the fixed test catalog's (`photocraft_text::fonts::catalog`), so the count is bounded, but on a
/// busy runner the threads can miss many passes.
const BAR_SETTLE_STEPS: u64 = 64;

fn bar_harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let _ = photocraft_text::shared();
    // As tall as a window, so the bar's popup lists open in full below it.
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1100.0, 600.0)).with_max_steps(BAR_SETTLE_STEPS).build_app(
        |ui, app: &mut PhotocraftApp| {
            ui.horizontal(|ui| show_bar(app, ui));
        },
        app,
    );
    h.run_steps(4);
    h
}

mod panel_defaults {
    use super::*;
    use crate::state::{Tool, ToolOptions};
    use egui::accesskit::Role;
    use photocraft_doc::text::{Caps, Kerning, Orientation, TextAlign, TextShape};

    /// Without a type target the preset chevrons set the next layer's defaults, with no
    /// document edit.
    #[test]
    fn presets_without_a_type_layer_set_the_defaults() {
        let app = new_app();
        let steps = app.session.active().unwrap().history.entries().len();
        let mut h = panel_harness(app);
        open_presets(&h, "Font size presets");
        h.run();
        reveal(&mut h, "72 pt");
        h.run();
        h.get_by_label("72 pt").click();
        h.run();
        assert_eq!(h.state().ui.tool_options.type_size, 72.0);
        open_presets(&h, "Leading presets");
        h.run();
        h.get_by_label("Auto").click();
        h.run();
        assert_eq!(h.state().ui.tool_options.type_style_defaults.get("leading"), Some(&json!("auto")));
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps, "defaults are not a document edit");
    }

    fn number(h: &mut Harness<'static, Box<PhotocraftApp>>, label: &str, value: &str) {
        let name = field_name(h, Role::SpinButton, label);
        h.get_by_role_and_label(Role::SpinButton, &name).click();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text(value.into()));
        h.key_press(egui::Key::Enter);
        h.run();
    }

    #[test]
    fn panel_defaults_are_editable_and_reach_every_type_tool() {
        let mut h = panel_harness(new_app());
        let revision = h.state().session.active().unwrap().revision;
        let layers = h.state().session.active().unwrap().doc.walk().len();
        number(&mut h, "Font size", "36");
        number(&mut h, "Tracking", "80");
        number(&mut h, "Leading", "44");
        number(&mut h, "Indent left margin", "9");
        for label in ["Faux Bold", "Fractions", "Standard Ligatures", "Discretionary Ligatures", "Center text"] {
            h.get_by_label(label).click();
            h.run();
        }
        assert_eq!(h.state().session.active().unwrap().revision, revision, "pre-styling does not create document history");
        assert_eq!(h.state().session.active().unwrap().doc.walk().len(), layers);
        let options = h.state().ui.tool_options.clone();
        for tool in [Tool::Type, Tool::VerticalType, Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
            for paragraph in [false, true] {
                let mut app = new_app();
                app.ui.tool_options = options.clone();
                app.ui.tool = tool;
                app.session.tools.foreground = [0.2, 0.4, 0.6, 1.0];
                let revision = app.session.active().unwrap().revision;
                drag_create(&mut app, [100.0, 100.0], if paragraph { [500.0, 400.0] } else { [100.0, 100.0] });
                let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
                let t = text(&app, id);
                let c = &t.char_runs()[0].style;
                let p = &t.paragraph_runs()[0].style;
                assert_eq!(c.size_pt, 36.0);
                assert_eq!(c.tracking, 80.0);
                assert_eq!(c.leading_pt, Some(44.0));
                assert!(c.faux_bold && c.discretionary_ligatures && !c.ligatures);
                assert!(photocraft_engine::type_extra_cmds::feature_on(c, "frac"));
                assert_eq!(c.color.to_rgba8(), [51, 102, 153, 255]);
                assert_eq!(p.align, TextAlign::Center);
                assert_eq!(p.start_indent_pt, 9.0);
                assert_eq!(t.orientation, if tool.is_vertical_type() { Orientation::Vertical } else { Orientation::Horizontal });
                assert_eq!(matches!(t.shape, TextShape::Box { .. }), paragraph);
                assert_eq!(app.ui.text_edit.as_ref().unwrap().mask, tool.is_type_mask());
                assert!(app.session.active().unwrap().revision > revision);
                cancel_edit(&mut app);
                assert!(app.session.active().unwrap().doc.layer(id).is_none(), "creating and formatting is one undoable edit");
                assert_eq!(app.ui.tool_options, options, "cancelling text does not discard the chosen defaults");
            }
        }
    }

    #[test]
    fn extended_defaults_merge_serialize_and_reset_without_replacing_legacy_options() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = app.ctx().clone();
        defaults_apply(
            &mut app,
            json!({
                "fontStyle": "Bold", "size": 32, "kerning": 25, "caps": "smallCaps",
                "baselineShift": 3, "horizontalScale": 90, "verticalScale": 110,
                "underline": true, "strikethrough": true, "fauxItalic": true,
                "firstLineIndent": 5, "endIndent": 7, "spaceBefore": 11, "spaceAfter": 13,
                "features": {"swsh": 1}, "variations": {"wght": 650, "wdth": 90}
            }),
        );
        defaults_apply(&mut app, json!({"features": {"frac": 1}, "variations": {"slnt": -8}}));
        let (c, p) = super::super::default_styles(&app);
        assert_eq!((c.size_pt, c.kern, c.kerning, c.caps), (32.0, 25.0, Kerning::Off, Caps::SmallCaps));
        assert_eq!((c.baseline_shift_pt, c.horizontal_scale, c.vertical_scale), (3.0, 0.9, 1.1));
        assert!(c.underline && c.strikethrough && c.faux_italic);
        assert_eq!((p.first_line_indent_pt, p.end_indent_pt, p.space_before_pt, p.space_after_pt), (5.0, 7.0, 11.0, 13.0));
        assert_eq!(c.features.iter().map(|f| f.tag.as_str()).collect::<Vec<_>>(), ["frac", "swsh"]);
        assert_eq!(c.variations.iter().map(|v| (v.axis.as_str(), v.value)).collect::<Vec<_>>(), [("slnt", -8.0), ("wdth", 90.0), ("wght", 650.0)]);
        // A named style replaces wght; it must not come back when a later layer is created.
        defaults_apply(&mut app, json!({"fontStyle": "Regular"}));
        let (c, _) = super::super::default_styles(&app);
        assert!(!c.variations.iter().any(|v| v.axis == "wght"));
        assert_eq!(c.variations.len(), 2);
        let saved = serde_json::to_value(&app.ui.tool_options).unwrap();
        let restored: ToolOptions = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(restored, app.ui.tool_options);
        crate::tool_memory::persist(&mut app, &ctx);
        let remembered = app.session.prefs().tool_options.clone();
        let mut restarted = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        restarted.session.prefs.edit(|p| p.tool_options = remembered);
        crate::tool_memory::restore(&mut restarted);
        assert_eq!(restarted.ui.tool_options, restored, "the real preference restore path retains panel defaults");
        let mut legacy = saved;
        legacy.as_object_mut().unwrap().remove("type_style_defaults");
        let legacy: ToolOptions = serde_json::from_value(legacy).unwrap();
        assert_eq!((legacy.type_font.as_str(), legacy.type_style.as_str(), legacy.type_size), ("Inter", "Regular", 32.0));
        assert!(legacy.type_style_defaults.is_empty());
        for tool in [Tool::Type, Tool::VerticalType, Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
            let mut reset = restored.clone();
            crate::tool_reset::reset_options(&mut reset, tool);
            assert!(reset.type_style_defaults.is_empty());
            assert_eq!(reset.type_size, ToolOptions::default().type_size);
        }
    }

    #[test]
    fn selected_text_edits_leave_panel_defaults_and_other_characters_untouched() {
        let mut app = new_app();
        defaults_apply(&mut app, json!({"tracking": 80, "fauxBold": true}));
        let defaults = app.ui.tool_options.type_style_defaults.clone();
        let id = LayerId(app.run("type.create", json!({"text": "AéBC", "size": 24})).unwrap()["layer"].as_u64().unwrap());
        app.ui.text_edit = Some(edit_of(id.0, 2, 1, "panel-default-selection"));
        defaults_apply(&mut app, json!({"tracking": 15}));
        super::super::toggle_opentype(&mut app, "type.openType.fractions", false);
        let runs = text(&app, id).char_runs();
        assert_eq!(runs.iter().map(|r| (r.len, r.style.tracking)).collect::<Vec<_>>(), [(1, 0.0), (2, 15.0), (2, 0.0)]);
        assert!(photocraft_engine::type_extra_cmds::feature_on(&runs[1].style, "frac"));
        assert!(!photocraft_engine::type_extra_cmds::feature_on(&runs[0].style, "frac"));
        assert_eq!(app.ui.tool_options.type_style_defaults, defaults);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(text(&app, id).char_runs().len(), 1, "selection edits keep their typing-session coalescing");
        assert_eq!(text(&app, id).char_runs()[0].style.tracking, 0.0);
    }

    /// The swatch's colour request opens the picker with the foreground; Cancel changes nothing.
    #[test]
    fn default_text_color_uses_foreground_and_cancel_preserves_it() {
        let mut app = new_app();
        let before = app.session.tools.foreground;
        let revision = app.session.active().unwrap().revision;
        // PLAN: assumed `open_color_picker` returns the hosted dialog's `DialogId`
        let dialog = super::super::open_color_picker(&mut app, [0.0; 3]);
        assert!(app.dialog_open(), "the picker is open");
        dialog_set(&mut app, dialog, "color", json!("#ff8800"));
        dialog_cancel(&mut app, dialog);
        assert_eq!(app.session.tools.foreground, before);
        let dialog = super::super::open_color_picker(&mut app, [0.0; 3]);
        dialog_set(&mut app, dialog, "color", json!("#336699"));
        dialog_confirm(&mut app, dialog);
        assert_eq!(super::super::default_styles(&app).0.color.to_rgba8(), [51, 102, 153, 255]);
        assert_eq!(app.session.active().unwrap().revision, revision);
        drag_create(&mut app, [100.0, 100.0], [100.0, 100.0]);
        let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        assert_eq!(text(&app, id).char_runs()[0].style.color.to_rgba8(), [51, 102, 153, 255]);
    }

    #[test]
    fn unknown_saved_defaults_cannot_become_creation_parameters() {
        let mut app = new_app();
        app.ui.tool_options.type_style_defaults =
            json!({"tracking": 20, "text": "Injected", "box": [0, 0, 2, 2], "coalesce": "unrelated"}).as_object().unwrap().clone();
        let p = super::super::default_style_params(&app);
        assert_eq!(p["tracking"], 20);
        assert!(p.get("text").is_none() && p.get("box").is_none() && p.get("coalesce").is_none());
        app.ui.tool_options.type_style_defaults = json!({
            "tracking": 1e300, "leading": 1e300, "spaceAfter": 1e300,
            "variations": {"wght": 1e300}, "features": {"frac": {"bad": 1}}, "fauxBold": true
        })
        .as_object()
        .unwrap()
        .clone();
        let p = super::super::default_style_params(&app);
        for key in ["tracking", "leading", "spaceAfter", "variations", "features"] {
            assert!(p.get(key).is_none(), "hostile {key} is ignored");
        }
        let (c, para) = super::super::default_styles(&app);
        assert!(c.faux_bold && c.leading_pt.is_none() && c.tracking == 0.0 && para.space_after_pt == 0.0);
    }
}

fn harness(ppp: f32, app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    make_harness_with(Harness::builder(), ppp, app)
}

/// 60 frames a second: every queued event is a frame, and the default quarter-second step puts
/// the second click of a double-click past egui's double-click delay.
fn fast_harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    make_harness_with(Harness::builder().with_step_dt(1.0 / 60.0), 1.0, app)
}

fn make_harness_with(builder: egui_kittest::HarnessBuilder<Box<PhotocraftApp>>, ppp: f32, app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    // The tests type CJK text; the UI stack loads a system face only after a pass that missed the glyph,
    // so the first pass draws tofu on any machine and the layout under test does not depend on a face.
    let builder = builder.allow_missing_glyphs().with_context(app.ctx().clone()).with_size(vec2(1200.0, 800.0)).with_pixels_per_point(ppp);
    let mut h = builder.build_app(
        |ui, app: &mut PhotocraftApp| {
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    h.run_steps(4);
    h
}

fn new_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1600, "height": 900})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = crate::state::Tool::Type;
    app
}

/// Screen position of a text-space point of layer `id`.
fn screen(h: &mut Harness<'static, Box<PhotocraftApp>>, id: LayerId, x: f32, y: f32) -> Pos2 {
    let app = h.state_mut();
    let (_, aff, _) = layout(app, id).unwrap();
    let p = aff.apply(Point::new(f64::from(x), f64::from(y)));
    canvas_at(app, p.x, p.y)
}

/// A point inside glyph `i` (character index): `f` of the way along its advance, in the upper
/// half of its line.
fn glyph(h: &mut Harness<'static, Box<PhotocraftApp>>, id: LayerId, i: usize, f: f32) -> Pos2 {
    let (l, _, text) = layout(h.state_mut(), id).unwrap();
    let b = text.char_indices().nth(i).unwrap().0;
    let c = l.clusters.iter().find(|c| c.range.start == b).unwrap().clone();
    let ln = &l.lines[c.line];
    screen(h, id, c.x + c.advance * f, ln.baseline - ln.ascent * 0.35)
}

fn press(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2, down: bool) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn click(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2) {
    h.hover_at(p);
    h.run_steps(1);
    press(h, p, true);
    press(h, p, false);
    h.run_steps(1);
}

/// Two clicks within egui's double-click delay (use [`fast_harness`]).
fn double_click(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2) {
    h.hover_at(p);
    h.run_steps(1);
    // One pass per press and release, as the window delivers them: the runtime takes one press per pass.
    for pressed in [true, false, true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(1);
}

fn drag(h: &mut Harness<'static, Box<PhotocraftApp>>, a: Pos2, b: Pos2) {
    drag_in(h, a, b, 6);
}

/// [`drag`] in `moves` pointer moves, one frame each.
fn drag_in(h: &mut Harness<'static, Box<PhotocraftApp>>, a: Pos2, b: Pos2, moves: u8) {
    h.hover_at(a);
    h.run_steps(1);
    press(h, a, true);
    for k in 1..=moves {
        h.hover_at(a + (b - a) * (f32::from(k) / f32::from(moves)));
        h.run_steps(1);
    }
    press(h, b, false);
    h.run_steps(1);
}

fn selection(h: &Harness<'static, Box<PhotocraftApp>>) -> (usize, usize) {
    let Some(e) = h.state().ui.text_edit.clone() else { return (usize::MAX, usize::MAX) };
    (e.anchor, e.caret)
}

/// Every combination the issue lists that this canvas has: zoom 25–200 %, HiDPI 1×/2×, a panned
/// view, View › Flip Horizontal, and point / paragraph text with identity or rotated+scaled layer
/// transforms. Clicks land before/after the glyph under the pointer; drags select from the glyph
/// pressed (not where egui recognised the drag) to the glyph released over. The view maps document
/// points to the screen the pointer presses.
#[test]
fn clicks_and_drags_land_on_the_glyph_under_the_pointer() {
    let rotated = Affine { m: [1.4 * 0.94, 1.4 * 0.342, -1.4 * 0.342, 1.4 * 0.94, 500.0, 420.0] };
    // One case per zoom, pairwise over the other axes: every ppp, flip and pan, and shape and transform
    // appears, ppp meets both flips, and the two ends of the range in points (25 % at 2x is 12.5 %, 200 %
    // at 1x is 200 %) are kept. The view maps the pointer to the document (the shown view's `to_doc`)
    // the same way for any layer, and the layer's transform maps the document to text space the same way
    // at any view, so the two halves need not be crossed.
    let pan = [37.0, -21.0];
    for (ppp, zoom, flip, pan, shape, tf) in [
        (2.0, 0.25f32, false, [0.0, 0.0], "point", None),
        (1.0, 0.5, true, pan, "box", None),
        (2.0, 1.0, true, pan, "point", Some(rotated)),
        (1.0, 2.0, false, [0.0, 0.0], "box", Some(rotated)),
    ] {
        let mut app = new_app();
        let size = 60.0 / zoom;
        let mut p = json!({"text": "HOHOHO HOHO", "size": size, "x": 300, "y": 420});
        if shape == "box" {
            p["box"] = json!([250.0, 250.0, size * 6.0, size * 4.0]);
        }
        let id = LayerId(app.run("type.create", p).unwrap()["layer"].as_u64().unwrap());
        if let Some(a) = tf {
            app.run("type.edit", json!({"layer": id.0, "transform": a.m})).unwrap();
        }
        app.ui.view.flip_horizontal = flip;
        let mut h = harness(ppp, app);
        // Centre the view on the text's middle, then pan.
        let (l, aff, _) = layout(h.state_mut(), id).unwrap();
        let b = l.bounds().unwrap();
        let m = aff.apply(Point::new(f64::from(b[0] + b[2]) / 2.0, f64::from(b[1] + b[3]) / 2.0));
        set_view(&mut h, zoom, [m.x as f32 + pan[0], m.y as f32 + pan[1]]);
        h.run_steps(2);
        let ctx = format!("ppp {ppp} zoom {zoom} flip {flip} {shape} transformed {}", tf.is_some());
        let rect = canvas_rect(h.state());
        for (i, f) in [(0, 0.2), (2, 0.8), (4, 0.3), (8, 0.7)] {
            let p = glyph(&mut h, id, i, f);
            assert!(rect.contains(p), "{ctx}: glyph {i} off screen at {p:?}");
        }
        // Click into the layer: caret before glyph 2 (left part), then after glyph 4.
        let p = glyph(&mut h, id, 2, 0.2);
        click(&mut h, p);
        assert_eq!(selection(&h), (2, 2), "{ctx}: click on the left of glyph 2");
        let p = glyph(&mut h, id, 4, 0.8);
        click(&mut h, p);
        assert_eq!(selection(&h), (5, 5), "{ctx}: click on the right of glyph 4");
        // Drag from glyph 1 (left part) to glyph 8 (right part): characters 1..9.
        let (a, b) = (glyph(&mut h, id, 1, 0.25), glyph(&mut h, id, 8, 0.75));
        drag(&mut h, a, b);
        assert_eq!(selection(&h), (1, 9), "{ctx}: drag selection");
        // And backwards, starting inside the egui drag threshold of a glyph edge.
        let (a, b) = (glyph(&mut h, id, 5, 0.85), glyph(&mut h, id, 3, 0.15));
        drag(&mut h, a, b);
        assert_eq!(selection(&h), (6, 3), "{ctx}: backwards drag selection");
    }
}

fn text(app: &PhotocraftApp, id: LayerId) -> TextLayer {
    text_layer(&app.session.active().unwrap().doc, id).unwrap().clone()
}

/// A PSD type layer keeps Photoshop's pixels until it is edited. Clicking into it re-renders it
/// with our engine first (so the caret sits on what is shown), inside the edit session's single
/// history step; Cancel brings Photoshop's pixels back.
#[test]
fn editing_psd_type_shows_our_layout_and_cancel_restores_it() {
    let mut app = new_app();
    let id = LayerId(app.run("type.create", json!({"text": "HOHOHO", "size": 120, "x": 300, "y": 420})).unwrap()["layer"].as_u64().unwrap());
    // Stand-in for Photoshop's rendering: the same text drawn somewhere else.
    let moved = {
        let mut t = text(&app, id);
        t.transform = Affine::translate(330.0, 380.0);
        photocraft_text::shared().lock().unwrap().render(&t, 72.0, photocraft_color::PixelFormat::RGBA8).1.surface
    };
    let st = app.session.active_mut().unwrap();
    let mut doc = (*st.doc).clone();
    if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
        t.cache = Some(moved.clone());
    }
    *st.doc_mut() = std::sync::Arc::new(doc);
    st.revision += 1;
    let steps = app.session.active().unwrap().history.entries().len();
    let mut h = harness(1.0, app);
    let p = glyph(&mut h, id, 2, 0.2);
    click(&mut h, p);
    assert_eq!(selection(&h), (2, 2));
    let ours = text(h.state(), id).cache.unwrap().content_bounds();
    assert_ne!(ours, moved.content_bounds(), "re-rendered where the caret is");
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps + 1);
    type_text(h.state_mut(), "x");
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps + 1, "one step for the session");
    cancel_edit(h.state_mut());
    assert_eq!(text(h.state(), id).cache.unwrap().content_bounds(), moved.content_bounds());
    assert_eq!(text(h.state(), id).text, "HOHOHO");
    // A native layer shows our layout already: clicking into it records nothing.
    let mut app = new_app();
    let id = LayerId(app.run("type.create", json!({"text": "HOHOHO", "size": 120, "x": 300, "y": 420})).unwrap()["layer"].as_u64().unwrap());
    let steps = app.session.active().unwrap().history.entries().len();
    let mut h = harness(1.0, app);
    let p = glyph(&mut h, id, 3, 0.2);
    click(&mut h, p);
    assert_eq!(selection(&h), (3, 3));
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps);
}

fn size_at(app: &PhotocraftApp, id: LayerId, ci: usize) -> f32 {
    let t = text(app, id);
    let b = t.text.char_indices().nth(ci).map_or(t.text.len(), |(b, _)| b);
    let mut at = 0;
    for r in t.char_runs() {
        if b < at + r.len {
            return r.style.size_pt;
        }
        at += r.len;
    }
    0.0
}

/// Font size from the options bar / Properties: the whole layer when no characters are selected,
/// only the selection when editing with one, as the layer appears (transform scale included), and
/// for type that went through a PSD file.
#[test]
fn size_applies_at_layer_and_selection_scope() {
    for psd in [false, true] {
        let mut app = new_app();
        let id = LayerId(app.run("type.create", json!({"text": "Hello world", "size": 20, "x": 300, "y": 420})).unwrap()["layer"].as_u64().unwrap());
        app.run("type.edit", json!({"layer": id.0, "transform": [2.0, 0.0, 0.0, 2.0, 300.0, 420.0]})).unwrap();
        let id = if psd {
            let doc = (*app.session.active().unwrap().doc).clone();
            let out = photocraft_io::export(&doc, "t.psd", &Default::default()).unwrap();
            let back = photocraft_io::import("t.psd", &out.bytes).unwrap().document;
            app.session.add_document(back, None);
            app.sync_views();
            let d = &app.session.active().unwrap().doc;
            let id = d.walk().into_iter().find(|(_, _, l)| matches!(l.content, LayerContent::Text(_))).map(|(_, _, l)| l.id).unwrap();
            let _ = app.session.select_layer(id);
            id
        } else {
            id
        };
        assert!((super::shown_scale(&app) - 2.0).abs() < 1e-4, "psd {psd}");
        let w0 = text(&app, id).cache.unwrap().content_bounds().width();
        // Layer scope: 60 pt as shown = 30 pt in the layer's own space.
        let k = super::shown_scale(&app);
        defaults_apply(&mut app, json!({"size": 60.0 / k}));
        assert_eq!((size_at(&app, id, 0), size_at(&app, id, 10)), (30.0, 30.0), "psd {psd}");
        let w1 = text(&app, id).cache.unwrap().content_bounds().width();
        assert!(w1 as f32 > w0 as f32 * 1.4, "psd {psd}: the pixels grow ({w0} → {w1})");
        // Selection scope: "world" only.
        app.ui.text_edit = Some(edit_of(id.0, 11, 6, "s"));
        defaults_apply(&mut app, json!({"size": 10.0}));
        assert_eq!((size_at(&app, id, 0), size_at(&app, id, 5), size_at(&app, id, 6), size_at(&app, id, 10)), (30.0, 30.0, 10.0, 10.0), "psd {psd}");
    }
}

fn history_len(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().history.entries().len()
}

/// Scrubbing the options bar's size field previews live (the size changes every frame) and is
/// one undo step per drag (one coalesce key per gesture).
#[test]
fn size_drag_is_live_and_one_history_step_per_drag() {
    let mut app = new_app();
    let id = LayerId(app.run("type.create", json!({"text": "Hello", "size": 20, "x": 300, "y": 420})).unwrap()["layer"].as_u64().unwrap());
    let mut h = bar_harness(app);
    let steps = history_len(&h);
    let mut last = 20.0;
    for d in 0..2 {
        let name = field_name(&h, egui::accesskit::Role::SpinButton, "Font size");
        let field = h.get_by_role_and_label(egui::accesskit::Role::SpinButton, &name).rect().center();
        h.hover_at(field);
        h.run_steps(1);
        press(&mut h, field, true);
        for k in 1..=5 {
            h.hover_at(field + vec2(8.0 * k as f32, 0.0));
            h.run_steps(1);
            let now = size_at(h.state(), id, 0);
            // The pass that moves the pointer past the drag slop starts the gesture (its `DragStart` reads the value
            // the scrub starts from); the value follows from the next pass on.
            if k == 1 {
                assert!(now >= last, "drag {d} step {k}: the gesture start keeps the size {last} → {now}");
            } else {
                assert!(now > last, "drag {d} step {k}: live size {last} → {now}");
            }
            last = now;
        }
        press(&mut h, field + vec2(40.0, 0.0), false);
        h.run_steps(2);
        assert_eq!(history_len(&h), steps + d + 1, "one step per drag");
    }
    assert!(h.state_mut().session.undo());
    assert!(h.state_mut().session.undo());
    assert_eq!(size_at(h.state(), id, 0), 20.0);
}

/// A point inside glyph `i` of vertical type: `f` of the way down its advance, right of the
/// column's centre line.
fn vglyph(h: &mut Harness<'static, Box<PhotocraftApp>>, id: LayerId, i: usize, f: f32) -> Pos2 {
    let (l, _, text) = layout(h.state_mut(), id).unwrap();
    assert!(l.vertical);
    let b = text.char_indices().nth(i).unwrap().0;
    let c = l.clusters.iter().find(|c| c.range.start == b).unwrap().clone();
    let ln = &l.lines[c.line];
    let (x, y) = l.to_text(c.x + c.advance * f, ln.baseline - ln.ascent * 0.35);
    screen(h, id, x, y)
}

fn key(h: &mut Harness<'static, Box<PhotocraftApp>>, k: egui::Key) {
    h.event(egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// Vertical type (#199): clicks and drags land on the glyph under the pointer down the column,
/// and the arrow keys follow the vertical flow (↓ next character, ← next column).
#[test]
fn vertical_type_caret_selection_and_arrows_follow_the_columns() {
    let rotated = Affine { m: [0.94, 0.342, -0.342, 0.94, 900.0, 200.0] };
    for ppp in [1.0, 2.0] {
        for tf in [None, Some(rotated)] {
            let mut app = new_app();
            let id = LayerId(app.run("type.create", json!({"text": "HOHOHO\nHOHO", "size": 60, "x": 800, "y": 120})).unwrap()["layer"].as_u64().unwrap());
            app.run("type.orientation.vertical", json!({"layer": id.0})).unwrap();
            // Snapping would pull the pointer onto the layer's edges and centre lines.
            app.ui.extras.snap = false;
            if let Some(a) = tf {
                app.run("type.edit", json!({"layer": id.0, "transform": a.m})).unwrap();
            }
            let mut h = harness(ppp, app);
            let (l, aff, _) = layout(h.state_mut(), id).unwrap();
            let b = l.bounds().unwrap();
            assert!(b[3] - b[1] > b[2] - b[0], "vertical bounds {b:?}");
            let m = aff.apply(Point::new(f64::from(b[0] + b[2]) / 2.0, f64::from(b[1] + b[3]) / 2.0));
            set_view(&mut h, 0.5, [m.x as f32, m.y as f32]);
            h.run_steps(2);
            let ctx = format!("ppp {ppp} transformed {}", tf.is_some());
            // Upper part of glyph 2 → caret before it; lower part of glyph 4 → after it.
            let p = vglyph(&mut h, id, 2, 0.2);
            click(&mut h, p);
            assert_eq!(selection(&h), (2, 2), "{ctx}");
            let p = vglyph(&mut h, id, 4, 0.8);
            click(&mut h, p);
            assert_eq!(selection(&h), (5, 5), "{ctx}");
            // Drag down the column from glyph 1 into the second column's glyph 2 (char 9).
            let (a, b) = (vglyph(&mut h, id, 1, 0.25), vglyph(&mut h, id, 9, 0.75));
            drag(&mut h, a, b);
            assert_eq!(selection(&h), (1, 10), "{ctx}: drag");
            // Arrows: ↓ next char, ↑ previous, ← next column (same position), → back.
            let p = vglyph(&mut h, id, 1, 0.2);
            click(&mut h, p);
            assert_eq!(selection(&h), (1, 1), "{ctx}");
            key(&mut h, egui::Key::ArrowDown);
            assert_eq!(selection(&h).1, 2, "{ctx}: ↓");
            key(&mut h, egui::Key::ArrowUp);
            assert_eq!(selection(&h).1, 1, "{ctx}: ↑");
            key(&mut h, egui::Key::ArrowLeft);
            assert_eq!(selection(&h).1, 8, "{ctx}: ← moves to the next column");
            key(&mut h, egui::Key::ArrowRight);
            assert_eq!(selection(&h).1, 1, "{ctx}: → moves back");
        }
    }
}

fn rgb_at(app: &PhotocraftApp, id: LayerId, ci: usize) -> [u8; 4] {
    let t = text(app, id);
    let b = t.text.char_indices().nth(ci).map_or(t.text.len(), |(b, _)| b);
    let mut at = 0;
    for r in t.char_runs() {
        if b < at + r.len {
            return r.style.color.to_rgba8();
        }
        at += r.len;
    }
    [0; 4]
}

/// A new foreground colour recolours the selected characters only, inside the editing session's
/// history step; with nothing selected (a caret, or not editing) the type keeps its colour.
#[test]
fn foreground_colour_recolours_only_selected_type() {
    let mut app = new_app();
    let id =
        LayerId(app.run("type.create", json!({"text": "Hello world", "size": 40, "x": 300, "y": 420, "color": "#000000"})).unwrap()["layer"].as_u64().unwrap());
    app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    super::foreground_changed(&mut app);
    assert_eq!(rgb_at(&app, id, 0), [0, 0, 0, 255], "not editing: unchanged");
    app.ui.text_edit = Some(edit_of(id.0, 3, 3, "s"));
    super::foreground_changed(&mut app);
    assert_eq!(rgb_at(&app, id, 0), [0, 0, 0, 255], "a caret: unchanged");
    let steps = app.session.active().unwrap().history.entries().len();
    app.ui.text_edit = Some(edit_of(id.0, 11, 6, "s"));
    super::foreground_changed(&mut app);
    app.run("tools.setColors", json!({"foreground": "#00ff00"})).unwrap();
    super::foreground_changed(&mut app);
    assert_eq!((rgb_at(&app, id, 0), rgb_at(&app, id, 5)), ([0, 0, 0, 255], [0, 0, 0, 255]));
    assert_eq!((rgb_at(&app, id, 6), rgb_at(&app, id, 10)), ([0, 255, 0, 255], [0, 255, 0, 255]));
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 1, "one step for the session");
}

#[test]
fn vertical_type_tool_creates_point_and_paragraph_text_with_one_undo() {
    use photocraft_doc::text::{Orientation, TextShape};
    for end in [[100.0, 100.0], [240.0, 230.0]] {
        let mut app = new_app();
        app.ui.tool = crate::state::Tool::VerticalType;
        let before = app.session.active().unwrap().history.entries().len();
        drag_create(&mut app, [100.0, 100.0], end);
        let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        let st = app.session.active().unwrap();
        let text = text_layer(&st.doc, id).unwrap();
        assert_eq!(text.orientation, Orientation::Vertical);
        assert_eq!(matches!(text.shape, TextShape::Box { .. }), end != [100.0, 100.0]);
        assert_eq!(st.history.entries().len(), before + 1);
        commit_edit(&mut app);
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.layer(id).is_none());
    }
}

/// #668: ⌘/Ctrl+T while typing shows or hides the Character panel (the editor's
/// `TextEditorOutput::character_panel`), as in Photoshop, instead of starting Free Transform on
/// the layer being typed into.
#[test]
fn command_t_while_typing_toggles_the_character_panel() {
    let mut app = new_app();
    let id = LayerId(app.run("type.create", json!({"text": "HOHO", "size": 60, "x": 300, "y": 420})).unwrap()["layer"].as_u64().unwrap());
    let mut h = harness(1.0, app);
    let p = glyph(&mut h, id, 1, 0.5);
    click(&mut h, p);
    assert!(h.state().ui.text_edit.is_some(), "editing");
    let before = crate::view_cmds::checked(h.state(), "window.panel.character");
    h.event(egui::Event::Key { key: egui::Key::T, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    h.run_steps(2);
    assert!(h.state().ui.transform.is_none(), "no Free Transform while typing");
    assert!(h.state().ui.text_edit.is_some(), "still editing");
    assert_ne!(crate::view_cmds::checked(h.state(), "window.panel.character"), before, "the Character panel toggled");
}

#[test]
fn text_color_dialog_edits_only_the_selected_range_and_cancel_is_inert() {
    let mut app = new_app();
    let id = LayerId(app.run("type.create", json!({"text":"Hello world","size":40,"color":"#000000"})).unwrap()["layer"].as_u64().unwrap());
    let peer = LayerId(app.run("type.create", json!({"text":"Selected peer","color":"#000000"})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
    app.ui.text_edit = Some(edit_of(id.0, 11, 6, "text-color-dialog-test"));
    let foreground = app.session.tools.foreground;
    let before = app.session.active().unwrap().history.entries().len();
    let selection = app.ui.text_edit.clone();
    let dialog = super::open_color_picker(&mut app, [0.0; 3]);
    dialog_set(&mut app, dialog, "color", json!("#ff0000"));
    dialog_cancel(&mut app, dialog);
    assert_eq!(rgb_at(&app, id, 6), [0, 0, 0, 255]);
    assert_eq!(app.session.active().unwrap().history.entries().len(), before);
    assert_eq!(app.ui.text_edit, selection);
    let dialog = super::open_color_picker(&mut app, [0.0; 3]);
    dialog_set(&mut app, dialog, "color", json!("#00ff00"));
    dialog_confirm(&mut app, dialog);
    assert_eq!(rgb_at(&app, id, 0), [0, 0, 0, 255]);
    assert_eq!(rgb_at(&app, id, 5), [0, 0, 0, 255]);
    assert_eq!(rgb_at(&app, id, 6), [0, 255, 0, 255]);
    assert_eq!(rgb_at(&app, id, 10), [0, 255, 0, 255]);
    assert_eq!(app.session.tools.foreground, foreground, "text-only picker leaves the foreground alone");
    assert_eq!(app.ui.text_edit, selection, "dialog preserves the caret and selection");
    assert_eq!(app.session.active().unwrap().history.entries().len(), before + 1);
    let dialog = super::open_color_picker(&mut app, [0.0, 1.0, 0.0]);
    dialog_set(&mut app, dialog, "color", json!("#0000ff"));
    dialog_confirm(&mut app, dialog);
    assert_eq!(app.session.active().unwrap().history.entries().len(), before + 1, "one editing-session undo step");
    assert_eq!(rgb_at(&app, peer, 0), [0, 0, 0, 255], "a selected peer is outside the active text edit");
}

fn type_command() -> Modifiers {
    Modifiers { command: true, ctrl: true, ..Modifiers::NONE }
}

/// The modifiers `type_command` holds, as the app's key rules read them.
fn command_mods() -> Mods {
    Mods { cmd: true, ..Mods::default() }
}

/// The modifiers held in the harness's last pass: the real keys plus the shell's latch (tp.T15).
fn held(h: &Harness<'static, Box<PhotocraftApp>>) -> Mods {
    craft::ui::runtime::pointer::frame(&h.ctx).mods
}

fn hold(h: &mut Harness<'static, Box<PhotocraftApp>>, mods: Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    h.run_steps(1);
}

fn transform_press(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2, down: bool, mods: Modifiers) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: mods });
    h.run_steps(1);
}

fn transform_drag(h: &mut Harness<'static, Box<PhotocraftApp>>, a: Pos2, b: Pos2, mods: Modifiers, release_command: bool) {
    hold(h, mods);
    h.hover_at(a);
    h.run_steps(1);
    transform_press(h, a, true, mods);
    for k in 1..=6 {
        if release_command && k == 4 {
            hold(h, Modifiers { command: false, ctrl: false, mac_cmd: false, ..mods });
        }
        h.hover_at(a + (b - a) * (k as f32 / 6.0));
        h.run_steps(1);
    }
    transform_press(h, b, false, if release_command { Modifiers::NONE } else { mods });
    hold(h, Modifiers::NONE);
}

fn transform_app(paragraph: bool, vertical: bool, transform: Affine) -> (Box<PhotocraftApp>, LayerId) {
    let mut app = new_app();
    // The tests assert where a drag lands, which a snap to a guide or an edge would move.
    app.ui.extras.snap = false;
    let mut p = json!({"text": "Hello é世界\nSecond line", "size": 32, "x": 300, "y": 420});
    if paragraph {
        p["box"] = json!([300, 300, 240, 180]);
    }
    if vertical {
        p["orientation"] = json!("vertical");
    }
    let id = LayerId(app.run("type.create", p).unwrap()["layer"].as_u64().unwrap());
    app.run("type.edit", json!({"layer": id.0, "transform": transform.m})).unwrap();
    begin_edit(&mut app).unwrap();
    let ed = app.ui.text_edit.as_mut().unwrap();
    ed.caret = 3;
    ed.anchor = 3;
    (app, id)
}

/// The type transform frame in the coordinates the tests work in: the source rectangle as
/// `[x0, y0, x1, y1]`, the box corners clockwise from the top left, and the reference point.
struct Frame {
    rect: [f64; 4],
    quad: [[f64; 2]; 4],
    pivot: [f64; 2],
}

fn frame_of(app: &mut PhotocraftApp) -> Option<Frame> {
    let f = crate::type_transform::frame(app)?;
    Some(Frame { rect: [f.rect.x0, f.rect.y0, f.rect.x1, f.rect.y1], quad: f.state.quad.map(|p| [p.x, p.y]), pivot: [f.state.pivot.x, f.state.pivot.y] })
}

fn frame_point(app: &PhotocraftApp, p: [f64; 2]) -> Pos2 {
    canvas_at(app, p[0], p[1])
}

fn assert_affine(a: Affine, b: Affine) {
    for (x, y) in a.m.into_iter().zip(b.m) {
        // Translations around 500 doc px at a scaled ppp cancel to ~2.7e-4 in f32 rounding.
        assert!((x - y).abs() < 1e-3, "{a:?} != {b:?}");
    }
}

/// #5956: while typing, a plain drag from just outside the text moves it (the overlay's move
/// reach), and typing carries on; a press further out commits the text.
#[test]
fn a_drag_just_outside_the_typed_text_moves_it() {
    let (app, id) = transform_app(false, false, Affine::translate(500.0, 350.0));
    let mut h = harness(1.0, app);
    let f = frame_of(h.state_mut()).unwrap();
    let zoom = f64::from(h.state().point_zoom());
    let mid = (f.quad[1][1] + f.quad[2][1]) / 2.0;
    let near = frame_point(h.state(), [f.quad[1][0] + 20.0 / zoom, mid]);
    let before = text(h.state(), id);
    let count = history_len(&h);
    transform_drag(&mut h, near, near + egui::vec2(60.0, 30.0), Modifiers::NONE, false);
    let after = text(h.state(), id);
    assert_affine(after.transform, Affine::translate(60.0 / zoom, 30.0 / zoom).mul(&before.transform));
    assert_eq!(after.text, before.text);
    assert_eq!(h.state().ui.text_edit.as_ref().map(|e| e.layer), Some(id.0), "still typing");
    assert_eq!(history_len(&h), count + 1);
    // Further out: the press commits the text.
    let f = frame_of(h.state_mut()).unwrap();
    let far = frame_point(h.state(), [f.quad[1][0] + 100.0 / zoom, (f.quad[1][1] + f.quad[2][1]) / 2.0]);
    click(&mut h, far);
    assert!(h.state().ui.text_edit.is_none(), "committed");
    assert_affine(text(h.state(), id).transform, after.transform);
}

/// #5956 review: the reach is screen points as the view draws them, so a pixel-aspect stretch
/// doesn't widen it.
#[test]
fn the_move_reach_is_on_screen_and_leaves_other_type_alone() {
    let (mut app, id) = transform_app(false, false, Affine::translate(500.0, 350.0));
    app.ui.view.pixel_aspect_correction = true;
    app.ui.view.pixel_aspect = "anamorphic2To1".into();
    let mut h = harness(1.0, app);
    h.run_steps(1);
    let f = frame_of(h.state_mut()).unwrap();
    let mid = (f.quad[1][1] + f.quad[2][1]) / 2.0;
    let edge = frame_point(h.state(), [f.quad[1][0], mid]);
    // 30 points out is in reach.
    let before = text(h.state(), id);
    transform_drag(&mut h, edge + egui::vec2(30.0, 0.0), edge + egui::vec2(90.0, 30.0), Modifiers::NONE, false);
    assert_ne!(text(h.state(), id).transform, before.transform, "a press in reach moves the text");
    // 50 points out is 25 document points across a 2:1 pixel: out of reach.
    let f = frame_of(h.state_mut());
    if let Some(f) = f {
        let mid = (f.quad[1][1] + f.quad[2][1]) / 2.0;
        let edge = frame_point(h.state(), [f.quad[1][0], mid]);
        let before = text(h.state(), id);
        transform_drag(&mut h, edge + egui::vec2(50.0, 0.0), edge + egui::vec2(110.0, 30.0), Modifiers::NONE, false);
        assert_affine(text(h.state(), id).transform, before.transform);
    }
}

/// #5956 review: a press on another type layer within reach of the text being typed is the Type
/// tool's (it commits and moves to that text), not a move of the edited text.
#[test]
fn a_press_on_other_type_near_the_text_does_not_move_it() {
    let mut app = new_app();
    let id = LayerId(app.run("type.create", json!({"text": "Hi", "size": 32, "x": 300, "y": 420})).unwrap()["layer"].as_u64().unwrap());
    let right = app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds().x1;
    let other = LayerId(app.run("type.create", json!({"text": "Other", "size": 32, "x": right + 6, "y": 420})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": id.0})).unwrap();
    begin_edit(&mut app).unwrap();
    let mut h = harness(1.0, app);
    h.run_steps(1);
    let f = frame_of(h.state_mut()).unwrap();
    let ob = h.state().session.active().unwrap().doc.layer(other).unwrap().surface().unwrap().content_bounds();
    let mid = f64::from(ob.y0 + ob.y1) / 2.0;
    let on_other = frame_point(h.state(), [f64::from(ob.x0) + 4.0, mid]);
    let edge = frame_point(h.state(), [f.quad[1][0], mid]);
    assert!(on_other.x - edge.x > 0.0 && on_other.x - edge.x < 40.0, "the other text is within reach: {} pt", on_other.x - edge.x);
    let before = text(h.state(), id);
    transform_drag(&mut h, on_other, on_other + egui::vec2(60.0, 30.0), Modifiers::NONE, false);
    assert_affine(text(h.state(), id).transform, before.transform);
    assert_ne!(h.state().ui.text_edit.as_ref().map(|e| e.layer), Some(id.0), "the edit moved on");
}

/// #1091: modifier changes alone show/hide the oriented frame without touching editing/history.
#[test]
fn temporary_type_transform_visibility_and_cursors() {
    let (app, id) = transform_app(false, false, Affine::translate(500.0, 350.0));
    let mut h = harness(1.0, app);
    let before = history_len(&h);
    let original = text(h.state(), id);
    assert!(!crate::type_transform::visible(h.state(), Mods::default()));
    hold(&mut h, type_command());
    assert!(crate::type_transform::visible(h.state(), held(&h)));
    assert!(frame_of(h.state_mut()).is_some());
    hold(&mut h, Modifiers::NONE);
    assert!(!crate::type_transform::visible(h.state(), held(&h)));
    assert_eq!(selection(&h), (3, 3));
    assert_eq!(text(h.state(), id).transform, original.transform);
    assert_eq!(history_len(&h), before);
    assert!(h.state().ui.transform.is_none());
    assert!(h.state().ui.type_transform.is_none());
    // macOS's real Control is not the logical Command modifier.
    assert!(!crate::type_transform::visible(h.state(), Mods { ctrl: true, ..Mods::default() }));
    h.state_mut().ui.shell.latched = command_mods();
    h.run_steps(1);
    assert!(crate::type_transform::visible(h.state(), held(&h)));
}

/// Real canvas input: layer-local scaling stays correct across orientation, zoom, HiDPI and flip.
#[test]
fn temporary_type_transform_scales_without_reflow_or_leaving_type() {
    // One case per zoom, pairwise over the other axes: both ppp values meet both flips, point, paragraph
    // and vertical type each appear, both sides of the 200 % view choice below run twice, and the ends of
    // the range in points (25 % at 2x, 800 % at 1x) are kept. The frame and the drag map through the view
    // the same way for any kind of type, and the kind decides the layer's frame the same way at any view.
    for (ppp, zoom, (paragraph, vertical, flip)) in
        [(2.0, 0.25, (false, false, false)), (1.0, 1.0, (true, false, true)), (2.0, 2.0, (false, true, true)), (1.0, 8.0, (false, false, false))]
    {
        let transform = Affine { m: [1.2, 0.25, 0.18, 0.9, 500.0, 360.0] };
        let (app, id) = transform_app(paragraph, vertical, transform);
        let mut h = harness(ppp, app);
        let f = frame_of(h.state_mut()).unwrap();
        let center = f.pivot;
        h.state_mut().ui.view.flip_horizontal = flip;
        // At high zoom, show the dragged corner; a release beyond the canvas is valid.
        let at = if zoom >= 2.0 { [f.quad[2][0] as f32, f.quad[2][1] as f32] } else { [center[0] as f32 + 17.0, center[1] as f32 - 9.0] };
        set_view(&mut h, zoom, at);
        h.run_steps(2);
        let before = text(h.state(), id);
        let count = history_len(&h);
        let (a, b) = (f.quad[2], [f.quad[0][0] + 1.25 * (f.quad[2][0] - f.quad[0][0]), f.quad[0][1] + 1.25 * (f.quad[2][1] - f.quad[0][1])]);
        let (a, b) = (frame_point(h.state(), a), frame_point(h.state(), b));
        assert!(canvas_rect(h.state()).contains(a));
        transform_drag(&mut h, a, b, type_command() | Modifiers::SHIFT, true);
        let after = text(h.state(), id);
        assert_affine(
            after.transform,
            transform.mul(&Affine::translate(f.rect[0], f.rect[1])).mul(&Affine::scale(1.25)).mul(&Affine::translate(-f.rect[0], -f.rect[1])),
        );
        assert_eq!(
            (after.text.as_str(), &after.runs, &after.paragraphs, after.orientation, after.shape),
            (before.text.as_str(), &before.runs, &before.paragraphs, before.orientation, before.shape)
        );
        assert_eq!(selection(&h), (3, 3));
        assert_eq!(h.state().ui.text_edit.as_ref().unwrap().layer, id.0);
        assert!(h.state().ui.tool.is_type());
        assert!(h.state().ui.transform.is_none());
        assert!(h.state().ui.type_transform.is_none());
        assert_eq!(history_len(&h), count + 1);
        // Typing goes through the window host, as the platform's reaches the focused editor.
        super::commit_tests::type_keys(h.state_mut(), "!");
        assert!(text(h.state(), id).text.starts_with("Hel!lo"));
        h.key_press(if vertical { egui::Key::ArrowUp } else { egui::Key::ArrowLeft });
        h.run_steps(1);
        assert_eq!(selection(&h), (3, 3));
        assert_eq!(history_len(&h), count + 1, "one edit session");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_affine(text(h.state(), id).transform, before.transform);
        assert_eq!(text(h.state(), id).text, before.text);
        h.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(text(h.state(), id).transform, after.transform);
    }
}

/// The preview of every drag mode leaves the engine and the history alone until the release, which
/// commits one change without reflowing a paragraph.
#[test]
fn temporary_type_transform_free_scale_center_scale_move_skew_and_rotation() {
    for gesture in ["free", "center", "move", "skew", "rotate", "snap"] {
        let initial = Affine::translate(400.0, 350.0).mul(&Affine::rotate(0.2));
        let (mut app, id) = transform_app(true, false, initial);
        let frame = frame_of(&mut app).unwrap();
        let before = text(&app, id);
        let (a, b, mods) = match gesture {
            "move" => (frame.quad[0].map(|v| v + 20.0), frame.quad[0].map(|v| v + 45.0), type_command()),
            "skew" => {
                let p = [(frame.quad[0][0] + frame.quad[1][0]) / 2.0, (frame.quad[0][1] + frame.quad[1][1]) / 2.0];
                (p, [p[0] + 30.0, p[1] + 12.0], type_command())
            }
            "rotate" | "snap" => {
                let a = [frame.quad[1][0] + 20.0, frame.quad[1][1] - 10.0];
                // A snapped drag turns in 15° steps from where it started, so the first preview steps need a
                // turn of at least half a step to change anything (#11099).
                let da: f64 = if gesture == "snap" { 0.9 } else { 0.42 };
                let (x, y) = (a[0] - frame.pivot[0], a[1] - frame.pivot[1]);
                let b = [frame.pivot[0] + x * da.cos() - y * da.sin(), frame.pivot[1] + x * da.sin() + y * da.cos()];
                (a, b, if gesture == "snap" { type_command() | Modifiers::SHIFT } else { type_command() })
            }
            _ => {
                let a = frame.quad[2];
                let local = [frame.rect[2] + 60.0, frame.rect[3] + 18.0];
                let p = initial.apply(Point::new(local[0], local[1]));
                (a, [p.x, p.y], if gesture == "center" { type_command() | Modifiers::ALT } else { type_command() })
            }
        };
        let count = app.session.active().unwrap().history.entries().len();
        tool_event(&mut app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, mods);
        for k in 1..=5 {
            let f = f64::from(k) / 5.0;
            tool_event(&mut app, ToolEvent::Move { x: a[0] + (b[0] - a[0]) * f, y: a[1] + (b[1] - a[1]) * f, pressure: 1.0 }, mods);
            assert!(crate::type_transform::display_doc(&mut app, 0).is_some(), "{gesture}");
            assert_eq!(text(&app, id).transform, initial, "{gesture}: preview leaves engine untouched");
        }
        assert_eq!(app.session.active().unwrap().history.entries().len(), count, "{gesture}: preview leaves history untouched");
        tool_event(&mut app, ToolEvent::Up { x: b[0], y: b[1] }, mods);
        let after = text(&app, id);
        assert_ne!(after.transform, initial, "{gesture}");
        assert_eq!(after.shape, before.shape, "{gesture}: no paragraph reflow");
        assert_eq!(app.session.active().unwrap().history.entries().len(), count + 1, "{gesture}: one step");
        assert_eq!(app.ui.text_edit.as_ref().map(|e| (e.anchor, e.caret)), Some((3, 3)), "{gesture}");
    }
}

/// Editing paragraph text, a click inside the box but off the glyphs moves the caret to the
/// nearest offset, as in Photoshop (the editor's rect is the box); a click outside the box commits.
#[test]
fn click_inside_the_paragraph_box_off_the_glyphs_moves_the_caret() {
    // The box is 240 × 180; the two 32 pt lines only fill its top.
    let (mut app, id) = transform_app(true, false, Affine::translate(400.0, 350.0));
    let (bx, by, _, h) = super::box_shape(&app, id).unwrap();
    let (l, aff, _) = layout(&mut app, id).unwrap();
    let click = |app: &mut PhotocraftApp, tx: f32, ty: f32| {
        let p = aff.apply(Point::new(f64::from(tx), f64::from(ty)));
        drag_create(app, [p.x, p.y], [p.x, p.y]);
    };
    let layers = app.session.active().unwrap().doc.layers.len();
    // Below the two lines, just right of the box's left edge and 40 units above its bottom: clear of the bottom-left
    // handle's reach at the zoom the view fits the document to (the reach is in screen points; the box shows at
    // about half size).
    assert!(!photocraft_text::text_point_inside(&l, bx + 8.0, by + h - 40.0, 6.0), "the point is off the glyphs");
    click(&mut app, bx + 8.0, by + h - 40.0);
    let ed = app.ui.text_edit.clone().expect("still editing");
    // "Hello é世界\n" is 10 characters: the start of the second line.
    assert_eq!((ed.layer, ed.caret, ed.anchor), (id.0, 10, 10));
    // Outside the box the edit commits, without starting new text.
    // 40 screen points out is past the move reach (38 pt), whatever the zoom.
    let out = 40.0 / app.point_zoom().max(0.01);
    click(&mut app, bx - out, by + h - 8.0);
    assert!(app.ui.text_edit.is_none());
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers);
}

#[test]
fn temporary_type_transform_noop_cancel_and_stale_capture() {
    for stop in ["noop", "escape", "focus", "tool", "undo", "delete", "document", "close"] {
        let original = Affine::translate(400.0, 350.0);
        let (app, id) = transform_app(false, false, original);
        let mut h = harness(1.0, app);
        let f = frame_of(h.state_mut()).unwrap();
        let a = f.quad[2];
        let count = history_len(&h);
        tool_event(h.state_mut(), ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, type_command());
        tool_event(h.state_mut(), ToolEvent::Move { x: a[0] + 50.0, y: a[1] + 20.0, pressure: 1.0 }, type_command());
        let preview = {
            let (shown, _) = crate::type_transform::display_doc(h.state_mut(), 0).unwrap();
            std::sync::Arc::downgrade(&shown)
        };
        assert_eq!(text(h.state(), id).transform, original);
        match stop {
            "noop" => {
                tool_event(h.state_mut(), ToolEvent::Up { x: a[0], y: a[1] }, Modifiers::NONE);
            }
            "escape" => {
                h.key_press(egui::Key::Escape);
                h.run_steps(2);
            }
            "focus" => {
                h.input_mut().focused = false;
                h.run_steps(2);
            }
            "tool" => {
                h.state_mut().ui.tool = crate::state::Tool::Brush;
                h.state_mut().sync_views();
                crate::canvas::testing::frame(h.state_mut());
            }
            "undo" => {
                h.state_mut().run("edit.undo", json!({})).unwrap();
            }
            "delete" => {
                h.state_mut().run("layer.delete", json!({"layer": id.0})).unwrap();
            }
            "close" => {
                h.state_mut().run("file.close", json!({})).unwrap();
            }
            _ => {
                h.state_mut().run("file.new", json!({"width": 20, "height": 20})).unwrap();
            }
        }
        assert!(h.state().ui.type_transform.is_none(), "{stop}");
        assert!(preview.upgrade().is_none(), "{stop}: release rendered preview without waiting for another frame");
        if ["noop", "escape", "focus", "tool"].contains(&stop) {
            assert_eq!(text(h.state(), id).transform, original);
            assert_eq!(history_len(&h), count);
        }
        if stop == "escape" {
            assert!(h.state().ui.text_edit.is_some());
        }
    }
}

#[test]
fn temporary_type_transform_control_roundtrip_and_input_safety() {
    let initial = Affine::translate(400.0, 350.0);
    let (mut app, id) = transform_app(true, false, initial);
    // A document selection must not restrict a text-layer transformation.
    app.run("select.rect", json!({"x": 1, "y": 1, "width": 2, "height": 2})).unwrap();
    let frame = frame_of(&mut app).unwrap();
    let a = frame.quad[2];
    crate::canvas::testing::press(&mut app, a, type_command());
    let inspected = crate::type_transform::inspect(&app);
    assert_eq!(inspected["frame"]["layer"], id.0, "the press alone captures the text: {inspected}");
    crate::canvas::testing::move_to(&mut app, [a[0] + 60.0, a[1] + 30.0], type_command());
    crate::canvas::testing::release(&mut app, [a[0] + 60.0, a[1] + 30.0], Modifiers::NONE);
    assert_ne!(text(&app, id).transform, initial);
    let doc = &app.session.active().unwrap().doc;
    for name in ["type.pcraft", "type.psd"] {
        let out = photocraft_io::export(doc, name, &Default::default()).unwrap();
        let back = photocraft_io::import(name, &out.bytes).unwrap().document;
        let t = back.walk().into_iter().find_map(|(_, _, l)| if let LayerContent::Text(t) = &l.content { Some(t) } else { None }).unwrap();
        assert_affine(t.transform, text(&app, id).transform);
        assert_eq!(t.text, text(&app, id).text);
        assert_eq!(t.shape, text(&app, id).shape);
    }
    for content in ["", "   ", "é👩‍💻世界", "\n\n"] {
        app.run("type.edit", json!({"layer": id.0, "text": content, "point": [400, 350]})).unwrap();
        assert!(crate::type_transform::frame(&mut app).is_some());
        crate::canvas::testing::press(&mut app, [400.0, 350.0], type_command());
        crate::canvas::testing::move_to(&mut app, [1e300, -1e300], type_command());
        crate::canvas::testing::release(&mut app, [400.0, 350.0], type_command());
        assert!(text(&app, id).transform.m.iter().all(|v| v.is_finite()));
        assert!(app.ui.type_transform.is_none());
    }
    app.run("layer.lockLayers", json!({"position": true})).unwrap();
    assert!(crate::type_transform::frame(&mut app).is_none());
}

/// The preview document has the pixel format of the document, whatever its depth and colour model.
#[test]
fn temporary_type_transform_preserves_depth_and_color_model() {
    for (depth, mode) in [(8, "rgb"), (16, "rgb"), (32, "rgb"), (16, "gray"), (8, "cmyk"), (16, "lab")] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 1200, "height": 900, "depth": depth, "mode": mode})).unwrap();
        let id = LayerId(app.run("type.create", json!({"text": "PhotoCraft é世界", "size": 32, "x": 300, "y": 350})).unwrap()["layer"].as_u64().unwrap());
        begin_edit(&mut app).unwrap();
        let format = app.session.active().unwrap().doc.pixel_format();
        let a = frame_of(&mut app).unwrap().quad[2];
        tool_event(&mut app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, type_command());
        tool_event(&mut app, ToolEvent::Move { x: a[0] + 30.0, y: a[1] + 15.0, pressure: 1.0 }, type_command());
        let (shown, _) = crate::type_transform::display_doc(&mut app, 0).unwrap();
        assert_eq!(shown.pixel_format(), format);
        assert_eq!(text_layer(&shown, id).unwrap().cache.as_ref().unwrap().format(), format);
        tool_event(&mut app, ToolEvent::Up { x: a[0] + 30.0, y: a[1] + 15.0 }, Modifiers::NONE);
        assert_eq!(text(&app, id).cache.as_ref().unwrap().format(), format);
        assert_eq!(text(&app, id).text, "PhotoCraft é世界");
    }
}

#[test]
fn temporary_type_transform_captures_press_and_rejects_collapsed_geometry() {
    let (app, id) = transform_app(false, false, Affine::translate(450.0, 350.0));
    let mut h = harness(1.0, app);
    let frame = frame_of(h.state_mut()).unwrap();
    let a = frame_point(h.state(), frame.quad[2]);
    let b = a + vec2(50.0, 20.0);
    hold(&mut h, type_command());
    h.hover_at(a);
    transform_press(&mut h, a, true, type_command());
    assert!(h.state().ui.type_transform.is_some(), "capture before any movement");
    hold(&mut h, Modifiers::NONE);
    h.hover_at(b);
    h.run_steps(2);
    transform_press(&mut h, b, false, Modifiers::NONE);
    assert_ne!(text(h.state(), id).transform, Affine::translate(450.0, 350.0));
    assert_eq!(selection(&h), (3, 3));
    let frame = frame_of(h.state_mut()).unwrap();
    let before = text(h.state(), id).transform;
    let (a, b) = (frame.quad[2], frame.quad[0]);
    tool_event(h.state_mut(), ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, type_command());
    tool_event(h.state_mut(), ToolEvent::Up { x: b[0], y: b[1] }, type_command());
    assert_eq!(text(h.state(), id).transform, before, "collapsed scale is rejected");
    let mut saved = serde_json::to_value(&h.state().ui).unwrap();
    saved["type_transform"] = json!({"invalid": "restored pointer capture"});
    let restored: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert!(restored.type_transform.is_none());
}

#[test]
fn temporary_type_transform_imported_psd_retains_editability_and_styles() {
    let (mut app, original_id) = transform_app(false, false, Affine { m: [1.5, 0.2, 0.1, 0.8, 400.0, 350.0] });
    app.run("type.setStyle", json!({"layer": original_id.0, "range": [0, 5], "fauxBold": true, "color": "#3366cc", "tracking": 25, "align": "center"}))
        .unwrap();
    app.run("layer.renameLayer", json!({"layer": original_id.0, "name": "Styled caption"})).unwrap();
    app.run("layer.layerStyle.dropShadow", json!({"layer": original_id.0, "distance": 8, "size": 4})).unwrap();
    let exported = photocraft_io::export(&app.session.active().unwrap().doc, "type.psd", &Default::default()).unwrap();
    let imported = photocraft_io::import("type.psd", &exported.bytes).unwrap().document;
    let mut app = new_app();
    app.session.add_document(imported, None);
    app.sync_views();
    let id = app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| matches!(l.content, LayerContent::Text(_))).unwrap().2.id;
    app.session.select_layer(id).unwrap();
    begin_edit(&mut app).unwrap();
    let before = text(&app, id);
    assert!(before.runs.len() > 1, "mixed character styles were imported");
    let layer_before = app.session.active().unwrap().doc.layer(id).unwrap().clone();
    assert!(!layer_before.effects.items.is_empty());
    let f = frame_of(&mut app).unwrap();
    let a = f.quad[2];
    tool_event(&mut app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, type_command());
    tool_event(&mut app, ToolEvent::Up { x: a[0] + 35.0, y: a[1] + 10.0 }, type_command());
    let after = text(&app, id);
    assert_eq!((after.text.as_str(), &after.runs, &after.paragraphs), (before.text.as_str(), &before.runs, &before.paragraphs));
    assert!(after.psd_raw.is_some());
    let layer_after = app.session.active().unwrap().doc.layer(id).unwrap();
    assert_eq!(layer_after.name, layer_before.name);
    assert_eq!(layer_after.effects, layer_before.effects);
    assert!(app.ui.text_edit.is_some());
    assert!(app.ui.transform.is_none());
    cancel_edit(&mut app);
    assert_affine(text(&app, id).transform, before.transform);
}

/// The style of a face named only by its PostScript name is the face's own subfamily, which the
/// Character controls' `CharacterValues.style` shows.
#[test]
fn postscript_only_style_shows_actual_subfamily() {
    // `selected_style` reads the engine only once the font scan is done (try_shared). Under nextest
    // this test runs in a process of its own, where nothing else has started the scan, so start it.
    let _ = photocraft_text::shared();
    let style = photocraft_doc::text::CharStyle { font_family: "Inter".into(), postscript_name: Some("Inter-SemiBold".into()), ..Default::default() };
    // PLAN: `values::character_values` reads `fonts::selected_style` for `CharacterValues.style`; no layer constructor sets only a PostScript name
    assert_eq!(super::selected_style(&style), "SemiBold");
}

/// A family's style list is the craft catalog's list for it, variable faces' named instances
/// included; an unknown family offers Regular alone.
#[test]
fn variable_faces_list_the_weights_of_their_axis() {
    let _ = photocraft_text::shared();
    let catalog = photocraft_text::fonts::catalog();
    let listed = super::fonts::styles("Inter");
    let expected: Vec<String> = catalog.styles("Inter").into_iter().map(|face| face.style).collect();
    assert!(!expected.is_empty(), "Inter is installed");
    for style in &expected {
        assert!(listed.contains(style), "{style} missing from {listed:?}");
    }
    assert_eq!(super::fonts::styles("Missing test family"), ["Regular"]);
}

/// Families the host serves are in the picker's source before they are fetched.
#[test]
fn the_font_menu_lists_served_families() {
    {
        let _catalog = crate::served_fonts::changing_catalog();
        photocraft_text::served::add_families(["Served Menu Test Serif".to_string()]);
    }
    let app = new_app();
    let mut state = super::font_state(&app);
    let source = state.source();
    assert!(source.served.iter().any(|f| f == "Served Menu Test Serif"));
}

/// #2236: formatting applies to the selected type layers, while an active text edit stays local.
mod multi_layer_formatting {
    use super::*;
    use egui::accesskit::Role;
    use egui_kittest::kittest::NodeT;
    use photocraft_doc::text::{AntiAlias, CharStyle, Orientation, TextAlign};

    fn selected(app: &mut PhotocraftApp, ids: &[LayerId]) {
        for (i, id) in ids.iter().enumerate() {
            app.run("layer.select", json!({"layer": id.0, "mode": if i == 0 { "replace" } else { "add" }})).unwrap();
        }
    }

    fn fixture() -> (Box<PhotocraftApp>, [LayerId; 3]) {
        let mut app = new_app();
        let ids = std::array::from_fn(|i| {
            LayerId(
                app.run("type.create", json!({"text": "AéBCD", "font": "Inter", "size": 12 + i * 4, "color": "#000000"})).unwrap()["layer"].as_u64().unwrap(),
            )
        });
        app.run("type.setStyle", json!({"layer": ids[0].0, "fontStyle": "Bold Italic", "tracking": 20})).unwrap();
        selected(&mut app, &ids[..2]);
        (app, ids)
    }

    fn style(app: &PhotocraftApp, id: LayerId, ci: usize) -> CharStyle {
        let t = text(app, id);
        let byte = t.text.char_indices().nth(ci).unwrap().0;
        let mut end = 0;
        t.char_runs()
            .into_iter()
            .find(|r| {
                end += r.len;
                byte < end
            })
            .unwrap()
            .style
    }

    /// The Properties panel (`properties`) or the options bar over `app`.
    fn controls(app: Box<PhotocraftApp>, properties: bool) -> Harness<'static, Box<PhotocraftApp>> {
        if properties { panel_harness(app) } else { bar_harness(app) }
    }

    fn number(h: &mut Harness<'static, Box<PhotocraftApp>>, label: &str, value: &str) {
        // Variable-font axes add numeric fields before size/leading. Select the setting by
        // its accessible name, so a font scan cannot shift this action onto another control.
        let name = field_name(h, Role::SpinButton, label);
        h.get_by_role_and_label(Role::SpinButton, &name).click();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text(value.into()));
        h.key_press(egui::Key::Enter);
        h.run();
    }

    fn preset(h: &mut Harness<'static, Box<PhotocraftApp>>, label: &str) {
        reveal(h, label);
        h.run();
        h.get_by_label(label).click();
        h.run();
    }

    /// Opens a preset chevron by its accessible name and picks `label`.
    fn pick(h: &mut Harness<'static, Box<PhotocraftApp>>, menu: &str, label: &str) {
        open_presets(h, menu);
        h.run();
        preset(h, label);
    }

    fn editing(app: &mut PhotocraftApp, id: LayerId) {
        app.ui.text_edit = Some(edit_of(id.0, 3, 1, "batch-type-edit"));
    }

    /// The options bar's direction is the Paragraph panel's: `TypeBar` has only alignment buttons
    /// (type_bar.rs:92), so the direction buttons are driven through the Paragraph controls.
    #[test]
    fn options_bar_direction_sets_selected_layers_and_new_type() {
        use photocraft_doc::text::TextDirection;
        let (app, ids) = fixture();
        let mut h = controls(app, true);
        h.get_by_label("Right-to-left paragraph direction").click();
        h.run();
        assert_eq!(h.state().ui.tool_options.type_direction, "rtl");
        for id in &ids[..2] {
            assert_eq!(text(h.state(), *id).paragraph_runs()[0].style.direction, TextDirection::Rtl);
        }
        assert_eq!(text(h.state(), ids[2]).paragraph_runs()[0].style.direction, TextDirection::Auto);
        drag_create(h.state_mut(), [40.0, 60.0], [40.0, 60.0]);
        let id = h.state().session.active().unwrap().active_layer.unwrap();
        assert!(!ids.contains(&id), "a new type layer");
        assert_eq!(text(h.state(), id).paragraph_runs()[0].style.direction, TextDirection::Rtl, "new type starts right to left");
    }

    #[test]
    fn character_panel_preset_menus_edit_size_leading_and_tracking() {
        let (app, ids) = fixture();
        let untouched = style(&app, ids[2], 0);
        let steps = app.session.active().unwrap().history.entries().len();
        let mut h = controls(app, true);

        // A preset is the same change as typing the number beside it: Auto is leading `None`.
        pick(&mut h, "Font size presets", "72 pt");
        pick(&mut h, "Leading presets", "Auto");
        pick(&mut h, "Tracking presets", "-5");

        for id in &ids[..2] {
            let c = style(h.state(), *id, 0);
            assert_eq!(c.size_pt, 72.0);
            assert_eq!(c.leading_pt, None, "Auto restores font-relative line spacing");
            assert_eq!(c.tracking, -5.0);
        }
        assert_eq!(style(h.state(), ids[2], 0), untouched, "unselected type layers are unchanged");
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps + 3, "one step per pick");

        // The numeric spin boxes must remain usable beside their preset chevrons.
        number(&mut h, "Font size", "40");
        assert_eq!(style(h.state(), ids[0], 0).size_pt, 40.0);
        assert_eq!(style(h.state(), ids[1], 0).size_pt, 40.0);
    }

    #[test]
    fn options_font_family_preserves_peer_faces_and_batches_layer_flags() {
        let _catalog = crate::served_fonts::steady_catalog();
        let (app, ids) = fixture();
        let before = ids.map(|id| style(&app, id, 0));
        let mut h = controls(app, false);
        h.get_by_role_and_label(Role::ComboBox, "Font family").click();
        h.run();
        h.event(egui::Event::Text("JetBrains".into()));
        h.run();
        h.key_press(egui::Key::ArrowDown);
        h.run();
        h.key_press(egui::Key::Enter);
        h.run();
        for i in 0..2 {
            let mut expected = before[i].clone();
            expected.font_family = "JetBrains Mono".into();
            // #3556: the new family's installed face at the peer's weight and italic is named (no
            // name without such a face), so a PSD names a real font.
            expected.postscript_name = photocraft_text::shared()
                .lock()
                .unwrap()
                .fonts
                .faces("JetBrains Mono")
                .into_iter()
                .find(|f| f.italic == expected.italic && f.weight.round() as u16 == expected.weight)
                .and_then(|f| f.postscript_name);
            assert_eq!(style(h.state(), ids[i], 0), expected, "family changes preserve each peer's face and metrics");
        }
        assert_eq!(style(h.state(), ids[2], 0), before[2]);
        h.get_all_by_role(Role::ComboBox).find(|n| n.value().as_deref() == Some("Smooth")).unwrap().focus();
        h.run_steps(2);
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        reveal(&mut h, "Sharp");
        h.get_by_label("Sharp").focus();
        h.run_steps(2);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        h.get_by_role_and_label(Role::Button, "Toggle text orientation").click();
        h.run();
        for id in &ids[..2] {
            let t = text(h.state(), *id);
            assert_eq!(t.antialias, AntiAlias::Sharp);
            assert_eq!(t.orientation, Orientation::Vertical);
        }
        assert_eq!(text(h.state(), ids[2]).orientation, Orientation::Horizontal);
        assert_ne!(text(h.state(), ids[2]).antialias, AntiAlias::Sharp);
    }

    /// The family field and the style menu describe the selected run, not the layer's first run
    /// (#9024), whichever way the selection was made; without a selection the first run shows, and
    /// without a type target the tool defaults. Showing them never adds history. "Mé Regular" has
    /// a two-byte `é`, so a byte offset taken for a character offset picks the wrong run.
    #[test]
    fn options_bar_font_and_style_follow_the_selected_run() {
        let mut app = new_app();
        let id = LayerId(
            app.run("type.create", json!({"text": "Mé Regular", "font": "Inter", "fontStyle": "Medium", "size": 20, "x": 300, "y": 420})).unwrap()["layer"]
                .as_u64()
                .unwrap(),
        );
        app.run("type.setStyle", json!({"layer": id.0, "range": [3, 10], "font": "JetBrains Mono", "fontStyle": "Regular"})).unwrap();
        assert_eq!((style(&app, id, 0).font_family.as_str(), style(&app, id, 5).font_family.as_str()), ("Inter", "JetBrains Mono"));
        let mut h = controls(app, false);
        let shown = |h: &Harness<'static, Box<PhotocraftApp>>| {
            (
                h.get_by_role_and_label(Role::ComboBox, "Font family").value().unwrap_or_default(),
                h.get_by_role_and_label(Role::ComboBox, &field_name(h, Role::ComboBox, "Style")).value(),
            )
        };
        let steps = history_len(&h);
        let (inter, mono) = (("Inter".to_string(), Some("Medium".to_string())), ("JetBrains Mono".to_string(), Some("Regular".to_string())));
        for (anchor, caret, want) in [(3, 10, &mono), (10, 3, &mono), (0, 2, &inter), (2, 0, &inter)] {
            h.state_mut().ui.text_edit = Some(edit_of(id.0, caret, anchor, "s"));
            h.run_steps(4);
            assert_eq!(&shown(&h), want, "selection {anchor}..{caret}");
        }
        // Ending the edit keeps the whole-layer rule: the layer is still selected, its style is the
        // first run's (Medium) and, the runs using two families, the family field is blank.
        h.state_mut().ui.text_edit = None;
        h.run_steps(4);
        assert_eq!(shown(&h), (String::new(), inter.1.clone()), "no edit: first run's style, mixed family blank");
        assert_eq!(history_len(&h), steps, "displaying the controls adds no history");
        // Control: no type target shows the tool defaults.
        h.state_mut().run("layer.new.layer", json!({})).unwrap();
        h.state_mut().ui.tool_options.type_font = "JetBrains Mono".into();
        h.state_mut().ui.tool_options.type_style = "Regular".into();
        h.run_steps(4);
        assert_eq!(shown(&h), mono, "no type target: the tool defaults");
    }

    /// A style picked in the Character panel's Style menu, with type layers selected, sets them
    /// and is kept for new type: the pick is one `{font, fontStyle}` change.
    #[test]
    fn a_style_picked_in_the_character_sections_font_menu_is_kept_for_new_type() {
        let _catalog = crate::served_fonts::steady_catalog();
        let (app, ids) = fixture();
        // The first style of the family other than the defaults'.
        let pick = super::super::fonts::styles("Inter").into_iter().find(|s| s != "Regular" && s != "Bold Italic").expect("Inter has several styles");
        assert_ne!(app.ui.tool_options.type_style, pick);
        let mut h = controls(app, true);
        h.get_by_role_and_label(Role::ComboBox, "Style").click();
        h.run();
        h.get_by_label(&super::super::fonts::style_label(&pick)).click();
        h.run();
        assert_eq!(h.state().ui.tool_options.type_style, pick, "new type takes the picked style");
        for id in &ids[..2] {
            assert_eq!(style(h.state(), *id, 0).font_style, pick, "the selected layers are set in it");
        }
    }

    #[test]
    fn properties_edit_shown_metrics_and_paragraphs_with_a_non_type_primary() {
        let (mut app, ids) = fixture();
        let untouched = style(&app, ids[2], 0);
        for (id, scale) in [(ids[0], 2.0), (ids[1], 4.0)] {
            app.run("type.edit", json!({"layer": id.0, "transform": [scale, 0.0, 0.0, scale, 0.0, 0.0]})).unwrap();
        }
        let raster = LayerId(app.run("layer.new.layer", json!({"name": "Selected raster"})).unwrap()["layer"].as_u64().unwrap());
        selected(&mut app, &[ids[0], ids[1], raster]);
        let mut h = panel_harness_without_frame_logic(app);
        number(&mut h, "Font size", "60");
        // Differ from the automatic 72 pt leading so this is an actual property change.
        number(&mut h, "Leading", "84");
        h.get_by_label("Faux Bold").click();
        h.run();
        h.get_by_label("Center text").click();
        h.run();
        number(&mut h, "Indent left margin", "9");
        for (id, scale) in [(ids[0], 2.0), (ids[1], 4.0)] {
            let c = style(h.state(), id, 0);
            assert_eq!(c.size_pt * scale, 60.0);
            assert_eq!(c.leading_pt.unwrap() * scale, 84.0);
            assert!(c.faux_bold);
            let p = text(h.state(), id).paragraph_runs()[0].style.clone();
            assert_eq!(p.align, TextAlign::Center);
            assert_eq!(p.start_indent_pt, 9.0);
        }
        assert_eq!(style(h.state(), ids[2], 0), untouched);
        assert_eq!(h.state().session.active().unwrap().active_layer, Some(raster));
        // The same controls edit only the selected Unicode span during a text session.
        let peer = text(h.state(), ids[1]).char_runs();
        editing(h.state_mut(), ids[0]);
        h.run();
        number(&mut h, "Font size", "40");
        assert_eq!((size_at(h.state(), ids[0], 0), size_at(h.state(), ids[0], 1), size_at(h.state(), ids[0], 3)), (30.0, 20.0, 30.0));
        assert_eq!(text(h.state(), ids[1]).char_runs(), peer);
    }

    /// #6491: with Units & Rulers ▸ Type set to pixels, the Character and Paragraph fields show
    /// and take pixels, like the options bar (`doc_unit`). 144 ppi so a pixel is not a point.
    #[test]
    fn character_and_paragraph_fields_follow_the_type_units_preference() {
        // The resolution is set before the type exists: Image Size keeps existing type's pixel
        // size (#7125), so a layer made earlier would be 6 pt here.
        let mut app = new_app();
        app.run("prefs.set", json!({"path": "unitsAndRulers.typeUnits", "value": "pixels"})).unwrap();
        app.run("image.imageSize", json!({"resolution": 144, "resample": "none"})).unwrap();
        let ids =
            [LayerId(app.run("type.create", json!({"text": "AéBCD", "font": "Inter", "size": 12, "color": "#000000"})).unwrap()["layer"].as_u64().unwrap())];
        selected(&mut app, &[ids[0]]);
        let mut h = controls(app, true);
        let shown = h.get_by_role_and_label(Role::SpinButton, "Font size").accesskit_node().numeric_value();
        assert_eq!(shown, Some(24.0), "12 pt at 144 ppi");
        number(&mut h, "Font size", "60");
        number(&mut h, "Leading", "84");
        number(&mut h, "Baseline shift", "10");
        number(&mut h, "Indent left margin", "9");
        let c = style(h.state(), ids[0], 0);
        assert_eq!((c.size_pt, c.leading_pt, c.baseline_shift_pt), (30.0, Some(42.0), 5.0));
        assert_eq!(text(h.state(), ids[0]).paragraph_runs()[0].style.start_indent_pt, 4.5);

        pick(&mut h, "Font size presets", "144 px");
        pick(&mut h, "Leading presets", "300 px");
        let c = style(h.state(), ids[0], 0);
        assert_eq!((c.size_pt, c.leading_pt), (72.0, Some(150.0)), "presets preserve point values in pixel display units");
    }

    /// #5717: a layer scaled so its shown size or leading is past a field's range (2400 pt, or
    /// 0.08 pt) keeps it while the Character panel and the options bar only show it: showing
    /// writes no edit.
    #[test]
    fn showing_a_scaled_layer_past_the_size_range_does_not_change_it() {
        for (layer, scale) in [(0, 200.0), (1, 0.005)] {
            for properties in [true, false] {
                let (mut app, ids) = fixture();
                let id = ids[layer];
                app.run("type.edit", json!({"layer": id.0, "transform": [scale, 0.0, 0.0, scale, 0.0, 0.0]})).unwrap();
                selected(&mut app, &[id]);
                let before = text(&app, id).char_runs();
                let steps = app.session.active().unwrap().history.entries().len();
                let mut h = controls(app, properties);
                h.run_steps(3);
                assert_eq!(text(h.state(), id).char_runs(), before, "scale {scale}, properties {properties}");
                assert_eq!(history_len(&h), steps, "no history step");
            }
        }
    }

    #[test]
    fn formatting_menus_isolate_the_edit_layer_and_range_but_honor_explicit_targets() {
        let (mut app, ids) = fixture();
        let ctx = app.ctx().clone();
        let peer = text(&app, ids[1]);
        editing(&mut app, ids[0]);
        for command in ["type.antiAlias.sharp", "type.orientation.vertical", "type.openType.discretionaryLigatures"] {
            crate::menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        }
        assert_eq!(text(&app, ids[0]).antialias, AntiAlias::Sharp);
        assert_eq!(text(&app, ids[0]).orientation, Orientation::Vertical);
        assert!(!style(&app, ids[0], 0).discretionary_ligatures);
        assert!(style(&app, ids[0], 1).discretionary_ligatures);
        assert!(!style(&app, ids[0], 3).discretionary_ligatures);
        assert_eq!(text(&app, ids[1]).char_runs(), peer.char_runs());
        assert_eq!(text(&app, ids[1]).antialias, peer.antialias);
        assert_eq!(text(&app, ids[1]).orientation, peer.orientation);
        crate::menus::invoke(&mut app, &ctx, "type.antiAlias.crisp", json!({"layer": ids[1].0})).unwrap();
        crate::menus::invoke(&mut app, &ctx, "type.orientation.vertical", json!({"layers": [ids[2].0]})).unwrap();
        assert_eq!(text(&app, ids[1]).antialias, AntiAlias::Crisp);
        assert_eq!(text(&app, ids[2]).orientation, Orientation::Vertical);
        assert_eq!(text(&app, ids[0]).antialias, AntiAlias::Sharp);
    }

    /// Type › Anti-Alias and Type › Orientation check the edited (else selected) layer's value.
    #[test]
    fn type_menu_checks_the_current_anti_alias_and_orientation() {
        let (mut app, ids) = fixture();
        app.run("type.antiAlias.none", json!({"layer": ids[0].0})).unwrap();
        app.run("type.orientation.vertical", json!({"layer": ids[0].0})).unwrap();
        app.run("type.antiAlias.crisp", json!({"layer": ids[1].0})).unwrap();
        app.run("type.orientation.horizontal", json!({"layer": ids[1].0})).unwrap();
        let checks = |app: &mut PhotocraftApp| -> Vec<String> {
            let ctx = app.ctx().clone();
            let rows = crate::menus::model(app, &ctx).rows();
            for id in ["type.antiAlias.none", "type.antiAlias.crisp", "type.orientation.horizontal", "type.orientation.vertical"] {
                let it = crate::menus::row(rows, id).unwrap();
                assert!(it.kind.checked().is_some(), "{id} is a check item");
            }
            rows.iter()
                .filter(|i| (i.command.id.starts_with("type.antiAlias.") || i.command.id.starts_with("type.orientation.")) && i.kind.checked() == Some(true))
                .map(|i| i.command.id.to_string())
                .collect()
        };
        editing(&mut app, ids[0]);
        assert_eq!(checks(&mut app), ["type.antiAlias.none", "type.orientation.vertical"]);
        app.ui.text_edit = None;
        app.session.active_mut().unwrap().set_active_layer(Some(ids[1]));
        assert_eq!(checks(&mut app), ["type.antiAlias.crisp", "type.orientation.horizontal"]);
    }

    #[test]
    fn warp_dialog_captures_layer_selection_or_inline_edit_before_confirmation() {
        for inline in [false, true] {
            let (mut app, ids) = fixture();
            if inline {
                editing(&mut app, ids[0]);
            }
            let styles = ids.map(|id| text(&app, id).char_runs());
            let steps = app.session.active().unwrap().history.entries().len();
            let ctx = app.ctx().clone();
            crate::menus::invoke(&mut app, &ctx, "type.warpText", json!({})).unwrap();
            let dialog = app.framework.dialogs.stack().top().expect("the Warp dialog is open");
            let mut h = Harness::builder()
                .with_context(app.ctx().clone())
                .with_size(vec2(900.0, 700.0))
                .build_app(|ui, app: &mut PhotocraftApp| crate::dialogs::run(app, ui), app);
            h.run_steps(4);
            h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Bend");
            assert!(h.query_all_by_label("Bend").count() >= 1, "the actual Warp form is visible");
            for label in ["Layer", "Layers", "Range", "Coalesce"] {
                assert!(h.query_by_label(label).is_none(), "target metadata must not become a form field: {label}");
            }
            dialog_set(h.state_mut(), dialog, "style", json!("arc"));
            dialog_set(h.state_mut(), dialog, "bend", json!(25.0));
            selected(h.state_mut(), &[ids[2]]);
            h.get_by_label("OK").click();
            h.run_steps(3);
            assert!(h.state().framework.dialogs.is_empty());
            for (i, id) in ids.into_iter().enumerate() {
                let t = text(h.state(), id);
                // PLAN: note 12 names `t.session.warp()`; `t` is the layer's `TextLayer` here, so its `warp` is read
                assert_eq!(t.warp.as_ref().map(|w| w.value), (i == 0 || (!inline && i == 1)).then_some(25.0), "inline={inline}, layer={i}");
                assert_eq!(t.char_runs(), styles[i], "Warp leaves character formatting intact");
            }
            assert_eq!(history_len(&h), steps + 1);
            if inline {
                assert_eq!(h.state().session.active().unwrap().coalesce.as_deref(), Some("batch-type-edit"));
            }
        }
    }

    /// The colour well's request captures the selected targets; Cancel edits nothing and OK
    /// recolours the layers selected when the request was made.
    #[test]
    fn text_swatch_captures_selected_targets_and_cancel_does_not_edit() {
        let (app, ids) = fixture();
        let originals = ids.map(|id| style(&app, id, 0));
        let mut h = controls(app, false);
        let steps = history_len(&h);
        for confirm in [false, true] {
            selected(h.state_mut(), &ids[..2]);
            h.run();
            h.get_by_label("Text color").click();
            h.run();
            let dialog = h.state().framework.dialogs.stack().top().expect("the colour picker is open");
            dialog_set(h.state_mut(), dialog, "color", json!("#00ff00"));
            selected(h.state_mut(), &[ids[2]]);
            if confirm {
                dialog_confirm(h.state_mut(), dialog);
            } else {
                dialog_cancel(h.state_mut(), dialog);
            }
            for i in 0..3 {
                let mut expected = originals[i].clone();
                if confirm && i < 2 {
                    expected.color = photocraft_color::Color::rgb(0.0, 1.0, 0.0);
                }
                assert_eq!(style(h.state(), ids[i], 0), expected);
            }
            assert_eq!(history_len(&h), steps + usize::from(confirm));
        }
        assert!(h.state_mut().session.undo());
        for (id, expected) in ids.into_iter().zip(originals) {
            assert_eq!(style(h.state(), id, 0), expected);
        }
    }
}

/// #1132: with the Move tool, double-clicking type on the canvas edits it (Photoshop CC 2019+):
/// the Type tool takes over with the caret on the glyph under the pointer, on whichever type
/// layer was double-clicked, even when another layer is active. Elsewhere it does nothing.
#[test]
fn move_tool_double_click_on_type_edits_it() {
    let mut app = new_app();
    let id = LayerId(app.run("type.create", json!({"text": "HOHOHO", "size": 120, "x": 300, "y": 420})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = crate::state::Tool::Move;
    let mut h = fast_harness(app);
    let p = glyph(&mut h, id, 2, 0.2);
    let steps = history_len(&h);
    double_click(&mut h, p);
    assert_eq!(h.state().ui.tool, crate::state::Tool::Type, "the Type tool takes over");
    assert_eq!(h.state().ui.text_edit.as_ref().map(|e| e.layer), Some(id.0));
    assert_eq!(selection(&h), (2, 2), "the caret sits on the double-clicked glyph");
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(id));
    assert_eq!(history_len(&h), steps, "nothing moved");
    type_text(h.state_mut(), "x");
    assert_eq!(text(h.state(), id).text, "HOxHOHO");
    // Off the text, a double-click stays with the Move tool.
    commit_edit(h.state_mut());
    h.state_mut().ui.tool = crate::state::Tool::Move;
    h.run_steps(2);
    let empty = screen(&mut h, id, -400.0, -300.0);
    double_click(&mut h, empty);
    assert_eq!(h.state().ui.tool, crate::state::Tool::Move);
    assert!(h.state().ui.text_edit.is_none());
}

fn selection_bounds(app: &PhotocraftApp) -> Option<photocraft_geom::Rect> {
    app.session.active().unwrap().doc.selection.as_ref().map(|m| m.content_bounds())
}

/// Click with `tool` at (200, 300), type `text` and commit.
fn type_with(app: &mut PhotocraftApp, tool: crate::state::Tool, text: &str) {
    app.ui.tool = tool;
    drag_create(app, [200.0, 300.0], [200.0, 300.0]);
    type_text(app, text); // replaces the Type tool's selected placeholder
    commit_edit(app);
}

/// #1055: the Type Mask tools type a selection in the shape of the glyphs, the same pixels the
/// Type tool's layer covers, as one history step that adds no layer. Vertical runs down a column.
#[test]
fn type_mask_commits_the_glyphs_as_the_selection_in_one_step() {
    use crate::state::Tool;
    let mut app = new_app();
    type_with(&mut app, Tool::Type, "I");
    let glyphs = app.session.active().unwrap().doc.layers.last().unwrap().surface().unwrap().content_bounds();

    let mut app = new_app();
    let layers = app.session.active().unwrap().doc.layers.len();
    let steps = app.session.active().unwrap().history.entries().len();
    type_with(&mut app, Tool::HorizontalTypeMask, "I");
    assert!(app.ui.text_edit.is_none());
    assert_eq!(selection_bounds(&app), Some(glyphs));
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.len(), layers, "no layer is left behind");
    assert_eq!(st.history.entries().len(), steps + 1);
    assert_eq!(st.history.entries().last().map(String::as_str), Some("Type Mask"));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(selection_bounds(&app), None, "one undo restores the selection");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers);

    let mut app = new_app();
    type_with(&mut app, Tool::VerticalTypeMask, "HELLO");
    let v = selection_bounds(&app).unwrap();
    assert!(v.height() > 2 * v.width(), "{v:?}");
}

/// #1055: Cancel throws a Type Mask session away: the selection and the layers stay as they were.
/// (Escape commits it, as in the reference application: #12070, `mask_escape_tests`.)
#[test]
fn type_mask_cancel_leaves_the_selection_unchanged() {
    let mut app = new_app();
    app.run("select.all", json!({})).unwrap();
    let (before, layers) = (selection_bounds(&app), app.session.active().unwrap().doc.layers.len());
    app.ui.tool = crate::state::Tool::HorizontalTypeMask;
    let mut h = harness(1.0, app);
    let p = canvas_at(h.state(), 200.0, 300.0);
    click(&mut h, p);
    assert!(h.state().ui.text_edit.as_ref().is_some_and(|ed| ed.mask), "typing a mask");
    type_text(h.state_mut(), "I");
    cancel_edit(h.state_mut());
    h.run_steps(2);
    assert!(h.state().ui.text_edit.is_none());
    assert_eq!(selection_bounds(h.state()), before);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers);
}

/// A harness with short frames, so a run of clicks stays inside egui's double-click delay.
fn quick_harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    make_harness_with(Harness::builder().with_step_dt(0.02), 1.0, app)
}

/// `n` quick clicks at `p`, then a pause long enough that the next click starts a new run.
fn clicks(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2, n: usize) -> (usize, usize) {
    h.hover_at(p);
    h.run_steps(1);
    for _ in 0..n {
        press(h, p, true);
        press(h, p, false);
    }
    let sel = selection(h);
    h.run_steps(40);
    sel
}

/// Centre the view on layer `id` at 50 % zoom.
fn centre_on(h: &mut Harness<'static, Box<PhotocraftApp>>, id: LayerId) {
    let (l, aff, _) = layout(h.state_mut(), id).unwrap();
    let b = l.bounds().unwrap();
    let m = aff.apply(Point::new(f64::from(b[0] + b[2]) / 2.0, f64::from(b[1] + b[3]) / 2.0));
    set_view(h, 0.5, [m.x as f32, m.y as f32]);
    h.run_steps(2);
}

/// Laid-out line (character range) holding character `i`.
fn line_chars(h: &mut Harness<'static, Box<PhotocraftApp>>, id: LayerId, i: usize) -> (usize, usize) {
    let (l, _, text) = layout(h.state_mut(), id).unwrap();
    let b = text.char_indices().nth(i).unwrap().0;
    let c = l.clusters.iter().find(|c| c.range.start == b).unwrap();
    let ln = &l.lines[c.line];
    (photocraft_text::char_index(&text, ln.range.start), photocraft_text::char_index(&text, ln.range.end))
}

/// The eight handles of the text frame as (left, right, top, bottom) sides.
const HANDLE_SIDES: [(bool, bool, bool, bool); 8] = [
    (true, false, true, false),
    (false, false, true, false),
    (false, true, true, false),
    (false, true, false, false),
    (false, true, false, true),
    (false, false, false, true),
    (true, false, false, true),
    (true, false, false, false),
];

/// #6819: all eight paragraph-box handles resize the box while editing, with View › Snap on (the
/// default), at 25–200 % zoom, 1× and 2× HiDPI, for a plain, a scaled and a rotated layer. A press
/// on a handle (anywhere in its grab area) never moves the caret, and the edited text's own glyph
/// bounds don't pull the press off the handle.
#[test]
fn every_paragraph_box_handle_resizes_the_box_with_snapping_on() {
    let scaled = Affine { m: [1.5, 0.0, 0.0, 0.75, 300.0, 300.0] };
    let rotated = Affine { m: [1.2 * 0.94, 1.2 * 0.342, -1.2 * 0.342, 1.2 * 0.94, 300.0, 300.0] };
    // Pairwise over (ppp, zoom, transform): each value of each axis appears once or more, and the two
    // cases at the ends of the range in points are kept. The frame's handles and their grab area are
    // screen-space parts of the framework's `TransformBox::text_frame` (`controls.rs`, `frame_box`) and
    // the snap reach is a screen distance, so the zoom and ppp only scale the view: 25 % at 2x is 12.5 %
    // in points and 200 % at 1x is 200 %, which hold 50 % and 100 % between them. The layer transform
    // goes through the same frame mapping at any view scale, so each one runs at one view.
    let cases = [(2.0, 0.25f32, "plain", None), (1.0, 2.0, "scaled", Some(scaled)), (2.0, 2.0, "rotated", Some(rotated))];
    for (case, (ppp, zoom, name, tf)) in cases.into_iter().enumerate() {
        let mut app = new_app();
        assert!(app.ui.extras.snap, "snapping is on by default");
        let size = 24.0 / zoom;
        let p = json!({"text": "HOHOHO HOHO HOHOHO HOHO HOHOHO", "size": size, "x": 300, "y": 300, "box": [300.0, 300.0, size * 8.0, size * 4.0]});
        let id = LayerId(app.run("type.create", p).unwrap()["layer"].as_u64().unwrap());
        if let Some(a) = tf {
            app.run("type.edit", json!({"layer": id.0, "transform": a.m})).unwrap();
        }
        let mut h = harness(ppp, app);
        let (bx, by, w, bh) = super::box_shape(h.state(), id).unwrap();
        let aff = layout(h.state_mut(), id).unwrap().1;
        let m = aff.apply(Point::new(f64::from(bx + w / 2.0), f64::from(by + bh / 2.0)));
        set_view(&mut h, zoom, [m.x as f32, m.y as f32]);
        h.run_steps(2);
        let ctx = format!("ppp {ppp} zoom {zoom} {name}");
        let p = glyph(&mut h, id, 2, 0.2);
        click(&mut h, p);
        assert_eq!(selection(&h), (2, 2), "{ctx}: editing");
        for (i, (l, r, t, b)) in HANDLE_SIDES.into_iter().enumerate() {
            // Press at the handle or inside its grab area, towards the text: corners (even `i`) and sides
            // take turns, and the next case swaps them, so every handle is pressed both ways and every case
            // presses both ways. The press picks the handle (`TransformBox::text_frame`) and the pass that
            // commits carries the final box (`box_of`), so the drag needs the move that starts it and the end.
            let off = if (i + case) % 2 == 0 { 0.0f32 } else { 4.0 };
            let (bx, by, w, bh) = super::box_shape(h.state(), id).unwrap();
            let hx = if l {
                bx
            } else if r {
                bx + w
            } else {
                bx + w / 2.0
            };
            let hy = if t {
                by
            } else if b {
                by + bh
            } else {
                by + bh / 2.0
            };
            let dir = vec2(f32::from(u8::from(r)) - f32::from(u8::from(l)), f32::from(u8::from(b)) - f32::from(u8::from(t)));
            let at = screen(&mut h, id, hx, hy);
            let out = (screen(&mut h, id, hx + dir.x * 10.0, hy + dir.y * 10.0) - at).normalized();
            let a = at - out * off;
            drag_in(&mut h, a, a + out * 30.0, 2);
            let ctx = format!("{ctx} handle {i} offset {off}");
            assert_eq!(selection(&h), (2, 2), "{ctx}: the caret stays put");
            let (_, _, w2, h2) = super::box_shape(h.state(), id).unwrap();
            // 30 points outward (a corner's diagonal: about 21 per side) is that over
            // zoom × layer scale in text units; snapping may shift the release by up to
            // its 8 point reach.
            let min = 6.0 / (zoom / h.state().canvas_ppp()) / 1.5;
            let grew = |was: f32, now: f32, moved: bool| if moved { now - was >= min } else { (now - was).abs() < 1e-3 };
            assert!(grew(w, w2, l || r), "{ctx}: width {w} -> {w2}");
            assert!(grew(bh, h2, t || b), "{ctx}: height {bh} -> {h2}");
        }
    }
}

/// Paragraph text: three clicks select only the visual (soft-wrapped) line, four the whole
/// paragraph; vertical type selects the column the same way (`LayerHost::line_range`).
#[test]
fn multi_click_separates_wrapped_lines_from_paragraphs() {
    let mut app = new_app();
    app.ui.extras.snap = false;
    let text = "one two three four five six\nseven";
    let id = LayerId(app.run("type.create", json!({"text": text, "size": 40, "box": [200, 150, 260, 600]})).unwrap()["layer"].as_u64().unwrap());
    let mut h = quick_harness(app);
    centre_on(&mut h, id);
    // Glyph 9 is inside "three"; the first paragraph wraps, so its line is only part of it.
    let line = line_chars(&mut h, id, 9);
    assert!(line.0 > 0 || line.1 < 27, "the paragraph wraps: line {line:?}");
    let p = glyph(&mut h, id, 9, 0.5);
    assert_eq!(clicks(&mut h, p, 2), (8, 13), "two clicks: the word \"three\"");
    assert_eq!(clicks(&mut h, p, 3), line, "three clicks: the visual line");
    assert_eq!(clicks(&mut h, p, 4), (0, 27), "four clicks: the wrapped paragraph");
    let p = glyph(&mut h, id, 30, 0.5);
    assert_eq!(clicks(&mut h, p, 4), (28, 33), "four clicks in the last paragraph");

    let mut app = new_app();
    app.ui.extras.snap = false;
    let id = LayerId(app.run("type.create", json!({"text": "HOHOHO\nHOHO", "size": 60, "x": 800, "y": 120})).unwrap()["layer"].as_u64().unwrap());
    app.run("type.orientation.vertical", json!({"layer": id.0})).unwrap();
    let mut h = quick_harness(app);
    centre_on(&mut h, id);
    let p = vglyph(&mut h, id, 9, 0.5);
    assert_eq!(clicks(&mut h, p, 3), (7, 11), "three clicks in vertical type: the column");
    assert_eq!(clicks(&mut h, p, 5), (0, 11), "five clicks: all text");
}

/// #3036: while typing, the working text has no Layers panel row. ⇧ or ⌥ on the click that
/// commits a Type Mask no longer combine: the glyphs replace the selection (#12073,
/// `mask_replace_selection_tests`).
#[test]
fn type_mask_shift_and_alt_replace_without_a_layer_row() {
    use crate::state::Tool;
    let mut app = new_app();
    type_with(&mut app, Tool::HorizontalTypeMask, "I");
    let glyphs = selection_bounds(&app).unwrap();
    let mut app = new_app();
    app.ui.extras.snap = false; // the second text would snap to the first one's selected edge
    app.run("select.rect", json!({"x": 1000, "y": 100, "width": 100, "height": 100})).unwrap();
    // Type, then commit with a click outside the text holding `m`.
    let type_and_commit = |app: &mut PhotocraftApp, m: Modifiers| {
        app.ui.tool = Tool::HorizontalTypeMask;
        drag_create(app, [200.0, 300.0], [200.0, 300.0]);
        type_text(app, "I");
        let doc = app.session.active().unwrap().doc.clone();
        let st = app.session.active().unwrap();
        let tree = crate::panels::layers_tree::PhotoLayers::new(app, st, &Default::default(), false, None);
        let rows = craft::layers::ui::tree::LayerTree::children(&tree, None);
        assert_eq!(rows.len(), doc.layers.len() - 1, "the working text is not listed");
        tool_event(app, ToolEvent::Down { x: 1500.0, y: 800.0, pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Up { x: 1500.0, y: 800.0 }, m);
        assert!(app.ui.text_edit.is_none());
    };
    type_and_commit(&mut app, Modifiers::SHIFT);
    assert_eq!(selection_bounds(&app), Some(glyphs), "⇧ replaces");
    type_and_commit(&mut app, Modifiers::ALT);
    assert_eq!(selection_bounds(&app), Some(glyphs), "⌥ replaces");
}

/// #7713: justification needs a paragraph box. On point text the Paragraph panel's four justify
/// buttons are greyed (`values::point_text`), as in Photoshop; on paragraph text and area text
/// they work; a selection mixing point and paragraph text greys them.
#[test]
fn justify_buttons_are_greyed_on_point_text() {
    use egui_kittest::kittest::NodeT;
    let justify = ["Justify last left", "Justify last centered", "Justify last right", "Justify all"];
    let area = |app: &mut PhotocraftApp| {
        app.run("type.create", json!({"text": "one two three four five six", "size": 24, "x": 300, "y": 420})).unwrap();
        std::sync::Arc::make_mut(app.session.active_mut().unwrap().doc_mut()).work_path =
            Some(photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[(300.0, 300.0), (600.0, 300.0), (600.0, 500.0), (300.0, 500.0)])]));
        app.run("type.fillPath", json!({})).unwrap();
    };
    for (kind, enabled) in [("point", false), ("paragraph", true), ("area", true)] {
        let mut app = new_app();
        match kind {
            "area" => area(&mut app),
            _ => {
                let mut p = json!({"text": "one two three four", "size": 24, "x": 300, "y": 420});
                if kind == "paragraph" {
                    p["box"] = json!([300, 300, 240, 180]);
                }
                app.run("type.create", p).unwrap();
            }
        }
        let h = panel_harness(app);
        for tip in justify {
            assert_eq!(!h.get_by_label(tip).accesskit_node().is_disabled(), enabled, "{tip} on {kind} text");
        }
        assert!(!h.get_by_label("Center text").accesskit_node().is_disabled(), "alignment itself stays available");
    }
    // A selection of a paragraph and a point type layer formats both: justify is greyed.
    let mut app = new_app();
    let point = app.run("type.create", json!({"text": "point", "size": 24, "x": 300, "y": 420})).unwrap()["layer"].as_u64().unwrap();
    let boxed =
        app.run("type.create", json!({"text": "boxed text", "size": 24, "x": 300, "y": 300, "box": [300, 300, 240, 180]})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.select", json!({"layer": point})).unwrap();
    app.run("layer.select", json!({"layer": boxed, "mode": "add"})).unwrap();
    assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
    let h = panel_harness(app);
    assert!(h.get_by_label("Justify all").accesskit_node().is_disabled(), "one of the formatted layers is point text");
}

/// A box that can't hold its text shows the overflow handle (the frame's `overflow` plus):
/// `ui.type_box_overflow` is the automation-visible fact. A fitting box (trailing breaks
/// included) and point text — which has no box — never set it.
#[test]
fn the_overflow_handle_flags_a_box_that_cannot_hold_its_text() {
    let build = |mut app: Box<PhotocraftApp>, p: serde_json::Value, caret: usize| {
        let id = app.run("type.create", p).unwrap()["layer"].as_u64().unwrap();
        app.session.select_layer(LayerId(id)).unwrap();
        app.ui.text_edit = Some(edit_of(id, caret, caret, "s"));
        let mut h = harness(1.0, app);
        h.run_steps(4);
        h
    };
    // In the harness's window a 40 pt box holds 1.5 lines, so the second line is dropped.
    let mut h = build(new_app(), json!({"text": "ab\ncd\nef", "size": 24, "box": [200, 150, 260, 40]}), 0);
    assert!(h.state().ui.type_box_overflow, "a box shorter than the text");
    // The same text in a tall box fits: nothing overflows.
    h = build(new_app(), json!({"text": "ab cdef gh", "size": 24, "box": [200, 150, 260, 600]}), 0);
    assert!(!h.state().ui.type_box_overflow, "a fitting box");
    // Point text has no box: much text is fine.
    h = build(new_app(), json!({"text": "ab cdef gh ab cdef gh ab cdef gh", "size": 24, "x": 200, "y": 200}), 0);
    assert!(!h.state().ui.type_box_overflow, "point text");
}

/// `ui.inspect` reports the overflow handle only while an edit session is open: the flag is a
/// per-frame snapshot, and a commit or cancel can end the session before the frame that resets
/// the flag — an agent would otherwise read `typeBoxOverflow: true` beside a null `textEdit`.
#[test]
fn inspect_gates_the_overflow_flag_on_the_session() {
    let mut a = new_app();
    let id = a.run("type.create", json!({"text": "ab\n.cd", "size": 24, "box": [200, 150, 260, 40]})).unwrap()["layer"].as_u64().unwrap();
    a.session.select_layer(LayerId(id)).unwrap();
    a.ui.text_edit = Some(edit_of(id, 0, 0, "s"));
    // Snapshot state as the canvas controls wrote it while the session was still open.
    a.ui.type_box_overflow = true;
    let ctx = a.ctx().clone();
    assert_eq!(crate::control::inspect(&a, &ctx)["typeBoxOverflow"], json!(true), "an open session reports the snapshot");
    // A commit (or cancel) between frames: `textEdit` is gone, so the flag must read as off.
    a.ui.text_edit = None;
    assert_eq!(crate::control::inspect(&a, &ctx)["typeBoxOverflow"], json!(false), "no session, no overflow");
    assert_eq!(crate::control::inspect(&a, &ctx)["textEdit"], json!(null), "the session is closed either way");
}

fn ellipse_layer(app: &mut PhotocraftApp) -> u64 {
    app.run("shape.create", json!({"kind": "ellipse", "rect": [200, 200, 400, 300], "fill": "#dddddd"})).unwrap()["layer"].as_u64().unwrap()
}

fn click_at(app: &mut PhotocraftApp, x: f64, y: f64) {
    drag_create(app, [x, y], [x, y]);
}

#[test]
fn clicking_inside_a_shape_starts_area_type_in_one_step() {
    use photocraft_doc::text::TextShape;
    let mut app = new_app();
    let shape = ellipse_layer(&mut app);
    assert!(matches!(path_target(&app, 400.0, 350.0), Some(PathClick::Area { shape: Some(s) }) if s == shape));
    let before = app.session.active().unwrap().history.entries().len();
    click_at(&mut app, 400.0, 350.0);
    let id = LayerId(app.ui.text_edit.as_ref().expect("editing").layer);
    let st = app.session.active().unwrap();
    assert!(matches!(text_layer(&st.doc, id).unwrap().shape, TextShape::Area { .. }));
    assert_eq!(st.history.entries().len(), before + 1, "create + fill is one step");
    commit_edit(&mut app);
    app.run("edit.undo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.layer(id).is_none());
}

#[test]
fn clicking_on_a_path_outline_starts_type_on_the_path_where_clicked() {
    use photocraft_doc::text::TextShape;
    let mut app = new_app();
    ellipse_layer(&mut app);
    // The ellipse's bottom point (400, 500); its perimeter is about 1105 px.
    let Some(PathClick::OnPath { start, .. }) = path_target(&app, 400.0, 500.0) else { panic!("on the outline") };
    assert!((0.0..1110.0).contains(&start), "{start}");
    // A point a quarter of the way further round is about 276 px further along (mod the perimeter).
    let Some(PathClick::OnPath { start: right, .. }) = path_target(&app, 600.0, 350.0) else { panic!("on the outline") };
    let d = (right - start).rem_euclid(1105.0);
    assert!((d - 276.0).abs() < 15.0 || (d - 829.0).abs() < 15.0, "{start} → {right}");
    let before = app.session.active().unwrap().history.entries().len();
    click_at(&mut app, 400.0, 500.0);
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    assert_eq!(app.session.active().unwrap().history.entries().len(), before + 1, "create + attach is one step");
    assert!(
        matches!(text_layer(&app.session.active().unwrap().doc, id).unwrap().shape, TextShape::Path { start: s, .. } if (f64::from(s) - start).abs() < 0.01)
    );
}

/// The path pointer shows what the release would do: kept while a press on a path is held or
/// barely moves (the release still makes path type there), none once a text box is dragged out
/// over a path (the release makes a box) or while a type edit is open (a click commits). The
/// pointer is the cursor the tool host reports for the pass.
// PLAN: assumed the host's `Cursor` is `Debug + PartialEq`
#[test]
fn the_path_pointer_is_off_while_dragging_a_text_box_or_editing() {
    let mut app = new_app();
    ellipse_layer(&mut app);
    let hover = |app: &mut PhotocraftApp, x: f64, y: f64| tool_event_response(app, ToolEvent::Move { x, y, pressure: 1.0 }, Modifiers::NONE).cursor;
    let path_pointer = hover(&mut app, 400.0, 350.0);
    assert!(path_pointer.is_some(), "a hover inside the shape shows a pointer");
    // Press and hold on the shape, then a flat drag: still a click on release.
    let pressed = tool_event_response(&mut app, ToolEvent::Down { x: 400.0, y: 350.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(crate::canvas::testing::in_gesture(&app), "the press starts a drag");
    assert_eq!(pressed.cursor, path_pointer, "press and hold");
    let flat = tool_event_response(&mut app, ToolEvent::Move { x: 1200.0, y: 351.0, pressure: 1.0 }, Modifiers::NONE);
    assert_eq!(flat.cursor, path_pointer, "a flat drag");
    tool_event(&mut app, ToolEvent::Up { x: 1200.0, y: 351.0 }, Modifiers::NONE);
    let id = LayerId(app.ui.text_edit.as_ref().expect("editing").layer);
    assert!(
        matches!(text_layer(&app.session.active().unwrap().doc, id).unwrap().shape, photocraft_doc::text::TextShape::Area { .. }),
        "the release made area type"
    );
    let mut app = new_app();
    ellipse_layer(&mut app);
    tool_event(&mut app, ToolEvent::Down { x: 300.0, y: 300.0, pressure: 1.0 }, Modifiers::NONE);
    let dragged = tool_event_response(&mut app, ToolEvent::Move { x: 400.0, y: 350.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(crate::canvas::testing::in_gesture(&app), "dragging out a box from inside the shape");
    assert_ne!(dragged.cursor, path_pointer, "the drag makes a box, not area type");
    tool_event(&mut app, ToolEvent::Up { x: 400.0, y: 350.0 }, Modifiers::NONE);
    assert!(app.ui.text_edit.is_some(), "the box is being edited");
    assert_ne!(hover(&mut app, 400.0, 350.0), path_pointer, "a click now commits");
}

#[test]
fn click_outside_paths_still_makes_point_text() {
    use photocraft_doc::text::TextShape;
    let mut app = new_app();
    ellipse_layer(&mut app);
    assert!(path_target(&app, 1200.0, 700.0).is_none());
    click_at(&mut app, 1200.0, 700.0);
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    assert_eq!(text_layer(&app.session.active().unwrap().doc, id).unwrap().shape, TextShape::Point);
    // A drag inside the shape still makes a text box, as in Photoshop.
    let mut app = new_app();
    ellipse_layer(&mut app);
    drag_create(&mut app, [300.0, 300.0], [450.0, 400.0]);
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    assert!(matches!(text_layer(&app.session.active().unwrap().doc, id).unwrap().shape, TextShape::Box { .. }));
}

#[test]
fn type_mask_tools_ignore_paths() {
    let mut app = new_app();
    ellipse_layer(&mut app);
    app.ui.tool = crate::state::Tool::HorizontalTypeMask;
    click_at(&mut app, 400.0, 350.0);
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    assert_eq!(text_layer(&app.session.active().unwrap().doc, id).unwrap().shape, photocraft_doc::text::TextShape::Point);
}

#[test]
fn over_existing_type_on_a_shape_the_pointer_is_the_plain_i_beam_and_a_click_edits_it() {
    let mut app = new_app();
    let text = app.run("type.create", json!({"text": "Hello", "x": 330, "y": 370, "size": 36})).unwrap()["layer"].as_u64().unwrap();
    let shape = ellipse_layer(&mut app); // the shape layer is active, so its path is the target
    let layers = app.session.active().unwrap().doc.walk().len();
    let mut h = harness(1.0, app);
    let on_text = glyph(&mut h, LayerId(text), 1, 0.5);
    let d = crate::canvas::testing::view(h.state()).expect("a canvas view").to_doc(on_text);
    assert!(matches!(path_target(h.state(), d.x, d.y), Some(PathClick::Area { shape: Some(s) }) if s == shape), "the text sits inside the shape");
    h.hover_at(on_text);
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Text, "a click here edits the type, so the plain I-beam");
    // Inside the shape but away from the type: the area-type pointer (drawn, not the OS I-beam).
    let inside = canvas_at(h.state(), 400.0, 250.0);
    h.hover_at(inside);
    h.run_steps(2);
    assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::Text, "area-type pointer inside the shape");
    click(&mut h, on_text);
    let app = h.state();
    assert_eq!(app.ui.text_edit.as_ref().expect("editing").layer, text, "the click edits the existing layer");
    assert_eq!(app.session.active().unwrap().doc.walk().len(), layers, "no new layer");
    assert_eq!(text_layer(&app.session.active().unwrap().doc, LayerId(text)).unwrap().shape, photocraft_doc::text::TextShape::Point);
}

#[test]
fn an_unselected_work_path_is_not_an_area_target() {
    // The canvas draws a Work Path only while it is selected in Paths (or a path tool is active,
    // #6551). One left unselected, as after Ctrl-click Load as Selection, is invisible with the
    // Type tool, so a click inside it is plain point text; its outline still takes type on a
    // path, as before area type (#5180).
    let mut app = new_app();
    std::sync::Arc::make_mut(app.session.active_mut().unwrap().doc_mut()).work_path =
        Some(photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[(300.0, 300.0), (700.0, 300.0), (700.0, 500.0), (300.0, 500.0)])]));
    app.ui.selected_path = None;
    assert!(path_target(&app, 500.0, 400.0).is_none(), "inside a hidden work path");
    assert!(matches!(path_target(&app, 500.0, 301.0), Some(PathClick::OnPath { shape: None, .. })));
    click_at(&mut app, 500.0, 400.0);
    let id = LayerId(app.ui.text_edit.as_ref().expect("editing").layer);
    assert_eq!(text_layer(&app.session.active().unwrap().doc, id).unwrap().shape, photocraft_doc::text::TextShape::Point);
    app.ui.selected_path = Some("work".into());
    assert!(matches!(path_target(&app, 500.0, 400.0), Some(PathClick::Area { shape: None })));
}

/// The options bar's alignment buttons show the type layer's paragraph, as Photoshop's bar
/// follows the text, and the tool defaults again without a type target.
#[test]
fn options_bar_alignment_follows_the_target_paragraph() {
    use egui::accesskit::Toggled;
    use egui_kittest::kittest::NodeT;
    let mut app = new_app();
    let id = app.run("type.create", json!({"text": "Middle", "size": 20, "x": 300, "y": 420})).unwrap()["layer"].as_u64().unwrap();
    app.run("type.setStyle", json!({"layer": id, "align": "center", "direction": "rtl"})).unwrap();
    assert_eq!(app.ui.tool_options.type_align, "left", "the tool default stays Left");
    let mut h = bar_harness(app);
    let on = |h: &Harness<'static, Box<PhotocraftApp>>, label: &str| h.get_by_label(label).accesskit_node().toggled() == Some(Toggled::True);
    assert!(on(&h, "Center text") && !on(&h, "Left align text"), "the layer's centred paragraph");
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(2);
    assert!(on(&h, "Left align text") && !on(&h, "Center text"), "no type target: the tool default");
}

/// #9179 review: a text edit left in a background document shows its own paragraph, not that of
/// a same-id layer in the active document (layer ids repeat across documents).
#[test]
fn styles_at_follows_the_text_edit_document() {
    let mut app = new_app();
    let a = app.session.active().unwrap().doc.id.0;
    let id = app.run("type.create", json!({"text": "Edited", "size": 20, "x": 300, "y": 420})).unwrap()["layer"].as_u64().unwrap();
    app.run("type.setStyle", json!({"layer": id, "align": "center"})).unwrap();
    app.ui.text_edit = Some(crate::state::TextEdit { document: Some(a), ..edit_of(id, 2, 2, "s") });
    // The same file opened twice: a second document whose layers have the same ids.
    let twin = app.session.active().unwrap().doc.as_ref().clone();
    app.session.open_document(twin, None);
    app.run("type.setStyle", json!({"layer": id, "align": "right"})).unwrap();
    assert_ne!(app.session.active().unwrap().doc.id.0, a, "the other document is active");
    let (_, p) = super::styles_at(&app).unwrap();
    assert_eq!(p.align, photocraft_doc::text::TextAlign::Center, "the edited document's paragraph");
}

/// A collapsed caret shows the paragraph it is in, as Photoshop's options bar and Paragraph panel
/// do; it read the first paragraph's (byte 0) wherever the caret was.
#[test]
fn styles_at_a_collapsed_caret_shows_the_caret_s_paragraph() {
    let mut app = new_app();
    let id = app.run("type.create", json!({"text": "First\rSecond", "size": 20, "x": 300, "y": 420})).unwrap()["layer"].as_u64().unwrap();
    app.run("type.setStyle", json!({"layer": id, "range": [6, 12], "align": "right"})).unwrap();
    use photocraft_doc::text::TextAlign;
    for (caret, want) in
        [(0, TextAlign::Left), (3, TextAlign::Left), (5, TextAlign::Left), (6, TextAlign::Right), (9, TextAlign::Right), (12, TextAlign::Right)]
    {
        app.ui.text_edit = Some(edit_of(id, caret, caret, "s"));
        let (_, p) = super::styles_at(&app).unwrap();
        assert_eq!(p.align, want, "caret {caret}");
    }
}
