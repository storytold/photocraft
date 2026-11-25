//! Measurement values with a unit, as existing scripts expect from `document.width` and
//! `document.height`.

use std::cell::RefCell;

use boa_engine::object::builtins::JsFunction;
use boa_engine::object::{ConstructorBuilder, FunctionObjectBuilder, ObjectInitializer};
use boa_engine::property::Attribute;
use boa_engine::{Context, Finalize, JsArgs, JsData, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, Trace, js_string};

use crate::value;

/// Default size of one pixel: 1/72 inch.
const DEFAULT_BASE_VALUE: f64 = 1.0 / 72.0;
const DEFAULT_BASE_TYPE: &str = "in";

/// Class-level `UnitValue.baseUnit`. `None` means the default of 1/72 in.
#[derive(Default)]
struct UnitValueClass {
    base: RefCell<Option<(f64, String)>>,
}

/// A measurement together with its unit.
#[derive(Clone, Debug, Trace, Finalize, JsData)]
#[boa_gc(unsafe_empty_trace)]
struct UnitValueData {
    value: f64,
    typ: String,
    /// Per-object base for pixels and percentages; `None` uses the class default.
    base: Option<(f64, String)>,
}

fn typ_err(message: impl Into<String>) -> JsError {
    JsNativeError::typ().with_message(message.into()).into()
}

fn err(message: impl Into<String>) -> JsError {
    JsNativeError::error().with_message(message.into()).into()
}

fn this_data(this: &JsValue) -> JsResult<UnitValueData> {
    this.as_object().and_then(|o| o.downcast_ref::<UnitValueData>().map(|d| d.clone())).ok_or_else(|| typ_err("not a unit value"))
}

fn class_base(ctx: &Context) -> (f64, String) {
    ctx.get_data::<UnitValueClass>().and_then(|c| c.base.borrow().clone()).unwrap_or_else(|| (DEFAULT_BASE_VALUE, DEFAULT_BASE_TYPE.to_owned()))
}

fn effective_base(data: &UnitValueData, ctx: &Context) -> (f64, String) {
    data.base.clone().unwrap_or_else(|| class_base(ctx))
}

/// Canonical abbreviation, or `"?"` when the name is not a known unit.
fn canonicalize(name: &str) -> String {
    let n = name.trim().to_ascii_lowercase();
    let typ = match n.as_str() {
        "in" | "inch" | "inches" => "in",
        "ft" | "foot" | "feet" => "ft",
        "yd" | "yard" | "yards" => "yd",
        "mi" | "mile" | "miles" => "mi",
        "mm" | "millimeter" | "millimeters" | "millimetre" | "millimetres" => "mm",
        "cm" | "centimeter" | "centimeters" | "centimetre" | "centimetres" => "cm",
        "m" | "meter" | "meters" | "metre" | "metres" => "m",
        "km" | "kilometer" | "kilometers" | "kilometre" | "kilometres" => "km",
        "pt" | "point" | "points" => "pt",
        "pc" | "pica" | "picas" => "pc",
        "tpt" | "traditional point" | "traditional points" => "tpt",
        "tpc" | "traditional pica" | "traditional picas" => "tpc",
        "ci" | "cicero" | "ciceros" => "ci",
        "px" | "pixel" | "pixels" => "px",
        "%" | "percent" => "%",
        _ => "?",
    };
    typ.to_owned()
}

fn to_inches(value: f64, typ: &str, base: &(f64, String)) -> Option<f64> {
    match typ {
        "in" => Some(value),
        "ft" => Some(value * 12.0),
        "yd" => Some(value * 36.0),
        "mi" => Some(value * 63_360.0),
        "mm" => Some(value / 25.4),
        "cm" => Some(value / 2.54),
        "m" => Some(value / 0.0254),
        "km" => Some(value / 0.0000254),
        "pt" => Some(value / 72.0),
        "pc" => Some(value / 6.0),
        "tpt" => Some(value / 72.27),
        "tpc" => Some(value * 12.0 / 72.27),
        "ci" => Some(value * 12.7872 / 72.0),
        "px" | "%" => {
            if base.1 == "px" || base.1 == "%" || base.1 == "?" {
                return None;
            }
            let base_in = to_inches(base.0, &base.1, &(DEFAULT_BASE_VALUE, DEFAULT_BASE_TYPE.to_owned()))?;
            if typ == "px" { Some(value * base_in) } else { Some(value / 100.0 * base_in) }
        }
        _ => None,
    }
}

fn from_inches(inches: f64, typ: &str, base: &(f64, String)) -> Option<f64> {
    match typ {
        "in" => Some(inches),
        "ft" => Some(inches / 12.0),
        "yd" => Some(inches / 36.0),
        "mi" => Some(inches / 63_360.0),
        "mm" => Some(inches * 25.4),
        "cm" => Some(inches * 2.54),
        "m" => Some(inches * 0.0254),
        "km" => Some(inches * 0.0000254),
        "pt" => Some(inches * 72.0),
        "pc" => Some(inches * 6.0),
        "tpt" => Some(inches * 72.27),
        "tpc" => Some(inches * 72.27 / 12.0),
        "ci" => Some(inches * 72.0 / 12.7872),
        "px" | "%" => {
            if base.1 == "px" || base.1 == "%" || base.1 == "?" {
                return None;
            }
            let base_in = to_inches(base.0, &base.1, &(DEFAULT_BASE_VALUE, DEFAULT_BASE_TYPE.to_owned()))?;
            if base_in == 0.0 {
                return None;
            }
            if typ == "px" { Some(inches / base_in) } else { Some(inches / base_in * 100.0) }
        }
        _ => None,
    }
}

fn convert_num(value: f64, from: &str, to: &str, base: &(f64, String)) -> Option<f64> {
    if from == to {
        return Some(value);
    }
    let inches = to_inches(value, from, base)?;
    from_inches(inches, to, base)
}

fn format_num(v: f64) -> String {
    if v.is_finite() && v == v.trunc() && v.abs() < 1e15 { format!("{v:.0}") } else { format!("{v}") }
}

fn stringify(data: &UnitValueData) -> String {
    if data.typ == "?" { "UnitValue 0.00000".to_owned() } else { format!("{} {}", format_num(data.value), data.typ) }
}

/// The `UnitValue` prototype as installed. Values made by the host use it, never what the global
/// `UnitValue` is now: a script may replace that, and reading it would run the script's code in
/// the middle of a host call.
#[derive(Trace, Finalize, JsData)]
struct UnitValueProto(JsObject);

fn object(data: UnitValueData, ctx: &mut Context) -> JsResult<JsObject> {
    let proto = ctx.realm().host_defined().get::<UnitValueProto>().map(|p| p.0.clone()).ok_or_else(|| typ_err("UnitValue is missing"))?;
    Ok(ObjectInitializer::with_native_data_and_proto(data, proto, ctx).build())
}

/// A document length of `px` pixels as a unit value in `typ` (`px`, `in`, `cm`, `mm`, `pt`,
/// `pc`), at the document's `dpi`: one pixel is 1/dpi inch, so `.as("px")` gives the pixels back.
pub(crate) fn document_length(px: f64, dpi: f64, typ: &str, ctx: &mut Context) -> JsResult<JsValue> {
    let base = (1.0 / dpi.max(1e-6), "in".to_owned());
    let typ = if typ == "%" { "px" } else { typ };
    // A unit the length can't be given in leaves it in pixels, labelled as pixels.
    let (value, typ) = convert_num(px, "px", typ, &base).map_or((px, "px"), |v| (v, typ));
    Ok(object(UnitValueData { value, typ: typ.into(), base: Some(base) }, ctx)?.into())
}

/// `v` as document pixels at `dpi`: a unit value converts from its own unit, anything else is a
/// number in `default_typ` (the ruler unit, as existing scripts expect).
pub(crate) fn to_document_pixels(v: &JsValue, dpi: f64, default_typ: &str, ctx: &mut Context) -> JsResult<f64> {
    let (value, typ) = length_of(v, default_typ, ctx)?;
    length_in_pixels(value, &typ, dpi)
}

/// `v` as a length: its number and unit (`default_typ` for a plain number). The script code a
/// conversion may run (`valueOf`) runs here, before anything is read for it.
pub(crate) fn length_of(v: &JsValue, default_typ: &str, ctx: &mut Context) -> JsResult<(f64, String)> {
    match v.as_object().and_then(|o| o.downcast_ref::<UnitValueData>().map(|d| d.clone())) {
        // A percentage is of the value's base: resolved to inches here, or refused when that base
        // is itself relative. It is never a count of pixels.
        Some(d) if d.typ == "%" => {
            let base = effective_base(&d, ctx);
            to_inches(d.value, "%", &base).map(|inches| (inches, "in".to_owned())).ok_or_else(|| {
                typ_err(format!("{} % is not a length here: its base unit, {} {}, is not an absolute length", d.value, base.0, base.1))
            })
        }
        Some(d) => Ok((d.value, d.typ)),
        None => Ok((v.to_number(ctx)?, default_typ.to_owned())),
    }
}

/// A length from [`length_of`] in document pixels at `dpi`.
pub(crate) fn length_in_pixels(value: f64, typ: &str, dpi: f64) -> JsResult<f64> {
    let base = (1.0 / dpi.max(1e-6), "in".to_owned());
    let typ = if typ == "%" { "px" } else { typ };
    convert_num(value, typ, "px", &base).filter(|f| f.is_finite()).ok_or_else(|| typ_err(format!("{value} {typ} is not a length")))
}

fn leading_number(s: &str) -> Option<(f64, &str)> {
    let s = s.trim_start();
    let mut end = 0;
    let b = s.as_bytes();
    if b.first().is_some_and(|c| *c == b'+' || *c == b'-') {
        end = 1;
    }
    let start_digits = end;
    while end < b.len() && (b[end].is_ascii_digit() || b[end] == b'.') {
        end += 1;
    }
    if end == start_digits {
        return None;
    }
    if end < b.len() && (b[end] == b'e' || b[end] == b'E') {
        let mut exp = end + 1;
        if exp < b.len() && (b[exp] == b'+' || b[exp] == b'-') {
            exp += 1;
        }
        let exp_start = exp;
        while exp < b.len() && b[exp].is_ascii_digit() {
            exp += 1;
        }
        if exp > exp_start {
            end = exp;
        }
    }
    let value = s[..end].parse().ok()?;
    Some((value, &s[end..]))
}

fn parse_args(args: &[JsValue], ctx: &mut Context) -> JsResult<UnitValueData> {
    if args.is_empty() {
        return Ok(UnitValueData { value: 0.0, typ: "?".into(), base: None });
    }
    if args.len() >= 2 {
        let value = args[0].to_number(ctx)?;
        let typ = canonicalize(&value::text(&args[1].to_string(ctx)?)?);
        if typ == "?" {
            return Ok(UnitValueData { value: 0.0, typ, base: None });
        }
        return Ok(UnitValueData { value, typ, base: None });
    }
    let text = value::text(&args[0].to_string(ctx)?)?;
    if let Some((value, rest)) = leading_number(&text) {
        let typ = canonicalize(rest);
        if typ == "?" && !rest.trim().is_empty() {
            return Ok(UnitValueData { value: 0.0, typ, base: None });
        }
        return Ok(UnitValueData { value, typ, base: None });
    }
    Ok(UnitValueData { value: 0.0, typ: canonicalize(&text), base: None })
}

fn construct(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    Ok(object(parse_args(args, ctx)?, ctx)?.into())
}

fn get_value(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(this_data(this)?.value))
}

fn set_value(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let obj = this.as_object().ok_or_else(|| typ_err("not a unit value"))?;
    let n = args.get_or_undefined(0).to_number(ctx)?;
    obj.downcast_mut::<UnitValueData>().ok_or_else(|| typ_err("not a unit value"))?.value = n;
    Ok(JsValue::undefined())
}

fn get_type(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(js_string!(this_data(this)?.typ.as_str()).into())
}

fn set_type(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let obj = this.as_object().ok_or_else(|| typ_err("not a unit value"))?;
    let typ = canonicalize(&value::text(&args.get_or_undefined(0).to_string(ctx)?)?);
    let mut data = obj.downcast_mut::<UnitValueData>().ok_or_else(|| typ_err("not a unit value"))?;
    if typ == "?" {
        data.value = 0.0;
    }
    data.typ = typ;
    Ok(JsValue::undefined())
}

fn get_base(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let data = this_data(this)?;
    let (value, typ) = effective_base(&data, ctx);
    Ok(object(UnitValueData { value, typ, base: None }, ctx)?.into())
}

fn set_base(this: &JsValue, args: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    let obj = this.as_object().ok_or_else(|| typ_err("not a unit value"))?;
    if obj.downcast_ref::<UnitValueData>().is_none() {
        return Err(typ_err("not a unit value"));
    }
    let v = args.get_or_undefined(0);
    let base = if v.is_null() {
        None
    } else {
        let rhs = this_data(v).map_err(|_| typ_err("baseUnit must be a unit value or null"))?;
        Some((rhs.value, rhs.typ))
    };
    obj.downcast_mut::<UnitValueData>().ok_or_else(|| typ_err("not a unit value"))?.base = base;
    Ok(JsValue::undefined())
}

fn get_class_base(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (value, typ) = class_base(ctx);
    Ok(object(UnitValueData { value, typ, base: None }, ctx)?.into())
}

fn set_class_base(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let v = args.get_or_undefined(0);
    let next = if v.is_null() {
        None
    } else {
        let rhs = this_data(v).map_err(|_| typ_err("baseUnit must be a unit value or null"))?;
        Some((rhs.value, rhs.typ))
    };
    if let Some(class) = ctx.get_data::<UnitValueClass>() {
        *class.base.borrow_mut() = next;
    }
    Ok(JsValue::undefined())
}

fn to_string(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(js_string!(stringify(&this_data(this)?).as_str()).into())
}

fn value_of(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(this_data(this)?.value))
}

fn as_unit(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let data = this_data(this)?;
    let typ = canonicalize(&value::text(&args.get_or_undefined(0).to_string(ctx)?)?);
    let base = effective_base(&data, ctx);
    let n = convert_num(data.value, &data.typ, &typ, &base).ok_or_else(|| err("unknown unit"))?;
    Ok(JsValue::from(n))
}

fn convert(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let typ = canonicalize(&value::text(&args.get_or_undefined(0).to_string(ctx)?)?);
    let obj = this.as_object().ok_or_else(|| typ_err("not a unit value"))?;
    let (value, from, base) = {
        let data = obj.downcast_ref::<UnitValueData>().ok_or_else(|| typ_err("not a unit value"))?;
        (data.value, data.typ.clone(), effective_base(&data, ctx))
    };
    let n = convert_num(value, &from, &typ, &base).ok_or_else(|| err("unknown unit"))?;
    let mut data = obj.downcast_mut::<UnitValueData>().ok_or_else(|| typ_err("not a unit value"))?;
    data.value = n;
    data.typ = typ;
    Ok(JsValue::from(true))
}

/// Install the `UnitValue` constructor and prototype.
pub(crate) fn install(ctx: &mut Context) -> JsResult<()> {
    ctx.insert_data(UnitValueClass::default());
    let get_value = fn_ptr(ctx, get_value, "get value", 0);
    let set_value = fn_ptr(ctx, set_value, "set value", 1);
    let get_type = fn_ptr(ctx, get_type, "get type", 0);
    let set_type = fn_ptr(ctx, set_type, "set type", 1);
    let get_base = fn_ptr(ctx, get_base, "get baseUnit", 0);
    let set_base = fn_ptr(ctx, set_base, "set baseUnit", 1);
    let get_class_base = fn_ptr(ctx, get_class_base, "get baseUnit", 0);
    let set_class_base = fn_ptr(ctx, set_class_base, "set baseUnit", 1);
    let mut builder = ConstructorBuilder::new(ctx, NativeFunction::from_fn_ptr(construct));
    builder.name("UnitValue");
    builder.length(2);
    builder.method(NativeFunction::from_fn_ptr(to_string), js_string!("toString"), 0);
    builder.method(NativeFunction::from_fn_ptr(value_of), js_string!("valueOf"), 0);
    builder.method(NativeFunction::from_fn_ptr(as_unit), js_string!("as"), 1);
    builder.method(NativeFunction::from_fn_ptr(convert), js_string!("convert"), 1);
    builder.accessor(js_string!("value"), Some(get_value), Some(set_value), Attribute::ENUMERABLE | Attribute::CONFIGURABLE);
    builder.accessor(js_string!("type"), Some(get_type), Some(set_type), Attribute::ENUMERABLE | Attribute::CONFIGURABLE);
    builder.accessor(js_string!("baseUnit"), Some(get_base), Some(set_base), Attribute::ENUMERABLE | Attribute::CONFIGURABLE);
    builder.static_accessor(js_string!("baseUnit"), Some(get_class_base), Some(set_class_base), Attribute::ENUMERABLE | Attribute::CONFIGURABLE);
    let ctor = builder.build();
    let realm = ctx.realm().clone();
    realm.host_defined_mut().insert(UnitValueProto(ctor.prototype()));
    ctx.register_global_property(js_string!("UnitValue"), ctor.constructor(), Attribute::WRITABLE | Attribute::CONFIGURABLE)?;
    Ok(())
}

fn fn_ptr(ctx: &mut Context, f: fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>, name: &str, len: usize) -> JsFunction {
    FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(f)).name(JsString::from(name)).length(len).build()
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_BASE_TYPE, DEFAULT_BASE_VALUE, canonicalize, convert_num};

    #[test]
    fn a_pixel_value_converts_to_inches_at_the_default_base() {
        let base = (DEFAULT_BASE_VALUE, DEFAULT_BASE_TYPE.to_owned());
        let inches = convert_num(1920.0, "px", "in", &base).unwrap();
        assert!((inches - 1920.0 / 72.0).abs() < 1e-10, "{inches}");
        assert_eq!(convert_num(1.0, "in", "px", &base), Some(72.0));
        assert_eq!(canonicalize("pixels"), "px");
        assert_eq!(canonicalize("centimeters"), "cm");
        assert_eq!(canonicalize("bogus"), "?");
    }
}
