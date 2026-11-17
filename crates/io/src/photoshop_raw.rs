//! Photoshop Raw (`*.RAW`, File › Open As): any file read as nothing but pixel bytes, laid out
//! as the Raw Options dialog describes them. Nothing in the file says how wide it is or how many
//! channels it holds, so the reader never sniffs: the caller passes a [`RawLayout`].
//!
//! Measured so far (#14051): one channel at 8 bits opens as a Grayscale 8-bit document with one
//! Background layer, each byte after the header one pixel's gray value, row by row. Layouts whose
//! result was not measured (several channels, 16 or 32 bits) and sizes that don't match the file
//! are refused with an error rather than guessed at.

use photocraft_codecs::{ChannelLayout, Image};

use crate::{ImportResult, IoError};

/// The longest side Photoshop opens (a Large Document Format's limit): a wider or taller layout
/// is refused before anything is read.
pub const MAX_SIDE: u32 = 300_000;

/// The most channels Photoshop's Raw Options accepts.
pub const MAX_CHANNELS: u32 = 56;

/// How the bytes of a 16- or 32-bit sample are ordered: Raw Options' Mac (most significant byte
/// first) or IBM PC (least significant first). It means nothing at 8 bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ByteOrder {
    /// Big-endian.
    #[default]
    Mac,
    /// Little-endian.
    IbmPc,
}

/// How a Photoshop Raw file's bytes make pixels: the fields of the Raw Options dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawLayout {
    /// Pixels per row.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// Channels per pixel (Raw Options' Count).
    pub channels: u32,
    /// Bits per sample: 8, 16 or 32.
    pub depth: u32,
    /// Byte order of 16- and 32-bit samples.
    pub byte_order: ByteOrder,
    /// Each pixel's channels next to each other, rather than one whole plane per channel.
    pub interleaved: bool,
    /// Bytes skipped at the start of the file before the first pixel.
    pub header: u64,
}

impl RawLayout {
    /// Raw Options' starting values for a file of `len` bytes: one interleaved channel at 8 bits,
    /// no header, and a square [`guess_size`] when the bytes make one (16 bytes: 4 × 4, as
    /// measured). Any other size starts at 0 × 0 for the user to type: what the reference
    /// application guesses for it has not been measured.
    pub fn defaults_for(len: u64) -> Self {
        let (width, height) = guess_size(len).unwrap_or((0, 0));
        RawLayout { width, height, channels: 1, depth: 8, byte_order: ByteOrder::default(), interleaved: true, header: 0 }
    }

    /// The bytes the pixels take after the header, `None` if that overflows `u64`.
    pub fn pixel_bytes(&self) -> Option<u64> {
        u64::from(self.width).checked_mul(u64::from(self.height))?.checked_mul(u64::from(self.channels))?.checked_mul(u64::from(self.depth / 8))
    }

    /// Whether this layout can be read from a file of `len` bytes, with the reason when it can't:
    /// a side of 0 or over [`MAX_SIDE`], a channel count of 0 or over [`MAX_CHANNELS`], a depth
    /// other than 8, 16 or 32, a header longer than the file, or pixels that don't take exactly
    /// the bytes after the header. Layouts the reader doesn't open yet are refused as well.
    pub fn check(&self, len: u64) -> Result<(), IoError> {
        let bad = |msg: String| Err(IoError::Read(msg));
        if !(1..=MAX_SIDE).contains(&self.width) || !(1..=MAX_SIDE).contains(&self.height) {
            return bad(format!("width and height must be 1 to {MAX_SIDE} pixels, not {} × {}", self.width, self.height));
        }
        if !(1..=MAX_CHANNELS).contains(&self.channels) {
            return bad(format!("the channel count must be 1 to {MAX_CHANNELS}, not {}", self.channels));
        }
        if !matches!(self.depth, 8 | 16 | 32) {
            return bad(format!("the depth must be 8, 16 or 32 bits, not {}", self.depth));
        }
        if self.header > len {
            return bad(format!("the {}-byte header is longer than the file ({len} bytes)", self.header));
        }
        let available = len - self.header;
        let needed = self.pixel_bytes().filter(|n| *n == available);
        if needed.is_none() {
            let needed = self.pixel_bytes().map_or_else(|| "more than 2^64".to_string(), |n| n.to_string());
            return bad(format!(
                "{} × {} pixels of {} channel(s) at {} bits take {needed} bytes, but the file has {available} after its {}-byte header",
                self.width, self.height, self.channels, self.depth, self.header
            ));
        }
        if self.channels != 1 || self.depth != 8 {
            let msg = format!("only 1 channel at 8 bits can be read so far, not {} channel(s) at {} bits", self.channels, self.depth);
            return Err(IoError::Unsupported(msg));
        }
        Ok(())
    }
}

/// The square a file of `len` pixel bytes makes, when it makes one within [`MAX_SIDE`]
/// (16 bytes: 4 × 4). `None` for 0 bytes and for any length that isn't a perfect square.
pub fn guess_size(len: u64) -> Option<(u32, u32)> {
    let side = len.isqrt();
    let side32 = u32::try_from(side).ok().filter(|s| (1..=MAX_SIDE).contains(s))?;
    (side * side == len).then_some((side32, side32))
}

/// The document a Photoshop Raw file named `name` makes, read as `layout` says (see
/// [`RawLayout::check`] for what is refused). The source is import-only: there is no Photoshop
/// Raw writer, so Save never writes the document back over the file it came from. As for every
/// other open, a scratch-disk failure while the tiles are built is an error, not a document.
pub fn import(name: &str, bytes: &[u8], layout: &RawLayout) -> Result<ImportResult, IoError> {
    let scratch_generation = photocraft_raster::spill::read_error_generation();
    layout.check(bytes.len() as u64)?;
    // `check` proved the header fits and the rest is exactly the pixels.
    let pixels = &bytes[layout.header as usize..];
    // The copy is as large as the file: a machine without that much memory gets an error, not an abort.
    let mut owned = crate::flat::try_buffer(pixels.len(), 1)?;
    owned.extend_from_slice(pixels);
    let img = Image::from_u8(layout.width, layout.height, ChannelLayout::Gray, owned)?;
    let mut r = crate::flat::image_to_document(name, &img)?;
    photocraft_raster::spill::check_since(scratch_generation).map_err(IoError::Unsupported)?;
    r.document.scratch_health().map_err(IoError::Unsupported)?;
    r.source_read_only = true;
    Ok(r)
}
