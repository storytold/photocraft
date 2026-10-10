//! The status bar's zoom field and Fit button sit inside the bar with an even margin above and
//! below, in every theme (#2823).

use egui::{Rect, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::{ThemeKind, Tokens};

const SIZE: egui::Vec2 = vec2(900.0, 200.0);

fn bottom_id() -> egui::Id {
    egui::Id::new("status-bar-test-bottom")
}

fn harness(kind: ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1080, "height": 1080, "resolution": 72, "background": "white"})).unwrap();
    app.sync_views();
    let mut h = Harness::builder().with_size(SIZE).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            // The first frame runs before `setup_context` installs the theme's fonts.
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                // Where the bar goes: the bottom of the harness's (margined) root Ui.
                let bottom = ui.max_rect().bottom();
                ui.ctx().data_mut(|d| d.insert_temp(bottom_id(), bottom));
                crate::panels::status_bar(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, kind);
    h.run_steps(3);
    h
}

#[test]
fn status_bar_controls_sit_inside_the_bar_with_even_margins() {
    for kind in ThemeKind::ALL {
        let h = harness(kind);
        let t = Tokens::for_kind(kind);
        let (bar_h, control_h) = crate::panels::status_bar_heights(&t);
        let margin = (bar_h - control_h) / 2.0;
        assert!(margin >= 3.0, "{kind:?}: margin {margin}");
        let bottom = h.ctx.data(|d| d.get_temp::<f32>(bottom_id())).unwrap();
        let bar = Rect::from_min_max(pos2(0.0, bottom - bar_h), pos2(SIZE.x, bottom));
        // Half a point of slack for egui's pixel rounding of the centred row.
        let inner = bar.shrink2(vec2(margin, margin - 0.5));
        // The zoom number (its box is painted around it, `control_h` tall and centred).
        let number = h.get_by_role(egui::accesskit::Role::SpinButton).rect();
        assert!(inner.contains_rect(number), "{kind:?}: bar {bar:?}, number {number:?}");
        // Pro (Photoshop) has no Fit button in the status bar.
        if t.pro {
            assert!(h.query_by_label("Fit").is_none(), "{kind:?}");
            continue;
        }
        let fit = h.get_by_label("Fit").rect();
        assert!(inner.contains_rect(fit), "{kind:?}: bar {bar:?}, Fit {fit:?}");
        assert!((fit.center().y - bar.center().y).abs() < 0.51, "{kind:?}: bar {bar:?}, Fit {fit:?}");
        assert!((fit.height() - control_h).abs() < 0.01, "{kind:?}: Fit {fit:?}");
    }
}
