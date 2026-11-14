//! Typed row conversion keeps validation out of the per-pixel framing loop.

use photocraft_codecs::f16;

use super::Precision;
use crate::HistoricalXcfSampleOrder;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SampleChanges {
    pub(crate) underflow: bool,
    pub(crate) coverage_clamped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RowSampleError {
    pub(crate) sample: usize,
    pub(crate) message: &'static str,
}

impl RowSampleError {
    fn new(sample: usize, message: &'static str) -> Self {
        Self { sample, message }
    }
}

/// Select storage and byte order once per row. F16/F32 are already native-range
/// values, so their decode/finite-validation passes need no f64 round trip or
/// per-sample Result. Only F64 can underflow or overflow native F32 storage.
pub(crate) fn row_samples(
    raw: &[u8],
    output: &mut [f32],
    precision: Precision,
    order: HistoricalXcfSampleOrder,
    channels: usize,
    coverage: bool,
) -> Result<SampleChanges, RowSampleError> {
    if !(1..=4).contains(&channels) || !output.len().is_multiple_of(channels) || output.len().checked_mul(precision.bytes) != Some(raw.len()) {
        return Err(RowSampleError::new(0, "sample row size does not match the image precision"));
    }
    let little = order == HistoricalXcfSampleOrder::LittleEndian;
    match (precision.float, precision.bytes) {
        (false, 1) => {
            for (out, value) in output.iter_mut().zip(raw) {
                *out = f32::from(*value) / 255.0;
            }
        }
        (false, 2) => {
            if little {
                for (out, value) in output.iter_mut().zip(raw.as_chunks::<2>().0) {
                    *out = f32::from(u16::from_le_bytes(*value)) / 65535.0;
                }
            } else {
                super::integer_samples(raw, output, 2).map_err(|_| RowSampleError::new(0, "integer row size does not match the image precision"))?;
            }
        }
        (false, 4) => {
            if little {
                for (out, value) in output.iter_mut().zip(raw.as_chunks::<4>().0) {
                    *out = (f64::from(u32::from_le_bytes(*value)) / f64::from(u32::MAX)) as f32;
                }
            } else {
                for (out, value) in output.iter_mut().zip(raw.as_chunks::<4>().0) {
                    *out = (f64::from(u32::from_be_bytes(*value)) / f64::from(u32::MAX)) as f32;
                }
            }
        }
        (true, 2) => {
            if little {
                for (out, value) in output.iter_mut().zip(raw.as_chunks::<2>().0) {
                    *out = f16::from_bits(u16::from_le_bytes(*value)).to_f32();
                }
            } else {
                for (out, value) in output.iter_mut().zip(raw.as_chunks::<2>().0) {
                    *out = f16::from_bits(u16::from_be_bytes(*value)).to_f32();
                }
            }
            return finish_float_row(output, channels, coverage);
        }
        (true, 4) => {
            if little {
                for (out, value) in output.iter_mut().zip(raw.as_chunks::<4>().0) {
                    *out = f32::from_le_bytes(*value);
                }
            } else {
                for (out, value) in output.iter_mut().zip(raw.as_chunks::<4>().0) {
                    *out = f32::from_be_bytes(*value);
                }
            }
            return finish_float_row(output, channels, coverage);
        }
        (true, 8) => return double_row(raw, output, little, channels, coverage),
        _ => return Err(RowSampleError::new(0, "sample size does not match the image precision")),
    }
    Ok(SampleChanges::default())
}

/// Validate the whole decoded row before normalization. Keeping byte decoding
/// separate lets the compiler vectorize it; no nonfinite value is published to a native surface.
fn finish_float_row(output: &mut [f32], channels: usize, coverage: bool) -> Result<SampleChanges, RowSampleError> {
    if let Some(sample) = output.iter().position(|value| !value.is_finite()) {
        return Err(RowSampleError::new(sample, "a floating-point sample is not finite"));
    }
    let mut changes = SampleChanges::default();
    let mut normalize = |value: &mut f32| {
        if *value < 0.0 {
            *value = 0.0;
            changes.coverage_clamped = true;
        } else if *value > 1.0 {
            *value = 1.0;
            changes.coverage_clamped = true;
        }
    };
    if coverage {
        output.iter_mut().for_each(&mut normalize);
    } else if matches!(channels, 2 | 4) {
        output.iter_mut().skip(channels - 1).step_by(channels).for_each(&mut normalize);
    }
    Ok(changes)
}

/// F64 coverage is classified before narrowing: a tiny negative or an overshoot
/// that rounds into [0,1] still needs a clamp notice. Color retains signed HDR.
fn double_row(raw: &[u8], output: &mut [f32], little: bool, channels: usize, coverage: bool) -> Result<SampleChanges, RowSampleError> {
    let mut changes = SampleChanges::default();
    for (sample, (out, bytes)) in output.iter_mut().zip(raw.as_chunks::<8>().0).enumerate() {
        let source = if little { f64::from_le_bytes(*bytes) } else { f64::from_be_bytes(*bytes) };
        if !source.is_finite() {
            return Err(RowSampleError::new(sample, "a floating-point sample is not finite"));
        }
        let is_coverage = coverage || (matches!(channels, 2 | 4) && sample % channels == channels - 1);
        let clamped = is_coverage && !(0.0..=1.0).contains(&source);
        let source = if clamped { source.clamp(0.0, 1.0) } else { source };
        if source.abs() > f64::from(f32::MAX) {
            return Err(RowSampleError::new(sample, "a floating-point sample exceeds native F32 range"));
        }
        let native = source as f32;
        if !native.is_finite() {
            return Err(RowSampleError::new(sample, "a floating-point sample exceeds native F32 range"));
        }
        changes.underflow |= source != 0.0 && native == 0.0;
        changes.coverage_clamped |= clamped;
        *out = native;
    }
    Ok(changes)
}

#[cfg(test)]
mod tests;
