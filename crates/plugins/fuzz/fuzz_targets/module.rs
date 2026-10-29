#![no_main]
//! Load arbitrary bytes as a WebAssembly filter plug-in (Filter › Plug-ins › Install, or a module
//! in the plug-ins folder): validation must refuse it with an error or load it, never panic.
//! A module that loads runs once on a small image, inside tight fuel, memory and time limits.

use libfuzzer_sys::fuzz_target;
use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_plugins::{Limits, Plugin};
use photocraft_raster::Surface;
use serde_json::json;

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_module_bytes: 1 << 20,
        max_memory_bytes: 16 << 20,
        fuel_base: 1_000_000,
        fuel_per_sample: 1_000,
        wall_time_ms: 2_000,
        ..Limits::default()
    };
    let Ok(plugin) = Plugin::load(data, limits) else { return };
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.fill_rect(Rect::new(0, 0, 8, 6), &[0.25, 0.5, 0.75, 1.0]);
    let _ = plugin.apply(&s, Rect::new(0, 0, 8, 6), None, &json!({}));
});
