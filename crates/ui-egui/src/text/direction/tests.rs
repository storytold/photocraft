//! Right-to-left shell layout: the direction rules, and where the mirrored chrome lands in a
//! running shell. The shell tests force the direction with `direction_scope`, so they don't
//! depend on a right-to-left language being selected.

use super::*;
use crate::PhotocraftApp;
use crate::i18n::{self, Lang};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

#[test]
fn direction_follows_the_language_unless_forced() {
    let _lang = i18n::language_scope(Lang::EN);
    let forced = direction_scope(Dir::Rtl);
    assert!(is_rtl());
    {
        let _inner = direction_scope(Dir::Ltr);
        assert!(!is_rtl());
    }
    assert!(is_rtl(), "the inner scope restores the outer one");
    drop(forced);
    assert_eq!(FORCED.get(), None);
}

#[test]
fn sides_and_rows_mirror() {
    {
        let _ltr = direction_scope(Dir::Ltr);
        assert_eq!(row().main_dir(), egui::Direction::LeftToRight);
        assert_eq!(trailing().main_dir(), egui::Direction::RightToLeft);
        assert_eq!(below(), egui::RectAlign::BOTTOM_START);
    }
    let _rtl = direction_scope(Dir::Rtl);
    assert_eq!(row().main_dir(), egui::Direction::RightToLeft);
    assert_eq!(trailing().main_dir(), egui::Direction::LeftToRight);
    assert_eq!(below(), egui::RectAlign::BOTTOM_END);
}

#[test]
fn body_and_footer_layouts_mirror() {
    {
        let _ltr = direction_scope(Dir::Ltr);
        assert_eq!(body_layout().main_dir(), egui::Direction::TopDown);
        assert_eq!(body_layout().cross_align(), Align::Min);
        assert_eq!(trailing_min().main_dir(), egui::Direction::RightToLeft);
        assert_eq!(trailing_min().cross_align(), Align::Min);
        assert_eq!(horizontal_nav_keys(), (egui::Key::ArrowRight, egui::Key::ArrowLeft));
    }
    let _rtl = direction_scope(Dir::Rtl);
    // Vertical + `Align::Max` is what makes egui mirror the nested rows of a body.
    assert_eq!(body_layout().main_dir(), egui::Direction::TopDown);
    assert_eq!(body_layout().cross_align(), Align::Max);
    assert_eq!(trailing_min().main_dir(), egui::Direction::LeftToRight);
    assert_eq!(trailing_min().cross_align(), Align::Min);
    assert_eq!(horizontal_nav_keys(), (egui::Key::ArrowLeft, egui::Key::ArrowRight));
}

#[test]
fn env_values_parse_strictly() {
    assert_eq!(parse_dir("rtl"), Some(Dir::Rtl));
    assert_eq!(parse_dir(" RTL "), Some(Dir::Rtl));
    assert_eq!(parse_dir("ltr"), Some(Dir::Ltr));
    assert_eq!(parse_dir(""), None);
    assert_eq!(parse_dir("ar"), None);
}

/// The egui direction flag ([`crate::theme::set_direction`]) must follow the direction scope so
/// every egui-owned widget (submenus, popups, scrollbars, window chrome) mirrors with it.
#[test]
fn right_to_left_flag_follows_direction() {
    let _lang = i18n::language_scope(Lang::EN);
    for (dir, want) in [(Dir::Ltr, false), (Dir::Rtl, true)] {
        let _dir = direction_scope(dir);
        let h = harness();
        assert_eq!(h.ctx.global_style().right_to_left, want, "{dir:?}");
    }
}

/// A committed language change flips the flag in the same frame (i18n::sync_context), in both
/// egui theme branches so an appearance switch cannot revert the direction.
#[test]
fn the_flag_follows_a_live_language_switch() {
    let Some(_) = photocraft_text::craft_fonts::CRAFT_FONTS.iter().find(|f| f.family == "Noto Sans Arabic") else {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        return;
    };
    let _lang = i18n::language_scope(Lang::EN);
    let mut h = harness();
    assert!(!h.ctx.global_style().right_to_left, "English starts left-to-right");
    h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "ar"})).expect("language");
    h.run_steps(4);
    assert!(h.ctx.global_style().right_to_left, "switching to Arabic flips the flag");
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        assert!(h.ctx.style_of(theme).right_to_left, "{theme:?} branch");
    }
    h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "en"})).expect("language");
    h.run_steps(4);
    assert!(!h.ctx.global_style().right_to_left, "switching back restores left-to-right");
}

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ProMedium);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).expect("new document");
        app
    });
    h.run_steps(4);
    h
}

/// Centre x of the widget labelled `label` (the first, when several are).
fn x_of(h: &Harness<'static, PhotocraftApp>, label: &str) -> f32 {
    h.query_all_by_label(label).next().unwrap_or_else(|| panic!("`{label}` drawn")).rect().center().x
}

/// Left x of the text `text` where it is painted (for widgets without an accessible label).
fn text_x(h: &Harness<'static, PhotocraftApp>, text: &str) -> f32 {
    fn find(shape: &egui::Shape, text: &str) -> Option<f32> {
        match shape {
            egui::Shape::Text(t) if t.galley.job.text == text => Some(t.pos.x),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
            _ => None,
        }
    }
    h.output().shapes.iter().find_map(|s| find(&s.shape, text)).unwrap_or_else(|| panic!("`{text}` painted"))
}

fn dock_x(h: &Harness<'static, PhotocraftApp>) -> f32 {
    crate::panel_docking::probe::panel_rect(&h.ctx, "layers").expect("Layers docked").center().x
}

#[test]
fn the_shell_mirrors_in_a_right_to_left_ui() {
    let _lang = i18n::language_scope(Lang::EN);
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let h = harness();
        let mid = 600.0;
        let (file, help, toolbar, dock, workspace) = (x_of(&h, "File"), x_of(&h, "Help"), x_of(&h, "Toolbar"), dock_x(&h), text_x(&h, "Essentials"));
        if dir == Dir::Rtl {
            assert!(file > help, "File starts the menu bar at the right: {file} vs {help}");
            assert!(toolbar > mid && dock < mid, "toolbox right, dock left: {toolbar}, {dock}");
            assert!(workspace < help, "the workspace switcher is at the far (left) end: {workspace}");
        } else {
            assert!(file < help, "{file} vs {help}");
            assert!(toolbar < mid && dock > mid, "{toolbar}, {dock}");
            assert!(workspace > help, "{workspace}");
        }
    }
}

#[test]
fn arrow_keys_step_through_the_menu_bar_in_screen_order() {
    let _lang = i18n::language_scope(Lang::EN);
    for (dir, towards_edit) in [(Dir::Ltr, egui::Key::ArrowRight), (Dir::Rtl, egui::Key::ArrowLeft)] {
        let _dir = direction_scope(dir);
        let mut h = harness();
        h.get_by_label("File").click();
        h.run_steps(4);
        assert!(h.query_by_label_contains("Undo").is_none(), "File is open, not Edit");
        // Edit is File's neighbour on screen: to its right, or to its left in a right-to-left UI.
        h.key_press(towards_edit);
        h.run_steps(4);
        assert!(h.query_by_label_contains("Undo").is_some(), "{dir:?}: {towards_edit:?} opens Edit");
    }
}

#[test]
fn menus_open_under_the_title_s_reading_start_edge() {
    let _lang = i18n::language_scope(Lang::EN);
    let _dir = direction_scope(Dir::Rtl);
    let mut h = harness();
    let edit = h.get_by_label("Edit").rect();
    h.get_by_label("Edit").click();
    h.run_steps(4);
    let undo = h.get_by_label_contains("Undo").rect();
    assert!(undo.right() <= edit.right() + 1.0 && undo.right() > edit.right() - 40.0, "the menu hangs left from Edit's right edge: {undo:?} {edit:?}");
}

#[test]
fn hand_painted_x_helpers_mirror_the_edges() {
    let rect = egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(300.0, 22.0));
    {
        let _ltr = direction_scope(Dir::Ltr);
        assert_eq!(step(), 1.0);
        assert_eq!(start_x(rect, 6.0), 106.0);
        assert_eq!(start_x_w(rect, 6.0, 22.0), 106.0);
        assert_eq!(end_x(rect, 10.0), 390.0);
    }
    let _rtl = direction_scope(Dir::Rtl);
    assert_eq!(step(), -1.0);
    assert_eq!(start_x(rect, 6.0), 394.0);
    assert_eq!(start_x_w(rect, 6.0, 22.0), 372.0);
    assert_eq!(end_x(rect, 10.0), 110.0);
}

#[test]
fn hand_painted_x_helpers_mirror_labels_aligns_and_glyphs() {
    let rect = egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(300.0, 22.0));
    {
        let _ltr = direction_scope(Dir::Ltr);
        assert_eq!(after_x(rect, 10.0, 30.0), 410.0);
        assert_eq!(start_align(), egui::Align2::LEFT_CENTER);
        assert_eq!(end_align(), egui::Align2::RIGHT_CENTER);
        assert!(start_side(105.0, 110.0));
        assert!(!start_side(115.0, 110.0));
        assert_eq!(glyph_x(102.0, 106.0), 102.0);
    }
    let _rtl = direction_scope(Dir::Rtl);
    assert_eq!(after_x(rect, 10.0, 30.0), 60.0);
    assert_eq!(after_x(rect, 10.0, 0.0), 90.0);
    assert_eq!(start_align(), egui::Align2::RIGHT_CENTER);
    assert_eq!(end_align(), egui::Align2::LEFT_CENTER);
    assert!(start_side(115.0, 110.0));
    assert!(!start_side(105.0, 110.0));
    assert_eq!(glyph_x(102.0, 106.0), 110.0);
}

/// `popup_body` re-scopes the body right-to-left under RTL — justified at `Align::Min`, pinned to
/// the parent `max_rect` — and keeps the popup's own layout left-to-right.
#[test]
fn popup_body_mirrors_only_under_rtl() {
    for (dir, mirrored) in [(Dir::Ltr, false), (Dir::Rtl, true)] {
        let _dir = direction_scope(dir);
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| s.right_to_left = dir == Dir::Rtl);
        let mut inner = None;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.allocate_ui_with_layout(egui::vec2(300.0, 200.0), Layout::left_to_right(Align::Center), |ui| {
                popup_body(ui, |ui| {
                    inner = Some((ui.layout().main_dir(), ui.layout().cross_align(), ui.max_rect().width()));
                    ui.label("Undo");
                });
            });
        });
        out.textures_delta.clear();
        let (main_dir, cross, max_w) = inner.expect("popup_body ran its contents");
        if mirrored {
            assert_eq!(main_dir, egui::Direction::TopDown, "{dir:?}");
            assert_eq!(cross, Align::Min, "{dir:?}: the cross align stays Min (Max ballooned popup width)");
            assert_eq!(max_w, 300.0, "{dir:?}: the scope is pinned to the parent max_rect");
        } else {
            assert_eq!(main_dir, egui::Direction::LeftToRight, "{dir:?}: LTR keeps the popup's own layout");
        }
    }
}

/// The RTL `popup_body` scope must not balloon the body's `min_rect` past the pinned width: the
/// cross align stays `Min`, so the popup measures the rows' width, not the full area width.
#[test]
fn popup_body_rtl_min_rect_stays_inside_the_pinned_width() {
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| s.right_to_left = dir == Dir::Rtl);
        let mut width = 0.0;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.allocate_ui_with_layout(egui::vec2(300.0, 200.0), Layout::top_down_justified(Align::Min), |ui| {
                popup_body(ui, |ui| {
                    let _ = ui.button("Undo");
                    let _ = ui.button("Preferences");
                });
                width = ui.min_rect().width();
            });
        });
        out.textures_delta.clear();
        assert!(width <= 300.5, "{dir:?}: popup_body ballooned its min_rect to {width} inside a 300-wide scope");
    }
}

/// Left-to-right `popup_body` is a pass-through: wrapping the same contents measures identically
/// to calling them directly.
#[test]
fn popup_body_ltr_matches_a_direct_call() {
    let _ltr = direction_scope(Dir::Ltr);
    let ctx = egui::Context::default();
    let mut widths = [0.0_f32; 2];
    let mut out = ctx.run_ui(Default::default(), |ui| {
        ui.allocate_ui_with_layout(egui::vec2(300.0, 200.0), Layout::top_down_justified(Align::Min), |ui| {
            let _ = ui.button("Undo");
            widths[0] = ui.min_rect().width();
        });
        ui.allocate_ui_with_layout(egui::vec2(300.0, 200.0), Layout::top_down_justified(Align::Min), |ui| {
            popup_body(ui, |ui| {
                let _ = ui.button("Undo");
            });
            widths[1] = ui.min_rect().width();
        });
    });
    out.textures_delta.clear();
    assert_eq!(widths[0], widths[1], "LTR: popup_body changed the body's measured width");
}

#[test]
fn float_window_x_pins_to_the_reading_end_edge() {
    let canvas = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 700.0));
    {
        let _ltr = direction_scope(Dir::Ltr);
        assert_eq!(float_window_x(canvas, 250.0, 16.0, 0.0), 734.0);
        assert_eq!(float_window_x(canvas, 250.0, 16.0, -280.0), 454.0);
    }
    let _rtl = direction_scope(Dir::Rtl);
    assert_eq!(float_window_x(canvas, 250.0, 16.0, 0.0), 16.0);
    assert_eq!(float_window_x(canvas, 250.0, 16.0, -280.0), 296.0);
    // A canvas narrower than the window keeps 8 px of it on the canvas.
    let narrow = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 700.0));
    assert_eq!(float_window_x(narrow, 250.0, 16.0, 0.0), 8.0);
    {
        let _ltr = direction_scope(Dir::Ltr);
        assert_eq!(float_window_x(narrow, 250.0, 16.0, 0.0), 8.0);
    }
}

#[test]
fn start_rect_family_mirrors_around_the_reading_start() {
    let rect = egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(300.0, 22.0));
    {
        let _ltr = direction_scope(Dir::Ltr);
        assert_eq!(start_rect(rect, egui::vec2(10.0, 2.0), egui::vec2(30.0, 10.0)).min, egui::pos2(110.0, 42.0));
        assert_eq!(start_cell(rect, 30.0).min, egui::pos2(100.0, 40.0));
        assert_eq!(start_cell(rect, 999.0).width(), 300.0, "clamped to the rect's width");
        assert_eq!(advance(100.0, 30.0), 130.0);
        assert_eq!(start_span(200.0, 40.0, 40.0, 62.0), egui::Rect::from_min_max(egui::pos2(200.0, 40.0), egui::pos2(240.0, 62.0)));
        let prev = egui::Rect::from_min_size(egui::pos2(110.0, 42.0), egui::vec2(30.0, 10.0));
        assert_eq!(name_span(prev, rect, 12.0, 4.0), (152.0, 396.0));
        assert_eq!(start_edge(rect), 100.0);
        let r1 = egui::Rect::from_min_size(egui::pos2(150.0, 0.0), egui::vec2(20.0, 10.0));
        let r2 = egui::Rect::from_min_size(egui::pos2(300.0, 0.0), egui::vec2(20.0, 10.0));
        assert_eq!(row_end_edge([r1, r2], 100.0), 320.0);
    }
    let _rtl = direction_scope(Dir::Rtl);
    assert_eq!(start_rect(rect, egui::vec2(10.0, 2.0), egui::vec2(30.0, 10.0)).min, egui::pos2(360.0, 42.0));
    assert_eq!(start_cell(rect, 30.0).min, egui::pos2(370.0, 40.0));
    assert_eq!(start_cell(rect, 999.0).min, egui::pos2(100.0, 40.0), "clamped to the rect's width");
    assert_eq!(advance(400.0, 30.0), 370.0);
    assert_eq!(start_span(200.0, 40.0, 40.0, 62.0), egui::Rect::from_min_max(egui::pos2(160.0, 40.0), egui::pos2(200.0, 62.0)));
    let prev = egui::Rect::from_min_size(egui::pos2(110.0, 42.0), egui::vec2(30.0, 10.0));
    assert_eq!(name_span(prev, rect, 12.0, 4.0), (98.0, 104.0));
    assert_eq!(start_edge(rect), 400.0);
    let r1 = egui::Rect::from_min_size(egui::pos2(150.0, 0.0), egui::vec2(20.0, 10.0));
    let r2 = egui::Rect::from_min_size(egui::pos2(300.0, 0.0), egui::vec2(20.0, 10.0));
    assert_eq!(row_end_edge([r1, r2], 400.0), 150.0);
}

#[test]
fn text_anchors_hug_the_mirrored_edges() {
    let rect = egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(300.0, 22.0));
    {
        let _ltr = direction_scope(Dir::Ltr);
        assert_eq!(start_text_anchor(rect, 6.0), (egui::pos2(106.0, 51.0), egui::Align2::LEFT_CENTER));
        assert_eq!(end_text_anchor(rect, 6.0), (egui::pos2(394.0, 51.0), egui::Align2::RIGHT_CENTER));
    }
    let _rtl = direction_scope(Dir::Rtl);
    assert_eq!(start_text_anchor(rect, 6.0), (egui::pos2(394.0, 51.0), egui::Align2::RIGHT_CENTER));
    assert_eq!(end_text_anchor(rect, 6.0), (egui::pos2(106.0, 51.0), egui::Align2::LEFT_CENTER));
}

/// Left x of the text `text` in a raw context's painted shapes.
fn raw_text_x(out: &egui::FullOutput, text: &str) -> f32 {
    fn find(shape: &egui::Shape, text: &str) -> Option<f32> {
        match shape {
            egui::Shape::Text(t) if t.galley.job.text == text => Some(t.pos.x),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
            _ => None,
        }
    }
    out.shapes.iter().find_map(|s| find(&s.shape, text)).unwrap_or_else(|| panic!("`{text}` painted"))
}

#[test]
fn paint_label_span_elides_and_aligns_into_the_reading_direction() {
    let font = egui::FontId::default();
    let color = egui::Color32::WHITE;
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let p = ui.painter().clone();
            if dir == Dir::Rtl {
                let rtl_span = paint_label_span(&p, 400.0, 50.0, 100.0, "a long layer name", font.clone(), color).expect("painted");
                assert!((rtl_span.right() - 400.0).abs() < 0.5, "{dir:?}: anchored at the reading start: {rtl_span:?}");
                assert!(rtl_span.width() <= 300.5, "{dir:?}: elided to the span");
                // An unconverted caller (anchor before the limit) keeps the left-to-right contract.
                let ltr_span = paint_label_span(&p, 100.0, 80.0, 400.0, "a long layer name", font.clone(), color).expect("painted");
                assert!((ltr_span.left() - 100.0).abs() < 0.5, "{dir:?}: unconverted span keeps the LTR contract: {ltr_span:?}");
            } else {
                let ltr_span = paint_label_span(&p, 100.0, 80.0, 400.0, "a long layer name", font.clone(), color).expect("painted");
                assert!((ltr_span.left() - 100.0).abs() < 0.5, "{dir:?}: anchored at the reading start: {ltr_span:?}");
                assert!(ltr_span.width() <= 300.5, "{dir:?}: elided to the span");
                assert!(paint_label_span(&p, 400.0, 50.0, 100.0, "x", font.clone(), color).is_none(), "{dir:?}: a negative-width span paints nothing");
            }
            // No room at all: nothing painted.
            assert!(paint_label_span(&p, 100.0, 110.0, 104.0, "x", font.clone(), color).is_none());
        });
        out.textures_delta.clear();
    }
}

#[test]
fn paint_elided_centres_on_cy_and_reports_elision() {
    let font = egui::FontId::default();
    let color = egui::Color32::WHITE;
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let p = ui.painter().clone();
            let short = paint_elided(&p, 400.0, 50.0, 300.0, "ok", font.clone(), color).expect("painted");
            assert!(!short.1, "{dir:?}: short text is not elided");
            assert!((short.0.center().y - 50.0).abs() < 0.5, "{dir:?}: vertically centred on cy");
            let long = paint_elided(&p, 400.0, 80.0, 60.0, "a much longer title than fits", font.clone(), color).expect("painted");
            assert!(long.1, "{dir:?}: long text is elided");
            assert!(long.0.width() <= 60.5, "{dir:?}: painted inside max_w: {:?}", long.0);
            if dir == Dir::Rtl {
                assert!((short.0.right() - 400.0).abs() < 0.5, "{dir:?}: anchored at the reading-start edge");
                assert!((long.0.right() - 400.0).abs() < 0.5, "{dir:?}: anchored at the reading-start edge");
            } else {
                assert!((short.0.left() - 400.0).abs() < 0.5, "{dir:?}: anchored at x");
            }
            assert!(elided_galley(&p, "x", font.clone(), color, false, 4.0).is_none(), "no room, no galley");
        });
        out.textures_delta.clear();
    }
}

#[test]
fn popup_anchor_reading_end_moves_past_the_reading_end() {
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let resp = ui.button("gear");
            let anchor = popup_anchor_reading_end(&resp, 30.0);
            let egui::PopupAnchor::ParentRect(r) = anchor else { panic!("ParentRect") };
            let want = if dir == Dir::Rtl { -30.0 } else { 30.0 };
            assert!((r.min.x - (resp.interact_rect.min.x + want)).abs() < 0.5, "{dir:?}: {r:?} vs {:?}", resp.interact_rect);
            assert!((r.min.y - resp.interact_rect.min.y).abs() < 0.5, "{dir:?}: y unchanged");
        });
        out.textures_delta.clear();
    }
}

#[test]
fn popup_area_near_hangs_off_the_reading_end_edge() {
    let anchor = egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(24.0, 24.0));
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let area = popup_area_near(ui, "flyout", anchor, 6.0);
            let dbg = format!("{area:?}");
            if dir == Dir::Rtl {
                assert!(dbg.contains("pivot: Align2(Max, Min)"), "{dir:?}: RIGHT_TOP: {dbg}");
                assert!(dbg.contains("new_pos: Some([94.0 40.0])"), "{dir:?}: {dbg}");
            } else {
                assert!(dbg.contains("pivot: Align2(Min, Min)"), "{dir:?}: LEFT_TOP: {dbg}");
                assert!(dbg.contains("new_pos: Some([130.0 40.0])"), "{dir:?}: {dbg}");
            }
        });
        out.textures_delta.clear();
    }
}

#[test]
fn form_grid_lays_caption_then_editor_in_reading_order() {
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.set_max_width(400.0);
            form_grid(ui, "test-form", [12.0, 6.0], 120.0, |row| {
                row("Alpha", &mut |ui, cap| {
                    assert_ne!(cap, egui::Id::NULL, "the editor gets the caption's Id");
                    ui.button("a-edit");
                });
                row("", &mut |ui, cap| {
                    if dir == Dir::Rtl {
                        assert_eq!(cap, egui::Id::NULL, "{dir:?}: a bare editor gets no caption Id");
                    }
                    ui.button("b-edit");
                });
            });
        });
        let caption_x = raw_text_x(&out, "Alpha");
        let edit_x = raw_text_x(&out, "a-edit");
        if dir == Dir::Rtl {
            assert!(caption_x > edit_x, "{dir:?}: caption leads from the right: {caption_x} vs {edit_x}");
        } else {
            assert!(caption_x < edit_x, "{dir:?}: caption leads from the left: {caption_x} vs {edit_x}");
        }
        out.textures_delta.clear();
    }
}

#[test]
fn ltr_grid_keeps_ltr_cell_order_under_rtl() {
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.set_max_width(400.0);
            let ir = ltr_grid(ui, "test-grid", 2, egui::vec2(6.0, 3.0), |ui| {
                ui.button("one");
                ui.button("two");
                ui.end_row();
            });
            let r = ir.response.rect;
            assert!(r.width() <= 400.5, "{dir:?}: grid inside the max width");
        });
        let one = raw_text_x(&out, "one");
        let two = raw_text_x(&out, "two");
        assert!(one < two, "{dir:?}: the grid scope stays left-to-right: {one} vs {two}");
        out.textures_delta.clear();
    }
}

#[test]
fn row_width_reads_max_rect_under_rtl() {
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.allocate_ui_with_layout(egui::vec2(300.0, 100.0), Layout::top_down(Align::Min), |ui| {
                let w = row_width(ui);
                if dir == Dir::Rtl {
                    assert_eq!(w, ui.max_rect().width(), "{dir:?}: max_rect width");
                } else {
                    assert_eq!(w, 300.0, "{dir:?}: available width");
                }
            });
        });
        out.textures_delta.clear();
    }
}

#[test]
fn end_side_helpers_mirror_the_end_edge() {
    let bar = egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(300.0, 22.0));
    {
        let _ltr = direction_scope(Dir::Ltr);
        assert_eq!(text_direction(), egui::epaint::text::TextDirection::LeftToRight);
        assert_eq!(end_corner_anchor(true, 16.0, -36.0), (egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -36.0)));
        assert_eq!(end_corner_anchor(false, 16.0, 8.0), (egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 8.0)));
        let (rest, strip) = split_end(bar, 60.0);
        assert_eq!(rest, egui::Rect::from_min_max(egui::pos2(100.0, 40.0), egui::pos2(340.0, 62.0)));
        assert_eq!(strip, egui::Rect::from_min_max(egui::pos2(340.0, 40.0), egui::pos2(400.0, 62.0)));
        let (rest, strip) = split_end(bar, 999.0);
        assert_eq!(strip, bar, "the strip is clamped into the bar");
        assert_eq!(rest.width(), 0.0);
        assert_eq!(end_span(340.0, 60.0, 40.0, 62.0), egui::Rect::from_min_max(egui::pos2(340.0, 40.0), egui::pos2(400.0, 62.0)));
    }
    let _rtl = direction_scope(Dir::Rtl);
    assert_eq!(text_direction(), egui::epaint::text::TextDirection::RightToLeft);
    assert_eq!(end_corner_anchor(true, 16.0, -36.0), (egui::Align2::LEFT_BOTTOM, egui::vec2(16.0, -36.0)));
    assert_eq!(end_corner_anchor(false, 16.0, 8.0), (egui::Align2::LEFT_TOP, egui::vec2(16.0, 8.0)));
    let (rest, strip) = split_end(bar, 60.0);
    assert_eq!(rest, egui::Rect::from_min_max(egui::pos2(160.0, 40.0), egui::pos2(400.0, 62.0)));
    assert_eq!(strip, egui::Rect::from_min_max(egui::pos2(100.0, 40.0), egui::pos2(160.0, 62.0)));
    let (rest, strip) = split_end(bar, 999.0);
    assert_eq!(strip, bar, "the strip is clamped into the bar");
    assert_eq!(rest.width(), 0.0);
    assert_eq!(end_span(160.0, 60.0, 40.0, 62.0), egui::Rect::from_min_max(egui::pos2(100.0, 40.0), egui::pos2(160.0, 62.0)));
}

#[test]
fn full_rows_and_trailing_row_pin_the_reading_order() {
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.allocate_ui_with_layout(egui::vec2(300.0, 200.0), Layout::top_down(Align::Min), |ui| {
                let want_row = if dir == Dir::Rtl { egui::Direction::RightToLeft } else { egui::Direction::LeftToRight };
                let want_trailing = if dir == Dir::Rtl { egui::Direction::LeftToRight } else { egui::Direction::RightToLeft };
                let mut seen = Vec::new();
                full_row(ui, |ui| {
                    seen.push((ui.layout().main_dir(), ui.max_rect().width(), ui.max_rect().height()));
                    ui.button("a");
                });
                full_trailing(ui, |ui| {
                    seen.push((ui.layout().main_dir(), ui.max_rect().width(), ui.max_rect().height()));
                    ui.button("b");
                });
                let ambient = ui.layout().main_dir();
                trailing_row(ui, |ui| {
                    seen.push((ui.layout().main_dir(), 0.0, 0.0));
                    ui.button("c");
                });
                assert_eq!(seen[0].0, want_row, "{dir:?}: full_row runs in reading order");
                assert_eq!(seen[1].0, want_trailing, "{dir:?}: full_trailing starts from the reading end");
                assert_eq!(seen[2].0, if dir == Dir::Rtl { want_trailing } else { ambient }, "{dir:?}: trailing_row pins only under RTL");
                for (d, w, h) in &seen[..2] {
                    assert!((*w - 300.0).abs() < 0.5 && *h > 0.0, "{dir:?}: pinned full-width interact-height row: {d:?} {w} {h}");
                }
            });
        });
        out.textures_delta.clear();
    }
}

#[test]
fn hscroll_row_centres_in_ltr_and_mirrors_the_region_in_rtl() {
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.set_max_width(400.0);
            let rects = std::cell::RefCell::new(Vec::new());
            hscroll_row(ui, |ui| {
                rects.borrow_mut().push(ui.button("first").rect);
                rects.borrow_mut().push(ui.button("second").rect);
            });
            let rects = rects.borrow();
            if dir == Dir::Rtl {
                assert!(rects[0].right() > rects[1].right(), "{dir:?}: the row anchors at the reading-start (right) edge: {:?}", *rects);
            } else {
                assert!(rects[0].right() < rects[1].right(), "{dir:?}: plain left-to-right row: {:?}", *rects);
            }
        });
        out.textures_delta.clear();
    }
}

#[test]
fn clamp_window_min_size_keeps_the_anchored_edge_fixed() {
    let rect = egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(300.0, 22.0));
    let min = egui::vec2(400.0, 50.0);
    {
        let _ltr = direction_scope(Dir::Ltr);
        // LTR anchors the window at the left edge: the rect grows rightward and downward.
        let c = clamp_window_min_size(rect, min);
        assert_eq!(c.min, egui::pos2(100.0, 40.0));
        assert_eq!(c.size(), egui::vec2(400.0, 50.0));
        assert!(c.width() >= min.x && c.height() >= min.y);
        // A rect already at least min wide is unchanged.
        let wide = egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(500.0, 60.0));
        assert_eq!(clamp_window_min_size(wide, min), wide);
    }
    let _rtl = direction_scope(Dir::Rtl);
    // RTL anchors the window at the right edge: the rect grows leftward, right edge fixed.
    let c = clamp_window_min_size(rect, min);
    assert_eq!(c.right(), rect.right(), "the anchored right edge stays put");
    assert_eq!(c.top(), rect.top());
    assert_eq!(c.size(), egui::vec2(400.0, 50.0));
    assert_eq!(c.min, egui::pos2(0.0, 40.0));
    assert!(c.width() >= min.x && c.height() >= min.y, "min width enforced in RTL too");
    let wide = egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(500.0, 60.0));
    assert_eq!(clamp_window_min_size(wide, min), wide);
}
