//! #9938: as in Photoshop, a character setting changed in the options bar or the Character panel
//! becomes the Type tool's setting for the next new type, whether it formatted a selected type
//! layer, the characters being edited, or nothing. Selecting a layer only shows its settings: it
//! leaves the defaults alone.

use egui::accesskit::Role;
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::text::{Caps, CharStyle, Kerning, TextAlign};
use photocraft_doc::{LayerId, TextLayer};
use serde_json::json;

use super::text_layer;
use crate::PhotocraftApp;

fn new_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1600, "height": 900})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Type;
    app
}

fn text(app: &PhotocraftApp, id: LayerId) -> TextLayer {
    text_layer(&app.session.active().unwrap().doc, id).unwrap().clone()
}

/// A selected type layer far from where [`new_type`] clicks.
fn selected_layer(app: &mut PhotocraftApp, props: serde_json::Value) -> LayerId {
    let mut p = json!({"text": "Dogs", "size": 24, "x": 1200, "y": 700, "color": "#000000"});
    for (k, v) in props.as_object().unwrap() {
        p[k] = v.clone();
    }
    let id = LayerId(app.run("type.create", p).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
    id
}

/// Characters `anchor..caret` of `id` selected in an inline edit.
fn editing(app: &mut PhotocraftApp, id: LayerId, anchor: usize, caret: usize) {
    app.ui.text_edit = Some(crate::state::TextEdit {
        layer: id.0,
        document: None,
        caret,
        anchor,
        session: "type-defaults-edit".into(),
        created: false,
        mask: false,
        dragging: false,
        resize: None,
        preedit: None,
    });
}

/// Click empty canvas with the Type tool, ending any edit first; the new layer's first run style.
fn new_type(app: &mut PhotocraftApp) -> (CharStyle, TextLayer) {
    if app.ui.text_edit.is_some() {
        super::commit(app);
    }
    super::pointer_up(app, [100.0, 100.0], [100.0, 100.0]);
    let t = text(app, LayerId(app.ui.text_edit.as_ref().unwrap().layer));
    (t.char_runs()[0].style.clone(), t)
}

/// The Character and Paragraph panels, under the options bar when `bar`.
fn controls(app: PhotocraftApp, bar: bool) -> Harness<'static, PhotocraftApp> {
    // Wait for the font scan: while it runs the panels repaint every 100 ms and `Harness::run` gives up.
    let _ = photocraft_text::shared();
    let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).with_max_steps(8).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                if bar {
                    ui.horizontal(|ui| super::options_bar(app, ui));
                }
                ui.columns(2, |columns| {
                    super::character_panel(app, &mut columns[0], false);
                    super::character_panel(app, &mut columns[1], true);
                });
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn number(h: &mut Harness<'static, PhotocraftApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::SpinButton, label).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(value.into()));
    h.key_press(egui::Key::Enter);
    h.run();
}

fn assert_panel_settings(c: &CharStyle, what: &str) {
    assert_eq!(c.size_pt, 36.0, "{what}: size");
    assert_eq!(c.tracking, 80.0, "{what}: tracking");
    assert_eq!(c.leading_pt, Some(44.0), "{what}: leading");
    assert_eq!((c.vertical_scale, c.horizontal_scale), (1.1, 0.9), "{what}: scale");
    assert_eq!(c.baseline_shift_pt, 3.0, "{what}: baseline shift");
    assert!(c.faux_bold && c.faux_italic && c.underline && c.strikethrough, "{what}: faux styles");
    assert_eq!(c.caps, Caps::AllCaps, "{what}: caps");
    assert!(c.discretionary_ligatures && photocraft_engine::type_extra_cmds::feature_on(c, "frac"), "{what}: OpenType");
}

/// With a type layer selected (not being edited), the Character panel formats it and the
/// settings carry over to the next new type.
#[test]
fn character_panel_changes_on_a_selected_type_layer_become_the_defaults() {
    let mut app = new_app();
    let id = selected_layer(&mut app, json!({}));
    let mut h = controls(app, false);
    number(&mut h, "Font size", "36");
    number(&mut h, "Tracking (1/1000 em)", "80");
    number(&mut h, "Leading (set to the font size × auto-leading when Auto)", "44");
    number(&mut h, "Vertical scale", "110");
    number(&mut h, "Horizontal scale", "90");
    number(&mut h, "Baseline shift", "3");
    for label in ["Faux Bold", "Faux Italic", "All Caps", "Underline", "Strikethrough", "Fractions", "Discretionary Ligatures"] {
        h.get_by_label(label).click();
        h.run();
    }
    assert_panel_settings(&text(h.state(), id).char_runs()[0].style, "the selected layer");
    let (c, _) = new_type(h.state_mut());
    assert_panel_settings(&c, "the next new type");
}

/// The same settings changed for selected characters while typing: only those characters change,
/// and the next new type gets them.
#[test]
fn changes_to_selected_characters_while_editing_become_the_defaults() {
    let mut app = new_app();
    let ctx = egui::Context::default();
    let id = selected_layer(&mut app, json!({}));
    editing(&mut app, id, 1, 3);
    for props in [
        json!({"fontStyle": "Bold"}),
        json!({"size": 36, "metricsAsShown": true}),
        json!({"leading": 44, "metricsAsShown": true}),
        json!({"tracking": 80}),
        json!({"kerning": "optical"}),
        json!({"verticalScale": 110}),
        json!({"horizontalScale": 90}),
        json!({"baselineShift": 3}),
        json!({"fauxBold": true}),
        json!({"fauxItalic": true}),
        json!({"caps": "smallCaps"}),
        json!({"underline": true}),
        json!({"variations": {"wdth": 90}}),
    ] {
        super::apply(&mut app, &ctx, props);
    }
    super::toggle_opentype(&mut app, "type.openType.fractions", false);
    let runs = text(&app, id).char_runs();
    assert_eq!(runs.iter().map(|r| (r.len, r.style.tracking)).collect::<Vec<_>>(), [(1, 0.0), (2, 80.0), (1, 0.0)], "only the selection changes");
    let (c, _) = new_type(&mut app);
    assert_eq!((c.font_style.as_str(), c.size_pt, c.leading_pt, c.tracking), ("Bold", 36.0, Some(44.0), 80.0));
    assert_eq!((c.kerning, c.vertical_scale, c.horizontal_scale, c.baseline_shift_pt), (Kerning::Optical, 1.1, 0.9, 3.0));
    assert!(c.faux_bold && c.faux_italic && c.underline);
    assert_eq!(c.caps, Caps::SmallCaps);
    assert!(c.variations.iter().any(|v| (v.axis.as_str(), v.value) == ("wdth", 90.0)));
    assert!(photocraft_engine::type_extra_cmds::feature_on(&c, "frac"));
}

/// Paragraph settings stay with the layer they format: the tool's paragraph defaults come from
/// the options bar and from the panels without a type target (TYPE-218-2).
#[test]
fn paragraph_changes_on_a_type_target_leave_the_paragraph_defaults() {
    let mut app = new_app();
    let ctx = egui::Context::default();
    let id = selected_layer(&mut app, json!({}));
    let options = app.ui.tool_options.clone();
    super::apply(&mut app, &ctx, json!({"startIndent": 9, "direction": "rtl", "spaceAfter": 4}));
    assert_eq!(text(&app, id).paragraph_runs()[0].style.start_indent_pt, 9.0);
    assert_eq!(app.ui.tool_options, options);
}

/// The text colour picked for a selected layer or selected characters is the next type's colour,
/// without changing the foreground colour; a later foreground colour is new type's again.
#[test]
fn a_text_color_picked_for_a_type_target_becomes_the_default() {
    for edit in [false, true] {
        let mut app = new_app();
        let id = selected_layer(&mut app, json!({}));
        if edit {
            editing(&mut app, id, 0, 2);
        }
        let (revision, foreground) = (app.session.active().unwrap().revision, app.session.tools.foreground);
        let dialog = super::open_color_picker(&mut app, [0.0; 3]);
        app.ui.dialogs.last_mut().unwrap().fields.insert("color".into(), json!("#336699"));
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert!(app.session.active().unwrap().revision > revision, "editing: {edit}");
        assert_eq!(text(&app, id).char_runs()[0].style.color.to_rgba8(), [51, 102, 153, 255], "editing: {edit}");
        assert_eq!(app.session.tools.foreground, foreground, "editing: {edit}");
        let saved: crate::state::ToolOptions = serde_json::from_value(serde_json::to_value(&app.ui.tool_options).unwrap()).unwrap();
        assert_eq!(saved, app.ui.tool_options, "the picked colour is saved with the tool options");
        let (c, _) = new_type(&mut app);
        assert_eq!(c.color.to_rgba8(), [51, 102, 153, 255], "editing: {edit}");
        app.run("tools.setColors", json!({"foreground": "#ff8800"})).unwrap();
        assert_eq!(super::default_style_params(&app)["color"], "#ff8800", "a new foreground colour is new type's again");
    }
}

/// Selecting a styled layer shows its settings in the options bar and the panels; the next new
/// type still gets the defaults the user set, not the layer's.
#[test]
fn selecting_a_type_layer_does_not_change_the_defaults() {
    let mut app = new_app();
    let id = selected_layer(&mut app, json!({}));
    app.run(
        "type.setStyle",
        json!({"layer": id.0, "fontStyle": "Bold", "size": 99, "tracking": 300, "leading": 120, "fauxBold": true, "caps": "allCaps",
               "baselineShift": 7, "verticalScale": 150, "color": "#ff0000", "align": "right"}),
    )
    .unwrap();
    app.run("type.antiAlias.smooth", json!({"layers": [id.0]})).unwrap();
    let (options, foreground) = (app.ui.tool_options.clone(), app.session.tools.foreground);
    let mut h = controls(app, true);
    h.run_steps(4);
    assert_eq!(h.state().ui.tool_options, options);
    assert_eq!(h.state().session.tools.foreground, foreground);
    let (c, t) = new_type(h.state_mut());
    assert_eq!((c.size_pt, c.tracking, c.leading_pt, c.baseline_shift_pt, c.vertical_scale), (options.type_size, 0.0, None, 0.0, 1.0));
    assert!(!c.faux_bold && c.caps == Caps::Normal && c.font_style == options.type_style);
    assert_eq!(t.paragraph_runs()[0].style.align, TextAlign::Left);
    assert_ne!(c.color.to_rgba8(), [255, 0, 0, 255]);
    assert_eq!(photocraft_engine::type_extra_cmds::aa_name(t.antialias), options.type_aa);
}

/// New type gets the options bar's anti-aliasing method, Sharp included: the engine creates
/// Smooth type, and Sharp (the tool's default, as in Photoshop) used to be skipped.
#[test]
fn new_type_takes_the_tools_anti_aliasing_method() {
    for aa in ["sharp", "crisp", "smooth", "none"] {
        let mut app = new_app();
        app.ui.tool_options.type_aa = aa.into();
        let (_, t) = new_type(&mut app);
        assert_eq!(photocraft_engine::type_extra_cmds::aa_name(t.antialias), aa);
    }
}

/// A style chosen at a collapsed caret is for the letters typed next there, and new type's too.
#[test]
fn a_style_chosen_at_a_caret_becomes_the_default() {
    let mut app = new_app();
    let ctx = egui::Context::default();
    let id = selected_layer(&mut app, json!({}));
    let before = text(&app, id).char_runs();
    editing(&mut app, id, 2, 2);
    super::apply(&mut app, &ctx, json!({"fontStyle": "Bold"}));
    assert!(app.ui.type_insertion_style.is_some(), "the caret keeps it for the next letters");
    assert_eq!(text(&app, id).char_runs(), before, "existing letters are unchanged");
    assert_eq!(app.ui.tool_options.type_style, "Bold");
}
