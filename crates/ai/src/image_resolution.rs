//! Size rules for the Images API, independent of native document dimensions.
use crate::{AiError, ImageSize, Result};

pub const OPENAI_MAX_PIXELS: u64 = 8_294_400;

pub fn openai_size(model: &str, requested: ImageSize) -> Result<ImageSize> {
    requested.pixels()?;
    if requested.width > requested.height.saturating_mul(3) || requested.height > requested.width.saturating_mul(3) {
        return Err(AiError::Unsupported("OpenAI Images supports aspect ratios between 1:3 and 3:1".into()));
    }
    let legacy = matches!(model, "gpt-image-1" | "gpt-image-1-mini" | "gpt-image-1.5");
    if legacy {
        if requested.width != requested.height {
            return Err(AiError::Unsupported("this legacy image profile supports square destinations; select GPT Image 2/2.5 for custom dimensions".into()));
        }
        return Ok(ImageSize { width: 1024, height: 1024 });
    }
    if requested.width == requested.height && requested.width <= 1024 {
        return Ok(ImageSize { width: 1024, height: 1024 });
    }
    let pixels = u64::from(requested.width) * u64::from(requested.height);
    let scale = (3840.0 / f64::from(requested.width.max(requested.height))).min((OPENAI_MAX_PIXELS as f64 / pixels as f64).sqrt()).min(1.0);
    let scale = if pixels < 655_360 { (700_000.0 / pixels as f64).sqrt() } else { scale };
    let quantize = |v: u32| (((f64::from(v) * scale).floor() as u32 / 16) * 16).clamp(16, 3840);
    let mut size = ImageSize { width: quantize(requested.width), height: quantize(requested.height) };
    // Rounding must not put an exact 3:1 ratio outside the documented bounds.
    if size.width > size.height * 3 {
        size.width = size.height * 3;
    }
    if size.height > size.width * 3 {
        size.height = size.width * 3;
    }
    if u64::from(size.width) * u64::from(size.height) < 655_360 {
        return Err(AiError::Unsupported("could not map this image format to a supported OpenAI size".into()));
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn four_k_and_portrait_are_native_and_eight_k_maps_to_four_k() {
        for (w, h) in [(3840, 2160), (2160, 3840), (2880, 2880)] {
            let size = ImageSize { width: w, height: h };
            assert_eq!(openai_size("gpt-image-2.5-sunburst", size).unwrap(), size);
        }
        let tiny = openai_size("gpt-image-2", ImageSize { width: 64, height: 32 }).unwrap();
        assert!(tiny.width > tiny.height);
        assert!(u64::from(tiny.width) * u64::from(tiny.height) >= 655_360);
        assert!(openai_size("gpt-image-2", ImageSize { width: 4096, height: 64 }).is_err());
        assert!(openai_size("gpt-image-1", ImageSize { width: 1536, height: 1024 }).is_err());
    }
}
