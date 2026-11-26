//! Native document policy for bounded, inert metadata from the shared reader.

use std::sync::Arc;

use craft_xcf::records::{
    self,
    metadata::{ImageMetadata, PacketKind},
};
use photocraft_doc::Document;
use photocraft_raster::Interrupt;

use super::{IoError, Result, Warnings, map_error};

const MAX_NATIVE_PACKET_BYTES: usize = 4 << 20;

pub(super) fn read<'a>(xcf: &craft_xcf::Xcf<'a>, budget: &mut records::Budget, warnings: &mut Warnings, ctl: &Interrupt<'_>) -> Result<ImageMetadata<'a>> {
    let keep_going = || !ctl.cancelled();
    let limits = records::metadata::Limits::default();
    let properties = xcf.image_properties(limits.records, budget, &keep_going).map_err(map_error)?;
    let evidence = records::metadata::image(&properties, limits, budget, &keep_going).map_err(map_error)?;
    if !evidence.complete() {
        warnings.note("the XCF optional metadata scan stopped early; complete guides and saved pixels were preserved");
    } else if !evidence.issues.is_empty() {
        warnings.note("the XCF contains invalid optional metadata; usable values and saved pixels were preserved");
    }
    Ok(evidence)
}

pub(super) fn resolution(source: &ImageMetadata<'_>, fallback: f32) -> (f32, f32) {
    if source.complete()
        && let Some(resolution) = source.resolution
    {
        return (resolution.value.horizontal, resolution.value.vertical);
    }
    (fallback, fallback)
}

pub(super) fn assigned_profile(source: &ImageMetadata<'_>, warnings: &mut Warnings) -> Option<Vec<u8>> {
    let bytes = source.profile.filter(|_| source.complete())?.value;
    if bytes.len() > MAX_NATIVE_PACKET_BYTES {
        warnings.note("the embedded XCF color profile exceeds the native metadata limit");
        return None;
    }
    Some(bytes.to_vec())
}

pub(super) fn apply(
    document: &mut Document,
    source: &ImageMetadata<'_>,
    axes: (f32, f32),
    version: u32,
    warnings: &mut Warnings,
    ctl: &Interrupt<'_>,
) -> Result<()> {
    // Cancellation must precede DPI/warning mutation even with no guide records.
    ctl.check().map_err(|_| IoError::Cancelled)?;
    document.resolution_dpi = axes.0;
    if let Some(warning) = crate::unequal_resolution_warning(f64::from(axes.0), f64::from(axes.1)) {
        warnings.note(&warning);
    }

    for guide in &source.guides {
        use craft_xcf::Orientation;
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let (target, extent) = match guide.value.orientation {
            Orientation::Horizontal => (&mut document.guides.horizontal, document.size.height),
            Orientation::Vertical => (&mut document.guides.vertical, document.size.width),
        };
        let position = i64::from(guide.value.position);
        if version >= 15 || (0..=i64::from(extent)).contains(&position) {
            target.push(f64::from(guide.value.position));
        }
    }

    // A bounded prefix cannot identify the final occurrence of a duplicate packet.
    if !source.complete() {
        return Ok(());
    }
    let last = |kind| source.packets.iter().rfind(|packet| packet.value.kind == kind).map(|packet| packet.value.bytes);
    if let Some(comment) = last(PacketKind::Comment) {
        if comment.len() > MAX_NATIVE_PACKET_BYTES {
            warnings.note("the XCF comment exceeds the native metadata limit");
        } else {
            let text = String::from_utf8_lossy(comment);
            let text = text.trim_end_matches('\0');
            if !text.is_empty() {
                document.metadata.text.push(("Comment".into(), text.to_owned()));
            }
        }
    }
    if let Some(packet) = last(PacketKind::Exif) {
        let data = packet.strip_prefix(b"Exif\0\0").unwrap_or(packet);
        if data.len() <= MAX_NATIVE_PACKET_BYTES && has_exif_directory(data) {
            document.metadata.exif = Some(Arc::new(data.to_vec()));
        } else {
            warnings.note("the XCF EXIF packet is invalid or exceeds the native metadata limit");
        }
    }
    super::metadata_packets::apply(document, source, warnings, ctl)
}

/// Validate the TIFF header and first IFD before storing a raw native EXIF packet.
pub(super) fn has_exif_directory(raw: &[u8]) -> bool {
    let Some(&[a, b, c, d, p0, p1, p2, p3]) = raw.first_chunk::<8>() else { return false };
    let little = match [a, b, c, d] {
        [b'I', b'I', 42, 0] => true,
        [b'M', b'M', 0, 42] => false,
        _ => return false,
    };
    let u16_at = |bytes| if little { u16::from_le_bytes(bytes) } else { u16::from_be_bytes(bytes) };
    let u32_at = |bytes| if little { u32::from_le_bytes(bytes) } else { u32::from_be_bytes(bytes) };
    let offset = u32_at([p0, p1, p2, p3]) as usize;
    let Some(directory) = raw.get(offset..).filter(|_| offset >= 8) else { return false };
    let Some(count) = directory.first_chunk::<2>() else { return false };
    let end = 2 + usize::from(u16_at(*count)) * 12;
    let Some(data) = directory.get(..end + 4).and_then(|data| data.get(2..end)) else { return false };
    data.as_chunks::<12>().0.iter().all(|&[_, _, t0, t1, n0, n1, n2, n3, v0, v1, v2, v3]| {
        let width = match u16_at([t0, t1]) {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 | 13 => 4,
            5 | 10 | 12 | 16..=18 => 8,
            _ => return false,
        };
        let size = usize::try_from(u32_at([n0, n1, n2, n3])).ok().and_then(|count| count.checked_mul(width));
        size.is_some_and(|size| size <= 4 || (u32_at([v0, v1, v2, v3]) as usize).checked_add(size).is_some_and(|end| end <= raw.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::super::{import, tests::TestImage};
    use super::*;

    #[test]
    fn last_valid_resolution_survives_invalid_optional_duplicates() {
        let mut source = TestImage::rgb8(16, 8);
        let pair = |x: f32, y: f32| [x.to_be_bytes(), y.to_be_bytes()].concat();
        source.image_props.extend([(19, pair(299.0, 250.0)), (19, pair(300.0, 150.0)), (19, pair(100.0, f32::NAN))]);
        let imported = import("unequal", &source.write(), &Interrupt::NONE).unwrap();
        assert_eq!(imported.document.resolution_dpi, 300.0);
        assert!(imported.warnings.iter().any(|note| note.contains("150.00 ppi")));
        assert!(imported.warnings.iter().any(|note| note.contains("invalid optional metadata")));
    }

    #[test]
    fn raw_exif_requires_a_complete_tiff_directory() {
        let exif = b"Exif\0\0II*\0\x08\0\0\0\0\0\0\0\0\0";
        let mut source = TestImage::rgb8(16, 8);
        let parasite = |packet: &[u8]| {
            let mut record = super::super::tests::W { out: Vec::new(), wide: true };
            record.string("exif-data");
            record.u32(0);
            record.u32(packet.len() as u32);
            record.out.extend_from_slice(packet);
            (21, record.out)
        };
        source.image_props.push(parasite(exif));
        let imported = import("valid exif", &source.write(), &Interrupt::NONE).unwrap();
        assert_eq!(imported.document.metadata.exif.as_deref().map(Vec::as_slice), Some(&exif[6..]));
        source.image_props.push(parasite(&exif[..exif.len() - 1]));
        let imported = import("truncated exif", &source.write(), &Interrupt::NONE).unwrap();
        assert!(imported.document.metadata.exif.is_none());
        assert!(imported.warnings.iter().any(|note| note.contains("EXIF packet is invalid")));
    }

    #[test]
    fn pre_cancelled_metadata_apply_keeps_resolution_metadata_and_warnings_unchanged() {
        let mut document = Document::new("Original", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        document.metadata.text.push(("Original note".into(), "Retain me".into()));
        let metadata = document.metadata.clone();
        let resolution = document.resolution_dpi;
        let source = ImageMetadata::default();
        let mut warnings = Warnings::default();
        let cancel = || true;
        let ctl = Interrupt::cancel_only(&cancel);
        assert!(matches!(apply(&mut document, &source, (300., 150.), 26, &mut warnings, &ctl), Err(IoError::Cancelled)));
        assert_eq!(document.resolution_dpi, resolution);
        assert_eq!(document.metadata, metadata);
        assert!(warnings.finish().is_empty());

        // The same empty-guide source still applies its real DPI and disclosure
        // when permitted: the cancellation test cannot pass through a no-op.
        let mut warnings = Warnings::default();
        apply(&mut document, &source, (300., 150.), 26, &mut warnings, &Interrupt::NONE).unwrap();
        assert_eq!(document.resolution_dpi, 300.);
        assert!(!warnings.finish().is_empty());
    }
}
