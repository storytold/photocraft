//! The Brush Preset picker mirrors in a right-to-left UI: the Size field hangs from the reading
//! end, the search row reads from the right, the gear and the other buttons hang from the reading
//! end too, and the resize grip sits in the bottom-left corner, where a drag to the left widens
//! the picker.
//!
//! The twins change only the direction: the same picker, the same brush and the same list in both
//! directions.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Event, Modifiers, PointerButton, Vec2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_engine::BrushSettings;
use photocraft_ui_egui::{PhotocraftApp, brush_picker, direction::{self, Dir}, theme};

/// The picker's content size in the tests.
fn content() -> Vec2 {
    vec2(300.0, 420.0)
}

/// The picker drawn in one direction: the harness, the frame's rect, the grip's id and the size
/// its last drag asked for.
struct Picker {
    harness: Harness<'static>,
    frame: Rc<RefCell<Option<egui::Rect>>>,
    grip: Rc<RefCell<Option<egui::Id>>>,
    dragged: Rc<RefCell<Option<Vec2>>>,
}

impl Picker {
    /// The picker drawn in `dir`.
    fn new(dir: Dir) -> Self {
        // The guard lives as long as the harness, so every frame is drawn in that direction.
        let guard = direction::direction_scope(dir);
        let mut brush = BrushSettings::default();
        let mut list = brush_picker::list_state();
        let frame = Rc::new(RefCell::new(None));
        let grip = Rc::new(RefCell::new(None));
        let dragged = Rc::new(RefCell::new(None));
        let (f, g, d) = (frame.clone(), grip.clone(), dragged.clone());
        let mut harness = Harness::builder()
            .with_size(vec2(800.0, 600.0))
            .build_ui(move |ui| {
                let _dir = &guard;
                let shown = egui::Frame::popup(ui.style()).show(ui, |ui| {
                    brush_picker::body(ui, &mut brush, &[], None, &mut list, content());
                });
                *f.borrow_mut() = Some(shown.response.rect);
                *g.borrow_mut() = Some(ui.scope_id().with("brush-picker-resize"));
                *d.borrow_mut() = brush_picker::resize_grip(ui, shown.response.rect, content());
            });
        harness.run_steps(2);
        // The fonts and the SVG icon loader, and the egui direction flag to match the scope's.
        PhotocraftApp::setup_context(&harness.ctx, Default::default());
        theme::set_direction(&harness.ctx, dir == Dir::Rtl);
        harness.run_steps(2);
        Self { harness, frame, grip, dragged }
    }

    /// The rect of the picker's frame.
    fn frame_rect(&self) -> egui::Rect {
        self.frame.borrow().clone().expect("the picker's frame")
    }

    /// The rect of the picker's resize grip.
    fn grip_rect(&self) -> egui::Rect {
        let id = self.grip.borrow().clone().expect("the resize grip");
        self.harness.ctx.read_response(id).expect("the grip's response").rect
    }
}

/// The absolute x of the painted text `text`, if the picker painted it.
fn text_x(p: &Picker, text: &str) -> Option<f32> {
    let mut found = None;
    for clipped in &p.harness.output().shapes {
        walk_text(&clipped.shape, text, &mut found);
    }
    found
}

fn walk_text(shape: &egui::Shape, text: &str, found: &mut Option<f32>) {
    match shape {
        egui::Shape::Text(t) if t.galley.text() == text && found.is_none() => {
            let dx = t.galley.rows.first().and_then(|r| r.glyphs.first()).map_or(0.0, |g| g.pos.x);
            *found = Some(t.pos.x + dx);
        }
        egui::Shape::Vec(v) => v.iter().for_each(|s| walk_text(s, text, found)),
        _ => {}
    }
}

/// The rect of the button with the accessible name `label`.
fn button_rect(p: &Picker, label: &str) -> egui::Rect {
    p.harness.get_by_label(label).rect()
}

/// The x of the painted text `text`; panics when the picker did not paint it.
fn must_x(p: &Picker, text: &str) -> f32 {
    text_x(p, text).unwrap_or_else(|| panic!("the picker painted the text {text:?}"))
}

/// The Size field hangs from the reading end of its row: the right in English, the left in Arabic.
#[test]
fn the_size_field_hangs_from_the_reading_end() {
    {
        let ltr = Picker::new(Dir::Ltr);
        let field = must_x(&ltr, "px");
        let caption = must_x(&ltr, "Size");
        assert!(field > caption, "LTR: the field sits right of the caption: {field} vs {caption}");
    }
    {
        let rtl = Picker::new(Dir::Rtl);
        let field = must_x(&rtl, "px");
        let caption = must_x(&rtl, "Size");
        assert!(field < caption, "RTL: the field sits left of the caption: {field} vs {caption}");
    }
}

/// The search row reads from the reading end: in Arabic the gear, new-preset and settings buttons
/// hang from the left of the search field, in English from its right, the gear outermost.
#[test]
fn the_search_row_reads_from_the_right() {
    {
        let ltr = Picker::new(Dir::Ltr);
        let field = must_x(&ltr, "Search Brushes");
        let gear = button_rect(&ltr, "Brush Preset Options").center().x;
        let new_preset = button_rect(&ltr, "New Brush Preset").center().x;
        let settings = button_rect(&ltr, "Brush Settings…").center().x;
        assert!(gear > field && new_preset > field && settings > field, "LTR: the buttons sit right of the field at {field}: {gear}, {new_preset}, {settings}");
        assert!(gear > new_preset && new_preset > settings, "LTR: the gear is the outermost button: {gear} > {new_preset} > {settings}");
    }
    {
        let rtl = Picker::new(Dir::Rtl);
        let field = must_x(&rtl, "Search Brushes");
        let gear = button_rect(&rtl, "Brush Preset Options").center().x;
        let new_preset = button_rect(&rtl, "New Brush Preset").center().x;
        let settings = button_rect(&rtl, "Brush Settings…").center().x;
        assert!(gear < field && new_preset < field && settings < field, "RTL: the buttons sit left of the field at {field}: {gear}, {new_preset}, {settings}");
        assert!(gear < new_preset && new_preset < settings, "RTL: the gear is the outermost button: {gear} < {new_preset} < {settings}");
    }
}

/// The gear hangs from the reading end of the picker itself: flush with the right edge in English,
/// with the left edge in Arabic.
#[test]
fn the_gear_button_hangs_from_the_reading_end() {
    {
        let ltr = Picker::new(Dir::Ltr);
        let frame = ltr.frame_rect();
        let gear = button_rect(&ltr, "Brush Preset Options");
        assert!(frame.right() - gear.right() < 24.0, "LTR: the gear hangs from the right edge: {gear:?} in {frame:?}");
    }
    {
        let rtl = Picker::new(Dir::Rtl);
        let frame = rtl.frame_rect();
        let gear = button_rect(&rtl, "Brush Preset Options");
        assert!(gear.left() - frame.left() < 24.0, "RTL: the gear hangs from the left edge: {gear:?} in {frame:?}");
    }
}

/// The resize grip sits in the bottom-right corner in English and the bottom-left in Arabic, with
/// the matching resize cursor.
#[test]
fn the_resize_grip_sits_in_the_reading_end_corner() {
    {
        let mut ltr = Picker::new(Dir::Ltr);
        let frame = ltr.frame_rect();
        let grip = ltr.grip_rect();
        assert!((grip.right() - frame.right()).abs() < 0.5, "LTR: the grip is in the bottom-right corner: {grip:?} in {frame:?}");
        assert!((grip.bottom() - frame.bottom()).abs() < 0.5, "LTR: the grip is at the bottom: {grip:?} in {frame:?}");
        ltr.harness.hover_at(grip.center());
        ltr.harness.run_steps(1);
        assert_eq!(ltr.harness.output().platform_output.cursor_icon, egui::CursorIcon::ResizeNwSe, "LTR: the cursor points the way the picker grows");
    }
    {
        let mut rtl = Picker::new(Dir::Rtl);
        let frame = rtl.frame_rect();
        let grip = rtl.grip_rect();
        assert!((grip.left() - frame.left()).abs() < 0.5, "RTL: the grip is in the bottom-left corner: {grip:?} in {frame:?}");
        assert!((grip.bottom() - frame.bottom()).abs() < 0.5, "RTL: the grip is at the bottom: {grip:?} in {frame:?}");
        rtl.harness.hover_at(grip.center());
        rtl.harness.run_steps(1);
        assert_eq!(rtl.harness.output().platform_output.cursor_icon, egui::CursorIcon::ResizeNeSw, "RTL: the cursor points the way the picker grows");
    }
}

/// The grip's drag follows its corner: to the right in English, to the left in Arabic, and both
/// widen the picker.
#[test]
fn the_grip_drag_follows_its_corner() {
    {
        let mut ltr = Picker::new(Dir::Ltr);
        drag(&mut ltr, vec2(40.0, 0.0));
        let size = ltr.dragged.borrow().expect("the drag asked for a size");
        assert!(size.x > content().x, "LTR: a drag to the right widens the picker: {size:?}");
    }
    {
        let mut rtl = Picker::new(Dir::Rtl);
        drag(&mut rtl, vec2(-40.0, 0.0));
        let size = rtl.dragged.borrow().expect("the drag asked for a size");
        assert!(size.x > content().x, "RTL: a drag to the left widens the picker: {size:?}");
    }
}

/// Drag the picker's grip by `by`, from its corner.
fn drag(p: &mut Picker, by: Vec2) {
    let at = p.grip_rect().center();
    p.harness.hover_at(at);
    p.harness.run_steps(1);
    p.harness.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    p.harness.run_steps(1);
    p.harness.event(Event::PointerMoved(at + by));
    p.harness.run_steps(2);
}
