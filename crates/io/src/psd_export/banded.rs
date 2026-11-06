//! Layers too large to gather at once, saved band by band (#5333).
//!
//! A layer record is one dense rectangle. Past [`BANDED_FROM`] its required channel planes are
//! gathered directly from tiles and encoded a band of rows at a time. RLE rows pack independently
//! (the row-count table is kept and written ahead of
//! the packed rows) and raw rows join, so those channels are the bytes the whole-layer encoder
//! writes. ZIP with prediction (32-bit) predicts per row and feeds one zlib stream: the same
//! pixels, but not always the same bytes, as zlib packs input it is given in parts differently.
//! With a stage each band's parts go to the scratch disk as they are made (see [`Parts`]).
//!
//! A banded layer is minutes of work on a large save, so it checks the save's interrupt before
//! each band and each channel's part of it: once cancelled it stops there (and doesn't retry
//! raw), leaving an incomplete model that the caller discards before the save's commit point.

use std::io::Write as _;

use photocraft_psd::compression::{PlaneLayout, encode_planes, predict};
use photocraft_psd::{ChannelData, Compression, PsdError};
use photocraft_raster::Surface;

use super::Ex;
use super::stage::Parts;
use crate::pixels::{psd_depth, surface_planes};

/// Dense source bytes of one layer from which its planes are gathered band by band.
pub(super) const BANDED_FROM: u64 = if cfg!(target_pointer_width = "64") { 1 << 30 } else { 256 << 20 };
/// Source-byte budget determining each band's height, retained for ZIP byte compatibility.
const BAND_BYTES: usize = 64 << 20;

#[cfg(test)]
thread_local! {
    /// `(BANDED_FROM, BAND_BYTES)` for this thread's exports, so tests band small layers.
    pub(crate) static BANDS_FOR_TESTS: std::cell::Cell<Option<(u64, usize)>> = const { std::cell::Cell::new(None) };
    /// Bands this thread has gathered and encoded.
    pub(crate) static BANDS_DONE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn banded_from() -> u64 {
    #[cfg(test)]
    if let Some((from, _)) = BANDS_FOR_TESTS.get() {
        return from;
    }
    BANDED_FROM
}

fn band_bytes() -> usize {
    #[cfg(test)]
    if let Some((_, bytes)) = BANDS_FOR_TESTS.get() {
        return bytes;
    }
    BAND_BYTES
}

/// One channel's encoding state across bands.
struct Channel {
    id: i16,
    table: Vec<u8>,
    zip: Option<flate2::write::ZlibEncoder<Vec<u8>>>,
    parts: Parts,
}

impl Ex<'_> {
    /// The channels of `s` over nonempty `r`, encoded band by band with optional alpha first.
    pub(super) fn banded_channels(&self, s: &Surface, r: photocraft_geom::Rect, include_alpha: bool) -> Vec<ChannelData> {
        // RLE fails only on a row too long for its count (not within PSD/PSB limits); then the
        // layer is written raw, as a channel encoded whole falls back to.
        match self.banded_pass(s, r, self.compression(), include_alpha) {
            Ok(channels) => channels,
            // Cancelled: the caller sees the interrupt and discards the whole model, so don't
            // encode the layer again.
            Err(_) if self.ctl.cancelled() => self.empty_channels(),
            Err(_) => self.banded_pass(s, r, Compression::Raw, include_alpha).unwrap_or_else(|_| self.empty_channels()),
        }
    }

    fn banded_pass(&self, s: &Surface, r: photocraft_geom::Rect, compression: Compression, include_alpha: bool) -> photocraft_psd::Result<Vec<ChannelData>> {
        let (w, h) = (r.width() as usize, r.height() as usize);
        let bpp = s.format().bytes_per_pixel();
        // Keep the original band heights: changing ZIP input boundaries can change its bytes.
        let band_rows = (band_bytes() / (w * bpp).max(1)).max(1);
        let depth = psd_depth(self.fmt.sample);
        let count = if self.version.is_psb() { 4 } else { 2 };
        let head = if compression == Compression::Rle { h.checked_mul(count).ok_or(PsdError::LimitExceeded("row count overflow"))? } else { 0 };
        let mut channels: Vec<Channel> = (0..=self.cc)
            .skip(usize::from(!include_alpha))
            .map(|k| Channel {
                id: k as i16 - 1,
                table: Vec::with_capacity(head),
                zip: (compression == Compression::ZipPrediction).then(|| flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default())),
                parts: Parts::new(self.stage.clone(), head),
            })
            .collect();
        let mut invert = vec![self.cmyk; self.cc];
        invert.push(false);
        let ctl = self.ctl;
        let check = || if ctl.cancelled() { Err(PsdError::Invalid("cancelled".into())) } else { Ok(()) };
        for y in (0..h).step_by(band_rows) {
            check()?;
            let rows = band_rows.min(h - y);
            let band = photocraft_geom::Rect::new(r.x0, r.y0.saturating_add_unsigned(y as u32), r.x1, r.y0.saturating_add_unsigned((y + rows) as u32));
            let lab16 = self.fmt.mode == photocraft_color::ColorMode::Lab && self.fmt.sample == photocraft_color::SampleType::U16;
            let mut planes: Vec<_> = surface_planes(s, band, self.cc + usize::from(include_alpha), &invert, lab16).into_iter().map(Some).collect();
            let layout = PlaneLayout { planes: 1, width: w, height: rows, depth, version: self.version };
            // Alpha (planes[cc]), when needed, precedes the colour channels.
            let work: Vec<(&mut Channel, Vec<u8>)> = channels
                .iter_mut()
                .map(|c| {
                    let k = if c.id == -1 { self.cc } else { c.id as usize };
                    (c, planes[k].take().unwrap_or_default())
                })
                .collect();
            let done = crate::pixels::par_map(work, |(c, mut plane)| -> photocraft_psd::Result<()> {
                check()?;
                match compression {
                    Compression::Rle => {
                        let mut packed = encode_planes(Compression::Rle, &plane, &layout)?;
                        let rest = packed.split_off(rows * count);
                        c.table.extend_from_slice(&packed);
                        c.parts.push(rest);
                    }
                    Compression::ZipPrediction => {
                        predict(&mut plane, &layout)?;
                        let zip = c.zip.as_mut().ok_or_else(|| PsdError::Invalid("banded ZIP channel without its stream".into()))?;
                        zip.write_all(&plane).map_err(|e| PsdError::Invalid(format!("zlib: {e}")))?;
                        let out = std::mem::take(zip.get_mut());
                        c.parts.push(out);
                    }
                    _ => {
                        c.parts.push(plane);
                        return Ok(());
                    }
                }
                photocraft_raster::scratch::give(plane);
                Ok(())
            });
            done.into_iter().collect::<photocraft_psd::Result<()>>()?;
            #[cfg(test)]
            BANDS_DONE.set(BANDS_DONE.get() + 1);
        }
        channels
            .into_iter()
            .map(|mut c| {
                if let Some(zip) = c.zip.take() {
                    c.parts.push(zip.finish().map_err(|e| PsdError::Invalid(format!("zlib: {e}")))?);
                }
                c.parts.finish(c.id, compression, &c.table)
            })
            .collect()
    }
}
