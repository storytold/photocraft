//! Real-file corpus sweeps (`corpus/`, fetched by `cargo xtask corpus --all`): a full sweep
//! ([`full_sweep`]: the nightly CI run) takes every file, every other run a fixed sample per
//! directory. The samples are chosen for coverage, not taken as the first files: together they
//! hold every colour mode and depth, PSD and PSB, and every tagged block key the directory has
//! outside its large files, plus the cases the corpus tests guard (each oracle kind, smart objects,
//! vector blocks, type), at small canvas sizes so that each corpus test stays near a second.

use std::path::{Path, PathBuf};

/// Whether this run takes every case of the long sweeps ([`crate::Sweep`]) and every corpus file:
/// `PHOTOCRAFT_FULL_SWEEP=1`, set by the nightly CI run and by a manual run's `full_sweep` input.
pub fn full_sweep() -> bool {
    std::env::var_os("PHOTOCRAFT_FULL_SWEEP").is_some_and(|v| v == "1")
}

/// The workspace's `corpus/` directory.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

/// The fixed sample of `corpus/<dir>` (paths relative to it), empty for a directory without one.
pub fn sample(dir: &str) -> &'static [&'static str] {
    match dir {
        "psd" => PSD,
        "psd-tools" => PSD_TOOLS,
        "photoshop" => PHOTOSHOP,
        _ => &[],
    }
}

/// The PSD and PSB files of `corpus/<dir>` this run takes, sorted: all of them in a full sweep,
/// otherwise the directory's sample. Fails when the directory is missing, or when a sample file is
/// (a pin moved: choose its replacement for the sample).
pub fn psd_files(dir: &str) -> Vec<PathBuf> {
    let base = root().join(dir);
    let all = all_psd_files(dir);
    if full_sweep() {
        eprintln!("corpus/{dir}: all {} files (PHOTOCRAFT_FULL_SWEEP=1)", all.len());
        return all;
    }
    let picked: Vec<PathBuf> = sample(dir).iter().map(|name| base.join(name)).collect();
    let missing: Vec<&PathBuf> = picked.iter().filter(|p| !all.contains(p)).collect();
    assert!(missing.is_empty(), "corpus/{dir}: sample files missing from the corpus (update the sample in photocraft-testkit): {missing:?}");
    eprintln!("corpus/{dir}: sample of {} of {} files; PHOTOCRAFT_FULL_SWEEP=1 runs every file", picked.len(), all.len());
    picked
}

/// Every PSD and PSB file of `corpus/<dir>`, sorted, in every run: for the corpus tests fast
/// enough to walk a whole directory. Fails when the directory is missing.
pub fn all_psd_files(dir: &str) -> Vec<PathBuf> {
    let base = root().join(dir);
    assert!(base.is_dir(), "{} is missing: run `cargo xtask corpus --all`", base.display());
    let mut all = Vec::new();
    collect(&base, &mut all);
    all.sort();
    all
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("psd") || e.eq_ignore_ascii_case("psb")) {
            out.push(p);
        }
    }
}

/// `corpus/psd`: the files ag-psd wrote (another writer's structures); its psd-tools copies are in
/// [`PSD_TOOLS`].
const PSD: &[&str] = &[
    "ag-psd/layer-mask/src.psd",                  // layer mask, FMsk
    "ag-psd/read-write/vector-and-mask/src.psd",  // vector mask with raster mask, plain shape, SoCo
    "ag-psd/read-write/winding-non-zero/src.psd", // non-zero winding shape
    "ag-psd/read-write/gradient/src.psd",         // vsms and vstk vector blocks
    "ag-psd/read-write/text/src.psd",             // type layer (TySh)
    "ag-psd/read-write/annotations/src.psd",      // annotations
    "ag-psd/read-write/layer-comps/src.psd",      // layer comps; thumbnail oracle
    "ag-psd/read-write/masks/src.psd",            // mask parameters
    "ag-psd/read-write/shapes/src.psd",           // linked and embedded smart objects (lnk2, lnkE), 15 vector blocks
];

/// `corpus/psd-tools`.
const PSD_TOOLS: &[&str] = &[
    "colormodes/100x20_1bit_bitmap_rle.psd",        // Bitmap, RLE; oracle through the psd crate
    "colormodes/4x4_8bit_index_color.psd",          // Indexed
    "colormodes/4x4_8bit_duotone.psd",              // Duotone (layered TIFF writes it flat)
    "colormodes/4x4_16bit_lab.psd",                 // Lab 16
    "colormodes/4x4_16bit_multichannel.psd",        // Multichannel
    "cmyk-gray-ramp.psd",                           // CMYK
    "colormodes/4x4_32bit_grayscale.psd",           // Grayscale 32
    "16bit5x5.psb",                                 // PSB (8-byte lengths), RGB 16
    "32bit5x5.psd",                                 // RGB 32, layered
    "2layer_8ele_tblocks.psd",                      // a 1 px document with Tblocks
    "layers-minimal/smartobject-layer.psd",         // embedded smart object
    "layers-minimal/type-layer.psd",                // type layer
    "layers-minimal/shape-layer.psd",               // shape layer
    "layers-minimal/solid-color-fill.psd",          // fill layer; thumbnail oracle
    "layers/curves-with-vectormask.psd",            // adjustment layer with a vector mask
    "layers/gradient-map.psd",                      // gradient map adjustment
    "effects/double-stroke-effects.psd",            // multiple layer effects of one kind
    "blend-modes/dissolve.psd",                     // dissolve oracle metric
    "transparency/knockout-deep-nested.psd",        // knockout in nested groups
    "passthrough_clipping_mask_adjustment.psd",     // pass-through group, clipping mask
    "mask-density-layervectormask.psd",             // layer and vector mask densities
    "descriptors/stroke-color-descriptors-lab.psd", // Lab colour descriptors
    "smart-object-slice.psb",                       // smart object in a PSB, slices
];

/// `corpus/photoshop`: one file per feature group (each `adjustments/<mode><bits>`, effects, smart
/// filters, type).
const PHOTOSHOP: &[&str] = &[
    "adjustments/rgb8/hue-saturation.psd",
    "adjustments/rgb16/gradient-map.psd",
    "adjustments/rgb32/brightness-contrast.psd",
    "adjustments/gray8/curves.psd",
    "adjustments/gray16/posterize-4.psd",
    "adjustments/gray32/levels.psd",
    "adjustments/lab8/threshold-128.psd",
    "adjustments/lab16/posterize-4.psd",
    "adjustments/cmyk8/levels.psd",
    "adjustments/cmyk16/color-balance.psd",
    "effects/outer-glow-soft.psd",
    "effects/bevel-inner-smooth.psd",
    "smart-filters/sf-curves.psd",               // re-renders like Photoshop (engine oracle)
    "smart-filters/sf-stack-blend-and-mask.psd", // filter stack with blending and a filter mask
    "text/point-sizes-colors-runs.psd",          // Txt2, style runs
];
