# Documentation

Open **Help › Documentation** to search the tools, menu actions, settings and documentation included
with PhotoCraft. Search results show where each item lives. Select a result and press **Enter** or
choose **Show in app** to highlight its toolbar button or open its menu path. **Tab** and
**Shift+Tab** move between matches without running the selected action. Choose **Open documentation**
or right-click a result in **Help › Search menus** to open its matching entry in Documentation.

## What you can search

The search index includes every toolbar tool, every live menu item, each visible Preferences field,
headings and nearby text from the app's guides, and the tool, panel, and context-menu inventories. Selecting a toolbar tool also
reveals it if it was hidden in toolbar preferences, then highlights its button.
The compact search field inside the native Help menu lists toolbar tools, Preferences fields, and
documentation alongside commands. Right-click a result to open its entry in Documentation.

Tool names include descriptions and keyboard shortcuts. Menu results are indexed from the live menu
catalog, including disabled commands, so the search also explains where a not-yet-available item
belongs. Documentation results use their heading and nearby text as searchable terms.

## Media previews

Documentation entries can include local image or animated GIF previews. Images are loaded by egui's image
loader from embedded bytes, so previews work offline and in the desktop and web builds. Add an image
or GIF next to a guide and attach it to the guide's entry in `crates/ui-egui/src/help_search.rs`.

## Your notes

The **Your Notes** tab stores personal tutorials separately from PhotoCraft's shipped guides. Notes
use CommonMark Markdown, including headings, lists, tables, code, links, and images. Use the insert
buttons for image/GIF Markdown or video links, then replace the example URL with a URL from your own
server (or a `file://` URL for a local image/GIF). Image links render in the preview; video links open
in the system browser. In Preview mode, click and edit rendered text without displaying Markdown
syntax; multiline editing supports paragraphs. Select text in the editor to apply bold, italic,
strikethrough, or link formatting. Formatting can be nested inside other styles, and applying a
style that is already active leaves the selection unchanged. **Markdown Source** switches to the full
source editor. Import adds notes from an exported JSON file to the current list;
it never replaces existing notes. Export saves the current notes as JSON. Notes are stored locally in
PhotoCraft's configuration (or browser storage on the web) and are limited to 64 notes, 256 KB each,
and 4 MB total.

## Brush tool

Paint strokes on the active layer. Brush size, opacity, flow and pen pressure are configured in the
options bar and Brush Settings.

## Layer effects

Non-destructive layer styles include shadows, glows, overlays, strokes and bevel effects. Open Layer
Style from the Layers panel or its context menu. The preview updates while the style is edited.

![PhotoCraft layer style preview](images/photocraft-layer-styles.jpg)
