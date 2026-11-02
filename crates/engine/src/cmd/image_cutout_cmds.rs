//! Cut Out (`image.cutOut`): removes a vertical or horizontal band from the whole image and joins
//! the two sides, like Snagit's Cut Out tool. The canvas shrinks by the band.
//!
//! Every pixel surface (layers, cached type/shape/smart pixels, masks, channels, selection) loses
//! the band, and whatever lay past it moves over. Type, shape and smart-object layers that lie
//! wholly past the band move with it and stay editable; ones that the band crosses are rasterized
//! first, since their live geometry cannot be split, and ones wholly inside it are deleted. Guides past the band move too. An optional
//! edge marks the seam with a zigzag or wave drawn on a new shape layer, so it can be restyled or
//! deleted. Everything is one history step.

use photocraft_algo::resample::crop_surface;
use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The band being removed: columns (`vertical`) or rows `from..to` in document pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Band {
    vertical: bool,
    from: i32,
    to: i32,
}

impl Band {
    fn width(&self) -> i32 {
        self.to - self.from
    }

    /// The part of `b` before the band.
    fn before(&self, b: Rect) -> Rect {
        if self.vertical { Rect::new(b.x0, b.y0, self.from.clamp(b.x0, b.x1), b.y1) } else { Rect::new(b.x0, b.y0, b.x1, self.from.clamp(b.y0, b.y1)) }
    }

    /// The part of `b` after the band.
    fn after(&self, b: Rect) -> Rect {
        if self.vertical { Rect::new(self.to.clamp(b.x0, b.x1), b.y0, b.x1, b.y1) } else { Rect::new(b.x0, self.to.clamp(b.y0, b.y1), b.x1, b.y1) }
    }

    /// How content after the band moves.
    fn shift(&self) -> (i32, i32) {
        if self.vertical { (-self.width(), 0) } else { (0, -self.width()) }
    }

    /// Where along the cut axis `r` lies: wholly before the band, wholly after it, wholly
    /// inside it (cut away entirely), or across one of its edges.
    fn side(&self, r: Rect) -> Side {
        let (lo, hi) = if self.vertical { (r.x0, r.x1) } else { (r.y0, r.y1) };
        if r.is_empty() || hi <= self.from {
            Side::Before
        } else if lo >= self.to {
            Side::After
        } else if lo >= self.from && hi <= self.to {
            Side::Inside
        } else {
            Side::Across
        }
    }

    /// A coordinate on the cut axis after the cut (inside the band: the seam).
    fn map(&self, v: f64) -> f64 {
        let (from, to) = (f64::from(self.from), f64::from(self.to));
        if v >= to {
            v - (to - from)
        } else if v > from {
            from
        } else {
            v
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Before,
    After,
    Inside,
    Across,
}

/// `s` without the band, the far side moved over it.
fn cut_surface(s: &Surface, band: Band) -> Surface {
    let b = s.content_bounds();
    if b.is_empty() {
        return s.clone();
    }
    let mut out = crop_surface(s, band.before(b));
    let after = band.after(b);
    if !after.is_empty() {
        let (dx, dy) = band.shift();
        out.write_interleaved(after.translate(dx, dy), &s.to_interleaved(after));
    }
    out
}

/// Where a live (type, shape, smart object) layer or a vector mask lies relative to the band.
fn live_bounds(l: &Layer) -> Option<Rect> {
    let content = match &l.content {
        LayerContent::Text(t) => t.cache.as_ref().map(Surface::content_bounds),
        LayerContent::Shape(sh) => sh.cache.as_ref().map(Surface::content_bounds),
        LayerContent::Smart(sm) => sm.cache.as_ref().map(Surface::content_bounds),
        _ => None,
    };
    let mask = l
        .vector_mask
        .as_ref()
        .and_then(|vm| vm.path.control_bounds())
        .map(|(x0, y0, x1, y1)| Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32));
    match (content, mask) {
        (Some(a), Some(b)) => Some(a.union(&b)),
        (a, b) => a.or(b),
    }
}

/// The layers (any depth) whose live geometry lies on `side` of the band.
fn on_side(layers: &[Layer], band: Band, side: Side, out: &mut Vec<LayerId>) {
    for l in layers {
        if let LayerContent::Group(g) = &l.content {
            on_side(&g.children, band, side, out);
        }
        if live_bounds(l).is_some_and(|b| band.side(b) == side) {
            out.push(l.id);
        }
    }
}

/// Moves the live geometry of every layer wholly past the band (its pixels move separately).
fn shift_geometry(layers: &mut [Layer], band: Band, dpi: f32) {
    let (dx, dy) = band.shift();
    let a = Affine::translate(f64::from(dx), f64::from(dy));
    for l in layers {
        if let LayerContent::Group(g) = &mut l.content {
            shift_geometry(&mut g.children, band, dpi);
            continue;
        }
        if live_bounds(l).is_some_and(|b| band.side(b) == Side::After) {
            crate::canvas_geom::transform_layer_geometry(l, &a, true, dpi);
        }
    }
}

fn cut_doc(doc: &mut Document, band: Band) {
    let dpi = doc.resolution_dpi;
    shift_geometry(&mut doc.layers, band, dpi);
    crate::image_cmds::for_each_surface(&mut doc.layers, true, &mut |surf, _| *surf = cut_surface(surf, band));
    for ch in doc.channels.iter_mut().chain(doc.quick_mask.as_mut()) {
        ch.surface = cut_surface(&ch.surface, band);
    }
    if let Some(sel) = &doc.selection {
        doc.selection = Some(cut_surface(sel, band));
    }
    let guides = if band.vertical { &mut doc.guides.vertical } else { &mut doc.guides.horizontal };
    for g in guides.iter_mut() {
        *g = band.map(*g);
    }
    let w = band.width().unsigned_abs();
    if band.vertical {
        doc.size.width = doc.size.width.saturating_sub(w).max(1);
    } else {
        doc.size.height = doc.size.height.saturating_sub(w).max(1);
    }
    crate::canvas_geom::refresh(doc, crate::canvas_geom::Refresh::Shapes);
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "image.cutOut".into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// The seam path: a zigzag or wave along the join at `at`, spanning `len` px.
fn seam_path(vertical: bool, at: f64, len: f64, edge: &str, amp: f64) -> Value {
    let step = amp * 2.0;
    let n = ((len / step).ceil() as usize).clamp(2, 20_000);
    let pt = |along: f64, off: f64| if vertical { [at + off, along] } else { [along, at + off] };
    let knots: Vec<Value> = (0..=n)
        .map(|i| {
            let along = (i as f64 * step).min(len);
            let off = if i % 2 == 0 { -amp } else { amp };
            let p = pt(along, off);
            if edge == "wave" {
                // Horizontal tangents at the crests: a smooth sine-like wave.
                let h = step * 0.36;
                json!({"anchor": p, "in": pt(along - h, off), "out": pt(along + h, off), "smooth": true})
            } else {
                json!({"anchor": p, "in": p, "out": p, "smooth": false})
            }
        })
        .collect();
    json!({"subpaths": [{"closed": false, "knots": knots}]})
}

/// Image › Cut Out.
fn cut_out(s: &mut Session, p: &Value) -> Result<Value> {
    let vertical = match p.get("axis").and_then(Value::as_str).unwrap_or("vertical") {
        "vertical" => true,
        "horizontal" => false,
        o => return Err(bad(format!("unknown axis `{o}` (vertical|horizontal)"))),
    };
    let num = |k: &str| -> Result<f64> {
        p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(format!("`{k}` must be a finite number of pixels")))
    };
    let (a, b) = (num("from")?, num("to")?);
    let edge = p.get("edge").and_then(Value::as_str).unwrap_or("straight");
    if !matches!(edge, "straight" | "sawtooth" | "wave") {
        return Err(bad(format!("unknown edge `{edge}` (straight|sawtooth|wave)")));
    }
    let color = p.get("edgeColor").cloned().unwrap_or_else(|| json!("#ffffff"));
    // Check the seam's colour before anything changes: a bad one must not leave half a cut.
    crate::vector_cmds::parse_stroke(&json!({"color": color}), None, photocraft_doc::Color::BLACK).map_err(|e| bad(format!("`edgeColor`: {e}")))?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let extent = if vertical { d.doc.size.width } else { d.doc.size.height };
    let other = f64::from(if vertical { d.doc.size.height } else { d.doc.size.width });
    let clamp = |v: f64| v.round().clamp(0.0, f64::from(extent)) as i32;
    let (from, to) = (clamp(a.min(b)), clamp(a.max(b)));
    if to - from < 1 {
        return Err(bad("the band to cut out must be at least 1 px inside the canvas"));
    }
    if to - from >= extent as i32 {
        return Err(bad("the band covers the whole image; leave something on one side"));
    }
    let band = Band { vertical, from, to };
    let (mut ids, mut inside) = (Vec::new(), Vec::new());
    on_side(&d.doc.layers, band, Side::Across, &mut ids);
    // Type, shapes and smart objects wholly inside the band are cut away with it: deleted
    // rather than left behind as empty layers.
    on_side(&d.doc.layers, band, Side::Inside, &mut inside);
    // Share one history step between the rasterizing, the cut and the seam.
    let key = match p.get("coalesce").and_then(Value::as_str) {
        Some(k) => k.to_string(),
        // The document revision, unlike the history length (capped by History States), moves
        // with every edit: two cuts in a row never share a step.
        None => format!("cut-out-{}", s.active().map_or(0, |d| d.revision)),
    };
    let mut rasterized = Vec::new();
    for id in ids {
        if crate::extra_cmds::rasterize_one(s, id, None, &key)? {
            rasterized.push(id.0);
        }
    }
    s.coalesce_request = Some(key.clone());
    let r = s.edit("Cut Out", |doc, active| {
        for id in &inside {
            doc.remove(*id);
        }
        if active.is_some_and(|a| doc.layer(a).is_none()) {
            *active = doc.layers.last().map(|l| l.id);
        }
        cut_doc(doc, band);
        Ok(())
    });
    s.coalesce_request = None;
    r?;
    let mut seam = Value::Null;
    if edge != "straight" {
        let amp = (other / 120.0).clamp(3.0, 14.0);
        let path = seam_path(vertical, f64::from(from), other, edge, amp);
        let stroke = json!({"width": (amp * 0.9).max(2.0), "color": color, "cap": "round", "join": "round"});
        let info = s.execute("shape.create", json!({"kind": "path", "path": path, "fill": null, "stroke": stroke, "name": "Cut Out Edge", "coalesce": key}))?;
        seam = info["layer"].clone();
        // Above everything (shape.create put it above the active layer, maybe inside a
        // group), so the seam shows over every layer that was cut.
        let id = LayerId(info["layer"].as_u64().ok_or_else(|| EngineError::Other("shape.create returned no layer".into()))?);
        s.coalesce_request = Some(key.clone());
        let r = s.edit("Cut Out", |doc, active| {
            if let Some(l) = doc.remove(id) {
                doc.layers.push(l);
                *active = Some(id);
            }
            Ok(())
        });
        s.coalesce_request = None;
        r?;
    }
    let size = s.active().ok_or(EngineError::NoDocument)?.doc.size;
    let deleted: Vec<u64> = inside.iter().map(|id| id.0).collect();
    Ok(json!({"width": size.width, "height": size.height, "removed": to - from, "rasterized": rasterized, "deleted": deleted, "edgeLayer": seam}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "image.cutOut",
        label: "Cut Out",
        menu: &[],
        shortcut: None,
        params: r##"{"axis":"vertical|horizontal"="vertical" (vertical removes columns),"from":px,"to":px (the band, clamped to the canvas),"edge":"straight|sawtooth|wave"="straight","edgeColor":"#rrggbb"="#ffffff"} → {width,height,removed,rasterized:[layer ids],deleted:[layer ids],edgeLayer:id|null}. Removes the band from every layer and joins the sides; the canvas shrinks. Type, shape and smart-object layers the band crosses are rasterized first; ones wholly inside it are deleted"##,
        enabled: has_doc,
        run: cut_out,
        journal: true,
    }]
}

#[cfg(test)]
mod tests;
