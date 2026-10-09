use std::io::Read;

use flate2::read::GzDecoder;

use crate::error::Error;

/// Reads deferred layer chunks and returns the uncompressed raw pixel byte buffer.
pub fn read_layer_chunks(input: &mut &[u8], length: usize, max_memory: usize) -> Result<Vec<u8>, Error> {
    let format_version = read_u8(input)?;
    if format_version != 0 && format_version != 1 {
        return Err(Error::Unsupported(format!("unsupported PDN chunk formatVersion {format_version}")));
    }

    let chunk_size = read_u32_be(input)? as usize;
    if chunk_size == 0 {
        return Err(Error::Malformed("PDN chunk size is zero"));
    }

    if length > max_memory {
        return Err(Error::Limit("PDN layer memory exceeds safety limit"));
    }

    let mut buffer = vec![0u8; length];
    if length == 0 {
        return Ok(buffer);
    }

    let chunk_count = length
        .checked_add(chunk_size - 1)
        .ok_or(Error::Malformed("chunk count overflow"))?
        / chunk_size;

    if chunk_count > 100_000 {
        return Err(Error::Limit("PDN layer chunk count exceeds safety limit"));
    }

    let mut chunks_found = vec![false; chunk_count];

    for _ in 0..chunk_count {
        let chunk_number = read_u32_be(input)? as usize;
        if chunk_number >= chunk_count {
            return Err(Error::Malformed("chunk number out of bounds"));
        }
        if *chunks_found.get(chunk_number).ok_or(Error::Malformed("chunk index out of bounds"))? {
            return Err(Error::Malformed("duplicate chunk number encountered"));
        }
        if let Some(f) = chunks_found.get_mut(chunk_number) {
            *f = true;
        }

        let data_size = read_u32_be(input)? as usize;
        if input.len() < data_size {
            return Err(Error::Malformed("unexpected EOF reading chunk payload"));
        }
        let (payload, rest) = input.split_at(data_size);
        *input = rest;

        let chunk_offset = chunk_number
            .checked_mul(chunk_size)
            .ok_or(Error::Malformed("chunk offset overflow"))?;
        let actual_chunk_size = std::cmp::min(chunk_size, length.saturating_sub(chunk_offset));
        let chunk_end = chunk_offset
            .checked_add(actual_chunk_size)
            .ok_or(Error::Malformed("chunk range overflow"))?;
        let chunk_dest = buffer
            .get_mut(chunk_offset..chunk_end)
            .ok_or(Error::Malformed("chunk destination out of buffer bounds"))?;

        if format_version == 0 {
            let mut decoder = GzDecoder::new(payload);
            decoder
                .read_exact(chunk_dest)
                .map_err(|_| Error::Malformed("GZIP decompression error in PDN chunk"))?;
        } else {
            if data_size != actual_chunk_size {
                return Err(Error::Malformed("uncompressed chunk size mismatch"));
            }
            chunk_dest.copy_from_slice(payload);
        }
    }

    if !chunks_found.iter().all(|&f| f) {
        return Err(Error::Malformed("not all chunks were found for layer"));
    }

    Ok(buffer)
}

/// Converts raw BGRA8 / BGR8 pixels to RGBA8 format.
pub fn convert_to_rgba8(raw_buffer: &[u8], width: u32, height: u32, stride: u32) -> Result<Vec<u8>, Error> {
    let w = width as usize;
    let h = height as usize;
    let mut s = stride as usize;
    if s == 0 {
        s = w.checked_mul(4).ok_or(Error::Malformed("stride overflow"))?;
    }
    let bpp = (s.checked_mul(8).ok_or(Error::Malformed("bpp overflow"))?)
        .checked_div(w.max(1))
        .unwrap_or(32);

    let total_pixels = w.checked_mul(h).ok_or(Error::Malformed("pixel count overflow"))?;
    let total_bytes = total_pixels
        .checked_mul(4)
        .ok_or(Error::Malformed("RGBA buffer overflow"))?;
    let mut rgba = vec![0u8; total_bytes];

    match bpp {
        32 => {
            for y in 0..h {
                let row_start = y.checked_mul(s).ok_or(Error::Malformed("row offset overflow"))?;
                let row_end = row_start
                    .checked_add(w.checked_mul(4).ok_or(Error::Malformed("row width overflow"))?)
                    .ok_or(Error::Malformed("row bounds overflow"))?;
                if row_end > raw_buffer.len() {
                    return Err(Error::Malformed("row extends past buffer length"));
                }
                let src_row = raw_buffer
                    .get(row_start..row_end)
                    .ok_or(Error::Malformed("invalid row slice"))?;
                let dst_start = y
                    .checked_mul(w)
                    .and_then(|v| v.checked_mul(4))
                    .ok_or(Error::Malformed("destination offset overflow"))?;
                let dst_row = rgba
                    .get_mut(dst_start..dst_start + w * 4)
                    .ok_or(Error::Malformed("invalid destination row slice"))?;

                for x in 0..w {
                    let b = *src_row.get(x * 4).ok_or(Error::Malformed("src B out of bounds"))?;
                    let g = *src_row.get(x * 4 + 1).ok_or(Error::Malformed("src G out of bounds"))?;
                    let r = *src_row.get(x * 4 + 2).ok_or(Error::Malformed("src R out of bounds"))?;
                    let a = *src_row.get(x * 4 + 3).ok_or(Error::Malformed("src A out of bounds"))?;
                    if let Some(px) = dst_row.get_mut(x * 4..x * 4 + 4) {
                        px[0] = r;
                        px[1] = g;
                        px[2] = b;
                        px[3] = a;
                    }
                }
            }
        }
        24 => {
            for y in 0..h {
                let row_start = y.checked_mul(s).ok_or(Error::Malformed("row offset overflow"))?;
                let row_end = row_start
                    .checked_add(w.checked_mul(3).ok_or(Error::Malformed("row width overflow"))?)
                    .ok_or(Error::Malformed("row bounds overflow"))?;
                if row_end > raw_buffer.len() {
                    return Err(Error::Malformed("row extends past buffer length"));
                }
                let src_row = raw_buffer
                    .get(row_start..row_end)
                    .ok_or(Error::Malformed("invalid row slice"))?;
                let dst_start = y
                    .checked_mul(w)
                    .and_then(|v| v.checked_mul(4))
                    .ok_or(Error::Malformed("destination offset overflow"))?;
                let dst_row = rgba
                    .get_mut(dst_start..dst_start + w * 4)
                    .ok_or(Error::Malformed("invalid destination row slice"))?;

                for x in 0..w {
                    let b = *src_row.get(x * 3).ok_or(Error::Malformed("src B out of bounds"))?;
                    let g = *src_row.get(x * 3 + 1).ok_or(Error::Malformed("src G out of bounds"))?;
                    let r = *src_row.get(x * 3 + 2).ok_or(Error::Malformed("src R out of bounds"))?;
                    if let Some(px) = dst_row.get_mut(x * 4..x * 4 + 4) {
                        px[0] = r;
                        px[1] = g;
                        px[2] = b;
                        px[3] = 255;
                    }
                }
            }
        }
        other => {
            return Err(Error::Unsupported(format!("unsupported PDN bits per pixel {other}")));
        }
    }

    Ok(rgba)
}

fn read_u8(input: &mut &[u8]) -> Result<u8, Error> {
    let (&first, rest) = input.split_first().ok_or(Error::Malformed("unexpected EOF in chunk stream"))?;
    *input = rest;
    Ok(first)
}

fn read_u32_be(input: &mut &[u8]) -> Result<u32, Error> {
    if input.len() < 4 {
        return Err(Error::Malformed("unexpected EOF reading u32 in chunk stream"));
    }
    let (head, rest) = input.split_at(4);
    *input = rest;
    let mut arr = [0u8; 4];
    arr.copy_from_slice(head);
    Ok(u32::from_be_bytes(arr))
}
