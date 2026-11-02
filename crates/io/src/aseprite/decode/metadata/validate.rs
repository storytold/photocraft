//! Validate opaque metadata before retaining it. Its original bytes survive, but malformed
//! counts, property trees and attachments must not be exported into another application.

use super::{Reader, Result, invalid};

pub(in crate::aseprite) fn validate(kind: u16, data: &[u8]) -> Result<()> {
    let mut r = Reader::new(data);
    match kind {
        0x2020 => {
            let flags = r.u32()?;
            if flags & !7 != 0 {
                return Err(invalid("unsupported user data flags"));
            }
            if flags & 1 != 0 {
                r.string()?;
            }
            if flags & 2 != 0 {
                r.take(4)?;
            }
            if flags & 4 != 0 {
                let len = (r.u32()? as usize).checked_sub(4).ok_or_else(|| invalid("invalid properties size"))?;
                let mut maps = Reader::new(r.take(len)?);
                let count = maps.u32()? as usize;
                if count > 65536 {
                    return Err(invalid("too many property maps"));
                }
                let mut budget = 65536usize;
                for _ in 0..count {
                    maps.u32()?;
                    properties(&mut maps, 0, &mut budget)?;
                }
                maps.end()?;
            }
        }
        0x2008 => {
            let count = r.u32()? as usize;
            if count > 65536 {
                return Err(invalid("too many external references"));
            }
            r.take(8)?;
            let mut ids = std::collections::BTreeSet::new();
            for _ in 0..count {
                if !ids.insert(r.u32()?) {
                    return Err(invalid("duplicate external reference id"));
                }
                if r.u8()? > 3 {
                    return Err(invalid("unsupported external reference type"));
                }
                r.take(7)?;
                r.string()?;
            }
        }
        0x2016 => {
            r.take(4)?;
            let (w, h) = (usize::from(r.u16()?), usize::from(r.u16()?));
            r.take(8)?;
            r.string()?;
            let bytes = w.div_ceil(8).checked_mul(h).ok_or_else(|| invalid("saved mask size overflow"))?;
            r.take(bytes)?;
        }
        0x2022 => {
            let count = r.u32()? as usize;
            let flags = r.u32()?;
            r.take(4)?;
            r.string()?;
            if count > 65536 || flags & !3 != 0 {
                return Err(invalid("invalid slice key count or flags"));
            }
            for _ in 0..count {
                r.take(20)?;
                if flags & 1 != 0 {
                    r.take(16)?;
                }
                if flags & 2 != 0 {
                    r.take(8)?;
                }
            }
        }
        0x2017 => {
            r.take(r.remaining())?;
        }
        _ => return Err(invalid("unsupported retained metadata kind")),
    }
    r.end()
}

fn properties(r: &mut Reader<'_>, depth: usize, budget: &mut usize) -> Result<()> {
    if depth > 128 {
        return Err(invalid("nested user properties exceed 128 levels"));
    }
    let count = r.u32()? as usize;
    *budget = budget.checked_sub(count).ok_or_else(|| invalid("user properties exceed 65536 entries"))?;
    for _ in 0..count {
        r.string()?;
        let kind = r.u16()?;
        value(r, kind, depth, budget)?;
    }
    Ok(())
}

fn value(r: &mut Reader<'_>, kind: u16, depth: usize, budget: &mut usize) -> Result<()> {
    if depth > 128 {
        return Err(invalid("nested user properties exceed 128 levels"));
    }
    match kind {
        1..=3 => {
            r.take(1)?;
        }
        4 | 5 => {
            r.take(2)?;
        }
        6 | 7 | 0xa | 0xb => {
            r.take(4)?;
        }
        8 | 9 | 0xc | 0xe | 0xf => {
            r.take(8)?;
        }
        0x10 | 0x13 => {
            r.take(16)?;
        }
        0xd => {
            r.string()?;
        }
        0x11 => {
            let count = r.u32()? as usize;
            let element = r.u16()?;
            *budget = budget.checked_sub(count).ok_or_else(|| invalid("user data vectors exceed 65536 elements"))?;
            for _ in 0..count {
                let kind = if element == 0 { r.u16()? } else { element };
                value(r, kind, depth + 1, budget)?;
            }
        }
        0x12 => properties(r, depth + 1, budget)?,
        _ => return Err(invalid(format!("unsupported user property type 0x{kind:04x}"))),
    }
    Ok(())
}
