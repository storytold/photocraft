//! File › Automate › Make Figure: lays several images out as one publication figure, the way
//! journal figures with panels a, b, c are built. The figure is a new document at a print width
//! (a journal column, or any width in mm) and resolution (300 dpi by default).
//!
//! Images of different sizes are matched: in the Rows layout every image in a row is scaled to the
//! same height and the row fills the width exactly, so nothing is cropped; in the Grid layout every
//! image is fitted into equal cells. Gutters (the lines between panels) have a width in mm and a
//! colour. Each image is its own pixel layer, and each panel gets an editable type layer label
//! (a, b, c / A, B, C / (a)…) in a corner, with an optional outline so it reads on any image.
//!
//! File › Automate › Prepare Collage for Publication (`figure.fromCollage`) does the same for a
//! collage already saved as one image (an OsiriX or PACS screen capture of a grid of series): it
//! finds the grid (or takes rows and columns), splits the image into its panels and lays them out
//! again with gutters and labels at the print size, in a new document. The collage is unchanged.

use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::file_cmds::{file_name, import, read_file, stem};
use crate::{EngineError, Result, Session};

const CMD: &str = "figure.make";
const COLLAGE: &str = "figure.fromCollage";
const MM_PER_INCH: f64 = 25.4;
/// Journal figure widths in mm, as Export for Publication offers them.
const COLUMNS: [(&str, f64); 3] = [("single", 85.0), ("oneAndHalf", 120.0), ("double", 175.0)];
const MAX_PANELS: usize = 48;
/// Largest side of the figure in pixels.
const MAX_SIDE: f64 = 30_000.0;

fn bad(msg: impl Into<String>) -> EngineError {
    bad_in(CMD, msg)
}

fn bad_in(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// The figure's options, checked before anything is opened or created.
struct Opts {
    columns: usize,
    grid: bool,
    width_px: u32,
    dpi: f64,
    gutter_px: u32,
    gutter_color: String,
    labels: Option<String>,
    corner: String,
    label_pt: f64,
    label_color: String,
    outline: bool,
    bold: bool,
    name: String,
}

/// The image files `figure.make` places, checked but not read.
fn files_param(p: &Value) -> Result<Vec<String>> {
    let files: Vec<String> = match p.get("files") {
        Some(Value::Array(a)) => {
            a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| bad("`files` must be a list of paths"))).collect::<Result<_>>()?
        }
        Some(Value::String(dir)) => crate::file_cmds::list_images(dir)?,
        _ => return Err(bad("missing `files` (a list of image paths, or a folder)")),
    };
    if files.is_empty() {
        return Err(bad("no images to place"));
    }
    if files.len() > MAX_PANELS {
        return Err(bad(format!("at most {MAX_PANELS} images per figure")));
    }
    Ok(files)
}

/// The layout options in `p` for `count` panels (`cmd` names the command in errors).
fn opts(cmd: &str, p: &Value, default_columns: usize, default_labels: &str, default_name: &str) -> Result<Opts> {
    let bad = |m: String| bad_in(cmd, m);
    let num = |k: &str, lo: f64, hi: f64, d: f64| -> Result<f64> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(d),
            Some(v) => {
                v.as_f64().filter(|x| x.is_finite() && (lo..=hi).contains(x)).ok_or_else(|| bad(format!("`{k}` must be a number between {lo} and {hi}")))
            }
        }
    };
    let text = |k: &str, d: &str| -> Result<String> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(d.to_string()),
            Some(v) => v.as_str().map(str::to_string).ok_or_else(|| bad(format!("`{k}` must be text"))),
        }
    };
    let columns = num("columns", 1.0, 12.0, default_columns as f64)?.round() as usize;
    let grid = match text("layout", "rows")?.as_str() {
        "rows" => false,
        "grid" => true,
        o => return Err(bad(format!("unknown layout `{o}` (rows|grid)"))),
    };
    let dpi = num("dpi", 72.0, 2400.0, 300.0)?;
    let width_mm = match text("width", "double")?.as_str() {
        "custom" => num("widthMm", 10.0, 2000.0, 175.0)?,
        w => COLUMNS.iter().find(|(n, _)| *n == w).map(|(_, mm)| *mm).ok_or_else(|| bad(format!("unknown width `{w}` (single|oneAndHalf|double|custom)")))?,
    };
    let width_px = (width_mm / MM_PER_INCH * dpi).round();
    if width_px > MAX_SIDE {
        return Err(bad(format!("{width_mm} mm at {dpi} dpi is wider than {MAX_SIDE} px")));
    }
    let gutter_mm = num("gutterMm", 0.0, 20.0, 1.0)?;
    let gutter = text("gutterColor", "#ffffff")?;
    if !is_hex(&gutter) {
        return Err(bad("`gutterColor` must be #rrggbb".into()));
    }
    let labels = match text("labels", default_labels)?.as_str() {
        "none" => None,
        l @ ("abc" | "ABC" | "(a)" | "(A)" | "a)" | "A)") => Some(l.to_string()),
        o => return Err(bad(format!("unknown labels `{o}` (abc|ABC|(a)|(A)|a)|A)|none)"))),
    };
    let corner = text("labelPosition", "topLeft")?;
    if !matches!(corner.as_str(), "topLeft" | "topRight" | "bottomLeft" | "bottomRight") {
        return Err(bad("`labelPosition` must be topLeft|topRight|bottomLeft|bottomRight".into()));
    }
    let label_color = text("labelColor", "#ffffff")?;
    if !is_hex(&label_color) {
        return Err(bad("`labelColor` must be #rrggbb".into()));
    }
    let outline = match p.get("labelOutline") {
        None | Some(Value::Null) => true,
        Some(v) => v.as_bool().ok_or_else(|| bad("`labelOutline` must be true or false".into()))?,
    };
    let bold = match p.get("labelBold") {
        None | Some(Value::Null) => true,
        Some(v) => v.as_bool().ok_or_else(|| bad("`labelBold` must be true or false".into()))?,
    };
    Ok(Opts {
        bold,
        columns,
        grid,
        width_px: width_px.max(16.0) as u32,
        dpi,
        gutter_px: (gutter_mm / MM_PER_INCH * dpi).round() as u32,
        gutter_color: gutter,
        labels,
        corner,
        label_pt: num("labelSizePt", 4.0, 72.0, 12.0)?,
        label_color,
        outline,
        name: text("name", default_name)?,
    })
}

fn is_hex(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// The label of panel `i` (0-based): a, b, … z, aa, ab …
fn label(style: &str, i: usize) -> String {
    let mut n = i;
    let mut s = String::new();
    loop {
        s.insert(0, (b'a' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    match style {
        "ABC" => s.to_uppercase(),
        "(a)" => format!("({s})"),
        "(A)" => format!("({})", s.to_uppercase()),
        "a)" => format!("{s})"),
        "A)" => format!("{})", s.to_uppercase()),
        _ => s,
    }
}

/// Panel rectangles `[x, y, w, h]` for images of `aspects` (width / height), and the figure height.
fn layout(aspects: &[f64], columns: usize, grid: bool, width: u32, gutter: u32) -> (Vec<[u32; 4]>, u32) {
    let (w, g) = (f64::from(width), f64::from(gutter));
    let mut rects = Vec::with_capacity(aspects.len());
    let mut y = 0.0;
    let cols = columns.max(1);
    // Grid cells share one size: the width split evenly, the height of the widest image's shape.
    let cell_w = ((w - (cols as f64 - 1.0) * g) / cols as f64).max(1.0);
    let cell_h = (cell_w / aspects.iter().copied().fold(f64::MIN, f64::max).max(0.01)).max(1.0);
    for row in aspects.chunks(cols) {
        if grid {
            for (i, a) in row.iter().enumerate() {
                let k = (cell_w / a).min(cell_h) / cell_h;
                let (pw, ph) = ((cell_h * k * a).round().max(1.0), (cell_h * k).round().max(1.0));
                let x0 = i as f64 * (cell_w + g) + (cell_w - pw) / 2.0;
                let y0 = y + (cell_h - ph) / 2.0;
                rects.push([x0.round() as u32, y0.round() as u32, pw as u32, ph as u32]);
            }
            y += cell_h.round() + g;
        } else {
            // Same height across the row, filling the width exactly.
            let sum: f64 = row.iter().sum();
            let h = ((w - (row.len() as f64 - 1.0) * g) / sum.max(0.01)).max(1.0).round();
            let mut x = 0.0;
            for (i, a) in row.iter().enumerate() {
                let pw = if i + 1 == row.len() { (w - x).max(1.0) } else { (h * a).round().max(1.0) };
                rects.push([x.round() as u32, y.round() as u32, pw as u32, h as u32]);
                x += pw + g;
            }
            y += h + g;
        }
    }
    (rects, (y - g).max(1.0).round() as u32)
}

/// One image flattened and resampled to `w` x `h`, as straight RGBA rows.
fn panel_pixels(src: Document, w: u32, h: u32) -> Result<photocraft_compose::Buffer> {
    let mut t = Session::new();
    t.add_document(src, None);
    t.execute("image.imageSize", json!({"width": w, "height": h, "resample": "automatic"}))?;
    let d = &t.active().ok_or(EngineError::NoDocument)?.doc;
    Ok(photocraft_compose::flatten(d))
}

fn make_figure(s: &mut Session, p: &Value) -> Result<Value> {
    let files = files_param(p)?;
    let default_columns = match files.len() {
        1..=3 => files.len(),
        4 => 2,
        _ => 3,
    };
    let o = opts(CMD, p, default_columns, "abc", "Figure")?;
    let mut sources = Vec::with_capacity(files.len());
    for f in &files {
        let bytes = read_file(f)?;
        let d = import(&file_name(f), &bytes).map_err(|e| bad(format!("{f}: {e}")))?;
        sources.push((stem(f), d));
    }
    let r = build(s, CMD, &o, sources)?;
    remember(s, CMD, p);
    Ok(r)
}

/// Lays `sources` (name, image) out as a new figure document with `o`'s gutters and labels.
fn build(s: &mut Session, cmd: &str, o: &Opts, sources: Vec<(String, Document)>) -> Result<Value> {
    let aspects: Vec<f64> = sources.iter().map(|(_, d)| f64::from(d.size.width.max(1)) / f64::from(d.size.height.max(1))).collect();
    let (rects, height) = layout(&aspects, o.columns, o.grid, o.width_px, o.gutter_px);
    if f64::from(height) > MAX_SIDE {
        return Err(bad_in(cmd, format!("the figure would be {height} px tall; use more columns or a smaller width")));
    }
    s.execute("file.new", json!({"width": o.width_px, "height": height, "background": o.gutter_color, "resolution": o.dpi, "name": o.name}))?;
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let label_px = o.label_pt / 72.0 * o.dpi;
    let mut panels = Vec::new();
    for (i, ((name, src), r)) in sources.into_iter().zip(&rects).enumerate() {
        let [x, y, w, h] = *r;
        let buf = panel_pixels(src, w, h).map_err(|e| bad_in(cmd, format!("{name}: {e}")))?;
        let n = fmt.channels();
        let mut data = vec![0.0f32; buf.px.len() * n];
        for (px, out) in buf.px.iter().zip(data.chunks_exact_mut(n)) {
            photocraft_raster::from_rgba_into(&fmt, *px, out);
        }
        let mut surf = photocraft_doc::Surface::new(fmt);
        let (x0, y0) = (x as i32, y as i32);
        surf.write_region(Rect::new(x0, y0, x0 + w as i32, y0 + h as i32), &data);
        let id = s.edit("Make Figure", |doc, active| {
            let l = Layer::new(name.clone(), LayerContent::Raster(surf));
            let id = l.id;
            *active = Some(id);
            doc.layers.push(l);
            Ok(id)
        })?;
        let mut label_id = Value::Null;
        if let Some(style) = &o.labels {
            let text = label(style, i);
            let margin = (label_px * 0.35).max(2.0);
            let (right, bottom) = (o.corner.ends_with("Right"), o.corner.starts_with("bottom"));
            let lx = if right { f64::from(x + w) - margin } else { f64::from(x) + margin };
            let ly = if bottom { f64::from(y + h) - margin - label_px * 0.2 } else { f64::from(y) + margin + label_px * 0.75 };
            let t = s.execute(
                "type.create",
                json!({"x": lx, "y": ly, "text": text, "size": o.label_pt, "color": o.label_color, "fauxBold": o.bold, "align": if right { "right" } else { "left" }, "name": format!("Label {text}")}),
            )?;
            label_id = t["layer"].clone();
            if o.outline
                && let Some(lid) = label_id.as_u64()
            {
                let outline_color = if o.label_color.eq_ignore_ascii_case("#000000") { "#ffffff" } else { "#000000" };
                s.execute(
                    "layer.layerStyle.stroke",
                    json!({"layer": lid, "size": (label_px * 0.06).max(1.0).round(), "position": "outside", "color": outline_color}),
                )?;
            }
        }
        panels.push(json!({"name": name, "layer": id.0, "rect": [x, y, w, h], "label": label_id}));
    }
    let size_mm = |px: u32| (f64::from(px) / o.dpi * MM_PER_INCH * 10.0).round() / 10.0;
    Ok(json!({"width": o.width_px, "height": height, "dpi": o.dpi, "widthMm": size_mm(o.width_px), "heightMm": size_mm(height), "panels": panels}))
}

/// Remembers `p`'s layout choices (not files or names) as the dialog's next defaults.
fn remember(s: &mut Session, cmd: &str, p: &Value) {
    let keep = [
        "layout",
        "columns",
        "rows",
        "width",
        "widthMm",
        "dpi",
        "gutterMm",
        "gutterColor",
        "labels",
        "labelPosition",
        "labelSizePt",
        "labelColor",
        "labelOutline",
        "labelBold",
    ];
    let saved: serde_json::Map<String, Value> = keep.iter().filter_map(|k| p.get(*k).filter(|v| !v.is_null()).map(|v| ((*k).to_string(), v.clone()))).collect();
    if !saved.is_empty() {
        s.edit_prefs(|pr| {
            pr.dialogs.insert(cmd.to_string(), Value::Object(saved));
        });
    }
}

/// How many equal parts `scores` (edge strength at each position along one axis) splits into,
/// and where the boundaries are (0 and `scores.len()` included): the count whose every boundary
/// sits on a strong edge, compared with the typical edge strength. Each boundary is the strongest
/// edge within 2 px of its equal-split position.
fn detect_parts(scores: &[f64]) -> (usize, Vec<u32>) {
    let n = scores.len();
    let whole = || vec![0, n as u32];
    if n < 16 {
        return (1, whole());
    }
    let mut sorted = scores.to_vec();
    sorted.sort_by(f64::total_cmp);
    let typical = sorted.get(n / 2).copied().unwrap_or(0.0).max(1e-4);
    let peak = |at: usize| -> (usize, f64) {
        let (lo, hi) = (at.saturating_sub(2), (at + 3).min(n));
        (lo..hi).map(|i| (i, scores.get(i).copied().unwrap_or(0.0))).fold((at, f64::MIN), |b, c| if c.1 > b.1 { c } else { b })
    };
    let mut best: (usize, f64, Vec<u32>) = (1, 0.0, whole());
    for parts in 2..=8usize {
        // The weakest of the boundaries decides: a real grid has an edge at every one of them.
        let edges: Vec<(usize, f64)> = (1..parts).map(|k| peak(k * n / parts)).collect();
        let weakest = edges.iter().map(|(_, v)| v / typical).fold(f64::MAX, f64::min);
        if weakest > 2.5 && weakest > best.1 * 0.85 {
            let mut at: Vec<u32> = vec![0];
            at.extend(edges.iter().map(|(i, _)| *i as u32));
            at.push(n as u32);
            best = (parts, weakest, at);
        }
    }
    (best.0, best.2)
}

/// The panel grid of a collage, from the edges running all the way across it.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub rows: usize,
    pub columns: usize,
    /// Column boundaries in px, 0 and the width included.
    pub xs: Vec<u32>,
    /// Row boundaries in px, 0 and the height included.
    pub ys: Vec<u32>,
}

pub fn detect_grid(doc: &Document) -> Grid {
    let buf = photocraft_compose::flatten(doc);
    let (w, h) = (buf.rect.width() as usize, buf.rect.height() as usize);
    let single = Grid { rows: 1, columns: 1, xs: vec![0, w as u32], ys: vec![0, h as u32] };
    if w < 16 || h < 16 || buf.px.len() < w * h {
        return single;
    }
    let lum = |p: &[f32; 4]| f64::from(0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]);
    let mut cols = vec![0.0f64; w];
    let mut rows = vec![0.0f64; h];
    let lines: Vec<&[[f32; 4]]> = buf.px.chunks_exact(w).collect();
    for (y, line) in lines.iter().enumerate() {
        for ((c, a), b) in cols.iter_mut().skip(1).zip(line.iter().skip(1)).zip(line.iter()) {
            *c += (lum(a) - lum(b)).abs();
        }
        if let (Some(above), Some(score)) = (y.checked_sub(1).and_then(|i| lines.get(i)), rows.get_mut(y)) {
            *score = line.iter().zip(above.iter()).map(|(a, b)| (lum(a) - lum(b)).abs()).sum::<f64>() / w as f64;
        }
    }
    for c in &mut cols {
        *c /= h as f64;
    }
    let ((r, ys), (c, xs)) = (detect_parts(&rows), detect_parts(&cols));
    Grid { rows: r, columns: c, xs, ys }
}

/// `figure.detectGrid`: the panel grid of the active document (Cut Out snaps to it).
fn detect_grid_cmd(s: &mut Session, _: &Value) -> Result<Value> {
    let g = detect_grid(&s.active().ok_or(EngineError::NoDocument)?.doc);
    Ok(json!({"rows": g.rows, "columns": g.columns, "x": g.xs, "y": g.ys}))
}

/// File › Automate › Prepare Collage for Publication.
fn from_collage(s: &mut Session, p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = Document::clone(&d.doc);
    let count = |k: &str| -> Result<Option<usize>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => match v.as_f64().filter(|n| n.is_finite() && (0.0..=12.0).contains(n)) {
                Some(n) if n.round() == 0.0 => Ok(None),
                Some(n) => Ok(Some(n.round() as usize)),
                None => Err(bad_in(COLLAGE, format!("`{k}` must be 0 (detect) to 12"))),
            },
        }
    };
    let (want_rows, want_cols) = (count("rows")?, count("columns")?);
    // The detected grid, unless both counts were given. Its edges are where the panels really
    // meet (they can be a pixel or two off an equal split), as Cut Out snaps to them.
    let grid = match (want_rows, want_cols) {
        (Some(_), Some(_)) => None,
        _ => Some(detect_grid(&doc)),
    };
    let rows = want_rows.or(grid.as_ref().map(|g| g.rows)).unwrap_or(1);
    let cols = want_cols.or(grid.as_ref().map(|g| g.columns)).unwrap_or(1);
    // Each panel needs at least one pixel: more rows or columns than the image has is an error,
    // not a figure with panels quietly missing.
    if rows > doc.size.height as usize || cols > doc.size.width as usize {
        return Err(bad_in(COLLAGE, format!("{rows} × {cols} panels don't fit a {} × {} px image", doc.size.width, doc.size.height)));
    }
    let stem = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(a, _)| a).to_string();
    // Columns of the new figure follow the collage's grid unless the dialog says otherwise.
    let mut q = p.clone();
    if let Some(o) = q.as_object_mut() {
        o.insert("columns".into(), json!(cols));
    }
    let o = opts(COLLAGE, &q, cols, "ABC", &format!("{stem} figure"))?;
    let (w, h) = (doc.size.width, doc.size.height);
    // Detected edges on an axis whose count is the detected one; an equal split otherwise.
    let edges = |n: usize, extent: u32, detected: Option<&Vec<u32>>| -> Vec<u32> {
        match detected.filter(|e| e.len() == n + 1) {
            Some(e) => e.clone(),
            None => (0..=n).map(|i| (i * extent as usize / n) as u32).collect(),
        }
    };
    let xs = edges(cols, w, grid.as_ref().map(|g| &g.xs));
    let ys = edges(rows, h, grid.as_ref().map(|g| &g.ys));
    let mut sources = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for c in 0..cols {
            let (Some(&x0), Some(&x1), Some(&y0), Some(&y1)) = (xs.get(c), xs.get(c + 1), ys.get(r), ys.get(r + 1)) else { continue };
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let mut t = Session::new();
            t.add_document(doc.clone(), None);
            t.execute("image.crop", json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0}))?;
            let panel = Document::clone(&t.active().ok_or(EngineError::NoDocument)?.doc);
            sources.push((format!("{stem} {}", r * cols + c + 1), panel));
        }
    }
    let mut result = build(s, COLLAGE, &o, sources)?;
    result["rows"] = json!(rows);
    result["columns"] = json!(cols);
    remember(s, COLLAGE, p);
    Ok(result)
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: CMD,
            label: "Make Figure…",
            menu: &["File", "Automate"],
            shortcut: None,
            params: r##"{"files":[paths]|folder (1..48 images, in panel order),"columns":1..12 (default by count),"layout":"rows|grid"="rows" (rows: equal height per row, filling the width, no cropping; grid: equal cells),"width":"single|oneAndHalf|double|custom"="double" (85, 120, 175 mm),"widthMm":mm (custom),"dpi":72..2400=300,"gutterMm":0..20=1,"gutterColor":"#rrggbb"="#ffffff","labels":"abc|ABC|(a)|(A)|a)|A)|none"="abc","labelPosition":"topLeft|topRight|bottomLeft|bottomRight"="topLeft","labelSizePt":4..72=12,"labelColor":"#rrggbb"="#ffffff","labelOutline":bool=true,"labelBold":bool=true,"name":str="Figure"} → {width,height,dpi,widthMm,heightMm,panels:[{file,layer,rect,label}]}. Opens a new document"##,
            enabled: always,
            run: make_figure,
            journal: true,
        },
        CommandSpec {
            id: COLLAGE,
            label: "Prepare Collage for Publication…",
            menu: &["File", "Automate"],
            shortcut: None,
            params: r##"{"rows":0..12=0 (0: detect),"columns":0..12=0 (0: detect), then Make Figure's layout keys: "width","widthMm","dpi","gutterMm","gutterColor","labels"="ABC","labelPosition","labelSizePt","labelColor","labelOutline","labelBold","name"} → Make Figure's result plus {rows,columns}. Splits the active collage (an OsiriX/PACS grid capture) into its panels and lays them out again with gutters and labels at print size in a new document; the collage is unchanged"##,
            enabled: has_doc,
            run: from_collage,
            journal: true,
        },
        CommandSpec {
            id: "figure.detectGrid",
            label: "Detect Panel Grid",
            menu: &[],
            shortcut: None,
            params: r##"{} → {rows,columns,x:[column boundaries px, 0 and width included],y:[row boundaries]}. The panel grid of the active collage"##,
            enabled: has_doc,
            run: detect_grid_cmd,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
