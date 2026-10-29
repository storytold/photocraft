//! Spike for a Fusion-style design timeline: can the session journal be replayed with a step
//! removed? Report only: `cargo test -p photocraft-engine --test it timeline_replay_spike -- --ignored --nocapture`.
use std::collections::HashMap;

use photocraft_doc::{Layer, LayerContent};
use photocraft_engine::Session;
use serde_json::{Value, json};

/// Active layer name after a step, as a proxy for which layer implicit-target steps hit.
fn active_name(s: &Session) -> Option<String> {
    let st = s.active()?;
    st.active_layer.and_then(|id| st.doc.layer(id)).map(|l| l.name.clone())
}

/// (document id, layer id): a smart object's contents document reuses its source layer's id, so a
/// layer id alone is ambiguous across documents.
type Key = (u64, u64);

fn active_id(s: &Session) -> Option<Key> {
    s.active().and_then(|st| st.active_layer.map(|l| (st.doc.id.0, l.0)))
}

/// Every layer of every open document, groups' children included, with group children stripped
/// so a group only counts as touched when the group itself changes.
fn shapes(s: &Session) -> Vec<(Key, Layer)> {
    s.documents()
        .iter()
        .flat_map(|st| {
            let d = st.doc.id.0;
            st.doc.walk().into_iter().map(move |(_, _, l)| {
                let mut l = l.clone();
                if let LayerContent::Group(g) = &mut l.content {
                    g.children.clear();
                }
                ((d, l.id.0), l)
            })
        })
        .collect()
}

fn doc_ids(s: &Session) -> Vec<u64> {
    s.documents().iter().map(|st| st.doc.id.0).collect()
}

fn active_doc(s: &Session) -> Option<u64> {
    s.active().map(|st| st.doc.id.0)
}

fn selected(s: &Session) -> Vec<Key> {
    s.active().map(|st| st.selected_layers.iter().map(|l| (st.doc.id.0, l.0)).collect()).unwrap_or_default()
}

fn layer_ids(s: &Session) -> Vec<u64> {
    shapes(s).into_iter().map(|((_, id), _)| id).collect()
}

/// Layers a step changed, created or removed.
fn touched(before: &[(Key, Layer)], after: &[(Key, Layer)]) -> Vec<Key> {
    let (b, a): (HashMap<_, _>, HashMap<_, _>) = (before.iter().map(|(i, l)| (*i, l)).collect(), after.iter().map(|(i, l)| (*i, l)).collect());
    let mut t: Vec<Key> = b.keys().chain(a.keys()).copied().filter(|i| b.get(i) != a.get(i)).collect();
    t.sort_unstable();
    t.dedup();
    t
}

/// One recorded step, with what a timeline would store beside the journal entry.
struct Step {
    id: String,
    /// Params with layer names resolved to ids.
    params: Value,
    /// Active layer when the step ran: its implicit target.
    before: Option<Key>,
    /// Selected layers when the step ran: multi-layer commands act on all of them.
    selected: Vec<Key>,
    /// Document the step ran in.
    doc: Option<u64>,
    /// Layers the step created, in walk order.
    created: Vec<Key>,
    created_docs: Vec<u64>,
    touched: Vec<Key>,
    after: Option<Key>,
    after_name: Option<String>,
}

fn run_step(s: &mut Session, steps: &mut Vec<Step>, id: &str, p: Value) {
    let mut stored = p.clone();
    if let Some(name) = p.get("layer").and_then(Value::as_str) {
        let st = s.active().unwrap();
        let l = st.doc.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap_or_else(|| panic!("{id}: no layer {name:?}")).2.id.0;
        stored["layer"] = json!(l);
    }
    let (before, sel, doc, d0, s0) = (active_id(s), selected(s), active_doc(s), doc_ids(s), shapes(s));
    s.execute(id, p).unwrap_or_else(|e| panic!("{id}: {e}"));
    let s1 = shapes(s);
    let created = s1.iter().map(|(i, _)| *i).filter(|i| !s0.iter().any(|(j, _)| j == i)).collect();
    let created_docs = doc_ids(s).into_iter().filter(|d| !d0.contains(d)).collect();
    steps.push(Step {
        id: id.into(),
        params: stored,
        before,
        selected: sel,
        doc,
        created,
        created_docs,
        touched: touched(&s0, &s1),
        after: active_id(s),
        after_name: active_name(s),
    });
}

fn record_script(script: &[(&str, Value)]) -> (Session, Vec<Step>) {
    let (mut s, mut steps) = (Session::new(), Vec::new());
    for (id, p) in script {
        run_step(&mut s, &mut steps, id, p.clone());
    }
    (s, steps)
}

/// The first script, with one step that names its layer by raw id as the journal stores it.
fn record() -> (Session, Vec<Step>) {
    let (mut s, mut steps) = (Session::new(), Vec::new());
    let mut run = |s: &mut Session, id: &str, p: Value| run_step(s, &mut steps, id, p);
    run(&mut s, "file.new", json!({"width": 64, "height": 48}));
    run(&mut s, "layer.new.layer", json!({"name": "A"}));
    let a = s.active().unwrap().active_layer.unwrap().0;
    run(&mut s, "select.rect", json!({"x": 4, "y": 4, "width": 20, "height": 20}));
    run(&mut s, "edit.fillForeground", json!({}));
    run(&mut s, "select.deselect", json!({}));
    run(&mut s, "layer.new.layer", json!({"name": "B"}));
    run(&mut s, "select.rect", json!({"x": 30, "y": 10, "width": 20, "height": 20}));
    run(&mut s, "edit.fillForeground", json!({}));
    run(&mut s, "select.deselect", json!({}));
    run(&mut s, "filter.blur.gaussianBlur", json!({"radius": 3}));
    run(&mut s, "layer.select", json!({"layer": "A"})); // by name
    run(&mut s, "image.adjustments.invert", json!({}));
    run(&mut s, "layer.duplicate", json!({}));
    run(&mut s, "layer.setProps", json!({"opacity": 0.5}));
    run(&mut s, "layer.setProps", json!({"layer": a, "visible": false})); // by id
    (s, steps)
}

/// Masks, clipping, a smart object with a smart filter, groups, merges, reordering, strokes and
/// an unseeded noise filter.
fn harsh_script() -> Vec<(&'static str, Value)> {
    let stroke = |pts: Value, color: &str| json!({"points": pts, "size": 6, "hardness": 1.0, "color": color});
    vec![
        ("file.new", json!({"width": 64, "height": 48})),
        ("layer.new.layer", json!({"name": "A"})),
        ("paint.stroke", stroke(json!([[4, 4], [40, 30]]), "#ff0000")),
        ("layer.new.layer", json!({"name": "B"})),
        ("select.rect", json!({"x": 20, "y": 8, "width": 30, "height": 30})),
        ("edit.fillForeground", json!({})),
        ("select.deselect", json!({})),
        ("layer.layerMask.revealAll", json!({})),
        ("paint.stroke", json!({"points": [[20, 20], [50, 20]], "size": 8, "hardness": 1.0, "color": "#000000", "target": "mask"})),
        ("layer.newAdjustmentLayer.invert", json!({})),
        ("layer.createClippingMask", json!({})),
        ("layer.select", json!({"layer": "A"})),
        ("filter.noise.addNoise", json!({"amount": 20})),
        ("layer.groupLayers", json!({})),
        ("layer.select", json!({"layer": "A"})),
        ("layer.new.layer", json!({"name": "C"})),
        ("paint.stroke", stroke(json!([[10, 40], [60, 40]]), "#0000ff")),
        ("layer.mergeDown", json!({})),
        ("layer.select", json!({"layer": "B"})),
        ("layer.smartObjects.convertToSmartObject", json!({})),
        ("filter.blur.gaussianBlur", json!({"radius": 2})),
        ("layer.arrange.sendBackward", json!({})),
        ("layer.duplicate", json!({})),
        ("layer.setProps", json!({"opacity": 0.5})),
    ]
}

/// Text and shape layers, a smart object whose contents are edited in their own document twice,
/// a smart filter over it, and multi-layer align and merge.
fn deep_script() -> Vec<(&'static str, Value)> {
    let stroke = |pts: Value, color: &str| json!({"points": pts, "size": 6, "hardness": 1.0, "color": color});
    vec![
        ("file.new", json!({"width": 64, "height": 48})),
        ("type.create", json!({"x": 4, "y": 20, "text": "Hello", "size": 12, "color": "#000000", "name": "T"})),
        ("type.setStyle", json!({"range": [0, 2], "color": "#ff0000"})),
        ("shape.create", json!({"kind": "rect", "rect": [30, 4, 20, 20], "fill": "#0000ff"})),
        ("shape.edit", json!({"name": "S"})),
        ("layer.new.layer", json!({"name": "P"})),
        ("paint.stroke", stroke(json!([[8, 30], [56, 44]]), "#ff00ff")),
        ("layer.smartObjects.convertToSmartObject", json!({})),
        ("layer.smartObjects.editContents", json!({})),
        ("paint.stroke", stroke(json!([[8, 44], [56, 30]]), "#00ff00")),
        ("layer.new.layer", json!({"name": "Inner"})),
        ("paint.stroke", stroke(json!([[30, 30], [30, 46]]), "#ffff00")),
        ("layer.smartObjects.saveContents", json!({})),
        ("document.activate", json!({"document": 0})),
        ("filter.blur.gaussianBlur", json!({"radius": 2})),
        ("layer.smartObjects.editContents", json!({})),
        ("layer.select", json!({"layer": "Inner"})),
        ("image.adjustments.invert", json!({})),
        ("layer.smartObjects.saveContents", json!({})),
        ("document.activate", json!({"document": 0})),
        ("layer.select", json!({"layer": "T"})),
        ("layer.select", json!({"layer": "S", "mode": "add"})),
        ("layer.align.leftEdges", json!({"to": "canvas"})),
        ("layer.mergeLayers", json!({})),
        ("type.create", json!({"x": 4, "y": 44, "text": "World", "size": 10, "color": "#000000", "name": "U"})),
        ("type.setStyle", json!({"fauxBold": true})),
    ]
}

fn flat(s: &Session) -> photocraft_compose::Buffer {
    photocraft_compose::flatten(&s.active().unwrap().doc)
}

/// Today's behaviour: replay the raw journal.
#[test]
#[ignore = "spike report, run with --ignored --nocapture"]
fn timeline_replay_spike() {
    let (orig, steps) = record();
    let journal = orig.journal.clone();

    // Full replay should reproduce the document exactly, ids included.
    let mut full = Session::new();
    for (i, (id, p)) in journal.iter().enumerate() {
        if let Err(e) = full.execute(id, p.clone()) {
            println!("full replay #{i} {id}: ERROR {e}");
        }
    }
    println!("full replay pixels match: {}; ids {:?} vs original {:?}\n", flat(&full) == flat(&orig), layer_ids(&full), layer_ids(&orig));

    // Drop each step in turn (not file.new) and classify what happens to the rest.
    let (mut loud, mut silent) = (0, 0);
    for k in 1..journal.len() {
        let mut s = Session::new();
        let mut report = Vec::new();
        for (i, (id, p)) in journal.iter().enumerate().filter(|(i, _)| *i != k) {
            match s.execute(id, p.clone()) {
                Err(e) => {
                    loud += 1;
                    report.push(format!("  #{i} {id}: ERROR {e}"));
                }
                Ok(_) if active_name(&s) != steps[i].after_name => {
                    silent += 1;
                    report.push(format!("  #{i} {id}: retargeted {:?} -> {:?}", steps[i].after_name, active_name(&s)));
                }
                Ok(_) => {}
            }
        }
        println!("drop #{k} {}: {}", journal[k].0, if report.is_empty() { "clean".into() } else { format!("\n{}", report.join("\n")) });
    }
    println!("\n{} steps; downstream errors: {loud}, silent retargets: {silent}", journal.len());
}

/// How a step uses the active layer. The real version would derive this from each command's
/// `enabled` precondition rather than a table.
#[derive(PartialEq)]
enum Role {
    /// Acts on it: if it came from a removed step, the step fails.
    Target,
    /// Only places new layers next to it: falls back to whatever is active.
    Anchor,
    /// Ignores it.
    Free,
}

fn role(id: &str) -> Role {
    match id {
        "file.new" | "layer.select" | "document.activate" => Role::Free,
        _ if id.starts_with("select.") => Role::Free,
        "layer.new.layer" | "layer.new.group" => Role::Anchor,
        _ if id.starts_with("layer.newAdjustmentLayer.") => Role::Anchor,
        _ => Role::Target,
    }
}

#[derive(Default)]
struct Tally {
    missing: usize,
    error: usize,
    reresolved: usize,
    silent: usize,
}

/// The chosen design, simulated: layers are known by (creating step, ordinal), every step
/// re-selects its recorded target first, and a step whose target came from a removed step fails.
/// Silent = the step changed different layers than when recorded, or left a different one active.
fn replay(steps: &[Step], drop: Option<usize>, t: &mut Tally) -> (Session, Vec<String>) {
    let origin: HashMap<Key, (usize, usize)> =
        steps.iter().enumerate().flat_map(|(i, st)| st.created.iter().enumerate().map(move |(n, &id)| (id, (i, n)))).collect();
    let doc_origin: HashMap<u64, usize> = steps.iter().enumerate().flat_map(|(i, st)| st.created_docs.iter().map(move |&d| (d, i))).collect();
    let mut s = Session::new();
    let mut live: HashMap<(usize, usize), Key> = HashMap::new();
    let mut live_docs: HashMap<usize, u64> = HashMap::new(); // creating step -> replay doc id
    let mut back: HashMap<Key, Key> = HashMap::new(); // replay key -> recorded key
    let mut report = Vec::new();
    for (i, st) in steps.iter().enumerate().filter(|(i, _)| Some(*i) != drop) {
        // Switching tabs is navigation, not a timeline feature: each step records its document.
        if st.id == "document.activate" {
            continue;
        }
        if let Some(d) = st.doc {
            let Some(index) = doc_origin.get(&d).and_then(|o| live_docs.get(o)).and_then(|d| doc_ids(&s).iter().position(|x| x == d)) else {
                t.missing += 1;
                report.push(format!("  #{i} {}: MISSING document made by removed step #{:?}", st.id, doc_origin.get(&d)));
                continue;
            };
            s.set_active(index);
        }
        let resolve = |o: Key| origin.get(&o).and_then(|r| live.get(r)).copied();
        let r = role(&st.id);
        let target = st.before.filter(|_| r != Role::Free);
        let explicit = st.params.get("layer").and_then(Value::as_u64).zip(st.doc).map(|(l, d)| (d, l));
        let gone = |o: &Option<Key>| o.is_some_and(|o| resolve(o).is_none());
        let others: Vec<Key> = if r == Role::Target { st.selected.iter().copied().filter(|&o| Some(o) != st.before).collect() } else { Vec::new() };
        let lost = others.iter().copied().find(|&o| resolve(o).is_none());
        if gone(&explicit) || (r == Role::Target && gone(&target)) || lost.is_some() {
            t.missing += 1;
            let o = explicit.filter(|_| gone(&explicit)).or(lost).or(target).and_then(|o| origin.get(&o));
            report.push(format!("  #{i} {}: MISSING layer made by removed step #{:?}", st.id, o.map(|r| r.0)));
            continue;
        }
        match target.map(&resolve) {
            Some(Some(t)) => {
                // Multi-layer target: the others, then the active one last so it stays active.
                for (n, &o) in others.iter().enumerate() {
                    s.execute("layer.select", json!({"layer": resolve(o).unwrap().1, "mode": if n == 0 { "replace" } else { "add" }})).unwrap();
                }
                s.execute("layer.select", json!({"layer": t.1, "mode": if others.is_empty() { "replace" } else { "add" }})).unwrap();
            }
            Some(None) => {
                t.reresolved += 1;
                report.push(format!("  #{i} {}: anchor gone, placed by {:?}", st.id, active_name(&s)));
            }
            None => {}
        }
        let mut p = st.params.clone();
        if let Some(e) = explicit {
            p["layer"] = json!(resolve(e).unwrap().1);
        }
        let (s0, d0) = (shapes(&s), doc_ids(&s));
        if let Err(e) = s.execute(&st.id, p) {
            t.error += 1;
            report.push(format!("  #{i} {}: ERROR {e}", st.id));
            continue;
        }
        if let Some(d) = doc_ids(&s).into_iter().find(|d| !d0.contains(d)) {
            live_docs.insert(i, d);
        }
        let s1 = shapes(&s);
        let made: Vec<Key> = s1.iter().map(|(i, _)| *i).filter(|i| !s0.iter().any(|(j, _)| j == i)).collect();
        if made.len() != st.created.len() {
            report.push(format!("  #{i} {}: made {} layers, recorded {}", st.id, made.len(), st.created.len()));
        }
        for (n, id) in made.into_iter().enumerate() {
            live.insert((i, n), id);
            if let Some(&rec) = st.created.get(n) {
                back.insert(id, rec);
            }
        }
        // Changing less than recorded is a no-op on missing content; changing a layer that wasn't
        // recorded is a retarget. Free steps don't use the active layer, so it isn't compared.
        let mut hit: Vec<Option<Key>> = touched(&s0, &s1).into_iter().map(|id| back.get(&id).copied()).collect();
        hit.sort_unstable();
        let rec: Vec<Option<Key>> = st.touched.iter().map(|&id| Some(id)).collect();
        let got = active_id(&s).and_then(|a| back.get(&a).copied());
        if hit.iter().any(|h| !rec.contains(h)) || (r != Role::Free && got != st.after) {
            t.silent += 1;
            let show = |v: &[Option<Key>]| v.iter().map(|o| o.map_or("?".into(), |k| format!("{}:{}", k.0, k.1))).collect::<Vec<_>>().join(",");
            report.push(format!(
                "  #{i} {}: SILENT touched [{}] recorded [{}]; active {:?} vs {:?}",
                st.id,
                show(&hit),
                show(&rec),
                active_name(&s),
                st.after_name
            ));
        }
    }
    (s, report)
}

fn run_design(label: &str, orig: &Session, steps: &[Step]) -> Tally {
    println!("===== {label}: {} steps =====", steps.len());
    for (i, st) in steps.iter().enumerate() {
        println!("  #{i:<2} {:<42} created {:?} touched {:?}", st.id, st.created, st.touched);
    }
    let mut t = Tally::default();
    let (full, report) = replay(steps, None, &mut t);
    println!("no removal: {}; pixels match: {}", if report.is_empty() { "clean" } else { "BROKEN" }, flat(&full) == flat(orig));
    for line in &report {
        println!("{line}");
    }
    let mut t = Tally::default();
    for k in 1..steps.len() {
        let (_, report) = replay(steps, Some(k), &mut t);
        println!("remove #{k} {}: {}", steps[k].id, if report.is_empty() { "clean".into() } else { format!("\n{}", report.join("\n")) });
    }
    println!("\n{label}: missing {}, engine errors {}, anchors re-resolved {}, SILENT {}\n", t.missing, t.error, t.reresolved, t.silent);
    t
}

/// What suppressing step 1 costs without checkpoints: the harsh script at photo size, replayed.
#[test]
#[ignore = "spike report, run with --ignored --nocapture"]
fn timeline_replay_cost() {
    fn scale(v: &mut Value, f: f64) {
        match v {
            Value::Number(n) => *v = json!(n.as_f64().unwrap() * f),
            Value::Array(a) => a.iter_mut().for_each(|x| scale(x, f)),
            Value::Object(o) => {
                for (k, x) in o.iter_mut() {
                    if matches!(k.as_str(), "points" | "x" | "y" | "width" | "height" | "size" | "radius") {
                        scale(x, f);
                    }
                }
            }
            _ => {}
        }
    }
    let mut script = harsh_script();
    for (_, p) in &mut script {
        scale(p, 100.0); // 64x48 -> 6400x4800, about 31 MP
    }
    let t0 = std::time::Instant::now();
    let (_, steps) = record_script(&script);
    let recorded = t0.elapsed();
    let t0 = std::time::Instant::now();
    let (s, report) = replay(&steps, Some(1), &mut Tally::default());
    println!("31 MP, {} steps: recorded in {recorded:?}; replay without step 1 in {:?} ({} report lines)", steps.len(), t0.elapsed(), report.len());
    drop(s);
}

#[test]
#[ignore = "spike report, run with --ignored --nocapture"]
fn timeline_replay_stable_refs() {
    let (orig, steps) = record();
    let t = run_design("simple", &orig, &steps);
    let (orig, steps) = record_script(&harsh_script());
    run_design("harsh", &orig, &steps);
    assert_eq!(t.silent, 0, "simple script: every break is loud");
}

#[test]
#[ignore = "spike report, run with --ignored --nocapture"]
fn timeline_replay_deep() {
    let (mut orig, steps) = record_script(&deep_script());
    orig.set_active(0);
    run_design("deep", &orig, &steps);
}
