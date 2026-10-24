//! Exact stock GIMP core graph roles, lowered only to ordinary native data.
//! Schema: pinned GIMP3.2.6 app/operations/gimpoperation{offset,equalize,
//! cagecoefcalc,cagetransform,histogramsink}.c class declarations and
//! libgimpbase/gimpbaseenums.h GimpOffsetType. No processing code is used.
use super::graph_schema::Kind;
use super::residual_spatial::{bounds, choice, color, composite, geometry_bytes, integer, invalid, mapped, rect};
use super::{MappingContext, MappingFailure, MappingResult, NativeBranch, StoredEffect, native_filter};
use photocraft_doc::import_conversion::ConversionReason;
use photocraft_doc::{Fill, Path, VectorMask};
use serde_json::json;

const NAMES: &[&str] = &["gimp:offset", "gimp:dissolve", "gimp:equalize", "gimp:cage-coef-calc", "gimp:cage-transform", "gimp:histogram-sink"];
const OFFSET_TYPES: &[(&str, i32)] =
    &[("color", 0), ("GIMP_OFFSET_COLOR", 0), ("transparent", 1), ("GIMP_OFFSET_TRANSPARENT", 1), ("wrap-around", 2), ("GIMP_OFFSET_WRAP_AROUND", 2)];

pub(super) fn operation(token: &str) -> Option<&'static str> {
    // Unqualified names refer to GEGL's namespace; never alias them to GIMP.
    NAMES.iter().copied().find(|name| *name == token)
}

pub(super) fn field(op: &str, name: &str) -> Option<Kind> {
    Some(match (op, name) {
        ("gimp:offset", "x" | "y") => Kind::Integer { min: i64::from(i32::MIN), max: i64::from(i32::MAX) },
        ("gimp:offset", "type") => Kind::Choice(OFFSET_TYPES),
        ("gimp:offset", "color") => Kind::Color,
        ("gimp:equalize" | "gimp:histogram-sink", "histogram") => {
            Kind::Unavailable("the actual GimpHistogram object is required; graph text does not supply its samples")
        }
        ("gimp:cage-coef-calc" | "gimp:cage-transform", "config") => {
            Kind::Unavailable("actual GimpCageConfig geometry and coefficient resources are required, not an invented default cage")
        }
        ("gimp:cage-transform", "fill-plain-color") => Kind::Boolean,
        ("gimp:histogram-sink", "aux") => Kind::Unavailable("the analysis sink's auxiliary GEGL buffer requires actual resource data"),
        ("gimp:dissolve", _) => return super::graph_schema::field("gimp:layer-mode", name),
        _ => return None,
    })
}

fn offset(e: &StoredEffect<'_>, c: &MappingContext) -> Result<super::NativeMapping, MappingFailure> {
    let x = integer(e, "x", 0, i32::MIN, i32::MAX)?;
    let y = integer(e, "y", 0, i32::MIN, i32::MAX)?;
    let kind = choice(e, "type", 2, &[0, 1, 2])?;
    let filter = native_filter("filter.other.offset", json!({"horizontal":x,"vertical":y,"undefinedAreas":if kind == 2 {"wrap"} else {"transparent"}}));
    if kind != 0 {
        // Validate a supplied literal even when its source control is inactive.
        if e.argument("color").map_err(invalid)?.is_some() {
            color(e, "color", [1.; 4], c)?;
        }
        return Ok(mapped(
            vec![filter],
            "Stock offset x/y become ordinary native integer Offset controls; source wrap/transparent maps to native wrap/transparent. Native finite filter frame, document profile/depth and alpha apply; source color is inactive in these modes.",
        ));
    }
    let b = bounds(c)?;
    geometry_bytes(c, 4, 16)?;
    let paint = color(e, "color", [1.; 4], c)?;
    let (x0, y0, x1, y1) = (f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1));
    let (dx, dy) = (f64::from(x), f64::from(y));
    let (ix0, iy0, ix1, iy1) = ((x0 + dx).max(x0), (y0 + dy).max(y0), (x1 + dx).min(x1), (y1 + dy).min(y1));
    let mut paths = Vec::with_capacity(4);
    if ix0 >= ix1 || iy0 >= iy1 {
        paths.push(rect(x0, y0, x1 - x0, y1 - y0));
    } else {
        for (rx, ry, rw, rh) in [(x0, y0, ix0 - x0, y1 - y0), (ix1, y0, x1 - ix1, y1 - y0), (ix0, y0, ix1 - ix0, iy0 - y0), (ix0, iy1, ix1 - ix0, y1 - iy1)] {
            if rw > 0. && rh > 0. {
                paths.push(rect(rx, ry, rw, rh));
            }
        }
    }
    if paths.is_empty() {
        return Ok(mapped(
            vec![filter],
            "Zero integer offset leaves no vacated native frame region; source fill color is inactive. Ordinary native Offset remains editable.",
        ));
    }
    Ok(composite(
        vec![
            NativeBranch { fill: Some(Fill::Solid(paint)), vector_mask: Some(VectorMask::new(Path::new(paths))), filter_frame: Some(b), ..Default::default() },
            NativeBranch { filters: vec![filter], ..Default::default() },
        ],
        "Stock offset color mode becomes ordinary transparent native Offset above an editable native SolidFill restricted to at most four vacated-frame vector rectangles. The fill does not replace transparency inside shifted source pixels. Explicit source literal/default-white color passes through native ICC conversion. Native finite frame, mask antialiasing, alpha and document compositing apply; no source offset callback remains.",
    ))
}

pub(super) fn map(e: &StoredEffect<'_>, c: &MappingContext) -> MappingResult {
    operation(e.operation)?;
    Some((|| {
        for argument in &e.arguments {
            if field(e.operation, argument.name).is_none() {
                return Err(invalid(format!("{} has no declared core property {}", e.operation, argument.name)));
            }
        }
        match e.operation {
            "gimp:offset" => offset(e, c),
            "gimp:equalize" => Err(MappingFailure::resource("Known stock Equalize consumes an actual serialized GimpHistogram object. No histogram samples are available here; the importer does not invent an owner-derived histogram or execute a source analysis operation.")),
            "gimp:cage-coef-calc" => Err(MappingFailure::resource("Known stock cage coefficient source requires actual GimpCageConfig geometry; it produces coefficient-buffer data, not an ordinary image identity. Missing geometry is retained without synthesizing a cage.")),
            "gimp:cage-transform" => Err(MappingFailure::resource("Known stock cage transform consumes actual cage configuration and auxiliary coefficient-buffer resources and produces coordinate data. Missing source geometry/resources are retained; no source evaluator or default cage is constructed.")),
            "gimp:histogram-sink" => Err(MappingFailure { reason: ConversionReason::NativePolicy, message: "Known stock histogram analysis sink has no image output. Its analysis object/buffer request is retained without executing a sink or pretending it is a pixel identity.".into() }),
            "gimp:dissolve" => Err("Known stock Dissolve is a binary layer-mode compositor; native Dissolve requires its actual main/auxiliary branch join, not a unary image filter.".into()),
            _ => Err(invalid("unrecognized exact core operation")),
        }
    })())
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, ArgumentValue, NativeStructure};
    use super::*;
    use photocraft_color::ColorMode;
    use photocraft_geom::Rect;
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            resolution_dpi: 72.,
            linear: false,
            owner_bounds: Rect::new(0, 0, 8, 6),
            native_filter_bounds: Rect::new(0, 0, 8, 6),
            target_profile: None,
            remaining_generation_bytes: usize::MAX,
            remaining_metadata_bytes: usize::MAX,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn number(name: &'static str, value: f64) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Literal(value) }
    }
    fn kind(value: i32) -> Argument<'static> {
        Argument { name: "type", value: ArgumentValue::Enum(value) }
    }
    #[test]
    fn exact_core_namespace_and_resource_roles_are_bounded() {
        for name in NAMES {
            assert_eq!(operation(name), Some(*name));
        }
        for unknown in ["offset", "dissolve", "gegl:offset", "gimp:future"] {
            assert_eq!(operation(unknown), None);
        }
        for name in ["gimp:equalize", "gimp:cage-coef-calc", "gimp:cage-transform"] {
            assert_eq!(map(&StoredEffect::test(name, vec![]), &context()).unwrap().unwrap_err().reason, ConversionReason::ResourceUnavailable);
        }
        let sink = map(&StoredEffect::test("gimp:histogram-sink", vec![]), &context()).unwrap().unwrap_err();
        assert_eq!(sink.reason, ConversionReason::NativePolicy);
        assert!(sink.message.contains("no image output"));
        assert!(matches!(field("gimp:equalize", "histogram"), Some(Kind::Unavailable(_))));
        assert!(matches!(field("gimp:cage-transform", "fill-plain-color"), Some(Kind::Boolean)));
    }
    #[test]
    fn offset_uses_actual_native_schema_and_rejects_invalid_source_domains() {
        let e = StoredEffect::test("gimp:offset", vec![number("x", f64::from(i32::MIN)), number("y", 3.)]);
        let mapped = map(&e, &context()).unwrap().unwrap();
        assert_eq!(mapped.filters[0].command, "filter.other.offset");
        assert_eq!(mapped.filters[0].params, json!({"horizontal":i32::MIN,"vertical":3,"undefinedAreas":"wrap"}));
        for args in
            [vec![number("x", 0.5)], vec![number("y", f64::from(i32::MAX) + 1.)], vec![number("x", f64::NAN)], vec![kind(3)], vec![number("unknown", 1.)]]
        {
            assert!(map(&StoredEffect::test("gimp:offset", args), &context()).unwrap().is_err());
        }
        let transparent = map(&StoredEffect::test("gimp:offset", vec![kind(1)]), &context()).unwrap().unwrap();
        assert_eq!(transparent.filters[0].params["undefinedAreas"], "transparent");
    }
    #[test]
    fn colored_offset_masks_only_vacated_bands_and_preflights_geometry() {
        let e = StoredEffect::test("gimp:offset", vec![kind(0), number("x", 2.), number("y", -1.)]);
        let mapped = map(&e, &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = mapped.structure else { panic!("native editable composite") };
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].vector_mask.as_ref().unwrap().path.subpaths.len(), 2);
        assert!(matches!(branches[0].fill, Some(Fill::Solid(_))));
        assert_eq!(branches[1].filters[0].params, json!({"horizontal":2,"vertical":-1,"undefinedAreas":"transparent"}));
        let mut tiny = context();
        tiny.remaining_generation_bytes = 1;
        assert_eq!(map(&e, &tiny).unwrap().unwrap_err().reason, ConversionReason::ResourceUnavailable);
        let huge = map(&StoredEffect::test("gimp:offset", vec![kind(0), number("x", f64::from(i32::MAX))]), &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = huge.structure else { panic!("fill frame") };
        assert_eq!(branches[0].vector_mask.as_ref().unwrap().path.subpaths.len(), 1);
    }
    #[test]
    fn native_offset_and_vacated_color_preserve_partial_alpha_at_all_depths() {
        use photocraft_algo::{FilterParams, UndefinedAreas};
        use photocraft_color::{PixelFormat, SampleType};
        use photocraft_doc::{Document, Layer, LayerContent};
        use photocraft_geom::Size;
        use photocraft_raster::{Interrupt, Surface};
        let frame = Rect::new(0, 0, 8, 6);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let format = PixelFormat::new(ColorMode::Rgb, depth, true);
            let mut source = Surface::new(format);
            source.write_region(frame, &[0.25, 0.5, 0.75, 0.5].repeat(48));
            let mapping = map(&StoredEffect::test("gimp:offset", vec![kind(0), number("x", 2.)]), &context()).unwrap().unwrap();
            let Some(NativeStructure::Composite { branches }) = mapping.structure else { panic!("native branches") };
            let p = &branches[1].filters[0].params;
            let actual = FilterParams::Offset {
                horizontal: p["horizontal"].as_i64().unwrap() as i32,
                vertical: p["vertical"].as_i64().unwrap() as i32,
                undefined: UndefinedAreas::Transparent,
            };
            let shifted = photocraft_algo::apply_in_with(&source, &actual, frame, frame, None, frame, &Interrupt::NONE).unwrap();
            // Independent declared native counterpart: a two-pixel shift, no wrap.
            assert_eq!(shifted.pixel(4, 3), source.pixel(2, 3));
            assert_eq!(shifted.pixel(0, 3), vec![0.; 4]);
            let mut doc = Document::new("Native offset counterpart", Size::new(8, 6), ColorMode::Rgb, depth);
            let mut fill = Layer::new("Vacated color", LayerContent::Fill(branches[0].fill.clone().unwrap()));
            fill.vector_mask = branches[0].vector_mask.clone();
            doc.layers.push(fill);
            doc.layers.push(Layer::new("Shifted original", LayerContent::Raster(shifted)));
            let rendered = photocraft_compose::render(&doc, frame);
            assert_eq!(rendered.get(0, 3), [1.; 4]);
            let expected = source.pixel(2, 3);
            for (actual, expected) in rendered.get(4, 3).iter().zip(expected) {
                assert!((actual - expected).abs() < 1e-5, "{depth:?} {actual} {expected}");
            }
        }
    }
}
