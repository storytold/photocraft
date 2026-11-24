use super::*;
use egui::Color32;
use serde_json::json;

fn app_with(width: u32, height: u32, background: &str) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": width, "height": height, "background": background})).unwrap();
    app
}

fn read(source: &impl RasterSource, region: PixelRect, factor: u32) -> ColorImage {
    let texels = |n: i32| (n as u32).div_ceil(factor) as usize;
    let size = [texels(region.width()), texels(region.height())];
    let mut out = ColorImage::filled(size, Color32::TRANSPARENT);
    source.read(region, factor, &mut out);
    out
}

/// What the second of two passes reports since the first.
#[derive(Debug)]
enum Want {
    Nothing,
    /// A region holding this rectangle, inside the document.
    Covers(PixelRect),
    /// The source cannot tell: the display reads everything.
    Everything,
    /// The display reads this rectangle again: a region holding it, or everything.
    Redraws(PixelRect),
}

type Step = fn(&mut PhotocraftApp);

/// One pass of a source: its revision and size, its damage since the revision asked about, and since a revision it
/// never handed out.
struct Pass {
    revision: Revision,
    size: PixelSize,
    damage: Option<PixelRect>,
    unknown: Option<PixelRect>,
}

fn pass(source: &impl RasterSource, since: Revision) -> Pass {
    Pass { revision: source.revision(), size: source.size(), damage: source.damage(since), unknown: source.damage(Revision::new(7)) }
}

/// Runs each case on a fresh document from `setup`: a pass, the event, a second pass, and the second pass's damage
/// since the first.
fn check_damage(setup: fn() -> Box<PhotocraftApp>, run: fn(&mut PhotocraftApp, &egui::Context, Revision) -> Pass, cases: &[(&str, Step, Want)]) {
    for (event, step, want) in cases {
        let mut app = setup();
        let ctx = app.ctx().clone();
        let first = run(&mut app, &ctx, Revision::new(7)).revision;
        step(&mut app);
        let Pass { size, damage, unknown, .. } = run(&mut app, &ctx, first);
        let bounds = PixelRect::of_size(size);
        match want {
            Want::Nothing => assert!(damage.is_some_and(|d| d.is_empty()), "{event}: {damage:?}"),
            Want::Covers(r) => {
                let d = damage.unwrap_or_else(|| panic!("{event}: the source could not tell"));
                assert_eq!(d.intersect(r), *r, "{event}: {d:?} misses {r:?}");
                assert_eq!(d.intersect(&bounds), d, "{event}: {d:?} leaves the document");
            }
            Want::Everything => assert_eq!(damage, None, "{event}"),
            Want::Redraws(r) => {
                if let Some(d) = damage {
                    assert_eq!(d.intersect(r), *r, "{event}: {d:?} misses {r:?}");
                }
            }
        }
        assert_eq!(unknown, None, "{event}: a revision the source never handed out");
    }
}

#[test]
fn composite_damage_since_the_shown_revision_follows_each_event() {
    let setup = || {
        let mut app = app_with(16, 8, "#336699");
        app.run("select.rect", json!({"x": 2, "y": 1, "width": 3, "height": 2})).unwrap();
        app
    };
    let cases: [(&str, Step, Want); 5] = [
        ("nothing changed", |_| {}, Want::Nothing),
        (
            "two pixel-neutral layer selections",
            |app| {
                let id = app.session.active().unwrap().active_layer.unwrap();
                app.session.select_layer(id).unwrap();
                app.session.select_layer(id).unwrap();
            },
            Want::Nothing,
        ),
        (
            "a fill inside the selection",
            |app| {
                app.run("edit.fill", json!({"color": "#cc2200"})).unwrap();
            },
            Want::Covers(PixelRect::new(2, 1, 5, 3)),
        ),
        (
            "a fill followed by pixel-neutral selections",
            |app| {
                app.run("edit.fill", json!({"color": "#cc2200"})).unwrap();
                let id = app.session.active().unwrap().active_layer.unwrap();
                app.session.select_layer(id).unwrap();
                app.session.select_layer(id).unwrap();
            },
            Want::Covers(PixelRect::new(2, 1, 5, 3)),
        ),
        (
            "the canvas grows",
            |app| {
                app.run("image.canvasSize", json!({"width": 24, "height": 12})).unwrap();
            },
            Want::Everything,
        ),
    ];
    check_damage(setup, |app, ctx, since| pass(&CompositePrep::new(app, ctx, 0, None, None).unwrap().source(app), since), &cases);
}

#[test]
fn neutral_revisions_preserve_a_prior_bounded_pixel_change() {
    let mut app = app_with(16, 8, "#336699");
    app.run("select.rect", json!({"x": 2, "y": 1, "width": 3, "height": 2})).unwrap();
    let ctx = app.ctx().clone();
    let before = CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app).revision();
    app.run("edit.fill", json!({"color": "#cc2200"})).unwrap();
    let expected = app.session.active().unwrap().last_damage.unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.session.select_layer(id).unwrap();
    app.session.select_layer(id).unwrap();
    let damage = CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app).damage(before).unwrap();
    let expected = PixelRect::new(expected.x0, expected.y0, expected.x1, expected.y1);
    assert_eq!(damage.intersect(&expected), damage);
    assert_eq!(damage.intersect(&PixelRect::new(2, 1, 5, 3)), PixelRect::new(2, 1, 5, 3));
}

#[test]
fn history_damage_includes_effect_reach_only_once() {
    let mut app = app_with(1024, 1024, "transparent");
    app.session
        .edit("add effect", |doc, _| {
            doc.layers.last_mut().unwrap().effects.items.push(photocraft_doc::Effect::default_drop_shadow());
            Ok(())
        })
        .unwrap();
    assert!(photocraft_compose::effects::margin(app.session.active().unwrap().doc.layers.last().unwrap()) > 0);
    app.run("select.rect", json!({"x": 300, "y": 300, "width": 3, "height": 2})).unwrap();
    let ctx = app.ctx().clone();
    let before = CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app).revision();
    let doc_revision = app.session.active().unwrap().revision;
    app.run("edit.fill", json!({"color": "#cc2200"})).unwrap();
    let damage = app.session.active().unwrap().damage_since(doc_revision).unwrap();
    assert!(!damage.is_empty());
    assert!(damage.x0 > 0 && damage.y0 > 0 && damage.x1 < 1024 && damage.y1 < 1024);
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.session.select_layer(id).unwrap();
    app.session.select_layer(id).unwrap();
    let actual = CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app).damage(before).unwrap();
    assert_eq!(actual, PixelRect::new(damage.x0, damage.y0, damage.x1, damage.y1));
}

#[test]
fn display_converts_no_more_than_the_recorded_damage() {
    use craft::behavior::{CanvasView, kurbo::Point};
    use craft::ui::canvas::{Canvas, RasterDisplay};
    use craft::ui::contract::{Show as _, ShowContainer as _};
    use craft::ui::testing::Fixture;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    struct Counted<'a> {
        source: PhotoComposite<'a>,
        pixels: &'a Cell<usize>,
    }
    impl RasterSource for Counted<'_> {
        fn size(&self) -> PixelSize {
            self.source.size()
        }
        fn revision(&self) -> Revision {
            self.source.revision()
        }
        fn damage(&self, since: Revision) -> Option<PixelRect> {
            self.source.damage(since)
        }
        fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
            self.pixels.set(self.pixels.get() + out.pixels.len());
            self.source.read(region, factor, out);
        }
    }

    let app = Rc::new(RefCell::new(app_with(16, 8, "#336699")));
    app.borrow_mut().run("select.rect", json!({"x": 2, "y": 1, "width": 3, "height": 2})).unwrap();
    let pixels = Rc::new(Cell::new(0));
    let (shown_app, shown_pixels) = (Rc::clone(&app), Rc::clone(&pixels));
    let view = CanvasView::new(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(160.0, 80.0)), Point::new(8.0, 4.0), 1.0);
    let mut fixture = Fixture::new(move |ui| {
        let mut app = shown_app.borrow_mut();
        let prep = CompositePrep::new(&mut app, ui.ctx(), 0, None, None).unwrap();
        let source = Counted { source: prep.source(&app), pixels: &shown_pixels };
        Canvas::new("Document").view(view).show(ui, |ui, body| {
            RasterDisplay::new("Picture", &source).show_at(ui, body.rect);
        });
    });
    fixture.room(egui::vec2(160.0, 80.0));
    fixture.settle();
    pixels.set(0);
    let id = app.borrow().session.active().unwrap().active_layer.unwrap();
    app.borrow_mut().session.select_layer(id).unwrap();
    app.borrow_mut().session.select_layer(id).unwrap();
    fixture.frame(Vec::new());
    assert_eq!(pixels.get(), 0, "neutral revisions need no conversion");
    app.borrow_mut().run("edit.fill", json!({"color": "#cc2200"})).unwrap();
    let damage = app.borrow().session.active().unwrap().last_damage.unwrap();
    app.borrow_mut().session.select_layer(id).unwrap();
    app.borrow_mut().session.select_layer(id).unwrap();
    pixels.set(0);
    fixture.frame(Vec::new());
    let area = (damage.width() as usize) * (damage.height() as usize);
    assert!(pixels.get() > 0 && pixels.get() <= area, "converted {} pixels for {damage:?}", pixels.get());
}

#[test]
fn overlay_damage_since_the_shown_revision_follows_each_event() {
    // A layer mask shown over the composite.
    let setup = || {
        let mut app = app_with(16, 8, "#336699");
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.run("view.layerMask", json!({"mode": "overlay"})).unwrap();
        app
    };
    let cases: [(&str, Step, Want); 2] = [
        ("nothing changed", |_| {}, Want::Nothing),
        (
            "the mask shows in grey instead",
            |app| {
                app.run("view.layerMask", json!({"mode": "gray"})).unwrap();
            },
            Want::Everything,
        ),
    ];
    check_damage(setup, |app, ctx, since| pass(&OverlayPrep::new(app, ctx, 0, None, None).unwrap().source(app), since), &cases);
}

/// A spot channel's edits and the picture under it redraw the overlay that prints it.
#[test]
fn overlay_damage_after_a_spot_edit_covers_the_ink() {
    // A cyan spot inked everywhere, over white.
    let setup = || {
        let mut app = app_with(16, 8, "white");
        app.run("select.all", json!({})).unwrap();
        app.run("channel.newSpot", json!({"color": "#00ffff", "solidity": 0})).unwrap();
        app
    };
    let all = PixelRect::new(0, 0, 16, 8);
    let cases: [(&str, Step, Want); 3] = [
        (
            "the spot's colour",
            |app| {
                app.run("channel.options", json!({"channel": 0, "color": "#ff00ff"})).unwrap();
            },
            Want::Redraws(all),
        ),
        (
            "the spot's solidity",
            |app| {
                app.run("channel.options", json!({"channel": 0, "solidity": 100})).unwrap();
            },
            Want::Redraws(all),
        ),
        (
            "a fill under the spot",
            |app| {
                app.run("edit.fill", json!({"color": "#808080"})).unwrap();
            },
            Want::Redraws(all),
        ),
    ];
    check_damage(setup, |app, ctx, since| pass(&OverlayPrep::new(app, ctx, 0, None, None).unwrap().source(app), since), &cases);
}

#[test]
fn read_reduces_from_the_region_origin() {
    // Columns 1 and 2 red, 0 and 3 blue: the texel of columns 1..3 at factor 2 is red, where cells counted from the
    // document's origin would mix red and blue.
    let mut app = app_with(4, 2, "#0000ff");
    app.run("select.rect", json!({"x": 1, "y": 0, "width": 2, "height": 2})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let ctx = app.ctx().clone();
    let source = CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app);
    let (red, blue) = (Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 0, 255));
    assert_eq!(read(&source, PixelRect::new(1, 0, 3, 2), 2).pixels, vec![red]);
    assert_eq!(read(&source, PixelRect::new(0, 0, 4, 1), 1).pixels, vec![blue, red, red, blue]);
    // A region past the document's end reads what lies inside and leaves the rest.
    assert_eq!(read(&source, PixelRect::new(2, 0, 6, 1), 1).pixels, vec![red, blue, Color32::TRANSPARENT, Color32::TRANSPARENT]);
}

#[test]
fn a_channel_view_draws_its_own_picture() {
    let mut app = app_with(8, 4, "#ffffff");
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    let ctx = app.ctx().clone();
    assert!(OverlayPrep::new(&mut app, &ctx, 0, None, None).is_none(), "the plain composite has no overlay");
    app.run("select.editInQuickMaskMode", json!({})).unwrap();
    let px = read(&OverlayPrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app), PixelRect::new(0, 0, 8, 1), 1).pixels;
    assert_eq!(Color32::WHITE.blend(px[0]), Color32::WHITE, "selected: nothing over the composite");
    assert_eq!(px[6], Color32::from_rgba_premultiplied(128, 0, 0, 128), "masked: the rubylith");
    let composite = read(&CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app), PixelRect::new(0, 0, 8, 1), 1).pixels;
    assert_eq!(composite[6], Color32::WHITE, "the composite stays the document's");
}

/// The CPU canvas (no usable GPU; a flipped or pixel-aspect-corrected view, or a GPU fallback, on a GPU machine) shows
/// a filter dialog's live preview. It showed the document unfiltered.
#[test]
fn the_composite_shows_a_filter_dialogs_preview() {
    for (size, zoom) in [([64u32, 48u32], 1.0f32), ([2400, 2200], 0.25)] {
        let mut app = app_with(size[0], size[1], "white");
        app.background_jobs = false;
        app.run("edit.fill", json!({"color": "#3399cc"})).unwrap();
        app.ui.views.get_mut(0).expect("the view").zoom = zoom;
        let ctx = app.ctx().clone();
        let first = PixelRect::new(0, 0, 1, 1);
        let plain = read(&CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app), first, 1).pixels;
        crate::filter_dialog::open(&mut app, "filter.stylize.solarize").unwrap();
        // The Preview box is on from the dialog's first drawn pass.
        for _ in 0..2 {
            crate::smart_filter_dialog::tests::pass(&mut app);
        }
        let shown = read(&CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app), first, 1).pixels;
        let preview = app.filter_preview.as_ref().and_then(|p| p.result.clone()).expect("the dialog's preview");
        let (display, _) = crate::canvas::canvas_display(&app, &preview, None);
        let expected = crate::canvas::display_image(display.as_deref(), &photocraft_compose::flatten(&preview));
        assert_eq!(shown.first(), expected.pixels.first(), "{size:?}: the canvas shows the preview");
        assert_ne!(shown, plain, "{size:?}: not the unfiltered document");
    }
}

/// A preview that changes the canvas's dimensions (Image › Image Rotation › Arbitrary 90° on a 64×32 document) lies
/// over its own area, centred on the document's, as the GPU canvas centres it; the channel view over it follows.
#[test]
fn a_size_changing_preview_lies_centred_on_the_document() {
    let size = |w, h| photocraft_geom::Size::new(w, h);
    assert_eq!(super::super::textures::preview_bounds(size(64, 32), size(64, 32), 1), DRect::new(0, 0, 64, 32));
    assert_eq!(super::super::textures::preview_bounds(size(2401, 2200), size(1201, 1100), 2), DRect::new(0, 0, 2401, 2200), "a proxy");
    assert_eq!(super::super::textures::preview_bounds(size(64, 32), size(32, 64), 1), DRect::new(16, -16, 48, 48), "rotated");
    assert_eq!(super::super::textures::preview_bounds(size(64, 32), size(80, 40), 1), DRect::new(-8, -4, 72, 36), "canvas grown");
    assert_eq!(super::super::textures::preview_bounds(size(4000, 2000), size(1000, 2000), 2), DRect::new(1000, -1000, 3000, 3000), "a rotated proxy");

    let mut app = app_with(64, 32, "white");
    app.background_jobs = false;
    app.run("channel.new", json!({"name": "Alpha", "fill": "black"})).unwrap();
    // Rotate Canvas has no Preview of its own (#14462); the canvas still places a size-changing one.
    let id = crate::filter_dialog::open_previewing(&mut app, "image.rotation.arbitrary").unwrap();
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "angle", json!(90.0)).unwrap();
    // The Preview box is on from the dialog's first drawn pass.
    for _ in 0..2 {
        crate::smart_filter_dialog::tests::pass(&mut app);
    }
    let bounds = |app: &mut PhotocraftApp| CompositePrep::new(app, &ctx, 0, None, None).unwrap().source(app).bounds();
    let rotated = DRect::new(16, -16, 48, 48);
    assert_eq!(bounds(&mut app), rotated, "centred on the document");
    // Preview off, then Cancel: the document again, unchanged.
    let revision = app.session.active().unwrap().revision;
    use egui_kittest::kittest::Queryable;
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .with_context(ctx.clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    h.run_steps(2);
    h.get_by_label("Preview").click();
    h.run_steps(2);
    let app = h.state_mut();
    assert_eq!(bounds(app), DRect::new(0, 0, 64, 32), "Preview off");
    app.framework.dialogs.cancel_by_id(&ctx, id);
    assert_eq!(bounds(app), DRect::new(0, 0, 64, 32), "after Cancel");
    assert_eq!(app.session.active().unwrap().revision, revision, "the document untouched");
}

/// #2078: a channel or mask view over the canvas shows the live stroke, not the committed channel over it, and a stroke
/// dropped without committing shows the channel as it was. One row per view target.
#[test]
fn channel_views_show_live_strokes_and_drop_discarded_ones() {
    type Target = fn(&mut PhotocraftApp);
    let targets: [(&str, Target); 5] = [
        ("quick mask", |app| {
            app.run("select.rect", json!({"x": 0, "y": 0, "width": 120, "height": 60})).unwrap();
            app.run("select.editInQuickMaskMode", json!({})).unwrap();
        }),
        ("alpha", |app| {
            app.run("channel.new", json!({"fill": "white"})).unwrap();
        }),
        ("red", |app| {
            app.run("channel.target", json!({"channel": "red"})).unwrap();
        }),
        ("layer mask, gray", |app| {
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.run("view.layerMask", json!({"mode": "gray"})).unwrap();
        }),
        ("layer mask, overlay", |app| {
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.run("view.layerMask", json!({"mode": "overlay"})).unwrap();
        }),
    ];
    let stroke = PixelRect::new(0, 24, 60, 36);
    let rect = DRect::new(0, 24, 60, 36);
    for (target, setup) in targets {
        let mut app = app_with(120, 60, "white");
        setup(&mut app);
        app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0]})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
        app.ui.tool = crate::state::Tool::Brush;
        let ctx = app.ctx().clone();
        let original = app.session.documents()[0].doc.clone();
        let view = |app: &PhotocraftApp, doc: &Document| {
            let st = &app.session.documents()[0];
            crate::channel_view::render(doc, &st.channel_view, rect, 1, crate::channel_view::Look::default(), st.active_layer).unwrap()
        };
        let shown = |app: &mut PhotocraftApp| read(&OverlayPrep::new(app, &ctx, 0, None, None).unwrap().source(app), stroke, 1).pixels;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 10.0, y: 30.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Move { x: 50.0, y: 30.0, pressure: 1.0 }, m);
        let live = crate::canvas::display_doc(&mut app, 0).0;
        let painted = shown(&mut app);
        assert_eq!(painted, view(&app, &live), "{target}: the live stroke");
        assert_ne!(painted, view(&app, &original), "{target}: the target was painted");
        assert!(std::sync::Arc::ptr_eq(&original, &app.session.documents()[0].doc), "{target}: nothing committed");
        crate::canvas::testing::key(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert_eq!(shown(&mut app), view(&app, &original), "{target}: a dropped stroke shows the channel as it was");
    }
}

mod gpu_tests {
    use super::*;

    /// The proxy preview is the GPU canvas's: without a GPU a large document's adjustment previews through the CPU proxy,
    /// and the layer path shows nothing (`adjust_preview::cheap`).
    #[test]
    fn during_a_proxy_adjustment_the_overlay_reads_the_preview_and_the_composite_the_document() {
        let _gpu = photocraft_testkit::gpu_lock();
        let rs = match std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) {
            Ok(rs) => rs,
            Err(error) if std::env::var_os("PHOTOCRAFT_REQUIRE_GPU_TESTS").is_some() => std::panic::resume_unwind(error),
            Err(_) => return,
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.set_wgpu(rs);
        app.run("file.new", json!({"width": 2400, "height": 2200, "background": "#808080"})).unwrap();
        let st = app.session.active().unwrap();
        assert!(crate::proxy::preview_factor(&st.doc, crate::proxy::reduced_previews(&app)) > 1, "large enough for a proxy preview");
        // Only the red channel shows: an opaque view over the composite.
        let view = &mut app.session.active_mut().unwrap().channel_view;
        view.set_color_visible(1, false);
        view.set_color_visible(2, false);
        let ctx = app.ctx().clone();
        let at = PixelRect::new(0, 0, 1, 1);
        let overlay_before = read(&OverlayPrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app), at, 1).pixels;
        let composite_before = read(&CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app), at, 1).pixels;
        let id = crate::adjust_dialog::open(&mut app, "image.adjustments.levels").unwrap();

        assert!(crate::adjust_dialog::apply_params(&mut app, id, &json!({"inBlack": 100})), "the dialog takes the black point");
        assert!(crate::adjust_preview::display_doc(&mut app, 0).is_some(), "the adjustment previews");
        let overlay = read(&OverlayPrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app), at, 1).pixels;
        let composite = read(&CompositePrep::new(&mut app, &ctx, 0, None, None).unwrap().source(&app), at, 1).pixels;
        assert_ne!(overlay, overlay_before, "the overlay shows the live preview");
        assert_eq!(composite, composite_before, "the composite shows the document");
    }
}
