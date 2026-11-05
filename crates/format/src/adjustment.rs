//! Large adjustment payloads use the bundle's bounded, content-addressed blob storage.
use std::sync::Arc;

use photocraft_doc::Adjustment;
use photocraft_doc::adjust::{LutFormat, LutSource};
use serde::{Deserialize, Serialize};

use crate::convert::{Fetch, Sink};
use crate::manifest::Hash;
use crate::{FormatError, Result};

/// Existing small adjustments retain their schema; old inline LUTs remain readable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AdjustmentM {
    Stored(StoredAdjustment),
    Inline(Adjustment),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StoredAdjustment {
    ColorLookup {
        name: String,
        lut: Option<Hash>,
        size: u32,
        #[serde(default)]
        tetrahedral: bool,
        #[serde(default)]
        dither: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<LutSourceM>,
    },
    Unsupported {
        psd_key: String,
        raw: Hash,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LutSourceM {
    pub format: LutFormat,
    pub bytes: Hash,
}

impl AdjustmentM {
    pub(crate) fn store(adjustment: &Adjustment, sink: &mut dyn Sink) -> Result<Self> {
        let stored = match adjustment {
            Adjustment::ColorLookup { name, lut, size, tetrahedral, dither, source } => StoredAdjustment::ColorLookup {
                name: name.clone(),
                lut: lut.as_ref().map(|values| store_lut(values, sink)).transpose()?,
                size: *size,
                tetrahedral: *tetrahedral,
                dither: *dither,
                source: source.as_ref().map(|source| LutSourceM { format: source.format, bytes: sink.blob(&source.bytes) }),
            },
            Adjustment::Unsupported { psd_key, raw } => StoredAdjustment::Unsupported { psd_key: psd_key.clone(), raw: sink.blob(&Arc::new(raw.to_vec())) },
            other => return Ok(Self::Inline(other.clone())),
        };
        Ok(Self::Stored(stored))
    }

    pub(crate) fn load(&self, fetch: &mut dyn Fetch) -> Result<Adjustment> {
        match self {
            Self::Inline(adjustment) => Ok(adjustment.clone()),
            Self::Stored(StoredAdjustment::Unsupported { psd_key, raw }) => {
                Ok(Adjustment::Unsupported { psd_key: psd_key.clone(), raw: fetch.blob(raw)?.as_slice().into() })
            }
            Self::Stored(StoredAdjustment::ColorLookup { name, lut, size, tetrahedral, dither, source }) => Ok(Adjustment::ColorLookup {
                name: name.clone(),
                lut: lut.as_ref().map(|hash| load_lut(hash, fetch)).transpose()?,
                size: *size,
                tetrahedral: *tetrahedral,
                dither: *dither,
                source: source
                    .as_ref()
                    .map(|source| -> Result<_> { Ok(LutSource { format: source.format, bytes: fetch.blob(&source.bytes)? }) })
                    .transpose()?,
            }),
        }
    }
}

fn store_lut(values: &[f32], sink: &mut dyn Sink) -> Result<Hash> {
    let len = values.len().checked_mul(4).ok_or_else(|| FormatError::LimitExceeded("Color Lookup table is too large".into()))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).map_err(|_| FormatError::LimitExceeded("not enough memory for Color Lookup table".into()))?;
    for (index, value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(FormatError::NonFinite { path: format!("ColorLookup.lut[{index}]") });
        }
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(sink.blob(&Arc::new(bytes)))
}

fn load_lut(hash: &str, fetch: &mut dyn Fetch) -> Result<Arc<Vec<f32>>> {
    let bytes = fetch.blob(hash)?;
    let (samples, remainder) = bytes.as_chunks::<4>();
    if !remainder.is_empty() {
        return Err(FormatError::corrupt("Color Lookup blob is not a sequence of f32 samples"));
    }
    let mut values = Vec::new();
    values.try_reserve_exact(samples.len()).map_err(|_| FormatError::LimitExceeded("not enough memory for Color Lookup table".into()))?;
    for sample in samples {
        let value = f32::from_le_bytes(*sample);
        if !value.is_finite() {
            return Err(FormatError::corrupt("Color Lookup blob contains a non-finite sample"));
        }
        values.push(value);
    }
    Ok(Arc::new(values))
}

#[cfg(test)]
mod tests;
