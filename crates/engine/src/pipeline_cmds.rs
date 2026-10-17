//! Pipeline tagging: free-form user data stored with the document, and layer ids that stay the
//! same when the file is saved and opened again.
//!
//! A tool that tracks what a document holds (a studio's publishing pipeline, an asset manager,
//! a batch script) leaves notes in the file — "this group is published as …", "this layer was
//! loaded from version 12" — and has to find them again later, here or after a trip through
//! Photoshop.
//!
//! * `document.userData` keeps JSON values under caller-chosen keys in the document's XMP packet
//!   (the `photocraft:UserData` property), so every format that carries XMP keeps it (PSD, PSB,
//!   TIFF, `.pcraft`, …). XMP asks other applications to preserve properties they don't know;
//!   both serializations one may save it back in are read.
//! * `layer.persistentId` returns the layer's PSD layer id (`lyid`), giving the layer one when
//!   it has none. The `id` of `document.inspect` is a session id: a PSD's layers get new ones
//!   every time the file opens, so a note that names a layer uses this id instead.

use std::collections::HashSet;
use std::sync::Arc;

use photocraft_doc::LayerId;
use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::file_cmds::{find_attr, find_element, xml_escape, xml_unescape, xmp_packet};
use crate::{EngineError, Result, Session};

const USER_DATA: &str = "document.userData";
const PERSISTENT_ID: &str = "layer.persistentId";

/// The XMP property holding the user data, as one JSON object.
const PROPERTY: &str = "photocraft:UserData";
/// The namespace of [`PROPERTY`].
const NAMESPACE: &str = "https://github.com/storytold/photocraft/ns/1.0/";

/// Longest key, in bytes.
const MAX_KEY: usize = 64;
/// Largest value, as JSON text.
const MAX_VALUE: usize = 1 << 20;
/// Largest user data of one document, as JSON text: the packet is written into every save.
const MAX_TOTAL: usize = 4 << 20;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_layers(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.layers.is_empty() { Err("the document has no layers".into()) } else { Ok(()) }
}

/// The user data in an XMP packet. Anything that isn't a JSON object of ours (a damaged or
/// oversized property) reads as empty rather than failing the document.
pub fn read_user_data(xmp: Option<&str>) -> Map<String, Value> {
    let x = xmp.unwrap_or("");
    // We write the element form; an XMP toolkit may save the property back as an attribute.
    let text = match find_element(x, PROPERTY) {
        Some((_, _, inner)) => xml_unescape(inner.trim()),
        None => match find_attr(x, PROPERTY) {
            Some((_, _, value)) => xml_unescape(value),
            None => return Map::new(),
        },
    };
    if text.len() > MAX_TOTAL {
        return Map::new();
    }
    match serde_json::from_str(&text) {
        Ok(Value::Object(data)) => data,
        _ => Map::new(),
    }
}

/// `xmp` with its user data replaced by `data` and every other property kept verbatim (a packet
/// is created when there is none). Empty `data` removes the property.
pub fn write_user_data(xmp: Option<&str>, data: &Map<String, Value>) -> String {
    let mut x = xmp_packet(xmp);
    while let Some((a, b, _)) = find_attr(&x, PROPERTY) {
        x.replace_range(a..b, "");
    }
    // Rewritten where it was, so repeated edits don't pile up `rdf:Description`s.
    let mut slot = None;
    while let Some((a, b, _)) = find_element(&x, PROPERTY) {
        x.replace_range(a..b, "");
        slot.get_or_insert(a);
    }
    if data.is_empty() {
        return x;
    }
    let element = format!("<{PROPERTY}>{}</{PROPERTY}>", xml_escape(&Value::Object(data.clone()).to_string()));
    match slot {
        Some(at) => x.insert_str(at, &element),
        None => {
            let at = x.rfind("</rdf:RDF>").unwrap_or(x.len());
            x.insert_str(at, &format!("  <rdf:Description rdf:about=\"\" xmlns:photocraft=\"{NAMESPACE}\">\n   {element}\n  </rdf:Description>\n "));
        }
    }
    x
}

/// The optional `key` param: 1 to [`MAX_KEY`] characters from `A-Z a-z 0-9 . _ -`.
fn key_param(p: &Value) -> Result<Option<&str>> {
    match p.get("key") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(k)) if !k.is_empty() && k.len() <= MAX_KEY && k.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) => Ok(Some(k)),
        Some(_) => Err(bad(USER_DATA, format!("`key` must be 1 to {MAX_KEY} characters from A-Z a-z 0-9 . _ -"))),
    }
}

/// These commands are not journaled (a pipeline's notes must not replay from a recorded action),
/// so the dispatcher doesn't refuse them while a background job runs on the document. A change
/// made then would be lost when the job applies its result, so it is refused here.
fn free_to_edit(s: &Session, cmd: &str) -> Result<()> {
    match s.edit_conflict() {
        Some(busy) => Err(EngineError::Disabled(cmd.to_string(), busy)),
        None => Ok(()),
    }
}

/// Store `data` as the active document's user data (one undo step).
fn store(s: &mut Session, data: &Map<String, Value>) -> Result<()> {
    free_to_edit(s, USER_DATA)?;
    let xmp = s.active().ok_or(EngineError::NoDocument)?.doc.metadata.xmp.clone();
    let new = write_user_data(xmp.as_deref(), data);
    // A packet too damaged to hold the property (an unclosed element of ours) must not look
    // like a successful save that then reads back empty.
    if read_user_data(Some(&new)) != *data {
        return Err(EngineError::Other("the document's XMP metadata is damaged, so user data can't be stored in it".into()));
    }
    s.edit("User Data", |doc, _| {
        doc.metadata.xmp = Some(new);
        Ok(())
    })
}

fn user_data(s: &mut Session, p: &Value) -> Result<Value> {
    let key = key_param(p)?;
    let remove = match p.get("remove") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(USER_DATA, "`remove` must be true or false"))?,
    };
    let value = p.get("value").filter(|v| !v.is_null());
    let mut data = read_user_data(s.active().ok_or(EngineError::NoDocument)?.doc.metadata.xmp.as_deref());
    let Some(key) = key else {
        if remove || value.is_some() {
            return Err(bad(USER_DATA, "`value` and `remove` need a `key`"));
        }
        return Ok(json!({"data": data}));
    };
    match (value, remove) {
        (Some(_), true) => Err(bad(USER_DATA, "pass `value` or `remove`, not both")),
        (None, false) => Ok(json!({"key": key, "value": data.get(key)})),
        (None, true) => {
            let removed = data.remove(key).is_some();
            if removed {
                store(s, &data)?;
            }
            Ok(json!({"key": key, "value": null, "removed": removed}))
        }
        (Some(value), false) => {
            if value.to_string().len() > MAX_VALUE {
                return Err(bad(USER_DATA, format!("`value` is larger than {MAX_VALUE} bytes of JSON")));
            }
            // Storing what is already there is not an edit: no undo step, a clean document stays clean.
            if data.get(key) != Some(value) {
                data.insert(key.to_string(), value.clone());
                if Value::Object(data.clone()).to_string().len() > MAX_TOTAL {
                    return Err(bad(USER_DATA, format!("the document's user data would be larger than {MAX_TOTAL} bytes of JSON")));
                }
                store(s, &data)?;
            }
            Ok(json!({"key": key, "value": value}))
        }
    }
}

fn persistent_id(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = match p.get("layer") {
        None | Some(Value::Null) => st.active_layer.ok_or_else(|| bad(PERSISTENT_ID, "no layer given and no active layer"))?,
        Some(v) => LayerId(v.as_u64().ok_or_else(|| bad(PERSISTENT_ID, "`layer` must be a layer id"))?),
    };
    let walk = st.doc.walk();
    let own = walk.iter().find(|(_, _, l)| l.id == id).ok_or(EngineError::NoLayer(id))?.2.psd_id;
    // The PSD writer keeps an id for the first layer carrying it and renumbers a second one.
    if let Some(n) = own
        && walk.iter().find(|(_, _, l)| l.psd_id == Some(n)).is_some_and(|(_, _, l)| l.id == id)
    {
        return Ok(json!({"layer": id.0, "persistentId": n, "assigned": false}));
    }
    let used: HashSet<u32> = walk.iter().filter_map(|(_, _, l)| l.psd_id).collect();
    // Past the highest id in use, so a deleted layer's id isn't handed to another layer while a
    // higher one exists; the lowest free id only once that range is exhausted.
    let next = used
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .or_else(|| (1..=u32::MAX).find(|n| !used.contains(n)))
        .ok_or_else(|| EngineError::Other("the document has no free layer id".into()))?;
    // An id lost to a job's result could later be handed to another layer.
    free_to_edit(s, PERSISTENT_ID)?;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    // No history step, as when the PSD writer numbers a layer. Copy-on-write: history snapshots
    // keep their own state.
    Arc::make_mut(&mut st.doc).layer_mut(id).ok_or(EngineError::NoLayer(id))?.psd_id = Some(next);
    Ok(json!({"layer": id.0, "persistentId": next, "assigned": true}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: USER_DATA,
            label: "Document User Data",
            menu: &[],
            shortcut: None,
            params: r##"{"key":str?,"value":json?,"remove":bool=false} (no key: everything, as {"data":{key:value}}; key alone: read it; value: store any JSON under the key; remove: delete the key. Keys are 1..64 characters from A-Z a-z 0-9 . _ -. Kept in the document's XMP, so it is saved with PSD, PSB, TIFF and .pcraft files; a change is one undo step)"##,
            enabled: has_doc,
            run: user_data,
            // A pipeline's notes about this document must not replay onto others from a recorded action.
            journal: false,
        },
        CommandSpec {
            id: PERSISTENT_ID,
            label: "Persistent Layer ID",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?} → {"layer","persistentId","assigned"}: the layer's PSD layer id, which stays the same when the file is saved and reopened (the "id" of document.inspect is a session id). A layer without one is given the next free id; not an undo step"##,
            enabled: has_layers,
            run: persistent_id,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        s
    }

    fn xmp(s: &Session) -> Option<&str> {
        s.active().unwrap().doc.metadata.xmp.as_deref()
    }

    #[test]
    fn user_data_sets_reads_lists_and_removes() {
        let mut s = session();
        assert_eq!(s.execute(USER_DATA, json!({})).unwrap(), json!({"data": {}}));
        assert_eq!(s.execute(USER_DATA, json!({"key": "pipeline"})).unwrap(), json!({"key": "pipeline", "value": null}));
        assert!(xmp(&s).is_none(), "reading never creates a packet");

        let tags = json!({"instances": [{"layer": 7, "product": "imageMain <&\"'> ünï 日本"}], "n": 1.5, "on": true});
        assert_eq!(s.execute(USER_DATA, json!({"key": "pipeline", "value": tags})).unwrap()["value"], tags);
        s.execute(USER_DATA, json!({"key": "other.tool-2", "value": "plain"})).unwrap();
        assert_eq!(s.execute(USER_DATA, json!({"key": "pipeline"})).unwrap()["value"], tags);
        assert_eq!(s.execute(USER_DATA, json!({})).unwrap()["data"], json!({"pipeline": tags, "other.tool-2": "plain"}));
        // One property, however often it is rewritten.
        for n in 0..5 {
            s.execute(USER_DATA, json!({"key": "pipeline", "value": {"n": n}})).unwrap();
        }
        let packet = xmp(&s).unwrap();
        assert_eq!(packet.matches("<photocraft:UserData>").count(), 1, "{packet}");
        assert_eq!(packet.matches("xmlns:photocraft=").count(), 1, "{packet}");

        assert_eq!(s.execute(USER_DATA, json!({"key": "pipeline", "remove": true})).unwrap(), json!({"key": "pipeline", "value": null, "removed": true}));
        assert_eq!(s.execute(USER_DATA, json!({"key": "pipeline", "remove": true})).unwrap()["removed"], false);
        assert_eq!(s.execute(USER_DATA, json!({})).unwrap()["data"], json!({"other.tool-2": "plain"}));
        s.execute(USER_DATA, json!({"key": "other.tool-2", "remove": true})).unwrap();
        assert!(!xmp(&s).unwrap().contains("photocraft:UserData"), "the last key takes the property with it");
    }

    #[test]
    fn user_data_is_an_undo_step_and_unchanged_values_are_not() {
        let mut s = session();
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
        let steps = s.active().unwrap().history.entries().len();
        s.execute(USER_DATA, json!({"key": "a", "value": [1, 2]})).unwrap();
        assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
        assert!(s.active().unwrap().is_dirty(), "the document has something to save");
        // The same value again, a read and a missing key's removal change nothing.
        let revision = s.active().unwrap().revision;
        s.execute(USER_DATA, json!({"key": "a", "value": [1, 2]})).unwrap();
        s.execute(USER_DATA, json!({"key": "a"})).unwrap();
        s.execute(USER_DATA, json!({"key": "missing", "remove": true})).unwrap();
        assert_eq!((s.active().unwrap().history.entries().len(), s.active().unwrap().revision), (steps + 1, revision));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.execute(USER_DATA, json!({})).unwrap()["data"], json!({}));
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.execute(USER_DATA, json!({"key": "a"})).unwrap()["value"], json!([1, 2]));
    }

    #[test]
    fn user_data_keeps_the_rest_of_the_packet_and_file_info() {
        let mut s = session();
        s.execute("file.fileInfo", json!({"title": "Shot 010", "keywords": ["a", "b"]})).unwrap();
        s.execute(USER_DATA, json!({"key": "k", "value": {"v": 1}})).unwrap();
        s.execute("file.fileInfo", json!({"description": "after"})).unwrap();
        let info = s.execute("file.fileInfo", json!({})).unwrap();
        assert_eq!((info["title"].as_str(), info["description"].as_str()), (Some("Shot 010"), Some("after")));
        assert_eq!(info["keywords"], json!(["a", "b"]));
        assert_eq!(s.execute(USER_DATA, json!({"key": "k"})).unwrap()["value"], json!({"v": 1}));
    }

    /// An XMP toolkit may save the property back as an attribute of its `rdf:Description`.
    #[test]
    fn user_data_reads_the_attribute_form_and_rewrites_it_as_one_element() {
        let packet = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
            <rdf:Description rdf:about=\"\" xmlns:photocraft=\"https://github.com/storytold/photocraft/ns/1.0/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
            photocraft:UserData=\"{&quot;k&quot;:{&quot;a&quot;:&quot;x &amp;amp; &lt;y&gt;&quot;}}\" dc:format=\"image/png\"/></rdf:RDF></x:xmpmeta>";
        assert_eq!(Value::Object(read_user_data(Some(packet))), json!({"k": {"a": "x &amp; <y>"}}));
        let mut data = read_user_data(Some(packet));
        data.insert("n".into(), json!(2));
        let out = write_user_data(Some(packet), &data);
        assert!(!out.contains("photocraft:UserData=\""), "{out}");
        assert!(out.contains("dc:format=\"image/png\""), "other properties stay: {out}");
        assert_eq!(read_user_data(Some(&out)), data);
    }

    #[test]
    fn hostile_packets_read_as_empty_and_never_panic() {
        let big = format!("<rdf:RDF><photocraft:UserData>{{\"k\":\"{}\"}}</photocraft:UserData></rdf:RDF>", "x".repeat(MAX_TOTAL));
        let deep = format!("<rdf:RDF><photocraft:UserData>{}</photocraft:UserData></rdf:RDF>", "[".repeat(100_000));
        for packet in [
            "",
            "<photocraft:UserData>",
            "<photocraft:UserData/>",
            "<photocraft:UserData>not json</photocraft:UserData>",
            "<photocraft:UserData>[1,2]</photocraft:UserData>",
            "<photocraft:UserData>{\"k\":</photocraft:UserData>",
            " photocraft:UserData=\"",
            " photocraft:UserData=\"{é\"",
            "<photocraft:UserDataX>{\"k\":1}</photocraft:UserDataX>",
            big.as_str(),
            deep.as_str(),
        ] {
            assert!(read_user_data(Some(packet)).is_empty(), "{}", &packet[..packet.len().min(60)]);
        }
        // A damaged packet is replaced or refused, never silently lost on a successful reply.
        let mut s = session();
        for damaged in ["<rdf:RDF><photocraft:UserData></rdf:RDF>", "garbage", "<rdf:RDF><photocraft:UserData>{</photocraft:UserData></rdf:RDF>"] {
            s.edit("setup", |doc, _| {
                doc.metadata.xmp = Some(damaged.to_string());
                Ok(())
            })
            .unwrap();
            match s.execute(USER_DATA, json!({"key": "k", "value": 1})) {
                Ok(_) => assert_eq!(s.execute(USER_DATA, json!({"key": "k"})).unwrap()["value"], 1, "{damaged}"),
                Err(e) => assert!(e.to_string().contains("damaged"), "{damaged}: {e}"),
            }
        }
    }

    #[test]
    fn user_data_bad_params_fail_gracefully() {
        let mut s = session();
        let long_key = "k".repeat(MAX_KEY + 1);
        let huge = "x".repeat(MAX_VALUE);
        for p in [
            json!({"key": ""}),
            json!({"key": long_key}),
            json!({"key": "has space"}),
            json!({"key": "sl/ash"}),
            json!({"key": "é"}),
            json!({"key": 7}),
            json!({"key": ["a"]}),
            json!({"value": 1}),
            json!({"remove": true}),
            json!({"key": "k", "remove": "yes"}),
            json!({"key": "k", "value": 1, "remove": true}),
            json!({"key": "k", "value": huge}),
        ] {
            assert!(s.execute(USER_DATA, p.clone()).is_err(), "{}", &p.to_string()[..p.to_string().len().min(80)]);
        }
        assert!(xmp(&s).is_none(), "nothing was stored");
        // The total is capped too: values that each fit can't grow the packet without bound.
        let chunk = "y".repeat(MAX_VALUE - 16);
        let mut stored = 0;
        for n in 0..8 {
            match s.execute(USER_DATA, json!({"key": format!("k{n}"), "value": chunk})) {
                Ok(_) => stored += 1,
                Err(e) => {
                    assert!(e.to_string().contains("would be larger"), "{e}");
                    break;
                }
            }
        }
        assert!((1..8).contains(&stored), "stored {stored}");
        assert_eq!(s.execute(USER_DATA, json!({})).unwrap()["data"].as_object().unwrap().len(), stored);
        assert!(Session::new().execute(USER_DATA, json!({})).is_err(), "no document: disabled");
    }

    /// A new layer (which becomes the active one), as its session id.
    fn new_layer(s: &mut Session, name: &str) -> u64 {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
        s.active().unwrap().active_layer.unwrap().0
    }

    fn psd_id(s: &Session, id: u64) -> Option<u32> {
        s.active().unwrap().doc.layer(LayerId(id)).unwrap().psd_id
    }

    #[test]
    fn persistent_id_is_assigned_once_and_reported_by_inspect() {
        let mut s = session();
        let a = new_layer(&mut s, "a");
        let b = new_layer(&mut s, "b");
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
        let steps = s.active().unwrap().history.entries().len();
        assert_eq!(psd_id(&s, a), None);
        // The active layer by default; the same id every time after that.
        assert_eq!(s.execute(PERSISTENT_ID, json!({})).unwrap(), json!({"layer": b, "persistentId": 1, "assigned": true}));
        assert_eq!(s.execute(PERSISTENT_ID, json!({"layer": b})).unwrap(), json!({"layer": b, "persistentId": 1, "assigned": false}));
        assert_eq!(s.execute(PERSISTENT_ID, json!({"layer": a})).unwrap()["persistentId"], 2);
        assert_eq!((psd_id(&s, a), psd_id(&s, b)), (Some(2), Some(1)));
        assert_eq!(s.active().unwrap().history.entries().len(), steps, "no history step");
        assert!(!s.active().unwrap().is_dirty(), "still clean");
        let layers = s.execute("document.inspect", json!({})).unwrap()["layers"].clone();
        let reported: Vec<(u64, Value)> = layers.as_array().unwrap().iter().map(|l| (l["id"].as_u64().unwrap(), l["persistentId"].clone())).collect();
        assert!(reported.contains(&(a, json!(2))) && reported.contains(&(b, json!(1))), "{reported:?}");
        assert!(reported.iter().any(|(_, id)| id.is_null()), "the background has none yet: {reported:?}");
    }

    #[test]
    fn persistent_id_never_shares_an_id_between_two_layers() {
        let mut s = session();
        let a = new_layer(&mut s, "a");
        let b = new_layer(&mut s, "b");
        // A file where two layers carry the same id, and one at the top of the range.
        s.edit("setup", |doc, _| {
            for id in [a, b] {
                doc.layer_mut(LayerId(id)).ok_or(EngineError::NoLayer(LayerId(id)))?.psd_id = Some(40);
            }
            Ok(())
        })
        .unwrap();
        let first = s.active().unwrap().doc.walk().iter().find(|(_, _, l)| l.psd_id == Some(40)).map(|(_, _, l)| l.id.0).unwrap();
        let second = if first == a { b } else { a };
        assert_eq!(s.execute(PERSISTENT_ID, json!({"layer": first})).unwrap()["assigned"], false);
        assert_eq!(s.execute(PERSISTENT_ID, json!({"layer": second})).unwrap(), json!({"layer": second, "persistentId": 41, "assigned": true}));
        // A duplicate starts without an id and gets its own.
        s.execute("layer.duplicate", json!({"layer": first})).unwrap();
        let copy = s.active().unwrap().active_layer.unwrap().0;
        assert_ne!(copy, first);
        assert_eq!(s.execute(PERSISTENT_ID, json!({"layer": copy})).unwrap()["persistentId"], 42);
        // With the top of the range taken, the lowest free id is used instead of overflowing.
        s.edit("setup", |doc, _| {
            doc.layer_mut(LayerId(first)).ok_or(EngineError::NoLayer(LayerId(first)))?.psd_id = Some(u32::MAX);
            Ok(())
        })
        .unwrap();
        let fresh = new_layer(&mut s, "c");
        assert_eq!(s.execute(PERSISTENT_ID, json!({"layer": fresh})).unwrap()["persistentId"], 1);
    }

    /// What both commands are for: the notes and the ids are still there after a save and reopen.
    #[test]
    fn user_data_and_persistent_ids_survive_psd_and_pcraft_round_trips() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
            new_layer(&mut s, "plain");
            let tagged = new_layer(&mut s, "tagged");
            let id = s.execute(PERSISTENT_ID, json!({"layer": tagged})).unwrap()["persistentId"].as_u64().unwrap();
            let tags = json!({"instances": [{"layer": id, "product": "imageHero <&> ü"}]});
            s.execute(USER_DATA, json!({"key": "pipeline", "value": tags})).unwrap();
            let doc = s.active().unwrap().doc.clone();

            let psd = photocraft_io::export(&doc, "psd", &Default::default()).unwrap().bytes;
            let from_psd = photocraft_io::import("round.psd", &psd).unwrap().document;
            let pcraft = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
            let from_pcraft = photocraft_format::load_from_bytes(&pcraft).unwrap();
            for (format, back) in [("psd", from_psd), ("pcraft", from_pcraft)] {
                assert_eq!(read_user_data(back.metadata.xmp.as_deref()).get("pipeline"), Some(&tags), "{format} at {depth} bits");
                // Reopened in a new session, the layer answers to the same id and keeps it.
                let mut reopened = Session::new();
                reopened.add_document(back, None);
                let layers = reopened.execute("document.inspect", json!({})).unwrap()["layers"].clone();
                let layer = layers.as_array().unwrap().iter().find(|l| l["name"] == "tagged").unwrap().clone();
                assert_eq!(layer["persistentId"].as_u64(), Some(id), "{format} at {depth} bits: {layer}");
                let again = reopened.execute(PERSISTENT_ID, json!({"layer": layer["id"]})).unwrap();
                assert_eq!((again["persistentId"].as_u64(), again["assigned"].as_bool()), (Some(id), Some(false)), "{format}");
                // A layer numbered afterwards never takes the tagged layer's id.
                let plain = layers.as_array().unwrap().iter().find(|l| l["name"] == "plain").unwrap()["id"].clone();
                assert_ne!(reopened.execute(PERSISTENT_ID, json!({"layer": plain})).unwrap()["persistentId"].as_u64(), Some(id), "{format}");
            }
        }
    }

    /// While a background job runs on the document, reads answer and changes wait: a change
    /// made then would be lost when the job applies.
    #[test]
    fn changes_are_refused_while_a_job_runs_on_the_document() {
        let mut s = session();
        let numbered = new_layer(&mut s, "numbered");
        let plain = new_layer(&mut s, "plain");
        s.execute(PERSISTENT_ID, json!({"layer": numbered})).unwrap();
        s.execute(USER_DATA, json!({"key": "k", "value": 1})).unwrap();
        // A job that holds the document until the gate opens.
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let open = gate.clone();
        let started = s
            .start_job(
                "test.gated",
                json!({}),
                "Gated Job",
                true,
                move |ctx| {
                    while !open.load(std::sync::atomic::Ordering::Relaxed) {
                        ctx.check()?;
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Ok(())
                },
                |s, ()| {
                    s.edit("Gated Job", |_, _| Ok(()))?;
                    Ok(json!({}))
                },
            )
            .unwrap();
        let crate::jobs::Started::Job(job) = started else { panic!("expected a background job") };

        for (cmd, params) in
            [(USER_DATA, json!({"key": "k", "value": 2})), (USER_DATA, json!({"key": "k", "remove": true})), (PERSISTENT_ID, json!({"layer": plain}))]
        {
            let e = s.execute(cmd, params.clone()).unwrap_err();
            assert!(matches!(&e, EngineError::Disabled(_, why) if why.contains("Gated Job")), "{cmd} {params}: {e}");
        }
        assert_eq!(s.execute(USER_DATA, json!({"key": "k"})).unwrap()["value"], 1);
        assert_eq!(s.execute(USER_DATA, json!({"key": "k", "value": 1})).unwrap()["value"], 1, "the value already there is not a change");
        assert_eq!(s.execute(PERSISTENT_ID, json!({"layer": numbered})).unwrap()["assigned"], false);
        assert_eq!(psd_id(&s, plain), None);

        gate.store(true, std::sync::atomic::Ordering::Relaxed);
        s.wait_job(job).unwrap();
        s.execute(USER_DATA, json!({"key": "k", "value": 2})).unwrap();
        assert_eq!(s.execute(PERSISTENT_ID, json!({"layer": plain})).unwrap()["assigned"], true);
    }

    #[test]
    fn persistent_id_bad_params_fail_gracefully() {
        let mut s = session();
        for p in [json!({"layer": 999_999}), json!({"layer": "x"}), json!({"layer": -1}), json!({"layer": 1.5}), json!({"layer": [1]})] {
            assert!(s.execute(PERSISTENT_ID, p.clone()).is_err(), "{p}");
        }
        assert!(Session::new().execute(PERSISTENT_ID, json!({})).is_err(), "no document: disabled");
    }
}
