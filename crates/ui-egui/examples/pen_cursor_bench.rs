//! Pen hover cursor work (#15465) through the shared tool host and offscreen frame fixture.
//!
//! Run with `cargo xtask perf --bench pen_cursor_bench`. Each sample includes the canvas host
//! frame at 125% pixel density. It ends before any OS cursor upload or GPU presentation.

use std::{sync::Arc, time::Instant};

use craft::behavior::CanvasView;
use craft::behavior::kurbo::{Point, Size};
use craft::draw::ui::pen::PenTool;
use craft::host::target::{Backend, Os, Runtime, ShellInfo};
use craft::ui::canvas::{Canvas, CanvasTool, ToolHost, tool_pass};
use craft::ui::contract::{Props as _, ShowContainer as _};
use craft::ui::testing::{Frames, install_spec, pointer};
use egui::{CursorIcon, Event, pos2};
use photocraft_testkit::perf::{Summary, report, row, write_report};
use serde_json::json;

const BENCH: &str = "pen_cursor_bench";
const SCALE: f32 = 1.25;

fn frames(shell: ShellInfo) -> Result<Frames, String> {
    let mut frames = Frames::uninstalled();
    let mut spec = install_spec();
    spec.shell_info = shell;
    craft::ui::install(&frames.ctx, spec).map_err(|err| format!("install the cursor fixture: {err}"))?;
    frames.pixels_per_point(SCALE);
    Ok(frames)
}

fn frame(frames: &mut Frames, pen: &mut PenTool, event: Event) -> f64 {
    let start = Instant::now();
    frames.frame(vec![event], |ui| {
        let view = CanvasView::new(ui.available_rect_before_wrap(), Point::new(256.0, 256.0), 1.0);
        Canvas::new("Pen cursor").id(egui::Id::unique(BENCH)).document(Size::new(512.0, 512.0)).view(view).tool_regions(pen.regions()).show(ui, |ui, cx| {
            if let Some(pass) = tool_pass(&cx.env) {
                let _ = ToolHost::new().run(ui, &pass, pen);
            }
        });
    });
    start.elapsed().as_secs_f64() * 1000.0
}

fn moved(sample: usize) -> Event {
    pointer::moved(pos2(320.0 + (sample % 32) as f32, 300.0 + (sample / 32) as f32))
}

fn painted_shapes(frames: &mut Frames) -> Result<usize, String> {
    let mut output = frames.take_paint().ok_or("paint was not recorded")?;
    output.textures_delta.clear();
    Ok(output.shapes.iter().filter(|shape| !matches!(shape.shape, egui::Shape::Noop)).count())
}

fn main() -> Result<(), String> {
    let mut samples = 1000;
    let mut json_path = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--quick" => samples = 100,
            "--json" => json_path = Some(args.next().ok_or("--json needs an output path")?),
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }

    // X11 uses scaled native bitmaps. The web shell selects the same cursor's painted fallback.
    let native_shell = ShellInfo::default().os(Os::Linux).backend(Backend::X11);
    let fallback_shell = native_shell.clone().runtime(Runtime::Web);
    let mut native = frames(native_shell)?;
    let mut native_pen = PenTool::new();

    frame(&mut native, &mut native_pen, pointer::gone());
    frame(&mut native, &mut native_pen, moved(0));
    let first = native.cursor_image().ok_or("Pen hover did not generate a native cursor image")?;
    let [width, height] = first.size;
    if width == 0 || height == 0 || first.hotspot[0] >= width || first.hotspot[1] >= height || !first.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0) {
        return Err("Pen hover generated an invalid cursor bitmap".into());
    }
    let mut last_image = Arc::clone(&first.rgba);

    // Record paint in a separate driver so output recording adds no work to the timed frames.
    {
        let mut recorded = frames(fallback_shell.clone())?;
        let mut pen = PenTool::new();
        recorded.record_paint();
        frame(&mut recorded, &mut pen, pointer::gone());
        let idle_shapes = painted_shapes(&mut recorded)?;
        frame(&mut recorded, &mut pen, moved(0));
        if painted_shapes(&mut recorded)? <= idle_shapes {
            return Err("Pen hover did not paint the fallback cursor".into());
        }
    }
    let mut fallback = frames(fallback_shell)?;
    let mut fallback_pen = PenTool::new();
    frame(&mut fallback, &mut fallback_pen, moved(0));

    let mut cold = Vec::with_capacity(samples);
    let mut warm = Vec::with_capacity(samples);
    let mut painted = Vec::with_capacity(samples);
    for sample in 1..=samples {
        // Leaving the window releases the real cache before this cold sample. Keep the
        // previous bitmap alive so a fresh allocation cannot reuse its address.
        frame(&mut native, &mut native_pen, pointer::gone());
        if native.cursor_image().is_some() {
            return Err("Pen cursor image remained after the pointer left".into());
        }
        cold.push(frame(&mut native, &mut native_pen, moved(sample)));
        let image = native.cursor_image().ok_or("Pen reentry did not generate a cursor image")?;
        if Arc::ptr_eq(&last_image, &image.rgba) {
            return Err("Pen reentry reused an image after the cursor was cleared".into());
        }
        last_image = Arc::clone(&image.rgba);
    }

    for sample in 1..=samples {
        warm.push(frame(&mut native, &mut native_pen, moved(sample)));
        let image = native.cursor_image().ok_or("Pen pointer move did not produce a cursor image")?;
        if !Arc::ptr_eq(&last_image, &image.rgba) {
            return Err("Pen pointer movement rebuilt an unchanged cursor image".into());
        }
    }

    for sample in 1..=samples {
        painted.push(frame(&mut fallback, &mut fallback_pen, moved(sample)));
        if fallback.cursor_image().is_some() || fallback.cursor_icon() != CursorIcon::None {
            return Err("Pen painted fallback did not suppress the system cursor".into());
        }
    }

    let timings = [("Pen native cursor, cold", cold), ("Pen native cursor, warm", warm), ("Pen painted cursor", painted)];
    for (name, values) in &timings {
        if let Some(summary) = Summary::of(values) {
            println!("{name}: p50 {:.3} ms, p95 {:.3} ms ({} samples at {SCALE}×)", summary.p50, summary.p95, summary.n);
        }
    }
    if let Some(path) = json_path {
        let rows = timings.iter().map(|(name, values)| row(name, values, None, None)).collect();
        write_report(&path, &report(BENCH, json!({ "pixels_per_point": SCALE, "native_bitmap": [width, height] }), rows, None))?;
    }
    Ok(())
}
