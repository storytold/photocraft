//! Asynchronous read-back of an external mask's coverage tiles (the GPU-commit prototype of
//! #17133): a stroke's coverage lives in the mask textures [`Compositor::stamp_dabs`] draws into,
//! and the CPU needs it as 64×64 tiles of `f32` to composite the stroke's pixels.
//!
//! [`MaskReadback::request`] records one copy per tile into a staging buffer (rows padded to the
//! 256-byte copy alignment: an R32F row of 64 is exactly that, an R16F row takes 128 of 256
//! bytes) and starts mapping it; [`MaskReadback::poll`] returns the tiles of the buffers that
//! finished, in the order they were requested, converting R16F to `f32`. Buffers are pooled and
//! at most [`MAX_IN_FLIGHT`] are out at once, so a stroke's read-back memory stays bounded. The
//! handle is shared (`Clone`): one thread may request while another polls.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use super::{Compositor, LayerId};

/// Side of a tile, in pixels (the stroke buffer's `COV_TILE`).
pub const TILE: i32 = 64;
/// Bytes one tile takes in a staging buffer: 64 rows of 256.
const TILE_BYTES: u64 = 64 * 256;
/// Tiles one staging buffer holds (4 MiB).
pub const MAX_TILES_PER_BUFFER: usize = 256;
/// Staging buffers in flight at most.
pub const MAX_IN_FLIGHT: usize = 8;

const PENDING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

struct Flight {
    buffer: wgpu::Buffer,
    keys: Vec<(i32, i32)>,
    tag: u64,
    half: bool,
    state: Arc<AtomicU8>,
}

#[derive(Default)]
struct State {
    pool: Vec<wgpu::Buffer>,
    flights: VecDeque<Flight>,
    /// Tiles no texture holds (nothing was stamped there): zeros, no copy needed.
    zeros: Vec<((i32, i32), u64)>,
    bytes: u64,
}

/// A shared queue of coverage-tile read-backs.
#[derive(Clone, Default)]
pub struct MaskReadback {
    inner: Arc<Mutex<State>>,
}

impl MaskReadback {
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Bytes copied back so far (staging bytes, padding included).
    pub fn bytes_requested(&self) -> u64 {
        self.state().bytes
    }

    /// Read-backs requested and not yet returned by [`poll`](Self::poll).
    pub fn in_flight(&self) -> usize {
        let s = self.state();
        s.flights.len() + usize::from(!s.zeros.is_empty())
    }

    /// Ask for the coverage tiles `keys` (`(x / 64, y / 64)`) of `layer`'s external mask as they
    /// are after the stamps submitted so far (the copies go on the queue after them, so they see
    /// every stamp submitted before this call and none submitted after it). `tag` comes back
    /// with the tiles from [`poll`](Self::poll): the caller's note of when it asked (a stroke's
    /// push epoch). Takes the leading keys that fit one staging buffer
    /// while fewer than [`MAX_IN_FLIGHT`] are out, and returns how many that is (the caller asks
    /// for the rest later). A tile with no texture behind it reads as zeros.
    pub fn request(&self, comp: &Compositor, device: &wgpu::Device, queue: &wgpu::Queue, layer: LayerId, keys: &[(i32, i32)], tag: u64) -> usize {
        let mut state = self.state();
        if state.flights.len() >= MAX_IN_FLIGHT {
            return 0;
        }
        let take = keys.len().min(MAX_TILES_PER_BUFFER);
        let cells = comp.external_mask_cells(layer);
        let mut copies = Vec::new();
        for &key in keys.iter().take(take) {
            let (x, y) = (key.0 * TILE, key.1 * TILE);
            match cells.iter().find(|(r, _)| r.x0 <= x && x + TILE <= r.x1 && r.y0 <= y && y + TILE <= r.y1) {
                Some((r, t)) => copies.push((key, *t, (x - r.x0) as u32, (y - r.y0) as u32)),
                None => state.zeros.push((key, tag)),
            }
        }
        let Some(half) = copies.first().map(|(_, t, _, _)| t.format() == wgpu::TextureFormat::R16Float) else { return take };
        let size = copies.len() as u64 * TILE_BYTES;
        let buffer = match state.pool.iter().position(|b| b.size() >= size) {
            Some(i) => state.pool.swap_remove(i),
            None => device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pc_mask_readback"),
                size: size.max(TILE_BYTES * 16),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pc_mask_readback") });
        for (i, (_, texture, x, y)) in copies.iter().enumerate() {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x: *x, y: *y, z: 0 }, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout { offset: i as u64 * TILE_BYTES, bytes_per_row: Some(256), rows_per_image: Some(TILE as u32) },
                },
                wgpu::Extent3d { width: TILE as u32, height: TILE as u32, depth_or_array_layers: 1 },
            );
        }
        queue.submit([encoder.finish()]);
        let state_flag = Arc::new(AtomicU8::new(PENDING));
        let flag = state_flag.clone();
        buffer.slice(0..size).map_async(wgpu::MapMode::Read, move |r| flag.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release));
        state.bytes += size;
        state.flights.push_back(Flight { buffer, keys: copies.iter().map(|c| c.0).collect(), tag, half, state: state_flag });
        take
    }

    /// The tiles whose read-back finished, oldest request first, as `COV_TILE`² values with the
    /// tag of their request. With
    /// `block`, waits for everything requested. A read-back the device failed is dropped (its
    /// tiles never arrive).
    pub fn poll(&self, device: &wgpu::Device, block: bool) -> Vec<((i32, i32), Vec<f32>, u64)> {
        let _ = device.poll(if block { wgpu::PollType::Wait { submission_index: None, timeout: None } } else { wgpu::PollType::Poll });
        let mut state = self.state();
        let mut out: Vec<((i32, i32), Vec<f32>, u64)> =
            std::mem::take(&mut state.zeros).into_iter().map(|(k, tag)| (k, vec![0.0; (TILE * TILE) as usize], tag)).collect();
        while let Some(front) = state.flights.front() {
            let flag = front.state.load(Ordering::Acquire);
            if flag == PENDING {
                break;
            }
            let Some(flight) = state.flights.pop_front() else { break };
            if flag == MAPPED {
                let size = flight.keys.len() as u64 * TILE_BYTES;
                if let Ok(view) = flight.buffer.slice(0..size).get_mapped_range() {
                    for (i, key) in flight.keys.iter().enumerate() {
                        let tile = &view[i * TILE_BYTES as usize..(i + 1) * TILE_BYTES as usize];
                        let mut values = Vec::with_capacity((TILE * TILE) as usize);
                        for row in tile.as_chunks::<256>().0 {
                            if flight.half {
                                values.extend(row.as_chunks::<2>().0.iter().take(TILE as usize).map(|b| super::f16_to_f32(u16::from_le_bytes(*b))));
                            } else {
                                values.extend(row.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)));
                            }
                        }
                        out.push((*key, values, flight.tag));
                    }
                }
                flight.buffer.unmap();
            }
            state.pool.push(flight.buffer);
        }
        out
    }
}
