//! #12860: the Object Selection gear popup ends with the reference application's **Overlay Options**
//! section (Color with thirteen names at Magenta, Outline 0-50 px at 0 px, Opacity whole percents
//! 0-100 at 32 %, and the Auto / Always / Never Show Overlay radios at Auto), and the options bar
//! carries the Show all objects toggle between Refresh and the gear, off by default and flipped while
//! N is held. The gear's four rows (#12857) stay as they were.

use egui::accesskit::{Role, Toggled};
use egui::{Modifiers, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::object_finder::Update;
use crate::state::{ObjectFinderMode, OverlayShow, Tool, ToolOptions};

type H = Harness<'static, Box<PhotocraftApp>>;

const GEAR: &str = "Set additional options";
const SHOW_ALL: &str = "Show all objects";
const SHOW_ALL_TIP: &str = "Show all objects (hold N to toggle preview mode)";
const COLORS: [&str; 13] = ["Blue", "Green", "Chartreuse", "Yellow", "Orange", "Red", "Magenta", "Purple", "Indigo", "Fuchsia", "Seafoam", "White", "Black"];

/// The options bar with `tool` active, no popup open.
fn bar_of(tool: Tool) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 160})).unwrap();
    app.ui.tool = tool;
    crate::theme::install_fonts(app.ctx());
    crate::theme::apply(app.ctx(), &crate::theme::AppTheme::pro());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1800.0, 400.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// The Object Selection options bar, `gear` with its popup open.
fn bar(gear: bool) -> H {
    let mut h = bar_of(Tool::ObjectSelection);
    if gear {
        let at = node(&h, GEAR, Role::Button).expect("the Object Selection gear");
        press(&mut h, &at);
    }
    h
}

fn gear() -> H {
    bar(true)
}

/// The one node labelled `label` with `role`.
fn node(h: &H, label: &str, role: Role) -> Option<Rect> {
    h.query_all_by_label(label).find(|n| n.accesskit_node().role() == role).map(|n| n.rect())
}

/// The first node labelled `label`, whatever role the framework gives a menu row.
fn row_node(h: &H, label: &str) -> Option<Rect> {
    h.query_all_by_label(label).next().map(|n| n.rect())
}

/// Presses the middle of `rect`: the pointer moves first and the press and release each get their
/// own pass, which is how the framework's popup hit tests see a press land on a row.
fn press(h: &mut H, rect: &Rect) {
    let at = rect.center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
}

fn press_named(h: &mut H, label: &str, role: Role) {
    press(h, &node(h, label, role).unwrap_or_else(|| panic!("no {role:?} labelled {label}")));
}

/// The field named `label` ("Outline", "Opacity"), with or without its caption's colon.
fn field(h: &H, label: &str) -> Option<Rect> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.trim_end_matches(':') == label)).map(|n| n.rect())
}

fn field_value(h: &H, label: &str) -> Option<f64> {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.trim_end_matches(':') == label))
        .and_then(|n| n.accesskit_node().numeric_value())
}

/// Types `text` into the field named `label` and leaves it, so its commit and any alert run.
fn type_into(h: &mut H, label: &str, text: &str) {
    press(h, &field(h, label).unwrap_or_else(|| panic!("no {label} field")));
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

fn radio(h: &H, label: &str) -> Option<Toggled> {
    h.query_all_by_role(Role::RadioButton).find(|n| n.accesskit_node().label().as_deref() == Some(label)).and_then(|n| n.accesskit_node().toggled())
}

fn on(h: &H, label: &str) -> bool {
    radio(h, label) == Some(Toggled::True)
}

/// Whether the bar's toggle button named `label` reads pressed (an off toggle reports no state at all).
fn pressed_on(h: &H, label: &str) -> bool {
    node(h, label, Role::Button).is_some()
        && h.query_all_by_label(label).any(|n| n.accesskit_node().role() == Role::Button && n.accesskit_node().toggled() == Some(Toggled::True))
}

fn alerts(h: &H) -> usize {
    crate::dialogs::alerts::alerts(h.state()).len()
}

fn color_dropdown(h: &H) -> Option<Rect> {
    h.query_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some("Color")).map(|n| n.rect())
}

fn color_value(h: &H) -> Option<String> {
    h.query_all_by_role(Role::ComboBox)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Color"))
        .and_then(|n| n.accesskit_node().value())
        .map(|v| v.as_str().to_owned())
}

/// The rect of the colour name `name` in the open list: the closed face carries the picked name too,
/// so the row furthest down is the list's own.
fn color_row(h: &H, name: &str) -> Option<Rect> {
    h.query_all_by_label(name).map(|n| n.rect()).max_by(|a, b| a.top().total_cmp(&b.top()))
}

const DISC: [f64; 2] = [100.0, 80.0];

/// A 200 x 160 document: a red disc (centre 100,80, radius 45) on blue, nothing selected, the Object
/// Selection tool active with the finder on.
fn disc_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 160})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 160})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#3366cc"})).unwrap();
    app.run("select.rect", json!({"x": 55, "y": 35, "width": 90, "height": 90, "ellipse": true})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#e63319"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::ObjectSelection;
    app.ui.tool_options.object_finder = true;
    app
}

/// Hovers at `at` until the lookup settles (time advances past the debounce).
fn hover(app: &mut PhotocraftApp, at: [f64; 2]) -> Update {
    let ctx = app.ctx().clone();
    let mut t = 100.0;
    loop {
        let u = crate::object_finder::update(app, Some(at), false, t, &ctx);
        if !u.busy || t > 130.0 {
            return u;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        t += 0.05;
    }
}

#[test]
fn the_gear_popup_ends_with_the_measured_overlay_rows() {
    let h = gear();
    assert!(h.query_by_label("Overlay Options").is_some(), "the section head the reference shows last");
    assert!(color_dropdown(&h).is_some(), "the Color dropdown");
    assert!(field(&h, "Outline").is_some(), "the Outline field");
    assert!(field(&h, "Opacity").is_some(), "the Opacity field");
    for label in ["Auto Show Overlay", "Always Show Overlay", "Never Show Overlay"] {
        assert!(radio(&h, label).is_some(), "the {label} radio");
    }
    // The section is below the four rows it follows, and inside the same popup.
    let subtract = row_node(&h, "Object Subtract");
    let head = node(&h, "Overlay Options", Role::Heading).expect("the section head");
    if let Some(row) = subtract {
        assert!(head.top() > row.top(), "the section follows the rows it ends the popup with");
    }
}

#[test]
fn the_overlay_rows_open_at_the_measured_defaults() {
    let h = gear();
    assert_eq!(color_value(&h).as_deref(), Some("Magenta"), "Magenta by default");
    assert_eq!(field_value(&h, "Outline"), Some(0.0), "Outline 0 px");
    assert_eq!(field_value(&h, "Opacity"), Some(32.0), "Opacity 32 %");
    assert!(on(&h, "Auto Show Overlay"), "Auto Show Overlay by default");
    assert!(!on(&h, "Always Show Overlay") && !on(&h, "Never Show Overlay"), "the other two are not picked");
    let tip = h
        .query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.trim_end_matches(':') == "Opacity"))
        .and_then(|n| n.accesskit_node().description())
        .map(|d| d.to_string());
    assert_eq!(tip.as_deref(), Some("Set selection preview overlay opacity."), "the measured tooltip");
}

#[test]
fn the_color_list_holds_the_measured_names_in_order() {
    let mut h = gear();
    let at = color_dropdown(&h).expect("the Color dropdown");
    press(&mut h, &at);
    // The open list shows a window of its rows with the entry the field starts on kept in view; the
    // other rows it shows are the measured list's own, in its order.
    let mut shown: Vec<(f32, &str)> = COLORS.iter().filter_map(|name| color_row(&h, name).map(|r| (r.top(), *name))).collect();
    shown.sort_by(|a, b| a.0.total_cmp(&b.0));
    let names: Vec<&str> = shown.iter().map(|(_, name)| *name).collect();
    assert!(names.len() >= 3, "the open list shows rows: {names:?}");
    assert!(names.contains(&"Magenta"), "the list opens on the entry it starts at");
    let rest: Vec<&str> = names.iter().copied().filter(|name| *name != "Magenta").collect();
    assert!(COLORS.windows(rest.len()).any(|window| window == rest), "the other rows are one run of the measured list, in its order: {rest:?}");
}

#[test]
fn the_two_fields_show_the_measured_defaults_in_their_own_units() {
    // The fields' own display: 0 px and 32 %, and an in-range value typed in stays.
    let mut h = gear();
    assert_eq!(field_value(&h, "Outline"), Some(0.0), "Outline 0 px");
    assert_eq!(field_value(&h, "Opacity"), Some(32.0), "Opacity 32 %");
    type_into(&mut h, "Outline", "20");
    assert_eq!(field_value(&h, "Outline"), Some(20.0), "20 px stays 20");
    assert_eq!(alerts(&h), 0, "20 px needs no alert");
}

#[test]
fn picking_a_radio_moves_only_that_radio() {
    // (the row to press, Auto, Always, Never)
    for (pick, auto, always, never) in
        [("Always Show Overlay", false, true, false), ("Never Show Overlay", false, false, true), ("Auto Show Overlay", true, false, false)]
    {
        let mut h = gear();
        press_named(&mut h, pick, Role::RadioButton);
        assert_eq!(on(&h, "Auto Show Overlay"), auto, "{pick}: Auto");
        assert_eq!(on(&h, "Always Show Overlay"), always, "{pick}: Always");
        assert_eq!(on(&h, "Never Show Overlay"), never, "{pick}: Never");
        assert_eq!(
            h.state().ui.tool_options.object_overlay_show,
            if pick == "Always Show Overlay" {
                OverlayShow::Always
            } else if pick == "Never Show Overlay" {
                OverlayShow::Never
            } else {
                OverlayShow::Auto
            },
            "{pick}: the setting"
        );
        assert!(h.query_by_label("Overlay Options").is_some(), "{pick}: the popup stays open to edit in");
    }
}

#[test]
fn the_show_all_objects_toggle_sits_between_refresh_and_the_gear() {
    let h = bar(false);
    let button = node(&h, SHOW_ALL, Role::Button).expect("the Show all objects button");
    assert!(node(&h, SHOW_ALL, Role::Button).is_some(), "the button is there");
    assert!(!pressed_on(&h, SHOW_ALL), "off by default");
    let tip = h
        .query_all_by_label(SHOW_ALL)
        .find(|n| n.accesskit_node().role() == Role::Button)
        .and_then(|n| n.accesskit_node().description())
        .map(|d| d.to_string());
    assert_eq!(tip.as_deref(), Some(SHOW_ALL_TIP), "the reference's tooltip");
    let gear_rect = node(&h, GEAR, Role::Button).expect("the gear");
    let refresh = node(&h, "Refresh", Role::Button).expect("the Refresh button");
    assert!(refresh.right() <= button.left() && button.right() <= gear_rect.left(), "Refresh, Show all objects, the gear");
}

#[test]
fn the_gear_rows_above_the_new_section_still_work() {
    let mut h = gear();
    for row in ["Object Finder", "Object Subtract", "Auto Refresh", "Manual Refresh"] {
        assert!(h.query_all_by_label(row).next().is_some(), "the {row} row is still there");
    }
    let before = h.state().ui.tool_options.object_subtract;
    let at = row_node(&h, "Object Subtract").expect("the Object Subtract row");
    press(&mut h, &at);
    assert_eq!(h.state().ui.tool_options.object_subtract, !before, "and it still toggles");
}

#[test]
fn quick_selection_gains_nothing_of_the_object_selection_section() {
    let h = bar_of(Tool::QuickSelection);
    assert!(node(&h, GEAR, Role::Button).is_none(), "Quick Selection has no gear at all");
    assert!(node(&h, SHOW_ALL, Role::Button).is_none(), "Quick Selection has no Show all objects button");
    assert!(field(&h, "Opacity").is_none() && field(&h, "Outline").is_none(), "and no Overlay Options fields");
}

// --- The tests above drive the popup and the bar as they are drawn; they fail on upstream/main.
// They compile there once the two references to the new settings are dropped: the
// `object_overlay_show` assertions in `picking_a_radio_moves_only_that_radio` and the
// `OverlayShow` in the `use` line at the top of the file. That cut gives eight tests, six of them
// red. The tests below name the new settings and `object_finder::overlay_look`, so they only
// compile with this change. ---

#[test]
fn the_outline_field_takes_the_measured_values_and_alerts() {
    // 0-50 px: past an end the closest value goes in and the alert says so.
    for (typed, want, alert) in [("999", 50.0, 1), ("-5", 0.0, 1), ("0", 0.0, 0), ("50", 50.0, 0)] {
        let mut h = gear();
        type_into(&mut h, "Outline", typed);
        assert_eq!(h.state().ui.tool_options.object_overlay_outline, want, "Outline {typed}");
        assert_eq!(alerts(&h), alert, "Outline {typed}: {alert} alert(s) expected");
    }
}

#[test]
fn the_opacity_field_takes_the_measured_values_and_alerts() {
    // Whole percents 0-100: past an end the closest value goes in and the alert says so.
    for (typed, want, alert) in [("150", 100.0, 1), ("-5", 0.0, 1), ("0", 0.0, 0), ("100", 100.0, 0)] {
        let mut h = gear();
        type_into(&mut h, "Opacity", typed);
        assert_eq!(h.state().ui.tool_options.object_overlay_opacity, want, "Opacity {typed}");
        assert_eq!(alerts(&h), alert, "Opacity {typed}: {alert} alert(s) expected");
    }
}

// No typed-fraction case is pinned here on purpose: the framework's whole-number field reads the
// separators of the machine's own locale, so "12.5" joins to 125 and clamps on a comma-decimal box
// while a point-decimal one reads twelve and a half and rounds. What a separator does belongs to
// the framework's number field, not to this ticket; the integer clamps in the two tests above hold
// whatever the locale is.

#[test]
fn the_colour_dropdown_holds_all_thirteen_names_in_its_steps() {
    // The list is longer than the window it opens with, so its own stepping walks it: Home is its
    // first name and twelve ArrowDowns reach its last.
    let mut h = gear();
    h.query_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some("Color")).expect("the Color dropdown").focus();
    h.run_steps(1);
    h.key_press(egui::Key::Home);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool_options.object_overlay_color, "blue", "the list starts on Blue");
    for name in &COLORS[1..] {
        h.key_press(egui::Key::ArrowDown);
        h.run_steps(2);
        assert_eq!(h.state().ui.tool_options.object_overlay_color, name.to_lowercase(), "ArrowDown reaches {name}");
    }
}

#[test]
fn picking_a_colour_in_the_gear_changes_the_hover_tint() {
    let mut h = gear();
    let magenta = egui::Color32::from_rgb(0xCC, 0x1A, 0x7E);
    assert_eq!(crate::object_finder::overlay_look(&h.state().ui.tool_options).map(|l| l.color), Some(magenta), "Magenta");
    h.query_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some("Color")).expect("the Color dropdown").focus();
    h.run_steps(1);
    // Two steps down the measured list from Magenta: Purple, then Indigo.
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool_options.object_overlay_color, "indigo", "the pick is stored");
    assert_eq!(
        crate::object_finder::overlay_look(&h.state().ui.tool_options).map(|l| l.color),
        Some(egui::Color32::from_rgb(0x54, 0x4B, 0xE7)),
        "and the tint follows it"
    );
}
#[test]
fn holding_n_flips_the_toggle_while_it_is_held() {
    let mut h = bar(false);
    let key = |h: &mut H, pressed: bool| {
        h.event(egui::Event::Key { key: egui::Key::N, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
    };
    key(&mut h, true);
    assert!(pressed_on(&h, SHOW_ALL), "N held: the button reads on");
    assert!(!h.state().ui.tool_options.object_show_all_objects, "the stored setting stays off");
    key(&mut h, false);
    assert!(!pressed_on(&h, SHOW_ALL), "released: back off");
    press_named(&mut h, SHOW_ALL, Role::Button);
    assert!(h.state().ui.tool_options.object_show_all_objects, "a click stores on");
    key(&mut h, true);
    assert!(!pressed_on(&h, SHOW_ALL), "N flips a stored on back off while held");
    key(&mut h, false);
    assert!(pressed_on(&h, SHOW_ALL), "and gives it back");
}

#[test]
fn typing_an_outline_and_an_opacity_puts_them_in_the_tool_options() {
    let mut h = gear();
    type_into(&mut h, "Outline", "12");
    assert_eq!(h.state().ui.tool_options.object_overlay_outline, 12.0, "Outline 12 px is stored");
    let mut h = gear();
    type_into(&mut h, "Opacity", "60");
    assert_eq!(crate::object_finder::overlay_look(&h.state().ui.tool_options).map(|l| l.opacity), Some(60.0 / 100.0), "Opacity 60 % reaches the tint");
}

#[test]
fn the_hover_tint_looks_as_the_settings_ask() {
    let look = crate::object_finder::overlay_look(&ToolOptions::default()).expect("Auto shows a tint");
    assert_eq!(look.color, egui::Color32::from_rgb(0xCC, 0x1A, 0x7E), "Magenta by default");
    assert_eq!(look.opacity, 32.0 / 100.0, "32 % by default");
    let picked =
        crate::object_finder::overlay_look(&ToolOptions { object_overlay_color: "seafoam".into(), object_overlay_opacity: 100.0, ..ToolOptions::default() })
            .expect("a picked colour shows a tint");
    assert_eq!(picked.color, egui::Color32::from_rgb(0x1C, 0x84, 0x88), "the picked colour");
    assert_eq!(picked.opacity, 1.0, "100 %");
    // The measured bottom of the Opacity field draws no overlay at all, like Never Show Overlay.
    let zero = crate::object_finder::overlay_look(&ToolOptions { object_overlay_opacity: 0.0, ..ToolOptions::default() }).expect("a look to judge");
    assert!(!crate::object_finder::overlay_draws(&zero), "0 % shows no overlay");
    let standard = crate::object_finder::overlay_look(&ToolOptions::default()).expect("Auto shows a tint");
    assert!(crate::object_finder::overlay_draws(&standard), "32 % draws the tint");
    // The Outline setting is kept for the tint; its width joins the call with the framework prop.
    assert_eq!(ToolOptions::default().object_overlay_outline, 0.0, "the Outline field starts at 0 px");
}

#[test]
fn never_shows_no_tint_and_always_keeps_todays_preview() {
    // Only Never is measured (#12860): no tint. Always keeps today's hover preview.
    assert!(crate::object_finder::overlay_look(&ToolOptions { object_overlay_show: OverlayShow::Never, ..ToolOptions::default() }).is_none(), "Never: no tint");
    assert!(
        crate::object_finder::overlay_look(&ToolOptions { object_overlay_show: OverlayShow::Always, ..ToolOptions::default() }).is_some(),
        "Always: today's tint"
    );
    // The finder itself is untouched by either: it keeps finding the object it highlights.
    let mut app = disc_app();
    assert!(hover(&mut app, DISC).shown.is_some());
    app.ui.tool_options.object_overlay_show = OverlayShow::Never;
    assert!(hover(&mut app, DISC).shown.is_some(), "Never hides the tint, not the hover");
    app.ui.tool_options.object_overlay_show = OverlayShow::Always;
    assert!(hover(&mut app, DISC).shown.is_some(), "Always shows what Auto shows");
}

#[test]
fn a_setting_written_past_its_range_still_tints_inside_it() {
    // Prefs written by hand reach the tool without the field's clamp.
    let look = crate::object_finder::overlay_look(&ToolOptions { object_overlay_opacity: 500.0, ..ToolOptions::default() }).expect("Auto shows a tint");
    assert_eq!(look.opacity, 1.0, "past the top of the range tints fully");
    let negative = crate::object_finder::overlay_look(&ToolOptions { object_overlay_opacity: -20.0, ..ToolOptions::default() }).expect("a look to judge");
    assert!(!crate::object_finder::overlay_draws(&negative), "below the bottom draws nothing");
    let unknown =
        crate::object_finder::overlay_look(&ToolOptions { object_overlay_color: "heliotrope".into(), ..ToolOptions::default() }).expect("Auto shows a tint");
    assert_eq!(unknown.color, egui::Color32::from_rgb(0xCC, 0x1A, 0x7E), "a name the list does not know tints with the default");
}

#[test]
fn the_new_settings_default_and_load_from_settings_written_before_them() {
    let d = ToolOptions::default();
    assert_eq!(d.object_overlay_color, "magenta");
    assert_eq!(d.object_overlay_outline, 0.0);
    assert_eq!(d.object_overlay_opacity, 32.0);
    assert_eq!(d.object_overlay_show, OverlayShow::Auto);
    assert!(!d.object_show_all_objects);
    // A settings file written before #12860 names none of them.
    let old: ToolOptions = serde_json::from_value(json!({ "object_finder": true })).unwrap();
    assert_eq!(old.object_overlay_opacity, 32.0, "the old file still loads Opacity 32 %");
    assert_eq!(old.object_overlay_color, "magenta");
    let mut v = serde_json::to_value(&d).unwrap();
    let m = v.as_object_mut().unwrap();
    for key in ["object_overlay_color", "object_overlay_outline", "object_overlay_opacity", "object_overlay_show", "object_show_all_objects"] {
        assert!(m.remove(key).is_some(), "{key} is stored so the control channel reads it");
    }
    let o: ToolOptions = serde_json::from_value(v).unwrap();
    assert_eq!(o, d, "and dropping them gives the defaults back");
    let picked = ToolOptions {
        object_overlay_color: "seafoam".into(),
        object_overlay_outline: 7.0,
        object_overlay_opacity: 5.0,
        object_overlay_show: OverlayShow::Never,
        object_show_all_objects: true,
        ..d
    };
    let back: ToolOptions = serde_json::from_value(serde_json::to_value(&picked).unwrap()).unwrap();
    assert_eq!(back, picked, "and they round-trip");
}

#[test]
fn reset_tool_restores_the_opacity_alone() {
    // The measured one: Reset Tool puts Opacity 0 back to 32 %. Color, Outline and the radios were
    // not measured, so they keep the setting they hold.
    let mut o = ToolOptions {
        object_overlay_opacity: 0.0,
        object_overlay_color: "indigo".into(),
        object_overlay_outline: 12.0,
        object_overlay_show: OverlayShow::Never,
        ..ToolOptions::default()
    };
    crate::tools::tool_reset::reset_options(&mut o, Tool::ObjectSelection);
    assert_eq!(o.object_overlay_opacity, 32.0, "the measured restore");
    assert_eq!(o.object_overlay_color, "indigo", "Color keeps its setting");
    assert_eq!(o.object_overlay_outline, 12.0, "Outline keeps its setting");
    assert_eq!(o.object_overlay_show, OverlayShow::Never, "the radios keep their setting");
    assert!(!o.object_show_all_objects, "Show all objects keeps its setting");
    assert_eq!(o.object_finder_mode, ObjectFinderMode::Auto, "the #12857 settings still reset");
}

#[test]
fn the_object_finder_still_highlights_what_a_click_selects() {
    // The neighbour: none of the new settings change what the finder finds with its own defaults.
    let r = hover(&mut disc_app(), DISC).shown.expect("the disc is highlighted");
    assert!(r.at(100, 80) >= 0.5 && r.at(5, 5) < 0.5, "the same object as before");
    let mut app = disc_app();
    app.ui.tool_options.object_overlay_show = OverlayShow::Never;
    let r = hover(&mut app, DISC).shown.expect("Never still finds it");
    assert!(r.at(100, 80) >= 0.5, "and the same one");
}
