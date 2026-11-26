//! The Zoom tool on the whole app: the GPU canvas keeps its composite while a zoom eases, and the
//! temporary and spring zooms ignore the Zoom Out mode of the options bar.

use craft::behavior::canvas::ZoomScheme;
use craft::behavior::kurbo::Size;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::document_display::{CanvasHost, shown_camera};
use crate::state::Tool;

const DOC: [u32; 2] = [400, 300];

fn doc_size() -> Size {
    Size::new(f64::from(DOC[0]), f64::from(DOC[1]))
}

/// The platform's modifiers for `Cmd` (⌘ on the Mac, Ctrl elsewhere), as egui-winit reports them.
fn platform(m: Modifiers) -> Modifiers {
    let mut out = Modifiers { alt: m.alt, shift: m.shift, ctrl: m.ctrl, ..Default::default() };
    if m.command {
        out.command = true;
        if cfg!(target_os = "macos") {
            out.mac_cmd = true;
        } else {
            out.ctrl = true;
        }
    }
    out
}

/// The app with `tool` selected, Zoom Out chosen, no animation and the view at 100 %.
fn zoom_out_harness(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": DOC[0], "height": DOC[1]})).unwrap();
        // No animation and no click-to-centre: a step lands on the first frame where the test reads it.
        app.run("prefs.set", json!({"values": {"general.animatedZoom": false, "tools.zoomClickedPointToCenter": false}})).unwrap();
        app.ui.tool = tool;
        app.ui.tool_options.zoom_in = false;
        app
    });
    h.run_steps(4);
    h.state_mut().run("view.actualPixels", json!({})).unwrap();
    h.run_steps(2);
    h
}

fn key(h: &mut Harness<'static, Box<PhotocraftApp>>, key: Key, pressed: bool, m: Modifiers) {
    let m = platform(m);
    h.event(Event::ModifiersChanged(m));
    h.event(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: m });
    h.run_steps(2);
}

fn click(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2, m: Modifiers) {
    let m = platform(m);
    h.event(Event::ModifiersChanged(m));
    h.event(Event::PointerMoved(p));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: m });
        h.step();
    }
}

/// With Zoom Out selected, the zoom a held ⌘⌥Space puts in effect keeps its own direction (zooming out), as the held
/// Zoom In turned around by ⌥ does.
#[test]
fn temporary_and_spring_zooms_ignore_the_zoom_out_mode() {
    // (case, key held, modifiers held with the key, modifiers at the click, ladder direction)
    let cases = [("⌘⌥Space click", Key::Space, Modifiers::COMMAND | Modifiers::ALT, Modifiers::COMMAND | Modifiers::ALT, -1)];
    for (name, held, held_mods, click_mods, dir) in cases {
        let mut h = zoom_out_harness(Tool::Brush);
        let start = h.state().current_zoom();
        let p = h.state().last_canvas_rect.center();
        key(&mut h, held, true, held_mods);
        click(&mut h, p, click_mods);
        key(&mut h, held, false, Modifiers::NONE);
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(2);
        let want = ZoomScheme::PHOTOSHOP.step(f64::from(start), dir, doc_size()) as f32;
        assert!((h.state().current_zoom() - want).abs() < 1e-5, "{name}: {} vs {want}", h.state().current_zoom());
        assert_eq!(h.state().ui.tool, Tool::Brush, "{name}: the selected tool is unchanged");
        assert!(!h.state().ui.tool_options.zoom_in, "{name}: Zoom Out stays selected");
    }
}

mod gpu_tests {
    use super::*;

    #[test]
    fn gpu_zoom_animation_reuses_the_document_composite() {
        let Ok(rs) = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())) else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs);
        let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_pixels_per_point(1.0).renderer(renderer).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro());
            let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
            app.set_wgpu(cc.wgpu_render_state.clone().unwrap());
            app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            app.run("edit.fill", json!({"color": "#2060c0"})).unwrap();
            app
        });
        // The harness sets egui's animation time to zero, which turns the framework's motion off; the ease needs it on.
        h.ctx.all_styles_mut(|style| style.animation_time = egui::Style::default().animation_time);
        let shown = |h: &Harness<'static, Box<PhotocraftApp>>| shown_camera(&h.ctx, CanvasHost::main(0), &h.state().ui.views[0], 1.0).0;
        h.run_steps(4);
        h.state_mut().run("view.actualPixels", json!({})).unwrap();
        h.step();
        let time = h.ctx.input(|i| i.time) + 0.3;
        h.input_mut().time = Some(time);
        h.step();
        let id = h.state().session.documents()[0].doc.id;
        assert!(h.state().canvases[&(id, crate::canvas::GPU_OUTPUT)].on_gpu);
        let directory = std::env::var_os("CRAFT_UI_ZOOM_FIXTURES").map(std::path::PathBuf::from);
        if let Some(directory) = &directory {
            std::fs::create_dir_all(directory).unwrap();
        }
        h.state_mut().run("view.zoomIn", json!({})).unwrap();
        // The canvas runs the command in its next pass: the zoom it asks for is the rung after 100 %.
        let target = craft::behavior::canvas::ZoomScheme::PHOTOSHOP.step(1.0, 1, craft::behavior::kurbo::Size::new(400.0, 300.0)) as f32;
        // The ease's length belongs to the framework's motion settings, so the steps are short and the test looks for a
        // frame between the start and the target zoom rather than naming a time for it.
        let mut midway = false;
        for step in 0..=10 {
            let name = format!("{:.2}s", f64::from(step) * 0.03);
            h.state_mut().perf.last_refresh = "";
            h.input_mut().time = Some(time + f64::from(step) * 0.03);
            h.step();
            assert!(h.state().perf.gpu, "{name}: keep the GPU canvas");
            assert_eq!(h.state().perf.last_refresh, "", "{name}: reuse the composite");
            let zoom = shown(&h);
            midway |= zoom > 1.0 && zoom < target;
            let image = h.render().unwrap();
            let center = h.state().last_canvas_rect.center();
            let pixel = image.get_pixel(center.x as u32, center.y as u32);
            assert!(pixel[2] > pixel[0] * 2, "{name}: document remains visible: {pixel:?}");
            if let Some(directory) = &directory {
                image.save(directory.join(format!("photocraft-zoom-{name}.png"))).unwrap();
            }
        }
        assert!(midway, "a frame of the ease showed a zoom between 1 and {target}");
        assert_eq!(shown(&h), target);
        h.state_mut().run("prefs.set", json!({"values": {"general.animatedZoom": false}})).unwrap();
        h.state_mut().run("view.zoomIn", json!({})).unwrap();
        h.state_mut().perf.last_refresh = "";
        h.step();
        assert_eq!(shown(&h), h.state().current_zoom());
        assert!(h.state().perf.gpu);
        assert_eq!(h.state().perf.last_refresh, "", "immediate zoom reuses the GPU composite");
        let image = h.render().unwrap();
        if let Some(directory) = &directory {
            image.save(directory.join("photocraft-zoom-disabled.png")).unwrap();
        }
    }
}
