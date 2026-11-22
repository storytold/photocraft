//! #15527: the Home screen uses the framework's Home layout, with Home and Learn in a left rail beside the content,
//! the community links at the bottom of that rail and the PhotoCraft title above the page.
use crate::PhotocraftApp;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

type H = Harness<'static, Box<PhotocraftApp>>;

const DROP_HINT: &str = "Drop an image or PSD anywhere to open it.";

fn harness() -> H {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| super::start_screen(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// The rectangle of the first node labelled `label`.
fn rect(h: &H, label: &str) -> egui::Rect {
    h.query_all_by_label(label).next().unwrap_or_else(|| panic!("{label} is shown")).rect()
}

#[test]
fn home_and_the_links_sit_in_a_rail_left_of_the_content_at_desktop_width() {
    let h = harness();
    let hint = rect(&h, DROP_HINT);
    let title = rect(&h, "PhotoCraft");
    for label in ["Home", "Learn", "Website", "Discord", "GitHub"] {
        let link = rect(&h, label);
        assert!(link.right() <= hint.left(), "{label} {link:?} must be in the rail left of the drop hint {hint:?}");
        assert!(link.right() <= title.left(), "{label} {link:?} must be left of the title {title:?}");
    }
    assert!(rect(&h, "Home").bottom() <= rect(&h, "Learn").top(), "Learn follows Home");
    let middle = h.ctx.content_rect().center().y;
    for label in ["Website", "Discord", "GitHub"] {
        assert!(rect(&h, label).top() > middle, "{label} sits at the bottom of the rail");
    }
    assert!(title.bottom() <= hint.top(), "the title is above the page");
}

/// The width of the first recent card on a Home screen `width` points wide, with six recent files.
fn card_width(width: f32) -> f32 {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    for n in 1..=6 {
        app.push_recent(&format!("/tmp/photocraft-home/card-{n}.png"));
    }
    let mut h = Harness::builder()
        .with_size(egui::vec2(width, 700.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| super::start_screen(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    rect(&h, "card-6.png").width()
}

/// #16172: the recent cards keep one size, as many to a row as fit, as before the framework move; they do not grow
/// to fill a wider window.
#[test]
fn recent_cards_keep_one_size_in_a_wider_window() {
    let (narrow, wide) = (card_width(1100.0), card_width(1600.0));
    assert!((narrow - wide).abs() < 1.0, "a card is {narrow} points wide at 1100 and {wide} at 1600");
    assert!(wide < 300.0, "a card is a card, not a share of the row: {wide}");
}
