//! Integration tests of `photocraft-io`: one test binary (one link) with a module per area.
//! A new integration test goes into `tests/it/<name>.rs` plus a `mod <name>;` line here,
//! not into a new top-level `tests/*.rs` file (each of those links a binary of its own).

mod common;

mod adjust_blend_roundtrip;
mod adjust_warnings;
mod affinity;
mod affinity_corpus;
mod annotations;
mod artboard_guide_order;
mod banded;
mod cancel;
mod channels;
mod color;
mod composite;
mod comps_artboards;
mod corpus;
mod corpus_regressions;
mod effects_multi;
mod exif_resolution;
mod flat;
mod heif;
mod heif_unsupported;
mod lab16;
mod large_psb;
mod layer_labels;
mod modes;
mod orientation;
mod pattern_fill_psd;
mod pdn_corpus;
mod png_text_metadata;
mod psd32_export;
mod psd_block_layout;
mod psd_empty_channels;
mod psd_kerning;
mod psd_structure;
mod psd_type_bounds;
mod psd_vertical_type;
mod raw;
mod resolution_axes;
mod roundtrip;
mod smart_contents_identity;
mod smart_corpus;
mod svg;
mod text_corpus;
mod text_styles;
mod tiff_corpus;
mod tiff_layers;
mod tiff_pages;
mod vector;
mod xmp_export;
