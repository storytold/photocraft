//! Portable stored color literals for native text and vectors.
use super::ColorLiteral;
use craft_cms::{Builtin, ColorSpace, Curve, Intent, Profile, Transform};

#[cfg(test)]
mod review_tests;

/// A transfer rewrite that cannot preserve its PCS must retain the saved raster,
/// rather than taking callers' ordinary explicit color-approximation path.
#[derive(Debug)]
pub(in crate::xcf) enum LiteralColorError {
    GrayLabPcsTransfer,
    Other(String),
}

impl std::fmt::Display for LiteralColorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GrayLabPcsTransfer => f.write_str("stored Gray Lab PCS requires a PCS-aware sample transfer-curve conversion"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

impl From<&str> for LiteralColorError {
    fn from(message: &str) -> Self {
        Self::Other(message.into())
    }
}

/// Decode only portable byte encodings. Native-endian source float blobs have no
/// byte-order marker; guessing from the importing machine would corrupt colors.
pub(in crate::xcf) fn literal_rgb(value: &ColorLiteral<'_>) -> Result<[f32; 4], LiteralColorError> {
    // IEEE positive zero has identical bytes in either byte order. This exact
    // transparent default is portable without guessing the author's host.
    if matches!(value.encoding, "RGBA float" | "R'G'B'A float" | "R~G~B~A float")
        && value.profile.is_empty()
        && value.pixels.len() == 16
        && value.pixels.iter().all(|byte| *byte == 0)
    {
        return Ok([0.0; 4]);
    }
    let (channels, alpha, linear, gray) = match value.encoding {
        "R'G'B'A u8" | "R~G~B~A u8" => (3, true, false, false),
        "R'G'B' u8" | "R~G~B~ u8" => (3, false, false, false),
        "RGBA u8" => (3, true, true, false),
        "RGB u8" => (3, false, true, false),
        "Y'A u8" | "Y~A u8" => (1, true, false, true),
        "Y' u8" | "Y~ u8" => (1, false, false, true),
        "YA u8" => (1, true, true, true),
        "Y u8" => (1, false, true, true),
        _ => {
            return Err(LiteralColorError::Other(format!(
                "stored color encoding {} is not independently portable; native-endian float colors need source byte-order evidence",
                value.encoding
            )));
        }
    };
    if value.pixels.len() != channels + usize::from(alpha) {
        return Err("stored color byte count does not match its encoding".into());
    }
    let fallback = if gray {
        Builtin::SGray
    } else if linear {
        Builtin::LinearSrgb
    } else {
        Builtin::Srgb
    };
    let mut source = if value.profile.is_empty() {
        fallback.profile().clone()
    } else {
        Profile::parse(value.profile).map_err(|error| LiteralColorError::Other(format!("stored literal ICC: {error}")))?
    };
    let expected = if gray { ColorSpace::Gray } else { ColorSpace::Rgb };
    if source.color_space != expected {
        return Err("stored literal profile model does not match its encoding".into());
    }
    // Encoding TRCs are independent of assigned colorants, exactly as for source pixels.
    if linear || value.encoding.contains('~') {
        if !source.is_matrix_shaper() || source.b2a.iter().any(Option::is_some) {
            return Err("stored literal ICC has no separable native encoding representation".into());
        }
        let curve = if linear { Curve::Identity } else { craft_cms::curve::srgb_trc() };
        if gray {
            // Gray Lab PCS curves yield L*/100, not luminance. An Identity or
            // sRGB luminance curve would silently change the literal's meaning.
            if source.pcs == craft_cms::Pcs::Lab {
                return Err(LiteralColorError::GrayLabPcsTransfer);
            }
            source.gray_trc = Some(curve);
        } else {
            source.trc = Some([curve.clone(), curve.clone(), curve]);
        }
    }
    let transform = Transform::new(&source, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false)
        .map_err(|error| LiteralColorError::Other(format!("stored literal native color conversion: {error}")))?;
    let samples: Vec<_> = value.pixels.iter().take(channels).map(|v| f32::from(*v) / 255.0).collect();
    let mut rgb = [0.0; 3];
    transform
        .convert_f32(&samples, channels, &mut rgb, 3, false)
        .map_err(|error| LiteralColorError::Other(format!("stored literal native color conversion: {error}")))?;
    if rgb.iter().any(|v| !v.is_finite()) {
        return Err("stored literal conversion is nonfinite".into());
    }
    let [r, g, b] = rgb;
    let a = if alpha { f32::from(*value.pixels.get(channels).ok_or("stored alpha is missing")?) / 255.0 } else { 1.0 };
    Ok([r, g, b, a])
}
