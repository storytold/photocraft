//! The Artboard tool's Add New Artboard placing mode (#11386): with the mode on, a click on empty
//! canvas puts a board of the bar's size with its top-left corner at the click.

use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The measured document: two boards, the 750 x 1334 one selected, the other one apart from it.
fn app_with_selected_board() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [2000, 0, 300, 300]})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [900, 0, 750, 1334]})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.ui.tool = Tool::Artboard;
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x, y }, m);
}

fn boards(app: &PhotocraftApp) -> Vec<Rect> {
    app.session.active().unwrap().doc.artboards().iter().map(|b| b.2.rect).collect()
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.entries().last().cloned()
}

/// A click puts one named, active board of the bar's size at the click, rounded to whole pixels, in one step; the mode
/// stays on for the next click.
#[test]
fn a_click_with_the_mode_on_puts_a_bar_sized_board_at_the_click() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    let before = boards(&app);
    click(&mut app, 222.0, 600.0);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let after = boards(&app);
    assert_eq!(after.len(), before.len() + 1);
    // 750 x 1334, top-left at the click, over the selected board (it did not move aside).
    assert_eq!(after.last().copied(), Some(Rect::new(222, 600, 972, 1934)));
    assert_eq!(after.get(..before.len()), Some(before.as_slice()));
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.last().map(|l| l.name.as_str()), Some("Artboard 3"));
    assert_eq!(st.active_layer, Some(st.doc.artboards().last().unwrap().0));
    assert_eq!(last_step(&app).as_deref(), Some("New Artboard"));
    // The mode stays on; the next click rounds to whole pixels and takes the new selected board's size.
    assert!(app.ui.artboard_tool.adding);
    // An empty spot on the document, in view at the fitted zoom.
    crate::canvas::testing::frame(&mut app);
    click(&mut app, 100.4, 200.6);
    assert_eq!(boards(&app).last().copied(), Some(Rect::new(100, 201, 850, 1535)));
    // One undo step each.
    app.run("edit.undo", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(boards(&app), before);
}

#[test]
fn with_no_board_the_click_uses_the_bars_tool_option_size() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Artboard;
    app.ui.artboard_tool.adding = true;
    click(&mut app, 40.0, 50.0);
    // The size the bar shows (the iPhone 8/7/6 default), not the canvas's 800 x 600.
    assert_eq!(boards(&app), vec![Rect::new(40, 50, 790, 1384)]);
}

/// Every way off the Artboard tool turns the mode off: selecting another tool and coming back, and the Shift+V
/// shortcut away and back. Nothing is placed afterwards.
#[test]
fn another_tool_turns_the_mode_off() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    crate::tool_memory::select(&mut app, Tool::Move);
    crate::tool_memory::select(&mut app, Tool::Artboard);
    assert!(!app.ui.artboard_tool.adding);

    app.ui.artboard_tool.adding = true;
    let press = |app: &mut PhotocraftApp, modifiers: Modifiers| crate::canvas::testing::key(app, egui::Key::V, modifiers);
    press(&mut app, Modifiers::SHIFT);
    assert_eq!(app.ui.tool, Tool::Move);
    press(&mut app, Modifiers::SHIFT);
    assert_eq!(app.ui.tool, Tool::Artboard);
    assert!(!app.ui.artboard_tool.adding);
    let before = boards(&app);
    click(&mut app, 222.0, 600.0);
    assert_eq!(boards(&app), before);
}

/// The Add New Artboard command flips the mode, publishes it as its checked state and adds no board.
#[test]
fn the_add_new_artboard_button_toggles_the_mode_and_adds_no_board() {
    use craft::draw::ui::artboard::ArtboardCommand;
    use craft::keymap::CommandRef;
    use craft::ui::runtime::commands::CommandState;

    let mut app = app_with_selected_board();
    let before = boards(&app);
    let ctx = app.ctx().clone();
    let command: CommandRef = ArtboardCommand::AddNew.into();
    // PLAN: assumed `PartialEq` on `CommandState` to read the published flags.
    let published = |app: &PhotocraftApp| crate::artboard_tool::command_states(app).into_iter().find(|(c, _)| *c == command).map(|(_, s)| s);
    assert_eq!(published(&app), Some(CommandState::new().enabled(true).checked(false)));
    assert!(crate::artboard_tool::run_fired(&mut app, &ctx, &command));
    assert!(app.ui.artboard_tool.adding, "the first run turns the mode on");
    assert_eq!(published(&app), Some(CommandState::new().enabled(true).checked(true)));
    assert_eq!(boards(&app), before);
    assert!(crate::artboard_tool::run_fired(&mut app, &ctx, &command));
    assert!(!app.ui.artboard_tool.adding, "the second run turns it off");
    assert_eq!(boards(&app), before);
}
