//! Learned coarse subject masks with existing image-guided edge refinement and editable output.
use super::{EngineError, Layer, LayerContent, Result, Session, Value, bad, edits, enabled, json, transport};
use photocraft_doc::{LayerId, LayerMask};
use photocraft_raster::Surface;

const SELECT: &str = "select.subjectEnki";
const REMOVE: &str = "layer.removeBackgroundEnki";
fn available(s: &Session) -> std::result::Result<(), String> {
    enabled(s)?;
    let st = s.active().ok_or("no document")?;
    let (w, h) = (st.doc.size.width, st.doc.size.height);
    if !(64..=1024).contains(&w) || !(64..=1024).contains(&h) || !w.is_multiple_of(8) || !h.is_multiple_of(8) {
        return Err("Learned masks currently require 64–1024 pixels per side, in multiples of 8; documents are never resized silently".into());
    }
    Ok(())
}
fn removable(s: &Session) -> std::result::Result<(), String> {
    available(s)?;
    let st = s.active().ok_or("no document")?;
    let id = st.active_layer.ok_or("select a pixel layer")?;
    let l = st.doc.layer(id).ok_or("no layer")?;
    if !matches!(l.content, LayerContent::Raster(_))
        || l.mask.is_some()
        || l.vector_mask.is_some()
        || l.clipped
        || !l.effects.items.is_empty()
        || l.video.is_some()
        || l.blend != photocraft_color::BlendMode::Normal
        || l.opacity != 1.0
    {
        return Err("Choose an unmasked pixel layer with normal blending, full opacity and no effects".into());
    }
    if st.doc.path_of(id).is_some_and(|p| p.len() > 1) {
        return Err("Choose a root pixel layer".into());
    }
    Ok(())
}
fn run(s: &mut Session, p: &Value, remove: bool) -> Result<Value> {
    if !p.is_object() {
        return Err(bad("params must be an object"));
    }
    if remove { removable(s) } else { available(s) }.map_err(EngineError::Other)?;
    let radius = p
        .get("refineRadius")
        .map_or(Ok(4.0), |v| v.as_f64().filter(|n| n.is_finite() && (0.0..=32.0).contains(n)).ok_or_else(|| bad("refineRadius must be 0–32")))?
        as f32;
    let dry = p.get("dryRun").map_or(Ok(false), |v| v.as_bool().ok_or_else(|| bad("dryRun must be boolean")))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if p.get("document").is_some_and(|v| v.as_u64() != Some(st.doc.id.0)) {
        return Err(bad("the target document is no longer active"));
    }
    if dry {
        return Ok(json!({"dryRun":true}));
    }
    let target = if remove { st.active_layer } else { None };
    let mut snapshot = (*st.doc).clone();
    if let Some(id) = target {
        let l = snapshot.layer(id).cloned().ok_or(EngineError::NoLayer(id))?;
        snapshot.layers = vec![l];
        if let Some(l) = snapshot.layers.first_mut() {
            l.visible = true;
        }
    }
    let label = if remove { "Remove Background with Enki" } else { "Select Subject with Enki" };
    crate::jobs::run(
        s,
        label,
        true,
        move |ctx| {
            let r = snapshot.bounds();
            let png = edits::subject_source(&snapshot, ctx)?;
            ctx.progress(0.1, "Finding subject with Enki");
            let (bytes, model) = transport::subject_mask(png, r.width(), r.height(), ctx)?;
            let image = photocraft_codecs::decode(&bytes).map_err(|e| bad(e.to_string()))?;
            if image.layout() != photocraft_codecs::ChannelLayout::Gray || image.sample_type() != photocraft_codecs::SampleType::U8 {
                return Err(bad("subject worker must return a grayscale probability mask"));
            }
            let mut mask = Surface::new(photocraft_color::PixelFormat::GRAY8);
            mask.write_region(r, &image.to_normalized());
            if radius > 0.0 {
                ctx.progress(0.85, "Refining subject edges");
                let params = photocraft_algo::matting::RefineParams { radius, smart_radius: true, ..Default::default() };
                let region = crate::smartselect_cmds::with_doc_sampler(&snapshot, None, true, |sampler, _| {
                    photocraft_algo::matting::refine_mask(sampler, &photocraft_algo::matting::surface_reader(&mask), r, r, &params)
                });
                if let Some(region) = region {
                    mask = photocraft_algo::matting::region_surface(&region);
                }
            }
            ctx.check()?;
            Ok((mask, model))
        },
        move |s, (mask, model)| apply(s, mask, target, &model),
    )
}
fn apply(s: &mut Session, mask: Surface, target: Option<LayerId>, model: &str) -> Result<Value> {
    s.edit(if target.is_some() { "Remove Background with Enki" } else { "Select Subject with Enki" }, move |doc, active| {
        if let Some(id) = target {
            let src = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let pixels = src.surface().ok_or_else(|| bad("subject layer no longer contains pixels"))?.clone();
            let mut layer = Layer::new(doc.copy_name(&src.name), LayerContent::Raster(pixels));
            layer.mask = Some(LayerMask { surface: mask, ..LayerMask::hide_all() });
            let new = doc.insert_above(Some(id), layer);
            if let Some(src) = doc.layer_mut(id) {
                src.visible = false;
            }
            *active = Some(new);
            Ok(json!({"layer":new.0,"model":model}))
        } else {
            doc.selection = Some(mask);
            Ok(json!({"selected":true,"model":model}))
        }
    })
}
pub(super) fn specs() -> Vec<crate::CommandSpec> {
    [(SELECT,"Subject with Enki",&["Select"][..],available as fn(&Session)->std::result::Result<(),String>,select as fn(&mut Session,&Value)->Result<Value>),(REMOVE,"Remove Background with Enki",&["Layer"][..],removable,remove)]
    .into_iter().map(|(id,label,menu,enabled,run)|crate::CommandSpec{id,label,menu,shortcut:None,params:r#"{"refineRadius":0..32=4,"document":id?,"dryRun":bool=false}. Learned subject mask, optional image-guided refinement. Remove creates a masked duplicate and hides the original in one undo."#,enabled,run,journal:true}).collect()
}
fn select(s: &mut Session, p: &Value) -> Result<Value> {
    run(s, p, false)
}
fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    run(s, p, true)
}

#[cfg(test)]
mod tests {
    use super::{LayerId, SELECT, Session, Surface, apply, json};
    #[test]
    fn learned_selection_and_cutout_are_single_undo_and_never_change_source_pixels() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":64,"height":64,"depth":depth})).unwrap();
            s.execute("layer.new.layer", json!({})).unwrap();
            s.execute("edit.fill", json!({"contents":"color","color":"#b86b35"})).unwrap();
            let id = s.active().unwrap().active_layer.unwrap();
            let before = s.active().unwrap().doc.clone();
            let mut mask = Surface::new(photocraft_color::PixelFormat::GRAY8);
            mask.fill_rect(photocraft_geom::Rect::from_xywh(8, 8, 32, 32), &[0.5]);
            apply(&mut s, mask.clone(), None, "mock").unwrap();
            assert_eq!(s.active().unwrap().doc.selection, Some(mask.clone()));
            assert!(s.undo());
            let result = apply(&mut s, mask.clone(), Some(id), "mock").unwrap();
            let new = LayerId(result["layer"].as_u64().unwrap());
            let st = s.active().unwrap();
            assert!(!st.doc.layer(id).unwrap().visible);
            assert_eq!(st.doc.layer(id).unwrap().surface(), before.layer(id).unwrap().surface());
            assert_eq!(st.doc.layer(new).unwrap().surface(), before.layer(id).unwrap().surface());
            assert_eq!(st.doc.layer(new).unwrap().mask.as_ref().unwrap().surface, mask);
            let bytes = photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap().layers, st.doc.layers);
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc.layers, before.layers);
            assert!(s.redo());
            assert!(s.execute(SELECT, json!({"refineRadius":100,"dryRun":true})).is_err());
            assert!(s.execute(SELECT, json!({"dryRun":true})).is_ok());
        }
    }
}
