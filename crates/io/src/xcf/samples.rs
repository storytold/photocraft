//! Versioned sample precision and bounded native numeric conversion.

use super::{Result, SampleType, f16, invalid};

/// How samples are stored: bytes per sample, whether they are floats, and whether the values are
/// linear light, the assigned profile's own curve, or the perceptual sRGB transfer curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Precision {
    pub(super) bytes: usize,
    pub(super) float: bool,
    pub(super) linear: bool,
    pub(super) perceptual: bool,
}

impl Precision {
    pub(super) const GAMMA8: Precision = Precision { bytes: 1, float: false, linear: false, perceptual: false };

    /// Interprets versioned precision codes into sample width, transfer encoding and native depth, rejecting
    /// unknown combinations.
    pub(super) fn from_header(version: u32, code: u32) -> Result<Precision> {
        let p = |bytes, float, linear| Ok(Precision { bytes, float, linear, perceptual: false });
        match version {
            0..=3 => Ok(Self::GAMMA8),
            4 => match code {
                0 => p(1, false, false),
                1 => p(2, false, false),
                2 => p(4, false, true),
                3 => p(2, true, true),
                4 => p(4, true, true),
                c => Err(invalid(format!("unknown precision {c} in an XCF version 4 file"))),
            },
            5 | 6 => match code {
                100 => p(1, false, true),
                150 => p(1, false, false),
                200 => p(2, false, true),
                250 => p(2, false, false),
                300 => p(4, false, true),
                350 => p(4, false, false),
                400 => p(2, true, true),
                450 => p(2, true, false),
                500 => p(4, true, true),
                550 => p(4, true, false),
                c => Err(invalid(format!("unknown precision {c} in an XCF version {version} file"))),
            },
            _ if code % 100 == 75 => {
                let mut precision = Self::from_header(version, code - 25)?;
                precision.perceptual = true;
                Ok(precision)
            }
            _ => match code {
                100 => p(1, false, true),
                150 => p(1, false, false),
                200 => p(2, false, true),
                250 => p(2, false, false),
                300 => p(4, false, true),
                350 => p(4, false, false),
                500 => p(2, true, true),
                550 => p(2, true, false),
                600 => p(4, true, true),
                650 => p(4, true, false),
                700 => p(8, true, true),
                750 => p(8, true, false),
                c => Err(invalid(format!("unknown precision {c}"))),
            },
        }
    }

    /// The document depth the samples open at.
    pub(super) fn depth(self) -> SampleType {
        match (self.float, self.bytes) {
            (false, 1) => SampleType::U8,
            (false, 2) => SampleType::U16,
            _ => SampleType::F32,
        }
    }

    /// Reports source storage wider than the chosen native representation so numerical-loss notices stay
    /// scoped correctly.
    pub(super) fn reduces_precision(self) -> bool {
        matches!((self.float, self.bytes), (false, 4) | (true, 8))
    }

    /// Test/reference entry point retaining the underflow-only tuple contract.
    #[cfg(test)]
    pub(super) fn sample(self, b: &[u8], coverage: bool) -> Result<(f32, bool)> {
        let (value, underflow, _) = self.sample_with_conversion(b, coverage)?;
        Ok((value, underflow))
    }

    /// Normalize finite coverage independently of signed HDR color and record the loss.
    pub(super) fn sample_with_conversion(self, b: &[u8], coverage: bool) -> Result<(f32, bool, bool)> {
        let source = match (self.float, self.bytes, b) {
            (false, 1, [v]) => f64::from(*v) / 255.0,
            (false, 2, [a, b]) => f64::from(u16::from_be_bytes([*a, *b])) / 65535.0,
            (false, 4, [a, b, c, d]) => f64::from(u32::from_be_bytes([*a, *b, *c, *d])) / f64::from(u32::MAX),
            (true, 2, [a, b]) => f64::from(f16::from_bits(u16::from_be_bytes([*a, *b])).to_f32()),
            (true, 4, [a, b, c, d]) => f64::from(f32::from_be_bytes([*a, *b, *c, *d])),
            (true, 8, [a, b, c, d, e, f, g, h]) => f64::from_be_bytes([*a, *b, *c, *d, *e, *f, *g, *h]),
            _ => return Err(invalid("sample size does not match the image precision")),
        };
        if !source.is_finite() {
            return Err(invalid("a floating-point sample is not finite"));
        }
        let coverage_clamped = coverage && !(0.0..=1.0).contains(&source);
        let source = if coverage { source.clamp(0.0, 1.0) } else { source };
        if source.abs() > f64::from(f32::MAX) {
            return Err(invalid("a floating-point sample exceeds native F32 range"));
        }
        let native = source as f32;
        if !native.is_finite() {
            return Err(invalid("a floating-point sample exceeds native F32 range"));
        }
        Ok((native, source != 0.0 && native == 0.0, coverage_clamped))
    }
}

/// U8/U16 samples already have finite normalized color and coverage domains.
/// Keep their row conversion separate from validation/narrowing of floats and U32.
#[inline]
pub(super) fn integer_samples(raw: &[u8], output: &mut [f32], sample_bytes: usize) -> Result<()> {
    match sample_bytes {
        1 if raw.len() == output.len() => {
            for (out, value) in output.iter_mut().zip(raw) {
                *out = f32::from(*value) / 255.0;
            }
        }
        2 if output.len().checked_mul(2) == Some(raw.len()) => {
            for (out, value) in output.iter_mut().zip(raw.as_chunks::<2>().0) {
                *out = f32::from(u16::from_be_bytes(*value)) / 65535.0;
            }
        }
        _ => return Err(invalid("integer row size does not match the image precision")),
    }
    Ok(())
}
