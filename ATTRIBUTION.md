# Attribution

Every non-code asset in this repository, with its author, source and license. PhotoCraft's own
code and original assets are MIT OR Apache-2.0 (see [`LICENSE-MIT`](LICENSE-MIT),
[`LICENSE-APACHE`](LICENSE-APACHE) and [`NOTICE`](NOTICE)). When you add an asset, add a row here
in the same change; third-party assets must be permissively licensed and keep their license file
next to them.

## Bundled in the app

| Path | Title | Author | Source | License |
|---|---|---|---|---|
| `crates/ui-egui/src/i18n/el.tsv` | Greek UI translations | PhotoCraft contributors | Original translations of PhotoCraft's English labels | MIT OR Apache-2.0, [`LICENSE-translations.txt`](crates/ui-egui/src/i18n/LICENSE-translations.txt) |
| `crates/ui-egui/src/i18n/de.tsv` | German UI translations | PhotoCraft contributors | Original translations of PhotoCraft's English labels | MIT OR Apache-2.0, [`LICENSE-translations.txt`](crates/ui-egui/src/i18n/LICENSE-translations.txt) |
| `crates/ui-egui/src/i18n/it.tsv` | Italian UI translations | PhotoCraft contributors | Original translations of PhotoCraft's English labels | MIT OR Apache-2.0, [`LICENSE-translations.txt`](crates/ui-egui/src/i18n/LICENSE-translations.txt) |
| `crates/ui-egui/src/i18n/ko.tsv` | Korean UI translations | PhotoCraft contributors | Original translations of PhotoCraft's English labels | MIT OR Apache-2.0, [`LICENSE-translations.txt`](crates/ui-egui/src/i18n/LICENSE-translations.txt) |
| `crates/ui-egui/src/i18n/pl.tsv` | Polish UI translations | PhotoCraft contributors | Original translations of PhotoCraft's English labels | MIT OR Apache-2.0, [`LICENSE-translations.txt`](crates/ui-egui/src/i18n/LICENSE-translations.txt) |
| `crates/ui-egui/src/i18n/pt-br.tsv` | Brazilian Portuguese UI translations | PhotoCraft contributors | Original translations of PhotoCraft's English labels | MIT OR Apache-2.0, [`LICENSE-translations.txt`](crates/ui-egui/src/i18n/LICENSE-translations.txt) |
| `assets/fonts/Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf` | Inter 4.001 (UI font) | The Inter Project Authors (Rasmus Andersson) | <https://github.com/rsms/inter> | SIL OFL 1.1, [`assets/fonts/OFL-Inter.txt`](assets/fonts/OFL-Inter.txt) |
| `assets/fonts/JetBrainsMono-Regular.ttf` | JetBrains Mono 2.305 (numeric font) | The JetBrains Mono Project Authors | <https://github.com/JetBrains/JetBrainsMono> | SIL OFL 1.1, [`assets/fonts/OFL-JetBrainsMono.txt`](assets/fonts/OFL-JetBrainsMono.txt) |
| `assets/icons/*.svg` except `slice-knife.svg`, `clip-below.svg`, `clip-release.svg`, `eraser-background.svg`, `eraser-magic.svg`, `lasso-magnetic.svg` and `direct-select.svg` (109 files) | Lucide icons | Lucide Icons and Contributors | <https://github.com/lucide-icons/lucide> (`icons/<name>.svg`; `align-*` are now named `text-align-*` upstream) | ISC, [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt) |
| `assets/icons/{check,chevron-down,chevron-right,chevron-up,chevrons-left,chevrons-right,circle,clock,compass,info,link,lock,minus,moon,move,navigation,plus,search,square,trash,triangle,type,x,zoom-in}.svg` | Lucide icons derived from Feather (subset of the row above) | Cole Bemis (Feather), Lucide Contributors | <https://github.com/feathericons/feather> via Lucide | MIT (Feather) and ISC (Lucide), [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt) |
| `assets/icons/slice-knife.svg` | Slice tool glyph | PhotoCraft contributors | Original work | MIT OR Apache-2.0 |
| `assets/icons/clip-below.svg`, `assets/icons/clip-release.svg` | Clipping-mask cursors (⌥ over the line between two layers): a hooked arrow onto the layer below; the same with a small "no" badge to release | PhotoCraft contributors | Original work | MIT OR Apache-2.0 |
| `assets/icons/eraser-background.svg`, `assets/icons/eraser-magic.svg` | Background Eraser and Magic Eraser tool glyphs: Lucide `eraser` plus added marks | Lucide Icons and Contributors; marks by PhotoCraft contributors | Derived from <https://github.com/lucide-icons/lucide> `icons/eraser.svg` | ISC, [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt) |
| `assets/icons/direct-select.svg` | Direct Selection tool glyph: Lucide `mouse-pointer-2` plus an anchor square | Lucide Icons and Contributors; square by PhotoCraft contributors | Derived from <https://github.com/lucide-icons/lucide> `icons/mouse-pointer-2.svg` | ISC, [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt) |
| `assets/icons/lasso-magnetic.svg` | Magnetic Lasso tool glyph, adapted from "Magnetic Lasso Tool" (redrawn on the 24 px toolbar grid) | Lil' Seal (Noun Project); adapted by PhotoCraft contributors | <https://thenounproject.com/icon/magnetic-lasso-tool-177513/> | CC BY 3.0, [`assets/icons/LICENSE-noun-magnetic-lasso.txt`](assets/icons/LICENSE-noun-magnetic-lasso.txt) |
| `assets/dict/en_US-scowl-50.txt.gz` | English word list for Check Spelling (SCOWL 2020.12.07, size 50, en_US; 72,403 words) | Kevin Atkinson and the SCOWL contributors | <http://wordlist.aspell.net/> | SCOWL license (permissive, keep the notice), [`assets/dict/LICENSE-SCOWL.txt`](assets/dict/LICENSE-SCOWL.txt), which also has the build recipe |
| `crates/cms/profiles/photocraft-coated-cmyk.icc` | PhotoCraft Coated CMYK (synthetic) ICC profile | PhotoCraft contributors (generated by `photocraft-cms`, `src/synth.rs`) | Original work, see [`crates/cms/README.md`](crates/cms/README.md) | CC0-1.0 |
| `assets/app-icon/` (all files) | PhotoCraft app icon (nine-tailed kitsune) | The project owner (drawn in ArtCraft, vectorised) | Original work, see [`assets/app-icon/README.md`](assets/app-icon/README.md) | MIT OR Apache-2.0, [`assets/app-icon/LICENSE.txt`](assets/app-icon/LICENSE.txt) |
| `packaging/macos/dmg/` (all files) | PhotoCraft DMG window background and layout | @XusBadia, from the PhotoCraft app icon | Original work, see [`packaging/macos/dmg/README.md`](packaging/macos/dmg/README.md) | MIT OR Apache-2.0, like the code |

### Optional build input: craft-fonts (not files in this repo)

Builds made with `CRAFT_FONTS_DIR` (all official releases) embed fonts from
[storytold/craft-fonts](https://github.com/storytold/craft-fonts) at a pinned commit; no font file
from it is committed here. Per-file authors, sources and licences:
[craft-fonts `ATTRIBUTION.md`](https://github.com/storytold/craft-fonts/blob/main/ATTRIBUTION.md).

| Font (embedded at build time) | Use | License |
|---|---|---|
| BIZ UDPGothic Regular, Bold (Morisawa) | Japanese UI and sans Type fallback (desktop) | SIL OFL 1.1, shipped as `OFL-biz-ud-pgothic.txt` |
| Shippori Mincho Regular (FONTDASU) | Japanese serif Type fallback (desktop) | SIL OFL 1.1, shipped as `OFL-shippori-mincho.txt` |
| BIZ UDMincho Regular (Morisawa) | Japanese serif Type fallback (desktop) | SIL OFL 1.1, shipped as `OFL-biz-ud-mincho.txt` |

The other built-in ICC profiles and the generated LUT looks are produced by code at runtime
(`crates/cms/src/builtin.rs`, `crates/cms/src/lutfile.rs`) and dedicated to the public domain
(CC0-1.0); they are not separate files.

## Documentation and README

| Path | Title | Author | Source | License |
|---|---|---|---|---|
| `docs/brand/` (all files) | ArtCraft name, wordmark and mark | ArtCraft Team | getartcraft.com | Not open source; trademarks of the ArtCraft Team, [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt) |
| `docs/images/photocraft-*.jpg` | PhotoCraft screenshots | PhotoCraft contributors (UI) | Rendered offscreen with the `snapshot` example | MIT OR Apache-2.0 (UI); the artwork in each is public domain, listed below |
| `docs/images/preferences-apply-*.png` | Preferences before and after adding Apply (no artwork) | PhotoCraft contributors | PhotoCraft control-channel capture and offscreen `snapshot` example | MIT OR Apache-2.0 |

Artwork shown in the screenshots (all public domain, via Wikimedia Commons; details in
[`docs/images/SOURCES.md`](docs/images/SOURCES.md)):

| Screenshot | Artwork | Source |
|---|---|---|
| `photocraft-demo.jpg` | *The Great Wave off Kanagawa*, Katsushika Hokusai, c. 1831 | [Commons](https://commons.wikimedia.org/wiki/File:Tsunami_by_hokusai_19th_century.jpg) |
| `photocraft-layer-styles.jpg` | *Earthrise*, William Anders, Apollo 8, 1968 (NASA) | [Commons](https://commons.wikimedia.org/wiki/File:NASA-Apollo8-Dec24-Earthrise.jpg) |
| `photocraft-masks.jpg` | *Girl with a Pearl Earring*, Johannes Vermeer, c. 1665 | [Commons](https://commons.wikimedia.org/wiki/File:1665_Girl_with_a_Pearl_Earring.jpg) |
| `photocraft-vector.jpg` | *Water Lilies*, Claude Monet, 1906 | [Commons](https://commons.wikimedia.org/wiki/File:Claude_Monet_-_Water_Lilies_-_1906,_Ryerson.jpg) |
| `photocraft-filters.jpg` | *The Starry Night*, Vincent van Gogh, 1889 | [Commons](https://commons.wikimedia.org/wiki/File:Van_Gogh_-_Starry_Night_-_Google_Art_Project.jpg) |
| `photocraft-adjustments.jpg` | *Impression, Sunrise*, Claude Monet, 1872 | [Commons](https://commons.wikimedia.org/wiki/File:Monet_-_Impression,_Sunrise.jpg) |
| `photocraft-transform.jpg` | *The Tetons and the Snake River*, Ansel Adams, 1942 (U.S. National Archives) | [Commons](https://commons.wikimedia.org/wiki/File:Adams_The_Tetons_and_the_Snake_River.jpg) |
| `photocraft-type.jpg` | *Among the Sierra Nevada, California*, Albert Bierstadt, 1868 | [Commons](https://commons.wikimedia.org/wiki/File:Albert_Bierstadt_-_Among_the_Sierra_Nevada,_California_-_Google_Art_Project.jpg) |
| `photocraft-export-light.jpg` | *The Kiss*, Gustav Klimt, 1907–1908 | [Commons](https://commons.wikimedia.org/wiki/File:Gustav_Klimt_016.jpg) |

## Test data (not committed, not shipped)

`corpus/` is gitignored and no test fixtures are committed to the repository.
`cargo xtask corpus --all` fetches every corpus at the pinned commits in `xtask/src/corpus_pins.rs`
and verifies each file against the sha256 lists in `xtask/*.sha256`, with the upstream licence
next to the files:

| Path (fetched) | Title | Author | Source | License |
|---|---|---|---|---|
| `corpus/photoshop/` (256 PSDs) | Photoshop oracle corpus: smart filters, layer-style effects, type, adjustments in every mode and depth | PhotoCraft contributors (authored with Adobe Photoshop 2026 by a script) | [https://github.com/storytold/photocraft-corpus](https://github.com/storytold/photocraft-corpus) (`photoshop/`, with its generator and README) | MIT OR Apache-2.0 |
| `corpus/psd-tools/` (309 files) | psd-tools test set | Kota Yamaguchi and contributors | [psd-tools `tests/psd_files`](https://github.com/psd-tools/psd-tools/tree/main/tests/psd_files) | MIT, Copyright (c) 2019 Kota Yamaguchi |
| `corpus/psd/` (170 files) | Small selection of the psd-tools and ag-psd test files | Kota Yamaguchi; Agamnentzar | psd-tools (above) and [ag-psd `test/`](https://github.com/Agamnentzar/ag-psd/tree/master/test) | MIT (both) |
| `corpus/heif/` (9 files, 0.1 MB) | HEIC/HEIF test files: `heic-rs/` checkerboards, RGB strips and a grid-tiled photo with EXIF and XMP (synthetic pixels encoded by macOS `sips`, each `.ref.png` Apple's decode); `pillow-heif/` the 10-bit RGBA `RGBA_10__29x100.heif` and its source `RGBA_16__29x100.png` | Thomas Braun (heic-rs); Pillow-Heif contributors | [heic-rs `tests/fixtures`](https://github.com/tbraun96/heic-rs/tree/main/tests/fixtures), [pillow-heif `tests/images`](https://github.com/bigcat88/pillow_heif/tree/master/tests/images) | MIT OR Apache-2.0 (heic-rs); BSD-3-Clause (pillow-heif) |
| `corpus/exr/` (5 files, 2.3 MB) | The deep OpenEXR test images `11`, `42`, `64`, `multivariate` and `objectid.deep.exr` | Contributors to the OpenEXR Project | [openexr `src/test/bin/test_images`](https://github.com/AcademySoftwareFoundation/openexr/tree/main/src/test/bin/test_images) | BSD-3-Clause |
| `corpus/affinity/` (21 files, 2.4 MB) | Public Affinity documents, unmodified: `vector-art/` (9), `afdesignload/` (9), `jac21/` (2) and `asset-store-template/` (1), each with its upstream licence next to it | samuel-etver; Nick Beeuwsaert; Jeremy Cantu; DAIKI (eviltwo) | [vector-art `simple/`](https://github.com/samuel-etver/vector-art/tree/255f8add3c8f0740196e22bd59502b811b532f0b/simple), [AFDesignLoad `testDesigns/`](https://github.com/NickBeeuwsaert/AFDesignLoad/tree/a18dd50a7079fb861a28835eeefa0d015f03f4a1/testDesigns), [Jac21/Branding `Logos/JC/DesignerFiles/Affinity`](https://github.com/Jac21/Branding/tree/57ae3f45bf4637f6927c0b07de25b922c55f944f/Logos/JC/DesignerFiles/Affinity), [AssetStoreTemplate `AssetStoreTemplate/`](https://github.com/eviltwo/AssetStoreTemplate/tree/f671981ee89d15526285b3cc87b4393d3defe644/AssetStoreTemplate) | CC0-1.0 (vector-art); MIT, Copyright (c) 2015 Nick Beeuwsaert; MIT, Copyright (c) 2018 Jeremy Cantu; MIT, Copyright (c) 2024 DAIKI |
| `corpus/pngsuite/` | PngSuite | Willem van Schaik | <http://www.schaik.com/pngsuite/> | Public domain |

Files copied into `corpus/` by hand (tiff, exr, raw) must be MIT, BSD or CC0.
