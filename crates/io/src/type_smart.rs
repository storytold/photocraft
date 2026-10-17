//! PSD export of type with a free Warp, Distort or Perspective ("Better Than PS",
//! [`photocraft_doc::text::TypeDistortion`]). Photoshop can't keep those on a type layer, so such
//! a layer is written as an embedded Smart Object holding the live type layer (a PSB cropped to
//! the type), placed with the distortion as the Smart Object's warp and Distort / Perspective.
//! Photoshop shows it as it looks here, and Edit Contents opens the editable type. Re-imported,
//! it stays a Smart Object.

use photocraft_doc::text::TypeDistortionKind;
use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource, TextLayer};
use photocraft_geom::Affine;
use photocraft_geom::warp::{BezierMesh, Warp, WarpStyle};

/// `doc` with every distorted type layer turned into a Smart Object holding it; `None` when
/// there is none (the document is written as it is).
pub(crate) fn with_smart_type(doc: &Document, warnings: &mut Vec<String>) -> Option<Document> {
    if !doc.walk().iter().any(|(_, _, l)| matches!(&l.content, LayerContent::Text(t) if !t.distortions.is_empty())) {
        return None;
    }
    let mut out = doc.clone();
    convert(doc, &mut out.layers, warnings, 0);
    Some(out)
}

fn convert(doc: &Document, layers: &mut [Layer], warnings: &mut Vec<String>, depth: usize) {
    for l in layers.iter_mut() {
        match &mut l.content {
            LayerContent::Group(g) if depth < 256 => convert(doc, &mut g.children, warnings, depth + 1),
            LayerContent::Text(t) if !t.distortions.is_empty() => match smart_of(doc, t, &l.name) {
                Ok(sm) => {
                    l.content = LayerContent::Smart(sm);
                    l.psd_blocks.retain(|(k, _)| k != b"TySh");
                    warnings.push(format!(
                        "layer \"{}\": type with a free warp or distort was written as a Smart Object holding the editable type (Photoshop keeps neither on type)",
                        l.name
                    ));
                }
                Err(e) => warnings.push(format!("layer \"{}\": its free warp or distort was not written to PSD ({e})", l.name)),
            },
            _ => {}
        }
    }
}

/// The Smart Object standing in for `t`: its source is the type without the distortions, on a
/// canvas cropped to it; the distortions become the placement's warp and perspective.
fn smart_of(doc: &Document, t: &TextLayer, name: &str) -> Result<SmartObject, String> {
    let fmt = doc.pixel_format();
    let mut plain = t.clone();
    plain.distortions.clear();
    plain.psd_raw = None;
    let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    crate::text_import::render_bounded(&mut eng, &mut plain, doc.resolution_dpi, fmt);
    let b = plain.cache.as_ref().map(|c| c.content_bounds()).filter(|b| !b.is_empty()).ok_or("the type is empty")?;
    let (w, h) = (b.width(), b.height());
    // The source: the type moved to the cropped canvas' origin.
    let origin = Affine::translate(f64::from(b.x0), f64::from(b.y0));
    let mut inner_text = plain.clone();
    inner_text.transform = Affine::translate(-f64::from(b.x0), -f64::from(b.y0)).mul(&t.transform);
    crate::text_import::render_bounded(&mut eng, &mut inner_text, doc.resolution_dpi, fmt);
    drop(eng);
    let mut inner = Document::new(name, photocraft_geom::Size::new(w, h), doc.mode, doc.depth);
    inner.resolution_dpi = doc.resolution_dpi;
    inner.icc_profile = doc.icc_profile.clone();
    inner.text_styles = doc.text_styles.clone();
    inner.layers.push(Layer::new(name, LayerContent::Text(inner_text)));
    let bytes = photocraft_format::save_to_bytes(&inner, &Default::default()).map_err(|e| e.to_string())?;

    // Document-space distortions: a trailing projective map is the placement, the rest the warp.
    let kinds: Vec<TypeDistortionKind> = t.distortions.iter().filter_map(|d| d.in_document(&t.transform)).collect();
    let (rest, projective) = match kinds.split_last() {
        Some((TypeDistortionKind::Projective(p), rest)) => (rest, Some(*p)),
        _ => (kinds.as_slice(), None),
    };
    let rect = [f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)];
    let src = [0.0, 0.0, f64::from(w), f64::from(h)];
    let near = |a: [f64; 4], c: [f64; 4]| a.iter().zip(c).all(|(x, y)| (x - y).abs() < 0.75);
    let warp = match rest {
        [] => None,
        // One warp over the type's box: exact (moved to the source origin).
        [TypeDistortionKind::Warp(wp)] if near(wp.bounds, rect) && (wp.style.is_preset() || wp.mesh.as_ref().is_some_and(|m| m.points.len() == 16)) => {
            let shift = |p: [f64; 2]| [p[0] - rect[0], p[1] - rect[1]];
            Some(Warp { bounds: src, mesh: wp.mesh.as_ref().map(|m| m.map_points(shift)), ..wp.clone() })
        }
        // Anything else: the whole map fitted as one patch (what a PSD warp holds).
        _ => {
            let n = rest.len();
            let doc_map = |x: f64, y: f64| {
                let (mut x, mut y) = (x, y);
                for d in t.distortions.iter().take(n) {
                    (x, y) = d.map_in_document(&t.transform, x, y);
                }
                (x, y)
            };
            let mesh = BezierMesh::fit(
                &|s, u| {
                    let (x, y) = doc_map(rect[0] + s * (rect[2] - rect[0]), rect[1] + u * (rect[3] - rect[1]));
                    [x - rect[0], y - rect[1]]
                },
                vec![0.0, 1.0],
                vec![0.0, 1.0],
            );
            Some(Warp::custom(mesh, src))
        }
    }
    .filter(|w| w.style != WarpStyle::None && !w.is_identity());
    let mut sm = SmartObject::new(SmartSource::Embedded { file_name: format!("{name}.pcraft"), bytes: std::sync::Arc::new(bytes) }, origin, t.cache.clone());
    sm.warp = warp;
    // Source pixels → document: the crop's offset, then the Distort / Perspective.
    sm.perspective = projective.map(|p| photocraft_doc::text::mat3_mul(&p, &[1.0, 0.0, rect[0], 0.0, 1.0, rect[1], 0.0, 0.0, 1.0]));
    Ok(sm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::text::TypeDistortion;

    /// A type layer with a free warp is written as a Smart Object with that warp, holding the
    /// live type; plain type is written as type.
    #[test]
    fn distorted_type_exports_as_a_smart_object_holding_the_type() {
        let mut doc = Document::new("t", photocraft_geom::Size::new(300, 120), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let fmt = doc.pixel_format();
        let mut t = TextLayer { text: "Hello".into(), size_pt: 40.0, transform: Affine::translate(20.0, 80.0), ..Default::default() };
        let mut eng = photocraft_text::shared().lock().unwrap();
        crate::text_import::render_bounded(&mut eng, &mut t, 72.0, fmt);
        drop(eng);
        let mut plain = t.clone();
        plain.text = "Plain".into();
        let b = t.cache.as_ref().unwrap().content_bounds();
        let r = [f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)];
        let mut mesh = BezierMesh::identity(r, 1, 1);
        mesh.points[15][1] += 20.0;
        t.distortions.push(TypeDistortion { frame: t.transform, kind: TypeDistortionKind::Warp(Warp::custom(mesh, r)) });
        doc.layers.push(Layer::new("Plain", LayerContent::Text(plain)));
        doc.layers.push(Layer::new("Bent", LayerContent::Text(t)));
        let (file, warnings) = crate::document_to_psd_with(&doc, &Default::default());
        assert!(warnings.iter().any(|w| w.contains("Smart Object")), "{warnings:?}");
        let bytes = file.to_bytes().unwrap();
        let back = crate::import("t.psd", &bytes).unwrap().document;
        assert!(matches!(back.layers[0].content, LayerContent::Text(_)), "plain type stays type");
        let LayerContent::Smart(sm) = &back.layers[1].content else { panic!("not a smart object: {}", back.layers[1].content.kind_name()) };
        let w = sm.warp.as_ref().expect("the warp is the smart object's");
        assert_eq!(w.style, WarpStyle::Custom);
        // Imported embedded files are kept in the document's linked-file blocks, by id.
        let SmartSource::Linked { path } = &sm.source else { panic!("source") };
        let file = crate::linked::find_linked_file(&back.metadata, path).expect("embedded PSB");
        assert!(file.file_name.ends_with(".psb"), "{}", file.file_name);
        let inner = crate::import(&file.file_name, &file.bytes).unwrap().document;
        assert!(matches!(&inner.layers[0].content, LayerContent::Text(t) if t.text == "Hello" && t.distortions.is_empty()), "Edit Contents opens the type");
    }
}
