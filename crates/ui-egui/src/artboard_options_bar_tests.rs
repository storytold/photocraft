//! The Artboard tool's options bar (#11385): the background for the boards it draws, the size the bar edits and
//! the gear menu's Auto-nest Layers.

use craft::behavior::kurbo::Size;
use craft::behavior::{EditEvent, EditEvents, GestureId};
use craft::draw::ui::artboard::ArtboardBackground as Shown;
use egui::Modifiers;
use photocraft_doc::{ArtboardBackground, LayerId};
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::artboard_tool::bar;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app_with_depth(depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600, "depth": depth})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.ui.tool = Tool::Artboard;
    app
}

fn app() -> PhotocraftApp {
    app_with_depth(8)
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

/// A click with the placing mode on.
fn place(app: &mut PhotocraftApp, x: f64, y: f64) {
    app.ui.artboard_tool.adding = true;
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x, y }, m);
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

fn boards(app: &PhotocraftApp) -> Vec<(LayerId, Rect, ArtboardBackground)> {
    app.session.active().unwrap().doc.artboards().iter().map(|b| (b.0, b.2.rect, b.2.background)).collect()
}

/// One proposed size, as the bar's W and H fields report a typed value.
fn size_event(w: f64, h: f64) -> EditEvents<Size> {
    EditEvents::from(EditEvent::update(GestureId::new(1), Size::new(w, h)))
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn last_label(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.entries().last().cloned()
}

/// With no board selected, the bar's size and background events edit only the tool's options: no history step, no board.
#[test]
fn width_and_height_edit_the_tool_option_when_no_board_is_selected() {
    let mut app = app();
    assert!(boards(&app).is_empty());
    assert_eq!(bar::bar_size(&app), [750, 1334]);
    let s0 = steps(&app);
    bar::route(&mut app, None, size_event(400.0, 500.0), Some(Shown::Black), None);
    assert_eq!((app.ui.artboard_tool.width, app.ui.artboard_tool.height), (400, 500));
    assert_eq!(app.ui.artboard_tool.background, "black");
    assert_eq!(bar::bar_size(&app), [400, 500]);
    // Out of range sizes are held to 1..=300000.
    bar::route(&mut app, None, size_event(0.0, 1.0e9), None, None);
    assert_eq!((app.ui.artboard_tool.width, app.ui.artboard_tool.height), (1, 300_000));
    assert_eq!(steps(&app), s0, "no history step");
    assert!(boards(&app).is_empty());
}

/// With a board selected the bar's size is the board's: a size event is one `layer.artboard.set` step that keeps the
/// top-left corner, and the tool options stay.
#[test]
fn with_a_board_selected_the_bar_size_is_the_boards_and_the_options_stay() {
    let mut app = app();
    bar::set_size(&mut app, 400.0, 500.0);
    drag(&mut app, [100.0, 100.0], [300.0, 300.0]);
    assert_eq!(boards(&app).first().map(|b| b.1), Some(Rect::new(100, 100, 300, 300)));
    assert_eq!(bar::bar_size(&app), [200, 200]);
    let id = boards(&app)[0].0;
    assert_eq!((app.ui.artboard_tool.width, app.ui.artboard_tool.height), (400, 500), "the tool option is untouched");
    let s0 = steps(&app);
    // Choosing iPhone 8/7/6 in the bar's Size menu is a size event of the preset's size.
    bar::route(&mut app, Some(id), size_event(750.0, 1334.0), None, None);
    assert_eq!(boards(&app)[0].1, Rect::new(100, 100, 850, 1434));
    assert_eq!(steps(&app), s0 + 1);
    assert_eq!(bar::bar_size(&app), [750, 1334]);
    assert_eq!((app.ui.artboard_tool.width, app.ui.artboard_tool.height), (400, 500));
}

/// The background reaches `layer.new.artboard` for a board the tool draws or places, at 8 and 16 bit.
#[test]
fn a_drawn_board_gets_the_bars_background() {
    for depth in [8, 16] {
        for (name, want) in [("white", ArtboardBackground::White), ("black", ArtboardBackground::Black), ("transparent", ArtboardBackground::Transparent)] {
            let mut app = app_with_depth(depth);
            app.ui.artboard_tool.background = name.to_string();
            drag(&mut app, [50.0, 50.0], [150.0, 120.0]);
            assert!(!app.ui.status_error, "{depth} bit {name}: {}", app.ui.status);
            let b = boards(&app);
            assert_eq!(b.len(), 1, "{depth} bit {name}");
            assert_eq!((b[0].1, b[0].2), (Rect::new(50, 50, 150, 120), want), "{depth} bit {name}");
            let mut app = app_with_depth(depth);
            app.ui.artboard_tool.background = name.to_string();
            place(&mut app, 20.0, 30.0);
            assert_eq!(boards(&app)[0].2, want, "placed, {depth} bit {name}");
        }
        // Other…: the picked colour.
        for placed in [false, true] {
            let mut app = app_with_depth(depth);
            bar::set_custom_background(&mut app, "#ff0000").unwrap();
            if placed {
                place(&mut app, 20.0, 30.0);
            } else {
                drag(&mut app, [50.0, 50.0], [150.0, 120.0]);
            }
            let ArtboardBackground::Custom(c) = boards(&app)[0].2 else { panic!("{depth} bit: a custom background") };
            assert_eq!(c.to_rgba8(), [255, 0, 0, 255], "{depth} bit");
        }
    }
}

/// OK in the picker with the colour the swatch shows leaves the boards' own colours alone: for one board with a
/// translucent colour and for two boards of different colours.
#[test]
fn ok_in_the_picker_without_a_change_leaves_the_boards_alone() {
    let mut app = app();
    let a = app.run("layer.new.artboard", json!({"rect": [0, 0, 100, 100]})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.artboard.set", json!({"layer": a, "background": "custom", "color": [0.2, 0.4, 0.6, 0.5]})).unwrap();
    app.run("layer.select", json!({"layer": a})).unwrap();
    let before = (steps(&app), boards(&app)[0].2);
    pick_from_swatch(&mut app, None);
    assert_eq!((steps(&app), boards(&app)[0].2), before, "no history step, colour bit-exact");
    // A real change still recolours.
    bar::set_custom_background(&mut app, "#00ff00").unwrap();
    assert_eq!(steps(&app), before.0 + 1);
    assert_ne!(boards(&app)[0].2, before.1);

    let mut app = self::app();
    two_coloured_boards(&mut app);
    let before = (steps(&app), colours(&app));
    pick_from_swatch(&mut app, None);
    assert_eq!((steps(&app), colours(&app)), before, "no history step, both colours bit-exact");
    assert_eq!((app.ui.artboard_tool.background.as_str(), app.ui.artboard_tool.background_color.as_str()), ("custom", "#336699"));
}

/// Two boards, both selected.
fn two_boards(app: &mut PhotocraftApp) -> (u64, u64) {
    let a = app.run("layer.new.artboard", json!({"rect": [0, 0, 100, 100]})).unwrap()["layer"].as_u64().unwrap();
    let b = app.run("layer.new.artboard", json!({"rect": [300, 0, 100, 100]})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.select", json!({"layer": a})).unwrap();
    app.run("layer.select", json!({"layer": b, "mode": "add"})).unwrap();
    (a, b)
}

/// Two selected boards with different custom colours; the first board's colour is the swatch's.
fn two_coloured_boards(app: &mut PhotocraftApp) -> (u64, u64) {
    let (a, b) = two_boards(app);
    app.run("layer.artboard.set", json!({"layer": a, "background": "custom", "color": "#336699"})).unwrap();
    app.run("layer.artboard.set", json!({"layer": b, "background": "custom", "color": "#cc3300"})).unwrap();
    (a, b)
}

/// Open the Color Picker from the swatch (as the bar does) and OK it with `color`, or as opened.
fn pick_from_swatch(app: &mut PhotocraftApp, color: Option<&str>) {
    let hex = bar::shown_background(app).1;
    let id =
        crate::color_picker_ui::open_for_target(app, "artboardBackground", "Color Picker (Color)", crate::color_picker_ui::parse_hex(&hex).unwrap()).unwrap();
    if let Some(c) = color {
        crate::color_picker_ui::testing::set_color(app, id, c);
    }
    crate::dialogs::testing::confirm(app, id).unwrap();
}

fn colours(app: &PhotocraftApp) -> Vec<ArtboardBackground> {
    boards(app).iter().map(|b| b.2).collect()
}

#[test]
fn a_changed_ok_with_boards_of_different_colours_recolours_both_in_one_step() {
    let mut app = app();
    two_coloured_boards(&mut app);
    let steps0 = steps(&app);
    pick_from_swatch(&mut app, Some("#00ff00"));
    assert_eq!(steps(&app), steps0 + 1);
    assert_eq!(last_label(&app).as_deref(), Some("Edit Artboards"));
    let green = |c: &ArtboardBackground| matches!(c, ArtboardBackground::Custom(c) if c.to_rgba8()[..3] == [0, 255, 0]);
    assert!(colours(&app).iter().all(green));
}

#[test]
fn an_unchanged_ok_with_only_web_colors_leaves_the_boards_alone() {
    let mut app = app();
    // Only Web Colors is the framework picker's remembered memory, read when the picker opens.
    // PLAN: assumed `app.framework.host()` is the `Host` that `Remembered` reads and writes.
    let mut memory = craft::color_ui::ui::ColorPickerMemory::default();
    memory.web_colors = true;
    craft::settings::Remembered::<craft::color_ui::ui::ColorPickerMemory>::new("colorPicker.memory").store(app.framework.host(), &memory);
    let (a, _) = two_boards(&mut app);
    app.run("layer.artboard.set", json!({"layer": a, "background": "custom", "color": [0.2, 0.4, 0.61, 1.0]})).unwrap();
    let before = (steps(&app), colours(&app));
    pick_from_swatch(&mut app, None);
    assert_eq!((steps(&app), colours(&app)), before, "no history step, colours bit-exact");
}

#[test]
fn a_colour_some_boards_already_have_recolours_only_the_others() {
    let mut app = app();
    let (_, b) = two_coloured_boards(&mut app);
    let b_before = colours(&app)[1];
    let steps0 = steps(&app);
    pick_from_swatch(&mut app, Some("#cc3300"));
    assert_eq!(steps(&app), steps0 + 1);
    assert_eq!(last_label(&app).as_deref(), Some("Edit Artboard"), "the label follows the one board that changed");
    assert_eq!(colours(&app)[1], b_before, "board {b} keeps its own value");
    assert!(matches!(colours(&app)[0], ArtboardBackground::Custom(c) if c.to_rgba8()[..3] == [0xcc, 0x33, 0x00]));
}

#[test]
fn a_background_choice_recolours_every_selected_board_in_one_step() {
    let mut app = app();
    let (a, b) = two_boards(&mut app);
    let before = steps(&app);
    bar::set_background(&mut app, "transparent", "#ffffff").unwrap();
    assert_eq!(steps(&app), before + 1);
    assert_eq!(last_label(&app).as_deref(), Some("Edit Artboards"));
    assert!(boards(&app).iter().all(|x| x.2 == ArtboardBackground::Transparent));
    assert_eq!(boards(&app).iter().map(|x| x.0.0).collect::<Vec<_>>(), vec![a, b]);
    assert_eq!(app.ui.artboard_tool.background, "transparent", "also the next drawn board's");
    // Other...: OK in the picker recolours them too.
    bar::set_custom_background(&mut app, "#00ff00").unwrap();
    assert_eq!(steps(&app), before + 2);
    for x in boards(&app) {
        let ArtboardBackground::Custom(c) = x.2 else { panic!("a custom background") };
        assert_eq!(c.to_rgba8(), [0, 255, 0, 255]);
    }
    assert_eq!((app.ui.artboard_tool.background.as_str(), app.ui.artboard_tool.background_color.as_str()), ("custom", "#00ff00"));
}

#[test]
fn other_opens_the_color_picker_and_ok_sets_the_option() {
    let mut app = app();
    app.ui.artboard_tool.background = "black".to_string();
    app.ui.artboard_tool.background_color = "#123456".to_string();
    let open_before = app.dialog_open();
    let rgb = crate::color_picker_ui::parse_hex("#123456").unwrap();
    let id = crate::color_picker_ui::open_for_target(&mut app, "artboardBackground", "Color Picker (Color)", rgb).unwrap();
    assert!(!open_before && app.dialog_open(), "the picker opened");
    // Cancel leaves the option alone; OK sets it.
    assert_eq!(app.ui.artboard_tool.background, "black");
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    assert_eq!((app.ui.artboard_tool.background.as_str(), app.ui.artboard_tool.background_color.as_str()), ("custom", "#123456"));
}

/// Auto-nest Layers off: a layer dragged with the Move tool onto another board stays where it was.
#[test]
fn auto_nest_layers_turns_the_move_tools_nesting_on_and_off() {
    for (nest, joins) in [(true, true), (false, false)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 300, "height": 100, "background": "transparent"})).unwrap();
        app.sync_views();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.session.execute("select.deselect", json!({})).unwrap();
        let red = app.session.active().unwrap().active_layer.unwrap();
        app.session.execute("layer.new.artboardFromLayers", json!({"name": "A"})).unwrap();
        app.session.execute("layer.artboard.set", json!({"width": 40, "height": 40})).unwrap();
        let b = LayerId(app.session.execute("layer.new.artboard", json!({"rect": [200, 20, 40, 40], "name": "B"})).unwrap()["layer"].as_u64().unwrap());
        app.session.execute("layer.select", json!({"layer": red.0})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.artboard_tool.auto_nest = nest;
        crate::canvas::move_ui::finish(&mut app, 190.0, 10.0);
        let board = app.session.active().unwrap().doc.artboard_of(red);
        assert_eq!(board == Some(b), joins, "auto_nest {nest}");
    }
}
