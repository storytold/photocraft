//! The paragraph box's overflow port: text the box can't hold draws a red “+” port on the box's
//! bottom edge near its right corner (Photoshop marks a boxed overflow the same way), and
//! automation reads the flag without a screen (`ui.type_box_overflow`). The port is a pure
//! painter overlay, so the GPU check looks for its red disc on the canvas, and for the
//! hover-delay tooltip's card next to it. Skips when no GPU adapter exists (see
//! `marquee_autoscroll` for the same pattern).

use egui::Pos2;
use photocraft_ui_egui::PhotocraftApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;
/// An offscreen wgpu harness running the app. Skip only when the machine has no adapter at
/// all: enumerate the adapters the same setup would consider, and only then let kittest's
/// render-state creation run uncaught — a real device failure fails the test instead of
/// posing as a headless machine (kittest turns it into a panic).
fn harness() -> Option<Harness> {
    use eframe::egui_wgpu::WgpuSetup;
    use eframe::wgpu;
    let setup = photocraft_ui_egui::gpu_canvas::wgpu_setup();
    let backends = match &setup {
        WgpuSetup::CreateNew(create) => create.instance_descriptor.backends,
        WgpuSetup::Existing(_) => wgpu::Backends::all(),
    };
    let instance = pollster::block_on(setup.new_instance());
    if pollster::block_on(instance.enumerate_adapters(backends)).is_empty() {
        eprintln!("skipping: no GPU adapter");
        return None;
    }
    let rs = egui_kittest::wgpu::create_render_state(setup, Default::default());
    Some(
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 640.0))
            .with_max_steps(64)
            .renderer(egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs))
            .build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                if let Some(rs) = cc.wgpu_render_state.as_ref() {
                    app.set_wgpu(rs.clone());
                }
                app
            }),
    )
}

/// An 800 × 600 white page, a paragraph box almost three lines tall holding six lines of 24 pt
/// type (clearly overflowing), the camera at 100 % centred on the box, and an inline edit
/// session open.
fn setup(h: &mut Harness) {
    let id = {
        let app = h.state_mut();
        let _ = app.run("file.new", json!({"width": 800, "height": 600, "background": "white"}));
        let id = app.run("type.create", json!({"text": "ab\ncd\nef\ngh\nij\nkl", "size": 24, "box": [100, 100, 300, 62]})).expect("create")["layer"]
            .as_u64()
            .expect("layer id");
        app.session.select_layer(photocraft_doc::LayerId(id)).expect("select");
        app.ui.extras.rulers = false;
        id
    };
    h.run_steps(2);
    {
        let app = h.state_mut();
        // The session belongs to the Type tool: any other tool commits it (lib.rs update).
        app.ui.tool = photocraft_ui_egui::state::Tool::Type;
        app.ui.views[0].zoom = 1.0;
        app.ui.views[0].center = [400.0, 300.0];
        app.ui.text_edit = Some(photocraft_ui_egui::state::TextEdit {
            layer: id,
            document: None,
            caret: 0,
            anchor: 0,
            session: "s".into(),
            created: false,
            mask: false,
            dragging: false,
            resize: None,
            preedit: None,
        });
    }
    h.run_steps(4);
}

/// The port's screen position: the box's corner through the 100 % view centred on (400, 300),
/// then exactly where `type_tool::overflow_port` hangs it — a little in from the corner, half
/// a disc below the edge. The `ui.type_box_overflow` flag is the automation contract for the
/// same fact without any screen.
fn port_screen(h: &Harness) -> Pos2 {
    let app = h.state();
    assert!(app.ui.type_box_overflow, "six lines overflow a ~2-line box");
    let rect = app.last_canvas_rect;
    let (corner_x, corner_y, centre_x, centre_y) = (400.0_f32, 162.0_f32, 400.0_f32, 300.0_f32);
    let corner = rect.center() + egui::vec2(corner_x - centre_x, corner_y - centre_y);
    corner + egui::vec2(-7.0, 4.5)
}

/// The rendered frame as flat RGBA rows (kittest's snapshot pixels).
struct Raster {
    w: usize,
    px: Vec<u8>,
}

fn render(h: &mut Harness) -> Raster {
    let img = h.render().expect("render");
    let (w, px) = (img.width() as usize, img.into_raw());
    Raster { w, px }
}

impl Raster {
    /// The pixel's RGB at an image position.
    fn rgb(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let i = (y * self.w + x) * 4;
        (self.px[i], self.px[i + 1], self.px[i + 2])
    }
}

fn reddish(img: &Raster, allowed: egui::Rect) -> usize {
    let mut n = 0;
    for y in allowed.min.y as usize..allowed.max.y as usize {
        for x in allowed.min.x as usize..allowed.max.x as usize {
            let (r, g, b) = img.rgb(x, y);
            // The port's danger red: the red channel well above both others, which a white page
            // and black type never produce.
            if r > 200 && g < 140 && b < 140 {
                n += 1;
            }
        }
    }
    n
}

/// The tooltip's card: an opaque grey block on the white canvas near the port.
fn grey_card(img: &Raster, allowed: egui::Rect) -> usize {
    let mut n = 0;
    for y in allowed.min.y as usize..allowed.max.y as usize {
        for x in allowed.min.x as usize..allowed.max.x as usize {
            let (r, g, b) = img.rgb(x, y);
            if (40..=150).contains(&r) && (i32::from(r) - i32::from(g)).abs() <= 6 && (i32::from(g) - i32::from(b)).abs() <= 6 {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn the_overflow_port_draws_on_the_box_and_its_tooltip_hovers() {
    let Some(mut h) = harness() else { return };
    setup(&mut h);
    let canvas = h.state().last_canvas_rect;
    let port = port_screen(&h);
    let around = egui::Rect::from_center_size(port, egui::vec2(40.0, 60.0)).intersect(canvas);
    // The port's red disc on the white page.
    let img = render(&mut h);
    let hits = reddish(&img, around);
    assert!(hits > 10, "the port's red disc renders: {hits} reddish pixels near {port:?}");
    // Hovering it past the tooltip's delay shows the explanation card to its right.
    h.hover_at(port);
    h.run_steps(8);
    let img = render(&mut h);
    let card = egui::Rect::from_min_size(port + egui::vec2(0.0, 0.0), egui::vec2(340.0, 44.0)).intersect(canvas);
    let hits = grey_card(&img, card);
    assert!(hits > 300, "the overflow tooltip renders: {hits} card pixels by {port:?}");
}
