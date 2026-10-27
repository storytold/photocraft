//! Bounded, length-prefixed protocol shared by the safe host and the external worker.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};
pub const MAGIC: &[u8; 8] = b"PC8BF001";
pub const MAX_METADATA: usize = 64 * 1024;
pub const MAX_PIXELS: usize = 512 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub path: String,
    pub entry: String,
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub depth: u8,
    pub show_ui: bool,
}
impl Request {
    /// Interleaved Gray/RGB samples, without alpha. Depth 16 uses full-range
    /// unsigned little-endian samples; the worker handles native ABI scaling.
    pub fn pixel_len(&self) -> io::Result<usize> {
        if self.width == 0 || self.height == 0 || self.width > 32767 || self.height > 32767 || !matches!(self.channels, 1 | 3) || !matches!(self.depth, 8 | 16)
        {
            return Err(invalid("8BF requires bounded Gray or RGB pixels at 8 or 16 bits per channel"));
        }
        let length = u64::from(self.width) * u64::from(self.height) * u64::from(self.channels) * u64::from(self.depth / 8);
        if length > MAX_PIXELS as u64 {
            return Err(invalid("8BF pixel buffer exceeds size limit"));
        }
        usize::try_from(length).map_err(|_| invalid("8BF pixel buffer exceeds address space"))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub status: String,
    pub message: String,
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub depth: u8,
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn read_exact(reader: &mut impl Read, bytes: &mut [u8], stage: &str) -> io::Result<()> {
    reader.read_exact(bytes).map_err(|error| io::Error::new(error.kind(), format!("8BF {stage}: {error}")))
}
pub fn read_frame<T: DeserializeOwned>(mut reader: impl Read) -> io::Result<(T, Vec<u8>)> {
    let mut magic = [0u8; 8];
    let mut meta_len = [0u8; 4];
    let mut pixel_len = [0u8; 8];
    read_exact(&mut reader, &mut magic, "frame magic")?;
    if &magic != MAGIC {
        return Err(invalid("invalid 8BF frame magic"));
    }
    read_exact(&mut reader, &mut meta_len, "metadata length")?;
    read_exact(&mut reader, &mut pixel_len, "pixel length")?;
    let meta_len = u32::from_le_bytes(meta_len) as usize;
    let pixel_len = usize::try_from(u64::from_le_bytes(pixel_len)).map_err(|_| invalid("8BF pixel length exceeds address space"))?;
    if meta_len > MAX_METADATA || pixel_len > MAX_PIXELS {
        return Err(invalid("8BF frame exceeds size limit"));
    }
    let mut metadata = vec![0u8; meta_len];
    read_exact(&mut reader, &mut metadata, "metadata payload")?;
    let metadata = serde_json::from_slice(&metadata).map_err(|e| invalid(format!("invalid 8BF metadata: {e}")))?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(pixel_len).map_err(|e| invalid(format!("cannot allocate 8BF pixel buffer: {e}")))?;
    pixels.resize(pixel_len, 0);
    read_exact(&mut reader, &mut pixels, "pixel payload")?;
    Ok((metadata, pixels))
}
pub fn write_frame<T: Serialize>(mut writer: impl Write, metadata: &T, pixels: &[u8]) -> io::Result<()> {
    let metadata = serde_json::to_vec(metadata).map_err(|e| invalid(e.to_string()))?;
    if metadata.len() > MAX_METADATA || pixels.len() > MAX_PIXELS {
        return Err(invalid("8BF frame exceeds size limit"));
    }
    writer.write_all(MAGIC)?;
    writer.write_all(&(metadata.len() as u32).to_le_bytes())?;
    writer.write_all(&(pixels.len() as u64).to_le_bytes())?;
    writer.write_all(&metadata)?;
    writer.write_all(pixels)?;
    writer.flush()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_roundtrip_and_truncation() {
        let request = Request { path: "Z:\\tmp\\filter.8bf".into(), entry: "PluginMain".into(), width: 2, height: 1, channels: 3, depth: 8, show_ui: true };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &request, &[0, 20, 255, 4, 5, 6]).unwrap();
        let (got, pixels): (Request, _) = read_frame(bytes.as_slice()).unwrap();
        assert_eq!(got.path, request.path);
        assert_eq!(pixels, [0, 20, 255, 4, 5, 6]);
        for n in [0, 7, 19, bytes.len() - 1] {
            assert!(read_frame::<Request>(&bytes[..n]).is_err());
        }
    }
    #[test]
    fn oversized_frame_is_rejected_before_payload_read() {
        for (meta, pixels) in [(MAX_METADATA as u32 + 1, 0), (1, MAX_PIXELS as u64 + 1)] {
            let mut frame = MAGIC.to_vec();
            frame.extend_from_slice(&meta.to_le_bytes());
            frame.extend_from_slice(&pixels.to_le_bytes());
            assert_eq!(read_frame::<Request>(frame.as_slice()).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }
}
