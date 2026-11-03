//! One-time native automatic tone actions resolved into ordinary Levels/Curves data.
//! Only existing native Auto Tone/Contrast ids are admitted; no generic command runner.
use crate::jobs::JobCtx;
use crate::{EngineError, Result};
use photocraft_doc::adjust::LevelsChannel;
use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

/// Shared native numeric analysis, projected into ordinary editable Levels data.
pub(crate) fn adjustment(surf: &Surface, area: Rect, kind: &str, ctx: &JobCtx) -> Result<Adjustment> {
    use photocraft_algo::tone::auto_levels::{self, Kind};
    ctx.check()?;
    let kind = match kind {
        "contrast" => Kind::Contrast,
        "color" => Kind::Color,
        _ => Kind::Tone,
    };
    let cancelled = || ctx.cancelled();
    let rows = auto_levels::analyze(surf, area, kind, &photocraft_raster::Interrupt::cancel_only(&cancelled))
        .map_err(|error| if error == auto_levels::Error::Cancelled { EngineError::Cancelled } else { EngineError::Other(error.to_string()) })?;
    let per = rows.map(|[in_black, in_white, gamma]| LevelsChannel { in_black, in_white, gamma, ..Default::default() });
    ctx.check()?;
    Ok(Adjustment::Levels { master: LevelsChannel::default(), per_channel: per, space: Default::default(), black: LevelsChannel::default() })
}
/// Detect native one-time automatic actions even when a Smart layer already has a rendered cache.
pub(crate) fn has_pending(layer: &Layer) -> bool {
    matches!(&layer.content,LayerContent::Smart(sm) if sm.smart_filters.iter().any(|f|matches!(f.command.as_str(),"image.autoTone"|"image.autoContrast")))
}

/// Uniform display bins may yield a one-bin Levels interval, narrower than the
/// normal command's sliders. Author the same native Levels response into fixed
/// ordinary pencil Curves instead of publishing invalid Levels parameters.
fn fixed_parameters(
    levels: &Adjustment,
    source: &Surface,
    ctx: &JobCtx,
    charge: &mut impl FnMut(u64) -> Result<()>,
) -> Result<(&'static str, serde_json::Value)> {
    ctx.check()?;
    // Conservative retained-parameter allowance for either Levels JSON or the
    // larger pencil Curves table. This is not the reusable full-frame workspace.
    charge(64 * 1024)?;
    let narrow = matches!(levels,Adjustment::Levels {master,per_channel,..} if std::iter::once(master).chain(per_channel).any(|row|(row.in_white-row.in_black)*255.<1.9999));
    if !narrow {
        return Ok(("image.adjustments.levels", crate::adjust_params::to_params(levels)));
    }
    ctx.check()?;
    let mut ramp = photocraft_compose::Buffer {
        rect: Rect::new(0, 0, 256, 1),
        px: (0..256)
            .map(|index| {
                let value = index as f32 / 255.;
                [value, value, value, 1.]
            })
            .collect(),
    };
    let format = source.format();
    photocraft_compose::adjust::apply_in(
        levels,
        &mut ramp,
        photocraft_compose::adjust::Transfer::for_document(format.mode, format.sample),
        if format.sample == photocraft_doc::SampleType::F32 { Some(format.sample) } else { None },
        format.mode,
    );
    let mut per: [Vec<photocraft_doc::adjust::CurvePoint>; 3] = std::array::from_fn(|_| Vec::with_capacity(256));
    for (index, pixel) in ramp.px.iter().enumerate() {
        for (row, &value) in per.iter_mut().zip(pixel) {
            if !value.is_finite() {
                return Err(EngineError::Other("native automatic tone curve response is nonfinite".into()));
            }
            row.push(photocraft_doc::adjust::CurvePoint { input: index as f32 / 255., output: value.clamp(0., 1.) });
        }
    }
    let curve = Adjustment::Curves { master: Vec::new(), per_channel: per, space: Default::default(), black: Vec::new(), pencil: true };
    let params = crate::adjust_params::to_params(&curve);
    // Validate the exact public representation before publication; Smart replay
    // remains deliberately more tolerant than newly created command data.
    crate::adjust_params::from_params("curves", &params, None, format.mode)?;
    ctx.check()?;
    Ok(("image.adjustments.curves", params))
}

/// Called only for private pending native contents, after their embedded source is prepared.
/// Charge and authorization failures leave this layer untouched; publication remains caller-owned.
#[cfg(test)]
pub(crate) fn materialize(doc: &Document, layer: &mut Layer, ctx: &JobCtx, charge: &mut impl FnMut(u64) -> Result<()>) -> Result<bool> {
    let prepaid = doc.size.area().checked_mul(32).ok_or_else(|| EngineError::Other("native auto analysis bounds overflow".into()))?;
    materialize_with_source(doc, layer, ctx, prepaid, charge, None)
}

/// Resolve analysis against the shared preparer's already prepared source, keeping its original locator intact.
/// The caller holds `prepaid_render_bytes` through this call; only retained parameter allocations debit `charge`.
pub(crate) fn materialize_with_source(
    doc: &Document,
    layer: &mut Layer,
    ctx: &JobCtx,
    prepaid_render_bytes: u64,
    charge: &mut impl FnMut(u64) -> Result<()>,
    source: Option<&crate::smart_cmds::SourceImage>,
) -> Result<bool> {
    ctx.check()?;
    if !has_pending(layer) {
        return Ok(false);
    }
    let LayerContent::Smart(original) = &layer.content else { return Ok(false) };
    let required = doc.size.area().checked_mul(32).ok_or_else(|| EngineError::Other("native auto analysis bounds overflow".into()))?;
    if required > prepaid_render_bytes {
        return Err(EngineError::Other("native auto analysis exceeds its prepaid render reservation".into()));
    }
    let mut pending = original.clone();
    for index in 0..pending.smart_filters.len() {
        ctx.check()?;
        let filter = pending.smart_filters.get(index).cloned().ok_or_else(|| EngineError::Other("native auto action index disappeared".into()))?;
        let kind = match filter.command.as_str() {
            "image.autoTone" => "tone",
            "image.autoContrast" => "contrast",
            _ => continue,
        };
        if !filter.params.as_object().is_some_and(|object| object.is_empty()) {
            return Err(EngineError::Other("one-time native automatic tone action requires an empty parameter object".into()));
        }
        crate::jobs::authorize_render_command(&filter.command, &filter.params)?;
        let source = crate::smart_cmds::render_below_filter_with_source(doc, &pending, index, ctx, source)?
            .ok_or_else(|| EngineError::Other("native auto action has no prepared source pixels".into()))?;
        let levels = adjustment(&source, doc.bounds(), kind, ctx)?;
        let (command, params) = fixed_parameters(&levels, &source, ctx, charge)?;
        crate::jobs::authorize_render_command(command, &params)?;
        ctx.check()?;
        let destination = pending.smart_filters.get_mut(index).ok_or_else(|| EngineError::Other("native auto action index disappeared".into()))?;
        destination.command = command.into();
        destination.params = params;
    }
    ctx.check()?;
    pending.cache = None;
    layer.content = LayerContent::Smart(pending);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{BlendMode, ColorMode, SampleType};
    use photocraft_doc::{SmartFilter, SmartObject, SmartSource};
    use photocraft_geom::{Affine, Size};
    use serde_json::{Value, json};
    use std::sync::Arc;

    /// Build an ordinary stored native filter fixture with default visible Normal blending.
    fn filter(command: &str) -> SmartFilter {
        SmartFilter { command: command.into(), params: json!({}), blend: BlendMode::Normal, opacity: 1.0, visible: true }
    }
    /// Create original native pixels and a pending automatic action at the requested depth.
    fn fixture(depth: SampleType, command: &str) -> (Document, Layer, Surface) {
        let mut source = Document::new("native source", Size::new(16, 2), ColorMode::Rgb, depth);
        let mut pixels = Surface::new(source.pixel_format());
        for x in 0..16 {
            let v = x as f32 / 15.0;
            pixels.fill_rect(Rect::new(x, 0, x + 1, 2), &[0.2 + 0.4 * v, 0.1 + 0.5 * v, 0.3 + 0.2 * v, 1.0]);
        }
        source.layers.push(Layer::new("pixels", LayerContent::Raster(pixels.clone())));
        let mut object = SmartObject::new(
            SmartSource::Embedded { file_name: "native.pcraft".into(), bytes: Arc::new(crate::smart_cmds::encode_source(&source).unwrap()) },
            Affine::IDENTITY,
            None,
        );
        object.smart_filters.push(filter(command));
        let layer = Layer::new("editable native auto", LayerContent::Smart(object));
        (Document::new("parent", source.size, source.mode, source.depth), layer, pixels)
    }
    /// Access the Smart fixture so tests can inspect rewritten native parameters and envelope state.
    fn object(layer: &Layer) -> &SmartObject {
        let LayerContent::Smart(object) = &layer.content else { panic!("expected ordinary Smart contents") };
        object
    }

    #[test]
    /// Compare one-time analysis with normal native Auto commands and require fixed editable data after native
    /// reopen.
    fn native_auto_matches_ordinary_commands_at_every_depth_and_reopens_as_fixed_levels() {
        for depth in SampleType::ALL {
            for command in ["image.autoTone", "image.autoContrast"] {
                let (mut doc, mut layer, pixels) = fixture(depth, command);
                let mut ordinary = doc.clone();
                ordinary.layers.push(Layer::new("pixels", LayerContent::Raster(pixels)));
                let mut session = crate::Session::new();
                session.add_document(ordinary, None);
                session.execute(command, json!({})).unwrap();
                let expected = session.active().unwrap().doc.layers[0].surface().unwrap();
                let mut charged = 0;
                assert!(
                    materialize(&doc, &mut layer, &JobCtx::new(), &mut |bytes| {
                        charged += bytes;
                        Ok(())
                    })
                    .unwrap()
                );
                assert_eq!(charged, 64 * 1024, "fixed Levels JSON remains charged while analysis reuses the prepaid render frame");
                assert_eq!(object(&layer).smart_filters[0].command, "image.adjustments.levels");
                let actual = crate::smart_cmds::render(&doc, object(&layer)).unwrap().unwrap();
                for x in 0..16 {
                    let a = actual.pixel(x, 0);
                    let b = expected.pixel(x, 0);
                    for c in 0..4 {
                        assert!((a[c] - b[c]).abs() < 0.025, "{depth:?} {command}: {a:?} {b:?}");
                    }
                }
                doc.layers.push(layer);
                let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
                let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                assert!(!has_pending(&reopened.layers[0]));
                let render = crate::smart_cmds::render(&reopened, object(&reopened.layers[0])).unwrap().unwrap();
                assert_eq!(actual.pixel(7, 0), render.pixel(7, 0));
                assert!(matches!(object(&reopened.layers[0]).source, SmartSource::Embedded { .. }));
            }
        }
    }

    #[test]
    /// Analyze the actual preceding native stages while retaining visibility, opacity, blend and mask semantics.
    fn automatic_analysis_uses_actual_native_prefix_and_preserves_hidden_envelope() {
        let (doc, mut layer, _) = fixture(SampleType::U8, "image.autoTone");
        let LayerContent::Smart(sm) = &mut layer.content else { panic!() };
        sm.smart_filters.insert(0, filter("image.adjustments.invert"));
        sm.smart_filters[1].visible = false;
        sm.smart_filters[1].opacity = 0.4;
        sm.smart_filters[1].blend = BlendMode::Luminosity;
        let input = crate::smart_cmds::render_below_filter(&doc, sm, 1).unwrap().unwrap();
        let expected = crate::adjust_params::to_params(&adjustment(&input, doc.bounds(), "tone", &JobCtx::new()).unwrap());
        materialize(&doc, &mut layer, &JobCtx::new(), &mut |_| Ok(())).unwrap();
        let resolved = &object(&layer).smart_filters[1];
        assert_eq!(resolved.params, expected);
        assert!(!resolved.visible);
        assert_eq!(resolved.opacity, 0.4);
        assert_eq!(resolved.blend, BlendMode::Luminosity);
        assert_eq!(object(&layer).smart_filters[0].command, "image.adjustments.invert");
    }

    #[test]
    /// Exercise the public preparation hook for cached and nested actions before document admission.
    fn preparation_resolves_cached_and_nested_native_actions_atomically_before_install() {
        let (mut doc, mut cached, pixels) = fixture(SampleType::U8, "image.autoTone");
        let LayerContent::Smart(sm) = &mut cached.content else { panic!() };
        sm.cache = Some(pixels);
        doc.layers.push(cached);
        let (mut nested_doc, nested, _) = fixture(SampleType::U8, "image.autoContrast");
        nested_doc.layers.push(nested);
        let parent = SmartObject::new(
            SmartSource::Embedded { file_name: "nested.pcraft".into(), bytes: Arc::new(crate::smart_cmds::encode_source(&nested_doc).unwrap()) },
            Affine::IDENTITY,
            None,
        );
        doc.layers.push(Layer::new("nested", LayerContent::Smart(parent)));
        let mut session = crate::Session::new();
        let before = doc.clone();
        for gate in [deny_levels as fn(&str, &Value) -> Result<()>, deny_nested_auto] {
            session.authorize = Some(gate);
            assert!(session.prepare_imported_document(&mut doc, &JobCtx::new()).is_err());
            assert_eq!(doc, before);
            assert!(session.active().is_none());
        }
        session.authorize = None;
        session.prepare_imported_document(&mut doc, &JobCtx::new()).unwrap();
        assert!(!has_pending(&doc.layers[0]));
        assert_eq!(object(&doc.layers[0]).smart_filters[0].command, "image.adjustments.levels");
        assert!(object(&doc.layers[0]).cache.is_some());
        let SmartSource::Embedded { bytes, .. } = &object(&doc.layers[1]).source else { panic!() };
        let nested = photocraft_format::load_from_bytes(bytes).unwrap();
        assert!(!has_pending(&nested.layers[0]));
        assert_eq!(object(&nested.layers[0]).smart_filters[0].command, "image.adjustments.levels");
        assert!(object(&nested.layers[0]).cache.is_some());
        assert!(session.active().is_none());
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let mut reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        let before = reopened.clone();
        session.prepare_imported_document(&mut reopened, &JobCtx::new()).unwrap();
        assert_eq!(reopened, before);
    }

    /// Reject the nested automatic command to prove preparation reinstalls normal stored-command authority.
    fn deny_nested_auto(id: &str, _: &Value) -> Result<()> {
        if id == "image.autoContrast" { Err(EngineError::Other("nested source action denied".into())) } else { Ok(()) }
    }

    /// Reject the automatic-analysis command without denying unrelated native work.
    fn deny_auto(id: &str, _: &Value) -> Result<()> {
        if id == "image.autoTone" { Err(EngineError::Other("auto denied".into())) } else { Ok(()) }
    }
    /// Reject the fixed Levels replacement so automatic materialization cannot bypass native command authority.
    fn deny_levels(id: &str, _: &Value) -> Result<()> {
        if id == "image.adjustments.levels" { Err(EngineError::Other("fixed levels denied".into())) } else { Ok(()) }
    }

    #[test]
    /// Check cancellation, short budgets, denied commands and invalid parameters preserve the complete pending
    /// layer.
    fn cancellation_budget_authority_and_invalid_requests_leave_layer_unchanged() {
        let (doc, layer, _) = fixture(SampleType::U8, "image.autoTone");
        let ctx = JobCtx::new();
        ctx.cancel();
        let mut pending = layer.clone();
        assert!(materialize(&doc, &mut pending, &ctx, &mut |_| Ok(())).is_err());
        assert_eq!(pending, layer);
        assert!(materialize(&doc, &mut pending, &JobCtx::new(), &mut |_| Err(EngineError::Other("budget".into()))).is_err());
        assert_eq!(pending, layer);
        for gate in [deny_auto as fn(&str, &Value) -> Result<()>, deny_levels] {
            let _scope = crate::jobs::render_authorization_scope(Some(gate));
            assert!(materialize(&doc, &mut pending, &JobCtx::new(), &mut |_| Ok(())).is_err());
            assert_eq!(pending, layer);
        }
        let LayerContent::Smart(sm) = &mut pending.content else { panic!() };
        sm.smart_filters[0].params = json!({"unrecognized":1});
        let before = pending.clone();
        assert!(materialize(&doc, &mut pending, &JobCtx::new(), &mut |_| Ok(())).is_err());
        assert_eq!(pending, before);
    }

    /// Reject the fixed Curves fallback to test authority on uniform automatic-tone results.
    fn deny_curves(id: &str, _: &Value) -> Result<()> {
        if id == "image.adjustments.curves" { Err(EngineError::Other("fixed curves denied".into())) } else { Ok(()) }
    }

    #[test]
    /// Verify uniform endpoint images use representable native Curves and remain stable after native save/reopen.
    fn opaque_uniform_auto_uses_valid_native_curves_and_reopens_at_every_depth() {
        for depth in SampleType::ALL {
            for value in [0., 1.] {
                for command in ["image.autoTone", "image.autoContrast"] {
                    let (mut document, mut layer, _) = fixture(depth, command);
                    let mut source = document.clone();
                    let mut pixels = Surface::new(source.pixel_format());
                    pixels.fill_rect(source.bounds(), &[value, value, value, 1.]);
                    source.layers.push(Layer::new("Uniform native pixels", LayerContent::Raster(pixels)));
                    let LayerContent::Smart(smart) = &mut layer.content else {
                        panic!("native smart fixture");
                    };
                    smart.source = SmartSource::Embedded {
                        file_name: "uniform.pcraft".into(),
                        bytes: Arc::new(photocraft_format::save_to_bytes(&source, &Default::default()).unwrap()),
                    };
                    let mut ordinary = crate::Session::new();
                    ordinary.add_document(source.clone(), None);
                    ordinary.execute(command, json!({})).unwrap();
                    let expected = ordinary.active().unwrap().doc.layers[0].surface().unwrap().clone();
                    let mut rejected = layer.clone();
                    {
                        let _scope = crate::jobs::render_authorization_scope(Some(deny_curves));
                        assert!(materialize(&document, &mut rejected, &JobCtx::new(), &mut |_| Ok(())).is_err());
                    }
                    assert_eq!(rejected, layer);
                    assert!(
                        materialize(&document, &mut rejected, &JobCtx::new(), &mut |bytes| if bytes == 64 * 1024 {
                            Err(EngineError::Other("curve allocation budget".into()))
                        } else {
                            Ok(())
                        })
                        .is_err()
                    );
                    assert_eq!(rejected, layer);
                    materialize(&document, &mut layer, &JobCtx::new(), &mut |_| Ok(())).unwrap();
                    let filter = &object(&layer).smart_filters[0];
                    assert_eq!(filter.command, "image.adjustments.curves");
                    assert_eq!(filter.params["pencil"], true);
                    // Exercise the current public command's strict schema, not only the
                    // tolerant stored Smart-filter replay decoder.
                    let mut strict = crate::Session::new();
                    strict.add_document(source, None);
                    strict.execute("filter.convertForSmartFilters", json!({})).unwrap();
                    strict.execute(&filter.command, filter.params.clone()).unwrap();
                    let rendered = crate::smart_cmds::render(&document, object(&layer)).unwrap().unwrap();
                    assert_eq!(rendered.read_region(document.bounds()), expected.read_region(document.bounds()), "{depth:?} {value} {command}");
                    document.layers.push(layer);
                    let bytes = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
                    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                    assert!(!has_pending(&reopened.layers[0]));
                    assert_eq!(object(&reopened.layers[0]).smart_filters[0].params, filter_params(&document.layers[0]));
                    let restored = crate::smart_cmds::render(&reopened, object(&reopened.layers[0])).unwrap().unwrap();
                    assert_eq!(restored.read_region(document.bounds()), rendered.read_region(document.bounds()));
                }
            }
        }
    }

    /// Read the materialized native filter parameters without deriving expectations from their imported source.
    fn filter_params(layer: &Layer) -> Value {
        object(layer).smart_filters[0].params.clone()
    }
}
