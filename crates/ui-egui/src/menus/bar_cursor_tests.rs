//! #16993: the menu titles sit on the title bar's move band, whose default open hand shows as Windows' four-way Move
//! cursor. The bar keeps the arrow over the titles and the free band, as an OS title bar does.

use craft::behavior::{Cursor, CursorIcon};
use craft::ui::runtime::cursor;
use egui::{Event, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use serde_json::json;

use crate::PhotocraftApp;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The whole app in a window with its own title bar and Windows' captions, whatever the OS or `PHOTOCRAFT_TITLEBAR`.
fn windowed() -> H {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        use craft::window::policy::{CaptionFamily, Captions, Decision, Desktop, DesktopKind, Frame};
        let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout: craft::window::Layout::default() });
        let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
        PhotocraftApp::setup_window(&cc.egui_ctx, crate::theme::AppTheme::pro(), Decision { frame, overridden: None, desktop }, None);
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app
    });
    h.run_steps(8);
    h
}

/// The cursor the frame settles on with the pointer at `at`, and the cursor the window shows.
fn cursor_at(h: &mut H, at: Pos2) -> (Option<Cursor>, egui::CursorIcon) {
    h.event(Event::PointerMoved(at));
    h.run_steps(2);
    (cursor::current(&h.ctx), h.output().platform_output.cursor_icon)
}

#[test]
fn the_title_bar_keeps_the_arrow_over_the_menu_titles_and_the_free_band() {
    let mut h = windowed();
    let file = h.get_by_label("File").rect();
    let help = h.get_by_label("Help").rect();
    let grab = Some(Cursor::System(CursorIcon::Grab));

    for (at, place) in
        [(file.center(), "the File title"), (help.center(), "the Help title"), (Pos2::new(help.right() + 40.0, help.center().y), "the free band")]
    {
        let (requested, shown) = cursor_at(&mut h, at);
        assert_ne!(requested, grab, "no open hand over {place}");
        assert_eq!(shown, egui::CursorIcon::Default, "the window shows the arrow over {place}");
    }
}
