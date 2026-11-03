//! Split into Layers (since 2026-11-02): SAM 3.1 finds what the
//! picture is of, each instance becomes its own layer with the picture's pixels and the
//! detector's (edge-refined) coverage as alpha, and the background under them is repainted by
//! the generative removal, so every layer is whole. On the owner's portrait this split 5 of 5
//! fill seeds cleanly in about 4.6 s on the RTX 5090, where Qwen-Image-Layered, which this
//! replaced, separated 5 of 10 seeds at about 12 s (the rest left the picture whole, often as
//! an opaque copy on top) and none of its authors' sampler settings did better (measured
//! 2026-11-02; the owner chose SAM alone).

use std::collections::BTreeMap;
use std::sync::Arc;

use photocraft_genai::GenerativeBackend;
use photocraft_geom::Rect;

use crate::EngineError;

use super::{
    Common, DEFAULT_MATTE_TEMPLATE, GenerativeInfo, Gray8, JobCtx, JobProgress, Layer, MATTE_DEFAULT_PROMPT, MAX_GROW, REMOVE_BG, REMOVE_MIN_REQUEST, Request,
    Result, Rgba8, SPLIT, Session, Task, Tuning, Value, at_least, embed_coverage, fill_rect, fill_region, gen_err, json, matte_request_size, name_selected,
    plan_common, removal_job, removal_prompt, remove_grow, resize_gray8, resize_rgba8, resolve_auto, set_generative_info, write_rgba8,
};

/// The most subjects one split gives their own layer (the largest kept).
pub(super) const MAX_SUBJECTS: usize = 7;

pub(super) struct SubjectsPlan {
    /// What to cut out (`prompt`; none: Select Subject's prompts in turn).
    pub prompt: Option<String>,
    /// At most this many subject layers (`layers` - 1, the background being one).
    pub subjects: usize,
    pub seed: Option<u64>,
    pub name: Option<String>,
    pub edges: Edges,
}

/// Where the subjects' soft edges (hair) come from (`edges`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Edges {
    /// `auto`: ViTMatte when the server has it, else the matte when it agrees with the
    /// detector, else `refine`.
    Auto,
    /// `vitmatte`: ViTMatte inside a band around the detector's mask (hair strand by strand),
    /// else as `auto`.
    Vitmatte,
    /// `matte`: Remove Background's matte model around the detector's mask, else `refine`.
    Matte,
    /// `refine`: the detector's hard mask, edge refined from the pixels (Select Subject's).
    Refine,
    /// `hard`: the detector's hard mask as it is.
    Hard,
}

impl Edges {
    pub(super) fn parse(v: Option<&str>) -> std::result::Result<Edges, String> {
        Ok(match v.map(str::trim).unwrap_or("auto") {
            "" | "auto" => Edges::Auto,
            "vitmatte" => Edges::Vitmatte,
            "matte" => Edges::Matte,
            "refine" => Edges::Refine,
            "hard" => Edges::Hard,
            other => return Err(format!("`edges` must be auto, vitmatte, matte, refine or hard (got `{other}`)")),
        })
    }

    fn name(self) -> &'static str {
        match self {
            Edges::Auto | Edges::Vitmatte => "vitmatte",
            Edges::Matte => "matte",
            Edges::Refine => "refine",
            Edges::Hard => "hard",
        }
    }
}

pub(super) fn run(s: &mut Session, plan: SubjectsPlan) -> Result<Value> {
    let SubjectsPlan { prompt, subjects, seed, name, edges } = plan;
    let finder = crate::select_ml_cmds::subject_finder(s, prompt.as_deref())?;
    let finder = finder.at_most(subjects.max(1));
    let asked = prompt.clone();
    let job =
        removal_job(s, "Split into Layers", 0, false, false, 2, seed, true, Tuning { fast_draft: s.prefs().integrations.fast_draft, ..Tuning::default() })?;
    let base = name.unwrap_or_else(|| "Split Layer".to_string());
    // The matte request (Remove Background's model), planned now; used when the server has it.
    let matte = plan_common(
        s,
        REMOVE_BG,
        &json!({"prompt": MATTE_DEFAULT_PROMPT, "template": DEFAULT_MATTE_TEMPLATE, "seed": seed}),
        DEFAULT_MATTE_TEMPLATE,
        Task::Matte,
        "",
    )
    .ok();
    crate::jobs::edit_job(
        s,
        "Split into Layers",
        move |doc, active, ctx| {
            let started = std::time::Instant::now();
            ctx.progress(0.0, "Finding subjects");
            let area = doc.bounds();
            let (w, h) = (area.width(), area.height());
            let (mut instances, found_by) = finder.instances(doc, *active, true, ctx)?;
            instances.truncate(subjects.max(1));
            ctx.check()?;
            // The picture as it is, for the subjects' pixels.
            let picture = photocraft_compose::render(doc, area).to_rgba8().pixels;
            // The hole: every subject, all or nothing, as the Remove tool's.
            let mut hole = vec![0.0f32; w as usize * h as usize];
            for (cov, _, _) in &instances {
                for (d, c) in hole.iter_mut().zip(cov) {
                    // Soft masks fade out: their faint fringe is still the subject's.
                    if *c > HOLE_MIN {
                        *d = 1.0;
                    }
                }
            }
            let (hole_rect, _) = crate::select_ml_cmds::coverage_bounds(&hole, area);
            let mut job = job;
            // The removal names what it removes ("Remove the woman…"): the 2.1 route is an
            // edit model with no noise mask, and told only "the object" it changed nothing (the
            // woman stayed in the background, measured in the app 2026-11-02). The prompt
            // names it when given, else the vision-language model does from the hole's views,
            // else the detector's prompt.
            // A person is named "person": the vision-language model named one by their clothes
            // ("the blue tank top") and the edit model, told to remove the top, kept the person
            // and repainted their body on every seed (measured in the app 2026-11-03).
            let name = match asked.as_deref() {
                Some(p) => Some(p.to_string()),
                None if subject_left_in(&finder.only(PERSON), doc, *active, &hole, ctx)? => Some(PERSON.to_string()),
                None => name_selected(doc, &job, area, &hole, ctx)?,
            };
            let name = name.unwrap_or_else(|| found_by.clone());
            let name = name.trim().strip_prefix("the ").unwrap_or(name.trim()).to_string();
            ctx.check()?;
            job.rewritten = Some(removal_prompt(&name));
            job.object = Some(name.clone());
            job.grow = remove_grow(hole_rect);
            let rect = fill_rect(hole_rect.inflate(job.grow.min(MAX_GROW) as i32).intersect(&area), job.tuning.remove_margin, area)?;
            let rect = at_least(rect, REMOVE_MIN_REQUEST, area);
            // Each subject's alpha: ViTMatte inside a band around the detector's mask (hair,
            // strand by strand), else the matte model's soft alpha around it, else the mask with
            // the edge refined from the pixels, as Select Subject does (or as `edges` asks).
            let backend = Arc::clone(&job.backend);
            let mut used = edges;
            let mut alphas = None;
            if matches!(edges, Edges::Auto | Edges::Vitmatte) {
                ctx.progress(0.05, "Matting hair");
                alphas = crate::select_ml_cmds::vitmatte_alphas(backend.as_ref(), &picture, area, &instances, ctx)?;
                used = Edges::Vitmatte;
            }
            ctx.check()?;
            if alphas.is_none()
                && matches!(edges, Edges::Auto | Edges::Vitmatte | Edges::Matte)
                && let Some(m) = &matte
            {
                ctx.progress(0.05, "Matting edges");
                alphas = matte_alphas(backend.as_ref(), m, &picture, (w, h), &instances, &name, ctx)?;
                used = Edges::Matte;
            }
            ctx.check()?;
            let alphas: Vec<Vec<f32>> = match alphas {
                Some(a) => a,
                None if used != Edges::Hard => {
                    used = Edges::Refine;
                    ctx.progress(0.3, "Refining edges");
                    instances.iter().map(|(cov, _, _)| crate::select_ml_cmds::refine_coverage(doc, *active, true, cov.clone(), area)).collect()
                }
                None => instances.iter().map(|(cov, _, _)| cov.clone()).collect(),
            };
            let template_id = job.template_id.clone();
            let below = *active;
            // The background: the hole filled, then checked with the detector. A fill that paints
            // the subject back (on a 12-megapixel portrait the edit model kept the head and
            // repainted the body, every seed tried, measured in the app 2026-11-03) is thrown
            // away and tried on the next seed; none clean, and the split fails with nothing
            // changed rather than hand back a repainted person.
            let mut ms = 0u64;
            let mut fill = None;
            for attempt in 0..FILL_TRIES {
                ctx.progress(0.35 + 0.5 * attempt as f32 / FILL_TRIES as f32, "Filling the background");
                let mut try_job = job.clone();
                try_job.seed = job.seed.wrapping_add(u64::from(attempt) * 1000);
                let (made, ..) = fill_region(doc, active, ctx, try_job, rect, embed_coverage(area, &hole, rect), None)?;
                ctx.check()?;
                ms += made.iter().map(|m| m.3).sum::<u64>();
                // The background whole: the picture with the fill over the hole, one layer.
                let background = photocraft_compose::render(doc, area).to_rgba8().pixels;
                let fill_seed = made.first().map_or(0, |m| m.1);
                ctx.progress(0.35 + 0.5 * (attempt as f32 + 0.9) / FILL_TRIES as f32, "Checking the background");
                let left = subject_left_in(&finder.only(check_prompt(&found_by)), doc, *active, &hole, ctx)?;
                for (id, ..) in &made {
                    doc.remove(*id);
                }
                *active = below;
                if !left {
                    fill = Some((background, fill_seed));
                    break;
                }
            }
            let Some((background, fill_seed)) = fill else {
                return Err(EngineError::Other(format!(
                    "Split into Layers could not clear the {} out of the background: the fill painted it back on {FILL_TRIES} tries, so nothing was changed",
                    check_prompt(&found_by).trim_start_matches("the ")
                )));
            };
            ctx.progress(0.95, "Placing");
            let count = 1 + instances.len();
            let mut layers: Vec<(Rgba8, String)> = Vec::with_capacity(count);
            layers.push((Rgba8::new(w, h, background).map_err(gen_err)?, format!("{base} 1/{count}")));
            // The largest subject lowest: the furthest back, as a rule.
            for (i, alpha) in alphas.iter().enumerate() {
                let px: Vec<u8> = picture
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(alpha)
                    .flat_map(|(p, a)| [p[0], p[1], p[2], (f32::from(p[3]) * a.clamp(0.0, 1.0)).round() as u8])
                    .collect();
                layers.push((Rgba8::new(w, h, px).map_err(gen_err)?, format!("{base} {}/{count}", i + 2)));
            }
            let mut ids = Vec::with_capacity(count);
            for (pixels, layer_name) in layers {
                let mut layer = Layer::raster(layer_name.clone(), doc.pixel_format());
                write_rgba8(&mut layer, &pixels, area)?;
                set_generative_info(
                    &mut layer,
                    &GenerativeInfo {
                        command: SPLIT.to_string(),
                        prompt: found_by.clone(),
                        template: template_id.clone(),
                        seed: fill_seed,
                        rect: [area.x0, area.y0, w as i32, h as i32],
                        name: layer_name,
                        ..GenerativeInfo::default()
                    },
                );
                let nid = doc.insert_above(*active, layer);
                *active = Some(nid);
                ids.push(nid);
            }
            Ok((ids, found_by, template_id, fill_seed, ms, started.elapsed().as_millis() as u64, (w, h), used.name()))
        },
        move |(ids, found_by, template_id, seed, fill_ms, total_ms, (w, h), matted)| {
            json!({
                "layers": ids.iter().map(|l| l.0).collect::<Vec<_>>(), "count": ids.len(),
                "subjects": ids.len().saturating_sub(1), "foundBy": found_by, "template": template_id, "seed": seed,
                "width": w, "height": h, "fillMs": fill_ms, "ms": total_ms, "edges": matted,
            })
        },
    )
}

/// What the detector is asked for on the filled background: the prompt that found the
/// subjects when it names a kind of thing ("person", the caller's prompt), else "person". The
/// generic prompts ("the main subject") find something on any picture: on a clean backdrop
/// they rejected all three fills (measured in the app 2026-11-03), and the failure this check
/// is for is a person painted back.
fn check_prompt(found_by: &str) -> &str {
    match found_by {
        "the main subject" | "the foreground object" | "object" => PERSON,
        p => p,
    }
}

/// The detector's and the removal's word for a person.
const PERSON: &str = "person";

/// Fills tried before Split gives up on a background with the subject painted back.
const FILL_TRIES: u32 = 3;

/// Of a detection on the filled background, the share inside the hole at which it counts as
/// the subject painted back (a person beside the hole overlaps it little).
const LEFT_IN_HOLE: f32 = 0.5;

/// Whether the detector (asked for what found the subjects) still finds a subject in the hole
/// of the picture as it is now (the fill over it): a detection mostly inside the hole and at
/// least a quarter of the hole's size. Nothing found is a clean fill.
fn subject_left_in(
    finder: &crate::select_ml_cmds::SubjectFinder,
    doc: &photocraft_doc::Document,
    active: Option<photocraft_doc::LayerId>,
    hole: &[f32],
    ctx: &JobCtx,
) -> Result<bool> {
    let holed = hole.iter().filter(|v| **v >= 0.5).count();
    if holed == 0 {
        return Ok(false);
    }
    let found = match finder.instances(doc, active, true, ctx) {
        Ok((found, _)) => found,
        Err(EngineError::Other(e)) if e.starts_with("nothing") => return Ok(false),
        Err(e) => return Err(e),
    };
    Ok(found.iter().any(|(cov, _, _)| {
        let (mut on, mut inside) = (0usize, 0usize);
        for (c, h) in cov.iter().zip(hole) {
            if *c >= 0.5 {
                on += 1;
                if *h >= 0.5 {
                    inside += 1;
                }
            }
        }
        on > 0 && inside as f32 >= LEFT_IN_HOLE * on as f32 && inside * 4 >= holed
    }))
}

/// The subjects' alphas from the matte model (Remove Background's), or `None` when the server
/// lacks it or its matte disagrees with the detector (kept little of the subjects, or much of
/// the rest). The detector's masks are hard-edged and clip hair (the owner, 2026-11-02); the
/// matte is soft alpha. Each subject takes the matte within a band around its own mask (a
/// fraction of its size, so stray hair stays and other things the matte kept do not), its
/// mask's interior whole, and a pixel near two subjects goes to the nearer.
fn matte_alphas(
    backend: &dyn GenerativeBackend,
    plan: &Common,
    picture: &[u8],
    (w, h): (u32, u32),
    instances: &[crate::select_ml_cmds::Instance],
    name: &str,
    ctx: &JobCtx,
) -> Result<Option<Vec<Vec<f32>>>> {
    if resolve_auto(backend, &[DEFAULT_MATTE_TEMPLATE], "") != DEFAULT_MATTE_TEMPLATE || instances.is_empty() {
        return Ok(None);
    }
    let wu = w as usize;
    let image = Rgba8::new(w, h, picture.to_vec()).map_err(gen_err)?;
    let (rw, rh) = matte_request_size(w, h);
    let image = if (rw, rh) == (w, h) { image } else { resize_rgba8(&image, rw, rh)? };
    let req = Request {
        template: DEFAULT_MATTE_TEMPLATE.to_string(),
        prompt: name.to_string(),
        negative: plan.negative.clone(),
        seed: plan.seed,
        steps: plan.steps,
        guidance: plan.guidance,
        image: Some(image),
        mask: None,
        models: plan.models.clone(),
        size: None,
        params: BTreeMap::new(),
        references: Vec::new(),
    };
    // A failed matte is not a failed split: the refined masks still serve.
    let Ok(resp) = backend.run(&req, &JobProgress::span(ctx, 0.05, 0.3)) else { return Ok(None) };
    let Some(out) = resp.images.into_iter().next() else { return Ok(None) };
    let plane = Gray8::new(out.width, out.height, out.data.as_chunks::<4>().0.iter().map(|p| p[3]).collect()).map_err(gen_err)?;
    let plane = if (plane.width, plane.height) == (w, h) { plane } else { resize_gray8(&plane, w, h)? };
    let matte: Vec<f32> = plane.data.iter().map(|v| f32::from(*v) / 255.0).collect();
    // Per subject, within a window around it: the distance to its mask (outside) and into it
    // (inside), and how far the matte decides (`reach`).
    struct Near {
        win: Rect,
        outside: Vec<f32>,
        inside: Vec<f32>,
        reach: f32,
    }
    let area = Rect::new(0, 0, w as i32, h as i32);
    let mut near = Vec::with_capacity(instances.len());
    for (cov, bounds, _) in instances {
        let reach = (bounds.width().max(bounds.height()) as f32 * BAND).max(MIN_BAND);
        let win = bounds.inflate(reach.ceil() as i32 + 2).intersect(&area);
        let (ww, wh) = (win.width() as usize, win.height() as usize);
        let mut on = Vec::with_capacity(ww * wh);
        for y in win.y0..win.y1 {
            let row = y as usize * wu;
            on.extend(cov[row + win.x0 as usize..row + win.x1 as usize].iter().map(|c| *c >= 0.5));
        }
        let off: Vec<bool> = on.iter().map(|b| !*b).collect();
        let (outside, inside) = (photocraft_algo::selection::edt(&on, ww, wh), photocraft_algo::selection::edt(&off, ww, wh));
        if outside.len() != ww * wh || inside.len() != ww * wh {
            return Ok(None);
        }
        near.push(Near { win, outside, inside, reach });
    }
    // The nearest subject's distance at each pixel; infinite where none is near.
    let mut nearest = vec![f32::INFINITY; matte.len()];
    for n in &near {
        let ww = n.win.width() as usize;
        for (j, d) in n.outside.iter().enumerate() {
            let k = (n.win.y0 as usize + j / ww) * wu + n.win.x0 as usize + j % ww;
            if *d < n.reach {
                nearest[k] = nearest[k].min(*d);
            }
        }
    }
    let mut alphas = vec![vec![0.0f32; matte.len()]; instances.len()];
    let (mut kept, mut core) = (0.0f64, 0usize);
    for (n, alpha) in near.iter().zip(alphas.iter_mut()) {
        let ww = n.win.width() as usize;
        for (j, (d_out, d_in)) in n.outside.iter().zip(&n.inside).enumerate() {
            let k = (n.win.y0 as usize + j / ww) * wu + n.win.x0 as usize + j % ww;
            if *d_out >= n.reach || (*d_out > 0.0 && *d_out > nearest[k] + 0.5) {
                continue;
            }
            if *d_in > n.reach * 0.5 {
                // Deep inside the mask: solid, whatever the matte made of it.
                alpha[k] = 1.0;
                kept += f64::from(matte[k]);
                core += 1;
            } else {
                alpha[k] = matte[k] * ((n.reach - d_out) / (n.reach * 0.5)).clamp(0.0, 1.0);
            }
        }
    }
    let (mut stray, mut rest) = (0.0f64, 0usize);
    for (m, d) in matte.iter().zip(&nearest) {
        if d.is_infinite() {
            stray += f64::from(*m);
            rest += 1;
        }
    }
    // The matte must agree with the detector: most of the subjects' interiors kept, little of
    // what lies well away from them.
    let kept_ok = core == 0 || kept / core as f64 >= MIN_KEPT;
    let stray_ok = rest == 0 || stray / rest as f64 <= MAX_STRAY;
    Ok((kept_ok && stray_ok).then_some(alphas))
}

/// Soft-mask coverage above this is part of the hole the removal repaints.
const HOLE_MIN: f32 = 0.1;

/// The band around a subject's mask the matte decides, as a fraction of its larger side...
const BAND: f32 = 0.04;
/// ...and at least this many pixels.
const MIN_BAND: f32 = 6.0;
/// The matte's least mean over the subjects' interiors to be trusted.
const MIN_KEPT: f64 = 0.6;
/// The matte's most mean over the rest (well away from every subject) to be trusted.
const MAX_STRAY: f64 = 0.2;

#[cfg(test)]
mod check_tests {
    use super::check_prompt;

    #[test]
    fn the_fill_check_asks_for_a_kind_of_thing() {
        assert_eq!(check_prompt("the main subject"), "person");
        assert_eq!(check_prompt("the foreground object"), "person");
        assert_eq!(check_prompt("object"), "person");
        assert_eq!(check_prompt("person"), "person");
        assert_eq!(check_prompt("the lighthouse"), "the lighthouse");
    }
}
