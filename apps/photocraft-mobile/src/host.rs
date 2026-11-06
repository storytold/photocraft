//! PhotoCraft hosted in a native view: the platform owns the app, the window and the frame clock
//! (SwiftUI with a `CADisplayLink`, later Compose with `Choreographer`); this owns egui, wgpu and
//! [`PhotocraftApp`], and draws into the view's surface when asked.
//!
//! No winit and no eframe runner: eframe's runner wants to own the process (`UIApplicationMain`,
//! `NativeActivity`), which leaves no room for native chrome around the canvas. The frame order is
//! eframe's own (`raw_input_hook`, then `logic` and `ui` inside one `run_ui` pass), so the app
//! can't tell the difference.

#![cfg_attr(not(target_os = "ios"), expect(dead_code, reason = "outside iOS only the unit tests build the host; the Swift shell calls it"))]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use eframe::App as _;
use eframe::egui_wgpu::{self, RenderState, RendererOptions, ScreenDescriptor, WgpuConfiguration};
use eframe::wgpu;
use egui::{Event, Pos2, RawInput, TouchDeviceId, TouchId, TouchPhase, ViewportId};

use crate::create_app;
use photocraft_ui_egui::PhotocraftApp;

/// Why the host could not start.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HostError {
    #[error("couldn't create a surface for the view: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    #[error("couldn't set up the GPU: {0}")]
    Gpu(#[from] egui_wgpu::WgpuError),
    #[error("the GPU can't draw to this view")]
    Unsupported,
}

/// A view's size: physical pixels and how many of them make one point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ViewSize {
    pub width_px: u32,
    pub height_px: u32,
    pub scale: f32,
}

impl ViewSize {
    fn points(self) -> egui::Vec2 {
        // Pixel sizes are far below f32's exact-integer range.
        #[expect(clippy::cast_precision_loss, reason = "view sizes are a few thousand pixels")]
        let px = egui::vec2(self.width_px as f32, self.height_px as f32);
        px / self.scale
    }
}

/// One touch as the platform reports it, in points from the view's top-left corner.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Touch {
    pub id: u64,
    pub phase: TouchPhase,
    pub pos: Pos2,
    /// 0..=1 where the device reports it (Apple Pencil, most styluses), 0 otherwise.
    pub force: f32,
}

pub(crate) struct Host {
    ctx: egui::Context,
    app: PhotocraftApp,
    frame: eframe::Frame,
    render_state: RenderState,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    size: ViewSize,
    started: Instant,
    /// Input gathered since the last frame.
    events: Vec<Event>,
    /// The touch that drives egui's pointer: the first one down, until it lifts.
    pointer_touch: Option<u64>,
    /// When egui next wants a frame; worker threads move it earlier through the repaint callback.
    next_frame: Arc<Mutex<Instant>>,
}

impl Host {
    /// Set up wgpu on `target` and build the app.
    ///
    /// # Safety
    /// `target` must stay valid (for a `CoreAnimationLayer`, the layer stays alive) until the
    /// host is dropped.
    #[expect(unsafe_code, reason = "wgpu can only make a surface from a raw platform handle")]
    pub(crate) unsafe fn new(target: wgpu::SurfaceTargetUnsafe, size: ViewSize) -> Result<Self, HostError> {
        let options = crate::options();
        let config: WgpuConfiguration = options.wgpu_options;
        let instance = pollster::block_on(config.wgpu_setup.new_instance());
        // SAFETY: the caller keeps the target alive for as long as the host, which owns the surface.
        let surface = unsafe { instance.create_surface_unsafe(target)? };
        let render_state = pollster::block_on(RenderState::create(&config, &instance, Some(&surface), RendererOptions::default()))?;
        let (width, height) = (size.width_px.max(1), size.height_px.max(1));
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: render_state.target_format,
            present_mode: wgpu::PresentMode::AutoVsync,
            view_formats: vec![render_state.target_format],
            desired_maximum_frame_latency: 2,
            ..surface.get_default_config(&render_state.adapter, width, height).ok_or(HostError::Unsupported)?
        };
        surface.configure(&render_state.device, &surface_config);

        let ctx = egui::Context::default();
        let next_frame = Arc::new(Mutex::new(Instant::now()));
        let wake = next_frame.clone();
        ctx.set_request_repaint_callback(move |info| {
            let at = Instant::now() + info.delay;
            let mut next = wake.lock().unwrap_or_else(PoisonError::into_inner);
            *next = (*next).min(at);
        });
        let app = create_app(&ctx, Some(render_state.clone()));
        Ok(Self {
            ctx,
            app,
            frame: eframe::Frame::_new_kittest(),
            render_state,
            surface,
            surface_config,
            size,
            started: Instant::now(),
            events: Vec::new(),
            pointer_touch: None,
            next_frame,
        })
    }

    pub(crate) fn resize(&mut self, size: ViewSize) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.surface_config.width = size.width_px.max(1);
        self.surface_config.height = size.height_px.max(1);
        self.surface.configure(&self.render_state.device, &self.surface_config);
        self.wake();
    }

    /// Touches become egui touch events (pinch and two-finger pans read these), and the first
    /// finger down also drives the pointer, so taps and drags work on every widget.
    pub(crate) fn touch(&mut self, t: Touch) {
        self.events.push(Event::Touch { device_id: TouchDeviceId(0), id: TouchId(t.id), phase: t.phase, pos: t.pos, force: Some(t.force) });
        let primary = match self.pointer_touch {
            None if t.phase == TouchPhase::Start => {
                self.pointer_touch = Some(t.id);
                true
            }
            Some(id) => id == t.id,
            None => false,
        };
        if primary {
            self.events.extend(pointer_events(t));
            if matches!(t.phase, TouchPhase::End | TouchPhase::Cancel) {
                self.pointer_touch = None;
            }
        }
        self.wake();
    }

    /// Run one frame if one is due: input arrived, or egui asked for it. Returns whether it drew.
    pub(crate) fn frame(&mut self) -> bool {
        let now = Instant::now();
        if self.events.is_empty() && now < *self.next_frame.lock().unwrap_or_else(PoisonError::into_inner) {
            return false;
        }
        // Until egui says otherwise, no further frame is needed.
        *self.next_frame.lock().unwrap_or_else(PoisonError::into_inner) = now + Duration::from_secs(3600);

        let mut raw = self.raw_input();
        self.app.raw_input_hook(&self.ctx, &mut raw);
        let (app, frame) = (&mut self.app, &mut self.frame);
        let output = self.ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx(), frame);
            app.ui(ui, frame);
        });
        let ppp = output.pixels_per_point;
        let primitives = self.ctx.tessellate(output.shapes, ppp);
        self.paint(&primitives, output.textures_delta, ppp);
        true
    }

    fn raw_input(&mut self) -> RawInput {
        let mut raw = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, self.size.points())),
            time: Some(self.started.elapsed().as_secs_f64()),
            events: std::mem::take(&mut self.events),
            focused: true,
            ..RawInput::default()
        };
        if let Some(viewport) = raw.viewports.get_mut(&ViewportId::ROOT) {
            viewport.native_pixels_per_point = Some(self.size.scale);
            viewport.inner_rect = raw.screen_rect;
            viewport.focused = Some(true);
        }
        raw
    }

    fn paint(&mut self, primitives: &[egui::ClippedPrimitive], mut textures: egui::TexturesDelta, ppp: f32) {
        let rs = &self.render_state;
        let screen = ScreenDescriptor { size_in_pixels: [self.surface_config.width, self.surface_config.height], pixels_per_point: ppp };
        let mut encoder = rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("photocraft-mobile") });
        let user_buffers = {
            let mut renderer = rs.renderer.write();
            for (id, deltas) in textures.set.drain() {
                for delta in &deltas {
                    renderer.update_texture(&rs.device, &rs.queue, id, delta);
                }
            }
            renderer.update_buffers(&rs.device, &rs.queue, &mut encoder, primitives, &screen)
        };
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                log::warn!("photocraft-mobile: no surface texture ({other:?}); reconfiguring");
                self.surface.configure(&rs.device, &self.surface_config);
                rs.queue.submit(user_buffers);
                self.wake();
                return;
            }
        };
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        {
            let renderer = rs.renderer.read();
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                        depth_slice: None,
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            renderer.render(&mut pass, primitives, &screen);
        }
        rs.queue.submit(user_buffers.into_iter().chain(std::iter::once(encoder.finish())));
        rs.queue.present(texture);
        let mut renderer = rs.renderer.write();
        for id in &textures.free {
            renderer.free_texture(id);
        }
    }

    fn wake(&self) {
        *self.next_frame.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }
}

/// The pointer events a primary touch stands for, as egui-winit makes them.
fn pointer_events(t: Touch) -> Vec<Event> {
    let button = |pressed| Event::PointerButton { pos: t.pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    match t.phase {
        TouchPhase::Start => vec![Event::PointerMoved(t.pos), button(true)],
        TouchPhase::Move => vec![Event::PointerMoved(t.pos)],
        TouchPhase::End => vec![button(false), Event::PointerGone],
        TouchPhase::Cancel => vec![Event::PointerGone],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(phase: TouchPhase) -> Touch {
        Touch { id: 7, phase, pos: Pos2::new(10.0, 20.0), force: 0.0 }
    }

    /// A tap is a press and a release at the same place, then the pointer leaves: a finger
    /// doesn't hover, so nothing stays highlighted after it lifts.
    #[test]
    fn a_tap_presses_releases_and_leaves() {
        let events: Vec<Event> = [TouchPhase::Start, TouchPhase::End].into_iter().flat_map(|p| pointer_events(touch(p))).collect();
        assert!(matches!(
            events.as_slice(),
            [Event::PointerMoved(_), Event::PointerButton { pressed: true, .. }, Event::PointerButton { pressed: false, .. }, Event::PointerGone]
        ));
    }

    /// A cancelled touch (the system took the gesture) must not click.
    #[test]
    fn a_cancelled_touch_never_releases_a_button() {
        assert!(pointer_events(touch(TouchPhase::Cancel)).iter().all(|e| !matches!(e, Event::PointerButton { .. })));
    }

    #[test]
    fn a_view_size_is_measured_in_points() {
        let size = ViewSize { width_px: 2064, height_px: 2752, scale: 2.0 };
        assert_eq!(size.points(), egui::vec2(1032.0, 1376.0));
    }
}
