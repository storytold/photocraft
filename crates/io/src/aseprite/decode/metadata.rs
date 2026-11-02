use super::{Arc, ColorMode, Decoder, MetadataTarget, PaletteEntry, Reader, Result, SpriteTag, invalid, sprite};
use photocraft_doc::sprite::SpriteChunk;

pub(in crate::aseprite) mod validate;

impl Decoder<'_> {
    pub(super) fn retain(&mut self, kind: u16, bytes: &[u8]) -> Result<()> {
        validate::validate(kind, bytes)?;
        self.metadata_bytes = self.metadata_bytes.checked_add(bytes.len()).ok_or_else(|| invalid("metadata size overflow"))?;
        if self.metadata_bytes > sprite::MAX_METADATA {
            return Err(invalid("sprite metadata exceeds 32 MiB"));
        }
        if kind != 0x2020 {
            self.target = MetadataTarget::Frame
        }
        self.frame.metadata.push(SpriteChunk { kind, target: self.target.clone(), bytes: Arc::new(bytes.to_vec()) });
        Ok(())
    }

    pub(super) fn precise(&mut self, r: &mut Reader<'_>) -> Result<()> {
        let MetadataTarget::Cel(id) = self.target else { return Err(invalid("cel extra record has no preceding cel")) };
        let flags = r.u32()?;
        if flags & !1 != 0 {
            return Err(invalid("unsupported cel extra flags"));
        }
        let bounds = [r.u32()? as i32, r.u32()? as i32, r.u32()? as i32, r.u32()? as i32];
        r.take(16)?;
        if flags & 1 != 0 {
            if bounds[2] <= 0 || bounds[3] <= 0 {
                return Err(invalid("precise cel dimensions must be positive"));
            }
            let cel = self.frame.cels.get_mut(&id).ok_or_else(|| invalid("cel extra target is missing"))?;
            Arc::make_mut(cel).precise = Some(bounds);
        }
        Ok(())
    }

    pub(super) fn profile(&mut self, data: &[u8], r: &mut Reader<'_>) -> Result<()> {
        if self.profile_seen {
            return Err(invalid("duplicate color profile"));
        }
        self.profile_seen = true;
        if data.len() > (16 << 20) + 20 {
            return Err(invalid("color profile exceeds 16 MiB"));
        }
        self.profile = color_profile(r, self.fmt.mode == ColorMode::Grayscale)?;
        self.profile_chunk = Some(Arc::new(data.to_vec()));
        Ok(())
    }

    pub(super) fn palette(&mut self, kind: u16, r: &mut Reader<'_>) -> Result<()> {
        self.palette_seen = true;
        // A tiny partial palette update can copy a large named palette from an earlier
        // frame. Charge that copy before Arc::make_mut, not after decoding all frames.
        if Arc::strong_count(&self.frame.palette) > 1 {
            let cost = self
                .frame
                .palette
                .iter()
                .try_fold(0usize, |n, e| n.checked_add(e.name.len() + std::mem::size_of::<PaletteEntry>()))
                .ok_or_else(|| invalid("palette memory size overflow"))?;
            self.metadata_bytes = self.metadata_bytes.checked_add(cost).ok_or_else(|| invalid("palette memory size overflow"))?;
            if self.metadata_bytes > sprite::MAX_METADATA {
                return Err(invalid("sprite palette metadata exceeds 32 MiB"));
            }
        }
        let palette = Arc::make_mut(&mut self.frame.palette);
        if kind == 0x2019 {
            let (size, first, last) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
            r.take(8)?;
            if size == 0 || size > 256 || first > last || last >= size {
                return Err(invalid("invalid palette size or update range"));
            }
            palette.resize_with(size, PaletteEntry::default);
            for i in first..=last {
                let flags = r.u16()?;
                if flags & !1 != 0 {
                    return Err(invalid("unsupported palette entry flags"));
                }
                let rgba = [r.u8()?, r.u8()?, r.u8()?, r.u8()?];
                let name = if flags & 1 != 0 { r.string()? } else { String::new() };
                self.metadata_bytes = self.metadata_bytes.checked_add(name.len()).ok_or_else(|| invalid("palette metadata size overflow"))?;
                if self.metadata_bytes > sprite::MAX_METADATA {
                    return Err(invalid("sprite palette metadata exceeds 32 MiB"));
                }
                *palette.get_mut(i).ok_or_else(|| invalid("palette entry is missing"))? = PaletteEntry { rgba, name };
            }
        } else {
            let packets = usize::from(r.u16()?);
            let mut at = 0usize;
            if palette.len() < 256 {
                palette.resize_with(256, PaletteEntry::default)
            }
            for _ in 0..packets {
                at = at.checked_add(usize::from(r.u8()?)).ok_or_else(|| invalid("palette offset overflow"))?;
                let n = usize::from(r.u8()?);
                for _ in 0..if n == 0 { 256 } else { n } {
                    let mut rgba = [r.u8()?, r.u8()?, r.u8()?, 255];
                    if kind == 0x0011 {
                        for c in rgba.iter_mut().take(3) {
                            if *c > 63 {
                                return Err(invalid("legacy 6-bit palette component is out of range"));
                            }
                            *c = (u16::from(*c) * 255 / 63) as u8;
                        }
                    }
                    palette.get_mut(at).ok_or_else(|| invalid("legacy palette range exceeds 256 entries"))?.rgba = rgba;
                    at += 1;
                }
            }
        }
        self.target = MetadataTarget::Palette;
        Ok(())
    }

    pub(super) fn tags(&mut self, r: &mut Reader<'_>) -> Result<()> {
        if !self.tags.is_empty() {
            return Err(invalid("duplicate animation tags"));
        }
        let count = usize::from(r.u16()?);
        r.take(8)?;
        for _ in 0..count {
            let (from, to, direction, repeat) = (usize::from(r.u16()?), usize::from(r.u16()?), r.u8()?, r.u16()?);
            r.take(6)?;
            let color = [r.u8()?, r.u8()?, r.u8()?];
            r.take(1)?;
            let name = r.string()?;
            self.metadata_bytes =
                self.metadata_bytes.checked_add(name.len() + std::mem::size_of::<SpriteTag>()).ok_or_else(|| invalid("tag metadata size overflow"))?;
            if self.metadata_bytes > sprite::MAX_METADATA {
                return Err(invalid("sprite tag metadata exceeds 32 MiB"));
            }
            self.tags.push(SpriteTag { name, from, to, direction, repeat, color });
        }
        self.target = MetadataTarget::Tags;
        Ok(())
    }
}

pub(in super::super) fn color_profile(r: &mut Reader<'_>, gray: bool) -> Result<Option<Arc<Vec<u8>>>> {
    use photocraft_cms::{Builtin, ColorSpace, Curve, Profile};
    let (kind, flags, gamma) = (r.u16()?, r.u16()?, r.u32()? as i32);
    r.take(8)?;
    if flags & !1 != 0 || (flags & 1 != 0 && gamma <= 0) {
        return Err(invalid("invalid fixed-gamma profile"));
    }
    let mut profile = match kind {
        0 if flags == 0 => return Ok(None),
        0 | 1 => {
            if gray {
                Builtin::SGray.profile().clone()
            } else {
                Builtin::Srgb.profile().clone()
            }
        }
        2 => {
            let len = r.u32()? as usize;
            if len > 16 << 20 {
                return Err(invalid("ICC profile exceeds 16 MiB"));
            }
            let profile = Profile::parse(r.take(len)?).map_err(|e| invalid(format!("invalid ICC profile: {e}")))?;
            if profile.color_space != if gray { ColorSpace::Gray } else { ColorSpace::Rgb } {
                return Err(invalid("ICC color space does not match the sprite"));
            }
            profile
        }
        _ => return Err(invalid("unsupported color profile")),
    };
    if flags & 1 != 0 {
        let curve = Curve::Gamma(f64::from(gamma) / 65536.0);
        if gray {
            profile.gray_trc = Some(curve);
        } else {
            if profile.matrix.is_none() {
                return Err(invalid("fixed gamma requires an RGB matrix profile"));
            }
            profile.trc = Some([curve.clone(), curve.clone(), curve]);
        }
        profile.a2b = [None, None, None];
        profile.b2a = [None, None, None];
        profile = profile.with_encoded_bytes();
    }
    Ok(Some(profile.to_bytes()))
}
