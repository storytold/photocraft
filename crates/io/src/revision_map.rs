//! Revision markers ⇄ PSD: a private image resource that carries the document's revision
//! markers (red boxes over content that must be changed; green once detection finds the content
//! changed). The id sits in the PSD spec's private resource range (0x0BB7–0x0FFF), so no
//! application assigns it a meaning; PhotoCraft writes it only while the document has markers,
//! and drops a stale imported copy as soon as markers exist in the document (the document's own
//! list is authoritative).
//!
//! Layout (big-endian):
//! ```text
//! u32 count
//! per marker:
//!   u16 name_len, UTF-8 name
//!   f64 pos_x, f64 pos_y          (box top-left, document pixels)
//!   f64 size_w, f64 size_h        (box size)
//!   u8  has_layer, u64 layer_id   (box follows this layer when it moves; 0/absent = fixed)
//!   u32 snap_w, u32 snap_h        (snapshot grid)
//!   u32 snap_len, RGBA8 snapshot
//!   u8  done                      (green box)
//!   u16 note_len, UTF-8 note
//! ```

use photocraft_doc::{Document, LayerId, RevisionMarker};

/// Private PSD image resource id for the revision markers (0x0F00 = 3840, inside the spec's
/// private 0x0BB7–0x0FFF range).
pub const REVISION_MARKERS: u16 = 0x0F00;

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_f64(out: &mut Vec<u8>, v: f64) {
    out.extend_from_slice(&v.to_bits().to_be_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    put_u16(out, b.len().min(u16::MAX as usize) as u16);
    out.extend_from_slice(&b[..b.len().min(u16::MAX as usize)]);
}

/// Encode the document's revision markers into the private resource's bytes (`None` when the
/// document has none, so a save leaves any imported copy alone).
pub fn encode(doc: &Document) -> Option<Vec<u8>> {
    if doc.revision_markers.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    put_u32(&mut out, doc.revision_markers.len() as u32);
    for m in &doc.revision_markers {
        put_str(&mut out, &m.name);
        put_f64(&mut out, m.pos[0]);
        put_f64(&mut out, m.pos[1]);
        put_f64(&mut out, m.size[0]);
        put_f64(&mut out, m.size[1]);
        match m.layer_id {
            Some(id) => {
                out.push(1);
                put_u64(&mut out, id.0);
            }
            None => out.push(0),
        }
        put_u32(&mut out, m.snapshot_size[0]);
        put_u32(&mut out, m.snapshot_size[1]);
        put_u32(&mut out, m.snapshot.len() as u32);
        out.extend_from_slice(&m.snapshot);
        out.push(u8::from(m.done));
        put_str(&mut out, &m.note);
    }
    Some(out)
}

struct R<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.d.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Option<u64> {
        let b = self.take(8)?;
        let a: [u8; 8] = b.try_into().ok()?;
        Some(u64::from_be_bytes(a))
    }
    fn f64(&mut self) -> Option<f64> {
        let b = self.take(8)?;
        let a: [u8; 8] = b.try_into().ok()?;
        Some(f64::from_bits(u64::from_be_bytes(a)))
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn str(&mut self) -> Option<String> {
        let n = usize::from(self.u16()?);
        let b = self.take(n)?;
        Some(String::from_utf8_lossy(b).into_owned())
    }
}

/// Parse a private resource's bytes back into revision markers; `None` when the block is not
/// valid (it is then kept as an unmodelled resource rather than dropped).
pub fn decode(data: &[u8]) -> Option<Vec<RevisionMarker>> {
    let mut r = R { d: data, p: 0 };
    let count = r.u32()? as usize;
    if count > 10_000 {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let name = r.str()?;
        let pos = [r.f64()?, r.f64()?];
        let size = [r.f64()?, r.f64()?];
        let layer_id = match r.u8()? {
            0 => None,
            1 => Some(LayerId(r.u64()?)),
            _ => return None,
        };
        let snap_w = r.u32()?;
        let snap_h = r.u32()?;
        let snap_len = r.u32()? as usize;
        if snap_w > 4096 || snap_h > 4096 || snap_len != (snap_w * snap_h * 4) as usize {
            return None;
        }
        let snapshot = r.take(snap_len)?.to_vec();
        let done = r.u8()? != 0;
        let note = r.str()?;
        out.push(RevisionMarker {
            name,
            pos,
            size,
            layer_id,
            snapshot,
            snapshot_size: [snap_w, snap_h],
            done,
            note,
        });
    }
    if r.p != data.len() {
        return None;
    }
    Some(out)
}

/// Whether an imported copy of the resource still describes the document. Once the document has
/// its own markers (added, edited or detected in this session), the stale imported bytes are
/// replaced by `encode`'s output on save.
pub fn keep_resource(doc: &Document, id: u16) -> bool {
    id == REVISION_MARKERS && doc.revision_markers.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(name: &str) -> RevisionMarker {
        RevisionMarker {
            name: name.into(),
            pos: [10.5, 20.25],
            size: [100.0, 60.0],
            layer_id: Some(LayerId(7)),
            snapshot: vec![0u8; 48 * 48 * 4],
            snapshot_size: [48, 48],
            done: false,
            note: "改这里".into(),
        }
    }

    #[test]
    fn roundtrip() {
        let mut d = Document::new("m", photocraft_geom::Size::new(100, 100), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        d.revision_markers.push(marker("A"));
        d.revision_markers.push(RevisionMarker { done: true, ..marker("B") });
        let data = encode(&d).unwrap();
        let back = decode(&data).unwrap();
        assert_eq!(back, d.revision_markers);
    }

    #[test]
    fn empty_is_none() {
        let d = Document::new("m", photocraft_geom::Size::new(10, 10), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        assert!(encode(&d).is_none());
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(decode(b"not a marker block at all").is_none());
        let mut d = Document::new("m", photocraft_geom::Size::new(10, 10), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        d.revision_markers.push(marker("A"));
        let data = encode(&d).unwrap();
        // Truncate the tail: must fail, not panic.
        assert!(decode(&data[..data.len() - 3]).is_none());
    }
}
