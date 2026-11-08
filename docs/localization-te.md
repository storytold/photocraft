# Telugu localization and terminology

This note goes with the Telugu catalog (`crates/ui-egui/src/i18n/te.tsv`). It covers every key
`cargo xtask i18n-coverage` measures and the runtime labels the other complete catalogs carry
(filter and adjustment parameters, Camera Raw, panels, dialogs, status messages). Translations are
written from the meaning of PhotoCraft's English labels; no proprietary translation resources,
fonts, screenshots or user documents are included. The catalog was machine-translated with Claude
for @tronicum against the terms below and has not yet been reviewed by a native speaker: review
corrections are welcome, and a changed term should change here and in every row that uses it.

## Choosing Telugu

Open **Edit > Preferences > Interface > Language** (Telugu: **సవరణ › ప్రాధాన్యతలు › ఇంటర్‌ఫేస్ › భాష**) and choose
**తెలుగు**. With `interface.language` set to `auto`, the system locales `te`, `te-IN` and
`te_IN.UTF-8` resolve to Telugu.

## Telugu script

- UI font: PhotoCraft bundles no Telugu face. The fallback list in `crates/text/src/scripts.rs`
  uses the system's Telugu fonts. The web build has no system fonts, so it needs a served `Telu`
  font before Telugu can render there.
- Loanwords keep ZWNJ where Telugu software spells them that way (e.g. లేయర్‌లు).
- Digits stay Western Arabic (0–9).

## Register and style

- **Clean-room.** Translate from the plain meaning of the English. Never copy or consult any
  product's Telugu string tables (Adobe, Microsoft, Google, GIMP, etc.).
- **Register: plain imperative for commands, polite for sentences.**
  - Menu commands, buttons, tool actions: the plain (2nd-person singular) imperative, short and
    neutral, as in most Telugu software: తెరువు, సేవ్ చేయి, మూసివేయి, తొలగించు, వర్తింపజేయి.
    Never mix in -అండి forms in menus or buttons.
  - Full-sentence instructions, hints, questions and errors addressed to the user: polite -అండి
    form: "ఒక లేయర్‌ను ఎంచుకోండి.", "మళ్ళీ ప్రయత్నించండి."
  - Status/result messages: passive past "-బడింది": "{file} సేవ్ చేయబడింది", "లేయర్ తొలగించబడింది".
  - Labels, panel names, field names: nouns (వెడల్పు, అపారదర్శకత, లేయర్‌లు).
- **Word order is SOV.** The verb goes last: "Duplicate Layer" → "లేయర్‌ను నకలు చేయి" (object
  suffix optional in short menu labels: "లేయర్ నకలు చేయి" is fine; be consistent throughout).
  "Add Layer Mask" → "లేయర్ మాస్క్ జోడించు". "X Tool" → "X సాధనం". "X Options" → "X ఐచ్ఛికాలు".
- **Ellipsis "…".** Keep the single character "…" (U+2026) at the end, attached to the last
  Telugu word with no space: "తెరువు…", "చిత్ర పరిమాణం…". Never turn it into "...".
- **Trailing colon** is kept, attached with no space: "Amount:" → "మొత్తం:".
- **Placeholders** `{name}`, `{n}`, `{file}` etc. are copied untouched (never translated or
  transliterated), and every placeholder in the source must appear in the translation. Move them
  where Telugu grammar needs them; add case suffixes after them with no space only when unavoidable
  (prefer rephrasing: "{file} ఫైల్‌ను" rather than "{file}ను").
- **Latin letters stay Latin:** product and company names (PhotoCraft, ArtCraft, Photoshop, Adobe,
  Camera Raw, GIMP), file formats and tech names (PSD, PSB, TIFF, PNG, JPEG, PDF, SVG, ICC, CMYK,
  RGB, Lab, sRGB, OpenType, MCP, GPU, LUT, HDR, 3D), key names (Ctrl, Alt, Shift, Cmd, Option,
  Tab, Esc, Enter, Space), units and symbols (px, pt, %, °, dpi, ppi, fps), numbers.
- **Escapes** `\n`, `\t`, `\\` are kept exactly where they are in the source.
- **Loanwords** are written in Telugu script with ZWNJ (U+200C) before a suffix after a final
  virama, so the consonant does not form a conjunct: లేయర్‌లు, పిక్సెల్‌లు, ఛానెల్‌ను, బ్రష్‌తో.
  Exception (established native plural): ఫైల్ → ఫైళ్లు.
- **Capitalisation** does not exist in Telugu; "Layer" and "layer" translate the same.
- **On/Off, toggles:** ఆన్ / ఆఫ్. Enable → ప్రారంభించు, Disable → నిలిపివేయి.
- **Key shortcuts in text** ("Ctrl+Z") stay Latin.

## Plurals

Telugu uses two forms: **one|other** (one is n = 1, everything else, including 0, is other).
`@plural` rows give `one|other`, both containing `{n}`:

| source | translation |
|---|---|
| `{n} item\|{n} items` | `{n} అంశం\|{n} అంశాలు` |
| `{n} layer\|{n} layers` | `{n} లేయర్\|{n} లేయర్‌లు` |
| `{n} pixel\|{n} pixels` | `{n} పిక్సెల్\|{n} పిక్సెల్‌లు` |
| `{n} step\|{n} steps` | `{n} దశ\|{n} దశలు` |
| `{n} action\|{n} actions` | `{n} చర్య\|{n} చర్యలు` |
| `Saved {n} action to {file}\|Saved {n} actions to {file}` | `{n} చర్య {file} లో సేవ్ చేయబడింది\|{n} చర్యలు {file} లో సేవ్ చేయబడ్డాయి` |

Note the verb agrees: singular "-బడింది", plural "-బడ్డాయి".

## Core terms

### Menus

| English | Telugu |
|---|---|
| File | ఫైల్ |
| Edit (menu) | సవరణ |
| Image | చిత్రం |
| Layer | లేయర్ |
| Type (menu) | టైప్ |
| Select (menu) | ఎంపిక |
| Filter | ఫిల్టర్ |
| 3D | 3D |
| View | వీక్షణ |
| Window | విండో |
| Help | సహాయం |
| Plugins / Plug-ins | ప్లగిన్‌లు |

### Common verbs and actions

| English | Telugu |
|---|---|
| New… | కొత్తది… (adjective: కొత్త) |
| Open… | తెరువు… |
| Open Recent | ఇటీవలివి తెరువు |
| Save | సేవ్ చేయి |
| Save As… | వేరే పేరుతో సేవ్ చేయి… |
| Save a Copy… | కాపీని సేవ్ చేయి… |
| Close | మూసివేయి |
| Close All | అన్నీ మూసివేయి |
| Revert | మునుపటి స్థితికి మార్చు |
| Import | దిగుమతి చేయి |
| Export | ఎగుమతి చేయి |
| Place | ఉంచు |
| Print… | ముద్రించు… |
| Quit / Exit | నిష్క్రమించు |
| Undo | చర్యరద్దు |
| Redo | మళ్ళీ చేయి |
| Cut | కట్ చేయి |
| Copy | కాపీ చేయి |
| Paste | అతికించు |
| Delete | తొలగించు |
| Clear | తుడిచివేయి |
| Duplicate | నకలు చేయి (noun: నకలు) |
| Merge / Merge Down / Merge Visible | విలీనం చేయి / క్రిందికి విలీనం చేయి / కనిపించేవాటిని విలీనం చేయి |
| Flatten Image | చిత్రాన్ని చదును చేయి |
| Rasterize | రాస్టరైజ్ చేయి |
| Transform / Free Transform | రూపాంతరం (verb: రూపాంతరం చేయి) / స్వేచ్ఛా రూపాంతరం |
| Rotate / Rotation | తిప్పు / భ్రమణం |
| Flip Horizontal / Flip Vertical | అడ్డంగా తిరగేయి / నిలువుగా తిరగేయి |
| Crop | క్రాప్ చేయి |
| Trim | ట్రిమ్ చేయి |
| Resize | పరిమాణం మార్చు |
| Scale | స్కేల్ చేయి (noun: స్కేల్) |
| Fill… | నింపు… |
| Stroke… | స్ట్రోక్… (verb: స్ట్రోక్ చేయి) |
| Feather… | ఫెదర్… (verb: ఫెదర్ చేయి) |
| Invert | విలోమం చేయి (adjustment name: విలోమం) |
| Select All | అన్నీ ఎంచుకో |
| Deselect | ఎంపిక తీసివేయి |
| Reselect | మళ్ళీ ఎంచుకో |
| Inverse (selection) | ఎంపికను విలోమం చేయి |
| Add | జోడించు |
| Remove | తీసివేయి |
| Rename… | పేరు మార్చు… |
| Show / Hide | చూపించు / దాచు |
| Lock / Unlock | లాక్ చేయి / అన్‌లాక్ చేయి |
| Group / Ungroup | సమూహం చేయి / సమూహం విడదీయి |
| Link / Unlink | లింక్ చేయి / లింక్ తీసివేయి |
| Align / Distribute | సమలేఖనం చేయి / పంపిణీ చేయి |
| Arrange | అమర్చు |
| Bring to Front / Send to Back | ముందుకు తీసుకురా / వెనక్కి పంపు |
| Load… | లోడ్ చేయి… |
| Browse… | బ్రౌజ్ చేయి… |
| Find / Replace | కనుగొను / భర్తీ చేయి |
| Convert to … | …కు మార్చు |
| Enable / Disable | ప్రారంభించు / నిలిపివేయి |
| Record / Play / Stop | రికార్డ్ చేయి / ప్లే చేయి / ఆపు |
| Zoom In / Zoom Out | జూమ్ ఇన్ / జూమ్ అవుట్ |
| Fit on Screen | స్క్రీన్‌కు సరిపోయేలా |

### Dialog buttons and common UI words

| English | Telugu |
|---|---|
| OK | సరే |
| Cancel | రద్దు చేయి |
| Apply | వర్తింపజేయి |
| Reset | రీసెట్ చేయి |
| Preview | ప్రివ్యూ |
| Yes / No | అవును / కాదు |
| Don't Save | సేవ్ చేయవద్దు |
| Done | పూర్తయింది |
| Next / Previous / Back | తదుపరి / మునుపటి / వెనుకకు |
| Default | డిఫాల్ట్ |
| Custom | అనుకూల |
| Auto | ఆటో |
| None / (none) | ఏదీ లేదు / (ఏదీ లేదు) |
| All | అన్నీ |
| Options | ఐచ్ఛికాలు (never ఎంపికలు, which is reserved for selection) |
| Settings / Preferences | సెట్టింగ్‌లు / ప్రాధాన్యతలు |
| Properties | లక్షణాలు |
| Name | పేరు |
| Untitled | పేరులేనిది |
| Error / Warning | లోపం / హెచ్చరిక |
| Preset | ప్రీసెట్ |
| Advanced | అధునాతన |
| Shortcut / Keyboard Shortcuts | షార్ట్‌కట్ / కీబోర్డ్ షార్ట్‌కట్‌లు |
| Workspace | వర్క్‌స్పేస్ |
| Document | పత్రం |
| Panel | ప్యానెల్ |
| Tool / Tools | సాధనం / సాధనాలు |
| History | చరిత్ర |
| Action / Actions | చర్య / చర్యలు |
| Batch | బ్యాచ్ |
| Script | స్క్రిప్ట్ |
| (coming soon) | (త్వరలో వస్తుంది) |

### Image, canvas and units

| English | Telugu |
|---|---|
| Canvas | కాన్వాస్ |
| Image Size… / Canvas Size… | చిత్ర పరిమాణం… / కాన్వాస్ పరిమాణం… |
| Size | పరిమాణం |
| Width / Height | వెడల్పు / ఎత్తు |
| Resolution | రిజల్యూషన్ |
| Pixel / Pixels | పిక్సెల్ / పిక్సెల్‌లు |
| Inches / Centimeters / Millimeters | అంగుళాలు / సెంటీమీటర్లు / మిల్లీమీటర్లు |
| Points / Picas | పాయింట్లు / పైకాలు |
| Percent | శాతం |
| Degrees | డిగ్రీలు |
| Pixels/Inch | పిక్సెల్‌లు/అంగుళం |
| Mode | మోడ్ |
| Bits/Channel | బిట్‌లు/ఛానెల్ |
| Artboard | ఆర్ట్‌బోర్డ్ |
| Ruler / Guides / Grid | రూలర్ / గైడ్‌లు / గ్రిడ్ |
| Snap | స్నాప్ |
| Top / Bottom / Left / Right / Center | పైన / క్రింద / ఎడమ / కుడి / మధ్య |
| Horizontal / Vertical | అడ్డం / నిలువు |

### Layers, masks, channels, paths, selection

| English | Telugu |
|---|---|
| Layer / Layers | లేయర్ / లేయర్‌లు |
| Background (layer) | నేపథ్యం |
| Layer Mask | లేయర్ మాస్క్ |
| Vector Mask | వెక్టర్ మాస్క్ |
| Clipping Mask | క్లిప్పింగ్ మాస్క్ |
| Mask | మాస్క్ |
| Quick Mask | త్వరిత మాస్క్ |
| Layer Style | లేయర్ శైలి |
| Blending Options… | బ్లెండింగ్ ఐచ్ఛికాలు… |
| Adjustment Layer | సర్దుబాటు లేయర్ |
| Fill Layer | నింపు లేయర్ |
| Smart Object | స్మార్ట్ ఆబ్జెక్ట్ |
| Smart Filter | స్మార్ట్ ఫిల్టర్ |
| Layer Comp | లేయర్ కాంప్ |
| Group (noun) | సమూహం |
| Opacity / Fill (layer) | అపారదర్శకత / నింపు |
| Transparency | పారదర్శకత |
| Visible / Hidden | కనిపించే / దాచిన |
| Drop Shadow / Inner Shadow | పడే నీడ / లోపలి నీడ |
| Outer Glow / Inner Glow | బయటి కాంతి / లోపలి కాంతి |
| Bevel & Emboss | బెవెల్ & ఎంబాస్ |
| Satin | శాటిన్ |
| Color / Gradient / Pattern Overlay | రంగు ఓవర్‌లే / గ్రేడియంట్ ఓవర్‌లే / ప్యాటర్న్ ఓవర్‌లే |
| Channel / Channels | ఛానెల్ / ఛానెల్‌లు |
| Alpha Channel / Spot Channel | ఆల్ఫా ఛానెల్ / స్పాట్ ఛానెల్ |
| Path / Paths (vector) | పాత్ / పాత్‌లు (a file path is ఫైల్ మార్గం) |
| Work Path | పని పాత్ |
| Anchor Point | యాంకర్ పాయింట్ |
| Shape | ఆకారం |
| Vector | వెక్టర్ |
| Selection | ఎంపిక |
| Select (verb) | ఎంచుకో |
| Modify / Expand / Contract | మార్చు / విస్తరించు / కుదించు |
| Grow / Smooth | పెంచు / నునుపు చేయి |
| Border / Edge | సరిహద్దు / అంచు |
| Tolerance | సహనం |
| Anti-alias | యాంటీ-ఏలియాస్ |
| Contiguous | ఆనుకుని ఉన్న |
| Sample (noun) / Sample (verb) | నమూనా / నమూనా తీయి |

### Tools

| English | Telugu |
|---|---|
| Move Tool | కదిలించే సాధనం |
| Marquee | మార్కీ |
| Lasso | లాసో |
| Magic Wand | మ్యాజిక్ వాండ్ |
| Quick Selection | త్వరిత ఎంపిక |
| Crop Tool | క్రాప్ సాధనం |
| Eyedropper | ఐడ్రాపర్ |
| Brush | బ్రష్ |
| Pencil | పెన్సిల్ |
| Eraser | ఎరేజర్ |
| Clone Stamp | క్లోన్ స్టాంప్ |
| Healing Brush / Spot Healing Brush | హీలింగ్ బ్రష్ / స్పాట్ హీలింగ్ బ్రష్ |
| Patch | ప్యాచ్ |
| Paint Bucket | పెయింట్ బకెట్ |
| Gradient | గ్రేడియంట్ |
| Blur / Sharpen / Smudge | బ్లర్ / పదును / స్మడ్జ్ |
| Dodge / Burn / Sponge | డాడ్జ్ / బర్న్ / స్పాంజ్ |
| Pen | పెన్ |
| Type Tool / Text | టైప్ సాధనం / టెక్స్ట్ |
| Hand Tool | హ్యాండ్ సాధనం |
| Zoom Tool | జూమ్ సాధనం |
| Hardness / Spacing / Flow | కాఠిన్యం / అంతరం / ప్రవాహం |
| Angle / Roundness | కోణం / గుండ్రదనం |
| Radius / Amount / Strength | వ్యాసార్ధం / మొత్తం / బలం |
| Density | సాంద్రత |
| Jitter | జిట్టర్ |

### Type

| English | Telugu |
|---|---|
| Font | ఫాంట్ |
| Character / Paragraph | అక్షరం / పేరా |
| Leading / Tracking / Kerning | లీడింగ్ / ట్రాకింగ్ / కెర్నింగ్ |
| Baseline Shift | బేస్‌లైన్ షిఫ్ట్ |
| Bold / Italic | బోల్డ్ / ఇటాలిక్ |
| Warp Text | టెక్స్ట్ వంచు |

### Colour

| English | Telugu |
|---|---|
| Color / Colors | రంగు / రంగులు |
| Foreground Color / Background Color | ముందుభాగ రంగు / నేపథ్య రంగు |
| Swatches | స్వాచ్‌లు |
| Pattern | ప్యాటర్న్ |
| Color Picker | రంగు ఎంపిక సాధనం |
| Color Profile / Assign Profile / Convert to Profile | రంగు ప్రొఫైల్ / ప్రొఫైల్ కేటాయించు / ప్రొఫైల్‌కు మార్చు |
| RGB Color / CMYK Color / Lab Color | RGB రంగు / CMYK రంగు / Lab రంగు |
| Grayscale / Bitmap / Indexed Color | గ్రేస్కేల్ / బిట్‌మ్యాప్ / ఇండెక్స్డ్ రంగు |
| Duotone / Multichannel | డ్యుయోటోన్ / మల్టీఛానెల్ |
| Hue / Saturation / Lightness | వర్ణం / సంతృప్తత / తేలికదనం |
| Brightness / Contrast | ప్రకాశం / కాంట్రాస్ట్ |
| Luminosity | కాంతిత్వం |
| Black / White / Gray | నలుపు / తెలుపు / బూడిద రంగు |
| Red / Green / Blue | ఎరుపు / ఆకుపచ్చ / నీలం |
| Cyan / Magenta / Yellow | సియాన్ / మెజెంటా / పసుపు |
| Shadows / Midtones / Highlights | నీడలు / మధ్యటోన్‌లు / హైలైట్‌లు |

### Adjustments

| English | Telugu |
|---|---|
| Adjustments / Adjustment | సర్దుబాట్లు / సర్దుబాటు |
| Levels… | స్థాయిలు… |
| Curves… | వక్రరేఖలు… |
| Hue/Saturation… | వర్ణం/సంతృప్తత… |
| Brightness/Contrast… | ప్రకాశం/కాంట్రాస్ట్… |
| Exposure… | ఎక్స్‌పోజర్… |
| Vibrance… | వైబ్రెన్స్… |
| Color Balance… | రంగు సమతుల్యం… |
| Black & White… | నలుపు & తెలుపు… |
| Photo Filter… | ఫోటో ఫిల్టర్… |
| Channel Mixer… | ఛానెల్ మిక్సర్… |
| Gradient Map… | గ్రేడియంట్ మ్యాప్… |
| Selective Color… | ఎంపిక చేసిన రంగు… |
| Posterize… | పోస్టరైజ్… |
| Threshold… | థ్రెషోల్డ్… |
| Desaturate | సంతృప్తత తొలగించు |
| Replace Color… / Match Color… | రంగు భర్తీ చేయి… / రంగు సరిపోల్చు… |
| Shadows/Highlights… | నీడలు/హైలైట్‌లు… |
| Auto Tone / Auto Contrast / Auto Color | ఆటో టోన్ / ఆటో కాంట్రాస్ట్ / ఆటో రంగు |

### Filters

| English | Telugu |
|---|---|
| Filter Gallery… | ఫిల్టర్ గ్యాలరీ… |
| Blur / Gaussian Blur… | బ్లర్ / గాస్సియన్ బ్లర్… |
| Sharpen / Unsharp Mask… | పదును / అన్‌షార్ప్ మాస్క్… |
| Noise / Add Noise… | నాయిస్ / నాయిస్ జోడించు… |
| Distort | వక్రీకరించు |
| Stylize | శైలీకరించు |
| Render | రెండర్ |
| Pixelate | పిక్సెలేట్ |
| Liquify… | లిక్విఫై… |
| Other | ఇతరం |

### Blend modes (nouns, not commands)

| English | Telugu |
|---|---|
| Blend Mode | బ్లెండ్ మోడ్ |
| Normal | సాధారణం |
| Dissolve | కరుగు |
| Darken / Lighten | ముదురు / లేత |
| Multiply | గుణించు |
| Color Burn / Linear Burn | రంగు బర్న్ / లీనియర్ బర్న్ |
| Darker Color / Lighter Color | మరింత ముదురు రంగు / మరింత లేత రంగు |
| Screen | స్క్రీన్ |
| Color Dodge / Linear Dodge (Add) | రంగు డాడ్జ్ / లీనియర్ డాడ్జ్ (జోడించు) |
| Overlay | ఓవర్‌లే |
| Soft Light / Hard Light | మృదువైన కాంతి / కఠిన కాంతి |
| Vivid Light / Linear Light / Pin Light | ప్రకాశవంతమైన కాంతి / లీనియర్ కాంతి / పిన్ కాంతి |
| Hard Mix | హార్డ్ మిక్స్ |
| Difference / Exclusion | వ్యత్యాసం / మినహాయింపు |
| Subtract / Divide | తీసివేత / భాగించు |
| Hue / Saturation / Color / Luminosity | వర్ణం / సంతృప్తత / రంగు / కాంతిత్వం |

## Reserved distinctions (do not swap)

- ఎంపిక = selection / Select menu only; options are ఐచ్ఛికాలు.
- రద్దు చేయి = Cancel; Undo is చర్యరద్దు.
- కట్ చేయి = Cut; Crop is క్రాప్ చేయి.
- నమూనా = sample; Pattern is ప్యాటర్న్; Swatches are స్వాచ్‌లు.
- పాత్ = vector path; ఫైల్ మార్గం = file-system path.
- అంచు = edge; సరిహద్దు = border.
- విలీనం చేయి = merge; Dissolve (blend mode) is కరుగు.
