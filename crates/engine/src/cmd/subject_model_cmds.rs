//! Select Subject and Remove Background on a learned model (see [`crate::subject_model`]): which
//! method a command uses, reading the picture for the model, and the command that downloads it.

use photocraft_algo::segment::Sampler;
use photocraft_algo::selection::Region;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::jobs::JobCtx;
use crate::subject_model::{ModelError, ModelStatus, SharedSubjectModel, SubjectImage};
use crate::{EngineError, Result, Session};

/// The command that downloads the model. Only a person agrees to its terms, so automation is
/// refused it (`photocraft_automation::workspace`).
pub const INSTALL_CMD: &str = "select.subjectModel.install";

/// Rows read at a time, so the picture is never held as floats in full.
const STRIP_ROWS: i32 = 256;

/// Largest picture handed to the model, in pixels (its bytes are held three times over).
const MAX_PIXELS: u64 = 400_000_000;

/// The model `p` asks for: `"method": "auto"` (the default) takes it when it is ready, `"model"`
/// fails if it is not, and `"classic"` is always the built-in method.
pub(crate) fn pick(s: &Session, cmd: &str, p: &Value) -> Result<Option<SharedSubjectModel>> {
    Ok(pick_named(s, cmd, p)?.map(|(model, _)| model))
}

/// [`pick`] with the model's name, which tells one model's answers from another's.
pub(crate) fn pick_named(s: &Session, cmd: &str, p: &Value) -> Result<Option<(SharedSubjectModel, String)>> {
    let method = p.get("method").and_then(Value::as_str).unwrap_or("auto");
    if !matches!(method, "auto" | "model" | "classic") {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("method must be auto, model or classic, not \"{method}\"") });
    }
    if method == "classic" {
        return Ok(None);
    }
    let Some(model) = s.subject_model() else {
        return if method == "model" { Err(EngineError::Other("this build has no subject model".into())) } else { Ok(None) };
    };
    match model.status() {
        ModelStatus::Ready { model: name } => Ok(Some((model, name))),
        _ if method == "auto" => Ok(None),
        ModelStatus::Offer(offer) => Err(EngineError::Other(format!("{} is not downloaded yet", offer.model))),
        ModelStatus::Unavailable { reason } => Err(EngineError::Other(reason)),
    }
}

/// `value` (0–1) as a byte.
fn byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Reads `area` for the model, transparent pixels over mid gray. Only the mask comes back, so
/// the model sees the colours the built-in method sees.
fn read(sampler: &dyn Sampler, area: Rect, ctx: &JobCtx) -> Result<SubjectImage> {
    let (width, height) = (area.width(), area.height());
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels == 0 || pixels > MAX_PIXELS {
        return Err(EngineError::Other(format!("the subject model can't read a picture of {width} × {height} pixels")));
    }
    let mut rgb = Vec::with_capacity(pixels as usize * 3);
    let mut y = area.y0;
    while y < area.y1 {
        ctx.check()?;
        let y1 = y.saturating_add(STRIP_ROWS).min(area.y1);
        for [r, g, b, a] in sampler.rgba(Rect::new(area.x0, y, area.x1, y1)) {
            let over = 0.5 * (1.0 - a);
            rgb.extend([byte(r * a + over), byte(g * a + over), byte(b * a + over)]);
        }
        y = y1;
    }
    Ok(SubjectImage { width, height, rgb })
}

/// Finds the subject in `area` with `model`. `None` when the model sees nothing there.
pub(crate) fn find(model: &SharedSubjectModel, sampler: &dyn Sampler, area: Rect, ctx: &JobCtx) -> Result<Option<Region>> {
    let image = read(sampler, area, ctx)?;
    let mask = model.matte(&image, &|| ctx.cancelled()).map_err(job_error)?;
    if mask.len() as u64 != u64::from(image.width) * u64::from(image.height) {
        return Err(EngineError::Other("the subject model answered with a mask of the wrong size".into()));
    }
    Ok(mask.iter().any(|v| *v >= 128).then(|| tight(area, &mask)))
}

/// [`find`] over `layer`'s pixels in `doc` (or the composite), read in the document's CMYK profile:
/// the job runs on a worker thread, where no document's profile is active.
pub(crate) fn find_in(
    model: &SharedSubjectModel,
    doc: &photocraft_doc::Document,
    layer: Option<photocraft_doc::LayerId>,
    all_layers: bool,
    area: Rect,
    ctx: &JobCtx,
) -> Result<Option<Region>> {
    let cmyk = photocraft_compose::cmyk_space(doc);
    photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
        crate::smartselect_cmds::with_doc_sampler(doc, layer, all_layers, |smp, _| find(model, smp, area, ctx))
    })
}

/// `mask` over `area`, cut down to the pixels it covers. `mask` covers something.
fn tight(area: Rect, mask: &[u8]) -> Region {
    let width = area.width() as usize;
    let covered = |row: &&[u8]| row.iter().any(|v| *v > 0);
    let rows: Vec<&[u8]> = mask.chunks_exact(width).collect();
    let top = rows.iter().position(covered).unwrap_or(0);
    let bottom = rows.iter().rposition(covered).map_or(rows.len(), |y| y + 1);
    let kept = rows.get(top..bottom).unwrap_or(&[]);
    let left = kept.iter().filter_map(|row| row.iter().position(|v| *v > 0)).min().unwrap_or(0);
    let right = kept.iter().filter_map(|row| row.iter().rposition(|v| *v > 0)).max().map_or(width, |x| x + 1);
    let mask = kept.iter().flat_map(|row| row.get(left..right).unwrap_or(&[]).iter().copied()).collect();
    Region { bbox: Rect::new(area.x0 + left as i32, area.y0 + top as i32, area.x0 + right as i32, area.y0 + bottom as i32), mask }
}

/// The model's answer as the engine's error: a cancel stays a cancel.
fn job_error(e: ModelError) -> EngineError {
    match e {
        ModelError::Cancelled => EngineError::Cancelled,
        ModelError::Failed(message) => EngineError::Other(message),
    }
}

/// Whether the download can start: only while the model is offered. The error says why not.
fn enabled(s: &Session) -> std::result::Result<(), String> {
    match s.subject_model().map(|m| m.status()) {
        Some(ModelStatus::Offer(_)) => Ok(()),
        Some(ModelStatus::Ready { model }) => Err(format!("{model} is already set up")),
        Some(ModelStatus::Unavailable { reason }) => Err(reason),
        None => Err("this build has no subject model".into()),
    }
}

/// `select.subjectModel.install`: downloads the model as a job that locks the document, after
/// `acceptTerms: true`.
fn install(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("acceptTerms").and_then(Value::as_bool) != Some(true) {
        return Err(EngineError::BadParams { cmd: INSTALL_CMD.into(), msg: "the download needs acceptTerms: true, after a person read the terms".into() });
    }
    enabled(s).map_err(EngineError::Other)?;
    let model = s.subject_model().ok_or_else(|| EngineError::Other("this build has no subject model".into()))?;
    // Locks the document: that is what shows the progress window, and the command waiting for the
    // model goes on with this document.
    crate::jobs::run(
        s,
        "Download Subject Model",
        true,
        move |ctx| model.install(&|fraction, message| ctx.progress(fraction, message), &|| ctx.cancelled()).map_err(job_error),
        |_, ()| Ok(json!({ "installed": true })),
    )
}

/// The install command spec (no menu path: Select Subject and Remove Background offer it).
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: INSTALL_CMD,
        label: "Download Subject Model",
        menu: &[],
        shortcut: None,
        params: r##"{"acceptTerms":bool} → {installed} (downloads the model Select Subject and Remove Background use; a person must have agreed to its terms)"##,
        enabled,
        run: install,
        journal: false,
    }]
}

#[cfg(test)]
mod tests;
