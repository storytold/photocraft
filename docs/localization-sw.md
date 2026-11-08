# Swahili localization and terminology

This note goes with the Swahili catalog (`crates/ui-egui/src/i18n/sw.tsv`). It covers every key
`cargo xtask i18n-coverage` measures and the runtime labels the other complete catalogs carry
(filter and adjustment parameters, Camera Raw, panels, dialogs, status messages). Translations are
written from the meaning of PhotoCraft's English labels; no proprietary translation resources,
fonts, screenshots or user documents are included. The catalog was machine-translated with Claude
for @tronicum against the terms below and has not yet been reviewed by a native speaker: review
corrections are welcome, and a changed term should change here and in every row that uses it.

## Choosing Swahili

Open **Edit > Preferences > Interface > Language** (Swahili: **Hariri › Mapendeleo › Kiolesura ›
Lugha**) and choose **Kiswahili**. With `interface.language` set to `auto`, the system locales
`sw`, `sw-KE`, `sw-TZ` and `sw_KE.UTF-8` resolve to Swahili.

## Swahili script

- Swahili is written in plain Latin script, so the UI font is Inter; no fallback font is needed.
- Digits are Western Arabic (0–9).
- The apostrophe in **ng'** is part of the spelling and marks the velar nasal, as in **Ung'aavu**
  (Luminosity) and **Ng'arisha** (Dodge). It is the plain ASCII `'`.

## Register and style

The catalog is written in Standard Swahili (Kiswahili Sanifu), in the neutral register of East
African software. It does not copy any other product's Swahili strings.

- **Sentence case.** Only the first word and proper names are capitalised, even where the English
  uses Title Case: `Save As…` is `Hifadhi kama…`, `Image Size…` is `Ukubwa wa picha…`, `Brush Tool`
  is `Zana ya brashi`. Product and format names keep their own case (PhotoCraft, Camera Raw).
- **Commands use the imperative**, the bare verb stem: Fungua, Hifadhi, Funga, Futa, Nakili. Menu
  items and buttons never use the infinitive (`Kufungua`) or the plural polite form (`Fungueni`).
- **Labels, panel names and headings** are nouns: `Layers` is `Safu`, `History` is `Historia`,
  `Selection` is `Uteuzi`.
- **Tool names** are `Zana ya` plus a noun, or `Zana ya ku-` plus a verb when the English names an
  action: `Zana ya brashi`, `Zana ya kupunguza` (Crop Tool), `Zana ya kusogeza` (Move Tool).
- **Ellipsis and colons.** The single character `…` stays exactly where the English has it, with
  no space before it, and is never written as `...`. Trailing colons stay too: `Size:` is
  `Ukubwa:`.
- **Placeholders** such as `{n}`, `{file}` and `{error}` are copied unchanged, but move to wherever
  Swahili word order needs them. Escapes `\n`, `\t` and `\\` stay as written.
- **Kept in Latin:** PhotoCraft, ArtCraft, Photoshop, Adobe, Camera Raw, file formats (PSD, PSB,
  TIFF, PNG, JPEG, EXR, SVG, PDF, GIF, WebP), ICC, CMYK, RGB, sRGB, Lab, HSB, HSL, OpenType, GPU,
  CPU, 3D, MCP, Wayland, X11, key names (Ctrl, Alt, Shift, Cmd, Esc, Enter, Tab, Space), unit
  symbols (px, pt, %, mm, cm, in, dpi, ppi, °) and the names Gaussian, Bicubic and Bilinear.
- **Quotation marks** follow the source (“ ” or " ").
- **Agreement.** Adjectives, possessives and verb prefixes follow the noun's class: `Safu mpya`
  (N), `Kikundi kipya` (KI-VI), `Mtindo mpya` (M-MI), `Dirisha jipya` (JI-MA), `Ubao mpya` (U).
- **Context column.** Rows with a context (`fontWeight`, `cameraRaw`, `layerLabel`, `curves`)
  override the plain row. An `@id` row translates the menu command with that id.

### Message patterns

- **Could not / Failed to:** `Imeshindwa ku-` plus the verb: `Could not open` is `Imeshindwa
  kufungua`, `Save failed: {error}` is `Imeshindwa kuhifadhi: {error}`. With a named subject the
  negative past agrees: `The file could not be opened.` is `Faili haikuweza kufunguliwa.`
- **Bare "Failed"** is `Imeshindikana`.
- **Done states** use the perfect passive, agreeing with the noun (`imefutwa`, `kimefutwa`).
- **"No X" / "Nothing to X":** `Hakuna safu` (No layers), `Hakuna cha kutendua` (Nothing to Undo).
- **"Not yet":** `bado` plus a negative: `bado haipatikani` (not available yet).
- **Tooltips** are plain present-tense statements, often with the habitual `hu-`: `Swaps height
  and width` is `Hubadilishana kimo na upana`.

## Plurals

Swahili uses two forms, **one|other**, where *one* is n = 1. An `@plural` row gives both forms
separated by `|`, each with every placeholder. The counted noun comes **before** the number
(`safu {n}`), and verbs agree with the noun's class in each form. N-class nouns (safu, picha,
faili, rangi, ruwaza, hatua) do not change; other classes swap their prefix.

| Source | Translation |
|---|---|
| `{n} Layer\|{n} Layers` | `Safu {n}\|Safu {n}` |
| `{n} item\|{n} items` | `Kipengee {n}\|Vipengee {n}` |
| `{n} action\|{n} actions` | `Kitendo {n}\|Vitendo {n}` |
| `{n} plane\|{n} planes` | `Bapa {n}\|Mabapa {n}` (a flat surface, never `ndege`, aircraft) |
| `Imported {n} style from {file}\|…` | `Mtindo {n} umeletwa kutoka {file}\|Mitindo {n} imeletwa kutoka {file}` |
| `Imported {n} pattern from {file}\|…` | `Ruwaza {n} imeletwa kutoka {file}\|Ruwaza {n} zimeletwa kutoka {file}` |

"to {file}" is `kwenye {file}`, "from {file}" is `kutoka {file}`.

## Core terms

### Menus

| English | Swahili |
|---|---|
| File / Edit / Image / Layer | Faili / Hariri / Picha / Safu |
| Type / Select / Filter / 3D | Maandishi / Chagua / Kichujio / 3D |
| View / Window / Help | Mwonekano / Dirisha / Msaada |

### Document, layers and selection

| English | Swahili |
|---|---|
| Document / Canvas | Hati / Turubai |
| Layer / Layers / Group | Safu / Safu / Kikundi (pl. vikundi) |
| Background (layer) | Mandharinyuma |
| Mask / Layer Mask / Clipping Mask | Barakoa / Barakoa ya safu / Barakoa ya kunasa |
| Channel / Alpha channel | Chaneli / chaneli ya alfa |
| Path / Work Path / Anchor point | Njia / Njia ya kazi / nukta ya nanga |
| Selection / Deselect / Inverse | Uteuzi / Ondoa uteuzi / Kinyume |
| Smart Object / Smart Filter | Kitu erevu / Kichujio erevu |
| Artboard / Guides / Grid / Rulers | Ubao wa sanaa / Miongozo / Gridi / Rula |
| Opacity / Fill: / Transparency | Ugubikaji / Ujazo: / Uwazi |
| Visibility / Lock | Uonekanaji / Funga kwa kufuli |

### Tools and panels

| English | Swahili |
|---|---|
| Tool / Panel / Toolbar / Workspace | Zana / Paneli / Upau wa zana / Eneo la kazi |
| Brush / Pencil / Eraser | Brashi / Penseli / Kifutio |
| Lasso / Magic Wand Tool | Kitanzi / Zana ya fimbo ya kichawi |
| Eyedropper / Patch Tool | Kidondoshi / Zana ya kiraka |
| Clone Stamp Tool | Zana ya muhuri wa kunakili |
| Healing Brush Tool | Zana ya brashi ya kuponya |
| Gradient / Paint Bucket Tool | Gradienti / Zana ya ndoo ya rangi |
| Blur / Sharpen / Smudge Tool | Zana ya ukungu / ya kunoa / ya kupaka |
| Dodge / Burn Tool | Zana ya kung'arisha / Zana ya kuchoma |
| Pen Tool / Horizontal Type Tool | Zana ya kalamu / Zana ya maandishi ya mlalo |
| Shape / Rectangle / Ellipse / Line | Umbo / Mstatili / Duaradufu / Mstari |
| Hand Tool / Zoom / Zoom In / Zoom Out | Zana ya mkono / Ukuzaji / Kuza / Dogesha |
| History / Actions / Properties | Historia / Vitendo / Sifa |
| Swatches / Presets / Styles / Pattern | Vibao vya rangi / Mipangilio-awali / Mitindo / Ruwaza |
| Character / Paragraph / Font | Herufi / Aya / Fonti |

### Adjustments, filters and effects

| English | Swahili |
|---|---|
| Adjustments / Adjustment Layer | Marekebisho / Safu ya marekebisho |
| Brightness / Contrast / Exposure | Mwangaza / Utofautishaji / Mfichuo |
| Levels / Curves / Vibrance | Viwango / Mikunjo / Uchangamfu |
| Hue / Saturation / Lightness | Aina ya rangi / Ukolezi / Uangavu |
| Color Balance / Black & White | Usawa wa rangi / Nyeusi na nyeupe |
| Invert / Posterize / Threshold | Geuza kinyume / Punguza toni / Kizingiti |
| Shadows / Highlights / Midtones | Vivuli / Sehemu angavu / Toni za kati |
| Noise / Reduce Noise / Grain | Kelele / Shusha kelele / Chembe |
| Distort / Warp / Liquify | Potosha / Pindisha / Yeyusha |
| Effects / Layer Style / Drop Shadow | Athari / Mtindo wa safu / Kivuli cha kuanguka |
| Outer Glow / Bevel & Emboss | Mng'ao wa nje / Mbonyeo na mchoro-tuta |
| Color Overlay | Tabaka la rangi |
| Stroke… (Edit) / Stroke (effect) | Chora mstari wa ukingo… / Mstari wa ukingo |
| Radius / Amount / Angle / Detail | Nusu-kipenyo / Kiasi / Pembe / Undani |
| Clarity / Dehaze / Texture | Uwazi wa undani / Ondoa ukungu wa angani / Unamu |

### Blend modes

| English | Swahili |
|---|---|
| Blend Mode | Hali ya uchanganyaji |
| Normal / Dissolve | Kawaida / Tawanya |
| Darken / Multiply / Color Burn | Tia giza / Zidisha / Choma rangi |
| Lighten / Screen / Color Dodge | Tia mwanga / Skrini / Ng'arisha rangi |
| Overlay / Soft Light / Hard Light | Funika / Mwanga laini / Mwanga mkali |
| Difference / Exclusion | Tofauti / Utengo |
| Subtract / Divide | Toa / Gawanya |
| Luminosity | Ung'aavu |

### Colour

| English | Swahili |
|---|---|
| Color / Foreground / Background Color | Rangi / Rangi ya mbele / Rangi ya nyuma |
| Color Mode / Color Picker | Hali ya rangi / Kichagua rangi |
| Color Profile / Working Space | Wasifu wa rangi / Nafasi ya kazi ya rangi |
| Gamut / Bit depth / Proof Colors | wigo wa rangi / Kina cha biti / Jaribio la rangi |
| Grayscale / Black / White | Kijivu / Nyeusi / Nyeupe |
| Red / Green / Blue | Nyekundu / Kijani / Bluu |
| Cyan / Magenta / Yellow | Samawati / Majenta / Njano |

### Buttons and common verbs

| English | Swahili |
|---|---|
| OK / Cancel / Apply / Reset | Sawa / Ghairi / Tekeleza / Weka upya |
| Open… / Save As… / Revert | Fungua… / Hifadhi kama… / Rudisha toleo lililohifadhiwa |
| Import / Export / Load / Place | Leta / Hamisha / Pakia / Weka |
| Undo / Redo | Tendua / Rudia |
| Cut / Copy / Paste / Delete / Clear | Kata / Nakili / Bandika / Futa / Safisha |
| Duplicate / Rename | Rudufu / Badilisha jina |
| Merge Down / Flatten Image | Unganisha chini / Bapisha picha |
| Rasterize / Transform / Free Transform | Geuza kuwa rasta / Geuza / Geuza huru |
| Scale / Rotate / Skew | Badilisha kipimo / Zungusha / Inamisha |
| Flip Horizontal / Crop / Trim | Pindua kwa mlalo / Punguza / Kata kingo |
| Feather / Smooth… / Expand / Contract | Lainisha kingo / Lainisha… / Panua / Bana |
| Link / Unlink / Group Layers | Unganisha / Tenganisha / Weka safu katika kikundi |
| Show / Hide / Enable / Disable | Onyesha / Ficha / Wezesha / Lemaza |

### Settings and units

| English | Swahili |
|---|---|
| Options / Settings / Preferences | Machaguo / Mipangilio / Mapendeleo |
| Default / Custom / Auto | Chaguo-msingi / Maalum / Otomatiki |
| Mode / Size / Width / Height | Hali / Ukubwa / Upana / Kimo |
| Resolution / Aspect Ratio | Msongo / Uwiano wa pande |
| Theme / Dark / Pro Light Theme | Mandhari / Giza / Mandhari ya Pro angavu |
| Keyboard Shortcuts / Plug-ins | Mikato ya kibodi / Programu-jalizi |
| Pixels / Inches / Percent | Pikseli / Inchi / Asilimia |
| Hardness / Spacing / Flow | Ugumu / Nafasi kati / Mtiririko |
| Tolerance / Anti-alias / Contiguous | Uvumilivu / Lainisha ncha / Zinazopakana |
| Font weights (`fontWeight`) | Nyembamba sana, Nyepesi, Kawaida, Wastani, Nzito, Mlazo |

## Kept apart

These pairs are easy to confuse; the catalog keeps them distinct.

- **Options** is `machaguo`; **Selection** is `uteuzi`; **Select** is `chagua`.
- **Size** is `ukubwa`; **Shape** is `umbo`.
- **Close** is `Funga`; **Lock** is `Funga kwa kufuli`.
- **Transform** is `Geuza`, **Convert to** is `Badilisha kuwa`, **Change** is `Badilisha`,
  **Replace** is `Weka badala`.
- **Invert** (colours) is `Geuza kinyume`, **Flip** is `Pindua`, **Inverse** (selection) is
  `Kinyume`.
- **Crop** is `Punguza`, **Cut** is `Kata`, **Trim** is `Kata kingo`, **Reduce** is `Shusha`.
- **Zoom In** is `Kuza`, **Grow** is `Ongeza eneo`, **Expand** is `Panua`.
- **Clear** is `Safisha`, **Delete** is `Futa`, **Remove** is `Ondoa`, **Purge** is `Ondoa kabisa`.
- **Brightness**, **Lightness** and **Luminosity** are `mwangaza`, `uangavu` and `ung'aavu`.
- **Opacity**, **Fill** and **Transparency** are `ugubikaji`, `ujazo` and `uwazi`.
- **Background** (layer) is `mandharinyuma`; background colour is `rangi ya nyuma`; **Theme** is
  `mandhari`.
- **Light** has four readings: the plain row and the font weight are `Nyepesi`, the light theme is
  `angavu`, Camera Raw light is `Mwanga`, and the **Lighten** blend mode is `Tia mwanga`.
- **Smooth** alone is the adjective `Laini`; the commands `Smooth…` and the `curves` Smooth are
  the verb `Lainisha`. **Feather** is `Lainisha kingo`.
- **Workspace** (layout) is `eneo la kazi`; colour **Working Space** is `nafasi ya kazi ya rangi`.

## Known ambiguities

Points for reviewers to check:

- **Rows / Columns** are `Safu mlalo` / `Safu wima`, while plain `safu` is also Layer. A grid
  dialog may read as if it talks about layers.
- **Link Layers** and **Merge Layers** both read `Unganisha safu`; only the menu position tells
  them apart. The bare verbs Link and Merge Down also share `Unganisha`.
- **Kerning** is `Nafasi kati ya herufi mbili` and **Tracking** is `Nafasi kati ya herufi`; they
  differ by one word. **Leading** is `Nafasi kati ya mistari`.
- **Clarity** is `Uwazi wa undani`, while plain `uwazi` is Transparency. **Transparent** is
  `Inayopenyeza mwanga`, not a form of `uwazi`.
