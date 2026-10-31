//! Independent mesh topology, UV masks and reference 3D preview for PhotoCraft.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod import;
pub mod lighting;
mod perspective;
pub mod prepared;
pub mod preview;
pub mod topology;
pub mod wrap;

use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);

pub const MAX_TRIANGLES: usize = 1_000_000;
pub const MAX_VERTICES: usize = 3_000_000;
pub const MAX_PIXELS: usize = 16_777_216;
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    pub color: [f32; 4],
    /// Resolved by the importer; an absent file remains visible to the user for relinking.
    pub base_color: Option<String>,
    #[serde(default)]
    pub normal_map: Option<String>,
    #[serde(default)]
    pub maps: std::collections::BTreeMap<preview::Channel, String>,
    #[serde(skip)]
    pub embedded: Option<Vec<u8>>,
    #[serde(skip)]
    pub embedded_normal: Option<Vec<u8>>,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            name: "Default".into(),
            color: [0.7, 0.7, 0.7, 1.0],
            base_color: None,
            normal_map: None,
            maps: Default::default(),
            embedded: None,
            embedded_normal: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Corner {
    /// Original control-point identity, preserved across UV splits and material boundaries.
    pub vertex: usize,
    /// Bottom-left texture origin, independent of the source format.
    pub uv: [f64; 2],
    /// Authored split normal in world space; absent values use the geometric face normal.
    #[serde(default)]
    pub normal: Option<[f64; 3]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Triangle {
    pub corners: [Corner; 3],
    pub material: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Scene {
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<Triangle>,
    pub materials: Vec<Material>,
    pub warnings: Vec<String>,
}

impl Scene {
    pub fn validate(&self) -> Result<()> {
        if self.triangles.is_empty() || self.positions.is_empty() || self.materials.is_empty() {
            return Err(Error("The model has no UV-mapped mesh".into()));
        }
        if self.triangles.len() > MAX_TRIANGLES || self.positions.len() > MAX_VERTICES {
            return Err(Error("Model exceeds the mesh budget (1M triangles / 3M vertices)".into()));
        }
        if self.positions.iter().flatten().any(|x| !x.is_finite() || x.abs() > 1e15) {
            return Err(Error("Model contains invalid positions".into()));
        }
        for t in &self.triangles {
            if t.material >= self.materials.len()
                || t.corners.iter().any(|c| {
                    c.vertex >= self.positions.len()
                        || c.uv.iter().any(|x| !x.is_finite() || x.abs() > 100_000.0)
                        || c.normal.is_some_and(|n| n.iter().any(|v| !v.is_finite() || v.abs() > 1e15) || glam::DVec3::from_array(n).length_squared() < 1e-24)
                })
            {
                return Err(Error("Model contains invalid vertex, material or UV data".into()));
            }
        }
        Ok(())
    }
}

pub(crate) fn filled<T: Clone>(len: usize, value: T) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| Error("Not enough memory for the UV operation".into()))?;
    v.resize(len, value);
    Ok(v)
}
