use super::paths::{join, sanitize, stem, write_file};
use super::save::{SaveOpts, encode};
use super::{ColorMode, Document, EngineError, Layer, LayerContent, LayerId, Rect, Result, Session, Surface, Value, from_rgba_into, json, str_param};

#[cfg(test)]
#[path = "layers_to_files_index_tests.rs"]
mod layers_to_files_index_tests;

// ---------- Export ----------

/// Layers to Files requires another layer when the document only contains a Background.
pub fn layers_to_files_needs_more_layers(doc: &Document) -> bool {
    doc.layers.len() == 1 && doc.background().is_some()
}

pub(super) fn layers_to_files(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.layersToFiles";
    let dir = str_param(p, "dir", cmd).or_else(|_| str_param(p, "output", cmd))?.to_string();
    let report = export_layers_report_with(s, p, |name, bytes| write_file(&join(&dir, name), bytes))?;
    let files: Vec<String> = report.files.iter().map(|name| join(&dir, name)).collect();
    Ok(json!({"files": files, "skipped": report.skipped}))
}

/// The bodies of `file.export.layersToFiles`, streamed to `write` as a single-component filename
/// plus its bytes: one file per exported layer (top to bottom, like Photoshop's numbering),
/// encoded but never touched as a path. Automation writes exactly these bytes beneath its write
/// root (#3216), so the two paths cannot drift. Returns the filenames written, in order.
pub fn export_layers_with(s: &Session, p: &Value, write: impl FnMut(&str, &[u8]) -> Result<()>) -> Result<Vec<String>> {
    export_layers_report_with(s, p, write).map(|report| report.files)
}

/// A top-level layer omitted from a successful Layers to Files export.
#[derive(Debug, serde::Serialize)]
pub struct SkippedExportLayer {
    /// Session layer id (names need not be unique).
    pub layer: u64,
    /// Original name, not the sanitized filename component.
    pub name: String,
    /// Why the layer produced no file.
    pub reason: &'static str,
}

/// Filenames written and layers skipped, each in top-to-bottom order.
#[derive(Debug, Default)]
pub struct LayerExportReport {
    pub files: Vec<String>,
    pub skipped: Vec<SkippedExportLayer>,
}

/// The same export as [`export_layers_with`], with an explanation for every omitted top-level
/// layer. Groups export merged, so their children are not separate exports or skips. Encoding
/// and write failures remain errors, never successful skips. Existing numbering is preserved.
pub fn export_layers_report_with(s: &Session, p: &Value, mut write: impl FnMut(&str, &[u8]) -> Result<()>) -> Result<LayerExportReport> {
    let cmd = "file.export.layersToFiles";
    let format = p.get("format").and_then(Value::as_str).unwrap_or("png").trim_start_matches('.').to_ascii_lowercase();
    // The format becomes the file extension inside one filename component handed to `write`.
    if format.is_empty() || !format.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "`format` must be a file extension such as png".into() });
    }
    let trim = p.get("trim").and_then(Value::as_bool).unwrap_or(false);
    let visible_only = p.get("visibleOnly").and_then(Value::as_bool).unwrap_or(true);
    // Callers outside `Session::execute` (headless workspace exports) carry the session's gate
    // into `encode`, so a PSD embeds no file a linked smart object names.
    let _authorization = crate::jobs::render_authorization_scope(s.authorize);
    let save = SaveOpts::from_params(s, p, cmd)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    if layers_to_files_needs_more_layers(&d.doc) {
        return Err(EngineError::Other("Layers to Files needs a document with multiple layers.".into()));
    }
    let prefix = p.get("prefix").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stem(&d.doc.name));
    let doc = d.doc.clone();
    let mut report = LayerExportReport::default();
    // Every layer takes its panel-order index, even one skipped as hidden or as an adjustment,
    // so a file keeps the same number whether Visible Layers Only is on or off (#13959).
    for (index, l) in doc.layers.iter().rev().enumerate() {
        if !l.visible && visible_only {
            report.skipped.push(SkippedExportLayer { layer: l.id.0, name: l.name.clone(), reason: "hidden by visibleOnly" });
            continue;
        }
        if matches!(l.content, LayerContent::Adjustment(_)) {
            report.skipped.push(SkippedExportLayer { layer: l.id.0, name: l.name.clone(), reason: "adjustment layer has no pixels of its own" });
            continue;
        }
        let mut one = (*doc).clone();
        let mut only = l.clone();
        only.visible = true;
        only.clipped = false;
        one.layers = vec![only].into();
        if trim {
            let mut bounds = Rect::EMPTY;
            let _ = photocraft_compose::render_bands(&one, one.bounds(), 0, |band| -> std::result::Result<(), ()> {
                let width = band.rect.width() as usize;
                for (i, rgba) in band.px.iter().enumerate() {
                    if rgba[3] > 0.0 && width > 0 {
                        let x = band.rect.x0 + (i % width) as i32;
                        let y = band.rect.y0 + (i / width) as i32;
                        bounds = bounds.union(&Rect::new(x, y, x + 1, y + 1));
                    }
                }
                Ok(())
            });
            if !bounds.is_empty() && bounds != one.bounds() {
                crate::image_cmds::crop_doc(&mut one, cmd, bounds, true, [0.0; 4])?;
            }
        }

        // PNG file names use dashes for spaces (measured); the other formats keep them.
        let part = |text: &str| if format == "png" { sanitize(text).replace(' ', "-") } else { sanitize(text) };
        // The prefix is optional; only the layer name needs the empty-name fallback.
        let prefix_part = if prefix.is_empty() { String::new() } else { part(&prefix) };
        let name = format!("{prefix_part}_{:04}_{}.{format}", index, part(&l.name));
        let (bytes, _warnings) = encode(&one, &name, save)?;
        write(&name, &bytes)?;
        report.files.push(name);
    }
    Ok(report)
}

/// Adjustment layers that can be represented as an RGB-only LUT, in document stack order.
/// A LUT has no spatial coordinates, so masks and clipping geometry are deliberately stripped
/// at bake time. Group adjustments need their enclosing group's compositing semantics and are
/// not silently flattened as independent layers.
fn lut_layers<'a>(doc: &'a Document, ids: Option<&[LayerId]>) -> Result<Vec<&'a Layer>> {
    const CMD: &str = "file.export.colorLookupTables";
    let mut selected = Vec::new();
    if let Some(ids) = ids {
        if ids.is_empty() {
            return Err(EngineError::BadParams { cmd: CMD.into(), msg: "select at least one adjustment layer".into() });
        }
        for id in ids {
            if selected.contains(id) {
                return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("duplicate layer id {}", id.0) });
            }
            let Some(layer) = doc.layer(*id) else { return Err(EngineError::NoLayer(*id)) };
            if !matches!(layer.content, LayerContent::Adjustment(_)) {
                return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("layer {} is not an adjustment layer", id.0) });
            }
            if !layer.visible {
                return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("layer {} is hidden", id.0) });
            }
            if !doc.layers.iter().any(|root| root.id == *id) {
                return Err(EngineError::BadParams {
                    cmd: CMD.into(),
                    msg: format!("layer {} is nested in a group; export top-level adjustment layers", id.0),
                });
            }
            selected.push(*id);
        }
    }
    let mut layers = Vec::new();
    for l in &doc.layers {
        if l.visible && matches!(l.content, LayerContent::Adjustment(_)) && (ids.is_none() || selected.contains(&l.id)) {
            layers.push(l);
        }
    }
    if layers.is_empty() {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "no visible adjustment layers to export".into() });
    }
    Ok(layers)
}

/// Bake the chosen adjustment stack against an identity RGB lattice.
/// Source order always follows the document stack, even if an explicit list is reversed.
fn bake_cube_layers(size: usize, title: &str, layers: &[&Layer], depth: photocraft_color::SampleType) -> String {
    let n = size;
    let (w, h) = ((n * n) as u32, n as u32);
    let mut lattice = Document::new("lut", photocraft_doc::Size::new(w, h), ColorMode::Rgb, photocraft_color::SampleType::F32);
    let fmt = lattice.pixel_format();
    let step = 1.0 / (n - 1) as f32;
    let mut data = Vec::with_capacity((w * h) as usize * fmt.channels());
    for g in 0..n {
        for b in 0..n {
            for r in 0..n {
                let mut px = [0.0f32; 8];
                let k = from_rgba_into(&fmt, [r as f32 * step, g as f32 * step, b as f32 * step, 1.0], &mut px);
                data.extend_from_slice(&px[..k]);
            }
        }
    }
    let mut surf = Surface::new(fmt);
    surf.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
    lattice.layers.push(Layer::new("Lattice", LayerContent::Raster(surf)));
    for l in layers {
        let mut a = (*l).clone();
        // Masks and clipping are spatial; a LUT is the adjustment stack's colour mapping.
        a.mask = None;
        a.vector_mask = None;
        a.clipped = false;
        lattice.layers.push(a);
    }
    let out = photocraft_compose::flatten(&lattice);
    // The shared writer owns CUBE syntax and precision; baking stays app-local.
    let rows = (0..n).flat_map(|b| (0..n).flat_map(move |g| (0..n).map(move |r| (b, g, r)))).map(|(b, g, r)| {
        let px = out.px[g * w as usize + b * n + r];
        [px[0], px[1], px[2]]
    });
    craft_cms::lutfile::write_cube_export("PhotoCraft", title, n, !matches!(depth, photocraft_color::SampleType::F32), rows)
}

/// How the refusal of a canvas too small for the lattice begins ([`bake_cube`] keeps the adjustment table for it alone).
const CANVAS_TOO_SMALL: &str = "the canvas is too small";

/// Most renders [`bake_cube_stack`] makes: a canvas far smaller than the lattice needs one pass
/// per lattice entry sharing a pixel, and the bake refuses rather than run for minutes.
const MAX_STACK_RENDERS: usize = 1 << 16;

/// Most 256 × 256 tiles one render's lattice surface may hold (64 MiB as F32 RGBA).
const MAX_LATTICE_TILES: usize = 64;

/// A lattice entry in [`bake_cube_stack`]: its lattice column (`b·n + r`), row (`g`) and the
/// canvas pixel it is laid on.
type LatticeEntry = (usize, usize, (usize, usize));

/// The canvas pixel lattice entry (`c`, `g`) is laid on, for an `n`-point table on a `w` × `h` canvas.
/// 64-bit products: `c * w` overflows `usize` on a 32-bit target (wasm32) for a wide canvas and would
/// read the wrong pixel. The quotients are below `w` and `h`, so they fit back.
fn lattice_pos(n: usize, w: usize, h: usize, c: usize, g: usize) -> (usize, usize) {
    let cols = (n * n) as u64;
    ((c as u64 * w as u64 / cols) as usize, (g as u64 * h as u64 / n as u64) as usize)
}

/// How many renders a whole-stack bake needs: a pass renders one entry per canvas pixel, so a pixel
/// shared by `k` entries is in `k` passes, and a row is rendered once per pass that reaches it, the
/// most any of its pixels share. Counted from the entries per pixel alone, so a request over the
/// limit costs no pass allocation.
fn stack_render_rows(n: usize, w: usize, h: usize) -> usize {
    let mut shared: std::collections::HashMap<(usize, usize), usize> = std::collections::HashMap::new();
    for g in 0..n {
        for c in 0..n * n {
            *shared.entry(lattice_pos(n, w, h, c, g)).or_insert(0) += 1;
        }
    }
    let mut per_row: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for (&(_, y), &k) in &shared {
        let m = per_row.entry(y).or_insert(0);
        *m = (*m).max(k);
    }
    per_row.values().sum()
}

/// A sample written into the lattice surface allocates its whole tile (1 MiB as F32 RGBA), so a pass
/// whose entries lie in many tiles of a huge canvas (35937 samples at 33 points, one tile each, is 35
/// GiB) is cut into chunks of at most [`MAX_LATTICE_TILES`] tiles, each rendered on its own. Entries
/// are in row order, so a chunk is a run of one row.
fn split_by_tiles(passes: Vec<Vec<LatticeEntry>>) -> Vec<Vec<LatticeEntry>> {
    let tile = photocraft_geom::TILE_SIZE as usize;
    let mut chunks: Vec<Vec<LatticeEntry>> = Vec::new();
    for pass in passes {
        let mut tiles = std::collections::HashSet::new();
        let mut current: Vec<LatticeEntry> = Vec::new();
        for e in pass {
            let t = (e.2.0 / tile, e.2.1 / tile);
            if !tiles.contains(&t) && tiles.len() >= MAX_LATTICE_TILES {
                chunks.push(std::mem::take(&mut current));
                tiles.clear();
            }
            tiles.insert(t);
            current.push(e);
        }
        if !current.is_empty() {
            chunks.push(current);
        }
    }
    chunks
}

/// The visible layers above the Background that a whole-stack table bakes (#13742).
fn stack_layers(doc: &Document) -> Vec<&Layer> {
    let bg = doc.background().map(|l| l.id);
    doc.layers.iter().filter(|l| l.visible && Some(l.id) != bg).collect()
}

/// Bake the document's whole visible stack, as the reference application does (#13742): the
/// lattice image takes the place of the Background's pixels, laid out over the canvas (lattice
/// column `b·n + r`, row `g`, stretched to the canvas size; each entry is read at its block's first
/// pixel), and every visible layer above it is composited over it as the document renders, with
/// opacity, blend mode, masks and clipping. A canvas smaller than the lattice takes several
/// passes. Only the rows holding entries are rendered, so a huge canvas stays cheap.
fn bake_cube_stack(size: usize, title: &str, doc: &Document) -> Result<String> {
    const CMD: &str = "file.export.colorLookupTables";
    let n = size;
    let (w, h) = (doc.size.width as usize, doc.size.height as usize);
    if w == 0 || h == 0 {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "the canvas is empty".into() });
    }
    let cols = n * n;
    let at = |c: usize, g: usize| lattice_pos(n, w, h, c, g);
    let too_many =
        || EngineError::BadParams { cmd: CMD.into(), msg: format!("{CANVAS_TOO_SMALL} for a {n}-point table; use fewer grid points or a larger canvas") };
    // The work limit is checked from the entries-per-pixel counts, before any pass is allocated.
    if stack_render_rows(n, w, h) > MAX_STACK_RENDERS {
        return Err(too_many());
    }
    // Entries sharing a canvas pixel go to successive passes.
    let mut seen = std::collections::HashMap::new();
    let mut passes: Vec<Vec<LatticeEntry>> = Vec::new();
    for g in 0..n {
        for c in 0..cols {
            let pos = at(c, g);
            let k = seen.entry(pos).or_insert(0usize);
            if passes.len() <= *k {
                passes.push(Vec::new());
            }
            passes[*k].push((c, g, pos));
            *k += 1;
        }
    }
    let passes = split_by_tiles(passes);
    let rows: usize = passes.iter().map(|p| p.iter().map(|e| e.2.1).collect::<std::collections::BTreeSet<_>>().len()).sum();
    if rows > MAX_STACK_RENDERS {
        return Err(too_many());
    }
    let step = 1.0 / (n - 1) as f32;
    let colour = |c: usize, g: usize| [(c % n) as f32 * step, g as f32 * step, (c / n) as f32 * step];
    // The lattice and the composite in floating point, whatever the document's depth: the
    // reference's tables of 8-bit documents are exact to their 1/32768 steps (mid grey is 0.5).
    let depth = photocraft_color::SampleType::F32;
    let fmt = photocraft_color::PixelFormat::new(ColorMode::Rgb, depth, true);
    let mut out = vec![[0.0f32; 3]; cols * n];
    for pass in &passes {
        let mut surf = Surface::new(fmt);
        for &(c, g, (x, y)) in pass {
            let rgb = colour(c, g);
            // A tile that cannot be allocated is an error of this export, not an abort.
            surf.try_write_region(
                Rect::new(x as i32, y as i32, x as i32 + 1, y as i32 + 1),
                &photocraft_raster::from_rgba(&fmt, [rgb[0], rgb[1], rgb[2], 1.0]),
            )?;
        }
        let mut grid = doc.clone();
        grid.selection = None;
        grid.depth = depth;
        let lattice = Layer::new("Lattice", LayerContent::Raster(surf));
        match grid.layers.iter().position(|l| Some(l.id) == doc.background().map(|b| b.id)) {
            Some(i) => {
                // The lattice takes the Background's place and its identity (locked transparency and position), so a
                // deep-knockout layer above it still knocks out to the lattice.
                let mut lattice = lattice;
                lattice.locks = grid.layers[i].locks;
                grid.layers[i] = lattice;
            }
            None => grid.layers.insert(0, lattice),
        }
        let ys: std::collections::BTreeSet<usize> = pass.iter().map(|e| e.2.1).collect();
        for y in ys {
            let band = photocraft_compose::render(&grid, Rect::new(0, y as i32, w as i32, y as i32 + 1));
            for &(c, g, (x, _)) in pass.iter().filter(|e| e.2.1 == y) {
                let px = band.px[x];
                out[g * cols + c] = [px[0], px[1], px[2]];
            }
        }
    }
    // The shared writer owns CUBE syntax and precision; red changes fastest, then green, then blue.
    let rows = (0..n).flat_map(|b| (0..n).flat_map(move |g| (0..n).map(move |r| (b, g, r)))).map(|(b, g, r)| out[g * cols + b * n + r]);
    Ok(craft_cms::lutfile::write_cube_export("PhotoCraft", title, n, !matches!(doc.depth, photocraft_color::SampleType::F32), rows))
}

/// The table `file.export.colorLookupTables` bakes by default: the whole visible stack of an RGB
/// document (#13742). Other modes, and a canvas too small for the lattice (the bake's own refusal),
/// keep the earlier table of the visible top-level adjustments; any other error of the stack bake,
/// an allocation failure included, is returned rather than hidden behind that table.
pub fn bake_cube(doc: &Document, size: usize, title: &str) -> Result<String> {
    let size = size.clamp(2, craft_cms::lutfile::MAX_SIZE);
    if doc.mode == ColorMode::Rgb {
        match bake_cube_stack(size, title, doc) {
            Ok(cube) => return Ok(cube),
            // Only the work-limit refusal (a canvas too small for the lattice) falls back: an empty canvas, an
            // allocation failure and the rest are the caller's.
            Err(EngineError::BadParams { msg, .. }) if msg.starts_with(CANVAS_TOO_SMALL) => {}
            Err(e) => return Err(e),
        }
    }
    let layers: Vec<&Layer> = doc.layers.iter().filter(|l| l.visible && matches!(l.content, LayerContent::Adjustment(_))).collect();
    Ok(bake_cube_layers(size, title, &layers, doc.depth))
}

/// Export either the whole visible stack (backward-compatible default), the Layers panel's
/// current selection, or an explicit list supplied by scripts/CLI/automation.
pub(super) fn color_lookup_tables(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.export.colorLookupTables";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let size = match p.get("size") {
        None | Some(Value::Null) => 33,
        Some(v) => v.as_u64().ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "size must be an integer".into() })?,
    };
    if !(2..=craft_cms::lutfile::MAX_SIZE as u64).contains(&size) {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("size must be 2..={}", craft_cms::lutfile::MAX_SIZE) });
    }
    let scope = match p.get("scope") {
        None | Some(Value::Null) => "all",
        Some(Value::String(s)) => s.as_str(),
        Some(_) => return Err(EngineError::BadParams { cmd: CMD.into(), msg: "scope must be all or selected".into() }),
    };
    if !matches!(scope, "all" | "selected") {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "scope must be all or selected".into() });
    }
    if p.get("layers").is_some() && scope == "selected" {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "use either layers or scope=selected, not both".into() });
    }
    let explicit: Option<Vec<LayerId>> = if let Some(v) = p.get("layers") {
        let Some(a) = v.as_array() else { return Err(EngineError::BadParams { cmd: CMD.into(), msg: "layers must be an array of layer IDs".into() }) };
        let mut out = Vec::with_capacity(a.len());
        for id in a {
            let Some(id) = id.as_u64() else { return Err(EngineError::BadParams { cmd: CMD.into(), msg: "layer IDs must be unsigned integers".into() }) };
            out.push(LayerId(id));
        }
        Some(out)
    } else if scope == "selected" {
        Some(d.selected_layers().to_vec())
    } else {
        None
    };
    let title = p.get("title").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stem(&d.doc.name));
    // The default bakes the whole visible stack of an RGB document (#13742); an explicit list or
    // the selection, and other modes, keep the adjustment-only table.
    let (cube, count) = if explicit.is_none() && d.doc.mode == ColorMode::Rgb {
        (bake_cube_stack(size as usize, &title, &d.doc)?, stack_layers(&d.doc).len())
    } else {
        let layers = lut_layers(&d.doc, explicit.as_deref())?;
        (bake_cube_layers(size as usize, &title, &layers, d.doc.depth), layers.len())
    };
    match p.get("path").and_then(Value::as_str) {
        Some(path) => {
            write_file(path, cube.as_bytes())?;
            Ok(json!({"path": path, "size": size, "layerCount": count}))
        }
        None => Ok(json!({"cube": cube, "size": size, "layerCount": count})),
    }
}

// ---------- guides ----------

pub(super) fn push_unique(v: &mut Vec<f64>, x: f64) {
    if !v.iter().any(|g| (g - x).abs() < 0.01) {
        v.push(x);
    }
}

#[cfg(test)]
#[path = "export_clut_stack_tests.rs"]
mod export_clut_stack_tests;
