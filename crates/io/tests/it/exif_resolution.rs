//! #1691: the EXIF resolution of an imported photo becomes the document's, and a PSD save keeps
//! its EXIF copy in step with the ResolutionInfo resource.

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, Metadata, SampleType, exif_resolution};
use photocraft_io::*;

/// Big-endian EXIF: XResolution 300/1, YResolution 300/1, ResolutionUnit 2.
fn camera_exif() -> Vec<u8> {
    let mut v = b"MM\0*".to_vec();
    v.extend_from_slice(&8u32.to_be_bytes());
    v.extend_from_slice(&3u16.to_be_bytes());
    let data = 8 + 2 + 3 * 12 + 4;
    for (tag, ty, value) in [(282u16, 5u16, data as u32), (283, 5, data as u32 + 8)] {
        v.extend_from_slice(&tag.to_be_bytes());
        v.extend_from_slice(&ty.to_be_bytes());
        v.extend_from_slice(&1u32.to_be_bytes());
        v.extend_from_slice(&value.to_be_bytes());
    }
    v.extend_from_slice(&296u16.to_be_bytes());
    v.extend_from_slice(&3u16.to_be_bytes());
    v.extend_from_slice(&1u32.to_be_bytes());
    v.extend_from_slice(&[0, 2, 0, 0]);
    v.extend_from_slice(&0u32.to_be_bytes());
    for _ in 0..2 {
        v.extend_from_slice(&300u32.to_be_bytes());
        v.extend_from_slice(&1u32.to_be_bytes());
    }
    v
}

#[test]
fn webp_with_exif_resolution_imports_at_it_and_psd_save_follows_the_document() {
    let meta = Metadata { exif: Some(camera_exif()), ..Default::default() };
    let img = Image::from_raw(6, 4, ChannelLayout::Rgb, SampleType::U8, vec![50; 72]).unwrap().with_meta(meta);
    let webp = photocraft_codecs::encode(&img, Format::WebP, &EncodeOptions::default()).unwrap();
    let mut doc = import("photo.webp", &webp).unwrap().document;
    assert_eq!(doc.resolution_dpi, 300.0);

    doc.resolution_dpi = 240.0;
    let psd = export(&doc, "photo.psd", &ExportOptions::default()).unwrap().bytes;
    let back = import("photo.psd", &psd).unwrap().document;
    assert_eq!(back.resolution_dpi, 240.0);
    assert_eq!(back.metadata.exif.as_deref().and_then(|e| exif_resolution(e)), Some((240.0, 240.0)));
}

/// A 300 ppi document opened from a TIFF of `layout` and `sample`, with EXIF and XMP.
fn tagged_document(layout: ChannelLayout, sample: SampleType) -> photocraft_doc::Document {
    let n = 6 * 4 * layout.channels();
    let img = Image::from_raw(6, 4, layout, SampleType::U8, (0..n).map(|i| (i * 11) as u8).collect()).unwrap().convert(layout, sample);
    let tiff = photocraft_codecs::encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
    let mut doc = import("source.tif", &tiff).unwrap().document;
    doc.resolution_dpi = 300.0;
    doc.metadata.exif = Some(camera_exif().into());
    doc.metadata.xmp = Some("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>".into());
    doc
}

/// #10105: Export As with Metadata "None" leaves out EXIF and XMP but keeps the document
/// resolution, in every model and depth.
#[test]
fn export_with_metadata_off_keeps_the_document_resolution() {
    use ChannelLayout::*;
    use SampleType::*;
    let opts = ExportOptions { encode: EncodeOptions { embed_metadata: false, ..Default::default() }, xmp: XmpEmbed::None, ..Default::default() };
    let docs = [(Gray, U8), (Rgb, U8), (Rgb, U16), (Cmyk, U8), (Rgb, F32)];
    for (layout, sample) in docs {
        let doc = tagged_document(layout, sample);
        let exts: &[&str] = if layout == Cmyk { &["jpg", "tif"] } else { &["jpg", "png", "tif"] };
        for ext in exts {
            let what = format!("{layout:?} {sample:?} → {ext}");
            let back = photocraft_codecs::decode(&export(&doc, ext, &opts).unwrap().bytes).unwrap();
            assert_eq!(back.meta.dpi, Some((300.0, 300.0)), "{what}");
            assert!(back.meta.xmp.is_none(), "{what}");
            // CMYK JPEG holds the resolution in an EXIF block without the source's tags.
            assert!(back.meta.exif.is_none() || (layout, *ext) == (Cmyk, "jpg"), "{what}");
        }
    }
}

/// The palette PNG an Indexed Color document exports writes the resolution with Metadata "None".
#[test]
fn indexed_png_with_metadata_off_keeps_the_document_resolution() {
    let pal = [[10, 20, 30], [200, 100, 50], [7, 8, 9]];
    let png = photocraft_codecs::encode_png_indexed(2, 2, &[1, 0, 2, 1], &pal, None, None, None).unwrap();
    let mut doc = import("indexed.png", &png).unwrap().document;
    assert_eq!(doc.mode, photocraft_color::ColorMode::Indexed);
    doc.resolution_dpi = 300.0;
    let opts = ExportOptions { encode: EncodeOptions { embed_metadata: false, ..Default::default() }, xmp: XmpEmbed::None, ..Default::default() };
    let back = photocraft_codecs::decode(&export(&doc, "png", &opts).unwrap().bytes).unwrap();
    assert_eq!(back.meta.dpi, Some((300.0, 300.0)));
}
