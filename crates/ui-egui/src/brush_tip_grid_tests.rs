use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

struct GridState {
    origin: egui::Pos2,
    available: f32,
    picked: Option<usize>,
    selected: Option<usize>,
}

fn presets(count: usize) -> Vec<BrushPreset> {
    (0..count)
        .map(|i| BrushPreset {
            name: format!("Grid tip {i:04}"),
            brush: BrushSettings { size: 47.0, ..Default::default() },
            builtin: false,
            group: "Synthetic grid".into(),
            folder: Vec::new(),
            captures: Default::default(),
        })
        .collect()
}

fn harness(kind: theme::ThemeKind, count: usize, enabled: bool) -> Harness<'static, GridState> {
    let tips = presets(count);
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(380.0, 360.0)).build_ui_state(
        move |ui, state: &mut GridState| {
            if !ready {
                crate::PhotocraftApp::setup_context(ui.ctx(), kind);
                ready = true;
                return;
            }
            ui.set_width(340.0);
            state.origin = ui.next_widget_position();
            state.available = ui.available_height();
            ui.add_enabled_ui(enabled, |ui| {
                if let Some(i) = tip_grid(ui, "grid-regression", &tips, state.selected) {
                    state.picked = Some(i);
                    state.selected = Some(i);
                }
            });
        },
        GridState { origin: egui::Pos2::ZERO, available: 0.0, picked: None, selected: (count > 7).then_some(7) },
    );
    h.run_steps(4);
    h
}

fn click(h: &mut Harness<'_, GridState>, pos: egui::Pos2) {
    h.hover_at(pos);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
}

#[test]
fn tip_grid_eight_compact_columns_pick_the_correct_tip_in_every_theme() {
    for kind in theme::ThemeKind::ALL {
        let mut h = harness(kind, 96, true);
        // Independent pointer positions for the reported eight 32-point columns.
        // Checking the selection result catches wrapping, gutters and hit boxes too.
        for row in 0..2 {
            for col in 0..8 {
                let p = h.state().origin + vec2(1.0 + (col as f32 + 0.5) * 32.0, 1.0 + (row as f32 + 0.5) * 40.0);
                click(&mut h, p);
                assert_eq!(h.state().picked, Some(row * 8 + col), "{kind:?}: row {row}, column {col}");
            }
        }
        assert!((h.get_by_role(Role::ScrollBar).rect().height() - h.state().available * 0.5).abs() < 2.0);
    }
}

#[test]
fn tip_grid_scrollbar_remains_visible_for_empty_short_and_long_lists() {
    fn visible_handle(shape: &egui::Shape, bar: egui::Rect, track: Color32) -> bool {
        match shape {
            egui::Shape::Rect(r) => bar.contains_rect(r.rect) && r.rect.height() >= 8.0 && r.fill != track && r.fill != Color32::TRANSPARENT,
            egui::Shape::Vec(v) => v.iter().any(|s| visible_handle(s, bar, track)),
            _ => false,
        }
    }
    for kind in theme::ThemeKind::ALL {
        for count in [0, 1, 8, 96] {
            let h = harness(kind, count, true);
            let bar = h.get_by_role(Role::ScrollBar).rect();
            assert!((bar.height() - h.state().available * 0.5).abs() < 2.0, "{count} tips: {bar:?}");
            assert!(
                h.output().shapes.iter().any(|s| visible_handle(&s.shape, bar, Tokens::get(&h.ctx).field)),
                "{kind:?}, {count} tips: the idle thumb must contrast with its track"
            );
            fn outer_height(shape: &egui::Shape, color: Color32) -> Option<(f32, f32)> {
                match shape {
                    egui::Shape::Rect(r) if r.stroke.width == 1.0 && r.stroke.color == color => Some((r.rect.width(), r.rect.height())),
                    egui::Shape::Vec(v) => v.iter().filter_map(|s| outer_height(s, color)).max_by(|a, b| a.0.total_cmp(&b.0)),
                    _ => None,
                }
            }
            let frame =
                h.output().shapes.iter().filter_map(|s| outer_height(&s.shape, Tokens::get(&h.ctx).field_border)).max_by(|a, b| a.0.total_cmp(&b.0)).unwrap();
            assert!((frame.1 - bar.height() - 2.0).abs() < 0.1, "the frame adds only its two one-point borders: {frame:?}, {bar:?}");
            if count == 0 {
                assert_eq!(brush_preview::render_count(&h.ctx), 0);
                assert!(h.query_by_label("Grid tip 0000").is_none());
            }
        }
    }
}

#[test]
fn tip_grid_selection_is_named_by_identity_and_accepts_space() {
    let mut h = harness(theme::ThemeKind::ProMedium, 96, true);
    // All these tips intentionally look alike. Their names, not shape equality, identify them.
    assert_eq!(h.get_by_label("Grid tip 0007").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    assert_eq!(h.get_by_label("Grid tip 0006").accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    let cell = h.get_by_label("Grid tip 0008");
    assert!((cell.rect().width() - 32.0).abs() < 0.1);
    cell.focus();
    h.run_steps(1);
    h.key_press(egui::Key::Space);
    h.run_steps(3);
    assert_eq!(h.state().picked, Some(8));
    assert_eq!(h.get_by_label("Grid tip 0008").accesskit_node().toggled(), Some(egui::accesskit::Toggled::True));
    assert_eq!(h.get_by_label("Grid tip 0007").accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
}

#[test]
fn tip_grid_scrolls_to_the_last_tip_without_rendering_the_whole_library() {
    let mut h = harness(theme::ThemeKind::ProMedium, 2500, true);
    assert!(h.query_by_label("Grid tip 2499").is_none());
    let renders = brush_preview::render_count(&h.ctx);
    assert!(renders > 0 && renders <= 64, "only the viewport's thumbnails render: {renders}");
    h.run_steps(8);
    assert_eq!(brush_preview::render_count(&h.ctx), renders, "idle frames reuse thumbnails");
    let bar = h.get_by_role(Role::ScrollBar).rect();
    let start = pos2(bar.center().x, bar.top() + 4.0);
    let end = pos2(bar.center().x, bar.bottom() - 2.0);
    h.hover_at(start);
    h.run_steps(1);
    h.drag_at(start);
    h.run_steps(2);
    h.hover_at(end);
    h.run_steps(2);
    h.drop_at(end);
    h.run_steps(4);
    h.get_by_label("Grid tip 2499").click();
    h.run_steps(3);
    assert_eq!(h.state().picked, Some(2499));
    assert!(brush_preview::render_count(&h.ctx) <= renders + 128, "scrolling loads bounded visible rows");
}

#[test]
fn tip_grid_disabled_section_preserves_its_selection() {
    let mut h = harness(theme::ThemeKind::ProMedium, 96, false);
    let p = h.state().origin + vec2(17.0, 21.0);
    click(&mut h, p);
    assert_eq!(h.state().picked, None);
    assert_eq!(h.state().selected, Some(7));
}

#[test]
fn tip_grid_paints_shared_one_pixel_seams_and_a_two_pixel_selection() {
    fn rectangles(shape: &egui::Shape, out: &mut Vec<egui::epaint::RectShape>) {
        match shape {
            egui::Shape::Rect(r) => out.push(r.clone()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| rectangles(s, out)),
            _ => {}
        }
    }
    for kind in theme::ThemeKind::ALL {
        let h = harness(kind, 96, true);
        let t = Tokens::get(&h.ctx);
        let mut rects = Vec::new();
        for s in &h.output().shapes {
            rectangles(&s.shape, &mut rects);
        }
        let selected: Vec<_> = rects.iter().filter(|r| r.stroke.color == t.accent && (r.stroke.width - 2.0).abs() < 0.01).collect();
        assert_eq!(selected.len(), 1, "{kind:?}: one named selection, even for look-alike tips");
        assert_eq!(selected[0].rect.size(), vec2(32.0, 40.0));
        assert!((selected[0].rect.left() - h.state().origin.x - 1.0 - 7.0 * 32.0).abs() < 0.1);
        let seams = rects.iter().filter(|r| r.stroke.color == t.field_border && r.stroke.width == 1.0 && r.rect.size() == vec2(32.0, 40.0)).count();
        assert!(seams >= 8, "{kind:?}: every cell keeps a one-point border");
    }
}
