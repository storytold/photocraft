//! **Every test-corpus pin lives here.** Real-file corpora are never committed to this
//! repository: `cargo xtask corpus --all` fetches them into `corpus/` (gitignored) at the commits
//! below and verifies each file against the `xtask/*.sha256` manifests. The CI corpus job's cache
//! key hashes this file and the manifests, so changing a pin is the only thing that refetches.
//!
//! - Our own oracles: https://github.com/storytold/photocraft-corpus (README, AGENTS.md).
//! - Third-party sets come from their upstreams (licences in ATTRIBUTION.md).
//!
//! Moving a pin: change the commit, run `cargo xtask corpus --<name> --update-manifest`, review the
//! manifest diff, and re-run `cargo xtask test-corpus` (adjust floors only upwards).

use crate::pinned::{PinnedCorpus, Upstream};

/// https://github.com/storytold/photocraft-corpus: our Photoshop-authored oracle PSDs
/// (`photoshop/`). Bump through a PR after committing there.
pub const PHOTOCRAFT_CORPUS_COMMIT: &str = "f5b1178cab15309e05b7b504545df3c701f6d8fd";
/// https://github.com/psd-tools/psd-tools (MIT): `tests/psd_files` (main, 2026-10-05).
pub const PSD_TOOLS_COMMIT: &str = "96eb134c17b2c65edf4c4151c0f00b802ada86c2";
/// https://github.com/Agamnentzar/ag-psd (MIT): `test/` (master, 2026-07-02).
pub const AG_PSD_COMMIT: &str = "387049670cb89b88fb8fe1b7c01aeacf98dd2e3b";
/// https://github.com/tbraun96/heic-rs (MIT OR Apache-2.0): `tests/fixtures` at the v0.1.1 tag, the
/// version photocraft-heif pins (2026-09-12).
pub const HEIC_RS_COMMIT: &str = "4f0d4df474c773dfc3b14fa80e7219be6898866e";
/// https://github.com/bigcat88/pillow_heif (BSD-3-Clause): `tests/images` (master, 2026-09-25).
pub const PILLOW_HEIF_COMMIT: &str = "b16be1196dfa465a342d68894696e685ca3655cb";
/// https://github.com/AcademySoftwareFoundation/openexr (BSD-3-Clause): `src/test/bin/test_images`
/// at the v3.5.2 tag (2026-10-03).
pub const OPENEXR_COMMIT: &str = "69b2604fc76e370615438bdc8d2cd95b9349c12e";
/// PngSuite (public domain), a fixed release archive.
pub const PNGSUITE_URL: &str = "http://www.schaik.com/pngsuite/PngSuite-2017jul19.tgz";

/// `corpus/photoshop`: 258 PSDs authored with Photoshop by the photocraft-corpus generator.
pub const PHOTOSHOP: PinnedCorpus = PinnedCorpus {
    name: "photoshop",
    dest: "photoshop",
    upstreams: &[Upstream {
        prefix: "",
        repo: "storytold/photocraft-corpus",
        commit: PHOTOCRAFT_CORPUS_COMMIT,
        subdir: "photoshop",
        extras: &[("README.md", "README.md"), ("LICENSE-MIT", "LICENSE-MIT"), ("LICENSE-APACHE", "LICENSE-APACHE")],
    }],
    manifest: "xtask/photoshop-corpus.sha256",
    exts: &["psd", "psb"],
    subset: false,
    sources_md: |c| {
        format!(
            "# Photoshop oracle corpus\n\n`photoshop/` of https://github.com/storytold/photocraft-corpus at commit {PHOTOCRAFT_CORPUS_COMMIT} \
             (see `README.md`). MIT OR Apache-2.0, authored by the PhotoCraft contributors. Fetched and sha256-verified by \
             `cargo xtask corpus --photoshop` against `{}`. Gitignored; never commit these files.\n",
            c.manifest
        )
    },
};

/// `corpus/psd-tools`: the complete psd-tools test set.
pub const PSD_TOOLS: PinnedCorpus = PinnedCorpus {
    name: "psd-tools",
    dest: "psd-tools",
    upstreams: &[Upstream { prefix: "", repo: "psd-tools/psd-tools", commit: PSD_TOOLS_COMMIT, subdir: "tests/psd_files", extras: &[("LICENSE", "LICENSE")] }],
    manifest: "xtask/psd-tools-corpus.sha256",
    exts: &["psd", "psb"],
    subset: false,
    sources_md: |c| {
        format!(
            "# psd-tools test corpus\n\nThe PSD/PSB files of https://github.com/psd-tools/psd-tools/tree/{PSD_TOOLS_COMMIT}/tests/psd_files, \
             unmodified. MIT licence, Copyright (c) 2019 Kota Yamaguchi (see `LICENSE`). Fetched and sha256-verified by \
             `cargo xtask corpus --psd-tools` against `{}`. Gitignored; never commit these files.\n",
            c.manifest
        )
    },
};

/// `corpus/psd`: the hand-picked mix of small psd-tools and ag-psd files (170) that most PSD
/// tests use. The manifest selects the files.
pub const PSD_MIXED: PinnedCorpus = PinnedCorpus {
    name: "psd",
    dest: "psd",
    upstreams: &[
        Upstream {
            prefix: "psd-tools/",
            repo: "psd-tools/psd-tools",
            commit: PSD_TOOLS_COMMIT,
            subdir: "tests/psd_files",
            extras: &[("LICENSE", "psd-tools/LICENSE")],
        },
        Upstream { prefix: "ag-psd/", repo: "Agamnentzar/ag-psd", commit: AG_PSD_COMMIT, subdir: "test", extras: &[("LICENSE", "ag-psd/LICENSE")] },
    ],
    manifest: "xtask/psd-corpus.sha256",
    exts: &["psd", "psb"],
    subset: true,
    sources_md: |c| {
        format!(
            "# PSD test corpus (mixed)\n\nSmall PSD/PSB files selected from https://github.com/psd-tools/psd-tools/tree/{PSD_TOOLS_COMMIT}/tests/psd_files \
             (`psd-tools/`, MIT, Copyright (c) 2019 Kota Yamaguchi) and https://github.com/Agamnentzar/ag-psd/tree/{AG_PSD_COMMIT}/test \
             (`ag-psd/`, MIT, Copyright (c) 2016 Agamnentzar), unmodified; licences next to them. Selected by and sha256-verified \
             against `{}` by `cargo xtask corpus --psd`. Gitignored; never commit these files.\n",
            c.manifest
        )
    },
};

/// `corpus/heif`: a few small HEIC/HEIF files for the `heif` feature's tests. heic-rs's
/// checkerboards, RGB strips and a grid-tiled photo with EXIF and XMP (synthetic pixels encoded by
/// macOS `sips`, each `.ref.png` Apple's own decode), and pillow-heif's 10-bit RGBA file with the
/// 16-bit PNG it was encoded from. The manifest selects the files.
pub const HEIF: PinnedCorpus = PinnedCorpus {
    name: "heif",
    dest: "heif",
    upstreams: &[
        Upstream {
            prefix: "heic-rs/",
            repo: "tbraun96/heic-rs",
            commit: HEIC_RS_COMMIT,
            subdir: "tests/fixtures",
            extras: &[("LICENSE-MIT", "heic-rs/LICENSE-MIT"), ("LICENSE-APACHE", "heic-rs/LICENSE-APACHE")],
        },
        Upstream {
            prefix: "pillow-heif/",
            repo: "bigcat88/pillow_heif",
            commit: PILLOW_HEIF_COMMIT,
            subdir: "tests/images",
            extras: &[("LICENSE.txt", "pillow-heif/LICENSE.txt")],
        },
    ],
    manifest: "xtask/heif-corpus.sha256",
    exts: &["heic", "heif", "png"],
    subset: true,
    sources_md: |c| {
        format!(
            "# HEIF test corpus\n\nSmall files selected from https://github.com/tbraun96/heic-rs/tree/{HEIC_RS_COMMIT}/tests/fixtures \
             (`heic-rs/`, MIT OR Apache-2.0, Thomas Braun) and https://github.com/bigcat88/pillow_heif/tree/{PILLOW_HEIF_COMMIT}/tests/images \
             (`pillow-heif/`, BSD-3-Clause, Pillow-Heif contributors), unmodified; licences next to them. Selected by and sha256-verified \
             against `{}` by `cargo xtask corpus --heif`. Gitignored; never commit these files.\n",
            c.manifest
        )
    },
};

/// `corpus/exr`: the deep OpenEXR test images (scanline and tiled deep data, several channel
/// types) the deep decoder was verified against. The manifest selects the files.
pub const EXR: PinnedCorpus = PinnedCorpus {
    name: "exr",
    dest: "exr",
    upstreams: &[Upstream {
        prefix: "",
        repo: "AcademySoftwareFoundation/openexr",
        commit: OPENEXR_COMMIT,
        subdir: "src/test/bin/test_images",
        extras: &[("LICENSE.md", "LICENSE.md")],
    }],
    manifest: "xtask/exr-corpus.sha256",
    exts: &["exr"],
    subset: true,
    sources_md: |c| {
        format!(
            "# Deep OpenEXR test corpus

The `*.deep.exr` files of https://github.com/AcademySoftwareFoundation/openexr/tree/{OPENEXR_COMMIT}/src/test/bin/test_images,              unmodified. BSD-3-Clause, Copyright Contributors to the OpenEXR Project (see `LICENSE.md`). Selected by and sha256-verified              against `{}` by `cargo xtask corpus --exr`. Gitignored; never commit these files.
",
            c.manifest
        )
    },
};

/// Every pinned corpus, in fetch order.
pub const ALL: &[&PinnedCorpus] = &[&PSD_MIXED, &PSD_TOOLS, &PHOTOSHOP, &HEIF, &EXR];
