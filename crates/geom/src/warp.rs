//! Warp geometry shared by Edit › Transform › Warp, smart-object warps and Warp Text.
//!
//! Two kinds of warp, both forward maps from a source box to document space:
//! - **Styles** ([`WarpStyle`], Photoshop's fifteen presets plus bend and horizontal/vertical
//!   distortion, the PSD `warp` descriptor's `warpStyle`/`warpValue`/`warpPerspective*`). Each
//!   is a single bicubic Bezier patch whose control grid follows the bend: measured from
//!   Photoshop's renders (see [`preset_grid`]), not taken from Adobe's code. Every style is the
//!   identity at bend 0 and continuous in the bend.
//! - **Custom meshes** ([`BezierMesh`]): a grid of bicubic Bezier patches. The default is one
//!   patch (a 4×4 control grid); split warps add patch rows/columns by exact de Casteljau
//!   subdivision, so splitting never changes the shape.
//! - **Cylinder** ([`WarpStyle::Cylinder`], Transform Warp only): seven `warpValues` that
//!   Photoshop turns into a 4 × 1 patch mesh wrapping the contents round a cylinder; see
//!   [`cylinder_mesh`].
//!
//! Pure geometry: resampling lives in `photocraft-algo::warp`.

use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

/// Warp style: none, a custom mesh, or one of Photoshop's presets (menu order).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, craft_ui_macros::PlainData)]
#[serde(rename_all = "camelCase")]
pub enum WarpStyle {
    #[default]
    None,
    Custom,
    Arc,
    ArcLower,
    ArcUpper,
    Arch,
    Bulge,
    ShellLower,
    ShellUpper,
    Flag,
    Wave,
    Fish,
    Rise,
    Fisheye,
    Inflate,
    Squeeze,
    Twist,
    /// Edit › Transform › Warp's Cylinder (`warpCylinder`): not one of the fifteen bend presets
    /// (Warp Text doesn't have it), it takes seven values instead of a bend; see
    /// [`Warp::cylinder`].
    Cylinder,
}

impl WarpStyle {
    /// The fifteen presets in Photoshop's menu order.
    pub const PRESETS: [WarpStyle; 15] = [
        WarpStyle::Arc,
        WarpStyle::ArcLower,
        WarpStyle::ArcUpper,
        WarpStyle::Arch,
        WarpStyle::Bulge,
        WarpStyle::ShellLower,
        WarpStyle::ShellUpper,
        WarpStyle::Flag,
        WarpStyle::Wave,
        WarpStyle::Fish,
        WarpStyle::Rise,
        WarpStyle::Fisheye,
        WarpStyle::Inflate,
        WarpStyle::Squeeze,
        WarpStyle::Twist,
    ];

    /// The styles every Warp menu lists: none, custom and the presets. [`WarpStyle::Cylinder`] is
    /// not one of them: Transform Warp's menu adds it after these (it has its own editor there),
    /// while Warp Text has no cylinder.
    pub fn all() -> impl Iterator<Item = WarpStyle> {
        [WarpStyle::None, WarpStyle::Custom].into_iter().chain(Self::PRESETS)
    }

    /// PSD `warpStyle` enum value (`warpArc`, `warpCustom`, `warpNone`…).
    pub fn psd_name(self) -> &'static str {
        match self {
            WarpStyle::None => "warpNone",
            WarpStyle::Custom => "warpCustom",
            WarpStyle::Arc => "warpArc",
            WarpStyle::ArcLower => "warpArcLower",
            WarpStyle::ArcUpper => "warpArcUpper",
            WarpStyle::Arch => "warpArch",
            WarpStyle::Bulge => "warpBulge",
            WarpStyle::ShellLower => "warpShellLower",
            WarpStyle::ShellUpper => "warpShellUpper",
            WarpStyle::Flag => "warpFlag",
            WarpStyle::Wave => "warpWave",
            WarpStyle::Fish => "warpFish",
            WarpStyle::Rise => "warpRise",
            WarpStyle::Fisheye => "warpFisheye",
            WarpStyle::Inflate => "warpInflate",
            WarpStyle::Squeeze => "warpSqueeze",
            WarpStyle::Twist => "warpTwist",
            WarpStyle::Cylinder => "warpCylinder",
        }
    }

    /// Short id used by commands (`arc`, `arcLower`, `custom`, `none`).
    pub fn id(self) -> &'static str {
        let p = self.psd_name().trim_start_matches("warp");
        // Lower-case first letter without allocating: every id is listed.
        match p {
            "None" => "none",
            "Custom" => "custom",
            "Arc" => "arc",
            "ArcLower" => "arcLower",
            "ArcUpper" => "arcUpper",
            "Arch" => "arch",
            "Bulge" => "bulge",
            "ShellLower" => "shellLower",
            "ShellUpper" => "shellUpper",
            "Flag" => "flag",
            "Wave" => "wave",
            "Fish" => "fish",
            "Rise" => "rise",
            "Fisheye" => "fisheye",
            "Inflate" => "inflate",
            "Squeeze" => "squeeze",
            "Cylinder" => "cylinder",
            _ => "twist",
        }
    }

    /// Menu label (`Arc Lower`).
    pub fn label(self) -> &'static str {
        match self {
            WarpStyle::None => "None",
            WarpStyle::Custom => "Custom",
            WarpStyle::Arc => "Arc",
            WarpStyle::ArcLower => "Arc Lower",
            WarpStyle::ArcUpper => "Arc Upper",
            WarpStyle::Arch => "Arch",
            WarpStyle::Bulge => "Bulge",
            WarpStyle::ShellLower => "Shell Lower",
            WarpStyle::ShellUpper => "Shell Upper",
            WarpStyle::Flag => "Flag",
            WarpStyle::Wave => "Wave",
            WarpStyle::Fish => "Fish",
            WarpStyle::Rise => "Rise",
            WarpStyle::Fisheye => "Fisheye",
            WarpStyle::Inflate => "Inflate",
            WarpStyle::Squeeze => "Squeeze",
            WarpStyle::Twist => "Twist",
            WarpStyle::Cylinder => "Cylinder",
        }
    }

    /// Parses a short id, a PSD name or a label (case-insensitive, spaces ignored).
    pub fn parse(s: &str) -> Option<WarpStyle> {
        let k: String = s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        Self::all().chain([WarpStyle::Cylinder]).find(|w| w.id().to_ascii_lowercase() == k || w.psd_name().to_ascii_lowercase() == k)
    }

    /// One of the fifteen bend presets ([`Self::PRESETS`]).
    pub fn is_preset(self) -> bool {
        Self::PRESETS.contains(&self)
    }
}

/// A preset style prepared for one box: its control grid and the box it bends.
#[derive(Clone, Copy, Debug, craft_ui_macros::PlainData)]
pub struct StyleWarp {
    /// The 4 × 4 Bezier control grid in box units (0..1, y down), row-major.
    grid: Grid,
    /// Box origin and size.
    o: (f64, f64),
    size: (f64, f64),
}

/// A single bicubic Bezier patch's control points, `[row][column]`, in box units.
type Grid = [[[f64; 2]; 4]; 4];

impl StyleWarp {
    /// Prepares `style` with bend / distortions in percent (-100..100) for `[x0, y0, x1, y1]`.
    /// `None` for non-presets, an all-zero warp or an empty box.
    pub fn new(style: WarpStyle, bend: f64, h_distort: f64, v_distort: f64, vertical: bool, bounds: [f64; 4]) -> Option<StyleWarp> {
        if !style.is_preset() {
            return None;
        }
        let [x0, y0, x1, y1] = bounds;
        let (hw, hh) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
        if !(hw > 0.0 && hh > 0.0) {
            return None;
        }
        let pct = |v: f64| if v.is_finite() { (v / 100.0).clamp(-1.0, 1.0) } else { 0.0 };
        let (b, hd, vd) = (pct(bend), pct(h_distort), pct(v_distort));
        if b == 0.0 && hd == 0.0 && vd == 0.0 {
            return None;
        }
        let grid = preset_grid(style, b, hd, vd, vertical, hh / hw);
        Some(StyleWarp { grid, o: (x0, y0), size: (2.0 * hw, 2.0 * hh) })
    }

    /// Longest segment (px) worth sending through [`StyleWarp::apply`] unsplit.
    pub fn max_segment(&self) -> f64 {
        (self.size.0.min(self.size.1) / 32.0).max(0.5)
    }

    /// Maps a point.
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let (u, v) = ((x - self.o.0) / self.size.0, (y - self.o.1) / self.size.1);
        let (bu, bv) = (bernstein(u), bernstein(v));
        let (mut px, mut py) = (0.0, 0.0);
        for (row, wv) in self.grid.iter().zip(bv) {
            for (p, wu) in row.iter().zip(bu) {
                px += wu * wv * p[0];
                py += wu * wv * p[1];
            }
        }
        (self.o.0 + px * self.size.0, self.o.1 + py * self.size.1)
    }

    /// The style as one Bezier patch over its box (every preset is one exactly).
    pub fn mesh(&self) -> BezierMesh {
        let points = self.grid.iter().flatten().map(|p| [self.o.0 + p[0] * self.size.0, self.o.1 + p[1] * self.size.1]).collect();
        BezierMesh { us: vec![0.0, 1.0], vs: vec![0.0, 1.0], points }
    }
}

/// The rest grid (no warp).
fn rest_grid() -> Grid {
    std::array::from_fn(|j| std::array::from_fn(|i| [i as f64 / 3.0, j as f64 / 3.0]))
}

/// A horizontal edge at height `y` (box units, 0..1 across) bent into a circular arc of `|b|·π`
/// that keeps its chord, bowing up when `up == (b > 0)`. `h` is the box's height over its width:
/// the arc is circular in pixels.
fn edge_arc(y: f64, b: f64, up: bool, h: f64) -> [[f64; 2]; 4] {
    if b.abs() < 1e-9 {
        return std::array::from_fn(|i| [i as f64 / 3.0, y]);
    }
    let th = b.abs() * PI;
    let k = 4.0 / 3.0 * (th / 4.0).tan() * 0.5 / (th / 2.0).sin();
    let s = if (b > 0.0) == up { -1.0 } else { 1.0 };
    let (c, n) = ((th / 2.0).cos(), (th / 2.0).sin());
    let dy = s * k * n / h;
    [[0.0, y], [k * c, y + dy], [1.0 - k * c, y + dy], [1.0, y]]
}

/// Arc: every row is a circular arc of `|b|·π` around one centre; the edge on the inside of the
/// bend keeps its chord, and the rows' radii grow by the box height towards the other edge.
/// Worked in units of the box width (`h` = height / width) so the arcs are circular in pixels.
fn arc_grid(b: f64, h: f64) -> Grid {
    if b.abs() < 1e-9 {
        return rest_grid();
    }
    let up = b > 0.0;
    let th = b.abs() * PI;
    let r = 0.5 / (th / 2.0).sin();
    let k = 4.0 / 3.0 * (th / 4.0).tan();
    let s = if up { -1.0 } else { 1.0 };
    let cy = if up { h + r * (th / 2.0).cos() } else { -r * (th / 2.0).cos() };
    std::array::from_fn(|j| {
        let v = j as f64 / 3.0;
        let big = r + h * if up { 1.0 - v } else { v };
        let at = |a: f64| [0.5 + big * a.sin(), cy + s * big * a.cos()];
        let tangent = |a: f64| [a.cos(), -s * a.sin()];
        let (p0, p3, t0, t1) = (at(-th / 2.0), at(th / 2.0), tangent(-th / 2.0), tangent(th / 2.0));
        let l = k * big;
        let pts = [p0, [p0[0] + l * t0[0], p0[1] + l * t0[1]], [p3[0] - l * t1[0], p3[1] - l * t1[1]], p3];
        pts.map(|p| [p[0], p[1] / h])
    })
}

/// Rows blended from `top` to `bottom`.
fn blend_rows(top: [[f64; 2]; 4], bottom: [[f64; 2]; 4]) -> Grid {
    std::array::from_fn(|j| {
        let t = j as f64 / 3.0;
        std::array::from_fn(|i| [top[i][0] + (bottom[i][0] - top[i][0]) * t, top[i][1] + (bottom[i][1] - top[i][1]) * t])
    })
}

/// Photoshop's warp presets as Bezier control grids, measured on Photoshop 27.11: every preset at
/// bend ±50 and ±100, Arc and Bulge with ±50 horizontal and vertical distortion, and the vertical
/// orientation, each fitted from scripted renders of a coordinate-coded grid image (every preset
/// is a single bicubic patch, within 0.5 px). These grids put Photoshop's pixels back within
/// 1 px (median) on all 70 cases, and on 32 more on 256 × 96 and 96 × 256 boxes, vertical, and
/// with both distortions: the arc family stays circular in pixels, the other offsets scale with
/// the box, and a vertical warp is the horizontal one with the axes (and distortions) swapped.
/// `b`, `hd`, `vd` are -1..1; `aspect` is the box's height over its width.
fn preset_grid(style: WarpStyle, b: f64, hd: f64, vd: f64, vertical: bool, aspect: f64) -> Grid {
    // Height over width of the box the bend runs along (the transposed box when vertical); in
    // that frame the two distortions trade places too.
    let h = if vertical { 1.0 / aspect } else { aspect };
    let (hd, vd) = if vertical { (vd, hd) } else { (hd, vd) };
    let mut g = rest_grid();
    match style {
        WarpStyle::Arc => g = arc_grid(b, h),
        WarpStyle::Arch => g = std::array::from_fn(|j| edge_arc(j as f64 / 3.0, b, true, h)),
        WarpStyle::Bulge => g = blend_rows(edge_arc(0.0, b, true, h), edge_arc(1.0, b, false, h)),
        WarpStyle::ArcLower => g = blend_rows(g[0], edge_arc(1.0, b, false, h)),
        WarpStyle::ArcUpper => g = blend_rows(edge_arc(0.0, b, true, h), g[3]),
        // The bulging edge and the outer points of the row next to it come from Arc (bent the
        // other way for Shell Lower); the rest stays put.
        WarpStyle::ShellLower => {
            let a = arc_grid(-b, h);
            g[3] = a[3];
            g[2][0] = a[2][0];
            g[2][3] = a[2][3];
        }
        WarpStyle::ShellUpper => {
            let a = arc_grid(b, h);
            g[0] = a[0];
            g[1][0] = a[1][0];
            g[1][3] = a[1][3];
        }
        WarpStyle::Flag => {
            for row in &mut g {
                row[1][1] -= 2.0 * b;
                row[2][1] += 2.0 * b;
            }
        }
        WarpStyle::Wave => {
            for row in &mut g[1..3] {
                row[1][1] += 4.0 / 3.0 * b;
                row[2][1] -= 4.0 / 3.0 * b;
            }
        }
        WarpStyle::Fish => {
            for (row, f) in g.iter_mut().zip([-2.0, -2.0 / 3.0, 2.0 / 3.0, 2.0]) {
                row[1][1] += f * b;
                row[2][1] -= f * b;
            }
        }
        WarpStyle::Rise => {
            for row in &mut g {
                row[0][1] += 2.0 * b;
                row[1][1] += 2.0 * b;
            }
        }
        WarpStyle::Fisheye => {
            for (j, i, dx, dy) in [(1, 1, -1.0, -1.0), (1, 2, 1.0, -1.0), (2, 1, -1.0, 1.0), (2, 2, 1.0, 1.0)] {
                g[j][i][0] += dx * 2.0 / 3.0 * b;
                g[j][i][1] += dy * 2.0 / 3.0 * b;
            }
        }
        // Edge midpoints move out (Inflate) or the sides in (Squeeze) by b/3, the inner points by b/9.
        WarpStyle::Inflate | WarpStyle::Squeeze => {
            let s = if style == WarpStyle::Inflate { 1.0 } else { -1.0 };
            for p in &mut g[0][1..3] {
                p[1] -= b / 3.0;
            }
            for p in &mut g[3][1..3] {
                p[1] += b / 3.0;
            }
            for row in &mut g[1..3] {
                row[0][0] -= s * b / 3.0;
                row[3][0] += s * b / 3.0;
            }
            for (j, i, dx, dy) in [(1, 1, -1.0, -1.0), (1, 2, 1.0, -1.0), (2, 1, -1.0, 1.0), (2, 2, 1.0, 1.0)] {
                g[j][i][0] += s * dx * b / 9.0;
                g[j][i][1] += dy * b / 9.0;
            }
        }
        // The inner points turn round the centre (the other way for a negative bend).
        WarpStyle::Twist => {
            let a = b.abs();
            if b >= 0.0 {
                g[1][1][0] += a;
                g[1][2][1] += a;
                g[2][1][1] -= a;
                g[2][2][0] -= a;
            } else {
                g[1][1][1] += a;
                g[1][2][0] -= a;
                g[2][1][0] += a;
                g[2][2][1] -= a;
            }
        }
        WarpStyle::None | WarpStyle::Custom | WarpStyle::Cylinder => {}
    }
    // Vertical distortion: the top and bottom rows scale about their end points' midpoint (by
    // 1 ∓ vd) and the middle rows follow by the blend of those two moves.
    if vd != 0.0 {
        let moved = |row: [[f64; 2]; 4], s: f64| -> [[f64; 2]; 4] {
            let m = [(row[0][0] + row[3][0]) / 2.0, (row[0][1] + row[3][1]) / 2.0];
            std::array::from_fn(|i| [m[0] + (row[i][0] - m[0]) * s - row[i][0], m[1] + (row[i][1] - m[1]) * s - row[i][1]])
        };
        let (d0, d3) = (moved(g[0], 1.0 - vd), moved(g[3], 1.0 + vd));
        for (j, row) in g.iter_mut().enumerate() {
            let t = j as f64 / 3.0;
            for (i, p) in row.iter_mut().enumerate() {
                p[0] += d0[i][0] + (d3[i][0] - d0[i][0]) * t;
                p[1] += d0[i][1] + (d3[i][1] - d0[i][1]) * t;
            }
        }
    }
    // Horizontal distortion: each column scales about its end points' midpoint by 1 + hd·(2i/3 − 1).
    if hd != 0.0 {
        for i in 0..4 {
            let s = 1.0 + hd * (2.0 * i as f64 / 3.0 - 1.0);
            let m = [(g[0][i][0] + g[3][i][0]) / 2.0, (g[0][i][1] + g[3][i][1]) / 2.0];
            for row in &mut g {
                row[i] = [m[0] + (row[i][0] - m[0]) * s, m[1] + (row[i][1] - m[1]) * s];
            }
        }
    }
    // Vertical orientation: the same grid with the axes swapped.
    if vertical {
        g = std::array::from_fn(|j| std::array::from_fn(|i| [g[i][j][1], g[i][j][0]]));
    }
    g
}

/// The one-patch pull's shares of a grab at `s` (0..1) for the four control lines of a patch:
/// the two lines bounding its third share it by where it is between them; an end line takes
/// [`ONE_PATCH_BORDER`] of its share, and the next line inward [`ONE_PATCH_SPILL`] of its
/// neighbour's.
fn one_patch_share(s: f64) -> [f64; 4] {
    let s = s.clamp(0.0, 1.0);
    let third = ((s * 3.0) as usize).min(2);
    let x = s * 3.0 - third as f64;
    match third {
        0 => [ONE_PATCH_BORDER * (1.0 - x), x, ONE_PATCH_SPILL * x, 0.0],
        1 => [0.0, 1.0 - x, x, 0.0],
        _ => [0.0, ONE_PATCH_SPILL * (1.0 - x), 1.0 - x, ONE_PATCH_BORDER * x],
    }
}

/// One-patch pull: the share a border line takes of a grab in its outer third (measured: 0.455).
const ONE_PATCH_BORDER: f64 = 0.455;
/// One-patch pull: the share of its neighbour's pull the next line inward takes (measured: 0.024).
const ONE_PATCH_SPILL: f64 = 0.024;

fn bernstein(t: f64) -> [f64; 4] {
    let s = 1.0 - t;
    [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t]
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// De Casteljau split of a cubic at `t`: 7 points (left 0..=3, right 3..=6).
fn split_cubic(p: [[f64; 2]; 4], t: f64) -> [[f64; 2]; 7] {
    let a = lerp(p[0], p[1], t);
    let b = lerp(p[1], p[2], t);
    let c = lerp(p[2], p[3], t);
    let d = lerp(a, b, t);
    let e = lerp(b, c, t);
    let f = lerp(d, e, t);
    [p[0], a, d, f, e, c, p[3]]
}

/// Photoshop's Cylinder warp as a Bezier mesh in the `warp` descriptor's value space (the space
/// of its `bounds`), from the seven `warpValues` `v` of a horizontal cylinder with no
/// perspective. Like a custom warp's mesh, its control-point box is what the placement quad
/// frames. `None` unless there are seven finite values with the right silhouette right of the
/// left one.
///
/// The model was measured against Photoshop 2026's renders of a UV grid (issue #4112: within
/// 0.3 px vertically and 1.2 px horizontally at the 99th percentile), not taken from Adobe's code:
/// - `v[0]`, `v[2]`: the left and right silhouettes. Half axis `a = (v[2] − v[0]) / 2`.
/// - `v[3]`, `v[1]`: the top and bottom edges at the silhouettes (the centre lines of the upper
///   and lower ellipses); `v[4]`, `v[5]`: those ellipses' depth in units of `a`.
/// - `v[6]` (0..1): how far the contents wrap round. The outer `uL` of the contents on each side
///   is on the back of the cylinder and collapses onto the silhouette: 1/6 at 0, 0.006 at 1.
///
/// Four patches across (`us = [0, uL, 0.5, 1 − uL, 1]`: back, front left, front right, back) and
/// one down; control rows are linear between the top and bottom edge.
pub fn cylinder_mesh(v: &[f64]) -> Option<BezierMesh> {
    let v: [f64; 7] = v.try_into().ok()?;
    if !v.iter().all(|x| x.is_finite()) || v[2] <= v[0] {
        return None;
    }
    let a = (v[2] - v[0]) / 2.0;
    let cx = (v[0] + v[2]) / 2.0;
    let [ul, kx, ky, my] = cylinder_shape(v[6]);
    let s = [-1.0, -1.0, -1.0, -1.0, -1.0, -kx, 0.0, kx, 1.0, 1.0, 1.0, 1.0, 1.0];
    let e = [0.0, 0.0, 0.0, 0.0, ky, my, 1.0, my, ky, 0.0, 0.0, 0.0, 0.0];
    let mut points = Vec::with_capacity(13 * 4);
    for r in 0..4 {
        let t = f64::from(r) / 3.0;
        for (sk, ek) in s.iter().zip(e) {
            let (top, bottom) = (v[3] + v[4] * a * ek, v[1] + v[5] * a * ek);
            points.push([cx + a * sk, top + (bottom - top) * t]);
        }
    }
    let mesh = BezierMesh { us: vec![0.0, ul, 0.5, 1.0 - ul, 1.0], vs: vec![0.0, 1.0], points };
    mesh.is_valid().then_some(mesh)
}

/// `[uL, kx, ky, my]` for a cylinder's wrap `g` (`warpValues[6]`, clamped to 0..1): the share of
/// the contents on the back on each side, then the front patches' inner x control point (in
/// units of the half axis) and their ellipse factors. Linear between the measured rows.
fn cylinder_shape(g: f64) -> [f64; 4] {
    let n = CYLINDER_SHAPE.len() - 1;
    let x = if g.is_finite() { g.clamp(0.0, 1.0) * n as f64 } else { 0.0 };
    let i = (x.floor() as usize).min(n - 1);
    let (a, b, t) = (CYLINDER_SHAPE[i], CYLINDER_SHAPE[i + 1], x - i as f64);
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

/// [`cylinder_shape`]'s rows for `g` = 0, 0.025 … 1, measured from Photoshop's renders (#4112):
/// no closed form was found.
const CYLINDER_SHAPE: [[f64; 4]; 41] = [
    [0.166169, 0.655404, 0.382374, 1.003104],
    [0.157304, 0.650228, 0.388326, 1.002685],
    [0.148438, 0.645374, 0.394211, 1.007918],
    [0.140060, 0.639652, 0.401415, 1.006938],
    [0.131885, 0.634064, 0.408465, 1.006069],
    [0.123875, 0.628845, 0.414586, 1.005743],
    [0.116162, 0.623565, 0.421033, 1.005233],
    [0.108674, 0.618563, 0.426905, 1.004990],
    [0.101198, 0.614439, 0.431943, 1.010787],
    [0.094322, 0.609611, 0.437749, 1.010560],
    [0.087757, 0.604929, 0.443406, 1.010361],
    [0.081514, 0.600390, 0.448907, 1.010190],
    [0.075599, 0.596012, 0.454288, 1.010019],
    [0.069994, 0.591836, 0.459420, 1.009888],
    [0.064709, 0.587844, 0.464361, 1.009768],
    [0.059740, 0.584026, 0.469109, 1.009669],
    [0.055072, 0.580427, 0.473609, 1.009588],
    [0.050712, 0.576990, 0.477904, 1.009528],
    [0.046635, 0.573760, 0.481968, 1.009477],
    [0.042824, 0.570758, 0.485764, 1.009440],
    [0.039295, 0.567910, 0.489377, 1.009411],
    [0.036023, 0.565229, 0.492800, 1.009382],
    [0.032681, 0.563478, 0.495519, 1.015285],
    [0.029877, 0.561152, 0.498513, 1.015270],
    [0.027282, 0.559012, 0.501313, 1.015245],
    [0.024894, 0.557004, 0.503913, 1.015241],
    [0.022693, 0.555163, 0.506331, 1.015225],
    [0.020668, 0.553463, 0.508575, 1.015213],
    [0.018816, 0.551877, 0.510651, 1.015217],
    [0.017106, 0.550434, 0.512556, 1.015214],
    [0.015542, 0.549100, 0.514319, 1.015213],
    [0.014115, 0.547868, 0.515936, 1.015223],
    [0.012806, 0.546743, 0.517432, 1.015223],
    [0.011600, 0.545730, 0.518775, 1.015228],
    [0.010506, 0.544788, 0.520034, 1.015230],
    [0.009510, 0.543920, 0.521179, 1.015238],
    [0.008600, 0.543134, 0.522224, 1.015244],
    [0.007763, 0.542420, 0.523173, 1.015251],
    [0.007006, 0.541757, 0.524065, 1.015252],
    [0.006315, 0.541164, 0.524867, 1.015252],
    [0.005686, 0.540608, 0.525602, 1.015261],
];

/// Inverse of the Bernstein interpolation matrix at t = 0, 1/3, 2/3, 1 (rows: control point,
/// columns: sample). Exact rational values.
const BERN_INV: [[f64; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [-5.0 / 6.0, 3.0, -1.5, 1.0 / 3.0], [1.0 / 3.0, -1.5, 3.0, -5.0 / 6.0], [0.0, 0.0, 0.0, 1.0]];

/// [`BezierMesh::param_at`] for many points: the coarse search's samples are bucketed by where
/// they land once, so each point looks only at the samples around it, not at all of them.
#[derive(Clone, Debug)]
pub struct MeshInverse {
    mesh: BezierMesh,
    /// The coarse samples: where `(s, t)` lands, and `(s, t)`.
    samples: Vec<([f64; 2], f64, f64)>,
    /// `side × side` buckets of sample indices, over the samples' box from `origin`.
    buckets: Vec<Vec<u32>>,
    origin: [f64; 2],
    cell: f64,
    side: usize,
}

impl MeshInverse {
    /// The samples per side of the coarse search, as [`BezierMesh::param_at`]'s.
    const N: usize = 48;
    const SIDE: usize = 32;

    pub fn new(mesh: BezierMesh) -> Self {
        let n = Self::N;
        let mut samples = Vec::with_capacity((n + 1) * (n + 1));
        for j in 0..=n {
            for i in 0..=n {
                let (s, t) = (i as f64 / n as f64, j as f64 / n as f64);
                samples.push((mesh.eval(s, t), s, t));
            }
        }
        let finite = samples.iter().map(|q| q.0).filter(|q| q[0].is_finite() && q[1].is_finite());
        let b = finite.fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, q| [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])]);
        let side = Self::SIDE;
        let span = (b[2] - b[0]).max(b[3] - b[1]);
        let cell = if span.is_finite() && span > 0.0 { span / side as f64 } else { 1.0 };
        let origin = if b[0] <= b[2] { [b[0], b[1]] } else { [0.0, 0.0] };
        let mut buckets = vec![Vec::new(); side * side];
        for (k, (q, _, _)) in samples.iter().enumerate() {
            if let Some(c) = Self::bucket(origin, cell, side, *q)
                && let Some(v) = buckets.get_mut(c[1] * side + c[0])
            {
                v.push(k as u32);
            }
        }
        MeshInverse { mesh, samples, buckets, origin, cell, side }
    }

    /// The bucket `q` falls in, clamped onto the grid; `None` for a non-finite point.
    fn bucket(origin: [f64; 2], cell: f64, side: usize, q: [f64; 2]) -> Option<[usize; 2]> {
        let at = |v: f64, o: f64| ((v - o) / cell).floor().clamp(0.0, (side - 1) as f64) as usize;
        (q[0].is_finite() && q[1].is_finite()).then(|| [at(q[0], origin[0]), at(q[1], origin[1])])
    }

    /// The mesh this inverts.
    pub fn mesh(&self) -> &BezierMesh {
        &self.mesh
    }

    /// As [`BezierMesh::param_at`]: the nearest coarse sample, searched outwards ring by ring of
    /// buckets until none left unsearched can be nearer, then Newton steps.
    pub fn param_at(&self, p: [f64; 2]) -> (f64, f64) {
        let Some(c) = Self::bucket(self.origin, self.cell, self.side, p) else { return (0.5, 0.5) };
        let (cx, cy, side) = (c[0] as i64, c[1] as i64, self.side as i64);
        let mut best = (0.5, 0.5, f64::MAX);
        let visit = |x: i64, y: i64, best: &mut (f64, f64, f64)| {
            let Some(v) = self.buckets.get((y * side + x) as usize) else { return };
            for &k in v {
                let Some((q, s, t)) = self.samples.get(k as usize) else { continue };
                let d = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
                if d < best.2 || (d == best.2 && k < self.index_of(best.0, best.1)) {
                    *best = (*s, *t, d);
                }
            }
        };
        let (o, cell) = (self.origin, self.cell);
        for r in 0..side {
            let (x0, x1, y0, y1) = (cx - r, cx + r, cy - r, cy + r);
            // The ring's buckets on the grid: its top and bottom rows, then its sides between.
            for x in x0.max(0)..=x1.min(side - 1) {
                if y0 >= 0 {
                    visit(x, y0, &mut best);
                }
                if y1 < side && y1 != y0 {
                    visit(x, y1, &mut best);
                }
            }
            for y in (y0 + 1).max(0)..=(y1 - 1).min(side - 1) {
                if x0 >= 0 {
                    visit(x0, y, &mut best);
                }
                if x1 < side && x1 != x0 {
                    visit(x1, y, &mut best);
                }
            }
            // The nearest an unsearched bucket can be: past a side of the searched square that
            // isn't the grid's edge (a point off the grid is past the edge it is off).
            let mut bound = f64::INFINITY;
            if x0 > 0 {
                bound = bound.min(p[0] - (o[0] + x0 as f64 * cell));
            }
            if x1 < side - 1 {
                bound = bound.min(o[0] + (x1 + 1) as f64 * cell - p[0]);
            }
            if y0 > 0 {
                bound = bound.min(p[1] - (o[1] + y0 as f64 * cell));
            }
            if y1 < side - 1 {
                bound = bound.min(o[1] + (y1 + 1) as f64 * cell - p[1]);
            }
            let bound = bound.max(0.0);
            if bound == f64::INFINITY || best.2 <= bound * bound {
                break;
            }
        }
        self.mesh.refine(p, best.0, best.1)
    }

    /// The sample index of `(s, t)`, so equally near samples are picked as `param_at`'s scan does.
    fn index_of(&self, s: f64, t: f64) -> u32 {
        let n = Self::N as f64;
        ((t * n).round() * (n + 1.0) + (s * n).round()) as u32
    }
}

/// A grid of bicubic Bezier patches over the unit square of a source box.
///
/// `us`/`vs` are the patch boundaries in the box's normalised coordinates (`0..=1`, increasing,
/// first 0 and last 1). Control points are a `(3·(us.len()-1)+1) × (3·(vs.len()-1)+1)` grid in
/// row-major order, in output (document) coordinates; neighbouring patches share their edge row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, craft_ui_macros::PlainData)]
pub struct BezierMesh {
    pub us: Vec<f64>,
    pub vs: Vec<f64>,
    pub points: Vec<[f64; 2]>,
}

/// The share of a split grid's line reach within which a press beside an inner anchor still pulls the line (see
/// `BezierMesh::beside_inner_anchor`): measured between 0.91 (a press 20.9 px off pulls it) and 0.93 (one 21.5 px off
/// moves the anchor), at a 23 px reach.
const ON_LINE_SHARE: f64 = 0.92;

impl BezierMesh {
    /// The identity mesh over `bounds` with `cols × rows` patches (Photoshop's default is 1 × 1,
    /// a 4 × 4 control grid).
    pub fn identity(bounds: [f64; 4], cols: usize, rows: usize) -> BezierMesh {
        let (cols, rows) = (cols.clamp(1, 64), rows.clamp(1, 64));
        let us: Vec<f64> = (0..=cols).map(|i| i as f64 / cols as f64).collect();
        let vs: Vec<f64> = (0..=rows).map(|i| i as f64 / rows as f64).collect();
        Self::fit(&|s, t| [bounds[0] + s * (bounds[2] - bounds[0]), bounds[1] + t * (bounds[3] - bounds[1])], us, vs)
    }

    /// Fits a mesh with patch boundaries `us`/`vs` to the map `f` (normalised → output) by
    /// interpolating it at the patch's thirds. Exact for bicubic maps (affine maps included).
    pub fn fit(f: &dyn Fn(f64, f64) -> [f64; 2], us: Vec<f64>, vs: Vec<f64>) -> BezierMesh {
        let (pc, pr) = (us.len() - 1, vs.len() - 1);
        let (nx, ny) = (3 * pc + 1, 3 * pr + 1);
        let mut points = vec![[0.0; 2]; nx * ny];
        for pj in 0..pr {
            for pi in 0..pc {
                let mut s = [[[0.0; 2]; 4]; 4]; // s[row j][col i]
                for (j, row) in s.iter_mut().enumerate() {
                    for (i, v) in row.iter_mut().enumerate() {
                        let u = us[pi] + (us[pi + 1] - us[pi]) * i as f64 / 3.0;
                        let t = vs[pj] + (vs[pj + 1] - vs[pj]) * j as f64 / 3.0;
                        *v = f(u, t);
                    }
                }
                for j in 0..4 {
                    for i in 0..4 {
                        let mut acc = [0.0; 2];
                        for (a, wa) in BERN_INV[j].iter().enumerate() {
                            for (b, wb) in BERN_INV[i].iter().enumerate() {
                                let k = wa * wb;
                                acc[0] += k * s[a][b][0];
                                acc[1] += k * s[a][b][1];
                            }
                        }
                        points[(3 * pj + j) * nx + 3 * pi + i] = acc;
                    }
                }
            }
        }
        BezierMesh { us, vs, points }
    }

    /// Control points per row.
    pub fn nx(&self) -> usize {
        3usize.saturating_mul(self.us.len().saturating_sub(1)).saturating_add(1)
    }
    /// Control point rows.
    pub fn ny(&self) -> usize {
        3usize.saturating_mul(self.vs.len().saturating_sub(1)).saturating_add(1)
    }
    pub fn point(&self, i: usize, j: usize) -> [f64; 2] {
        self.points.get(j.saturating_mul(self.nx()).saturating_add(i)).copied().unwrap_or([0.0; 2])
    }

    /// Checks the invariants (knots and point count); malformed data from files is rejected.
    pub fn is_valid(&self) -> bool {
        let knots_ok = |k: &[f64]| (2..=65).contains(&k.len()) && k[0] == 0.0 && k[k.len() - 1] == 1.0 && k.windows(2).all(|w| w[1] > w[0]);
        if !knots_ok(&self.us) || !knots_ok(&self.vs) {
            return false;
        }
        let Some(nx) = (self.us.len() - 1).checked_mul(3).and_then(|v| v.checked_add(1)) else {
            return false;
        };
        let Some(ny) = (self.vs.len() - 1).checked_mul(3).and_then(|v| v.checked_add(1)) else {
            return false;
        };
        let Some(expected) = nx.checked_mul(ny) else {
            return false;
        };
        self.points.len() == expected && self.points.iter().all(|p| p[0].is_finite() && p[1].is_finite())
    }

    fn locate(knots: &[f64], s: f64) -> (usize, f64) {
        if knots.len() < 2 {
            return (0, 0.0);
        }
        let n = knots.len() - 1;
        let s = if s.is_finite() { s.clamp(0.0, 1.0) } else { 0.0 };
        let i = knots[1..n].partition_point(|&k| k < s);
        let w = knots[i + 1] - knots[i];
        (i, if w > 0.0 { (s - knots[i]) / w } else { 0.0 })
    }

    /// Evaluates the surface at normalised `(s, t)`.
    pub fn eval(&self, s: f64, t: f64) -> [f64; 2] {
        if self.us.len() < 2 || self.vs.len() < 2 {
            return [0.0; 2];
        }
        let (pi, a) = Self::locate(&self.us, s);
        let (pj, b) = Self::locate(&self.vs, t);
        let (ba, bb) = (bernstein(a), bernstein(b));
        let nx = self.nx();
        let mut out = [0.0; 2];
        for (j, wb) in bb.iter().enumerate() {
            for (i, wa) in ba.iter().enumerate() {
                let idx = (3 * pj + j).saturating_mul(nx).saturating_add(3 * pi + i);
                let Some(&p) = self.points.get(idx) else {
                    return [0.0; 2];
                };
                let k = wa * wb;
                out[0] += p[0] * k;
                out[1] += p[1] * k;
            }
        }
        out
    }

    /// Splits the patch column containing `s` at `s` (a vertical split line). Exact: the surface
    /// is unchanged. Returns false when `s` is on or too close to an existing boundary.
    pub fn split_u(&mut self, s: f64) -> bool {
        let (pi, a) = Self::locate(&self.us, s);
        if !(0.01..=0.99).contains(&a) {
            return false;
        }
        let (nx, ny) = (self.nx(), self.ny());
        let mut pts = Vec::with_capacity((nx + 3) * ny);
        for j in 0..ny {
            let row = &self.points[j * nx..(j + 1) * nx];
            pts.extend_from_slice(&row[..3 * pi]);
            let seg = [row[3 * pi], row[3 * pi + 1], row[3 * pi + 2], row[3 * pi + 3]];
            pts.extend_from_slice(&split_cubic(seg, a));
            pts.extend_from_slice(&row[3 * pi + 4..]);
        }
        self.points = pts;
        self.us.insert(pi + 1, s.clamp(0.0, 1.0));
        true
    }

    /// Splits the patch row containing `t` at `t` (a horizontal split line). Exact.
    pub fn split_v(&mut self, t: f64) -> bool {
        self.transpose();
        let ok = self.split_u(t);
        self.transpose();
        ok
    }

    /// Removes interior vertical boundary `k` (1..us.len()-1), merging its two patch columns.
    /// Exact when the patches came from an unedited split; otherwise the outer control points are
    /// kept and the inner handles extrapolated.
    pub fn remove_split_u(&mut self, k: usize) -> bool {
        if k == 0 || k >= self.us.len().saturating_sub(1) {
            return false;
        }
        let a = (self.us[k] - self.us[k - 1]) / (self.us[k + 1] - self.us[k - 1]);
        let (nx, ny) = (self.nx(), self.ny());
        let mut pts = Vec::with_capacity((nx - 3) * ny);
        let c0 = 3 * (k - 1);
        for j in 0..ny {
            let row = &self.points[j * nx..(j + 1) * nx];
            let (p0, q1, r2, p3) = (row[c0], row[c0 + 1], row[c0 + 5], row[c0 + 6]);
            // De Casteljau inverse: q1 = lerp(p0, p1, a), r2 = lerp(p2, p3, a).
            let p1 = [(q1[0] - (1.0 - a) * p0[0]) / a, (q1[1] - (1.0 - a) * p0[1]) / a];
            let p2 = [(r2[0] - a * p3[0]) / (1.0 - a), (r2[1] - a * p3[1]) / (1.0 - a)];
            pts.extend_from_slice(&row[..c0]);
            pts.extend_from_slice(&[p0, p1, p2, p3]);
            pts.extend_from_slice(&row[c0 + 7..]);
        }
        self.points = pts;
        self.us.remove(k);
        true
    }

    /// Removes interior horizontal boundary `k`.
    pub fn remove_split_v(&mut self, k: usize) -> bool {
        self.transpose();
        let ok = self.remove_split_u(k);
        self.transpose();
        ok
    }

    fn transpose(&mut self) {
        let (nx, ny) = (self.nx(), self.ny());
        let mut pts = vec![[0.0; 2]; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                pts[i * ny + j] = self.points[j * nx + i];
            }
        }
        self.points = pts;
        std::mem::swap(&mut self.us, &mut self.vs);
    }

    /// Applies `f` to every control point (exact for affine maps).
    pub fn map_points(&self, f: impl Fn([f64; 2]) -> [f64; 2]) -> BezierMesh {
        BezierMesh { us: self.us.clone(), vs: self.vs.clone(), points: self.points.iter().map(|p| f(*p)).collect() }
    }

    /// Normalised `(s, t)` whose surface point is nearest `p` (coarse search, then Newton
    /// steps on the patch). Used to place split lines where the user clicks.
    pub fn param_at(&self, p: [f64; 2]) -> (f64, f64) {
        let n = 48;
        let mut best = (0.5, 0.5, f64::MAX);
        for j in 0..=n {
            for i in 0..=n {
                let (s, t) = (i as f64 / n as f64, j as f64 / n as f64);
                let q = self.eval(s, t);
                let d = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
                if d < best.2 {
                    best = (s, t, d);
                }
            }
        }
        self.refine(p, best.0, best.1)
    }

    /// Newton steps from `(s, t)` towards the surface point nearest `p`, kept on the mesh.
    fn refine(&self, p: [f64; 2], s: f64, t: f64) -> (f64, f64) {
        let (mut s, mut t) = (s, t);
        for _ in 0..12 {
            // Differences step inward at the far edges, where a step outward reads the clamped edge and no slope.
            let (hs, ht) = (if s + 1e-5 > 1.0 { -1e-5 } else { 1e-5 }, if t + 1e-5 > 1.0 { -1e-5 } else { 1e-5 });
            let q = self.eval(s, t);
            let (qs, qt) = (self.eval(s + hs, t), self.eval(s, t + ht));
            let (a, b, c, d) = ((qs[0] - q[0]) / hs, (qt[0] - q[0]) / ht, (qs[1] - q[1]) / hs, (qt[1] - q[1]) / ht);
            let det = a * d - b * c;
            if det.abs() < 1e-12 {
                break;
            }
            let (ex, ey) = (p[0] - q[0], p[1] - q[1]);
            s = (s + (d * ex - b * ey) / det).clamp(0.0, 1.0);
            t = (t + (a * ey - c * ex) / det).clamp(0.0, 1.0);
        }
        (s, t)
    }

    /// The pull on a one-patch grid (the Default grid), as measured in the reference application
    /// (154 drags of its default warp grid, read back from the saved mesh): each axis on its own.
    /// The grid's thirds are its handle lines; a grab in a third moves the two lines that bound it,
    /// shared by where the grab is between them (`1 - x` and `x`). The border line takes
    /// [`ONE_PATCH_BORDER`] of its share, and the next line inward a [`ONE_PATCH_SPILL`] of its
    /// neighbour's. A point moves by its column's share times its row's, all scaled so that the
    /// grabbed surface point lands exactly at its start + `d`. So a grab in the middle bends the
    /// inside of the picture and leaves its frame alone, and one near an edge or corner bows that
    /// edge or moves that corner too.
    fn pull_one_patch(&mut self, s: f64, t: f64, d: [f64; 2]) {
        let (ws, wt, bs, bt) = (one_patch_share(s), one_patch_share(t), bernstein(s), bernstein(t));
        // The grabbed point's move per unit of pull: the axes are independent, so it factors.
        let dot = |w: [f64; 4], b: [f64; 4]| w.iter().zip(b).map(|(w, b)| w * b).sum::<f64>();
        let at_grab = dot(ws, bs) * dot(wt, bt);
        if at_grab <= 1e-12 || self.points.len() != 16 {
            return;
        }
        for (k, p) in self.points.iter_mut().enumerate() {
            let f = ws[k % 4] * wt[k / 4] / at_grab;
            p[0] += f * d[0];
            p[1] += f * d[1];
        }
    }

    /// Pulls the surface point at normalised `(s, t)` by `d`. `base` is the control grid at the
    /// start of the drag. A press that is not near a grid line: see [`Self::pull_at`].
    pub fn pull(&mut self, base: &[[f64; 2]], s: f64, t: f64, d: [f64; 2]) {
        if base.len() != self.points.len() {
            return;
        }
        self.points.copy_from_slice(base);
        let press = self.eval(s, t);
        self.pull_at(base, s, t, press, d, 0.0);
    }

    /// Pulls the surface grabbed at `press` (its grid parameters `(s, t)`) by `d`, as dragging
    /// the warp grid does in the reference application (measured from its saved meshes).
    /// `base` is the control grid at the start of the drag; `reach` how close (in the mesh's
    /// units) a press must be to a grid line to pull that line.
    /// - The unsplit Default grid bends smoothly ([`Self::pull_one_patch`]).
    /// - On a split grid, a press within `reach` of an anchor line (a split line or the border)
    ///   pulls that line: its segment bends by the one-patch rule along it, and its nearest point
    ///   lands on the pointer; except in the third of a segment next to an inner anchor, which moves
    ///   as below. A moved anchor takes the points around it along; a moved handle
    ///   turns its anchor's frame, as dragging that handle does.
    /// - Anywhere else on a split grid, the grab moves part of the grid rigidly by `d`, chosen per
    ///   axis by where in its patch the press is: the near third moves the nearest anchor line with
    ///   its handles, the middle third the patch's two anchor lines and all between them.
    pub fn pull_at(&mut self, base: &[[f64; 2]], s: f64, t: f64, press: [f64; 2], d: [f64; 2], reach: f64) {
        if base.len() != self.points.len() || !d.iter().all(|v| v.is_finite()) || !press.iter().all(|v| v.is_finite()) {
            return;
        }
        self.points.copy_from_slice(base);
        let (cols, rows) = (self.us.len() - 1, self.vs.len() - 1);
        if cols == 1 && rows == 1 {
            self.pull_one_patch(s, t, d);
            return;
        }
        if reach > 0.0
            && let Some(line) = self.nearest_anchor_line(press, s, t, reach)
            && !self.beside_inner_anchor(line, press, reach)
        {
            self.pull_line(base, line, press, d);
            return;
        }
        let (nx, ny) = (self.nx(), self.ny());
        let (x0, x1) = Self::cell_range(&self.us, nx, s);
        let (y0, y1) = Self::cell_range(&self.vs, ny, t);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if let Some(p) = self.points.get_mut(y * nx + x) {
                    p[0] += d[0];
                    p[1] += d[1];
                }
            }
        }
    }

    /// The control lines (first and last index) a grab at `s` moves rigidly along one axis of a
    /// split grid: by the third of its patch it is in, the nearer anchor line with its two handle
    /// lines, or (the middle third) both anchor lines and everything between, with their handles.
    fn cell_range(knots: &[f64], n: usize, s: f64) -> (usize, usize) {
        let (m, a) = Self::locate(knots, s);
        let (lo, hi) = (3 * m, 3 * m + 3);
        let (first, last) = if a < 1.0 / 3.0 {
            (lo.saturating_sub(1), lo + 1)
        } else if a > 2.0 / 3.0 {
            (hi - 1, hi + 1)
        } else {
            (lo.saturating_sub(1), hi + 1)
        };
        (first, last.min(n - 1))
    }

    /// Whether a press near `line` but off it lies beside an inner anchor (a split line's crossing with another anchor
    /// line, not the border): its nearest point on the line in the third of its segment next to that anchor, and more
    /// than [`ON_LINE_SHARE`] of `reach` from the line. There the reference application moves the anchor with its
    /// handles, as a press off the lines does, rather than pulling the line. Measured on split grids: a press 21.5 px
    /// from a split line pulls the line mid-segment and moves the crossing beside it; one 20.9 px off, or nearer, pulls
    /// the line.
    fn beside_inner_anchor(&self, (column, k, v): (bool, usize, f64), press: [f64; 2], reach: f64) -> bool {
        let knots = if column { &self.vs } else { &self.us };
        let (m, a) = Self::locate(knots, v);
        if !((a < 1.0 / 3.0 && m > 0) || (a > 2.0 / 3.0 && m + 2 < knots.len())) {
            return false;
        }
        let Some(&across) = (if column { self.us.get(k) } else { self.vs.get(k) }) else {
            return false;
        };
        let on_line = if column { self.eval(across, v) } else { self.eval(v, across) };
        (on_line[0] - press[0]).hypot(on_line[1] - press[1]) > reach * ON_LINE_SHARE
    }

    /// The anchor line (column `true` or row `false`, and its knot index) a press pulls: of those
    /// within `reach` of `press`, the one nearest in the grid's coordinates `(s, t)`; with the
    /// parameter along it of its point nearest the press.
    /// `self` still holds the grid the drag began with (see [`Self::pull_at`]).
    fn nearest_anchor_line(&self, press: [f64; 2], s: f64, t: f64, reach: f64) -> Option<(bool, usize, f64)> {
        let mut best: Option<(f64, (bool, usize, f64))> = None;
        for (column, knots, along) in [(true, &self.us, t), (false, &self.vs, s)] {
            for (k, &knot) in knots.iter().enumerate() {
                // The point of this line nearest the press: a short search around the press's own
                // parameter along it.
                let at = |v: f64| if column { self.eval(knot, v) } else { self.eval(v, knot) };
                let (mut v, mut dist) = (along, f64::MAX);
                for step in 0..=40 {
                    let w = (along - 0.1 + 0.2 * step as f64 / 40.0).clamp(0.0, 1.0);
                    let q = at(w);
                    let e = (q[0] - press[0]).hypot(q[1] - press[1]);
                    if e < dist {
                        (v, dist) = (w, e);
                    }
                }
                // Of the lines within reach, the one nearest in the grid's own coordinates: a press 11.4 px beside a
                // vertical line and 8.6 px below a horizontal one pulls the vertical line on a grid 1.5 times wider
                // than it is tall.
                let across = (knot - if column { s } else { t }).abs();
                if dist <= reach && best.is_none_or(|b| across < b.0) {
                    best = Some((across, (column, k, v)));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Pulls anchor line `(column, k, v)` so that its point at `v` lands on `press + d`.
    fn pull_line(&mut self, base: &[[f64; 2]], (column, k, v): (bool, usize, f64), press: [f64; 2], d: [f64; 2]) {
        let (nx, ny) = (self.nx(), self.ny());
        let knots = if column { &self.vs } else { &self.us };
        let (m, a) = Self::locate(knots, v);
        let w = one_patch_share(a);
        let b = bernstein(a);
        let at_grab: f64 = w.iter().zip(b).map(|(w, b)| w * b).sum();
        if at_grab <= 1e-12 {
            return;
        }
        let knot = if column { self.us.get(k).copied() } else { self.vs.get(k).copied() };
        let Some(knot) = knot else { return };
        // Still the drag's starting grid here: nothing has moved yet.
        let on_line = if column { self.eval(knot, v) } else { self.eval(v, knot) };
        // The line's nearest point goes to the pointer.
        let target = [press[0] + d[0] - on_line[0], press[1] + d[1] - on_line[1]];
        let index = |r: usize| if column { (3 * k, r) } else { (r, 3 * k) };
        let moves: Vec<((usize, usize), [f64; 2])> = (0..4).map(|q| (index(3 * m + q), [target[0] * w[q] / at_grab, target[1] * w[q] / at_grab])).collect();
        // The anchors first, with the points around them; then the segment's own handles.
        for &((x, y), mv) in [moves[0], moves[3]].iter() {
            if mv == [0.0, 0.0] {
                continue;
            }
            for yy in y.saturating_sub(1)..=(y + 1).min(ny - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(nx - 1) {
                    if let Some(p) = self.points.get_mut(yy * nx + xx) {
                        p[0] += mv[0];
                        p[1] += mv[1];
                    }
                }
            }
        }
        for &((x, y), mv) in [moves[1], moves[2]].iter() {
            if let (Some(q), Some(b)) = (self.points.get_mut(y * nx + x), base.get(y * nx + x)) {
                *q = [b[0] + mv[0], b[1] + mv[1]];
            }
        }
        for &((x, y), mv) in [moves[1], moves[2]].iter() {
            if mv != [0.0, 0.0] {
                self.turn_frame(base, x, y);
            }
        }
    }

    /// Drags control point `index` by `d` from the grid `base` it had when the drag began, as the
    /// reference application does (measured from its saved meshes):
    /// - on the unsplit Default grid every control point is free: the point moves alone;
    /// - on a split grid an anchor moves with the points around it (its handles and the twists
    ///   between them), as one block;
    /// - an interior anchor's handle goes where it is dragged and turns the anchor's whole frame
    ///   with it: the other handles and the twists turn by the same angle, keeping their lengths,
    ///   so every line stays smooth through the anchor;
    /// - an edge anchor's handle along the edge turns the opposite handle (and the twist beside
    ///   it); its handle into the picture, and a mesh corner's handles, move alone.
    pub fn drag_point(&mut self, base: &[[f64; 2]], index: usize, d: [f64; 2]) {
        if base.len() != self.points.len() || index >= base.len() || !d.iter().all(|v| v.is_finite()) {
            return;
        }
        self.points.copy_from_slice(base);
        let (nx, ny) = (self.nx(), self.ny());
        let (i, j) = (index % nx, index / nx);
        let unsplit = self.us.len() == 2 && self.vs.len() == 2;
        match (i % 3, j % 3) {
            (0, 0) if !unsplit => {
                for y in j.saturating_sub(1)..=(j + 1).min(ny - 1) {
                    for x in i.saturating_sub(1)..=(i + 1).min(nx - 1) {
                        if let Some(p) = self.points.get_mut(y * nx + x) {
                            p[0] += d[0];
                            p[1] += d[1];
                        }
                    }
                }
            }
            (0, _) | (_, 0) if !unsplit => self.drag_handle(base, i, j, d),
            _ => {
                if let Some(p) = self.points.get_mut(index) {
                    p[0] += d[0];
                    p[1] += d[1];
                }
            }
        }
    }

    /// The handle at column `i`, row `j` of a split grid, dragged by `d` (see [`Self::drag_point`]).
    fn drag_handle(&mut self, base: &[[f64; 2]], i: usize, j: usize, d: [f64; 2]) {
        let nx = self.nx();
        if let (Some(p), Some(b)) = (self.points.get_mut(j * nx + i), base.get(j * nx + i)) {
            *p = [b[0] + d[0], b[1] + d[1]];
        }
        self.turn_frame(base, i, j);
    }

    /// After the handle at column `i`, row `j` has moved (and perhaps its anchor), turns the rest
    /// of its anchor's frame by the angle the handle turned through, measured from `base`:
    /// an interior anchor's other handles turn (keeping their lengths) and each twist moves by
    /// both its handles' moves; an edge anchor's handle along the edge turns the opposite handle
    /// and the twist beside it; an edge anchor's handle into the picture, and a mesh corner's
    /// handles, turn nothing.
    fn turn_frame(&mut self, base: &[[f64; 2]], i: usize, j: usize) {
        let (nx, ny) = (self.nx(), self.ny());
        let near = |k: usize| if k % 3 == 1 { k - 1 } else { k + 1 };
        let (ai, aj) = if i.is_multiple_of(3) { (i, near(j)) } else { (near(i), j) };
        let get = |pts: &[[f64; 2]], x: usize, y: usize| pts.get(y * nx + x).copied();
        let (Some(a0), Some(a1), Some(h0), Some(h1)) = (get(base, ai, aj), get(&self.points, ai, aj), get(base, i, j), get(&self.points, i, j)) else {
            return;
        };
        let (on_x, on_y) = (ai == 0 || ai == nx - 1, aj == 0 || aj == ny - 1);
        if on_x && on_y {
            return;
        }
        let (v0, v1) = ([h0[0] - a0[0], h0[1] - a0[1]], [h1[0] - a1[0], h1[1] - a1[1]]);
        if v0[0].hypot(v0[1]) < 1e-9 || v1[0].hypot(v1[1]) < 1e-9 {
            return;
        }
        let angle = (v0[0] * v1[1] - v0[1] * v1[0]).atan2(v0[0] * v1[0] + v0[1] * v1[1]);
        let (sin, cos) = angle.sin_cos();
        // Where a point of the frame goes: turned about the anchor, wherever the anchor now is.
        let turn = |p: [f64; 2]| {
            let (x, y) = (p[0] - a0[0], p[1] - a0[1]);
            [a1[0] + x * cos - y * sin, a1[1] + x * sin + y * cos]
        };
        if on_x || on_y {
            let along = if on_x { i == ai } else { j == aj };
            if !along {
                return;
            }
            let (oi, oj) = (2 * ai as isize - i as isize, 2 * aj as isize - j as isize);
            let inward = |k: usize, last: usize| {
                if k == 0 {
                    1isize
                } else if k == last {
                    -1
                } else {
                    0
                }
            };
            let (ti, tj) = if on_x { (oi + inward(ai, nx - 1), oj) } else { (oi, oj + inward(aj, ny - 1)) };
            for (x, y) in [(oi, oj), (ti, tj)] {
                if x >= 0 && y >= 0 && (x as usize) < nx && (y as usize) < ny {
                    let (x, y) = (x as usize, y as usize);
                    if let (Some(b), Some(p)) = (get(base, x, y), self.points.get_mut(y * nx + x)) {
                        *p = turn(b);
                    }
                }
            }
            return;
        }
        // An interior anchor: the other handles turn; each twist moves by its two handles' moves
        // (relative to the anchor).
        let handles = [(ai - 1, aj), (ai + 1, aj), (ai, aj - 1), (ai, aj + 1)];
        let mut rel = [[0.0f64; 2]; 4];
        for (k, &(x, y)) in handles.iter().enumerate() {
            let Some(b) = get(base, x, y) else { continue };
            let new = if (x, y) == (i, j) { h1 } else { turn(b) };
            rel[k] = [new[0] - a1[0] - (b[0] - a0[0]), new[1] - a1[1] - (b[1] - a0[1])];
            if let Some(p) = self.points.get_mut(y * nx + x) {
                *p = new;
            }
        }
        for (hx, hy) in [(0, 2), (1, 2), (0, 3), (1, 3)] {
            let (x, y) = (handles[hx].0, handles[hy].1);
            if let Some(p) = self.points.get_mut(y * nx + x) {
                p[0] += rel[hx][0] + rel[hy][0];
                p[1] += rel[hx][1] + rel[hy][1];
            }
        }
    }

    /// Bounding box of the control points (the surface lies inside it).
    pub fn control_bounds(&self) -> [f64; 4] {
        self.points.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
    }
}

/// A complete warp: what Photoshop's `warp` descriptor stores, plus the custom mesh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, craft_ui_macros::PlainData)]
#[serde(rename_all = "camelCase", from = "SavedWarp")]
pub struct Warp {
    pub style: WarpStyle,
    /// Bend in percent (-100..100), presets only.
    #[serde(default)]
    pub bend: f64,
    #[serde(default)]
    pub h_distort: f64,
    #[serde(default)]
    pub v_distort: f64,
    /// Warp along the vertical axis.
    #[serde(default)]
    pub vertical: bool,
    /// The source box `[x0, y0, x1, y1]` the warp is defined over.
    pub bounds: [f64; 4],
    /// Control mesh for [`WarpStyle::Custom`] and [`WarpStyle::Cylinder`] (output coordinates).
    #[serde(default)]
    pub mesh: Option<BezierMesh>,
    /// A cylinder's seven `warpValues` (see [`Warp::cylinder`]); empty for other styles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<f64>,
    /// The mesh was given with fewer than four control points along a side: the reference
    /// application's preset envelopes (every bend preset but Fisheye, Twist and the Shells, which
    /// it stores as full 4 × 4 meshes), and custom envelopes read from a file. It tessellates such
    /// a surface by a different rule than a full mesh; see the renderer.
    #[serde(default)]
    pub envelope: bool,
}

/// A [`Warp`] as saved: a file from before [`Warp::envelope`] has none, and its presets are
/// envelopes all the same ([`Warp::preset`]), so it reopens as it would be made now.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedWarp {
    style: WarpStyle,
    #[serde(default)]
    bend: f64,
    #[serde(default)]
    h_distort: f64,
    #[serde(default)]
    v_distort: f64,
    #[serde(default)]
    vertical: bool,
    bounds: [f64; 4],
    #[serde(default)]
    mesh: Option<BezierMesh>,
    #[serde(default)]
    values: Vec<f64>,
    #[serde(default)]
    envelope: Option<bool>,
}

impl From<SavedWarp> for Warp {
    fn from(w: SavedWarp) -> Warp {
        let envelope = w.envelope.unwrap_or_else(|| preset_envelope(w.style));
        Warp {
            style: w.style,
            bend: w.bend,
            h_distort: w.h_distort,
            v_distort: w.v_distort,
            vertical: w.vertical,
            bounds: w.bounds,
            mesh: w.mesh,
            values: w.values,
            envelope,
        }
    }
}

/// Whether `style`'s preset is an envelope ([`Warp::envelope`]): every bend preset but Fisheye,
/// Twist and the Shells; never a custom mesh or a cylinder.
fn preset_envelope(style: WarpStyle) -> bool {
    !matches!(
        style,
        WarpStyle::None | WarpStyle::Custom | WarpStyle::Cylinder | WarpStyle::Fisheye | WarpStyle::Twist | WarpStyle::ShellLower | WarpStyle::ShellUpper
    )
}

/// Relative to the magnitude so the fit's rounding at large origins isn't rejected.
fn point_fits(expected: [f64; 2], actual: [f64; 2]) -> bool {
    let tol = |e: f64| (1e-9 * e.abs().max(1.0)).min(1e-6);
    !((actual[0] - expected[0]).abs() >= tol(expected[0]) || (actual[1] - expected[1]).abs() >= tol(expected[1]))
}

impl Warp {
    /// No warp over `bounds`.
    pub fn none(bounds: [f64; 4]) -> Warp {
        Warp { style: WarpStyle::None, bend: 0.0, h_distort: 0.0, v_distort: 0.0, vertical: false, bounds, mesh: None, values: Vec::new(), envelope: false }
    }

    /// A preset style.
    pub fn preset(style: WarpStyle, bend: f64, bounds: [f64; 4]) -> Warp {
        Warp { style, bend, envelope: preset_envelope(style), ..Warp::none(bounds) }
    }

    /// A custom warp from a mesh.
    pub fn custom(mesh: BezierMesh, bounds: [f64; 4]) -> Warp {
        Warp { style: WarpStyle::Custom, mesh: Some(mesh), ..Warp::none(bounds) }
    }

    /// A horizontal Cylinder warp over `bounds` from its seven `warpValues` (see
    /// [`cylinder_mesh`]). `None` when they don't describe one.
    pub fn cylinder(values: &[f64], bounds: [f64; 4]) -> Option<Warp> {
        let mesh = cylinder_mesh(values)?;
        Some(Warp { style: WarpStyle::Cylinder, mesh: Some(mesh), values: values.to_vec(), ..Warp::none(bounds) })
    }

    fn style_warp(&self) -> Option<StyleWarp> {
        StyleWarp::new(self.style, self.bend, self.h_distort, self.v_distort, self.vertical, self.bounds)
    }

    /// Whether the warp maps every point of its box onto itself (within `1e-9` px).
    pub fn is_identity(&self) -> bool {
        match self.style {
            WarpStyle::None => true,
            WarpStyle::Custom | WarpStyle::Cylinder => match &self.mesh {
                None => true,
                Some(m) => self.mesh_is_box_fit(m),
            },
            _ => self.style_warp().is_none(),
        }
    }

    /// An identity warp with nothing to keep: no splits either. A split grid that is still flat
    /// renders as no warp, but the reference application keeps it (its lines come back when Warp
    /// is chosen again), so it is stored.
    pub fn is_plain_identity(&self) -> bool {
        self.is_identity() && self.mesh.as_ref().is_none_or(|m| m.us.len() <= 2 && m.vs.len() <= 2)
    }

    /// True when every control point of `m` sits where the box's own grid puts it.
    fn mesh_is_box_fit(&self, m: &BezierMesh) -> bool {
        if !m.is_valid() {
            return false;
        }
        let (pc, pr) = (m.us.len() - 1, m.vs.len() - 1);
        for pj in 0..pr {
            for j in 0..4 {
                if !self.mesh_row_is_box_fit(m, pj, j, pc) {
                    return false;
                }
            }
        }
        true
    }

    /// One sample row (`j` within patch row `pj`) of [`Self::mesh_is_box_fit`].
    fn mesh_row_is_box_fit(&self, m: &BezierMesh, pj: usize, j: usize, pc: usize) -> bool {
        let nx = m.nx();
        let (Some(&v0), Some(&v1)) = (m.vs.get(pj), m.vs.get(pj + 1)) else {
            return false;
        };
        let v = v0 + (v1 - v0) * j as f64 / 3.0;
        for pi in 0..pc {
            let (Some(&u0), Some(&u1)) = (m.us.get(pi), m.us.get(pi + 1)) else {
                return false;
            };
            for i in 0..4 {
                let u = u0 + (u1 - u0) * i as f64 / 3.0;
                let expected = self.box_point(u, v);
                let Some(&actual) = m.points.get((3 * pj + j) * nx + 3 * pi + i) else {
                    return false;
                };
                if !point_fits(expected, actual) {
                    return false;
                }
            }
        }
        true
    }

    fn box_point(&self, s: f64, t: f64) -> [f64; 2] {
        let b = self.bounds;
        [b[0] + s * (b[2] - b[0]), b[1] + t * (b[3] - b[1])]
    }

    /// Maps a source point (inside the box) to output space.
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        match self.style {
            WarpStyle::None => (x, y),
            WarpStyle::Custom | WarpStyle::Cylinder => match &self.mesh {
                Some(m) => {
                    let b = self.bounds;
                    let (w, h) = (b[2] - b[0], b[3] - b[1]);
                    if w <= 0.0 || h <= 0.0 {
                        return (x, y);
                    }
                    let p = m.eval((x - b[0]) / w, (y - b[1]) / h);
                    (p[0], p[1])
                }
                None => (x, y),
            },
            _ => match self.style_warp() {
                Some(sw) => sw.apply(x, y),
                None => (x, y),
            },
        }
    }

    /// The warp as a control mesh (a preset is its own Bezier patch split into `cols × rows`;
    /// custom and cylinder warps return their own mesh).
    pub fn to_mesh(&self, cols: usize, rows: usize) -> BezierMesh {
        if let (WarpStyle::Custom | WarpStyle::Cylinder, Some(m)) = (self.style, &self.mesh) {
            return m.clone();
        }
        let (cols, rows) = (cols.clamp(1, 64), rows.clamp(1, 64));
        // A preset is one Bezier patch: split it exactly.
        if let Some(sw) = self.style_warp() {
            let mut m = sw.mesh();
            for i in 1..cols {
                m.split_u(i as f64 / cols as f64);
            }
            for j in 1..rows {
                m.split_v(j as f64 / rows as f64);
            }
            return m;
        }
        let us: Vec<f64> = (0..=cols).map(|i| i as f64 / cols as f64).collect();
        let vs: Vec<f64> = (0..=rows).map(|i| i as f64 / rows as f64).collect();
        BezierMesh::fit(
            &|s, t| {
                let p = self.box_point(s, t);
                let (x, y) = self.map(p[0], p[1]);
                [x, y]
            },
            us,
            vs,
        )
    }

    /// The same warp followed by the affine map `[a, b, c, d, e, f]` (`x' = a·x + c·y + e`,
    /// `y' = b·x + d·y + f`), as a custom mesh (exact for custom warps).
    pub fn then_affine(&self, m: [f64; 6]) -> Warp {
        let mesh = self.to_mesh(4, 4).map_points(|p| [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]);
        Warp::custom(mesh, self.bounds)
    }

    /// Approximate output bounds (sampled; includes the control mesh for custom warps).
    pub fn output_bounds(&self) -> [f64; 4] {
        let n = 32;
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for j in 0..=n {
            for i in 0..=n {
                let p = self.box_point(i as f64 / n as f64, j as f64 / n as f64);
                let (x, y) = self.map(p[0], p[1]);
                b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
        }
        b
    }
}

#[cfg(test)]
mod tests;
