//! ScriptUI: the windows and controls a script builds with `new Window(...)` and `add(...)`.
//!
//! Every window and control is an ordinary object the script can read and write (`text`,
//! `value`, `enabled`, `selection`, `onClick`, …), made from one shared prototype and tagged
//! with native data the script cannot forge. Which control holds which is kept here, in the
//! realm's host data, not in the objects, so a script that rewrites `children` cannot build a
//! cycle or reach another window's controls.
//!
//! A dialog's `show()` runs an event loop: it sends the window's controls to the UI as a
//! [`ModalKind::Window`] modal, applies the values the user changed, and runs the handler of
//! the control the user clicked or edited, live, in the same context, until the window is
//! closed. On a host that can wait (the script host process) each event costs one round trip;
//! a host that cannot wait replays the run up to the event instead, like any other dialog.
//! Palettes do not wait: `show()`, `update()` and `app.refresh()` send their controls and
//! return at once, and their handlers do not run.

use boa_engine::object::builtins::JsArray;
use boa_engine::object::{FunctionObjectBuilder, ObjectInitializer};
use boa_engine::property::Attribute;
use boa_engine::{Context, Finalize, JsData, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, Trace, js_string};
use serde_json::{Map, Value, json};

use crate::ModalKind;
use crate::natives::{error, modal_value, window_update};
use crate::value::{self, MAX_TEXT_UNITS};

type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

/// Windows and controls one run may create in all.
pub const MAX_ELEMENTS: usize = 4096;
/// Items one list may hold.
pub const MAX_ITEMS: usize = 10_000;
/// Items all of a run's lists may hold together, closed windows' included (a run's controls are
/// never freed): a dialog per document with a long font list still fits.
pub const MAX_TOTAL_ITEMS: usize = 250_000;
/// How deep controls may be nested in a window. Real dialogs nest a few levels; the bound keeps
/// every window well inside what the script host's messages can carry (`serde_json` refuses
/// input nested more than 128 arrays and objects deep, and each level here is two).
pub const MAX_DEPTH: usize = 16;

/// `show()`'s result when the window was closed by its default (OK) button or by `close()`
/// with no value.
const RESULT_OK: i32 = 1;
/// `show()`'s result when the window was cancelled: its cancel button, Escape, or its close box.
const RESULT_CANCEL: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Window,
    Panel,
    Group,
    Button,
    Checkbox,
    Radio,
    EditText,
    StaticText,
    Dropdown,
    ListBox,
    Progress,
    Slider,
}

const KINDS: [(Kind, &str); 12] = [
    (Kind::Window, "window"),
    (Kind::Panel, "panel"),
    (Kind::Group, "group"),
    (Kind::Button, "button"),
    (Kind::Checkbox, "checkbox"),
    (Kind::Radio, "radiobutton"),
    (Kind::EditText, "edittext"),
    (Kind::StaticText, "statictext"),
    (Kind::Dropdown, "dropdownlist"),
    (Kind::ListBox, "listbox"),
    (Kind::Progress, "progressbar"),
    (Kind::Slider, "slider"),
];

impl Kind {
    fn name(self) -> &'static str {
        KINDS.iter().find(|(k, _)| *k == self).map_or("group", |(_, n)| n)
    }

    fn code(self) -> u8 {
        KINDS.iter().position(|(k, _)| *k == self).and_then(|i| u8::try_from(i).ok()).unwrap_or(0)
    }

    fn from_code(code: u8) -> Self {
        KINDS.get(usize::from(code)).map_or(Kind::Group, |(k, _)| *k)
    }

    /// The control a script names in `add(type, …)`. `iconbutton` is drawn as a button.
    fn parse(name: &str) -> Option<Self> {
        match name {
            "iconbutton" => Some(Kind::Button),
            "window" => None,
            _ => KINDS.iter().find(|(_, n)| *n == name).map(|(k, _)| *k),
        }
    }

    fn container(self) -> bool {
        matches!(self, Kind::Window | Kind::Panel | Kind::Group)
    }

    fn list(self) -> bool {
        matches!(self, Kind::Dropdown | Kind::ListBox)
    }
}

/// Marks a window or control: its index in [`Registry::elements`].
#[derive(Debug, Clone, Copy, Trace, Finalize, JsData)]
// Sound: holds only an integer, no GC pointers.
#[boa_gc(empty_trace)]
struct UiRef(u32);

/// Marks an item of a list.
#[derive(Debug, Clone, Copy, Trace, Finalize, JsData)]
// Sound: holds no GC pointers.
#[boa_gc(empty_trace)]
struct ItemRef;

#[derive(Trace, Finalize)]
struct Element {
    obj: JsObject,
    /// A list's items, in order.
    items: Vec<JsObject>,
    /// The array its `items` property holds, kept so an added item is appended, not the whole
    /// array rebuilt.
    items_array: Option<JsObject>,
    /// A list's selected item (its `selection`).
    selection: Option<JsObject>,
    /// Lists: the items as the window last sent them, so the index the user chose names the item
    /// they saw even when a getter of the script's changed the list while the window was read.
    sent: Option<Vec<JsObject>>,
    /// Lists: counts the assignments to `selection`, so one an item's `selected` setter makes
    /// meanwhile is not undone by the assignment it interrupted.
    selections: u64,
    children: Vec<u32>,
    parent: Option<u32>,
    /// The window this control is in (a window's own id for a window).
    window: u32,
    kind: u8,
    /// Windows: a palette (or `"window"`), which does not wait for the user.
    palette: bool,
    /// Windows: shown and not closed since.
    shown: bool,
    /// Windows: counts the shows and hides the app was told of, so a failed `show()` does not
    /// undo one a handler made meanwhile.
    visibility: u64,
    /// Windows: closed while its `show()` waits; `result` is what `show()` returns.
    closed: bool,
    /// Dialogs: inside their `show()`, so a handler's own `show()` of the same dialog can't
    /// reset the `closed` and `result` the outer one waits on.
    showing: bool,
    result: JsValue,
}

/// All windows and controls of the current attempt. A replay builds them again, in the same
/// order, so their ids match the ones the UI saw.
#[derive(Trace, Finalize, JsData)]
struct Registry {
    elements: Vec<Element>,
    /// Items in all lists ([`MAX_TOTAL_ITEMS`]).
    #[unsafe_ignore_trace]
    items: usize,
    proto: JsObject,
    item_proto: JsObject,
}

fn with_registry<R>(ctx: &Context, f: impl FnOnce(&mut Registry) -> R) -> JsResult<R> {
    let realm = ctx.realm().clone();
    let mut data = realm.host_defined_mut();
    let reg = data.get_mut::<Registry>().ok_or_else(|| error("ScriptUI is not available"))?;
    Ok(f(reg))
}

fn element<R>(ctx: &Context, id: u32, f: impl FnOnce(&Element) -> R) -> JsResult<R> {
    with_registry(ctx, |r| r.elements.get(id as usize).map(f))?.ok_or_else(|| error("no such ScriptUI control"))
}

fn element_mut<R>(ctx: &Context, id: u32, f: impl FnOnce(&mut Element) -> R) -> JsResult<R> {
    with_registry(ctx, |r| r.elements.get_mut(id as usize).map(f))?.ok_or_else(|| error("no such ScriptUI control"))
}

fn kind_of(ctx: &Context, id: u32) -> JsResult<Kind> {
    element(ctx, id, |e| Kind::from_code(e.kind))
}

fn ui_id(v: &JsValue) -> Option<u32> {
    v.as_object().and_then(|o| o.downcast_ref::<UiRef>().map(|r| r.0))
}

fn this_id(this: &JsValue) -> JsResult<u32> {
    ui_id(this).ok_or_else(|| JsNativeError::typ().with_message("not a ScriptUI window or control").into())
}

fn is_item(v: &JsValue) -> bool {
    v.as_object().is_some_and(|o| o.downcast_ref::<ItemRef>().is_some())
}

fn string(s: &str) -> JsValue {
    JsValue::from(JsString::from(s))
}

fn get(obj: &JsObject, key: &str, ctx: &mut Context) -> JsResult<JsValue> {
    obj.get(JsString::from(key), ctx)
}

fn set(obj: &JsObject, key: &str, v: impl Into<JsValue>, ctx: &mut Context) -> JsResult<()> {
    obj.set(JsString::from(key), v.into(), true, ctx).map(|_| ())
}

/// `String(v)` for a single value; `undefined` and `null` become empty.
fn text_of(v: &JsValue, ctx: &mut Context) -> JsResult<String> {
    if v.is_undefined() || v.is_null() {
        return Ok(String::new());
    }
    Ok(value::texts(std::slice::from_ref(v), ctx)?.concat())
}

/// A finite number, or `None` for `undefined`, `null` and what does not convert to one.
fn number_of(v: &JsValue, ctx: &mut Context) -> JsResult<Option<f64>> {
    if v.is_undefined() || v.is_null() {
        return Ok(None);
    }
    let n = v.to_number(ctx)?;
    Ok(n.is_finite().then_some(n))
}

fn plain_object(ctx: &mut Context) -> JsObject {
    JsObject::with_object_proto(ctx.intrinsics())
}

fn array(values: Vec<JsValue>, ctx: &mut Context) -> JsValue {
    JsArray::from_iter(values, ctx).into()
}

// ---------- building ----------

/// Make a window or control. `parent` is `None` for a window.
fn create(ctx: &mut Context, kind: Kind, parent: Option<u32>, type_name: &str, text: &JsValue, props: &JsValue) -> JsResult<JsObject> {
    let (id, proto, window) = with_registry(ctx, |r| {
        let id = r.elements.len();
        let window = parent.and_then(|p| r.elements.get(p as usize)).map(|p| p.window);
        (id, r.proto.clone(), window)
    })?;
    if id >= MAX_ELEMENTS {
        return Err(JsNativeError::range().with_message(format!("a script can create at most {MAX_ELEMENTS} ScriptUI windows and controls")).into());
    }
    let id = u32::try_from(id).map_err(|_| error("too many ScriptUI controls"))?;
    let window = window.unwrap_or(id);
    let obj = JsObject::from_proto_and_data(Some(proto), UiRef(id));
    with_registry(ctx, |r| {
        r.elements.push(Element {
            obj: obj.clone(),
            items: Vec::new(),
            items_array: None,
            selection: None,
            selections: 0,
            sent: None,
            children: Vec::new(),
            parent,
            window,
            kind: kind.code(),
            palette: false,
            shown: false,
            visibility: 0,
            closed: false,
            showing: false,
            result: JsValue::undefined(),
        });
    })?;
    if let Some(p) = parent {
        element_mut(ctx, p, |e| e.children.push(id))?;
        sync_children(ctx, p)?;
    }

    set(&obj, "type", string(type_name), ctx)?;
    let text = text_of(text, ctx)?;
    set(&obj, "text", string(&text), ctx)?;
    set(&obj, "enabled", true, ctx)?;
    set(&obj, "visible", true, ctx)?;
    set(&obj, "helpTip", string(""), ctx)?;
    let props = match props.as_object() {
        Some(p) => p.clone(),
        None => plain_object(ctx),
    };
    set(&obj, "properties", props, ctx)?;
    let parent_obj = match parent {
        Some(p) => JsValue::from(element(ctx, p, |e| e.obj.clone())?),
        None => JsValue::null(),
    };
    set(&obj, "parent", parent_obj, ctx)?;
    let window_obj = element(ctx, window, |e| e.obj.clone())?;
    set(&obj, "window", window_obj, ctx)?;
    let size = plain_object(ctx);
    set(&size, "width", -1, ctx)?;
    set(&size, "height", -1, ctx)?;
    set(&obj, "preferredSize", size, ctx)?;
    if kind.container() {
        let children = array(Vec::new(), ctx);
        set(&obj, "children", children, ctx)?;
        set(&obj, "orientation", string(if kind == Kind::Group { "row" } else { "column" }), ctx)?;
    }
    match kind {
        Kind::Window => {
            set(&obj, "defaultElement", JsValue::null(), ctx)?;
            set(&obj, "cancelElement", JsValue::null(), ctx)?;
        }
        Kind::Checkbox | Kind::Radio => set(&obj, "value", false, ctx)?,
        Kind::Dropdown | Kind::ListBox => {
            let items = array(Vec::new(), ctx);
            set(&obj, "items", items, ctx)?;
            // An accessor, as in ScriptUI: `list.selection = 0` selects the first item, and
            // `list.selection` is then that item (`.text`, `.index`), not the number.
            let get = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(selection_get)).name(js_string!("selection")).build();
            let put = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(selection_set)).name(js_string!("selection")).length(1).build();
            let desc = boa_engine::property::PropertyDescriptor::builder().get(get).set(put).enumerable(true).configurable(true);
            obj.define_property_or_throw(js_string!("selection"), desc, ctx)?;
        }
        Kind::Progress | Kind::Slider => {
            set(&obj, "value", 0, ctx)?;
            set(&obj, "minvalue", 0, ctx)?;
            set(&obj, "maxvalue", 100, ctx)?;
        }
        _ => {}
    }
    let layout = ObjectInitializer::new(ctx)
        .function(NativeFunction::from_fn_ptr(nothing), js_string!("layout"), 1)
        .function(NativeFunction::from_fn_ptr(nothing), js_string!("resize"), 0)
        .build();
    set(&obj, "layout", layout, ctx)?;
    Ok(obj)
}

fn sync_children(ctx: &mut Context, id: u32) -> JsResult<()> {
    let (obj, kids) = with_registry(ctx, |r| {
        let e = r.elements.get(id as usize)?;
        let kids: Vec<JsValue> = e.children.iter().filter_map(|c| r.elements.get(*c as usize)).map(|c| c.obj.clone().into()).collect();
        Some((e.obj.clone(), kids))
    })?
    .ok_or_else(|| error("no such ScriptUI control"))?;
    let kids = array(kids, ctx);
    set(&obj, "children", kids, ctx)
}

fn sync_items(ctx: &mut Context, id: u32) -> JsResult<()> {
    let items = element(ctx, id, |e| e.items.clone())?;
    for (i, item) in items.iter().enumerate() {
        set(item, "index", i as f64, ctx)?;
    }
    // Read again: an `index` setter of the script's may have changed the list meanwhile.
    let (obj, items) = element(ctx, id, |e| (e.obj.clone(), e.items.clone()))?;
    let values: Vec<JsValue> = items.into_iter().map(JsValue::from).collect();
    let values = JsArray::from_iter(values, ctx);
    element_mut(ctx, id, |e| e.items_array = Some(values.clone().into()))?;
    set(&obj, "items", JsValue::from(values), ctx)
}

/// Count `n` more items (negative: fewer) against [`MAX_TOTAL_ITEMS`]; `false` when that would
/// pass it.
fn count_items(ctx: &Context, n: isize) -> JsResult<bool> {
    with_registry(ctx, |r| match r.items.checked_add_signed(n).filter(|t| *t <= MAX_TOTAL_ITEMS) {
        Some(t) => {
            r.items = t;
            true
        }
        None if n < 0 => {
            r.items = 0;
            true
        }
        None => false,
    })
}

fn add_item(ctx: &mut Context, list: u32, text: &JsValue, separator: bool) -> JsResult<JsValue> {
    // Converted first: the text's toString() may itself add items. The item is counted and
    // added in one step, before anything else of the script's (a setter reached through the
    // prototype) can run, so nothing it does can push the list past its limit.
    let text = if separator { "-".to_owned() } else { text_of(text, ctx)? };
    let proto = with_registry(ctx, |r| r.item_proto.clone())?;
    let item = JsObject::from_proto_and_data(Some(proto), ItemRef);
    let count = element_mut(ctx, list, |e| {
        let count = e.items.len();
        if count < MAX_ITEMS {
            e.items.push(item.clone());
        }
        count
    })?;
    if count >= MAX_ITEMS {
        return Err(JsNativeError::range().with_message(format!("a list can hold at most {MAX_ITEMS} items")).into());
    }
    if !count_items(ctx, 1)? {
        element_mut(ctx, list, |e| e.items.retain(|i| !JsObject::equals(i, &item)))?;
        return Err(JsNativeError::range().with_message(format!("a script's lists can hold at most {MAX_TOTAL_ITEMS} items in all")).into());
    }
    let filled = (|| -> JsResult<()> {
        set(&item, "text", string(&text), ctx)?;
        set(&item, "type", string(if separator { "separator" } else { "item" }), ctx)?;
        set(&item, "selected", false, ctx)?;
        set(&item, "index", count as f64, ctx)
    })();
    // A setter of the script's that throws leaves no half-made item in the list.
    if let Err(e) = filled {
        unadd(ctx, list, &item)?;
        return Err(e);
    }
    // Appended to the list's own array, so adding n items is linear, not quadratic; by
    // definition, not assignment, so no setter of the script's runs here. Only while that array
    // still matches the list item for item up to the new one; otherwise (the script replaced
    // `items`, or a setter of its added or removed items meanwhile) the array is made again.
    let (obj, len, kept) = element(ctx, list, |e| (e.obj.clone(), e.items.len(), e.items_array.clone()))?;
    let published = (|| -> JsResult<()> {
        match kept.and_then(|a| JsArray::from_object(a).ok()) {
            Some(arr) if len == count + 1 && arr.length(ctx)? == count as u64 && arr.create_data_property(count, item.clone(), ctx)? => {
                set(&obj, "items", JsValue::from(arr), ctx)
            }
            _ => sync_items(ctx, list),
        }
    })();
    // A setter of the script's on `items` that throws leaves the item out too: out of the list,
    // and out of the array the list keeps for `items`, which already held it.
    if let Err(e) = published {
        unadd(ctx, list, &item)?;
        unpublish(ctx, list, &item)?;
        return Err(e);
    }
    Ok(item.into())
}

/// Take `item` out of the array `list` keeps for its `items`, by identity, keeping the rest (an
/// item a setter of the script's added meanwhile included). By definition, not assignment, so
/// no setter of the script's runs.
fn unpublish(ctx: &mut Context, list: u32, item: &JsObject) -> JsResult<()> {
    let Some(arr) = element(ctx, list, |e| e.items_array.clone())?.and_then(|a| JsArray::from_object(a).ok()) else { return Ok(()) };
    let len = arr.length(ctx)?;
    let mut kept = Vec::new();
    let mut found = false;
    for i in 0..len {
        let v = arr.get(i, ctx)?;
        if v.as_object().is_some_and(|o| JsObject::equals(&o, item)) {
            found = true;
        } else {
            kept.push(v);
        }
    }
    if !found {
        return Ok(());
    }
    let kept_len = kept.len();
    for (i, v) in kept.into_iter().enumerate() {
        arr.create_data_property_or_throw(i, v, ctx)?;
    }
    // An array's own `length`: shortening it drops the elements left over past the kept ones
    // (the item may have been in it more than once).
    let kept_len = kept_len as u64;
    arr.set(js_string!("length"), kept_len, true, ctx)?;
    Ok(())
}

/// Take back `item`, added to `list` by an `add` that failed: out of the list, and its count
/// given back unless a setter of the script's removed it already (`remove` gives it back).
fn unadd(ctx: &mut Context, list: u32, item: &JsObject) -> JsResult<()> {
    let removed = element_mut(ctx, list, |e| {
        let before = e.items.len();
        e.items.retain(|i| !JsObject::equals(i, item));
        // A setter that chose it meanwhile chose an item that is gone; another choice stays.
        if e.selection.as_ref().is_some_and(|s| JsObject::equals(s, item)) {
            e.selection = None;
        }
        e.items.len() != before
    })?;
    if removed {
        count_items(ctx, -1)?;
    }
    Ok(())
}

/// `new Window(type, title, bounds, properties)`.
fn window_ctor(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let style = text_of(args.first().unwrap_or(&JsValue::undefined()), ctx)?;
    if style.contains('{') {
        return Err(error("ScriptUI resource strings are not supported yet; build the window with add()"));
    }
    let style = style.trim().to_ascii_lowercase();
    let palette = match style.as_str() {
        "dialog" => false,
        "palette" | "window" => true,
        _ => return Err(JsNativeError::typ().with_message(format!("unknown window type \"{style}\" (use \"dialog\" or \"palette\")")).into()),
    };
    let title = args.get(1).cloned().unwrap_or_default();
    let props = args.get(3).cloned().unwrap_or_default();
    let bounds = copy_bounds(ctx, args.get(2))?;
    let obj = create(ctx, Kind::Window, None, &style, &title, &props)?;
    keep_bounds(ctx, &obj, bounds)?;
    let id = this_id(&obj.clone().into())?;
    element_mut(ctx, id, |e| e.palette = palette)?;
    Ok(obj.into())
}

/// A copy of the `bounds` given to `new Window` or `add`, to keep as the control's `bounds`
/// ([`keep_bounds`]), which sizes it as one set afterwards would ([`size_hint`]). Read before the
/// control is made, so a getter that throws leaves no control behind. ScriptUI keeps its own
/// copy too: changing the script's array afterwards does not resize the control.
fn copy_bounds(ctx: &mut Context, bounds: Option<&JsValue>) -> JsResult<Option<JsValue>> {
    let Some(b) = bounds.and_then(JsValue::as_object).filter(|b| !b.is_callable()) else { return Ok(None) };
    if b.is_array() {
        let mut values = Vec::with_capacity(4);
        for i in 0..4u32 {
            values.push(b.get(i, ctx)?);
        }
        return Ok(Some(JsArray::from_iter(values, ctx).into()));
    }
    let copy = JsObject::with_object_proto(ctx.intrinsics());
    for key in ["x", "y", "width", "height", "left", "top", "right", "bottom"] {
        let v = get(&b, key, ctx)?;
        if !v.is_undefined() {
            set(&copy, key, v, ctx)?;
        }
    }
    Ok(Some(copy.into()))
}

fn keep_bounds(ctx: &mut Context, obj: &JsObject, bounds: Option<JsValue>) -> JsResult<()> {
    match bounds {
        Some(b) => set(obj, "bounds", b, ctx),
        None => Ok(()),
    }
}

/// `container.add(type, bounds, text, properties)`, `list.add("item", text)`.
fn add(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this)?;
    let parent = kind_of(ctx, id)?;
    let name = text_of(args.first().unwrap_or(&JsValue::undefined()), ctx)?.trim().to_ascii_lowercase();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    if parent.list() {
        return match name.as_str() {
            "item" => add_item(ctx, id, &arg(1), false),
            "separator" => add_item(ctx, id, &JsValue::undefined(), true),
            _ => Err(error(format!("a {} can only add \"item\" or \"separator\"", parent.name()))),
        };
    }
    if !parent.container() {
        return Err(error(format!("a {} cannot hold other controls", parent.name())));
    }
    if name.contains('{') {
        return Err(error("ScriptUI resource strings are not supported yet; add one control at a time"));
    }
    let kind = Kind::parse(&name).ok_or_else(|| error(format!("the ScriptUI control \"{name}\" is not supported")))?;
    let depth = with_registry(ctx, |r| {
        let mut depth = 1;
        let mut at = Some(id);
        while let Some(p) = at.and_then(|i| r.elements.get(i as usize)).and_then(|e| e.parent) {
            depth += 1;
            at = Some(p);
            if depth > MAX_DEPTH {
                break;
            }
        }
        depth
    })?;
    // `depth` is the new control's level; the window is level 0.
    if depth > MAX_DEPTH {
        return Err(JsNativeError::range().with_message(format!("ScriptUI controls can be nested at most {MAX_DEPTH} deep")).into());
    }
    let third = arg(2);
    let (text, props) = match kind {
        // add("progressbar", bounds, value, max) and add("slider", bounds, value, min, max).
        Kind::Progress | Kind::Slider => (JsValue::undefined(), JsValue::undefined()),
        Kind::Dropdown | Kind::ListBox if third.is_object() && !third.is_callable() => (JsValue::undefined(), arg(3)),
        _ => (third.clone(), arg(3)),
    };
    let bounds = copy_bounds(ctx, args.get(1))?;
    // A list's items are counted before the list is made, so one past the limit adds nothing.
    let items = match kind {
        Kind::Dropdown | Kind::ListBox => third.as_object().filter(|_| !third.is_callable()),
        _ => None,
    };
    let len = match &items {
        Some(list) => list.get(js_string!("length"), ctx)?.to_length(ctx)?,
        None => 0,
    };
    if len > MAX_ITEMS as u64 {
        return Err(JsNativeError::range().with_message(format!("a list can hold at most {MAX_ITEMS} items")).into());
    }
    let next = with_registry(ctx, |r| r.elements.len())?;
    let made = (|| -> JsResult<JsObject> {
        let obj = create(ctx, kind, Some(id), kind.name(), &text, &props)?;
        keep_bounds(ctx, &obj, bounds)?;
        init_control(ctx, &obj, kind, &third, items.as_ref(), len, args)?;
        Ok(obj)
    })();
    match made {
        Ok(obj) => Ok(obj.into()),
        Err(e) => {
            // A setter or conversion of the script's that throws leaves the parent as it was:
            // the half-made control is taken out again, and its items given back.
            if let Ok(child) = u32::try_from(next)
                && with_registry(ctx, |r| r.elements.len() > next)?
            {
                // The setup's error is the one the script sees, unless the run was stopped while
                // the parent's `children` were published again.
                if let Err(stopped) = detach(ctx, id, child)
                    && stopped.as_engine().is_some()
                {
                    return Err(stopped);
                }
            }
            Err(e)
        }
    }
}

/// Take control `child` out of `parent`, giving back the items it and its controls hold.
fn detach(ctx: &mut Context, parent: u32, child: u32) -> JsResult<()> {
    let held = element_mut(ctx, parent, |e| {
        let before = e.children.len();
        e.children.retain(|c| *c != child);
        e.children.len() != before
    })?;
    // The native side first: a setter of the script's on `children` that throws must not leave
    // the child linked or its items counted.
    let mut stack = vec![child];
    let mut seen = 0usize;
    while let Some(at) = stack.pop() {
        seen += 1;
        if seen > MAX_ELEMENTS {
            break;
        }
        let (items, children) = element_mut(ctx, at, |e| {
            if at == child {
                e.parent = None;
            }
            (std::mem::take(&mut e.items).len(), e.children.clone())
        })?;
        if items > 0 {
            count_items(ctx, -(items as isize))?;
        }
        stack.extend(children);
    }
    if held {
        sync_children(ctx, parent)?;
    }
    Ok(())
}

/// The rest of `add(type, bounds, …)` once control `obj` is made: a progress bar's or slider's
/// values, a list's items.
fn init_control(ctx: &mut Context, obj: &JsObject, kind: Kind, third: &JsValue, items: Option<&JsObject>, len: u64, args: &[JsValue]) -> JsResult<()> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    let child = this_id(&obj.clone().into())?;
    match kind {
        Kind::Progress => {
            if let Some(v) = number_of(third, ctx)? {
                set(obj, "value", v, ctx)?;
            }
            if let Some(max) = number_of(&arg(3), ctx)? {
                set(obj, "maxvalue", max, ctx)?;
            }
        }
        Kind::Slider => {
            if let Some(v) = number_of(third, ctx)? {
                set(obj, "value", v, ctx)?;
            }
            if let Some(min) = number_of(&arg(3), ctx)? {
                set(obj, "minvalue", min, ctx)?;
            }
            if let Some(max) = number_of(&arg(4), ctx)? {
                set(obj, "maxvalue", max, ctx)?;
            }
        }
        Kind::Dropdown | Kind::ListBox => {
            if let Some(list) = items {
                for i in 0..len {
                    let v = list.get(i, ctx)?;
                    add_item(ctx, child, &v, false)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// `container.remove(control or index)`, `list.remove(item or index)`.
fn remove(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this)?;
    let kind = kind_of(ctx, id)?;
    let what = args.first().cloned().unwrap_or_default();
    if kind.list() {
        // An index is converted before the items are read: a `valueOf` may change the list.
        let index = if is_item(&what) { None } else { number_of(&what, ctx)?.filter(|n| *n >= 0.0) };
        let items = element(ctx, id, |e| e.items.clone())?;
        let at = match what.as_object() {
            Some(o) if is_item(&what) => items.iter().position(|i| JsObject::equals(i, &o)),
            _ => index.map(|n| n as usize).filter(|i| *i < items.len()),
        };
        let Some(at) = at else { return Ok(JsValue::undefined()) };
        let removed = element_mut(ctx, id, |e| (at < e.items.len()).then(|| e.items.remove(at)))?;
        if removed.is_some() {
            count_items(ctx, -1)?;
        }
        if let (Some(removed), Some(obj)) = (removed, this.as_object()) {
            let sel = get(&obj, "selection", ctx)?;
            if sel.as_object().is_some_and(|s| JsObject::equals(&s, &removed)) {
                set(&obj, "selection", JsValue::null(), ctx)?;
            }
        }
        sync_items(ctx, id)?;
        return Ok(JsValue::undefined());
    }
    if !kind.container() {
        return Ok(JsValue::undefined());
    }
    let index = if ui_id(&what).is_some() { None } else { number_of(&what, ctx)?.filter(|n| *n >= 0.0) };
    let children = element(ctx, id, |e| e.children.clone())?;
    let child = match ui_id(&what) {
        Some(c) => children.iter().position(|k| *k == c),
        None => index.map(|n| n as usize).filter(|i| *i < children.len()),
    };
    if let Some(at) = child {
        let gone = element_mut(ctx, id, |e| (at < e.children.len()).then(|| e.children.remove(at)))?;
        if let Some(gone) = gone {
            element_mut(ctx, gone, |e| e.parent = None)?;
        }
        sync_children(ctx, id)?;
    }
    Ok(JsValue::undefined())
}

/// `list.removeAll()`.
fn remove_all(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this)?;
    if kind_of(ctx, id)?.list() {
        // Emptied and unselected in one step, before anything of the script's runs: an `items`
        // setter that throws leaves no removed item selected, and one that adds and selects an
        // item keeps that selection.
        let n = element_mut(ctx, id, |e| {
            e.selection = None;
            std::mem::take(&mut e.items).len()
        })?;
        count_items(ctx, -isize::try_from(n).unwrap_or(isize::MAX))?;
        sync_items(ctx, id)?;
    }
    Ok(JsValue::undefined())
}

/// `list.find(text)`: the first item with this text, or `null`.
fn find(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this)?;
    let wanted = text_of(args.first().unwrap_or(&JsValue::undefined()), ctx)?;
    let items = element(ctx, id, |e| e.items.clone())?;
    for item in items {
        let t = get(&item, "text", ctx)?;
        if text_of(&t, ctx)? == wanted {
            return Ok(item.into());
        }
    }
    Ok(JsValue::null())
}

fn item_to_string(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let Some(obj) = this.as_object().filter(|_| is_item(this)) else { return Ok(string("")) };
    let t = get(&obj, "text", ctx)?;
    Ok(string(&text_of(&t, ctx)?))
}

fn nothing(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

// ---------- what the UI sees ----------

/// The window's controls as JSON for the UI.
fn serialize(ctx: &mut Context, window: u32) -> JsResult<Value> {
    let mut budget = MAX_TEXT_UNITS;
    let (default, cancel) = special_buttons(ctx, window)?;
    let mut tree = node(ctx, window, 0, &mut budget, (default, cancel))?;
    if let Value::Object(m) = &mut tree {
        let palette = element(ctx, window, |e| e.palette)?;
        m.insert("style".into(), json!(if palette { "palette" } else { "dialog" }));
    }
    Ok(tree)
}

fn node(ctx: &mut Context, id: u32, depth: usize, budget: &mut usize, special: (Option<u32>, Option<u32>)) -> JsResult<Value> {
    if depth > MAX_DEPTH {
        return Err(JsNativeError::range().with_message(format!("ScriptUI controls can be nested at most {MAX_DEPTH} deep")).into());
    }
    let (obj, kind, children, items) = element(ctx, id, |e| (e.obj.clone(), Kind::from_code(e.kind), e.children.clone(), e.items.clone()))?;
    let mut m = Map::new();
    m.insert("id".into(), json!(id));
    m.insert("type".into(), json!(kind.name()));
    let text = |key: &str, ctx: &mut Context, budget: &mut usize| -> JsResult<String> {
        let v = get(&obj, key, ctx)?;
        let t = text_of(&v, ctx)?;
        let units = t.encode_utf16().count();
        *budget = budget
            .checked_sub(units)
            .ok_or_else(|| JsNativeError::range().with_message(format!("a ScriptUI window can hold at most {MAX_TEXT_UNITS} UTF-16 units of text")))?;
        Ok(t)
    };
    m.insert("text".into(), json!(text("text", ctx, budget)?));
    m.insert("helpTip".into(), json!(text("helpTip", ctx, budget)?));
    m.insert("enabled".into(), json!(get(&obj, "enabled", ctx)?.to_boolean()));
    m.insert("visible".into(), json!(get(&obj, "visible", ctx)?.to_boolean()));
    let props = get(&obj, "properties", ctx)?;
    if let Some(props) = props.as_object() {
        let name = get(&props, "name", ctx)?;
        if !name.is_undefined() && !name.is_null() {
            let name = text_of(&name, ctx)?;
            m.insert("name".into(), json!(value::cut(&name, 256)));
        }
        for flag in ["multiline", "readonly", "noecho"] {
            if get(&props, flag, ctx)?.to_boolean() {
                m.insert(flag.into(), json!(true));
            }
        }
    }
    if let Some((w, h)) = size_hint(ctx, &obj)? {
        if let Some(w) = w {
            m.insert("width".into(), json!(w));
        }
        if let Some(h) = h {
            m.insert("height".into(), json!(h));
        }
    }
    if kind == Kind::EditText
        && let Some(c) = number_of(&get(&obj, "characters", ctx)?, ctx)?
    {
        m.insert("characters".into(), json!(c.clamp(0.0, 1000.0)));
    }
    if kind.container() {
        let o = text_of(&get(&obj, "orientation", ctx)?, ctx)?.to_ascii_lowercase();
        let o = match o.as_str() {
            "row" | "stack" => o,
            _ => "column".into(),
        };
        m.insert("orientation".into(), json!(o));
        let align = get(&obj, "alignChildren", ctx)?;
        // A string, or [horizontal, vertical]: the first part decides here.
        let align = match align.as_object().filter(|o| o.is_array()) {
            Some(a) => a.get(0, ctx)?,
            None => align,
        };
        let align = text_of(&align, ctx)?.to_ascii_lowercase();
        if matches!(align.as_str(), "left" | "right" | "center" | "fill" | "top" | "bottom") {
            m.insert("alignChildren".into(), json!(align));
        }
    }
    match kind {
        Kind::Checkbox | Kind::Radio => {
            m.insert("value".into(), json!(get(&obj, "value", ctx)?.to_boolean()));
        }
        Kind::Progress | Kind::Slider => {
            let min = number_of(&get(&obj, "minvalue", ctx)?, ctx)?.unwrap_or(0.0);
            let max = number_of(&get(&obj, "maxvalue", ctx)?, ctx)?.unwrap_or(100.0);
            let v = number_of(&get(&obj, "value", ctx)?, ctx)?.unwrap_or(min);
            m.insert("min".into(), json!(min));
            m.insert("max".into(), json!(max));
            m.insert("value".into(), json!(v));
        }
        Kind::Dropdown | Kind::ListBox => {
            let mut texts = Vec::with_capacity(items.len());
            for item in &items {
                if get(item, "type", ctx)?.as_string().is_some_and(|t| t.to_std_string_escaped() == "separator") {
                    texts.push(Value::Null);
                    continue;
                }
                let v = get(item, "text", ctx)?;
                let t = text_of(&v, ctx)?;
                *budget = budget
                    .checked_sub(t.encode_utf16().count())
                    .ok_or_else(|| JsNativeError::range().with_message(format!("a ScriptUI window can hold at most {MAX_TEXT_UNITS} UTF-16 units of text")))?;
                texts.push(json!(t));
            }
            m.insert("items".into(), Value::Array(texts));
            m.insert("selection".into(), json!(selection_index(ctx, &obj, &items)?));
            element_mut(ctx, id, |e| e.sent = Some(items.clone()))?;
        }
        Kind::Button => {
            if special.0 == Some(id) {
                m.insert("default".into(), json!(true));
            }
            if special.1 == Some(id) {
                m.insert("cancel".into(), json!(true));
            }
        }
        _ => {}
    }
    if kind.container() {
        let mut kids = Vec::with_capacity(children.len());
        for c in children {
            kids.push(node(ctx, c, depth + 1, budget, special)?);
        }
        m.insert("children".into(), Value::Array(kids));
    }
    Ok(Value::Object(m))
}

/// `list.selection`: the selected item, or `null`.
fn selection_get(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this)?;
    Ok(element(ctx, id, |e| e.selection.clone())?.map_or(JsValue::null(), JsValue::from))
}

/// `list.selection = x`: one of its items, an item's index, or `null` (none). Anything else
/// selects nothing. Each item's `selected` follows.
fn selection_set(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this)?;
    let what = args.first().cloned().unwrap_or_default();
    // Converted before the items are read: a `valueOf` may change the list.
    let index = if is_item(&what) { None } else { number_of(&what, ctx)?.filter(|n| *n >= 0.0 && n.fract() == 0.0) };
    let items = element(ctx, id, |e| e.items.clone())?;
    let chosen = match what.as_object() {
        Some(o) if is_item(&what) => items.iter().find(|i| JsObject::equals(i, &o)).cloned(),
        _ => index.and_then(|n| items.get(n as usize).cloned()),
    };
    let this_one = element_mut(ctx, id, |e| {
        e.selections += 1;
        e.selections
    })?;
    for item in &items {
        let on = chosen.as_ref().is_some_and(|c| JsObject::equals(c, item));
        set(item, "selected", on, ctx)?;
        // An item's `selected` setter assigned `selection` again: that assignment wrote every
        // flag and kept its own choice, which stands. Going on would undo its flags.
        if element(ctx, id, |e| e.selections)? != this_one {
            return Ok(JsValue::undefined());
        }
    }
    // Unless an item's `selected` setter took the choice out of the list meanwhile.
    element_mut(ctx, id, |e| e.selection = chosen.filter(|c| e.items.iter().any(|i| JsObject::equals(i, c))))?;
    Ok(JsValue::undefined())
}

/// A list's selected item index: `selection` may be an item or a number.
fn selection_index(ctx: &mut Context, obj: &JsObject, items: &[JsObject]) -> JsResult<Option<usize>> {
    let sel = get(obj, "selection", ctx)?;
    if let Some(o) = sel.as_object()
        && is_item(&sel)
    {
        return Ok(items.iter().position(|i| JsObject::equals(i, &o)));
    }
    Ok(number_of(&sel, ctx)?.filter(|n| *n >= 0.0).map(|n| n as usize).filter(|i| *i < items.len()))
}

/// `(width, height)` asked for through `preferredSize`, `size` or `bounds`, in pixels.
fn size_hint(ctx: &mut Context, obj: &JsObject) -> JsResult<Option<(Option<f64>, Option<f64>)>> {
    let pick = |v: Option<f64>| v.filter(|n| *n > 0.0).map(|n| n.min(4000.0));
    for key in ["preferredSize", "size"] {
        let v = get(obj, key, ctx)?;
        let Some(o) = v.as_object() else { continue };
        let (w, h) = if o.is_array() { (o.get(0, ctx)?, o.get(1, ctx)?) } else { (get(&o, "width", ctx)?, get(&o, "height", ctx)?) };
        let (w, h) = (pick(number_of(&w, ctx)?), pick(number_of(&h, ctx)?));
        if w.is_some() || h.is_some() {
            return Ok(Some((w, h)));
        }
    }
    let b = get(obj, "bounds", ctx)?;
    if let Some(o) = b.as_object() {
        let (w, h) = if o.is_array() {
            let n = |i: u32, ctx: &mut Context| -> JsResult<Option<f64>> {
                let v = o.get(i, ctx)?;
                number_of(&v, ctx)
            };
            match (n(0, ctx)?, n(1, ctx)?, n(2, ctx)?, n(3, ctx)?) {
                (Some(l), Some(t), Some(r), Some(bt)) => (Some(r - l), Some(bt - t)),
                _ => (None, None),
            }
        } else {
            let side = |key: &str, ctx: &mut Context| -> JsResult<Option<f64>> {
                let v = get(&o, key, ctx)?;
                number_of(&v, ctx)
            };
            let (w, h) = (side("width", ctx)?, side("height", ctx)?);
            // `{left, top, right, bottom}` is the other way ScriptUI writes bounds.
            let (l, t, r, bt) = (side("left", ctx)?, side("top", ctx)?, side("right", ctx)?, side("bottom", ctx)?);
            (w.or_else(|| Some(r? - l?)), h.or_else(|| Some(bt? - t?)))
        };
        let (w, h) = (pick(w), pick(h));
        if w.is_some() || h.is_some() {
            return Ok(Some((w, h)));
        }
    }
    Ok(None)
}

/// A button whose name or text is "ok" (or "cancel"), ignoring case and the `&` of a
/// keyboard shortcut.
fn named(ctx: &mut Context, obj: &JsObject, wanted: &str) -> JsResult<bool> {
    let props = get(obj, "properties", ctx)?;
    if let Some(p) = props.as_object() {
        let name = get(&p, "name", ctx)?;
        if !name.is_undefined() && text_of(&name, ctx)?.eq_ignore_ascii_case(wanted) {
            return Ok(true);
        }
    }
    let text = text_of(&get(obj, "text", ctx)?, ctx)?.replace('&', "");
    Ok(text.trim().eq_ignore_ascii_case(wanted))
}

/// The window's default and cancel buttons: `defaultElement` / `cancelElement`, or else the
/// first button named or labelled OK / Cancel.
fn special_buttons(ctx: &mut Context, window: u32) -> JsResult<(Option<u32>, Option<u32>)> {
    let wobj = element(ctx, window, |e| e.obj.clone())?;
    let mut out = [None, None];
    for (slot, (key, name)) in [("defaultElement", "ok"), ("cancelElement", "cancel")].into_iter().enumerate() {
        let set_to = get(&wobj, key, ctx)?;
        let explicit = ui_id(&set_to).filter(|c| element(ctx, *c, |e| e.window == window).unwrap_or(false));
        let found = match explicit {
            Some(c) => Some(c),
            None => {
                let buttons = with_registry(ctx, |r| {
                    r.elements
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| e.window == window && Kind::from_code(e.kind) == Kind::Button && attached(r, e))
                        .map(|(i, e)| (i, e.obj.clone()))
                        .collect::<Vec<_>>()
                })?;
                let mut hit = None;
                for (i, b) in buttons {
                    if named(ctx, &b, name)? {
                        hit = u32::try_from(i).ok();
                        break;
                    }
                }
                hit
            }
        };
        if let Some(o) = out.get_mut(slot) {
            *o = found;
        }
    }
    Ok((out[0], out[1]))
}

/// Whether a control is still in its window (not removed, nor inside a removed group).
fn attached(r: &Registry, e: &Element) -> bool {
    let mut at = e;
    for _ in 0..=MAX_DEPTH {
        match at.parent {
            None => return Kind::from_code(at.kind) == Kind::Window,
            Some(p) => match r.elements.get(p as usize) {
                Some(next) => at = next,
                None => return false,
            },
        }
    }
    false
}

// ---------- showing ----------

/// `window.show()`. A dialog waits here, running handlers, until it is closed; a palette is
/// sent to the UI and `show()` returns at once.
fn show(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this)?;
    if kind_of(ctx, id)? != Kind::Window {
        return Err(error("show() is a method of windows"));
    }
    let (palette, showing) = element(ctx, id, |e| (e.palette, e.showing))?;
    if showing {
        return Err(error("the dialog is already showing"));
    }
    let (was_shown, visibility) = element_mut(ctx, id, |e| {
        let was = std::mem::replace(&mut e.shown, true);
        e.closed = false;
        e.result = JsValue::undefined();
        e.showing = !palette;
        (was, e.visibility)
    })?;
    let shown = show_and_wait(ctx, id, palette, was_shown, visibility);
    if !palette {
        element_mut(ctx, id, |e| e.showing = false)?;
    }
    shown
}

/// [`show`] of window `id` once it is marked shown: the `onShow` handler, then a palette's
/// update, or a dialog's loop until it is closed.
fn show_and_wait(ctx: &mut Context, id: u32, palette: bool, was_shown: bool, visibility: u64) -> JsResult<JsValue> {
    let obj = element(ctx, id, |e| e.obj.clone())?;
    // A window the app never got is not shown, so a later close() or app.refresh() sends
    // nothing for it; one already on screen stays shown, with the controls it had.
    let shown = |ctx: &mut Context| -> JsResult<()> {
        call_handler(ctx, &obj, "onShow")?;
        if palette && element(ctx, id, |e| e.shown)? {
            let tree = serialize(ctx, id)?;
            // Reading the controls runs the script's getters, which may have closed it.
            if element(ctx, id, |e| e.shown)? {
                window_update(ctx, id, Some(tree))?;
                element_mut(ctx, id, |e| e.visibility += 1)?;
            }
        }
        Ok(())
    };
    if let Err(e) = shown(ctx) {
        // Unless a handler showed or hid it meanwhile: that stands.
        element_mut(ctx, id, |e| {
            if e.visibility == visibility {
                e.shown = was_shown;
            }
        })?;
        return Err(e);
    }
    if palette {
        return Ok(JsValue::undefined());
    }
    let waited = (|| -> JsResult<()> {
        while !element(ctx, id, |e| e.closed)? {
            let tree = serialize(ctx, id)?;
            // Reading the controls runs the script's getters, which may have closed it.
            if element(ctx, id, |e| e.closed)? {
                break;
            }
            let answer = modal_value(ctx, ModalKind::Window, json!({ "id": id, "window": tree }))?;
            dispatch(ctx, id, &answer)?;
        }
        Ok(())
    })();
    // Hidden however the loop ended: a handler's error that the script catches must not leave
    // the dialog on screen, holding the app's input, for the rest of the run.
    element_mut(ctx, id, |e| e.shown = false)?;
    let hidden = window_update(ctx, id, None);
    waited?;
    hidden?;
    let result = element(ctx, id, |e| e.result.clone())?;
    Ok(if result.is_undefined() { JsValue::from(RESULT_OK) } else { result })
}

/// Mark `window` closed with `result` (the window's own id, or any control's).
fn close_window(ctx: &mut Context, any: u32, result: JsValue) -> JsResult<()> {
    let window = element(ctx, any, |e| e.window)?;
    let (palette, shown) = element_mut(ctx, window, |e| {
        e.closed = true;
        e.result = result;
        (e.palette, e.shown)
    })?;
    if palette && shown {
        // Hidden once the app has been told; a hide that fails leaves it shown.
        window_update(ctx, window, None)?;
        element_mut(ctx, window, |e| {
            e.shown = false;
            e.visibility += 1;
        })?;
    }
    Ok(())
}

/// `window.close(result)`: `show()` returns `result`, or 1 without one.
fn close(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this)?;
    let result = args.first().cloned().filter(|v| !v.is_undefined()).unwrap_or(JsValue::from(RESULT_OK));
    close_window(ctx, id, result)?;
    Ok(JsValue::undefined())
}

/// `window.hide()`: closes it; a dialog's `show()` returns 0.
fn hide(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this)?;
    close_window(ctx, id, JsValue::from(0))?;
    Ok(JsValue::undefined())
}

/// `window.update()`: send a shown palette's controls again.
fn update(this: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this)?;
    let window = element(ctx, id, |e| e.window)?;
    flush(ctx, window)?;
    Ok(JsValue::undefined())
}

fn flush(ctx: &mut Context, window: u32) -> JsResult<()> {
    if element(ctx, window, |e| e.palette && e.shown)? {
        let sent = element(ctx, window, |e| e.visibility)?;
        let tree = serialize(ctx, window)?;
        // Reading the controls runs the script's getters, which may have closed it, or sent a
        // newer update of it (`update()` from a getter): this older one is not sent after it.
        if !element(ctx, window, |e| e.shown && e.visibility == sent)? {
            return Ok(());
        }
        window_update(ctx, window, Some(tree))?;
        // The app has it now: a failing show() around this must not take that back.
        element_mut(ctx, window, |e| e.visibility += 1)?;
    }
    Ok(())
}

/// `app.refresh()`: send every shown palette's controls again.
pub(crate) fn refresh(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let shown: Vec<u32> =
        with_registry(ctx, |r| r.elements.iter().enumerate().filter(|(_, e)| e.palette && e.shown).filter_map(|(i, _)| u32::try_from(i).ok()).collect())?;
    // Every palette is sent even when one can't be; the first failure is the script's error.
    let mut first = None;
    for w in shown {
        if let Err(e) = flush(ctx, w) {
            first.get_or_insert(e);
        }
    }
    match first {
        Some(e) => Err(e),
        None => Ok(JsValue::undefined()),
    }
}

/// `control.notify(eventName)`: run the control's handler as if the user had acted.
fn notify(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = this_id(this)?;
    let kind = kind_of(ctx, id)?;
    let event = match args.first().filter(|v| !v.is_undefined()) {
        Some(v) => text_of(v, ctx)?,
        None => (if matches!(kind, Kind::Button | Kind::Checkbox | Kind::Radio) { "onClick" } else { "onChange" }).to_owned(),
    };
    let obj = element(ctx, id, |e| e.obj.clone())?;
    if !call_handler(ctx, &obj, &event)? && event == "onClick" && kind == Kind::Button {
        default_click(ctx, id)?;
    }
    Ok(JsValue::undefined())
}

/// Call `obj[name]` if it is a function. `Ok(true)` when there was one.
fn call_handler(ctx: &mut Context, obj: &JsObject, name: &str) -> JsResult<bool> {
    let h = get(obj, name, ctx)?;
    match h.as_callable() {
        Some(f) => {
            f.call(&obj.clone().into(), &[], ctx)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// A click on a button without `onClick`: the default button closes with 1, the cancel
/// button with 2.
fn default_click(ctx: &mut Context, id: u32) -> JsResult<()> {
    let window = element(ctx, id, |e| e.window)?;
    let (default, cancel) = special_buttons(ctx, window)?;
    if default == Some(id) {
        close_window(ctx, window, JsValue::from(RESULT_OK))?;
    } else if cancel == Some(id) {
        close_window(ctx, window, JsValue::from(RESULT_CANCEL))?;
    }
    Ok(())
}

/// Whether the user can act on a control: it is still in its window, and it and every group
/// around it are enabled and visible.
fn usable(ctx: &mut Context, id: u32) -> JsResult<bool> {
    let mut at = id;
    for _ in 0..=MAX_DEPTH + 1 {
        let (obj, kind) = element(ctx, at, |e| (e.obj.clone(), Kind::from_code(e.kind)))?;
        if !get(&obj, "enabled", ctx)?.to_boolean() || !get(&obj, "visible", ctx)?.to_boolean() {
            return Ok(false);
        }
        match element(ctx, at, |e| e.parent)? {
            Some(p) => at = p,
            // A control taken out with remove() is no longer in its window; the getters read
            // on the way may have taken it, or a group it is in, out: checked again natively.
            None => return Ok(kind == Kind::Window && in_window(ctx, id)?),
        }
    }
    Ok(false)
}

/// Whether control `id` is still in a window, following only the native parent links.
fn in_window(ctx: &Context, id: u32) -> JsResult<bool> {
    let mut at = id;
    for _ in 0..=MAX_DEPTH + 1 {
        let (parent, kind) = element(ctx, at, |e| (e.parent, Kind::from_code(e.kind)))?;
        match parent {
            Some(p) => at = p,
            None => return Ok(kind == Kind::Window),
        }
    }
    Ok(false)
}

/// Apply one UI event to `window`: first the values the user changed, then the event.
///
/// The answer is `{"event": "click" | "change" | "changing" | "close", "id": control,
/// "values": [{"id": control, "value": …}]}`. Values of controls in other windows, and of
/// controls the user cannot edit, are ignored.
fn dispatch(ctx: &mut Context, window: u32, answer: &Value) -> JsResult<()> {
    if let Some(values) = answer.get("values").and_then(Value::as_array) {
        for v in values {
            let Some(id) = v.get("id").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok()) else { continue };
            let Ok((kind, owner, obj, sent)) = element(ctx, id, |e| (Kind::from_code(e.kind), e.window, e.obj.clone(), e.sent.clone())) else {
                continue;
            };
            if owner != window || !usable(ctx, id)? {
                continue;
            }
            // Read after `usable`, whose read of `enabled` may run a getter of the script's.
            let items = element(ctx, id, |e| e.items.clone())?;
            let value = v.get("value").unwrap_or(&Value::Null);
            match kind {
                Kind::EditText => {
                    // A read-only field the user could not have edited keeps its text.
                    let readonly = match get(&obj, "properties", ctx)?.as_object() {
                        Some(props) => get(&props, "readonly", ctx)?.to_boolean(),
                        None => false,
                    };
                    if let Some(t) = value.as_str().filter(|_| !readonly)
                        && t.encode_utf16().count() <= MAX_TEXT_UNITS
                    {
                        set(&obj, "text", string(t), ctx)?;
                    }
                }
                Kind::Checkbox | Kind::Radio => {
                    if let Some(b) = value.as_bool() {
                        set(&obj, "value", b, ctx)?;
                    }
                }
                Kind::Slider => {
                    if let Some(n) = value.as_f64().filter(|n| n.is_finite()) {
                        set(&obj, "value", n, ctx)?;
                    }
                }
                Kind::Dropdown | Kind::ListBox => {
                    // The index is into the items the user saw; one no longer in the list is
                    // ignored rather than taken as whichever item now has that index.
                    let shown = sent.as_deref().unwrap_or(&items);
                    let chosen = match value.as_u64() {
                        Some(i) => match usize::try_from(i).ok().and_then(|i| shown.get(i)).filter(|c| items.iter().any(|it| JsObject::equals(it, c))) {
                            Some(c) => Some(c.clone()),
                            None => continue,
                        },
                        None => None,
                    };
                    // One assignment, which sets the flags itself: a `selected` setter's own
                    // choice made meanwhile then stands (see `selection_set`). A script that
                    // redefined `selection` handles the flags itself.
                    set(&obj, "selection", chosen.map_or(JsValue::null(), JsValue::from), ctx)?;
                }
                _ => {}
            }
        }
    }
    let event = answer.get("event").and_then(Value::as_str).unwrap_or("");
    let target = answer.get("id").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok());
    if event == "close" {
        let wobj = element(ctx, window, |e| e.obj.clone())?;
        let h = get(&wobj, "onClose", ctx)?;
        if let Some(f) = h.as_callable() {
            let keep_open = f.call(&wobj.clone().into(), &[], ctx)?;
            // `onClose` returning false keeps the window open.
            if keep_open.as_boolean() == Some(false) {
                return Ok(());
            }
            // One that closed it itself (`w.close(7)`) keeps the result it gave.
            if element(ctx, window, |e| e.closed)? {
                return Ok(());
            }
        }
        return close_window(ctx, window, JsValue::from(RESULT_CANCEL));
    }
    let Some(id) = target else { return Ok(()) };
    let Ok((kind, owner, obj)) = element(ctx, id, |e| (Kind::from_code(e.kind), e.window, e.obj.clone())) else { return Ok(()) };
    if owner != window || !usable(ctx, id)? {
        return Ok(());
    }
    let handler = match event {
        "click" => "onClick",
        "change" => "onChange",
        "changing" => "onChanging",
        _ => return Ok(()),
    };
    if !call_handler(ctx, &obj, handler)? && event == "click" && kind == Kind::Button {
        default_click(ctx, id)?;
    }
    Ok(())
}

/// `ctor.prototype` is `proto` and `proto.constructor` is `ctor`, as for a class, so
/// `instanceof` and `constructor` work on the objects `ctor` makes.
pub(crate) fn link_prototype(ctor: &JsObject, proto: &JsObject, ctx: &mut Context) -> JsResult<()> {
    use boa_engine::property::PropertyDescriptor;
    ctor.define_property_or_throw(
        js_string!("prototype"),
        PropertyDescriptor::builder().value(proto.clone()).writable(false).enumerable(false).configurable(false),
        ctx,
    )?;
    proto.define_property_or_throw(
        js_string!("constructor"),
        PropertyDescriptor::builder().value(ctor.clone()).writable(true).enumerable(false).configurable(true),
        ctx,
    )?;
    Ok(())
}

/// Install `Window` and the shared prototypes into a fresh context.
pub(crate) fn install(ctx: &mut Context) -> JsResult<()> {
    let methods: [(&str, Native, usize); 10] = [
        ("add", add, 4),
        ("remove", remove, 1),
        ("removeAll", remove_all, 0),
        ("find", find, 1),
        ("show", show, 0),
        ("close", close, 1),
        ("hide", hide, 0),
        ("center", nothing, 0),
        ("update", update, 0),
        ("notify", notify, 1),
    ];
    let mut init = ObjectInitializer::new(ctx);
    for (name, f, len) in methods {
        init.function(NativeFunction::from_fn_ptr(f), JsString::from(name), len);
    }
    let proto = init.build();
    let item_proto = ObjectInitializer::new(ctx).function(NativeFunction::from_fn_ptr(item_to_string), js_string!("toString"), 0).build();
    let realm = ctx.realm().clone();
    realm.host_defined_mut().insert(Registry { elements: Vec::new(), items: 0, proto: proto.clone(), item_proto });
    let ctor = FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(window_ctor)).name(js_string!("Window")).length(4).constructor(true).build();
    // A native constructor gets no `prototype` of its own; without one `instanceof` throws.
    link_prototype(&ctor, &proto, ctx)?;
    ctx.register_global_property(js_string!("Window"), ctor, Attribute::WRITABLE | Attribute::CONFIGURABLE)?;
    Ok(())
}
