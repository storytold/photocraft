//! #144: the name never runs under the right-hand indicators; the fx triangle; #143: Collapse
//! All Groups.

use egui::{Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Effect, LayerContent};
use serde_json::json;

use super::{Indicator, RowRects, layout, recorded};
use crate::PhotocraftApp;

const LONG: &str = "Button / Primary / Hover state — a very long descriptive layer name 背景のテクスチャ that keeps going";

/// Long-named layers with every indicator, nested four groups deep.
fn busy() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let mut prev = None;
    for depth in 0..4 {
        let a = s.execute("layer.new.layer", json!({"name": format!("{LONG} {depth}")})).unwrap()["layer"].as_u64().unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
        s.execute("layer.layerStyle.dropShadow", json!({"layer": a})).unwrap();
        // #153: the vector mask thumbnail and both link chains take row width too.
        s.execute("layer.vectorMask.revealAll", json!({"layer": a})).unwrap();
        s.execute("layer.setProps", json!({"layer": a, "blend": "Multiply", "locks": {"all": true}})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({"layer": a})).unwrap();
        let b = s.execute("layer.new.layer", json!({"name": format!("{LONG} b{depth}")})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.setProps", json!({"layer": b, "clipped": true})).unwrap();
        s.execute("layer.select", json!({"layer": a})).unwrap();
        s.execute("layer.select", json!({"layer": b, "mode": "toggle"})).unwrap();
        s.execute("layer.linkLayers", json!({})).unwrap();
        if let Some(p) = prev {
            s.execute("layer.select", json!({"layer": p, "mode": "toggle"})).unwrap();
        }
        let g = s.execute("layer.groupLayers", json!({"name": format!("{LONG} group {depth}")})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.layerStyle.dropShadow", json!({"layer": g})).unwrap();
        prev = Some(g);
    }
    s
}

fn harness(session: photocraft_engine::Session, ppp: f32, theme: &str, dock_width: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_pixels_per_point(ppp).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(session, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new(
        "ui.set",
        json!({"theme": theme, "dockWidth": dock_width, "dock": {"collapsed": ["color", "properties", "history", "navigator"]}}),
    );
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    h
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.intersects(b) && a.intersect(b).area() > 0.0
}

fn check(rows: &[RowRects], what: &str) {
    assert!(rows.len() >= 8, "{what}: rows drawn ({})", rows.len());
    let mut with_fx = 0;
    for r in rows {
        let name = r.name.unwrap_or_else(|| panic!("{what}: layer {} has no room for its name", r.layer));
        assert!(name.right() <= r.row.right(), "{what}: name inside its row");
        for (k, ind) in &r.indicators {
            assert!(!overlaps(name, *ind), "{what}: layer {} name {name:?} overlaps {k:?} {ind:?}", r.layer);
            assert!(ind.right() <= r.row.right() - super::RIGHT_PAD + 0.01, "{what}: {k:?} clear of the scrollbar");
        }
        for (i, (ka, a)) in r.indicators.iter().enumerate() {
            for (kb, b) in &r.indicators[i + 1..] {
                assert!(!overlaps(*a, *b), "{what}: {ka:?} overlaps {kb:?}");
            }
        }
        with_fx += r.indicators.iter().any(|(k, _)| *k == Indicator::Fx) as usize;
    }
    assert!(with_fx >= 4, "{what}: fx badges drawn ({with_fx})");
}

#[test]
fn layout_gives_the_name_what_the_indicators_leave() {
    let row = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 32.0));
    let items = [(Indicator::Lock, 14.0), (Indicator::FxTriangle, 10.0), (Indicator::Fx, 12.0), (Indicator::Link, 14.0), (Indicator::Blend, 40.0)];
    let (rects, right) = layout(row, 100.0, &items);
    assert_eq!(rects.len(), 5);
    assert!(rects.iter().all(|(_, r)| r.left() > right && r.right() <= 300.0 - super::RIGHT_PAD));
    // Narrow: the optional blend label goes first; the rest stay.
    let (rects, right) = layout(row, 200.0, &items);
    assert_eq!(rects.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [Indicator::Lock, Indicator::FxTriangle, Indicator::Fx, Indicator::Link]);
    assert!(right < rects[3].1.left());
}

#[test]
fn long_names_never_run_under_the_indicators() {
    for ppp in [1.0, 1.5, 2.0] {
        for width in [250.0, 290.0, 520.0] {
            let h = harness(busy(), ppp, "promedium", width);
            check(&recorded(&h.ctx), &format!("@{ppp}x {width}pt"));
        }
    }
    for theme in ["pro", "studio", "studiolight", "classic"] {
        let h = harness(busy(), 1.0, theme, 250.0);
        check(&recorded(&h.ctx), theme);
    }
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn the_fx_triangle_hides_and_shows_the_effects_rows() {
    let mut h = harness(busy(), 1.0, "promedium", 290.0);
    let top = h.state().session.active().unwrap().doc.layers.last().unwrap().clone();
    let rows = |h: &Harness<'_, PhotocraftApp>| recorded(&h.ctx).iter().map(|r| r.row.top()).collect::<Vec<_>>();
    let before = rows(&h);
    let label = format!("Collapse effects {}", top.name);
    let p = h.get_by_label(&label).rect().center();
    let active = h.state().session.active().unwrap().active_layer;
    click(&mut h, p);
    assert!(h.state().session.active().unwrap().fx_collapsed.contains(&top.id));
    assert_eq!(h.state().session.active().unwrap().active_layer, active, "the triangle doesn't select");
    // Its Effects / Drop Shadow sub-rows are gone, so the next layer row moved up.
    let after = rows(&h);
    assert!(after[1] < before[1], "rows moved up: {:?} -> {:?}", &before[..2], &after[..2]);
    let p = h.get_by_label(&format!("Expand effects {}", top.name)).rect().center();
    click(&mut h, p);
    assert!(h.state().session.active().unwrap().fx_collapsed.is_empty());
}

#[test]
fn a_configured_but_disabled_effect_stays_discoverable_in_the_panel() {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let layer_id = session.execute("layer.new.layer", json!({"name": "Styled"})).unwrap()["layer"].as_u64().unwrap();
    session.execute("layer.layerStyle.dropShadow", json!({"layer": layer_id})).unwrap();
    let state = session.active_mut().unwrap();
    let doc = std::sync::Arc::make_mut(&mut state.doc);
    let layer = doc.layers.iter_mut().find(|layer| layer.id.0 == layer_id).unwrap();
    let Some(Effect::DropShadow(shadow)) = layer.effects.items.first_mut() else { panic!("drop shadow was configured") };
    shadow.common.enabled = false;

    let h = harness(session, 1.0, "promedium", 290.0);
    let effect_row = h.get_by_label("Drop Shadow").rect();
    assert!(effect_row.is_positive(), "a configured disabled effect remains visible for discovery");
}

/// #1622: the eyes on the effects rows work. The "Effects" eye hides and shows the whole list, an
/// effect's eye only that effect; each click is one history step, keeps the effect's settings and
/// never opens the Layer Style dialog.
#[test]
fn the_eyes_on_the_effects_rows_hide_and_show_effects() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let id = s.execute("layer.new.layer", json!({"name": "Styled"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.layerStyle.dropShadow", json!({"layer": id})).unwrap();
    s.execute("layer.layerStyle.colorOverlay", json!({"layer": id})).unwrap();
    let mut h = harness(s, 1.0, "promedium", 290.0);
    // (the whole list is on, each effect is on).
    let fx = |h: &Harness<'_, PhotocraftApp>| {
        let fx = &h.state().session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().effects;
        (fx.enabled, fx.items.iter().map(Effect::enabled).collect::<Vec<_>>())
    };
    let steps = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
    let eye = |h: &Harness<'_, PhotocraftApp>, row: &str| {
        let r = h.get_by_label(row).rect();
        pos2(r.left() + 15.0, r.center().y)
    };
    let (settings, n) = (h.state().session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().effects.items.clone(), steps(&h));
    assert_eq!(fx(&h), (true, vec![true, true]));

    let p = eye(&h, "Color Overlay");
    click(&mut h, p);
    assert_eq!(fx(&h), (true, vec![true, false]), "only the clicked effect is hidden");
    assert_eq!(steps(&h), n + 1, "one history step");
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Hide Color Overlay"));
    // The row stays (its eye box empty), and a click there shows the effect again.
    let p = eye(&h, "Color Overlay");
    click(&mut h, p);
    assert_eq!(fx(&h), (true, vec![true, true]));

    let p = eye(&h, "Effects");
    click(&mut h, p);
    assert_eq!(fx(&h), (false, vec![true, true]), "the Effects eye hides the list; each effect keeps its own eye");
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Hide Layer Effects"));
    let p = eye(&h, "Effects");
    click(&mut h, p);
    assert_eq!(fx(&h), (true, vec![true, true]));

    assert_eq!(h.state().session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().effects.items, settings, "settings survive");
    assert_eq!(steps(&h), n + 4);
    assert!(h.state().ui.dialogs.is_empty(), "an eye click never opens the Layer Style dialog");
}

fn groups_open(s: &photocraft_engine::Session) -> Vec<bool> {
    s.active().unwrap().doc.walk().into_iter().filter_map(|(_, _, l)| if let LayerContent::Group(g) = &l.content { Some(g.expanded) } else { None }).collect()
}

#[test]
fn collapse_all_groups_from_the_panel_menu_and_it_is_saved() {
    let mut h = harness(busy(), 1.0, "promedium", 290.0);
    assert!(groups_open(&h.state().session).iter().all(|o| *o));
    let menu = crate::dock::last_rects(&h.ctx).into_iter().find(|(g, _)| *g == crate::dock::Group::Layers).expect("layers group").1;
    // The hamburger sits at the right end of the group's tab strip.
    click(&mut h, pos2(menu.right() - 14.0, menu.top() + 14.0));
    let item = h.get_by_label("Collapse All Groups").rect().center();
    click(&mut h, item);
    let open = groups_open(&h.state().session);
    assert_eq!(open.len(), 4);
    assert!(open.iter().all(|o| !o), "every group closed: {open:?}");
    // Collapsed state is document data: it survives a PSD and a .pcraft round trip.
    let doc = (*h.state().session.active().unwrap().doc).clone();
    for name in ["c.psd", "c.pcraft"] {
        let bytes = photocraft_io::export(&doc, name, &Default::default()).unwrap().bytes;
        let back = photocraft_io::import(name, &bytes).unwrap().document;
        let mut s = photocraft_engine::Session::new();
        s.add_document(back, None);
        assert!(groups_open(&s).iter().all(|o| !o), "{name}: groups stay closed");
    }
}

/// Dragging down the eye column hides (or shows) every layer swept over, in one history step, and
/// never reorders the layers, even when one pointer move jumps several rows; a hidden layer's eye
/// box is empty and a click shows it again.
#[test]
fn dragging_down_the_eyes_sweeps_visibility_without_reordering() {
    for moves in [10, 1] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        for i in 0..4 {
            s.execute("layer.new.layer", json!({"name": format!("L{i}")})).unwrap();
        }
        let mut h = harness(s, 1.0, "promedium", 290.0);
        let order = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().doc.layers.iter().map(|l| l.id).collect::<Vec<_>>();
        let visible = |h: &Harness<'_, PhotocraftApp>, id: u64| h.state().session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().visible;
        let steps = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
        let (before, steps_before) = (order(&h), steps(&h));
        // Rows top to bottom: L3, L2, L1, L0, Background.
        let rows = recorded(&h.ctx);
        let eye = |r: &RowRects| pos2(r.row.left() + 17.0, r.row.center().y);
        let (first, last) = (eye(&rows[0]), eye(&rows[2]));
        h.event(egui::Event::PointerMoved(first));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: first, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        for k in 1..=moves {
            h.event(egui::Event::PointerMoved(first + (last - first) * (k as f32 / moves as f32)));
            h.run_steps(1);
        }
        h.event(egui::Event::PointerButton { pos: last, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
        for r in &rows[..3] {
            assert!(!visible(&h, r.layer), "{moves} moves: layer {} swept hidden", r.layer);
        }
        assert!(visible(&h, rows[3].layer), "{moves} moves: rows past the sweep are untouched");
        assert_eq!(order(&h), before, "{moves} moves: an eye drag never reorders the layers");
        assert_eq!(steps(&h), steps_before + 1, "{moves} moves: the whole sweep is one history step");
        assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Layer Visibility"));
        // A click on the (empty) eye box of a hidden layer shows it again.
        let p = eye(&recorded(&h.ctx)[1]);
        click(&mut h, p);
        assert!(visible(&h, rows[1].layer));
    }
}

/// #736: a layer row dropped on the footer's New Layer button is duplicated, on New Group it is
/// grouped, on Delete it is deleted; a row outside the selection goes alone.
#[test]
fn dropping_a_row_on_the_footer_buttons_duplicates_groups_and_deletes() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    for i in 0..3 {
        s.execute("layer.new.layer", json!({"name": format!("L{i}")})).unwrap();
    }
    let mut h = harness(s, 1.0, "promedium", 290.0);
    let names = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
    let drag_to = |h: &mut Harness<'static, PhotocraftApp>, row: usize, label: &str| {
        let from = recorded(&h.ctx)[row].row.center();
        let to = h.get_by_label(label).rect().center();
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        for k in 1..=8 {
            h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 8.0)));
            h.run_steps(1);
        }
        h.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
    };
    // Rows top to bottom: L2 (selected), L1, L0, Background. L1 isn't selected: it goes alone.
    drag_to(&mut h, 1, "Create a new layer");
    assert_eq!(names(&h), ["Background", "L0", "L1", "L1 copy", "L2"]);
    let steps = h.state().session.active().unwrap().history.past_len();
    // Rows: L2, L1 copy (now selected), L1, L0, Background.
    drag_to(&mut h, 3, "Create a new group");
    let doc = &h.state().session.active().unwrap().doc;
    let group = doc.layers.iter().find(|l| l.is_group()).expect("a group");
    let LayerContent::Group(g) = &group.content else { panic!("not a group") };
    assert_eq!(g.children.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["L0"]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "one undo step");
    let n = h.state().session.active().unwrap().doc.layers.len();
    drag_to(&mut h, 0, "Delete layer");
    assert!(!names(&h).iter().any(|n| n == "L2"));
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), n - 1);
}

/// Photoshop: ⌥ held when a dragged row is dropped copies the layer there (one "Duplicate Layer"
/// step) and leaves the original in place; without ⌥ the same drag moves it.
#[test]
fn alt_dragging_a_row_drops_a_copy_and_a_plain_drag_moves() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    for i in 0..3 {
        s.execute("layer.new.layer", json!({"name": format!("L{i}")})).unwrap();
    }
    let mut h = harness(s, 1.0, "promedium", 290.0);
    let names = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
    let steps = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
    // Drag row `from` to the upper quarter of row `to` (= above it), with `release` held on drop.
    let drag = |h: &mut Harness<'static, PhotocraftApp>, from: usize, to: usize, release: Modifiers| {
        let rows = recorded(&h.ctx);
        let a = rows[from].row.center();
        let b = pos2(rows[to].row.center().x, rows[to].row.top() + rows[to].row.height() * 0.25);
        h.event(egui::Event::PointerMoved(a));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        for k in 1..=8 {
            h.event(egui::Event::PointerMoved(a + (b - a) * (k as f32 / 8.0)));
            h.run_steps(1);
        }
        // ⌥ only needs to be down when the row is dropped.
        h.event(egui::Event::ModifiersChanged(release));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: b, button: PointerButton::Primary, pressed: false, modifiers: release });
        h.run_steps(1);
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(3);
    };
    // Rows top to bottom: L2, L1, L0, Background. ⌥-drag L0 above L2.
    let before = steps(&h);
    drag(&mut h, 2, 0, Modifiers::ALT);
    assert_eq!(names(&h), ["Background", "L0", "L1", "L2", "L0 copy"]);
    assert_eq!(steps(&h), before + 1, "one undo step");
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Duplicate Layer"));
    let active = h.state().session.active().unwrap().active_layer;
    assert_eq!(active, h.state().session.active().unwrap().doc.layers.last().map(|l| l.id), "the copy is active");
    // Rows: L0 copy, L2, L1, L0, Background. A plain drag of L1 above L0 copy moves it.
    drag(&mut h, 2, 0, Modifiers::NONE);
    assert_eq!(names(&h), ["Background", "L0", "L2", "L0 copy", "L1"]);
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Reorder Layers"));
}

/// Photoshop's Layers footer is a bar along the panel's bottom edge: its buttons sit in the middle
/// of the bar's height, from the right. They used to hang under the line with twice the gap below.
#[test]
fn the_footer_is_a_bar_along_the_panel_bottom_with_its_buttons_centred() {
    for theme in ["promedium", "studio"] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let h = harness(s, 1.0, theme, 290.0);
        let group = crate::dock::last_rects(&h.ctx).into_iter().find(|(g, _)| *g == crate::dock::Group::Layers).unwrap().1;
        let delete = h.get_by_label("Delete layer").rect();
        let bar_middle = group.bottom() - crate::widgets::FOOTER_BAR / 2.0;
        assert!((delete.center().y - bar_middle).abs() <= 1.5, "{theme}: button at {delete:?}, bar middle {bar_middle}, group {group:?}");
        assert!(group.right() - delete.right() <= 10.0, "{theme}: laid out from the right: {delete:?} in {group:?}");
        let new_layer = h.get_by_label("Create a new layer").rect();
        assert_eq!(new_layer.center().y, delete.center().y, "{theme}: one row");
        assert!(new_layer.right() <= delete.left(), "{theme}: New Layer left of Delete");
    }
}
