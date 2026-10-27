# Localization parity

> **Last reviewed:** 2026-10-26 · **Last updated:** 2026-10-26 · **Change:** minor (histogram, self-update and Artboards rows in every catalog but Hindi; coverage refreshed to the 2,054-key universe) · **Target:** Adobe Photoshop 2026

Per-language status of PhotoCraft's UI. How catalogs work and how to add one:
[`localization.md`](localization.md). Language notes: [Korean](localization-ko.md), [Spanish](localization-es.md),
[Simplified Chinese](localization-zh-hans.md), [Italian](localization-it.md), [Russian](localization-ru.md).

**How measured.** `cargo xtask i18n-coverage` (`xtask/src/i18n_coverage.rs`) defines the key
universe: `tl!("…")` literals and `trn(` plurals in `ui-egui` (tests excluded), menu-catalog
labels and path segments, `menus.rs` UI commands, and engine `CommandSpec` labels and menu paths:
**2,054 English keys** on 2026-10-26 (1,608 on 2026-10-10; the universe grows as the UI does). Each catalog (`crates/ui-egui/src/i18n/<code>.tsv`) was
checked against that universe by a source script replicating the xtask (no build). Catalogs hold
more rows than the universe because they also translate dynamic `tl!(expr)` strings (~159 call
sites the tool can't enumerate). **Not covered anywhere:** engine error and status messages and
filter parameter help, which are English in every language; so no language is `full`. The report
also lists, per language, the keys its catalog lacks; those show in English. `cargo xtask ci` prints
it as its last step and never fails on it.

**Photoshop 2026** ships UI language packs for cs, da, de, en, es, fi, fr (and fr-CA), hu, it,
ja, ko, nb, nl, pl, pt-BR, ru, sk, sv, tr, uk, zh-Hans, zh-Hant (24 `.lproj` markers; one pack is
installed per Creative Cloud install, here `en_US`). It has no Hindi, Arabic, Indonesian or
Vietnamese UI; Arabic and Hebrew users get English "Middle East" builds with the World-Ready
composer, and its text engine handles RTL, Devanagari and Indic scripts.

**Summary.** Every shipped translation catalog but Hindi is at 100% of measured keys: 9 of the 12
key languages (the rest: English is the source, Hindi is at 14%, Arabic is absent), plus 11 other
languages, also all at 100%. Status stays `partial` everywhere because of
the engine messages. Weighted by speakers, ~55% ready (Hindi dominates the weighting and is still
at 14%). Remaining: 80–140 h, plus native-speaker review (recorded only for Italian,
2026-10-12).

## The twelve key languages

| Language | Code | UI strings (measured keys) | Catalog rows | Dialogs / tooltips / help | Script support | Native review | Status | To `full` |
|---|---|---|---:|---|---|---|---|---:|
| English | `en` | 2,054 / 2,054 (100%) | source | all | Latin | – | full | 0 h |
| Simplified Chinese | `zh-hans` | 2,054 / 2,054 (100%) | 3,148 | dialogs, tooltips yes; engine messages English | CJK fonts (lazy loader), IME preedit/commit, vertical type | no | partial | 8–12 h |
| Spanish | `es` | 2,054 / 2,054 (100%) | 4,426 | as above; complete dynamic UI key set | Latin | no | partial | 6–10 h |
| Hindi | `hi` | 289 / 2,054 (14%) | 565 | Top menus, toolbox tools, camera raw, dialogs, plurals | Devanagari shaping in the text engine (parley/HarfRust); no Devanagari fallback font (TYPE-218-3) | no | partial | 15–20 h |
| Arabic | `ar` | 0 (0%) | – | – | Arabic shaping and bidi in the text engine, Arabic fallback font; RTL shell layout but not inside panels, and UI labels in LTR word order (DIST-220-4), paragraph direction buttons but no visual RTL caret (TYPE-218-2 partial), layer names as tofu (#1408, #1693) | no | none | 40–60 h |
| French | `fr` | 2,054 / 2,054 (100%) | 4,506 | as above | Latin | no | partial | 6–10 h |
| Portuguese (Brazil) | `pt-br` | 2,054 / 2,054 (100%) | 4,394 | as above | Latin; every `pt` locale uses it | no | partial | 6–10 h |
| Indonesian | `id` | 2,054 / 2,054 (100%) | 3,487 | as above | Latin | no | partial | 6–10 h |
| Japanese | `ja` | 2,054 / 2,054 (100%) | 4,144 | as above | CJK fonts, IME (Windows IME broken, #590), vertical type (tategaki); installed Japanese fonts missing from the list (#2330) | no | partial | 10–16 h |
| German | `de` | 2,054 / 2,054 (100%) | 3,489 | as above | Latin | no | partial | 6–10 h |
| Korean | `ko` | 2,054 / 2,054 (100%) | 3,684 | as above; terminology from Photoshop's Korean UI ([notes](localization-ko.md)) | Hangul fonts, IME | no | partial | 6–10 h |
| Vietnamese | `vi` | 2,054 / 2,054 (100%) | 3,963 | dialogs, tooltips yes; engine messages English | Latin with stacked diacritics (shaping supported) | no | partial | 6–10 h |

## Other shipped languages

11 more, each at 2,054 / 2,054 measured keys, status `partial` for the same reason: Traditional
Chinese `zh-hant` (3,104 rows), Russian `ru` (4,667 keys; also every dynamic label the
other catalogs translate, [notes](localization-ru.md)), Ukrainian `uk` (4,178), Czech `cs` (4,214),
Polish `pl` (4,085), Greek `el` (4,167), Dutch `nl` (4,224), Italian `it` (4,581; also every dynamic label the
other catalogs translate; reviewed by a native speaker, [notes](localization-it.md)), Hungarian `hu`
(4,379). Turkish `tr`
(4,193 rows) also covers every measured key. Serbian `sr` (4,537 rows, Latin script, Ekavian, added 2026-10-12) covers every
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
- WebAssembly plug-ins disappear from Filter › Plug-ins when the UI isn't English (#2540).
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
| 2026-10-13 | minor | Italian: the `Screen` blend mode read *Schermo* (the display) and is now *Scherma*; the Dodge/Burn/Sponge tool names were already Adobe's (*Scherma*, *Brucia*, *Spugna*) and stay. Pinned by a test |
| 2026-10-18 | minor | Russian: 1,229 dynamic labels added (filter and adjustment parameters, Camera Raw, lighting, History/Layers/paths menus, metadata, System Info, status templates, dialogs) with a terminology note ([localization-ru](localization-ru.md)) |
| 2026-10-22 | minor | Arabic row: paragraph direction has Auto / Left-to-right / Right-to-left buttons in the Paragraph panel and Type options bar (#6793); the visual RTL caret is still missing (TYPE-218-2 partial) |
| 2026-10-26 | minor | Histogram panel, self-update (#9322) and Artboards strings translated in 20 catalogs (229 rows); table refreshed to today's 2,054-key universe (measured at 1,608 on 2026-10-10) and the Vietnamese/Hungarian rows it had missed since 2026-10-12. Hindi is the one partial catalog (289 / 2,054 — its dynamic-labels batch is its own follow-up) |
