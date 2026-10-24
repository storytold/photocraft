//! Layer and smart-contents ids are keyed to the step that minted them, so replaying a journal
//! in a fresh session reproduces them, and steps that name a layer by id still find it.
use photocraft_doc::{Layer, LayerContent};
use photocraft_engine::Session;
use serde_json::json;

/// Every layer id and smart-contents id, in document order.
fn ids(s: &Session) -> Vec<u64> {
    s.documents()
        .iter()
        .flat_map(|st| {
            st.doc.walk().into_iter().flat_map(|(_, _, l): (_, _, &Layer)| {
                let contents = if let LayerContent::Smart(sm) = &l.content { Some(sm.contents_id.0) } else { None };
                [Some(l.id.0), contents]
            })
        })
        .flatten()
        .collect()
}

#[test]
fn replayed_journal_mints_identical_ids() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    let a = s.active().unwrap().active_layer.unwrap().0;
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fillForeground", json!({})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
    s.execute("layer.duplicate", json!({})).unwrap();
    s.execute("layer.select", json!({"layer": a})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "C"})).unwrap();

    let mut r = Session::new();
    for (i, (id, p)) in s.journal.iter().enumerate() {
        r.execute(id, p.clone()).unwrap_or_else(|e| panic!("replay #{i} {id}: {e}"));
    }
    let (orig, replayed) = (ids(&s), ids(&r));
    assert_eq!(replayed, orig);
    let mut unique = orig.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), orig.len(), "ids are unique across the session's documents: {orig:?}");
    assert!(orig.iter().all(|&id| photocraft_doc::id_step(id).is_some()), "every id is step-keyed: {orig:?}");
}

#[test]
fn adopted_document_moves_the_step_clock_past_its_ids() {
    let doc = {
        let _g = photocraft_doc::IdScope::new(50).enter();
        let mut d = photocraft_doc::Document::new("d", photocraft_doc::Size::new(4, 4), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
        d.layers.push(Layer::raster("L", d.pixel_format()));
        d
    };
    let mut s = Session::new();
    s.add_document(doc, None);
    s.execute("layer.new.layer", json!({})).unwrap();
    let new = s.active().unwrap().active_layer.unwrap().0;
    assert!(photocraft_doc::id_step(new).is_some_and(|step| step > 50), "{new:#x}");
}
