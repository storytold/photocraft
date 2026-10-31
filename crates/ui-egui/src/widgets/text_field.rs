use super::panel::surface;
use super::{Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, theme, vec2};

/// The most characters a [`text_field`] holds unless its caller sets a limit: far beyond any
/// search, name or hex value, yet a pasted multi-gigabyte string is cut at the field instead of
/// being laid out every frame (#2150).
pub const TEXT_FIELD_LIMIT: usize = 1024;

/// The most characters one typed, pasted or IME-committed text event carries into the app
/// ([`cap_text_events`]): a megabyte of text, more than any field or text layer is pasted.
pub const INPUT_EVENT_LIMIT: usize = 1 << 20;

/// Cuts every text, paste and IME event to [`INPUT_EVENT_LIMIT`] characters (at a character
/// boundary) before any widget sees it, so no text field -- whatever its own limit -- has to
/// lay out an 8 GB paste (#2150).
pub fn cap_text_events(events: &mut [egui::Event]) {
    fn cut(text: &mut String) -> bool {
        let Some((end, _)) = text.char_indices().nth(INPUT_EVENT_LIMIT) else { return false };
        text.truncate(end);
        true
    }
    for event in events {
        match event {
            egui::Event::Text(text) | egui::Event::Paste(text) | egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                cut(text);
            }
            // The highlighted range indexes the old text; past the cut it would point nowhere.
            egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) => {
                let truncated = cut(text);
                if truncated && active_range_chars.as_ref().is_some_and(|r| r.end > INPUT_EVENT_LIMIT) {
                    *active_range_chars = None;
                }
            }
            _ => {}
        }
    }
}

/// Height of a [`text_field`]'s box, matching a [`value_field`].
pub const TEXT_FIELD_H: f32 = 24.0;

/// Inner padding of a [`text_field`]'s box: 6 pt either side, 2 pt above and below.
const TEXT_FIELD_PAD: Vec2 = vec2(6.0, 2.0);

/// An owned right-to-left layouter for a single-line edit: the galley's base direction is pinned
/// [`egui::epaint::text::TextDirection::RightToLeft`], so Arabic or Hebrew content shapes from
/// the reading start whatever script the text begins with (a Latin- or digit-first value would
/// otherwise infer a left-to-right paragraph). The line anchors at its end (`halign = Max`),
/// which is what a [`egui::Align2::RIGHT_CENTER`] edit hands egui's default layouter; line height
/// and language match the default too, so only the direction changes.
pub fn rtl_singleline_layouter(
    font: egui::FontId,
) -> impl FnMut(&egui::Ui, &dyn egui::TextBuffer, f32) -> std::sync::Arc<egui::Galley> {
    move |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.as_str().to_owned(), font.clone(), ui.visuals().text_color());
        job.direction = egui::epaint::text::TextDirection::RightToLeft;
        job.halign = egui::Align::Max;
        job.language = ui.language();
        job.keep_trailing_whitespace = true;
        let line_height = ui.fonts_mut(|f| f.row_height(&font)) + ui.spacing().extra_text_line_spacing;
        for section in &mut job.sections {
            section.format.line_height = Some(line_height);
        }
        ui.fonts_mut(|f| f.layout_job(job))
    }
}

/// A raw single-line edit for inputs that do not go through [`text_field`]: in a right-to-left
/// UI it aligns at the reading start and installs `layouter` (build it with
/// [`rtl_singleline_layouter`]) so the text's base direction is right-to-left as well. The
/// layouter must outlive the returned editor, hence the parameter — the same shape as the new
/// document dialog's name field. In a left-to-right UI the plain edit comes back unchanged,
/// pixel-identical.
pub fn rtl_singleline<'t>(
    text: &'t mut String,
    layouter: &'t mut dyn FnMut(&egui::Ui, &dyn egui::TextBuffer, f32) -> std::sync::Arc<egui::Galley>,
) -> egui::TextEdit<'t> {
    let edit = egui::TextEdit::singleline(text);
    if !crate::direction::is_rtl() {
        return edit;
    }
    edit.align(egui::Align2::RIGHT_CENTER).layouter(layouter)
}

/// A single-line edit in the body font, `width` wide, `hint` shown while empty; in a right-to-left
/// UI the text reads from the reading start via [`rtl_singleline`]. Left-to-right the plain edit
/// comes back unchanged, pixel-identical.
pub fn rtl_singleline_edit(ui: &mut Ui, text: &mut String, width: f32, hint: &str) -> Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut layouter = rtl_singleline_layouter(font);
    ui.add(rtl_singleline(text, &mut layouter).desired_width(width).hint_text(hint.to_owned()))
}

/// [`rtl_singleline`] with `configure` applied to the editor (an accessible name, say).
pub fn rtl_singleline_add(ui: &mut Ui, text: &mut String, configure: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>) -> Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut layouter = rtl_singleline_layouter(font);
    ui.add(configure(rtl_singleline(text, &mut layouter)))
}

/// An owned right-to-left layouter for a multiline edit: the galley's base direction is pinned
/// right-to-left and every line anchors at its end, wrapping at the given width.
pub fn rtl_multiline_layouter(font: egui::FontId) -> impl FnMut(&egui::Ui, &dyn egui::TextBuffer, f32) -> std::sync::Arc<egui::Galley> {
    move |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut job = egui::text::LayoutJob::simple(text.as_str().to_owned(), font.clone(), ui.visuals().text_color(), wrap_width);
        job.direction = egui::epaint::text::TextDirection::RightToLeft;
        job.halign = egui::Align::Max;
        job.keep_trailing_whitespace = true;
        ui.fonts_mut(|f| f.layout_job(job))
    }
}

/// A multiline edit `rows` rows tall and `width` wide; in a right-to-left UI it aligns to the
/// reading end with the base direction pinned by [`rtl_multiline_layouter`]. Left-to-right the
/// plain edit comes back unchanged, pixel-identical.
pub fn rtl_multiline_edit(ui: &mut Ui, text: &mut String, rows: usize, width: f32) -> Response {
    let edit = egui::TextEdit::multiline(text).desired_rows(rows).desired_width(width);
    if !crate::direction::is_rtl() {
        return ui.add(edit);
    }
    let mut layouter = rtl_multiline_layouter(egui::FontId::default());
    ui.add(edit.align(egui::Align2::RIGHT_TOP).layouter(&mut layouter))
}

/// A single-line text field in PhotoCraft's shared field style (#3789): the same box as a
/// [`value_field`] -- the theme's field fill and `field_border`, rounded by `radius_sm` -- with a
/// consistent inner padding and no focus outline. egui's default white focus ring (its
/// `selection.stroke`) is what made the Preferences search field look different from the Color
/// Picker's hex field, so every plain, non-numeric input (search, hex, name, filter) is drawn
/// through here to look and behave the same. `id` carries the field's text and focus across
/// frames; `hint` shows while it is empty; `mono` picks the monospaced face for hex and other
/// code-like values.
pub fn text_field(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, width: f32, mono: bool) -> Response {
    text_field_ex(ui, id, text, hint, width, mono, None)
}

/// [`text_field`] limited to `char_limit` characters (a hex field's six digits, say), or to
/// [`TEXT_FIELD_LIMIT`] without one.
pub fn text_field_ex(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, width: f32, mono: bool, char_limit: Option<usize>) -> Response {
    text_field_in(ui, id, text, hint, [width, 0.0], mono, char_limit).0
}

/// A plain text field reserving space at its right for an embedded popup arrow.
/// Returns the editor and the full field rectangle; the arrow gets its own interaction id.
pub fn text_field_with_trailing(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, width: f32, trailing: f32) -> (Response, Rect) {
    text_field_in(ui, id, text, hint, [width, trailing], false, None)
}

fn text_field_in(
    ui: &mut Ui,
    id: egui::Id,
    text: &mut String,
    hint: &str,
    [width, trailing]: [f32; 2],
    mono: bool,
    char_limit: Option<usize>,
) -> (Response, Rect) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(16.0), TEXT_FIELD_H), Sense::hover());
    surface(ui, rect, t.field, false);
    if !t.bevel {
        ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    }
    let font = if mono { theme::mono(12.0) } else { egui::FontId::proportional(12.5) };
    let mut inner = rect.shrink2(TEXT_FIELD_PAD);
    inner.max.x = (inner.max.x - trailing).max(inner.min.x + 1.0);
    let limit = char_limit.unwrap_or(TEXT_FIELD_LIMIT);
    // A value already past the limit (restored from a state file, set by automation) is shown and
    // edited as its first `limit` characters, so it is never laid out whole; the caller's string
    // changes only when that copy is edited.
    let mut bounded = text.char_indices().nth(limit).map(|(end, _)| text[..end].to_string());
    let shown = match bounded.as_mut() {
        Some(b) => b,
        None => &mut *text,
    };
    // Hint and value align at the field's reading start — the right edge in a right-to-left UI;
    // monospaced hex and other code-like input stays left-to-right in both directions. Under
    // RTL the edit also pins the galley's base direction right-to-left, so Arabic shapes from
    // the reading start instead of inferring left-to-right from its first characters.
    let rtl = !mono && crate::direction::is_rtl();
    let align = if rtl { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER };
    let mut rtl_layouter = rtl.then(|| rtl_singleline_layouter(font.clone()));
    // TextEdit draws its text at the top of its rect; fill the box and centre the line in it, so
    // the text sits on the field's middle like a dropdown's or value field's.
    let mut edit = egui::TextEdit::singleline(shown)
        .id(id)
        .hint_text(hint.to_owned())
        .frame(egui::Frame::NONE)
        .font(font)
        .min_size(inner.size())
        .align(align)
        .char_limit(limit);
    if let Some(layouter) = rtl_layouter.as_mut() {
        edit = edit.layouter(layouter);
    }
    // A child Ui keeps the field's selection colour local: `Visuals::selection` also fills selected
    // list rows and slider trails, which must keep the theme's.
    let layout = egui::Layout::centered_and_justified(egui::Direction::TopDown);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(layout));
    // The theme's 24 pt interact height would make the edit taller than the box's inside.
    child.spacing_mut().interact_size.y = inner.height();
    child.style_mut().visuals.selection.bg_fill = t.field_selection();
    let response = child.add(edit);
    if let Some(b) = bounded
        && response.changed()
    {
        *text = b;
    }
    (response, rect)
}

pub(super) fn first_field_id() -> egui::Id {
    egui::Id::unique("pc-focus-first-field")
}

/// While `on`, the next number field drawn takes the keyboard focus with its text selected (a
/// dialog opening on its first value, as in Photoshop). Turn it off once the body is laid out.
pub fn focus_first_field(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| {
        if on {
            d.insert_temp(first_field_id(), true);
        } else {
            d.remove::<bool>(first_field_id());
        }
    });
}

#[cfg(test)]
mod tests;
