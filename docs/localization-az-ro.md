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

Static catalog checks inspect duplicate keys, TSV structure, placeholders in every plural form,
trailing ellipses, the complete menu tree and source-derived localization keys. Rust tests,
Clippy, wasm checks and offscreen app rendering require a Rust-equipped environment; the
implementation environment had neither `cargo` nor `rustc`, so those checks were not run here.

Before merging, run `cargo test -p photocraft-ui-egui`,
`cargo clippy -p photocraft-ui-egui --all-targets -- -D warnings`, `cargo xtask layers`,
`cargo xtask wasm`, `cargo xtask i18n-coverage` and the offscreen snapshot example.
