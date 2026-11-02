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
    /// `auto`: the matte when the server has it and it agrees with the detector, else `refine`.
    Auto,
    /// `matte`: as `auto` (the name says what was asked for).
    Matte,
    /// `soft[:t]`: the detector's own soft mask (its logits through a sigmoid at temperature t).
    Soft(f32),
    /// `refine`: the detector's hard mask, edge refined from the pixels (Select Subject's).
    Refine,
    /// `hair`: as `auto`, plus the strands past the edge where the detector finds hair: the
    /// Refine Edge Brush painted along that edge (Photoshop's Refine Hair) estimates their
    /// coverage from the colours, and only what it adds outside the subject's mask is kept.
    Hair,
    /// `hard`: the detector's hard mask as it is.
    Hard,
}

impl Edges {
    pub(super) fn parse(v: Option<&str>) -> std::result::Result<Edges, String> {
        let v = v.map(str::trim).unwrap_or("auto");
        Ok(match v {
            "" | "auto" => Edges::Auto,
            "matte" => Edges::Matte,
            "refine" => Edges::Refine,
            "hair" => Edges::Hair,
            "hard" => Edges::Hard,
            "soft" => Edges::Soft(DEFAULT_SOFT),
            _ => match v.strip_prefix("soft:").and_then(|t| t.parse::<f32>().ok()).filter(|t| *t > 0.0 && *t <= 64.0) {
                Some(t) => Edges::Soft(t),
                None => return Err(format!("`edges` must be auto, matte, soft, soft:<0..64>, refine, hair or hard (got `{v}`)")),
            },
        })
    }

    fn name(self) -> &'static str {
        match self {
            Edges::Auto | Edges::Matte => "matte",
            Edges::Soft(_) => "soft",
            Edges::Refine => "refine",
            Edges::Hair => "hair",
            Edges::Hard => "hard",
        }
    }
}

/// The detector's soft-edge temperature when `soft` names none.
const DEFAULT_SOFT: f32 = 1.0;

pub(super) fn run(s: &mut Session, plan: SubjectsPlan) -> Result<Value> {
    let SubjectsPlan { prompt, subjects, seed, name, edges } = plan;
    let finder = crate::select_ml_cmds::subject_finder(s, prompt.as_deref())?;
    let finder = finder.at_most(subjects.max(1));
    let finder = if let Edges::Soft(t) = edges { finder.soft_edges(f64::from(t)) } else { finder };
    // Hair, for the Refine Edge Brush (`hair`); none found is no hair to refine.
    let hair_finder = match edges {
        Edges::Hair => Some(crate::select_ml_cmds::subject_finder(s, Some(HAIR_PROMPT))?.at_most(MAX_SUBJECTS)),
        _ => None,
    };
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
            let name = match asked.as_deref() {
                Some(p) => Some(p.to_string()),
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
            // Each subject's alpha: the matte model's soft alpha around the detector's mask
            // (hair), else the detector's mask with the edge refined from the pixels, as Select
            // Subject does (or as `edges` asks).
            let backend = Arc::clone(&job.backend);
            let matted = match (&matte, edges) {
                (Some(m), Edges::Auto | Edges::Matte | Edges::Hair) => {
                    ctx.progress(0.05, "Matting edges");
                    matte_alphas(backend.as_ref(), m, &picture, (w, h), &instances, &name, ctx)?
                }
                _ => None,
            };
            ctx.check()?;
            let edge_base = match (&matted, edges) {
                (Some(_), _) => Edges::Matte,
                (None, Edges::Auto | Edges::Matte | Edges::Hair) => Edges::Refine,
                (None, e) => e,
            };
            let mut alphas: Vec<Vec<f32>> = match matted {
                Some(a) => a,
                None if edge_base == Edges::Refine => {
                    ctx.progress(0.3, "Refining edges");
                    instances.iter().map(|(cov, _, _)| crate::select_ml_cmds::refine_coverage(doc, *active, true, cov.clone(), area)).collect()
                }
                None => instances.iter().map(|(cov, _, _)| cov.clone()).collect(),
            };
            // Refine Hair: strands past the edge, added to what the edge already has (painting
            // the brush over the hair itself let the colour estimate punch holes in it and eat
            // the jaw beside it, measured in the app 2026-11-02).
            let mut used = edge_base;
            if let Some(f) = &hair_finder {
                ctx.progress(0.32, "Finding hair");
                let hair = match f.instances(doc, *active, true, ctx) {
                    Ok((found, _)) => {
                        let mut u = vec![0.0f32; w as usize * h as usize];
                        for (cov, _, _) in &found {
                            for (d, c) in u.iter_mut().zip(cov) {
                                *d = d.max(*c);
                            }
                        }
                        Some(u)
                    }
                    Err(e) if ctx.cancelled() => return Err(e),
                    Err(_) => None,
                };
                if let Some(hair) = hair {
                    ctx.progress(0.34, "Refining hair");
                    for ((cov, bounds, _), alpha) in instances.iter().zip(alphas.iter_mut()) {
                        let Some((brush, near)) = hair_brush(cov, &hair, area, *bounds) else { continue };
                        let strands = crate::select_ml_cmds::refine_coverage_brushed(doc, *active, true, cov.clone(), area, Some(&brush));
                        for (((a, s), n), c) in alpha.iter_mut().zip(&strands).zip(&near).zip(cov) {
                            if *n && *c < 0.5 {
                                *a = a.max(*s);
                            }
                        }
                    }
                    used = Edges::Hair;
                }
            }
            let template_id = job.template_id.clone();
            let below = *active;
            ctx.progress(0.35, "Filling the background");
            let (made, ..) = fill_region(doc, active, ctx, job, rect, embed_coverage(area, &hole, rect), None)?;
            ctx.check()?;
            ctx.progress(0.95, "Placing");
            // The background whole: the picture with the fill over the hole, one layer.
            let background = photocraft_compose::render(doc, area).to_rgba8().pixels;
            let fill_seed = made.first().map_or(0, |m| m.1);
            let ms = made.iter().map(|m| m.3).sum::<u64>();
            for (id, ..) in &made {
                doc.remove(*id);
            }
            *active = below;
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

/// What the detector is asked for to find hair.
const HAIR_PROMPT: &str = "hair";

/// The Refine Edge Brush Photoshop's Refine Hair paints: dabs along the subject's edge where it
/// meets hair, wide enough to reach the strands beyond it (a fraction of the subject's size),
/// and where the strands it finds may count (near the hair). `None` when the subject has no
/// edge in hair.
fn hair_brush(subject: &[f32], hair: &[f32], area: Rect, bounds: Rect) -> Option<(photocraft_algo::matting::EdgeBrush, Vec<bool>)> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    if subject.len() != w * h || hair.len() != w * h {
        return None;
    }
    let reach = (bounds.width().max(bounds.height()) as f32 * HAIR_BAND).max(MIN_HAIR_BAND);
    // Near hair: within half the reach of the detector's hair (its mask stops short of strands).
    let on: Vec<bool> = hair.iter().map(|c| *c >= 0.5).collect();
    if !on.iter().any(|b| *b) {
        return None;
    }
    let to_hair = photocraft_algo::selection::edt(&on, w, h);
    // One dab per cell of half the reach along the subject's edge.
    let step = (reach * 0.5).max(1.0) as usize;
    let mut cells = std::collections::BTreeSet::new();
    let mut strokes = Vec::new();
    let inside = |x: usize, y: usize| subject[y * w + x] >= 0.5;
    let win = bounds.inflate(1).intersect(&area);
    for y in win.y0.max(0) as usize..(win.y1 as usize).min(h) {
        for x in win.x0.max(0) as usize..(win.x1 as usize).min(w) {
            if !inside(x, y) {
                continue;
            }
            let edge = x == 0 || y == 0 || x + 1 == w || y + 1 == h || !inside(x - 1, y) || !inside(x + 1, y) || !inside(x, y - 1) || !inside(x, y + 1);
            if !edge || to_hair.get(y * w + x).is_none_or(|d| *d > reach * 0.5) {
                continue;
            }
            if cells.insert((x / step, y / step)) {
                strokes.push(photocraft_algo::matting::EdgeStroke {
                    points: vec![(area.x0 as f32 + x as f32 + 0.5, area.y0 as f32 + y as f32 + 0.5)],
                    size: reach * 2.0,
                    add: true,
                });
            }
        }
    }
    if strokes.is_empty() {
        return None;
    }
    // Where strands may be added: within the reach of the detector's hair.
    let near = to_hair.iter().map(|d| *d <= reach).collect();
    Some((photocraft_algo::matting::EdgeBrush::new(&strokes, area)?, near))
}

/// The hair band's half-width, as a fraction of the subject's larger side...
const HAIR_BAND: f32 = 0.03;
/// ...and at least this many pixels.
const MIN_HAIR_BAND: f32 = 8.0;
