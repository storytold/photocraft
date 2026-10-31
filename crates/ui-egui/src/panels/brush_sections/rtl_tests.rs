//! Right-to-left twins of the Brush Settings section bodies: the Size row reads from the
//! right, the angle and roundness captions hug the reading-start edge of their 72 px block,
//! and the tip grid's first cell is the right-most. Direction only — the language stays
//! English, like the left-to-right twins.

use egui::accesskit::Role;
use egui::{Pos2, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable, by};

use super::*;

/// The section body under test: the working brush, the presets its grids show and the last
/// preset picked in a grid.
struct SectionState {
    brush: BrushSettings,
    presets: Vec<BrushPreset>,
    picked: Option<String>,
}

/// One section body in a harness. The direction scope must already be set when this runs:
/// the theme installs the egui direction flag with it.
fn section_harness(section: usize, state: SectionState) -> Harness<'static, SectionState> {
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(340.0, 1100.0)).build_ui_state(
        move |ui, state: &mut SectionState| {
            if !ready {
                crate::PhotocraftApp::setup_context(ui.ctx(), theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            if let Some(name) = section_body(ui, &mut state.brush, section, &state.presets, None, Tool::Brush) {
                state.picked = Some(name);
            }
        },
        state,
    );
    h.run_steps(3);
    h
}

/// The caption label with this text, not the field it names.
fn caption_rect(h: &Harness<'_, SectionState>, text: &str) -> Rect {
    h.query_all(by().label(text).include_labels()).find(|n| n.accesskit_node().role() == Role::Label).unwrap_or_else(|| panic!("the `{text}` caption")).rect()
}

/// The value field with this accessible name, not the caption.
fn field_rect(h: &Harness<'_, SectionState>, text: &str) -> Rect {
    h.query_all(by().label(text).include_labels()).find(|n| n.accesskit_node().role() != Role::Label).unwrap_or_else(|| panic!("the `{text}` field")).rect()
}

/// The painted box of the number field: the 70×24 rounded rect the number sits in.
fn field_box(h: &Harness<'_, SectionState>, number: Rect) -> Rect {
    h.output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(r) => Some(r.rect),
            _ => None,
        })
        .find(|r| (r.width() - 70.0).abs() < 1.0 && (r.height() - 24.0).abs() < 1.0 && r.contains_rect(number))
        .unwrap_or_else(|| panic!("the field's box around {number:?}"))
}

/// A sampled tip, so the Size row offers its "Restore original size" button.
fn sampled_brush() -> BrushSettings {
    BrushSettings { tip: paint::TipShape::Sampled(paint::GrayTile::from_fn(8, 8, |_, _| 1.0)), size: 77.0, ..Default::default() }
}

/// A preset whose tip size is distinct, so the grid's painted size numbers identify it.
fn preset(name: &str, size: f32) -> BrushPreset {
    let brush = BrushSettings { size, ..Default::default() };
    BrushPreset { name: name.to_string(), brush, builtin: true, group: String::new(), folder: Vec::new(), captures: Default::default() }
}

/// A press and a release on the same spot.
fn click_at(h: &mut Harness<'static, SectionState>, pos: Pos2) {
    h.hover_at(pos);
    h.run_steps(1);
    h.drag_at(pos);
    h.run_steps(1);
    h.drop_at(pos);
    h.run_steps(2);
}

/// The painted text of every shape, with the centre of each galley.
fn painted_text(h: &Harness<'_, SectionState>) -> Vec<(String, Pos2)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
        match shape {
            egui::Shape::Text(t) => {
                let centre = t.pos + t.galley.rect.size() / 2.0;
                out.push((t.galley.job.text.clone(), centre));
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for shape in &h.output().shapes {
        walk(&shape.shape, &mut out);
    }
    out
}

/// The Size row reads from the right: the caption leads, the px field ends the row and the
/// restore button sits between them — the mirror of the left-to-right `[Size][restore][field]`.
#[test]
fn rtl_size_row_reads_from_the_right() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let h = section_harness(0, SectionState { brush: sampled_brush(), presets: vec![], picked: None });
    let caption = caption_rect(&h, "Size");
    let field = field_rect(&h, "Size");
    assert!(caption.left() > field.right(), "the Size caption leads from the right: {caption:?}, {field:?}");
    let restore = h.get_by_role_and_label(Role::Button, "Restore original size").rect();
    assert!(
        restore.left() > field.right() && restore.right() < caption.left(),
        "the restore button sits between the field and the caption: {restore:?}, {field:?}, {caption:?}"
    );
}

/// The tip grid's cells flow from the right: the first preset is the right-most cell, and
/// clicking it picks that preset.
#[test]
fn rtl_tip_grid_starts_at_the_right() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let presets = vec![preset("First", 11.0), preset("Second", 22.0), preset("Third", 33.0)];
    let mut h = section_harness(0, SectionState { brush: sampled_brush(), presets, picked: None });
    let texts = painted_text(&h);
    let number = |text: &str| texts.iter().find(|(s, _)| s == text).map(|(_, p)| *p).unwrap_or_else(|| panic!("the `{text}` size number"));
    let (first, second, third) = (number("11"), number("22"), number("33"));
    assert!(first.x > second.x && second.x > third.x, "the first preset leads from the right: {first:?}, {second:?}, {third:?}");
    click_at(&mut h, first);
    assert_eq!(h.state().picked.as_deref(), Some("First"), "the right-most cell picks the first preset");
}

/// TEMPORARY PROBE — remove before finishing.
#[test]
fn probe_rtl_window_columns() {
    use egui_kittest::kittest::Queryable;
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.panels.brush_settings = true;
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut crate::PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::brush_panel::window(app, &ctx);
        },
        app,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    eprintln!("PROBE is_rtl={}", crate::direction::is_rtl());
    for label in ["Brush Tip Shape", "Scattering", "Size"] {
        for n in h.query_all(by().label(label).include_labels()) {
            eprintln!("PROBE {label:?} rect={:?} role={:?}", n.rect(), n.accesskit_node().role());
        }
    }
    for s in &h.output().shapes {
        let r = s.visual_rect();
        if r.width() > 400.0 {
            eprintln!("PROBE wide shape {:?} clip={:?}", r, s.clip_rect);
        }
    }
}

/// The angle and roundness captions hug the reading-start edge of their 72 px block: in a
/// right-to-left UI the caption ends at the block's right edge, the mirror of the
/// left-to-right row, so it sits right next to its field.
#[test]
fn rtl_angle_captions_hug_their_block() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let h = section_harness(0, SectionState { brush: sampled_brush(), presets: vec![], picked: None });
    let spacing = h.ctx.style_of(egui::Theme::Dark).spacing.item_spacing.x;
    for text in ["Angle:", "Roundness:"] {
        let caption = caption_rect(&h, text);
        let field = field_rect(&h, text);
        // The block keeps its 72 px reserve and ends one item spacing before the field's box.
        let block_right = field_box(&h, field).left() - spacing;
        assert!((caption.right() - block_right).abs() < 1.5, "the {text} caption ends at its block's right edge: {caption:?}, block right {block_right}");
    }
}
