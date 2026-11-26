//! The edited type layer as the framework text editor's host.

use std::ops::Range;

use craft::behavior::kurbo::Point;
use craft::text::ui::editor::{CaretLine, EditKind, HostLine, HostRun, StyleKey, TextEditorHost, TextSelection};
use photocraft_doc::LayerId;
use photocraft_text::warp::{Placement, Warp};

use super::editing::{kern_pair, step_char_metric, toggle_char_style, toggle_hyphenate};
use super::layout::{doc_point, hit_at, is_vertical, layout, path_warp};
use super::{Affine, Arc, PhotocraftApp, TextLayout, byte_of, insertion_style, text_layer};

/// A type layer's text, layout and formatting for one editing session. Geometry is in document units.
pub(crate) struct LayerHost<'a> {
    app: &'a mut PhotocraftApp,
    layer: LayerId,
}

impl<'a> LayerHost<'a> {
    /// The host of `layer` in the active document.
    pub(crate) fn new(app: &'a mut PhotocraftApp, layer: LayerId) -> Self {
        Self { app, layer }
    }

    fn laid_out(&mut self) -> Option<Laid> {
        let (l, aff, text) = layout(self.app, self.layer)?;
        let warp = path_warp(self.app, self.layer, &l);
        Some(Laid { l, aff, text, warp })
    }
}

/// The layer's layout with what maps its line space to the document.
struct Laid {
    l: Arc<TextLayout>,
    aff: Affine,
    text: String,
    warp: Option<Warp>,
}

impl Laid {
    /// A line-space point in document units.
    fn at(&self, x: f32, y: f32) -> Point {
        let p = doc_point(&self.l, &self.aff, self.warp.as_ref(), x, y);
        Point::new(p.x, p.y)
    }
}

/// The chars before byte `byte` of `text`.
fn chars_before(text: &str, byte: usize) -> usize {
    text.char_indices().take_while(|(i, _)| *i < byte).count()
}

/// A line's text without its break, and its chars in `text`.
fn line_chars<'t>(text: &'t str, bytes: &Range<usize>) -> (&'t str, Range<usize>) {
    let line = text.get(bytes.clone()).unwrap_or("").trim_end_matches(['\n', '\r']);
    let start = chars_before(text, bytes.start);
    (line, start..start + line.chars().count())
}

impl TextEditorHost for LayerHost<'_> {
    fn text(&self) -> &str {
        self.app.session.active().and_then(|st| text_layer(&st.doc, self.layer)).map_or("", |t| t.text.as_str())
    }

    fn replace(&mut self, range: Range<usize>, text: &str, kind: EditKind) -> Option<usize> {
        // A free transform in progress commits before the text changes under it.
        crate::type_transform::finish(self.app);
        // The engine inserts the whole text, its line breaks as `\n`.
        insertion_style::replace(self.app, range.start, range.end, text, kind).then(|| insertion_style::stored_chars(text))
    }

    fn hit(&mut self, at: Point) -> (usize, usize) {
        hit_at(self.app, self.layer, at.x, at.y)
    }

    fn caret(&mut self, offset: usize, line: Option<usize>) -> Option<CaretLine> {
        let g = self.laid_out()?;
        let byte = byte_of(&g.text, offset);
        let (x, top, bot) = if g.l.lines.is_empty() {
            (0.0, -(12.0 * g.l.px_per_pt.max(1.0)), 3.0)
        } else if let Some(ln) = line.and_then(|i| g.l.lines.get(i)).filter(|ln| byte == ln.range.end) {
            // At a soft wrap the offset ends this line and starts the next: the caller named this one.
            (ln.x1, ln.baseline - ln.ascent, ln.baseline + ln.descent)
        } else {
            g.l.caret(byte)
        };
        Some(CaretLine::new(g.at(x, top), g.at(x, bot)))
    }

    fn selection_quads(&mut self, range: Range<usize>) -> Vec<[Point; 4]> {
        let Some(g) = self.laid_out() else { return Vec::new() };
        let (a, b) = (byte_of(&g.text, range.start), byte_of(&g.text, range.end));
        let mut out = Vec::new();
        if a >= b {
            return out;
        }
        for (li, ln) in g.l.lines.iter().enumerate() {
            let xs: Vec<(f32, f32)> =
                g.l.clusters.iter().filter(|c| c.line == li && c.range.start >= a && c.range.end <= b).map(|c| (c.x, c.x + c.advance)).collect();
            let (mut x0, mut x1) = xs.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (p, q)| (lo.min(*p), hi.max(*q)));
            // A selected line break shows as a small sliver past the line end.
            if b > ln.range.end && a <= ln.range.end {
                x1 = x1.max(ln.x1 + (ln.ascent + ln.descent) * 0.25);
                x0 = x0.min(ln.x1);
            }
            let (top, bot) = (ln.baseline - ln.ascent, ln.baseline + ln.descent);
            let quad = |p: f32, q: f32| [g.at(p, top), g.at(q, top), g.at(q, bot), g.at(p, bot)];
            if let Some(w) = &g.warp {
                // On a path each glyph turns on its own; glyphs past the path's end are not drawn, so neither is their highlight.
                for (p, q) in xs.into_iter().filter(|&(p, q)| w.place_glyph(f64::from(p + q) / 2.0) != Placement::Hidden) {
                    out.push(quad(p, q));
                }
            } else if x0 < x1 {
                out.push(quad(x0, x1));
            }
        }
        out
    }

    fn underline_segments(&mut self, range: Range<usize>) -> Vec<[Point; 2]> {
        let Some(g) = self.laid_out() else { return Vec::new() };
        let (a, b) = (byte_of(&g.text, range.start), byte_of(&g.text, range.end));
        let mut out = Vec::new();
        for (li, ln) in g.l.lines.iter().enumerate() {
            let (x0, x1) =
                g.l.clusters
                    .iter()
                    .filter(|c| c.line == li && c.range.start >= a && c.range.end <= b)
                    .fold((f32::MAX, f32::MIN), |(lo, hi), c| (lo.min(c.x), hi.max(c.x + c.advance)));
            if x0 >= x1 {
                continue;
            }
            let y = ln.baseline + ln.descent * 0.5;
            // On a path the run is cut into short pieces so the line bends with it.
            let n = if g.warp.is_some() { ((x1 - x0) / 4.0).ceil().clamp(1.0, 512.0) as usize } else { 1 };
            for i in 0..n {
                let (p, q) = (x0 + (x1 - x0) * i as f32 / n as f32, x0 + (x1 - x0) * (i + 1) as f32 / n as f32);
                out.push([g.at(p, y), g.at(q, y)]);
            }
        }
        out
    }

    fn lines(&mut self) -> Vec<HostLine> {
        let Some(g) = self.laid_out() else { return Vec::new() };
        let mut out = Vec::with_capacity(g.l.lines.len());
        for (li, ln) in g.l.lines.iter().enumerate() {
            let (line_text, chars) = line_chars(&g.text, &ln.range);
            let (top, bot) = (ln.baseline - ln.ascent, ln.baseline + ln.descent);
            let corners = [g.at(ln.x0, top), g.at(ln.x1, top), g.at(ln.x1, bot), g.at(ln.x0, bot)];
            // `HostLine::rect` is an `egui::Rect` in document units.
            let rect = egui::Rect::from_min_max(
                egui::pos2(corners.iter().map(|p| p.x).fold(f64::MAX, f64::min) as f32, corners.iter().map(|p| p.y).fold(f64::MAX, f64::min) as f32),
                egui::pos2(corners.iter().map(|p| p.x).fold(f64::MIN, f64::max) as f32, corners.iter().map(|p| p.y).fold(f64::MIN, f64::max) as f32),
            );
            let origin = g.at(ln.x0, ln.baseline);
            let step = g.at(ln.x0 + 1.0, ln.baseline);
            let unit = (step.x - origin.x).hypot(step.y - origin.y) as f32;
            let mut groups: Vec<(bool, Vec<_>)> = Vec::new();
            for c in g.l.clusters.iter().filter(|c| c.line == li) {
                match groups.last_mut() {
                    Some((rtl, members)) if *rtl == c.rtl => members.push(c),
                    _ => groups.push((c.rtl, vec![c])),
                }
            }
            let runs = groups
                .into_iter()
                .map(|(rtl, members)| {
                    let lo = members.iter().map(|c| c.range.start).min().unwrap_or(ln.range.start);
                    let hi = members.iter().map(|c| c.range.end).max().unwrap_or(lo);
                    let (mut positions, mut widths) = (Vec::new(), Vec::new());
                    for c in &members {
                        let n = g.text.get(c.range.clone()).map_or(1, |s| s.chars().count().max(1));
                        for k in 0..n {
                            positions.push((c.x - ln.x0 + c.advance * k as f32 / n as f32) * unit);
                            widths.push(c.advance / n as f32 * unit);
                        }
                    }
                    let rel = |at: usize| at.saturating_sub(ln.range.start).min(line_text.len());
                    HostRun::new(rel(lo)..rel(hi), rtl, positions, widths)
                })
                .collect();
            out.push(HostLine::new(line_text.to_owned(), chars, rect, origin.y as f32, runs));
        }
        out
    }

    fn line_range(&mut self, line: usize) -> Range<usize> {
        let Some(g) = self.laid_out() else { return 0..0 };
        g.l.lines.get(line).map_or(0..0, |ln| line_chars(&g.text, &ln.range).1)
    }

    fn line_step(&mut self, offset: usize, lines: i32) -> usize {
        let Some((l, _, text)) = layout(self.app, self.layer) else { return offset };
        // The position along the line stays that of the start offset over every step.
        let x = l.caret(byte_of(&text, offset)).0;
        (0..lines.unsigned_abs()).fold(offset, |at, _| photocraft_text::line_step(&l, &text, at, x, lines.signum()))
    }

    fn line_edge(&mut self, offset: usize, end: bool) -> usize {
        let Some((l, _, text)) = layout(self.app, self.layer) else { return offset };
        photocraft_text::line_edge(&l, &text, offset, end)
    }

    fn vertical(&self) -> bool {
        is_vertical(self.app, self.layer)
    }

    fn style_key(&mut self, key: StyleKey, selection: TextSelection) -> bool {
        // The formatting functions read their range from the session's selection.
        if let Some(ed) = self.app.ui.text_edit.as_mut() {
            ed.caret = selection.caret;
            ed.anchor = selection.anchor;
        }
        match key {
            StyleKey::Toggle(which) => toggle_char_style(self.app, which),
            StyleKey::Hyphenate => toggle_hyphenate(self.app),
            StyleKey::Leading(v) => step_char_metric(self.app, "leadingBy", v),
            StyleKey::BaselineShift(v) => step_char_metric(self.app, "baselineShiftBy", v),
            StyleKey::Tracking(v) => step_char_metric(self.app, "trackingBy", v),
            StyleKey::Kern(v) => kern_pair(self.app, self.layer, selection.caret, v),
            _ => return false,
        }
        true
    }
}

/// Typed `s` over the session's selection, as the editor's typing does: the caret ends after the new text.
#[cfg(test)]
pub(crate) fn type_text(app: &mut PhotocraftApp, s: &str) {
    let Some((layer, caret)) = app.ui.text_edit.as_ref().map(|ed| (LayerId(ed.layer), ed.caret.min(ed.anchor))) else { return };
    let range = super::selected_range(app).map_or(caret..caret, |[a, b]| a..b);
    if range.is_empty() && s.is_empty() {
        return;
    }
    LayerHost::new(app, layer).replace(range, s, EditKind::Typed);
}
