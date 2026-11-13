//! Targa writer: craft-libs' `craft-tga` (run-length packets that end with their scanline, rows
//! bottom-up, alpha bits in the descriptor, the TGA 2.0 footer, #6382). The `image` crate still
//! reads Targa ([`super::via_image`]).

use crate::Format;
use crate::error::CodecError;
use crate::image::{ChannelLayout, Image, SampleType};

/// Encodes an 8-bit Gray, Gray+alpha, RGB or RGBA image as a run-length encoded Targa.
pub(crate) fn encode(img: &Image) -> Result<Vec<u8>, CodecError> {
    if img.sample_type() != SampleType::U8 {
        return Err(CodecError::encode(Format::Tga, format!("{:?} samples (Targa stores 8 bits)", img.sample_type())));
    }
    let layout = match img.layout() {
        ChannelLayout::Gray => craft_tga::Layout::Gray,
        ChannelLayout::GrayA => craft_tga::Layout::GrayAlpha,
        ChannelLayout::Rgb => craft_tga::Layout::Rgb,
        ChannelLayout::Rgba => craft_tga::Layout::Rgba,
        l => return Err(CodecError::encode(Format::Tga, format!("no Targa layout for {l:?}"))),
    };
    let (w, h) = img.dimensions();
    craft_tga::encode(w, h, layout, img.data()).map_err(|e| match e {
        craft_tga::Error::Encode(m) if m == "buffer size mismatch" => CodecError::InvalidImage(m),
        e => CodecError::encode(Format::Tga, e),
    })
}
