//! Right-to-left twins of the Layers panel rows (#144): in both directions a row's children
//! share one leading edge (eye column, thumbnail, name, in reading order) and one trailing
//! edge (the indicators packed against the pad edge where the overlay scrollbar sits).
//! Direction only — the language stays English, like the left-to-right twins in `tests`.

use egui::vec2;
use egui_kittest::Harness;
use serde_json::json;

use super::{GAP, RIGHT_PAD, RowRects, recorded};
use crate::PhotocraftApp;
use crate::direction::{Dir, direction_scope};

/// A locked, styled layer above a plain one, so rows carry a thumbnail, a name and indicators.
fn session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let a = s.execute("layer.new.layer", json!({"name": "Styled"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.layerStyle.dropShadow", json!({"layer": a})).unwrap();
    s.execute("layer.setProps", json!({"layer": a, "blend": "Multiply", "locks": {"all": true}})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Plain"})).unwrap();
    s
}

/// The rows the Layers panel drew, in `dir`.
fn rows_in(dir: Dir) -> Vec<RowRects> {
    let _scope = direction_scope(dir);
    let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(session(), crate::Services::default())
    });
    h.ctx.all_styles_mut(|st| st.right_to_left = dir == Dir::Rtl);
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new(
        "ui.set",
        json!({"theme": "promedium", "dockWidth": 290.0, "dock": {"collapsed": ["color", "properties", "history", "navigator"]}}),
    );
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    recorded(&ctx)
}

/// Every row's children keep the reading order and share the pad edge in both directions:
/// left-to-right the eye and thumbnail hug the left (leading) edge and the indicators pack
/// from the right (trailing) pad edge; right-to-left the whole row mirrors as a unit.
#[test]
fn row_children_share_one_leading_and_one_trailing_edge_in_both_directions() {
    for dir in [Dir::Ltr, Dir::Rtl] {
        let rtl = dir == Dir::Rtl;
        let rows = rows_in(dir);
        assert!(rows.len() >= 3, "{dir:?}: rows drawn ({})", rows.len());
        let mut with_indicators = 0;
        for r in &rows {
            let name = r.name.unwrap_or_else(|| panic!("{dir:?}: layer {} has no name rect", r.layer));
            let thumb = r.thumbnail.unwrap_or_else(|| panic!("{dir:?}: layer {} has no thumbnail", r.layer));
            // Leading edge: the thumbnail sits past the eye column on the reading-start side.
            if rtl {
                assert!(thumb.right() <= r.row.right() - 30.0 + 0.01, "{dir:?}: thumbnail off the right edge: {thumb:?} in {r:?}", r = r.row);
                assert!(name.right() <= thumb.left() + 0.01, "{dir:?}: name right of its thumbnail: {name:?} {thumb:?}");
            } else {
                assert!(thumb.left() >= r.row.left() + 30.0 - 0.01, "{dir:?}: thumbnail off the left edge: {thumb:?} in {:?}", r.row);
                assert!(name.left() >= thumb.right() - 0.01, "{dir:?}: name left of its thumbnail: {name:?} {thumb:?}");
            }
            // Trailing edge: the first indicator hugs the pad edge (the scrollbar side).
            let Some(&(_, first)) = r.indicators.first() else { continue };
            with_indicators += 1;
            let last = r.indicators.last().map(|(_, r)| *r).unwrap_or(first);
            if rtl {
                assert!((first.left() - (r.row.left() + RIGHT_PAD)).abs() < 0.01, "{dir:?}: first indicator off the left pad edge: {first:?} in {:?}", r.row);
                assert!(name.left() >= last.right() + GAP - 0.01, "{dir:?}: name under the indicators: {name:?} {last:?}");
            } else {
                assert!((r.row.right() - RIGHT_PAD - first.right()).abs() < 0.01, "{dir:?}: first indicator off the right pad edge: {first:?} in {:?}", r.row);
                assert!(name.right() <= last.left() - GAP + 0.01, "{dir:?}: name under the indicators: {name:?} {last:?}");
            }
        }
        assert!(with_indicators >= 1, "{dir:?}: the styled layer drew its indicators");
    }
}

/// The Layers flyout's rows (`panel_menu`) go through `menu_row`/`menus::menu_button`, so under
/// RTL the row labels hug the popup's reading-start (right) edge; LTR keeps them at the left.
#[test]
fn panel_menu_rows_hug_the_reading_start_edge() {
    let rects = |dir: Dir| -> Vec<(String, egui::Rect)> {
        let _scope = direction_scope(dir);
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| s.right_to_left = dir == Dir::Rtl);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let strip = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), vec2(220.0, 120.0));
            let mut strip_ui = ui.new_child(egui::UiBuilder::new().max_rect(strip).layout(egui::Layout::top_down_justified(egui::Align::Min)));
            crate::menus::popup_body(&mut strip_ui, |ui| {
                let _ = crate::menu_keys::menu_row(ui, "New Layer…", None, false, true);
                let _ = crate::menu_keys::menu_row(ui, "Panel Options…", None, false, true);
            });
        });
        let v = out
            .shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(ts) => Some((ts.galley.text().to_string(), ts.visual_bounding_rect())),
                _ => None,
            })
            .collect();
        out.textures_delta.clear();
        v
    };
    let at = |v: &[(String, egui::Rect)], s: &str| v.iter().find(|(t, _)| t.contains(s)).map(|(_, r)| *r).unwrap_or(egui::Rect::ZERO);
    let ltr = rects(Dir::Ltr);
    let rtl = rects(Dir::Rtl);
    let (l, r) = (at(&ltr, "New Layer"), at(&rtl, "New Layer"));
    assert!(l.is_positive() && r.is_positive(), "the row label is drawn both ways: {l:?} {r:?}");
    assert!(l.left() < 20.0 + 40.0, "ltr: the label hugs the left edge: {l:?}");
    assert!(r.right() > 240.0 - 40.0, "rtl: the label hugs the right edge: {r:?}");
    assert!(r.right() > l.right() + 10.0, "rtl: the label sits at the reading start: ltr {l:?} rtl {r:?}");
}
