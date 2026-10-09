//! Documentation search: tools, menu actions, settings and the project's documentation, then highlight the
//! control or open the relevant documentation without executing an action.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

use egui::{Align2, Color32, CornerRadius, Id, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::state::Tool;
use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, icons};

const DOC_SOURCES: &[(&str, &str)] = &[
    ("README.md", include_str!("../../../README.md")),
    ("docs/architecture.md", include_str!("../../../docs/architecture.md")),
    ("docs/roadmap.md", include_str!("../../../docs/roadmap.md")),
    ("docs/parity.md", include_str!("../../../docs/parity.md")),
    ("docs/context-menu-inventory.md", include_str!("../../../docs/context-menu-inventory.md")),
    ("docs/context-menu-parity.md", include_str!("../../../docs/context-menu-parity.md")),
    ("docs/ui-design.md", include_str!("../../../docs/ui-design.md")),
    ("docs/help-search.md", include_str!("../../../docs/help-search.md")),
    ("docs/control-protocol.md", include_str!("../../../docs/control-protocol.md")),
    ("docs/development.md", include_str!("../../../docs/development.md")),
    ("docs/color-picker.md", include_str!("../../../docs/color-picker.md")),
    ("docs/camera-raw-zoom.md", include_str!("../../../docs/camera-raw-zoom.md")),
    ("docs/camera-raw-histogram.md", include_str!("../../../docs/camera-raw-histogram.md")),
    ("docs/plugins.md", include_str!("../../../docs/plugins.md")),
];

const USER_NOTE_LIMIT: usize = 64;
const USER_NOTE_BYTES_LIMIT: usize = 256 * 1024;
const USER_NOTES_TOTAL_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct UserNote {
    pub(crate) title: String,
    pub(crate) markdown: String,
}

pub(crate) fn load_notes(text: Option<&str>) -> Vec<UserNote> {
    let Some(text) = text.filter(|text| text.len() <= USER_NOTES_TOTAL_LIMIT) else { return Vec::new() };
    let Ok(notes) = serde_json::from_str::<Vec<UserNote>>(text) else { return Vec::new() };
    let valid = notes.len() <= USER_NOTE_LIMIT && notes.iter().all(|note| note.title.chars().count() <= 160 && note.markdown.len() <= USER_NOTE_BYTES_LIMIT);
    if valid { notes } else { Vec::new() }
}

fn append_imported_notes(existing: &mut Vec<UserNote>, bytes: &[u8]) -> Result<std::ops::Range<usize>, String> {
    if bytes.len() > USER_NOTES_TOTAL_LIMIT {
        return Err("The notes file is larger than 4 MB".into());
    }
    let imported: Vec<UserNote> = serde_json::from_slice(bytes).map_err(|error| format!("Couldn't read notes file: {error}"))?;
    if imported.iter().any(|note| note.title.chars().count() > 160 || note.markdown.len() > USER_NOTE_BYTES_LIMIT) {
        return Err("The notes file contains a note above the size limit".into());
    }
    let new_len = existing.len().checked_add(imported.len()).ok_or("Too many notes")?;
    if new_len > USER_NOTE_LIMIT {
        return Err(format!("Import would exceed the {USER_NOTE_LIMIT}-note limit"));
    }
    let start = existing.len();
    existing.extend(imported);
    let total_bytes = serde_json::to_vec(existing).map_err(|error| error.to_string())?.len();
    if total_bytes > USER_NOTES_TOTAL_LIMIT {
        existing.truncate(start);
        return Err("Import would exceed the 4 MB notes limit".into());
    }
    Ok(start..existing.len())
}

fn export_notes(notes: &[UserNote]) -> Result<Vec<u8>, String> {
    if notes.len() > USER_NOTE_LIMIT || notes.iter().any(|note| note.title.chars().count() > 160 || note.markdown.len() > USER_NOTE_BYTES_LIMIT) {
        return Err("Documentation notes exceed the export limits".into());
    }
    let bytes = serde_json::to_vec_pretty(notes).map_err(|error| format!("Couldn't encode notes: {error}"))?;
    if bytes.len() > USER_NOTES_TOTAL_LIMIT {
        return Err("Documentation notes exceed the 4 MB export limit".into());
    }
    Ok(bytes)
}

fn save_notes(app: &mut PhotocraftApp) {
    let valid = app.documentation_notes.len() <= USER_NOTE_LIMIT
        && app.documentation_notes.iter().all(|note| note.title.chars().count() <= 160 && note.markdown.len() <= USER_NOTE_BYTES_LIMIT);
    if !valid {
        app.ui.status = "Documentation notes exceed the size limit".into();
        app.ui.status_error = true;
        return;
    }
    let Some(save) = app.services.save_documentation.as_mut() else { return };
    let Ok(text) = serde_json::to_string(&app.documentation_notes) else {
        app.ui.status = "Couldn't save Documentation notes".into();
        app.ui.status_error = true;
        return;
    };
    if text.len() > USER_NOTES_TOTAL_LIMIT {
        app.ui.status = "Documentation notes exceed the 4 MB storage limit".into();
        app.ui.status_error = true;
        return;
    }
    if let Err(error) = save(&text) {
        app.ui.status = format!("Couldn't save Documentation notes: {error}");
        app.ui.status_error = true;
    }
}

#[derive(Clone)]
enum Action {
    Tool(Tool),
    Menu { id: String, path: Vec<String> },
    Setting { section: String },
    UserNote { index: usize },
    Documentation(String),
}

#[derive(Clone)]
struct Entry {
    title: String,
    location: String,
    description: String,
    search_text: String,
    documentation_url: String,
    action: Action,
    media: Option<HelpMedia>,
}

#[derive(Clone)]
pub(crate) struct HelpMedia {
    pub(crate) uri: &'static str,
    pub(crate) bytes: &'static [u8],
    pub(crate) caption: &'static str,
}

pub(crate) struct DocumentationResult {
    pub(crate) title: String,
    pub(crate) location: String,
    pub(crate) description: String,
    pub(crate) url: String,
    pub(crate) media: Option<HelpMedia>,
    score: i32,
}

#[derive(Clone)]
struct Hit {
    score: i32,
    entry: Entry,
}

#[derive(Clone, Default)]
struct VisibleTargets {
    frame: u64,
    rects: HashMap<String, Rect>,
}

#[derive(Clone)]
struct Navigation {
    target: String,
    path: Vec<String>,
    until: f64,
}

fn targets_id() -> Id {
    Id::new("help-search-visible-targets")
}

fn navigation_id() -> Id {
    Id::new("help-search-navigation")
}

fn query_id() -> Id {
    Id::new("help-search-query")
}

fn previous_query_id() -> Id {
    Id::new("help-search-previous-query")
}

fn selected_id() -> Id {
    Id::new("help-search-selected")
}

/// Record a real control's screen rectangle for this egui pass.
pub(crate) fn register_target(ctx: &egui::Context, target: impl Into<String>, rect: Rect) {
    let frame = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        let mut targets: VisibleTargets = data.get_temp(targets_id()).unwrap_or_default();
        if targets.frame != frame {
            targets.frame = frame;
            targets.rects.clear();
        }
        targets.rects.insert(target.into(), rect);
        data.insert_temp(targets_id(), targets);
    });
}

/// Highlight a navigated menu item while its menu path is open.
pub(crate) fn highlight_id(id: &str) -> String {
    format!("menu:{id}")
}

/// Open the Documentation search section.
pub(crate) fn open(app: &mut PhotocraftApp, ctx: &egui::Context) {
    open_with_query(app, ctx, "");
}

pub(crate) fn open_with_query(app: &mut PhotocraftApp, ctx: &egui::Context, query: &str) {
    app.ui.help_search_open = true;
    ctx.data_mut(|data| {
        data.insert_temp(query_id(), query.to_string());
        data.insert_temp(Id::new("help-search-section"), "search".to_string());
        data.insert_temp(selected_id(), 0usize);
    });
}

pub(crate) fn open_note(app: &mut PhotocraftApp, ctx: &egui::Context, index: usize) {
    app.ui.help_search_open = true;
    ctx.data_mut(|data| {
        data.insert_temp(query_id(), String::new());
        data.insert_temp(Id::new("help-search-section"), "notes".to_string());
        data.insert_temp(Id::new("help-search-note"), index);
    });
}

/// Find the active menu navigation request, if it has not expired.
pub(crate) fn menu_navigation(ctx: &egui::Context) -> Option<(String, Vec<String>)> {
    ctx.data(|data| data.get_temp::<Navigation>(navigation_id())).filter(|nav| ctx.input(|i| i.time) < nav.until).map(|nav| (nav.target, nav.path))
}

fn docs_index() -> &'static [Entry] {
    static INDEX: OnceLock<Vec<Entry>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut entries = Vec::new();
        for (path, source) in DOC_SOURCES {
            let mut title = String::new();
            let mut section = String::new();
            let mut depth = 0usize;
            let flush = |title: &str, section: &str, depth: usize, entries: &mut Vec<Entry>| {
                if title.is_empty() || depth > 3 {
                    return;
                }
                let description = doc_snippet(section);
                let anchor = slug(title);
                let url = format!("https://github.com/storytold/photocraft/blob/main/{path}#{anchor}");
                let media = if title.to_lowercase().contains("layer style") || title.to_lowercase().contains("layer effect") {
                    Some(HelpMedia {
                        uri: "bytes://help-search/layer-styles.jpg",
                        bytes: include_bytes!("../../../docs/images/photocraft-layer-styles.jpg"),
                        caption: "Layer styles in PhotoCraft",
                    })
                } else {
                    None
                };
                entries.push(Entry {
                    title: title.to_string(),
                    location: format!("Documentation › {path}"),
                    description,
                    search_text: format!("{title} {path} {section}"),
                    documentation_url: url.clone(),
                    action: Action::Documentation(url),
                    media,
                });
            };
            for line in source.lines() {
                let hashes = line.chars().take_while(|c| *c == '#').count();
                if hashes > 0 && line.chars().nth(hashes) == Some(' ') {
                    flush(&title, &section, depth, &mut entries);
                    title = line[hashes + 1..].trim().to_string();
                    section.clear();
                    depth = hashes;
                } else {
                    section.push_str(line);
                    section.push('\n');
                }
            }
            flush(&title, &section, depth, &mut entries);
        }
        entries
    })
}

fn preference_label(key: &str) -> String {
    let mut label = String::new();
    for (index, character) in key.chars().enumerate() {
        if index == 0 {
            label.extend(character.to_uppercase());
        } else if character.is_uppercase() {
            label.push(' ');
            label.extend(character.to_lowercase());
        } else {
            label.push(character);
        }
    }
    label.replace("Ui ", "UI ").replace("Gpu", "GPU ").replace("Psd", "PSD")
}

fn settings_entries() -> Vec<Entry> {
    let values = photocraft_engine::prefs::Preferences::default().to_json();
    let mut entries = Vec::new();
    for (section_id, section_title) in photocraft_engine::prefs::SECTIONS {
        let Some(fields) = values.get(section_id).and_then(serde_json::Value::as_object) else { continue };
        for (key, value) in fields {
            let path = format!("{section_id}.{key}");
            if photocraft_engine::prefs::is_hidden(&path) {
                continue;
            }
            let title = preference_label(key);
            let location = format!("Edit › Preferences › {section_title}");
            let values_text = match value {
                serde_json::Value::String(value) => value.clone(),
                serde_json::Value::Bool(value) => value.to_string(),
                serde_json::Value::Number(value) => value.to_string(),
                _ => String::new(),
            };
            let option_text = photocraft_engine::prefs::choices(&path).unwrap_or_default().join(" ");
            entries.push(Entry {
                title: title.clone(),
                location: location.clone(),
                description: format!("Configure {title} in Preferences › {section_title}."),
                search_text: format!("{title} {key} {section_title} {section_id} {values_text} {option_text} setting preference option"),
                documentation_url: documentation_url_for(
                    &format!("{title} {section_title}"),
                    "https://github.com/storytold/photocraft/blob/main/docs/help-search.md#what-you-can-search",
                ),
                action: Action::Setting { section: section_id.to_string() },
                media: None,
            });
        }
    }
    entries
}

pub(crate) fn setting_results(query: &str) -> Vec<DocumentationResult> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut results: Vec<_> = settings_entries()
        .into_iter()
        .filter_map(|entry| {
            crate::palette::fuzzy_score(query, &format!("{} {} {}", entry.title, entry.location, entry.search_text)).map(|score| {
                let exact_option = entry.search_text.to_lowercase().contains(&query.to_lowercase());
                DocumentationResult {
                    title: entry.title,
                    location: entry.location,
                    description: entry.description,
                    url: entry.documentation_url,
                    media: entry.media,
                    score: score + if exact_option { 100 } else { 0 },
                }
            })
        })
        .collect();
    results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
    results.truncate(12);
    results
}

pub(crate) fn note_results(app: &PhotocraftApp, query: &str) -> Vec<DocumentationResult> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut results: Vec<_> = app
        .documentation_notes
        .iter()
        .enumerate()
        .filter_map(|(index, note)| {
            let search = format!("{} {}", note.title, note.markdown);
            crate::palette::fuzzy_score(query, &search).map(|score| DocumentationResult {
                title: note.title.clone(),
                location: format!("Documentation › Your Notes · {}", index + 1),
                description: doc_snippet(&note.markdown),
                url: format!("photocraft-note://{index}"),
                media: None,
                score,
            })
        })
        .collect();
    results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
    results.truncate(8);
    results
}

#[derive(Default)]
struct PreviewSegment {
    text: String,
    source: Option<std::ops::Range<usize>>,
    strong: bool,
    emphasis: bool,
    strike: bool,
    code: bool,
    link: bool,
}

fn markdown_text_offsets(source: &str, rendered: &str) -> Option<Vec<usize>> {
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let mut output = String::new();
    let mut offsets = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        let (byte, character) = *chars.get(index)?;
        if character == '\\'
            && let Some((_, next_character)) = chars.get(index + 1).copied()
            && "\\`*_{}[]()#+-.!>~|".contains(next_character)
        {
            offsets.push(byte);
            output.push(next_character);
            index += 2;
        } else {
            offsets.push(byte);
            output.push(character);
            index += 1;
        }
    }
    if output != rendered {
        return None;
    }
    offsets.push(source.len());
    Some(offsets)
}

fn char_range_to_bytes(text: &str, range: std::ops::Range<usize>) -> Option<std::ops::Range<usize>> {
    if range.start >= range.end {
        return None;
    }
    let char_count = text.chars().count();
    if range.end > char_count {
        return None;
    }
    let byte_at = |index: usize| if index == char_count { Some(text.len()) } else { text.char_indices().nth(index).map(|(byte, _)| byte) };
    let start = byte_at(range.start)?;
    let end = byte_at(range.end)?;
    (start < end).then_some(start..end)
}

#[derive(Default)]
struct PreviewBlock {
    segments: Vec<PreviewSegment>,
    images: Vec<String>,
    video: Option<String>,
    heading: Option<pulldown_cmark::HeadingLevel>,
    rule: bool,
}

#[derive(Clone, Copy)]
enum PreviewFormat {
    Bold,
    Italic,
    Strikethrough,
    Link,
}

fn format_markdown_segment(text: &str, format: PreviewFormat, strong: bool, emphasis: bool) -> String {
    match format {
        PreviewFormat::Bold if emphasis => format!("__{text}__"),
        PreviewFormat::Bold => format!("**{text}**"),
        PreviewFormat::Italic if strong => format!("_{text}_"),
        PreviewFormat::Italic => format!("*{text}*"),
        PreviewFormat::Strikethrough => format!("~~{text}~~"),
        PreviewFormat::Link => format!("[{text}](https://example.com)"),
    }
}

fn format_preview_buffer(text: &str, selection: std::ops::Range<usize>, format: PreviewFormat, strong: bool, emphasis: bool) -> Option<String> {
    let range = char_range_to_bytes(text, selection)?;
    let before = escape_markdown_text(text.get(..range.start)?);
    let selected = escape_markdown_text(text.get(range.clone())?);
    let after = escape_markdown_text(text.get(range.end..)?);
    Some(format!("{before}{}{after}", format_markdown_segment(&selected, format, strong, emphasis)))
}

fn render_editable_markdown(ui: &mut egui::Ui, markdown: &mut String, note_id: usize, format: Option<PreviewFormat>) -> (bool, Option<String>) {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut blocks = Vec::<PreviewBlock>::new();
    let mut block = PreviewBlock::default();
    let mut strong = false;
    let mut emphasis = false;
    let mut strike = false;
    let mut link = false;
    let mut list_depth = 0usize;
    let mut changed = false;
    let mut open_url = None;
    let parser = Parser::new_ext(markdown, Options::all()).into_offset_iter();
    for (event, range) in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                if !block.segments.is_empty() || !block.images.is_empty() {
                    blocks.push(std::mem::take(&mut block));
                }
                block.heading = Some(level);
            }
            Event::End(TagEnd::Heading(_)) | Event::End(TagEnd::Paragraph) => blocks.push(std::mem::take(&mut block)),
            Event::Start(Tag::Item) => {
                block.segments.push(PreviewSegment { text: format!("{}• ", "  ".repeat(list_depth)), ..Default::default() });
            }
            Event::End(TagEnd::Item) => blocks.push(std::mem::take(&mut block)),
            Event::Start(Tag::List(_)) => list_depth = list_depth.saturating_add(1),
            Event::End(TagEnd::List(_)) => list_depth = list_depth.saturating_sub(1),
            Event::Start(Tag::Strong) => strong = true,
            Event::End(TagEnd::Strong) => strong = false,
            Event::Start(Tag::Emphasis) => emphasis = true,
            Event::End(TagEnd::Emphasis) => emphasis = false,
            Event::Start(Tag::Strikethrough) => strike = true,
            Event::End(TagEnd::Strikethrough) => strike = false,
            Event::Start(Tag::Link { .. }) => link = true,
            Event::End(TagEnd::Link) => link = false,
            Event::Text(text) => {
                let editable_range = markdown.get(range.clone()).and_then(|source| markdown_text_offsets(source, &text)).map(|_| range);
                block.segments.push(PreviewSegment { text: text.to_string(), source: editable_range, strong, emphasis, strike, code: false, link });
            }
            Event::Code(text) => {
                let editable_range = markdown.get(range.clone()).and_then(|source| markdown_text_offsets(source, &text)).map(|_| range);
                block.segments.push(PreviewSegment { text: text.to_string(), source: editable_range, strong, emphasis, strike, code: true, link });
            }
            Event::SoftBreak | Event::HardBreak => {
                if !block.segments.is_empty() {
                    blocks.push(std::mem::take(&mut block));
                }
            }
            Event::TaskListMarker(checked) => block.segments.push(PreviewSegment { text: if checked { "☑ " } else { "☐ " }.into(), ..Default::default() }),
            Event::Start(Tag::Image { dest_url, .. }) => block.images.push(dest_url.to_string()),
            Event::Html(text) | Event::InlineHtml(text) => block.video = video_source(&text),
            Event::Rule => {
                block.rule = true;
                blocks.push(std::mem::take(&mut block));
            }
            Event::End(TagEnd::TableCell) => block.segments.push(PreviewSegment { text: "  |  ".into(), ..Default::default() }),
            _ => {}
        }
    }
    if !block.segments.is_empty() || !block.images.is_empty() || block.video.is_some() || block.rule {
        blocks.push(block);
    }

    let mut replacements = Vec::<(std::ops::Range<usize>, String)>::new();
    let editing_id = Id::new(("documentation-note-preview-edit", note_id));
    let buffer_id = Id::new(("documentation-note-preview-buffer", note_id));
    let editing_segment = ui.ctx().data(|data| data.get_temp::<Id>(editing_id));
    let mut next_edit: Option<(Id, String)> = None;
    let mut finish_edit = false;
    let mut formatted = false;
    for (block_index, block) in blocks.iter().enumerate() {
        if block.rule {
            ui.separator();
            continue;
        }
        ui.horizontal_wrapped(|ui| {
            for (segment_index, segment) in block.segments.iter().enumerate() {
                if let Some(source) = segment.source.clone() {
                    let edit_id = Id::new(("documentation-note-preview-text", note_id, block_index, segment_index));
                    if format.is_some() && editing_segment == Some(edit_id) {
                        let selection =
                            egui::TextEdit::load_state(ui.ctx(), edit_id).and_then(|state| state.cursor.char_range()).map(|range| range.as_sorted_char_range());
                        let buffer = ui.ctx().data(|data| data.get_temp::<String>(buffer_id)).unwrap_or_else(|| segment.text.clone());
                        if let Some(selection) = selection.map(|range| range.start.0..range.end.0)
                            && let Some(replacement) =
                                format_preview_buffer(&buffer, selection, format.unwrap_or(PreviewFormat::Bold), segment.strong, segment.emphasis)
                        {
                            let Some(format) = format else { continue };
                            if matches!(format, PreviewFormat::Bold) && segment.strong
                                || matches!(format, PreviewFormat::Italic) && segment.emphasis
                                || matches!(format, PreviewFormat::Strikethrough) && segment.strike
                                || matches!(format, PreviewFormat::Link) && segment.link
                            {
                                continue;
                            }
                            replacements.push((source.clone(), replacement));
                            formatted = true;
                            finish_edit = true;
                        }
                        continue;
                    }
                    let size = match block.heading {
                        Some(pulldown_cmark::HeadingLevel::H1) => 23.0,
                        Some(pulldown_cmark::HeadingLevel::H2) => 20.0,
                        Some(pulldown_cmark::HeadingLevel::H3) => 17.0,
                        Some(_) => 15.0,
                        None => 14.0,
                    };
                    if editing_segment == Some(edit_id) {
                        let mut value = ui.ctx().data(|data| data.get_temp::<String>(buffer_id)).unwrap_or_else(|| segment.text.clone());
                        let width = ((value.chars().count() as f32 * size * 0.58) + 32.0).clamp(80.0, ui.available_width().max(80.0));
                        let response = ui.add_sized(
                            vec2(width, (value.lines().count().max(2) as f32 * 22.0).min(160.0)),
                            egui::TextEdit::multiline(&mut value)
                                .id(edit_id)
                                .font(if segment.code { egui::FontId::monospace(size) } else { egui::FontId::proportional(size) })
                                .desired_width(width)
                                .desired_rows(2)
                                .frame(egui::Frame::NONE),
                        );
                        if response.changed() {
                            ui.ctx().data_mut(|data| data.insert_temp(buffer_id, value.clone()));
                        }
                        if response.lost_focus() {
                            replacements.push((source, escape_markdown_text(&value)));
                            changed = true;
                            finish_edit = true;
                        }
                    } else {
                        let mut text = RichText::new(&segment.text).size(size);
                        if segment.strong {
                            text = text.strong();
                        }
                        if segment.emphasis {
                            text = text.italics();
                        }
                        if segment.strike {
                            text = text.strikethrough();
                        }
                        if segment.code {
                            text = text.family(egui::FontFamily::Monospace);
                        }
                        if segment.link {
                            text = text.color(Tokens::get(ui.ctx()).accent);
                        }
                        if ui.add(egui::Label::new(text).sense(Sense::click())).clicked() {
                            next_edit = Some((edit_id, segment.text.clone()));
                            ui.ctx().memory_mut(|memory| memory.request_focus(edit_id));
                            ui.ctx().request_repaint();
                        }
                    }
                } else {
                    ui.label(RichText::new(&segment.text).size(if block.heading.is_some() { 17.0 } else { 14.0 }));
                }
            }
        });
        for image in &block.images {
            if image.starts_with("https://") || image.starts_with("http://") || image.starts_with("file://") || image.starts_with("data:image/") {
                ui.add(egui::Image::from_uri(image).max_size(vec2(ui.available_width().min(480.0), 260.0)).maintain_aspect_ratio(true));
            }
        }
        if let Some(url) = &block.video
            && ui.link("▶ Open video").clicked()
        {
            open_url = Some(url.clone());
        }
        ui.add_space(4.0);
    }
    if finish_edit {
        ui.ctx().data_mut(|data| data.remove::<Id>(editing_id));
        ui.ctx().data_mut(|data| data.remove::<String>(buffer_id));
    }
    if let Some((edit_id, value)) = next_edit {
        ui.ctx().data_mut(|data| {
            data.insert_temp(editing_id, edit_id);
            data.insert_temp(buffer_id, value);
        });
    }
    changed |= apply_markdown_edits(markdown, replacements);
    changed |= formatted;
    (changed, open_url)
}

fn apply_markdown_edits(markdown: &mut String, mut edits: Vec<(std::ops::Range<usize>, String)>) -> bool {
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut changed = false;
    for (range, replacement) in edits {
        if range.start <= range.end && range.end <= markdown.len() && markdown.is_char_boundary(range.start) && markdown.is_char_boundary(range.end) {
            markdown.replace_range(range, &replacement);
            changed = true;
        }
    }
    changed
}

fn escape_markdown_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '\\' | '*' | '_' | '[' | ']' | '`' | '#') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn video_source(raw_html: &str) -> Option<String> {
    let lower = raw_html.to_ascii_lowercase();
    if !lower.contains("<video") && !lower.contains("<iframe") {
        return None;
    }
    for attribute in ["src=\"", "src='", "href=\""] {
        if let Some(start) = lower.find(attribute) {
            let value_start = start.checked_add(attribute.len())?;
            let quote = attribute.chars().last()?;
            let end = raw_html.get(value_start..)?.find(quote)? + value_start;
            return raw_html.get(value_start..end).map(str::to_string);
        }
    }
    None
}

fn doc_snippet(section: &str) -> String {
    let mut text = String::new();
    for line in section.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with('|') || line.starts_with("```") || line.starts_with("![") || line.starts_with("<") {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        let plain = line.trim_start_matches(['-', '*', ' ']).replace(['*', '`', '[', ']'], "");
        text.push_str(&plain);
        if text.chars().count() >= 240 {
            break;
        }
    }
    let shortened: String = text.chars().take(240).collect();
    if shortened.is_empty() { "Open this section in the PhotoCraft documentation.".into() } else { shortened }
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

pub(crate) fn tool_description(tool: Tool) -> &'static str {
    match tool {
        Tool::Brush => "Paint strokes on the active layer. Brush size, opacity, flow and pen pressure are configured in the options bar and Brush Settings.",
        Tool::Move => "Move or transform the selected layer. Use Auto-Select and transform controls in the options bar.",
        Tool::Type => "Create editable text layers. Click to create point text or drag to make a text frame.",
        Tool::Eraser => "Erase pixels on the active layer or mask. Its size and hardness use Brush Settings.",
        Tool::CloneStamp => "Paint with pixels sampled from another location. Option-click (Alt-click on Windows/Linux) to choose a source.",
        Tool::Healing => "Blend sampled pixels into the destination while adapting to nearby texture and tone.",
        Tool::SpotHealing => "Paint over small marks to remove them using nearby image content.",
        Tool::Gradient => "Fill a layer or selection with a gradient. Choose the gradient and style in the options bar.",
        Tool::Crop => "Set a crop boundary and apply it to trim the image or canvas.",
        Tool::Eyedropper => "Sample a color from the canvas and make it the foreground color.",
        Tool::Pen => "Create and edit vector paths using anchor points and Bézier handles.",
        Tool::Hand => "Pan around the canvas. Double-click to fit the document in the window.",
        _ => "Choose this tool in the toolbar to use it on the canvas. Its shortcut is shown beside the location.",
    }
}

/// Relevant documentation sections for the native Help menu search.
pub(crate) fn documentation_results(query: &str) -> Vec<DocumentationResult> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut results: Vec<_> = docs_index()
        .iter()
        .filter_map(|entry| {
            crate::palette::fuzzy_score(query, &format!("{} {} {}", entry.title, entry.location, entry.search_text)).map(|score| DocumentationResult {
                title: entry.title.clone(),
                location: entry.location.clone(),
                description: entry.description.clone(),
                url: entry.documentation_url.clone(),
                media: entry.media.clone(),
                score,
            })
        })
        .collect();
    results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
    results.truncate(6);
    results
}

fn documentation_url_for(query: &str, fallback: &str) -> String {
    docs_index()
        .iter()
        .filter_map(|entry| match &entry.action {
            Action::Documentation(url) => crate::palette::fuzzy_score(query, &entry.title).map(|score| (score, url)),
            _ => None,
        })
        .max_by_key(|(score, _)| *score)
        .filter(|(score, _)| *score > 0)
        .map(|(_, url)| url.clone())
        .unwrap_or_else(|| fallback.to_string())
}

fn entries(app: &PhotocraftApp) -> Vec<Entry> {
    let lang = crate::i18n::Lang::from_pref(&app.session.prefs().interface.language);
    let mut entries = Vec::new();
    for tool in Tool::ALL {
        let label = crate::i18n::tr(lang, tool.label()).to_string();
        let title = format!("{} Tool", label.trim_end_matches(" Tool"));
        let location = format!("Toolbar  ·  {}", if tool.key() == '\0' { "no shortcut".into() } else { tool.key().to_string() });
        entries.push(Entry {
            title: title.clone(),
            location,
            description: tool_description(tool).into(),
            search_text: format!("{title} {} toolbar tool {}", tool.label(), tool.key()),
            documentation_url: documentation_url_for(
                &title,
                "https://github.com/storytold/photocraft/blob/main/docs/architecture.md#tools-pointer-events-in-ops-and-overlays-out",
            ),
            action: Action::Tool(tool),
            media: None,
        });
    }
    for item in crate::menus::menu_items(app).into_iter().filter(|item| item.label != "---") {
        let path_en = item.path.clone();
        let path = item.path.iter().map(|part| crate::i18n::tr(lang, part).to_string()).collect::<Vec<_>>();
        let label = crate::i18n::tr_id(lang, &item.id, &item.label).to_string();
        let location = format!("{}{}", path.join(" › "), item.shortcut.as_deref().map(|s| format!("  ·  {}", crate::shortcuts::pretty(s))).unwrap_or_default());
        let description = format!("Find {label} in {}. Documentation highlights its menu location without running it.", path.join(" › "));
        entries.push(Entry {
            title: label.clone(),
            location,
            search_text: format!("{label} {} {}", item.id, item.path.join(" ")),
            documentation_url: documentation_url_for(
                &format!("{label} {}", item.path.join(" ")),
                "https://github.com/storytold/photocraft/blob/main/docs/help-search.md#what-you-can-search",
            ),
            description,
            action: Action::Menu { id: item.id, path: path_en },
            media: None,
        });
    }
    entries.extend(settings_entries());
    for (index, note) in app.documentation_notes.iter().enumerate() {
        entries.push(Entry {
            title: note.title.clone(),
            location: "Documentation › Your Notes".into(),
            description: doc_snippet(&note.markdown),
            search_text: format!("{} {} personal note tutorial markdown", note.title, note.markdown),
            documentation_url: "https://github.com/storytold/photocraft/blob/main/docs/help-search.md#your-notes".into(),
            action: Action::UserNote { index },
            media: None,
        });
    }
    // Search Menus already exposes documentation through its dedicated
    // Documentation section; mixing these rows into the menu results makes
    // the same topic appear twice.
    entries
}

fn search(app: &PhotocraftApp, query: &str) -> Vec<Hit> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<Hit> = entries(app)
        .into_iter()
        .filter_map(|entry| {
            crate::palette::fuzzy_score(query, &format!("{} {} {}", entry.title, entry.location, entry.search_text)).map(|mut score| {
                if matches!(entry.action, Action::Tool(_)) {
                    score += 20;
                }
                Hit { score, entry }
            })
        })
        .collect();
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.entry.title.cmp(&b.entry.title)));
    hits.truncate(80);
    hits
}

fn cycle(current: usize, length: usize, reverse: bool) -> usize {
    if length == 0 {
        0
    } else if reverse {
        if current == 0 { length - 1 } else { current - 1 }
    } else {
        (current + 1) % length
    }
}

fn navigate(ctx: &egui::Context, entry: &Entry, app: &mut PhotocraftApp) {
    match &entry.action {
        Action::Tool(tool) => navigate_tool(app, ctx, *tool),
        Action::Menu { id, path } => {
            let until = ctx.input(|i| i.time) + 5.0;
            ctx.data_mut(|data| {
                data.insert_temp(navigation_id(), Navigation { target: highlight_id(id), path: path.clone(), until });
            });
        }
        Action::Setting { section } => {
            crate::prefs_ui::open_preferences(app, section);
        }
        Action::UserNote { index } => {
            ctx.data_mut(|data| {
                data.insert_temp(Id::new("help-search-section"), "notes".to_string());
                data.insert_temp(Id::new("help-search-note"), *index);
                data.insert_temp(query_id(), String::new());
            });
        }
        Action::Documentation(url) => {
            crate::links::open(app, ctx, url);
        }
    }
    app.ui.help_search_open = matches!(entry.action, Action::UserNote { .. });
}

pub(crate) fn navigate_tool(app: &mut PhotocraftApp, ctx: &egui::Context, tool: Tool) {
    app.ui.panels.toolbar = true;
    app.ui.tool = tool;
    app.session.edit_prefs(|prefs| prefs.toolbar.hidden.retain(|name| name != &format!("{tool:?}")));
    let until = ctx.input(|i| i.time) + 5.0;
    ctx.data_mut(|data| {
        data.insert_temp(navigation_id(), Navigation { target: format!("tool:{tool:?}"), path: Vec::new(), until });
    });
}

fn highlight_opacity(seconds_remaining: f64) -> f32 {
    seconds_remaining.clamp(0.0, 1.0) as f32
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    let user_interacted =
        ctx.input(|input| input.pointer.any_pressed() || input.events.iter().any(|event| matches!(event, egui::Event::Key { pressed: true, .. })));
    let navigation = ctx.data(|data| data.get_temp::<Navigation>(navigation_id()));
    if navigation.as_ref().is_some_and(|nav| now >= nav.until || user_interacted) {
        ctx.data_mut(|data| data.remove::<Navigation>(navigation_id()));
    }
    let targets: VisibleTargets = ctx.data(|data| data.get_temp(targets_id()).unwrap_or_default());
    if let Some(nav) = ctx.data(|data| data.get_temp::<Navigation>(navigation_id()))
        && let Some(rect) = targets.rects.get(&nav.target)
    {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("help-search-target-highlight")));
        let rect = rect.expand(4.0);
        let t = Tokens::get(ctx);
        let seconds_remaining = nav.until - now;
        let opacity = highlight_opacity(seconds_remaining);
        painter.rect_filled(rect, CornerRadius::same(t.radius_sm as u8), t.accent.gamma_multiply(0.16 * opacity));
        painter.rect_stroke(rect, CornerRadius::same(t.radius_sm as u8), Stroke::new(3.0, t.accent.gamma_multiply(opacity)), StrokeKind::Outside);
        if opacity > 0.0 && opacity < 1.0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        } else if seconds_remaining > 1.0 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(seconds_remaining - 1.0));
        }
    }
    if !app.ui.help_search_open {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let panel_width = (screen.width() - 32.0).clamp(480.0, 900.0);
    let section_id = Id::new("help-search-section");
    let mut section = ctx.data(|data| data.get_temp::<String>(section_id).unwrap_or_else(|| "search".into()));
    let query_is_empty = ctx.data(|data| data.get_temp::<String>(query_id()).unwrap_or_default().trim().is_empty());
    let panel_height = if query_is_empty && section != "notes" { 132.0 } else { (screen.height() - 96.0).clamp(120.0, 680.0) };
    egui::Area::new(Id::new("help-search-scrim")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let response = ui.allocate_exact_size(screen.size(), Sense::click()).1;
        ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(145));
        if response.clicked() {
            app.ui.help_search_open = false;
        }
    });
    let mut selected_for_navigation: Option<Entry> = None;
    egui::Area::new(Id::new("help-search-panel"))
        .order(egui::Order::Tooltip)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(pos2(screen.center().x, screen.top() + 48.0))
        .show(ctx, |ui| {
            ui.set_width(panel_width);
            ui.set_height(panel_height);
            egui::Frame::popup(ui.style())
                .fill(t.card)
                .stroke(Stroke::new(1.0, t.card_border))
                .corner_radius(CornerRadius::same(t.radius_lg as u8))
                .shadow(egui::Shadow { offset: [0, 18], blur: 48, spread: 0, color: t.shadow })
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                    ui.set_max_width(panel_width - 28.0);
                    let panel_height = (panel_height - 128.0).max(0.0);
                    let column_width = (panel_width - 38.0) * 0.5;
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                        icons::paint(ui, r, "search", 18.0, t.accent);
                        ui.heading(tl!("Documentation"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(tl!("Close")).clicked() {
                                app.ui.help_search_open = false;
                            }
                        });
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.selectable_label(section == "search", tl!("Search")).clicked() {
                            section = "search".into();
                        }
                        if ui.selectable_label(section == "notes", tl!("Your Notes")).clicked() {
                            section = "notes".into();
                        }
                    });
                    ui.data_mut(|data| data.insert_temp(section_id, section.clone()));
                    if section == "notes" {
                        ui.add_space(12.0);
                        let notes_width = panel_width - 56.0;
                        let notes_height = (panel_height - 112.0).max(120.0);
                        ui.allocate_ui_with_layout(vec2(notes_width, notes_height), egui::Layout::top_down(egui::Align::Min), |notes_ui| {
                            show_notes(notes_ui, app, ctx, notes_height, notes_width);
                        });
                        return;
                    }
                    let mut query: String = ui.data_mut(|data| data.get_temp(query_id()).unwrap_or_default());
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .id(query_id())
                            .hint_text(tl!("Search tools, menus, settings and documentation…"))
                            .desired_width(f32::INFINITY),
                    );
                    if !field.has_focus() {
                        field.request_focus();
                    }
                    ui.add_space(8.0);
                    let hits = search(app, &query);
                    let (tab, shift_tab, enter, escape) = ctx.input(|input| {
                        (
                            input.key_pressed(egui::Key::Tab) && !input.modifiers.shift,
                            input.key_pressed(egui::Key::Tab) && input.modifiers.shift,
                            input.key_pressed(egui::Key::Enter),
                            input.key_pressed(egui::Key::Escape),
                        )
                    });
                    if tab || shift_tab || enter {
                        ctx.input_mut(|input| {
                            input.consume_key(input.modifiers, egui::Key::Tab);
                            input.consume_key(input.modifiers, egui::Key::Enter);
                        });
                    }
                    let mut selected: usize = ui.data_mut(|data| data.get_temp(selected_id()).unwrap_or(0));
                    let previous_query: String = ui.data_mut(|data| data.get_temp(previous_query_id()).unwrap_or_default());
                    if query != previous_query {
                        selected = 0;
                    }
                    if tab || shift_tab {
                        selected = cycle(selected, hits.len(), shift_tab);
                    }
                    selected = selected.min(hits.len().saturating_sub(1));
                    let mut click_selected = None;
                    if query.trim().is_empty() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(tl!("Search tools, menus, settings, and documentation.")).color(t.text_dim));
                    } else {
                        ui.horizontal_top(|ui| {
                            ui.allocate_ui_with_layout(vec2(column_width, panel_height), egui::Layout::top_down(egui::Align::Min), |ui| {
                                egui::ScrollArea::vertical().id_salt("help-search-results").max_height(panel_height).show(ui, |ui| {
                                    if query.trim().is_empty() {
                                        ui.label(RichText::new(tl!("Search for a tool, menu item, setting or topic.")).color(t.text_dim));
                                    } else if hits.is_empty() {
                                        ui.label(RichText::new(tl!("No matches. Try a shorter term.")).color(t.text_faint));
                                    }
                                    for (index, hit) in hits.iter().enumerate() {
                                        let selected_row = index == selected;
                                        let response = ui.allocate_response(vec2(column_width, 26.0), Sense::click());
                                        if selected_row || response.hovered() {
                                            ui.painter().rect_filled(response.rect, t.radius_sm, if selected_row { t.accent_soft } else { t.hover });
                                        }
                                        ui.painter().text(
                                            pos2(response.rect.left() + 8.0, response.rect.center().y),
                                            Align2::LEFT_CENTER,
                                            &hit.entry.title,
                                            theme::medium(13.0),
                                            t.text,
                                        );
                                        ui.painter().text(
                                            response.rect.right_center() - vec2(8.0, 0.0),
                                            Align2::RIGHT_CENTER,
                                            &hit.entry.location,
                                            egui::FontId::proportional(10.5),
                                            t.text_faint,
                                        );
                                        if selected_row && (tab || shift_tab) {
                                            ui.scroll_to_rect(response.rect, Some(egui::Align::Center));
                                        }
                                        if response.clicked() {
                                            click_selected = Some(index);
                                        }
                                        if response.secondary_clicked() {
                                            crate::links::open(app, ctx, &hit.entry.documentation_url);
                                            app.ui.help_search_open = false;
                                        }
                                        ui.add_space(2.0);
                                    }
                                });
                            });
                            ui.separator();
                            ui.allocate_ui_with_layout(vec2(column_width, panel_height), egui::Layout::top_down(egui::Align::Min), |ui| {
                                if let Some(hit) = hits.get(selected) {
                                    ui.heading(&hit.entry.title);
                                    ui.label(RichText::new(&hit.entry.location).color(t.text_faint));
                                    ui.add_space(10.0);
                                    ui.label(&hit.entry.description);
                                    if let Some(media) = &hit.entry.media {
                                        ui.add_space(8.0);
                                        ui.add(egui::Image::from_bytes(media.uri, media.bytes).max_size(vec2(405.0, 190.0)).maintain_aspect_ratio(true));
                                        ui.label(RichText::new(media.caption).small().color(t.text_faint));
                                    }
                                    ui.add_space(12.0);
                                    if ui.link(tl!("Open documentation ↗")).clicked() {
                                        crate::links::open(app, ctx, &hit.entry.documentation_url);
                                    }
                                    match &hit.entry.action {
                                        Action::Documentation(_) => {}
                                        _ => {
                                            if ui.button(tl!("Show in app")).clicked() {
                                                click_selected = Some(selected);
                                                selected_for_navigation = Some(hit.entry.clone());
                                            }
                                        }
                                    }
                                }
                            });
                        });
                    }
                    if let Some(index) = click_selected {
                        selected = index.min(hits.len().saturating_sub(1));
                    }
                    if enter && let Some(hit) = hits.get(selected) {
                        selected_for_navigation = Some(hit.entry.clone());
                    }
                    if escape {
                        app.ui.help_search_open = false;
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new(tl!("Tab / Shift+Tab cycle matches  ·  Enter shows the selected item  ·  Esc closes")).small().color(t.text_faint));
                    ui.data_mut(|data| {
                        data.insert_temp(query_id(), query.clone());
                        data.insert_temp(selected_id(), selected);
                        data.insert_temp(previous_query_id(), query);
                    });
                });
        });
    if let Some(entry) = selected_for_navigation {
        navigate(ctx, &entry, app);
    }
}

fn show_notes(ui: &mut egui::Ui, app: &mut PhotocraftApp, ctx: &egui::Context, height: f32, width: f32) {
    let t = Tokens::get(ctx);
    let mut changed = false;
    let mut delete = false;
    let mut open_url = None;
    let mut selected = ctx.data(|data| data.get_temp::<usize>(Id::new("help-search-note")).unwrap_or(0));
    let list_width = 205.0_f32.min((width * 0.32).max(150.0));
    let editor_width = (width - list_width - 18.0).max(220.0);
    ui.horizontal_top(|ui| {
        let sidebar_height = height.max(120.0);
        ui.allocate_ui_with_layout(vec2(list_width, sidebar_height), egui::Layout::top_down(egui::Align::Min), |ui| {
            let actions_height = 108.0_f32.min(sidebar_height);
            let list_height = (sidebar_height - actions_height - 8.0).max(0.0);
            ui.allocate_ui(vec2(list_width, list_height), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("documentation-notes-list")
                    .max_height(list_height)
                    .show_rows(ui, 34.0, app.documentation_notes.len(), |ui, rows| {
                        ui.set_width(list_width - 4.0);
                        for index in rows {
                            let title = app.documentation_notes.get(index).map(|note| note.title.as_str()).unwrap_or("");
                            if ui.add_sized(vec2(list_width - 8.0, 32.0), egui::Button::selectable(selected == index, title)).clicked() {
                                selected = index;
                            }
                        }
                    });
                if app.documentation_notes.is_empty() {
                    ui.label(RichText::new(tl!("Add personal tutorials and reference notes here.")).color(t.text_dim));
                }
            });
            ui.allocate_ui(vec2(list_width, actions_height), |ui| {
                if ui.add_sized(vec2(list_width - 8.0, 28.0), egui::Button::new(tl!("Export notes…"))).clicked() {
                    let result = app.pick_save("PhotoCraft-Notes.json", |app, path| {
                        let bytes = export_notes(&app.documentation_notes)?;
                        let write = app.services.write.as_mut().ok_or("Note export isn't available on this platform")?;
                        write(&path, &bytes)?;
                        app.ui.status = format!("Exported notes to {path}");
                        app.ui.status_error = false;
                        Ok(serde_json::Value::Null)
                    });
                    if let Err(error) = result {
                        app.ui.status = format!("Couldn't export notes: {error}");
                        app.ui.status_error = true;
                    }
                }
                ui.add_space(4.0);
                if ui.add_sized(vec2(list_width - 8.0, 28.0), egui::Button::new(tl!("Import notes…"))).clicked() {
                    let selection_ctx = ctx.clone();
                    let result = app.pick_file_bytes(move |app, name, bytes| {
                        let range = append_imported_notes(&mut app.documentation_notes, &bytes).map_err(|error| format!("{name}: {error}"))?;
                        let imported = range.len();
                        if imported > 0 {
                            selection_ctx.data_mut(|data| data.insert_temp(Id::new("help-search-note"), range.start));
                        }
                        save_notes(app);
                        app.ui.status = format!("Imported {imported} note(s); existing notes were kept");
                        app.ui.status_error = false;
                        Ok(serde_json::Value::Null)
                    });
                    if let Err(error) = result {
                        app.ui.status = format!("Couldn't import notes: {error}");
                        app.ui.status_error = true;
                    }
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button(tl!("New note")).clicked() && app.documentation_notes.len() < USER_NOTE_LIMIT {
                        app.documentation_notes.push(UserNote {
                            title: "New tutorial".into(),
                            markdown: "# New tutorial\n\nWrite your notes in **Markdown** here.\n\nAdd an image with `![alt text](https://your-server/image.gif)` or link to a video with `[Watch video](https://your-server/tutorial.mp4)`.".into(),
                        });
                        selected = app.documentation_notes.len().saturating_sub(1);
                        changed = true;
                    }
                    if !app.documentation_notes.is_empty() && ui.button(tl!("Delete note")).clicked() {
                        delete = true;
                    }
                });
                if delete {
                    if selected < app.documentation_notes.len() {
                        app.documentation_notes.remove(selected);
                    }
                    selected = selected.min(app.documentation_notes.len().saturating_sub(1));
                    changed = true;
                }
            });
        });
        ui.separator();
        ui.allocate_ui_with_layout(vec2(editor_width, height.max(120.0)), egui::Layout::top_down(egui::Align::Min), |ui| {
            if let Some(note) = app.documentation_notes.get_mut(selected) {
                ui.add_sized(vec2(editor_width - 8.0, 28.0), egui::TextEdit::singleline(&mut note.title).hint_text(tl!("Note title")));
                let mode_id = Id::new("documentation-note-editor-mode");
                let mut code_mode = ui.ctx().data(|data| data.get_temp::<bool>(mode_id).unwrap_or(false));
                ui.horizontal(|ui| {
                    if ui.selectable_label(!code_mode, tl!("Preview")).clicked() {
                        code_mode = false;
                    }
                    if ui.selectable_label(code_mode, tl!("Markdown Source")).clicked() {
                        code_mode = true;
                    }
                });
                ui.ctx().data_mut(|data| data.insert_temp(mode_id, code_mode));
                let mut format = None;
                if !code_mode {
                    let edit_id = Id::new(("documentation-note-preview-edit", selected));
                    let active_segment = ui.ctx().data(|data| data.get_temp::<Id>(edit_id));
                    let has_selection = active_segment
                        .and_then(|segment_id| egui::TextEdit::load_state(ui.ctx(), segment_id))
                        .and_then(|state| state.cursor.char_range())
                        .is_some_and(|range| range.as_sorted_char_range().start < range.as_sorted_char_range().end);
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(has_selection, egui::Button::new(RichText::new("B").strong())).on_hover_text(tl!("Bold selected text")).clicked() {
                            format = Some(PreviewFormat::Bold);
                        }
                        if ui.add_enabled(has_selection, egui::Button::new(RichText::new("I").italics())).on_hover_text(tl!("Italic selected text")).clicked() {
                            format = Some(PreviewFormat::Italic);
                        }
                        if ui.add_enabled(has_selection, egui::Button::new(RichText::new("S").strikethrough())).on_hover_text(tl!("Strikethrough selected text")).clicked() {
                            format = Some(PreviewFormat::Strikethrough);
                        }
                        if ui.add_enabled(has_selection, egui::Button::new(tl!("Link"))).on_hover_text(tl!("Link selected text")).clicked() {
                            format = Some(PreviewFormat::Link);
                        }
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tl!("Insert image/GIF link")).clicked() {
                        note.markdown.push_str("\n\n![Description](https://your-server/image.gif)\n");
                        changed = true;
                    }
                    if ui.button(tl!("Insert video link")).clicked() {
                        note.markdown.push_str("\n\n[Video walkthrough](https://your-server/video.mp4)\n");
                        changed = true;
                    }
                });
                let content_height = (height - 90.0).max(100.0);
                if code_mode {
                    let body = ui.add_sized(
                        vec2(editor_width - 8.0, content_height),
                        egui::TextEdit::multiline(&mut note.markdown)
                            .font(egui::FontId::monospace(13.0))
                            .hint_text(tl!("Markdown: headings, links, images, GIFs, and video links…")),
                    );
                    changed |= body.changed();
                } else {
                    egui::ScrollArea::vertical().id_salt(("documentation-note-preview", selected)).max_height(content_height).show(ui, |ui| {
                        ui.set_width(editor_width - 8.0);
                        let (preview_changed, clicked_url) = render_editable_markdown(ui, &mut note.markdown, selected, format);
                        changed |= preview_changed;
                        open_url = clicked_url;
                    });
                }
                if note.markdown.len() > USER_NOTE_BYTES_LIMIT {
                    ui.colored_label(t.danger, tl!("Note is too large to save (256 KB maximum)."));
                }
            } else {
                ui.label(RichText::new(tl!("Select a note or create a new one.")).color(t.text_dim));
            }
        });
    });
    if let Some(url) = open_url {
        crate::links::open(app, ctx, &url);
    }
    ctx.data_mut(|data| data.insert_temp(Id::new("help-search-note"), selected));
    if changed {
        let within_limits = app.documentation_notes.len() <= USER_NOTE_LIMIT
            && app.documentation_notes.iter().all(|note| note.title.chars().count() <= 160 && note.markdown.len() <= USER_NOTE_BYTES_LIMIT);
        if within_limits {
            save_notes(app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docs_heading_anchor_slug_is_stable() {
        assert_eq!(slug("Tools & Toolbar — Brush"), "tools-toolbar-brush");
    }

    #[test]
    fn brush_is_searchable_as_a_toolbar_target() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let hits = search(&app, "Brush");
        assert!(hits.iter().any(|hit| matches!(hit.entry.action, Action::Tool(Tool::Brush))));
        assert!(hits.iter().any(|hit| hit.entry.title.contains("Brush")));
    }

    #[test]
    fn index_covers_every_toolbar_tool_and_live_menu_item() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let indexed = entries(&app);
        let tools = indexed.iter().filter(|entry| matches!(entry.action, Action::Tool(_))).count();
        let menus = indexed.iter().filter(|entry| matches!(entry.action, Action::Menu { .. })).count();
        let live_menu_items = crate::menus::menu_items(&app).iter().filter(|item| item.label != "---").count();
        assert_eq!(tools, Tool::ALL.len());
        assert_eq!(menus, live_menu_items);
    }

    #[test]
    fn navigating_to_a_hidden_toolbar_tool_reveals_it_for_highlighting() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.edit_prefs(|prefs| prefs.toolbar.hidden = vec!["Brush".into()]);
        let brush = entries(&app).into_iter().find(|entry| matches!(entry.action, Action::Tool(Tool::Brush))).expect("Brush is indexed");
        let ctx = egui::Context::default();
        navigate(&ctx, &brush, &mut app);
        assert!(app.ui.panels.toolbar);
        assert_eq!(app.ui.tool, Tool::Brush);
        assert!(!app.session.prefs().toolbar.hidden.iter().any(|name| name == "Brush"));
    }

    #[test]
    fn docs_index_attaches_local_visuals_for_complex_topics() {
        let effects = docs_index().iter().find(|entry| entry.title == "Layer effects").expect("help guide is indexed");
        let media = effects.media.as_ref().expect("complex topic has a preview");
        assert!(media.uri.ends_with(".jpg"));
        assert!(!media.bytes.is_empty());
    }

    #[test]
    fn documentation_results_include_descriptions_and_media() {
        let results = documentation_results("layer effects");
        let effects = results.iter().find(|result| result.title == "Layer effects").expect("matching documentation is returned");
        assert!(!effects.description.is_empty());
        assert!(effects.url.contains("docs/help-search.md#layer-effects"));
        assert!(effects.media.as_ref().is_some_and(|media| !media.bytes.is_empty()));
        assert!(documentation_results("").is_empty());
    }

    #[test]
    fn combined_search_omits_duplicate_documentation_rows() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(search(&app, "architecture").iter().all(|hit| !matches!(hit.entry.action, Action::Documentation(_))));
        assert!(!documentation_results("architecture").is_empty());
    }

    #[test]
    fn settings_index_covers_visible_preferences_and_searches_fields() {
        let indexed = settings_entries();
        assert!(indexed.iter().any(|entry| entry.title == "Show tooltips"));
        assert!(!indexed.iter().any(|entry| entry.search_text.contains("general.colorPicker")));
        let results = setting_results("UI scale");
        assert!(results.iter().any(|result| result.title == "UI scale"));
        let theme_choice = photocraft_engine::prefs::choices("interface.theme").and_then(|choices| choices.first()).copied();
        if let Some(choice) = theme_choice {
            assert!(setting_results(choice).iter().any(|result| result.title == "Theme"));
        }
    }

    #[test]
    fn personal_notes_are_searchable_from_the_native_help_menu() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.documentation_notes.push(UserNote { title: "My layer walkthrough".into(), markdown: "Use a mask before painting.".into() });
        let results = note_results(&app, "layer walkthrough");
        assert_eq!(results.first().map(|result| result.title.as_str()), Some("My layer walkthrough"));
        assert_eq!(results.first().map(|result| result.url.as_str()), Some("photocraft-note://0"));
    }

    #[test]
    fn commonmark_parser_accepts_tables_tasks_links_images_and_code() {
        use pulldown_cmark::{Event, Options, Parser, Tag};
        let markdown = "# Guide\n\n| A | B |\n| - | - |\n| 1 | 2 |\n\n- [x] done\n\n[site](https://example.test) ![image](https://example.test/a.gif)\n\n```rust\nlet x = 1;\n```";
        let events = Parser::new_ext(markdown, Options::all()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(event, Event::Start(Tag::Table(_)))));
        assert!(events.iter().any(|event| matches!(event, Event::TaskListMarker(true))));
        assert!(events.iter().any(|event| matches!(event, Event::Start(Tag::Link { .. }))));
        assert!(events.iter().any(|event| matches!(event, Event::Start(Tag::Image { .. }))));
        assert!(events.iter().any(|event| matches!(event, Event::Start(Tag::CodeBlock(_)))));
    }

    #[test]
    fn preview_edits_keep_markdown_formatting_and_link_targets() {
        let mut markdown = "**bold** and [a link](https://example.test)".to_string();
        assert!(apply_markdown_edits(&mut markdown, vec![(2..6, "strong".into()), (14..20, "new link".into())]));
        assert_eq!(markdown, "**strong** and [new link](https://example.test)");
        assert_eq!(escape_markdown_text("plain * text"), "plain \\* text");
        assert_eq!(format_markdown_segment("selected", PreviewFormat::Bold, false, false), "**selected**");
        assert_eq!(format_markdown_segment("selected", PreviewFormat::Link, false, false), "[selected](https://example.com)");
        let nested = format!("**{} rest**", format_markdown_segment("selected", PreviewFormat::Italic, true, false));
        let events = pulldown_cmark::Parser::new_ext(&nested, pulldown_cmark::Options::all()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(event, pulldown_cmark::Event::Start(pulldown_cmark::Tag::Strong))));
        assert!(events.iter().any(|event| matches!(event, pulldown_cmark::Event::Start(pulldown_cmark::Tag::Emphasis))));
        assert!(markdown_text_offsets(r"A \*marked\* word", "A *marked* word").is_some());
        let raw = r"a \*b\*";
        let rendered = "a *b*";
        assert!(markdown_text_offsets(raw, rendered).is_some());
        assert_eq!(format_preview_buffer("A selected phrase", 2..10, PreviewFormat::Bold, false, false).as_deref(), Some("A **selected** phrase"));
    }

    #[test]
    fn importing_notes_appends_without_replacing_existing_notes() {
        let mut existing = vec![UserNote { title: "Keep this".into(), markdown: "Existing".into() }];
        let incoming = serde_json::to_vec(&vec![UserNote { title: "Imported".into(), markdown: "New note".into() }]).unwrap();
        let added = append_imported_notes(&mut existing, &incoming).unwrap();
        assert_eq!(added, 1..2);
        assert_eq!(existing.iter().map(|note| note.title.as_str()).collect::<Vec<_>>(), vec!["Keep this", "Imported"]);
    }

    #[test]
    fn importing_notes_rejects_limits_without_changing_existing_notes() {
        let mut existing = vec![UserNote { title: "Keep this".into(), markdown: "Existing".into() }];
        let too_many = vec![UserNote::default(); USER_NOTE_LIMIT];
        let incoming = serde_json::to_vec(&too_many).unwrap();
        assert!(append_imported_notes(&mut existing, &incoming).is_err());
        assert_eq!(existing.len(), 1);
        assert!(append_imported_notes(&mut existing, b"not JSON").is_err());
    }

    #[test]
    fn user_notes_load_and_save_through_the_documentation_service() {
        use std::sync::{Arc, Mutex};
        let persisted = Arc::new(Mutex::new(Some(r#"[{"title":"My guide","markdown":"![preview](https://example.test/a.gif)"}]"#.to_string())));
        let read = persisted.clone();
        let write = persisted.clone();
        let services = crate::Services {
            load_documentation: Some(Box::new(move || read.lock().unwrap_or_else(|e| e.into_inner()).clone())),
            save_documentation: Some(Box::new(move |text| {
                *write.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        assert_eq!(app.documentation_notes.first().map(|note| note.title.as_str()), Some("My guide"));
        app.documentation_notes.push(UserNote { title: "New note".into(), markdown: "[video](https://example.test/video.mp4)".into() });
        save_notes(&mut app);
        let saved = persisted.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert_eq!(load_notes(saved.as_deref()).len(), 2);
    }

    #[test]
    fn notes_reject_malformed_or_oversized_storage() {
        assert!(load_notes(Some("not json")).is_empty());
        let huge_note = serde_json::to_string(&vec![UserNote { title: "large".into(), markdown: "x".repeat(USER_NOTE_BYTES_LIMIT + 1) }]).unwrap();
        assert!(load_notes(Some(&huge_note)).is_empty());
    }

    #[test]
    fn markdown_video_tags_accept_a_source_url() {
        assert_eq!(video_source(r#"<video controls src="https://example.test/clip.mp4"></video>"#).as_deref(), Some("https://example.test/clip.mp4"));
        assert_eq!(video_source("<p>not media</p>"), None);
    }

    #[test]
    fn tab_cycles_all_matches_and_wraps_in_both_directions() {
        assert_eq!(cycle(0, 3, false), 1);
        assert_eq!(cycle(2, 3, false), 0);
        assert_eq!(cycle(0, 3, true), 2);
        assert_eq!(cycle(1, 3, true), 0);
        assert_eq!(cycle(0, 0, false), 0);
    }

    #[test]
    fn target_highlight_fades_during_its_last_second() {
        assert_eq!(highlight_opacity(2.0), 1.0);
        assert_eq!(highlight_opacity(0.5), 0.5);
        assert_eq!(highlight_opacity(0.0), 0.0);
    }

    #[test]
    fn help_menu_command_opens_the_search_overlay() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(crate::menus::invoke(&mut app, &egui::Context::default(), "help.search", serde_json::Value::Null).is_ok());
        assert!(app.ui.help_search_open);
    }
}
