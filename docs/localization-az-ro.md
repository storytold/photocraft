# Azerbaijani and Romanian UI catalogs

Added 2026-10-31. The language picker offers **Azərbaycanca** (`az`, Latin script) and
**Română** (`ro`). The source and fallback remain English; command IDs, automation protocol
values and user-created names are unchanged. Regional preferences and system locales such as
`az-AZ`, `az_Latn_AZ.UTF-8`, `ro-RO` and `ro-MD` resolve through the existing locale matcher.
Romanian from Moldova uses the same catalog as Romanian from Romania.

Each catalog contains 4,566 entries: the existing catalogs' key union plus the current
source scan's missing font-search label. This includes 32 plural messages, command-ID entries
and contextual strings for Camera Raw, font weights, camera metadata and the Layers panel.
Catalog entry counts do not measure translation quality or cover untranslated engine errors.

## Terminology and provenance

| English | Azerbaijani | Romanian |
|---|---|---|
| Layer | Qat | Strat |
| Layer mask | Qat maskası | Mască de strat |
| Stroke (shape outline) | Kontur | Contur |
| Brush stroke | Fırça izi | Tușă |
| Crop | Kəsmə | Decupare |
| Rulers | Xətkeşlər | Rigle |
| Path | Yol | Traseu |
| Artboard | İş lövhəsi | Planșă de desen |
| Preset | Hazır ayar | Presetare |
| Light (Camera Raw) | İşıq | Lumină |
| Light (font weight) | İncə | Subțire |

The English source labels were translated with Google Translate to produce drafts. Core menu
and tool terms, contextual meanings and all plural messages were edited with OpenAI Codex.
The catalogs were not copied from Photoshop or another product's localization resources.
Product names, color profile names and technical identifiers remain unchanged where appropriate.
Both catalogs carry the project's MIT OR Apache-2.0 license; attribution is in `ATTRIBUTION.md`
and `crates/ui-egui/src/i18n/LICENSE-translations.txt`.

**Native-speaker review is pending.** In particular, long help text and specialist filter names
need linguistic review. Complete menu coverage means that the catalog has the lookup keys;
it does not certify every translation as idiomatic.

## Plurals

The rules follow [Unicode CLDR cardinal plurals](https://github.com/unicode-org/cldr/blob/main/common/supplemental/plurals.xml)
for integer counts (the UI plural API takes `u64`). Azerbaijani selects `one` at 1 and `other`
at every other count; counted nouns remain singular in both forms, such as `1 qat` and `20 qat`.

Romanian has three forms:

- `one`: exactly 1 (`1 strat`).
- `few`: 0 or a remainder of 1–19 modulo 100, except exactly 1 (`0 straturi`, `2 straturi`, `101 straturi`).
- `other`: all other integers, with **de** before the noun (`20 de straturi`, `100 de straturi`).

The implementation uses bounded integer arithmetic and handles `u64::MAX` without overflow.

## Validation

Regression tests cover regional and mixed-case locale tags, native language names, LTR direction,
fallback behavior, contextual terms, placeholders, menu coverage and plural counts around 0, 1,
19/20, 100/101, 119/120 and `u64::MAX`. Shell tests cover language switching, English restoration,
persisted preferences, restart and first-launch Auto detection for the new languages.

Local verification on 2026-10-31 used Rust 1.99.1 and MSVC on Windows x64:

- The native `photocraft` binary and `snapshot` example built successfully with the locked
  dependencies. These were unoptimized debug builds, not performance measurements.
- All six Azerbaijani/Romanian unit and shell tests passed, including saving through
  `Services.save_prefs`, loading after restart, preference labels and UI glyph coverage.
- `cargo fmt --all -- --check`, `cargo clippy -p photocraft-ui-egui --all-targets -- -D warnings`,
  `cargo xtask layers` and `cargo xtask wasm` passed. The wasm check covered 24 crates and
  the optional HEIF feature.
- `cargo xtask i18n-coverage` reported 2,204 / 2,204 source keys for each new language.
  Static TSV, duplicate, placeholder and ellipsis checks passed; the Rust test comparing
  packed catalogs with their source files also passed.
- Editor and Interface Preferences screenshots for both languages were rendered offscreen
  at 1440 × 900 logical pixels and 1.5× scale, using `--safe-gpu`, and visually inspected.
  This caught incorrect translations of Kind, Type Layer, Light Theme and preference labels,
  which were corrected and covered by regressions. These captures verify the CPU canvas
  path, not hardware-GPU rendering or native window/input behavior.

**The full test gate is not green.** The i18n subset finished with 59 passed and eight failed:
the existing Chinese/Japanese/Korean/Arabic shell checks reported missing glyphs in this
build without the optional `CRAFT_FONTS_DIR` input. All six new-language tests passed.
The full `cargo test -p photocraft-ui-egui --no-fail-fast` run terminated both its unit and
integration test processes with `0xc0000005` (`STATUS_ACCESS_VIOLATION`) during graphics tests;
it did not complete. These broader checks need a successful rerun before claiming full
validation. Native-speaker review also remains pending.
