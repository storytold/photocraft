//! Split into Layers (since 2026-11-02): SAM 3.1 finds what the
//! picture is of, each instance becomes its own layer with the picture's pixels and the
//! detector's (edge-refined) coverage as alpha, and the background under them is repainted by
//! the generative removal, so every layer is whole. On the owner's portrait this split 5 of 5
//! fill seeds cleanly in about 4.6 s on the RTX 5090, where Qwen-Image-Layered, which this
//! replaced, separated 5 of 10 seeds at about 12 s (the rest left the picture whole, often as
//! an opaque copy on top) and none of its authors' sampler settings did better (measured
//! 2026-11-02; the owner chose SAM alone).

use super::*;

/// The most subjects one split gives their own layer (the largest kept).
pub(super) const MAX_SUBJECTS: usize = 7;

pub(super) struct SubjectsPlan {
    /// What to cut out (`prompt`; none: Select Subject's prompts in turn).
    pub prompt: Option<String>,
    /// At most this many subject layers (`layers` - 1, the background being one).
    pub subjects: usize,
    pub seed: Option<u64>,
    pub name: Option<String>,
}

pub(super) fn run(s: &mut Session, plan: SubjectsPlan) -> Result<Value> {
    let SubjectsPlan { prompt, subjects, seed, name } = plan;
    let finder = crate::select_ml_cmds::subject_finder(s, prompt.as_deref())?;
    let job = removal_job(s, "Split into Layers", 0, false, false, 2, seed, true, Tuning { fast_draft: s.prefs().integrations.fast_draft, ..Tuning::default() })?;
    let base = name.unwrap_or_else(|| "Split Layer".to_string());
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
            // Each subject's alpha: the detector's mask with the edge refined from the pixels
            // (hair, soft edges), as Select Subject does.
            ctx.progress(0.05, "Refining edges");
            let alphas: Vec<Vec<f32>> = instances.iter().map(|(cov, _, _)| crate::select_ml_cmds::refine_coverage(doc, *active, true, cov.clone(), area)).collect();
            // The hole: every subject, all or nothing, as the Remove tool's.
            let mut hole = vec![0.0f32; w as usize * h as usize];
            for (cov, _, _) in &instances {
                for (d, c) in hole.iter_mut().zip(cov) {
                    if *c > 0.0 {
                        *d = 1.0;
                    }
                }
            }
            let (hole_rect, _) = crate::select_ml_cmds::coverage_bounds(&hole, area);
            let mut job = job;
            job.grow = remove_grow(hole_rect);
            let rect = fill_rect(hole_rect.inflate(job.grow.min(MAX_GROW) as i32).intersect(&area), job.tuning.remove_margin, area)?;
            let rect = at_least(rect, REMOVE_MIN_REQUEST, area);
            let template_id = job.template_id.clone();
            let below = *active;
            ctx.progress(0.1, "Filling the background");
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
                    .chunks_exact(4)
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
            Ok((ids, found_by, template_id, fill_seed, ms, started.elapsed().as_millis() as u64, (w, h)))
        },
        move |(ids, found_by, template_id, seed, fill_ms, total_ms, (w, h))| {
            json!({
                "layers": ids.iter().map(|l| l.0).collect::<Vec<_>>(), "count": ids.len(),
                "subjects": ids.len().saturating_sub(1), "foundBy": found_by, "template": template_id, "seed": seed,
                "width": w, "height": h, "fillMs": fill_ms, "ms": total_ms,
            })
        },
    )
}
