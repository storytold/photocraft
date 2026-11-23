//! Image › Analysis › Find Sun: where the sun is in a 2:1 equirectangular (latitude/longitude)
//! sky, and the colour and strength a directional light needs to match it.
//!
//! **Sphere.** The image is a full sphere seen from its centre: longitude runs from −180° at the
//! left edge to +180° at the right, latitude from +90° at the top row to −90° at the bottom. The
//! direction to longitude `lon`, latitude `lat` is `(cos lat · sin lon, sin lat, cos lat · cos lon)`:
//! +Y is up, the image centre faces +Z and a quarter to its right faces +X. [`Frame`] converts
//! that into other axis conventions.
//!
//! **Finding the sun** ([`find`]): the brightest 3×3 neighbourhood is the seed; a flood fill
//! (wrapping across the left/right seam) grows the bright region around it, and the sun is that
//! region's brightness-weighted centroid on the sphere. The sun counts as clear when the region
//! is small and very bright (above white in a 32-bit image, clipped at white otherwise) and not
//! below the horizon; an overcast sky only has a brightest area.
//!
//! **Light** ([`measure`]): a directional light lights a surface square-on with
//! `colour × intensity`, and a sky shown at exposure 1 adds its radiance with the same scale, so
//! a sun that delivers irradiance `E` needs `colour × intensity = E / π`.
//! - A 32-bit (linear) image whose sun peaks at 1.5 or more is *measured*: `E` is the sun's
//!   pixels minus the median of the sky in a ring just outside it, summed over the solid angle
//!   each pixel covers. The cutoff sits well above white because a clipped sun in a 32-bit file
//!   (one converted from 8 bits with some exposure, or saved after an exposure change) often
//!   lands a little above 1, and measuring it would give a light far too weak; a real sun in the
//!   reduced sample is hundreds or thousands.
//! - An 8/16-bit image, or a 32-bit one whose sun is clipped, has lost the sun's real
//!   brightness, so the light is *estimated*: the sky's light on flat ground `D` is measured
//!   from the image, and a clear-sky model where the diffuse sky light on a horizontal surface is
//!   about 0.12 of the sun's direct normal irradiance (the ASHRAE clear-sky model's diffuse
//!   factor) gives the sun's strength. The colour follows the sun's height (warmer near the
//!   horizon). 8/16-bit values are decoded as sRGB first.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use serde::{Deserialize, Serialize};

/// A sky image: straight-alpha RGBA, row-major, `width × height`.
pub struct Sky<'a> {
    pub px: &'a [[f32; 4]],
    pub width: usize,
    pub height: usize,
    /// Linear light (32-bit documents). Otherwise the values are sRGB-encoded.
    pub linear: bool,
}

impl Sky<'_> {
    fn valid(&self) -> bool {
        self.width >= 2 && self.height >= 2 && self.px.len() == self.width.saturating_mul(self.height)
    }
}

/// What [`find`] found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Found {
    /// Unit direction to the sun ([module docs](self) convention).
    pub direction: [f64; 3],
    /// A small, very bright sun above the horizon; false for an overcast sky's brightest area.
    pub clear: bool,
}

/// The light that matches the sun in the image ([`measure`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Light {
    /// Measured from the pixels (true) or estimated from the sky (false).
    pub measured: bool,
    /// A 32-bit image whose sun is clipped (it reaches white but no further), so it could only be
    /// estimated. False when nothing near the sun is that bright, as on a spot the user picked.
    pub clipped: bool,
    /// Linear RGB, largest channel 1.
    pub colour: [f64; 3],
    /// Light intensity at sky exposure 1: `max(E) / π` (0 when it could only be estimated and the
    /// sun is below the horizon or the sky gives no light).
    pub intensity: f64,
    /// Correlated colour temperature of `colour` in kelvin, when it is near the Planckian locus.
    pub kelvin: Option<f64>,
    /// Sun elevation in degrees.
    pub elevation: f64,
}

/// Axis conventions for reporting a direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Frame {
    /// +Y up, left-handed: image centre +Z, right of it +X.
    YUpLeftHanded,
    /// +Y up, right-handed: image centre −Z, right of it +X.
    YUpRightHanded,
    /// +Z up, right-handed: image centre +Y, right of it +X.
    ZUpRightHanded,
}

impl Frame {
    pub const ALL: [Frame; 3] = [Frame::YUpLeftHanded, Frame::YUpRightHanded, Frame::ZUpRightHanded];

    /// `d` (module convention) in this frame.
    pub fn from_sphere(self, d: [f64; 3]) -> [f64; 3] {
        match self {
            Frame::YUpLeftHanded => d,
            Frame::YUpRightHanded => [d[0], d[1], -d[2]],
            Frame::ZUpRightHanded => [d[0], d[2], d[1]],
        }
    }
}

/// Unit direction to image position `u, v` (0..1 across and down).
pub fn direction(u: f64, v: f64) -> [f64; 3] {
    let lon = (u - 0.5) * TAU;
    let lat = FRAC_PI_2 - v * PI;
    [lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos()]
}

/// Image position `[u, v]` (0..1) of direction `d`.
pub fn position(d: [f64; 3]) -> [f64; 2] {
    let [x, y, z] = normalize(d);
    let lon = x.atan2(z);
    let lat = y.clamp(-1.0, 1.0).asin();
    [(lon / TAU + 0.5).rem_euclid(1.0), (FRAC_PI_2 - lat) / PI]
}

/// Azimuth (degrees, 0 at the image centre, positive to the right, in (−180, 180]) and elevation
/// (degrees) of direction `d`.
pub fn azimuth_elevation(d: [f64; 3]) -> [f64; 2] {
    let [x, y, z] = normalize(d);
    let mut az = x.atan2(z).to_degrees();
    if az <= -180.0 {
        az += 360.0;
    }
    [az, y.clamp(-1.0, 1.0).asin().to_degrees()]
}

/// `d` turned about the vertical axis so that the image centre faces where `heading` degrees to
/// its right faced before (a sky shown turned by `heading`).
pub fn turn(d: [f64; 3], heading: f64) -> [f64; 3] {
    let (s, c) = heading.to_radians().sin_cos();
    [d[0] * c + d[2] * s, d[1], -d[0] * s + d[2] * c]
}

/// Rotation of a directional light that shines away from the sun `d` in a +Y-up, left-handed
/// frame whose lights shine along their +Z: `(quaternion [x, y, z, w], Euler [x, y, z] in
/// degrees)`, with the Euler angles applied Z, then X, then Y, and no roll.
pub fn light_rotation(d: [f64; 3]) -> ([f64; 4], [f64; 3]) {
    let [x, y, z] = normalize(d);
    let f = [-x, -y, -z];
    let pitch = (-f[1]).clamp(-1.0, 1.0).asin();
    let yaw = if f[0].abs() < 1e-12 && f[2].abs() < 1e-12 { 0.0 } else { f[0].atan2(f[2]) };
    let (sx, cx) = (pitch / 2.0).sin_cos();
    let (sy, cy) = (yaw / 2.0).sin_cos();
    ([cy * sx, cx * sy, -sy * sx, cx * cy], [pitch.to_degrees(), yaw.to_degrees(), 0.0])
}

fn normalize(d: [f64; 3]) -> [f64; 3] {
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if l > 0.0 && l.is_finite() { [d[0] / l, d[1] / l, d[2] / l] } else { [0.0, 0.0, 1.0] }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn luma(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Linear `c` (0..1) encoded as sRGB.
pub fn linear_to_srgb(c: f64) -> f64 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// sRGB decoding sampled at 4096 steps, read with linear interpolation: [`measure`] decodes every
/// pixel of an 8/16-bit sky, and a power per channel made dragging the sun in the dialog slow.
struct Decode(Vec<f64>);

impl Decode {
    const STEPS: usize = 4096;

    fn new(linear: bool) -> Self {
        Decode(if linear { Vec::new() } else { (0..=Self::STEPS).map(|i| srgb_to_linear(i as f64 / Self::STEPS as f64)).collect() })
    }

    fn channel(&self, c: f64) -> f64 {
        if self.0.is_empty() {
            return c;
        }
        let t = c.clamp(0.0, 1.0) * Self::STEPS as f64;
        let i = (t as usize).min(Self::STEPS - 1);
        let f = t - i as f64;
        self.0[i] + (self.0[i + 1] - self.0[i]) * f
    }

    /// Linear RGB of pixel `p` (`None` when mostly transparent).
    fn rgb(&self, p: [f32; 4]) -> Option<[f64; 3]> {
        if p[3] < 0.5 {
            return None;
        }
        Some([p[0], p[1], p[2]].map(|c| self.channel(if c.is_finite() { f64::from(c) } else { 0.0 })))
    }
}

/// Pixel-centre directions of a `w × h` sky as sines and cosines per column and row, so a pass over
/// every pixel does no trigonometry.
struct Grid {
    lon: Vec<(f64, f64)>,
    lat: Vec<(f64, f64)>,
}

impl Grid {
    fn new(w: usize, h: usize) -> Self {
        Grid {
            lon: (0..w).map(|x| (((x as f64 + 0.5) / w as f64 - 0.5) * TAU).sin_cos()).collect(),
            lat: (0..h).map(|y| (FRAC_PI_2 - (y as f64 + 0.5) / h as f64 * PI).sin_cos()).collect(),
        }
    }

    fn direction(&self, x: usize, y: usize) -> [f64; 3] {
        let ((sin_lon, cos_lon), (sin_lat, cos_lat)) = (self.lon[x], self.lat[y]);
        [cos_lat * sin_lon, sin_lat, cos_lat * cos_lon]
    }

    /// Rows that can hold a direction within `degrees` of `d`: those whose latitude is that close.
    fn rows_near(&self, d: [f64; 3], degrees: f64) -> std::ops::Range<usize> {
        let h = self.lat.len();
        let lat = d[1].clamp(-1.0, 1.0).asin();
        let reach = degrees.to_radians();
        let row = |lat: f64| ((FRAC_PI_2 - lat) / PI * h as f64).clamp(0.0, h as f64);
        (row(lat + reach).floor() as usize)..(row(lat - reach).ceil() as usize).min(h)
    }

    /// Columns that can hold a direction within `degrees` of `d`, wrapping across the left/right
    /// seam: every column when that cap reaches a pole.
    fn columns_near(&self, d: [f64; 3], degrees: f64) -> Vec<usize> {
        let w = self.lon.len();
        let (lat, reach) = (d[1].clamp(-1.0, 1.0).asin(), degrees.to_radians());
        if lat.abs() + reach >= FRAC_PI_2 {
            return (0..w).collect();
        }
        // Half the cap's width in longitude, and a column's width of margin each side.
        let half = (reach.sin() / lat.cos()).clamp(-1.0, 1.0).asin();
        let span = ((half / TAU * w as f64).ceil() as usize + 1).min(w / 2);
        let centre = ((d[0].atan2(d[2]) / TAU + 0.5) * w as f64).floor() as isize;
        let w = w as isize;
        (-(span as isize)..=span as isize).map(|i| (centre + i).rem_euclid(w) as usize).collect()
    }
}

/// The sun in `sky`, or `None` when there is no light at all (or the image is unusable).
pub fn find(sky: &Sky<'_>) -> Option<Found> {
    if !sky.valid() {
        return None;
    }
    let (w, h) = (sky.width, sky.height);
    let n = w * h;
    // Brightness as stored (encoded or linear): the seed and the clip test use it.
    let lum: Vec<f64> =
        sky.px.iter().map(|p| if p[3] < 0.5 { 0.0 } else { luma([p[0], p[1], p[2]].map(|c| if c.is_finite() { f64::from(c) } else { 0.0 })) }).collect();
    let at = |x: usize, y: usize| lum[y * w + x];
    let mut best = f64::NEG_INFINITY;
    let mut seed = None;
    for y in 1..h - 1 {
        for x in 0..w {
            let (l, r) = ((x + w - 1) % w, (x + 1) % w);
            let s: f64 = [y - 1, y, y + 1].iter().map(|&yy| at(l, yy) + at(x, yy) + at(r, yy)).sum();
            if s > best {
                best = s;
                seed = Some(y * w + x);
            }
        }
    }
    // The winning window's centre can be dark (a lone bright pixel wins every window it is in, and
    // the first of those is centred beside it), so the seed is the window's brightest pixel.
    let centre = seed?;
    let (cx, cy) = (centre % w, centre / w);
    let seed = [cy - 1, cy, cy + 1]
        .into_iter()
        .flat_map(|y| [(cx + w - 1) % w, cx, (cx + 1) % w].map(|x| y * w + x))
        .max_by(|&a, &b| lum[a].total_cmp(&lum[b]))
        .unwrap_or(centre);
    let peak = lum[seed];
    if peak <= 0.0 {
        return None;
    }
    let threshold = peak * if sky.linear && peak > 1.5 { 0.5 } else { 0.96 };
    let mut seen = vec![false; n];
    let mut stack = vec![seed];
    seen[seed] = true;
    let mut sum = [0.0; 3];
    let mut count = 0usize;
    while let Some(i) = stack.pop() {
        if count >= n / 20 {
            break;
        }
        let (x, y) = (i % w, i / w);
        let weight = lum[i] - threshold * 0.9;
        let d = direction((x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64);
        for (s, c) in sum.iter_mut().zip(d) {
            *s += c * weight;
        }
        count += 1;
        let mut push = |j: usize| {
            if !seen[j] && lum[j] >= threshold {
                seen[j] = true;
                stack.push(j);
            }
        };
        push(y * w + (x + 1) % w);
        push(y * w + (x + w - 1) % w);
        if y > 0 {
            push((y - 1) * w + x);
        }
        if y + 1 < h {
            push((y + 1) * w + x);
        }
    }
    let direction = normalize(sum);
    let bright = if sky.linear { peak > 2.0 } else { peak > 0.97 };
    Some(Found { direction, clear: count < n / 400 && bright && direction[1] > -0.05 })
}

/// The light matching a sun in direction `sun` (from [`find`], or picked by the user).
pub fn measure(sky: &Sky<'_>, sun: [f64; 3]) -> Option<Light> {
    if !sky.valid() {
        return None;
    }
    let sun = normalize(sun);
    let (w, h) = (sky.width, sky.height);
    let (d_lon, d_lat) = (TAU / w as f64, PI / h as f64);
    let (grid, decode) = (Grid::new(w, h), Decode::new(sky.linear));
    // The sun is looked for within 10° of `sun`; the sky in the ring from 10° to 16° is its
    // background.
    let (cos_sun, cos_ring) = (10f64.to_radians().cos(), 16f64.to_radians().cos());
    let mut ring: [Vec<f64>; 3] = Default::default();
    let mut peak = 0.0f64;
    let columns_ring = grid.columns_near(sun, 16.0);
    for y in grid.rows_near(sun, 16.0) {
        for &x in &columns_ring {
            let Some(c) = decode.rgb(sky.px[y * w + x]) else { continue };
            let k = dot(grid.direction(x, y), sun);
            if k < cos_ring {
                continue;
            }
            if k < cos_sun {
                for (r, v) in ring.iter_mut().zip(c) {
                    r.push(v);
                }
            } else {
                peak = peak.max(luma(c));
            }
        }
    }
    let background = ring.map(|mut r| {
        r.sort_by(f64::total_cmp);
        r.get(r.len() / 2).copied().unwrap_or(0.0)
    });
    let background_luma = luma(background);
    // The sun's energy above the background (E), and the sky's light on flat ground (D).
    // The sun's energy above the background (E): every pixel near it.
    let in_sun = |d: [f64; 3], c: [f64; 3]| dot(d, sun) >= cos_sun && luma(c) > 2.0 * background_luma + 1e-4;
    let mut e = [0.0f64; 3];
    let near_sun = grid.rows_near(sun, 10.0);
    let columns_sun = grid.columns_near(sun, 10.0);
    for y in near_sun.clone() {
        let solid_angle = grid.lat[y].1 * d_lon * d_lat;
        for &x in &columns_sun {
            let Some(c) = decode.rgb(sky.px[y * w + x]) else { continue };
            if in_sun(grid.direction(x, y), c) {
                for i in 0..3 {
                    e[i] += (c[i] - background[i]).max(0.0) * solid_angle;
                }
            }
        }
    }
    // See the module docs for why the cutoff is 1.5 and not 1.
    let measured = sky.linear && peak >= 1.5 && luma(e) > 0.0;
    // The sky's light on flat ground (D), which only an estimate needs. The sky's light changes
    // slowly across the sky, so every `step`-th pixel each way stands for its `step × step` block.
    let mut ground = [0.0f64; 3];
    if !measured {
        let step = if w >= 512 { 4 } else { 1 };
        for y in (0..h).step_by(step).filter(|&y| grid.lat[y].0 > 0.0) {
            let solid_angle = grid.lat[y].1 * d_lon * d_lat * (step * step) as f64;
            for x in (0..w).step_by(step) {
                let Some(c) = decode.rgb(sky.px[y * w + x]) else { continue };
                let d = grid.direction(x, y);
                if near_sun.contains(&y) && in_sun(d, c) {
                    continue;
                }
                for i in 0..3 {
                    ground[i] += c[i] * d[1] * solid_angle;
                }
            }
        }
    }
    let elevation = sun[1].clamp(-1.0, 1.0).asin().to_degrees();
    let (colour, intensity) = if measured {
        let m = e[0].max(e[1]).max(e[2]);
        (e.map(|c| c / m), m / PI)
    } else {
        // Sun colour temperature by height: about 2000 K at the horizon, 5800 K high up.
        let kelvin = 2000.0 + 3800.0 * (1.0 - (-elevation.max(0.0) / 9.0).exp());
        let colour = kelvin_to_linear(kelvin);
        // Clear-sky direct normal irradiance from the sky's diffuse light on the ground.
        let direct_normal = luma(ground) / 0.12;
        // A linear sky with negative values can make the ground light negative: no sunlight then.
        let intensity = if elevation <= 0.0 || direct_normal <= 0.0 { 0.0 } else { direct_normal / (PI * luma(colour)) };
        (colour, intensity)
    };
    Some(Light { measured, clipped: sky.linear && !measured && peak >= 0.9, colour, intensity, kelvin: linear_to_kelvin(colour), elevation })
}

/// Linear sRGB of a black body at `kelvin` (1667–25000 K), largest channel 1: the Planckian
/// locus in CIE xy (Kim et al., *Design of advanced color temperature control system for HDTV
/// applications*, 2002, cubic spline fit) to linear sRGB.
pub fn kelvin_to_linear(kelvin: f64) -> [f64; 3] {
    let [x, y] = planckian_xy(kelvin);
    let (cx, cz) = (x / y, (1.0 - x - y) / y);
    let rgb = [
        3.240_454_2 * cx - 1.537_138_5 - 0.498_531_4 * cz,
        -0.969_266_0 * cx + 1.876_010_8 + 0.041_556_0 * cz,
        0.055_643_4 * cx - 0.204_025_9 + 1.057_225_2 * cz,
    ]
    .map(|c| c.max(0.0));
    let m = rgb[0].max(rgb[1]).max(rgb[2]);
    if m > 0.0 { rgb.map(|c| c / m) } else { [1.0; 3] }
}

/// CIE xy of a black body at `kelvin`, clamped to 1667–25000 K (Kim et al. 2002, as above).
fn planckian_xy(kelvin: f64) -> [f64; 2] {
    let t = kelvin.clamp(1667.0, 25000.0);
    let k = 1e3 / t;
    let (k2, k3) = (k * k, k * k * k);
    let x = if t <= 4000.0 {
        -0.266_123_9 * k3 - 0.234_358_9 * k2 + 0.877_695_6 * k + 0.179_910
    } else {
        -3.025_846_9 * k3 + 2.107_037_9 * k2 + 0.222_634_7 * k + 0.240_390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.106_381_4 * x3 - 1.348_110_20 * x2 + 2.185_558_32 * x - 0.202_196_83
    } else if t <= 4000.0 {
        -0.954_947_6 * x3 - 1.374_185_93 * x2 + 2.091_370_15 * x - 0.167_488_67
    } else {
        3.081_758_0 * x3 - 5.873_386_70 * x2 + 3.751_129_97 * x - 0.370_014_83
    };
    [x, y]
}

/// CIE 1960 uv of CIE xy.
fn uv(x: f64, y: f64) -> [f64; 2] {
    let d = -2.0 * x + 12.0 * y + 3.0;
    [4.0 * x / d, 6.0 * y / d]
}

/// Correlated colour temperature of linear sRGB `c` (McCamy's approximation, 1992), when it lands
/// between 1667 and 25000 K (the range [`planckian_xy`] covers) and `c` is near the Planckian
/// locus: within 0.05 in CIE 1960 uv of the black body at that temperature, the limit beyond which
/// CIE 015 says a correlated colour temperature should not be quoted.
pub fn linear_to_kelvin(c: [f64; 3]) -> Option<f64> {
    let big_x = 0.412_456_4 * c[0] + 0.357_576_1 * c[1] + 0.180_437_5 * c[2];
    let big_y = 0.212_672_9 * c[0] + 0.715_152_2 * c[1] + 0.072_175_0 * c[2];
    let big_z = 0.019_333_9 * c[0] + 0.119_192_0 * c[1] + 0.950_304_1 * c[2];
    let sum = big_x + big_y + big_z;
    if sum <= 0.0 {
        return None;
    }
    let (x, y) = (big_x / sum, big_y / sum);
    let n = (x - 0.3320) / (0.1858 - y);
    let k = 449.0 * n.powi(3) + 3525.0 * n.powi(2) + 6823.3 * n + 5520.33;
    if !(1667.0..=25000.0).contains(&k) {
        return None;
    }
    let ([u, v], [pu, pv]) = (uv(x, y), {
        let [px, py] = planckian_xy(k);
        uv(px, py)
    });
    ((u - pu).hypot(v - pv) <= 0.05).then_some(k)
}

#[cfg(test)]
#[path = "sun_tests.rs"]
mod tests;
