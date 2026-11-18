use super::build::{MAX_TEXT_INDEX, utf16_len};
use super::import::parse_txt2;
use super::{D, E, Id, parse_tysh, write_tysh};

/// The most values a `Txt2` object may hold to serve as the template for new objects. Photoshop's
/// own objects hold a few hundred (611 at most in the file from #7698).
pub(super) const TEMPLATE_NODES: usize = 4096;

/// Cumulative node and byte budgets across all run dictionaries cloned while refitting one `Txt2`
/// object (`runs_per` and `runs_to_length`), so an edited object with a large preserved run and
/// many output paragraphs falls back to a fresh object instead of amplifying memory.
const MAX_REFIT_RUN_NODES: usize = 16_384;

const MAX_REFIT_RUN_BYTES: usize = 1 << 20;

/// Cumulative node and string-byte budgets for the style runs copied from one preserved run while
/// restyling (`txt2_style_runs`, `td_restyled`, [`with_modes`]): a template within
/// [`TEMPLATE_NODES`] can still hold long strings, copied once per output run.
const MAX_COPIED_NODES: usize = 1 << 22;
const MAX_COPIED_BYTES: usize = 64 << 20;

/// Whether `copies` copies of `v` fit [`MAX_COPIED_NODES`] and [`MAX_COPIED_BYTES`].
#[cfg(test)]
pub(super) fn copies_fit(v: &E, copies: usize) -> bool {
    copies_fit_plus(v, copies, 0, 0)
}

/// [`copies_fit`] where each copy also gains up to `extra_nodes` nodes and `extra_bytes` key bytes
/// (keys the caller adds after copying).
pub(super) fn copies_fit_plus(v: &E, copies: usize, extra_nodes: usize, extra_bytes: usize) -> bool {
    let (mut nodes, mut bytes) = (MAX_COPIED_NODES, MAX_COPIED_BYTES);
    if !fits_budget(v, &mut nodes, &mut bytes) {
        return false;
    }
    let (n, b) = (MAX_COPIED_NODES - nodes, MAX_COPIED_BYTES - bytes);
    n.saturating_add(extra_nodes).checked_mul(copies).is_some_and(|t| t <= MAX_COPIED_NODES)
        && b.saturating_add(extra_bytes).checked_mul(copies).is_some_and(|t| t <= MAX_COPIED_BYTES)
}

/// Values at most this large survive [`compact`]: far above any style value Photoshop writes.
const SMALL_NODES: usize = 256;
const SMALL_BYTES: usize = 4096;

/// `v` without its oversized values, for a style run template that fails [`copies_fit`]: along
/// `path` each dictionary keeps its small entries and the next step of the path, so every
/// ordinary key is still copied into the new runs and only data no real file holds at that size
/// is left out. `None` when `v` is not a dictionary.
pub(super) fn compact(v: &E, path: &[&str]) -> Option<E> {
    let E::Dict(items) = v else { return None };
    let small = |k: &str, x: &E| k.len() <= SMALL_BYTES && fits_budget(x, &mut { SMALL_NODES }, &mut { SMALL_BYTES });
    let kept = items.iter().filter_map(|(k, x)| match path.split_first() {
        Some((head, rest)) if k == head => compact(x, rest).map(|c| (k.clone(), c)),
        _ => small(k, x).then(|| (k.clone(), x.clone())),
    });
    Some(E::Dict(kept.collect()))
}

/// Whether `v` holds at most `budget` values, counting down `budget` (stops at the first one over).
pub(super) fn fits(v: &E, budget: &mut usize) -> bool {
    let Some(left) = budget.checked_sub(1) else { return false };
    *budget = left;
    match v {
        E::Dict(items) => items.iter().all(|(_, x)| fits(x, budget)),
        E::Array(items) => items.iter().all(|x| fits(x, budget)),
        _ => true,
    }
}

/// Whether `v` fits within the remaining `(nodes, bytes)` budget, deducting its nodes and string
/// bytes as it walks.
fn fits_budget(v: &E, nodes: &mut usize, bytes: &mut usize) -> bool {
    let Some(left) = nodes.checked_sub(1) else { return false };
    *nodes = left;
    match v {
        E::Dict(items) => items.iter().all(|(k, x)| {
            let Some(rem) = bytes.checked_sub(k.len()) else { return false };
            *bytes = rem;
            fits_budget(x, nodes, bytes)
        }),
        E::Array(items) => items.iter().all(|x| fits_budget(x, nodes, bytes)),
        E::String(s) | E::RawString(s) | E::Name(s) => {
            let Some(rem) = bytes.checked_sub(s.len()) else { return false };
            *bytes = rem;
            true
        }
        _ => true,
    }
}

/// Whether the text object `object` has a view naming one frame (`/1 /0 [ << /0 n >> ]`), as
/// Photoshop writes it.
pub(super) fn names_one_frame(object: &E) -> bool {
    object.path(&["1", "0"]).and_then(E::as_array).is_some_and(|f| f.len() == 1 && f.first().and_then(|x| x.get("0")).is_some())
}

/// A text object for the slot `at`, which the file's block has none for: the file's object
/// `template` refitted to `text` by [`edited_object`], its view now naming frame `at` and its view
/// runs (`/1 /1 /0`, one per paragraph) remade for the new paragraphs. `None` when the template
/// can't be refitted.
pub(super) fn new_object(template: &E, at: usize, text: &str) -> Option<E> {
    let paragraphs: Vec<usize> = text.split_inclusive('\r').map(|p| p.encode_utf16().count()).collect();
    let frame = template.path(&["1", "0"]).and_then(E::as_array).and_then(|f| f.first()).and_then(|f| f.get("0")).cloned();
    let at = i64::try_from(at).ok()?;
    let mut object = edited_object(template, text)?;
    let view = object.get_mut("1")?;
    view.set("0", E::Array(vec![E::Dict(vec![("0".into(), E::Int(at))])]));
    // The rendered strikes' frame strike (`/F`) names the template's frame too.
    if let Some(E::Array(strikes)) = view.get_mut("2") {
        for s in strikes.iter_mut() {
            let Some(E::Array(parts)) = s.get_mut("6") else { continue };
            for p in parts.iter_mut().filter(|p| p.get("99").and_then(E::as_str) == Some("F") && p.get("10") == frame.as_ref()) {
                p.set("10", E::Int(at));
            }
        }
    }
    if let Some(runs) = view.path(&["1", "0"]).and_then(E::as_array) {
        let (mut nodes, mut bytes) = (MAX_REFIT_RUN_NODES, MAX_REFIT_RUN_BYTES);
        let fitted = runs_per(runs, &paragraphs, "1", &mut nodes, &mut bytes)?;
        view.get_mut("1")?.set("0", E::Array(fitted));
    }
    Some(object)
}

/// Whether `object` is the bare `<< /0 << /0 text /6 runs >> >>` object [`build_txt2`] writes
/// when it has nothing to take a shape from: no view, no paragraph runs.
pub(super) fn is_stub(object: &E) -> bool {
    let only = |v: &E, keys: &[&str]| matches!(v, E::Dict(items) if items.iter().all(|(k, _)| keys.contains(&k.as_str())));
    only(object, &["0"]) && object.get("0").is_some_and(|m| only(m, &["0", "6"]))
}

/// Gives the block `outer` a frame (`/0 /8 /0`) for each of its `count` text objects: Photoshop
/// writes one per object, and a reader indexes them by object number. Missing frames are the
/// small point-text frame Photoshop writes (`<< /0 << /2 << /6 [ -1.0 -1.0 ] /11 << /4 -1 /7
/// false >> >> >> >>`), never copies of a file's frame: the slot count reaches
/// [`MAX_TEXT_INDEX`], and a large frame copied that often would blow up the save. A block
/// without a frame list is left as it is.
pub(super) fn pad_frames(outer: &mut E, count: usize) {
    let Some(E::Array(frames)) = outer.get_mut("0").and_then(|v| v.get_mut("8")).and_then(|v| v.get_mut("0")) else { return };
    let count = count.min(MAX_TEXT_INDEX as usize + 1);
    if frames.len() < count {
        frames.resize(count, crate::psd_path::point_frame());
    }
}

/// Whether the view (`/1`) of the text object `object` covers `text` (`\r` breaks, trailing
/// `\r`): its runs (`/1 /0`, one per paragraph, each `<< … /1 length >>`) add up to the text's
/// length in UTF-16 units. Photoshop writes them so in every file it saves (all 59 text objects
/// of the corpus); a view left from another text, as releases before this check kept one after
/// an edit, does not. A view without runs covers any text: it says nothing about one.
pub(super) fn view_covers(object: &E, text: &str) -> bool {
    let Some(runs) = object.path(&["1", "1", "0"]).and_then(E::as_array) else { return true };
    let total = runs.iter().try_fold(0usize, |sum, r| sum.checked_add(usize::try_from(r.get("1").and_then(E::as_i64)?).ok()?));
    total == Some(utf16_len(text) as usize)
}

/// Makes `view` (a text object's `/1`) the view of a text that has not been laid out, with the
/// paragraph lengths `paragraphs` (UTF-16 units): the shape Photoshop writes for such type
/// (`text/path-wave-open.psd` in the corpus). Its runs (`/1 /0`) become one per paragraph, each
/// marked not laid out (`/0 << /1 0 >>`), and its rendered strikes (`/2 [ << /99 /PC … /6 [ … ]
/// >> ]`) lose their frame strikes (`/6`): the lines, glyph numbers and per-strike character
/// counts of the text they were laid out for. Photoshop lays the text out again. The frame the
/// view names (`/0`) and its other keys stay. `None` when the run budget is exceeded.
fn unrendered_view(view: &mut E, paragraphs: &[usize], nodes: &mut usize, bytes: &mut usize) -> Option<()> {
    if let Some(runs) = view.path(&["1", "0"]).and_then(E::as_array).filter(|r| r.iter().all(|x| matches!(x, E::Dict(_)) && x.get("1").is_some())) {
        let mut fitted = runs_per(runs, paragraphs, "1", nodes, bytes)?;
        for run in &mut fitted {
            if let Some(state @ E::Dict(_)) = run.get_mut("0")
                && state.get("1").is_some()
            {
                state.set("1", E::Int(0));
            }
        }
        view.get_mut("1")?.set("0", E::Array(fitted));
    }
    if let Some(E::Array(strikes)) = view.get_mut("2") {
        for strike in strikes.iter_mut() {
            if let E::Dict(items) = strike {
                items.retain(|(k, _)| k != "6");
            }
        }
    }
    Some(())
}

/// The file's `Txt2` text object `object` (`<< /0 model /1 view … >>`) for the changed engine
/// text `text` (`\r` breaks, trailing `\r`): the model's text replaced, its paragraph runs (`/5`)
/// one per paragraph of the new text (each keeping the style of the file's paragraph at that
/// position, else its last), and every other run array of the model (style runs `/6`, …) cut or
/// extended at its end to the new length. The glyph pen positions (`/21`) belong to the old text
/// and are dropped. So does the view's layout when the text changed, or when the view does not
/// cover the text ([`view_covers`]): it is written as the view of type not laid out yet
/// ([`unrendered_view`]). Kept, it counted the old text's characters beside the new text, a
/// state no file Photoshop saved is in. The frame the view names and the other keys are kept.
/// `None` when the object or one of its run arrays isn't shaped as Photoshop writes it: the
/// caller writes a fresh object instead.
pub(super) fn edited_object(object: &E, text: &str) -> Option<E> {
    let stale = object.path(&["0", "0"]).and_then(E::as_str) != Some(text) || !view_covers(object, text);
    let mut object = object.clone();
    let E::Dict(items) = &mut object else { return None };
    items.retain(|(k, _)| k != "21");
    let total = utf16_len(text) as usize;
    let paragraphs: Vec<usize> = text.split_inclusive('\r').map(|p| p.encode_utf16().count()).collect();
    let (mut nodes, mut bytes) = (MAX_REFIT_RUN_NODES, MAX_REFIT_RUN_BYTES);
    if stale && let Some(view @ E::Dict(_)) = object.get_mut("1") {
        unrendered_view(view, &paragraphs, &mut nodes, &mut bytes)?;
    }
    let model = object.get_mut("0").filter(|m| matches!(m, E::Dict(_)))?;
    let E::Dict(entries) = model else { return None };
    for (key, value) in entries.iter_mut() {
        if key.as_str() == "0" {
            *value = E::String(text.to_string());
            continue;
        }
        let Some(runs) = value.get("0").and_then(E::as_array).filter(|r| r.iter().all(|x| matches!(x, E::Dict(_)) && x.get("1").is_some())) else {
            continue;
        };
        let fitted = if key.as_str() == "5" {
            runs_per(runs, &paragraphs, "1", &mut nodes, &mut bytes)?
        } else {
            runs_to_length(runs, total, "1", &mut nodes, &mut bytes)?
        };
        value.set("0", E::Array(fitted));
    }
    Some(object)
}

/// The older Photoshop's `Txt2` text object `object` (`<< /Model << /Text … /ParagraphRun … >>
/// /View … >>`) for the changed engine text `text` (`\r` breaks, trailing `\r`): the model's text
/// replaced and every run array of the model refitted the way [`edited_object`] refits the
/// numbered objects' — the paragraph runs (`/ParagraphRun`) one per paragraph of the new text, the
/// others (`/StyleRun`, `/KernRun`, …) cut or extended to the new total. The model's other keys,
/// the view and the rest are kept: these blocks (the objects at `/DocumentObjects /TextObjects`
/// that older Photoshop versions write and re-render from, #6971) name their keys and number
/// their runs with `/Length` in UTF-16 units instead of `/1`. `None` when the object or one of
/// its run arrays isn't shaped as Photoshop writes it: the caller then leaves the entry as the
/// file has it, rather than write text its runs don't cover (#6968).
pub(super) fn td_edited_object(object: &E, text: &str) -> Option<E> {
    let mut object = object.clone();
    let model = object.get_mut("Model").filter(|m| matches!(m, E::Dict(_)))?;
    let E::Dict(entries) = model else { return None };
    // The text must be there to replace: refitted runs over a text the model doesn't hold would
    // be a half-edited object.
    if !entries.iter().any(|(k, v)| k == "Text" && matches!(v, E::String(_) | E::RawString(_))) {
        return None;
    }
    let total = utf16_len(text) as usize;
    let paragraphs: Vec<usize> = text.split_inclusive('\r').map(|p| p.encode_utf16().count()).collect();
    let (mut nodes, mut bytes) = (MAX_REFIT_RUN_NODES, MAX_REFIT_RUN_BYTES);
    for (key, value) in entries.iter_mut() {
        if key.as_str() == "Text" {
            *value = E::String(text.to_string());
            continue;
        }
        if !matches!(value, E::Dict(_)) {
            continue;
        }
        // A model dict without `RunArray` isn't a run array (the view's keys live elsewhere), so
        // it is skipped; a named `RunArray` is unambiguous — malformed, the whole entry is left to
        // the file rather than half-updated.
        let Some(runs) = value.get("RunArray") else { continue };
        let fitted = match runs {
            E::Array(runs) if runs.iter().all(|x| matches!(x, E::Dict(_)) && x.get("Length").is_some()) => {
                if key.as_str() == "ParagraphRun" {
                    runs_per(runs, &paragraphs, "Length", &mut nodes, &mut bytes)?
                } else {
                    runs_to_length(runs, total, "Length", &mut nodes, &mut bytes)?
                }
            }
            _ => return None,
        };
        value.set("RunArray", E::Array(fitted));
    }
    Some(object)
}

/// `runs` (`<< … /1 length >>`, the length key named `len_key`) remade with the lengths `lens`:
/// run i keeps the content of the file's run i, else of its last one. `None` for an empty `runs`
/// or when the cumulative `nodes` / `bytes` budget is exceeded.
fn runs_per(runs: &[E], lens: &[usize], len_key: &str, nodes: &mut usize, bytes: &mut usize) -> Option<Vec<E>> {
    let last = runs.last()?;
    lens.iter()
        .enumerate()
        .map(|(i, &len)| {
            let src = runs.get(i).unwrap_or(last);
            if !fits_budget(src, nodes, bytes) {
                return None;
            }
            let mut r = src.clone();
            r.set(len_key, E::Int(i64::try_from(len).ok()?));
            Some(r)
        })
        .collect()
}

/// `runs` (`<< … /1 length >>`, the length key named `len_key`) covering `total` UTF-16 units:
/// the leading runs as they were, cut where `total` ends, the last kept run extended when `total`
/// is longer. Empty runs are dropped. `None` when `runs` is empty, a length isn't a count, or the
/// cumulative `nodes` / `bytes` budget is exceeded.
fn runs_to_length(runs: &[E], total: usize, len_key: &str, nodes: &mut usize, bytes: &mut usize) -> Option<Vec<E>> {
    let mut out: Vec<(E, usize)> = Vec::new();
    let mut left = total;
    for r in runs {
        let len = r.get(len_key).and_then(E::as_i64).and_then(|l| usize::try_from(l).ok())?;
        let take = len.min(left);
        if take > 0 {
            if !fits_budget(r, nodes, bytes) {
                return None;
            }
            out.push((r.clone(), take));
            left -= take;
        }
    }
    if left > 0 {
        match out.last_mut() {
            Some(last) => last.1 = last.1.checked_add(left)?,
            None => {
                let last = runs.last()?;
                if !fits_budget(last, nodes, bytes) {
                    return None;
                }
                out.push((last.clone(), left));
            }
        }
    }
    out.into_iter()
        .map(|(mut r, len)| {
            r.set(len_key, E::Int(i64::try_from(len).ok()?));
            Some(r)
        })
        .collect()
}

/// The file's own `Txt2` style runs `runs` with the auto-kern modes `modes` (`(mode, UTF-16
/// length)`, see [`txt2_mode_runs`]) set at `/0 /0 /6 /11`, keeping every other style key. A
/// run a mode change falls inside is split there. `None` when the runs don't cover the text or
/// lack a style dictionary: the caller writes its own runs instead. Runs are moved, not copied;
/// only the extra pieces a split makes are copies, charged to the copy budget, so an unchanged
/// object keeps its runs whatever their size.
pub(super) fn with_modes(runs: Vec<E>, modes: &[(i64, usize)]) -> Option<Vec<E>> {
    let lens: Vec<usize> = runs.iter().map(|r| r.get("1").and_then(E::as_i64).and_then(|l| usize::try_from(l).ok())).collect::<Option<_>>()?;
    let total = modes.iter().try_fold(0usize, |a, m| a.checked_add(m.1))?;
    if lens.iter().try_fold(0usize, |a, l| a.checked_add(*l))? != total {
        return None;
    }
    let mut out = Vec::with_capacity(runs.len());
    let (mut nodes, mut bytes) = (MAX_COPIED_NODES, MAX_COPIED_BYTES);
    let (mut mi, mut mode_end) = (0usize, modes.first()?.1);
    let mut at = 0usize;
    for (run, len) in runs.into_iter().zip(lens) {
        if len == 0 {
            out.push(run);
            continue;
        }
        let mut run = Some(run);
        let end = at + len;
        while at < end {
            while at >= mode_end && mi + 1 < modes.len() {
                mi += 1;
                mode_end = mode_end.saturating_add(modes.get(mi)?.1);
            }
            let piece_end = end.min(mode_end);
            if piece_end <= at {
                return None;
            }
            // The last piece takes the run itself; every earlier one (a mode boundary inside the
            // run) is a copy.
            let mut piece = if piece_end == end {
                run.take()?
            } else {
                if !fits_budget(run.as_ref()?, &mut nodes, &mut bytes) {
                    return None;
                }
                run.clone()?
            };
            if piece_end - at != len {
                piece.set("1", E::Int(i64::try_from(piece_end - at).ok()?));
            }
            let style = piece.get_mut("0").and_then(|v| v.get_mut("0")).and_then(|v| v.get_mut("6")).filter(|v| matches!(v, E::Dict(_)))?;
            let mode = modes.get(mi)?.0;
            // Keep the file's value (and its number type) when the mode is the same.
            if style.get("11").and_then(E::as_i64) != Some(mode) {
                style.set("11", E::Int(mode));
            }
            out.push(piece);
            at = piece_end;
        }
    }
    Some(out)
}

/// The layer's text-object number in the document's `Txt2` block, when its `TySh` names one
/// within `0..=`[`MAX_TEXT_INDEX`]. PSD export keeps a preserved number — its regenerated `Txt2`
/// object sits at that slot, and [`apply_txt2`] looks the slot up — and numbers layers without
/// one (or with a file-controlled out-of-range one) afresh.
pub fn text_index(tysh: &[u8]) -> Option<i32> {
    let t = parse_tysh(tysh)?;
    match t.text.get("TextIndex") {
        Some(D::Integer(i)) if (0..=MAX_TEXT_INDEX).contains(i) => Some(*i),
        _ => None,
    }
}

/// How many text objects the document `Txt2` block `data` holds (`/1 /1`), named by a layer or
/// not; 0 when it doesn't parse. PSD export numbers new type layers past them, so a new layer
/// never lands on a deleted layer's object.
pub fn txt2_object_count(data: &[u8]) -> usize {
    parse_txt2(data).and_then(|t| t.path(&["1", "1"]).and_then(E::as_array).map(<[E]>::len)).unwrap_or(0)
}

/// Rewrites a `TySh`'s `TextIndex`: the layer's text-object number in the document's `Txt2`
/// block ([`build_txt2`] writes them in this order). `None` when `tysh` doesn't parse.
pub fn set_text_index(tysh: &[u8], index: i32) -> Option<Vec<u8>> {
    let mut t = parse_tysh(tysh)?;
    match t.text.items.iter_mut().find(|(k, _)| k.is("TextIndex")) {
        Some(e) => e.1 = D::Integer(index),
        // Where Photoshop writes it: right before `EngineData`.
        None => {
            let at = t.text.items.iter().position(|(k, _)| k.is("EngineData")).unwrap_or(t.text.items.len());
            t.text.items.insert(at, (Id::new("TextIndex"), D::Integer(index)));
        }
    }
    write_tysh(&t)
}

#[cfg(test)]
mod copy_budget_tests {
    use super::{E, MAX_COPIED_BYTES, MAX_COPIED_NODES, compact, copies_fit, copies_fit_plus, with_modes};

    /// `<< /0 << /0 << /6 << /11 1 /99 (filler) >> >> >> /1 len >>`, a Txt2 style run.
    fn run(len: i64, filler: usize) -> E {
        let style = E::Dict(vec![("11".into(), E::Int(1)), ("99".into(), E::String("x".repeat(filler)))]);
        let inner = E::Dict(vec![("6".into(), style)]);
        E::Dict(vec![("0".into(), E::Dict(vec![("0".into(), inner)])), ("1".into(), E::Int(len))])
    }

    #[test]
    fn copies_of_a_large_preserved_run_are_refused() {
        assert!(copies_fit(&run(1, 16), 1000));
        let big = run(1, 1 << 20);
        assert!(copies_fit(&big, 63));
        assert!(!copies_fit(&big, MAX_COPIED_BYTES >> 20), "64 copies of 1 MiB pass the byte budget");
        assert!(!copies_fit(&big, usize::MAX));
    }

    #[test]
    fn keys_added_per_copy_count_against_the_node_budget() {
        let small = run(1, 16);
        let copies = MAX_COPIED_NODES / 8;
        assert!(copies_fit(&small, copies), "the template alone passes");
        assert!(!copies_fit_plus(&small, copies, 18, 0), "the keys each copy gains do not");
    }

    #[test]
    fn key_bytes_added_per_copy_count_against_the_byte_budget() {
        let small = run(1, 16);
        let copies = 100_000;
        assert!(copies_fit_plus(&small, copies, 0, 0), "the template alone passes");
        assert!(!copies_fit_plus(&small, copies, 0, 700), "the key bytes each copy gains do not");
    }

    #[test]
    fn compacting_drops_only_oversized_values() {
        let big = run(5, 1 << 20);
        let small = compact(&big, &["0", "0", "6"]).unwrap();
        // The 1 MiB string goes; the mode key, the path and the length stay.
        assert_eq!(small.path(&["0", "0", "6", "11"]).and_then(E::as_i64), Some(1));
        assert!(small.path(&["0", "0", "6", "99"]).is_none());
        assert_eq!(small.get("1").and_then(E::as_i64), Some(5));
        assert!(copies_fit(&small, 100_000));
        // An ordinary run comes back whole.
        assert_eq!(compact(&run(5, 16), &["0", "0", "6"]), Some(run(5, 16)));
    }

    #[test]
    fn mode_splits_stop_at_the_copy_budget() {
        // One run, 1 MiB of preserved text, split at every one of 200 mode boundaries.
        let modes: Vec<(i64, usize)> = (0..200).map(|i| (i % 2, 1)).collect();
        assert!(with_modes(vec![run(200, 1 << 20)], &modes).is_none());
        assert_eq!(with_modes(vec![run(200, 16)], &modes).map(|v| v.len()), Some(200));
        // Unsplit runs are moved, not copied: an unchanged object keeps them whatever their size.
        let kept = vec![run(100, 1 << 20), run(100, 1 << 20)];
        let modes = [(1, 100), (1, 100)];
        assert_eq!(with_modes(kept.clone(), &modes), Some(kept));
    }
}

#[cfg(test)]
mod view_tests {
    use super::{E, edited_object, utf16_len, view_covers};

    /// A laid-out object as Photoshop writes one: `text` in the model, and a view of `counted`
    /// UTF-16 units in one run and one line.
    fn laid_out(text: &str, counted: i64) -> E {
        let run = |len: i64| E::Dict(vec![("0".into(), E::Dict(vec![("1".into(), E::Int(1))])), ("1".into(), E::Int(len))]);
        let segment = E::Dict(vec![("99".into(), E::Name("S".into())), ("15".into(), E::Dict(vec![("0".into(), E::Int(counted))]))]);
        let frame_strike = E::Dict(vec![("99".into(), E::Name("F".into())), ("10".into(), E::Int(3)), ("6".into(), E::Array(vec![segment]))]);
        let root = E::Dict(vec![("99".into(), E::Name("PC".into())), ("5".into(), E::Int(0)), ("6".into(), E::Array(vec![frame_strike]))]);
        let view = E::Dict(vec![
            ("0".into(), E::Array(vec![E::Dict(vec![("0".into(), E::Int(3))])])),
            ("1".into(), E::Dict(vec![("0".into(), E::Array(vec![run(counted)]))])),
            ("2".into(), E::Array(vec![root])),
        ]);
        let runs = |len: i64| E::Dict(vec![("0".into(), E::Array(vec![E::Dict(vec![("0".into(), E::dict()), ("1".into(), E::Int(len))])]))]);
        let len = utf16_len(text);
        let model = E::Dict(vec![("0".into(), E::String(text.into())), ("5".into(), runs(len)), ("6".into(), runs(len))]);
        E::Dict(vec![("0".into(), model), ("1".into(), view)])
    }

    fn view_run_lengths(object: &E) -> Vec<i64> {
        object.path(&["1", "1", "0"]).and_then(E::as_array).unwrap().iter().map(|r| r.get("1").and_then(E::as_i64).unwrap()).collect()
    }

    /// The view of an edited object is the view of the new text, not laid out: the lines Photoshop
    /// stored count the old text's characters, and its own files never hold a view that disagrees
    /// with the text.
    #[test]
    fn an_edited_object_leaves_out_the_old_texts_lines() {
        let edited = edited_object(&laid_out("Old\r", 4), "A\rBC\r").unwrap();
        assert_eq!(edited.path(&["0", "0"]).and_then(E::as_str), Some("A\rBC\r"));
        assert_eq!(view_run_lengths(&edited), [2, 3], "one view run per paragraph of the new text");
        for run in edited.path(&["1", "1", "0"]).and_then(E::as_array).unwrap() {
            assert_eq!(run.path(&["0", "1"]), Some(&E::Int(0)), "marked not laid out");
        }
        let root = &edited.path(&["1", "2"]).and_then(E::as_array).unwrap()[0];
        assert_eq!(root, &E::Dict(vec![("99".into(), E::Name("PC".into())), ("5".into(), E::Int(0))]), "the strike root keeps no lines");
        assert_eq!(edited.path(&["1", "0"]), laid_out("Old\r", 4).path(&["1", "0"]), "the view still names its frame");
        assert!(view_covers(&edited, "A\rBC\r"));
        // The same length, another text: the lines hold the old text's glyphs.
        let edited = edited_object(&laid_out("Old\r", 4), "New\r").unwrap();
        assert!(edited.path(&["1", "2"]).and_then(E::as_array).unwrap()[0].get("6").is_none());
    }

    /// Refitting an object to the text it already holds (a style-only edit) keeps its layout,
    /// unless the view counts another text: the state an earlier release left after an edit.
    #[test]
    fn a_refit_to_the_same_text_keeps_a_view_that_covers_it() {
        let object = laid_out("Old\r", 4);
        assert_eq!(edited_object(&object, "Old\r").unwrap().get("1"), object.get("1"));
        let stale = laid_out("Newer!!\r", 4);
        assert!(!view_covers(&stale, "Newer!!\r"));
        let repaired = edited_object(&stale, "Newer!!\r").unwrap();
        assert_eq!(view_run_lengths(&repaired), [8]);
        assert!(repaired.path(&["1", "2"]).and_then(E::as_array).unwrap()[0].get("6").is_none());
    }

    /// A view without runs says nothing about a text; lengths that are no counts never cover one,
    /// and never overflow.
    #[test]
    fn view_cover_reads_hostile_lengths() {
        let with_runs = |lens: &[E]| {
            let runs = lens.iter().map(|l| E::Dict(vec![("1".into(), l.clone())])).collect();
            E::Dict(vec![("1".into(), E::Dict(vec![("1".into(), E::Dict(vec![("0".into(), E::Array(runs))]))]))])
        };
        assert!(view_covers(&E::dict(), "AV\r"));
        assert!(view_covers(&E::Dict(vec![("1".into(), E::Dict(vec![("0".into(), E::Array(vec![]))]))]), "AV\r"));
        assert!(view_covers(&with_runs(&[E::Int(1), E::Int(2)]), "AV\r"));
        assert!(!view_covers(&with_runs(&[E::Int(2)]), "AV\r"));
        assert!(!view_covers(&with_runs(&[E::Int(-1), E::Int(4)]), "AV\r"));
        assert!(!view_covers(&with_runs(&[E::Int(i64::MAX), E::Int(i64::MAX)]), "AV\r"));
        assert!(!view_covers(&with_runs(&[E::String("3".into())]), "AV\r"));
        assert!(!view_covers(&with_runs(&[]), "AV\r"));
        // And a view that isn't shaped as runs is refitted without a panic.
        let mut odd = laid_out("Old\r", 4);
        odd.get_mut("1").unwrap().set("1", E::Int(7));
        odd.get_mut("1").unwrap().set("2", E::Bool(true));
        assert!(edited_object(&odd, "AV\r").is_some());
    }
}
