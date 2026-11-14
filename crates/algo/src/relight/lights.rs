//! Relight light rig: the shading that replaces the recovered shading, for either the legacy
//! single directional lamp or up to [`RELIGHT_MAX_LIGHTS`] point / directional lights.

use photocraft_geom::Rect;

use super::{Params, light_color, light_dir, norm3};
use crate::{RELIGHT_MAX_LIGHTS, RelightLight, RelightLightKind};

/// One resolved light in pixel space.
struct Lamp {
    /// Point lights: position in pixels (z = height above the image); `None` for directional.
    pos: Option<[f32; 3]>,
    /// Directional: unit vector towards the light.
    dir: [f32; 3],
    /// Linear colour pre-multiplied by intensity / 100.
    col: [f32; 3],
    /// Wrap amount, 0…1.
    wrap: f32,
    /// Point falloff reach in pixels.
    reach: f32,
}

/// All lights of one `relight_rect` call, built once.
pub(super) struct Rig {
    ambient: f32,
    lamps: Vec<Lamp>,
    /// Legacy warmth tint applied after shading (`[1; 3]` for multi-light rigs, which is exact).
    tint: [f32; 3],
}

fn fin(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl Rig {
    pub(super) fn new(p: &Params, bounds: Rect) -> Rig {
        let ambient = fin(p.ambient, 0.0).clamp(0.0, 100.0) / 100.0;
        if p.lights.is_empty() {
            let inten = p.intensity.clamp(0.0, 100.0) / 100.0;
            let lamp = Lamp { pos: None, dir: light_dir(p.angle, p.elevation), col: [inten; 3], wrap: 0.0, reach: 1.0 };
            return Rig { ambient, lamps: vec![lamp], tint: light_color(p.warmth) };
        }
        let width = bounds.width() as f32;
        let height = bounds.height() as f32;
        let side = width.min(height).max(1.0);
        let lamps = p.lights.iter().take(RELIGHT_MAX_LIGHTS).filter_map(|l| resolve(l, bounds, width, height, side)).collect();
        Rig { ambient, lamps, tint: [1.0; 3] }
    }

    /// Ambient plus every light's contribution for a pixel with normal `nrm` at (`x`, `y`).
    fn shade(&self, nrm: [f32; 3], x: i32, y: i32) -> [f32; 3] {
        let mut acc = [self.ambient; 3];
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        for lamp in &self.lamps {
            let (l, att) = match lamp.pos {
                Some(pos) => {
                    let v = [pos[0] - px, pos[1] - py, pos[2]];
                    let d2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
                    let r = d2 / (lamp.reach * lamp.reach);
                    (norm3(v), 1.0 / (1.0 + r))
                }
                None => (lamp.dir, 1.0),
            };
            let dot = nrm[0] * l[0] + nrm[1] * l[1] + nrm[2] * l[2];
            let ndl = ((dot + lamp.wrap) / (1.0 + lamp.wrap)).max(0.0);
            for (a, c) in acc.iter_mut().zip(lamp.col) {
                *a += c * ndl * att;
            }
        }
        acc
    }

    /// Relit colour: `(c / s0) * shade * tint` on the colour channels, alpha untouched.
    pub(super) fn relit(&self, c: [f32; 4], s0: f32, nrm: [f32; 3], x: i32, y: i32) -> [f32; 4] {
        let shade = self.shade(nrm, x, y);
        let mut out = c;
        for (k, (sh, t)) in shade.iter().zip(self.tint).enumerate() {
            let (Some(ch), Some(slot)) = (c.get(k).copied(), out.get_mut(k)) else { continue };
            *slot = (ch / s0) * *sh * t;
        }
        out
    }
}

/// Resolve one light; `None` when it is switched off or holds a non-finite field.
fn resolve(l: &RelightLight, bounds: Rect, width: f32, height: f32, side: f32) -> Option<Lamp> {
    let nums = [l.x, l.y, l.height, l.angle, l.elevation, l.intensity, l.diffusion];
    if !nums.iter().chain(l.color.iter()).all(|v| v.is_finite()) {
        return None;
    }
    let inten = l.intensity.clamp(0.0, 100.0) / 100.0;
    if inten <= 0.0 {
        return None;
    }
    let col = [l.color[0].clamp(0.0, 1.0) * inten, l.color[1].clamp(0.0, 1.0) * inten, l.color[2].clamp(0.0, 1.0) * inten];
    let wrap = l.diffusion.clamp(0.0, 100.0) / 100.0;
    let (pos, dir) = match l.kind {
        RelightLightKind::Point => {
            let x = bounds.x0 as f32 + l.x.clamp(-0.5, 1.5) * width;
            let y = bounds.y0 as f32 + l.y.clamp(-0.5, 1.5) * height;
            let z = l.height.clamp(0.01, 2.0) * side;
            (Some([x, y, z]), [0.0, 0.0, 1.0])
        }
        RelightLightKind::Directional => (None, light_dir(l.angle, l.elevation)),
    };
    let reach = (side * (0.6 + 0.8 * wrap)).max(1.0);
    Some(Lamp { pos, dir, col, wrap, reach })
}
