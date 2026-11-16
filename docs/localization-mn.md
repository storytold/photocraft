# Mongolian Cyrillic interface

PhotoCraft registers Mongolian as `mn`; the picker reads **Монгол** from the shared catalog's
`@id language.name` row. Tags `mn-MN`, `mn-Cyrl-MN` and `mn_MN.UTF-8` resolve through the
existing locale matcher. This catalog uses Cyrillic. Traditional vertical Mongolian script is
not implemented; other `mn-*` tags currently fall back to Cyrillic.

## Catalog dependency

Translations belong in [storytold/craft-i18n](https://github.com/storytold/craft-i18n):
`locales/mn/common.tsv` and `locales/mn/photocraft.tsv`. They are a separate contribution;
`craft-i18n.rev` must be updated to a merged catalog commit before this language ships.
Until then, the PhotoCraft change is a draft. The registry uses `complete_menus: false`;
new or missing source strings fall back to English.

As with other languages, `CRAFT_I18N_DIR` embeds the catalogs at build time. A build without
catalogs remains English-only. Local validation uses the proposed shared catalogs; the
unchanged release pin does not contain Mongolian yet. No catalog TSV files are restored in
this repository.

## Terminology and review

| English | Mongolian |
|---|---|
| Layer | Давхарга |
| Brush | Багс |
| Selection | Сонголт |
| Layer mask | Давхаргын маск |
| Stroke (outline) | Хүрээ / хүрээлэх |
| Path | Зам |
| Artboard | Ажлын самбар |
| Preset | Бэлэн тохиргоо |
| Pattern | Хээ |
| Opacity | Тунгалаг бус байдал |
| Hue / saturation | Өнгөний туяа / ханалт |

CLDR provides `one` and `other`. Both catalog forms use the same counted noun, for example
`1 давхарга` and `3 давхарга`; the shared runtime owns the plural rules. The English source
keys, command IDs, placeholders, escapes and trailing dialog ellipses retain their meaning.

The catalog was prepared from public PhotoCraft English labels, using Google Translate for
drafts and edits to core terminology and contextual/count messages. No proprietary
localization resources were consulted or copied. The shared repository records provenance
and the MIT OR Apache-2.0 license. Native-speaker and image-editor terminology review remains
pending, especially for long help strings and rare filters. Key coverage is not a measure of
linguistic quality. Engine errors and parameter help can still appear in English.

## Verification

The tests cover registration, regional and system locale matching, counted nouns (including
extreme counts), fallback, live switching, persistence/restart, default-font Cyrillic glyphs
(including Өө and Үү), and Camera Raw controls and retained state. The Camera Raw test reuses
the existing all-language scenario. Catalog-dependent checks skip when Mongolian is absent;
the registry assertion and fixture plural test run even in English-only builds.

Test the proposed catalog checkout with `CRAFT_I18N_DIR` set to its absolute path, then run
`cargo xtask i18n-report`. Validate the catalogs with the shared repository's own checker.
The source report is informational: missing entries always show in English.

On 2026-11-11, the proposed catalogs resolved 2,521/2,521 measured source keys. The three
Mongolian unit/live tests and the focused RAW Develop integration test passed with those
catalogs embedded. Strict UI clippy, wasm (23 crates plus optional HEIF), formatting, layering
and panel-command checks passed. English/Mongolian offscreen editor renders were visually
inspected. The shared catalog checker, strict clippy, formatting and all 41 Python 3.14 tool
tests passed in the catalog checkout. These translated results use the proposed catalogs,
not the unchanged release pin; they do not remove the merge/pin dependency.

The complete translated UI library run is not green: 6,984 passed, 25 failed and 24 ignored.
Failures include missing CJK/Arabic glyphs, font/focus assertions and a native-menu structure
assertion. These are not claimed to be a clean baseline. The obsolete fill-rasterization
source key discovered during migration was removed and its replacement translated; the
specific regression passes. The app change remains a draft until the catalog/pin dependency
and CI are resolved.
