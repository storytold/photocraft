# Traditional Chinese catalog

The `zh-hant` catalog contains original Traditional Chinese translations using
common Taiwanese interface terminology. It covers the same 1,936 English
source/context keys as the Japanese and Simplified Chinese catalogs based on
main at `a96a621`, plus three font-style context entries. It uses the shared
i18n foundation merged in PhotoCraft #169.
No proprietary translation resources were consulted or copied. Contributions
use the repository's MIT OR Apache-2.0 license.

## Selecting the language

Select **Preferences > Interface > Language > 繁體中文**, or set
`interface.language` to `zh-hant` with the existing `prefs.set` command.

Locale tags `zh-Hant`, `zh-TW`, `zh-HK` and `zh-MO` select this catalog, including
underscore, encoding-suffix and case variants. They share one Taiwanese-oriented
translation; this does not provide separate Hong Kong or Macau wording. Explicit
script tags take precedence over the region: `zh-Hant-CN` selects Traditional
Chinese, while `zh-Hans-TW`, bare `zh`, `zh-CN` and `zh-SG` resolve to Simplified
Chinese when that catalog is registered. They do not select Traditional Chinese;
unsupported locales use the existing English fallback.

The existing framework detects locale environment variables and macOS preferred
languages. Automatic Windows UI-language and browser-language detection are not
implemented; select the language manually there. Command IDs, the control
channel, CLI and MCP retain their English identifiers. Status messages, errors
and user-supplied names retain the upstream localization behavior.

## Terminology

| English | Traditional Chinese |
| --- | --- |
| Save / Export / Preferences | 儲存 / 匯出 / 偏好設定 |
| Undo / Redo | 復原 / 重做 |
| Layer / Layer Comp | 圖層 / 圖層構圖 |
| Mask / Clipping Mask | 遮色片 / 剪裁遮色片 |
| Selection / Feather | 選取範圍 / 羽化 |
| Brush / Brush Stroke | 筆刷 / 筆觸 |
| Stroke (layer effect) | 筆畫 |
| Fill / Gradient | 填色 / 漸層 |
| Blend Mode / Opacity | 混合模式 / 不透明度 |
| Adjustment Layer | 調整圖層 |
| Smart Object / Smart Filter | 智慧型物件 / 智慧型濾鏡 |
| Canvas / Artboard / Workspace | 畫布 / 工作區域 / 工作環境 |
| Path / Rasterize | 路徑 / 點陣化 |
| Transform / Warp | 變形 / 彎曲 |
| Pattern / Swatch | 圖樣 / 色票 |
| Color / Channel / Profile | 色彩 / 色版 / 色彩描述檔 |
| Font / Glyph / Shortcut | 字型 / 字形 / 快速鍵 |

Font weight labels use the `font-style` context: Light / Medium / Black
are 細體 / 中體 / 黑體. The same English words in color and size controls retain
their plain translations. Stored font style names stay in English.

## Validation and maintenance

Keep the TSV's context/source columns, command IDs, placeholders, escapes and
dialog ellipses intact. Missing entries use the shared English fallback.
Chinese uses one plural form. Product names, technology names and units such
as PhotoCraft, RGB, CMYK, Lab, OpenType and px retain their spelling.

Run the UI crate's tests and all-target Clippy, `cargo xtask layers` and
`cargo xtask wasm`. The existing complete-catalog checks enforce menu, `tl!`
literal, preference-label and blend-mode coverage, as well as parsing,
placeholder and ellipsis consistency. Traditional Chinese tests cover locale
selection, the Simplified/Traditional distinction, English fallback, plural
messages, font-style contexts and formatted shortcut hints. Review New Document,
Interface and Transparency Preferences, and the type style controls with the
stock offscreen `snapshot` example.

Native font delivery uses the existing system CJK fallback. This catalog adds
no font assets or overrides; changing the UI language does not change the
system locale's font ordering. The web build has no system CJK fallback,
so WASM compilation alone does not verify Traditional Chinese glyph rendering.

When source wording changes, check the actual UI call site, translate the new
meaning, retain stable context/source keys and rerun the catalog checks. Review
technical terms in context rather than mechanically converting Simplified
Chinese characters.
