use super::*;

use std::sync::{Mutex, PoisonError};

use adapter::{Adapter, Descriptor, Device, DeviceKind, Execution, Operation, Output};
use photocraft_doc::{Layer, Size};

const ID: &str = "org.photocraft.test.reference";

#[derive(Default)]
struct Reference {
    seen: Mutex<Vec<Request>>,
    cancel: Option<JobCtx>,
}

impl Adapter for Reference {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            api_version: adapter::API_VERSION,
            id: ID.into(),
            name: "Reference CPU adapter (not AI)".into(),
            version: "1".into(),
            operations: vec![
                Operation { id: "image".into(), output_kind: OutputKind::Image, color_modes: vec!["rgb".into(), "gray".into(), "cmyk".into(), "lab".into()] },
                Operation { id: "mask".into(), output_kind: OutputKind::Mask, color_modes: vec!["rgb".into(), "gray".into(), "cmyk".into(), "lab".into()] },
                Operation { id: "depth".into(), output_kind: OutputKind::Depth, color_modes: vec!["rgb".into()] },
            ],
            devices: vec![Device {
                id: "cpu".into(),
                kind: DeviceKind::Cpu,
                backend: "reference".into(),
                name: "Reference CPU".into(),
                operations: vec!["image".into(), "mask".into(), "depth".into()],
            }],
            resource_formats: vec!["opaque-reference".into()],
            params_schema: json!({}),
        }
    }

    fn run(&self, request: &Request, ctx: &dyn adapter::Context) -> adapter::Result<Response> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner).push(request.clone());
        ctx.progress(0.6);
        ctx.progress(f32::NAN);
        ctx.progress(0.2);
        if let Some(cancel) = &self.cancel {
            cancel.cancel();
        }
        let kind = match request.operation.as_str() {
            "mask" => OutputKind::Mask,
            "depth" => OutputKind::Depth,
            _ => OutputKind::Image,
        };
        let mut image = request.image.clone();
        if kind == OutputKind::Image {
            image.data.fill(0.75);
        } else {
            image.channels = 1;
            image.color_mode = "gray".into();
            image.source_depth = 32;
            image.alpha = false;
            image.icc_profile.clear();
            image.data = vec![0.625; image.width as usize * image.height as usize];
        }
        Ok(Response {
            output: Output { kind, image },
            execution: Execution { devices: vec!["cpu".into()], backend: "reference".into(), fallback_reason: None },
        })
    }
}

fn session(mode: ColorMode, depth: SampleType) -> (Session, LayerId, Arc<Reference>) {
    let mut doc = Document::new("Adapter test", Size::new(3, 2), mode, depth);
    doc.icc_profile = Some(Arc::new(vec![11, 22, 33, 44]));
    let mut surface = Surface::new(doc.pixel_format());
    let mut px = vec![0.25; surface.channels()];
    *px.last_mut().unwrap() = 0.5;
    surface.fill_rect(doc.bounds(), &px);
    surface.write_pixel(-1, 0, &px);
    let layer = Layer::new("Pixels", LayerContent::Raster(surface));
    let id = layer.id;
    doc.layers.push(layer);
    let mut s = Session::new();
    s.add_document(doc, None);
    let adapter = Arc::new(Reference::default());
    s.register_adapter(adapter.clone()).unwrap();
    (s, id, adapter)
}

fn params(operation: &str, output: &str) -> Value {
    json!({"provider": ID, "operation": operation, "output": output})
}

fn samples(s: &Session, layer: LayerId) -> Vec<f32> {
    let st = s.active().unwrap();
    st.doc.layer(layer).unwrap().surface().unwrap().read_region(st.doc.bounds())
}

#[test]
fn empty_registry_is_usable_registration_is_session_local_and_duplicates_fail() {
    let mut s = Session::new();
    assert_eq!(s.execute(LIST, json!({})).unwrap(), json!({"apiVersion": 1, "installed": []}));
    let reference = Arc::new(Reference::default());
    s.register_adapter(reference.clone()).unwrap();
    assert!(s.register_adapter(reference.clone()).is_err());
    assert!(!s.unregister_adapter("not.registered"));
    assert!(s.unregister_adapter(ID));
    assert!(!s.unregister_adapter(ID));
    assert_eq!(s.execute(LIST, json!({})).unwrap()["installed"], json!([]));
    s.register_adapter(reference).unwrap();
    let list = s.execute(LIST, json!({})).unwrap();
    assert_eq!(list["installed"].as_array().unwrap().len(), 1);
    assert_eq!(list["installed"][0]["id"], ID);
    assert_eq!(list["installed"][0]["devices"][0]["kind"], "cpu");
    assert_eq!(Session::new().execute(LIST, json!({})).unwrap()["installed"], json!([]));
    assert!(s.execute(LIST, json!({"provider": ID})).is_err());
    assert!(crate::commands::find("plugin.adapter.install").is_some());
    assert!(crate::commands::find("plugin.adapter.loadDll").is_none());
}

#[test]
fn native_floats_modes_depths_profiles_resources_and_undo_are_preserved() {
    let opaque = "  not-a-real-file://../original filter.8bf?worker=no%20rewrite\n";
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in SampleType::ALL {
            let (mut s, layer, adapter) = session(mode, depth);
            let before = samples(&s, layer);
            let before_history = s.active().unwrap().history.entries().len();
            let mut p = params("image", "pixels");
            p["resource"] = json!(opaque);
            p["params"] = json!({"amount": 0.75, "nested": {"data": [1, 2, 3]}});
            p["layer"] = json!(layer.0);
            let result = s.execute(RUN, p).unwrap();
            assert_eq!(result["execution"]["devices"], json!(["cpu"]));
            assert_eq!(result["execution"]["backend"], "reference");
            let seen = adapter.seen.lock().unwrap();
            let request = seen.last().unwrap();
            assert_eq!(request.image.source_depth, depth.bits() as u8);
            assert_eq!(request.image.color_mode, color_mode(mode));
            assert_eq!(request.image.channels as usize, mode.color_channels() + 1);
            assert_eq!(request.image.icc_profile, vec![11, 22, 33, 44]);
            assert_eq!(request.image.data, before);
            assert_eq!(request.resource.as_deref(), Some(opaque));
            assert_eq!(request.params["nested"]["data"], json!([1, 2, 3]));
            assert_eq!(request.device, None);
            assert!(!request.allow_fallback);
            drop(seen);
            let st = s.active().unwrap();
            assert_eq!(st.doc.mode, mode);
            assert_eq!(st.doc.depth, depth);
            assert_eq!(st.doc.icc_profile.as_deref().map(Vec::as_slice), Some([11, 22, 33, 44].as_slice()));
            assert_eq!(st.doc.layer(layer).unwrap().surface().unwrap().format().sample, depth);
            assert_eq!(st.history.entries().len(), before_history + 1);
            let surface = st.doc.layer(layer).unwrap().surface().unwrap();
            let mut outside = vec![0.0; surface.channels()];
            surface.read_pixel(-1, 0, &mut outside);
            assert_eq!(outside, before[..outside.len()]);
            assert_ne!(samples(&s, layer), before);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(samples(&s, layer), before);
            s.execute("edit.redo", json!({})).unwrap();
            assert_ne!(samples(&s, layer), before);
        }
    }
}

#[test]
fn floating_hdr_input_is_not_clipped_or_quantized() {
    let (mut s, layer, adapter) = session(ColorMode::Rgb, SampleType::F32);
    s.edit("HDR", |doc, _| {
        doc.layer_mut(layer).unwrap().surface_mut().unwrap().write_pixel(0, 0, &[-1.25, 2.125, 0.1234567, 0.5]);
        Ok(())
    })
    .unwrap();
    s.execute(RUN, params("image", "pixels")).unwrap();
    assert_eq!(&adapter.seen.lock().unwrap()[0].image.data[..4], &[-1.25, 2.125, 0.1234567, 0.5]);
}

#[test]
fn malformed_params_fail_before_adapter_or_any_document_mutation() {
    let (mut s, layer, adapter) = session(ColorMode::Rgb, SampleType::U16);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.entries();
    for (key, wrong) in [
        ("provider", json!(17)),
        ("provider", Value::Null),
        ("operation", json!([])),
        ("resource", json!(false)),
        ("resource", Value::Null),
        ("params", json!([])),
        ("params", Value::Null),
        ("device", json!(7)),
        ("device", Value::Null),
        ("allowFallback", json!(1)),
        ("allowFallback", Value::Null),
        ("output", json!(false)),
        ("output", Value::Null),
        ("layer", json!("Pixels")),
        ("layer", json!(-1)),
        ("layer", json!(1.5)),
        ("layer", Value::Null),
        ("target", json!("mask")),
        ("target", json!({"channel": 0})),
        ("target", Value::Null),
        ("coalesce", json!(3)),
        ("typo", json!(true)),
    ] {
        let mut p = params("image", "pixels");
        p[key] = wrong;
        assert!(s.execute(RUN, p.clone()).is_err(), "{p}");
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc), "{p}");
    }
    for key in ["provider", "operation", "output"] {
        let mut p = params("image", "pixels");
        p.as_object_mut().unwrap().remove(key);
        assert!(s.execute(RUN, p).is_err());
    }
    assert_eq!(s.active().unwrap().history.entries(), history);
    assert!(adapter.seen.lock().unwrap().is_empty());
    assert!(!samples(&s, layer).is_empty());
}

#[test]
fn explicit_pixel_layer_is_used_instead_of_active_layer_and_empty_icc_is_not_replaced() {
    let (mut s, layer, reference) = session(ColorMode::Rgb, SampleType::F32);
    s.edit("no assigned profile", |doc, _| {
        doc.icc_profile = None;
        Ok(())
    })
    .unwrap();
    let other = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    let other_before = samples(&s, LayerId(other));
    let before = samples(&s, layer);
    let mut p = params("image", "pixels");
    p["layer"] = json!(layer.0);
    s.execute(RUN, p).unwrap();
    assert_ne!(samples(&s, layer), before);
    assert_eq!(samples(&s, LayerId(other)), other_before);
    assert_eq!(s.active().unwrap().active_layer, Some(LayerId(other)));
    assert!(s.active().unwrap().doc.icc_profile.is_none());
    assert!(reference.seen.lock().unwrap()[0].image.icc_profile.is_empty());
}

#[test]
fn invalid_adapter_output_fails_without_any_write_or_undo_step() {
    struct Invalid {
        field: &'static str,
    }
    impl Adapter for Invalid {
        fn descriptor(&self) -> Descriptor {
            Reference::default().descriptor()
        }
        fn run(&self, request: &Request, ctx: &dyn adapter::Context) -> adapter::Result<Response> {
            let mut response = Reference::default().run(request, ctx)?;
            let image = &mut response.output.image;
            match self.field {
                "origin" => image.x += 1,
                "size" => image.width += 1,
                "mode" => image.color_mode = "cmyk".into(),
                "depth" => image.source_depth = 8,
                "icc" => image.icc_profile.push(99),
                "length" => {
                    image.data.pop();
                }
                "nan" => {
                    image.data[0] = f32::NAN;
                }
                "alpha" => {
                    image.alpha = false;
                }
                _ => response.output.kind = OutputKind::Mask,
            }
            Ok(response)
        }
    }
    for field in ["origin", "size", "mode", "depth", "icc", "length", "nan", "alpha", "kind"] {
        let (base, layer, _) = session(ColorMode::Rgb, SampleType::U16);
        let mut s = Session::new();
        s.add_document((*base.active().unwrap().doc).clone(), None);
        s.register_adapter(Arc::new(Invalid { field })).unwrap();
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.entries();
        assert!(s.execute(RUN, params("image", "pixels")).is_err(), "{field}");
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc), "{field}");
        assert_eq!(s.active().unwrap().history.entries(), history);
        assert_eq!(samples(&s, layer), samples(&base, layer));
    }
}

#[test]
fn selection_blends_native_channels_and_transparency_lock_preserves_alpha() {
    for depth in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
            let (mut s, layer, _) = session(mode, depth);
            s.edit("selection and lock", |doc, _| {
                let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                selection.write_region(doc.bounds(), &[0.0, 0.25, 0.5, 1.0, 0.125, 0.75]);
                doc.selection = Some(selection);
                doc.layer_mut(layer).unwrap().locks.transparency = true;
                Ok(())
            })
            .unwrap();
            let before = samples(&s, layer);
            s.execute(RUN, params("image", "pixels")).unwrap();
            let after = samples(&s, layer);
            let channels = mode.color_channels() + 1;
            let tolerance = 1.0
                / match depth {
                    SampleType::U8 => 255.0,
                    SampleType::U16 => 65535.0,
                    SampleType::F32 => 1e6,
                };
            for (i, (old, new)) in before.chunks_exact(channels).zip(after.chunks_exact(channels)).enumerate() {
                let coverage = [0.0, 0.25, 0.5, 1.0, 0.125, 0.75][i];
                for c in 0..channels - 1 {
                    assert!((new[c] - (old[c] + (0.75 - old[c]) * coverage)).abs() <= tolerance);
                }
                assert_eq!(new.last(), old.last());
            }
        }
    }
}

/// Return explicit uncomposited candidate samples without changing their format metadata.
struct Candidate(Vec<f32>);

impl Adapter for Candidate {
    fn descriptor(&self) -> Descriptor {
        Reference::default().descriptor()
    }

    fn run(&self, request: &Request, _ctx: &dyn adapter::Context) -> adapter::Result<Response> {
        let mut image = request.image.clone();
        image.data.clone_from(&self.0);
        Ok(Response {
            output: Output { kind: OutputKind::Image, image },
            execution: Execution { devices: vec!["cpu".into()], backend: "reference".into(), fallback_reason: None },
        })
    }
}

fn candidate_session(depth: SampleType, old: &[[f32; 4]; 6], candidate: &[[f32; 4]; 6], coverage: &[f32; 6]) -> (Session, LayerId) {
    let (mut s, layer, _) = session(ColorMode::Rgb, depth);
    s.unregister_adapter(ID);
    s.register_adapter(Arc::new(Candidate(candidate.iter().flatten().copied().collect()))).unwrap();
    s.edit("candidate input", |doc, _| {
        let bounds = doc.bounds();
        doc.layer_mut(layer).unwrap().surface_mut().unwrap().write_region(bounds, &old.iter().flatten().copied().collect::<Vec<_>>());
        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        selection.write_region(bounds, coverage);
        doc.selection = Some(selection);
        Ok(())
    })
    .unwrap();
    (s, layer)
}

#[test]
fn unlocked_selection_mixes_associated_colour_and_preserves_coverage_endpoints() {
    let old = [[1.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.125, 0.375, 0.625, 0.0], [1.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 0.25]];
    let candidate = [[0.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 0.0], [1.0, 1.0, 1.0, 1.0], [0.125, 0.375, 0.625, 0.0], [0.0, 1.0, 0.0, 0.75]];
    let expected = [[1.0, 0.0, 0.0, 0.5], [0.0, 1.0, 0.0, 0.25], [0.0, 0.0, 0.0, 0.0], old[3], candidate[4], [0.25, 0.75, 0.0, 0.5]];
    for depth in SampleType::ALL {
        let (mut s, layer) = candidate_session(depth, &old, &candidate, &[0.5, 0.25, 0.5, 0.0, 1.0, 0.5]);
        let before = samples(&s, layer);
        s.execute(RUN, params("image", "pixels")).unwrap();
        let after = samples(&s, layer);
        let tolerance = match depth {
            SampleType::U8 => 1.0 / 255.0 + 1e-6,
            SampleType::U16 => 1.0 / 65535.0 + 1e-6,
            SampleType::F32 => 1e-6,
        };
        for (pixel, (actual, expected)) in after.as_chunks::<4>().0.iter().zip(expected).enumerate() {
            for (channel, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                assert!((actual - expected).abs() <= tolerance, "{depth:?} pixel {pixel} channel {channel}: {actual} != {expected}");
            }
        }
        assert_eq!(&after[12..16], &before[12..16], "zero coverage must retain hidden colour");
        if depth == SampleType::F32 {
            assert_eq!(&after[16..20], &candidate[4], "full coverage must retain the candidate's hidden colour");
        }
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(samples(&s, layer), before);
    }
}

#[test]
fn unlocked_selection_retains_extreme_hdr_and_subnormal_alpha_without_overflow() {
    let h = f32::MAX;
    let tiny = f32::from_bits(1);
    let old = [[h, -h, h, 1.0], [0.0, 0.0, 0.0, tiny], [h, -h, h, 0.0], [h, -h, h, 0.0], [0.0, 0.0, 0.0, 1.0], [h, -h, h, 1.0]];
    let candidate = [[-h, h, -h, 0.25], [h, -h, h, tiny], [-h, h, -h, 0.0], [-h, h, -h, 1.0], [h, -h, h, 0.0], [h, -h, h, 1.0]];
    let expected = [[0.6 * h, -0.6 * h, 0.6 * h, 0.625], [0.5 * h, -0.5 * h, 0.5 * h, tiny], [0.0, 0.0, 0.0, 0.0], old[3], candidate[4], [h, -h, h, 1.0]];
    let (mut s, layer) = candidate_session(SampleType::F32, &old, &candidate, &[0.5, 0.5, 0.5, 0.0, 1.0, 0.5]);
    s.execute(RUN, params("image", "pixels")).unwrap();
    for (pixel, (actual, expected)) in samples(&s, layer).as_chunks::<4>().0.iter().zip(expected).enumerate() {
        for (channel, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert!(actual.is_finite(), "pixel {pixel} channel {channel}");
            if expected == 0.0 || expected.abs() <= 1.0 {
                assert_eq!(*actual, expected, "pixel {pixel} channel {channel}");
            } else {
                assert!((f64::from(*actual) / f64::from(expected) - 1.0).abs() < 1e-6, "pixel {pixel} channel {channel}: {actual} != {expected}");
            }
        }
    }
}

#[test]
fn mask_selection_replaces_without_second_selection_multiplication_and_undoes() {
    let (mut s, layer, adapter) = session(ColorMode::Cmyk, SampleType::U16);
    s.edit("old selection", |doc, _| {
        doc.selection = Some(Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.25]));
        Ok(())
    })
    .unwrap();
    let before_pixels = samples(&s, layer);
    let history = s.active().unwrap().history.entries().len();
    let result = s.execute(RUN, params("mask", "selection")).unwrap();
    assert_eq!(result["output"], "selection");
    let selection = s.active().unwrap().doc.selection.as_ref().unwrap();
    assert_eq!(selection.format().sample, SampleType::F32);
    assert_eq!(selection.read_region(s.active().unwrap().doc.bounds()), vec![0.625; 6]);
    assert_eq!(samples(&s, layer), before_pixels);
    assert_eq!(s.active().unwrap().history.entries().len(), history + 1);
    let seen = adapter.seen.lock().unwrap();
    let input_selection = seen[0].selection.as_ref().unwrap();
    assert_eq!((input_selection.color_mode.as_str(), input_selection.source_depth, input_selection.channels, input_selection.alpha), ("gray", 32, 1, false));
    assert_eq!(input_selection.data, vec![0.25; 6]);
    assert!(input_selection.icc_profile.is_empty());
    drop(seen);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(0, 0, 0), 0.25);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(0, 0, 0), 0.625);
}

#[test]
fn mask_layer_destination_respects_locks_background_and_kind() {
    let (mut s, layer, _) = session(ColorMode::Rgb, SampleType::U16);
    for p in [params("image", "selection"), params("mask", "pixels"), params("depth", "pixels"), params("depth", "depthLayer")] {
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute(RUN, p).is_err());
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
    s.execute(RUN, params("mask", "layerMask")).unwrap();
    let mask = s.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap();
    assert_eq!(mask.surface.format(), PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false));
    assert!((mask.surface.sample_channel(0, 0, 0) - 0.625).abs() <= 1.0 / 65535.0);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.layer(layer).unwrap().mask.is_none());
    for all in [false, true] {
        s.edit("lock", |doc, _| {
            doc.layer_mut(layer).unwrap().locks.pixels = !all;
            doc.layer_mut(layer).unwrap().locks.all = all;
            Ok(())
        })
        .unwrap();
        for output in ["pixels", "layerMask"] {
            let before = s.active().unwrap().doc.clone();
            assert!(s.execute(RUN, params(if output == "pixels" { "image" } else { "mask" }, output)).is_err());
            assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
        }
    }
    let mut bg = Session::new();
    bg.execute("file.new", json!({"width": 2, "height": 2})).unwrap();
    bg.register_adapter(Arc::new(Reference::default())).unwrap();
    let before = bg.active().unwrap().doc.clone();
    assert!(bg.execute(RUN, params("mask", "layerMask")).is_err());
    assert!(Arc::ptr_eq(&before, &bg.active().unwrap().doc));
}

#[test]
fn unsupported_input_targets_and_non_raster_layers_are_explicit_errors() {
    use crate::channel_cmds::ChannelTarget;
    let (mut s, layer, adapter) = session(ColorMode::Rgb, SampleType::F32);
    for target in [ChannelTarget::Color(0), ChannelTarget::Alpha(0), ChannelTarget::LayerMask, ChannelTarget::VectorMask] {
        s.active_mut().unwrap().channel_view.target = target;
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute(RUN, params("image", "pixels")).is_err());
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
    s.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
    s.tools.mask_targeted = true;
    assert!(s.execute(RUN, params("image", "pixels")).is_err());
    s.tools.mask_targeted = false;
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(s.execute(RUN, params("image", "pixels")).is_err());
    s.execute("edit.undo", json!({})).unwrap();
    s.edit("group", |doc, _| {
        doc.layer_mut(layer).unwrap().content = Layer::group("Group", Vec::new()).content;
        Ok(())
    })
    .unwrap();
    assert!(s.execute(RUN, params("image", "pixels")).is_err());
    assert!(adapter.seen.lock().unwrap().is_empty());
}

#[test]
fn adapter_queries_and_refused_runs_never_commit_a_floating_selection() {
    let (mut s, _, reference) = session(ColorMode::Rgb, SampleType::F32);
    s.edit("selection", |doc, _| {
        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        selection.fill_rect(Rect::new(0, 0, 2, 1), &[1.0]);
        doc.selection = Some(selection);
        Ok(())
    })
    .unwrap();
    s.execute("select.float", json!({"dx": 1, "dy": 1})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.entries();
    let floating = s.active().unwrap().floating.clone().unwrap();
    s.execute(LIST, json!({})).unwrap();
    for p in [params("image", "pixels"), json!({"provider": false}), params("mask", "selection")] {
        assert!(s.execute(RUN, p).is_err());
        let st = s.active().unwrap();
        assert!(Arc::ptr_eq(&before, &st.doc));
        assert_eq!(st.history.entries(), history);
        let now = st.floating.as_ref().unwrap();
        assert_eq!((now.layer, now.offset, now.revision), (floating.layer, floating.offset, floating.revision));
        assert!(Arc::ptr_eq(&now.parts, &floating.parts));
    }
    assert!(reference.seen.lock().unwrap().is_empty());
}

#[test]
fn expanded_document_modes_and_mismatched_layer_formats_are_not_mislabeled() {
    for mode in [ColorMode::Indexed, ColorMode::Bitmap, ColorMode::Duotone, ColorMode::Multichannel] {
        let (mut s, _, reference) = session(mode, SampleType::U8);
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute(RUN, params("image", "pixels")).as_ref().is_err_and(|e| e.to_string().contains("unsupported input mode")));
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
        assert!(reference.seen.lock().unwrap().is_empty());
    }
    let (mut s, layer, reference) = session(ColorMode::Rgb, SampleType::U16);
    s.edit("mismatched layer", |doc, _| {
        doc.layer_mut(layer).unwrap().content = LayerContent::Raster(Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, true)));
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute(RUN, params("image", "pixels")).is_err());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    assert!(reference.seen.lock().unwrap().is_empty());
}

#[test]
fn oversized_opaque_resource_and_parameters_are_rejected_without_invocation() {
    let (mut s, _, reference) = session(ColorMode::Rgb, SampleType::U8);
    let limits = Limits::default();
    for (key, value) in [
        ("resource", json!("x".repeat(limits.max_resource_bytes + 1))),
        ("params", json!({"value": "x".repeat(limits.max_json_bytes + 1)})),
        ("params", json!({"values": vec![Value::Null; limits.max_json_array_items + 1]})),
    ] {
        let mut p = params("image", "pixels");
        p[key] = value;
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute(RUN, p).is_err());
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
    let mut nested = json!({});
    for _ in 0..limits.max_json_depth + 2 {
        nested = json!({"nested": nested});
    }
    assert!(checked_params(&nested, &limits).is_err());
    assert!(reference.seen.lock().unwrap().is_empty());
}

#[test]
fn preflight_shares_exact_escaped_json_and_root_depth_budgets_without_side_effects() {
    // Shared validation counts compact JSON syntax and escapes, not only raw UTF-8 content.
    let escaped = json!({"\u{0001}\"\\": ["\u{0000}\n\t\"\\", true, false, null, -3.25]});
    let encoded = serde_json::to_vec(&escaped).unwrap();
    let exact = Limits { max_json_bytes: encoded.len(), ..Limits::default() };
    assert!(exact.check_params(&escaped).is_ok());
    assert_eq!(checked_params(&escaped, &exact).unwrap(), escaped);
    let short = Limits { max_json_bytes: encoded.len() - 1, ..exact };
    assert!(short.check_params(&escaped).is_err());
    assert!(checked_params(&escaped, &short).is_err());

    // The parameter object itself is depth one, including an empty root object.
    let root = json!({});
    let one = Limits { max_json_depth: 1, ..Limits::default() };
    assert!(one.check_params(&root).is_ok());
    assert!(checked_params(&root, &one).is_ok());
    let zero = Limits { max_json_depth: 0, ..one };
    assert!(zero.check_params(&root).is_err());
    assert!(checked_params(&root, &zero).is_err());
    let nested = json!({"outer": {"inner": 1}});
    let three = Limits { max_json_depth: 3, ..Limits::default() };
    assert!(three.check_params(&nested).is_ok());
    assert_eq!(checked_params(&nested, &three).unwrap(), nested);
    let two = Limits { max_json_depth: 2, ..three };
    assert!(two.check_params(&nested).is_err());
    assert!(checked_params(&nested, &two).is_err());

    let defaults = Limits::default();
    let oversized_escaped = json!({"escaped": "\u{0000}".repeat(defaults.max_json_bytes / 6)});
    assert!(oversized_escaped["escaped"].as_str().unwrap().len() < defaults.max_json_bytes);
    assert!(serde_json::to_vec(&oversized_escaped).unwrap().len() > defaults.max_json_bytes);
    assert!(defaults.check_params(&oversized_escaped).is_err());
    let mut deepest_allowed = json!({});
    for _ in 1..defaults.max_json_depth {
        deepest_allowed = json!({"nested": deepest_allowed});
    }
    assert!(defaults.check_params(&deepest_allowed).is_ok());
    let too_deep = json!({"nested": deepest_allowed});
    assert!(defaults.check_params(&too_deep).is_err());

    let (mut s, _, reference) = session(ColorMode::Rgb, SampleType::F32);
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.entries();
    let revision = s.active().unwrap().revision;
    let journal = s.journal.clone();
    for (key, value) in [("params", oversized_escaped), ("params", too_deep), ("coalesce", json!("x".repeat(1025)))] {
        let mut p = params("image", "pixels");
        p[key] = value;
        assert!(matches!(preflight(RUN, &p), Err(EngineError::BadParams { .. })), "{key}");
        assert!(matches!(s.execute(RUN, p.clone()), Err(EngineError::BadParams { .. })), "{key}");
        assert!(matches!(s.start(RUN, p), Err(EngineError::BadParams { .. })), "{key}");
        let st = s.active().unwrap();
        assert!(Arc::ptr_eq(&before, &st.doc), "{key}");
        assert_eq!(st.history.entries(), history, "{key}");
        assert_eq!(st.revision, revision, "{key}");
        assert_eq!(s.journal, journal, "{key}");
        assert!(!s.has_jobs(), "{key}");
        assert!(reference.seen.lock().unwrap().is_empty(), "{key}");
    }
    let mut boundary = params("image", "pixels");
    boundary["coalesce"] = json!("x".repeat(1024));
    assert!(preflight(RUN, &boundary).is_ok());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    assert!(reference.seen.lock().unwrap().is_empty());
}

#[test]
fn cancel_before_or_during_inline_work_never_writes_back_or_journals() {
    let (mut s, _, _) = session(ColorMode::Rgb, SampleType::F32);
    let ctx = JobCtx::new();
    ctx.cancel();
    s.set_inline_job_ctx(Some(ctx));
    let before = s.active().unwrap().doc.clone();
    let journal = s.journal.len();
    assert!(matches!(s.execute(RUN, params("image", "pixels")), Err(EngineError::Cancelled)));
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    assert_eq!(s.journal.len(), journal);

    let (mut s, _, _) = session(ColorMode::Rgb, SampleType::F32);
    let ctx = JobCtx::new();
    let reference = Reference { cancel: Some(ctx.clone()), ..Reference::default() };
    let mut descriptor = reference.descriptor();
    descriptor.id = "org.photocraft.test.cancel".into();
    struct Cancel {
        reference: Reference,
        descriptor: Descriptor,
    }
    impl Adapter for Cancel {
        fn descriptor(&self) -> Descriptor {
            self.descriptor.clone()
        }
        fn run(&self, r: &Request, c: &dyn adapter::Context) -> adapter::Result<Response> {
            self.reference.run(r, c)
        }
    }
    s.register_adapter(Arc::new(Cancel { reference, descriptor })).unwrap();
    s.set_inline_job_ctx(Some(ctx));
    let before = s.active().unwrap().doc.clone();
    let mut p = params("image", "pixels");
    p["provider"] = json!("org.photocraft.test.cancel");
    assert!(matches!(s.execute(RUN, p), Err(EngineError::Cancelled)));
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
}

#[test]
fn explicit_device_never_silently_falls_back() {
    let (mut s, _, reference) = session(ColorMode::Rgb, SampleType::U8);
    let before = s.active().unwrap().doc.clone();
    let mut p = params("image", "pixels");
    p["device"] = json!("gpu");
    assert!(s.execute(RUN, p).is_err());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    assert!(reference.seen.lock().unwrap().is_empty());

    struct Fallback;
    impl Adapter for Fallback {
        fn descriptor(&self) -> Descriptor {
            let mut descriptor = Reference::default().descriptor();
            descriptor.id = "org.photocraft.test.fallback".into();
            descriptor.devices.push(Device {
                id: "gpu".into(),
                kind: DeviceKind::Gpu,
                backend: "gpu-test".into(),
                name: "Test assertion only, not probed GPU".into(),
                operations: vec!["image".into()],
            });
            descriptor
        }
        fn run(&self, request: &Request, ctx: &dyn adapter::Context) -> adapter::Result<Response> {
            let mut response = Reference::default().run(request, ctx)?;
            response.execution.fallback_reason = Some("test GPU unavailable; used CPU".into());
            Ok(response)
        }
    }
    s.register_adapter(Arc::new(Fallback)).unwrap();
    let mut p = params("image", "pixels");
    p["provider"] = json!("org.photocraft.test.fallback");
    p["device"] = json!("gpu");
    assert!(s.execute(RUN, p.clone()).is_err());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    p["allowFallback"] = json!(true);
    let result = s.execute(RUN, p).unwrap();
    assert_eq!(result["execution"]["devices"], json!(["cpu"]));
    assert!(result["execution"]["fallback_reason"].as_str().unwrap().contains("GPU unavailable"));
}

#[test]
fn dimensions_samples_icc_and_allocation_are_checked_before_read_or_mutation() {
    let (mut s, _, reference) = session(ColorMode::Rgb, SampleType::U8);
    for size in [Size::new(u32::MAX, 1), Size::new(0, 1), Size::new(32768, 32768)] {
        let st = s.active_mut().unwrap();
        Arc::make_mut(&mut st.doc).size = size;
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute(RUN, params("image", "pixels")).is_err());
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
    assert!(reference.seen.lock().unwrap().is_empty());
    let (mut s, _, reference) = session(ColorMode::Rgb, SampleType::U8);
    Arc::make_mut(&mut s.active_mut().unwrap().doc).icc_profile = Some(Arc::new(vec![0; adapter::MAX_ICC_BYTES + 1]));
    assert!(s.execute(RUN, params("image", "pixels")).is_err());
    assert!(reference.seen.lock().unwrap().is_empty());
    let (mut s, _, reference) = session(ColorMode::Rgb, SampleType::U8);
    let before = s.active().unwrap().doc.clone();
    crate::allocation::fail_next_for_test();
    assert!(s.execute(RUN, params("image", "pixels")).is_err());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    assert!(reference.seen.lock().unwrap().is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn background_job_progress_completion_undo_cancel_and_stale_document() {
    use crate::jobs::{JobOutcome, Started};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    struct Waiting {
        entered: Arc<AtomicBool>,
        release: Arc<AtomicBool>,
    }
    impl Adapter for Waiting {
        fn descriptor(&self) -> Descriptor {
            Reference::default().descriptor()
        }
        fn run(&self, request: &Request, ctx: &dyn adapter::Context) -> adapter::Result<Response> {
            ctx.progress(0.4);
            self.entered.store(true, Ordering::Release);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !self.release.load(Ordering::Acquire) {
                if ctx.cancelled() {
                    return Err(adapter::Error::Canceled);
                }
                if Instant::now() > deadline {
                    return Err(adapter::Error::Failed("test release timeout".into()));
                }
                std::thread::yield_now();
            }
            Reference::default().run(request, ctx)
        }
    }
    fn wait_entered(entered: &AtomicBool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !entered.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "worker did not start");
            std::thread::yield_now();
        }
    }
    for state in ["success", "cancel", "stale"] {
        let (base, layer, _) = session(ColorMode::Rgb, SampleType::F32);
        let mut s = Session::new();
        s.add_document((*base.active().unwrap().doc).clone(), None);
        let entered = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        s.register_adapter(Arc::new(Waiting { entered: entered.clone(), release: release.clone() })).unwrap();
        let before = s.active().unwrap().doc.clone();
        let journal = s.journal.len();
        let Started::Job(job) = s.start(RUN, params("image", "pixels")).unwrap() else { panic!("expected background job") };
        wait_entered(&entered);
        assert_eq!(s.job(job).unwrap().progress, 0.4);
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
        // Removing the registration blocks new runs but does not invalidate this job's Arc.
        assert!(s.unregister_adapter(ID));
        assert_eq!(s.execute(LIST, json!({})).unwrap()["installed"], json!([]));
        if state == "cancel" {
            assert!(s.cancel_job(job));
        } else if state == "stale" {
            s.edit("external edit", |doc, _| {
                doc.name = "Changed while running".into();
                Ok(())
            })
            .unwrap();
        }
        release.store(true, Ordering::Release);
        let result = s.wait_job(job);
        match state {
            "success" => {
                assert!(result.is_ok(), "{result:?}");
                assert_ne!(samples(&s, layer), samples(&base, layer));
                assert_eq!(s.journal.len(), journal + 1);
                s.execute("edit.undo", json!({})).unwrap();
                assert_eq!(samples(&s, layer), samples(&base, layer));
            }
            "cancel" => {
                assert!(result.is_err());
                assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
                assert_eq!(s.journal.len(), journal);
                assert!(s.poll_jobs().iter().any(|event| event.outcome == JobOutcome::Cancelled));
            }
            _ => {
                assert!(result.as_ref().is_err_and(|e| e.to_string().contains("changed while")), "{result:?}");
                assert_eq!(samples(&s, layer), samples(&base, layer));
                assert_eq!(s.active().unwrap().doc.name, "Changed while running");
                assert_eq!(s.journal.len(), journal);
            }
        }
    }
}
