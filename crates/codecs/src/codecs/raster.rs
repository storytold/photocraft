//! DDS, DICOM, FITS, IFF/ILBM, PCX, PNM and XPM: the format code is craft-libs'
//! `craft-raster-codecs`. This maps its image, limits and errors onto this crate's, and the
//! encoders take this crate's [`Plan`] (the layout and depth conversion stays here).

use craft_raster_codecs as rc;

use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::format::Format;
use crate::image::{ChannelLayout, DecodeWarning, Image, SampleType};
use crate::options::Limits;

fn limits(l: &Limits) -> rc::Limits {
    rc::Limits { max_width: l.max_width, max_height: l.max_height, max_pixels: l.max_pixels, max_alloc: l.max_alloc }
}

fn err(format: Format, e: rc::CodecError) -> CodecError {
    match e {
        rc::CodecError::Unsupported { reason, .. } => CodecError::Unsupported { format, reason },
        rc::CodecError::Malformed { message, .. } => CodecError::Malformed { format, message },
        rc::CodecError::LimitExceeded(m) => CodecError::LimitExceeded(m),
        rc::CodecError::Encode { message, .. } => CodecError::Encode { format, message },
        rc::CodecError::InvalidImage(m) => CodecError::InvalidImage(m),
    }
}

/// A decoded image as this crate's.
fn from_rc(format: Format, img: rc::Image) -> Result<Image, CodecError> {
    let layout = match img.layout() {
        rc::ChannelLayout::Gray => ChannelLayout::Gray,
        rc::ChannelLayout::GrayA => ChannelLayout::GrayA,
        rc::ChannelLayout::Rgb => ChannelLayout::Rgb,
        rc::ChannelLayout::Rgba => ChannelLayout::Rgba,
        rc::ChannelLayout::Cmyk => ChannelLayout::Cmyk,
        rc::ChannelLayout::CmykA => ChannelLayout::CmykA,
    };
    let sample = match img.sample_type() {
        rc::SampleType::U8 => SampleType::U8,
        rc::SampleType::U16 => SampleType::U16,
        rc::SampleType::F32 => SampleType::F32,
    };
    let (w, h) = img.dimensions();
    let (dpi, text) = (img.dpi, img.text.clone());
    let warnings: Vec<DecodeWarning> = img.warnings.iter().map(|rc::DecodeWarning::MorePages { total }| DecodeWarning::MorePages { total: *total }).collect();
    let mut out = Image::from_raw(w, h, layout, sample, img.into_data()).map_err(|e| CodecError::malformed(format, e))?;
    out.meta.dpi = dpi;
    out.meta.text = text;
    out.warnings = warnings;
    Ok(out)
}

/// `image` converted as `plan` says, as the shared crate's image (with its resolution and text).
fn to_rc(format: Format, image: &Image, plan: Plan) -> Result<rc::Image, CodecError> {
    let img = image.converted(plan.layout, plan.sample);
    let layout = match img.layout() {
        ChannelLayout::Gray => rc::ChannelLayout::Gray,
        ChannelLayout::GrayA => rc::ChannelLayout::GrayA,
        ChannelLayout::Rgb => rc::ChannelLayout::Rgb,
        ChannelLayout::Rgba => rc::ChannelLayout::Rgba,
        ChannelLayout::Cmyk => rc::ChannelLayout::Cmyk,
        ChannelLayout::CmykA => rc::ChannelLayout::CmykA,
        l => return Err(CodecError::encode(format, format!("cannot store {l:?}"))),
    };
    let sample = match img.sample_type() {
        SampleType::U8 => rc::SampleType::U8,
        SampleType::U16 => rc::SampleType::U16,
        SampleType::F32 => rc::SampleType::F32,
        s => return Err(CodecError::encode(format, format!("unsupported sample {s:?}"))),
    };
    let (w, h) = img.dimensions();
    let mut out = rc::Image::from_raw(w, h, layout, sample, img.data().to_vec()).map_err(|e| err(format, e))?;
    out.dpi = img.meta.dpi;
    out.text = img.meta.text.clone();
    Ok(out)
}

pub(crate) mod dds {
    use super::{CodecError, Format, Image, Limits, Plan, SampleType, err, from_rc, limits, rc, to_rc};
    use crate::options::EncodeOptions;
    const F: Format = Format::Dds;

    pub(crate) fn has_additional_images(bytes: &[u8]) -> Result<bool, CodecError> {
        rc::dds::has_additional_images(bytes).map_err(|e| err(F, e))
    }
    pub(crate) fn decode(b: &[u8], l: &Limits) -> Result<Image, CodecError> {
        from_rc(F, rc::dds::decode(b, &limits(l)).map_err(|e| err(F, e))?)
    }
    pub(crate) fn encode(image: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
        let img = to_rc(F, image, Plan { sample: SampleType::U8, ..plan })?;
        rc::dds::encode(&img, opts.dds).map_err(|e| err(F, e))
    }
}

pub(crate) mod dicom {
    use super::{CodecError, Format, Image, Limits, err, from_rc, limits, rc};
    const F: Format = Format::Dicom;

    pub(crate) fn is_dicom(b: &[u8]) -> bool {
        rc::dicom::is_dicom(b)
    }
    pub(crate) fn decode(b: &[u8], l: &Limits) -> Result<Image, CodecError> {
        from_rc(F, rc::dicom::decode(b, &limits(l)).map_err(|e| err(F, e))?)
    }
}

pub(crate) mod fits {
    use super::{CodecError, Format, Image, Limits, Plan, err, from_rc, limits, rc, to_rc};
    use crate::options::EncodeOptions;
    const F: Format = Format::Fits;

    pub(crate) fn has_additional_images(bytes: &[u8]) -> Result<bool, CodecError> {
        rc::fits::has_additional_images(bytes).map_err(|e| err(F, e))
    }
    pub(crate) fn decode(b: &[u8], l: &Limits) -> Result<Image, CodecError> {
        from_rc(F, rc::fits::decode(b, &limits(l)).map_err(|e| err(F, e))?)
    }
    pub(crate) fn encode(image: &Image, plan: Plan, _opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
        rc::fits::encode(&to_rc(F, image, plan)?).map_err(|e| err(F, e))
    }
}

pub(crate) mod iff {
    use super::{CodecError, Format, Image, Limits, Plan, err, from_rc, limits, rc, to_rc};
    const F: Format = Format::Iff;

    pub(crate) fn looks_like_iff(b: &[u8]) -> bool {
        rc::iff::looks_like_iff(b)
    }
    pub(crate) fn decode(b: &[u8], l: &Limits) -> Result<Image, CodecError> {
        from_rc(F, rc::iff::decode(b, &limits(l)).map_err(|e| err(F, e))?)
    }
    pub(crate) fn encode(image: &Image, plan: Plan) -> Result<Vec<u8>, CodecError> {
        rc::iff::encode(&to_rc(F, image, plan)?).map_err(|e| err(F, e))
    }
}

pub(crate) mod pcx {
    use super::{CodecError, Format, Image, Limits, err, from_rc, limits, rc};
    const F: Format = Format::Pcx;

    pub(crate) fn looks_like_pcx(b: &[u8]) -> bool {
        rc::pcx::looks_like_pcx(b)
    }
    pub(crate) fn decode(b: &[u8], l: &Limits) -> Result<Image, CodecError> {
        from_rc(F, rc::pcx::decode(b, &limits(l)).map_err(|e| err(F, e))?)
    }
}

pub(crate) mod pnm {
    use super::{CodecError, Format, Image, Limits, Plan, err, from_rc, limits, rc, to_rc};
    use crate::options::EncodeOptions;
    const F: Format = Format::Pnm;

    pub(crate) fn decode(b: &[u8], l: &Limits) -> Result<Image, CodecError> {
        from_rc(F, rc::pnm::decode(b, &limits(l)).map_err(|e| err(F, e))?)
    }
    pub(crate) fn encode(image: &Image, plan: Plan, _opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
        rc::pnm::encode(&to_rc(F, image, plan)?).map_err(|e| err(F, e))
    }
}

pub(crate) mod xpm {
    use super::{CodecError, Format, Image, Limits, err, from_rc, limits, rc};
    const F: Format = Format::Xpm;

    pub(crate) fn decode(b: &[u8], l: &Limits) -> Result<Image, CodecError> {
        from_rc(F, rc::xpm::decode(b, &limits(l)).map_err(|e| err(F, e))?)
    }
}
