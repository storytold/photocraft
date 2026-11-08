# Marathi localization and terminology

This note goes with the Marathi catalog (`crates/ui-egui/src/i18n/mr.tsv`). It covers every key
`cargo xtask i18n-coverage` measures and the runtime labels the other complete catalogs carry
(filter and adjustment parameters, Camera Raw, panels, dialogs, status messages). Translations are
written from the meaning of PhotoCraft's English labels; no proprietary translation resources,
fonts, screenshots or user documents are included. The catalog was machine-translated with Claude
for @tronicum against the terms below and has not yet been reviewed by a native speaker: review
corrections are welcome, and a changed term should change here and in every row that uses it.

## Choosing Marathi

Open **Edit > Preferences > Interface > Language** (Marathi: **संपादन › प्राधान्ये › इंटरफेस › भाषा**) and choose
**मराठी**. With `interface.language` set to `auto`, the system locales `mr`, `mr-IN` and
`mr_IN.UTF-8` resolve to Marathi.

## Marathi script

- Marathi is written in Devanagari, like Hindi, but with Marathi spelling and vocabulary (ळ,
  ॲ/ॅ for the English "a" in loanwords such as ॲडजस्टमेंट and पॅनेल, verb forms such as करा).
- UI font: the Devanagari fallback in `crates/text/src/scripts.rs` (Noto Sans Devanagari and the
  system Devanagari faces). On the web the first served font for `Deva` draws it.
- Digits stay Western Arabic (0–9); the catalog uses no Devanagari digits.

## Register and style

- **Write Marathi, not Hindi.** Common slips to avoid:

  | Hindi (wrong) | Marathi (use) |
  |---|---|
  | करें, चुनें, हटाएँ, दिखाएँ, छिपाएँ, बदलें | करा, निवडा, हटवा, दाखवा, लपवा, बदला |
  | और, या | आणि, किंवा |
  | का / की / के | -चा / -ची / -चे (joined: लेयरचा, प्रतिमेची) |
  | में, से, पर, के लिए | -मध्ये, -पासून / -ने, -वर, -साठी |
  | है / हैं, नहीं | आहे / आहेत, नाही |
  | सभी, सब | सर्व |
  | चयन, चित्र | निवड, प्रतिमा |
  | रंग भरें | रंग भरा |

  Use ळ where Marathi spells it (काळा, पिवळा, निळा, वेळ, कळ, जुळवा, मूळ, ओळ).
- **Spelling of loanwords:** no nukta (फाइल, झूम, फिल्टर, रास्टराइझ, not फ़ाइल/ज़ूम).
  English "z" is झ; the English short "a" in cat/panel is written with ॅ (पॅनेल, कॅनव्हास, चॅनेल,
  मॅजिक, स्नॅप, टॅब), and the "o" in shop/top with ॉ (कॉपी, फॉन्ट, ऑब्जेक्ट). "v/w" is व्ह where
  Marathi does so (व्हिडिओ, कॅनव्हास, व्हायब्रन्स).
- **Menu commands and buttons:** polite imperative in -आ (उघडा, जतन करा, हटवा, लागू करा). Put the
  object first: "Delete Layer" → "लेयर हटवा", "Save As…" → "म्हणून जतन करा…", "New Layer…" →
  "नवीन लेयर…" (noun phrases stay noun phrases).
- **Checkbox and option labels:** noun phrase or imperative as the English does ("Show Grid" →
  "ग्रिड दाखवा", "Anti-alias" → "अँटी-अलियास").
- **Status/progress messages:** passive past for done ("जतन केले", "{name} उघडले"), present
  progressive for ongoing ("लोड होत आहे…"). Errors: "… करता आले नाही" ("Could not open {path}" →
  "{path} उघडता आले नाही").
- **Tooltips and help sentences:** full, plain Marathi sentences ending in the Latin full stop "."
  (never the danda "।"). Address the user with the polite imperative, not "तुम्ही" unless needed.
- **Ellipsis "…":** keep the single character U+2026 exactly where the English has it, directly
  after the last word, no space: "उघडा…", "इतर म्हणून जतन करा…". Never convert to "...".
- **Trailing colon:** keep it, no space before: "आकार:", "लक्ष्य:".
- **Placeholders** `{name}`, `{n}`, `{path}` … are copied unchanged, never translated or
  transliterated; add Marathi case endings after them with a hyphen where needed ("{name}-मध्ये"
  is acceptable, but prefer rephrasing: "{name} मध्ये").
- **Escapes** `\n`, `\t`, `\\` stay exactly as in the source.
- **Latin script stays Latin:** product, technology and file-format names (PhotoCraft, Photoshop,
  ArtCraft, PSD, PSB, TIFF, PNG, JPEG, EXR, SVG, PDF, ICC, CMYK, RGB, Lab, HSB, sRGB, OpenType,
  GPU, CPU, wgpu, MCP, JSON, Camera Raw, 3D), key names and shortcuts (Ctrl, Alt, Shift, Cmd,
  Esc, Tab, Enter, Space, Ctrl+S), units (px, pt, %, in, cm, mm, dpi, ppi, °, MB, GB), single
  axis letters (X, Y, W, H) and channel letters (R, G, B, A, L, a, b).
- **Digits:** use Western digits 0-9, never Devanagari digits.
- **Capitalisation** does not exist in Devanagari; do not try to imitate Title Case.
- **Tool names:** "<name> Tool" → "<name> साधन" ("Brush Tool" → "ब्रश साधन").
- **Context column:** `cameraRaw`, `fontWeight`, `smartSvg` etc. disambiguate; e.g. in
  `fontWeight` "Light" is a weight (लाइट), not illumination (प्रकाश) or a theme (फिकट).
- Loanwords are used for established photo-editing terms (tool, filter and blend-mode names,
  layer, mask, channel, path); plain Marathi for everyday actions and words.

## Plurals

Marathi uses two forms, **one|other**, where *one* is n = 1. An `@plural` row gives
`one|other` Marathi forms separated by `|`, both containing the same placeholder:

| Source | Translation |
|---|---|
| `{n} item\|{n} items` | `{n} आयटम\|{n} आयटम` |
| `{n} layer\|{n} layers` | `{n} लेयर\|{n} लेयर` |
| `{n} sample\|{n} samples` | `{n} नमुना\|{n} नमुने` |
| `{n} file\|{n} files` | `{n} फाइल\|{n} फाइल्स` |
| `{n} path\|{n} paths` | `{n} पाथ\|{n} पाथ` |
| `{n} color\|{n} colors` | `{n} रंग\|{n} रंग` |

Loanwords ending in a consonant usually stay invariant (लेयर, पाथ, आयटम); native nouns inflect
(नमुना/नमुने, रेषा/रेषा, निवड/निवडी). Always fill both forms even when they are identical.

## Core terms

### Menus

| English | Marathi |
|---|---|
| File | फाइल |
| Edit | संपादन |
| Image | प्रतिमा |
| Layer | लेयर |
| Type (menu) | मजकूर |
| Select (menu) | निवडा |
| Filter | फिल्टर |
| 3D | 3D |
| View | दृश्य |
| Window | विंडो |
| Help | मदत |
| Plugins / Plug-ins | प्लग-इन |

### Common verbs and actions

| English | Marathi |
|---|---|
| New | नवीन |
| Open… | उघडा… |
| Open As… | म्हणून उघडा… |
| Open Recent | अलीकडील उघडा |
| Save | जतन करा |
| Save As… | म्हणून जतन करा… |
| Save a Copy… | प्रत जतन करा… |
| Close | बंद करा |
| Close All | सर्व बंद करा |
| Revert | पूर्वस्थितीत आणा |
| Undo | पूर्ववत करा |
| Redo | पुन्हा करा |
| Cut | कापा |
| Copy | कॉपी करा |
| Paste | पेस्ट करा |
| Delete | हटवा |
| Clear | साफ करा |
| Duplicate | डुप्लिकेट करा |
| Merge | विलीन करा |
| Merge Down / Merge Visible | खाली विलीन करा / दृश्यमान विलीन करा |
| Flatten Image | प्रतिमा सपाट करा |
| Rasterize | रास्टराइझ करा |
| Transform / Free Transform | ट्रान्सफॉर्म / मुक्त ट्रान्सफॉर्म |
| Convert | रूपांतरित करा |
| Rotate | फिरवा |
| Flip / Flip Horizontal / Flip Vertical | पलटा / आडवे पलटा / उभे पलटा |
| Crop | क्रॉप करा |
| Resize / Image Size… / Canvas Size… | आकार बदला / प्रतिमेचा आकार… / कॅनव्हासचा आकार… |
| Scale | स्केल करा |
| Skew / Distort / Perspective / Warp | तिरके करा / विरूपित करा / परिप्रेक्ष्य / वार्प |
| Fill… | भरा… |
| Stroke… | स्ट्रोक… |
| Feather… | फेदर… |
| Invert | उलटवा |
| Inverse (selection) | निवड उलटवा |
| Move | हलवा |
| Import / Export | आयात करा / निर्यात करा |
| Load / Place | लोड करा / ठेवा |
| Create / Add / Remove | तयार करा / जोडा / काढा |
| Replace | बदला |
| Rename | नाव बदला |
| Show / Hide | दाखवा / लपवा |
| Lock / Unlock | लॉक करा / अनलॉक करा |
| Link / Unlink | लिंक करा / अनलिंक करा |
| Group / Ungroup | गट करा / गट रद्द करा |
| Enable / Disable | सक्षम करा / अक्षम करा |
| Apply | लागू करा |
| Commit (transform/edit) | लागू करा |
| Reset | रीसेट करा |
| Restore Defaults | डीफॉल्ट पुनर्संचयित करा |
| Browse… | ब्राउझ करा… |
| Search | शोधा |
| Zoom In / Zoom Out | झूम इन / झूम आउट |
| Fit on Screen | स्क्रीनवर बसवा |
| Drag / Click | ओढा / क्लिक करा |
| Print… | मुद्रित करा… |

### Dialog buttons and states

| English | Marathi |
|---|---|
| OK | ठीक आहे |
| Cancel | रद्द करा |
| Preview | पूर्वावलोकन |
| Yes / No | होय / नाही |
| Done | पूर्ण |
| Don't Save | जतन करू नका |
| Default | डीफॉल्ट |
| Custom | सानुकूल |
| Auto | स्वयं |
| None | काहीही नाही |
| All | सर्व |
| On / Off | चालू / बंद |
| Options | पर्याय |
| Settings | सेटिंग्ज |
| Preferences | प्राधान्ये |
| Preset / Presets | प्रीसेट |
| Error / Warning | त्रुटी / चेतावणी |
| Untitled | शीर्षकहीन |
| Loading… | लोड होत आहे… |
| Name / Value / Amount | नाव / मूल्य / प्रमाण |

### Document, canvas, view

| English | Marathi |
|---|---|
| Document | दस्तऐवज |
| Canvas | कॅनव्हास |
| Artboard | आर्टबोर्ड |
| Background | पार्श्वभूमी |
| Size | आकार |
| Width / Height | रुंदी / उंची |
| Resolution | रिझोल्यूशन |
| Pixel / Pixels | पिक्सेल |
| Bit Depth | बिट डेप्थ |
| Mode | मोड |
| Guides / Grid / Rulers | मार्गदर्शक रेषा / ग्रिड / रूलर |
| Snap | स्नॅप |
| Slice | स्लाइस |
| Horizontal / Vertical | आडवे / उभे |
| Top / Bottom / Left / Right | वर / खाली / डावीकडे / उजवीकडे |
| Angle / Radius / Distance | कोन / त्रिज्या / अंतर |
| Workspace | कार्यक्षेत्र |
| Theme / Light / Dark (theme) | थीम / फिकट / गडद |
| Panel | पॅनेल |
| Tab | टॅब |
| Screen Mode | स्क्रीन मोड |

### Layers, masks, channels, paths, selection

| English | Marathi |
|---|---|
| Layer / Layers | लेयर |
| Layer Group | लेयर गट |
| Group | गट |
| Background layer | पार्श्वभूमी लेयर |
| Adjustment Layer | ॲडजस्टमेंट लेयर |
| Fill Layer | फिल लेयर |
| Smart Object | स्मार्ट ऑब्जेक्ट |
| Layer Style | लेयर शैली |
| Effects | इफेक्ट्स |
| Opacity | अपारदर्शकता |
| Fill (opacity) | भरण |
| Blend Mode | मिश्रण मोड |
| Mask | मास्क |
| Layer Mask / Vector Mask | लेयर मास्क / व्हेक्टर मास्क |
| Clipping Mask | क्लिपिंग मास्क |
| Quick Mask | क्विक मास्क |
| Channel / Channels | चॅनेल |
| Alpha Channel | अल्फा चॅनेल |
| Path / Paths | पाथ |
| Work Path | कार्य पाथ |
| Anchor Point | अँकर बिंदू |
| Shape | आकृती (never आकार, which is Size) |
| Selection | निवड |
| Select All | सर्व निवडा |
| Deselect | निवड रद्द करा |
| Reselect | पुन्हा निवडा |
| Modify (selection) | सुधारा |
| Expand / Contract / Smooth | विस्तारवा / आकुंचित करा / गुळगुळीत करा |
| Edge / Edges | कडा |
| Tolerance | सहनशीलता |
| Contiguous | सलग |
| Anti-alias | अँटी-अलियास |
| Transparency / Transparent | पारदर्शकता / पारदर्शक |
| Visible / Hidden | दृश्यमान / लपवलेले |
| History | इतिहास |
| Actions | क्रिया |

### Tools

| English | Marathi |
|---|---|
| Tool | साधन |
| Move Tool | मूव्ह साधन |
| Marquee (Rectangular / Elliptical) | मार्की (आयताकृती / लंबवर्तुळाकार) |
| Lasso / Polygonal / Magnetic | लॅसो / बहुभुज / चुंबकीय |
| Magic Wand | मॅजिक वँड |
| Quick Selection | जलद निवड |
| Crop Tool | क्रॉप साधन |
| Eyedropper | आयड्रॉपर |
| Brush / Pencil | ब्रश / पेन्सिल |
| Eraser | खोडरबर |
| Paint Bucket | पेंट बादली |
| Gradient | ग्रेडियंट |
| Clone Stamp | क्लोन स्टॅम्प |
| Healing Brush / Spot Healing Brush | हीलिंग ब्रश / स्पॉट हीलिंग ब्रश |
| Patch | पॅच |
| Blur / Sharpen / Smudge | ब्लर / शार्पन / स्मज |
| Dodge / Burn / Sponge | डॉज / बर्न / स्पंज |
| Pen Tool | पेन साधन |
| Type Tool / Text | मजकूर साधन / मजकूर |
| Rectangle / Ellipse / Polygon / Line | आयत / लंबवर्तुळ / बहुभुज / रेषा |
| Hand Tool / Zoom Tool | हात साधन / झूम साधन |
| Liquify | लिक्विफाय |
| Brush Size / Hardness / Spacing | ब्रशचा आकार / कडकपणा / मधले अंतर |
| Flow / Pressure / Jitter | प्रवाह / दाब / जिटर |
| Pattern | पॅटर्न |
| Sample (noun) | नमुना |

### Type

| English | Marathi |
|---|---|
| Font | फॉन्ट |
| Font Size | फॉन्टचा आकार |
| Character / Paragraph | वर्ण / परिच्छेद |
| Style | शैली |
| Bold / Italic / Regular | ठळक / तिरपा / नियमित |
| Leading / Tracking / Kerning | लीडिंग / ट्रॅकिंग / कर्निंग |
| Align / Justify | संरेखित करा / समरेखित करा |

### Adjustments and filters

| English | Marathi |
|---|---|
| Adjustments | ॲडजस्टमेंट्स |
| Levels… | लेव्हल्स… |
| Curves… | कर्व्ह्ज… |
| Brightness/Contrast… | उजळपणा/कॉन्ट्रास्ट… |
| Hue/Saturation… | छटा/संपृक्तता… |
| Hue | छटा |
| Saturation | संपृक्तता |
| Brightness / Lightness | उजळपणा / फिकटपणा |
| Contrast | कॉन्ट्रास्ट |
| Exposure | एक्सपोजर |
| Vibrance | व्हायब्रन्स |
| Color Balance… | रंग संतुलन… |
| Black & White… | कृष्णधवल… |
| Photo Filter… | फोटो फिल्टर… |
| Channel Mixer… | चॅनेल मिक्सर… |
| Invert (adjustment) | उलटवा |
| Posterize… | पोस्टराइझ… |
| Threshold… | थ्रेशोल्ड… |
| Gradient Map… | ग्रेडियंट मॅप… |
| Selective Color… | निवडक रंग… |
| Shadows/Highlights… | सावल्या/हायलाइट्स… |
| Desaturate | रंगहीन करा |
| Shadows / Midtones / Highlights | सावल्या / मध्यम टोन / हायलाइट्स |
| Histogram | हिस्टोग्राम |
| Noise | नॉइज |
| Gaussian Blur… | गॉसियन ब्लर… |
| Unsharp Mask… | अनशार्प मास्क… |
| Detail | तपशील |

### Blend modes

| English | Marathi |
|---|---|
| Normal | सामान्य |
| Dissolve | डिझॉल्व्ह |
| Darken | गडद करा |
| Multiply | गुणाकार |
| Color Burn | कलर बर्न |
| Linear Burn | लिनियर बर्न |
| Darker Color | अधिक गडद रंग |
| Lighten | फिकट करा |
| Screen | स्क्रीन |
| Color Dodge | कलर डॉज |
| Linear Dodge (Add) | लिनियर डॉज (बेरीज) |
| Lighter Color | अधिक फिकट रंग |
| Overlay | ओव्हरले |
| Soft Light / Hard Light | सॉफ्ट लाइट / हार्ड लाइट |
| Vivid Light / Linear Light / Pin Light | व्हिव्हिड लाइट / लिनियर लाइट / पिन लाइट |
| Hard Mix | हार्ड मिक्स |
| Difference | फरक |
| Exclusion | एक्सक्लूजन |
| Subtract / Divide | वजाबाकी / भागाकार |
| Hue / Saturation / Color / Luminosity (modes) | छटा / संपृक्तता / रंग / ल्युमिनोसिटी |

### Colour

| English | Marathi |
|---|---|
| Color / Colors | रंग |
| Foreground / Background Color | अग्रभूमी रंग / पार्श्वभूमी रंग |
| Color Picker | रंग निवडक |
| Swatches | स्वॉचेस |
| Color Mode | रंग मोड |
| Grayscale / Gray | ग्रेस्केल / राखाडी |
| Red / Green / Blue | लाल / हिरवा / निळा |
| Cyan / Magenta / Yellow / Black | सायन / मॅजेंटा / पिवळा / काळा |
| White | पांढरा |
| Color Profile / Working Space | रंग प्रोफाइल / कार्य स्पेस |
| Convert to Profile… | प्रोफाइलमध्ये रूपांतरित करा… |
| Proof Colors | प्रूफ रंग |
| Gamut | गॅमट |

### Misc UI

| English | Marathi |
|---|---|
| Frame / Timeline / Video | फ्रेम / टाइमलाइन / व्हिडिओ |
| Source / Target | स्रोत / लक्ष्य |
| Current / Recent | सध्याचा / अलीकडील |
| Memory | मेमरी |
| Shortcut / Keyboard Shortcuts | शॉर्टकट / कीबोर्ड शॉर्टकट |
| Light (illumination) | प्रकाश |
| Shadow (effect) | सावली |
| Point | बिंदू |
| Profile | प्रोफाइल |
| Quick | जलद |
