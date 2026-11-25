//! The document model scripts see beyond `app` and its documents: layers and layer sets, type,
//! History states, the selection, sizes in the ruler unit, and saving.
//!
//! Every value is read from the session when it is asked for, and every change is one session
//! command from [`crate::commands`]. Commands act on the active document, so a change to another
//! document makes it active first. Layers are identified by their layer id, which stays the same
//! while the layer exists, so a `Layer` object keeps meaning the same layer when others are added
//! or moved. One object is kept per layer, document and collection for identity comparisons
//! (`a === b`), up to [`MAX_OBJECTS`] of them in a run: past that, reading one that was not kept
//! gives a new, equal object each time (`a.id == b.id` still holds), so memory stays bounded.

use std::cell::Cell;
use std::collections::HashMap;

use boa_engine::object::builtins::JsArray;
use boa_engine::object::{FunctionObjectBuilder, ObjectInitializer};
use boa_engine::property::Attribute;
use boa_engine::{Context, Finalize, JsData, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, Trace, js_string};
use serde_json::{Value, json};

use crate::commands::BLENDS;
use crate::natives::{DocKey, Native, command, doc_id, document_object, entry_id, error, error_with_number, getter, resolve, session_inspect};
use crate::{unit_value, value};

/// Most layers walked when looking one up.
const MAX_WALK: usize = 1 << 20;

// ---------- enumerations ----------

/// The enumerations scripts compare against. A value is the text `"Name.VALUE"`, so `==`
/// between what a getter returns and the constant holds.
const ENUMS: &[(&str, &[&str])] = &[
    ("Units", &["PIXELS", "INCHES", "CM", "MM", "POINTS", "PICAS", "PERCENT"]),
    ("TypeUnits", &["PIXELS", "POINTS", "MM"]),
    (
        "LayerKind",
        &[
            "NORMAL",
            "TEXT",
            "SOLIDFILL",
            "SMARTOBJECT",
            "LEVELS",
            "CURVES",
            "HUESATURATION",
            "BRIGHTNESSCONTRAST",
            "COLORBALANCE",
            "CHANNELMIXER",
            "EXPOSURE",
            "GRADIENTMAP",
            "INVERSION",
            "PHOTOFILTER",
            "POSTERIZE",
            "SELECTIVECOLOR",
            "THRESHOLD",
            "VIBRANCE",
            "BLACKANDWHITE",
            "COLORLOOKUP",
        ],
    ),
    ("SaveOptions", &["DONOTSAVECHANGES", "SAVECHANGES", "PROMPTTOSAVECHANGES"]),
    ("ElementPlacement", &["PLACEBEFORE", "PLACEAFTER", "PLACEATBEGINNING", "PLACEATEND", "INSIDE"]),
    ("AnchorPosition", &["TOPLEFT", "TOPCENTER", "TOPRIGHT", "MIDDLELEFT", "MIDDLECENTER", "MIDDLERIGHT", "BOTTOMLEFT", "BOTTOMCENTER", "BOTTOMRIGHT"]),
    ("ResampleMethod", &["NEARESTNEIGHBOR", "BILINEAR", "BICUBIC", "BICUBICSHARPER", "BICUBICSMOOTHER", "PRESERVEDETAILS", "AUTOMATIC"]),
    ("ExportType", &["SAVEFORWEB"]),
    ("SaveDocumentType", &["JPEG", "PNG", "COMPUSERVEGIF", "PHOTOSHOP", "TIFF"]),
    ("DialogModes", &["ALL", "ERROR", "NO"]),
    ("TrimType", &["TRANSPARENT", "TOPLEFT", "BOTTOMRIGHT"]),
    ("TIFFEncoding", &["NONE", "TIFFLZW", "TIFFZIP"]),
    ("SelectionType", &["REPLACE"]),
    ("NewDocumentMode", &["RGB", "GRAYSCALE", "CMYK", "LAB", "BITMAP"]),
    ("DocumentFill", &["WHITE", "BACKGROUNDCOLOR", "TRANSPARENT"]),
];

/// `BlendMode.X` for a blend label: `"Linear Dodge (Add)"` is `LINEARDODGE`.
fn blend_enum(label: &str) -> String {
    let base = label.split(" (").next().unwrap_or(label);
    format!("BlendMode.{}", base.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_uppercase())
}

/// The blend label for `BlendMode.X`, or for a label itself.
fn blend_label(v: &str) -> Option<&'static str> {
    BLENDS.iter().copied().find(|l| *l == v || blend_enum(l) == v)
}

/// Unit preference name ↔ `Units` value ↔ unit-value type.
const UNITS: &[(&str, &str, &str)] = &[
    ("pixels", "Units.PIXELS", "px"),
    ("inches", "Units.INCHES", "in"),
    ("cm", "Units.CM", "cm"),
    ("mm", "Units.MM", "mm"),
    ("points", "Units.POINTS", "pt"),
    ("picas", "Units.PICAS", "pc"),
    ("percent", "Units.PERCENT", "%"),
];
const TYPE_UNITS: &[(&str, &str, &str)] = &[("pixels", "TypeUnits.PIXELS", "px"), ("points", "TypeUnits.POINTS", "pt"), ("mm", "TypeUnits.MM", "mm")];

const RULERS: &str = "unitsAndRulers.rulers";
const TYPE: &str = "unitsAndRulers.typeUnits";

fn pref_row(path: &str) -> &'static [(&'static str, &'static str, &'static str)] {
    if path == TYPE { TYPE_UNITS } else { UNITS }
}

/// The unit-value type of a unit preference (`px` when it is unknown).
fn unit_typ(ctx: &mut Context, path: &str) -> JsResult<&'static str> {
    let v = command(ctx, "prefs.get", json!({ "path": path }))?;
    Ok(pref_row(path).iter().find(|(n, _, _)| v.as_str() == Some(n)).map_or("px", |(_, _, t)| *t))
}

fn get_unit_pref(ctx: &mut Context, path: &str) -> JsResult<JsValue> {
    let v = command(ctx, "prefs.get", json!({ "path": path }))?;
    let e = pref_row(path).iter().find(|(n, _, _)| v.as_str() == Some(n)).map_or(pref_row(path)[0].1, |(_, e, _)| *e);
    Ok(JsValue::from(JsString::from(e)))
}

fn set_unit_pref(ctx: &mut Context, path: &str, v: &JsValue) -> JsResult<JsValue> {
    let t = text(v, ctx)?;
    let name = pref_row(path)
        .iter()
        .find(|(n, e, _)| *e == t || *n == t)
        .map(|(n, _, _)| *n)
        .ok_or_else(|| error(format!("{t} is not a unit for this preference")))?;
    command(ctx, "prefs.set", json!({ "path": path, "value": name }))?;
    Ok(JsValue::undefined())
}

fn rulers_get(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    get_unit_pref(ctx, RULERS)
}
fn rulers_set(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    set_unit_pref(ctx, RULERS, args.first().unwrap_or(&JsValue::undefined()))
}
fn type_units_get(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    get_unit_pref(ctx, TYPE)
}
fn type_units_set(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    set_unit_pref(ctx, TYPE, args.first().unwrap_or(&JsValue::undefined()))
}

// ---------- small helpers ----------

fn text(v: &JsValue, ctx: &mut Context) -> JsResult<String> {
    Ok(value::texts(std::slice::from_ref(v), ctx)?.concat())
}

fn arg(args: &[JsValue], i: usize) -> Option<&JsValue> {
    args.get(i).filter(|v| !v.is_undefined() && !v.is_null())
}

fn type_error(m: &str) -> boa_engine::JsError {
    JsNativeError::typ().with_message(m.to_owned()).into()
}

fn js(v: &Value, ctx: &mut Context) -> JsResult<JsValue> {
    value::from_json(v, ctx)
}

/// The document a `Document` object means.
fn doc_key(this: &JsValue) -> JsResult<DocKey> {
    doc_id(this).ok_or_else(|| type_error("not a Document"))
}

/// Run editing command `id` on document `doc`: the host makes it active and runs the command in
/// one call, so a tab closed or switched meanwhile can't redirect it.
fn edit(ctx: &mut Context, doc: DocKey, id: &str, params: Value) -> JsResult<Value> {
    let mut p = params;
    p[crate::commands::ON_DOCUMENT] = json!(doc.serial);
    command(ctx, id, p)
}

/// `document.inspect` of `key`.
fn inspect(ctx: &mut Context, key: DocKey) -> JsResult<Value> {
    crate::natives::inspect_document(key, false, ctx)
}

/// [`inspect`] with the History panel's rows, which a long history makes large: only the
/// History members ask for them.
fn inspect_history(ctx: &mut Context, key: DocKey) -> JsResult<Value> {
    crate::natives::inspect_document(key, true, ctx)
}

fn dpi(doc: &Value) -> f64 {
    doc["resolution"].as_f64().filter(|r| r.is_finite() && *r > 0.0).unwrap_or(72.0)
}

/// A length of `px` pixels in the ruler unit.
pub(crate) fn ruler_length(ctx: &mut Context, px: f64, dpi: f64) -> JsResult<JsValue> {
    let typ = unit_typ(ctx, RULERS)?;
    unit_value::document_length(px, dpi, typ, ctx)
}

/// Farthest a length may reach, in pixels: past any document, and small enough that sums and
/// differences of lengths stay exact.
const MAX_PX: f64 = 1e9;

/// A script length (a unit value, or a number in the ruler unit) in pixels: finite, and within
/// [`MAX_PX`] of zero.
fn to_px(ctx: &mut Context, v: &JsValue, dpi: f64) -> JsResult<f64> {
    let typ = unit_typ(ctx, RULERS)?;
    let px = unit_value::to_document_pixels(v, dpi, typ, ctx)?;
    if px.is_finite() && px.abs() <= MAX_PX {
        Ok(px)
    } else {
        Err(JsNativeError::range().with_message(format!("a length must be a finite number within {MAX_PX:e} pixels")).into())
    }
}

/// A length a script gave, converted (its `valueOf` run) but not yet in pixels: [`px_at`] makes
/// it pixels at the resolution read after every argument is converted.
struct Length(f64, String);

fn length(ctx: &mut Context, v: &JsValue) -> JsResult<Length> {
    let typ = unit_typ(ctx, RULERS)?;
    let (value, typ) = unit_value::length_of(v, typ, ctx)?;
    Ok(Length(value, typ))
}

fn px_at(l: &Length, dpi: f64) -> JsResult<f64> {
    let px = unit_value::length_in_pixels(l.0, &l.1, dpi)?;
    if px.is_finite() && px.abs() <= MAX_PX {
        Ok(px)
    } else {
        Err(JsNativeError::range().with_message(format!("a length must be a finite number within {MAX_PX:e} pixels")).into())
    }
}

/// A document side in whole pixels, as `documents.add` takes it.
fn side(px: f64) -> JsResult<f64> {
    let px = px.round();
    if (1.0..=300_000.0).contains(&px) { Ok(px) } else { Err(JsNativeError::range().with_message("a document side is 1 to 300000 pixels").into()) }
}

fn whole(px: f64) -> i64 {
    // In range: commands bound every position, and the cast saturates.
    px.round() as i64
}

/// Four lengths `[left, top, right, bottom]` from `[x, y, w, h]`.
fn bounds(ctx: &mut Context, xywh: &Value, dpi: f64) -> JsResult<JsValue> {
    let n = |i: usize| xywh.get(i).and_then(Value::as_f64).unwrap_or(0.0);
    let (x, y, w, h) = (n(0), n(1), n(2), n(3));
    let mut out = Vec::with_capacity(4);
    for v in [x, y, x + w, y + h] {
        out.push(ruler_length(ctx, v, dpi)?);
    }
    Ok(JsArray::from_iter(out, ctx).into())
}

// ---------- object caches and prototypes ----------

#[derive(Trace, Finalize, JsData)]
struct Protos {
    layer: JsObject,
    text: JsObject,
    history: JsObject,
    selection: JsObject,
}

fn protos(ctx: &Context) -> JsResult<(JsObject, JsObject, JsObject, JsObject)> {
    let realm = ctx.realm().clone();
    let data = realm.host_defined();
    let p = data.get::<Protos>().ok_or_else(|| error("the document model is missing"))?;
    Ok((p.layer.clone(), p.text.clone(), p.history.clone(), p.selection.clone()))
}

/// One object per layer, History row and selection, so `==` compares what they stand for.
#[derive(Default, Trace, Finalize, JsData)]
struct Objects(HashMap<(u8, u64, u64, u64), JsObject>);

const LAYER: u8 = 0;
const HISTORY: u8 = 1;
const SELECTION: u8 = 2;
/// A collection, `COLLECTION + which` (the open documents are `COLLECTION + DOCUMENTS`).
const COLLECTION: u8 = 16;
const DOCUMENTS: u8 = 15;

/// The array kept for the collection `key`, filled with `items`; `methods` adds its methods the
/// first time. The same collection read twice is the same object (`doc.layers === doc.layers`),
/// as in Photoshop, while the run keeps fewer than [`MAX_OBJECTS`] objects, and an array a
/// script holds shows the members as they are at the last read.
pub(crate) fn live_collection(
    ctx: &mut Context,
    key: (u8, u64, u64, u64),
    items: Vec<JsValue>,
    methods: impl FnOnce(&JsArray, &mut Context) -> JsResult<()>,
) -> JsResult<JsValue> {
    // Refilled by defining its elements, never by assignment: a setter a script put on
    // `Array.prototype` does not run inside the host call.
    let refill = |arr: &JsArray, items: &[JsValue], ctx: &mut Context| -> JsResult<bool> {
        if !arr.set(js_string!("length"), 0, false, ctx)? || arr.length(ctx)? != 0 {
            return Ok(false);
        }
        for (i, v) in items.iter().enumerate() {
            if !arr.create_data_property(i, v.clone(), ctx)? {
                return Ok(false);
            }
        }
        Ok(true)
    };
    if let Some(arr) = cached(ctx, key).and_then(|o| JsArray::from_object(o).ok())
        && refill(&arr, &items, ctx)?
    {
        return Ok(arr.into());
    }
    // New, or the script froze or sealed the one it had: a new array from now on.
    let arr = JsArray::from_iter(items, ctx);
    methods(&arr, ctx)?;
    cache(ctx, key, &arr.clone().into());
    Ok(arr.into())
}

/// The cache key of the open documents' collection.
pub(crate) const fn documents_key() -> (u8, u64, u64, u64) {
    (COLLECTION + DOCUMENTS, 0, 0, 0)
}

fn cached(ctx: &Context, key: (u8, u64, u64, u64)) -> Option<JsObject> {
    ctx.realm().host_defined().get::<Objects>().and_then(|c| c.0.get(&key).cloned())
}

/// Most objects kept for identity, of each kind (layers and collections here, documents in
/// `natives`). Past it a new object is not kept: reading the same layer or document again then
/// gives an equal but not identical object, and memory stays bounded.
pub(crate) const MAX_OBJECTS: usize = 1 << 16;

fn cache(ctx: &Context, key: (u8, u64, u64, u64), obj: &JsObject) {
    let realm = ctx.realm().clone();
    let mut data = realm.host_defined_mut();
    if data.get::<Objects>().is_none() {
        data.insert(Objects::default());
    }
    if let Some(c) = data.get_mut::<Objects>()
        && (c.0.len() < MAX_OBJECTS || c.0.contains_key(&key))
    {
        c.0.insert(key, obj.clone());
    }
}

fn uncache(ctx: &Context, key: (u8, u64, u64, u64)) {
    if let Some(c) = ctx.realm().clone().host_defined_mut().get_mut::<Objects>() {
        c.0.remove(&key);
    }
}

/// A `Layer` (an `ArtLayer` or a `LayerSet`): its document and layer id. The id changes only
/// when the layer is replaced by one of another kind (`kind = LayerKind.TEXT`).
#[derive(Debug, Trace, Finalize, JsData)]
// Sound: holds only integers, no GC pointers.
#[boa_gc(unsafe_empty_trace)]
struct LayerRef {
    doc: DocKey,
    id: Cell<u64>,
}

/// A layer's `textItem`.
#[derive(Debug, Clone, Copy, Trace, Finalize, JsData)]
#[boa_gc(empty_trace)]
struct TextRef {
    doc: DocKey,
    layer: u64,
}

/// A History panel state of a document, by the state's id: it means that state while it exists,
/// wherever its row moves, and nothing once an edit has discarded it.
#[derive(Debug, Clone, Copy, Trace, Finalize, JsData)]
#[boa_gc(empty_trace)]
struct HistoryRef {
    doc: DocKey,
    state: u64,
}

/// A document's `selection`.
#[derive(Debug, Clone, Copy, Trace, Finalize, JsData)]
#[boa_gc(empty_trace)]
struct SelectionRef(DocKey);

fn layer_object(ctx: &mut Context, doc: DocKey, id: u64) -> JsResult<JsObject> {
    let key = (LAYER, doc.id, doc.serial, id);
    if let Some(o) = cached(ctx, key) {
        return Ok(o);
    }
    let (proto, _, _, _) = protos(ctx)?;
    let obj = JsObject::from_proto_and_data(Some(proto), LayerRef { doc, id: Cell::new(id) });
    cache(ctx, key, &obj);
    Ok(obj)
}

fn history_object(ctx: &mut Context, doc: DocKey, state: u64) -> JsResult<JsObject> {
    let key = (HISTORY, doc.id, doc.serial, state);
    if let Some(o) = cached(ctx, key) {
        return Ok(o);
    }
    let (_, _, proto, _) = protos(ctx)?;
    let obj = JsObject::from_proto_and_data(Some(proto), HistoryRef { doc, state });
    cache(ctx, key, &obj);
    Ok(obj)
}

fn layer_ref(this: &JsValue) -> JsResult<(DocKey, u64)> {
    this.as_object().and_then(|o| o.downcast_ref::<LayerRef>().map(|l| (l.doc, l.id.get()))).ok_or_else(|| type_error("not a Layer"))
}

/// Layer `id` in the tree `layers`, with the id of the group holding it.
fn find(layers: &Value, id: u64) -> Option<(Value, Option<u64>)> {
    let mut stack: Vec<(&Value, Option<u64>)> = layers.as_array()?.iter().map(|l| (l, None)).collect();
    let mut seen = 0usize;
    while let Some((l, parent)) = stack.pop() {
        seen += 1;
        if seen > MAX_WALK {
            return None;
        }
        if l["id"].as_u64() == Some(id) {
            return Some((l.clone(), parent));
        }
        if let Some(kids) = l["children"].as_array() {
            stack.extend(kids.iter().map(|k| (k, l["id"].as_u64())));
        }
    }
    None
}

/// The layer `this` stands for: its document key, id, `document.inspect` node, parent group,
/// and the document's `document.inspect`.
fn node(ctx: &mut Context, this: &JsValue) -> JsResult<(DocKey, u64, Value, Option<u64>, Value)> {
    let (doc, id) = layer_ref(this)?;
    let d = inspect(ctx, doc)?;
    let (n, parent) = find(&d["layers"], id).ok_or_else(|| error("the layer no longer exists"))?;
    Ok((doc, id, n, parent, d))
}

fn is_set(n: &Value) -> bool {
    n["kind"].as_str() == Some("Group")
}

// ---------- collections ----------

/// Which layers a collection holds.
const ALL: u8 = 0;
const ART: u8 = 1;
const SETS: u8 = 2;

/// What `add` and `getByName` of a collection need.
#[derive(Clone, Trace, Finalize)]
struct Coll {
    id: u64,
    serial: u64,
    /// The group holding them; `u64::MAX` for the document's top level.
    parent: u64,
    which: u8,
}

fn coll_doc(c: &Coll) -> DocKey {
    DocKey { id: c.id, serial: c.serial }
}

fn members(d: &Value, c: &Coll) -> Vec<(u64, String)> {
    let level = if c.parent == u64::MAX { Some(d["layers"].clone()) } else { find(&d["layers"], c.parent).map(|(n, _)| n["children"].clone()) };
    level
        .and_then(|l| l.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter(|n| match c.which {
            ART => !is_set(n),
            SETS => is_set(n),
            _ => true,
        })
        .filter_map(|n| Some((n["id"].as_u64()?, n["name"].as_str().unwrap_or("").to_owned())))
        .collect()
}

fn collection(ctx: &mut Context, doc: DocKey, parent: Option<u64>, which: u8) -> JsResult<JsValue> {
    let d = inspect(ctx, doc)?;
    collection_of(ctx, doc, parent, which, &d)
}

/// [`collection`] from the document's inspection `d`, already read.
fn collection_of(ctx: &mut Context, doc: DocKey, parent: Option<u64>, which: u8, d: &Value) -> JsResult<JsValue> {
    let c = Coll { id: doc.id, serial: doc.serial, parent: parent.unwrap_or(u64::MAX), which };
    let mut objs = Vec::new();
    for (id, _) in members(d, &c) {
        objs.push(JsValue::from(layer_object(ctx, doc, id)?));
    }
    let key = (COLLECTION + which, doc.id, doc.serial, c.parent);
    live_collection(ctx, key, objs, move |arr, ctx| {
        let by_name = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_copy_closure_with_captures(coll_get_by_name, c.clone()))
            .name(js_string!("getByName"))
            .length(1)
            .build();
        arr.create_data_property_or_throw(js_string!("getByName"), by_name, ctx)?;
        if which != ALL {
            let add =
                FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_copy_closure_with_captures(coll_add, c)).name(js_string!("add")).length(0).build();
            arr.create_data_property_or_throw(js_string!("add"), add, ctx)?;
        }
        Ok(())
    })
}

fn coll_get_by_name(_: &JsValue, args: &[JsValue], c: &Coll, ctx: &mut Context) -> JsResult<JsValue> {
    let name = text(args.first().unwrap_or(&JsValue::undefined()), ctx)?;
    let d = inspect(ctx, coll_doc(c))?;
    match members(&d, c).into_iter().find(|(_, n)| *n == name) {
        Some((id, _)) => Ok(layer_object(ctx, coll_doc(c), id)?.into()),
        None => Err(error_with_number(ctx, "No such element", 1302)),
    }
}

/// `artLayers.add()` / `layerSets.add()`: a new layer or group, inside this collection's group.
fn coll_add(_: &JsValue, _: &[JsValue], c: &Coll, ctx: &mut Context) -> JsResult<JsValue> {
    let doc = coll_doc(c);
    let id = if c.which == SETS { "layer.new.group" } else { "layer.new.layer" };
    // A group's collection: made inside that group in the same edit, so a group deleted
    // meanwhile is an error that leaves the document as it was.
    let params = if c.parent == u64::MAX { json!({}) } else { json!({ "group": c.parent }) };
    let made = edit(ctx, doc, id, params)?;
    let new = match made["layer"].as_u64() {
        Some(l) => l,
        None => inspect(ctx, doc)?["activeLayer"].as_u64().ok_or_else(|| error("the new layer was not reported"))?,
    };
    // The document's own collections add beside the active layer, as Photoshop's do: inside
    // the active layer's group when it is in one (scripts move it out with `move` if they want).
    Ok(layer_object(ctx, doc, new)?.into())
}

// ---------- layers ----------

fn layer_name(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (_, _, n, _, _) = node(ctx, this)?;
    js(&n["name"], ctx)
}

fn layer_set_name(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (doc, id) = layer_ref(this)?;
    let name = text(args.first().unwrap_or(&JsValue::undefined()), ctx)?;
    edit(ctx, doc, "layer.renameLayer", json!({ "layer": id, "name": name }))?;
    Ok(JsValue::undefined())
}

fn set_props(ctx: &mut Context, this: &JsValue, props: Value) -> JsResult<JsValue> {
    let (doc, id) = layer_ref(this)?;
    let mut p = props;
    p["layer"] = json!(id);
    edit(ctx, doc, "layer.setProps", p)?;
    Ok(JsValue::undefined())
}

fn layer_visible(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (_, _, n, _, _) = node(ctx, this)?;
    Ok(JsValue::from(n["visible"].as_bool().unwrap_or(true)))
}

fn layer_set_visible(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let v = args.first().is_some_and(JsValue::to_boolean);
    set_props(ctx, this, json!({ "visible": v }))
}

/// Whether the layer's own lock `key` (`all`, `pixels`, `position`, `transparency`) is on. A
/// layer reports its locks only when one is set.
fn lock(this: &JsValue, ctx: &mut Context, key: &str) -> JsResult<JsValue> {
    let (_, _, n, _, _) = node(ctx, this)?;
    Ok(JsValue::from(n["locks"][key].as_bool().unwrap_or(false)))
}

fn set_lock(this: &JsValue, args: &[JsValue], ctx: &mut Context, key: &str) -> JsResult<JsValue> {
    let v = args.first().is_some_and(JsValue::to_boolean);
    set_props(ctx, this, json!({ "locks": { key: v } }))
}

fn lock_all(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    lock(this, ctx, "all")
}

fn set_lock_all(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    set_lock(this, args, ctx, "all")
}

fn lock_pixels(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    lock(this, ctx, "pixels")
}

fn set_lock_pixels(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    set_lock(this, args, ctx, "pixels")
}

fn lock_position(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    lock(this, ctx, "position")
}

fn set_lock_position(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    set_lock(this, args, ctx, "position")
}

fn lock_transparency(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    lock(this, ctx, "transparency")
}

fn set_lock_transparency(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    set_lock(this, args, ctx, "transparency")
}

/// A stored 0..1 fraction (single precision) as a percentage without float noise.
fn as_percent(v: Option<f64>) -> f64 {
    (v.unwrap_or(1.0) * 100.0 * 1e4).round() / 1e4
}

fn percent(args: &[JsValue], ctx: &mut Context) -> JsResult<f64> {
    let v = args.first().unwrap_or(&JsValue::undefined()).to_number(ctx)?;
    if (0.0..=100.0).contains(&v) { Ok(v / 100.0) } else { Err(JsNativeError::range().with_message("an opacity is 0 to 100").into()) }
}

fn layer_opacity(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (_, _, n, _, _) = node(ctx, this)?;
    Ok(JsValue::from(as_percent(n["opacity"].as_f64())))
}

fn layer_set_opacity(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let v = percent(args, ctx)?;
    set_props(ctx, this, json!({ "opacity": v }))
}

fn layer_fill(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (_, _, n, _, _) = node(ctx, this)?;
    Ok(JsValue::from(as_percent(n["fill"].as_f64())))
}

fn layer_set_fill(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let v = percent(args, ctx)?;
    set_props(ctx, this, json!({ "fill": v }))
}

fn layer_blend(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (_, _, n, _, _) = node(ctx, this)?;
    Ok(JsValue::from(JsString::from(blend_enum(n["blend"].as_str().unwrap_or("Normal")).as_str())))
}

fn layer_set_blend(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let t = text(args.first().unwrap_or(&JsValue::undefined()), ctx)?;
    let label = blend_label(&t).ok_or_else(|| error(format!("{t} is not a blend mode PhotoCraft has")))?;
    set_props(ctx, this, json!({ "blend": label }))
}

/// `LayerKind.X` of a layer node; a group has none.
fn kind_of(n: &Value) -> Option<String> {
    let k = match n["kind"].as_str()? {
        "Group" => return None,
        "Type" => "TEXT".to_owned(),
        "Fill" | "Shape" => "SOLIDFILL".to_owned(),
        "Smart Object" => "SMARTOBJECT".to_owned(),
        "Adjustment" => {
            let a = n["adjustment"]["kind"].as_str().unwrap_or("");
            match a {
                "invert" => "INVERSION".to_owned(),
                "blackWhite" | "blackAndWhite" => "BLACKANDWHITE".to_owned(),
                _ => a.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_uppercase(),
            }
        }
        _ => "NORMAL".to_owned(),
    };
    Some(format!("LayerKind.{k}"))
}

fn layer_kind(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (_, _, n, _, _) = node(ctx, this)?;
    Ok(kind_of(&n).map_or(JsValue::undefined(), |k| JsValue::from(JsString::from(k.as_str()))))
}

/// `layer.kind = LayerKind.TEXT` on a new, empty layer turns it into an empty type layer in its
/// place, as scripts do to make text. Other kinds can't be set.
fn layer_set_kind(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let want = text(args.first().unwrap_or(&JsValue::undefined()), ctx)?;
    let (doc, id, n, _, d) = node(ctx, this)?;
    if kind_of(&n).as_deref() == Some(want.as_str()) {
        return Ok(JsValue::undefined());
    }
    let empty = n["bounds"].as_array().is_none_or(|b| b.get(2).and_then(Value::as_f64) == Some(0.0));
    let plain = n["kind"].as_str() == Some("Pixel") && d["backgroundLayer"].as_u64() != Some(id);
    if want != "LayerKind.TEXT" || !empty || !plain {
        return Err(error("only an empty pixel layer can be made a text layer (kind = LayerKind.TEXT); other kinds can't be set"));
    }
    // One edit: the layer becomes the text layer in place, its id and settings kept, so a run
    // stopped right after leaves nothing half done.
    edit(ctx, doc, "type.create", json!({ "x": 0, "y": 0, "text": "", "layer": id }))?;
    Ok(JsValue::undefined())
}

fn layer_typename(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (_, _, n, _, _) = node(ctx, this)?;
    Ok(JsValue::from(if is_set(&n) { js_string!("LayerSet") } else { js_string!("ArtLayer") }))
}

fn layer_bounds(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (_, _, n, _, d) = node(ctx, this)?;
    bounds(ctx, &n["bounds"], dpi(&d))
}

fn layer_is_background(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (_, id, _, _, d) = node(ctx, this)?;
    Ok(JsValue::from(d["backgroundLayer"].as_u64() == Some(id)))
}

fn layer_parent(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (doc, _, _, parent, _) = node(ctx, this)?;
    match parent {
        Some(p) => Ok(layer_object(ctx, doc, p)?.into()),
        None => Ok(document_object(doc, ctx).into()),
    }
}

fn layer_id(this: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let (_, id) = layer_ref(this)?;
    Ok(JsValue::from(id as f64))
}

fn layer_children(this: &JsValue, ctx: &mut Context, which: u8) -> JsResult<JsValue> {
    let (doc, id, n, _, d) = node(ctx, this)?;
    if !is_set(&n) {
        return Ok(JsValue::undefined());
    }
    // The inspection that found the group lists its members too: no second read.
    collection_of(ctx, doc, Some(id), which, &d)
}

fn layer_layers(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    layer_children(this, ctx, ALL)
}
fn layer_art_layers(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    layer_children(this, ctx, ART)
}
fn layer_layer_sets(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    layer_children(this, ctx, SETS)
}

fn layer_text_item(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (doc, id, n, _, _) = node(ctx, this)?;
    if n["kind"].as_str() != Some("Type") {
        return Err(error("the layer is not a text layer"));
    }
    let (_, proto, _, _) = protos(ctx)?;
    Ok(JsObject::from_proto_and_data(Some(proto), TextRef { doc, layer: id }).into())
}

fn layer_to_string(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (_, _, n, _, _) = node(ctx, this)?;
    let kind = if is_set(&n) { "LayerSet" } else { "ArtLayer" };
    Ok(JsValue::from(JsString::from(format!("[{kind} {}]", n["name"].as_str().unwrap_or("")).as_str())))
}

fn layer_remove(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (doc, id) = layer_ref(this)?;
    edit(ctx, doc, "layer.delete", json!({ "layer": id }))?;
    uncache(ctx, (LAYER, doc.id, doc.serial, id));
    Ok(JsValue::undefined())
}

fn layer_duplicate(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (doc, id) = layer_ref(this)?;
    // Made where it goes in one command (`moveTo` with `copy`), so a copy that can't go there,
    // or a run stopped meanwhile, leaves no copy behind; the destination is read first. A group
    // copied into itself is the exception (see below).
    let made = match arg(args, 0) {
        Some(rel) => {
            let (target, position) = destination(ctx, doc, rel, args.get(1))?;
            // A copy of a group placed inside that group (`INSIDE` it, or at its end, beside its
            // last layer) is not a move the app makes in one step: the copy is made first and
            // then moved in, as a layer copied from the group would be. The run can't be stopped
            // between the two (neither charges the budget there), and a copy that can't go in
            // is deleted again, so a stop never leaves it beside the group.
            let node = find(&inspect(ctx, doc)?["layers"], id).map(|(node, _)| node);
            let inside_itself = node
                .is_some_and(|node| node["children"].is_array() && (if target == id { position == "into" } else { find(&node["children"], target).is_some() }));
            if inside_itself {
                let on = |mut p: Value| {
                    p[crate::commands::ON_DOCUMENT] = json!(doc.serial);
                    p
                };
                // Both fit on the tape, or neither is made: the tape refusing the move would
                // stop the run with the copy beside the group. (Room for their small requests and
                // replies, and for the delete if the move is refused.)
                crate::natives::room_for(ctx, 3, 64 << 10)?;
                let made = crate::natives::command_charged(ctx, "layer.duplicate", on(json!({ "layer": id })), (true, false))?;
                let new = made["layer"].as_u64().ok_or_else(|| error("the copy was not reported"))?;
                let placed =
                    crate::natives::command_charged(ctx, "layer.moveTo", on(json!({ "layer": new, "target": target, "position": position })), (false, true));
                if let Err(e) = placed {
                    // Refused (not stopped: nothing charged before it): the copy goes again.
                    if e.as_engine().is_none() {
                        let _ = crate::natives::command_charged(ctx, "layer.delete", on(json!({ "layer": new })), (false, false));
                    }
                    return Err(e);
                }
                made
            } else {
                edit(ctx, doc, "layer.moveTo", json!({ "layer": id, "target": target, "position": position, "copy": true }))?
            }
        }
        None => edit(ctx, doc, "layer.duplicate", json!({ "layer": id }))?,
    };
    let new = made["layer"].as_u64().ok_or_else(|| error("the copy was not reported"))?;
    Ok(layer_object(ctx, doc, new)?.into())
}

fn layer_translate(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (doc, id) = layer_ref(this)?;
    // Converted before the resolution is read: a `valueOf` may change it.
    let lx = arg(args, 0).map(|v| length(ctx, v)).transpose()?;
    let ly = arg(args, 1).map(|v| length(ctx, v)).transpose()?;
    let r = dpi(&inspect(ctx, doc)?);
    let dx = lx.map_or(Ok(0.0), |l| px_at(&l, r))?;
    let dy = ly.map_or(Ok(0.0), |l| px_at(&l, r))?;
    edit(ctx, doc, "layer.translate", json!({ "layer": id, "dx": whole(dx), "dy": whole(dy) }))?;
    Ok(JsValue::undefined())
}

/// Move layer `id` relative to `rel` (a layer, or the document) by `placement`.
fn move_layer(ctx: &mut Context, doc: DocKey, id: u64, rel: &JsValue, placement: Option<&JsValue>) -> JsResult<()> {
    let (target, position) = destination(ctx, doc, rel, placement)?;
    if target == id {
        return Ok(());
    }
    edit(ctx, doc, "layer.moveTo", json!({ "layer": id, "target": target, "position": position }))?;
    Ok(())
}

/// Where `move(rel, placement)` puts a layer: the layer it goes beside or into, and how.
fn destination(ctx: &mut Context, doc: DocKey, rel: &JsValue, placement: Option<&JsValue>) -> JsResult<(u64, &'static str)> {
    let place = match placement.filter(|p| !p.is_undefined()) {
        Some(p) => text(p, ctx)?,
        None => "ElementPlacement.PLACEBEFORE".to_owned(),
    };
    let d = inspect(ctx, doc)?;
    let (target, position) = if let Ok((rdoc, rid)) = layer_ref(rel) {
        if rdoc != doc {
            return Err(error("a layer can only move within its document"));
        }
        match place.as_str() {
            "ElementPlacement.PLACEBEFORE" => (rid, "above"),
            "ElementPlacement.PLACEAFTER" => (rid, "below"),
            "ElementPlacement.INSIDE" | "ElementPlacement.PLACEATBEGINNING" => (rid, "into"),
            // `into` puts it first in the group: the end is below the group's last layer.
            "ElementPlacement.PLACEATEND" => {
                let last = find(&d["layers"], rid).and_then(|(g, _)| g["children"].as_array().and_then(|k| k.last()).and_then(|l| l["id"].as_u64()));
                match last {
                    Some(l) => (l, "below"),
                    None => (rid, "into"),
                }
            }
            other => return Err(error(format!("{other} is not an ElementPlacement"))),
        }
    } else if doc_id(rel) == Some(doc) {
        let top: Vec<u64> = d["layers"].as_array().map(|l| l.iter().filter_map(|n| n["id"].as_u64()).collect()).unwrap_or_default();
        match place.as_str() {
            "ElementPlacement.PLACEATBEGINNING" => (*top.first().ok_or_else(|| error("the document has no layers"))?, "above"),
            "ElementPlacement.PLACEATEND" => {
                let bottom = *top.last().ok_or_else(|| error("the document has no layers"))?;
                // Nothing goes under the Background.
                if d["backgroundLayer"].as_u64() == Some(bottom) { (bottom, "above") } else { (bottom, "below") }
            }
            other => return Err(error(format!("a layer moves to the document with PLACEATBEGINNING or PLACEATEND, not {other}"))),
        }
    } else {
        return Err(type_error("move() takes a layer or its document"));
    };
    Ok((target, position))
}

fn layer_move(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (doc, id) = layer_ref(this)?;
    let rel = arg(args, 0).ok_or_else(|| type_error("move() needs a layer or document to move relative to"))?.clone();
    move_layer(ctx, doc, id, &rel, args.get(1))?;
    Ok(JsValue::undefined())
}

fn layer_merge(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (doc, id) = layer_ref(this)?;
    // The layer the merge reports, not the active one read after it: the user may have
    // selected another layer by then.
    let merged = edit(ctx, doc, "layer.mergeDown", json!({ "layer": id }))?["layer"].as_u64().ok_or_else(|| error("the merged layer was not reported"))?;
    Ok(layer_object(ctx, doc, merged)?.into())
}

// ---------- type ----------

fn text_ref(this: &JsValue) -> JsResult<TextRef> {
    this.as_object().and_then(|o| o.downcast_ref::<TextRef>().map(|t| *t)).ok_or_else(|| type_error("not a TextItem"))
}

/// The text layer's node and its document's inspection.
fn text_node(ctx: &mut Context, t: TextRef) -> JsResult<(Value, Value)> {
    let d = inspect(ctx, t.doc)?;
    let (n, _) = find(&d["layers"], t.layer).filter(|(n, _)| n["kind"].as_str() == Some("Type")).ok_or_else(|| error("the text layer no longer exists"))?;
    Ok((n, d))
}

fn text_contents(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (n, _) = text_node(ctx, text_ref(this)?)?;
    js(&n["text"]["text"], ctx)
}

fn text_set_contents(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let t = text_ref(this)?;
    let contents = text(args.first().unwrap_or(&JsValue::undefined()), ctx)?;
    edit(ctx, t.doc, "type.edit", json!({ "layer": t.layer, "text": contents }))?;
    Ok(JsValue::undefined())
}

fn text_size(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (n, d) = text_node(ctx, text_ref(this)?)?;
    let pt = n["text"]["sizePt"].as_f64().unwrap_or(12.0);
    let r = dpi(&d);
    let typ = unit_typ(ctx, TYPE)?;
    unit_value::document_length(pt * r / 72.0, r, typ, ctx)
}

/// Apply character settings to all of a text layer's text.
fn restyle(ctx: &mut Context, t: TextRef, style: Value) -> JsResult<JsValue> {
    let (n, _) = text_node(ctx, t)?;
    let len = n["text"]["text"].as_str().map_or(0, |s| s.chars().count());
    let mut run = json!({ "start": 0, "end": len });
    if let (Some(r), Some(s)) = (run.as_object_mut(), style.as_object()) {
        r.extend(s.clone());
    }
    edit(ctx, t.doc, "type.edit", json!({ "layer": t.layer, "runs": [run] }))?;
    Ok(JsValue::undefined())
}

fn text_set_size(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let t = text_ref(this)?;
    // Converted before the resolution is read: a `valueOf` may change it.
    let typ = unit_typ(ctx, TYPE)?;
    let (value, typ) = unit_value::length_of(args.first().unwrap_or(&JsValue::undefined()), typ, ctx)?;
    let r = dpi(&inspect(ctx, t.doc)?);
    let pt = unit_value::length_in_pixels(value, &typ, r)? * 72.0 / r;
    // The sizes the engine takes, so a size accepted here is one it applies.
    let (min, max) = (photocraft_engine::type_cmds::TYPE_SIZE_MIN_PT, photocraft_engine::type_cmds::TYPE_SIZE_MAX_PT);
    if !(min..=max).contains(&pt) {
        return Err(JsNativeError::range().with_message(format!("a type size is {min} to {max} points")).into());
    }
    restyle(ctx, t, json!({ "size": pt }))
}

fn text_font(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (n, _) = text_node(ctx, text_ref(this)?)?;
    js(&n["text"]["font"], ctx)
}

fn text_set_font(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let t = text_ref(this)?;
    let font = text(args.first().unwrap_or(&JsValue::undefined()), ctx)?;
    restyle(ctx, t, json!({ "font": font }))
}

/// Where text `t` is anchored, in pixels (the transform's translation: the baseline start of point
/// text), and its document's resolution. Read from the document's inspection, so reading it
/// doesn't make that document the active one.
fn text_anchor(ctx: &mut Context, t: TextRef) -> JsResult<(f64, f64, f64)> {
    let d = inspect(ctx, t.doc)?;
    let (n, _) = find(&d["layers"], t.layer).ok_or_else(|| error("the layer no longer exists"))?;
    let transform = &n["text"]["transform"];
    let tr = |i: usize| transform.get(i).and_then(Value::as_f64).ok_or_else(|| error("the layer is no longer a text layer"));
    Ok((tr(4)?, tr(5)?, dpi(&d)))
}

fn text_position(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let t = text_ref(this)?;
    let (x, y, r) = text_anchor(ctx, t)?;
    let out = vec![ruler_length(ctx, x, r)?, ruler_length(ctx, y, r)?];
    Ok(JsArray::from_iter(out, ctx).into())
}

fn text_set_position(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let t = text_ref(this)?;
    let p = args.first().and_then(JsValue::as_object).ok_or_else(|| type_error("a position is [x, y]"))?;
    let (px, py) = (p.get(0, ctx)?, p.get(1, ctx)?);
    // Converted first: a `valueOf` may itself move the text or change the resolution, and the
    // move is from where it is once the target is known.
    let (lx, ly) = (length(ctx, &px)?, length(ctx, &py)?);
    let (x, y, r) = text_anchor(ctx, t)?;
    let (nx, ny) = (px_at(&lx, r)?, px_at(&ly, r)?);
    // Text goes only where `artLayers.add` and `type.create` can put it.
    if [nx, ny].iter().any(|v| v.abs() > crate::commands::SIDE) {
        return Err(JsNativeError::range().with_message(format!("a text position is within {} pixels of the origin", crate::commands::SIDE)).into());
    }
    edit(ctx, t.doc, "type.edit", json!({ "layer": t.layer, "move": [nx - x, ny - y] }))?;
    Ok(JsValue::undefined())
}

// ---------- history ----------

fn history_ref(v: &JsValue) -> Option<HistoryRef> {
    v.as_object().and_then(|o| o.downcast_ref::<HistoryRef>().map(|h| *h))
}

/// The History panel rows of a document's inspection, as (state id, name), and the current
/// row's index.
fn rows(d: &Value) -> (Vec<(u64, String)>, usize) {
    let names = |k: &str| d[k].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect::<Vec<_>>()).unwrap_or_default();
    let past = names("history");
    let current = past.len().saturating_sub(1);
    let ids: Vec<u64> = d["historyIds"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    (ids.into_iter().zip(past.into_iter().chain(names("historyRedo"))).collect(), current)
}

/// The row of state `h` now, or an error when an edit has discarded it.
fn row_of(d: &Value, h: HistoryRef) -> JsResult<(usize, String)> {
    let (rows, _) = rows(d);
    rows.into_iter().enumerate().find(|(_, (id, _))| *id == h.state).map(|(i, (_, name))| (i, name)).ok_or_else(|| error("the History state no longer exists"))
}

fn history_name(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let h = history_ref(this).ok_or_else(|| type_error("not a HistoryState"))?;
    let d = inspect_history(ctx, h.doc)?;
    let (_, name) = row_of(&d, h)?;
    Ok(JsValue::from(JsString::from(name.as_str())))
}

fn history_parent(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let h = history_ref(this).ok_or_else(|| type_error("not a HistoryState"))?;
    Ok(document_object(h.doc, ctx).into())
}

fn doc_history_states(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let d = inspect_history(ctx, doc)?;
    let (rows, _) = rows(&d);
    let mut out = Vec::with_capacity(rows.len());
    for (id, _) in rows {
        out.push(JsValue::from(history_object(ctx, doc, id)?));
    }
    Ok(JsArray::from_iter(out, ctx).into())
}

fn doc_active_history(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let d = inspect_history(ctx, doc)?;
    let (rows, current) = rows(&d);
    let (id, _) = rows.get(current).ok_or_else(|| error("the document has no History state"))?;
    Ok(history_object(ctx, doc, *id)?.into())
}

fn doc_set_active_history(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let h = args
        .first()
        .and_then(history_ref)
        .filter(|h| h.doc == doc)
        .ok_or_else(|| type_error("activeHistoryState takes one of this document's historyStates"))?;
    let d = inspect_history(ctx, doc)?;
    let (row, _) = row_of(&d, h)?;
    edit(ctx, doc, "history.selectState", json!({ "state": row }))?;
    Ok(JsValue::undefined())
}

// ---------- the selection ----------

fn selection_doc(this: &JsValue) -> JsResult<DocKey> {
    this.as_object().and_then(|o| o.downcast_ref::<SelectionRef>().map(|s| s.0)).ok_or_else(|| type_error("not a Selection"))
}

fn doc_selection(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let key = (SELECTION, doc.id, doc.serial, 0);
    if let Some(o) = cached(ctx, key) {
        return Ok(o.into());
    }
    let (_, _, _, proto) = protos(ctx)?;
    let obj = JsObject::from_proto_and_data(Some(proto), SelectionRef(doc));
    cache(ctx, key, &obj);
    Ok(obj.into())
}

fn sel_simple(this: &JsValue, ctx: &mut Context, id: &str) -> JsResult<JsValue> {
    let doc = selection_doc(this)?;
    edit(ctx, doc, id, json!({}))?;
    Ok(JsValue::undefined())
}

fn sel_all(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    sel_simple(this, ctx, "select.all")
}

fn sel_deselect(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    sel_simple(this, ctx, "select.deselect")
}

/// `selection.select(region)`: the rectangle around the region's `[x, y]` points (replacing
/// the selection; other selection types and shapes are not supported yet).
fn sel_select(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = selection_doc(this)?;
    if let Some(t) = arg(args, 1) {
        let t = text(t, ctx)?;
        if t != "SelectionType.REPLACE" {
            return Err(error(format!("{t} is not supported yet; only SelectionType.REPLACE")));
        }
    }
    let region = args.first().and_then(JsValue::as_object).ok_or_else(|| type_error("select() takes an array of [x, y] points"))?;
    let len = region.get(js_string!("length"), ctx)?.to_length(ctx)?;
    if len == 0 || len > 1 << 16 {
        return Err(type_error("select() takes 1 to 65536 points"));
    }
    // Every point converted before the resolution is read: a `valueOf` may change it.
    let mut points = Vec::with_capacity(len as usize);
    for i in 0..len {
        let p = region.get(i, ctx)?;
        let p = p.as_object().ok_or_else(|| type_error("each point is [x, y]"))?;
        let (px, py) = (p.get(0, ctx)?, p.get(1, ctx)?);
        points.push((length(ctx, &px)?, length(ctx, &py)?));
    }
    let d = inspect(ctx, doc)?;
    let r = dpi(&d);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (lx, ly) in &points {
        let (x, y) = (px_at(lx, r)?, px_at(ly, r)?);
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let (x, y) = (whole(x0), whole(y0));
    let (w, h) = (whole(x1) - x, whole(y1) - y);
    if w < 1 || h < 1 {
        return Err(error("the region encloses no pixels"));
    }
    edit(ctx, doc, "select.rect", json!({ "x": x, "y": y, "width": w, "height": h }))?;
    Ok(JsValue::undefined())
}

fn sel_bounds(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = selection_doc(this)?;
    let d = inspect(ctx, doc)?;
    if d["selectionBounds"].is_null() {
        return Err(error("there is no selection"));
    }
    bounds(ctx, &d["selectionBounds"], dpi(&d))
}

// ---------- documents ----------

fn doc_active_layer(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let d = inspect(ctx, doc)?;
    let id = d["activeLayer"].as_u64().ok_or_else(|| error("the document has no active layer"))?;
    Ok(layer_object(ctx, doc, id)?.into())
}

fn doc_set_active_layer(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let (ldoc, id) = args.first().map(layer_ref).transpose()?.ok_or_else(|| type_error("activeLayer takes a layer"))?;
    if ldoc != doc {
        return Err(error("activeLayer takes a layer of this document"));
    }
    edit(ctx, doc, "layer.select", json!({ "layer": id }))?;
    Ok(JsValue::undefined())
}

fn doc_layers(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    collection(ctx, doc_key(this)?, None, ALL)
}
fn doc_art_layers(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    collection(ctx, doc_key(this)?, None, ART)
}
fn doc_layer_sets(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    collection(ctx, doc_key(this)?, None, SETS)
}

/// `close(saveOptions)`: closes without saving when told not to save or when nothing changed.
/// A changed document is never thrown away unasked: save it with `saveAs` first.
fn doc_close(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let opt = match arg(args, 0) {
        Some(v) => text(v, ctx)?,
        None => "SaveOptions.PROMPTTOSAVECHANGES".to_owned(),
    };
    let discard = opt == "SaveOptions.DONOTSAVECHANGES";
    let (_, entry) = resolve(doc, ctx)?;
    if !discard && entry["dirty"].as_bool().unwrap_or(true) {
        return Err(error("the document has unsaved changes: save it with saveAs() first, or close it with SaveOptions.DONOTSAVECHANGES"));
    }
    // "ask" again in the command: changes made since the check above are not thrown away.
    edit(ctx, doc, "file.close", json!({ "save": if discard { "discard" } else { "ask" } }))?;
    Ok(JsValue::undefined())
}

/// The document a command made, from its result (`id` and `serial`): not whichever is active
/// by the time the script asks, which the user may have changed.
fn made_document(ctx: &mut Context, made: &Value) -> JsResult<JsValue> {
    let key = entry_id(made).map_err(|_| error("the new document was not reported"))?;
    Ok(document_object(key, ctx).into())
}

fn doc_duplicate(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let (_, entry) = resolve(doc, ctx)?;
    let name = match arg(args, 0) {
        Some(v) => text(v, ctx)?,
        None => format!("{} copy", entry["name"].as_str().unwrap_or("Untitled")),
    };
    let merged = arg(args, 1).is_some_and(JsValue::to_boolean);
    let made = edit(ctx, doc, "image.duplicate", json!({ "name": name, "mergedOnly": merged }))?;
    made_document(ctx, &made)
}

fn doc_simple(this: &JsValue, ctx: &mut Context, id: &str) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    edit(ctx, doc, id, json!({}))?;
    Ok(JsValue::undefined())
}

/// `doc.suspendHistory(name, code)`: run `code` as one History step. A whole run is one step
/// already, so this runs `code`, in the global scope as Photoshop does.
fn doc_suspend_history(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    if crate::natives::doc_id(this).is_none() {
        return Err(error("suspendHistory: not a Document"));
    }
    let code = args.get(1).cloned().unwrap_or_default().to_string(ctx)?;
    // The realm's own `eval`, not whatever the script put in its place.
    let eval = ctx.intrinsics().objects().eval();
    eval.call(&JsValue::undefined(), &[code.into()], ctx)?;
    Ok(JsValue::undefined())
}

fn doc_flatten(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    doc_simple(this, ctx, "layer.flattenImage")
}

fn doc_merge_visible(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    doc_simple(this, ctx, "layer.mergeVisible")
}

const RESAMPLES: &[(&str, &str)] = &[
    ("ResampleMethod.NEARESTNEIGHBOR", "nearest"),
    ("ResampleMethod.BILINEAR", "bilinear"),
    ("ResampleMethod.BICUBIC", "bicubic"),
    ("ResampleMethod.BICUBICSHARPER", "bicubicSharper"),
    ("ResampleMethod.BICUBICSMOOTHER", "bicubicSmoother"),
    ("ResampleMethod.PRESERVEDETAILS", "preserveDetails"),
    ("ResampleMethod.AUTOMATIC", "automatic"),
];

/// `resizeImage(width, height, resolution, resampleMethod)`: a side left out keeps the
/// proportions; sizes are unit values or numbers in the ruler unit.
fn doc_resize_image(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let res = arg(args, 2).map(|v| v.to_number(ctx)).transpose()?;
    // A NaN is in no range either.
    if res.is_some_and(|r| !(1.0..=30_000.0).contains(&r)) {
        return Err(JsNativeError::range().with_message("a resolution is 1 to 30000 pixels per inch").into());
    }
    let w = arg(args, 0).map(|v| length(ctx, v)).transpose()?;
    let h = arg(args, 1).map(|v| length(ctx, v)).transpose()?;
    let resample = match arg(args, 3) {
        Some(m) => {
            let m = text(m, ctx)?;
            Some(RESAMPLES.iter().find(|(e, _)| *e == m).map(|(_, n)| *n).ok_or_else(|| error(format!("{m} is not a ResampleMethod PhotoCraft has")))?)
        }
        None => None,
    };
    // Read once every argument is converted: a `valueOf` or `toString` may itself have resized
    // the document or changed its resolution. Sizes in physical units mean pixels at the
    // resolution the image will have; the proportions kept are the document's now.
    let d = inspect(ctx, doc)?;
    let r = res.unwrap_or_else(|| dpi(&d));
    let w = w.map(|l| px_at(&l, r)).transpose()?;
    let h = h.map(|l| px_at(&l, r)).transpose()?;
    let (w0, h0) = (d["width"].as_f64().unwrap_or(1.0), d["height"].as_f64().unwrap_or(1.0));
    let (w, h) = match (w, h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, h0 * w / w0),
        (None, Some(h)) => (w0 * h / h0, h),
        (None, None) => (w0, h0),
    };
    let mut p = json!({ "width": side(w)?, "height": side(h)? });
    if let Some(res) = res {
        p["resolution"] = json!(res);
    }
    if let Some(name) = resample {
        p["resample"] = json!(name);
    }
    edit(ctx, doc, "image.imageSize", p)?;
    Ok(JsValue::undefined())
}

const ANCHOR_NAMES: &[(&str, &str)] = &[
    ("AnchorPosition.TOPLEFT", "topLeft"),
    ("AnchorPosition.TOPCENTER", "top"),
    ("AnchorPosition.TOPRIGHT", "topRight"),
    ("AnchorPosition.MIDDLELEFT", "left"),
    ("AnchorPosition.MIDDLECENTER", "center"),
    ("AnchorPosition.MIDDLERIGHT", "right"),
    ("AnchorPosition.BOTTOMLEFT", "bottomLeft"),
    ("AnchorPosition.BOTTOMCENTER", "bottom"),
    ("AnchorPosition.BOTTOMRIGHT", "bottomRight"),
];

fn doc_resize_canvas(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let w = arg(args, 0).map(|v| length(ctx, v)).transpose()?;
    let h = arg(args, 1).map(|v| length(ctx, v)).transpose()?;
    let anchor = match arg(args, 2) {
        Some(a) => {
            let a = text(a, ctx)?;
            ANCHOR_NAMES.iter().find(|(e, _)| *e == a).map(|(_, n)| *n).ok_or_else(|| error(format!("{a} is not an AnchorPosition")))?
        }
        None => "center",
    };
    // Read once every argument is converted (a `valueOf` or `toString` may resize or change the
    // resolution): the lengths at the resolution now, a side left out as it is now.
    let d = inspect(ctx, doc)?;
    let r = dpi(&d);
    let w = match w {
        Some(l) => px_at(&l, r)?,
        None => d["width"].as_f64().unwrap_or(1.0),
    };
    let h = match h {
        Some(l) => px_at(&l, r)?,
        None => d["height"].as_f64().unwrap_or(1.0),
    };
    edit(ctx, doc, "image.canvasSize", json!({ "width": side(w)?, "height": side(h)?, "anchor": anchor }))?;
    Ok(JsValue::undefined())
}

/// `crop([left, top, right, bottom])`.
fn doc_crop(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let b = args.first().and_then(JsValue::as_object).ok_or_else(|| type_error("crop() takes [left, top, right, bottom]"))?;
    // Converted before the resolution is read: a `valueOf` may change it.
    let mut lengths = Vec::with_capacity(4);
    for i in 0..4u32 {
        let e = b.get(i, ctx)?;
        lengths.push(length(ctx, &e)?);
    }
    let d = inspect(ctx, doc)?;
    let r = dpi(&d);
    let mut v = [0.0; 4];
    for (slot, l) in v.iter_mut().zip(&lengths) {
        *slot = px_at(l, r)?;
    }
    let [l, t, rt, bt] = v;
    let (w, h) = ((rt - l).round(), (bt - t).round());
    if w < 1.0 || h < 1.0 {
        return Err(error("the crop bounds enclose no pixels"));
    }
    edit(ctx, doc, "image.crop", json!({ "x": l.round(), "y": t.round(), "width": w, "height": h }))?;
    Ok(JsValue::undefined())
}

/// `trim(type, top, left, bottom, right)`.
fn doc_trim(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let based = match arg(args, 0) {
        Some(t) => match text(t, ctx)?.as_str() {
            "TrimType.TRANSPARENT" => "transparent",
            "TrimType.TOPLEFT" => "topLeft",
            "TrimType.BOTTOMRIGHT" => "bottomRight",
            other => return Err(error(format!("{other} is not a TrimType"))),
        },
        None => "transparent",
    };
    let side = |i: usize| arg(args, i).is_none_or(JsValue::to_boolean);
    let p = json!({ "basedOn": based, "top": side(1), "left": side(2), "bottom": side(3), "right": side(4) });
    edit(ctx, doc, "image.trim", p)?;
    Ok(JsValue::undefined())
}

/// A path from a `File` object (its `fsName`) or text.
fn path_of(v: &JsValue, ctx: &mut Context) -> JsResult<String> {
    let p = match v.as_object() {
        Some(o) => o.get(js_string!("fsName"), ctx)?,
        None => v.clone(),
    };
    let p = text(&p, ctx)?;
    if p.is_empty() { Err(type_error("a file to save to is needed")) } else { Ok(p) }
}

/// `path` for a save in a format whose extensions are `exts`: as it is when it has one of them,
/// with the first added when it has none. Another extension is an error: the file the user chose
/// is the one written, and its name says its format.
fn format_path(path: &str, exts: &[&str], kind: &str) -> JsResult<String> {
    let cut = path.rfind(['/', '\\']).map_or(0, |i| i + 1);
    let name = path.get(cut..).unwrap_or("");
    match name.rfind('.').filter(|i| *i > 0).and_then(|i| name.get(i + 1..)) {
        None => Ok(format!("{path}.{}", exts.first().copied().unwrap_or("png"))),
        Some(ext) if exts.iter().any(|e| e.eq_ignore_ascii_case(ext)) => Ok(path.to_owned()),
        Some(ext) => Err(error(format!("{name} is a .{ext} file; {kind} save .{}", exts.join(", .")))),
    }
}

fn opt(o: &JsObject, key: &str, ctx: &mut Context) -> JsResult<Option<JsValue>> {
    let v = o.get(JsString::from(key), ctx)?;
    Ok(if v.is_undefined() || v.is_null() { None } else { Some(v) })
}

/// `saveAs(file, options, asCopy)`: writes a copy in the options' format (JPEG, PNG, PSD, TIFF;
/// the extension follows it), to a file or folder the user chose in this run. The document
/// keeps its own file and name.
fn doc_save_as(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let mut path = path_of(args.first().unwrap_or(&JsValue::undefined()), ctx)?;
    let mut p = json!({});
    if let Some(o) = arg(args, 1).and_then(JsValue::as_object) {
        let kind = opt(&o, "typename", ctx)?.map(|v| text(&v, ctx)).transpose()?.unwrap_or_default();
        let exts: &[&str] = match kind.as_str() {
            "JPEGSaveOptions" => &["jpg", "jpeg", "jpe"],
            "PNGSaveOptions" => &["png"],
            "PhotoshopSaveOptions" => &["psd"],
            "TiffSaveOptions" => &["tif", "tiff"],
            other => return Err(error(format!("saving with {other} options is not supported yet"))),
        };
        path = format_path(&path, exts, &kind)?;
        if kind == "JPEGSaveOptions"
            && let Some(q) = opt(&o, "quality", ctx)?
        {
            p["quality"] = json!(q.to_number(ctx)?.round().clamp(0.0, 12.0));
        }
        if kind == "PhotoshopSaveOptions"
            && let Some(l) = opt(&o, "layers", ctx)?
        {
            p["layers"] = json!(l.to_boolean());
        }
        if kind == "TiffSaveOptions" {
            if let Some(l) = opt(&o, "layers", ctx)? {
                p["tiffLayers"] = json!(l.to_boolean());
            }
            if let Some(c) = opt(&o, "imageCompression", ctx)? {
                p["tiffCompression"] = json!(match text(&c, ctx)?.as_str() {
                    "TIFFEncoding.TIFFLZW" => "lzw",
                    "TIFFEncoding.TIFFZIP" => "zip",
                    _ => "none",
                });
            }
        }
    }
    p["path"] = json!(path);
    edit(ctx, doc, "file.saveACopy", p)?;
    Ok(JsValue::undefined())
}

/// `exportDocument(file, ExportType.SAVEFORWEB, options)`.
fn doc_export(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let doc = doc_key(this)?;
    let path = path_of(args.first().unwrap_or(&JsValue::undefined()), ctx)?;
    if let Some(t) = arg(args, 1) {
        let t = text(t, ctx)?;
        if t != "ExportType.SAVEFORWEB" {
            return Err(error(format!("{t} is not supported yet; only ExportType.SAVEFORWEB")));
        }
    }
    let mut p = json!({ "path": path, "format": "gif" });
    if let Some(o) = arg(args, 2).and_then(JsValue::as_object) {
        let png8 = opt(&o, "PNG8", ctx)?.is_some_and(|v| v.to_boolean());
        if let Some(f) = opt(&o, "format", ctx)? {
            p["format"] = json!(match text(&f, ctx)?.as_str() {
                "SaveDocumentType.JPEG" => "jpeg",
                "SaveDocumentType.PNG" if png8 => "png8",
                "SaveDocumentType.PNG" => "png24",
                "SaveDocumentType.COMPUSERVEGIF" => "gif",
                other => return Err(error(format!("{other} can't be exported for the web"))),
            });
        }
        if let Some(q) = opt(&o, "quality", ctx)? {
            p["quality"] = json!(q.to_number(ctx)?.clamp(0.0, 100.0));
        }
        if let Some(c) = opt(&o, "colors", ctx)? {
            p["colors"] = json!(c.to_number(ctx)?.round().clamp(2.0, 256.0));
        }
        if let Some(t) = opt(&o, "transparency", ctx)? {
            p["transparency"] = json!(t.to_boolean());
        }
    }
    edit(ctx, doc, "file.export.saveForWebLegacy", p)?;
    Ok(JsValue::undefined())
}

fn doc_save(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Err(error("save() is not available to scripts yet; use saveAs() with a file chosen by File.saveDialog()"))
}

/// The members a `Document` object gets: (name, getter, setter).
pub(crate) const DOC_ACCESSORS: &[(&str, Native, Option<Native>)] = &[
    ("activeLayer", doc_active_layer, Some(doc_set_active_layer)),
    ("layers", doc_layers, None),
    ("artLayers", doc_art_layers, None),
    ("layerSets", doc_layer_sets, None),
    ("historyStates", doc_history_states, None),
    ("activeHistoryState", doc_active_history, Some(doc_set_active_history)),
    ("selection", doc_selection, None),
];

/// The methods a `Document` object gets.
pub(crate) const DOC_METHODS: &[(&str, Native, usize)] = &[
    ("close", doc_close, 1),
    ("duplicate", doc_duplicate, 2),
    ("flatten", doc_flatten, 0),
    ("mergeVisibleLayers", doc_merge_visible, 0),
    ("resizeImage", doc_resize_image, 4),
    ("resizeCanvas", doc_resize_canvas, 3),
    ("crop", doc_crop, 1),
    ("trim", doc_trim, 5),
    ("saveAs", doc_save_as, 3),
    ("save", doc_save, 0),
    ("exportDocument", doc_export, 3),
    ("suspendHistory", doc_suspend_history, 2),
];

// ---------- app ----------

const MODES: &[(&str, &str)] = &[
    ("NewDocumentMode.RGB", "rgb"),
    ("NewDocumentMode.GRAYSCALE", "grayscale"),
    ("NewDocumentMode.CMYK", "cmyk"),
    ("NewDocumentMode.LAB", "lab"),
    ("NewDocumentMode.BITMAP", "bitmap"),
];

/// `app.documents.add(width, height, resolution, name, mode, initialFill)`.
pub(crate) fn documents_add(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let res = match arg(args, 2) {
        Some(r) => r.to_number(ctx)?,
        None => 72.0,
    };
    if !(1.0..=30_000.0).contains(&res) {
        return Err(JsNativeError::range().with_message("a resolution is 1 to 30000 pixels per inch").into());
    }
    let side = |ctx: &mut Context, i: usize, d: f64| -> JsResult<f64> {
        match arg(args, i) {
            Some(v) => Ok(to_px(ctx, v, res)?.round()),
            None => Ok(d),
        }
    };
    let (w, h) = (side(ctx, 0, 504.0)?, side(ctx, 1, 360.0)?);
    if !(1.0..=300_000.0).contains(&w) || !(1.0..=300_000.0).contains(&h) {
        return Err(JsNativeError::range().with_message("a document side is 1 to 300000 pixels").into());
    }
    let mut p = json!({ "width": w, "height": h, "resolution": res });
    if let Some(n) = arg(args, 3) {
        p["name"] = json!(text(n, ctx)?);
    }
    if let Some(m) = arg(args, 4) {
        let m = text(m, ctx)?;
        p["mode"] = json!(MODES.iter().find(|(e, _)| *e == m).map(|(_, n)| *n).ok_or_else(|| error(format!("{m} is not a NewDocumentMode")))?);
    }
    if let Some(f) = arg(args, 5) {
        p["background"] = json!(match text(f, ctx)?.as_str() {
            "DocumentFill.WHITE" => "white",
            "DocumentFill.BACKGROUNDCOLOR" => "backgroundColor",
            "DocumentFill.TRANSPARENT" => "transparent",
            other => return Err(error(format!("{other} is not a DocumentFill"))),
        });
    }
    let made = command(ctx, "file.new", p)?;
    made_document(ctx, &made)
}

/// `app.documents.getByName(name)`.
pub(crate) fn documents_get_by_name(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let name = text(args.first().unwrap_or(&JsValue::undefined()), ctx)?;
    let s = session_inspect(ctx)?;
    let found = s["documents"].as_array().and_then(|docs| docs.iter().find(|d| d["name"].as_str() == Some(name.as_str())).cloned());
    match found {
        Some(e) => {
            let key = entry_id(&e)?;
            Ok(document_object(key, ctx).into())
        }
        None => Err(error_with_number(ctx, "No such element", 1302)),
    }
}

/// An options class: `new JPEGSaveOptions()` is an object with its `typename` and defaults.
fn options_object(ctx: &mut Context, typename: &str, defaults: &[(&str, JsValue)]) -> JsValue {
    let mut o = ObjectInitializer::new(ctx);
    o.property(js_string!("typename"), JsString::from(typename), Attribute::all());
    for (k, v) in defaults {
        o.property(JsString::from(*k), v.clone(), Attribute::all());
    }
    o.build().into()
}

fn jpeg_options(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    Ok(options_object(ctx, "JPEGSaveOptions", &[("quality", JsValue::from(8))]))
}
fn png_options(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    Ok(options_object(ctx, "PNGSaveOptions", &[]))
}
fn psd_options(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    Ok(options_object(ctx, "PhotoshopSaveOptions", &[("layers", JsValue::from(true))]))
}
fn tiff_options(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    Ok(options_object(ctx, "TiffSaveOptions", &[("layers", JsValue::from(false)), ("imageCompression", JsValue::from(js_string!("TIFFEncoding.NONE")))]))
}
fn web_options(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let defaults = [
        ("format", JsValue::from(js_string!("SaveDocumentType.COMPUSERVEGIF"))),
        ("PNG8", JsValue::from(false)),
        ("quality", JsValue::from(60)),
        ("colors", JsValue::from(256)),
        ("transparency", JsValue::from(true)),
    ];
    Ok(options_object(ctx, "ExportOptionsSaveForWeb", &defaults))
}

// ---------- install ----------

fn proto(ctx: &mut Context, accessors: &[(&str, Native, Option<Native>)], methods: &[(&str, Native, usize)]) -> JsObject {
    let fns: Vec<(&str, _, Option<_>)> = accessors.iter().map(|(n, g, s)| (*n, getter(ctx, *g), s.map(|s| getter(ctx, s)))).collect();
    let mut o = ObjectInitializer::new(ctx);
    for (n, g, s) in fns {
        o.accessor(JsString::from(n), Some(g), s, Attribute::CONFIGURABLE);
    }
    for (n, f, len) in methods {
        o.function(NativeFunction::from_fn_ptr(*f), JsString::from(*n), *len);
    }
    o.build()
}

/// Install the enumerations, option classes, prototypes and `app.preferences`.
pub(crate) fn install(ctx: &mut Context, app: &JsObject) -> JsResult<()> {
    for (name, values) in ENUMS {
        let mut o = ObjectInitializer::new(ctx);
        for v in *values {
            o.property(JsString::from(*v), JsString::from(format!("{name}.{v}")), Attribute::ENUMERABLE);
        }
        let o = o.build();
        ctx.register_global_property(JsString::from(*name), o, Attribute::WRITABLE | Attribute::CONFIGURABLE)?;
    }
    let mut blend = ObjectInitializer::new(ctx);
    for l in BLENDS {
        let e = blend_enum(l);
        let key = e.trim_start_matches("BlendMode.").to_owned();
        blend.property(JsString::from(key.as_str()), JsString::from(e.as_str()), Attribute::ENUMERABLE);
    }
    let blend = blend.build();
    ctx.register_global_property(js_string!("BlendMode"), blend, Attribute::WRITABLE | Attribute::CONFIGURABLE)?;

    let classes: [(&str, Native); 5] = [
        ("JPEGSaveOptions", jpeg_options),
        ("PNGSaveOptions", png_options),
        ("PhotoshopSaveOptions", psd_options),
        ("TiffSaveOptions", tiff_options),
        ("ExportOptionsSaveForWeb", web_options),
    ];
    for (name, f) in classes {
        let ctor = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(f)).name(JsString::from(name)).length(0).constructor(true).build();
        ctx.register_global_property(JsString::from(name), ctor, Attribute::WRITABLE | Attribute::CONFIGURABLE)?;
    }

    let layer = proto(
        ctx,
        &[
            ("name", layer_name, Some(layer_set_name)),
            ("visible", layer_visible, Some(layer_set_visible)),
            ("opacity", layer_opacity, Some(layer_set_opacity)),
            ("fillOpacity", layer_fill, Some(layer_set_fill)),
            ("blendMode", layer_blend, Some(layer_set_blend)),
            ("kind", layer_kind, Some(layer_set_kind)),
            ("typename", layer_typename, None),
            ("bounds", layer_bounds, None),
            ("isBackgroundLayer", layer_is_background, None),
            ("allLocked", lock_all, Some(set_lock_all)),
            ("pixelsLocked", lock_pixels, Some(set_lock_pixels)),
            ("positionLocked", lock_position, Some(set_lock_position)),
            ("transparentPixelsLocked", lock_transparency, Some(set_lock_transparency)),
            ("parent", layer_parent, None),
            ("id", layer_id, None),
            ("layers", layer_layers, None),
            ("artLayers", layer_art_layers, None),
            ("layerSets", layer_layer_sets, None),
            ("textItem", layer_text_item, None),
        ],
        &[
            ("remove", layer_remove, 0),
            ("duplicate", layer_duplicate, 2),
            ("translate", layer_translate, 2),
            ("move", layer_move, 2),
            ("merge", layer_merge, 0),
            ("toString", layer_to_string, 0),
        ],
    );
    let text_proto = proto(
        ctx,
        &[
            ("contents", text_contents, Some(text_set_contents)),
            ("size", text_size, Some(text_set_size)),
            ("font", text_font, Some(text_set_font)),
            ("position", text_position, Some(text_set_position)),
        ],
        &[],
    );
    let history = proto(ctx, &[("name", history_name, None), ("parent", history_parent, None)], &[]);
    history.set(js_string!("snapshot"), false, true, ctx)?;
    let selection = proto(ctx, &[("bounds", sel_bounds, None)], &[("selectAll", sel_all, 0), ("deselect", sel_deselect, 0), ("select", sel_select, 4)]);
    let realm = ctx.realm().clone();
    realm.host_defined_mut().insert(Protos { layer, text: text_proto, history, selection });

    let prefs = proto(ctx, &[("rulerUnits", rulers_get, Some(rulers_set)), ("typeUnits", type_units_get, Some(type_units_set))], &[]);
    app.set(js_string!("preferences"), prefs, true, ctx)?;
    // Scripts set this to keep dialogs from opening; PhotoCraft opens none of its own for them.
    app.set(js_string!("displayDialogs"), js_string!("DialogModes.ALL"), true, ctx)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_modes_round_trip_through_their_constants() {
        for l in BLENDS {
            assert_eq!(blend_label(&blend_enum(l)), Some(*l), "{l}");
        }
        assert_eq!(blend_enum("Linear Dodge (Add)"), "BlendMode.LINEARDODGE");
        assert_eq!(blend_enum("Pass Through"), "BlendMode.PASSTHROUGH");
    }

    #[test]
    fn layers_are_found_anywhere_in_the_tree() {
        let tree = json!([{"id": 3, "children": [{"id": 5}, {"id": 6, "children": [{"id": 9}]}]}, {"id": 2}]);
        assert_eq!(find(&tree, 9).map(|(n, p)| (n["id"].clone(), p)), Some((json!(9), Some(6))));
        assert_eq!(find(&tree, 2).map(|(_, p)| p), Some(None));
        assert!(find(&tree, 7).is_none());
    }

    #[test]
    fn a_save_keeps_the_chosen_name_and_adds_a_missing_extension() {
        let jpeg = &["jpg", "jpeg", "jpe"];
        assert_eq!(format_path("/a/b/out.JPEG", jpeg, "JPEGSaveOptions").ok().as_deref(), Some("/a/b/out.JPEG"));
        assert_eq!(format_path("C:\\a\\out", &["psd"], "PhotoshopSaveOptions").ok().as_deref(), Some("C:\\a\\out.psd"));
        assert_eq!(format_path("/a.b/.hidden", &["tif"], "TiffSaveOptions").ok().as_deref(), Some("/a.b/.hidden.tif"));
        assert!(format_path("/a/b/out.png", jpeg, "JPEGSaveOptions").is_err());
    }
}
