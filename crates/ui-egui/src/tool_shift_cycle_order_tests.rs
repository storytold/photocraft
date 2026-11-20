//! Shift+letter steps through a key's tool group in the toolbar flyout's order, wrapping, and the
//! anchor point tools have no letter so P never reaches them.

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::panels::toolbar::{TOOL_SECTIONS, toolbar_position};
use crate::shortcut_dispatch::tests::{app_harness, tap};
use crate::state::Tool;

/// The whole app with a small document and a second layer.
fn app() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app_harness(app)
}

fn shift_press(h: &mut Harness<'static, Box<PhotocraftApp>>, key: Key) {
    tap(h, key, Modifiers::SHIFT);
}

/// Every lettered tool group in toolbar order, with its letter.
fn groups() -> Vec<(char, Vec<Tool>)> {
    let mut keys: Vec<char> = Tool::ALL.iter().map(|t| t.key()).filter(|k| *k != '\0').collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|k| {
            let mut group: Vec<Tool> = Tool::ALL.iter().copied().filter(|t| t.key() == k && toolbar_position(*t) != usize::MAX).collect();
            group.sort_by_key(|t| toolbar_position(*t));
            (k, group)
        })
        .collect()
}

fn key_for(letter: char) -> Key {
    crate::spring_tools::key_of(letter).unwrap()
}

fn sequence(letter: char, steps: usize) -> Vec<Tool> {
    let mut h = app();
    h.state_mut().ui.tool = Tool::Hand;
    (0..steps)
        .map(|_| {
            shift_press(&mut h, key_for(letter));
            h.state().ui.tool
        })
        .collect()
}

/// [`shift_press`] with two frames instead of three: the runtime takes the key in the first and the app picks the
/// tool it fired in the second (`shortcut_dispatch::pick_tool`); the third only drew the toolbar again.
fn quick_shift_press(h: &mut Harness<'static, Box<PhotocraftApp>>, key: Key) {
    h.input_mut().events.extend([
        egui::Event::ModifiersChanged(Modifiers::SHIFT),
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::SHIFT },
        egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::SHIFT },
        egui::Event::ModifiersChanged(Modifiers::NONE),
    ]);
    h.run_steps(2);
}

#[test]
fn from_every_tool_shift_letter_lands_on_the_next_tool_in_toolbar_order() {
    let mut h = app();
    for (letter, group) in groups() {
        for (i, from) in group.iter().enumerate() {
            h.state_mut().ui.tool = *from;
            quick_shift_press(&mut h, key_for(letter));
            let want = group[(i + 1) % group.len()];
            assert_eq!(h.state().ui.tool, want, "Shift+{letter} from {from:?}");
        }
    }
}

#[test]
fn shift_letter_wraps_from_the_last_tool_to_the_first() {
    let mut h = app();
    for (letter, group) in groups() {
        h.state_mut().ui.tool = *group.last().unwrap();
        shift_press(&mut h, key_for(letter));
        assert_eq!(h.state().ui.tool, group[0], "Shift+{letter} wraps");
    }
}

#[test]
fn the_first_shift_letter_from_another_tool_lands_on_the_groups_first_tool() {
    let mut h = app();
    for (letter, group) in groups() {
        if group.contains(&Tool::Hand) {
            continue;
        }
        h.state_mut().ui.tool = Tool::Hand;
        h.state_mut().ui.group_tools.clear();
        shift_press(&mut h, key_for(letter));
        assert_eq!(h.state().ui.tool, group[0], "Shift+{letter} from the Hand tool");
    }
}

#[test]
fn w_cycles_object_selection_quick_selection_magic_wand() {
    assert_eq!(sequence('W', 4), vec![Tool::ObjectSelection, Tool::QuickSelection, Tool::MagicWand, Tool::ObjectSelection]);
}

#[test]
fn t_cycles_horizontal_vertical_vertical_mask_horizontal_mask() {
    assert_eq!(sequence('T', 5), vec![Tool::Type, Tool::VerticalType, Tool::VerticalTypeMask, Tool::HorizontalTypeMask, Tool::Type]);
}

#[test]
fn p_cycles_the_three_pens_and_never_reaches_an_anchor_point_tool() {
    let seen = sequence('P', 7);
    assert_eq!(&seen[..4], &[Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen, Tool::Pen]);
    for t in seen {
        assert!(!matches!(t, Tool::AddAnchorPoint | Tool::DeleteAnchorPoint | Tool::ConvertPoint), "{t:?}");
    }
}

#[test]
fn the_anchor_point_tools_have_no_letter_but_stay_in_the_pen_flyout() {
    for t in [Tool::AddAnchorPoint, Tool::DeleteAnchorPoint, Tool::ConvertPoint] {
        assert_eq!(t.key(), '\0', "{t:?}");
        assert!(TOOL_SECTIONS.iter().flat_map(|s| s.iter()).any(|slot| slot.contains(&t)), "{t:?} stays in the toolbar");
    }
}

#[test]
fn the_cycle_order_is_the_flattened_toolbar_order() {
    let flat: Vec<Tool> = TOOL_SECTIONS.iter().flat_map(|s| s.iter()).flat_map(|slot| slot.iter().copied()).collect();
    for (i, t) in flat.iter().enumerate() {
        assert_eq!(toolbar_position(*t), i, "{t:?}");
    }
}

/// With the Shift switch off the plain letter cycles too, in the flyout's order (the order Shift+letter steps in).
#[test]
fn with_the_shift_switch_preference_off_plain_w_keeps_the_old_order() {
    let mut h = app();
    h.state_mut().session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = false);
    h.state_mut().ui.tool = Tool::MagicWand;
    let mut seen = Vec::new();
    for _ in 0..3 {
        tap(&mut h, Key::W, Modifiers::NONE);
        seen.push(h.state().ui.tool);
    }
    assert_eq!(seen, vec![Tool::ObjectSelection, Tool::QuickSelection, Tool::MagicWand]);
}

#[test]
fn a_plain_w_from_outside_the_group_keeps_selecting_what_it_selected_before() {
    let mut h = app();
    h.state_mut().ui.tool = Tool::Move;
    tap(&mut h, Key::W, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::ObjectSelection, "fresh W selects the first tool in the flyout");
}
