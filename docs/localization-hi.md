# Hindi terminology and style

This note goes with the completion of the Hindi catalog (`crates/ui-egui/src/i18n/hi.tsv`).
Translations are written from the meaning of PhotoCraft's English labels using ordinary Hindi
computing and image-editing vocabulary; no proprietary translation resources, string tables,
fonts, screenshots or user documents were consulted or included. The catalog has not yet had a
native speaker's review: corrections are welcome, and this page is where term choices are
recorded so a change can be made everywhere at once.

## Register and style

- **Who it is for.** Hindi-speaking designers and photographers mostly learn image editing with
  English terms. The catalog follows the vocabulary they already use: established image-editing
  jargon is written as a Devanagari loanword (*लेयर*, *मास्क*, *ब्रश*, *फ़िल्टर*, *ग्रेडिएंट*,
  *पिक्सेल*, *ओपेसिटी*), while everyday words and actions are plain Hindi (*सहेजें*, *खोलें*,
  *हटाएँ*, *रद्द करें*, *चयन*, *रंग*, *चौड़ाई*). Avoid Sanskritised coinages nobody uses
  (*परत* for Layer, *तूलिका* for Brush).
- **Commands** are polite imperatives ending in *-ें* / *-एँ*: *सहेजें*, *खोलें*, *हटाएँ*,
  *दिखाएँ*, *छिपाएँ*, *बदलें*, *लागू करें*. A verb borrowed from English takes *करें*:
  *कॉपी करें*, *क्रॉप करें*, *रीसेट करें*.
- **Nouns and panel names** stay nouns: *लेयर्स*, *चैनल्स*, *हिस्ट्री*, *स्तर…*, *कर्व्स…*.
- **Tool names** transliterate the English name and end in *टूल* (*ब्रश टूल*, *मैजिक वैंड टूल*);
  Select-family tools keep *चयन* (*क्विक चयन टूल*, *पाथ चयन टूल*).
- **Messages** are full sentences ending in the danda *।* (or *?* for questions). Use the formal
  *आप* register; never *तुम*.
- **Spelling.** Use nukta where Hindi computing text normally does (*फ़ाइल*, *ज़ूम*, *फ़ॉन्ट*,
  *साइज़*), and the chandrabindu form *-एँ* (*हटाएँ*, *दिखाएँ*, *प्राथमिकताएँ*), not *-एं*.
  Loanword plurals take *-्स* (*लेयर्स*, *गाइड्स*, *टूल्स*) in labels; after a number, Hindi
  plural forms follow the `@plural` rules below.
- **Kept as is:** product and technology names (PhotoCraft, ArtCraft, Camera Raw, Photoshop,
  OpenType, RGB, CMYK, Lab, HSB, HSL, ICC, PSD, PNG, JPEG, TIFF, WebP, OpenEXR, AVIF, GPU, CPU,
  MCP, sRGB, Adobe RGB (1998)), file extensions, keyboard keys and shortcuts (`Ctrl`, `Shift`,
  `Alt`, `(F)`), units' symbols (px, %, °, pt, mm, cm, in), and `{placeholders}`. Digits stay
  Latin (1, 2, 3), as in Hindi editions of desktop software.
- **Punctuation.** Keep the trailing `…` of dialog commands, `:` on field labels, `›` in menu paths,
  `·` separators and the typographic quotes `‘ ’ “ ”` the English uses.

## Plurals

Hindi treats 0 and 1 as singular (`plural_fr`: 0 and 1 → first form). `@plural` rows have two
forms, `one|other`. Hindi often uses the same noun for both (*{n} लेयर|{n} लेयर्स*,
*{n} फ़ाइल|{n} फ़ाइलें*), and verbs agree in number and gender (*लोड हुई|लोड हुईं*).

## Terminology

| English | Hindi | Note |
|---|---|---|
| Layer / Layers | लेयर / लेयर्स | feminine: *लेयर हटा दी गई* |
| Mask | मास्क | *लेयर मास्क*, *क्लिपिंग मास्क* |
| Selection / Select | चयन / चयन करें | *चयनित* = selected; *अचयनित करें* = Deselect |
| Brush | ब्रश | |
| Tool | टूल | |
| Image | इमेज | |
| Document | दस्तावेज़ | |
| File | फ़ाइल | |
| Canvas | कैनवास | |
| Color / Colour | रंग | in established compounds *कलर* (*कलर बैलेंस*, *कलर मिक्सर*) |
| Opacity | ओपेसिटी | |
| Fill | भरें (verb) / फ़िल (layer property) | *फ़िल लेयर* |
| Stroke | स्ट्रोक | |
| Gradient | ग्रेडिएंट | |
| Pattern | पैटर्न | |
| Swatches | स्वॉचेस | |
| Path | पाथ | *वर्क पाथ* for Work Path in labels |
| Channel | चैनल | |
| Adjustment(s) | समायोजन | *एडजस्टमेंट लेयर* for Adjustment Layer |
| Filter | फ़िल्टर | |
| Effect(s) | इफ़ेक्ट / इफ़ेक्ट्स | |
| Blend mode / Blending | ब्लेंड मोड / ब्लेंडिंग | |
| Preset(s) | प्रीसेट / प्रीसेट्स | |
| Preferences | प्राथमिकताएँ | |
| Settings | सेटिंग्स | |
| Options | विकल्प | |
| Preview | पूर्वावलोकन | |
| History | हिस्ट्री | panel and History Brush |
| Action(s) | एक्शन / एक्शन्स | the Actions panel |
| Workspace | कार्यस्थान | |
| Guide(s) | गाइड / गाइड्स | |
| Ruler(s) | रूलर / रूलर्स | |
| Grid | ग्रिड | |
| Artboard | आर्टबोर्ड | |
| Smart Object | स्मार्ट ऑब्जेक्ट | |
| Shape (vector) | शेप / शेप्स | shape layers, the Shapes panel, shape operations: *शेप लेयर*, *शेप्स मिलाएँ*, *कस्टम शेप टूल*; feminine |
| Shape (form) | आकार | a generic outline: *ब्रश टिप आकार*, *आकार ब्लर*, *आकार डायनेमिक्स* |
| Working space | वर्किंग स्पेस | colour settings; not *कार्यस्थान* (Workspace) |
| File path (disk) | फ़ाइल पथ | *पाथ* is only the vector path |
| W / H (abbreviations) | चौ / ऊँ | short for चौड़ाई / ऊँचाई |
| Highlights / Midtones / Shadows | हाइलाइट्स / मिडटोन्स / शैडो | spelled *शैडो* everywhere |
| Hue / Saturation / Brightness | ह्यू / सैचुरेशन / ब्राइटनेस | *चमक* only in *चमक/कंट्रास्ट…* |
| Contrast | कंट्रास्ट | |
| Exposure | एक्सपोज़र | |
| Feather | फ़ेदर | |
| Tolerance | टॉलरेंस | |
| Resolution | रिज़ॉल्यूशन | |
| Size | साइज़ | dimensions (*Camera Raw* keeps *आकार* for its Size slider) |
| Width / Height | चौड़ाई / ऊँचाई | |
| Angle | कोण | |
| Radius | त्रिज्या | |
| Amount | मात्रा | |
| Threshold | थ्रेशहोल्ड | |
| Noise | नॉइज़ | |
| Blur / Sharpen | ब्लर / शार्पन | |
| Transform | रूपांतरण | *मुक्त रूपांतरण* = Free Transform |
| Warp | वार्प | menu verb *मोड़ें* kept |
| Crop | क्रॉप | |
| Merge | मर्ज | |
| Group | समूह | |
| Link / Linked | लिंक / लिंक्ड | |
| Lock / Unlock | लॉक करें / अनलॉक करें | |
| Show / Hide | दिखाएँ / छिपाएँ | |
| Visible / Hidden | दृश्यमान / छिपा हुआ | |
| Enable / Disable | सक्षम करें / अक्षम करें | |
| Delete / Remove | हटाएँ | |
| Clear | साफ़ करें | |
| Reset | रीसेट करें | |
| Undo / Redo | पूर्ववत करें / फिर से करें | |
| Apply | लागू करें | |
| Cancel / OK | रद्द करें / ठीक है | |
| Done | संपन्न | |
| Default | डिफ़ॉल्ट | |
| Custom | कस्टम | |
| Foreground / Background color | फ़ोरग्राउंड रंग / बैकग्राउंड रंग | |
| Sample / Sampling | नमूना / सैंपलिंग | *Sample All Layers* = *सभी लेयर्स का नमूना लें* |
| Font | फ़ॉन्ट | |
| Type (text) | टाइप | *टाइप लेयर* |
| Character / Paragraph | कैरेक्टर / पैराग्राफ़ | the panels |
| Horizontal / Vertical | क्षैतिज / लंबवत | |
| Clockwise / Counter Clockwise | दक्षिणावर्त / वामावर्त | |
| Plug-in | प्लग-इन | |
| Error / Warning | त्रुटि / चेतावनी | |
| Export / Import | एक्सपोर्ट / इम्पोर्ट | |
| Print | प्रिंट | |
| Zoom | ज़ूम | |
| Tooltip | टूलटिप | |
| Shortcut | शॉर्टकट | |
| Panel | पैनल | |
| Menu | मेनू | |
