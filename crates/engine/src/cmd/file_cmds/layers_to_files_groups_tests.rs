//! #13957: Layers to Files walks groups: every layer in a group gets a file named
//! `<prefix>_<group index>s_<index in group>_<name>`, a group gets none, and art layers and groups
//! are counted separately at each level.

use super::*;
use photocraft_doc::TextLayer;

/// The issue's `l2f-test`, top to bottom: `Type` (text), group `Grp` holding `Inner`, `Hidden`
/// (hidden), `Blue Half`, `Red`, `Background`.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 15, "name": "l2f-test"})).unwrap();
    for name in ["Red", "Blue Half", "Hidden", "Inner"] {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
        if name == "Hidden" {
            s.execute("layer.setProps", json!({"visible": false})).unwrap();
        }
    }
    s.edit("groups", |doc, _| {
        let inner = doc.layers.pop().unwrap();
        doc.layers.push(Layer::group("Grp", vec![inner]));
        doc.layers.push(Layer::new("Type", LayerContent::Text(TextLayer { text: "T".into(), ..Default::default() })));
        Ok(())
    })
    .unwrap();
    s
}

/// Runs the export with `params` and returns the report and each file's bytes by name.
fn export(s: &Session, params: Value) -> (LayerExportReport, Vec<(String, Vec<u8>)>) {
    let mut written = Vec::new();
    let report = export_layers_report_with(s, &params, |name, bytes| {
        written.push((name.to_string(), bytes.to_vec()));
        Ok(())
    })
    .unwrap();
    (report, written)
}

fn skipped(report: &LayerExportReport) -> Vec<&str> {
    report.skipped.iter().map(|l| l.name.as_str()).collect()
}

#[test]
fn layers_in_a_group_get_a_file_each_and_the_group_none() {
    let (report, written) = export(&session(), json!({"format": "psd", "visibleOnly": false}));
    let expected = [
        "l2f-test_0000_Type.psd",
        "l2f-test_0000s_0000_Inner.psd",
        "l2f-test_0001_Hidden.psd",
        "l2f-test_0002_Blue Half.psd",
        "l2f-test_0003_Red.psd",
        "l2f-test_0004_Background.psd",
    ];
    assert_eq!(report.files, expected);
    assert_eq!(written.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), expected);
    assert!(report.skipped.is_empty());
    // Inner's PSD keeps it inside Grp (Pass Through), with nothing else in the file.
    let (_, inner) = &written[1];
    let doc = photocraft_io::import("x.psd", inner).unwrap().document;
    assert_eq!(doc.layers.len(), 1, "{:?}", doc.layers.iter().map(|l| &l.name).collect::<Vec<_>>());
    let grp = &doc.layers[0];
    assert_eq!(grp.name, "Grp");
    assert_eq!(grp.blend, photocraft_doc::BlendMode::PassThrough);
    let children: Vec<&str> = grp.children().unwrap().iter().map(|l| l.name.as_str()).collect();
    assert_eq!(children, ["Inner"]);
}

#[test]
fn nested_groups_repeat_the_pattern_and_count_per_level() {
    let mut s = session();
    // Inside Grp, above Inner: `Deep` holding `Leaf`, then a second top-level group `Second`
    // holding `Other`, above Type.
    s.edit("nest", |doc, _| {
        let leaf = Layer::new("Leaf", LayerContent::Text(TextLayer { text: "L".into(), ..Default::default() }));
        let other = Layer::new("Other", LayerContent::Text(TextLayer { text: "O".into(), ..Default::default() }));
        let grp = doc.layers.iter_mut().find(|l| l.name == "Grp").unwrap();
        grp.children_mut().unwrap().push(Layer::group("Deep", vec![leaf]));
        doc.layers.push(Layer::group("Second", vec![other]));
        Ok(())
    })
    .unwrap();
    let (report, _) = export(&s, json!({"visibleOnly": false}));
    assert_eq!(
        report.files,
        [
            "l2f-test_0000s_0000_Other.png",
            "l2f-test_0000_Type.png",
            "l2f-test_0001s_0000s_0000_Leaf.png",
            "l2f-test_0001s_0000_Inner.png",
            "l2f-test_0001_Hidden.png",
            "l2f-test_0002_Blue Half.png",
            "l2f-test_0003_Red.png",
            "l2f-test_0004_Background.png",
        ]
    );
}

#[test]
fn a_hidden_groups_layers_follow_visible_layers_only() {
    let mut s = session();
    s.edit("hide Grp", |doc, _| {
        doc.layers.iter_mut().find(|l| l.name == "Grp").unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    let (report, written) = export(&s, json!({"visibleOnly": false}));
    assert!(report.files.contains(&"l2f-test_0000s_0000_Inner.png".to_string()), "{:?}", report.files);
    // Its file shows Inner although Grp is hidden in the document.
    let (_, inner) = written.iter().find(|(n, _)| n.contains("Inner")).unwrap();
    let doc = photocraft_io::import("x.png", inner).unwrap().document;
    assert_eq!((doc.size.width, doc.size.height), (20, 15));
    let (report, _) = export(&s, json!({"visibleOnly": true}));
    assert_eq!(report.files, ["l2f-test_0000_Type.png", "l2f-test_0002_Blue Half.png", "l2f-test_0003_Red.png", "l2f-test_0004_Background.png"]);
    assert_eq!(skipped(&report), ["Inner", "Hidden"]);
}

#[test]
fn empty_and_deep_groups_never_panic() {
    let mut s = session();
    s.edit("empty and deep", |doc, _| {
        doc.layers.push(Layer::group("Empty", Vec::new()));
        let mut deep = Layer::new("Bottom", LayerContent::Text(TextLayer { text: "B".into(), ..Default::default() }));
        for i in 0..200 {
            deep = Layer::group(format!("G{i}"), vec![deep]);
        }
        doc.layers.push(deep);
        Ok(())
    })
    .unwrap();
    let e = export_layers_report_with(&s, &json!({"visibleOnly": false}), |_, _| Ok(())).unwrap_err();
    assert!(e.to_string().contains("nested more than"), "{e}");
    let mut s = session();
    s.edit("empty", |doc, _| {
        doc.layers.push(Layer::group("Empty", Vec::new()));
        Ok(())
    })
    .unwrap();
    let (report, _) = export(&s, json!({"visibleOnly": false}));
    // The empty group still takes group number 0000, so Grp is 0001s.
    assert_eq!(report.files[1], "l2f-test_0001s_0000_Inner.png");
}
