//! The file types the Open and Save pickers offer, as [`PickRequest`] builders. Extensions are
//! listed in lower case; the file service adds the case variants a case-sensitive picker needs.

use std::path::{Path, PathBuf};

use craft::host::PickRequest;
use photocraft_color::{ColorMode, SampleType};

/// Everything File › Open reads: PhotoCraft, Photoshop, OpenRaster, Krita and Affinity documents, flat images, and
/// Photoshop brushes (.abr), gradients (.grd), layer styles (.asl), swatches (.aco, .act, .ase) and keyboard shortcut sets
/// (.kys), which go to the preset libraries and Edit › Keyboard Shortcuts.
pub(super) const OPEN_EXTS: &[&str] = &[
    "pcraft", "pdn", "ora", "xcf", "xcfgz", "gz", "kra", "krz", "psd", "psb", "psdt", "png", "jpg", "jpeg", "tif", "tiff", "webp", "gif", "bmp", "dib", "tga",
    "ico", "qoi", "exr", "hdr", "pbm", "pgm", "ppm", "pnm", "pam", "pfm", "pcx", "dds", "xpm", "fits", "fit", "fts", "dpx", "cin", "dcm", "dicom", "dc3",
    "iff", "lbm", "ilbm", "heic", "heif", "hif", "dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf", "abr", "grd", "asl", "svg", "svgz",
    "aco", "act", "ase", "kys", "af", "afdesign", "afphoto", "afpub",
];
const SVG_EXTS: &[&str] = &["svg", "svgz"];

/// File › Open's file types after All Formats, as Photoshop lists them: one per format, so a folder
/// of mixed files can be narrowed to one kind (#6337). The document formats, the rest by name as in
/// [`SAVE_FILTERS`], then the app's own resources. Together they cover every extension of
/// [`OPEN_EXTS`]. An explicit SVG filter also keeps vector artwork discoverable when a native
/// picker defaults to another filter.
pub(super) const OPEN_FILTERS: &[(&str, &[&str])] = &[
    ("Photoshop", &["psd", "psb", "psdt"]),
    ("PhotoCraft", &["pcraft"]),
    ("Affinity", &["af", "afdesign", "afphoto", "afpub"]),
    ("Amiga IFF", &["iff", "lbm", "ilbm"]),
    ("BMP", &["bmp", "dib"]),
    ("DDS", &["dds"]),
    ("DICOM", &["dcm", "dicom", "dc3"]),
    ("DPX and Cineon", &["dpx", "cin"]),
    ("FITS", &["fits", "fit", "fts"]),
    ("GIF", &["gif"]),
    ("GIMP", &["xcf", "xcfgz", "gz"]),
    ("HEIF", &["heic", "heif", "hif"]),
    ("ICO", &["ico"]),
    ("JPEG", &["jpg", "jpeg"]),
    ("Krita", &["kra", "krz"]),
    ("OpenEXR", &["exr"]),
    ("OpenRaster", &["ora"]),
    ("Paint.NET", &["pdn"]),
    ("PCX", &["pcx"]),
    ("PNG", &["png"]),
    ("Portable Bitmap", &["pbm", "pgm", "ppm", "pnm", "pam", "pfm"]),
    ("QOI", &["qoi"]),
    ("Radiance HDR", &["hdr"]),
    ("RAW images", &["dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf"]),
    ("SVG", SVG_EXTS),
    ("Targa", &["tga"]),
    ("TIFF", &["tif", "tiff"]),
    ("WebP", &["webp"]),
    ("XPM", &["xpm"]),
    ("Brushes", &["abr"]),
    ("Gradients", &["grd"]),
    ("Styles", &["asl"]),
    ("Swatches", &["aco", "act", "ase"]),
    ("Keyboard Shortcuts", &["kys"]),
];

/// File › Save As formats: (filter name, extensions). PSD, PSB and PhotoCraft's own format, which
/// keep the most of a document, then the rest by name, so a long list stays easy to search. The
/// filter matching the suggested name's extension comes first, so a .pcraft document saves as
/// .pcraft by default and everything else keeps defaulting to PSD. The PSD type takes `.psb` too:
/// Windows adds the selected type's extension to a name whose extension isn't one of the type's,
/// so `art.psb` typed under the PSD type would come back as `art.psb.psd`.
pub(super) const SAVE_FILTERS: &[(&str, &[&str])] = &[
    ("PSD Document", &["psd", "psb"]),
    ("PSB Large Document", &["psb"]),
    ("PhotoCraft", &["pcraft"]),
    ("Amiga IFF", &["iff"]),
    ("BMP", &["bmp", "dib"]),
    ("Cineon", &["cin"]),
    // Game and 3D textures; DDS Options asks for the compression and mipmaps (#1327).
    ("DDS", &["dds"]),
    ("DPX", &["dpx"]),
    ("FITS", &["fits", "fit"]),
    ("GIF", &["gif"]),
    // BMP and GIF are also in Photoshop's Save As list; ICO is PhotoCraft's own (#4357): up to
    // 256×256, with alpha.
    ("ICO", &["ico"]),
    ("JPEG", &["jpg", "jpeg"]),
    ("JPEG XL", &["jxl"]),
    // The encoder picks P5, P6, P7 or PFM from the image, not from the name, so only the neutral
    // .pnm is offered: a .pbm or .pgm name would hold pixmap data (CodeRabbit on #9775).
    ("Netpbm", &["pnm"]),
    ("OpenEXR", &["exr"]),
    ("OpenRaster", &["ora"]),
    ("PDF", &["pdf"]),
    ("PNG", &["png"]),
    ("Radiance HDR", &["hdr"]),
    ("Targa", &["tga"]),
    ("TIFF", &["tif", "tiff"]),
    ("WebP", &["webp"]),
];

/// Save a Copy has measured type lists for these mode/depth combinations.
#[derive(Clone, Copy)]
pub(super) enum SaveCopyFormats {
    Rgb8,
    Rgb32,
    Cmyk8,
}

impl SaveCopyFormats {
    pub(super) fn for_document(mode: ColorMode, depth: SampleType) -> Option<Self> {
        match (mode, depth) {
            (ColorMode::Rgb, SampleType::U8) => Some(Self::Rgb8),
            (ColorMode::Rgb, SampleType::F32) => Some(Self::Rgb32),
            (ColorMode::Cmyk, SampleType::U8) => Some(Self::Cmyk8),
            _ => None,
        }
    }

    fn includes(self, ext: &str) -> bool {
        match self {
            Self::Rgb8 => !matches!(ext, "cin" | "exr" | "hdr"),
            // JPEG XL can read floats, but its writer reduces them to integer samples.
            Self::Rgb32 => {
                matches!(ext, "psd" | "psb" | "pcraft")
                    || photocraft_codecs::from_extension(ext).is_some_and(|format| {
                        let caps = format.caps();
                        caps.write && caps.depths.contains(&photocraft_codecs::SampleType::F32) && caps.layouts.iter().any(|layout| layout.is_rgb())
                    })
            }
            Self::Cmyk8 => matches!(ext, "psd" | "psb" | "pcraft" | "jpg" | "pdf" | "tif"),
        }
    }

    fn lists_extension(self, ext: &str) -> bool {
        SAVE_FILTERS.iter().any(|(_, exts)| self.includes(exts[0]) && exts.iter().any(|allowed| ext.eq_ignore_ascii_case(allowed)))
    }

    pub(super) fn allows_path(self, path: &Path) -> bool {
        let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else { return false };
        // A typed extension can also name a supported codec via an alias, such as .fts for FITS.
        self.lists_extension(ext) || photocraft_codecs::from_extension(ext).is_some_and(|format| format.caps().write && self.includes(format.extensions()[0]))
    }
}

/// Non-document files the shell saves (Swatches panel exports): offered alone, so the dialog
/// never swaps their extension for a document format's.
const OTHER_SAVE_FILTERS: &[(&str, &[&str])] = &[("Color Swatches", &["aco"]), ("Color Table", &["act"]), ("Swatch Exchange", &["ase"])];

/// The extensions a Netpbm subtype's name carries; the save types offer only the neutral `.pnm`.
const NETPBM_SUBTYPES: &[&str] = &["pbm", "pgm", "ppm", "pam", "pfm"];

/// `req` with the Open picker's types: All Formats then one type per format, or, for a command
/// that reads one kind of file (`extensions` given), only those as "Supported Files", so it does
/// not inherit the image formats.
pub(super) fn open_filters(req: PickRequest, extensions: &[&str]) -> PickRequest {
    if !extensions.is_empty() {
        return req.filter("Supported Files", extensions);
    }
    OPEN_FILTERS.iter().fold(req.filter("All Formats", OPEN_EXTS), |req, (name, exts)| req.filter(*name, exts))
}

/// `suggested` with a Netpbm subtype's extension (an opened .pgm) as the neutral .pnm, the name
/// every save dialog starts from: the encoder picks P5, P6, P7 or PFM from the image, so the old
/// suffix could name other data, and macOS keeps the suggested name as it is.
fn netpbm_as_pnm(suggested: &str) -> String {
    let path = Path::new(suggested);
    let subtype = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).is_some_and(|e| NETPBM_SUBTYPES.contains(&e.as_str()));
    if subtype { path.with_extension("pnm").to_string_lossy().into_owned() } else { suggested.to_string() }
}

/// Lists the save dialog's file types with the `suggested` type first (added if unlisted), so the dialog keeps that extension instead of .psd.
fn save_filters(suggested: &Path, copy: Option<SaveCopyFormats>) -> Vec<(String, Vec<String>)> {
    let ext = suggested.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    // A Netpbm subtype's name (an opened .pgm) starts on the Netpbm type as .pnm: the encoder picks
    // the subtype from the image, so the old name could describe other data (CodeRabbit on #9775).
    let ext = if NETPBM_SUBTYPES.contains(&ext.as_str()) { "pnm".to_string() } else { ext };
    if let Some((name, exts)) = OTHER_SAVE_FILTERS.iter().find(|(_, exts)| exts.contains(&ext.as_str())) {
        return vec![(name.to_string(), exts.iter().map(|e| e.to_string()).collect())];
    }
    let mut v: Vec<(String, Vec<String>)> = SAVE_FILTERS
        .iter()
        .filter(|(_, exts)| copy.is_none_or(|formats| formats.includes(exts[0])))
        .map(|(name, exts)| (name.to_string(), exts.iter().map(|e| e.to_string()).collect()))
        .collect();
    // The type the extension is first for (`.psb` starts on PSB, not on PSD's second extension),
    // else any type that lists it.
    let own = v.iter().position(|(_, exts)| exts.first() == Some(&ext));
    match own.or_else(|| v.iter().position(|(_, exts)| exts.contains(&ext))) {
        Some(i) => {
            let (name, mut exts) = v.remove(i);
            // Windows adds the first type's first extension to a bare name: `cat.jpeg` stays
            // `.jpeg`, and `sky.fit` stays `.fit`.
            exts.retain(|e| *e != ext);
            exts.insert(0, ext);
            v.insert(0, (name, exts));
        }
        None if !ext.is_empty() => v.insert(0, (ext.to_ascii_uppercase(), vec![ext])),
        None => {}
    }
    v
}

/// The Save picker for `purpose`, suggesting the file `suggested` (a name, or a path whose folder
/// the picker starts in) with the types of [`save_filters`]. A name with an extension suggests it
/// for an answer typed without one.
pub(super) fn save_request(purpose: &str, suggested: &str, copy: Option<SaveCopyFormats>) -> PickRequest {
    let suggested = netpbm_as_pnm(suggested);
    let mut path = PathBuf::from(suggested);
    if let Some(formats) = copy
        && !path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| formats.lists_extension(ext))
    {
        path.set_extension("psd");
    }
    let mut req = PickRequest::save(purpose);
    if let Some(name) = path.file_name() {
        req = req.file_name(name.to_string_lossy());
    }
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        req = req.suggested_extension(ext);
    }
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        req = req.directory(dir);
    }
    for (name, exts) in save_filters(&path, copy) {
        let exts: Vec<&str> = exts.iter().map(String::as_str).collect();
        req = req.filter(name, &exts);
    }
    req
}
