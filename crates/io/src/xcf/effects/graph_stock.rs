//! Bounded stock graph-node adapters. Property facts come from GEGL 0.4.72
//! registrations/declarations; all output is ordinary native editable data.
use super::graph_schema::Kind;
use super::residual_spatial::{bounds, color, composite, generation_bytes, geometry_bytes, invalid, rect, replace, shape};
use super::{MappingContext, MappingFailure, MappingResult, NativeBranch, NativeMapping, StoredEffect, native_filter};
use photocraft_doc::{Fill, GradientStyle, Layer, LayerContent};
use photocraft_geom::Affine;

const SAMPLERS: &[(&str, i32)] =
    &[("nearest", 0), ("Nearest", 0), ("linear", 1), ("Linear", 1), ("cubic", 2), ("Cubic", 2), ("nohalo", 3), ("NoHalo", 3), ("lohalo", 4), ("LoHalo", 4)];
const NAMES: &[&str] = &[
    "gegl:nop",
    "gegl:clone",
    "gegl:cache",
    "gegl:color",
    "gegl:rectangle",
    "gegl:linear-gradient",
    "gegl:radial-gradient",
    "gegl:translate",
    "gegl:rotate",
    "gegl:scale-ratio",
    "gegl:opacity",
    "gegl:scale-size",
    "gegl:scale-size-keepaspect",
    "gegl:rotate-on-center",
    "gegl:shear",
    "gegl:reflect",
    "gegl:transform",
    "gegl:reset-origin",
    "gegl:border-align",
];
// Exact previously audited stock registrations, not prefix-based admission.
const UNMAPPED: &[&str] = &[
    "gegl:mix",
    "gegl:weighted-blend",
    "gegl:piecewise-blend",
    "gegl:add",
    "gegl:subtract",
    "gegl:multiply",
    "gegl:divide",
    "gegl:gamma",
    "gegl:gray",
    "gegl:contrast-curve",
    "gegl:posterize",
    "gegl:threshold",
    "gegl:wavelet-blur",
    "gegl:map-absolute",
    "gegl:map-relative",
    "gimp:offset",
    "gimp:dissolve",
    "gimp:equalize",
    "gimp:cage-coef-calc",
    "gimp:cage-transform",
    "gimp:histogram-sink",
];
const RESOURCES: &[&str] = &[
    "gegl:buffer-source",
    "gegl:open-buffer",
    "gegl:gegl-buffer-load",
    "gegl:load",
    "gegl:layer",
    "gegl:convert-format",
    "gegl:cast-format",
    "gegl:convert-space",
    "gegl:cast-space",
    "gegl:lcms-from-profile",
];

pub(super) fn operation(token: &str) -> Option<&'static str> {
    NAMES.iter().chain(UNMAPPED).chain(RESOURCES).copied().find(|name| *name == token || name.strip_prefix("gegl:") == Some(token))
}
pub(super) fn generator(op: &str) -> bool {
    matches!(op, "gegl:color" | "gegl:rectangle" | "gegl:linear-gradient" | "gegl:radial-gradient")
}
fn placement(op: &str) -> bool {
    matches!(
        op,
        "gegl:translate"
            | "gegl:rotate"
            | "gegl:scale-ratio"
            | "gegl:scale-size"
            | "gegl:scale-size-keepaspect"
            | "gegl:rotate-on-center"
            | "gegl:shear"
            | "gegl:reflect"
            | "gegl:transform"
            | "gegl:reset-origin"
            | "gegl:border-align"
    )
}
pub(super) fn field(op: &str, name: &str) -> Option<Kind> {
    let full = Kind::Number { min: -f64::MAX, max: f64::MAX };
    Some(match (op, name) {
        ("gegl:clone", "ref") => Kind::Text,
        ("gegl:transform", "transform") => Kind::Text,
        ("gegl:border-align", "snap-integer") => Kind::Boolean,
        ("gegl:border-align", "horizontal-margin" | "vertical-margin") => full,
        ("gegl:border-align", "x" | "y") => Kind::Number { min: -2., max: 3. },
        ("gegl:opacity", "value") => Kind::Number { min: -10., max: 10. },
        ("gegl:cache", "cache") => Kind::Unavailable("an external GEGL buffer has no portable graph literal"),
        ("gegl:color", "format") => Kind::Unavailable("a Babl format object has no portable graph literal"),
        ("gegl:color", "value") | ("gegl:rectangle", "color") | ("gegl:linear-gradient" | "gegl:radial-gradient", "start-color" | "end-color") => Kind::Color,
        ("gegl:rectangle", "width" | "height") => Kind::Number { min: 0., max: f64::MAX },
        ("gegl:rectangle", "x" | "y")
        | ("gegl:linear-gradient" | "gegl:radial-gradient", "start-x" | "start-y" | "end-x" | "end-y")
        | ("gegl:translate", "x" | "y")
        | ("gegl:shear" | "gegl:reflect", "x" | "y")
        | ("gegl:rotate" | "gegl:rotate-on-center", "degrees") => full,
        ("gegl:scale-ratio" | "gegl:scale-size" | "gegl:scale-size-keepaspect", "x" | "y") => Kind::Number { min: -9000., max: 9000. },
        (_, "origin-x" | "origin-y") if placement(op) => full,
        (_, "near-z") if placement(op) => Kind::Number { min: 0., max: 1. },
        (_, "sampler") if placement(op) => Kind::Choice(SAMPLERS),
        _ => return None,
    })
}
fn number(e: &StoredEffect<'_>, key: &str, default: f64) -> Result<f64, MappingFailure> {
    let v = e.number(key).map_err(invalid)?.unwrap_or(default);
    let Some(Kind::Number { min, max }) = field(e.operation, key) else {
        return Err(invalid("undeclared graph numeric property"));
    };
    if !v.is_finite() || v < min || v > max {
        return Err(invalid(format!("{} {key} is outside its declared source domain", e.operation)));
    }
    Ok(v)
}
fn finite_f32(v: f64) -> Result<f32, MappingFailure> {
    let f = v as f32;
    if !f.is_finite() { Err(MappingFailure::resource("stock graph coordinates exceed native finite geometry")) } else { Ok(f) }
}
/// SVG 1.1 section7.6 transform-list data, translated into ordinary Affine.
/// https://www.w3.org/TR/SVG11/coords.html#TransformAttribute
fn svg_matrix(raw: &str) -> Result<Affine, MappingFailure> {
    if raw.len() > 4096 {
        return Err(MappingFailure::resource("SVG transform literal exceeds bounded parser allowance"));
    }
    let mut rest = raw.trim();
    let mut total = Affine::IDENTITY;
    let mut operations = 0usize;
    while !rest.is_empty() {
        operations += 1;
        if operations > 64 {
            return Err(MappingFailure::resource("SVG transform list exceeds64 native affine steps"));
        }
        let (name, tail) = rest.split_once('(').ok_or("unsupported SVG transform syntax")?;
        let (inner, tail) = tail.split_once(')').ok_or("unterminated SVG transform")?;
        if inner.split(',').any(|part| part.trim().is_empty()) {
            return Err(invalid("empty SVG transform coefficient"));
        }
        let mut values = [0.; 6];
        let mut count = 0usize;
        for token in inner.split(|ch: char| ch == ',' || ch.is_ascii_whitespace()).filter(|s| !s.is_empty()) {
            let slot = values.get_mut(count).ok_or("SVG transform has excessive coefficients")?;
            *slot = token.parse::<f64>().map_err(|_| invalid("invalid SVG transform coefficient"))?;
            if !slot.is_finite() {
                return Err(invalid("SVG transform coefficient must be finite"));
            }
            count += 1;
        }
        let [a, b, c, d, e, f] = values;
        let matrix = match (name.trim(), count) {
            ("matrix", 6) => Affine { m: [a, b, c, d, e, f] },
            ("translate", 1) => Affine::translate(a, 0.),
            ("translate", 2) => Affine::translate(a, b),
            ("scale", 1) => Affine::scale(a),
            ("scale", 2) => Affine { m: [a, 0., 0., b, 0., 0.] },
            ("rotate", 1) => Affine::rotate(a.rem_euclid(360.).to_radians()),
            ("rotate", 3) => Affine::translate(b, c).mul(&Affine::rotate(a.rem_euclid(360.).to_radians())).mul(&Affine::translate(-b, -c)),
            ("skewX", 1) => Affine { m: [1., 0., a.to_radians().tan(), 1., 0., 0.] },
            ("skewY", 1) => Affine { m: [1., a.to_radians().tan(), 0., 1., 0., 0.] },
            _ => return Err("unsupported SVG transform name or coefficient count".into()),
        };
        total = total.mul(&matrix);
        if total.m.iter().any(|v| !v.is_finite()) {
            return Err(MappingFailure::resource("SVG affine composition exceeds native finite geometry"));
        }
        rest = tail.trim_start();
        if let Some(tail) = rest.strip_prefix(',') {
            rest = tail.trim_start();
            if rest.is_empty() {
                return Err(invalid("trailing SVG transform separator"));
            }
        }
    }
    Ok(total)
}
fn placement_matrix(e: &StoredEffect<'_>, c: &MappingContext<'_>) -> Result<Affine, MappingFailure> {
    let b = c.owner_bounds;
    if b.is_empty() {
        return Err("stock placement requires actual nonempty native input bounds".into());
    }
    let mut ox = number(e, "origin-x", 0.)?;
    let mut oy = number(e, "origin-y", 0.)?;
    let matrix = match e.operation {
        "gegl:translate" => Affine::translate(number(e, "x", 0.)?, number(e, "y", 0.)?),
        "gegl:rotate" | "gegl:rotate-on-center" => {
            if e.operation == "gegl:rotate-on-center" {
                ox = f64::from(b.x0) + f64::from(b.width()) / 2.;
                oy = f64::from(b.y0) + f64::from(b.height()) / 2.;
            }
            Affine::rotate(-number(e, "degrees", 0.)?.rem_euclid(360.).to_radians())
        }
        "gegl:scale-ratio" => Affine { m: [number(e, "x", 0.)?, 0., 0., number(e, "y", 0.)?, 0., 0.] },
        "gegl:scale-size" => Affine { m: [number(e, "x", 100.)? / f64::from(b.width()), 0., 0., number(e, "y", 100.)? / f64::from(b.height()), 0., 0.] },
        "gegl:scale-size-keepaspect" => {
            let x = number(e, "x", -1.)?;
            let y = number(e, "y", -1.)?;
            // Native fit-to-box policy: negative dimensions are unspecified, both
            // unspecified preserve input. Positive limits select the smaller ratio.
            let scale = match (x >= 0., y >= 0.) {
                (true, true) => (x / f64::from(b.width())).min(y / f64::from(b.height())),
                (true, false) => x / f64::from(b.width()),
                (false, true) => y / f64::from(b.height()),
                (false, false) => 1.,
            };
            Affine::scale(scale)
        }
        "gegl:shear" => Affine { m: [1., number(e, "y", 0.)?, number(e, "x", 0.)?, 1., 0., 0.] },
        "gegl:reflect" => {
            let x = number(e, "x", 0.)?;
            let y = number(e, "y", 0.)?;
            let length = x.hypot(y);
            if !length.is_finite() || length == 0. {
                return Err("reflection needs a nonzero finite source direction vector; the zero-vector source default remains retained".into());
            }
            let x = x / length;
            let y = y / length;
            Affine { m: [2. * x * x - 1., 2. * x * y, 2. * x * y, 2. * y * y - 1., 0., 0.] }
        }
        "gegl:reset-origin" => Affine::translate(-f64::from(b.x0), -f64::from(b.y0)),
        "gegl:border-align" => {
            let target = bounds(c)?;
            let x = number(e, "x", 0.5)?;
            let y = number(e, "y", 0.5)?;
            let mx = number(e, "horizontal-margin", 0.)?;
            let my = number(e, "vertical-margin", 0.)?;
            let mut dx = f64::from(target.x0) - f64::from(b.x0) + (f64::from(target.width()) - f64::from(b.width())) * x + mx * (1. - 2. * x);
            let mut dy = f64::from(target.y0) - f64::from(b.y0) + (f64::from(target.height()) - f64::from(b.height())) * y + my * (1. - 2. * y);
            if e.boolean("snap-integer").map_err(invalid)?.unwrap_or(true) {
                dx = dx.round();
                dy = dy.round();
            }
            Affine::translate(dx, dy)
        }
        "gegl:transform" => {
            let raw = match e.argument("transform").map_err(invalid)? {
                None => "",
                Some(super::ArgumentValue::String(v)) => v,
                _ => return Err(invalid("SVG transform must be stored text")),
            };
            svg_matrix(raw)?
        }
        _ => return Err("unowned stock placement".into()),
    };
    Ok(Affine::translate(ox, oy).mul(&matrix).mul(&Affine::translate(-ox, -oy)))
}
fn map_known(e: &StoredEffect<'_>, c: &MappingContext<'_>) -> Result<NativeMapping, MappingFailure> {
    if RESOURCES.contains(&e.operation) {
        return Err(MappingFailure::resource(format!(
            "{} requires an explicitly serialized portable resource or native color-format interpretation; graph paths never trigger file I/O",
            e.operation
        )));
    }
    if UNMAPPED.contains(&e.operation) {
        return Err(format!("{} is a known stock graph node whose native adapter is not implemented", e.operation).into());
    }
    generation_bytes(c, 4096)?;
    if c.remaining_metadata_bytes < 4096 {
        return Err(MappingFailure::resource("stock graph node exceeds remaining native metadata allowance"));
    }
    for arg in &e.arguments {
        e.argument(arg.name).map_err(invalid)?;
        match field(e.operation, arg.name) {
            None => return Err(format!("{}: stock graph property {} has no native adapter", e.operation, arg.name).into()),
            Some(Kind::Unavailable(reason)) => return Err(MappingFailure::resource(reason)),
            _ => {}
        }
    }
    match e.operation {
        "gegl:opacity" => {
            generation_bytes(c, 16_384)?;
            if c.remaining_metadata_bytes < 16_384 {
                return Err(MappingFailure::resource("native alpha table exceeds remaining metadata allowance"));
            }
            let value = number(e, "value", 1.)?;
            let values: Vec<f64> = (0..256).map(|i| (f64::from(i) / 255. * value).clamp(0., 1.)).collect();
            Ok(NativeMapping { filters: vec![native_filter("filter.other.alphaCurve",serde_json::json!({"values":values}))], notes: vec!["Unary stock opacity becomes native Alpha Curve with a 256-sample transfer; native clamped coverage and interpolation apply. This adapter does not invent an auxiliary input.".into()], ..Default::default() })
        }
        "gegl:nop" | "gegl:cache" | "gegl:clone" => {
            if e.argument("ref").map_err(invalid)?.is_some() {
                return Err(MappingFailure::resource("clone ref requires a resolved graph input reference"));
            }
            Ok(NativeMapping {
                notes: vec!["Stock unary identity uses its bound native input; source buffer-cache policy is superseded by native caching.".into()],
                ..Default::default()
            })
        }
        "gegl:color" => Ok(replace(
            Layer::new("Imported graph color", LayerContent::Fill(Fill::Solid(color(e, "value", [0., 0., 0., 1.], c)?))),
            "Stock color becomes an editable native solid fill, clipped to the native document canvas.",
        )),
        "gegl:rectangle" => {
            let (x, y, w, h) = (number(e, "x", 42.)?, number(e, "y", 42.)?, number(e, "width", 23.)?, number(e, "height", 42.)?);
            for v in [x, y, x + w, y + h] {
                finite_f32(v)?;
            }
            geometry_bytes(c, 1, 4)?;
            let paths = if w == 0. || h == 0. { Vec::new() } else { vec![rect(x, y, w, h)] };
            Ok(replace(
                shape("Imported graph rectangle", paths, color(e, "color", [0., 0.50196, 0., 1.], c)?),
                "Stock rectangle becomes an editable native vector rectangle; native edge antialiasing applies.",
            ))
        }
        "gegl:linear-gradient" | "gegl:radial-gradient" => {
            let b = bounds(c)?;
            let end = if e.operation == "gegl:radial-gradient" { 50. } else { 150. };
            let from = [finite_f32(number(e, "start-x", 25.)?)?, finite_f32(number(e, "start-y", 25.)?)?];
            let to = [finite_f32(number(e, "end-x", end)?)?, finite_f32(number(e, "end-y", end)?)?];
            let distance = (to[0] - from[0]).hypot(to[1] - from[1]);
            if !distance.is_finite() {
                return Err(MappingFailure::resource("stock gradient handle distance exceeds finite native geometry"));
            }
            let style = if e.operation == "gegl:radial-gradient" { GradientStyle::Radial } else { GradientStyle::Linear };
            let (angle, scale, offset) = photocraft_compose::gradient_fill::from_handles(style, from, to, b, 0.);
            if !angle.is_finite() || !scale.is_finite() || !offset.0.is_finite() || !offset.1.is_finite() {
                return Err(MappingFailure::resource("stock gradient geometry exceeds native finite parameters"));
            }
            let mut fill = Fill::gradient(
                vec![(0., color(e, "start-color", [0., 0., 0., 1.], c)?), (1., color(e, "end-color", [1., 1., 1., 1.], c)?)],
                angle,
                scale,
                style,
                false,
            );
            if let Fill::Gradient { offset: o, align, .. } = &mut fill {
                *o = offset;
                *align = false;
            }
            Ok(replace(
                Layer::new("Imported graph gradient", LayerContent::Fill(fill)),
                format!(
                    "Stock gradient endpoints become native gradient handles. Source reach {distance} uses the existing native half-pixel construction minimum and native scale floor; a zero-length source uses native zero-degree fallback, preserving its linear midpoint or radial center. Native interpolation, transfer and alpha rules apply within the native canvas."
                ),
            ))
        }
        op if placement(op) => {
            let near_z = number(e, "near-z", 0.)?;
            if !(0.0..=1.0).contains(&near_z) {
                return Err(invalid("stock affine near-z must remain within its declared 0..1 domain"));
            }
            let sampler = e.enum_value("sampler").map_err(invalid)?.unwrap_or(1);
            if !(0..=4).contains(&sampler) {
                return Err(invalid("unknown stock transform sampler"));
            }
            let transform = placement_matrix(e, c)?;
            let determinant = transform.m[0] * transform.m[3] - transform.m[1] * transform.m[2];
            if determinant == 0. {
                return Ok(replace(
                    Layer::new("Collapsed graph placement", LayerContent::Fill(Fill::Solid(super::resources::native_rgba([0., 0., 0., 0.], c)?))),
                    "Singular affine placement becomes native transparent output with original input retained.",
                ));
            }
            if transform.m.iter().any(|v| !v.is_finite()) {
                return Err(MappingFailure::resource("stock affine geometry overflows native placement"));
            }
            Ok(composite(
                vec![NativeBranch { transform, ..Default::default() }],
                format!(
                    "Stock placement becomes editable native Smart affine data. All admitted transforms are 2D affine with constant homogeneous z=1; declared near-z={near_z} in 0..1 is inactive under this native 2D policy, without introducing a 3D clipping evaluator. Native pixel-size fit, shear-factor units, frame-center rotation and canvas-target alignment use the fixed native input frame rather than inferred dynamic source content bounds; aspect fit treats negative dimensions as unspecified. Native resampling/clipping supersede source sampler and target discovery. SVG transform lists become bounded ordinary native Affine composition."
                ),
            ))
        }
        _ => Err("unowned stock graph node".into()),
    }
}
pub(super) fn map(e: &StoredEffect<'_>, c: &MappingContext<'_>) -> MappingResult {
    operation(e.operation)?;
    Some(map_known(e, c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xcf::effects::{Argument, ArgumentValue, NativeStructure};
    use photocraft_color::ColorMode;
    use photocraft_geom::Rect;
    fn context() -> MappingContext<'static> {
        MappingContext {
            mode: ColorMode::Rgb,
            resolution_dpi: 72.,
            linear: false,
            owner_bounds: Rect::new(0, 0, 64, 32),
            native_filter_bounds: Rect::new(0, 0, 64, 32),
            target_profile: None,
            remaining_generation_bytes: usize::MAX,
            remaining_metadata_bytes: usize::MAX,
            interrupt: photocraft_raster::Interrupt::NONE,
        }
    }
    fn number_arg(name: &'static str, value: f64) -> Argument<'static> {
        Argument { name, value: ArgumentValue::Literal(value) }
    }
    #[test]
    fn stock_names_and_explicit_resource_policy_do_not_admit_future_nodes() {
        assert_eq!(operation("translate"), Some("gegl:translate"));
        assert_eq!(operation("gegl:future-node"), None);
        // Average exists only in optional workshop/generated, absent from the
        // official installed modules; source-tree presence is not stock proof.
        assert_eq!(operation("gegl:average"), None);
        assert_eq!(operation("gegl:piecewise-blend"), Some("gegl:piecewise-blend"));
        assert!(generator("gegl:rectangle"));
        assert!(!generator("gegl:clone"));
        assert!(map(&StoredEffect::test("gegl:load", vec![]), &context()).unwrap().unwrap_err().message.contains("never trigger file I/O"));
        assert!(map(&StoredEffect::test("gegl:gamma", vec![]), &context()).unwrap().unwrap_err().message.contains("not implemented"));
        assert!(map(&StoredEffect::test("gegl:clone", vec![Argument { name: "ref", value: ArgumentValue::String("missing") }]), &context()).unwrap().is_err());
    }
    #[test]
    fn stock_gradient_has_independently_declared_native_handles_and_stops() {
        let e = StoredEffect::test(
            "gegl:linear-gradient",
            vec![number_arg("start-x", 4.), number_arg("start-y", 8.), number_arg("end-x", 36.), number_arg("end-y", 8.)],
        );
        let mapped = map(&e, &context()).unwrap().unwrap();
        let Some(NativeStructure::Replace { layer, .. }) = mapped.structure else { panic!("native fill required") };
        let LayerContent::Fill(fill) = &layer.content else { panic!("native fill required") };
        let (from, to) = photocraft_compose::gradient_fill::fill_handles(fill, Rect::new(0, 0, 64, 32)).unwrap();
        assert!((from[0] - 4.).abs() < 0.001 && (from[1] - 8.).abs() < 0.001);
        assert!((to[0] - 36.).abs() < 0.001 && (to[1] - 8.).abs() < 0.001);
        let Fill::Gradient { stops, align, .. } = fill else { panic!("gradient required") };
        assert!(!align);
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].1.to_rgb(), [0., 0., 0.]);
        assert_eq!(stops[1].1.to_rgb(), [1., 1., 1.]);
    }
    #[test]
    fn stock_rotation_is_counterclockwise_about_declared_origin_and_scale_zero_is_empty() {
        let e = StoredEffect::test("gegl:rotate", vec![number_arg("degrees", 90.), number_arg("origin-x", 10.), number_arg("origin-y", 20.)]);
        let mapped = map(&e, &context()).unwrap().unwrap();
        let Some(NativeStructure::Composite { branches }) = mapped.structure else { panic!("placement required") };
        let m = branches[0].transform.m;
        // (11,20) rotates around (10,20) to (10,19) in screen coordinates.
        assert!((m[0] * 11. + m[2] * 20. + m[4] - 10.).abs() < 1e-10);
        assert!((m[1] * 11. + m[3] * 20. + m[5] - 19.).abs() < 1e-10);
        assert!(matches!(map(&StoredEffect::test("gegl:scale-ratio", vec![]), &context()).unwrap().unwrap().structure, Some(NativeStructure::Replace { .. })));
        for near_z in [0., 0.1, 1.] {
            let mapped = map(&StoredEffect::test("gegl:translate", vec![number_arg("x", 2.), number_arg("near-z", near_z)]), &context()).unwrap().unwrap();
            let Some(NativeStructure::Composite { branches }) = mapped.structure else { panic!("native 2D affine") };
            assert_eq!(branches[0].transform, Affine::translate(2., 0.));
            assert!(mapped.notes.iter().any(|note| note.contains("constant homogeneous z=1")));
        }
        for invalid in [-0.01, 1.01, f64::NAN] {
            assert!(map(&StoredEffect::test("gegl:translate", vec![number_arg("near-z", invalid)]), &context()).unwrap().is_err());
        }
    }
    #[test]
    fn short_and_zero_gradients_use_independent_native_minimum_handles_and_reopen_each_depth() {
        use photocraft_color::{Color, SampleType};
        use photocraft_doc::Document;
        use photocraft_geom::Size;
        for (op, style, unit) in [("gegl:linear-gradient", GradientStyle::Linear, 64.), ("gegl:radial-gradient", GradientStyle::Radial, 32.)] {
            for end in [12.25, 12.5] {
                let source =
                    StoredEffect::test(op, vec![number_arg("start-x", 12.25), number_arg("start-y", 8.), number_arg("end-x", end), number_arg("end-y", 8.)]);
                let mapping = map(&source, &context()).unwrap().unwrap();
                assert!(mapping.notes.iter().any(|note| note.contains("half-pixel")));
                let Some(NativeStructure::Replace { layer, .. }) = mapping.structure else { panic!("native gradient fill") };
                let cx = if style == GradientStyle::Linear { (12.25 + end as f32) / 2. } else { 12.25 };
                let mut expected = Fill::gradient(vec![(0., Color::rgb(0., 0., 0.)), (1., Color::rgb(1., 1., 1.))], 0., 0.5 / unit, style, false);
                let Fill::Gradient { offset, align, .. } = &mut expected else { panic!("native gradient") };
                *offset = ((cx - 32.) / 64., -8. / 32.);
                *align = false;
                let LayerContent::Fill(actual) = &layer.content else { panic!("native fill") };
                assert_eq!(actual, &expected);
                for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                    let mut doc = Document::new("Imported short gradient", Size::new(64, 32), ColorMode::Rgb, depth);
                    doc.layers.push((*layer).clone());
                    let mut counterpart = Document::new("Independent native minimum", doc.size, doc.mode, depth);
                    counterpart.layers.push(Layer::new("Native minimum", LayerContent::Fill(expected.clone())));
                    let pixels = photocraft_compose::render(&doc, doc.bounds()).px;
                    assert!(pixels.iter().any(|pixel| pixel[3] > 0.));
                    assert_eq!(pixels, photocraft_compose::render(&counterpart, counterpart.bounds()).px);
                    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
                    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                    assert_eq!(pixels, photocraft_compose::render(&reopened, reopened.bounds()).px);
                }
            }
        }
        let source = StoredEffect::test("gegl:linear-gradient", vec![number_arg("start-x", 1.), number_arg("end-x", 1.)]);
        let mut c = context();
        c.remaining_generation_bytes = 10;
        assert!(map(&source, &c).unwrap().is_err());
        c = context();
        c.remaining_metadata_bytes = 10;
        assert!(map(&source, &c).unwrap().is_err());
        let cancel = || true;
        c = context();
        c.interrupt = photocraft_raster::Interrupt::cancel_only(&cancel);
        assert!(map(&source, &c).unwrap().is_err());
    }
    #[test]
    fn unary_opacity_uses_native_clamped_alpha_table_and_preflights_budget() {
        let e = StoredEffect::test("gegl:opacity", vec![number_arg("value", 0.5)]);
        let mapped = map(&e, &context()).unwrap().unwrap();
        assert_eq!(mapped.filters[0].command, "filter.other.alphaCurve");
        let values = mapped.filters[0].params["values"].as_array().unwrap();
        assert_eq!(values.len(), 256);
        assert_eq!(values[0], 0.);
        assert_eq!(values[255], 0.5);
        let mut c = context();
        c.remaining_metadata_bytes = 0;
        assert!(map(&e, &c).unwrap().is_err());
        assert!(map(&StoredEffect::test("gegl:rectangle", vec![number_arg("width", -1.)]), &context()).unwrap().is_err());
    }
    #[test]
    fn additional_placements_have_independent_native_matrices_and_declared_domains() {
        let mut c = context();
        c.owner_bounds = Rect::new(10, 20, 30, 30);
        c.native_filter_bounds = Rect::new(0, 0, 100, 80);
        let get = |op, args| placement_matrix(&StoredEffect::test(op, args), &c).unwrap().m;
        assert_eq!(get("gegl:scale-size", vec![number_arg("x", 40.), number_arg("y", 30.)]), [2., 0., 0., 3., 0., 0.]);
        assert_eq!(get("gegl:scale-size-keepaspect", vec![number_arg("x", 40.), number_arg("y", 15.)]), [1.5, 0., 0., 1.5, 0., 0.]);
        assert_eq!(get("gegl:scale-size-keepaspect", vec![]), Affine::IDENTITY.m);
        assert_eq!(get("gegl:shear", vec![number_arg("x", 2.), number_arg("y", 0.25)]), [1., 0.25, 2., 1., 0., 0.]);
        assert_eq!(get("gegl:reflect", vec![number_arg("x", 1.)]), [1., 0., 0., -1., 0., 0.]);
        assert_eq!(get("gegl:reset-origin", vec![]), [1., 0., 0., 1., -10., -20.]);
        assert_eq!(get("gegl:border-align", vec![]), [1., 0., 0., 1., 30., 15.]);
        let centered = get("gegl:rotate-on-center", vec![number_arg("degrees", 180.), number_arg("origin-x", 999.)]);
        assert!((centered[4] - 40.).abs() < 1e-9 && (centered[5] - 50.).abs() < 1e-9);
        assert_eq!(svg_matrix("matrix(1 2 3 4 5 6)").unwrap().m, [1., 2., 3., 4., 5., 6.]);
        assert_eq!(svg_matrix("").unwrap().m, Affine::IDENTITY.m);
        assert!(svg_matrix("matrix(1,2,3,4,5,6,7,8,9)").is_err());
        assert_eq!(svg_matrix("translate(1,2) scale(2,3)").unwrap().m, [2., 0., 0., 3., 1., 2.]);
        assert!(map(&StoredEffect::test("gegl:reflect", vec![]), &c).unwrap().is_err());
        assert!(map(&StoredEffect::test("gegl:scale-size", vec![number_arg("x", 9001.)]), &c).unwrap().is_err());
    }
    #[test]
    fn rectangle_is_native_editable_geometry_with_source_independent_multidepth_reopen() {
        use photocraft_color::SampleType;
        use photocraft_doc::Document;
        use photocraft_geom::Size;
        fn prepare_shapes(doc: &mut Document) {
            let format = doc.pixel_format();
            let bounds = doc.bounds();
            for layer in &mut doc.layers {
                if let LayerContent::Shape(shape) = &mut layer.content {
                    shape.cache = Some(photocraft_vector::render_shape(shape, format, bounds));
                    assert!(shape.cache.as_ref().is_some_and(|surface| surface.tile_count() > 0));
                }
            }
        }
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let source =
                StoredEffect::test("gegl:rectangle", vec![number_arg("x", 2.), number_arg("y", 3.), number_arg("width", 5.), number_arg("height", 4.)]);
            let mapped = map(&source, &context()).unwrap().unwrap();
            let Some(NativeStructure::Replace { layer, .. }) = mapped.structure else { panic!("native shape required") };
            let mut actual = Document::new("Imported rectangle", Size::new(16, 12), ColorMode::Rgb, depth);
            actual.layers.push(*layer);
            let mut expected = Document::new("Independent native rectangle", Size::new(16, 12), ColorMode::Rgb, depth);
            let paint = super::super::resources::native_rgba([0., 0.50196, 0., 1.], &context()).unwrap();
            expected.layers.push(shape("Expected", vec![photocraft_doc::Subpath::polygon(&[(2., 3.), (7., 3.), (7., 7.), (2., 7.)])], paint));
            prepare_shapes(&mut actual);
            prepare_shapes(&mut expected);
            let before = photocraft_compose::render(&actual, actual.bounds()).px;
            assert_eq!(before, photocraft_compose::render(&expected, expected.bounds()).px);
            assert_eq!(before[0], [0.; 4]);
            assert!(
                before[4 * 16 + 3].iter().zip([0., 0.50196, 0., 1.]).all(|(actual, expected)| (*actual - expected).abs() <= 1. / 255.),
                "depth={depth:?}, interior={:?}, expected_native_paint={paint:?}, mode={:?}, ICC={:?}",
                before[4 * 16 + 3],
                actual.mode,
                actual.icc_profile.as_ref().map(|p| p.len())
            ); // Diagnose native transfer rather than infer its output encoding.
            let bytes = photocraft_format::save_to_bytes(&actual, &Default::default()).unwrap();
            drop(source);
            let mut reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
            assert!(matches!(&reopened.layers[0].content, LayerContent::Shape(_)));
            prepare_shapes(&mut reopened);
            assert_eq!(before, photocraft_compose::render(&reopened, reopened.bounds()).px);
        }
    }
}
