//! Conversions between script values and the text and JSON that cross into the host.

use boa_engine::builtins::error::Error as ErrorData;
use boa_engine::object::builtins::JsProxy;
use boa_engine::property::PropertyKey;
use boa_engine::{Context, Finalize, JsData, JsError, JsNativeError, JsObject, JsResult, JsString, JsSymbol, JsValue, Trace, js_string};
use serde_json::Value;

/// Deepest array nesting a value may have when it is converted to text.
pub const MAX_DEPTH: usize = 128;
/// Most array slots one conversion to text may join, counted along every path: an array's
/// whole `length`, holes included, each time it is met.
pub const MAX_ITEMS: usize = 1 << 20;
/// Longest text, in UTF-16 units, one conversion may hand to the host.
pub const MAX_TEXT_UNITS: usize = 16 << 20;
/// Longest error text a failed run reports, in characters, before it is cut.
pub const MAX_ERROR_CHARS: usize = 2000;
/// What a failed run reports for a thrown object that is not an Error and has no string `message`.
pub const NOT_AN_ERROR: &str = "Uncaught exception: the script threw an object that is not an Error.";

/// A script string as Rust text. A lone surrogate at the top level is a `TypeError` the script
/// can catch.
pub fn text(s: &JsString) -> JsResult<String> {
    s.to_std_string().map_err(|_| JsNativeError::typ().with_message("string contains an unpaired surrogate").into())
}

/// The intrinsic `Array.prototype.toString`, `Array.prototype.join` and
/// `Object.prototype.toString` of a context, taken before any script ran. Converting a value to
/// text, [`texts`] runs the intrinsic array conversion itself, iteratively and within bounds,
/// instead of calling the engine's `join`, which converts nested arrays by native recursion.
#[derive(Trace, Finalize, JsData)]
struct Intrinsics {
    array_to_string: JsObject,
    array_join: JsObject,
    object_to_string: JsObject,
}

/// Remember the context's intrinsic conversions. Call once, before any script runs.
pub(crate) fn install(ctx: &mut Context) -> JsResult<()> {
    let function = |o: &JsObject, name: &str, ctx: &mut Context| -> JsResult<JsObject> {
        o.get(JsString::from(name), ctx)?.as_object().ok_or_else(|| JsNativeError::error().with_message(format!("the intrinsic {name} is missing")).into())
    };
    let array = ctx.intrinsics().constructors().array().prototype();
    let object = ctx.intrinsics().constructors().object().prototype();
    let intrinsics = Intrinsics {
        array_to_string: function(&array, "toString", ctx)?,
        array_join: function(&array, "join", ctx)?,
        object_to_string: function(&object, "toString", ctx)?,
    };
    let realm = ctx.realm().clone();
    realm.host_defined_mut().insert(intrinsics);
    Ok(())
}

/// The arguments of one host call (`$.writeln`'s parts, a dialog's arguments) as text, each the
/// way `String(value)` converts it, in the same order: `Symbol.toPrimitive`, then `toString`,
/// then `valueOf`, each read once. Getters, proxy traps and conversion methods are script code,
/// and their host calls are counted like any other. A failure in them is a catchable error at
/// the call.
///
/// Where the conversion reaches the intrinsic `Array.prototype.toString` with the intrinsic
/// `join`, the join is done here, with the engine's result but without its native recursion:
/// every element is read once, nested arrays are followed to at most [`MAX_DEPTH`] levels, an
/// array met again on the path it is on becomes `''` (as every engine joins a cycle), at most
/// [`MAX_ITEMS`] slots are visited, and the text may not exceed [`MAX_TEXT_UNITS`] units; past a
/// bound the conversion is a `RangeError`.
///
/// The arguments are held to [`MAX_TEXT_UNITS`] together, checked as each one is converted, so
/// many large arguments are refused before they are all built.
pub fn texts(args: &[JsValue], ctx: &mut Context) -> JsResult<Vec<String>> {
    let mut total = 0usize;
    let mut out = Vec::with_capacity(args.len().min(64));
    for a in args {
        let s = to_js_text(a, ctx)?;
        total = total.saturating_add(s.len());
        if total > MAX_TEXT_UNITS {
            return Err(JsNativeError::range()
                .with_message(format!("the arguments are too long to pass to the host together (more than {MAX_TEXT_UNITS} UTF-16 units)"))
                .into());
        }
        out.push(text(&s)?);
    }
    Ok(out)
}

fn to_js_text(v: &JsValue, ctx: &mut Context) -> JsResult<JsString> {
    let s = match v.as_string() {
        Some(s) => s,
        None => {
            let mut join = Join::default();
            join.value(v, ctx)?;
            JsString::from(join.out.as_slice())
        }
    };
    if s.len() > MAX_TEXT_UNITS {
        return Err(too_long());
    }
    Ok(s)
}

/// `v` as a script value. The engine converts by recursion, so a value nested deeper than
/// [`MAX_DEPTH`] is refused first (found without recursion).
pub fn from_json(v: &Value, ctx: &mut Context) -> JsResult<JsValue> {
    if json_depth_over(v, MAX_DEPTH) {
        return Err(too_deep());
    }
    JsValue::from_json(v, ctx)
}

/// Whether `v` nests arrays and objects more than `max` levels deep.
fn json_depth_over(v: &Value, max: usize) -> bool {
    let mut stack = vec![(v, 0usize)];
    while let Some((v, depth)) = stack.pop() {
        let children: Box<dyn Iterator<Item = &Value>> = match v {
            Value::Array(a) => Box::new(a.iter()),
            Value::Object(o) => Box::new(o.values()),
            _ => continue,
        };
        if depth >= max {
            return true;
        }
        stack.extend(children.map(|c| (c, depth + 1)));
    }
    false
}

fn too_long() -> JsError {
    JsNativeError::range().with_message(format!("text is too long to pass to the host (more than {MAX_TEXT_UNITS} UTF-16 units)")).into()
}

fn too_large() -> JsError {
    JsNativeError::range().with_message(format!("value is too large to convert to text (more than {MAX_ITEMS} items)")).into()
}

fn too_deep() -> JsError {
    JsNativeError::range().with_message(format!("value is nested too deeply to convert to text (more than {MAX_DEPTH} levels)")).into()
}

fn no_primitive() -> JsError {
    JsNativeError::typ().with_message("cannot convert object to primitive value").into()
}

/// One conversion to text: the text so far, and the bounds it is held to.
#[derive(Default)]
struct Join {
    out: Vec<u16>,
    /// Array slots visited so far, along every path.
    slots: usize,
    /// The arrays being joined, outermost first.
    path: Vec<JsObject>,
}

/// What calling a `toString` or `valueOf` gave.
enum Converted {
    /// The intrinsic join already appended the text.
    Appended,
    /// A value to use if it is a primitive.
    Value(JsValue),
}

impl Join {
    /// Append `String(v)`.
    fn value(&mut self, v: &JsValue, ctx: &mut Context) -> JsResult<()> {
        let Some(o) = v.as_object() else {
            let s = v.to_string(ctx)?;
            return self.push(&s);
        };
        // ToPrimitive(o, string): Symbol.toPrimitive first.
        let exotic = o.get(JsSymbol::to_primitive(), ctx)?;
        if !exotic.is_null_or_undefined() {
            let f = exotic.as_callable().ok_or_else(|| JsError::from(JsNativeError::typ().with_message("Symbol.toPrimitive is not a function")))?;
            let r = f.call(&o.clone().into(), &[js_string!("string").into()], ctx)?;
            if r.is_object() {
                return Err(no_primitive());
            }
            let s = r.to_string(ctx)?;
            return self.push(&s);
        }
        // OrdinaryToPrimitive(o, string).
        for name in ["toString", "valueOf"] {
            let method = o.get(JsString::from(name), ctx)?;
            let Some(f) = method.as_callable() else { continue };
            match self.call(&o, &f, ctx)? {
                Converted::Appended => return Ok(()),
                Converted::Value(r) if !r.is_object() => {
                    let s = r.to_string(ctx)?;
                    return self.push(&s);
                }
                Converted::Value(_) => {}
            }
        }
        Err(no_primitive())
    }

    /// Call a conversion method of `o`. The intrinsic `Array.prototype.toString` is run here:
    /// it reads `join` once and, when that is the intrinsic join, joins within bounds. The
    /// intrinsic `join` called directly as the method is joined within bounds as well.
    fn call(&mut self, o: &JsObject, f: &JsObject, ctx: &mut Context) -> JsResult<Converted> {
        let Some(i) = ctx.realm().host_defined().get::<Intrinsics>().map(|i| (i.array_to_string.clone(), i.array_join.clone(), i.object_to_string.clone()))
        else {
            return Ok(Converted::Value(f.call(&o.clone().into(), &[], ctx)?));
        };
        let (array_to_string, array_join, object_to_string) = i;
        // The intrinsic join itself as the conversion method (`{ toString: [].join }`): joined
        // here too, with its default separator, never by the engine's recursion.
        if JsObject::equals(f, &array_join) {
            self.array(o, ctx)?;
            return Ok(Converted::Appended);
        }
        if !JsObject::equals(f, &array_to_string) {
            return Ok(Converted::Value(f.call(&o.clone().into(), &[], ctx)?));
        }
        let join = o.get(js_string!("join"), ctx)?;
        match join.as_callable() {
            Some(j) if JsObject::equals(&j, &array_join) => {
                self.array(o, ctx)?;
                Ok(Converted::Appended)
            }
            Some(j) => Ok(Converted::Value(j.call(&o.clone().into(), &[], ctx)?)),
            None => Ok(Converted::Value(object_to_string.call(&o.clone().into(), &[], ctx)?)),
        }
    }

    /// Append `Array.prototype.join.call(array)`: every element, `,` between them.
    fn array(&mut self, array: &JsObject, ctx: &mut Context) -> JsResult<()> {
        if self.path.iter().any(|o| JsObject::equals(o, array)) {
            // A cycle: every engine joins an array already being joined as ''.
            return Ok(());
        }
        if self.path.len() >= MAX_DEPTH {
            return Err(too_deep());
        }
        // LengthOfArrayLike, as `join` reads it (a getter or proxy trap on `length` runs once).
        let len = array.get(js_string!("length"), ctx)?.to_length(ctx)?;
        let len = usize::try_from(len).map_err(|_| too_large())?;
        self.slots = self.slots.checked_add(len).filter(|&n| n <= MAX_ITEMS).ok_or_else(too_large)?;
        self.path.push(array.clone());
        let mut result = Ok(());
        for i in 0..len {
            if i > 0 {
                // The separator counts too: past the limit, no further element is read.
                if self.out.len() >= MAX_TEXT_UNITS {
                    result = Err(too_long());
                    break;
                }
                self.out.push(u16::from(b','));
            }
            let element = match array.get(i, ctx) {
                Ok(e) => e,
                Err(e) => {
                    result = Err(e);
                    break;
                }
            };
            if element.is_null_or_undefined() {
                continue;
            }
            if let Err(e) = self.value(&element, ctx) {
                result = Err(e);
                break;
            }
        }
        self.path.pop();
        result
    }

    fn push(&mut self, s: &JsString) -> JsResult<()> {
        if self.out.len().saturating_add(s.len()) > MAX_TEXT_UNITS {
            return Err(too_long());
        }
        self.out.extend(s.iter());
        Ok(())
    }
}

/// The text of an error that ended a run, at most [`MAX_ERROR_CHARS`] characters plus a cut
/// marker. It is built without running script code. The engine's own text (with its backtrace)
/// is kept only when it is known to be safe to build: the engine formats every character of a
/// message and recurses into any object it shows, so a huge message took gigabytes and a deep
/// object overflowed the stack. Otherwise:
/// - an Error with a long or non-string name or message, or one whose name or message could not
///   be found within [`MAX_PROTO_HOPS`] prototypes, is `"Name: message"`, each part cut, and a
///   part that is not a string is left out;
/// - a long string or symbol description is cut;
/// - any other object is `Uncaught exception: <message>` when it has a string `message` data
///   property, else [`NOT_AN_ERROR`].
pub fn error_text(e: &JsError) -> String {
    let Some(v) = e.as_opaque() else { return cut(&e.to_string(), MAX_ERROR_CHARS) };
    if let Some(s) = v.as_string() {
        if s.len() > MAX_ERROR_CHARS {
            return cut_shortened(&format!("\"{}", s.slice(0, whole(&s, MAX_ERROR_CHARS)).to_std_string_escaped()), MAX_ERROR_CHARS);
        }
    } else if let Some(sym) = v.as_symbol() {
        if let Some(d) = sym.description().filter(|d| d.len() > MAX_ERROR_CHARS) {
            return cut_shortened(&format!("Symbol({}", d.slice(0, whole(&d, MAX_ERROR_CHARS)).to_std_string_escaped()), MAX_ERROR_CHARS);
        }
    } else if let Some(obj) = v.as_object() {
        let name = data_on_chain(&obj, &js_string!("name").into());
        let message = data_on_chain(&obj, &js_string!("message").into());
        if !obj.is::<ErrorData>() {
            return match message.string() {
                Some(m) => {
                    let (m, shortened) = short(&m);
                    marked(&format!("Uncaught exception: {m}"), MAX_ERROR_CHARS, shortened)
                }
                None => NOT_AN_ERROR.to_owned(),
            };
        }
        // Safe for the engine to format: each part absent, or a string of bounded length. An
        // incomplete search is not proof of absence: the engine searches farther and may find a
        // value that formats without bound.
        let small = |p: &Lookup| match p {
            Lookup::Absent => true,
            Lookup::Data(v) => v.as_string().is_some_and(|s| s.len() <= MAX_ERROR_CHARS),
            Lookup::Unknown => false,
        };
        if !small(&name) || !small(&message) {
            let name = name.string().map_or_else(
                || "Error".to_owned(),
                |n| {
                    let (n, shortened) = short(&n);
                    marked(&n, 100, shortened)
                },
            );
            return match message.string().map(|m| short(&m)) {
                Some((m, shortened)) if !m.is_empty() => marked(&format!("{name}: {m}"), MAX_ERROR_CHARS, shortened),
                _ => name,
            };
        }
    }
    cut(&e.to_string(), MAX_ERROR_CHARS)
}

/// At most [`MAX_ERROR_CHARS`] UTF-16 units of `s`, and whether some were left out: a text of
/// characters outside the BMP can lose units yet hold fewer characters than [`cut`] counts.
fn short(s: &JsString) -> (String, bool) {
    let shortened = s.len() > MAX_ERROR_CHARS;
    (s.slice(0, whole(s, s.len().min(MAX_ERROR_CHARS))).to_std_string_escaped(), shortened)
}

/// `n`, or one less when the first `n` units of `s` would end inside a surrogate pair: a cut
/// keeps whole characters.
fn whole(s: &JsString, n: usize) -> usize {
    let splits = n > 0 && n < s.len() && matches!(s.code_point_at(n - 1), boa_engine::string::CodePoint::Unicode(c) if c.len_utf16() == 2);
    if splits { n - 1 } else { n }
}

/// Prototype hops when reading Error data; enough for a subclass, short of a cycle.
const MAX_PROTO_HOPS: usize = 32;

/// What [`data_on_chain`] found.
pub(crate) enum Lookup {
    /// A data property, with its value.
    Data(JsValue),
    /// No property within the chain searched, and the chain ended.
    Absent,
    /// The search stopped short: a proxy (whose traps are not run) or a chain longer than
    /// [`MAX_PROTO_HOPS`]. The property may still exist.
    Unknown,
}

impl Lookup {
    /// The value as a string, when it was found and is one.
    pub(crate) fn string(&self) -> Option<JsString> {
        match self {
            Lookup::Data(v) => v.as_string(),
            _ => None,
        }
    }
}

/// A data property of `obj` or of its first [`MAX_PROTO_HOPS`] prototypes. Accessors are not run:
/// one met on the way counts as [`Lookup::Unknown`], as does a proxy.
pub(crate) fn data_on_chain(obj: &JsObject, key: &PropertyKey) -> Lookup {
    let mut at = Some(obj.clone());
    for _ in 0..MAX_PROTO_HOPS {
        let Some(o) = at else { return Lookup::Absent };
        if JsProxy::from_object(o.clone()).is_ok() {
            return Lookup::Unknown;
        }
        if let Some(d) = o.borrow().properties().get(key) {
            return d.value().cloned().map_or(Lookup::Unknown, Lookup::Data);
        }
        at = o.prototype();
    }
    Lookup::Unknown
}

/// [`cut`], marked as cut also when `shortened` says text was left out before: one marker,
/// always at the end.
fn marked(s: &str, max: usize, shortened: bool) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}… (cut)", &s[..at]),
        None if shortened => format!("{s}… (cut)"),
        None => s.to_owned(),
    }
}

/// [`marked`] for a text already shortened.
fn cut_shortened(s: &str, max: usize) -> String {
    marked(s, max, true)
}

/// The first `max` characters of `s`, with a marker when it was longer.
pub(crate) fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}… (cut)", &s[..at]),
        None => s.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_nested_past_the_limit_is_refused_without_recursion() {
        let mut ctx = Context::default();
        let mut deep = Value::Null;
        for _ in 0..200_000 {
            deep = Value::Array(vec![deep]);
        }
        let err = from_json(&deep, &mut ctx).unwrap_err();
        assert!(err.to_string().contains("nested too deeply"), "{err}");
        // serde_json's own drop recursion is not ours to fix: leak the test value.
        std::mem::forget(deep);
        let mut ok = json_of_depth(MAX_DEPTH - 1);
        assert!(from_json(&ok, &mut ctx).is_ok());
        ok = json_of_depth(MAX_DEPTH + 1);
        assert!(from_json(&ok, &mut ctx).is_err());
    }

    fn json_of_depth(n: usize) -> Value {
        let mut v = Value::Null;
        for i in 0..n {
            v = if i % 2 == 0 { Value::Array(vec![v]) } else { serde_json::json!({ "k": v }) };
        }
        v
    }
}
