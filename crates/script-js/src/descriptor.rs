//! `ActionDescriptor` as scripts use it to keep settings between runs, the type-id functions
//! (`charIDToTypeID`, `stringIDToTypeID` and their inverses), and `app.putCustomOptions`,
//! `getCustomOptions` and `eraseCustomOptions`.
//!
//! A descriptor holds keyed values: text, whole numbers, numbers, booleans, unit numbers and
//! enumerated values. It is kept as JSON by the `scripts.customOptions.*` commands. String ids
//! hash to the same number in every run, so options kept under them can be read back later.

use std::cell::RefCell;
use std::collections::HashMap;

use boa_engine::object::{FunctionObjectBuilder, ObjectInitializer};
use boa_engine::property::Attribute;
use boa_engine::{Context, Finalize, JsData, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, Trace, js_string};
use serde_json::{Value, json};

use crate::natives::{Native, command, error, error_with_number, getter};
use crate::value;

/// Most keys in one descriptor.
const MAX_KEYS: usize = 1024;
/// Longest text value, in bytes.
const MAX_TEXT: usize = 32 << 10;

#[derive(Debug, Clone, PartialEq)]
enum Item {
    Text(String),
    Integer(i64),
    Double(f64),
    Boolean(bool),
    Unit(u32, f64),
    Enum(u32, u32),
}

impl Item {
    fn type_name(&self) -> &'static str {
        match self {
            Item::Text(_) => "DescValueType.STRINGTYPE",
            Item::Integer(_) => "DescValueType.INTEGERTYPE",
            Item::Double(_) => "DescValueType.DOUBLETYPE",
            Item::Boolean(_) => "DescValueType.BOOLEANTYPE",
            Item::Unit(..) => "DescValueType.UNITDOUBLE",
            Item::Enum(..) => "DescValueType.ENUMERATEDTYPE",
        }
    }

    fn to_json(&self, key: u32) -> Value {
        match self {
            Item::Text(s) => json!({ "key": key, "type": "string", "value": s }),
            Item::Integer(i) => json!({ "key": key, "type": "integer", "value": i }),
            Item::Double(d) => json!({ "key": key, "type": "double", "value": d }),
            Item::Boolean(b) => json!({ "key": key, "type": "boolean", "value": b }),
            Item::Unit(u, d) => json!({ "key": key, "type": "unitDouble", "unit": u, "value": d }),
            Item::Enum(t, v) => json!({ "key": key, "type": "enumerated", "enumType": t, "value": v }),
        }
    }

    fn from_json(v: &Value) -> Option<(u32, Item)> {
        let key = u32::try_from(v["key"].as_u64()?).ok()?;
        let id = |k: &str| v[k].as_u64().and_then(|n| u32::try_from(n).ok());
        let item = match v["type"].as_str()? {
            "string" => Item::Text(v["value"].as_str()?.chars().take(MAX_TEXT).collect()),
            "integer" => Item::Integer(v["value"].as_i64()?),
            "double" => Item::Double(v["value"].as_f64().filter(|f| f.is_finite())?),
            "boolean" => Item::Boolean(v["value"].as_bool()?),
            "unitDouble" => Item::Unit(id("unit")?, v["value"].as_f64().filter(|f| f.is_finite())?),
            "enumerated" => Item::Enum(id("enumType")?, id("value")?),
            _ => return None,
        };
        Some((key, item))
    }
}

/// An `ActionDescriptor`'s values, in the order they were put.
#[derive(Debug, Default, Trace, Finalize, JsData)]
// Sound: holds no GC pointers.
#[boa_gc(unsafe_empty_trace)]
struct Descriptor(RefCell<Vec<(u32, Item)>>);

/// The prototype and the string ids seen in this run.
#[derive(Trace, Finalize, JsData)]
struct Registry {
    proto: JsObject,
    #[unsafe_ignore_trace]
    strings: RefCell<HashMap<u32, String>>,
}

fn proto(ctx: &Context) -> JsResult<JsObject> {
    ctx.realm().host_defined().get::<Registry>().map(|r| r.proto.clone()).ok_or_else(|| error("ActionDescriptor is not available"))
}

fn type_error(m: &str) -> boa_engine::JsError {
    JsNativeError::typ().with_message(m.to_owned()).into()
}

fn text(v: Option<&JsValue>, ctx: &mut Context) -> JsResult<String> {
    Ok(value::texts(std::slice::from_ref(v.unwrap_or(&JsValue::undefined())), ctx)?.concat())
}

fn id(v: Option<&JsValue>, ctx: &mut Context) -> JsResult<u32> {
    v.unwrap_or(&JsValue::undefined()).to_u32(ctx)
}

fn with<R>(this: &JsValue, f: impl FnOnce(&mut Vec<(u32, Item)>) -> R) -> JsResult<R> {
    let o = this.as_object().ok_or_else(|| type_error("not an ActionDescriptor"))?;
    let d = o.downcast_ref::<Descriptor>().ok_or_else(|| type_error("not an ActionDescriptor"))?;
    let mut items = d.0.borrow_mut();
    Ok(f(&mut items))
}

fn put(this: &JsValue, key: u32, item: Item) -> JsResult<JsValue> {
    with(this, |items| match items.iter().position(|(k, _)| *k == key) {
        Some(i) => {
            if let Some(slot) = items.get_mut(i) {
                slot.1 = item;
            }
            Ok(())
        }
        None if items.len() >= MAX_KEYS => Err(error(format!("a descriptor holds at most {MAX_KEYS} keys"))),
        None => {
            items.push((key, item));
            Ok(())
        }
    })??;
    Ok(JsValue::undefined())
}

fn get(this: &JsValue, key: u32) -> JsResult<Item> {
    with(this, |items| items.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()))?.ok_or_else(|| error("the descriptor has no such key"))
}

fn put_string(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let k = id(args.first(), ctx)?;
    let v = text(args.get(1), ctx)?;
    if v.len() > MAX_TEXT {
        return Err(JsNativeError::range().with_message(format!("a descriptor text holds at most {} KiB", MAX_TEXT >> 10)).into());
    }
    put(this, k, Item::Text(v))
}

fn put_integer(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let k = id(args.first(), ctx)?;
    let v = args.get(1).unwrap_or(&JsValue::undefined()).to_i32(ctx)?;
    put(this, k, Item::Integer(i64::from(v)))
}

fn finite(v: Option<&JsValue>, ctx: &mut Context) -> JsResult<f64> {
    let f = v.unwrap_or(&JsValue::undefined()).to_number(ctx)?;
    if f.is_finite() { Ok(f) } else { Err(JsNativeError::range().with_message("a descriptor number must be finite").into()) }
}

fn put_double(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let k = id(args.first(), ctx)?;
    let v = finite(args.get(1), ctx)?;
    put(this, k, Item::Double(v))
}

fn put_boolean(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let k = id(args.first(), ctx)?;
    put(this, k, Item::Boolean(args.get(1).is_some_and(JsValue::to_boolean)))
}

fn put_unit_double(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let k = id(args.first(), ctx)?;
    let unit = id(args.get(1), ctx)?;
    let v = finite(args.get(2), ctx)?;
    put(this, k, Item::Unit(unit, v))
}

fn put_enumerated(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let k = id(args.first(), ctx)?;
    let t = id(args.get(1), ctx)?;
    let v = id(args.get(2), ctx)?;
    put(this, k, Item::Enum(t, v))
}

fn mismatch() -> boa_engine::JsError {
    error("the descriptor's value under that key is of another type")
}

fn get_string(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    match get(this, id(args.first(), ctx)?)? {
        Item::Text(s) => Ok(JsValue::from(JsString::from(s.as_str()))),
        _ => Err(mismatch()),
    }
}

fn get_integer(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    match get(this, id(args.first(), ctx)?)? {
        #[allow(clippy::cast_precision_loss)]
        Item::Integer(i) => Ok(JsValue::from(i as f64)),
        _ => Err(mismatch()),
    }
}

fn get_double(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    match get(this, id(args.first(), ctx)?)? {
        Item::Double(d) => Ok(JsValue::from(d)),
        #[allow(clippy::cast_precision_loss)]
        Item::Integer(i) => Ok(JsValue::from(i as f64)),
        _ => Err(mismatch()),
    }
}

fn get_boolean(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    match get(this, id(args.first(), ctx)?)? {
        Item::Boolean(b) => Ok(JsValue::from(b)),
        _ => Err(mismatch()),
    }
}

fn get_unit_value(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    match get(this, id(args.first(), ctx)?)? {
        Item::Unit(_, d) => Ok(JsValue::from(d)),
        _ => Err(mismatch()),
    }
}

fn get_unit_type(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    match get(this, id(args.first(), ctx)?)? {
        Item::Unit(u, _) => Ok(JsValue::from(u)),
        _ => Err(mismatch()),
    }
}

fn get_enum_value(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    match get(this, id(args.first(), ctx)?)? {
        Item::Enum(_, v) => Ok(JsValue::from(v)),
        _ => Err(mismatch()),
    }
}

fn get_enum_type(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    match get(this, id(args.first(), ctx)?)? {
        Item::Enum(t, _) => Ok(JsValue::from(t)),
        _ => Err(mismatch()),
    }
}

fn get_type(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let item = get(this, id(args.first(), ctx)?)?;
    Ok(JsValue::from(JsString::from(item.type_name())))
}

fn has_key(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let k = id(args.first(), ctx)?;
    Ok(JsValue::from(with(this, |items| items.iter().any(|(key, _)| *key == k))?))
}

fn get_key(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let i = args.first().unwrap_or(&JsValue::undefined()).to_u32(ctx)?;
    let k = with(this, |items| usize::try_from(i).ok().and_then(|i| items.get(i)).map(|(k, _)| *k))?;
    k.map(JsValue::from).ok_or_else(|| JsNativeError::range().with_message("no key at that index").into())
}

fn count(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let n = with(this, |items| items.len())?;
    Ok(JsValue::from(u32::try_from(n).unwrap_or(u32::MAX)))
}

fn erase(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let k = id(args.first(), ctx)?;
    with(this, |items| items.retain(|(key, _)| *key != k))?;
    Ok(JsValue::undefined())
}

fn clear(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    with(this, Vec::clear)?;
    Ok(JsValue::undefined())
}

fn new_descriptor(ctx: &mut Context, items: Vec<(u32, Item)>) -> JsResult<JsValue> {
    Ok(JsObject::from_proto_and_data(Some(proto(ctx)?), Descriptor(RefCell::new(items))).into())
}

fn construct(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    new_descriptor(ctx, Vec::new())
}

// ---------- type ids ----------

/// `charIDToTypeID("Nm  ")`: the four characters as a big-endian number.
fn char_to_id(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let s = text(args.first(), ctx)?;
    let bytes: Vec<u8> = s.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect();
    let [a, b, c, d] = bytes.as_slice() else { return Err(error("a char ID is four characters")) };
    Ok(JsValue::from(u32::from_be_bytes([*a, *b, *c, *d])))
}

fn id_to_char(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let b = id(args.first(), ctx)?.to_be_bytes();
    let printable = b.iter().all(|c| (0x20..0x7f).contains(c));
    let s: String = if printable { b.iter().map(|c| char::from(*c)).collect() } else { String::new() };
    Ok(JsValue::from(JsString::from(s.as_str())))
}

/// The id of a string: the same in every run (FNV-1a), with the top bit set so it never equals
/// a char id made of printable characters.
fn string_id(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h | 0x8000_0000
}

fn string_to_id(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let s = text(args.first(), ctx)?;
    let id = string_id(&s);
    if let Some(r) = ctx.realm().host_defined().get::<Registry>() {
        let mut strings = r.strings.borrow_mut();
        if strings.len() < 1 << 16 {
            strings.entry(id).or_insert(s);
        }
    }
    Ok(JsValue::from(id))
}

fn id_to_string(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = id(args.first(), ctx)?;
    let s = ctx.realm().host_defined().get::<Registry>().and_then(|r| r.strings.borrow().get(&id).cloned()).unwrap_or_default();
    Ok(JsValue::from(JsString::from(s.as_str())))
}

// ---------- custom options ----------

fn put_custom(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let key = text(args.first(), ctx)?;
    let items = match args.get(1).and_then(JsValue::as_object) {
        Some(o) => o.downcast_ref::<Descriptor>().map(|d| d.0.borrow().clone()),
        None => None,
    }
    .ok_or_else(|| type_error("putCustomOptions takes a key and an ActionDescriptor"))?;
    let persistent = args.get(2).is_none_or(|v| v.is_undefined() || v.to_boolean());
    let value = json!({ "items": items.iter().map(|(k, v)| v.to_json(*k)).collect::<Vec<_>>() });
    command(ctx, "scripts.customOptions.put", json!({ "key": key, "value": value, "persistent": persistent }))?;
    Ok(JsValue::undefined())
}

fn get_custom(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let key = text(args.first(), ctx)?;
    let v = command(ctx, "scripts.customOptions.get", json!({ "key": key }))?;
    if v.is_null() {
        return Err(error_with_number(ctx, format!("there are no custom options for {key}"), 8500));
    }
    let items: Vec<(u32, Item)> = v["items"].as_array().map(|a| a.iter().take(MAX_KEYS).filter_map(Item::from_json).collect()).unwrap_or_default();
    new_descriptor(ctx, items)
}

fn erase_custom(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let key = text(args.first(), ctx)?;
    command(ctx, "scripts.customOptions.erase", json!({ "key": key }))?;
    Ok(JsValue::undefined())
}

/// Install `ActionDescriptor`, `DescValueType`, the type-id functions and the custom-options
/// methods of `app`.
pub(crate) fn install(ctx: &mut Context, app: &JsObject) -> JsResult<()> {
    let count_get = getter(ctx, count);
    let methods: [(&str, Native, usize); 21] = [
        ("putString", put_string, 2),
        ("getString", get_string, 1),
        ("putInteger", put_integer, 2),
        ("getInteger", get_integer, 1),
        ("putDouble", put_double, 2),
        ("getDouble", get_double, 1),
        ("putBoolean", put_boolean, 2),
        ("getBoolean", get_boolean, 1),
        ("putUnitDouble", put_unit_double, 3),
        ("getUnitDoubleValue", get_unit_value, 1),
        ("getUnitDoubleType", get_unit_type, 1),
        ("putEnumerated", put_enumerated, 3),
        ("getEnumerationValue", get_enum_value, 1),
        ("getEnumerationType", get_enum_type, 1),
        ("getType", get_type, 1),
        ("hasKey", has_key, 1),
        ("getKey", get_key, 1),
        ("erase", erase, 1),
        ("clear", clear, 0),
        ("isEqual", is_equal, 1),
        ("toString", desc_to_string, 0),
    ];
    let mut p = ObjectInitializer::new(ctx);
    p.accessor(js_string!("count"), Some(count_get), None, Attribute::CONFIGURABLE);
    for (n, f, len) in methods {
        p.function(NativeFunction::from_fn_ptr(f), JsString::from(n), len);
    }
    let proto = p.build();
    let ctor = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(construct))
        .name(js_string!("ActionDescriptor"))
        .length(0)
        .constructor(true)
        .build();
    crate::scriptui::link_prototype(&ctor, &proto, ctx)?;
    ctx.register_global_property(js_string!("ActionDescriptor"), ctor, Attribute::WRITABLE | Attribute::CONFIGURABLE)?;
    let realm = ctx.realm().clone();
    realm.host_defined_mut().insert(Registry { proto, strings: RefCell::default() });

    let mut types = ObjectInitializer::new(ctx);
    for t in ["STRINGTYPE", "INTEGERTYPE", "DOUBLETYPE", "BOOLEANTYPE", "UNITDOUBLE", "ENUMERATEDTYPE"] {
        types.property(JsString::from(t), JsString::from(format!("DescValueType.{t}")), Attribute::ENUMERABLE);
    }
    let types = types.build();
    ctx.register_global_property(js_string!("DescValueType"), types, Attribute::WRITABLE | Attribute::CONFIGURABLE)?;

    for (name, f, len) in [
        ("charIDToTypeID", char_to_id as Native, 1),
        ("typeIDToCharID", id_to_char, 1),
        ("stringIDToTypeID", string_to_id, 1),
        ("typeIDToStringID", id_to_string, 1),
    ] {
        ctx.register_global_builtin_callable(JsString::from(name), len, NativeFunction::from_fn_ptr(f))?;
    }
    for (name, f, len) in [("putCustomOptions", put_custom as Native, 3), ("getCustomOptions", get_custom, 1), ("eraseCustomOptions", erase_custom, 1)] {
        let f = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(f)).name(JsString::from(name)).length(len).build();
        app.set(JsString::from(name), f, true, ctx)?;
    }
    Ok(())
}

fn is_equal(this: &JsValue, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let a = with(this, |items| items.clone())?;
    let b = match args.first().and_then(JsValue::as_object) {
        Some(o) => o.downcast_ref::<Descriptor>().map(|d| d.0.borrow().clone()),
        None => None,
    };
    Ok(JsValue::from(b.is_some_and(|b| b == a)))
}

fn desc_to_string(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(js_string!("[ActionDescriptor]")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_ids_are_stable_and_never_char_ids() {
        assert_eq!(string_id("myScriptSettings"), string_id("myScriptSettings"));
        assert_ne!(string_id("a"), string_id("b"));
        assert!(string_id("x") >= 0x8000_0000);
    }

    #[test]
    fn items_round_trip_through_json() {
        let items = [Item::Text("hi".into()), Item::Integer(-3), Item::Double(0.5), Item::Boolean(true), Item::Unit(7, 2.5), Item::Enum(1, 2)];
        for (i, item) in items.into_iter().enumerate() {
            let key = u32::try_from(i).unwrap();
            assert_eq!(Item::from_json(&item.to_json(key)), Some((key, item)));
        }
        assert_eq!(Item::from_json(&json!({"key": 1, "type": "double", "value": null})), None);
        assert_eq!(Item::from_json(&json!({"key": -1, "type": "integer", "value": 1})), None);
    }
}
