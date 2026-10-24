//! Coordinate-map import policy: actual auxiliary native pixels become static
//! embedded native Displace data. Public GEGL map-relative/map-absolute property
//! declarations and operation documentation describe inputs; no source kernel is used.
use super::graph_schema::Kind;
use super::residual_spatial::invalid;
use super::{MappingContext, MappingFailure, NativeMapCoordinates, NativeStage, StoredEffect};

const SAMPLERS: &[(&str, i32)] =
    &[("nearest", 0), ("Nearest", 0), ("linear", 1), ("Linear", 1), ("cubic", 2), ("Cubic", 2), ("nohalo", 3), ("NoHalo", 3), ("lohalo", 4), ("LoHalo", 4)];
const EDGES: &[(&str, i32)] =
    &[("none", 0), ("None", 0), ("clamp", 1), ("Clamp", 1), ("loop", 2), ("Loop", 2), ("black", 3), ("Black", 3), ("white", 4), ("White", 4)];
pub(super) fn operation(token: &str) -> Option<&'static str> {
    match token {
        "map-relative" | "gegl:map-relative" => Some("gegl:map-relative"),
        "map-absolute" | "gegl:map-absolute" => Some("gegl:map-absolute"),
        _ => None,
    }
}
pub(super) fn field(op: &str, name: &str) -> Option<Kind> {
    match (op, name) {
        ("gegl:map-relative", "scaling") => Some(Kind::Number { min: 0., max: 5000. }),
        ("gegl:map-relative" | "gegl:map-absolute", "sampler-type") => Some(Kind::Choice(SAMPLERS)),
        ("gegl:map-relative" | "gegl:map-absolute", "abyss-policy") => Some(Kind::Choice(EDGES)),
        _ => None,
    }
}
pub(super) fn lower(
    e: &StoredEffect<'_>,
    c: &MappingContext<'_>,
    main: usize,
    aux: usize,
    next: usize,
) -> Result<(Vec<NativeStage>, usize, Vec<String>), MappingFailure> {
    c.interrupt.check().map_err(|e| e.to_string())?;
    let coordinates = match e.operation {
        "gegl:map-relative" => NativeMapCoordinates::Offsets,
        "gegl:map-absolute" => NativeMapCoordinates::Positions,
        _ => return Err(invalid("unknown coordinate map operation")),
    };
    for argument in &e.arguments {
        e.argument(argument.name).map_err(invalid)?;
        if field(e.operation, argument.name).is_none() {
            return Err(invalid("unknown coordinate map property"));
        }
    }
    if main >= next || aux >= next || next > 64 {
        return Err(invalid("coordinate map requires actual earlier native inputs"));
    }
    let scale = e.number("scaling").map_err(invalid)?.unwrap_or(1.);
    let sampler = e.enum_value("sampler-type").map_err(invalid)?.unwrap_or(2);
    let edge = e.enum_value("abyss-policy").map_err(invalid)?.unwrap_or(0);
    if !scale.is_finite() || !(0.0..=5000.0).contains(&scale) || !(0..=4).contains(&sampler) || !(0..=4).contains(&edge) {
        return Err(invalid("coordinate map controls exceed declared domains"));
    }
    let bounds = c.native_filter_bounds;
    let metadata_limit = usize::try_from(bounds.width())
        .ok()
        .and_then(|w| usize::try_from(bounds.height()).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_add(16_384))
        .filter(|n| !bounds.is_empty() && *n <= c.remaining_metadata_bytes && *n <= super::super::MAX_METADATA_BYTES)
        .ok_or_else(|| MappingFailure::resource("Actual coordinate-map frame exceeds remaining native parameter storage allowance"))?;
    Ok((
        vec![NativeStage::Displace { input: main, map: aux, coordinates, scale: scale as f32, wrap: edge == 2, metadata_limit }],
        next,
        vec![format!(
            "Actual coordinate-map input and auxiliary image become native Displace with static embedded two-plane U8 map data. Source sampler={sampler} uses ordinary native bilinear sampling. Source abyss={edge} becomes native {} edge policy. Native map profile/channel conversion, finite +/-1278.72px editable scale range and integer-control/U8 precision apply; source coordinate sampling is not emulated.",
            if edge == 2 { "wrap" } else { "repeat" }
        )],
    ))
}

#[cfg(test)]
mod tests {
    use super::super::{Argument, ArgumentValue};
    use super::*;
    use photocraft_color::ColorMode;
    use photocraft_geom::Rect;
    use photocraft_raster::Interrupt;
    #[test]
    fn exact_coordinate_controls_and_actual_references() {
        let context = MappingContext {
            mode: ColorMode::Rgb,
            linear: false,
            owner_bounds: Rect::new(0, 0, 8, 6),
            native_filter_bounds: Rect::new(0, 0, 8, 6),
            target_profile: None,
            resolution_dpi: 72.,
            remaining_generation_bytes: 1 << 24,
            remaining_metadata_bytes: 1 << 20,
            interrupt: Interrupt::NONE,
        };
        let e = StoredEffect::test(
            "gegl:map-relative",
            vec![Argument { name: "scaling", value: ArgumentValue::Literal(2.5) }, Argument { name: "abyss-policy", value: ArgumentValue::Enum(2) }],
        );
        let (steps, output, _) = lower(&e, &context, 2, 3, 4).unwrap();
        assert_eq!(output, 4);
        assert!(matches!(
            steps[0],
            NativeStage::Displace { input: 2, map: 3, coordinates: NativeMapCoordinates::Offsets, scale: 2.5, wrap: true, metadata_limit: 16576 }
        ));
        let absolute = StoredEffect::test("gegl:map-absolute", vec![]);
        assert!(matches!(lower(&absolute, &context, 0, 1, 2).unwrap().0[0], NativeStage::Displace { coordinates: NativeMapCoordinates::Positions, .. }));
        assert!(lower(&e, &context, 4, 3, 4).is_err());
        let invalid = StoredEffect::test("gegl:map-relative", vec![Argument { name: "scaling", value: ArgumentValue::Literal(f64::NAN) }]);
        assert!(lower(&invalid, &context, 0, 1, 2).is_err());
        let context = MappingContext { remaining_metadata_bytes: 100, ..context };
        assert!(lower(&e, &context, 0, 1, 2).is_err());
    }
}
