//! Image validation state shared by the outer parser and all embedded SVG resolvers.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use super::{IoError, MAX_BOUNDED_SVG_BYTES, MAX_INFLATED_BYTES, MAX_RASTER_PIXELS, MAX_XML_DEPTH, data_href_validation, raster_validation, tree_limits, usvg};

#[cfg(test)]
#[path = "integrity_budget_tests.rs"]
mod integrity_budget_tests;

struct RasterRecord {
    // Retain successful and failed sources: parser-discarded image allocations
    // must not recycle an address and bypass the validation-work allowance.
    _encoded: Arc<Vec<u8>>,
    result: Result<(), raster_validation::Failure>,
}

pub(super) struct Integrity {
    pixel_limit: u64,
    pixels_left: u64,
    svg_bytes_left: usize,
    raster: HashMap<usize, RasterRecord>,
    encoded_left: usize,
    invalid_raster: usize,
    invalid_svg: usize,
    raster_limits: usize,
    svg_limits: usize,
    fatal: bool,
}

impl Integrity {
    pub(super) fn new(pixels: u64) -> Self {
        Self {
            pixel_limit: pixels.min(MAX_RASTER_PIXELS),
            pixels_left: pixels.min(MAX_RASTER_PIXELS),
            svg_bytes_left: MAX_BOUNDED_SVG_BYTES as usize,
            raster: HashMap::new(),
            encoded_left: (MAX_INFLATED_BYTES + MAX_BOUNDED_SVG_BYTES) as usize,
            invalid_raster: 0,
            invalid_svg: 0,
            raster_limits: 0,
            svg_limits: 0,
            fatal: false,
        }
    }

    pub(super) fn usage(&self, mut usage: craft_svg::tree::ImageUsage) -> Result<craft_svg::tree::ImageUsage, IoError> {
        if self.fatal {
            return Err(IoError::Svg("embedded image validation metadata exceeds its bounded allowance".into()));
        }
        // Raster callbacks use the exact rendering backend here, rather than decoding
        // once generically in the shared resolver and then again for validation.
        usage.raster_pixels = usage.raster_pixels.max(self.pixel_limit.saturating_sub(self.pixels_left));
        usage.invalid_raster_images = usage.invalid_raster_images.saturating_add(self.invalid_raster);
        usage.invalid_svg_images = usage.invalid_svg_images.saturating_add(self.invalid_svg);
        usage.raster_limit_images = usage.raster_limit_images.saturating_add(self.raster_limits);
        usage.svg_limit_images = usage.svg_limit_images.saturating_add(self.svg_limits);
        usage.svg_resource_limit_images = usage.svg_resource_limit_images.saturating_add(self.svg_limits);
        // Repairing a malformed href can shorten text. Its original expanded UTF-8
        // bytes remain charged, including XML declarations removed for transport.
        usage.embedded_svg_bytes = usage.embedded_svg_bytes.max((MAX_BOUNDED_SVG_BYTES as usize).saturating_sub(self.svg_bytes_left));
        Ok(usage)
    }

    fn raster(&mut self, kind: &usvg::ImageKind) -> bool {
        if self.fatal {
            return false;
        }
        let encoded = match kind {
            usvg::ImageKind::SVG(_) => return true,
            usvg::ImageKind::PNG(data) | usvg::ImageKind::JPEG(data) | usvg::ImageKind::GIF(data) | usvg::ImageKind::WEBP(data) => data,
        };
        let key = Arc::as_ptr(encoded).addr();
        let result = if let Some(record) = self.raster.get(&key) {
            record.result
        } else {
            // Bound auxiliary records independently of a hostile count of tiny images.
            let Some(left) = self.encoded_left.checked_sub(encoded.len()) else {
                self.fatal = true;
                return false;
            };
            if self.raster.len() >= 100_000 || self.raster.try_reserve(1).is_err() {
                self.fatal = true;
                return false;
            }
            self.encoded_left = left;
            let result = raster_validation::validate(kind, &mut self.pixels_left);
            self.raster.insert(key, RasterRecord { _encoded: Arc::clone(encoded), result });
            result
        };
        match result {
            Ok(()) => true,
            Err(raster_validation::Failure::Invalid) => {
                self.invalid_raster = self.invalid_raster.saturating_add(1);
                false
            }
            Err(raster_validation::Failure::Limit) => {
                self.raster_limits = self.raster_limits.saturating_add(1);
                false
            }
        }
    }
}

pub(super) fn wrap(base: usvg::ImageHrefResolver<'static>, state: Arc<Mutex<Integrity>>) -> usvg::ImageHrefResolver<'static> {
    usvg::ImageHrefResolver {
        resolve_string: base.resolve_string,
        resolve_data: Box::new(move |mime, data, options| {
            if state.lock().unwrap_or_else(PoisonError::into_inner).fatal {
                return None;
            }
            if let Some(kind) = raster_kind(mime, &data) {
                let valid = state.lock().unwrap_or_else(PoisonError::into_inner).raster(&kind);
                return valid.then_some(kind);
            }
            let svg = matches!(mime, "image/svg+xml" | "text/plain");
            if !svg {
                // Count failed declared images, while preserving the existing policy:
                // non-image media types are ignored before decoding or pixel admission.
                if mime.starts_with("image/") {
                    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                    state.invalid_raster = state.invalid_raster.saturating_add(1);
                }
                return None;
            }
            let data = if svg {
                let room = state.lock().unwrap_or_else(PoisonError::into_inner).svg_bytes_left;
                // Inflate once under the original aggregate allowance, then pass UTF-8
                // text onward. Nested parses retain these exact same wrapped options.
                let text = match craft_svg::svgz::text(&data, room) {
                    Ok(text) if text.len() <= room => text,
                    error => {
                        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                        // Failed inflation/encoding already spent work. Match the shared
                        // resolver: none of its remaining allowance can be retried.
                        state.svg_bytes_left = 0;
                        if matches!(error, Err(craft_svg::svgz::Error::TooLarge(_))) || error.is_ok() {
                            state.svg_limits = state.svg_limits.saturating_add(1);
                        } else {
                            state.invalid_svg = state.invalid_svg.saturating_add(1);
                        }
                        return None;
                    }
                };
                state.lock().unwrap_or_else(PoisonError::into_inner).svg_bytes_left = room.saturating_sub(text.len());
                let repaired = match data_href_validation::repair(&text, &tree_limits(MAX_XML_DEPTH, MAX_RASTER_PIXELS)) {
                    Ok(text) => text,
                    Err(_) => {
                        let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                        state.invalid_svg = state.invalid_svg.saturating_add(1);
                        return None;
                    }
                };
                // The declaration was already validated against original UTF-8 text.
                // Removing it prevents a Latin-1 declaration from decoding UTF-8 twice.
                let normalized = if repaired.strip_prefix("<?xml").is_some_and(|rest| rest.starts_with([' ', '\t', '\r', '\n'])) {
                    repaired.find("?>").and_then(|at| repaired.get(at + 2..)).unwrap_or(&repaired)
                } else {
                    &repaired
                };
                let mut bytes = Vec::new();
                if bytes.try_reserve_exact(normalized.len()).is_err() {
                    state.lock().unwrap_or_else(PoisonError::into_inner).fatal = true;
                    return None;
                }
                bytes.extend_from_slice(normalized.as_bytes());
                Arc::new(bytes)
            } else {
                data
            };
            // Keep shared SVG nesting/XML accounting. Its nested parsers reuse this
            // wrapper, so every raster still follows our one exact, bounded decode.
            (base.resolve_data)(mime, data, options)
        }),
    }
}

// Preserve the shared resolver's MIME/magic agreement. A forged MIME must not
// borrow a successful validation belonging to a different rendering decoder.
fn raster_kind(mime: &str, data: &Arc<Vec<u8>>) -> Option<usvg::ImageKind> {
    use photocraft_codecs::Format;
    if !matches!(mime, "image/jpg" | "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "text/plain") {
        return None;
    }
    let format = photocraft_codecs::detect(data)?;
    let accepted = match mime {
        "image/jpg" | "image/jpeg" => format == Format::Jpeg,
        "image/png" => format == Format::Png,
        "image/gif" => format == Format::Gif,
        "image/webp" => format == Format::WebP,
        "text/plain" => true,
        _ => false,
    };
    if !accepted {
        return None;
    }
    Some(match format {
        Format::Png => usvg::ImageKind::PNG(Arc::clone(data)),
        Format::Jpeg => usvg::ImageKind::JPEG(Arc::clone(data)),
        Format::Gif => usvg::ImageKind::GIF(Arc::clone(data)),
        Format::WebP => usvg::ImageKind::WEBP(Arc::clone(data)),
        _ => return None,
    })
}
