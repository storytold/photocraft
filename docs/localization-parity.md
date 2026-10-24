# Localization parity

> **Last reviewed:** 2026-10-22 · **Last updated:** 2026-10-22 · **Change:** minor (coverage re-measured, 1,982 keys; Hindi partial; Arabic paragraph direction buttons, #6793; previously minor: Russian dynamic labels) · **Target:** Adobe Photoshop 2026

Per-language status of PhotoCraft's UI. How catalogs work and how to add one:
[`localization.md`](localization.md). Language notes: [Korean](localization-ko.md), [Spanish](localization-es.md),
[Simplified Chinese](localization-zh-hans.md), [Italian](localization-it.md), [Russian](localization-ru.md).

**How measured.** `cargo xtask i18n-coverage` (`xtask/src/i18n_coverage.rs`) defines the key
universe: `tl!("…")` literals and `trn(` plurals in `ui-egui` (tests excluded), menu-catalog
labels and path segments, `menus.rs` UI commands, and engine `CommandSpec` labels and menu paths:
**1,982 English keys** on 2026-10-22 (1,918 on 2026-10-21, 1,772 on 2026-10-12, 1,608 on 2026-10-10). Each catalog (`crates/ui-egui/src/i18n/<code>.tsv`)
is checked against that universe by the xtask itself; "rows" are the non-comment, non-blank lines
of the `.tsv`. Catalogs hold more rows than the universe because they also translate dynamic `tl!(expr)` strings (~159 call
sites the tool can't enumerate). **Not covered anywhere:** engine error and status messages and
filter parameter help, which are English in every language; so no language is `full`. The report
also lists, per language, the keys its catalog lacks; those show in English. `cargo xtask ci` prints
it as its last step and never fails on it.

**Photoshop 2026** ships UI language packs for cs, da, de, en, es, fi, fr (and fr-CA), hu, it,
ja, ko, nb, nl, pl, pt-BR, ru, sk, sv, tr, uk, zh-Hans, zh-Hant (24 `.lproj` markers; one pack is
installed per Creative Cloud install, here `en_US`). It has no Hindi, Arabic, Indonesian or
Vietnamese UI; Arabic and Hebrew users get English "Middle East" builds with the World-Ready
composer, and its text engine handles RTL, Devanagari and Indic scripts.

**Summary.** 10 of the 12 key languages at 100% of measured keys (English, Simplified Chinese,
Spanish, French, Portuguese (Brazil), Indonesian, Japanese, German, Korean, Vietnamese; status
`partial` because of the engine messages), Hindi partial (267 of 1,982, 13.5%), Arabic absent, plus
the other shipped languages below. Weighted by
speakers, ~55% ready. Remaining: 80–140 h, plus native-speaker review (recorded only for Italian,
2026-10-12).

## The twelve key languages

| Language | Code | UI strings (measured keys) | Catalog rows | Dialogs / tooltips / help | Script support | Native review | Status | To `full` |
|---|---|---|---:|---|---|---|---|---:|
| English | `en` | 1,982 / 1,982 (100%) | source | all | Latin | – | full | 0 h |
| Simplified Chinese | `zh-hans` | 1,982 / 1,982 (100%) | 2,934 | dialogs, tooltips yes; engine messages English | CJK fonts (lazy loader), IME preedit/commit, vertical type | no | partial | 8–12 h |
| Spanish | `es` | 1,982 / 1,982 (100%) | 4,219 | as above; complete dynamic UI key set | Latin | no | partial | 6–10 h |
| Hindi | `hi` | 267 / 1,982 (13.5%) | 491 | Top menus, toolbox tools, Camera Raw, dialogs, plurals | Devanagari shaping in the text engine (parley/HarfRust); no Devanagari fallback font (TYPE-218-3) | no | partial | 15–20 h |
| Arabic | `ar` | 0 (0%) | – | – | Arabic shaping and bidi in the text engine, Arabic fallback font; RTL shell layout but not inside panels, and UI labels in LTR word order (DIST-220-4), paragraph direction buttons but no visual RTL caret (TYPE-218-2 partial), layer names as tofu (#1408, #1693) | no | none | 40–60 h |
| French | `fr` | 1,982 / 1,982 (100%) | 4,308 | as above | Latin | no | partial | 6–10 h |
| Portuguese (Brazil) | `pt-br` | 1,982 / 1,982 (100%) | 4,201 | as above | Latin; every `pt` locale uses it | no | partial | 6–10 h |
| Indonesian | `id` | 1,982 / 1,982 (100%) | 3,289 | as above | Latin | no | partial | 6–10 h |
| Japanese | `ja` | 1,982 / 1,982 (100%) | 3,922 | as above | CJK fonts, IME (Windows IME broken, #590), vertical type (tategaki) | no | partial | 10–16 h |
| German | `de` | 1,982 / 1,982 (100%) | 3,281 | as above | Latin | no | partial | 6–10 h |
| Korean | `ko` | 1,982 / 1,982 (100%) | 3,484 | as above; terminology from Photoshop's Korean UI ([notes](localization-ko.md)) | Hangul fonts, IME | no | partial | 6–10 h |
| Vietnamese | `vi` | 1,982 / 1,982 (100%) | 3,721 | as above | Latin with stacked diacritics (shaping supported) | no | partial | 6–10 h |

## Other shipped languages

11 more, measured 2026-10-22, status `partial` for the same reason. All cover every measured key:
Traditional Chinese `zh-hant` (2,893 rows), Russian `ru` (4,370; also every dynamic label the
other catalogs translate, [notes](localization-ru.md)), Ukrainian `uk` (3,894), Czech `cs` (3,985),
Polish `pl` (3,880), Greek `el` (3,990), Dutch `nl` (4,068), Hungarian `hu` (4,219), Italian `it` (4,370;
also every dynamic label the other catalogs translate; reviewed by a native speaker,
[notes](localization-it.md)), Turkish `tr` (4,063 rows). Serbian `sr` (4,307 rows, Latin script,
Ekavian, added 2026-10-12) covers every
measured key and the union of the other catalogs' dynamic labels; its one/few/many plurals reuse
the Russian rule.

Czech UI follow-up (2026-10-11): status and notice wrappers, hardware labels, brush and
transform accessibility names, rasterization prompts, profile mismatch messages and preset
counts now translate at their render/assignment sites. Navigator's zoom level uses a context
(`Zvětšení`), while the tool remains `Lupa`. Engine errors, profile/file/document names stay
verbatim; copied System Info and control API diagnostics remain English while their on-screen
labels translate. Import warning counts use parentheses in Czech. New dynamic templates fall
back to English in catalogs without entries. This is additional UI coverage, not a claim that
engine localization or native review is complete.

## Cross-language gaps

- Engine error, status and parameter-help messages are English in every language.
- Right-to-left UI layout is partial (2026-10-10): the shell mirrors for Arabic, Hebrew, Persian,
  Urdu and other right-to-left codes (`direction.rs`), but panel interiors, dialogs, menu rows,
  submenus and document tabs stay left to right. UI labels also lack an Arabic fallback font, and
  egui 0.36 joins letters within a word but orders a line's words left to right (no bidi).
- Web build: no browser-locale detection, no CJK web fonts.
- A user couldn't find the language option (#2532): it is in Preferences › Interface.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | major | First version, measured from the catalogs with the `i18n-coverage` rules |
| 2026-10-10 | minor | Right-to-left shell layout (menu bar, title, options and status bars, toolbox and dock sides) |
| 2026-10-11 | minor | Czech dynamic status/notice and hardware text, render labels and Navigator zoom context; engine messages remain English |
| 2026-10-11 | minor | Czech review follow-ups: English System Info diagnostics, id-aware shortcut labels, display-only auto translation, warning punctuation and wording; catalog row count refreshed |
| 2026-10-11 | minor | Italian: 1,287 dynamic labels added (filter parameters, Camera Raw, lighting, panels, dialogs) and terminology made consistent |
| 2026-10-12 | minor | Added Turkish (`tr`) catalog with 100% measured UI coverage |
| 2026-10-12 | minor | Spanish: 1,276 runtime-only labels added (filter and adjustment parameters, Camera Raw, panels, dialogs and status wrappers); generated-label coverage test added |
| 2026-10-12 | minor | Italian native-speaker review: mistranslations (Trap, HDR Toning, Display Text) and phrasing fixed |
| 2026-10-12 | minor | Serbian (`sr`, Latin, Ekavian) catalog: every measured key plus the dynamic labels of the other catalogs |
| 2026-10-12 | minor | Re-measured with `cargo xtask i18n-coverage`: 1,772 keys; Vietnamese catalog shipped (100%, replaces "none"); Simplified Chinese, French, Indonesian and Korean at 1,758 of 1,772; plug-in menu rows stay in Filter › Plug-ins in every language (#2540, #2627) and installed Japanese fonts are listed on Linux (#2330, #2961) |
| 2026-10-13 | minor | Italian: the `Screen` blend mode read *Schermo* (the display) and is now *Scherma*; the Dodge/Burn/Sponge tool names were already Adobe's (*Scherma*, *Brucia*, *Spugna*) and stay. Pinned by a test |
| 2026-10-14 | minor | Re-measured with `cargo xtask i18n-coverage` after merging main: 1,871 keys; Simplified Chinese, French, Indonesian and Korean at 1,857, the other key languages at 100%; catalog rows re-counted; other shipped languages refreshed (Hungarian listed, Polish at 1,904) |
| 2026-10-18 | minor | Russian: 1,229 dynamic labels added (filter and adjustment parameters, Camera Raw, lighting, History/Layers/paths menus, metadata, System Info, status templates, dialogs) with a terminology note ([localization-ru](localization-ru.md)) |
| 2026-10-21 | minor | Re-measured current main: 1,918 keys, 1,904 for French, Indonesian, Korean, Polish and Simplified Chinese; catalog rows recounted |
| 2026-10-22 | minor | Arabic row: paragraph direction has Auto / Left-to-right / Right-to-left buttons in the Paragraph panel and Type options bar (#6793); the visual RTL caret is still missing (TYPE-218-2 partial) |
| 2026-10-22 | minor | Re-measured current main: 1,982 keys, every key language except Hindi (267) and Arabic (none) at 100%; Hindi catalog listed as partial; catalog rows recounted |
