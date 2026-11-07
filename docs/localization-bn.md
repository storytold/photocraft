# Bengali catalog

The `bn` catalog is registered as **বাংলা** in the shared i18n foundation.
Select **Preferences > Interface > Language > বাংলা**, or set
`interface.language` to `bn` through the existing `prefs.set` command.
`bn-BD`, `bn-IN` and similar tags resolve to this catalog; with no saved
preference, `auto` follows the system locale.

The native build renders Bengali through the generic non-CJK script fallback:
the first time Bengali text is drawn, the font loader tries the system
candidates for `Script::Bengali` (Noto Sans Bengali on Linux, Bangla MN on
macOS, Nirmala UI on Windows; see `photocraft_text::scripts`). The web build
bundles no Bengali font: it fetches the first font the host lists for the
`Beng` script in `fonts/manifest.txt` (see `photocraft_text::served`) when
বাংলা is selected or Bengali text is drawn. Where the host serves none,
Bengali labels appear as boxes.

## Files and integration

- `crates/ui-egui/src/i18n/bn.tsv` contains the translations, independent of
  the lookup implementation. Its UTF-8 columns are `context<TAB>source<TAB>translation`.
- `crates/ui-egui/src/i18n.rs` registers `bn` with the Bengali plural rule
  (CLDR: 0 and 1 take the singular, like French) and `complete_menus: true`.
  Shared tests enforce coverage of every menu string, `tl!` literal, blend
  mode, brush section and generated preference label.
- Shortcut templates retain `{key}` so the shell supplies the correct key for
  the current platform.

Keep English source keys, contexts, command IDs, placeholders and escapes intact.
Retain the trailing `…` on commands that open a dialog. Missing translations use
the framework's English fallback. User-supplied names and document data are not
translated. Product names and technology names such as PhotoCraft, ArtCraft,
OpenType, RGB, CMYK and Lab retain their spelling. Status messages, errors and
automation output remain English as described in [UI design](ui-design.md#localisation);
complete catalog coverage does not claim those strings are translated.

## Terminology

Bengali computing vocabulary follows the conventions of Bengali GNOME/KDE
localisations: tool, filter, adjustment and effect names are transliterated
(লেয়ার, ব্লার, সিলেকশন, ফিল্টার), while actions use natural Bengali
(সংরক্ষণ করুন, নির্বাচন করুন, বাতিল করুন). Interface verbs use the formal
আপনি-form.

| English | বাংলা |
| --- | --- |
| Layer / Layer Comp | লেয়ার / লেয়ার কম্প |
| Mask / Clipping Mask | মাস্ক / ক্লিপিং মাস্ক |
| Selection / Feather | সিলেকশন / ফেদার |
| Blend Mode / Opacity | ব্লেন্ড মোড / অস্বচ্ছতা |
| Adjustment Layer | অ্যাডজাস্টমেন্ট লেয়ার |
| Smart Object / Smart Filter | স্মার্ট অবজেক্ট / স্মার্ট ফিল্টার |
| Canvas / Artboard | ক্যানভাস / আর্টবোর্ড |
| Brush / Stroke | ব্রাশ / স্ট্রোক |
| Fill / Gradient | ফিল / গ্রেডিয়েন্ট |
| Path / Rasterize | পাথ / র‍্যাস্টারাইজ |
| Preset / Swatch | প্রিসেট / সোয়াচ |
| Export / Preferences | এক্সপোর্ট / পছন্দসমূহ |
