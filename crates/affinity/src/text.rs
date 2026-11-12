//! Artistic and frame text: the story's characters with their font, size, colour, tracking and
//! leading, the paragraph alignment and the first baseline. Layout (line breaks in frames, kerning,
//! OpenType features) is left to the importing application.

use crate::model::{Affine, Align, Point, Reader, Text, TextRun, rect};
use crate::paint::{self, Paint};
use crate::stream::{ObjId, Stream, Value};

/// Characters read per text node (stories are split into blocks; this caps hostile ones).
const MAX_CHARS: usize = 1 << 20;

pub(crate) fn read(r: &mut Reader, id: ObjId, world: Affine) -> Option<Text> {
    let s = r.s;
    let story = s.obj(id, b"StSt")?;
    let frame = s.obj(id, b"TxtH")?;
    let artistic = s.is(frame, b"ArFr");
    let bounds = s.floats::<4>(frame, b"FrmB").map(rect)?;
    let mut runs = Vec::new();
    let mut align = None;
    let mut total = 0usize;
    let mut placeholders = false;
    // Shared structures are traversed once per occurrence: charge the raw slots (nulls and empty
    // blocks included) before collecting them, against a document-wide budget.
    if !r.charge_text(s.array_len(story, b"Blok").unwrap_or(0)) {
        r.warn("text beyond the document text budget (left out)");
        return None;
    }
    'blocks: for block in s.objs(story, b"Blok") {
        let glyphs = s.obj(block, b"Glyp")?;
        let runs_n = s.obj(block, b"GAtt").and_then(|g| s.array_len(g, b"Runs")).unwrap_or(0);
        if !r.charge_text(1 + runs_n + s.array_len(glyphs, b"Mixd").unwrap_or(0)) {
            r.warn("text beyond the document text budget (truncated)");
            break;
        }
        // Applied to `align` only once this block passes the character limit below, so a discarded
        // block does not set the alignment of the text kept from earlier blocks.
        // Scanning the paragraph runs (null slots included) is charged before it is done.
        let mut block_align = None;
        if align.is_none()
            && let Some(p) = s.obj(block, b"PAtt")
        {
            if r.charge_text(s.array_len(p, b"Runs").unwrap_or(0)) {
                block_align = s.objs(p, b"Runs").first().copied().and_then(|run| s.obj(run, b"Item")).map(|p| paragraph_align(r, p));
            } else {
                r.warn("text beyond the document text budget (paragraph alignment left out)");
            }
        }
        // `block_chars` scans the `Utf8` string, or walks each segment's `Glys` slots (nulls included) and `Utf8` text, and stops at the character limit: charge only that scan, first.
        let slots = scan_cost(s, glyphs, room(total), r.text_left());
        if !r.charge_text(slots) {
            r.warn("text beyond the document text budget (truncated)");
            break;
        }
        let chars = block_chars(s, glyphs, total, &mut placeholders);
        total = total.checked_add(chars.len())?;
        if total > MAX_CHARS {
            r.warn("text longer than a million characters (truncated)");
            break;
        }
        if align.is_none() {
            align = block_align;
        }
        let mut start = 0usize;
        for run in s.obj(block, b"GAtt").map(|g| s.objs(g, b"Runs")).unwrap_or_default() {
            let end = usize::try_from(s.int(run, b"Indx")?).ok()?.min(chars.len());
            if end <= start {
                continue;
            }
            let text: String = chars.get(start..end)?.iter().filter(|c| **c != '\0' && **c != '\u{FFFC}').collect();
            start = end;
            let Some(attrs) = s.obj(run, b"Item") else { continue };
            if !text.is_empty() {
                if !r.charge_text(text.len()) {
                    r.warn("text beyond the document text budget (truncated)");
                    break 'blocks;
                }
                runs.push(run_attrs(r, attrs, text, world));
            }
        }
    }
    if placeholders {
        r.warn("text fields such as page numbers (left out of the text)");
    }
    let align = align.unwrap_or(Align::Left);
    let first_baseline = bounds.y0 + s.f64(frame, b"ArtV").unwrap_or(0.0);
    let anchor_x = match align {
        Align::Center => (bounds.x0 + bounds.x1) / 2.0,
        Align::Right => bounds.x1,
        _ => bounds.x0,
    };
    if !artistic {
        // The raw slot count, not a walk of the slots: a shared frame is read for every occurrence.
        if s.array_len(frame, b"ColW").is_some_and(|n| n > 1) {
            r.warn("text frames with several columns (imported as one column)");
        }
        if s.class(id) == Some(crate::stream::Tag::of(b"TxtC")) {
            r.warn("text frames with a curved outline (imported as rectangular frames)");
        }
    }
    Some(Text { runs, align, anchor: Point { x: anchor_x, y: first_baseline }, frame: if artistic { None } else { Some(bounds) }, transform: world })
}

/// One past the allowance is enough to trip the check in `read`; `block_chars` stops there.
fn room(total: usize) -> usize {
    MAX_CHARS.saturating_sub(total).saturating_add(1)
}

/// The bytes of `t` that holding `want` characters of it scans, and the characters taken.
fn prefix(t: &str, want: usize) -> (usize, usize) {
    t.char_indices().enumerate().find(|(n, _)| *n == want).map_or((t.len(), t.chars().count()), |(n, (i, _))| (i, n))
}

/// What `block_chars` scans for a block, in the units `charge_text` takes: the `Utf8` bytes it
/// reads, or per segment its `Glys` slots (nulls included) and the `Utf8` bytes it reads, up to the
/// character allowance `room`; text past it is never scanned, so it is not charged. Segments are
/// shared and repeated by reference, so a segment's slots are counted before they are walked, and
/// once the count passes `budget` (the text work left) it is returned without walking further: the
/// charge fails as it would after the whole walk.
fn scan_cost(s: &Stream, glyphs: ObjId, room: usize, budget: usize) -> usize {
    if let Some(t) = s.str(glyphs, b"Utf8") {
        return prefix(t, room).0;
    }
    let (mut len, mut cost) = (0usize, 0usize);
    for seg in s.objs(glyphs, b"Mixd") {
        if len >= room {
            break;
        }
        cost = cost.saturating_add(s.array_len(seg, b"Glys").unwrap_or(0));
        if cost > budget {
            return cost;
        }
        let inline = s.objs(seg, b"Glys").len();
        len += inline.min(room - len);
        let (bytes, chars) = prefix(s.str(seg, b"Utf8").unwrap_or_default(), room.saturating_sub(len));
        len += chars;
        cost = cost.saturating_add(bytes);
    }
    cost
}

/// The characters of one text block. A block without a UTF-8 string is a list of segments: each
/// inline object (page number, anchor) takes one `U+FFFC` before its segment's text. Stops once
/// `total` plus these characters would pass the cap.
fn block_chars(s: &Stream, glyphs: ObjId, total: usize, placeholders: &mut bool) -> Vec<char> {
    let room = room(total);
    match s.str(glyphs, b"Utf8") {
        Some(t) => t.chars().take(room).collect(),
        None => {
            // Inline objects (page numbers, anchors) each take one index before the segment text.
            let mut v = Vec::new();
            // Shared segments repeated by reference must not expand beyond the allowance first.
            for seg in s.objs(glyphs, b"Mixd") {
                if v.len() >= room {
                    break;
                }
                let inline = s.objs(seg, b"Glys").len();
                v.extend(std::iter::repeat_n('\u{FFFC}', inline.min(room - v.len())));
                *placeholders |= inline > 0;
                v.extend(s.str(seg, b"Utf8").unwrap_or_default().chars().take(room.saturating_sub(v.len())));
            }
            v
        }
    }
}

fn slot<T: Copy>(v: Option<&Value>, i: usize, f: impl Fn(&Value) -> Option<T>) -> Option<T> {
    match v? {
        Value::Array(a) => a.get(i).and_then(f),
        _ => None,
    }
}

fn float(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) if f.is_finite() => Some(*f),
        _ => None,
    }
}

fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        Value::UInt(u) => i64::try_from(*u).ok(),
        _ => None,
    }
}

/// Longest font name kept; real PostScript and family names are far shorter.
const MAX_FONT_NAME: usize = 256;

/// A font name for one run. Runs repeat the same shared string, so its copies are charged
/// against a document-wide byte budget; an overlong name or an exhausted budget leaves it empty.
fn font_name(r: &mut Reader, font: Option<ObjId>, tag: &[u8; 4]) -> String {
    let name = font.and_then(|f| r.s.str(f, tag)).unwrap_or_default();
    if name.len() > MAX_FONT_NAME {
        r.warn("font names longer than 256 bytes (left empty)");
        return String::new();
    }
    if !r.charge_font_name(name.len()) {
        r.warn("font names beyond the document metadata budget (left empty)");
        return String::new();
    }
    name.to_string()
}

fn run_attrs(r: &mut Reader, attrs: ObjId, text: String, world: Affine) -> TextRun {
    let s = r.s;
    let doubles = s.field(attrs, b"Doub");
    let ints = s.field(attrs, b"Ints");
    let size = slot(doubles, 0, float).filter(|v| *v > 0.0 && *v < 1e6).unwrap_or(12.0 * r.dpi / 72.0);
    let tracking = slot(doubles, 4, float).unwrap_or(0.0);
    let leading = (slot(ints, 4, int) == Some(1)).then(|| slot(doubles, 13, float)).flatten().filter(|v| *v > 0.0);
    // Shared attributes are read for every run: take the fill and stroke slots by index, never
    // copying the whole object list (null slots included) per run.
    let objects = s.field(attrs, b"Objs");
    let obj = |v: &Value| if let Value::Obj(o) = v { *o } else { None };
    let fill = slot(objects, 0, obj).and_then(|f| paint::descriptor(r, f, world)).unwrap_or(Paint::None);
    if slot(objects, 1, obj).and_then(|f| paint::descriptor(r, f, world)).is_some_and(|p| p != Paint::None) {
        r.warn("outlined (stroked) text");
    }
    if slot(doubles, 10, float).is_some_and(|h| (h - 1.0).abs() > 1e-3) {
        r.warn("horizontally scaled text");
    }
    let font = s.obj(attrs, b"DFnt");
    let postscript = font_name(r, font, b"Post");
    let family = font_name(r, font, b"Famy");
    TextRun {
        text,
        postscript,
        family,
        weight: font.and_then(|f| s.int(f, b"Wegt")).unwrap_or(400),
        italic: font.and_then(|f| s.bool(f, b"Ital")).unwrap_or(false),
        size,
        tracking,
        leading,
        fill,
    }
}

fn paragraph_align(r: &mut Reader, p: ObjId) -> Align {
    match slot(r.s.field(p, b"Ints"), 0, int) {
        Some(1) => Align::Center,
        Some(2) => Align::Right,
        Some(3) => Align::Justify,
        Some(0) | None => Align::Left,
        Some(_) => {
            r.warn("an unknown paragraph alignment (imported as left)");
            Align::Left
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{self, F, tag};

    #[test]
    fn the_scan_cost_stops_once_the_glyph_slots_pass_the_budget() {
        // Three references to one segment of ten null glyph slots: thirty slots in all.
        let mut refs = vec![F::Def(1, vec![tag(b"Segm")], vec![(tag(b"Glys"), F::Shared(vec![F::Null; 10]))])];
        refs.extend([F::Ref(1), F::Ref(1)]);
        let s = crate::stream::parse(&synth::stream(&[(tag(b"Glyp"), F::Obj(tag(b"Glyp"), vec![(tag(b"Mixd"), F::Shared(refs))]))])).unwrap();
        let glyphs = s.obj(s.root, b"Glyp").unwrap();
        assert_eq!(scan_cost(&s, glyphs, room(0), usize::MAX), 30);
        assert_eq!(scan_cost(&s, glyphs, room(0), 30), 30);
        // Past the budget at the second segment: neither it nor the third is walked.
        assert_eq!(scan_cost(&s, glyphs, room(0), 15), 20);
    }
}
