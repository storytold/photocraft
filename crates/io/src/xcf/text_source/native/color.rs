//! Convert admitted source literals through their actual profiles. A native-endian
//! float blob is not evidence of its author's byte order and remains unsupported.

use craft_cms::{Builtin, ColorSpace, Curve, Intent, Profile, Transform, TransformOptions};
use craft_xcf::records::text::Value;
use photocraft_color::{Color, ColorMode};

use super::{Result, field_error, unsupported};

pub(in crate::xcf) struct NativeColors {
    mode: ColorMode,
    transform: std::sync::Arc<Transform>,
}

impl NativeColors {
    /// Construct once per import from the internal encoding profile, even when
    /// missing-profile policy deliberately leaves the document untagged.
    pub(in crate::xcf) fn new(bytes: &[u8], mode: ColorMode) -> Result<Self> {
        let profile = Profile::parse(bytes).map_err(|_| unsupported("the native text destination profile is invalid"))?;
        if !matches!(
            (mode, profile.color_space),
            (ColorMode::Rgb, ColorSpace::Rgb)
                | (ColorMode::Grayscale, ColorSpace::Gray)
                | (ColorMode::Cmyk, ColorSpace::Cmyk)
                | (ColorMode::Lab, ColorSpace::Lab)
        ) {
            return Err(unsupported("the native text profile and document color model differ"));
        }
        let transform = craft_cms::transform::cached(
            Builtin::Srgb.profile(),
            &profile,
            TransformOptions { intent: Intent::RelativeColorimetric, bpc: false, precise_float: true },
        )
        .map_err(|_| unsupported("the source profile cannot encode editable text colors"))?;
        Ok(Self { mode, transform })
    }

    pub(super) fn convert(&self, color: Color) -> Result<Color> {
        let mut converted = [0.; 4];
        let channels = self.mode.color_channels();
        let output = converted.get_mut(..channels).ok_or_else(|| unsupported("too many native text color channels"))?;
        self.transform.convert_f32(&color.c[..3], 3, output, channels, false).map_err(|_| unsupported("the source text color conversion failed"))?;
        if converted.iter().any(|sample| !sample.is_finite()) {
            return Err(unsupported("the source text color conversion is nonfinite"));
        }
        Ok(Color { mode: self.mode, c: converted, alpha: color.alpha })
    }
}

pub(super) fn source_color(values: &[Value<'_>]) -> Result<Color> {
    let [value] = values else { return Err(unsupported("unmapped source text color fields")) };
    let fields = value.list().map_err(field_error)?;
    let kind = fields.first().ok_or_else(|| unsupported("missing source text color kind"))?.text().map_err(field_error)?;
    // Legacy numeric GimpRGB values are portable source scalars, unlike the
    // independently native-endian GEGL float blobs below. RGB implies opacity.
    if (kind == "color-rgb" && fields.len() == 4) || (kind == "color-rgba" && fields.len() == 5) {
        let mut rgba = [0., 0., 0., 1.];
        for (sample, value) in rgba.iter_mut().zip(fields.iter().skip(1)) {
            *sample = value.text().map_err(field_error)?.parse::<f32>().map_err(|_| unsupported("invalid source text color sample"))?;
            if !sample.is_finite() || !(0.0..=1.0).contains(sample) {
                return Err(unsupported("source text color sample is outside 0..1"));
            }
        }
        return Ok(Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3]));
    }
    if kind != "color" || !(5..=6).contains(&fields.len()) {
        return Err(unsupported("unmapped source text color encoding"));
    }
    let encoding = fields.get(1).ok_or_else(|| unsupported("missing source color encoding"))?.text().map_err(field_error)?;
    let pixels = fields.get(3).ok_or_else(|| unsupported("missing source color pixels"))?.bytes().map_err(field_error)?;
    let count = |at: usize| -> Result<usize> {
        fields
            .get(at)
            .ok_or_else(|| unsupported("missing source color size"))?
            .text()
            .map_err(field_error)?
            .parse()
            .map_err(|_| unsupported("invalid source color size"))
    };
    let profile = fields.get(5).map(Value::bytes).transpose().map_err(field_error)?.unwrap_or_default();
    if count(2)? != pixels.len() || count(4)? != profile.len() || (profile.is_empty() && fields.len() != 5) {
        return Err(unsupported("source color payload lengths differ"));
    }
    portable_literal(encoding, pixels, profile)
}

fn portable_literal(encoding: &str, pixels: &[u8], profile: &[u8]) -> Result<Color> {
    if matches!(encoding, "RGBA float" | "R'G'B'A float" | "R~G~B~A float") && profile.is_empty() && pixels.len() == 16 && pixels.iter().all(|byte| *byte == 0)
    {
        return Ok(Color::rgba(0., 0., 0., 0.));
    }
    let (channels, alpha, linear, gray) = match encoding {
        "R'G'B'A u8" | "R~G~B~A u8" => (3, true, false, false),
        "R'G'B' u8" | "R~G~B~ u8" => (3, false, false, false),
        "RGBA u8" => (3, true, true, false),
        "RGB u8" => (3, false, true, false),
        "Y'A u8" | "Y~A u8" => (1, true, false, true),
        "Y' u8" | "Y~ u8" => (1, false, false, true),
        "YA u8" => (1, true, true, true),
        "Y u8" => (1, false, true, true),
        _ => return Err(unsupported("stored text color lacks portable sample byte-order evidence")),
    };
    if pixels.len() != channels + usize::from(alpha) {
        return Err(unsupported("source color size differs from its encoding"));
    }
    let fallback = if gray {
        Builtin::SGray
    } else if linear {
        Builtin::LinearSrgb
    } else {
        Builtin::Srgb
    };
    let mut source = if profile.is_empty() {
        fallback.profile().clone()
    } else {
        Profile::parse(profile).map_err(|_| unsupported("invalid embedded text color profile"))?
    };
    if source.color_space != if gray { ColorSpace::Gray } else { ColorSpace::Rgb } {
        return Err(unsupported("source color profile model differs from its encoding"));
    }
    if linear || encoding.contains('~') {
        if !source.is_matrix_shaper() || source.b2a.iter().any(Option::is_some) {
            return Err(unsupported("source text color transfer cannot be separated from its profile"));
        }
        let curve = if linear { Curve::Identity } else { craft_cms::curve::srgb_trc() };
        if gray {
            // Gray Lab PCS carries L*, not luminance. Replacing its curve would
            // change meaning; retain the saved source raster instead.
            if source.pcs == craft_cms::Pcs::Lab {
                return Err(unsupported("Gray Lab PCS text color needs a PCS-aware transfer conversion"));
            }
            source.gray_trc = Some(curve);
        } else {
            source.trc = Some([curve.clone(), curve.clone(), curve]);
        }
    }
    let transform = Transform::new(&source, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false)
        .map_err(|_| unsupported("source text literal profile conversion is unsupported"))?;
    let mut samples = [0.; 3];
    for (out, sample) in samples.iter_mut().zip(pixels.iter().take(channels)) {
        *out = f32::from(*sample) / 255.;
    }
    let mut rgb = [0.; 3];
    transform
        .convert_f32(samples.get(..channels).ok_or_else(|| unsupported("source color channel count"))?, channels, &mut rgb, 3, false)
        .map_err(|_| unsupported("source text literal profile conversion failed"))?;
    if rgb.iter().any(|sample| !sample.is_finite()) {
        return Err(unsupported("nonfinite source text literal color"));
    }
    let alpha = if alpha { f32::from(*pixels.get(channels).ok_or_else(|| unsupported("missing text alpha"))?) / 255. } else { 1. };
    Ok(Color::rgba(rgb[0], rgb[1], rgb[2], alpha))
}
