//! Layer-preserving Photoshop PDF import.
//!
//! Photoshop stores its editable layer section in a page's `/PieceInfo` →
//! `/AdobePhotoshop` → `/Private` dictionary.  The bytes are TIFF-style
//! `ImageSourceData` tagged blocks, rather than a complete PSD file, so this
//! module supplies the header, image resources and composite metadata that
//! PSD's importer normally reads from the surrounding file.
//!
//! Auto and Layers modes limit the source PDF to 512 MiB before loading it.
//! Explicit Render mode uses the renderer's own resource guards instead.
//! A valid AdobePhotoshop dictionary without the optional Private entry has
//! no preserved layers; a present but malformed Private entry is corruption.

use std::io::Read as _;

use flate2::read::ZlibDecoder;
use hayro::hayro_syntax::object::{Array as HayroArray, Dict as HayroDict, Name as HayroName, Stream as HayroStream};
use hayro::hayro_syntax::page::Page;
use photocraft_psd::tiff::{ImageSourceData, SIGNATURE, resources_from_bytes};
use photocraft_psd::{ColorMode, Compression, Header, ImageData};
use photocraft_raster::Interrupt;

use crate::psd_import::{check_group_depth, psd_to_document_owned};
use crate::{ImportResult, IoError};

/// Largest private Photoshop stream accepted from a PDF.  This cap includes
/// the decoded layer section: it prevents a small Flate stream from creating
/// an unbounded allocation before the PSD parser gets a chance to validate it.
const MAX_PRIVATE_BYTES: usize = 512 << 20;
const MAX_PDF_BYTES: usize = 512 << 20;

/// Outcome of probing one PDF page for Photoshop's private layer section.
///
/// [`Self::Absent`] is deliberately narrow: it means the page has no
/// Photoshop preservation entry. Every other failure is terminal in
/// [`photocraft_doc::PdfImportMode::Auto`], so an unreadable or damaged private section is not
/// silently replaced with a raster image.
#[derive(Debug)]
pub(crate) enum LayeredPdfError {
    /// The page has no Photoshop preservation entry.
    Absent,
    /// The PDF itself cannot be read, its input/page selection is invalid, or
    /// a PDF or Photoshop-private stream exceeds an import limit.
    Pdf(String),
    Cancelled,
    /// A typed admission, source-read or allocation failure from the PSD conversion.
    Import(IoError),
    /// Photoshop preservation metadata was present but is invalid or
    /// unsupported.
    Corrupt(String),
}

impl LayeredPdfError {
    /// Wraps a PDF input, selection, or resource-limit error message.
    fn pdf(message: impl Into<String>) -> Self {
        Self::Pdf(message.into())
    }

    /// Wraps an error describing invalid or unsupported Photoshop preservation data.
    fn corrupt(message: impl Into<String>) -> Self {
        Self::Corrupt(message.into())
    }

    /// Converts probe failures to import errors, preserving typed import errors and explaining explicit render-mode recovery.
    pub(crate) fn into_io(self) -> IoError {
        match self {
            Self::Absent => IoError::Pdf("this PDF has no Photoshop editable layer data; choose PDF render mode".into()),
            Self::Pdf(message) => IoError::Pdf(message),
            Self::Cancelled => IoError::Cancelled,
            Self::Import(error) => error,
            Self::Corrupt(message) => IoError::Pdf(format!(
                "the Photoshop editable layer data in this PDF is corrupt or unsupported ({message}); choose PDF render mode to open its page appearance"
            )),
        }
    }
}

/// Probes the selected page for Photoshop-preserved layers. Callers may fall
/// back only when this reports [`LayeredPdfError::Absent`].
pub(crate) fn import_page(name: &str, page: &Page<'_>, options: &crate::ImportOptions, ctl: &Interrupt) -> Result<ImportResult, LayeredPdfError> {
    ctl.check().map_err(|_| LayeredPdfError::Cancelled)?;
    layered_page(page, options, ctl).map(|(mut document, warnings)| {
        document.name = name.to_owned();
        ImportResult { document, warnings, source_read_only: true, preview_only: false }
    })
}

/// Loads a size-checked PDF and imports a one-based page's preserved layers with default import settings.
#[cfg(test)]
fn layered_result(name: &str, bytes: &[u8], page: usize, password: Option<&str>, ctl: &Interrupt) -> Result<ImportResult, LayeredPdfError> {
    ctl.check().map_err(|_| LayeredPdfError::Cancelled)?;
    validate_pdf_size_len(bytes.len(), photocraft_doc::PdfImportMode::Layers)?;
    let pdf = crate::pdf_import::load(bytes, password).map_err(|error| LayeredPdfError::pdf(error.to_string()))?;
    let page = pdf
        .pages()
        .get(page.checked_sub(1).ok_or_else(|| LayeredPdfError::pdf("PDF page numbers start at 1"))?)
        .ok_or_else(|| LayeredPdfError::pdf(format!("PDF has {} page(s); page {page} was requested", pdf.pages().len())))?;
    import_page(name, page, &crate::ImportOptions::default(), ctl)
}

/// Reconstructs an editable document from Photoshop private streams after validation and PSD memory admission.
/// Returns combined import warnings, rejecting damaged editable channels rather than substituting a rendered page.
fn layered_page(page: &Page<'_>, options: &crate::ImportOptions, ctl: &Interrupt) -> Result<(photocraft_doc::Document, Vec<String>), LayeredPdfError> {
    let private = photoshop_private_hayro(page)?;
    ctl.check().map_err(|_| LayeredPdfError::Cancelled)?;
    let mut image_source = decode_hayro_stream(private_stream_hayro(&private, b"StandardImageFileData")?, "StandardImageFileData")?;
    validate_layer_block(&image_source)?;
    image_source.try_reserve(SIGNATURE.len()).map_err(|_| LayeredPdfError::corrupt("not enough memory for Photoshop layer data"))?;
    drop(image_source.splice(..0, SIGNATURE.iter().copied()));
    let (layers, mut warnings) =
        ImageSourceData::from_bytes(&image_source).map_err(|error| LayeredPdfError::corrupt(format!("could not parse StandardImageFileData: {error}")))?;
    drop(image_source);
    let mut resources = if private.contains_key(b"ImageResources") {
        let data = decode_hayro_stream(private_stream_hayro(&private, b"ImageResources")?, "ImageResources")?;
        let (resources, resource_warnings) =
            resources_from_bytes(&data).map_err(|error| LayeredPdfError::corrupt(format!("could not parse ImageResources: {error}")))?;
        warnings.extend(resource_warnings.into_iter().map(|warning| format!("image resources: {warning}")));
        resources
    } else {
        Vec::new()
    };
    let composite = private_stream_hayro(&private, b"CompositeImage")?;
    let header = composite_header_hayro(&composite)?;
    if !resources.iter().any(|resource| resource.id == photocraft_psd::resources::ids::ICC_PROFILE)
        && let Some(profile) = composite_icc_profile(&composite)?
    {
        resources.push(photocraft_psd::ImageResource::new(photocraft_psd::resources::ids::ICC_PROFILE, profile));
    }
    let file = layers.into_psd(header, resources, ImageData::new(Compression::Raw, Vec::new()));
    check_group_depth(&file).map_err(|error| LayeredPdfError::corrupt(error.to_string()))?;
    ctl.check().map_err(|_| LayeredPdfError::Cancelled)?;
    let estimate = crate::psd_budget::estimate(&file).map_err(LayeredPdfError::Import)?;
    let _admission = crate::admit_psd(&estimate, ctl, options).map_err(LayeredPdfError::Import)?;
    let txt2 = file.global_blocks.iter().find(|block| &block.key == b"Txt2").and_then(|block| photocraft_text::psd::parse_txt2(&block.data));
    let (mut document, psd_warnings, damaged, fatal_error) = psd_to_document_owned(file, ctl, None, true).ok_or(LayeredPdfError::Cancelled)?;
    if let Some(error) = fatal_error {
        return Err(LayeredPdfError::Import(error));
    }
    if damaged {
        return Err(LayeredPdfError::corrupt("one or more editable layer channels could not be decoded"));
    }
    crate::text_import::prepare(&mut document, txt2.as_ref());
    warnings.extend(psd_warnings);
    Ok((document, warnings))
}

/// Finds the page's Photoshop Private dictionary, distinguishing absent preservation entries from malformed dictionaries.
fn photoshop_private_hayro<'a>(page: &Page<'a>) -> Result<HayroDict<'a>, LayeredPdfError> {
    let raw = page.raw();
    if !raw.contains_key(b"PieceInfo") {
        return Err(LayeredPdfError::Absent);
    }
    let piece = raw.get::<HayroDict<'_>>(b"PieceInfo").ok_or_else(|| LayeredPdfError::corrupt("invalid PieceInfo dictionary"))?;
    if !piece.contains_key(b"AdobePhotoshop") {
        return Err(LayeredPdfError::Absent);
    }
    let adobe = piece.get::<HayroDict<'_>>(b"AdobePhotoshop").ok_or_else(|| LayeredPdfError::corrupt("invalid AdobePhotoshop dictionary"))?;
    if !adobe.contains_key(b"Private") {
        return Err(LayeredPdfError::Absent);
    }
    adobe.get::<HayroDict<'_>>(b"Private").ok_or_else(|| LayeredPdfError::corrupt("invalid Photoshop Private dictionary"))
}

/// Retrieves a required Photoshop private stream, reporting missing or invalid entries as corrupt preservation data.
fn private_stream_hayro<'a>(private: &HayroDict<'a>, key: &[u8]) -> Result<HayroStream<'a>, LayeredPdfError> {
    private.get::<HayroStream<'_>>(key).ok_or_else(|| LayeredPdfError::corrupt(format!("missing {key:?} in Photoshop Private data")))
}

/// Reads a size-bounded private stream, accepting only unfiltered bytes or a single Flate filter.
fn decode_hayro_stream(stream: HayroStream<'_>, label: &str) -> Result<Vec<u8>, LayeredPdfError> {
    let raw = stream.raw_data();
    if raw.len() > MAX_PRIVATE_BYTES {
        return Err(LayeredPdfError::pdf(format!("{label} compressed data exceeds the {} MiB import limit", MAX_PRIVATE_BYTES >> 20)));
    }
    match stream.filters().as_slice() {
        [] => bounded_copy(&raw, label),
        [hayro::hayro_syntax::Filter::FlateDecode] => inflate_bounded(&raw, label),
        _ => Err(LayeredPdfError::corrupt(format!("{label} uses unsupported PDF stream filters"))),
    }
}

/// Builds and validates a PSD header from composite dimensions, component depth, and supported device or ICC colour space.
fn composite_header_hayro(composite: &HayroStream<'_>) -> Result<Header, LayeredPdfError> {
    let dict = composite.dict();
    let integer = |key: &[u8]| dict.get::<i64>(key).ok_or_else(|| LayeredPdfError::corrupt(format!("CompositeImage {key:?} is invalid")));
    let width = integer(b"Width")?;
    let height = integer(b"Height")?;
    let depth = u16::try_from(integer(b"BitsPerComponent")?).map_err(|_| LayeredPdfError::corrupt("CompositeImage BitsPerComponent is out of range"))?;
    if !matches!(depth, 8 | 16 | 32) {
        return Err(LayeredPdfError::corrupt(format!("CompositeImage has unsupported {depth}-bit components")));
    }
    let color_name = dict.get::<HayroName<'_>>(b"ColorSpace");
    let channels = match color_name.as_ref().map(|name| name.as_ref()) {
        Some(b"DeviceGray") => 1,
        Some(b"DeviceRGB") => 3,
        Some(b"DeviceCMYK") => 4,
        _ => composite_icc_channels(composite)?,
    };
    let mode = match channels {
        1 => ColorMode::Grayscale,
        3 => ColorMode::Rgb,
        4 => ColorMode::Cmyk,
        _ => return Err(LayeredPdfError::corrupt(format!("CompositeImage has unsupported {channels}-channel colour space"))),
    };
    let header = Header::new(photocraft_psd::Version::Psd, positive_u32(width, "Width")?, positive_u32(height, "Height")?, channels, depth, mode);
    header.validate().map_err(|error| LayeredPdfError::corrupt(error.to_string()))?;
    Ok(header)
}

/// Retrieves the profile stream from an ICCBased composite colour space, rejecting other array forms.
fn composite_icc_stream<'a>(composite: &HayroStream<'a>) -> Result<HayroStream<'a>, LayeredPdfError> {
    let color =
        composite.dict().get::<HayroArray<'_>>(b"ColorSpace").ok_or_else(|| LayeredPdfError::corrupt("CompositeImage uses an unsupported colour space"))?;
    let mut entries = color.flex_iter();
    if entries.next::<HayroName<'_>>().is_none_or(|name| name.as_ref() != b"ICCBased") {
        return Err(LayeredPdfError::corrupt("CompositeImage uses an unsupported colour space"));
    }
    entries.next::<HayroStream<'_>>().ok_or_else(|| LayeredPdfError::corrupt("ICCBased CompositeImage has no profile"))
}

/// Reads the composite ICC profile's declared channel count as an unsigned 16-bit value.
fn composite_icc_channels(composite: &HayroStream<'_>) -> Result<u16, LayeredPdfError> {
    let profile = composite_icc_stream(composite)?;
    let count = profile.dict().get::<i64>(b"N").ok_or_else(|| LayeredPdfError::corrupt("CompositeImage ICC profile channel count is invalid"))?;
    u16::try_from(count).map_err(|_| LayeredPdfError::corrupt("CompositeImage ICC profile channel count is out of range"))
}

/// Decodes the composite's bounded ICC profile when its colour space is an array; returns `None` for non-array spaces.
fn composite_icc_profile(composite: &HayroStream<'_>) -> Result<Option<Vec<u8>>, LayeredPdfError> {
    if composite.dict().get::<HayroArray<'_>>(b"ColorSpace").is_none() {
        return Ok(None);
    }
    let profile = composite_icc_stream(composite)?;
    decode_hayro_stream(profile, "CompositeImage ICC profile").map(Some)
}

/// Validates the preservation-path input size without inspecting the bytes,
/// keeping the allocation boundary and Render exemption testable without a
/// huge PDF. Call before loading; Render retains its own resource guards.
pub(crate) fn validate_pdf_size_len(len: usize, mode: photocraft_doc::PdfImportMode) -> Result<(), LayeredPdfError> {
    if mode != photocraft_doc::PdfImportMode::Render && len > MAX_PDF_BYTES {
        return Err(LayeredPdfError::pdf(format!("PDF is larger than the {} MiB import limit", MAX_PDF_BYTES >> 20)));
    }
    Ok(())
}

/// The first Photoshop tagged block is the layer-info `Lr16`/`Layr` block.
/// Its declared payload must be bounded before the PSD parser walks records.
fn validate_layer_block(data: &[u8]) -> Result<(), LayeredPdfError> {
    let header = data.get(..12).ok_or_else(|| LayeredPdfError::corrupt("StandardImageFileData is shorter than a Photoshop tagged-block header"))?;
    if &header[..4] != b"8BIM" {
        return Err(LayeredPdfError::corrupt("StandardImageFileData does not start with a Photoshop tagged block"));
    }
    if !matches!(&header[4..8], b"Layr" | b"Lr16" | b"Lr32") {
        return Err(LayeredPdfError::corrupt("StandardImageFileData does not start with a Photoshop layer block"));
    }
    let declared = usize::try_from(u32::from_be_bytes([header[8], header[9], header[10], header[11]]))
        .map_err(|_| LayeredPdfError::corrupt("layer block length is out of range"))?;
    if declared > MAX_PRIVATE_BYTES {
        return Err(LayeredPdfError::pdf("StandardImageFileData layer block exceeds the import limit"));
    }
    if declared > data.len().saturating_sub(12) {
        return Err(LayeredPdfError::corrupt("StandardImageFileData declares a layer block outside its stream bounds"));
    }
    Ok(())
}

/// Copies uncompressed private data only when it fits the private-stream byte limit.
fn bounded_copy(data: &[u8], label: &str) -> Result<Vec<u8>, LayeredPdfError> {
    if data.len() > MAX_PRIVATE_BYTES {
        return Err(LayeredPdfError::pdf(format!("{label} exceeds the {} MiB import limit", MAX_PRIVATE_BYTES >> 20)));
    }
    Ok(data.to_vec())
}

/// Inflates at most one byte beyond the private-data limit, so reaching the
/// cap proves the stream is oversized without retaining an unbounded result.
fn inflate_bounded(data: &[u8], label: &str) -> Result<Vec<u8>, LayeredPdfError> {
    let mut decoder = ZlibDecoder::new(data);
    let mut decoded = Vec::new();
    decoded.try_reserve(data.len().min(MAX_PRIVATE_BYTES)).map_err(|_| LayeredPdfError::pdf(format!("not enough memory to decode {label}")))?;
    let mut limited = decoder.by_ref().take(u64::try_from(MAX_PRIVATE_BYTES.saturating_add(1)).unwrap_or(u64::MAX));
    limited.read_to_end(&mut decoded).map_err(|error| LayeredPdfError::corrupt(format!("could not inflate {label}: {error}")))?;
    if decoded.len() > MAX_PRIVATE_BYTES {
        return Err(LayeredPdfError::pdf(format!("{label} decoded data exceeds the {} MiB import limit", MAX_PRIVATE_BYTES >> 20)));
    }
    Ok(decoded)
}

/// Converts a composite dimension to a nonzero 32-bit unsigned integer, reporting invalid values as corrupt data.
fn positive_u32(value: i64, label: &str) -> Result<u32, LayeredPdfError> {
    u32::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| LayeredPdfError::corrupt(format!("CompositeImage {label} must be a positive 32-bit integer")))
}

#[cfg(test)]
mod tests;
