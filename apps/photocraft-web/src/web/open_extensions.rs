//! Suffixes passed to rfd's browser File Upload filter.
//!
//! rfd prefixes each entry with a dot and forwards it to HTML `accept`. Compound
//! suffixes keep XCF bzip2/XZ inputs selectable without admitting all `.bz2`/`.xz`
//! archives. The filter is advisory; import still validates the selected bytes.

/// Everything File › Open reads: PhotoCraft, Photoshop, OpenRaster, Krita and Affinity documents, flat images, and
/// Photoshop brushes (.abr), gradients (.grd), layer styles (.asl) and swatches (.aco, .act, .ase), which go to the preset libraries.
pub(super) const OPEN_EXTS: &[&str] = &[
    "pcraft", "pdn", "ora", "xcf", "xcfgz", "gz", "xcf.gz", "xcf.bz2", "xcfbz2", "xcf.xz", "xcfxz", "kra", "krz", "psd", "psb", "psdt", "png", "jpg", "jpeg",
    "tif", "tiff", "webp", "gif", "bmp", "tga", "ico", "qoi", "exr", "hdr", "pbm", "pgm", "ppm", "pam", "pfm", "dds", "fits", "fit", "fts", "dpx", "cin",
    "dcm", "dicom", "dc3", "iff", "lbm", "ilbm", "heic", "heif", "hif", "dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf", "abr", "grd",
    "asl", "svg", "svgz", "aco", "act", "ase", "af", "afdesign", "afphoto", "afpub",
];

#[cfg(test)]
mod tests {
    use super::OPEN_EXTS;

    #[test]
    fn open_picker_includes_all_xcf_suffixes_without_generic_bzip2_or_xz() {
        for suffix in ["xcf", "xcf.gz", "xcfgz", "xcf.bz2", "xcfbz2", "xcf.xz", "xcfxz"] {
            assert!(OPEN_EXTS.contains(&suffix), "missing picker suffix .{suffix}");
            assert!(photocraft_io::is_xcf_input_name(&format!("page.{suffix}")), "picker suffix must be admitted by import");
        }
        assert!(!OPEN_EXTS.contains(&"bz2"));
        assert!(!OPEN_EXTS.contains(&"xz"));
        // Preserve existing SVG/generic gzip and native/Photoshop picker behavior.
        for suffix in ["gz", "svg", "svgz", "pcraft", "psd", "psb", "afphoto", "png", "dng", "abr"] {
            assert!(OPEN_EXTS.contains(&suffix), "existing picker suffix .{suffix}");
        }
        let mut unique = std::collections::HashSet::new();
        for suffix in OPEN_EXTS {
            assert!(!suffix.is_empty() && !suffix.starts_with('.') && !suffix.contains(','));
            assert!(unique.insert(suffix), "duplicate picker suffix .{suffix}");
        }
    }
}
