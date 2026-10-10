# Localization parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** major (first version: per-language status measured from the catalogs, against Photoshop 2026's language packs) · **Target:** Adobe Photoshop 2026

Per-language status of PhotoCraft's UI. How catalogs work and how to add one:
[`localization.md`](localization.md). Language notes: [Korean](localization-ko.md),
[Simplified Chinese](localization-zh-hans.md).

**How measured.** `cargo xtask i18n-coverage` (`xtask/src/i18n_coverage.rs`) defines the key
universe: `tl!("…")` literals and `trn(` plurals in `ui-egui` (tests excluded), menu-catalog
labels and path segments, `menus.rs` UI commands, and engine `CommandSpec` labels and menu paths:
**1,608 English keys** on 2026-10-10. Each catalog (`crates/ui-egui/src/i18n/<code>.tsv`) was
checked against that universe by a source script replicating the xtask (no build). Catalogs hold
more rows than the universe because they also translate dynamic `tl!(expr)` strings (~159 call
sites the tool can't enumerate). **Not covered anywhere:** engine error and status messages and
filter parameter help, which are English in every language; so no language is `full`.

**Photoshop 2026** ships UI language packs for cs, da, de, en, es, fi, fr (and fr-CA), hu, it,
ja, ko, nb, nl, pl, pt-BR, ru, sk, sv, tr, uk, zh-Hans, zh-Hant (24 `.lproj` markers; one pack is
installed per Creative Cloud install, here `en_US`). It has no Hindi, Arabic, Indonesian or
Vietnamese UI; Arabic and Hebrew users get English "Middle East" builds with the World-Ready
composer, and its text engine handles RTL, Devanagari and Indic scripts.

**Summary.** 7 of the 12 key languages at 100% of measured keys (status `partial` because of the
engine messages), 3 absent (Hindi, Arabic, Vietnamese), plus 8 other languages. Weighted by
speakers, ~55% ready. Remaining: 80–140 h, plus native-speaker review (none recorded for any
catalog).

## The twelve key languages

| Language | Code | UI strings (measured keys) | Catalog rows | Dialogs / tooltips / help | Script support | Native review | Status | To `full` |
|---|---|---|---:|---|---|---|---|---:|
| English | `en` | 1,608 / 1,608 (100%) | source | all | Latin | – | full | 0 h |
| Simplified Chinese | `zh-hans` | 1,608 / 1,608 (100%) | 2,366 | dialogs, tooltips yes; engine messages English | CJK fonts (lazy loader), IME preedit/commit, vertical type | no | partial | 8–12 h |
| Spanish | `es` | 1,608 / 1,608 (100%) | 2,413 | as above | Latin | no | partial | 6–10 h |
| Hindi | `hi` | 0 (0%) | – | – | Devanagari shaping in the text engine (parley/HarfRust); no Devanagari fallback font (TYPE-218-3) | no | none | 20–30 h |
| Arabic | `ar` | 0 (0%) | – | – | Arabic shaping and bidi in the text engine, Arabic fallback font; no RTL UI layout (DIST-220-4), paragraph direction has no UI (TYPE-218-2), layer names as tofu (#1408, #1693) | no | none | 40–60 h |
| French | `fr` | 1,608 / 1,608 (100%) | 2,766 | as above | Latin | no | partial | 6–10 h |
| Portuguese (Brazil) | `pt-br` | 1,608 / 1,608 (100%) | 2,788 | as above | Latin; every `pt` locale uses it | no | partial | 6–10 h |
| Indonesian | `id` | 1,608 / 1,608 (100%) | 2,764 | as above | Latin | no | partial | 6–10 h |
| Japanese | `ja` | 1,608 / 1,608 (100%) | 3,356 | as above | CJK fonts, IME (Windows IME broken, #590), vertical type (tategaki); installed Japanese fonts missing from the list (#2330) | no | partial | 10–16 h |
| German | `de` | 1,608 / 1,608 (100%) | 2,400 | as above | Latin | no | partial | 6–10 h |
| Korean | `ko` | 1,608 / 1,608 (100%) | 2,936 | as above; terminology from Photoshop's Korean UI ([notes](localization-ko.md)) | Hangul fonts, IME | no | partial | 6–10 h |
| Vietnamese | `vi` | 0 (0%) | – | – | Latin with stacked diacritics (shaping supported) | no | none | 12–18 h |

## Other shipped languages

8 more, each at 1,608 / 1,608 measured keys, status `partial` for the same reason: Traditional
Chinese `zh-hant` (2,329 rows), Russian `ru` (2,544), Ukrainian `uk` (3,375), Czech `cs` (2,337),
Polish `pl` (3,366), Greek `el` (3,481), Dutch `nl` (3,568), Italian `it` (2,411).

## Cross-language gaps

- Engine error, status and parameter-help messages are English in every language.
- No right-to-left UI layout (needed for Arabic, Hebrew, Persian, Urdu).
- Web build: no browser-locale detection, no CJK web fonts.
- WebAssembly plug-ins disappear from Filter › Plug-ins when the UI isn't English (#2540).
- A user couldn't find the language option (#2532): it is in Preferences › Interface.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | major | First version, measured from the catalogs with the `i18n-coverage` rules |
