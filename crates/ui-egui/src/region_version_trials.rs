//! Trials of the regions' content versions in the whole app: an edit in one region, or a drag on the canvas, that
//! changes what another settled region shows without a command shows there in the frames it asks for. A version that
//! missed the edited state would replay the old value until a command ran or the pointer passed over the region.

use craft::behavior::dock::Action;
use egui::accesskit::Role;
use egui::{Event, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The whole app on a new document with `tool`, and an adjustment layer of `kind` when one is asked for.
fn harness(tool: Tool, kind: Option<&'static str>) -> H {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        if let Some(kind) = kind {
            s.execute(&format!("layer.newAdjustmentLayer.{kind}"), json!({})).unwrap();
        }
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default());
        app.ui.tool = tool;
        app
    });
    h.run_steps(8);
    h
}

/// The node of `role` named `label` (with or without its caption's colon) inside `within`.
fn node_rect(h: &H, role: Role, label: &str, within: Rect) -> Option<Rect> {
    h.query_all_by_role(role)
        .find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == label) && within.contains(n.rect().center()))
        .map(|n| n.rect())
}

/// The value field named `label` inside `within`.
fn field_rect(h: &H, label: &str, within: Rect) -> Option<Rect> {
    let fields = [Role::SpinButton, Role::ComboBox, Role::EditableComboBox, Role::TextInput];
    h.query_all_by_label(label).find(|n| fields.contains(&n.accesskit_node().role()) && within.contains(n.rect().center())).map(|n| n.rect())
}

/// The number the node named `label` inside `within` shows, from its value text ("37 pt") or its numeric value.
fn shown_number(h: &H, label: &str, within: Rect) -> Option<f64> {
    h.query_all_by_label(label).filter(|n| within.contains(n.rect().center())).find_map(|n| {
        let node = n.accesskit_node();
        node.numeric_value().or_else(|| node.value().and_then(|v| v.split_whitespace().next().and_then(|v| v.parse().ok())))
    })
}

/// The screen above the canvas (the options bar) and the screen right of it (the panels).
fn bar_and_panels(h: &H) -> (Rect, Rect) {
    let canvas = h.state().last_canvas_rect;
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0));
    (Rect::from_min_max(screen.min, egui::pos2(screen.max.x, canvas.top())), Rect::from_min_max(egui::pos2(canvas.right(), canvas.top()), screen.max))
}

fn press(h: &mut H, at: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    h.run_steps(1);
}

/// Types `text` over the spin field at `at` and presses Enter.
fn type_into(h: &mut H, at: Pos2, text: &str) {
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    press(h, at, true);
    press(h, at, false);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

/// With no type target, a font size typed in the Character panel is the Type tool's default, set without a command,
/// and the options bar's size shows it; a size typed in the options bar shows in the Character panel. The pointer
/// stays over the field typed in, so only the content version runs the other region.
#[test]
fn a_type_default_typed_in_one_region_shows_in_the_other() {
    let mut h = harness(Tool::Type, None);
    crate::panel_docking::show_panel(h.state_mut(), "character", Some(true)).unwrap();
    h.run_steps(4);
    let (bar, panels) = bar_and_panels(&h);
    let in_panel = field_rect(&h, "Font size", panels).expect("the Character panel's Font size field");
    let in_bar = field_rect(&h, "Set font size", bar).expect("the options bar's font size field");

    type_into(&mut h, in_panel.center(), "37");
    assert_eq!(h.state().ui.tool_options.type_size, 37.0, "the typed size is the tool default");
    assert_eq!(shown_number(&h, "Set font size", bar), Some(37.0), "the options bar shows the size typed in the Character panel");

    type_into(&mut h, in_bar.center(), "41");
    assert_eq!(h.state().ui.tool_options.type_size, 41.0, "the typed size is the tool default");
    assert_eq!(shown_number(&h, "Font size", panels), Some(41.0), "the Character panel shows the size typed in the options bar");
}

/// With no type target, the colour of new type is the foreground, and a foreground set as the Color panel and the
/// Swatches set it (`tools.setColors`, no menu command) shows in the Character panel's Text color well.
#[test]
fn a_new_foreground_shows_in_the_character_panel() {
    let mut h = harness(Tool::Type, None);
    crate::panel_docking::show_panel(h.state_mut(), "character", Some(true)).unwrap();
    h.run_steps(4);
    let (_, panels) = bar_and_panels(&h);
    let well = |h: &H| {
        h.query_all_by_role(Role::ColorWell)
            .find(|n| n.accesskit_node().label().is_some_and(|l| l == "Text color") && panels.contains(n.rect().center()))
            .and_then(|n| n.accesskit_node().value().map(|v| v.to_string()))
    };
    assert_eq!(well(&h).as_deref(), Some("#000000"), "new type is black, the default foreground");
    h.state_mut().run("tools.setColors", json!({"foreground": [1.0, 0.0, 0.0, 1.0]})).unwrap();
    h.run_steps(2);
    assert_eq!(well(&h).as_deref(), Some("#ff0000"), "the Character panel shows the new foreground");
}

/// While a Properties slider of an adjustment layer is held, the Histogram counts the adjusted document and shows it
/// over the original: the live preview moves its version, and its own wakes land the count.
#[test]
fn the_histogram_follows_an_adjustment_drag() {
    let mut h = harness(Tool::Brush, Some("brightnessContrast"));
    crate::panel_docking::apply(h.state_mut(), vec![Action::set_iconic("histogram".to_owned(), false), Action::activate("histogram".to_owned())]).unwrap();
    h.run_steps(30);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0));
    let slider = node_rect(&h, Role::Slider, "Brightness", screen).expect("the Brightness slider");
    assert!(h.state().doc_hist.is_some(), "the Histogram counted the document");
    assert!(h.state().doc_hist_preview.is_none(), "no preview yet");
    let at = slider.center();
    h.event(Event::PointerMoved(at));
    h.run_steps(2);
    press(&mut h, at, true);
    for i in 1..=6 {
        h.event(Event::PointerMoved(at + vec2(4.0 * i as f32, 0.0)));
        h.run_steps(1);
    }
    assert!(h.state().live_adjust.is_some(), "the drag previews live");
    // The slider is held still: the count runs on a worker, and the panel's wakes land it.
    for _ in 0..240 {
        if h.state().doc_hist_preview.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(4));
        h.run_steps(1);
    }
    let preview = h.state().doc_hist_preview.as_ref().map(|e| e.key.preview);
    assert!(preview.is_some_and(|p| p != 0), "the Histogram shows the adjusted document while the slider is held");
    press(&mut h, at + vec2(24.0, 0.0), false);
}
