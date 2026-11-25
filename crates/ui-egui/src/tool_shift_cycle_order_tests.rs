//! Shift+letter steps through a key's tool group in the toolbar flyout's order, wrapping, and the
//! anchor point tools have no letter so P never reaches them.

use craft::keymap::Chord;
use craft::ui::runtime::commands::{FirePhase, FireSource, Fired};
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

/// The app with the same document as [`app`], but with no harness and no frames: the per-tool assertions below
/// call [`crate::shortcut_dispatch::run_fired`], which is the entry a fired Shift+letter reaches once the runtime
/// has taken the key. One whole-app frame costs more than the pick itself (#15651), and 64 tool members x two frames
/// per press made these three tests the slowest of their file. The keyboard's way to this entry stays pressed for
/// one tool per kind of group in [`a_real_shift_letter_takes_the_next_tool_of_its_group`], and the chord itself is
/// pinned for every group letter by [`every_group_letter_shift_chord_reaches_its_group_step`] below. These sweeps
/// hand the command to [`crate::shortcut_dispatch::run_fired`] directly, and no menu shows a `tools.nextInGroup`
/// row, so `input::shortcut_dispatch::tests::every_displayed_shortcut_runs_its_menu_item` never takes their branch.
fn bare_app() -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let ctx = app.ctx().clone();
    (app, ctx)
}

/// Fire `tools.nextInGroup.<letter>`, the command Shift+letter is bound to, as a key press fires it.
fn fire_next_in_group(app: &mut PhotocraftApp, ctx: &egui::Context, letter: char) {
    let id = crate::shortcut_dispatch::next_in_group_id(letter);
    let command = crate::shortcut_dispatch::keymap_command(&id);
    crate::shortcut_dispatch::run_fired(app, ctx, &[Fired::new(command, FireSource::Key, FirePhase::Press, 0.0)]);
    app.sync_views();
}

#[test]
fn from_every_tool_shift_letter_lands_on_the_next_tool_in_toolbar_order() {
    let (mut app, ctx) = bare_app();
    for (letter, group) in groups() {
        for (i, from) in group.iter().enumerate() {
            app.ui.tool = *from;
            fire_next_in_group(&mut app, &ctx, letter);
            let want = group[(i + 1) % group.len()];
            assert_eq!(app.ui.tool, want, "Shift+{letter} from {from:?}");
        }
    }
}

#[test]
fn shift_letter_wraps_from_the_last_tool_to_the_first() {
    let (mut app, ctx) = bare_app();
    for (letter, group) in groups() {
        app.ui.tool = *group.last().unwrap();
        fire_next_in_group(&mut app, &ctx, letter);
        assert_eq!(app.ui.tool, group[0], "Shift+{letter} wraps");
    }
}

#[test]
fn the_first_shift_letter_from_another_tool_lands_on_the_groups_first_tool() {
    let (mut app, ctx) = bare_app();
    for (letter, group) in groups() {
        if group.contains(&Tool::Hand) {
            continue;
        }
        app.ui.tool = Tool::Hand;
        app.ui.group_tools.clear();
        fire_next_in_group(&mut app, &ctx, letter);
        assert_eq!(app.ui.tool, group[0], "Shift+{letter} from the Hand tool");
    }
}

/// The keyboard route itself, once per kind of group: a shared group's middle tool, the wrap from its last tool,
/// the Pen group (whose flyout holds three anchor tools the letter never reaches), a group entered from a tool
/// outside it, and a single-tool group. Every other member of every group goes through the same `pick_tool` line
/// and is asserted tool by tool in the three tests above.
#[test]
fn a_real_shift_letter_takes_the_next_tool_of_its_group() {
    let mut h = app();
    h.state_mut().ui.tool = Tool::PolygonLasso;
    quick_shift_press(&mut h, Key::L);
    assert_eq!(h.state().ui.tool, Tool::MagneticLasso, "Shift+L from the Polygonal Lasso");
    h.state_mut().ui.tool = Tool::CustomShape;
    quick_shift_press(&mut h, Key::U);
    assert_eq!(h.state().ui.tool, Tool::Rectangle, "Shift+U wraps to the group's first tool");
    // The Pen group steps its three pens only: the anchor tools are in its flyout but not under its letter.
    h.state_mut().ui.tool = Tool::CurvaturePen;
    quick_shift_press(&mut h, Key::P);
    assert_eq!(h.state().ui.tool, Tool::Pen, "Shift+P wraps the three pens");
    h.state_mut().ui.tool = Tool::Hand;
    h.state_mut().ui.group_tools.clear();
    quick_shift_press(&mut h, Key::G);
    assert_eq!(h.state().ui.tool, Tool::Gradient, "Shift+G from the Hand picks the group's first tool");
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.group_tools.clear();
    quick_shift_press(&mut h, Key::K);
    assert_eq!(h.state().ui.tool, Tool::Frame, "Shift+K picks the only tool of the Frame group");
}

/// The chord each group step hangs on: `command_rows` gives every `tools.nextInGroup` row its default
/// `Shift+<LETTER>` binding, and the keymap is what turns the pressed chord into the command the fired sweeps hand to
/// `run_fired` themselves. The sweeps and that entry point pin nothing here, and no menu shows a `tools.nextInGroup`
/// row, so this table - one settled app, one keymap lookup per letter, no frames per letter - is what fails when a
/// row's chord is dropped. The Space group of the Pen flyout's anchor tools (their `key()`, `state::tool`) takes the
/// same loop: its default text is `Shift+ ` with no key name, which `Chord::parse` refuses, so production binds
/// Shift+Space to nothing - pinned too, and this row changes the day a spellable binding for it is added.
#[test]
fn every_group_letter_shift_chord_reaches_its_group_step() {
    let h = app();
    for (letter, _) in crate::tool_shortcuts::group_letters() {
        let id = crate::shortcut_dispatch::next_in_group_id(letter);
        let command = crate::shortcut_dispatch::keymap_command(&id);
        let bound = craft::ui::runtime::commands::chord(&h.ctx, &command);
        let want = Chord::parse(&format!("Shift+{}", letter.to_ascii_uppercase())).ok();
        assert_eq!(bound, want, "Shift+{letter} reaches {id}");
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
