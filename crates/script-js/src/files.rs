//! `File` and `Folder`: paths, and the pickers that let the user choose one.
//!
//! These are names only. A script cannot read, write, list or test files through them: there is
//! no file access in a run, so the methods that would need it throw a named error. What a
//! picker returns is the path the user chose, for the script to show or to pass back to the app.

use boa_engine::object::builtins::JsArray;
use boa_engine::object::{FunctionObjectBuilder, ObjectInitializer};
use boa_engine::property::Attribute;
use boa_engine::{Context, Finalize, JsData, JsObject, JsResult, JsString, JsValue, NativeFunction, Trace, js_string};
use serde_json::{Value, json};

use crate::ModalKind;
use crate::natives::{error, modal_value};
use crate::value;

type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

/// Paths one multiple-file answer may hold.
pub const MAX_CHOSEN: usize = 10_000;
/// Longest path kept from an answer, in UTF-16 units.
const MAX_PATH_UNITS: usize = 32 << 10;

const NO_ACCESS: &str = "scripts cannot read or write files in PhotoCraft yet";

/// Marks a `File` or `Folder` object (the prototype tells which).
#[derive(Debug, Clone, Copy, Trace, Finalize, JsData)]
// Sound: holds no GC pointers.
#[boa_gc(empty_trace)]
struct PathRef;

/// The prototypes, in the realm's host data.
#[derive(Trace, Finalize, JsData)]
struct Protos {
    file: JsObject,
    folder: JsObject,
}

fn protos(ctx: &Context) -> JsResult<(JsObject, JsObject)> {
    let realm = ctx.realm().clone();
    let data = realm.host_defined();
    let p = data.get::<Protos>().ok_or_else(|| error("File is not available"))?;
    Ok((p.file.clone(), p.folder.clone()))
}

fn text_of(v: Option<&JsValue>, ctx: &mut Context) -> JsResult<Option<String>> {
    match v {
        None => Ok(None),
        Some(v) if v.is_undefined() || v.is_null() => Ok(None),
        Some(v) => Ok(Some(value::texts(std::slice::from_ref(v), ctx)?.concat())),
    }
}

/// The last part of a path, after `/` or `\`, ignoring a trailing separator.
fn base_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed)
}

/// The path without its last part, or empty.
fn parent(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(0) => trimmed.get(..1).unwrap_or(""),
        Some(i) => trimmed.get(..i).unwrap_or(""),
        None => "",
    }
}

/// A `File` or `Folder` object for `path`.
fn make(ctx: &mut Context, path: &str, folder: bool) -> JsResult<JsObject> {
    let (file_proto, folder_proto) = protos(ctx)?;
    let obj = JsObject::from_proto_and_data(Some(if folder { folder_proto } else { file_proto }), PathRef);
    let s = |t: &str| JsValue::from(JsString::from(t));
    let full = path.replace('\\', "/");
    let uri: String = full
        .chars()
        .flat_map(|c| match c {
            ' ' => "%20".chars().collect::<Vec<_>>(),
            '%' => "%25".chars().collect(),
            c => vec![c],
        })
        .collect();
    for (key, v) in [
        ("fsName", s(path)),
        ("fullName", s(&full)),
        ("absoluteURI", s(&uri)),
        ("name", s(base_name(path))),
        ("displayName", s(base_name(path))),
        ("path", s(&parent(path).replace('\\', "/"))),
    ] {
        obj.set(JsString::from(key), v, true, ctx)?;
    }
    Ok(obj)
}

fn ctor(folder: bool, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let path = text_of(args.first(), ctx)?.unwrap_or_default();
    Ok(make(ctx, &path, folder)?.into())
}

/// `File(path)`, `new File(path)`.
fn file_ctor(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    ctor(false, args, ctx)
}

/// `Folder(path)`, `new Folder(path)`.
fn folder_ctor(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    ctor(true, args, ctx)
}

fn to_string(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let Some(obj) = this.as_object() else { return Ok(JsValue::from(js_string!(""))) };
    let v = obj.get(js_string!("fullName"), ctx)?;
    Ok(JsValue::from(JsString::from(text_of(Some(&v), ctx)?.unwrap_or_default().as_str())))
}

fn no_access(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Err(error(NO_ACCESS))
}

/// The picker's arguments: `prompt`, a filter when it is text (a filter function cannot be sent
/// to the app and is ignored), and whether several files may be chosen.
fn picker_args(ctx: &mut Context, args: &[JsValue], with_filter: bool, with_multiple: bool) -> JsResult<Value> {
    let prompt = text_of(args.first(), ctx)?;
    let mut out = json!({ "prompt": prompt });
    if with_filter {
        let filter = match args.get(1) {
            Some(v) if v.is_callable() => None,
            v => text_of(v, ctx)?,
        };
        out["filter"] = json!(filter);
    }
    if with_multiple {
        out["multiple"] = json!(args.get(2).is_some_and(JsValue::to_boolean));
    }
    Ok(out)
}

/// A path from the answer, if it is one.
fn answered_path(v: &Value) -> Option<&str> {
    v.as_str().filter(|p| !p.is_empty() && !(p.len() > MAX_PATH_UNITS && p.encode_utf16().count() > MAX_PATH_UNITS))
}

fn pick(ctx: &mut Context, kind: ModalKind, args: Value, multiple: bool) -> JsResult<JsValue> {
    let answer = modal_value(ctx, kind, args)?;
    let folder = kind == ModalKind::SelectFolder;
    if multiple {
        let Some(list) = answer.as_array() else {
            return match answered_path(&answer) {
                Some(p) => {
                    let f = make(ctx, p, false)?;
                    Ok(JsArray::from_iter([JsValue::from(f)], ctx).into())
                }
                None => Ok(JsValue::null()),
            };
        };
        let mut files = Vec::new();
        for p in list.iter().take(MAX_CHOSEN).filter_map(answered_path) {
            files.push(JsValue::from(make(ctx, p, false)?));
        }
        if files.is_empty() {
            return Ok(JsValue::null());
        }
        return Ok(JsArray::from_iter(files, ctx).into());
    }
    match answered_path(&answer) {
        Some(p) => Ok(make(ctx, p, folder)?.into()),
        None => Ok(JsValue::null()),
    }
}

/// `File.openDialog(prompt, filter, multiSelect)` and `file.openDlg(...)`.
fn open_dialog(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let a = picker_args(ctx, args, true, true)?;
    let multiple = a["multiple"].as_bool().unwrap_or(false);
    pick(ctx, ModalKind::OpenFile, a, multiple)
}

/// `File.saveDialog(prompt, filter)` and `file.saveDlg(...)`.
fn save_dialog(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let a = picker_args(ctx, args, true, false)?;
    pick(ctx, ModalKind::SaveFile, a, false)
}

/// `Folder.selectDialog(prompt)` and `folder.selectDlg(...)`.
fn select_dialog(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let a = picker_args(ctx, args, false, false)?;
    pick(ctx, ModalKind::SelectFolder, a, false)
}

/// Install `File` and `Folder`.
pub(crate) fn install(ctx: &mut Context) -> JsResult<()> {
    // Methods that would read or write the file system: a named error, not "not a function".
    let blocked = ["open", "read", "readln", "readch", "write", "writeln", "close", "remove", "rename", "copy", "execute", "create", "getFiles", "seek"];
    let mut file = ObjectInitializer::new(ctx);
    file.function(NativeFunction::from_fn_ptr(to_string), js_string!("toString"), 0);
    file.function(NativeFunction::from_fn_ptr(open_dialog), js_string!("openDlg"), 3);
    file.function(NativeFunction::from_fn_ptr(save_dialog), js_string!("saveDlg"), 2);
    for m in blocked {
        file.function(NativeFunction::from_fn_ptr(no_access), JsString::from(m), 0);
    }
    let file_proto = file.build();
    let mut folder = ObjectInitializer::new(ctx);
    folder.function(NativeFunction::from_fn_ptr(to_string), js_string!("toString"), 0);
    folder.function(NativeFunction::from_fn_ptr(select_dialog), js_string!("selectDlg"), 1);
    for m in blocked {
        folder.function(NativeFunction::from_fn_ptr(no_access), JsString::from(m), 0);
    }
    let folder_proto = folder.build();
    let realm = ctx.realm().clone();
    realm.host_defined_mut().insert(Protos { file: file_proto.clone(), folder: folder_proto.clone() });

    let fs = if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "Macintosh"
    } else {
        "Unix"
    };
    let statics: [(&str, Native, usize, bool); 2] = [("File", file_ctor, 1, false), ("Folder", folder_ctor, 1, true)];
    for (name, f, len, is_folder) in statics {
        let ctor = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(f)).name(JsString::from(name)).length(len).constructor(true).build();
        crate::natives::link_prototype(&ctor, if is_folder { &folder_proto } else { &file_proto }, ctx)?;
        let pickers: &[(&str, Native, usize)] =
            if is_folder { &[("selectDialog", select_dialog, 1)] } else { &[("openDialog", open_dialog, 3), ("saveDialog", save_dialog, 2)] };
        for (pname, pf, plen) in pickers {
            let p = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(*pf)).name(JsString::from(*pname)).length(*plen).build();
            ctor.set(JsString::from(*pname), p, true, ctx)?;
        }
        ctor.set(js_string!("fs"), JsValue::from(JsString::from(fs)), true, ctx)?;
        ctx.register_global_property(JsString::from(name), ctor, Attribute::WRITABLE | Attribute::CONFIGURABLE)?;
    }
    Ok(())
}
