//! The global objects scripts see. All of them are Rust natives; no JavaScript source is
//! evaluated besides the user's script.

use std::collections::HashMap;

use boa_engine::object::builtins::{JsArray, JsFunction};
use boa_engine::object::{FunctionObjectBuilder, ObjectInitializer};
use boa_engine::property::{Attribute, PropertyDescriptor};
use boa_engine::{Context, Finalize, JsData, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, Trace, js_string};
use serde_json::{Value, json};

use crate::runner::{Shared, Stop};
use crate::{APP_VERSION, ModalKind, abort, value};

pub(crate) type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

/// Which document a handle means: its session document id plus its open serial (`id` and
/// `serial` in `session.inspect`). A closed document's id can go to a newer document; the serial
/// is never reused, so a stale handle can't take that document over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DocKey {
    pub(crate) id: u64,
    pub(crate) serial: u64,
}

/// The document a `Document` object stands for. Tabs can move or close while the run waits
/// at a dialog, so the key is looked up again (by serial) on every access. One wrapper is
/// cached per id and serial in the current context so identity comparisons work.
#[derive(Debug, Clone, Copy, Trace, Finalize, JsData)]
// Sound: DocRef holds only integers, no GC pointers.
#[boa_gc(empty_trace)]
struct DocRef(DocKey);

/// One Document wrapper per id and serial for the current context. Kept in the realm's host
/// data, which scripts cannot reach, and traced so the wrappers stay alive.
#[derive(Default, Trace, Finalize, JsData)]
struct DocCache(HashMap<(u64, u64), JsObject>);

fn shared(ctx: &Context) -> JsResult<Shared> {
    ctx.get_data::<Shared>().cloned().ok_or_else(|| JsNativeError::error().with_message("script host is missing").into())
}

pub(crate) fn error(message: impl Into<String>) -> JsError {
    JsNativeError::error().with_message(message.into()).into()
}

/// A catchable `Error` with a `number` existing scripts read, e.g. 1302 when no document is active.
pub(crate) fn error_with_number(ctx: &mut Context, message: impl Into<String>, number: i32) -> JsError {
    let obj = JsNativeError::error().with_message(message.into()).into_opaque(ctx);
    // Defined, not assigned: a setter on `Error.prototype` must not take the value or throw.
    match obj.create_data_property_or_throw(js_string!("number"), number, ctx) {
        Ok(_) => JsError::from_opaque(obj.into()),
        Err(e) => e,
    }
}

/// `message` for a command the host refused, or `e` itself when it stops the run: a stop must
/// stay uncatchable, never become an `Error` the script could catch and go on after.
fn host_failure(ctx: &Context, e: JsError, message: &str) -> JsError {
    match shared(ctx) {
        Ok(s) if s.borrow().stop.is_none() => error(message),
        _ => e,
    }
}

/// End the run with an uncatchable error, remembering why.
fn stop(ctx: &mut Context, why: Stop) -> JsError {
    if let Ok(s) = shared(ctx) {
        s.borrow_mut().stop = Some(why);
    }
    abort::uncatchable()
}

/// Charge the budget once, stopping the run when it is spent or when a stop is already
/// pending. A stop the script swallowed (only possible if the abort ever became catchable) must
/// not reach the host: every later checkpoint stops again.
pub(crate) fn checkpoint(ctx: &mut Context) -> JsResult<()> {
    let s = shared(ctx)?;
    let go = {
        let mut st = s.borrow_mut();
        st.stop.is_none() && st.budget.charge()
    };
    if go {
        return Ok(());
    }
    let why = s.borrow_mut().stop.take().unwrap_or(Stop::TimeLimit);
    Err(stop(ctx, why))
}

/// Count one host call, with the output it writes, against the run's bounds: past them the run
/// stops here.
fn count(ctx: &mut Context, output: usize) -> JsResult<()> {
    let counted = shared(ctx)?.borrow_mut().bounds.call(output);
    counted.map_err(|m| stop(ctx, Stop::Fault(m)))
}

/// Run a session command. Only the commands in [`crate::commands`] with the parameters they
/// allow get through, the same check the script host's supervisor applies.
pub(crate) fn command(ctx: &mut Context, id: &str, params: Value) -> JsResult<Value> {
    crate::commands::check_command(id, &params).map_err(error)?;
    checkpoint(ctx)?;
    count(ctx, 0)?;
    let result = shared(ctx)?.borrow_mut().host.execute(id, params);
    // A slow command counts too: charge again once it returned.
    checkpoint(ctx)?;
    result.map_err(|e| error(e.0))
}

/// Open a modal dialog of `kind` with the script's arguments as text, and return the user's
/// answer, waited for in the host call.
fn modal(ctx: &mut Context, kind: ModalKind, args: &[JsValue]) -> JsResult<JsValue> {
    let args = Value::Array(value::texts(args, ctx)?.into_iter().map(Value::String).collect());
    checkpoint(ctx)?;
    count(ctx, 0)?;
    let s = shared(ctx)?;
    // Time at an open dialog is not the script's: stop the budget while the host waits.
    s.borrow_mut().budget.pause();
    let answer = s.borrow_mut().host.modal(kind, &args);
    s.borrow_mut().budget.resume();
    let answer = answer.map_err(|e| stop(ctx, Stop::Fault(e.0)))?;
    match (kind, answer) {
        (ModalKind::Confirm, v) => Ok(JsValue::from(v.as_bool().unwrap_or(false))),
        (ModalKind::Prompt, Value::String(s)) => Ok(JsValue::from(JsString::from(s.as_str()))),
        (ModalKind::Prompt, _) => Ok(JsValue::null()),
        _ => Ok(JsValue::undefined()),
    }
}

fn alert(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    modal(ctx, ModalKind::Alert, args)
}

fn confirm(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    modal(ctx, ModalKind::Confirm, args)
}

fn prompt(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    modal(ctx, ModalKind::Prompt, args)
}

fn writeln(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let line = value::texts(args, ctx)?.concat();
    checkpoint(ctx)?;
    count(ctx, line.len())?;
    shared(ctx)?.borrow_mut().host.write_line(&line);
    // The write's own time counts too, also when it is the script's last act.
    checkpoint(ctx)?;
    Ok(JsValue::undefined())
}

fn app_name(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let name = shared(ctx)?.borrow().app_name.clone();
    Ok(JsValue::from(JsString::from(name.as_str())))
}

pub(crate) fn session_inspect(ctx: &mut Context) -> JsResult<Value> {
    command(ctx, "session.inspect", json!({}))
}

/// Most document objects one context keeps.
pub(crate) const MAX_OBJECTS: usize = 1 << 16;

pub(crate) fn document_object(id: DocKey, ctx: &mut Context) -> JsObject {
    let key = (id.id, id.serial);
    let cached = ctx.realm().host_defined().get::<DocCache>().and_then(|c| c.0.get(&key).cloned());
    if let Some(obj) = cached {
        return obj;
    }
    let getters: [(JsString, Native); 3] = [(js_string!("name"), doc_name), (js_string!("resolution"), doc_resolution), (js_string!("saved"), doc_saved)];
    let accessors: Vec<(JsString, JsFunction)> = getters.into_iter().map(|(k, f)| (k, getter(ctx, f))).collect();
    let mut init = ObjectInitializer::with_native_data(DocRef(id), ctx);
    for (k, g) in accessors {
        init.accessor(k, Some(g), None, Attribute::ENUMERABLE | Attribute::CONFIGURABLE);
    }
    init.function(NativeFunction::from_fn_ptr(doc_to_string), js_string!("toString"), 0);
    let obj = init.build();
    let realm = ctx.realm().clone();
    let mut data = realm.host_defined_mut();
    if data.get::<DocCache>().is_none() {
        data.insert(DocCache::default());
    }
    // Bounded: past the limit a document object is not kept, and identity comparisons of
    // documents opened after it fail.
    if let Some(cache) = data.get_mut::<DocCache>()
        && (cache.0.len() < MAX_OBJECTS || cache.0.contains_key(&key))
    {
        cache.0.insert(key, obj.clone());
    }
    obj
}

fn doc_to_string(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    doc_name(this, args, ctx)
}

pub(crate) fn getter(ctx: &mut Context, f: Native) -> JsFunction {
    FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(f)).length(0).build()
}

pub(crate) fn doc_id(v: &JsValue) -> Option<DocKey> {
    v.as_object().and_then(|o| o.downcast_ref::<DocRef>().map(|d| d.0))
}

/// The document key of an entry of `session.inspect`'s `documents`.
pub(crate) fn entry_id(entry: &Value) -> JsResult<DocKey> {
    match (entry["id"].as_u64(), entry["serial"].as_u64()) {
        (Some(id), Some(serial)) => Ok(DocKey { id, serial }),
        _ => Err(error("the session did not report a document id")),
    }
}

/// The `session.inspect` entry of document `id`.
fn resolve(id: DocKey, ctx: &mut Context) -> JsResult<Value> {
    let s = session_inspect(ctx)?;
    s["documents"]
        .as_array()
        .and_then(|docs| docs.iter().find(|d| entry_id(d).ok() == Some(id)))
        .cloned()
        .ok_or_else(|| error("the document is no longer open"))
}

/// The document's own values (its name and resolution): neither its layers nor its History are
/// read, whatever their size.
fn this_doc(this: &JsValue, ctx: &mut Context) -> JsResult<Value> {
    let id = doc_id(this).ok_or_else(|| JsNativeError::typ().with_message("not a Document"))?;
    query_document(id, json!({ "serial": id.serial, "history": false, "layers": false }), ctx)
}

fn query_document(id: DocKey, params: Value, ctx: &mut Context) -> JsResult<Value> {
    let d = command(ctx, "document.inspect", params).map_err(|e| host_failure(ctx, e, "the document is no longer open"))?;
    // Checked against a host that answers for another document, or names none.
    if d.get("id").and_then(Value::as_u64) != Some(id.id) || d.get("serial").and_then(Value::as_u64) != Some(id.serial) {
        return Err(error("the document is no longer open"));
    }
    Ok(d)
}

pub(crate) fn field(doc: &Value, key: &str, ctx: &mut Context) -> JsResult<JsValue> {
    value::from_json(doc.get(key).unwrap_or(&Value::Null), ctx)
}

fn doc_name(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let d = this_doc(this, ctx)?;
    field(&d, "name", ctx)
}

fn doc_resolution(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let d = this_doc(this, ctx)?;
    field(&d, "resolution", ctx)
}

fn doc_saved(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = doc_id(this).ok_or_else(|| JsNativeError::typ().with_message("not a Document"))?;
    let entry = resolve(id, ctx)?;
    let dirty = entry.get("dirty").and_then(Value::as_bool).ok_or_else(|| error("the session did not report whether the document is saved"))?;
    let has_path = entry.get("path").is_some_and(Value::is_string);
    Ok(JsValue::from(!dirty && has_path))
}

fn app_documents(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let s = session_inspect(ctx)?;
    let ids = s["documents"].as_array().map_or(Ok(Vec::new()), |docs| docs.iter().map(entry_id).collect::<JsResult<Vec<_>>>())?;
    let docs: Vec<JsValue> = ids.into_iter().map(|id| document_object(id, ctx).into()).collect();
    Ok(JsArray::from_iter(docs, ctx).into())
}

fn app_active_document(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let s = session_inspect(ctx)?;
    let active = s["active"].as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| error_with_number(ctx, "No such element", 1302))?;
    let entry = s["documents"].get(active).ok_or_else(|| error_with_number(ctx, "No such element", 1302))?;
    let id = entry_id(entry)?;
    Ok(document_object(id, ctx).into())
}

fn app_set_active_document(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = args.first().and_then(doc_id).ok_or_else(|| error("Object expected"))?;
    let s = session_inspect(ctx)?;
    if !s["documents"].as_array().is_some_and(|docs| docs.iter().any(|d| entry_id(d).ok() == Some(id))) {
        return Err(error("Object expected"));
    }
    command(ctx, "document.activate", json!({ "serial": id.serial })).map_err(|e| host_failure(ctx, e, "Object expected"))?;
    Ok(JsValue::undefined())
}

/// Properties of `app` a script may name as globals, as in Photoshop.
const APP_GLOBALS: &[&str] = &["activeDocument", "documents"];
/// Those of them `app` takes a value for.
const APP_SETTABLE: &[&str] = &["activeDocument"];

fn app_global_get(_: &JsValue, _: &[JsValue], key: &JsString, ctx: &mut Context) -> JsResult<JsValue> {
    let app = ctx.global_object().get(js_string!("app"), ctx)?;
    match app.as_object() {
        Some(app) => app.get(key.clone(), ctx),
        None => Ok(JsValue::undefined()),
    }
}

/// `activeDocument = doc` sets `app`'s; a name `app` can't take (`var documents = []`)
/// becomes the script's own variable, as a plain global would.
fn app_global_set(_: &JsValue, args: &[JsValue], key: &JsString, ctx: &mut Context) -> JsResult<JsValue> {
    let value = args.first().cloned().unwrap_or_default();
    let app = ctx.global_object().get(js_string!("app"), ctx)?;
    if let Some(app) = app.as_object()
        && APP_SETTABLE.iter().any(|k| key.to_std_string_lossy() == *k)
    {
        app.set(key.clone(), value, false, ctx)?;
        return Ok(JsValue::undefined());
    }
    let own = PropertyDescriptor::builder().value(value).writable(true).enumerable(true).configurable(true).build();
    ctx.global_object().define_property_or_throw(key.clone(), own, ctx)?;
    Ok(JsValue::undefined())
}

fn no_op(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

/// `$.os`: the system as ExtendScript names it, closely enough for the checks scripts make.
fn host_os() -> &'static str {
    if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "Macintosh OS"
    } else if cfg!(target_arch = "wasm32") {
        "Web"
    } else {
        "Unix"
    }
}

/// Install the globals into a fresh context.
pub(crate) fn install(ctx: &mut Context) -> JsResult<()> {
    // Before any script could replace them.
    value::install(ctx)?;
    for (name, f, len) in [("alert", alert as Native, 1), ("confirm", confirm, 1), ("prompt", prompt, 2)] {
        ctx.register_global_builtin_callable(JsString::from(name), len, NativeFunction::from_fn_ptr(f))?;
    }

    let get_name = getter(ctx, app_name);
    let get_documents = getter(ctx, app_documents);
    let get_active = getter(ctx, app_active_document);
    let set_active = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(app_set_active_document)).length(1).build();
    let app = ObjectInitializer::new(ctx)
        .accessor(js_string!("name"), Some(get_name), None, Attribute::ENUMERABLE)
        .property(js_string!("version"), js_string!(APP_VERSION), Attribute::ENUMERABLE)
        .accessor(js_string!("documents"), Some(get_documents), None, Attribute::ENUMERABLE)
        .accessor(js_string!("activeDocument"), Some(get_active), Some(set_active), Attribute::ENUMERABLE)
        // The window is always in front of a script the user ran from it.
        .function(NativeFunction::from_fn_ptr(no_op), js_string!("bringToFront"), 0)
        .build();
    ctx.register_global_property(js_string!("app"), app, Attribute::all())?;
    // Photoshop scripts name these without `app.` (`activeDocument.name`, `documents.length`).
    for key in APP_GLOBALS {
        let key = JsString::from(*key);
        let get = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_copy_closure_with_captures(app_global_get, key.clone())).length(0).build();
        let set = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_copy_closure_with_captures(app_global_set, key.clone())).length(1).build();
        let desc = PropertyDescriptor::builder().get(get).set(set).enumerable(false).configurable(true).build();
        ctx.global_object().define_property_or_throw(key, desc, ctx)?;
    }

    let dollar = ObjectInitializer::new(ctx)
        .function(NativeFunction::from_fn_ptr(writeln), js_string!("writeln"), 1)
        // What scripts test to pick line endings and paths (`$.os.search(/windows/i)`).
        .property(js_string!("os"), JsString::from(host_os()), Attribute::READONLY | Attribute::NON_ENUMERABLE | Attribute::PERMANENT)
        .build();
    ctx.register_global_property(js_string!("$"), dollar, Attribute::all())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_allowed_commands_are_registered_and_nothing_else_is_reachable() {
        let allowed: Vec<&str> = crate::commands::allowed().collect();
        let registered: Vec<&str> = photocraft_engine::command_specs().iter().map(|c| c.id).collect();
        for id in &allowed {
            assert!(registered.contains(id), "{id} is allowed but no such command is registered");
        }
        // Every other registered command is refused, whatever its parameters.
        let mut refused = 0;
        for id in registered.iter().filter(|id| !allowed.contains(id)) {
            for params in [serde_json::json!({}), serde_json::json!({"onDocument": 1})] {
                assert!(crate::commands::check_command(id, &params).is_err(), "{id} is not allowed but passes the check");
            }
            refused += 1;
        }
        assert!(refused > allowed.len() * 10, "the registry walk saw the real registry: {refused} refused");
    }
}
