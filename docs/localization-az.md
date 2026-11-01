# Azerbaijani UI catalog

Azerbaijani (Latin script) is offered as **Azərbaycanca** (`az`). Regional preferences and
system locales such as `az-AZ`, `az_Latn_AZ.UTF-8` and `AZ-LATN-AZ` use the existing locale
matcher. English remains the source and fallback. The runtime registry has 28 languages
including English; this PR adds only Azerbaijani.

The catalog contains 4,566 entries, including 32 plural messages, 118 contextual entries and
eight command-ID overrides. It covers the existing catalogs' dynamic-label union and all
2,204 keys from the source coverage scan. Catalog completeness does not establish linguistic
quality or include untranslated engine errors.

## Terminology and provenance

| English | Azerbaijani |
|---|---|
| Layer | Qat |
| Layer mask | Qat maskası |
| Stroke (outline) | Kontur |
| Brush stroke | Fırça izi |
| Crop | Kəsmə |
| Rulers | Xətkeşlər |
| Path | Yol |
| Artboard | İş lövhəsi |
| Preset | Hazır ayar |
| Light (Camera Raw) | İşıq |
| Light (font weight) | İncə |

Google Translate supplied drafts of PhotoCraft's English labels. Core terminology, contexts
and plural messages were edited with OpenAI Codex. No proprietary localization resources were
used. The translations are MIT OR Apache-2.0, with attribution in `ATTRIBUTION.md` and
`crates/ui-egui/src/i18n/LICENSE-translations.txt`.

**Native-speaker review is pending**, particularly for long help text and specialist names.
The catalog preserves dotted and dotless I (`İ/i`, `I/ı`) verbatim. Tests cover exact lookup
and UI glyphs for `ƏəĞğIıİiÖöŞşÜüÇç`. Locale codes use ASCII case-insensitive matching; this
does not claim a general audit of locale-sensitive UI search or text case conversion.

## Plurals

The existing one/other integer rule selects `one` at 1 and `other` at all other `u64` counts.
Counted nouns remain singular in both forms: `1 qat`, `20 qat`. Tests include zero, teens,
hundreds, thousands and `u64::MAX`.

## Validation

The Azerbaijani-only revision was checked on 2026-11-01, using Rust 1.99.1/MSVC on Windows x64:

- All six Azerbaijani regression tests passed: locale tags, fallback, contexts, menu coverage,
  plurals, alphabet glyphs, live switching and actual save/load persistence across restart.
- All 38 `i18n::tests::` unit checks passed, including catalog consistency and the test comparing
  the packed catalogs with the source files.
- The native application and offscreen snapshot tool built with locked dependencies. Full
  workspace formatting, Clippy with `--all-targets -- -D warnings`, layering, wasm (24 crates
  plus optional HEIF) and catalog coverage passed. Azerbaijani covered 2,204/2,204 source keys.
- The editor and Interface Preferences were rendered offscreen at 1440 × 900 logical pixels,
  1.5× scale, using `--safe-gpu`, and both screenshots were visually inspected.

**Full-suite validation remains incomplete.** The full UI test run terminated both unit and
integration test processes with `0xc0000005` (`STATUS_ACCESS_VIOLATION`), also with one test
thread. The full i18n subset finished with 59 passed and eight existing CJK/Arabic missing-glyph
failures without the optional craft-fonts input; the six Azerbaijani tests passed. No full-suite
success is claimed. Native-speaker review remains pending. Offscreen CPU captures do not
establish hardware-GPU or native window/input acceptance.
