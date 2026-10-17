# Arabic localization: first review batch

Arabic user help is available in the documentation book:
[دليل البداية](../book/src/arabic/quick-start.md),
[الأدوات بأمثلة](../book/src/arabic/tools.md) and
[قاموس المصطلحات](../book/src/arabic/glossary.md).
Those guides use the current English UI labels; they do not activate the Arabic catalog.

Status: **staged, not selectable**. This is an original Modern Standard Arabic
catalog for review, not a claim that the application has a working Arabic UI.
The draft is `crates/ui-egui/src/i18n/ar.tsv`; it is deliberately absent from
`i18n::LANGUAGES`, so Arabic system locales continue to follow the existing
fallback and are not switched to an incomplete interface.

## Scope

The first batch covers common menu labels, everyday tools, New Document controls,
shared buttons and language preferences. Untranslated strings remain English
when the catalog is eventually registered. Engine errors, API command IDs,
file names, user-entered layer names and preset names are outside this batch.

The translations were written from the meaning of the English source labels.
No proprietary localization resource was used. They have the same
MIT OR Apache-2.0 license as PhotoCraft; see `LICENSE-translations.txt` beside
the catalog. A human Arabic language review is still required.

## Vocabulary

Use concise Modern Standard Arabic rather than a regional dialect. Preserve
product names (PhotoCraft, ArtCraft), format names (PSD, PNG), technical codes
(RGB, CMYK, Lab), numbers and `{name}` placeholders. Keep the source ellipsis
on commands that open dialogs. Do not put visual-order text or Arabic
presentation-form characters into the catalog: translations must stay in logical
Unicode order for a future bidi-capable renderer.

| English | Arabic | Meaning |
|---|---|---|
| Layer | طبقة | One independently editable element of the document |
| Layer Mask | قناع الطبقة | Visibility control that preserves the layer's pixels |
| Selection | تحديد | The area on which an operation acts |
| Canvas | مساحة الرسم | The document's drawing area |
| Artboard | لوحة تصميم | A separate layout area within a document |
| Opacity | عتامة | How opaque an element is |
| Fill | تعبئة | Filling an area; keep distinct from opacity |
| Resolution | دقة | Pixel density; not the document's width and height |
| Preset | إعداد مسبق | A reusable set of settings |
| Smart Object | عنصر ذكي | A layer retaining its source content |

## Rendering work required before registration

Native script-font fallback already includes Arabic families in
`crates/text/src/scripts.rs` and loads installed fonts through
`cjk_fonts.rs`. That helps glyph coverage, including Arabic layer names, but
does not solve bidirectional layout. Availability depends on installed fonts;
the native system-font path is not available on the web.

The current egui 0.36 UI shapes Arabic words but does not correctly order a
mixed-direction line or handle its text caret. The existing shell mirrors for
RTL language codes, while panel interiors, dialogs and menu rows still need
work. These issues are separate from the document text engine.

Before adding `ar` to `LANGUAGES`:

1. Verify correct Arabic and mixed Arabic/English word order, punctuation and
   numerals in labels and editable fields, with matching caret and selection.
2. Verify glyph coverage on Windows, macOS, Linux and the web, including the
   language's own name in Preferences before it is selected.
3. Inspect New Document, Preferences, menus and layer names in small and large
   windows, in dark and light themes. Check English → Arabic → English in
   one session and saved preferences after restart.
4. Add Arabic's six integer plural forms (zero, one, two, few, many, other)
   before translating `@plural` messages. Do not use the English plural rule.
5. Update tests that currently assume every registered language is LTR, review
   partial-catalog coverage expectations, and keep `complete_menus` false
   until its coverage gates actually pass.

## Verification

`i18n::arabic_tests` exercises the actual catalog parser, placeholder and
ellipsis integrity, source-key presence, packing round trip and the staging
gate. Run it along with the normal localization checks:

```sh
cargo test -p photocraft-ui-egui --locked arabic
cargo test -p photocraft-ui-egui --locked
cargo clippy -p photocraft-ui-egui --all-targets --locked -- -D warnings
cargo xtask i18n-coverage
cargo xtask layers
cargo xtask wasm
```

The ordinary coverage report lists registered languages only; the staged
catalog must not be described as a shipped or fully translated language.
An HTML or document preview is not evidence that egui renders Arabic correctly;
the rendering checks above require the real application.

## ملاحظات للمراجع العربي

هذه بداية تعريب للمراجعة، ولن يظهر خيار العربية في البرنامج بهذه الدفعة.
اختيار المصطلحات يهدف إلى جعل الأدوات مفهومة ومتسقة، مع الحفاظ على أسماء
الملفات والصيغ كما هي.

عند المراجعة، افحص معنى العبارة في مكان استخدامها، لا ترجمة الكلمة وحدها.
مثلًا: «مساحة الرسم» هي حدود الصورة، أما «لوحة التصميم» فهي مساحة تصميم مستقلة
داخل المستند. «الحفظ» يحفظ المشروع، و«التصدير» ينتج ملفًا للاستخدام أو المشاركة.

بعد تجهيز عرض العربية، ينبغي تجربة عبارة مثل
«تصميم PhotoCraft رقم 12 — طبقة جديدة» وفحص ترتيب الكلمات والأرقام، وتحريك
مؤشر الكتابة وتحديد جزء من النص ونسخه ولصقه. صحة الترجمة وحدها لا تكفي
لإثبات صحة العرض والتحرير.
