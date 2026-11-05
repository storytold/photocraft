# Azerbaijani UI catalog

Azerbaijani (Latin script) is offered as **Azərbaycanca** (`az`). Regional preferences and
system locales such as `az-AZ`, `az_Latn_AZ.UTF-8` and `AZ-LATN-AZ` use the existing locale
matcher. English remains the source and fallback. The runtime registry has 29 languages
including English; this PR adds only Azerbaijani.

The catalog contains 4,600 entries, including 32 plural messages, 118 contextual entries and
eight command-ID overrides. The 2026-11-05 merge added the 29 new OpenType menu keys from
current `main`. The follow-up also adds five frame conversion/tool labels; all menu labels and the eight intentional command-ID overrides are checked.
The 2026-11-01 source scan covered 2,204/2,204 keys. Catalog completeness does not establish linguistic
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

Validation on 2026-11-05 used Rust 1.99.1/MSVC on Windows x64. The checks below were run
on the first merge (`25ed8e5cb`); the subsequent merge of `405b56c3c` and five frame labels
is separately rechecked below:

- All six Azerbaijani regressions passed, including actual preference persistence, the full
  menu tree and all eight intentional command-ID overrides.
- All 40 `i18n::tests::` unit checks passed, including the preserved Bosnian tests and
  packed-catalog/source consistency. These overlap with four Azerbaijani regressions.
- Workspace formatting, Clippy (`--all-targets -- -D warnings`), layering (31 crates),
  wasm (24 crates plus optional HEIF) and the offscreen snapshot build passed.
- Latest source coverage is 2,254/2,331 keys (96.70%); 77 newer strings fall back to English.
  The 666 menu-catalog keys and eight command-ID overrides are present.
- Editor and Interface Preferences were rendered at 1440 × 900 logical pixels, 1.5× scale,
  using `--safe-gpu`, and both screenshots were visually inspected.

After merging `405b56c3c`, all six Azerbaijani regressions and all 40 i18n unit checks
were rerun and passed, including the five frame-label assertions and packed catalogs.
Formatting, the 31-crate layering check, Clippy (`--all-targets -- -D warnings`)
and wasm (24 crates plus optional HEIF) also passed again. The full suite and CPU
snapshots were not rerun for this follow-up; their earlier results above still apply.

**The full test gate is not green.** The complete i18n subset finished with 61 passed and
eight CJK/Arabic missing-glyph failures without the optional craft-fonts input. All Azerbaijani
and Bosnian checks passed. The full UI unit and integration processes again terminated with
`0xc0000005` (`STATUS_ACCESS_VIOLATION`), with one test thread; full-suite success is not established.
Native-speaker review remains pending. CPU captures do not establish hardware-GPU or native
window/input acceptance.
