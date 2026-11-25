//! The global objects scripts see. All of them are Rust natives; no JavaScript source is
//! evaluated besides the user's script.

use std::collections::HashMap;

use boa_engine::context::time::{Clock, StdClock};
use boa_engine::object::builtins::JsFunction;
use boa_engine::object::{FunctionObjectBuilder, ObjectInitializer};
use boa_engine::property::{Attribute, PropertyDescriptor};
use boa_engine::{Context, Finalize, JsData, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, Trace, js_string};
use serde_json::{Value, json};

use crate::runner::{Shared, Stop};
use crate::tape::{Begin, Call, Outcome};
use crate::{APP_VERSION, ModalKind, abort, unit_value, value};

pub(crate) type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

/// Which document a handle means: its session document id plus its open serial (`id` and
/// `serial` in `session.inspect`). A closed document's id can go to a newer document; the serial
/// is never reused, so a stale handle can't take that document over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DocKey {
    pub(crate) id: u64,
    pub(crate) serial: u64,
}

/// The document a `Document` object stands for. Tabs can move or close while the run is paused
/// at a dialog, so the key is resolved to the current tab index on every access. One wrapper is
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

/// End the attempt with an uncatchable error, remembering why.
fn stop(ctx: &mut Context, why: Stop) -> JsError {
    if let Ok(s) = shared(ctx) {
        s.borrow_mut().stop = Some(why);
    }
    abort::uncatchable()
}

/// Charge the budget once, stopping the attempt when it is spent or when a stop is already
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

/// Start a taped call: charge the budget, then answer from the tape or report it live.
fn begin(ctx: &mut Context, call: Call) -> JsResult<Result<Outcome, usize>> {
    begin_with(ctx, call, true)
}

/// [`begin`], charging the budget first only with `charge`.
fn begin_with(ctx: &mut Context, call: Call, charge: bool) -> JsResult<Result<Outcome, usize>> {
    if charge {
        checkpoint(ctx)?;
    }
    let begun = shared(ctx)?.borrow_mut().tape.begin(call);
    match begun {
        Err(divergence) => Err(stop(ctx, Stop::Fault(divergence))),
        Ok(Begin::Replay(o)) => Ok(Ok(o)),
        Ok(Begin::Live(at)) => Ok(Err(at)),
    }
}

/// Run a session command through the tape. Only the commands in [`crate::commands`] with the
/// parameters they allow get through, the same check the script host's supervisor applies.
pub(crate) fn command(ctx: &mut Context, id: &str, params: Value) -> JsResult<Value> {
    command_charged(ctx, id, params, (true, true))
}

/// [`command`], with the budget charged (and a stop taken) before and after it only as
/// `charged` says. Two commands that must not be split by a stop run with the charge between
/// them left out: the first not charged after, the second not before.
pub(crate) fn command_charged(ctx: &mut Context, id: &str, params: Value, charged: (bool, bool)) -> JsResult<Value> {
    crate::commands::check_command(id, &params).map_err(error)?;
    shared(ctx)?.borrow().grants.check(id, &params).map_err(error)?;
    let outcome = match begin_with(ctx, Call::Command { id: id.into(), params: params.clone() }, charged.0)? {
        Ok(o) => o,
        Err(at) => {
            let s = shared(ctx)?;
            let o = {
                let mut st = s.borrow_mut();
                let o = match st.host.execute(id, params) {
                    Ok(v) => Outcome::Return(v),
                    Err(e) => Outcome::Throw(e.0),
                };
                st.tape.finish(at, o.clone());
                o
            };
            // A slow command counts too: charge again once it returned.
            if charged.1 {
                checkpoint(ctx)?;
            }
            o
        }
    };
    match outcome {
        Outcome::Return(v) => Ok(v),
        Outcome::Throw(m) => Err(error(m)),
        Outcome::Modal(_) => Err(stop(ctx, Stop::Fault("replay divergence: a command was recorded as a dialog".into()))),
    }
}

fn modal(ctx: &mut Context, kind: ModalKind, args: &[JsValue]) -> JsResult<JsValue> {
    let args = Value::Array(value::texts(args, ctx)?.into_iter().map(Value::String).collect());
    let answer = modal_value(ctx, kind, args)?;
    match (kind, answer) {
        (ModalKind::Confirm, v) => Ok(JsValue::from(v.as_bool().unwrap_or(false))),
        (ModalKind::Prompt, Value::String(s)) => Ok(JsValue::from(JsString::from(s.as_str()))),
        (ModalKind::Prompt, _) => Ok(JsValue::null()),
        _ => Ok(JsValue::undefined()),
    }
}

/// Open a modal dialog of `kind` with `args` through the tape and return the UI's answer:
/// recorded on a replay, or waited for live (pausing the run when the host cannot wait).
pub(crate) fn modal_value(ctx: &mut Context, kind: ModalKind, args: Value) -> JsResult<Value> {
    match begin(ctx, Call::Modal { kind, args: args.clone() })? {
        Ok(Outcome::Modal(Some(answer))) => {
            shared(ctx)?.borrow_mut().grants.record(kind, &answer);
            Ok(answer)
        }
        // Still open from an earlier attempt: pause the run.
        Ok(Outcome::Modal(None)) => Err(stop(ctx, Stop::Paused)),
        Ok(_) => Err(stop(ctx, Stop::Fault("replay divergence: a dialog was recorded as a command".into()))),
        Err(at) => {
            let s = shared(ctx)?;
            // Time at an open dialog is not the script's: stop the budget while the host waits.
            s.borrow_mut().budget.pause();
            let waited = s.borrow_mut().host.modal(kind, &args);
            s.borrow_mut().budget.resume();
            match waited {
                // The host cannot wait for the answer: pause, and replay once it is in.
                None => Err(stop(ctx, Stop::Paused)),
                Some(Err(e)) => Err(stop(ctx, Stop::Fault(e.0))),
                Some(Ok(answer)) => {
                    let mut st = s.borrow_mut();
                    st.grants.record(kind, &answer);
                    st.tape.finish(at, Outcome::Modal(Some(answer.clone())));
                    Ok(answer)
                }
            }
        }
    }
}

/// Show, update (`Some`) or hide (`None`) a window that does not wait, through the tape: a
/// replay does not send it again.
pub(crate) fn window_update(ctx: &mut Context, id: u32, window: Option<Value>) -> JsResult<()> {
    let outcome = match begin(ctx, Call::Window { id, window: window.clone() })? {
        // A replay fails where the live attempt failed.
        Ok(o) => o,
        Err(at) => {
            let s = shared(ctx)?;
            let o = {
                let mut st = s.borrow_mut();
                let o = match st.host.window(id, window.as_ref()) {
                    Ok(()) => Outcome::Return(Value::Null),
                    Err(e) => Outcome::Throw(e.0),
                };
                st.tape.finish(at, o.clone());
                o
            };
            // The time the host took counts, as after a command.
            checkpoint(ctx)?;
            o
        }
    };
    match outcome {
        Outcome::Throw(m) => Err(error(m)),
        _ => Ok(()),
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
    if let Err(at) = begin(ctx, Call::Output(line.clone()))? {
        {
            let s = shared(ctx)?;
            let mut st = s.borrow_mut();
            st.host.write_line(&line);
            st.tape.finish(at, Outcome::Return(Value::Null));
        }
        // The write's own time counts too, also when it is the script's last act.
        checkpoint(ctx)?;
    }
    Ok(JsValue::undefined())
}

fn random(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    // Charged like every other taped call, live and in replay.
    checkpoint(ctx)?;
    let s = shared(ctx)?;
    let (v, overflow) = {
        let mut st = s.borrow_mut();
        let v = match st.tape.take_random() {
            Some(v) => v,
            None => {
                let v = st.next_random();
                st.tape.push_random(v);
                v
            }
        };
        (v, st.tape.overflow().map(str::to_owned))
    };
    // Past the reading bound the run stops here, not at its next host call.
    match overflow {
        Some(m) => Err(stop(ctx, Stop::Fault(m))),
        None => Ok(JsValue::from(v)),
    }
}

/// `Date.now()`: the taped wall clock, stopping at once past the reading bound. (`new Date()`
/// reads the same tape through the context clock, which can't stop the run itself; it stops at
/// the next host call.)
fn date_now(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    // Charged like every other taped call, live and in replay.
    checkpoint(ctx)?;
    let s = shared(ctx)?;
    let (v, overflow) = {
        let mut st = s.borrow_mut();
        let v = st.tape.clock(|| StdClock::new().system_time_millis());
        (v, st.tape.overflow().map(str::to_owned))
    };
    match overflow {
        Some(m) => Err(stop(ctx, Stop::Fault(m))),
        None => Ok(JsValue::from(v)),
    }
}

fn app_name(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let name = shared(ctx)?.borrow().app_name.clone();
    Ok(JsValue::from(JsString::from(name.as_str())))
}

pub(crate) fn session_inspect(ctx: &mut Context) -> JsResult<Value> {
    command(ctx, "session.inspect", json!({}))
}

pub(crate) fn document_object(id: DocKey, ctx: &mut Context) -> JsObject {
    let key = (id.id, id.serial);
    let cached = ctx.realm().host_defined().get::<DocCache>().and_then(|c| c.0.get(&key).cloned());
    if let Some(obj) = cached {
        return obj;
    }
    let getters: [(JsString, Native); 5] = [
        (js_string!("name"), doc_name),
        (js_string!("width"), doc_width),
        (js_string!("height"), doc_height),
        (js_string!("resolution"), doc_resolution),
        (js_string!("saved"), doc_saved),
    ];
    let mut accessors: Vec<(JsString, JsFunction, Option<JsFunction>)> = getters.into_iter().map(|(k, f)| (k, getter(ctx, f), None)).collect();
    for (k, g, s) in crate::dom::DOC_ACCESSORS {
        accessors.push((JsString::from(*k), getter(ctx, *g), s.map(|s| getter(ctx, s))));
    }
    let mut init = ObjectInitializer::with_native_data(DocRef(id), ctx);
    for (k, g, s) in accessors {
        init.accessor(k, Some(g), s, Attribute::ENUMERABLE | Attribute::CONFIGURABLE);
    }
    init.function(NativeFunction::from_fn_ptr(doc_to_string), js_string!("toString"), 0);
    for (k, f, len) in crate::dom::DOC_METHODS {
        init.function(NativeFunction::from_fn_ptr(*f), JsString::from(*k), *len);
    }
    let obj = init.build();
    let realm = ctx.realm().clone();
    let mut data = realm.host_defined_mut();
    if data.get::<DocCache>().is_none() {
        data.insert(DocCache::default());
    }
    // Bounded as the layer objects are: past the limit a document object is not kept.
    if let Some(cache) = data.get_mut::<DocCache>()
        && (cache.0.len() < crate::dom::MAX_OBJECTS || cache.0.contains_key(&key))
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

/// The current tab index and `session.inspect` entry of document `id`.
pub(crate) fn resolve(id: DocKey, ctx: &mut Context) -> JsResult<(usize, Value)> {
    let s = session_inspect(ctx)?;
    s["documents"]
        .as_array()
        .and_then(|docs| docs.iter().enumerate().find(|(_, d)| entry_id(d).ok() == Some(id)))
        .map(|(i, d)| (i, d.clone()))
        .ok_or_else(|| error("the document is no longer open"))
}

/// The document's own values (name, size, resolution): neither its layers nor its History are
/// read, whatever their size.
fn this_doc(this: &JsValue, ctx: &mut Context) -> JsResult<Value> {
    let id = doc_id(this).ok_or_else(|| JsNativeError::typ().with_message("not a Document"))?;
    query_document(id, json!({ "serial": id.serial, "history": false, "layers": false }), ctx)
}

/// `document.inspect` of document `id`, by its serial: one call, so a tab closed or moved in the
/// meantime can't make it read another document.
pub(crate) fn inspect_document(id: DocKey, history: bool, ctx: &mut Context) -> JsResult<Value> {
    query_document(id, json!({ "serial": id.serial, "history": history }), ctx)
}

fn query_document(id: DocKey, params: Value, ctx: &mut Context) -> JsResult<Value> {
    let d = command(ctx, "document.inspect", params).map_err(|e| host_failure(ctx, e, "the document is no longer open"))?;
    // The id is checked too, against a host that answers for another document.
    if d.get("id").and_then(Value::as_u64).is_some_and(|i| i != id.id) {
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

/// The document's width in the ruler unit (`app.preferences.rulerUnits`).
fn doc_width(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let d = this_doc(this, ctx)?;
    let w = d["width"].as_f64().ok_or_else(|| error("the session did not report a document size"))?;
    crate::dom::ruler_length(ctx, w, d["resolution"].as_f64().unwrap_or(72.0))
}

/// The document's height in the ruler unit.
fn doc_height(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let d = this_doc(this, ctx)?;
    let h = d["height"].as_f64().ok_or_else(|| error("the session did not report a document size"))?;
    crate::dom::ruler_length(ctx, h, d["resolution"].as_f64().unwrap_or(72.0))
}

fn doc_resolution(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let d = this_doc(this, ctx)?;
    field(&d, "resolution", ctx)
}

fn doc_saved(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = doc_id(this).ok_or_else(|| JsNativeError::typ().with_message("not a Document"))?;
    let (_, entry) = resolve(id, ctx)?;
    let dirty = entry.get("dirty").and_then(Value::as_bool).ok_or_else(|| error("the session did not report whether the document is saved"))?;
    let has_path = entry.get("path").is_some_and(Value::is_string);
    Ok(JsValue::from(!dirty && has_path))
}

fn app_documents(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let s = session_inspect(ctx)?;
    let ids = s["documents"].as_array().map_or(Ok(Vec::new()), |docs| docs.iter().map(entry_id).collect::<JsResult<Vec<_>>>())?;
    let docs: Vec<JsValue> = ids.into_iter().map(|id| document_object(id, ctx).into()).collect();
    crate::dom::live_collection(ctx, crate::dom::documents_key(), docs, |arr, ctx| {
        let methods: [(&str, Native, usize); 2] = [("add", crate::dom::documents_add, 6), ("getByName", crate::dom::documents_get_by_name, 1)];
        for (k, f, len) in methods {
            let f = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(f)).name(JsString::from(k)).length(len).build();
            arr.create_data_property_or_throw(JsString::from(k), f, ctx)?;
        }
        Ok(())
    })
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
const APP_GLOBALS: &[&str] = &["activeDocument", "documents", "preferences", "displayDialogs"];
/// Those of them `app` takes a value for.
const APP_SETTABLE: &[&str] = &["activeDocument", "displayDialogs"];

fn app_global_get(_: &JsValue, _: &[JsValue], key: &JsString, ctx: &mut Context) -> JsResult<JsValue> {
    let app = ctx.global_object().get(js_string!("app"), ctx)?;
    match app.as_object() {
        Some(app) => app.get(key.clone(), ctx),
        None => Ok(JsValue::undefined()),
    }
}

/// `activeDocument = doc` sets `app`'s; a name `app` can't take (`var documents = []`,
/// `preferences = {...}`) becomes the script's own variable, as a plain global would.
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
    let set_active = getter(ctx, app_set_active_document);
    let app = ObjectInitializer::new(ctx)
        .accessor(js_string!("name"), Some(get_name), None, Attribute::ENUMERABLE)
        .property(js_string!("version"), js_string!(APP_VERSION), Attribute::ENUMERABLE)
        .accessor(js_string!("documents"), Some(get_documents), None, Attribute::ENUMERABLE)
        .accessor(js_string!("activeDocument"), Some(get_active), Some(set_active), Attribute::ENUMERABLE)
        .function(NativeFunction::from_fn_ptr(crate::scriptui::refresh), js_string!("refresh"), 0)
        // The window is always in front of a script the user ran from it.
        .function(NativeFunction::from_fn_ptr(no_op), js_string!("bringToFront"), 0)
        .build();
    crate::dom::install(ctx, &app)?;
    crate::descriptor::install(ctx, &app)?;
    ctx.register_global_property(js_string!("app"), app, Attribute::all())?;
    // Photoshop scripts name these without `app.` (`activeDocument.name`, `preferences.rulerUnits`).
    for key in APP_GLOBALS {
        let key = JsString::from(*key);
        let get = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_copy_closure_with_captures(app_global_get, key.clone())).length(0).build();
        let set = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_copy_closure_with_captures(app_global_set, key.clone())).length(1).build();
        let desc = PropertyDescriptor::builder().get(get).set(set).enumerable(false).configurable(true).build();
        ctx.global_object().define_property_or_throw(key, desc, ctx)?;
    }

    unit_value::install(ctx)?;
    crate::scriptui::install(ctx)?;
    crate::files::install(ctx)?;

    let dollar = ObjectInitializer::new(ctx)
        .function(NativeFunction::from_fn_ptr(writeln), js_string!("writeln"), 1)
        // What scripts test to pick line endings and paths (`$.os.search(/windows/i)`).
        .property(js_string!("os"), JsString::from(host_os()), Attribute::READONLY | Attribute::NON_ENUMERABLE | Attribute::PERMANENT)
        .build();
    ctx.register_global_property(js_string!("$"), dollar, Attribute::all())?;

    // Replays depend on these going through the tape: without them the run can't start.
    let math = ctx.global_object().get(js_string!("Math"), ctx)?;
    let math = math.as_object().ok_or_else(|| error("internal error: Math is missing"))?;
    let f = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(random)).name(js_string!("random")).length(0).build();
    math.set(js_string!("random"), f, true, ctx)?;
    let date = ctx.global_object().get(js_string!("Date"), ctx)?;
    let date = date.as_object().ok_or_else(|| error("internal error: Date is missing"))?;
    let f = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(date_now)).name(js_string!("now")).length(0).build();
    date.set(js_string!("now"), f, true, ctx)?;
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
