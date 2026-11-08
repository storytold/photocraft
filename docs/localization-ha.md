# Hausa localization and terminology

This note goes with the Hausa catalog (`crates/ui-egui/src/i18n/ha.tsv`). It covers every key
`cargo xtask i18n-coverage` measures and the runtime labels the other complete catalogs carry
(filter and adjustment parameters, Camera Raw, panels, dialogs, status messages). Translations are
written from the meaning of PhotoCraft's English labels; no proprietary translation resources,
fonts, screenshots or user documents are included. The catalog was machine-translated with Claude
for @tronicum against the terms below and has not yet been reviewed by a native speaker: review
corrections are welcome, and a changed term should change here and in every row that uses it.

## Choosing Hausa

Open **Edit > Preferences > Interface > Language** (Hausa: **Gyara › Fifiko › Fuskar aiki ›
Harshe**) and choose **Hausa**. With `interface.language` set to `auto`, the system locales `ha`,
`ha-NG` and `ha_NG.UTF-8` resolve to Hausa.

## Hausa script

- Hausa is written in Latin script (Boko) with the hooked letters ɓ ɗ ƙ ƴ (capitals Ɓ Ɗ Ƙ Ƴ).
  They are never replaced with plain b, d, k or y.
- The glottal stop is written ʼ (U+02BC MODIFIER LETTER APOSTROPHE), never an ASCII apostrophe:
  aʼa, maʼauni, Nauʼin launi.
- UI font: Inter, the default UI face, covers all of these letters; no fallback font is needed.
- Digits are Western Arabic (0–9).

## Register and style

- **Register:** neutral, standard (Kano-based) Hausa as used in news and phone interfaces. The
  tone is polite and impersonal. The user is addressed with the imperative (Danna…, Zaɓa…), and
  gendered "ka/ki" is avoided; where a pronoun is unavoidable the catalog uses plural-polite "ku".
- **Top-level menus are nouns:** Fayil, Gyara, Hoto, Layer, Rubutu, Zaɓa, Filta, 3D, Duba, Taga,
  Taimako.
- **Commands, buttons and menu items use the plain imperative** (the verb base form): Buɗe,
  Ajiye, Rufe, Kwafi, Liƙa, Share, Juya. The verbal noun (buɗewa, ajiyewa) appears only in
  progress text: "Ana ajiyewa…" (Saving…), "Ana buɗewa…" (Opening…).
- **Status messages:** done is "An …" ("An ajiye {file}" = Saved {file}), failure is "An kasa …"
  ("An kasa buɗe {file}" = Could not open {file}), progress is "Ana …".
- **Sentence case:** only the first word and names are capitalised. Mid-string "layer", "mask"
  and "pikseli" are lowercase; Latin names keep their case.
- **Ellipsis:** "…" (U+2026) and "..." stay exactly as in the source, at the end, with no space
  before them: "Save As…" → "Ajiye a matsayin…".
- **Trailing colons** stay ("Size:" → "Girma:"), as do other punctuation, “quotes”, "·", "+", "/"
  and the spacing around placeholders.
- **Placeholders** `{name}`, `{n}`, `{file}`, `{what}`, `{errors}` … are copied untouched. They
  may move to fit Hausa word order. The escapes `\n`, `\t` and `\\` stay exactly as in the source.
- **Latin names stay unchanged:** PhotoCraft, ArtCraft, Photoshop, Camera Raw, PSD, PSB, TIFF,
  PNG, JPEG, GIF, WebP, PDF, SVG, ICC, EXIF, XMP, MCP, CMYK, RGB, Lab, HSB, HSL, sRGB, Adobe RGB,
  OpenType, Unicode, GPU, CPU, wgpu, 3D, HDR and "8-bit" / "16-bit" / "32-bit". Theme product
  names (Pro, Studio, Adwaita, Breeze, Solarized, Classic) keep their name; only "Theme" and
  "Light/Dark" are translated. Key names (Ctrl, Alt, Shift, Cmd, Option, Esc, Enter, Return, Tab,
  Space, Backspace, Delete), units (px, pt, %, in, cm, mm, dpi, ppi, °) and channel letters (R, G,
  B, C, M, Y, K, L, a, b) also stay in Latin.
- **Grammatical gender:** loanwords (layer, mask, fayil, burushi, pikseli, firam) are masculine
  (sabon layer, layer ɗin). Native nouns ending in -a are usually feminine (sabuwar tasha, sabuwar
  hanya, taswira → taswirar). The linker follows the gender: masculine -n / na, feminine -r / ta.
- **Possessive compounds** use the linker: "Layer Mask" → "Mask na layer", "Channel Options" →
  "Zaɓuɓɓukan tasha", "Brush Size" → "Girman burushi".
- **"X Tool"** → "Kayan aikin X" ("Brush Tool" → "Kayan aikin burushi"). In the tool list, where
  the English has no "Tool", the name stands alone.

## Plurals

Hausa uses two forms, **one|other**: *one* is n = 1 and *other* is everything else (0, 2, 3, …,
1.5). An `@plural` row gives both forms separated by `|`, and both contain `{n}` and every other
placeholder of the source. The number follows the noun: "{n} files" → "fayiloli {n}".

| Source | Translation |
|---|---|
| `{n} item\|{n} items` | `abu {n}\|abubuwa {n}` |
| `{n} layer\|{n} layers` | `layer {n}\|layer {n}` |
| `{n} step\|{n} steps` | `mataki {n}\|matakai {n}` |
| `{n} document\|{n} documents` | `daftari {n}\|daftarori {n}` |
| `Saved {n} color to {file}\|…` | `An ajiye launi {n} a {file}\|An ajiye launuka {n} a {file}` |

The *other* form uses the Hausa plural noun where an established one exists (fayiloli, launuka,
hotuna, daftarori, matakai, ayyuka, tashoshi, hanyoyi, abubuwa, salailai, adoji, samfura,
gutsuttsura). Loanwords and nouns without a common plural stay the same in both forms: layer,
mask, pikseli, burushi, firam, gargaɗi, and tasiri for counted effects. Impersonal "An …" phrasing
keeps the verb identical in both forms, so only the noun changes.

## Reserved distinctions

| English | Hausa | Not to be confused with |
|---|---|---|
| Select | Zaɓa | Selection = Yankin zaɓi |
| Options | Zaɓuɓɓuka | Settings = Saituna; Preferences = Fifiko |
| Size | Girma | Shape = Siffa; Scale = Daidaita girma; Image Size = Girman hoto |
| Light (theme) | Mai haske | Lighten = Ƙara haske; Brightness = Haske; Lightness = Matakin haske |
| Dark (theme) | Mai duhu | Darken = Ƙara duhu |
| Copy | Kwafi | Duplicate = Ninka; Clone Stamp = Hatimin kwafi |
| Delete | Share | Remove = Cire; Clear = Goge; Eraser = Magogi |
| Undo | Warware | Cancel = Soke; Revert = Koma kan ajiya |
| Redo | Sake yi | Repeat = Maimaita; Again = Kuma |
| Merge | Haɗe | Link = Haɗa da mahaɗi; Flatten = Shimfiɗe |
| Steps | Matakai | Levels = Matakan haske |
| Path | Hanya | File path = Wurin fayil; Shortcut = Gajeriyar hanya; Method = Dabara |
| Panel | Fanel | Screen = Allo; Artboard = Allon zane |
| Canvas | Fagen zane | Background = Bango |
| Document | Daftari | Paper = Takarda |
| Tool | Kayan aiki | Action = Aiki (pl. ayyuka) |
| Group | Rukuni | Batch = Gungu |
| Preset | Saitin shiryayye | Swatches = Samfuran launi; Sample = Samfuri |
| Difference | Bambanci | Contrast = Bambancin haske |
| Opacity | Kauri | Transparency = Bayyanar ciki; Density = Cunkoso; Width = Faɗi |
| Adjustments | Daidaitawa | Edit (menu) = Gyara |
| Invert | Juyar | Inverse = Juyi; Rotate = Juya; Flip = Kifa |

## Core terms

### Menus and common commands

| English | Hausa |
|---|---|
| File / Edit / Image / Layer | Fayil / Gyara / Hoto / Layer |
| Type / Select / Filter / View | Rubutu / Zaɓa / Filta / Duba |
| Window / Help | Taga / Taimako |
| New / Open… / Open Recent | Sabo (sabon/sabuwar + noun) / Buɗe… / Buɗe na kwanan nan |
| Save / Save As… / Close | Ajiye / Ajiye a matsayin… / Rufe |
| Import / Export / Print… | Shigo da / Fitar da / Buga… |
| Cut / Copy / Paste | Yanke / Kwafi / Liƙa |
| Transform / Free Transform | Sauya fasali / Sauya fasali sakaka |
| Crop / Rasterize | Datse / Mayar zuwa pikseli |
| Fill… / Stroke… / Feather… | Cika… / Layin gefe… / Laushin gefe… |
| Deselect / Reselect | Cire zaɓi / Sake zaɓa |
| Show / Hide / Lock | Nuna / Ɓoye / Kulle |
| Apply / Reset / Preview | Aiwatar / Sake saitawa / Duban farko |
| OK / Yes / No | To / Ee / Aʼa |
| Default / Custom / Auto | Na asali / Na musamman / Atomatik |
| Zoom In / Zoom Out | Zuƙo ciki / Zuƙo waje |

### Tools and panels

| English | Hausa |
|---|---|
| Move Tool | Kayan aikin matsarwa (the command Move is Matsar) |
| Lasso / Magic Wand | Igiya / Sandar sihiri |
| Eyedropper / Ruler | Maɗaukin launi / Rula |
| Brush / Pencil / Eraser | Burushi / Fensir / Magogi |
| Healing Brush / Patch | Burushin warkarwa / Faci |
| Gradient / Paint Bucket | Zamewar launi / Bokitin fenti |
| Blur / Sharpen / Smudge | Dusashe / Kaifafa / Shafa |
| Dodge / Burn / Sponge | Haskaka / Ƙona / Soso |
| Pen / Hand / Zoom | Alƙalami / Hannu / Zuƙowa |
| Channels / Paths / History | Tashoshi / Hanyoyi / Tarihi |
| Actions / Properties / Swatches | Ayyuka / Kaddarori / Samfuran launi |
| Character / Paragraph | Harafi / Sakin layi |
| Workspace / Theme / Language | Wurin aiki / Jigo / Harshe |
| Guides / Grid / Snap | Jagorori / Raga / Manne |

### Layers, masks, selections

| English | Hausa |
|---|---|
| Layer Mask / Vector Mask | Mask na layer / Mask na vector |
| Clipping Mask / Quick Mask | Mask mai yankewa / Mask cikin sauri |
| Smart Object / Smart Filter | Abu mai wayo / Filta mai wayo |
| Adjustment Layer | Layer na daidaitawa |
| Layer Style / Blend Mode | Salon layer / Yanayin gauraya |
| Opacity / Fill | Kauri / Cika |
| Channel / Alpha Channel | Tasha / Tashar alpha |
| Selection / Feather / Anti-alias | Yankin zaɓi / Laushin gefe / Santsin gefe |
| Tolerance / Contiguous | Juriya / A haɗe |
| Snapshot / Layer Comp | Hoton lokaci / Tsarin layer |
| Drop Shadow / Outer Glow | Inuwa a ƙasa / Sheƙi na waje |
| Color Overlay / Pattern Overlay | Ɗora launi / Ɗora ado |

### Adjustments, colour, blend modes

| English | Hausa |
|---|---|
| Levels / Curves / Exposure | Matakan haske / Lanƙwasai / Fitowar haske |
| Hue/Saturation / Vibrance | Nauʼin launi/Ƙarfin launi / Rayuwar launi |
| Black & White / Threshold | Baƙi da fari / Iyaka |
| Shadows / Midtones / Highlights | Inuwoyi / Tsakiyar haske / Wurare masu haske |
| Gaussian Blur / Unsharp Mask | Dusashe na Gauss / Kaifafa da mask |
| Noise / Vignette | Ɗigo-ɗigo / Duhun gefe |
| Color / Foreground / Background Color | Launi / Launin gaba / Launin bango |
| Grayscale / Color Space / Bit depth | Launin toka / Sararin launi / Zurfin bit |
| Resolution / Width / Height | Ƙuduri / Faɗi / Tsayi |
| Red / Green / Blue / Yellow | Ja / Kore / Shuɗi / Rawaya |
| Normal / Multiply / Screen | Na alʼada / Ninkawa / Allo |
| Overlay / Soft Light / Hard Light | Ɗorawa / Haske mai laushi / Haske mai ƙarfi |
| Color Burn / Color Dodge | Ƙona launi / Haskaka launi |
| Exclusion / Luminosity | Keɓewa / Hasken launi |

### Brushes, type, automation

| English | Hausa |
|---|---|
| Hardness / Spacing / Flow | Tauri / Tazara / Kwarara |
| Pressure / Tilt / Smoothing | Matsi / Karkata / Santsi |
| Font / Font size | Nauʼin harafi / Girman harafi |
| Scripts | Sikirifi |
| Batch / Step | Gungu / Mataki (pl. matakai) |
| Frame / Video / Memory | Firam / Bidiyo / Ƙwaƙwalwa |

The plain row "Script" reads "Rubutun lanƙwasa": it is the cursive type style, not an automation
script. Automation scripts are "sikirifi" ("Scripts", "Script Events Manager…").

## Terms settled during translation

| English | Hausa | Note |
|---|---|---|
| Rows | Layuka | |
| Columns | Ginshiƙai | |
| Working Space | Sararin aiki | colour working space; Workspace (panel layout) is Wurin aiki |
| Rasterize | Mayar zuwa pikseli | |
| Vanishing Point | Wurin ɓacewa | |
| Scratch disk | faifan maʼajiyar wucin gadi | "Scratch: {size}" = "Maʼajiyar wucin gadi: {size}" |
| Leading | Tazarar layuka | "Auto-Leading" uses the singular: Tazarar layi atomatik |
| Tracking | Tazarar haruffa | |
| Thumbnail | ƙaramin hoto | plural ƙananan hotuna |
| Folder | Babban fayil | |
| Gap | Gibi | |
| Grain | Ƙwaya | |
| Clipboard | allon kwafi | |
| Stop (gradient, colour stop) | ɗigon tsayawa (pl. ɗigogin tsayawa) | not tasha, which is Channel |
