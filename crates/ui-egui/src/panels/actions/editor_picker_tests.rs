//! CPU egui interaction/layout checks; no GPU or image-golden qualification.
use egui::{Color32, Id, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};

use super::{Choice, show, thumbnail_style};
use crate::{
    PhotocraftApp,
    theme::{ThemeKind, Tokens},
};

const UNKNOWN_ICON: &str = " future-icon-图标 ";
const UNKNOWN_COLOR: &str = "@atn-color:65535";

struct State {
    icon: String,
    color: String,
    choices: Vec<Choice>,
}

// Synthetic indexed sentinels, not a proposed application/reference palette.
fn swatches() -> [Color32; 12] {
    std::array::from_fn(|index| Color32::from_rgb(index as u8, 31, 47))
}

fn harness(icon: &str, color: &str) -> Harness<'static, State> {
    let state = State { icon: icon.into(), color: color.into(), choices: Vec::new() };
    let mut h = Harness::builder().with_size(egui::vec2(600.0, 700.0)).build_ui_state(
        |ui, state: &mut State| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            if let Some(choice) = show(ui, Id::unique("action-editor-picker-test"), &state.icon, &state.color, &swatches()) {
                state.choices.push(choice);
            }
        },
        state,
    );
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::Studio);
    h.run_steps(3);
    h
}

fn open(h: &mut Harness<'_, State>) {
    assert!(!egui::Popup::is_any_open(&h.ctx));
    h.get_by_role_and_label(Role::Button, "Choose icon").click();
    h.run_steps(3);
}

fn choose(h: &mut Harness<'_, State>, label: &str, expected: Choice) {
    let before = h.state().choices.len();
    h.get_by_role_and_label(Role::Button, label).click();
    h.run_steps(3);
    assert_eq!(h.state().choices.len(), before + 1, "one deliberate choice for {label}");
    assert_eq!(h.state().choices.last(), Some(&expected));
    assert!(!egui::Popup::is_any_open(&h.ctx));
}

fn grid(h: &Harness<'_, State>, labels: &[&str], columns: usize) {
    let positions: Vec<_> = labels.iter().map(|label| h.get_by_role_and_label(Role::Button, label).rect().center()).collect();
    for row in positions.chunks(columns) {
        assert!(row.iter().all(|point| (point.y - row[0].y).abs() < 0.5));
        assert!(row.windows(2).all(|pair| pair[0].x < pair[1].x));
    }
    for rows in positions.chunks(columns).collect::<Vec<_>>().windows(2) {
        assert!(rows[0][0].y < rows[1][0].y);
        assert!((rows[0][0].x - rows[1][0].x).abs() < 0.5);
    }
}

#[test]
fn idle_and_dismissed_picker_preserve_unknown_metadata_without_a_choice() {
    let mut h = harness(UNKNOWN_ICON, UNKNOWN_COLOR);
    h.run_steps(3);
    assert!(h.state().choices.is_empty());
    open(&mut h);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!egui::Popup::is_any_open(&h.ctx));
    assert!(h.state().choices.is_empty());
    assert_eq!(h.state().icon, UNKNOWN_ICON);
    assert_eq!(h.state().color, UNKNOWN_COLOR);
}

#[test]
fn default_icon_is_an_explicit_choice_distinct_from_click_action() {
    let mut h = harness(UNKNOWN_ICON, "blue2");
    open(&mut h);
    choose(&mut h, "Default icon", Choice::Icon(""));
    assert_eq!(h.state().icon, UNKNOWN_ICON, "the caller still owns the raw value");
    h.state_mut().icon.clear(); // The caller acknowledges the deliberate default selection.
    open(&mut h);
    choose(&mut h, "Click action", Choice::Icon("ClickAction"));
    assert!(h.state().icon.is_empty());
    assert_eq!(h.state().color, "blue2");
}

#[test]
fn six_by_five_icon_grid_returns_the_measured_identifier_for_each_button() {
    for (_, asset) in super::GLYPHS {
        assert!(crate::icons::exists(asset), "the real consumer must embed {asset}; fallback squares are not glyph registration");
    }
    // The issue's measured row order paired with human-readable button labels.
    let expected = [
        ("ClickAction", "Click action"),
        ("ActionsCrop", "Crop"),
        ("LayerMerge", "Merge layers"),
        ("AdjustmentGeneral", "Adjustments"),
        ("HueAndSat", "Hue and saturation"),
        ("ActionsExposure", "Exposure"),
        ("ColorLookUpTables", "Color lookup tables"),
        ("BlackAndWhite", "Black and white"),
        ("SmartRemove", "Smart removal"),
        ("ActionsDodge", "Dodge"),
        ("ActionsBurn", "Burn"),
        ("ActionsErase", "Erase"),
        ("LayerEffects", "Layer effects"),
        ("AIGenEffects", "Generated layer effects"),
        ("Sharpen", "Sharpen"),
        ("ObjectSelectionLasso", "Object selection"),
        ("MagicWand", "Magic wand"),
        ("AIGenerate", "Generate"),
        ("TextColor", "Text color"),
        ("ActionsText", "Text"),
        ("ActionsDownload", "Download"),
        ("ActionsLayers", "Layers"),
        ("Brush", "Brush"),
        ("SelectSubject", "Select subject"),
        ("RemoveBackground", "Remove background"),
        ("GaussianBlur", "Blur"),
        ("LayerHarmonize", "Harmonize layers"),
        ("P_AdjustClarity_32x32", "Clarity"),
        ("AIStyles", "Generated styles"),
        ("ShowGrid", "Show grid"),
    ];
    let mut h = harness(UNKNOWN_ICON, UNKNOWN_COLOR);
    open(&mut h);
    grid(&h, &expected.map(|(_, label)| label), 6);
    for (index, (identifier, label)) in expected.into_iter().enumerate() {
        if index > 0 {
            open(&mut h);
        }
        choose(&mut h, label, Choice::Icon(identifier));
    }
    assert_eq!(h.state().icon, UNKNOWN_ICON);
    assert_eq!(h.state().color, UNKNOWN_COLOR);
}

#[test]
fn two_by_six_colour_grid_keeps_classic_blue_and_new_blue_distinct() {
    let expected = [
        ("", "None"),
        ("red", "Red"),
        ("orange", "Orange"),
        ("yellow", "Yellow"),
        ("green", "Green"),
        ("teal", "Teal"),
        ("blue", "Light blue"),
        ("blue2", "Blue"),
        ("magenta", "Magenta"),
        ("purple", "Purple"),
        ("violet", "Lavender"),
        ("gray", "Gray"),
    ];
    let mut h = harness(UNKNOWN_ICON, UNKNOWN_COLOR);
    open(&mut h);
    grid(&h, &expected.map(|(_, label)| label), 6);
    for (index, (name, label)) in expected.into_iter().enumerate() {
        if index > 0 {
            open(&mut h);
        }
        choose(&mut h, label, Choice::Color(name));
    }
    assert_eq!(h.state().icon, UNKNOWN_ICON);
    assert_eq!(h.state().color, UNKNOWN_COLOR);
}

#[test]
fn thumbnail_uses_the_callers_indexed_palette_and_theme_neutral_fallback() {
    let palette = swatches();
    for kind in ThemeKind::ALL {
        let tokens = Tokens::for_kind(kind);
        for unknown_or_none in ["", UNKNOWN_COLOR, "future-color", "Blue", "blue2 "] {
            assert_eq!(thumbnail_style(unknown_or_none, &palette, &tokens), (tokens.card, tokens.text));
            assert_eq!(super::swatch_style(unknown_or_none, &palette, &tokens), (tokens.card, tokens.text));
        }
        assert_eq!(thumbnail_style("blue", &palette, &tokens).0, palette[5]);
        assert_eq!(thumbnail_style("blue2", &palette, &tokens).0, palette[9]);
        assert_ne!(palette[5], palette[9]);
        let actual = tokens.action_swatches();
        for (index, name) in photocraft_engine::actions_cmds::BUTTON_COLORS.iter().enumerate().skip(1) {
            assert_eq!(thumbnail_style(name, &actual, &tokens), (actual[index], Color32::WHITE), "{kind:?}: {name}");
            assert_eq!(super::swatch_style(name, &actual, &tokens), (actual[index], Tokens::action_thumbnail_ink(actual[index])));
        }
        assert_eq!(super::swatch_style("yellow", &actual, &tokens).1, Color32::BLACK);
    }
    assert_eq!(super::display_glyph(UNKNOWN_ICON), "square");
    assert_eq!(super::display_glyph(""), "square");
    assert_eq!(super::display_glyph("ClickAction"), "action-editor-click");
}
