//! PDF import, as Photoshop's File › Open on a PDF (its Import PDF dialog): one page rasterised
//! into a transparent `Layer 1` at the chosen resolution, colour mode and depth, cropped to one of
//! the page's boxes or to the bounding box of its marks. Pages are interpreted and rasterised by
//! `hayro` (pure Rust, on the CPU).
//!
//! What Photoshop 25.4 does was measured on synthetic PDFs (page boxes, rotation, the bounding box
//! of fills, strokes, text and images, device colour spaces, optional content, annotations) and
//! is followed here:
//!
//! * pixel size = round-half-up(box in points × ppi / 72) per axis; `/Rotate` turns the page,
//!   `/UserUnit` is ignored;
//! * a page box missing from the file falls back as the PDF specification says (Bleed, Trim and
//!   Art boxes to the crop box, the crop box to the media box), every box is cut to the media box;
//! * the bounding box is the geometric extent of every mark, painted or not (white fills, fully
//!   transparent fills and invisible text count), cut by the clip in force and by the crop box;
//!   strokes grow their path bounds by max(line width / 2, 4) + 2 points on each side; text uses a
//!   generous box around each glyph origin (see [`GLYPH_BOX`]); an empty page is its crop box;
//! * a one-page PDF whose only mark is a single image (a scan, a photo wrapped as a PDF) opens as
//!   that image: its own pixels on a `Background` layer, resolution from its placed width, Gray
//!   or RGB as the image is, every option ignored;
//! * the document is named after the file (one page: the stem; several: `stem-N.pdf`), has no
//!   embedded profile (the working space applies), and isn't tied to the file for Save;
//! * DeviceCMYK converts like Photoshop's U.S. Web Coated (SWOP) v2 (hayro's compact CMYK profile
//!   is within one level of it); DeviceGray differs: Photoshop reads it through Dot Gain 20 %,
//!   hayro linearly (a mid grey is 149 there and 128 here).
//!
//! Known gaps: Media Box shows nothing outside the crop box (hayro clips to it); `NoView`
//! annotations are drawn; non-embedded fonts other than the standard 14 fall back to hayro's
//! base-14 stand-ins; Anti-aliased off settles coverage and opacity under one half as out.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::{
    BlendMode, CacheKey, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageData, ImageDrawProps, InterpreterCache, InterpreterSettings, LumaData,
    SoftMask, interpret_page,
};
use hayro::hayro_syntax::content::TypedIter;
use hayro::hayro_syntax::content::ops::TypedInstruction;
use hayro::hayro_syntax::object::dict::keys;
use hayro::hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier, Stream};
use hayro::hayro_syntax::page::{Page, Resources, Rotation};
use hayro::hayro_syntax::{Filter, LoadPdfError, Pdf};
use hayro::kurbo::{Affine, BezPath, Rect as PtRect, Shape};
use hayro::vello_cpu::color::palette::css::TRANSPARENT;
use hayro::vello_cpu::peniko::ImageAlphaType;
use hayro::vello_cpu::{self, Pixmap, RenderContext};
use hayro::{RenderCache, RenderSettings, render_into};
use photocraft_cms::{Builtin, Intent, Transform};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, PdfBox, PdfImportMode, PdfImportOptions};
use photocraft_geom::Size;
use photocraft_raster::{Interrupt, Surface};

use crate::{ImportResult, IoError};

/// Largest side a page opens at; a larger page is scaled down to fit, with a warning.
pub const MAX_SIDE: u32 = 16_384;
/// Most pixels a page opens with (a Letter page at 1200 ppi is 135 MP); past it the page is
/// scaled down, with a warning.
pub const MAX_PAGE_PIXELS: u64 = 64_000_000;
/// Most pixels one raster image, stencil, soft mask or explicit mask of a page may decode to,
/// as the file declares its size: an image past it refuses the page before anything is decoded.
pub const MAX_IMAGE_PIXELS: u64 = MAX_PAGE_PIXELS;
/// Most pixels every image a page's resources reach may decode to together (the interpreter
/// keeps decoded images while the page renders).
pub const MAX_PAGE_IMAGE_PIXELS: u64 = 4 * MAX_PAGE_PIXELS;
/// Most streams (image and Form XObjects, tiling patterns, Type 3 glyph procedures, annotation
/// appearances) a page's resource tree is walked through before it is refused as unboundable.
const MAX_XOBJECTS: usize = 10_000;
/// Most tiles a JPX codestream may split into: the decoder builds one structure per tile up
/// front, whatever the image size (a 64 MP image in 256 px tiles has 1024).
const MAX_JPX_TILES: u64 = 65_536;
/// Deepest nesting of Form XObjects the walk follows.
const MAX_FORM_DEPTH: usize = 32;
/// Most Form XObject draws a page's content may make, repeats included. The interpreter limits
/// how deep forms nest (50) but not how often one is drawn, so a form drawing its child twice,
/// 40 levels down, is 2^40 draws: the page would never finish interpreting or rendering.
const MAX_FORM_DRAWS: u64 = 100_000;
/// Most Form XObject draws one tiling pattern's cell may make. The renderer interprets the cell
/// again for every fill painted with the pattern, so this keeps a pattern used many times from
/// multiplying a large form tree by the number of fills.
const MAX_PATTERN_FORM_DRAWS: u64 = 1_000;
/// Most bytes one content stream (the page's contents, a form, a tiling pattern, a Type 3 glyph,
/// a soft-mask group, an appearance) may inflate to. Deflate reaches about 1000:1, so a few MB in
/// the file could otherwise ask for gigabytes when the stream is decoded.
const MAX_CONTENT_BYTES: u64 = 256 << 20;
/// Most dictionaries, arrays and streams [`codec_check`] walks through for one page.
const MAX_OBJECTS: usize = 200_000;
/// Deepest nesting of dictionaries and arrays [`codec_check`] follows.
const MAX_OBJECT_DEPTH: usize = 256;
/// The resolution the dialog starts at, as Photoshop's.
pub const DEFAULT_RESOLUTION: f32 = 300.0;
/// The resolution range the dialog accepts.
pub const MIN_RESOLUTION: f32 = 1.0;
pub const MAX_RESOLUTION: f32 = 30_000.0;
/// The error for a PDF that can't be read without its user password; the UI asks for one then.
pub const PASSWORD_REQUIRED: &str = "the PDF is protected by a password";
/// The error for a password the PDF doesn't accept.
pub const WRONG_PASSWORD: &str = "that isn't the PDF's password";

/// The box around one glyph, in glyph units (1000 per em), relative to its origin: left, bottom,
/// right, top. Photoshop's text bounds are not the ink but a generous box per glyph, as measured
/// on Helvetica and Times text ("H" at 48 pt → 166 × 85 pt; the right edge is always the last
/// glyph's origin + 2.41 em, whatever the glyph). Courier and embedded fonts measure differently
/// (their own font boxes); this one box stands for all of them.
pub const GLYPH_BOX: [f64; 4] = [-1050.0, -530.0, 2410.0, 1240.0];

/// `true` if `bytes` may start a PDF file: the `%PDF-` marker within the first kilobyte (the
/// specification allows leading bytes before it, and files do carry a byte-order mark or a
/// shell line there). Opening decides with [`crate::is_pdf_input`], which leaves a file that
/// doesn't start with the marker to the SVG and raster importers when they take it.
pub fn is_pdf(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(1024)).unwrap_or(bytes);
    head.windows(5).any(|w| w == b"%PDF-")
}

/// Photoshop's Crop To choice: which page box becomes the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CropTo {
    /// The smallest box around every mark on the page (the default).
    #[default]
    BoundingBox,
    MediaBox,
    CropBox,
    BleedBox,
    TrimBox,
    ArtBox,
}

impl CropTo {
    /// Every choice, in the dialog's order.
    pub const ALL: [CropTo; 6] = [CropTo::BoundingBox, CropTo::MediaBox, CropTo::CropBox, CropTo::BleedBox, CropTo::TrimBox, CropTo::ArtBox];

    /// The value in command params (`"crop":"boundingBox|mediaBox|cropBox|bleedBox|trimBox|artBox"`).
    pub fn id(self) -> &'static str {
        match self {
            CropTo::BoundingBox => "boundingBox",
            CropTo::MediaBox => "mediaBox",
            CropTo::CropBox => "cropBox",
            CropTo::BleedBox => "bleedBox",
            CropTo::TrimBox => "trimBox",
            CropTo::ArtBox => "artBox",
        }
    }

    /// The dialog label (English source string).
    pub fn label(self) -> &'static str {
        match self {
            CropTo::BoundingBox => "Bounding Box",
            CropTo::MediaBox => "Media Box",
            CropTo::CropBox => "Crop Box",
            CropTo::BleedBox => "Bleed Box",
            CropTo::TrimBox => "Trim Box",
            CropTo::ArtBox => "Art Box",
        }
    }

    /// The choice an id or label names, case-insensitively.
    pub fn from_id(s: &str) -> Option<CropTo> {
        let s = s.trim();
        CropTo::ALL.into_iter().find(|c| c.id().eq_ignore_ascii_case(s) || c.label().eq_ignore_ascii_case(s))
    }
}

/// What the Import PDF dialog asked for.
#[derive(Clone, PartialEq)]
pub struct PdfOpenOptions {
    /// Whether Photoshop's preserved tagged layers are opened when available.
    pub layer_mode: PdfImportMode,
    /// The page to open, counted from 1.
    pub page: usize,
    pub crop: CropTo,
    /// Pixels per inch of the document.
    pub resolution: f32,
    /// RGB, Grayscale, CMYK or Lab (anything else opens as RGB).
    pub mode: ColorMode,
    /// 8 or 16 bits per channel (anything else opens as 8).
    pub depth: SampleType,
    pub anti_alias: bool,
    /// The dialog's Name field: the document name's stem; the file's stem when `None`.
    pub name: Option<String>,
    /// The dialog's Width and Height, in pixels, when the user changed them: the page is scaled
    /// to this size at `resolution`. `None` sizes the page from its box and the resolution.
    pub pixel_size: Option<(u32, u32)>,
    /// The user password of an encrypted PDF.
    pub password: Option<String>,
    /// The dialog's Images choice: open raster image number N of the file (counted through the
    /// pages from 1) instead of a page; `page` and the page options are ignored then.
    pub image: Option<usize>,
    /// Photoshop's Suppress Warnings: open without import notes.
    pub suppress_warnings: bool,
}

/// Prints every option but the password (logs and error messages carry `{opts:?}`).
impl std::fmt::Debug for PdfOpenOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfOpenOptions")
            .field("page", &self.page)
            .field("crop", &self.crop)
            .field("layer_mode", &self.layer_mode)
            .field("resolution", &self.resolution)
            .field("mode", &self.mode)
            .field("depth", &self.depth)
            .field("anti_alias", &self.anti_alias)
            .field("name", &self.name)
            .field("pixel_size", &self.pixel_size)
            .field("password", &self.password.as_ref().map(|_| "…"))
            .field("image", &self.image)
            .field("suppress_warnings", &self.suppress_warnings)
            .finish()
    }
}

impl Default for PdfOpenOptions {
    fn default() -> Self {
        PdfOpenOptions {
            layer_mode: PdfImportMode::Auto,
            page: 1,
            crop: CropTo::BoundingBox,
            resolution: DEFAULT_RESOLUTION,
            mode: ColorMode::Rgb,
            depth: SampleType::U8,
            anti_alias: true,
            name: None,
            pixel_size: None,
            password: None,
            image: None,
            suppress_warnings: false,
        }
    }
}

impl PdfOpenOptions {
    /// The persistent PDF smart-object settings represented by this dialog.
    /// Password, display name and warning suppression are intentionally transient.
    pub fn projection(&self) -> PdfImportOptions {
        PdfImportOptions {
            mode: self.layer_mode,
            page: self.page,
            dpi: self.resolution,
            crop_box: match self.crop {
                CropTo::BoundingBox => PdfBox::BoundingBox,
                CropTo::MediaBox => PdfBox::Media,
                CropTo::CropBox => PdfBox::Crop,
                CropTo::BleedBox => PdfBox::Bleed,
                CropTo::TrimBox => PdfBox::Trim,
                CropTo::ArtBox => PdfBox::Art,
            },
            color_mode: self.mode,
            depth: self.depth,
            anti_alias: self.anti_alias,
            pixel_size: self.pixel_size,
            image: self.image,
        }
    }

    /// Reconstructs the persistent portion of an Import PDF dialog.
    pub fn from_projection(projection: PdfImportOptions) -> Self {
        PdfOpenOptions {
            layer_mode: projection.mode,
            page: projection.page,
            crop: match projection.crop_box {
                PdfBox::BoundingBox => CropTo::BoundingBox,
                PdfBox::Media => CropTo::MediaBox,
                PdfBox::Crop => CropTo::CropBox,
                PdfBox::Bleed => CropTo::BleedBox,
                PdfBox::Trim => CropTo::TrimBox,
                PdfBox::Art => CropTo::ArtBox,
            },
            resolution: projection.dpi,
            mode: projection.color_mode,
            depth: projection.depth,
            anti_alias: projection.anti_alias,
            name: None,
            pixel_size: projection.pixel_size,
            password: None,
            image: projection.image,
            suppress_warnings: false,
        }
    }
}

/// One page's boxes, in PDF points (`[x0, y0, x1, y1]`, y up), every one cut to the media box
/// and the missing ones filled in as the specification says, plus its `/Rotate`.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfPage {
    pub media: [f32; 4],
    pub crop: [f32; 4],
    pub bleed: [f32; 4],
    pub trim: [f32; 4],
    pub art: [f32; 4],
    /// 0, 90, 180 or 270 degrees clockwise.
    pub rotation: u16,
}

impl PdfPage {
    /// The page box `crop` names (the Bounding Box choice is [`bounding_box`]'s business: here
    /// it is the crop box, which bounds it).
    pub fn box_for(&self, crop: CropTo) -> [f32; 4] {
        match crop {
            CropTo::MediaBox => self.media,
            CropTo::CropBox | CropTo::BoundingBox => self.crop,
            CropTo::BleedBox => self.bleed,
            CropTo::TrimBox => self.trim,
            CropTo::ArtBox => self.art,
        }
    }

    /// Width and height in points of box `b` as the page is displayed: a page rotated by 90 or
    /// 270 degrees swaps them.
    pub fn displayed_size(&self, b: [f32; 4]) -> (f32, f32) {
        let (w, h) = ((b[2] - b[0]).max(0.0), (b[3] - b[1]).max(0.0));
        if self.rotation == 90 || self.rotation == 270 { (h, w) } else { (w, h) }
    }
}

/// What the dialog needs to know about a file before any page is rasterised.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfInfo {
    pub pages: Vec<PdfPage>,
}

/// A page rendered small, straight-alpha RGBA, for the dialog's thumbnails.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Pixel size of a `(width, height)` in points at `ppi`: Photoshop rounds half up on each axis
/// (A4 at 300 ppi is 2480 × 3508; 100.5 pt at 72 ppi is 101 px).
pub fn pixel_size(pt: (f32, f32), ppi: f32) -> (u32, u32) {
    let px = |v: f32| ((v * ppi / 72.0) + 0.5).floor().clamp(1.0, 1.0e9) as u32;
    (px(pt.0), px(pt.1))
}

/// Would `bytes` open as a plain image (a one-page PDF drawing nothing but one raster image)?
/// Photoshop opens such a file like an image, without its Import PDF dialog.
pub fn opens_as_image(bytes: &[u8], password: Option<&str>) -> bool {
    let Ok(pdf) = load(bytes, password) else { return false };
    if pdf.pages().len() != 1 {
        return false;
    }
    let Some(page) = pdf.pages().first() else { return false };
    // Interpreting the page decodes its content and fonts: only streams no image codec decodes.
    if codec_check(page).is_err() {
        return false;
    }
    let m = marks(page, None);
    m.paths == 0 && m.glyphs == 0 && m.stencils == 0 && m.images.len() == 1
}

/// Does opening `bytes` need a user password?
pub fn needs_password(bytes: &[u8]) -> bool {
    matches!(Pdf::new(Arc::new(bytes.to_vec())), Err(LoadPdfError::Decryption(_)))
}

/// The pages of a PDF: boxes and rotation of each.
pub fn info(bytes: &[u8], password: Option<&str>) -> Result<PdfInfo, IoError> {
    let pdf = load(bytes, password)?;
    let pages = pdf.pages().iter().map(page_info).collect();
    Ok(PdfInfo { pages })
}

/// The bounding box of page `page` (from 1), in PDF points of the unrotated page: the smallest
/// box around every mark, cut by the crop box; the crop box itself when the page draws nothing.
pub fn bounding_box(bytes: &[u8], page: usize, password: Option<&str>) -> Result<[f32; 4], IoError> {
    let pdf = load(bytes, password)?;
    let p = page_of(&pdf, page)?;
    let info = page_info(p);
    codec_check(p)?;
    Ok(marks(p, None).bounds_within(info.crop))
}

/// Page `page` (from 1) rendered to fit `max_side` pixels on its longer side, on nothing (the
/// dialog paints the paper).
pub fn thumbnail(bytes: &[u8], page: usize, max_side: u32, password: Option<&str>) -> Result<Thumbnail, IoError> {
    let pdf = load(bytes, password)?;
    let p = page_of(&pdf, page)?;
    let info = page_info(p);
    let (dw, dh) = info.displayed_size(info.crop);
    let k = max_side.clamp(1, 2048) as f32 / dw.max(dh).max(1.0e-3);
    let side = |v: f32| ((v * k).round() as u32).clamp(1, 2048);
    let (w, h) = (side(dw), side(dh));
    // A small thumbnail still decodes the page's images at their own size: within budget first.
    preflight(p)?;
    let rgba = render_page(p, info.crop, info.rotation, (w, h), true);
    Ok(Thumbnail { width: w, height: h, rgba })
}

/// Opens one page of a PDF as a document (see the module docs). `name` is the file name.
pub fn import_pdf(name: &str, bytes: &[u8], opts: &PdfOpenOptions, ctl: &Interrupt) -> Result<ImportResult, IoError> {
    let mut r = import_pdf_inner(name, bytes, opts, ctl)?;
    if opts.suppress_warnings {
        r.warnings.clear();
    }
    Ok(r)
}

fn import_pdf_inner(name: &str, bytes: &[u8], opts: &PdfOpenOptions, ctl: &Interrupt) -> Result<ImportResult, IoError> {
    if let Some(index) = opts.image {
        return import_image(name, bytes, index, opts.name.as_deref(), opts.password.as_deref(), ctl);
    }
    ctl.check().map_err(|_| IoError::Cancelled)?;
    crate::pdf_layers::validate_pdf_size_len(bytes.len()).map_err(crate::pdf_layers::LayeredPdfError::into_io)?;
    let pdf = load(bytes, opts.password.as_deref())?;
    let count = pdf.pages().len();
    let page = page_of(&pdf, opts.page)?;
    if opts.layer_mode != photocraft_doc::PdfImportMode::Render {
        match crate::pdf_layers::import_page(name, page, ctl) {
            Ok(result) => return Ok(result),
            Err(crate::pdf_layers::LayeredPdfError::Absent) if opts.layer_mode == photocraft_doc::PdfImportMode::Auto => {}
            Err(error) => return Err(error.into_io()),
        }
    }
    let mut warnings = Vec::new();
    ctl.progress(0.05);
    // Every image the page can decode is sized from the file before any of it is decoded.
    preflight(page)?;
    // A one-page PDF that is nothing but one picture opens as that picture.
    if count == 1
        && let Some(document) = image_only(name, page)?
    {
        return Ok(ImportResult { document, warnings, source_read_only: true, preview_only: false });
    }
    let info = page_info(page);
    let bx = match opts.crop {
        CropTo::BoundingBox => marks(page, None).bounds_within(info.crop),
        c => info.box_for(c),
    };
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(0.15);
    let ppi = if opts.resolution.is_finite() { opts.resolution.clamp(MIN_RESOLUTION, MAX_RESOLUTION) } else { DEFAULT_RESOLUTION };
    let (mut w, mut h) = match opts.pixel_size {
        Some((w, h)) => (w.max(1), h.max(1)),
        None => pixel_size(info.displayed_size(bx), ppi),
    };
    let budget = (MAX_PAGE_PIXELS as f64 / (f64::from(w) * f64::from(h))).sqrt();
    let fit = (f64::from(MAX_SIDE) / f64::from(w)).min(f64::from(MAX_SIDE) / f64::from(h)).min(budget).min(1.0);
    if fit < 1.0 {
        warnings.push(format!("PDF: the {w} × {h} px page was scaled down to fit {MAX_SIDE} px and {} MP", MAX_PAGE_PIXELS / 1_000_000));
        w = ((f64::from(w) * fit).floor() as u32).max(1);
        h = ((f64::from(h) * fit).floor() as u32).max(1);
    }
    let rgba = render_page(page, bx, info.rotation, (w, h), opts.anti_alias);
    ctl.check().map_err(|_| IoError::Cancelled)?;
    ctl.progress(0.8);
    let mode = match opts.mode {
        m @ (ColorMode::Grayscale | ColorMode::Cmyk | ColorMode::Lab) => m,
        _ => ColorMode::Rgb,
    };
    let depth = if opts.depth == SampleType::U16 { SampleType::U16 } else { SampleType::U8 };
    let mut doc = Document::new(page_name(name, opts.name.as_deref(), opts.page, count), Size::new(w, h), mode, depth);
    doc.resolution_dpi = ppi;
    let fmt = doc.pixel_format();
    let mut surface = Surface::new(fmt);
    write_bands(&mut surface, &rgba, (w, h), fmt.mode, ctl)?;
    surface.prune();
    doc.layers.push(Layer::new("Layer 1", LayerContent::Raster(surface)));
    Ok(ImportResult { document: doc, warnings, source_read_only: true, preview_only: false })
}

/// The document name Photoshop gives page `page` of a `count`-page file: the chosen name or the
/// file's stem, with `-N.pdf` appended when the file has several pages.
pub fn page_name(file: &str, chosen: Option<&str>, page: usize, count: usize) -> String {
    let stem = match chosen.map(str::trim).filter(|s| !s.is_empty()) {
        // The dialog's Name field starts as the file name, `.pdf` included.
        Some(s) => s.strip_suffix(".pdf").or_else(|| s.strip_suffix(".PDF")).unwrap_or(s).to_string(),
        None => stem_of(file),
    };
    if count > 1 { format!("{stem}-{page}.pdf") } else { stem }
}

/// The document name Photoshop gives image `index` (from 1) of a file opened in Images mode:
/// `stem-N`, no extension.
pub fn image_name(file: &str, chosen: Option<&str>, index: usize) -> String {
    let stem = match chosen.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => s.strip_suffix(".pdf").or_else(|| s.strip_suffix(".PDF")).unwrap_or(s).to_string(),
        None => stem_of(file),
    };
    format!("{stem}-{index}")
}

fn stem_of(file: &str) -> String {
    let name = file.rsplit(['/', '\\']).next().unwrap_or(file);
    name.rsplit_once('.').map_or(name.to_string(), |(s, _)| s.to_string())
}

/// A raster image drawn by a page, as the dialog's Images mode lists them: numbered through
/// the file from 1, with the pixel size the file declares. Image masks don't count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfImage {
    pub index: usize,
    pub page: usize,
    pub width: u32,
    pub height: u32,
}

/// Every raster image the file draws, page by page (Photoshop's Images mode).
pub fn images(bytes: &[u8], password: Option<&str>) -> Result<Vec<PdfImage>, IoError> {
    let pdf = load(bytes, password)?;
    let mut out = Vec::new();
    for (p, page) in pdf.pages().iter().enumerate() {
        codec_check(page)?;
        let m = marks(page, None);
        for placed in m.images {
            out.push(PdfImage { index: out.len() + 1, page: p + 1, width: placed.size.0, height: placed.size.1 });
        }
    }
    Ok(out)
}

/// Image `index` (from 1, counted through the file) as the document Photoshop opens for it in
/// Images mode: the image's own pixels on a `Background` layer, named `stem-N`, untitled (not tied
/// to the PDF for Save).
pub fn import_image(name: &str, bytes: &[u8], index: usize, chosen: Option<&str>, password: Option<&str>, ctl: &Interrupt) -> Result<ImportResult, IoError> {
    ctl.check().map_err(|_| IoError::Cancelled)?;
    let pdf = load(bytes, password)?;
    let mut seen = 0usize;
    for page in pdf.pages().iter() {
        // Count this page's images first (no image decoding), then decode the one asked for.
        codec_check(page)?;
        let n = marks(page, None).images.len();
        if index > seen && index <= seen + n {
            ctl.progress(0.3);
            // The one image decodes only within the page's budgets.
            preflight(page)?;
            let placed = marks(page, Some(index - seen - 1)).images.into_iter().nth(index - seen - 1);
            let doc = placed
                .and_then(|p| image_document(&image_name(name, chosen, index), &p))
                .ok_or_else(|| IoError::Pdf(format!("image {index} couldn't be decoded")))?;
            // Not tied to the PDF for Save: Photoshop leaves it untitled.
            return Ok(ImportResult { document: doc, warnings: Vec::new(), source_read_only: true, preview_only: false });
        }
        seen = seen.saturating_add(n);
        ctl.check().map_err(|_| IoError::Cancelled)?;
    }
    Err(IoError::Pdf(format!("image {index} doesn't exist: the PDF has {seen} image(s)")))
}

/// Image `index` (from 1) reduced to fit `max_side`, for the dialog's Images mode.
pub fn image_thumbnail(bytes: &[u8], index: usize, max_side: u32, password: Option<&str>) -> Result<Thumbnail, IoError> {
    let r = import_image("thumb.pdf", bytes, index, None, password, &Interrupt::NONE)?;
    let doc = r.document;
    let (w, h) = (doc.size.width, doc.size.height);
    let k = (max_side.clamp(1, 2048) as f32 / w.max(h).max(1) as f32).min(1.0);
    let (tw, th) = (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1));
    let Some(surface) = doc.layers.first().and_then(|l| l.surface()) else { return Err(IoError::Pdf("no image".to_string())) };
    let n = surface.format().channels();
    let gray = doc.mode == ColorMode::Grayscale;
    let mut rgba = Vec::with_capacity((tw as usize).saturating_mul(th as usize).saturating_mul(4));
    // Nearest-neighbour sampling is plenty for a thumbnail.
    for y in 0..th {
        let sy = ((y as f32 + 0.5) / k).floor().clamp(0.0, (h - 1) as f32) as i32;
        for x in 0..tw {
            let sx = ((x as f32 + 0.5) / k).floor().clamp(0.0, (w - 1) as f32) as i32;
            let px = surface.pixel(sx, sy);
            let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            let (r, g, b) = if gray { (to8(px[0]), to8(px[0]), to8(px[0])) } else { (to8(px[0]), to8(px[1]), to8(px[2])) };
            rgba.extend_from_slice(&[r, g, b, to8(px.get(n - 1).copied().unwrap_or(1.0))]);
        }
    }
    Ok(Thumbnail { width: tw, height: th, rgba })
}

// ------------------------------------------------------------------------------------- reading

pub(crate) fn load(bytes: &[u8], password: Option<&str>) -> Result<Pdf, IoError> {
    let data = Arc::new(bytes.to_vec());
    let r = match password {
        Some(p) => Pdf::new_with_password(data, p),
        None => Pdf::new(data),
    };
    r.map_err(|e| match e {
        LoadPdfError::Decryption(d) => {
            let what = format!("{d:?}");
            if password.is_some() && what.contains("Password") {
                IoError::Pdf(WRONG_PASSWORD.to_string())
            } else if what.contains("Password") {
                IoError::Pdf(PASSWORD_REQUIRED.to_string())
            } else {
                IoError::Pdf(format!("the file's encryption isn't supported ({what})"))
            }
        }
        LoadPdfError::Invalid => IoError::Pdf("not a PDF that can be read (damaged, truncated or not a PDF at all)".to_string()),
    })
}

fn page_of<'a>(pdf: &'a Pdf, page: usize) -> Result<&'a Page<'a>, IoError> {
    let n = pdf.pages().len();
    if n == 0 {
        return Err(IoError::Pdf("the PDF has no pages".to_string()));
    }
    pdf.pages().get(page.wrapping_sub(1)).ok_or_else(|| IoError::Pdf(format!("page {page} doesn't exist: the PDF has {n} page(s)")))
}

/// `[x0, y0, x1, y1]` with the corners ordered.
fn norm(r: hayro::hayro_syntax::object::Rect) -> [f32; 4] {
    [r.x0.min(r.x1) as f32, r.y0.min(r.y1) as f32, r.x0.max(r.x1) as f32, r.y0.max(r.y1) as f32]
}

fn inter(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    [x0, y0, a[2].min(b[2]).max(x0), a[3].min(b[3]).max(y0)]
}

fn has_area(b: [f32; 4]) -> bool {
    b[2] > b[0] && b[3] > b[1]
}

fn to_pt(b: [f32; 4]) -> PtRect {
    PtRect::new(f64::from(b[0]), f64::from(b[1]), f64::from(b[2]), f64::from(b[3]))
}

fn page_info(p: &Page) -> PdfPage {
    // A page without a usable media box is treated as A4, as hayro renders it.
    let mut media = norm(p.media_box());
    if !has_area(media) {
        media = [0.0, 0.0, 595.276, 841.89];
    }
    let mut crop = inter(norm(p.crop_box()), media);
    if !has_area(crop) {
        crop = media;
    }
    let get = |key: &[u8]| p.raw().get::<hayro::hayro_syntax::object::Rect>(key).map(norm).map(|b| inter(b, media)).filter(|b| has_area(*b)).unwrap_or(crop);
    let rotation = match p.rotation() {
        Rotation::None => 0,
        Rotation::Horizontal => 90,
        Rotation::Flipped => 180,
        Rotation::FlippedHorizontal => 270,
    };
    PdfPage { media, crop, bleed: get(b"BleedBox"), trim: get(b"TrimBox"), art: get(b"ArtBox"), rotation }
}

fn settings() -> InterpreterSettings {
    InterpreterSettings::default()
}

// ---------------------------------------------------------------------------------- rendering

/// PDF points of box `bx` → the unrotated page with y down, then turned by `rotation`, at one
/// pixel per point (the page's own `initial_transform`, for any box).
fn oriented(bx: [f32; 4], rotation: u16) -> Affine {
    let (x0, y1) = (f64::from(bx[0]), f64::from(bx[3]));
    let (w, h) = (f64::from(bx[2] - bx[0]), f64::from(bx[3] - bx[1]));
    let upright = Affine::new([1.0, 0.0, 0.0, -1.0, -x0, y1]);
    let turn = match rotation {
        90 => Affine::new([0.0, 1.0, -1.0, 0.0, h, 0.0]),
        180 => Affine::new([-1.0, 0.0, 0.0, -1.0, w, h]),
        270 => Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, w]),
        _ => Affine::IDENTITY,
    };
    turn * upright
}

/// Box `bx` of `page` rendered into `w × h` pixels, straight-alpha RGBA, transparent where the
/// page paints nothing. Callers run [`preflight`] first: rendering decodes the page's images at
/// the size the file declares, whatever `w × h`.
fn render_page(page: &Page, bx: [f32; 4], rotation: u16, (w, h): (u32, u32), anti_alias: bool) -> Vec<u8> {
    let (w, h) = (w.clamp(1, MAX_SIDE), h.clamp(1, MAX_SIDE));
    let (bw, bh) = (f64::from(bx[2] - bx[0]).max(1.0e-6), f64::from(bx[3] - bx[1]).max(1.0e-6));
    let (dw, dh) = if rotation == 90 || rotation == 270 { (bh, bw) } else { (bw, bh) };
    let transform = Affine::scale_non_uniform(f64::from(w) / dw, f64::from(h) / dh) * oriented(bx, rotation);
    // The page renders on this thread; PhotoCraft's own parallelism runs around it.
    let render_settings = vello_cpu::RenderSettings { num_threads: 0, ..Default::default() };
    let mut ctx = RenderContext::new_with(w as u16, h as u16, render_settings);
    render_into(page, &RenderCache::new(), &settings(), &RenderSettings::default(), &mut ctx, transform);
    ctx.flush();
    let mut pixmap = Pixmap::new(w as u16, h as u16);
    ctx.render_with(
        &mut pixmap,
        &mut vello_cpu::Resources::default(),
        vello_cpu::RasterizerSettings { target_init: vello_cpu::TargetInit::Clear(TRANSPARENT), ..Default::default() },
    );
    let mut rgba = pixmap.take_rgba8(ImageAlphaType::Alpha);
    if !anti_alias {
        // Anti-aliased off: a pixel is in or out. The renderer resets its own aliasing threshold
        // while hayro drives it, so the coverage is settled afterwards: half or more is in.
        for px in rgba.as_chunks_mut::<4>().0 {
            px[3] = if px[3] >= 128 { 255 } else { 0 };
        }
    }
    rgba
}

/// Straight-alpha sRGB bytes → the document's interleaved normalised channels: sRGB as is, and
/// the other modes through the working profiles Photoshop assigns (Dot Gain 20 %, the coated
/// CMYK profile, Lab D50), relative colorimetric with black point compensation.
/// Rows converted at a time. Converting the whole page at once held an f32 copy of it, plus a
/// second one for Gray, CMYK and Lab: about 1 GB and 1.3 GB at the 64 MP cap, on top of the
/// rendered RGBA. A band keeps that to a few MB.
const BAND_ROWS: u32 = 256;

/// Writes the rendered `w × h` RGBA page into `surface` in `mode`, a band of rows at a time.
fn write_bands(surface: &mut Surface, rgba: &[u8], (w, h): (u32, u32), mode: ColorMode, ctl: &Interrupt) -> Result<(), IoError> {
    let dst = match mode {
        ColorMode::Grayscale => Some(Builtin::DotGain20Compat),
        ColorMode::Cmyk => Some(Builtin::CoatedCmyk),
        ColorMode::Lab => Some(Builtin::LabD50),
        _ => None,
    };
    let t = dst
        .map(|d| Transform::new(Builtin::Srgb.profile(), d.profile(), Intent::RelativeColorimetric, true))
        .transpose()
        .map_err(|e| IoError::Pdf(format!("colour conversion: {e}")))?;
    let n = if t.is_some() { mode.color_channels() } else { 3 };
    let row = (w as usize).checked_mul(4).ok_or_else(|| IoError::Pdf("PDF: the page is too wide".into()))?;
    let mut y = 0u32;
    while y < h {
        ctl.check().map_err(|_| IoError::Cancelled)?;
        let rows = BAND_ROWS.min(h - y);
        let start = (y as usize).saturating_mul(row);
        let band = rgba.get(start..start.saturating_add(rows as usize * row)).ok_or_else(|| IoError::Pdf("PDF: the rendered page is short".into()))?;
        let src: Vec<f32> = band.iter().map(|&v| f32::from(v) / 255.0).collect();
        let px = match &t {
            Some(t) => {
                let mut out = vec![0.0f32; src.len() / 4 * (n + 1)];
                t.convert_f32(&src, 4, &mut out, n + 1, true);
                out
            }
            None => src,
        };
        let y0 = i32::try_from(y).map_err(|_| IoError::Pdf("PDF: the page is too tall".into()))?;
        surface.write_region(photocraft_geom::Rect::from_xywh(0, y0, w, rows), &px);
        y += rows;
    }
    Ok(())
}

// --------------------------------------------------------------------------------- budgets

/// Whether an image the file declares as `w × h` px may be decoded at all.
fn image_fits(w: u32, h: u32) -> bool {
    w > 0 && h > 0 && w <= MAX_SIDE && h <= MAX_SIDE && u64::from(w) * u64::from(h) <= MAX_IMAGE_PIXELS
}

fn image_error(what: &str, w: u32, h: u32) -> IoError {
    IoError::Pdf(format!("PDF: {what} of {w} × {h} px is past the {MAX_SIDE} px / {} MP limit, so the page can't be opened", MAX_IMAGE_PIXELS / 1_000_000))
}

fn total_error() -> IoError {
    IoError::Pdf(format!("PDF: the page's images decode to more than {} MP together, so it can't be opened", MAX_PAGE_IMAGE_PIXELS / 1_000_000))
}

fn no_size(what: &str) -> IoError {
    IoError::Pdf(format!("PDF: {what} doesn't declare its size, so its decoding can't be bounded; the page can't be opened"))
}

fn unchecked(what: &str, codec: &str, why: &str) -> IoError {
    IoError::Pdf(format!("PDF: {what} is a {codec} image whose size can't be checked before decoding ({why}), so the page can't be opened"))
}

/// The `/Width × /Height` an image stream declares (`/W`, `/H` in an inline image).
fn declared_size(dict: &Dict<'_>) -> Option<(u32, u32)> {
    let w = dict.get::<u32>(b"Width").or_else(|| dict.get::<u32>(b"W"))?;
    let h = dict.get::<u32>(b"Height").or_else(|| dict.get::<u32>(b"H"))?;
    Some((w, h))
}

/// What a JPX (JPEG 2000) stream encodes, read from its headers without decoding: the image size
/// and the number of components.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct JpxHeader {
    size: (u32, u32),
    components: u16,
}

/// What an image codec builds, as its own data or parameters say rather than the dictionary's
/// `/Width × /Height` (which these decoders don't go by).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoded {
    /// JPXDecode: what the codestream encodes.
    Jpx(JpxHeader),
    /// CCITTFaxDecode: `/Columns × max(/Rows, /Height)` from its `/DecodeParms`, the buffer the
    /// decoder allocates before it reads any data.
    Ccitt((u32, u32)),
    /// JBIG2Decode: the page its page information segment declares (or its stripes add up to).
    Jbig2((u32, u32)),
}

impl Encoded {
    fn size(self) -> (u32, u32) {
        match self {
            Encoded::Jpx(j) => j.size,
            Encoded::Ccitt(s) | Encoded::Jbig2(s) => s,
        }
    }

    /// The pixels decoding builds: a JPX image with more than four components counted once per
    /// four of them.
    fn pixels(self) -> u64 {
        let (w, h) = self.size();
        let planes = match self {
            Encoded::Jpx(j) => u64::from(j.components).div_ceil(4).max(1),
            _ => 1,
        };
        u64::from(w) * u64::from(h) * planes
    }

    fn describe(self) -> String {
        let (w, h) = self.size();
        match self {
            Encoded::Jpx(j) => format!("a JPX image whose codestream encodes {w} × {h} px with {} component(s)", j.components),
            Encoded::Ccitt(_) => format!("a CCITT fax image whose /DecodeParms make the decoder build {w} × {h} px"),
            Encoded::Jbig2(_) => format!("a JBIG2 image whose page information gives {w} × {h} px"),
        }
    }
}

/// One image a stream decodes to, sized from the file without decoding: an image, or its soft
/// mask or explicit mask, each an image of its own.
#[derive(Clone, Debug)]
struct Part {
    what: &'static str,
    /// The dictionary's `/Width × /Height`.
    declared: (u32, u32),
    /// For a stream an image codec decodes, what that codec builds, whatever the dictionary
    /// declares.
    encoded: Option<Encoded>,
}

impl Part {
    /// The pixels decoding it builds: the larger of the declared and the encoded size.
    fn pixels(&self) -> u64 {
        let declared = u64::from(self.declared.0) * u64::from(self.declared.1);
        self.encoded.map_or(declared, |e| declared.max(e.pixels()))
    }

    /// Refuses the part when decoding it alone would pass the per-image limits.
    fn fits(&self) -> Result<(), IoError> {
        let (w, h) = self.declared;
        if !image_fits(w, h) {
            return Err(image_error(self.what, w, h));
        }
        if let Some(e) = self.encoded
            && (!image_fits(e.size().0, e.size().1) || self.pixels() > MAX_IMAGE_PIXELS)
        {
            return Err(IoError::Pdf(format!(
                "PDF: {} of {w} × {h} px is {}, past the {MAX_SIDE} px / {} MP limit, so the page can't be opened",
                self.what,
                e.describe(),
                MAX_IMAGE_PIXELS / 1_000_000
            )));
        }
        Ok(())
    }
}

/// The image codecs, by their PDF names: each sizes what it builds by its own data or parameters,
/// so only images, whose size [`preflight`] checks against them, may use one.
fn image_codec(f: Filter) -> Option<&'static str> {
    match f {
        Filter::DctDecode => Some("DCTDecode"),
        Filter::JpxDecode => Some("JPXDecode"),
        Filter::Jbig2Decode => Some("JBIG2Decode"),
        Filter::CcittFaxDecode => Some("CCITTFaxDecode"),
        _ => None,
    }
}

/// The filter a name stands for, as `hayro-syntax` reads it (abbreviations included).
fn filter_named(name: &[u8]) -> Option<Filter> {
    Some(match name {
        keys::ASCII_HEX_DECODE | keys::ASCII_HEX_DECODE_ABBREVIATION => Filter::AsciiHexDecode,
        keys::ASCII85_DECODE | keys::ASCII85_DECODE_ABBREVIATION => Filter::Ascii85Decode,
        keys::LZW_DECODE | keys::LZW_DECODE_ABBREVIATION => Filter::LzwDecode,
        keys::FLATE_DECODE | keys::FLATE_DECODE_ABBREVIATION => Filter::FlateDecode,
        keys::RUN_LENGTH_DECODE | keys::RUN_LENGTH_DECODE_ABBREVIATION => Filter::RunLengthDecode,
        keys::CCITTFAX_DECODE | keys::CCITTFAX_DECODE_ABBREVIATION => Filter::CcittFaxDecode,
        keys::JBIG2_DECODE => Filter::Jbig2Decode,
        keys::DCT_DECODE | keys::DCT_DECODE_ABBREVIATION => Filter::DctDecode,
        keys::JPX_DECODE => Filter::JpxDecode,
        keys::CRYPT => Filter::Crypt,
        _ => return None,
    })
}

/// A stream's filters, each with the `/DecodeParms` it gets, paired as `hayro-syntax` pairs them:
/// one filter name and one dictionary, or two arrays read side by side, a filter it doesn't know
/// skipped with its parameters.
fn filter_chain<'a>(dict: &Dict<'a>) -> Vec<(Filter, Dict<'a>)> {
    let one = dict.get::<Name<'_>>(b"F").or_else(|| dict.get::<Name<'_>>(b"Filter")).and_then(|n| filter_named(n.as_ref()));
    if let Some(f) = one {
        let params = dict.get::<Dict<'_>>(b"DP").or_else(|| dict.get::<Dict<'_>>(b"DecodeParms")).unwrap_or_default();
        return vec![(f, params)];
    }
    let Some(names) = dict.get::<Array<'_>>(b"F").or_else(|| dict.get::<Array<'_>>(b"Filter")) else { return Vec::new() };
    let mut params = dict.get::<Array<'_>>(b"DP").or_else(|| dict.get::<Array<'_>>(b"DecodeParms")).map(|a| a.iter::<Object<'_>>());
    let mut out = Vec::new();
    for name in names.iter::<Name<'_>>() {
        let p = params.as_mut().and_then(Iterator::next).and_then(Object::into_dict).unwrap_or_default();
        if let Some(f) = filter_named(name.as_ref()) {
            out.push((f, p));
        }
    }
    out
}

/// Refuses a content stream whose Deflate data would inflate past [`MAX_CONTENT_BYTES`], measured
/// by inflating into nothing (no buffer is kept) before anything decodes it for real. `hayro` has
/// no decode limit of its own. Streams whose first filter isn't FlateDecode are left to it: the
/// ASCII filters shrink their input, and LZW and run-length content is rare.
fn content_bounded(stream: &Stream<'_>, what: &str) -> Result<(), IoError> {
    let chain: Vec<Filter> = filter_chain(stream.dict()).into_iter().map(|(f, _)| f).collect();
    let Some(flate) = chain.iter().position(|f| *f == Filter::FlateDecode) else { return Ok(()) };
    // A second decompressing stage after the measured one would expand its output unmeasured.
    if chain.iter().skip(flate + 1).any(|f| matches!(f, Filter::FlateDecode | Filter::LzwDecode | Filter::RunLengthDecode)) {
        return Err(IoError::Pdf(format!("PDF: {what} is decompressed twice in a row, so its size can't be checked; the page can't be opened")));
    }
    let raw = stream.raw_data();
    let mut data = std::borrow::Cow::Borrowed(raw.as_ref());
    // The ASCII filters only shrink their input, so they're undone here to reach the Deflate data.
    // Anything else ahead of FlateDecode (LZW, run-length) could expand it unmeasured.
    for f in chain.iter().take(flate) {
        let decoded = match f {
            Filter::AsciiHexDecode => ascii_hex(&data),
            Filter::Ascii85Decode => ascii85(&data),
            _ => {
                return Err(IoError::Pdf(format!("PDF: {what} is decompressed twice in a row, so its size can't be checked; the page can't be opened")));
            }
        };
        // Data these decoders refuse might still get through a more lenient one, unmeasured.
        let Some(d) = decoded else {
            return Err(IoError::Pdf(format!("PDF: {what} has ASCII-encoded data that can't be read, so its size can't be checked; the page can't be opened")));
        };
        data = std::borrow::Cow::Owned(d);
    }
    // `hayro` reads zlib data, or raw deflate when there's no zlib header.
    let n = inflated_len(flate2::read::ZlibDecoder::new(data.as_ref())).max(inflated_len(flate2::read::DeflateDecoder::new(data.as_ref())));
    if n > MAX_CONTENT_BYTES {
        return Err(IoError::Pdf(format!("PDF: {what} inflates past {} MB, so the page can't be opened", MAX_CONTENT_BYTES >> 20)));
    }
    Ok(())
}

/// ASCIIHexDecode: pairs of hex digits, whitespace ignored, up to `>`; an odd last digit is
/// followed by 0. `None` on any other byte.
fn ascii_hex(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut high: Option<u8> = None;
    for &c in data {
        let v = match c {
            b'>' => break,
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            c if c.is_ascii_whitespace() || c == 0 => continue,
            _ => return None,
        };
        match high.take() {
            Some(h) => out.push(h << 4 | v),
            None => high = Some(v),
        }
    }
    if let Some(h) = high {
        out.push(h << 4);
    }
    Some(out)
}

/// ASCII85Decode: groups of five characters `!` to `u` give four bytes, `z` four zeros,
/// whitespace is ignored, `~>` ends it. `None` on any other byte or an overflowing group.
fn ascii85(data: &[u8]) -> Option<Vec<u8>> {
    let word = |g: &[u8; 5]| g.iter().try_fold(0u32, |a, &d| a.checked_mul(85)?.checked_add(u32::from(d)));
    let mut out = Vec::with_capacity(data.len() / 5 * 4 + 4);
    let mut group = [0u8; 5];
    let mut n = 0;
    for &c in data {
        match c {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group[n] = c - b'!';
                n += 1;
                if n == 5 {
                    out.extend_from_slice(&word(&group)?.to_be_bytes());
                    n = 0;
                }
            }
            c if c.is_ascii_whitespace() || c == 0 => {}
            _ => return None,
        }
    }
    if n > 1 {
        // A partial last group: padded with `u`, n - 1 bytes kept.
        group.iter_mut().skip(n).for_each(|g| *g = 84);
        out.extend_from_slice(&word(&group)?.to_be_bytes()[..n - 1]);
    }
    Some(out)
}

/// How many bytes `r` yields, counting stops just past [`MAX_CONTENT_BYTES`]; data after a
/// decoding error isn't counted (the decoder stops there too).
fn inflated_len(mut r: impl std::io::Read) -> u64 {
    // On the heap: this runs at the bottom of `codec_check`'s recursive walk.
    let mut buf = vec![0u8; 64 * 1024];
    let mut n = 0u64;
    while n <= MAX_CONTENT_BYTES {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(k) => n = n.saturating_add(k as u64),
        }
    }
    n
}

/// Refuses `stream` when an image codec decodes it: for anything but an image (a content stream,
/// a font, a profile, `/JBIG2Globals`) that codec's output couldn't be bounded.
fn not_image_coded(stream: &Stream<'_>, what: &str) -> Result<(), IoError> {
    match stream.filters().into_iter().find_map(image_codec) {
        Some(codec) => {
            Err(IoError::Pdf(format!("PDF: {what}, which isn't an image, is coded with {codec}, which only images may use, so the page can't be opened")))
        }
        None => Ok(()),
    }
}

/// Sizes one image stream without decoding it. The image codecs size what they build by their own
/// data or parameters, so those are read too: a JPX codestream's header or a JBIG2 stream's segment
/// headers (the codec must then be the stream's one filter: behind another filter the bytes would
/// have to be decoded first; data that can't be read refuses the image) and a CCITT fax filter's
/// `/DecodeParms`. A stream with two image codecs is refused.
fn part_of(stream: &Stream<'_>, what: &'static str) -> Result<Part, IoError> {
    let declared = declared_size(stream.dict()).ok_or_else(|| no_size(what))?;
    let chain = filter_chain(stream.dict());
    let mut codecs = chain.iter().filter(|(f, _)| image_codec(*f).is_some());
    let (codec, second) = (codecs.next(), codecs.next());
    if second.is_some() {
        return Err(IoError::Pdf(format!("PDF: {what} goes through two image codecs, so its decoding can't be bounded; the page can't be opened")));
    }
    let alone = chain.len() == 1;
    let encoded = match codec {
        Some((Filter::JpxDecode, _)) if !alone => return Err(unchecked(what, "JPX", "JPXDecode isn't its only filter")),
        Some((Filter::JpxDecode, _)) => Some(Encoded::Jpx(jpx_header(&stream.raw_data()).map_err(|why| unchecked(what, "JPX", why))?)),
        Some((Filter::Jbig2Decode, _)) if !alone => return Err(unchecked(what, "JBIG2", "JBIG2Decode isn't its only filter")),
        Some((Filter::Jbig2Decode, params)) => {
            let globals = jbig2_globals(params).map_err(|why| unchecked(what, "JBIG2", why))?;
            Some(Encoded::Jbig2(jbig2_page(&stream.raw_data(), globals.as_deref()).map_err(|why| unchecked(what, "JBIG2", why))?))
        }
        Some((Filter::CcittFaxDecode, params)) => Some(Encoded::Ccitt(ccitt_size(params, declared.1))),
        _ => None,
    };
    Ok(Part { what, declared, encoded })
}

/// An image stream (an XObject or an inline image) with its soft mask and explicit mask, each
/// sized from the file. `/Mask` may also be a colour-key array, which decodes nothing extra.
fn image_parts(stream: &Stream<'_>, what: &'static str) -> Result<Vec<Part>, IoError> {
    let mut parts = vec![part_of(stream, what)?];
    for (key, what) in [(&b"SMask"[..], "a soft mask"), (&b"Mask"[..], "a mask")] {
        if let Some(m) = stream.dict().get::<Stream<'_>>(key) {
            parts.push(part_of(&m, what)?);
        }
    }
    Ok(parts)
}

/// The buffer a CCITT fax filter allocates before it reads any data, `/Columns` (1728 when absent)
/// by the larger of `/Rows` and the image's `/Height`, read exactly as `hayro-syntax` reads them.
fn ccitt_size(params: &Dict<'_>, height: u32) -> (u32, u32) {
    // Past `u32` keeps `u32::MAX`, which the per-image limits refuse; `as u32` wrapped it small.
    let columns = params.get::<usize>(b"Columns").map_or(1728, |c| u32::try_from(c).unwrap_or(u32::MAX));
    // `/Rows` read as the decoder reads it (`usize`), so a value past `u32` isn't dropped in
    // favour of the smaller `/Height`.
    let rows = params.get::<usize>(b"Rows").map_or(0, |r| u32::try_from(r).unwrap_or(u32::MAX));
    (columns, rows.max(height))
}

/// The `/JBIG2Globals` a JBIG2 filter's parameters name, decoded as the decoder decodes them (one
/// that doesn't decode is left out, as the decoder leaves it out). `Err` when an image codec would
/// decode them.
fn jbig2_globals(params: &Dict<'_>) -> Result<Option<Vec<u8>>, &'static str> {
    let Some(g) = params.get::<Stream<'_>>(b"JBIG2Globals") else { return Ok(None) };
    if g.filters().into_iter().any(|f| image_codec(f).is_some()) {
        return Err("its JBIG2Globals stream is coded with an image codec");
    }
    Ok(g.decoded().ok().map(|d| d.into_owned()))
}

/// The page a JBIG2 stream decodes to, read from its segment headers (the globals' first, as
/// `hayro-jbig2` reads them, embedded organisation, ITU-T T.88 7.2) without decoding: the page
/// information segment with the lowest number gives the size, an unknown height (0xFFFFFFFF) the
/// end-of-stripe rows. The decoder and the image built from it allocate that page; every generic
/// or refinement region it composes onto it declares its own size, which must fit the per-image
/// limits too. Symbol dictionaries, text regions, pattern dictionaries and halftone regions are
/// refused: the decoder builds their symbols and patterns at sizes the coded data gives, which
/// can't be read without decoding. A segment that can't be read is refused rather than decoded.
fn jbig2_page(data: &[u8], globals: Option<&[u8]>) -> Result<(u32, u32), &'static str> {
    let mut page: Option<(u32, u32, u32)> = None;
    let mut stripes: Option<u32> = None;
    for part in globals.into_iter().chain([data]) {
        let mut at = 0;
        while at < part.len() {
            let seg = jbig2_segment(part, at)?;
            let body = seg.data;
            match seg.kind {
                // Page information: width, height, then resolutions, flags and striping.
                48 => {
                    let (Some(w), Some(h)) = (be32(body, 0), be32(body, 4)) else { return Err("its page information is truncated") };
                    if page.is_none_or(|(n, _, _)| seg.number < n) {
                        page = Some((seg.number, w, h));
                    }
                }
                // End of stripe: the last row of the stripe.
                50 => {
                    if let Some(rows) = be32(body, 0).and_then(|r| r.checked_add(1)) {
                        stripes = Some(stripes.map_or(rows, |s| s.max(rows)));
                    }
                }
                // Generic and generic refinement regions, intermediate or immediate: the region
                // segment information starts with the bitmap's width and height.
                36 | 38 | 39 | 40 | 42 | 43 => {
                    let (Some(w), Some(h)) = (be32(body, 0), be32(body, 4)) else { return Err("a region segment is truncated") };
                    if !image_fits(w, h) {
                        return Err("a region segment declares a bitmap past the limits");
                    }
                }
                0 | 4 | 6 | 7 | 16 | 20 | 22 | 23 => return Err("it uses symbol or halftone coding, whose bitmaps take their sizes from the coded data"),
                // End of page or file, profiles, tables, colour palettes, extensions: no bitmap.
                49 | 51 | 52 | 53 | 54 | 62 => {}
                _ => return Err("it has a segment of an unknown type"),
            }
            at = seg.next;
            // The decoder stops reading this part at an end-of-file segment.
            if seg.kind == 51 {
                break;
            }
        }
    }
    let Some((_, w, h)) = page else { return Err("it has no page information segment") };
    let h = if h == u32::MAX { stripes.ok_or("its page height is unknown and no stripe ends")? } else { h };
    Ok((w, h))
}

/// One segment of a JBIG2 stream: its number, type and data, and where the next one starts.
struct Jbig2Segment<'a> {
    number: u32,
    kind: u8,
    data: &'a [u8],
    next: usize,
}

/// The segment at `at`, read as `hayro-jbig2` reads it (ITU-T T.88 7.2): number, flags (type and
/// page association size), referred-to segments (short or long form, each number 1, 2 or 4 bytes
/// by the segment's own number), page association, data length. An unknown length (0xFFFFFFFF)
/// ends at the first end marker after the region segment information and flags (0x00 0x00 with
/// MMR, else 0xFF 0xAC), plus the row count after it.
fn jbig2_segment(d: &[u8], at: usize) -> Result<Jbig2Segment<'_>, &'static str> {
    const SHORT: &str = "a segment header is truncated";
    let number = be32(d, at).ok_or(SHORT)?;
    let flags = *d.get(at + 4).ok_or(SHORT)?;
    let count_byte = *d.get(at + 5).ok_or(SHORT)?;
    let (count, mut p) = match count_byte >> 5 {
        c @ 0..=4 => (u64::from(c), at + 6),
        7 => {
            let c = be32(d, at + 5).ok_or(SHORT)? & 0x1FFF_FFFF;
            let retention = (c as usize).checked_add(1).ok_or(SHORT)?.div_ceil(8);
            (u64::from(c), (at + 9).checked_add(retention).ok_or(SHORT)?)
        }
        _ => return Err("a segment header is invalid"),
    };
    let width: u64 = if number <= 256 {
        1
    } else if number <= 65_536 {
        2
    } else {
        4
    };
    let referred = usize::try_from(count * width).map_err(|_| SHORT)?;
    p = p.checked_add(referred).ok_or(SHORT)?;
    p = p.checked_add(if flags & 0x40 != 0 { 4 } else { 1 }).ok_or(SHORT)?;
    let length = be32(d, p).ok_or(SHORT)?;
    let start = p + 4;
    let end = if length == u32::MAX {
        let mmr = d.get(start + 17).ok_or(SHORT)? & 1 != 0;
        let marker: [u8; 2] = if mmr { [0x00, 0x00] } else { [0xFF, 0xAC] };
        let from = start + 18;
        let tail = d.get(from..).ok_or(SHORT)?;
        let found = tail.windows(6).position(|w| w.starts_with(&marker)).ok_or("a region of unknown length has no end marker")?;
        from + found + 6
    } else {
        start.checked_add(length as usize).ok_or(SHORT)?
    };
    let data = d.get(start..end).ok_or(SHORT)?;
    Ok(Jbig2Segment { number, kind: flags & 0x3F, data, next: end })
}

fn be16(d: &[u8], at: usize) -> Option<u16> {
    d.get(at..at + 2)?.try_into().ok().map(u16::from_be_bytes)
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4)?.try_into().ok().map(u32::from_be_bytes)
}

fn be64(d: &[u8], at: usize) -> Option<u64> {
    d.get(at..at + 8)?.try_into().ok().map(u64::from_be_bytes)
}

/// Reads what a JPX stream encodes from its headers, as `hayro_jpeg2000` reads them before it
/// decodes: a raw codestream (the SOC marker, then the SIZ marker segment) or a JP2 file (boxes,
/// the codestream in `jp2c` and an image header in `jp2h`/`ihdr`; the larger of the two sizes
/// counts). `Err` says why it couldn't be read, so the image is refused rather than decoded.
fn jpx_header(data: &[u8]) -> Result<JpxHeader, &'static str> {
    const CODESTREAM: &[u8] = b"\xFF\x4F\xFF\x51";
    const JP2: &[u8] = b"\x00\x00\x00\x0C\x6A\x50\x20\x20";
    if let Some(siz) = data.strip_prefix(CODESTREAM) {
        return siz_header(siz);
    }
    if !data.starts_with(JP2) {
        return Err("neither a JPEG 2000 codestream nor a JP2 file");
    }
    let mut header = None;
    let mut ihdr = None;
    for (kind, body) in jp2_boxes(data) {
        match &kind {
            b"jp2h" => {
                for (kind, body) in jp2_boxes(body) {
                    if &kind == b"ihdr" {
                        let (Some(h), Some(w)) = (be32(body, 0), be32(body, 4)) else { return Err("the JP2 image header is truncated") };
                        ihdr = Some((w, h));
                    }
                }
            }
            b"jp2c" => {
                let Some(siz) = body.strip_prefix(CODESTREAM) else { return Err("the JP2 codestream box doesn't hold a codestream") };
                header = Some(siz_header(siz)?);
            }
            _ => {}
        }
    }
    let Some(mut header) = header else { return Err("the JP2 file has no codestream box") };
    if let Some((w, h)) = ihdr {
        header.size = (header.size.0.max(w), header.size.1.max(h));
    }
    Ok(header)
}

/// The boxes of a JP2 file or superbox, `(type, contents)`: a 32-bit length (0: to the end; 1: a
/// 64-bit length follows) and a four-byte type. Stops at a truncated box, as the decoder does.
fn jp2_boxes(mut data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    while let (Some(len), Some(kind)) = (be32(data, 0), data.get(4..8).and_then(|k| <[u8; 4]>::try_from(k).ok())) {
        let (start, end) = match len {
            0 => (8, data.len()),
            1 => match be64(data, 8).and_then(|l| usize::try_from(l).ok()) {
                Some(end) => (16, end),
                None => break,
            },
            n => (8, n as usize),
        };
        if end < start || end > data.len() {
            break;
        }
        out.push((kind, &data[start..end]));
        data = &data[end..];
    }
    out
}

/// The SIZ marker segment after its `FF 51` marker: Lsiz, Rsiz, the reference grid (Xsiz, Ysiz),
/// the image origin on it (XOsiz, YOsiz), the tile size (XTsiz, YTsiz) and origin (XTOsiz,
/// YTOsiz), then Csiz. The image is `(Xsiz − XOsiz) × (Ysiz − YOsiz)` px; the decoder also builds
/// one structure per tile up front, so their number is bounded too.
fn siz_header(siz: &[u8]) -> Result<JpxHeader, &'static str> {
    let field = |i: usize| be32(siz, 4 + 4 * i);
    let (Some(xsiz), Some(ysiz), Some(xosiz), Some(yosiz), Some(xtsiz), Some(ytsiz), Some(xtosiz), Some(ytosiz), Some(csiz)) =
        (field(0), field(1), field(2), field(3), field(4), field(5), field(6), field(7), be16(siz, 36))
    else {
        return Err("the SIZ marker segment is truncated");
    };
    if xosiz >= xsiz || yosiz >= ysiz || csiz == 0 {
        return Err("the SIZ marker segment is inconsistent");
    }
    if xtsiz == 0 || ytsiz == 0 || xtosiz > xosiz || ytosiz > yosiz {
        return Err("the SIZ marker segment tiles the image inconsistently");
    }
    let tiles = u64::from(xsiz - xtosiz).div_ceil(u64::from(xtsiz)) * u64::from(ysiz - ytosiz).div_ceil(u64::from(ytsiz));
    if tiles > MAX_JPX_TILES {
        return Err("the codestream has more tiles than can be decoded within the budget");
    }
    Ok(JpxHeader { size: (xsiz - xosiz, ysiz - yosiz), components: csiz })
}

/// The pixels a page's images decode to together.
#[derive(Default)]
struct Budget {
    total: u64,
}

impl Budget {
    /// Charges an image with its masks, refusing a part past the per-image limits or a total past
    /// the page's.
    fn charge(&mut self, parts: &[Part]) -> Result<(), IoError> {
        for p in parts {
            p.fits()?;
            self.total = self.total.saturating_add(p.pixels());
        }
        if self.total > MAX_PAGE_IMAGE_PIXELS { Err(total_error()) } else { Ok(()) }
    }
}

/// Refuses a page whose images couldn't be decoded within the budgets, before anything is
/// decoded or rendered (the decoders build each image at the size the file declares, or that its
/// codec's own data or parameters give, however small the output asked for). Nothing here decodes
/// an image:
///
/// * first [`codec_check`] reads the dictionaries of every stream the page can reach and refuses
///   one that isn't an image but would go through an image codec, so what is decoded below (the
///   content, fonts, Form XObjects) never runs one;
/// * the content is interpreted once through the [`Marks`] device, and every image it places
///   (XObjects, inline images and stencils, through Form XObjects, Type 3 glyphs and annotation
///   appearances) is charged once per drawing, as the renderer decodes an image every time it
///   draws one; a raster image is sized from its own stream, a stencil (whose stream the
///   interpreter doesn't show) from the walk below;
/// * every image stream the page's resources reach is sized, drawn or not: `/XObject` images with
///   their soft and explicit masks, through Form XObjects, tiling patterns, Type 3 glyph
///   procedures, the groups of `/ExtGState` soft masks and annotation appearances, and the inline
///   images in all those content streams; one the content never placed is charged once.
///
/// Refused: an image past [`MAX_IMAGE_PIXELS`] or [`MAX_SIDE`] on a side, a page whose images pass
/// [`MAX_PAGE_IMAGE_PIXELS`] together, an image that doesn't declare its size, a JPX or JBIG2 image
/// whose headers can't be read or a JBIG2 image in symbol or halftone coding (so its decoding
/// can't be bounded), a stream other than an image that an image codec would decode, a stencil
/// drawn from a stream the walk didn't reach, and a resource tree too deep or too large to walk.
fn preflight(page: &Page) -> Result<(), IoError> {
    codec_check(page)?;
    let mut walk = Walk::default();
    // What the content places, once per drawing.
    let mut unsourced = Vec::new();
    for p in marks(page, None).placements {
        walk.placed.insert(p.key);
        match p.parts {
            Some(parts) => walk.budget.charge(&parts?)?,
            None => unsourced.push((p.key, p.size)),
        }
    }
    // What the resources reach, drawn or not.
    walk.scan(page.typed_operations())?;
    walk.resources(page.resources().clone(), 0)?;
    walk.annotations(page)?;
    // A stencil is sized by the stream the walk found under its key.
    for (key, (w, h)) in unsourced {
        match walk.known.get(&key) {
            Some(parts) => walk.budget.charge(parts)?,
            None => {
                return Err(IoError::Pdf(format!(
                    "PDF: a stencil image of {w} × {h} px is drawn from a stream that couldn't be inspected, so the page can't be opened"
                )));
            }
        }
    }
    Ok(())
}

/// Refuses a page whose content draws Form XObjects more than [`MAX_FORM_DRAWS`] times in all
/// (counting a form drawn twice by a form drawn twice as four), or a form that draws itself,
/// before anything interprets the page. Each form's content is read once; the count then
/// multiplies through the cached lists, so it costs no more than the cap.
fn form_draws(page: &Page) -> Result<(), IoError> {
    let mut forms = Forms::default();
    let page_scope = Scope { x: page.resources().x_objects.clone(), id: ScopeId::Page };
    let mut drawn = 0u64;
    let top = forms.children(page.typed_operations(), &page_scope.x);
    for child in top {
        forms.draw(&child, &page_scope, 1, &mut drawn)?;
    }
    // The other content the renderer interprets on its own, on the same count: annotation
    // appearances, and the page's tiling patterns, Type 3 glyphs and soft-mask groups.
    for name in page.resources().patterns.keys() {
        if let Some(p) = page.resources().patterns.get::<Stream<'_>>(&name) {
            forms.pattern(&p, &page_scope, 1, &mut drawn)?;
        }
    }
    for (stream, inherited) in other_contents(page, &page_scope) {
        let scope = Scope::of(&stream, &inherited);
        let Ok(data) = stream.decoded() else { continue };
        let kids = forms.children(TypedIter::new(&data), &scope.x);
        for child in kids {
            forms.draw(&child, &scope, 1, &mut drawn)?;
        }
    }
    Ok(())
}

/// The `/XObject` dictionary `Do` names resolve in, and the object whose `/Resources` supplied
/// it: a form without `/Resources` resolves names in its parent's, so the same form can draw
/// different children under different parents.
#[derive(Clone)]
struct Scope<'a> {
    x: Dict<'a>,
    id: ScopeId,
}

/// Which `/Resources` a [`Scope`] came from, unique within one page: a font dictionary is often
/// direct (no object number), so fonts are told apart by their name in the page's `/Font`.
#[derive(Clone, PartialEq, Eq, Hash)]
enum ScopeId {
    /// The page's own resources.
    Page,
    /// No resources (an annotation appearance without its own).
    Empty,
    /// The `/Resources` of the page's Type 3 font with this name.
    Font(Vec<u8>),
    /// The `/Resources` of this stream.
    Stream(ObjectIdentifier),
}

impl<'a> Scope<'a> {
    /// `stream`'s own resources, or `inherited` when it has none.
    fn of(stream: &Stream<'a>, inherited: &Scope<'a>) -> Scope<'a> {
        match stream.dict().get::<Dict<'a>>(b"Resources") {
            Some(r) => Scope { x: Resources::new(r).x_objects, id: ScopeId::Stream(stream.obj_id()) },
            None => inherited.clone(),
        }
    }
}

/// The content streams of a page the renderer interprets besides its own content, each with the
/// `/XObject` dictionary it falls back on when it has no `/Resources`: annotation normal
/// appearances (none), and the page's tiling patterns (the page's), Type 3 glyph procedures (the
/// font's resources, else the page's) and `/ExtGState` soft-mask groups (the page's).
fn other_contents<'a>(page: &Page<'a>, page_scope: &Scope<'a>) -> Vec<(Stream<'a>, Scope<'a>)> {
    let res = page.resources();
    let none = Scope { x: Dict::default(), id: ScopeId::Empty };
    let mut out: Vec<(Stream<'a>, Scope<'a>)> = appearance_streams(page).into_iter().map(|s| (s, none.clone())).collect();
    for name in res.fonts.keys() {
        let Some(font) = res.fonts.get::<Dict<'a>>(&name) else { continue };
        let Some(procs) = font.get::<Dict<'a>>(b"CharProcs") else { continue };
        let scope = match font.get::<Dict<'a>>(b"Resources") {
            Some(r) => Scope { x: Resources::new(r).x_objects, id: ScopeId::Font(name.as_ref().to_vec()) },
            None => page_scope.clone(),
        };
        for glyph in procs.keys() {
            if let Some(s) = procs.get::<Stream<'a>>(&glyph) {
                out.push((s, scope.clone()));
            }
        }
    }
    for name in res.ext_g_states.keys() {
        let group = res.ext_g_states.get::<Dict<'a>>(&name).and_then(|gs| gs.get::<Dict<'a>>(b"SMask")).and_then(|sm| sm.get::<Stream<'a>>(b"G"));
        if let Some(s) = group {
            out.push((s, page_scope.clone()));
        }
    }
    out
}

/// The appearance streams the interpreter draws for the page's annotations: the normal
/// appearance, in the state `/AS` selects; hidden annotations are skipped.
fn appearance_streams<'a>(page: &Page<'a>) -> Vec<Stream<'a>> {
    let Some(annots) = page.raw().get::<Array<'a>>(b"Annots") else { return Vec::new() };
    let mut out = Vec::new();
    for annot in annots.iter::<Dict<'a>>() {
        if annot.get::<u32>(b"F").unwrap_or(0) & 2 != 0 {
            continue;
        }
        let Some(ap) = annot.get::<Dict<'a>>(b"AP") else { continue };
        let stream = match ap.get::<Object<'a>>(b"N") {
            Some(Object::Stream(s)) => Some(s),
            Some(Object::Dict(states)) => {
                annot.get::<Name<'_>>(b"AS").and_then(|state| states.get::<Stream<'a>>(state)).or_else(|| states.get::<Stream<'a>>(b"Off"))
            }
            _ => None,
        };
        out.extend(stream);
    }
    out
}

/// The Form XObjects each form draws, read once per form for [`form_draws`].
#[derive(Default)]
struct Forms<'a> {
    /// A content stream's children, by the stream's object and the [`Scope`] its names
    /// resolved in.
    children: HashMap<(ObjectIdentifier, ScopeId), Vec<Stream<'a>>>,
    /// The forms being drawn right now (a form among them that draws itself loops).
    open: HashSet<ObjectIdentifier>,
}

impl<'a> Forms<'a> {
    /// The Form XObjects `ops` draws, in order, repeats kept; `Do` names resolve in `xobjects`.
    fn children(&self, mut ops: TypedIter<'_>, xobjects: &Dict<'a>) -> Vec<Stream<'a>> {
        let mut out = Vec::new();
        while let Some(op) = ops.next() {
            if let TypedInstruction::XObject(x) = op
                && let Some(s) = xobjects.get::<Stream<'a>>(x.0)
                && matches!(s.dict().get::<Name<'_>>(b"Subtype").as_ref().map(AsRef::<[u8]>::as_ref), Some(b"Form"))
            {
                out.push(s);
            }
        }
        out
    }

    /// One draw of `form` (its `/Resources`, or `inherited` without them), and everything it
    /// draws in turn, counted into `drawn`.
    fn draw(&mut self, form: &Stream<'a>, inherited: &Scope<'a>, depth: usize, drawn: &mut u64) -> Result<(), IoError> {
        *drawn = drawn.saturating_add(1);
        if *drawn > MAX_FORM_DRAWS {
            return Err(IoError::Pdf(format!("PDF: the page draws Form XObjects more than {MAX_FORM_DRAWS} times, so it can't be opened")));
        }
        if depth > MAX_FORM_DEPTH {
            return Err(IoError::Pdf(format!("PDF: Form XObjects nest deeper than {MAX_FORM_DEPTH}, so the page can't be opened")));
        }
        let id = form.obj_id();
        if !self.open.insert(id) {
            return Err(IoError::Pdf("PDF: a Form XObject draws itself, so the page can't be opened".into()));
        }
        let own = form.dict().get::<Dict<'a>>(b"Resources").map(Resources::new);
        let scope = Scope::of(form, inherited);
        let kids = self.kids(form, &scope);
        for kid in &kids {
            self.draw(kid, &scope, depth + 1, drawn)?;
        }
        // The tiling patterns in the form's own resources, counted on every draw of the form (a
        // form without resources uses its parent's, whose patterns are counted there).
        if let Some(r) = own {
            for name in r.patterns.keys() {
                if let Some(p) = r.patterns.get::<Stream<'a>>(&name) {
                    self.pattern(&p, &scope, depth + 1, drawn)?;
                }
            }
        }
        self.open.remove(&id);
        Ok(())
    }

    /// The Form XObjects `stream`'s content draws with its names resolved in `scope`, read once
    /// per stream and scope.
    fn kids(&mut self, stream: &Stream<'a>, scope: &Scope<'a>) -> Vec<Stream<'a>> {
        let key = (stream.obj_id(), scope.id.clone());
        if let Some(k) = self.children.get(&key) {
            return k.clone();
        }
        // An undecodable stream draws nothing (the interpreter skips it too).
        let k = stream.decoded().map(|data| self.children(TypedIter::new(&data), &scope.x)).unwrap_or_default();
        self.children.insert(key, k.clone());
        k
    }

    /// One interpretation of a tiling pattern's cell (its `/Resources`, or `inherited`), counted
    /// into `drawn`; a cell drawing more than [`MAX_PATTERN_FORM_DRAWS`] forms is refused.
    fn pattern(&mut self, pattern: &Stream<'a>, inherited: &Scope<'a>, depth: usize, drawn: &mut u64) -> Result<(), IoError> {
        if depth > MAX_FORM_DEPTH {
            return Err(IoError::Pdf(format!("PDF: Form XObjects nest deeper than {MAX_FORM_DEPTH}, so the page can't be opened")));
        }
        let scope = Scope::of(pattern, inherited);
        let before = *drawn;
        for kid in self.kids(pattern, &scope) {
            self.draw(&kid, &scope, depth + 1, drawn)?;
            if drawn.saturating_sub(before) > MAX_PATTERN_FORM_DRAWS {
                return Err(IoError::Pdf(format!(
                    "PDF: a tiling pattern draws Form XObjects more than {MAX_PATTERN_FORM_DRAWS} times per fill, so the page can't be opened"
                )));
            }
        }
        Ok(())
    }
}

/// The walk of a page's resources for [`preflight`].
#[derive(Default)]
struct Walk {
    budget: Budget,
    /// The keys of what the content placed (charged already).
    placed: HashSet<u128>,
    /// Every image stream found, by the key the interpreter reports for it.
    known: HashMap<u128, Vec<Part>>,
    /// Streams walked already (a form or pattern several resource dictionaries share).
    visited: HashSet<ObjectIdentifier>,
    /// Streams visited, against [`MAX_XOBJECTS`].
    seen: usize,
}

impl Walk {
    fn count(&mut self) -> Result<(), IoError> {
        self.seen += 1;
        if self.seen > MAX_XOBJECTS {
            return Err(IoError::Pdf(format!("PDF: more than {MAX_XOBJECTS} XObjects to check, so the page can't be opened")));
        }
        Ok(())
    }

    /// `true` the first time `stream` is met.
    fn first(&mut self, stream: &Stream<'_>) -> bool {
        self.visited.insert(stream.obj_id())
    }

    /// Sizes an image stream the resources or a content stream reach, drawn or not: within the
    /// per-image limits, and charged once when the content never placed it (the renderer draws
    /// tiling patterns and soft masks on its own, which the content walk doesn't see).
    fn image(&mut self, stream: &Stream<'_>, what: &'static str) -> Result<(), IoError> {
        let parts = image_parts(stream, what)?;
        for p in &parts {
            p.fits()?;
        }
        let key = stream.cache_key();
        if !self.placed.contains(&key) {
            self.budget.charge(&parts)?;
        }
        // Identical dictionaries share a key: the costlier stream stands for them.
        let cost = |parts: &[Part]| parts.iter().map(Part::pixels).sum::<u64>();
        if self.known.get(&key).is_none_or(|k| cost(k) < cost(&parts)) {
            self.known.insert(key, parts);
        }
        Ok(())
    }

    /// The inline images among a content stream's operators.
    fn scan(&mut self, mut ops: TypedIter<'_>) -> Result<(), IoError> {
        while let Some(op) = ops.next() {
            if let TypedInstruction::InlineImage(i) = op {
                self.image(i.0, "an inline image")?;
            }
        }
        Ok(())
    }

    /// A content stream with resources of its own (a Form XObject, a tiling pattern, an annotation
    /// appearance): scanned for inline images, its resources walked.
    fn content(&mut self, stream: &Stream<'_>, depth: usize) -> Result<(), IoError> {
        if !self.first(stream) {
            return Ok(());
        }
        self.count()?;
        // Decoded as the interpreter decodes it, through general-purpose filters only.
        not_image_coded(stream, "a content stream")?;
        if let Ok(data) = stream.decoded() {
            self.scan(TypedIter::new(&data))?;
        }
        self.nested(stream.dict().get::<Dict<'_>>(b"Resources"), depth)
    }

    fn nested(&mut self, resources: Option<Dict<'_>>, depth: usize) -> Result<(), IoError> {
        match resources {
            Some(r) => self.resources(Resources::new(r), depth + 1),
            None => Ok(()),
        }
    }

    /// A resource dictionary: image XObjects are sized; Form XObjects, tiling patterns, Type 3
    /// glyph procedures and the groups of `/ExtGState` soft masks are scanned, with their own
    /// resources.
    fn resources(&mut self, res: Resources<'_>, depth: usize) -> Result<(), IoError> {
        if depth > MAX_FORM_DEPTH {
            return Err(IoError::Pdf(format!("PDF: Form XObjects nest deeper than {MAX_FORM_DEPTH}, so the page can't be opened")));
        }
        for name in res.x_objects.keys() {
            // Not a stream: the interpreter draws nothing for it.
            let Some(stream) = res.x_objects.get::<Stream<'_>>(&name) else { continue };
            match stream.dict().get::<Name<'_>>(b"Subtype").as_ref().map(AsRef::<[u8]>::as_ref) {
                Some(b"Image") => {
                    if self.first(&stream) {
                        self.count()?;
                        self.image(&stream, "an image")?;
                    }
                }
                Some(b"Form") => self.content(&stream, depth)?,
                _ => {}
            }
        }
        for name in res.patterns.keys() {
            // A tiling pattern is a content stream; a shading pattern is a dictionary, no image.
            if let Some(stream) = res.patterns.get::<Stream<'_>>(&name) {
                self.content(&stream, depth)?;
            }
        }
        for name in res.fonts.keys() {
            let Some(font) = res.fonts.get::<Dict<'_>>(&name) else { continue };
            if !matches!(font.get::<Name<'_>>(b"Subtype").as_ref().map(AsRef::<[u8]>::as_ref), Some(b"Type3")) {
                continue;
            }
            // Type 3 glyphs are content streams drawn with the font's resources (or the page's).
            if let Some(procs) = font.get::<Dict<'_>>(b"CharProcs") {
                for glyph in procs.keys() {
                    if let Some(stream) = procs.get::<Stream<'_>>(&glyph)
                        && self.first(&stream)
                    {
                        self.count()?;
                        not_image_coded(&stream, "a Type 3 glyph")?;
                        if let Ok(data) = stream.decoded() {
                            self.scan(TypedIter::new(&data))?;
                        }
                    }
                }
            }
            self.nested(font.get::<Dict<'_>>(b"Resources"), depth)?;
        }
        for name in res.ext_g_states.keys() {
            // A soft mask in the graphics state is a transparency group the renderer draws.
            let group = res.ext_g_states.get::<Dict<'_>>(&name).and_then(|gs| gs.get::<Dict<'_>>(b"SMask")).and_then(|sm| sm.get::<Stream<'_>>(b"G"));
            if let Some(stream) = group {
                self.content(&stream, depth)?;
            }
        }
        Ok(())
    }

    /// The appearance streams the interpreter draws for the page's annotations: the normal
    /// appearance, in the state `/AS` selects; hidden annotations are skipped.
    fn annotations(&mut self, page: &Page) -> Result<(), IoError> {
        for s in appearance_streams(page) {
            self.content(&s, 0)?;
        }
        Ok(())
    }
}

/// Refuses a page that would hand a stream other than an image to an image codec (DCT, JPX,
/// JBIG2, CCITT fax), before anything of the page is decoded. Those codecs size what they build by
/// their own data, which nothing checks outside an image (see [`part_of`]), and `hayro` applies a
/// stream's filters whatever the stream is for: the page's content, Form XObjects, tiling patterns,
/// shadings and functions, Type 3 glyph procedures, soft-mask groups, annotation appearances, font
/// programs and CMaps, ICC profiles, `/JBIG2Globals`. None of those has a use for an image codec.
///
/// Only dictionaries are read. Everything the page's content, resources (its own or inherited),
/// group and annotation appearances reach is walked; a stream counts as an image only where the
/// interpreter decodes one, an `/XObject` entry with `/Subtype /Image` and the `/SMask` and
/// `/Mask` streams of an image, and such an image's own codec is [`preflight`]'s to size. A walk
/// past [`MAX_OBJECTS`] or [`MAX_OBJECT_DEPTH`] refuses the page. So does a non-image stream that
/// inflates past [`MAX_CONTENT_BYTES`], and Form XObject draws past [`MAX_FORM_DRAWS`].
fn codec_check(page: &Page) -> Result<(), IoError> {
    let mut walk = Codecs::default();
    match page.raw().get::<Object<'_>>(b"Contents") {
        Some(Object::Stream(s)) => walk.stream(&s, b"Contents", 0)?,
        Some(Object::Array(a)) => {
            for s in a.iter::<Stream<'_>>() {
                walk.stream(&s, b"Contents", 0)?;
            }
        }
        _ => {}
    }
    let r = page.resources();
    walk.x_objects(&r.x_objects, None, 0)?;
    for d in [&r.ext_g_states, &r.fonts, &r.properties, &r.color_spaces, &r.patterns, &r.shadings] {
        walk.names(d, None, 0)?;
    }
    if let Some(group) = page.raw().get::<Dict<'_>>(b"Group") {
        walk.dict(&group, false, ref_of(page.raw(), b"Group"), 0)?;
    }
    if let Some(annots) = page.raw().get::<Array<'_>>(b"Annots") {
        for annot in annots.iter::<Dict<'_>>() {
            if let Some(ap) = annot.get::<Dict<'_>>(b"AP") {
                walk.dict(&ap, false, ref_of(&annot, b"AP"), 0)?;
            }
        }
    }
    // Every entry point that interprets a page runs this first, so the Form XObject draw count
    // is checked here too (after the streams it decodes were measured above).
    form_draws(page)
}

/// The indirect object `d`'s entry `key` refers to, if it is a reference. (A dictionary's own
/// `obj_id` is that of the object it sits in, direct or not.)
fn ref_of(d: &Dict<'_>, key: &[u8]) -> Option<ObjectIdentifier> {
    d.get_ref(key).map(ObjectIdentifier::from)
}

/// The walk of [`codec_check`].
#[derive(Default)]
struct Codecs {
    /// Indirect objects walked already, by whether they were met as an image (a stream met as
    /// both is checked as both).
    visited: HashSet<(ObjectIdentifier, bool)>,
    /// Dictionaries, arrays and streams walked, against [`MAX_OBJECTS`].
    seen: usize,
}

impl Codecs {
    /// Counts an object about to be walked; `false` when the indirect object `id` was walked
    /// already in this role.
    fn enter(&mut self, id: Option<ObjectIdentifier>, image: bool, depth: usize) -> Result<bool, IoError> {
        if depth > MAX_OBJECT_DEPTH || self.seen >= MAX_OBJECTS {
            return Err(IoError::Pdf("PDF: the page's resources nest too deep or hold too many objects to check, so the page can't be opened".to_string()));
        }
        if let Some(id) = id
            && !self.visited.insert((id, image))
        {
            return Ok(false);
        }
        self.seen += 1;
        Ok(true)
    }

    /// Entry `key` of `d`, whatever it holds.
    fn entry(&mut self, d: &Dict<'_>, key: &[u8], depth: usize) -> Result<(), IoError> {
        match d.get::<Object<'_>>(key) {
            Some(v) => self.value(key, v, ref_of(d, key), depth),
            None => Ok(()),
        }
    }

    /// `v`, found under `key`; `id` when it is an indirect object.
    fn value(&mut self, key: &[u8], v: Object<'_>, id: Option<ObjectIdentifier>, depth: usize) -> Result<(), IoError> {
        match v {
            Object::Dict(d) => self.dict(&d, false, id, depth),
            Object::Stream(s) => self.stream(&s, key, depth),
            Object::Array(a) => {
                if self.enter(id, false, depth)? {
                    for v in a.iter::<Object<'_>>() {
                        self.value(key, v, None, depth + 1)?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// A dictionary (`image` for an image stream's, whose masks are images too).
    fn dict(&mut self, d: &Dict<'_>, image: bool, id: Option<ObjectIdentifier>, depth: usize) -> Result<(), IoError> {
        if !self.enter(id, image, depth)? {
            return Ok(());
        }
        self.entries(d, image, depth)
    }

    /// An object's entries. Keys that lead back up the page tree or to data the renderer never
    /// decodes are skipped; the dictionaries that map names a file chooses (resources, Type 3
    /// glyphs, appearance states, colorants) are walked whole, whatever the names.
    fn entries(&mut self, d: &Dict<'_>, image: bool, depth: usize) -> Result<(), IoError> {
        for key in d.keys() {
            let k: &[u8] = key.as_ref();
            match k {
                b"Parent" | b"P" | b"Metadata" | b"PieceInfo" | b"Thumb" | b"AF" | b"EF" | b"OPI" | b"Ref" | b"Alternates" => {}
                b"XObject" => {
                    if let Some(x) = d.get::<Dict<'_>>(k) {
                        self.x_objects(&x, ref_of(d, k), depth + 1)?;
                    }
                }
                b"Font" | b"ExtGState" | b"ColorSpace" | b"Pattern" | b"Shading" | b"Properties" | b"CharProcs" | b"Colorants" | b"N" | b"R" | b"D" => {
                    match d.get::<Object<'_>>(k) {
                        Some(Object::Dict(names)) => self.names(&names, ref_of(d, k), depth + 1)?,
                        Some(v) => self.value(k, v, ref_of(d, k), depth + 1)?,
                        None => {}
                    }
                }
                b"SMask" | b"Mask" if image => match d.get::<Object<'_>>(k) {
                    Some(Object::Stream(m)) => self.image(&m, depth + 1)?,
                    Some(v) => self.value(k, v, ref_of(d, k), depth + 1)?,
                    None => {}
                },
                _ => self.entry(d, k, depth + 1)?,
            }
        }
        Ok(())
    }

    /// A dictionary of names the file chose: every entry is walked.
    fn names(&mut self, d: &Dict<'_>, id: Option<ObjectIdentifier>, depth: usize) -> Result<(), IoError> {
        if !self.enter(id, false, depth)? {
            return Ok(());
        }
        for key in d.keys() {
            self.entry(d, key.as_ref(), depth + 1)?;
        }
        Ok(())
    }

    /// A stream met anywhere but as an image (`key` names where): refused if an image codec
    /// decodes it.
    fn stream(&mut self, s: &Stream<'_>, key: &[u8], depth: usize) -> Result<(), IoError> {
        if !self.enter(Some(s.obj_id()), false, depth)? {
            return Ok(());
        }
        let what = format!("the /{} stream", String::from_utf8_lossy(key));
        not_image_coded(s, &what)?;
        content_bounded(s, &what)?;
        self.entries(s.dict(), false, depth)
    }

    /// An image stream: its codec is the image's (sized by [`preflight`]); what it refers to (an
    /// ICC profile, `/JBIG2Globals`) is checked.
    fn image(&mut self, s: &Stream<'_>, depth: usize) -> Result<(), IoError> {
        if !self.enter(Some(s.obj_id()), true, depth)? {
            return Ok(());
        }
        self.entries(s.dict(), true, depth)
    }

    /// An `/XObject` dictionary: the interpreter decodes an entry as an image by its `/Subtype`.
    fn x_objects(&mut self, x: &Dict<'_>, id: Option<ObjectIdentifier>, depth: usize) -> Result<(), IoError> {
        if !self.enter(id, false, depth)? {
            return Ok(());
        }
        for key in x.keys() {
            match x.get::<Object<'_>>(&key) {
                Some(Object::Stream(s)) if s.dict().get::<Name<'_>>(b"Subtype").as_ref().map(AsRef::<[u8]>::as_ref) == Some(b"Image") => {
                    self.image(&s, depth + 1)?
                }
                Some(v) => self.value(key.as_ref(), v, ref_of(x, key.as_ref()), depth + 1)?,
                None => {}
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------------------------------- marks

/// What a page draws, as a [`Device`]: the extent of every mark (clipped), counts per kind, and
/// the raster images with their placement (decoded when asked, for the image-only shortcut).
struct Marks {
    clips: Vec<PtRect>,
    bounds: Option<PtRect>,
    paths: usize,
    glyphs: usize,
    stencils: usize,
    /// Which raster image (by order of drawing) to decode; the others are only placed.
    decode: Option<usize>,
    images: Vec<Placed>,
    /// Every image and stencil placed, inline images included, in drawing order.
    placements: Vec<Placement>,
}

/// One image the content placed, for the budget: what the renderer would decode for it.
struct Placement {
    /// The interpreter's key for the image's stream (identical dictionaries share one).
    key: u128,
    /// The pixel size the file declares.
    size: (u32, u32),
    /// The image and its masks sized from its stream; `None` for a stencil, whose stream the
    /// interpreter doesn't show ([`preflight`] sizes it from the resources instead).
    parts: Option<Result<Vec<Part>, IoError>>,
}

struct Placed {
    transform: Affine,
    /// The pixel size the file declares.
    size: (u32, u32),
    decoded: Option<(ImageData, Option<LumaData>)>,
}

impl Marks {
    fn mark(&mut self, r: PtRect) {
        let r = match self.clips.last() {
            Some(c) => r.intersect(*c),
            None => r,
        };
        if !(r.width() > 0.0 && r.height() > 0.0) || !r.x0.is_finite() || !r.y0.is_finite() || !r.x1.is_finite() || !r.y1.is_finite() {
            return;
        }
        self.bounds = Some(self.bounds.map_or(r, |b| b.union(r)));
    }

    /// The marks' box cut to `within`, or `within` itself for a page that draws nothing.
    fn bounds_within(&self, within: [f32; 4]) -> [f32; 4] {
        match self.bounds {
            Some(b) => {
                let r = inter([b.x0 as f32, b.y0 as f32, b.x1 as f32, b.y1 as f32], within);
                if has_area(r) { r } else { within }
            }
            None => within,
        }
    }
}

/// Interprets `page` through a [`Marks`] device, in PDF points (y up); `decode` picks the raster
/// image (by drawing order) whose pixels are wanted.
fn marks(page: &Page, decode: Option<usize>) -> Marks {
    let mut device = Marks { clips: Vec::new(), bounds: None, paths: 0, glyphs: 0, stencils: 0, decode, images: Vec::new(), placements: Vec::new() };
    let cache = InterpreterCache::new();
    let media = to_pt(norm(page.media_box()));
    let mut context = Context::new(Affine::IDENTITY, media, &cache, page.xref(), settings());
    interpret_page(page, &mut context, &mut device);
    device
}

impl<'a> Device<'a> for Marks {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, draw_mode: &DrawMode) {
        let mut r = path.bounding_box();
        if let DrawMode::Stroke(s) | DrawMode::FillAndStroke(_, s) = draw_mode {
            let grow = (f64::from(s.line_width) / 2.0).max(4.0) + 2.0;
            r = r.inflate(grow, grow);
        }
        self.mark(props.transform.transform_rect_bbox(r));
        self.paths = self.paths.saturating_add(1);
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        let r = clip_path.path.bounding_box();
        let r = match self.clips.last() {
            Some(c) => r.intersect(*c),
            None => r,
        };
        self.clips.push(r);
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}

    fn draw_glyph_run(&mut self, glyph_run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        let b = PtRect::new(GLYPH_BOX[0], GLYPH_BOX[1], GLYPH_BOX[2], GLYPH_BOX[3]);
        for g in glyph_run.glyphs() {
            self.mark((props.transform * g.transform()).transform_rect_bbox(b));
            self.glyphs = self.glyphs.saturating_add(1);
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        // The transform places the image's own pixel grid (top row first).
        let (w, h) = (f64::from(image.width()), f64::from(image.height()));
        self.mark(props.transform.transform_rect_bbox(PtRect::new(0.0, 0.0, w, h)));
        let (key, size) = (image.cache_key(), (image.width(), image.height()));
        match image {
            Image::Stencil(_) => {
                self.stencils = self.stencils.saturating_add(1);
                self.placements.push(Placement { key, size, parts: None });
            }
            Image::Raster(r) => {
                // Sized from its stream, inline images included; nothing is decoded for that.
                let what = if r.stream().dict().obj_id().is_none() { "an inline image" } else { "an image" };
                let parts = image_parts(r.stream(), what);
                let mut decoded = None;
                // Decoded only within the budget ([`preflight`] refused the page otherwise).
                if self.decode == Some(self.images.len()) && parts.as_ref().is_ok_and(|p| p.iter().all(|x| x.fits().is_ok())) {
                    r.with_rgba(|data, alpha| decoded = Some((data, alpha)), None);
                }
                self.images.push(Placed { transform: props.transform, size, decoded });
                self.placements.push(Placement { key, size, parts: Some(parts) });
            }
        }
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn pop_transparency_group(&mut self) {}
}

/// The image of a page that draws exactly one raster image and nothing else, as the document
/// Photoshop opens for it (see [`image_document`]). `None` for any other page.
fn image_only(name: &str, page: &Page) -> Result<Option<Document>, IoError> {
    let m = marks(page, Some(0));
    if m.paths != 0 || m.glyphs != 0 || m.stencils != 0 || m.images.len() != 1 {
        return Ok(None);
    }
    Ok(m.images.first().and_then(|p| image_document(name, p)))
}

/// A placed, decoded image as Photoshop opens it: its own pixels on a locked `Background`
/// layer, Grayscale or RGB as the image is, 8 bits, at the resolution its placed width implies
/// (rotation and uneven scaling in the matrix are ignored, as Photoshop ignores them). `None`
/// when it wasn't decoded or is absurd.
fn image_document(name: &str, placed: &Placed) -> Option<Document> {
    let (data, alpha) = placed.decoded.as_ref()?;
    let (w, h) = (data.width(), data.height());
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PAGE_PIXELS {
        return None;
    }
    // The placed width in points: the declared pixel width along the matrix's x axis.
    let c = placed.transform.as_coeffs();
    let placed_pt = c[0].hypot(c[1]) * f64::from(placed.size.0);
    let ppi = if placed_pt > 1.0e-6 { (f64::from(w) * 72.0 / placed_pt) as f32 } else { 72.0 };
    let (mode, n, color): (ColorMode, usize, &[u8]) = match data {
        ImageData::Luma(l) => (ColorMode::Grayscale, 1, &l.data),
        ImageData::Rgb(r) => (ColorMode::Rgb, 3, &r.data),
    };
    let count = (w as usize).saturating_mul(h as usize);
    if color.len() < count.saturating_mul(n) {
        return None;
    }
    let alpha = alpha.as_ref().filter(|a| a.width == w && a.height == h && a.data.len() >= count);
    let mut doc = Document::new(name, Size::new(w, h), mode, SampleType::U8);
    doc.resolution_dpi = ppi.clamp(MIN_RESOLUTION, MAX_RESOLUTION);
    let fmt = doc.pixel_format();
    let mut px = Vec::with_capacity(count.saturating_mul(n + 1));
    for (i, p) in color.chunks_exact(n).take(count).enumerate() {
        px.extend(p.iter().map(|&v| f32::from(v) / 255.0));
        px.push(alpha.and_then(|a| a.data.get(i)).map_or(1.0, |&a| f32::from(a) / 255.0));
    }
    let mut layer = Layer::raster("Background", fmt);
    layer.locks.transparency = true;
    layer.locks.position = true;
    if let Some(s) = layer.surface_mut() {
        s.write_region(doc.bounds(), &px);
    }
    doc.layers.push(layer);
    Some(doc)
}

#[cfg(test)]
mod tests;
