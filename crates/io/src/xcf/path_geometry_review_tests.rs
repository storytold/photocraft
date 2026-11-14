//! Independent wire fixtures must not turn omitted stroke geometry into a replacement raster.
use super::tests::{W, hierarchy};
use super::*;
use photocraft_doc::import_conversion::ImportConversion;

#[derive(Clone, Copy)]
struct SourcePath<'a> {
    tattoo: u32,
    strokes: &'a [u32],
    axes: u32,
    extra_metadata: bool,
}

fn geometry(w: &mut W, source: SourcePath<'_>) {
    for &kind in source.strokes {
        for value in [kind, 1, source.axes, 3] {
            w.u32(value);
        }
        for (tag, x, y) in [(1, 0.0f32, 0.0f32), (0, 1., 1.), (1, 2., 2.)] {
            w.u32(tag);
            w.u32(x.to_bits());
            w.u32(y.to_bits());
            for _ in 2..source.axes {
                w.u32(0.5f32.to_bits());
            }
        }
    }
}

fn vector_style(tattoo: u32) -> Vec<u8> {
    let mut w = W { out: Vec::new(), wide: false };
    for value in [0, tattoo, 1, 0] {
        w.u32(value); // Unmodified pixels, local path, fill enabled, stroke disabled.
    }
    for _ in 0..2 {
        w.u32(0); // Solid paint.
        w.u32(1); // Antialias.
        w.string("R'G'B'A u8");
        w.u32(4);
        w.out.extend([255, 0, 0, 255]);
        w.u32(0); // No profile.
        w.string(""); // No resource.
    }
    for value in [1f32.to_bits(), 0, 0, 4f32.to_bits(), 0] {
        w.u32(value);
    }
    w.out
}

fn fixture(standalone: bool, paths: &[SourcePath<'_>], references: &[u32]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut w = W::new(26);
    for value in [1, 1, 0, 150] {
        w.u32(value);
    }
    let mut retained = Vec::new();
    if !standalone {
        let mut payload = W { out: Vec::new(), wide: false };
        for value in [1, 0, paths.len() as u32] {
            payload.u32(value);
        }
        for &path in paths {
            payload.string("Source path");
            for value in [path.tattoo, u32::from(path.extra_metadata), u32::from(path.extra_metadata), 0, path.strokes.len() as u32] {
                payload.u32(value);
            }
            geometry(&mut payload, path);
        }
        w.prop(25, &payload.out);
        retained.push(payload.out);
    }
    w.prop(17, &[0]);
    w.end();
    let layers: Vec<_> = references.iter().map(|_| w.slot()).collect();
    w.ptr(0);
    w.ptr(0); // Channels.
    let pointers: Vec<_> = if standalone { paths.iter().map(|_| w.slot()).collect() } else { Vec::new() };
    w.ptr(0); // Path pointer terminator.
    for (&layer, &tattoo) in layers.iter().zip(references) {
        let at = w.out.len();
        w.patch(layer, at);
        for value in [1, 1, 1] {
            w.u32(value);
        }
        w.string(&format!("Vector {tattoo}"));
        w.prop(47, &vector_style(tattoo));
        w.end();
        let raster = w.slot();
        w.ptr(0); // Mask.
        w.ptr(0); // Effects.
        let at = hierarchy(&mut w, 1, 1, 4, &[17, 31, 57, 255], Compression::None);
        w.patch(raster, at);
    }
    for (&pointer, &path) in pointers.iter().zip(paths) {
        let at = w.out.len();
        w.patch(pointer, at);
        w.string("Source path");
        let mut payload = W { out: Vec::new(), wide: false };
        payload.prop_u32(20, path.tattoo);
        payload.prop(43, &[]);
        if path.extra_metadata {
            payload.prop_u32(99, 1);
        }
        payload.end();
        payload.u32(1);
        payload.u32(path.strokes.len() as u32);
        geometry(&mut payload, path);
        w.u32(payload.out.len() as u32);
        w.out.extend(payload.out);
        retained.push(w.out[at..].to_vec());
    }
    (w.out, retained)
}

fn assert_saved_raster(layer: &Layer) {
    let LayerContent::Raster(pixels) = &layer.content else { panic!("omitted geometry must not replace saved pixels") };
    assert_eq!(pixels.pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
}

#[test]
fn unsupported_or_partial_path_geometry_preserves_vector_pixels_and_native_report() {
    for standalone in [false, true] {
        for strokes in [&[2][..], &[1, 2][..], &[2, 1][..]] {
            let source = SourcePath { tattoo: 72, strokes, axes: 2, extra_metadata: false };
            let (bytes, retained) = fixture(standalone, &[source], &[72]);
            let imported = crate::import("incomplete-vector.xcf", &bytes).unwrap();
            assert!(imported.source_read_only && !imported.preview_only);
            assert_saved_raster(&imported.document.layers[0]);
            assert_eq!(imported.document.paths[0].path.subpaths.len(), usize::from(strokes.contains(&1)));
            assert!(imported.warnings.iter().any(|warning| warning.contains("incomplete vector geometry") && warning.contains("opens as pixels")));
            let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("typed report") };
            for feature in ["xcf.path", "xcf.vector_layer"] {
                assert!(report.entries.iter().any(|entry| entry.feature_code == feature && entry.reason == ConversionReason::MissingMapping));
            }
            assert!(
                report
                    .entries
                    .iter()
                    .any(|entry| entry.feature_code == "xcf.vector_layer" && entry.targets.iter().any(|t| t.layer_id == Some(imported.document.layers[0].id)))
            );
            for raw in retained {
                assert!(report.attachments.iter().any(|attachment| attachment.bytes.as_slice() == raw));
            }
            let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            assert_saved_raster(&reopened.layers[0]);
            assert_eq!(reopened.paths, imported.document.paths);
            assert_eq!(reopened.metadata.import_conversion, imported.document.metadata.import_conversion);
        }
    }
}

#[test]
fn complete_empty_and_auxiliary_only_paths_still_install_editable_shapes() {
    for standalone in [false, true] {
        for (strokes, axes, extra_metadata) in [(&[][..], 2, false), (&[1][..], 2, false), (&[1][..], 6, true)] {
            let source = SourcePath { tattoo: 72, strokes, axes, extra_metadata };
            let (bytes, _) = fixture(standalone, &[source], &[72]);
            let imported = crate::import("complete-vector.xcf", &bytes).unwrap();
            let LayerContent::Shape(shape) = &imported.document.layers[0].content else { panic!("complete geometry remains editable") };
            assert_eq!(shape.path, imported.document.paths[0].path);
            assert_eq!(shape.path.subpaths.len(), strokes.len());
            assert!(!imported.warnings.iter().any(|warning| warning.contains("incomplete vector geometry")));
            let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
            let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
            let LayerContent::Shape(shape) = &reopened.layers[0].content else { panic!("native editable shape") };
            assert_eq!(shape.path, imported.document.paths[0].path);
        }
    }
}

#[test]
fn incomplete_geometry_is_owner_local_and_duplicate_tattoos_stay_ambiguous() {
    for standalone in [false, true] {
        let incomplete = SourcePath { tattoo: 72, strokes: &[2], axes: 2, extra_metadata: false };
        let complete = SourcePath { tattoo: 73, strokes: &[1], axes: 2, extra_metadata: false };
        let (bytes, _) = fixture(standalone, &[incomplete, complete], &[72, 73]);
        let imported = crate::import("two-vectors.xcf", &bytes).unwrap();
        assert_saved_raster(imported.document.layers.iter().find(|layer| layer.name == "Vector 72").unwrap());
        assert!(matches!(imported.document.layers.iter().find(|layer| layer.name == "Vector 73").unwrap().content, LayerContent::Shape(_)));
        let duplicate = SourcePath { tattoo: 72, ..complete };
        for paths in [[incomplete, duplicate], [duplicate, incomplete]] {
            let (bytes, _) = fixture(standalone, &paths, &[72]);
            assert_saved_raster(&crate::import("ambiguous-vector.xcf", &bytes).unwrap().document.layers[0]);
        }
    }
}
