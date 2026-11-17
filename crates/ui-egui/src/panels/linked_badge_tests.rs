//! #14118: in the Layers panel a linked smart object's thumbnail carries a chain-link badge in its
//! bottom-right corner instead of the smart object badge, and a red missing-link badge while its
//! file is not there; the plain badge comes back by itself once the file is back. An embedded
//! smart object keeps its badge. The badges follow the Layers panel thumbnail size.

use egui::{Color32, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{LayerContent, LayerId, SmartSource};
use serde_json::json;

use super::*;
use crate::mask_thumbs_ui::THUMB_LAYER;
use crate::state::LayerThumbnailSize;

/// A 100 × 60 px PNG in a fresh temporary folder; returns the folder and the file.
fn png() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("linked-100x60.png");
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 100, "height": 60, "background": "#336699"})).unwrap();
    s.execute("file.saveACopy", json!({"path": file.to_string_lossy(), "layers": false})).unwrap();
    (directory, file)
}

/// The whole app, Pro theme, with the Layers panel shown: a square 120 × 120 px document (so a
/// layer thumbnail fills its cell) with `file` placed embedded, then linked on top. Returns the
/// harness and the (embedded, linked) layer ids.
fn app(file: &std::path::Path) -> (Harness<'static, PhotocraftApp>, u64, u64) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 120, "height": 120})).unwrap();
    let mut place = |command: &str| {
        s.execute(command, json!({"path": file.to_string_lossy(), "center": [60, 60], "fit": false})).unwrap();
        s.active().unwrap().active_layer.unwrap().0
    };
    let (embedded, linked) = (place("file.placeEmbedded"), place("file.placeLinked"));
    let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ProMedium);
        PhotocraftApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"dockTabs": {"layers": 0}}));
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    (h, embedded, linked)
}

/// Run the link watcher's look on the spot (the app runs it every two seconds) and redraw.
fn look(h: &mut Harness<'_, PhotocraftApp>) {
    h.state_mut().run("layer.smartObjects.updateChanged", json!({})).unwrap();
    h.run_steps(4);
}

/// Where layer `id`'s thumbnail is drawn: the image painted with its texture.
fn thumb(h: &Harness<'_, PhotocraftApp>, id: u64) -> Rect {
    let tex = h.state().thumbs.get(&(LayerId(id), THUMB_LAYER)).expect("layer thumbnail drawn").1.id();
    let mut drawn = h.output().shapes.iter().filter_map(|s| match &s.shape {
        egui::Shape::Mesh(m) if m.texture_id == tex => Some(m.calc_bounds()),
        _ => None,
    });
    let r = drawn.next().expect("thumbnail painted");
    assert!(drawn.next().is_none(), "one thumbnail for layer {id}");
    r
}

/// The badge frames painted over `area`: (rect, fill, border colour). A frame is painted as a
/// filled rect and a 1 px border on the same rect, so the two are paired here.
fn frames(h: &Harness<'_, PhotocraftApp>, area: Rect) -> Vec<(Rect, Color32, Color32)> {
    let output = h.output();
    let rects: Vec<&egui::epaint::RectShape> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if area.contains_rect(r.rect) => Some(r),
            _ => None,
        })
        .collect();
    let border = |rect: Rect| rects.iter().find(|r| r.rect == rect && r.stroke.width == 1.0 && r.fill == Color32::TRANSPARENT).map(|r| r.stroke.color);
    rects.iter().filter(|r| r.fill != Color32::TRANSPARENT).filter_map(|r| Some((r.rect, r.fill, border(r.rect)?))).collect()
}

/// The colours of the icons (tinted images other than the layer thumbnails) painted inside `area`.
fn icon_tints(h: &Harness<'_, PhotocraftApp>, area: Rect) -> Vec<Color32> {
    let tints = h.output().shapes.iter().filter_map(|s| match &s.shape {
        egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() && area.contains_rect(m.calc_bounds()) => m.vertices.first().map(|v| v.color),
        _ => None,
    });
    tints.collect()
}

/// The badge rect expected over a `thumb` cell: 2 px past its bottom-right corner, `side` px.
fn badge_at(thumb: Rect, side: f32) -> Rect {
    Rect::from_min_size(thumb.right_bottom() + vec2(2.0 - side, 2.0 - side), vec2(side, side))
}

#[test]
fn a_linked_smart_object_shows_a_chain_link_badge_and_an_embedded_one_keeps_its_badge() {
    let (_directory, file) = png();
    let (h, embedded, linked) = app(&file);
    let t = Tokens::get(&h.ctx);
    let LayerContent::Smart(sm) = &h.state().session.active().unwrap().doc.layer(LayerId(linked)).unwrap().content else { panic!("smart object") };
    assert!(matches!(sm.source, SmartSource::Linked { .. }));

    // The Pro panel's default Medium thumbnail is 24 px: the chain link sits where the smart
    // object badge sits, at the same 11 px.
    let cell = thumb(&h, linked);
    assert_eq!(cell.size(), vec2(24.0, 24.0));
    let badge = h.get_by_label(LINKED_BADGE).rect();
    assert_eq!(badge, badge_at(cell, 11.0));
    assert_eq!(badge, thumb_badge_rect(cell), "in place of the smart object badge");
    assert_eq!(frames(&h, badge.expand(0.5)), [(badge, t.field, t.field_border)], "one frame, in the theme's colours");
    assert_eq!(icon_tints(&h, badge), [t.icon], "one icon, in the icon colour");
    assert!(h.query_by_label(LINK_MISSING_BADGE).is_none());

    // The embedded copy of the same file keeps the smart object badge, and is not named linked.
    let cell = thumb(&h, embedded);
    let frame = thumb_badge_rect(cell);
    assert_eq!(frames(&h, frame.expand(0.5)), [(frame, t.field, t.field_border)]);
    assert_eq!(h.query_all_by_label(LINKED_BADGE).count(), 1, "only the linked layer");
}

#[test]
fn a_missing_linked_file_turns_the_badge_red_until_the_file_is_back() {
    let (directory, file) = png();
    let (mut h, embedded, linked) = app(&file);
    let t = Tokens::get(&h.ctx);
    look(&mut h);
    assert!(h.query_by_label(LINK_MISSING_BADGE).is_none());
    let pixels = thumb(&h, linked);

    // Renamed while the document is open: the next look marks it missing.
    let away = directory.path().join("renamed.png");
    std::fs::rename(&file, &away).unwrap();
    look(&mut h);
    assert!(h.query_by_label(LINKED_BADGE).is_none(), "the chain link gives way");
    let badge = h.get_by_label(LINK_MISSING_BADGE).rect();
    assert_eq!(badge, badge_at(pixels, 11.0), "in the same corner");
    assert_eq!(frames(&h, badge.expand(0.5)), [(badge, t.field, t.danger)], "a red frame");
    assert_eq!(icon_tints(&h, badge), [t.danger], "a red icon");
    assert_eq!(thumb(&h, linked), pixels, "the layer keeps its thumbnail");
    let frame = thumb_badge_rect(thumb(&h, embedded));
    assert_eq!(frames(&h, frame.expand(0.5)), [(frame, t.field, t.field_border)], "the embedded copy is not missing");

    // Renamed back: the plain chain link returns with nothing else done.
    std::fs::rename(&away, &file).unwrap();
    look(&mut h);
    assert!(h.query_by_label(LINK_MISSING_BADGE).is_none());
    let badge = h.get_by_label(LINKED_BADGE).rect();
    assert_eq!(frames(&h, badge.expand(0.5)), [(badge, t.field, t.field_border)]);
    assert_eq!(icon_tints(&h, badge), [t.icon]);
}

#[test]
fn the_linked_badges_follow_the_thumbnail_size() {
    let (_directory, file) = png();
    let (mut h, _, linked) = app(&file);
    // Small 18 px, Medium 24 px (Pro), Large 48 px.
    let sizes = [(LayerThumbnailSize::Small, 18.0, 9.0), (LayerThumbnailSize::Medium, 24.0, 11.0), (LayerThumbnailSize::Large, 48.0, 18.0)];
    let check = |h: &mut Harness<'_, PhotocraftApp>, label: &str| {
        for (setting, cell, side) in sizes {
            h.state_mut().ui.layers_panel_options.thumbnail_size = setting;
            h.run_steps(4);
            let pixels = thumb(h, linked);
            assert_eq!(pixels.size(), vec2(cell, cell), "{setting:?}");
            assert_eq!(h.get_by_label(label).rect(), badge_at(pixels, side), "{label} at {setting:?}");
        }
        h.state_mut().ui.layers_panel_options.thumbnail_size = LayerThumbnailSize::None;
        h.run_steps(4);
        assert!(h.query_by_label(label).is_none(), "no thumbnail, no badge");
    };
    check(&mut h, LINKED_BADGE);
    std::fs::remove_file(&file).unwrap();
    h.state_mut().ui.layers_panel_options.thumbnail_size = LayerThumbnailSize::Medium;
    look(&mut h);
    check(&mut h, LINK_MISSING_BADGE);
}

#[test]
fn a_click_on_the_badge_still_selects_the_layer() {
    let (_directory, file) = png();
    let (mut h, embedded, linked) = app(&file);
    h.state_mut().run("layer.select", json!({"layer": embedded})).unwrap();
    h.run_steps(4);
    let badge = h.get_by_label(LINKED_BADGE).rect();
    // The part of the badge over the thumbnail, so the press lands inside the row.
    h.event(egui::Event::PointerMoved(badge.left_top() + vec2(2.0, 2.0)));
    h.run_steps(2);
    h.event(egui::Event::PointerButton {
        pos: badge.left_top() + vec2(2.0, 2.0),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    h.run_steps(1);
    h.event(egui::Event::PointerButton {
        pos: badge.left_top() + vec2(2.0, 2.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(LayerId(linked)));
}

/// The kind alone: a linked smart object asks whether its stored path is missing; an embedded one
/// and a PSD's placed layer (whose path is a uuid in the PSD's own file table) never do.
#[test]
fn only_a_linked_file_gets_a_link_badge() {
    let (_directory, file) = png();
    let (h, embedded, linked) = app(&file);
    let doc = &h.state().session.active().unwrap().doc;
    let layer = |id: u64| doc.layer(LayerId(id)).unwrap().clone();
    let LayerContent::Smart(sm) = &layer(linked).content else { panic!("smart object") };
    let SmartSource::Linked { path: stored } = &sm.source else { panic!("linked") };
    let mut asked = Vec::new();
    let kind = thumb_badge_kind(&layer(linked), |p| {
        asked.push(p.to_string());
        true
    });
    assert_eq!(kind, Some(ThumbBadge::LinkMissing));
    assert_eq!(asked, [stored.as_str()], "asked once, with the path as stored");
    assert_eq!(thumb_badge_kind(&layer(linked), |_| false), Some(ThumbBadge::Linked));
    assert_eq!(thumb_badge_kind(&layer(embedded), |_| true), Some(ThumbBadge::Smart));
    let mut psd = layer(embedded);
    let LayerContent::Smart(sm) = &mut psd.content else { panic!("smart object") };
    sm.source = SmartSource::Linked { path: "a1b2c3d4-0000-1111-2222-333344445555".into() };
    sm.psd_raw = Some(std::sync::Arc::new(vec![0; 4]));
    assert_eq!(thumb_badge_kind(&psd, |_| true), Some(ThumbBadge::Smart));
}
