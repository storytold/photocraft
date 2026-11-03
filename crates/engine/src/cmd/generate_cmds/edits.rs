//! Source-aware image edits; generated samples are exposed only through the saved edit mask.
use super::{EditInput, Request, bad, decode, enabled, transport, validate_png};
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_algo::resample::translate_surface;
use photocraft_color::{BlendMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;
use serde_json::{Value, json};

mod harmonize;

const FILL: &str = "image.fillSelection";
const REMOVE: &str = "image.removeObject";
const BACKGROUND: &str = "image.replaceBackground";
const EXPAND: &str = "image.expandCanvas";

struct Edit {
    region: Rect,
    mask: Surface,
    expand: Option<(u32, u32, u32, u32)>,
}

fn selected(s: &Session) -> std::result::Result<(), String> {
    enabled(s)?;
    let st = s.active().ok_or("no document open")?;
    let mask = st.doc.selection.as_ref().ok_or("select the region to edit first")?;
    if mask.content_bounds().intersect(&st.doc.bounds()).is_empty() {
        return Err("the selection is empty".into());
    }
    Ok(())
}

fn expandable(s: &Session) -> std::result::Result<(), String> {
    enabled(s)?;
    let st = s.active().ok_or("no document open")?;
    if st.doc.walk().iter().any(|(_, _, l)| {
        !matches!(l.content, LayerContent::Raster(_) | LayerContent::Group(_))
            || l.artboard().is_some()
            || l.video.is_some()
            || !l.effects.items.is_empty()
            || l.blend == BlendMode::Dissolve
    }) {
        return Err(
            "Expand Canvas currently supports static pixel layers and groups without effects or dissolve blending; use a rasterized duplicate for other content"
                .into(),
        );
    }
    Ok(())
}

fn selection_edit(s: &Session) -> Result<Edit> {
    selected(s).map_err(EngineError::Other)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let selection = doc.selection.as_ref().ok_or_else(|| bad("select the region to edit first"))?;
    let bounds = selection.content_bounds().intersect(&doc.bounds());
    if bounds.width() > 1024 || bounds.height() > 1024 {
        return Err(bad("the selected edit region exceeds 1024 pixels per side; use a smaller selection"));
    }
    // Keep context when space permits; never shrink the selection to satisfy model limits.
    let side = |n: u32| n.saturating_add(128).clamp(64, 1024).div_ceil(8) * 8;
    let (w, h) = (side(bounds.width()), side(bounds.height()));
    let origin = |a: i32, len: u32, total: u32, size: u32| {
        a.saturating_add((len / 2) as i32).saturating_sub((size / 2) as i32).clamp(0, total.saturating_sub(size).min(i32::MAX as u32) as i32)
    };
    let region = Rect::from_xywh(origin(bounds.x0, bounds.width(), doc.size.width, w), origin(bounds.y0, bounds.height(), doc.size.height, h), w, h);
    if selection.format().channels() != 1 {
        return Err(bad("selection must be a single-channel mask"));
    }
    let mut mask = Surface::new(selection.format());
    let valid = region.intersect(&doc.bounds());
    let values = selection.read_region(valid);
    if values.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(bad("selection contains invalid mask weights"));
    }
    mask.write_region(valid, &values);
    if !mask.read_region(valid).iter().any(|v| *v > 0.0) {
        return Err(bad("the edit mask is empty"));
    }
    Ok(Edit { region, mask, expand: None })
}

fn expansion(s: &Session, p: &Value) -> Result<Edit> {
    expandable(s).map_err(EngineError::Other)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let margin = |key: &str| -> Result<u32> {
        p.get(key)
            .map_or(Some(0), Value::as_u64)
            .filter(|n| *n <= 512)
            .map(|n| n as u32)
            .ok_or_else(|| bad(format!("{key} must be a whole number from 0 to 512")))
    };
    let (left, right, top, bottom) = (margin("left")?, margin("right")?, margin("top")?, margin("bottom")?);
    if left + right + top + bottom == 0 {
        return Err(bad("set an expansion margin on at least one side"));
    }
    let w = doc.size.width.saturating_add(left).saturating_add(right);
    let h = doc.size.height.saturating_add(top).saturating_add(bottom);
    if !(64..=1024).contains(&w) || !(64..=1024).contains(&h) || w % 8 != 0 || h % 8 != 0 {
        return Err(bad("the expanded canvas must be 64–1024 pixels per side and multiples of 8; dimensions are never rounded silently"));
    }
    let region = Rect::from_xywh(0, 0, w, h);
    let mut mask = Surface::new(PixelFormat::GRAY8);
    mask.fill_rect(region, &[1.0]);
    mask.fill_rect(Rect::from_xywh(left as i32, top as i32, doc.size.width, doc.size.height), &[0.0]);
    Ok(Edit { region, mask, expand: Some((left, top, w, h)) })
}

fn source(doc: &Document, edit: &Edit, ctx: &crate::jobs::JobCtx) -> Result<String> {
    ctx.check()?;
    let original = edit.expand.map_or(edit.region, |_| doc.bounds());
    let fmt = doc.pixel_format();
    let flat = photocraft_compose::composite_to_surface(doc, original.intersect(&doc.bounds()), fmt, None);
    let (dx, dy) = edit.expand.map_or((-edit.region.x0, -edit.region.y0), |(l, t, _, _)| (l as i32, t as i32));
    let flat = translate_surface(&flat, dx, dy);
    let mut wire = Document::new("Edit source", Size::new(edit.region.width(), edit.region.height()), doc.mode, doc.depth);
    wire.icc_profile = doc.icc_profile.clone();
    wire.layers.push(Layer::new("Source", LayerContent::Raster(flat)));
    crate::image_cmds::change_depth(&mut wire, SampleType::U16);
    crate::color_cmds::convert_document(&mut wire, photocraft_cms::Builtin::Srgb.profile(), photocraft_cms::Intent::RelativeColorimetric, true)?;
    crate::image_cmds::change_depth(&mut wire, SampleType::U8);
    // Diffusion input needs visible context in transparent padding; originals keep their alpha.
    wire.layers.insert(0, Layer::new("Context matte", LayerContent::Fill(photocraft_doc::Fill::Solid(photocraft_color::Color::rgb(0.5, 0.5, 0.5)))));
    let (png, _) = crate::file_cmds::encode(&wire, "Source.png", crate::file_cmds::SaveOpts::default())?;
    validate_png(&png, edit.region.width(), edit.region.height())?;
    ctx.check()?;
    Ok(photocraft_paint::tile::b64_encode(&png))
}

fn mask_png(edit: &Edit) -> Result<String> {
    let values = edit.mask.read_region(edit.region);
    let bytes: Vec<u8> = values.into_iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
    let image = photocraft_codecs::Image::from_raw(
        edit.region.width(),
        edit.region.height(),
        photocraft_codecs::ChannelLayout::Gray,
        photocraft_codecs::SampleType::U8,
        bytes,
    )
    .map_err(|e| bad(e.to_string()))?;
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).map_err(|e| bad(e.to_string()))?;
    Ok(photocraft_paint::tile::b64_encode(&png))
}

fn run(s: &mut Session, p: &Value, kind: &str) -> Result<Value> {
    start(s, p, kind).map_err(|error| match error {
        EngineError::BadParams { msg, .. } => EngineError::BadParams { cmd: kind.into(), msg },
        other => other,
    })
}

fn start(s: &mut Session, p: &Value, kind: &str) -> Result<Value> {
    let edit = if kind == EXPAND {
        expansion(s, p)?
    } else if kind == BACKGROUND {
        background_edit(s)?
    } else {
        selection_edit(s)?
    };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if p.get("document").is_some_and(|v| v.as_u64() != Some(st.doc.id.0)) {
        return Err(bad("the target document is no longer active"));
    }
    let mut params = p.clone();
    if !params.is_object() {
        return Err(bad("params must be an object"));
    }
    params["width"] = json!(edit.region.width());
    params["height"] = json!(edit.region.height());
    if kind == REMOVE && params.get("prompt").is_none() {
        params["prompt"] = json!("Remove the selected object and reconstruct the surrounding background naturally.");
    }
    let mut request = Request::parse(&params)?;
    let harmonize = params.get("harmonize").map_or(Ok(kind != BACKGROUND), |v| v.as_bool().ok_or_else(|| bad("harmonize must be boolean")))?;
    if request.dry_run {
        return Ok(
            json!({"dryRun":true,"operation":kind,"region":[edit.region.x0,edit.region.y0,edit.region.width(),edit.region.height()],"harmonize":harmonize,"requires":"masked_edit_revision 1"}),
        );
    }
    let snapshot = st.doc.clone();
    let format = st.doc.pixel_format();
    let profile = crate::color_cmds::document_profile(&st.doc);
    let intent = s.color.settings.intent();
    let bpc = s.color.settings.bpc;
    let label = match kind {
        EXPAND => "Expand Canvas",
        REMOVE => "Remove Object",
        BACKGROUND => "Replace Background",
        _ => "Fill Selection",
    };
    crate::jobs::run(
        s,
        label,
        true,
        move |ctx| {
            ctx.progress(0.0, "Preparing image context and edit mask");
            let source_png = source(&snapshot, &edit, ctx)?;
            request.edit = Some(EditInput { source_png, mask_png: mask_png(&edit)? });
            let output = transport::generate(&request, ctx)?;
            let mut surface = decode(&output, &profile, format, intent, bpc)?;
            if harmonize {
                ctx.progress(0.95, "Matching generated edge colors");
                let reference = if let Some((left, top, _, _)) = edit.expand {
                    let reference = photocraft_compose::composite_to_surface(&snapshot, snapshot.bounds(), format, None);
                    translate_surface(&reference, left as i32, top as i32)
                } else {
                    let reference = photocraft_compose::composite_to_surface(&snapshot, edit.region.intersect(&snapshot.bounds()), format, None);
                    translate_surface(&reference, -edit.region.x0, -edit.region.y0)
                };
                harmonize::match_edges(&mut surface, &reference, &edit.mask, edit.region, ctx)?;
            }
            ctx.check()?;
            Ok((surface, edit, output.seed, output.model))
        },
        move |s, (surface, edit, seed, model)| apply_edit(s, surface, edit, label, seed, &model),
    )
}

fn apply_edit(s: &mut Session, surface: Surface, edit: Edit, label: &str, seed: i64, model: &str) -> Result<Value> {
    let surface = translate_surface(&surface, edit.region.x0, edit.region.y0);
    if let Some((left, top, _, _)) = edit.expand {
        s.set_sampler_map(photocraft_geom::Affine::translate(left as f64, top as f64));
    }
    let region = edit.region;
    let id = s.edit(label, move |doc, active| {
        if let Some((left, top, w, h)) = edit.expand {
            crate::image_cmds::translate_doc(doc, EXPAND, left as i32, top as i32)?;
            doc.size = Size::new(w, h);
        }
        let mut layer = Layer::new(doc.next_layer_name(label), LayerContent::Raster(surface));
        layer.mask = Some(LayerMask { surface: edit.mask, ..LayerMask::hide_all() });
        // Root insertion avoids inheriting clipping masks or changing a group's blending context.
        let id = doc.insert_above(None, layer);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer":id.0,"seed":seed,"model":model,"region":[region.x0,region.y0,region.width(),region.height()]}))
}

pub(super) fn specs() -> Vec<CommandSpec> {
    [(FILL,"Fill Selection…",selected as fn(&Session)->std::result::Result<(),String>,fill as fn(&mut Session,&Value)->Result<Value>),
     (REMOVE,"Remove Object…",selected,remove), (EXPAND,"Expand Canvas…",expandable,expand), (BACKGROUND,"Replace Background…",selected,background)]
    .into_iter().map(|(id,label,enabled,run)|CommandSpec {id,label,menu:&["Image"],shortcut:None,
        params:r#"{"prompt":text (optional for Remove Object),"model":id?,"negative":text?,"seed":i64=-1,"steps":1..50?,"cfg":1..20?,"strength":0.05..1?,"harmonize":bool (defaults to false for Replace Background, true otherwise),"document":id?,"left":0..512,"right":0..512,"top":0..512,"bottom":0..512 (Expand Canvas only),"dryRun":bool=false (validate without inference)} → {layer,seed,model,region}. Source-aware masked editing, new root layer, one undo. Fill/Remove require selection; bounded context never shrinks the selection. Expand supports raster layers/groups without effects; source pixels remain beneath a zero-weight protection mask. Requires Enki masked_edit_revision 1."#,
        enabled,run,journal:true}).collect()
}
fn fill(s: &mut Session, p: &Value) -> Result<Value> {
    run(s, p, FILL)
}
fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    run(s, p, REMOVE)
}
fn expand(s: &mut Session, p: &Value) -> Result<Value> {
    run(s, p, EXPAND)
}

#[cfg(test)]
mod tests;

pub(super) fn subject_source(doc: &Document, ctx: &crate::jobs::JobCtx) -> Result<String> {
    source(doc, &Edit { region: doc.bounds(), mask: Surface::new(PixelFormat::GRAY8), expand: None }, ctx)
}

fn background(s: &mut Session, p: &Value) -> Result<Value> {
    run(s, p, BACKGROUND)
}
fn background_edit(s: &Session) -> Result<Edit> {
    selected(s).map_err(EngineError::Other)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let r = doc.bounds();
    if !(64..=1024).contains(&r.width()) || !(64..=1024).contains(&r.height()) || !r.width().is_multiple_of(8) || !r.height().is_multiple_of(8) {
        return Err(bad("Background replacement requires 64–1024 pixels per side, in multiples of 8"));
    }
    let selected = doc.selection.as_ref().ok_or_else(|| bad("select the subject to keep first"))?;
    if selected.format().channels() != 1 {
        return Err(bad("subject selection must be single-channel"));
    }
    let values = selected.read_region(r);
    if values.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Err(bad("invalid subject mask weights"));
    }
    let values: Vec<f32> = values.into_iter().map(|v| 1.0 - v).collect();
    if !values.iter().any(|v| *v > 0.0) {
        return Err(bad("the subject fills the canvas; there is no background to replace"));
    }
    let mut mask = Surface::new(selected.format());
    mask.write_region(r, &values);
    Ok(Edit { region: r, mask, expand: None })
}
