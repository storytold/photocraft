//! Place in resumable steps (#10031): decode, flatten, resize and translate, each in pieces small
//! enough to run between two frames where there are no worker threads (the web build).
//!
//! Every piece computes what the one-shot code computes, so the placed layer is the same whether
//! a worker thread, a [`PlaceSteps`] driven to the end, or a frame-by-frame [`Session::poll_jobs`]
//! did the work. `fine` (a cooperative session) cuts the work into pieces; without it each stage
//! is one unit and native code keeps its parallel resize and flatten.

use photocraft_algo::resample::ResizeSteps;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_io::PngImport;
use photocraft_raster::Surface;
use web_time::Instant;

use super::open::{PLACE_EMBEDDED, PlaceInput, Prepared};
use super::paths::{file_name, stem};
use super::{
    Affine, Arc, Document, EngineError, LayerContent, Resample, Result, SmartObject, SmartSource, Value, check_flatten_size, f64_param, flattened_checked,
    resize_surface, translate_surface,
};
use crate::jobs::{JobCtx, StepWork};

/// Pixels a flatten band covers at most: four tiles of a tile row (about 5 ms on one core).
const BAND_PIXELS: i64 = 4 * (TILE_SIZE as i64) * (TILE_SIZE as i64);

/// A place as a [`StepWork`]; its result is what [`super::open::apply_place`] inserts.
pub(crate) struct PlaceSteps {
    input: PlaceInput,
    /// Cut the work into small pieces (for a session without threads).
    fine: bool,
    vector: bool,
    stage: Stage,
    /// Where the placed image lands, known once it is decoded.
    plan: Plan,
}

#[derive(Clone, Copy, Default)]
struct Plan {
    w: f64,
    h: f64,
    scale: f64,
    dx: f64,
    dy: f64,
}

enum Stage {
    /// Nothing done yet.
    Decode,
    /// A plain PNG, a few rows at a time.
    DecodePng(Box<PngImport>),
    Flatten(Box<Flatten>),
    Resize(Box<ResizeSteps>),
    /// The fitted pixels, to move into place and wrap in a layer.
    Finish(Surface),
    Spent,
}

impl PlaceSteps {
    /// `fine` cuts the work into pieces of a few milliseconds, as a session without worker
    /// threads needs; without it each stage is one unit of work.
    pub(crate) fn new(input: PlaceInput, fine: bool) -> Self {
        let vector = photocraft_io::svg::is_svg(&input.bytes);
        Self { input, fine, vector, stage: Stage::Decode, plan: Plan::default() }
    }

    fn io_error(&self, e: photocraft_io::IoError) -> EngineError {
        match e {
            photocraft_io::IoError::Cancelled => EngineError::Cancelled,
            e => EngineError::Other(format!("{}: {e}", self.input.name)),
        }
    }

    /// One unit of work; `Some` when the place is prepared.
    fn advance(&mut self, ctx: &JobCtx, deadline: Instant) -> Result<Option<Prepared>> {
        match std::mem::replace(&mut self.stage, Stage::Spent) {
            Stage::Decode => {
                let input = &self.input;
                let png = if self.fine { PngImport::new(&input.name, &input.bytes).map_err(|e| self.io_error(e))? } else { None };
                match png {
                    Some(png) => self.stage = Stage::DecodePng(Box::new(png)),
                    None => {
                        let imported = ctx
                            .stage(0.0, 0.6, "Decoding", |ctl| photocraft_io::import_with_options_and_interrupt(&input.name, &input.bytes, &input.opts, ctl))
                            .map_err(|e| self.io_error(e))?;
                        self.decoded(imported, false, ctx)?;
                    }
                }
            }
            Stage::DecodePng(mut png) => {
                ctx.progress(0.55 * png.progress(), "Decoding");
                match png.step(&mut || Instant::now() < deadline).map_err(|e| self.io_error(e))? {
                    Some(imported) => self.decoded(imported, true, ctx)?,
                    None => self.stage = Stage::DecodePng(png),
                }
            }
            Stage::Flatten(mut flatten) => {
                ctx.progress(0.6 + 0.1 * flatten.progress(), "Flattening");
                match flatten.advance()? {
                    Some(px) => self.flattened(px, ctx),
                    None => self.stage = Stage::Flatten(flatten),
                }
            }
            Stage::Resize(mut resize) => {
                ctx.progress(0.7 + 0.2 * resize.progress(), "Resizing");
                self.stage = match resize.step(&mut || Instant::now() < deadline) {
                    Some(px) => Stage::Finish(px),
                    None => Stage::Resize(resize),
                };
            }
            Stage::Finish(px) => return self.finish(px, ctx).map(Some),
            Stage::Spent => return Err(EngineError::Other("internal error: the place was already prepared".into())),
        }
        Ok(None)
    }

    /// The file is decoded: where it lands, then on to flattening. `banded`: the document is a
    /// plain image, which flattens band by band.
    fn decoded(&mut self, imported: photocraft_io::ImportResult, banded: bool, ctx: &JobCtx) -> Result<()> {
        if imported.preview_only {
            return Err(EngineError::Other(format!(
                "{}: only this Affinity file's embedded preview could be read; open it with File › Open to see the warning, or export PSD or PNG from Affinity before using it here",
                self.input.name
            )));
        }
        let src = imported.document;
        let (w, h) = (src.size.width as f64, src.size.height as f64);
        let (cw, ch) = self.input.canvas;
        let p = &self.input.params;
        let scale = match f64_param(p, "scale") {
            Some(k) => (k / 100.0).max(1e-4),
            None if self.input.fit && (w > cw || h > ch) => (cw / w).min(ch / h),
            None => 1.0,
        };
        let center = match p.get("center").and_then(Value::as_array) {
            Some(c) if c.len() == 2 => (c[0].as_f64().unwrap_or(cw / 2.0), c[1].as_f64().unwrap_or(ch / 2.0)),
            _ => (cw / 2.0, ch / 2.0),
        };
        let (dx, dy) = ((center.0 - w * scale / 2.0).round(), (center.1 - h * scale / 2.0).round());
        self.plan = Plan { w, h, scale, dx, dy };
        ctx.progress(0.6, "Flattening");
        self.stage = Stage::Flatten(Box::new(Flatten::new(src, banded && self.fine, self.input.fmt)?));
        Ok(())
    }

    /// The image is flattened: on to resizing it to fit, if it needs that.
    fn flattened(&mut self, px: Surface, ctx: &JobCtx) {
        let scale = self.plan.scale;
        self.stage = if (scale - 1.0).abs() <= 1e-9 {
            Stage::Finish(px)
        } else if self.fine {
            Stage::Resize(Box::new(ResizeSteps::new(px, scale, scale, Resample::Bicubic, None)))
        } else {
            ctx.progress(0.7, "Resizing");
            Stage::Finish(resize_surface(&px, scale, scale, Resample::Bicubic))
        };
    }

    /// Moves the pixels into place and wraps them in the layer's content.
    fn finish(&mut self, px: Surface, ctx: &JobCtx) -> Result<Prepared> {
        let Plan { w, h, scale, dx, dy } = self.plan;
        let input = &self.input;
        let px = translate_surface(&px, dx as i32, dy as i32);
        let label = if input.linked.is_some() { "Place Linked" } else { PLACE_EMBEDDED };
        let content = if input.smart {
            let source = match &input.linked {
                Some(path) => SmartSource::Linked { path: path.clone() },
                None => SmartSource::Embedded { file_name: file_name(&input.name), bytes: Arc::clone(&input.bytes) },
            };
            let mut so = SmartObject::new(source, Affine { m: [scale, 0.0, 0.0, scale, dx, dy] }, Some(px));
            // A vector source (SVG) renders at its placement scale instead of as a resampled raster.
            if self.vector {
                ctx.progress(0.9, "Rendering");
                ctx.check()?;
                if let Some(sharp) = crate::smart_cmds::render(&input.doc, &so)? {
                    so.cache = Some(sharp);
                }
            }
            LayerContent::Smart(so)
        } else {
            LayerContent::Raster(px)
        };
        Ok(Prepared { content, layer_name: stem(&input.name), label, scale, bounds: [dx, dy, dx + w * scale, dy + h * scale], smart: input.smart })
    }
}

impl StepWork<Prepared> for PlaceSteps {
    fn step(&mut self, ctx: &JobCtx, deadline: Instant) -> Result<Option<Prepared>> {
        loop {
            ctx.check()?;
            if let Some(prepared) = self.advance(ctx, deadline)? {
                return Ok(Some(prepared));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }
}

/// The decoded document being flattened to the surface it is resized from.
pub(super) struct Flatten {
    doc: Document,
    fmt: photocraft_color::PixelFormat,
    /// Flatten band by band; otherwise in one go.
    banded: bool,
    bounds: Rect,
    /// The next band's top left corner.
    at: (i32, i32),
    /// Width of a band, a whole number of tiles.
    cols: i32,
    out: Option<Surface>,
}

impl Flatten {
    pub(super) fn new(doc: Document, banded: bool, fmt: photocraft_color::PixelFormat) -> Result<Self> {
        check_flatten_size(&doc, "file.place")?;
        let bounds = doc.bounds();
        // Bands hold whole tiles, so each tile is written by one band and can move as it is. A
        // document that does not start on the tile grid (none does) flattens in one go.
        let banded = banded && bounds.x0.rem_euclid(TILE_SIZE) == 0 && bounds.y0.rem_euclid(TILE_SIZE) == 0;
        let cols = i32::try_from(BAND_PIXELS / i64::from(TILE_SIZE) / i64::from(TILE_SIZE)).unwrap_or(1).max(1) * TILE_SIZE;
        Ok(Self { doc, fmt, banded, bounds, at: (bounds.x0, bounds.y0), cols, out: None })
    }

    /// How much of the flatten is done, `0.0..=1.0`.
    fn progress(&self) -> f32 {
        let rows = i64::from(self.bounds.height()).max(1);
        (i64::from(self.at.1.saturating_sub(self.bounds.y0)) as f32 / rows as f32).clamp(0.0, 1.0)
    }

    /// Flattens the next band (the whole document if not banded); `Some` with the surface once
    /// the last one is in.
    pub(super) fn advance(&mut self) -> Result<Option<Surface>> {
        if !self.banded {
            return flattened_checked(&self.doc, self.fmt, "file.place").map(Some);
        }
        let (x, y) = self.at;
        // A band is part of one tile row, at most `cols` wide.
        let y1 = (y.div_euclid(TILE_SIZE).saturating_add(1).saturating_mul(TILE_SIZE)).min(self.bounds.y1);
        let x1 = x.saturating_add(self.cols).min(self.bounds.x1);
        let band = photocraft_compose::composite_to_surface(&self.doc, Rect::new(x, y, x1, y1), self.fmt, None);
        // The band's tiles move into the result as they are, with no copy of their pixels.
        let out = self.out.get_or_insert_with(|| Surface::with_default(self.fmt, &band.default_pixel()));
        out.put_tiles(band.tiles().map(|(c, t)| (*c, Arc::clone(t))));
        self.at = if x1 >= self.bounds.x1 { (self.bounds.x0, y1) } else { (x1, y) };
        Ok(if self.at.1 >= self.bounds.y1 { Some(self.out.take().unwrap_or_else(|| Surface::new(self.fmt))) } else { None })
    }
}
