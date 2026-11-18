//! Channel-edit regressions retained across the shared UI migration.
use super::super::*;
use super::*;

const GUIDE: Color32 = Color32::from_rgb(0x4a, 0xff, 0xff);

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    for (orientation, position) in [("vertical", 100), ("vertical", 300), ("horizontal", 100), ("horizontal", 260)] {
        app.run("view.newGuide", json!({"orientation": orientation, "position": position})).unwrap();
    }
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 160})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::RectMarquee;
    app.ui.extras.rulers = false;
    app.ui.extras.guides = true;
    app
}

fn painted(app: &mut PhotocraftApp, ppp: f32, zoom: f32, primary: bool) -> (Vec<usize>, Vec<usize>) {
    let ctx = app.ctx().clone();
    PhotocraftApp::setup_context(&ctx, Default::default());
    ctx.set_pixels_per_point(ppp);
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(700.0, 500.0));
    let view = View { zoom, center: [300.0, 200.0], doc_size: [600, 400], ..Default::default() };
    let mut frames = craft::ui::testing::Frames::over(ctx.clone());
    frames.screen(rect.size());
    frames.pixels_per_point(ppp);
    frames.record_paint();
    for _ in 0..3 {
        app.framework_before_input(&ctx);
        frames.frame_without_input(Vec::new(), |ui| {
            let host = if primary { CanvasHost::main(0) } else { CanvasHost { idx: 0, window: Some(64) } };
            canvas_view(app, ui, host, rect, view.clone(), primary);
        });
    }
    let mut output = frames.take_paint().unwrap();
    output.textures_delta.clear();
    let (mut guides, mut ants) = (Vec::new(), Vec::new());
    fn leaves<'a>(s: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
        match s {
            egui::Shape::Vec(children) => children.iter().for_each(|s| leaves(s, out)),
            s => out.push(s),
        }
    }
    let mut shapes = Vec::new();
    for s in &output.shapes {
        leaves(&s.shape, &mut shapes);
    }
    for (i, s) in shapes.into_iter().enumerate() {
        match s {
            egui::Shape::LineSegment { stroke, .. } if stroke.color == GUIDE => guides.push(i),
            egui::Shape::Mesh(m)
                if m.vertices.iter().any(|v| v.color.r() == 0 && v.color.g() == 0 && v.color.b() == 0 && v.color.a() > 0)
                    && m.vertices.iter().any(|v| v.color == Color32::WHITE) =>
            {
                ants.push(i)
            }
            _ => {}
        }
    }
    (guides, ants)
}

#[test]
fn isolated_channel_transform_ants_stay_above_guides_in_each_view() {
    for colors in [vec![1], vec![0, 2]] {
        for warp in [false, true] {
            let mut app = app();
            app.run("channel.target", json!({"colors": colors})).unwrap();
            let ctx = egui::Context::default();
            crate::transform_tool::begin(&mut app, &ctx).unwrap();
            if warp {
                crate::transform_tool::enter_warp(&mut app, &ctx);
                crate::channel_transform_preview::edit(&mut app, |t| {
                    t.warp = Some(photocraft_geom::warp::Warp::preset(photocraft_geom::warp::WarpStyle::Arc, 25.0, t.rect));
                });
            } else {
                crate::channel_transform_preview::edit(&mut app, |t| {
                    t.quad = t.quad.map(|[x, y]| [x + 37.0, y + 19.0]);
                });
            }
            for primary in [true, false] {
                let (guides, ants) = painted(&mut app, 1.5, 0.5, primary);
                assert!(!ants.is_empty(), "{colors:?}, warp={warp}, primary={primary}");
                // The shared canvas now shows the document's guides in secondary views too.
                assert_eq!(guides.len(), 4);
                assert!(ants.iter().min().unwrap() > guides.iter().max().unwrap());
            }
            crate::transform_tool::cancel(&mut app);
        }
    }
}
