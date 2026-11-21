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
