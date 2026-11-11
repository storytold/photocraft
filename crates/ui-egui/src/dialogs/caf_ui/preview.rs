use super::{CMD, CafWorkspace, Color32, LayerId, MAX_CAF_POINTS, MAX_OUTLINE_EDGES, PhotocraftApp, Rect, TextureHandle, Value, json, stroke_json};

/// What the plan (without the brush strokes) says: the window, the fill area and the sampling
/// option's area, over `rect`.
pub(super) struct Base {
    pub(super) window: Rect,
    pub(super) rect: Rect,
    pub(super) hole: Vec<bool>,
    pub(super) base: Vec<bool>,
    /// The runs `[x0, x1)` of pixels outside the fill area, row by row of `rect` left to right,
    /// row `i`'s at `outside[rows[i]..rows[i + 1]]`: `outside_fill_bounds` looks a row up in
    /// them instead of scanning its pixels.
    outside: Vec<(i32, i32)>,
    rows: Vec<usize>,
}

impl Base {
    pub(super) fn new(window: Rect, rect: Rect, hole: Vec<bool>, base: Vec<bool>) -> Self {
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        let mut outside = Vec::new();
        let mut rows = Vec::with_capacity(h + 1);
        rows.push(0);
        for y in 0..h {
            let mut run: Option<i32> = None;
            for x in 0..w {
                // A pixel `hole` doesn't cover is outside the fill area.
                let out = !hole.get(y * w + x).copied().unwrap_or(false);
                let dx = rect.x0 + x as i32;
                match (out, run) {
                    (true, None) => run = Some(dx),
                    (false, Some(a)) => {
                        outside.push((a, dx));
                        run = None;
                    }
                    _ => {}
                }
            }
            if let Some(a) = run {
                outside.push((a, rect.x1));
            }
            rows.push(outside.len());
        }
        Self { window, rect, hole, base, outside, rows }
    }

    /// The index into `hole` and `base` of document point `(x, y)`, when it is inside `rect`.
    pub(super) fn index(&self, x: f64, y: f64) -> Option<usize> {
        let (xi, yi) = (x.floor() as i32, y.floor() as i32);
        self.rect.contains(xi, yi).then(|| (yi - self.rect.y0) as usize * self.rect.width() as usize + (xi - self.rect.x0) as usize)
    }

    /// Bounds of the pixels of `r` outside the fill area, pixel by pixel (everything beyond
    /// `rect` is outside it); empty when the fill area covers all of `r`. Each row of `r` within
    /// `rect` is looked up in its runs outside the fill area, not scanned pixel by pixel.
    pub(super) fn outside_fill_bounds(&self, r: Rect) -> Rect {
        let inner = r.intersect(&self.rect);
        if inner.is_empty() {
            return if r.is_empty() { Rect::EMPTY } else { r };
        }
        let beyond = [
            Rect::new(r.x0, r.y0, r.x1, inner.y0),
            Rect::new(r.x0, inner.y1, r.x1, r.y1),
            Rect::new(r.x0, inner.y0, inner.x0, inner.y1),
            Rect::new(inner.x1, inner.y0, r.x1, inner.y1),
        ];
        let mut out = beyond.iter().filter(|b| !b.is_empty()).fold(Rect::EMPTY, |o, b| o.union(b));
        for y in inner.y0..inner.y1 {
            let i = (y - self.rect.y0) as usize;
            let runs = &self.outside[self.rows[i]..self.rows[i + 1]];
            // The first run ending past `inner.x0` and the last starting before `inner.x1`.
            let first = runs.partition_point(|&(_, e)| e <= inner.x0);
            let last = runs.partition_point(|&(s, _)| s < inner.x1);
            if first < last {
                out = out.union(&Rect::new(runs[first].0.max(inner.x0), y, runs[last - 1].1.min(inner.x1), y + 1));
            }
        }
        out
    }
}

/// A computed preview: the filled rectangle as an image, or why it failed.
#[cfg(not(target_arch = "wasm32"))]
type PreviewResult = Result<(Rect, egui::ColorImage), String>;

/// The preview being computed or shown.
#[derive(Default)]
pub(super) struct Preview {
    /// Bumped whenever the state changes; a result for an older generation is dropped.
    pub(super) want: u64,
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) running: Option<(u64, std::sync::mpsc::Receiver<PreviewResult>, photocraft_engine::jobs::JobCtx)>,
    pub(super) shown: u64,
    /// A finished preview waiting to be uploaded (on the UI thread, which has the context).
    pub(super) pending_image: Option<(Rect, egui::ColorImage)>,
    pub(super) tex: Option<(TextureHandle, Rect)>,
    /// Screen px per document px (0 = frame the window on the next frame) and the view centre.
    pub(super) zoom: f32,
    pub(super) center: [f64; 2],
    pub(super) error: Option<String>,
}

impl Preview {
    /// Nothing to preview for state `generation` (a Custom area not painted yet): the image is
    /// shown unfilled, with no error.
    fn clear(&mut self, generation: u64) {
        self.shown = generation;
        self.tex = None;
        self.pending_image = None;
        self.error = None;
    }

    /// Takes the result of the preview of state `generation`: an image to upload, or the error,
    /// which stands for that state until it changes (the same preview is not started again).
    fn finish(&mut self, generation: u64, r: Result<(Rect, egui::ColorImage), String>) {
        self.shown = generation;
        self.tex = None;
        match r {
            Ok(img) => {
                self.pending_image = Some(img);
                self.error = None;
            }
            Err(e) => {
                self.pending_image = None;
                self.error = Some(e);
            }
        }
    }
}

/// The preview's error when the sampling area has no pixels outside the fill area (the engine's
/// fill refuses that case with the same words).
const NOTHING_TO_SAMPLE: &str = "nothing to sample from: the sampling area has no pixels outside the fill area";

/// The plan of the current state (without strokes when `strokes` is false).
pub(super) fn plan_params(w: &CafWorkspace, strokes: bool) -> Value {
    let o = &w.opts;
    let mut p = json!({
        "sampling": o.sampling,
        "sampleAllLayers": o.sample_all,
        "colorAdaptation": o.color_adaptation,
        "rotationAdaptation": o.rotation,
        "scale": o.scale,
        "mirror": o.mirror,
        "output": o.output,
        "layer": w.layer.0,
        "fillEdits": w.fill_edits,
    });
    if strokes && !w.strokes.is_empty() {
        p["samplingStrokes"] = Value::Array(w.strokes.iter().map(stroke_json).collect());
    }
    p
}

pub(super) fn points_limit() -> String {
    format!("at most {MAX_CAF_POINTS} points in all sampling strokes and lasso edits: undo some, or reset")
}

/// The plan without strokes: the window, fill area and the option's area. Custom starts empty
/// (the window and fill area are those of Rectangular).
pub(super) fn compute_base(app: &PhotocraftApp, w: &CafWorkspace) -> Result<Base, String> {
    let st = app.session.active().ok_or("no document")?;
    let mut p = plan_params(w, false);
    let custom = w.opts.sampling == "custom";
    if custom {
        p["sampling"] = json!("rectangular");
    }
    let plan = photocraft_engine::edit_menu_cmds::content_aware_plan(&st.doc, w.layer, &p, CMD).map_err(|e| e.to_string())?;
    let base = if custom { vec![false; plan.base.len()] } else { plan.base };
    Ok(Base::new(plan.window, plan.rect, plan.hole, base))
}

/// Unit edges between the fill area and the rest, in document px.
pub(super) fn outline(b: &Base) -> Option<Vec<[[f32; 2]; 2]>> {
    let (w, h) = (b.rect.width() as usize, b.rect.height() as usize);
    let (ox, oy) = (b.rect.x0 as f32, b.rect.y0 as f32);
    let at = |x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && b.hole.get(y as usize * w + x as usize).copied().unwrap_or(false);
    let mut out = Vec::new();
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if !at(x, y) {
                continue;
            }
            let (fx, fy) = (ox + x as f32, oy + y as f32);
            if !at(x - 1, y) {
                out.push([[fx, fy], [fx, fy + 1.0]]);
            }
            if !at(x + 1, y) {
                out.push([[fx + 1.0, fy], [fx + 1.0, fy + 1.0]]);
            }
            if !at(x, y - 1) {
                out.push([[fx, fy], [fx + 1.0, fy]]);
            }
            if !at(x, y + 1) {
                out.push([[fx, fy + 1.0], [fx + 1.0, fy + 1.0]]);
            }
            if out.len() > MAX_OUTLINE_EDGES {
                return None;
            }
        }
    }
    Some(out)
}

/// Recompute the base after a change of the fill area, the sampling option or the document.
pub(super) fn rebase(app: &mut PhotocraftApp) {
    let Some(w) = app.distort.caf.as_ref() else { return };
    let r = compute_base(app, w);
    let Some(w) = app.distort.caf.as_mut() else { return };
    match r {
        Ok(b) => {
            w.set_base(b);
            w.error = None;
        }
        Err(e) => {
            w.error = Some(e);
            w.restate_all();
        }
    }
    w.stale();
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn cancel_preview(w: CafWorkspace) {
    if let Some((_, _, job)) = w.preview.running {
        job.cancel();
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) fn cancel_preview(_: CafWorkspace) {}

/// The document changed under the workspace (an Apply): new textures, a new base.
pub(super) fn refresh(app: &mut PhotocraftApp) {
    let rev = app.session.active().map(|s| s.revision);
    if let Some(w) = app.distort.caf.as_mut() {
        w.image_tex = None;
        w.revision = rev.unwrap_or(w.revision);
    }
    rebase(app);
}

/// Computes the preview now, on this thread (tests and the control channel).
pub(super) fn wait_preview(app: &mut PhotocraftApp) -> Result<(), String> {
    let (doc, layer, p, want, limit) = {
        let w = app.distort.caf.as_mut().ok_or("Content-Aware Fill is not open")?;
        if w.awaiting_custom_area() {
            w.preview.clear(w.preview.want);
            return Ok(());
        }
        let st = app.session.active().ok_or("no document")?;
        (st.doc.clone(), w.layer, plan_params(w, true), w.preview.want, w.texture_limit)
    };
    let r = compute_preview(&doc, layer, &p, &photocraft_raster::Interrupt::NONE, limit);
    let w = app.distort.caf.as_mut().ok_or("Content-Aware Fill is not open")?;
    let err = r.as_ref().err().cloned();
    w.preview.finish(want, r);
    err.map_or(Ok(()), Err)
}

/// The filled rectangle as an image (what Apply would write there), as a texture no wider than
/// `max_side`: past the device's limit it is box-averaged to fit (the rectangle it is drawn over
/// stays the plan's).
fn compute_preview(
    doc: &photocraft_doc::Document,
    layer: LayerId,
    p: &Value,
    ctl: &photocraft_raster::Interrupt,
    max_side: usize,
) -> Result<(Rect, egui::ColorImage), String> {
    let plan = photocraft_engine::edit_menu_cmds::content_aware_plan_with(doc, layer, p, CMD, ctl).map_err(|e| match e {
        photocraft_engine::EngineError::Cancelled => "cancelled".to_string(),
        e => e.to_string(),
    })?;
    // Patches come only from the sampling area outside the fill area. With none (the Sampling
    // Brush erased it all, a Custom area not painted yet) the fill would come out black, so the
    // preview says so instead of showing that.
    if !plan.source.iter().any(|s| *s) {
        return Err(NOTHING_TO_SAMPLE.into());
    }
    let (w, h) = (plan.rect.width() as usize, plan.rect.height() as usize);
    let n = plan.fmt.channels();
    let filled =
        photocraft_algo::content_aware::fill_with(w, h, n, &plan.img, &plan.hole, &plan.source, &plan.opts, ctl).map_err(|_| "cancelled".to_string())?;
    let own = plan.layer_pixels();
    // Pixel `i`: the fill over the fill area (a partly selected pixel by its coverage), the
    // layer's own pixels elsewhere.
    let pixel = |i: usize| -> [f32; 4] {
        let src = if plan.hole[i] { &filled[i * n..(i + 1) * n] } else { &own[i * n..(i + 1) * n] };
        let mut c = photocraft_raster::to_rgba(&plan.fmt, src);
        if plan.hole[i] {
            let k = plan.cover[i];
            let under = photocraft_raster::to_rgba(&plan.fmt, &own[i * n..(i + 1) * n]);
            for j in 0..4 {
                c[j] = under[j] + (c[j] - under[j]) * k;
            }
        }
        c
    };
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let color = |c: [f32; 4]| Color32::from_rgba_unmultiplied(b(c[0]), b(c[1]), b(c[2]), b(c[3]));
    let f = crate::filter_preview_ui::texture_factor(w as u32, h as u32, max_side) as usize;
    if f == 1 {
        let px = (0..w * h).map(|i| color(pixel(i))).collect();
        return Ok((plan.rect, egui::ColorImage::new([w, h], px)));
    }
    let (tw, th) = (w.div_ceil(f), h.div_ceil(f));
    let mut px = Vec::with_capacity(tw * th);
    for ty in 0..th {
        for tx in 0..tw {
            // The block's premultiplied average.
            let (mut acc, mut count) = ([0.0f32; 4], 0.0f32);
            for y in ty * f..((ty + 1) * f).min(h) {
                for x in tx * f..((tx + 1) * f).min(w) {
                    let c = pixel(y * w + x);
                    for j in 0..3 {
                        acc[j] += c[j] * c[3];
                    }
                    acc[3] += c[3];
                    count += 1.0;
                }
            }
            let c = if acc[3] > 0.0 { [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3] / count] } else { [0.0; 4] };
            px.push(color(c));
        }
    }
    Ok((plan.rect, egui::ColorImage::new([tw, th], px)))
}

/// Starts a background preview when the shown one is out of date (and the pointer is up), and
/// collects a finished one.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn drive_preview(app: &mut PhotocraftApp, ctx: &egui::Context, dragging: bool) {
    let doc = app.session.active().map(|s| s.doc.clone());
    let Some(w) = app.distort.caf.as_mut() else { return };
    // Custom with nothing to sample shows the image unfilled straight away, also while a
    // superseded preview winds down (its result is for an older generation and is dropped).
    if w.awaiting_custom_area() {
        w.preview.clear(w.preview.want);
    }
    let pv = &mut w.preview;
    if let Some((generation, rx, _)) = &pv.running {
        match rx.try_recv() {
            Ok(r) => {
                let generation = *generation;
                pv.running = None;
                if generation == pv.want {
                    match r {
                        Err(e) if e == "cancelled" => {}
                        r => pv.finish(generation, r),
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Superseded: stop it, a new one starts when it has gone.
                if *generation != pv.want
                    && let Some((_, _, job)) = &pv.running
                {
                    job.cancel();
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => pv.running = None,
        }
    }
    if pv.running.is_some() || pv.shown == pv.want || dragging {
        return;
    }
    let Some(doc) = doc else { return };
    let (layer, p, generation, limit) = (w.layer, plan_params(w, true), w.preview.want, w.texture_limit);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let job = photocraft_engine::jobs::JobCtx::new();
    let work = job.clone();
    let repaint = ctx.clone();
    let spawned = std::thread::Builder::new().name("caf-preview".into()).spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work.stage(0.0, 1.0, "", |ctl| compute_preview(&doc, layer, &p, ctl, limit))))
            .unwrap_or_else(|_| Err("the preview failed".into()));
        let _ = tx.send(r);
        repaint.request_repaint();
    });
    match spawned {
        Ok(_) => w.preview.running = Some((generation, rx, job)),
        Err(e) => w.preview.error = Some(format!("could not start the preview: {e}")),
    }
}

/// The web build has no threads: the preview is computed when the pointer comes up.
#[cfg(target_arch = "wasm32")]
pub(super) fn drive_preview(app: &mut PhotocraftApp, _ctx: &egui::Context, dragging: bool) {
    // Custom with nothing to sample shows the image unfilled straight away, also while the
    // pointer is still down on a stroke that erased it all.
    if let Some(w) = app.distort.caf.as_mut()
        && w.awaiting_custom_area()
    {
        w.preview.clear(w.preview.want);
    }
    let stale = app.distort.caf.as_ref().is_some_and(|w| w.preview.shown != w.preview.want);
    if stale && !dragging {
        let _ = wait_preview(app);
    }
}
