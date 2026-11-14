//! The Custom Shape picker's gear menu, right-click menu and view modes (#13468): the reference
//! editor's entries in its order with its separators, which entries are greyed, and what each does.

use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::*;
use crate::preset_panels::PresetUi;

/// The gear menu, measured top to bottom; "―" is a separator.
const GEAR: &[&str] = &[
    "Rename Shape…",
    "Delete Shape",
    "―",
    "Text Only",
    "Small Thumbnail",
    "Large Thumbnail",
    "Small List",
    "Large List",
    "―",
    "Append Default Shapes…",
    "Import Shapes…",
    "Export Selected Shapes…",
];

/// The right-click menu, measured top to bottom.
const CONTEXT: &[&str] = &["Append Default Shapes…", "Import Shapes…", "Export Selected Shapes…", "Rename Shape…", "Delete Shape…", "―", "New Shape Group…"];

fn harness() -> Harness<'static, PhotocraftApp> {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| {
                ui.horizontal(|ui| crate::preset_panels::shape_picker(app, ui));
            });
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

fn group_names(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    photocraft_engine::presets::shapes::all_groups(&h.state().session).iter().map(|g| g.name.clone()).collect()
}

fn names(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    photocraft_engine::presets::shapes::all_groups(&h.state().session).iter().flat_map(|g| g.items.iter().map(|s| s.name.clone())).collect()
}

/// A built-in shape drawn at the top of the picker, so a click on it lands in view.
fn first_shape(h: &Harness<'static, PhotocraftApp>) -> String {
    names(h).into_iter().next().expect("the default shapes")
}

/// [`harness`] with the first shape picked as the Custom Shape tool's shape.
fn harness_with_shape() -> (Harness<'static, PhotocraftApp>, String) {
    let mut h = harness();
    let name = first_shape(&h);
    h.state_mut().ui.presets_ui.custom_shape = name.clone();
    h.run_steps(2);
    (h, name)
}

fn open_picker(h: &mut Harness<'static, PhotocraftApp>) {
    h.get_by_label("Custom Shape Picker").click();
    h.run_steps(3);
}

fn open_gear(h: &mut Harness<'static, PhotocraftApp>) {
    open_picker(h);
    h.get_by_label("Custom Shape Options").click();
    h.run_steps(3);
}

fn open_context(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    open_picker(h);
    h.get_by_label(label).click_secondary();
    h.run_steps(3);
}

fn labels(menu: &[Option<ShapeMenu>], context: bool) -> Vec<&'static str> {
    menu.iter().map(|e| e.map_or("―", |e| e.label(context))).collect()
}

fn entries(menu: &[&'static str]) -> Vec<&'static str> {
    menu.iter().copied().filter(|l| *l != "―").collect()
}

fn disabled(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

fn tops(h: &Harness<'static, PhotocraftApp>, menu: &[&'static str]) -> Vec<f32> {
    entries(menu).iter().map(|l| h.get_by_label(l).rect().top()).collect()
}

#[test]
fn the_gear_menu_lists_the_reference_entries_in_order_with_its_separators() {
    assert_eq!(labels(SHAPE_GEAR_MENU, false), GEAR);
    let (mut h, _) = harness_with_shape();
    open_gear(&mut h);
    let tops = tops(&h, GEAR);
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "drawn top to bottom in the measured order: {tops:?}");
    assert!(h.query_all_by_label("Delete Shape…").next().is_none(), "the gear menu's Delete Shape has no ellipsis");
}

#[test]
fn the_right_click_menu_lists_the_reference_entries_in_order_and_opens_on_a_shape_or_a_group() {
    assert_eq!(labels(SHAPE_CONTEXT_MENU, true), CONTEXT);
    let (mut h, name) = harness_with_shape();
    open_context(&mut h, &name);
    let tops = tops(&h, CONTEXT);
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "drawn top to bottom in the measured order: {tops:?}");
    assert!(h.query_all_by_label("Delete Shape").next().is_none(), "the right-click menu's Delete Shape has an ellipsis");
    assert!(h.query_by_label(&name).is_some(), "the picker stays open under the menu");
    // A group's caption opens it too.
    let (mut h, _) = harness_with_shape();
    let group = group_names(&h).into_iter().next().expect("a group");
    open_context(&mut h, &group);
    assert!(h.query_by_label("New Shape Group…").is_some());
}

#[test]
fn rename_delete_and_export_are_greyed_as_the_reference_editor_greys_them() {
    let (mut h, _) = harness_with_shape();
    open_gear(&mut h);
    assert!(!disabled(&h, "Rename Shape…") && !disabled(&h, "Delete Shape"), "a shape is selected");
    assert!(disabled(&h, "Export Selected Shapes…"), "there is no shape export yet");
    for live in ["Append Default Shapes…", "Import Shapes…", "Text Only", "Large List"] {
        assert!(!disabled(&h, live), "{live}");
    }
    // A current shape that is not in the library is no selection.
    let mut h = harness();
    h.state_mut().ui.presets_ui.custom_shape = "No Such Shape".into();
    h.run_steps(2);
    open_gear(&mut h);
    assert!(disabled(&h, "Rename Shape…") && disabled(&h, "Delete Shape") && disabled(&h, "Export Selected Shapes…"));
    assert!(!disabled(&h, "Import Shapes…"));
    // The right-click menu on a group's caption has no target then either; New Shape Group stays on.
    let mut h = harness();
    h.state_mut().ui.presets_ui.custom_shape = "No Such Shape".into();
    let group = group_names(&h).into_iter().next().expect("a group");
    open_context(&mut h, &group);
    assert!(disabled(&h, "Rename Shape…") && disabled(&h, "Delete Shape…") && disabled(&h, "Export Selected Shapes…"));
    assert!(!disabled(&h, "New Shape Group…") && !disabled(&h, "Append Default Shapes…"));
}

#[test]
fn export_selected_shapes_is_greyed_in_both_menus() {
    let (mut h, name) = harness_with_shape();
    open_gear(&mut h);
    assert!(disabled(&h, "Export Selected Shapes…"));
    let (mut h, _) = harness_with_shape();
    open_context(&mut h, &name);
    assert!(disabled(&h, "Export Selected Shapes…"));
}

#[test]
fn the_view_is_one_choice_defaulting_to_small_thumbnail_and_the_picker_stays_open() {
    let (mut h, name) = harness_with_shape();
    assert_eq!(h.state().ui.presets_ui.shape_picker_view, ShapeView::SmallThumbnail);
    open_gear(&mut h);
    for label in ["Text Only", "Small Thumbnail", "Large Thumbnail", "Small List", "Large List"] {
        let on = h.get_by_label(label).accesskit_node().toggled() == Some(Toggled::True);
        assert_eq!(on, label == "Small Thumbnail", "{label}");
    }
    h.get_by_label("Large List").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.presets_ui.shape_picker_view, ShapeView::LargeList);
    assert!(h.query_by_label(&name).is_some(), "the picker is still open, now as a list");
    // Every view lists the shapes and picking one closes the picker.
    for view in [ShapeView::TextOnly, ShapeView::SmallThumbnail, ShapeView::LargeThumbnail, ShapeView::SmallList, ShapeView::LargeList] {
        let (mut h, name) = harness_with_shape();
        h.state_mut().ui.presets_ui.shape_picker_view = view;
        h.run_steps(2);
        open_picker(&mut h);
        let other = names(&h).get(1).cloned().expect("a second shape");
        h.get_by_label(&other).click();
        h.run_steps(3);
        assert_eq!(h.state().ui.presets_ui.custom_shape, other, "{view:?}");
        assert_ne!(other, name);
        assert!(h.query_by_label(&other).is_none(), "{view:?}: the picker closed");
    }
}

#[test]
fn the_view_is_saved_and_a_bad_saved_value_falls_back_to_the_default() {
    let mut ui = PresetUi { shape_picker_view: ShapeView::LargeThumbnail, ..Default::default() };
    let v = serde_json::to_value(&ui).unwrap();
    assert_eq!(v["shapePickerView"], json!("largeThumbnail"));
    let back: PresetUi = serde_json::from_value(v).unwrap();
    assert_eq!(back.shape_picker_view, ShapeView::LargeThumbnail);
    for bad in [json!("sideways"), json!(7), json!(null), json!({"a": 1})] {
        let back: PresetUi = serde_json::from_value(json!({"shapePickerView": bad})).unwrap();
        assert_eq!(back.shape_picker_view, ShapeView::SmallThumbnail);
    }
    ui.shape_picker_view = ShapeView::default();
    assert_eq!(serde_json::from_value::<PresetUi>(json!({})).unwrap().shape_picker_view, ui.shape_picker_view);
}

#[test]
fn rename_shape_renames_it_in_the_library_and_the_tool_follows() {
    let (mut h, name) = harness_with_shape();
    open_gear(&mut h);
    h.get_by_label("Rename Shape…").click();
    h.run_steps(3);
    assert!(h.state().ui.presets_ui.renaming.as_ref().is_some_and(|r| r.0 == "shapePicker" && r.1 == name), "the picker's own rename row");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_role(Role::TextInput).type_text("Renamed Shape");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let all = names(&h);
    assert!(all.iter().any(|n| n == "Renamed Shape") && !all.contains(&name), "renamed in the library");
    assert_eq!(h.state().ui.presets_ui.custom_shape, "Renamed Shape", "the tool's shape follows");
    assert!(h.state().ui.presets_ui.renaming.is_none());
}

#[test]
fn delete_shape_removes_it_and_the_tool_falls_back_to_the_default() {
    let (mut h, name) = harness_with_shape();
    open_gear(&mut h);
    h.get_by_label("Delete Shape").click();
    h.run_steps(3);
    assert!(!names(&h).contains(&name), "{name} is deleted");
    assert!(h.state().ui.presets_ui.custom_shape.is_empty(), "the tool's shape is cleared");
    // The right-click menu deletes the shape it was opened on, not the current one.
    let (mut h, current) = harness_with_shape();
    let other = names(&h).get(1).cloned().expect("a second shape");
    open_context(&mut h, &other);
    h.get_by_label("Delete Shape…").click();
    h.run_steps(3);
    assert!(!names(&h).contains(&other));
    assert!(names(&h).contains(&current));
    assert_eq!(h.state().ui.presets_ui.custom_shape, current);
}

#[test]
fn new_shape_group_and_append_default_shapes_add_groups() {
    let (mut h, name) = harness_with_shape();
    let before = group_names(&h).len();
    open_context(&mut h, &name);
    h.get_by_label("New Shape Group…").click();
    h.run_steps(3);
    assert_eq!(group_names(&h).len(), before + 1, "an empty group");
    assert!(h.state().ui.presets_ui.renaming.as_ref().is_some_and(|r| r.0 == "shapePicker" && r.1.starts_with("group:")));
    let (mut h, _) = harness_with_shape();
    let before = group_names(&h).len();
    open_gear(&mut h);
    h.get_by_label("Append Default Shapes…").click();
    h.run_steps(3);
    assert!(group_names(&h).len() > before, "the defaults are appended");
}

/// A pointer click (not an assistive-technology one) on a menu entry must not close the picker.
#[test]
fn a_pointer_click_on_a_gear_entry_keeps_the_picker_open() {
    let (mut h, name) = harness_with_shape();
    open_gear(&mut h);
    let at = h.get_by_label("Small List").rect().center();
    for ev in [
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
    ] {
        h.input_mut().events.push(ev);
        h.run_steps(1);
    }
    h.run_steps(3);
    assert_eq!(h.state().ui.presets_ui.shape_picker_view, ShapeView::SmallList);
    assert!(h.query_by_label(&name).is_some(), "the picker is still open");
    assert!(h.query_by_label("Large List").is_none(), "the menu closed");
}
