//! Dynamic Text: type that sizes itself to fill its box or area shape, like Photoshop's
//! Convert to Dynamic Text. The fit itself runs in `type_cmds::refresh`, so every edit of a
//! fitted layer refits it in the same history step.

use photocraft_doc::text::{Orientation, TextDirection, TextFit, TextShape};
use photocraft_doc::{LayerContent, TextLayer};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::type_extra_cmds::{point_to_box, with_text};
use crate::{EngineError, Result, Session};

const CONVERT: &str = "type.convertToDynamicText";
const RELEASE: &str = "type.releaseDynamicText";
const SET: &str = "type.setDynamicText";
const NOT_FITTED: &str = "the type layer is not Dynamic Text";
const CONDENSED: &str = "Release Dynamic Text can't keep condensed type (Horizontal Scale below 100%) yet; set it to 100% or more first";

fn bad(cmd: &str, m: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: m.into() }
}

/// Why `t` cannot be fitted, or `None`. The solver does not check these itself: vertical type,
/// right-to-left paragraphs (set so, or Auto with a right-to-left first strong character) and
/// path text are refused here, and `refresh` skips them.
pub fn fit_refusal(t: &TextLayer) -> Option<&'static str> {
    if t.orientation == Orientation::Vertical {
        return Some("Dynamic Text does not support vertical type");
    }
    let mut at = 0usize;
    let rtl = t.paragraph_runs().iter().any(|p| {
        let range = at..at.saturating_add(p.len);
        at = range.end;
        match p.style.direction {
            TextDirection::Rtl => true,
            TextDirection::Ltr => false,
            TextDirection::Auto => t.text.get(range).and_then(photocraft_text::layout::first_strong_rtl) == Some(true),
        }
    });
    if rtl {
        return Some("Dynamic Text does not support right-to-left text");
    }
    if matches!(t.shape, TextShape::Path { .. }) {
        return Some("Dynamic Text does not support type on a path");
    }
    None
}

/// The one guard that keeps an edit from breaking Dynamic Text: a layer that was fitted before
/// the edit (`was_fitted`, read before the edit changed it) and is still fitted after it must
/// still be fittable box or area type. Otherwise the edit fails as `cmd`, and the caller's
/// `Session::edit` discards it with no history step. Convert and Release pass: before Convert
/// the layer was not fitted, after Release it no longer is.
pub(crate) fn guard_fit(was_fitted: bool, t: &TextLayer, cmd: &str) -> Result<()> {
    if !was_fitted || t.fit.is_none() {
        return Ok(());
    }
    let why = fit_refusal(t).or_else(|| (!matches!(t.shape, TextShape::Box { .. } | TextShape::Area { .. })).then_some("Dynamic Text needs box or area type"));
    match why {
        Some(why) => Err(bad(cmd, &format!("Release Dynamic Text first ({why})"))),
        None => Ok(()),
    }
}

/// The running command's id for [`guard_fit`]; `fallback` (the edit's label) when the helper
/// runs outside `Session::execute`, as in a direct engine call.
pub(crate) fn guard_cmd<'a>(s: &Session, fallback: &'a str) -> &'a str {
    s.running_command.unwrap_or(fallback)
}

fn active_text(s: &Session) -> std::result::Result<&TextLayer, String> {
    let d = s.active().ok_or("no document open")?;
    if d.selected_layers().len() > 1 {
        return Err("select one type layer".into());
    }
    match d.active_layer.and_then(|id| d.doc.layer(id)).map(|l| &l.content) {
        Some(LayerContent::Text(t)) => Ok(t),
        _ => Err("the active layer is not a type layer".into()),
    }
}

fn can_convert(s: &Session) -> std::result::Result<(), String> {
    let t = active_text(s)?;
    if t.fit.is_some() {
        return Err("the type layer is already Dynamic Text".into());
    }
    fit_refusal(t).map_or(Ok(()), |m| Err(m.into()))
}

fn has_fit(s: &Session) -> std::result::Result<(), String> {
    if active_text(s)?.fit.is_some() { Ok(()) } else { Err(NOT_FITTED.into()) }
}

/// Dynamic Text works on one layer: an explicit `"layers"` list, or several selected layers
/// with no `"layer"`, is refused.
fn single_layer(s: &Session, cmd: &str, p: &Value) -> Result<()> {
    let several = "Dynamic Text works on one type layer at a time";
    if p.get("layers").is_some() {
        return Err(bad(cmd, several));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if p.get("layer").is_none() && st.selected_layers().len() > 1 {
        return Err(bad(cmd, several));
    }
    Ok(())
}

fn convert(s: &mut Session, p: &Value) -> Result<Value> {
    single_layer(s, CONVERT, p)?;
    let boxed = with_text(s, p, "Convert to Dynamic Text", |t, dpi| {
        if t.fit.is_some() {
            return Err(bad(CONVERT, "the type layer is already Dynamic Text"));
        }
        if let Some(m) = fit_refusal(t) {
            return Err(bad(CONVERT, m));
        }
        let boxed = matches!(t.shape, TextShape::Point).then(|| point_to_box(t, dpi));
        // `with_text` re-renders through `refresh`, which runs the fit in this same step.
        t.fit = Some(TextFit::default());
        Ok(boxed)
    })?;
    let mut out = json!({"fit": true});
    if let Some(b) = boxed {
        out["box"] = b;
    }
    Ok(out)
}

fn release(s: &mut Session, p: &Value) -> Result<Value> {
    single_layer(s, RELEASE, p)?;
    let keep_look = match p.get("keepLook") {
        None | Some(Value::Null) => true,
        Some(v) => v.as_bool().ok_or_else(|| bad(RELEASE, "\"keepLook\" must be true or false"))?,
    };
    with_text(s, p, "Release Dynamic Text", |t, dpi| {
        if t.fit.is_none() {
            return Err(bad(RELEASE, NOT_FITTED));
        }
        // Photoshop's toggle off: the text goes back to its own sizes, which box and area text
        // keep while fitted. Only area text saved by the uniform-fit build holds fitted sizes
        // (`area_scale`); they are scaled back.
        if !keep_look {
            let area_scale = t.fit.take().map_or(1.0, |f| f.area_scale);
            if area_scale.is_finite() && area_scale > 0.0 && area_scale != 1.0 {
                photocraft_text::fit::apply_scale(t, 1.0 / area_scale);
            }
            return Ok(());
        }
        // The fitted look becomes ordinary type: a size and a break per line. Layers the solver
        // never fitted (vertical, right-to-left) have no solved lines to bake.
        if fit_refusal(t).is_some() {
            t.fit = None;
        } else {
            let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
            let baked = photocraft_text::fit::bake(&mut eng, t, dpi).map_err(|e| match e {
                photocraft_text::fit::BakeError::Condensed => bad(RELEASE, CONDENSED),
                e => bad(RELEASE, &format!("the fitted look cannot be kept as ordinary type: {e}")),
            })?;
            *t = baked;
        }
        Ok(())
    })?;
    Ok(json!({"fit": false}))
}

/// A gap param: absent (or null) keeps the current gap; otherwise a number in
/// `min..=GAP_MAX` percent (`min` differs per key).
fn gap(p: &Value, key: &str, min: f32) -> Result<Option<f32>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let range = f64::from(min)..=f64::from(TextFit::GAP_MAX);
            let n = v.as_f64().filter(|n| range.contains(n));
            n.map(|n| Some(n as f32)).ok_or_else(|| bad(SET, &format!("\"{key}\" must be a number from {} to {} percent", min, TextFit::GAP_MAX)))
        }
    }
}

fn set_gaps(s: &mut Session, p: &Value) -> Result<Value> {
    single_layer(s, SET, p)?;
    let (word, line) = (gap(p, "wordGap", TextFit::WORD_GAP_MIN)?, gap(p, "lineGap", TextFit::LINE_GAP_MIN)?);
    if word.is_none() && line.is_none() {
        return Err(bad(SET, "give wordGap or lineGap"));
    }
    let fit = with_text(s, p, "Dynamic Text Options", |t, _| {
        let fit = t.fit.as_mut().ok_or_else(|| bad(SET, NOT_FITTED))?;
        if let Some(w) = word {
            fit.word_gap = w;
        }
        if let Some(l) = line {
            fit.line_gap = l;
        }
        Ok(fit.clone())
    })?;
    Ok(json!({"wordGap": fit.word_gap, "lineGap": fit.line_gap}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: CONVERT,
            label: "Convert to Dynamic Text",
            menu: &["Type"],
            shortcut: None,
            params: r##"{"layer":id? (default active Type layer)} → the text sizes itself to fill its box or area shape on every edit (point text becomes box text first); not for vertical, right-to-left or path text"##,
            enabled: can_convert,
            journal: true,
            run: convert,
        },
        CommandSpec {
            id: RELEASE,
            label: "Release Dynamic Text",
            menu: &["Type"],
            shortcut: None,
            params: r##"{"layer":id? (default active Type layer),"keepLook":bool? (default true; false = back to the text's own sizes, as Photoshop's toggle does)} → fit off; with keepLook the text keeps its look as ordinary type (a size and a line break per line), a box and an area alike"##,
            enabled: has_fit,
            journal: true,
            run: release,
        },
        CommandSpec {
            id: SET,
            label: "Dynamic Text Options",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id? (default active Type layer),"wordGap":-100..200? (percent of each space's own width, added to it: -100 removes the spaces),"lineGap":-50..200? (percent of the type size, added to each line advance)} → refits"##,
            enabled: has_fit,
            journal: true,
            run: set_gaps,
        },
    ]
}

#[cfg(test)]
#[path = "type_fit_cmds/tests.rs"]
mod tests;
