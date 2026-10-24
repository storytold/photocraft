//! Arabic joining for fonts that carry no OpenType positional features.
//!
//! A font such as Muna maps the Arabic Presentation Forms (U+FB50..U+FEFC) in its `cmap` but has
//! no `init`/`medi`/`fina`/`isol` lookups. HarfBuzz joins such a font anyway: its Arabic shaper
//! synthesises the positional forms (and the lam-alef ligatures) from the presentation forms.
//! HarfRust, which parley shapes with, does not (its `arabic_fallback_shape` returns false), so
//! every letter would come out isolated.
//!
//! This module does the same work on the shaped glyphs, so byte offsets, clusters and carets never
//! change: a letter's glyph is swapped for the presentation form its joining context calls for, and
//! the advance difference is reported per cluster for the caller to fold into the line's kerning,
//! which keeps alignment anchored.
//!
//! The letter table is derived from HarfBuzz's `hb-ot-shaper-arabic-table.hh` (itself generated from
//! Unicode's `ArabicShaping.txt` and `UnicodeData.txt`); joining types follow `ArabicShaping.txt`.
//! A letter missing from the table is treated as non-joining and keeps its shaped glyph.

use std::ops::Range;

use parley::FontData;
use skrifa::MetadataProvider;
use skrifa::raw::TableProvider;

/// Positional form of a joining letter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Form {
    Isolated,
    Final,
    Initial,
    Medial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Joining {
    /// Joins on both sides (Unicode `D`).
    Dual,
    /// Joins to the letter before it only (Unicode `R`).
    Right,
    /// ZWJ: joins both neighbours.
    Causing,
    /// Combining marks: skipped when looking for neighbours.
    Transparent,
    /// Everything else, ZWNJ included: breaks the join.
    None,
}

const ZWJ: char = '\u{200D}';
const LAM: char = '\u{0644}';

// The LETTERS table and the lam-alef ligatures in `lam_alef` are derived from HarfBuzz,
// `src/hb-ot-shaper-arabic-table.hh` (https://github.com/harfbuzz/harfbuzz), under the
// "Old MIT" license:
//
// Copyright © 2010-2022  Google, Inc.
// Copyright © 2015-2020  Ebrahim Byagowi
// Copyright © 2019,2020  Facebook, Inc.
// Copyright © 2012,2015  Mozilla Foundation
// Copyright © 2011  Codethink Limited
// Copyright © 2008,2010  Nokia Corporation and/or its subsidiary(-ies)
// Copyright © 2009  Keith Stribley
// Copyright © 2011  Martin Hosken and SIL International
// Copyright © 2007  Chris Wilson
// Copyright © 2005,2006,2020,2021,2022,2023  Behdad Esfahbod
// Copyright © 2004,2007,2008,2009,2010,2013,2021,2022,2023  Red Hat, Inc.
// Copyright © 1998-2005  David Turner and Werner Lemberg
// Copyright © 2016  Igalia S.L.
// Copyright © 2022  Matthias Clasen
// Copyright © 2018,2021  Khaled Hosny
// Copyright © 2018,2019,2020  Adobe, Inc
// Copyright © 2013-2015  Alexei Podtelezhnikov
//
// For full copyright notices consult the individual files in the package.
//
// Permission is hereby granted, without written agreement and without
// license or royalty fees, to use, copy, modify, and distribute this
// software and its documentation for any purpose, provided that the
// above copyright notice and the following two paragraphs appear in
// all copies of this software.
//
// IN NO EVENT SHALL THE COPYRIGHT HOLDER BE LIABLE TO ANY PARTY FOR
// DIRECT, INDIRECT, SPECIAL, INCIDENTAL, OR CONSEQUENTIAL DAMAGES
// ARISING OUT OF THE USE OF THIS SOFTWARE AND ITS DOCUMENTATION, EVEN
// IF THE COPYRIGHT HOLDER HAS BEEN ADVISED OF THE POSSIBILITY OF SUCH
// DAMAGE.
//
// THE COPYRIGHT HOLDER SPECIFICALLY DISCLAIMS ANY WARRANTIES, INCLUDING,
// BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND
// FITNESS FOR A PARTICULAR PURPOSE.  THE SOFTWARE PROVIDED HEREUNDER IS
// ON AN "AS IS" BASIS, AND THE COPYRIGHT HOLDER HAS NO OBLIGATION TO
// PROVIDE MAINTENANCE, SUPPORT, UPDATES, ENHANCEMENTS, OR MODIFICATIONS.

/// Letters with their joining type and presentation forms `[isolated, final, initial, medial]`
/// (0 = the form does not exist).
const LETTERS: &[(u32, Joining, [u32; 4])] = &[
    (0x0621, Joining::None, [0xFE80, 0, 0, 0]),
    (0x0622, Joining::Right, [0xFE81, 0xFE82, 0, 0]),
    (0x0623, Joining::Right, [0xFE83, 0xFE84, 0, 0]),
    (0x0624, Joining::Right, [0xFE85, 0xFE86, 0, 0]),
    (0x0625, Joining::Right, [0xFE87, 0xFE88, 0, 0]),
    (0x0626, Joining::Dual, [0xFE89, 0xFE8A, 0xFE8B, 0xFE8C]),
    (0x0627, Joining::Right, [0xFE8D, 0xFE8E, 0, 0]),
    (0x0628, Joining::Dual, [0xFE8F, 0xFE90, 0xFE91, 0xFE92]),
    (0x0629, Joining::Right, [0xFE93, 0xFE94, 0, 0]),
    (0x062A, Joining::Dual, [0xFE95, 0xFE96, 0xFE97, 0xFE98]),
    (0x062B, Joining::Dual, [0xFE99, 0xFE9A, 0xFE9B, 0xFE9C]),
    (0x062C, Joining::Dual, [0xFE9D, 0xFE9E, 0xFE9F, 0xFEA0]),
    (0x062D, Joining::Dual, [0xFEA1, 0xFEA2, 0xFEA3, 0xFEA4]),
    (0x062E, Joining::Dual, [0xFEA5, 0xFEA6, 0xFEA7, 0xFEA8]),
    (0x062F, Joining::Right, [0xFEA9, 0xFEAA, 0, 0]),
    (0x0630, Joining::Right, [0xFEAB, 0xFEAC, 0, 0]),
    (0x0631, Joining::Right, [0xFEAD, 0xFEAE, 0, 0]),
    (0x0632, Joining::Right, [0xFEAF, 0xFEB0, 0, 0]),
    (0x0633, Joining::Dual, [0xFEB1, 0xFEB2, 0xFEB3, 0xFEB4]),
    (0x0634, Joining::Dual, [0xFEB5, 0xFEB6, 0xFEB7, 0xFEB8]),
    (0x0635, Joining::Dual, [0xFEB9, 0xFEBA, 0xFEBB, 0xFEBC]),
    (0x0636, Joining::Dual, [0xFEBD, 0xFEBE, 0xFEBF, 0xFEC0]),
    (0x0637, Joining::Dual, [0xFEC1, 0xFEC2, 0xFEC3, 0xFEC4]),
    (0x0638, Joining::Dual, [0xFEC5, 0xFEC6, 0xFEC7, 0xFEC8]),
    (0x0639, Joining::Dual, [0xFEC9, 0xFECA, 0xFECB, 0xFECC]),
    (0x063A, Joining::Dual, [0xFECD, 0xFECE, 0xFECF, 0xFED0]),
    // Tatweel joins but has no presentation forms.
    (0x0640, Joining::Dual, [0, 0, 0, 0]),
    (0x0641, Joining::Dual, [0xFED1, 0xFED2, 0xFED3, 0xFED4]),
    (0x0642, Joining::Dual, [0xFED5, 0xFED6, 0xFED7, 0xFED8]),
    (0x0643, Joining::Dual, [0xFED9, 0xFEDA, 0xFEDB, 0xFEDC]),
    (0x0644, Joining::Dual, [0xFEDD, 0xFEDE, 0xFEDF, 0xFEE0]),
    (0x0645, Joining::Dual, [0xFEE1, 0xFEE2, 0xFEE3, 0xFEE4]),
    (0x0646, Joining::Dual, [0xFEE5, 0xFEE6, 0xFEE7, 0xFEE8]),
    (0x0647, Joining::Dual, [0xFEE9, 0xFEEA, 0xFEEB, 0xFEEC]),
    (0x0648, Joining::Right, [0xFEED, 0xFEEE, 0, 0]),
    (0x0649, Joining::Dual, [0xFEEF, 0xFEF0, 0xFBE8, 0xFBE9]),
    (0x064A, Joining::Dual, [0xFEF1, 0xFEF2, 0xFEF3, 0xFEF4]),
    (0x0679, Joining::Dual, [0xFB66, 0xFB67, 0xFB68, 0xFB69]),
    (0x067E, Joining::Dual, [0xFB56, 0xFB57, 0xFB58, 0xFB59]),
    (0x0686, Joining::Dual, [0xFB7A, 0xFB7B, 0xFB7C, 0xFB7D]),
    (0x0688, Joining::Right, [0xFB88, 0xFB89, 0, 0]),
    (0x0691, Joining::Right, [0xFB8C, 0xFB8D, 0, 0]),
    (0x0698, Joining::Right, [0xFB8A, 0xFB8B, 0, 0]),
    (0x06A4, Joining::Dual, [0xFB6A, 0xFB6B, 0xFB6C, 0xFB6D]),
    (0x06A9, Joining::Dual, [0xFB8E, 0xFB8F, 0xFB90, 0xFB91]),
    (0x06AF, Joining::Dual, [0xFB92, 0xFB93, 0xFB94, 0xFB95]),
    (0x06BA, Joining::Dual, [0xFB9E, 0xFB9F, 0, 0]),
    (0x06BE, Joining::Dual, [0xFBAA, 0xFBAB, 0xFBAC, 0xFBAD]),
    (0x06C0, Joining::Right, [0xFBA4, 0xFBA5, 0, 0]),
    (0x06C1, Joining::Dual, [0xFBA6, 0xFBA7, 0xFBA8, 0xFBA9]),
    (0x06CC, Joining::Dual, [0xFBFC, 0xFBFD, 0xFBFE, 0xFBFF]),
    (0x06D2, Joining::Right, [0xFBAE, 0xFBAF, 0, 0]),
    (0x06D3, Joining::Right, [0xFBB0, 0xFBB1, 0, 0]),
];

fn letter(c: char) -> Option<&'static (u32, Joining, [u32; 4])> {
    LETTERS.binary_search_by_key(&u32::from(c), |row| row.0).ok().and_then(|i| LETTERS.get(i))
}

fn is_transparent(c: char) -> bool {
    matches!(u32::from(c),
        0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4 | 0x06E7..=0x06E8 | 0x06EA..=0x06ED)
}

fn joining(c: char) -> Joining {
    if c == ZWJ {
        Joining::Causing
    } else if is_transparent(c) {
        Joining::Transparent
    } else {
        letter(c).map_or(Joining::None, |row| row.1)
    }
}

/// The positional form of every character of `text` (`None` for characters that do not join).
pub(super) fn joining_forms(text: &str) -> Vec<Option<Form>> {
    let kinds: Vec<Joining> = text.chars().map(joining).collect();
    let mut forms = Vec::with_capacity(kinds.len());
    for (i, &kind) in kinds.iter().enumerate() {
        if !matches!(kind, Joining::Dual | Joining::Right) {
            forms.push(None);
            continue;
        }
        let prev = kinds.get(..i).and_then(|k| k.iter().rev().find(|&&p| p != Joining::Transparent));
        let next = kinds.get(i + 1..).and_then(|k| k.iter().find(|&&n| n != Joining::Transparent));
        let from_prev = matches!(prev, Some(Joining::Dual | Joining::Causing));
        let to_next = kind == Joining::Dual && matches!(next, Some(Joining::Dual | Joining::Right | Joining::Causing));
        forms.push(Some(match (from_prev, to_next) {
            (true, true) => Form::Medial,
            (true, false) => Form::Final,
            (false, true) => Form::Initial,
            (false, false) => Form::Isolated,
        }));
    }
    forms
}

/// The presentation-form character for `c` in `form`, if Unicode has one.
pub(super) fn presentation_form(c: char, form: Form) -> Option<char> {
    let forms = letter(c)?.2;
    let code = match form {
        Form::Isolated => forms[0],
        Form::Final => forms[1],
        Form::Initial => forms[2],
        Form::Medial => forms[3],
    };
    if code == 0 { None } else { char::from_u32(code) }
}

/// The lam-alef ligature (U+FEF5..U+FEFC) for a lam in `lam_form` followed by `alef`. A lam joined
/// from the previous letter (medial) takes the final ligature, otherwise the isolated one.
pub(super) fn lam_alef(alef: char, lam_form: Form) -> Option<char> {
    let isolated = match alef {
        '\u{0622}' => 0xFEF5,
        '\u{0623}' => 0xFEF7,
        '\u{0625}' => 0xFEF9,
        '\u{0627}' => 0xFEFB,
        _ => return None,
    };
    match lam_form {
        Form::Initial => char::from_u32(isolated),
        Form::Medial => char::from_u32(isolated + 1),
        Form::Isolated | Form::Final => None,
    }
}

/// True when a font's GSUB feature tags include no Arabic positional feature, which is when
/// HarfBuzz falls back to presentation forms.
pub(super) fn lacks_positional_features(tags: impl IntoIterator<Item = [u8; 4]>) -> bool {
    !tags.into_iter().any(|t| matches!(&t, b"init" | b"medi" | b"fina" | b"isol"))
}

fn face_needs_fallback(font: &skrifa::FontRef<'_>) -> bool {
    let tags: Vec<[u8; 4]> = match font.gsub().and_then(|g| g.feature_list()) {
        Ok(list) => list.feature_records().iter().map(|r| r.feature_tag().to_be_bytes()).collect(),
        Err(_) => Vec::new(),
    };
    lacks_positional_features(tags)
}

/// A shaped cluster of a run, in visual order.
pub(super) struct ClusterIn {
    /// Byte range in the shaped text.
    pub range: Range<usize>,
    /// Number of glyphs the cluster shaped to.
    pub glyphs: usize,
    /// Advance of its first glyph (px).
    pub first_advance: f32,
}

/// Glyph swaps for one run.
pub(super) struct RunPlan {
    /// Per cluster (visual order): the advance change of its first glyph (px, before horizontal
    /// scale), to be folded into the line as kerning after the cluster.
    pub cluster_delta: Vec<f32>,
    /// Per glyph of the run (visual order): the replacement glyph id.
    pub glyph_ids: Vec<Option<u32>>,
}

/// Plans the swaps for a run, or `None` when the font already joins Arabic itself or nothing
/// changes.
pub(super) fn plan_run(font: &FontData, coords: &[i16], size: f32, text: &str, clusters: &[ClusterIn]) -> Option<RunPlan> {
    let has_arabic = clusters.iter().any(|c| text.get(c.range.clone()).is_some_and(|s| s.chars().any(|ch| matches!(u32::from(ch), 0x0600..=0x06FF))));
    if !has_arabic {
        return None;
    }
    let face = skrifa::FontRef::from_index(font.data.as_ref(), font.index).ok()?;
    if !face_needs_fallback(&face) {
        return None;
    }
    let coords: Vec<skrifa::instance::NormalizedCoord> = coords.iter().map(|&c| skrifa::instance::NormalizedCoord::from_bits(c)).collect();
    let location = skrifa::instance::LocationRef::new(&coords);
    let upem = f32::from(face.metrics(skrifa::instance::Size::unscaled(), location).units_per_em).max(1.0);
    let scale = size / upem;
    let charmap = face.charmap();
    let glyph_metrics = face.glyph_metrics(skrifa::instance::Size::unscaled(), location);
    let lookup = |c: char| charmap.map(c).map(|g| (g.to_u32(), glyph_metrics.advance_width(g).unwrap_or(0.0) * scale));

    let forms = joining_forms(text);
    let char_starts: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let form_at = |byte: usize| char_starts.binary_search(&byte).ok().and_then(|ci| forms.get(ci).copied().flatten());

    let mut first_glyph = Vec::with_capacity(clusters.len());
    let mut total = 0usize;
    for c in clusters {
        first_glyph.push(total);
        total = total.saturating_add(c.glyphs);
    }
    let mut plan = RunPlan { cluster_delta: vec![0.0; clusters.len()], glyph_ids: vec![None; total] };
    let mut changed = false;

    // The cluster's joining letter, when it holds exactly one (plus marks); more means the font
    // ligated the cluster itself.
    let single_letter = |c: &ClusterIn| -> Option<char> {
        let s = text.get(c.range.clone())?;
        let mut letters = s.chars().filter(|&ch| !is_transparent(ch));
        let ch = letters.next()?;
        if letters.next().is_some() || c.glyphs == 0 || !s.starts_with(ch) { None } else { Some(ch) }
    };

    for (i, c) in clusters.iter().enumerate() {
        let (Some(ch), Some(form)) = (single_letter(c), form_at(c.range.start)) else { continue };
        let Some((gid, adv)) = presentation_form(ch, form).and_then(lookup) else { continue };
        if let (Some(slot), Some(delta)) = (first_glyph.get(i).and_then(|&g| plan.glyph_ids.get_mut(g)), plan.cluster_delta.get_mut(i)) {
            *slot = Some(gid);
            *delta = adv - c.first_advance;
            changed = true;
        }
    }

    // Lam + alef: the lam's glyph becomes the ligature; the alef's becomes an empty (space) glyph
    // with its advance taken back.
    let Some((space, _)) = lookup(' ') else { return changed.then_some(plan) };
    for (i, lam) in clusters.iter().enumerate() {
        let (Some(LAM), Some(form)) = (single_letter(lam), form_at(lam.range.start)) else { continue };
        for j in [i.wrapping_sub(1), i + 1] {
            let Some(alef) = clusters.get(j) else { continue };
            let Some(alef_ch) = single_letter(alef) else { continue };
            if alef.range.start != lam.range.end || alef.glyphs != 1 || lam.glyphs != 1 {
                continue;
            }
            let Some((lig, lig_adv)) = lam_alef(alef_ch, form).and_then(lookup) else { continue };
            let (Some(&lg), Some(&ag)) = (first_glyph.get(i), first_glyph.get(j)) else { continue };
            if let Some(slot) = plan.glyph_ids.get_mut(lg) {
                *slot = Some(lig);
            }
            if let Some(slot) = plan.glyph_ids.get_mut(ag) {
                *slot = Some(space);
            }
            if let Some(d) = plan.cluster_delta.get_mut(i) {
                *d = lig_adv - lam.first_advance;
            }
            if let Some(d) = plan.cluster_delta.get_mut(j) {
                *d = -alef.first_advance;
            }
            changed = true;
            break;
        }
    }
    changed.then_some(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beh_hah_reh_is_initial_medial_final() {
        assert_eq!(joining_forms("بحر"), [Some(Form::Initial), Some(Form::Medial), Some(Form::Final)]);
    }

    #[test]
    fn right_joining_letters_do_not_join_forward() {
        assert_eq!(joining_forms("اب"), [Some(Form::Isolated), Some(Form::Isolated)]);
        assert_eq!(joining_forms("با"), [Some(Form::Initial), Some(Form::Final)]);
        assert_eq!(joining_forms("لا"), [Some(Form::Initial), Some(Form::Final)]);
    }

    #[test]
    fn marks_are_transparent_and_keep_the_join() {
        assert_eq!(joining_forms("بَح"), [Some(Form::Initial), None, Some(Form::Final)]);
    }

    #[test]
    fn zwnj_breaks_the_join_and_zwj_forces_it() {
        assert_eq!(joining_forms("ب\u{200C}ح"), [Some(Form::Isolated), None, Some(Form::Isolated)]);
        assert_eq!(joining_forms("ب\u{200D}"), [Some(Form::Initial), None]);
        assert_eq!(joining_forms("\u{200D}ب"), [None, Some(Form::Final)]);
    }

    #[test]
    fn non_arabic_text_never_joins() {
        assert_eq!(joining_forms("a ب 1"), [None, None, Some(Form::Isolated), None, None]);
        assert!(joining_forms("").is_empty());
    }

    #[test]
    fn presentation_forms_follow_the_table() {
        assert_eq!(presentation_form('ب', Form::Initial), Some('\u{FE91}'));
        assert_eq!(presentation_form('ب', Form::Medial), Some('\u{FE92}'));
        assert_eq!(presentation_form('ب', Form::Final), Some('\u{FE90}'));
        assert_eq!(presentation_form('ب', Form::Isolated), Some('\u{FE8F}'));
        assert_eq!(presentation_form('ا', Form::Initial), None);
        assert_eq!(presentation_form('a', Form::Isolated), None);
    }

    #[test]
    fn table_is_sorted_and_consistent() {
        assert!(LETTERS.windows(2).all(|w| w[0].0 < w[1].0));
        for &(cp, kind, f) in LETTERS {
            assert!(char::from_u32(cp).is_some());
            if kind == Joining::Right {
                assert_eq!((f[2], f[3]), (0, 0), "{cp:04X}");
            }
        }
    }

    #[test]
    fn lam_alef_ligature_depends_on_the_lam_form() {
        assert_eq!(lam_alef('ا', Form::Initial), Some('\u{FEFB}'));
        assert_eq!(lam_alef('ا', Form::Medial), Some('\u{FEFC}'));
        assert_eq!(lam_alef('أ', Form::Initial), Some('\u{FEF7}'));
        assert_eq!(lam_alef('آ', Form::Medial), Some('\u{FEF6}'));
        assert_eq!(lam_alef('إ', Form::Initial), Some('\u{FEF9}'));
        assert_eq!(lam_alef('ب', Form::Initial), None);
        assert_eq!(lam_alef('ا', Form::Final), None);
    }

    #[test]
    fn positional_features_decide_whether_the_fallback_runs() {
        assert!(lacks_positional_features([*b"liga", *b"rlig"]));
        assert!(lacks_positional_features(std::iter::empty()));
        for tag in [b"init", b"medi", b"fina", b"isol"] {
            assert!(!lacks_positional_features([*b"liga", *tag]));
        }
    }
}
