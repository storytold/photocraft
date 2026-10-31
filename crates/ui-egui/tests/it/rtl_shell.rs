//! The shell's top bar in a right-to-left UI: the document title's base text direction follows
//! the UI language (the bar's chrome geometry is covered by the `panels::titlebar` tests).

use egui::{Pos2, RawInput, Rect, vec2};
use photocraft_engine::Session;
use photocraft_ui_egui::{
    PhotocraftApp,
    direction::{self, Dir},
    panels,
};
use serde_json::json;

/// A document whose name mixes Arabic and Latin, so the title's base direction is observable.
const NAME: &str = "لاشي مكتوب.png";

/// The start of the names here, so the title's galley is told from the bar's other text.
const NAME_START: &str = "لاشي";

/// A name far too long for the free gap between the menus and the controls, so the title bar
/// elides it.
const LONG_NAME: &str = concat!(
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
    "لاشي مكتوب لاشي مكتوب ",
);

/// A headless app showing its title bar, like the unit `Win` harness in `panels::titlebar::tests`.
struct Bar {
    ctx: egui::Context,
    app: PhotocraftApp,
}

impl Bar {
    fn new(name: &str) -> Self {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let mut app = PhotocraftApp::new(Session::new(), Default::default());
        app.custom_titlebar = true;
        app.run("file.new", json!({ "width": 64, "height": 64, "name": name })).expect("new document");
        Self { ctx, app }
    }

    fn frame(&mut self) -> egui::FullOutput {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 600.0))), ..Default::default() };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| panels::title_bar(app, ui));
        // The headless test never uploads the font atlas.
        out.textures_delta.clear();
        out
    }
}

/// The x of glyph `c` in the painted title galley (the only shape whose text starts with the
/// document's name), if it paints that glyph.
fn title_x_of(out: &egui::FullOutput, c: char) -> Option<f32> {
    fn walk(s: &egui::Shape, c: char, found: &mut Option<f32>) {
        match s {
            egui::Shape::Text(t) => {
                if t.galley.text().starts_with(NAME_START) && found.is_none() {
                    *found = t.galley.rows.iter().flat_map(|r| r.glyphs.iter()).find(|g| g.chr == c).map(|g| g.pos.x);
                }
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, c, found)),
            _ => {}
        }
    }
    let mut found = None;
    out.shapes.iter().for_each(|cs| walk(&cs.shape, c, &mut found));
    found
}

/// The x of glyph `c` in the painted title galley.
fn x_of(out: &egui::FullOutput, c: char) -> f32 {
    title_x_of(out, c).unwrap_or_else(|| panic!("the title galley paints the glyph {c:?}"))
}

#[test]
fn the_title_keeps_an_ltr_base_in_an_ltr_ui() {
    let _ltr = direction::direction_scope(Dir::Ltr);
    let mut bar = Bar::new(NAME);
    let out = bar.frame();
    assert!(x_of(&out, 'g') > x_of(&out, 'ل'), "LTR base: \".png\" at the visual right of the name");
    assert!(x_of(&out, 'ل') > x_of(&out, 'م'), "the Arabic name still reads right to left");
}

#[test]
fn the_title_reads_right_to_left_in_an_rtl_ui() {
    let _rtl = direction::direction_scope(Dir::Rtl);
    let mut bar = Bar::new(NAME);
    let out = bar.frame();
    assert!(x_of(&out, 'ل') > x_of(&out, 'g'), "RTL base: the name leads from the right, \".png\" at the left");
    assert!(x_of(&out, 'ل') > x_of(&out, 'م'), "the Arabic name still reads right to left");
}

#[test]
fn the_elided_title_ends_at_the_visual_end_in_an_ltr_ui() {
    let _ltr = direction::direction_scope(Dir::Ltr);
    let mut bar = Bar::new(LONG_NAME);
    let out = bar.frame();
    let ellipsis = title_x_of(&out, '\u{2026}').expect("the elided title paints the overflow character");
    let first = title_x_of(&out, 'ل').expect("the elided title paints the start of the name");
    assert!(ellipsis > first, "LTR: the overflow character ends the name at the visual right");
}

#[test]
fn the_elided_title_ends_at_the_visual_end_in_an_rtl_ui() {
    let _rtl = direction::direction_scope(Dir::Rtl);
    let mut bar = Bar::new(LONG_NAME);
    let out = bar.frame();
    let ellipsis = title_x_of(&out, '\u{2026}').expect("the elided title paints the overflow character");
    let first = title_x_of(&out, 'ل').expect("the elided title paints the start of the name");
    assert!(ellipsis < first, "RTL: the overflow character ends the name at the visual left");
}
