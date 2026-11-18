//! The Layers panel filter over PhotoCraft's layers: what the app adds to `listed` (the fill rule,
//! missing links) and the control channel's reading of the criteria (`patched`).

use craft::layers::ui::filter::{EffectKind, FilterBy, FilterKind, LayerAttribute, LayerFilter, SmartKind};
use photocraft_color::BlendMode;
use photocraft_doc::{Document, LabelColor, Layer};
use photocraft_engine::Session;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::panels::layers_tree::{blend_key, ui_label};

/// `doc`'s layers, top to bottom, groups before their contents.
fn top_down(doc: &Document) -> Vec<&Layer> {
    fn add<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
        for l in layers.iter().rev() {
            out.push(l);
            add(l.children().unwrap_or_default(), out);
        }
    }
    let mut out = Vec::new();
    add(&doc.layers, &mut out);
    out
}

/// The names the Layers panel lists under `app`'s filter, top to bottom: every layer while the
/// filter is off, else those `PhotoLayers::listed` keeps.
pub(super) fn listed_app(app: &PhotocraftApp) -> Vec<String> {
    use craft::layers::ui::tree::LayerTree;
    let filter = app.ui.layer_filter.clone();
    let st = app.session.active().unwrap();
    let tree = crate::panels::layers_tree::PhotoLayers::new(app, st, &filter, false, None);
    top_down(&st.doc).into_iter().filter(|l| !filter.active() || tree.listed(&l.id, &filter)).map(|l| l.name.clone()).collect()
}

/// The names `f` lists in a session holding `doc`.
pub(super) fn listed(doc: &Document, f: &LayerFilter) -> Vec<String> {
    let mut s = Session::new();
    s.add_document(doc.clone(), None);
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.ui.layer_filter = f.clone();
    listed_app(&app)
}

/// A filter of type `by` with `edit` applied to the other criteria.
pub(super) fn filter(by: FilterBy, edit: impl FnOnce(&mut LayerFilter)) -> LayerFilter {
    let mut f = LayerFilter::default();
    f.by = by;
    edit(&mut f);
    f
}

/// One table over `patched`: each field of the protocol, fields left out, and the refusals.
#[test]
fn patched_reads_the_protocol_and_refuses_bad_values() {
    let default = LayerFilter::default();
    let ink = filter(FilterBy::Name, |f| f.name = "ink".into());
    let red = filter(FilterBy::Kind, |f| f.color = ui_label(LabelColor::Red));
    let accepted: Vec<(LayerFilter, Value, LayerFilter)> = vec![
        (default.clone(), json!({"by": "artboard"}), filter(FilterBy::Artboard, |_| {})),
        (default.clone(), json!({"by": "group"}), filter(FilterBy::Group, |_| {})),
        (default.clone(), json!({"by": "name", "name": "ink"}), ink.clone()),
        (
            default.clone(),
            json!({"by": "smartObject", "smart": ["missing", "embedded", "missing"]}),
            filter(FilterBy::SmartObject, |f| f.smart = vec![SmartKind::Missing, SmartKind::Embedded]),
        ),
        (filter(FilterBy::SmartObject, |f| f.smart = vec![SmartKind::Linked]), json!({"smart": []}), filter(FilterBy::SmartObject, |_| {})),
        (
            ink.clone(),
            json!({"by": "mode", "mode": "Multiply", "kinds": ["type", "pixel", "type"]}),
            filter(FilterBy::Mode, |f| {
                f.name = "ink".into();
                f.mode = blend_key(BlendMode::Multiply);
                f.kinds = vec![FilterKind::Type, FilterKind::Pixel];
            }),
        ),
        (default.clone(), json!({"mode": "Normal"}), default.clone()),
        (default.clone(), json!({"kinds": ["smart"]}), filter(FilterBy::Kind, |f| f.kinds = vec![FilterKind::SmartObject])),
        (default.clone(), json!({"by": "effect", "effect": "overlay"}), filter(FilterBy::Effect, |f| f.effect = EffectKind::Overlay)),
        (default.clone(), json!({"by": "color", "color": "red"}), filter(FilterBy::Color, |f| f.color = ui_label(LabelColor::Red))),
        (red, json!({"color": "none"}), default.clone()),
        (ink.clone(), json!({}), ink.clone()),
    ];
    for (start, patch, want) in accepted {
        assert_eq!(patched(&start, &patch).unwrap(), want, "{patch}");
    }
    for attribute in LayerAttribute::ALL {
        let f = patched(&default, &json!({"by": "attribute", "attribute": attribute})).unwrap();
        assert_eq!((f.by, f.attribute), (FilterBy::Attribute, attribute));
    }

    let refused = [
        (json!("name"), "object"),
        (json!({"by": "colour"}), "colour"),
        (json!({"by": "smart"}), "smartObject"),
        (json!({"by": "kind", "extra": true}), "extra"),
        (json!({"kinds": ["pixels"]}), "pixels"),
        (json!({"kinds": "pixel"}), "list"),
        (json!({"name": 3}), "name"),
        (json!({"mode": "Nonsense"}), "Nonsense"),
        (json!({"mode": 4}), "mode"),
        (json!({"color": "crimson"}), "crimson"),
        (json!({"color": 2}), "color"),
        (json!({"effect": "glow"}), "dropShadow"),
        (json!({"effect": 4}), "effect"),
        (json!({"attribute": "hidden"}), "noAdvancedBlending"),
        (json!({"attribute": 3}), "attribute"),
        (json!({"attribute": null}), "attribute"),
        (json!({"smart": ["library"]}), "library"),
        (json!({"smart": "linked"}), "list"),
        (json!({"by": "kind", "smart": [1]}), "smart"),
    ];
    for (patch, needle) in refused {
        let err = patched(&ink, &patch).expect_err(&patch.to_string());
        assert!(err.contains(needle), "{patch}: {err}");
    }
    let err = patched(&default, &json!({"attribute": "hidden"})).unwrap_err();
    assert!(err.contains("hidden") && err.contains("notVisible"), "{err}");
    let err = patched(&default, &json!({"by": "nope"})).unwrap_err();
    assert!(err.contains("group"), "the error lists group: {err}");

    // The Mode list has no Pass Through (#10878).
    let err = patched(&default, &json!({"mode": "Pass Through"})).unwrap_err();
    assert!(err.contains("Pass Through"), "the error names what was asked for: {err}");
    for m in BlendMode::LAYER_MODES {
        assert!(err.contains(m.label()), "the error names the valid mode {}: {err}", m.label());
    }
}

/// Creates a PNG fixture and returns its temporary directory with the file path.
fn art_file() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("art.png");
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "#ff0000"})).unwrap();
    s.execute("file.saveACopy", json!({"path": file.to_string_lossy(), "layers": false})).unwrap();
    (directory, file)
}

/// "Linked art" (linked to `file`) above "Embedded art" (a copy of `file`), "Plain" and
/// "Background".
fn smart_session(file: &std::path::Path) -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Plain"})).unwrap();
    for (command, name) in [("file.placeEmbedded", "Embedded art"), ("file.placeLinked", "Linked art")] {
        s.execute(command, json!({"path": file.to_string_lossy()})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap().0;
        s.execute("layer.setProps", json!({"layer": id, "name": name})).unwrap();
    }
    s
}

#[test]
fn the_smart_object_filter_lists_linked_missing_and_embedded_smart_objects() {
    let (_directory, file) = art_file();
    let mut app = PhotocraftApp::new(smart_session(&file), crate::Services::default());
    let by_smart = |app: &mut PhotocraftApp, smart: &[SmartKind]| {
        app.ui.layer_filter = filter(FilterBy::SmartObject, |f| f.smart = smart.to_vec());
        listed_app(app)
    };
    assert_eq!(by_smart(&mut app, &[]), ["Linked art", "Embedded art", "Plain", "Background"], "no toggle on: every layer");
    assert_eq!(by_smart(&mut app, &[SmartKind::Linked]), ["Linked art"]);
    assert_eq!(by_smart(&mut app, &[SmartKind::Embedded]), ["Embedded art"]);
    assert_eq!(by_smart(&mut app, &[SmartKind::Linked, SmartKind::Embedded]), ["Linked art", "Embedded art"]);
    assert_eq!(by_smart(&mut app, &[SmartKind::Missing]), Vec::<String>::new(), "the file is there");
    std::fs::remove_file(&file).unwrap();
    app.run("layer.smartObjects.updateChanged", json!({})).unwrap();
    assert_eq!(by_smart(&mut app, &[SmartKind::Missing]), ["Linked art"]);
    assert_eq!(by_smart(&mut app, &[SmartKind::Missing, SmartKind::Embedded]), ["Linked art", "Embedded art"]);
}

use super::patched;
