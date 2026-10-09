//! Bounded ISO-BMFF property inspection and single-picture muxing (ISO/IEC 14496-12,
//! AVIF specification). AV1 payload parsing remains with avif-parse.

use crate::{CodecError, Format};

const F: Format = Format::Avif;
const MAX_BOXES: usize = 4096;
const MAX_METADATA: usize = 16 << 20;

fn bad() -> CodecError {
    CodecError::malformed(F, "invalid or truncated AVIF container")
}

pub(super) struct BoxRef<'a> {
    pub kind: &'a [u8],
    pub data: &'a [u8],
}

pub(super) fn boxes(mut data: &[u8]) -> Result<Vec<BoxRef<'_>>, CodecError> {
    let mut out = Vec::new();
    while !data.is_empty() {
        if out.len() >= MAX_BOXES {
            return Err(CodecError::LimitExceeded("too many AVIF boxes".into()));
        }
        let h = data.get(..8).ok_or_else(bad)?;
        let size = u32::from_be_bytes(h.get(..4).ok_or_else(bad)?.try_into().map_err(|_| bad())?);
        let (len, header) = match size {
            0 => (data.len(), 8),
            1 => (usize::try_from(u64::from_be_bytes(data.get(8..16).ok_or_else(bad)?.try_into().map_err(|_| bad())?)).map_err(|_| bad())?, 16),
            n => (n as usize, 8),
        };
        if len < header {
            return Err(bad());
        }
        let body = data.get(header..len).ok_or_else(bad)?;
        out.push(BoxRef { kind: h.get(4..8).ok_or_else(bad)?, data: body });
        data = data.get(len..).ok_or_else(bad)?;
    }
    Ok(out)
}

fn find<'a>(boxes: &'a [BoxRef<'a>], kind: &[u8]) -> Result<&'a [u8], CodecError> {
    boxes.iter().find(|b| b.kind == kind).map(|b| b.data).ok_or_else(bad)
}

fn take<'a>(data: &mut &'a [u8], n: usize) -> Result<&'a [u8], CodecError> {
    let v = data.get(..n).ok_or_else(bad)?;
    *data = data.get(n..).ok_or_else(bad)?;
    Ok(v)
}
fn number(data: &mut &[u8], n: usize) -> Result<u32, CodecError> {
    Ok(take(data, n)?.iter().fold(0u32, |a, b| (a << 8) | u32::from(*b)))
}

#[derive(Default)]
pub(super) struct Properties {
    pub dimensions: Option<(u32, u32)>,
    pub icc: Option<Vec<u8>>,
    pub cicp: Option<(u16, u16, u16, bool)>,
}

/// Read only the primary item's associated properties, never a thumbnail's profile.
pub(super) fn properties(bytes: &[u8]) -> Result<Properties, CodecError> {
    let top = boxes(bytes)?;
    let ftyp = find(&top, b"ftyp")?;
    if ftyp.get(..4) == Some(b"avis")
        || ftyp.get(8..).is_some_and(|b| b.as_chunks::<4>().0.iter().any(|c| c == b"avis"))
        || top.iter().any(|b| b.kind == b"moov")
    {
        return Err(CodecError::unsupported(F, "AVIF image sequences are not supported; export a still image first"));
    }
    let meta = boxes(find(&top, b"meta")?.get(4..).ok_or_else(bad)?)?;
    let pitm = find(&meta, b"pitm")?;
    let mut id = pitm.get(4..).ok_or_else(bad)?;
    let primary = number(&mut id, if pitm.first() == Some(&0) { 2 } else { 4 })?;
    let iprp = boxes(find(&meta, b"iprp")?)?;
    let props = boxes(find(&iprp, b"ipco")?)?;
    let mut out = Properties::default();
    for assoc in iprp.iter().filter(|b| b.kind == b"ipma") {
        let mut data = assoc.data;
        let version = number(&mut data, 1)?;
        let flags = number(&mut data, 3)?;
        let count = number(&mut data, 4)?;
        if count > MAX_BOXES as u32 {
            return Err(CodecError::LimitExceeded("too many AVIF item associations".into()));
        }
        for _ in 0..count {
            let item = number(&mut data, if version == 0 { 2 } else { 4 })?;
            let n = number(&mut data, 1)?;
            for _ in 0..n {
                let wide = flags & 1 != 0;
                let index = number(&mut data, if wide { 2 } else { 1 })?;
                let essential = index & if wide { 0x8000 } else { 0x80 } != 0;
                let index = index & if wide { 0x7fff } else { 0x7f };
                if index == 0 || item != primary {
                    continue;
                }
                let p = props.get(index as usize - 1).ok_or_else(bad)?;
                match p.kind {
                    b"ispe" => {
                        let mut size = p.data.get(4..).ok_or_else(bad)?;
                        out.dimensions = Some((number(&mut size, 4)?, number(&mut size, 4)?));
                    }
                    b"colr" if p.data.get(..4) == Some(b"prof") || p.data.get(..4) == Some(b"rICC") => {
                        let icc = p.data.get(4..).ok_or_else(bad)?;
                        if icc.len() > MAX_METADATA {
                            return Err(CodecError::LimitExceeded("AVIF ICC profile exceeds 16 MiB".into()));
                        }
                        out.icc = Some(icc.to_vec());
                    }
                    b"colr" if p.data.get(..4) == Some(b"nclx") => {
                        let mut c = p.data.get(4..).ok_or_else(bad)?;
                        out.cicp = Some((number(&mut c, 2)? as u16, number(&mut c, 2)? as u16, number(&mut c, 2)? as u16, number(&mut c, 1)? & 128 != 0));
                    }
                    b"irot" | b"imir" | b"clap" => {
                        return Err(CodecError::unsupported(
                            F,
                            "AVIF rotation, mirror and clean-aperture properties are not supported; bake the transform first",
                        ));
                    }
                    b"colr" => return Err(CodecError::unsupported(F, "AVIF colour property must be ICC or nclx")),
                    b"pixi" | b"av1C" | b"auxC" | b"prem" => {}
                    _ if essential => {
                        return Err(CodecError::unsupported(F, format!("essential AVIF property {} is unsupported", String::from_utf8_lossy(p.kind))));
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(out)
}

fn boxed(kind: &[u8; 4], body: &[u8]) -> Result<Vec<u8>, CodecError> {
    let len = body.len().checked_add(8).and_then(|v| u32::try_from(v).ok()).ok_or_else(bad)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len as usize).map_err(|_| CodecError::LimitExceeded("not enough memory for AVIF container".into()))?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    Ok(out)
}
fn full(kind: &[u8; 4], body: &[u8]) -> Result<Vec<u8>, CodecError> {
    let mut data = vec![0; 4];
    data.extend_from_slice(body);
    boxed(kind, &data)
}
fn join(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.iter().flatten().copied().collect()
}

/// Add ICC using a fresh, small still-picture container; preserve encoder AV1 configuration.
pub(super) fn mux(encoded: &[u8], icc: &[u8], w: u32, h: u32, depth: u8) -> Result<Vec<u8>, CodecError> {
    if icc.len() > MAX_METADATA {
        return Err(CodecError::LimitExceeded("AVIF ICC profile exceeds 16 MiB".into()));
    }
    let mut reader = encoded;
    let parsed = avif_parse::read_avif(&mut reader).map_err(|e| CodecError::malformed(F, e))?;
    let top = boxes(encoded)?;
    let meta = boxes(find(&top, b"meta")?.get(4..).ok_or_else(bad)?)?;
    let iprp = boxes(find(&meta, b"iprp")?)?;
    let props = boxes(find(&iprp, b"ipco")?)?;
    let config = props.iter().find(|b| b.kind == b"av1C").map(|b| b.data).ok_or_else(bad)?;
    let alpha = parsed.alpha_item.as_deref();
    let n = if alpha.is_some() { 2u16 } else { 1 };
    let mut infos = n.to_be_bytes().to_vec();
    for id in 1..=n {
        let mut item = vec![2, 0, 0, 0];
        item.extend_from_slice(&id.to_be_bytes());
        item.extend_from_slice(&[0, 0]);
        item.extend_from_slice(b"av01");
        item.push(0);
        infos.extend_from_slice(&boxed(b"infe", &item)?);
    }
    let iinf = full(b"iinf", &infos)?;
    let pitm = full(b"pitm", &1u16.to_be_bytes())?;
    let mut size = w.to_be_bytes().to_vec();
    size.extend_from_slice(&h.to_be_bytes());
    let mut colr = b"prof".to_vec();
    colr.extend_from_slice(icc);
    let mut prop_boxes = vec![full(b"ispe", &size)?, boxed(b"av1C", config)?, full(b"pixi", &[3, depth, depth, depth])?, boxed(b"colr", &colr)?];
    if alpha.is_some() {
        // Alpha has its own monochrome av1C/pixi and the standard alpha auxiliary type.
        let ac = props.iter().filter(|b| b.kind == b"av1C").nth(1).map(|b| b.data).ok_or_else(bad)?;
        prop_boxes.push(boxed(b"av1C", ac)?);
        prop_boxes.push(full(b"pixi", &[1, depth])?);
        prop_boxes.push(full(b"auxC", b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha\0")?);
    }
    let mut associations = u32::from(n).to_be_bytes().to_vec();
    associations.extend_from_slice(&[0, 1, 4, 0x81, 0x82, 0x83, 0x84]);
    if alpha.is_some() {
        associations.extend_from_slice(&[0, 2, 4, 0x81, 0x85, 0x86, 0x87]);
    }
    let iprp = boxed(b"iprp", &join(&[boxed(b"ipco", &join(&prop_boxes))?, full(b"ipma", &associations)?]))?;
    let iref = if alpha.is_some() { full(b"iref", &boxed(b"auxl", &[0, 2, 0, 1, 0, 1])?)? } else { Vec::new() };
    let hdlr = full(b"hdlr", b"\0\0\0\0pict\0\0\0\0\0\0\0\0\0\0\0\0PhotoCraft\0")?;
    let ftyp = boxed(b"ftyp", b"avif\0\0\0\0avifmif1miafMA1A")?;
    let make_meta = |offset: u32| -> Result<Vec<u8>, CodecError> {
        let mut iloc = vec![0x44, 0];
        iloc.extend_from_slice(&n.to_be_bytes());
        let mut pos = offset;
        for (id, payload) in [(1u16, Some(parsed.primary_item.as_slice())), (2, alpha)] {
            let Some(payload) = payload else { continue };
            let len = u32::try_from(payload.len()).map_err(|_| bad())?;
            iloc.extend_from_slice(&id.to_be_bytes());
            iloc.extend_from_slice(&[0, 0, 0, 1]);
            iloc.extend_from_slice(&pos.to_be_bytes());
            iloc.extend_from_slice(&len.to_be_bytes());
            pos = pos.checked_add(len).ok_or_else(bad)?;
        }
        full(b"meta", &join(&[hdlr.clone(), pitm.clone(), full(b"iloc", &iloc)?, iinf.clone(), iprp.clone(), iref.clone()]))
    };
    let meta_size = make_meta(0)?.len();
    let offset = ftyp.len().checked_add(meta_size).and_then(|v| v.checked_add(8)).and_then(|v| u32::try_from(v).ok()).ok_or_else(bad)?;
    let mut pixels = parsed.primary_item.to_vec();
    if let Some(a) = alpha {
        pixels.extend_from_slice(a);
    }
    Ok(join(&[ftyp, make_meta(offset)?, boxed(b"mdat", &pixels)?]))
}
