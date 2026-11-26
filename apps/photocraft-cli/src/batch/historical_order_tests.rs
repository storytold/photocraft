use crate::document::historical_order_tests::rgb16;
use crate::test_support::{Invoke, invoke};
use serde_json::json;
use std::path::Path;

fn run_batch(input: &Path, output: &Path, actions: &Path, order: &str, recursive: bool) -> Invoke {
    let mut args = vec![
        "batch",
        "--in",
        input.to_str().unwrap(),
        "--out",
        output.to_str().unwrap(),
        "--actions",
        actions.to_str().unwrap(),
        "--format",
        "pcraft",
        "--no-gpu",
        "--historical-xcf-sample-order",
        order,
    ];
    if recursive {
        args.push("--recursive");
    }
    invoke(&args)
}

fn assert_native(path: &Path, expected: [u16; 4], order: &str) {
    let bytes = std::fs::read(path).unwrap();
    let doc = photocraft_format::load_from_bytes(&bytes).unwrap();
    let pixel = doc.layers[0].surface().unwrap().read_region(photocraft_doc::Rect::new(0, 0, 1, 1));
    assert_eq!(pixel, expected.map(|value| f32::from(value) / 65535.));
    let Some(photocraft_doc::import_conversion::ImportConversion::Report(report)) = doc.metadata.import_conversion.as_deref() else {
        panic!("Batch output retains the explicit order evidence");
    };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.historical_sample_order" && entry.native_description.contains(order)));
}

#[test]
fn recorded_same_file_alias_inherits_order_through_called_actions_in_both_listing_modes() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in");
    std::fs::create_dir(&input).unwrap();
    let source = input.join("source.xcf");
    let alias = dir.path().join("same-source.data");
    let actions = dir.path().join("actions.json");
    for (order, little) in [("little-endian", true), ("big-endian", false)] {
        let original = rgb16(11, little);
        std::fs::write(&source, &original).unwrap();
        std::fs::hard_link(&source, &alias).unwrap();
        let action_set = json!({"version": 1, "actions": [
            {"name": "Outer", "steps": [["actions.play", {"action": "Inner"}]]},
            {"name": "Inner", "steps": [["file.openAs", {"path": alias, "as": "xcf", "historicalXcfSampleOrder": null}]]}
        ]});
        std::fs::write(&actions, action_set.to_string()).unwrap();
        for recursive in [false, true] {
            let output = dir.path().join(format!("out-{order}-{recursive}"));
            let mut args = vec![
                "batch",
                "--in",
                input.to_str().unwrap(),
                "--out",
                output.to_str().unwrap(),
                "--actions",
                actions.to_str().unwrap(),
                "--action",
                "Outer",
                "--format",
                "pcraft",
                "--no-gpu",
                "--historical-xcf-sample-order",
                order,
            ];
            if recursive {
                args.push("--recursive");
            }
            let result = invoke(&args);
            assert_eq!(result.code, 0, "{}{}", result.stdout, result.stderr);
            assert_native(&output.join("source.pcraft"), [0x1234, 0x5678, 0x9abc, 0xffff], order);
            assert_eq!(std::fs::read(&source).unwrap(), original);
            assert_eq!(std::fs::read_to_string(&actions).unwrap(), action_set.to_string());
        }
        std::fs::remove_file(&alias).unwrap();
    }
}

#[test]
fn recorded_open_does_not_inherit_another_source_choice_and_keeps_explicit_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in");
    std::fs::create_dir(&input).unwrap();
    let source = input.join("source.xcf");
    let other = dir.path().join("other.xcf");
    let actions = dir.path().join("actions.json");
    let original = rgb16(7, true);
    let foreign = rgb16(7, false);
    std::fs::write(&source, &original).unwrap();
    std::fs::write(&other, &foreign).unwrap();
    for (label, path, explicit, succeeds, expected) in [
        ("different-missing", &other, None, false, [0; 4]),
        ("different-explicit", &other, Some("big-endian"), true, [0x1234, 0x5678, 0x9abc, 0xffff]),
        ("same-explicit", &source, Some("big-endian"), true, [0x3412, 0x7856, 0xbc9a, 0xffff]),
    ] {
        std::fs::write(&actions, json!([["file.openAs", {"path": path, "as": "xcf", "historicalXcfSampleOrder": explicit}]]).to_string()).unwrap();
        let output = dir.path().join(label);
        let result = run_batch(&input, &output, &actions, "little-endian", false);
        if succeeds {
            assert_eq!(result.code, 0, "{}{}", result.stdout, result.stderr);
            assert_native(&output.join("source.pcraft"), expected, "big-endian");
        } else {
            assert_eq!(result.code, 1, "{}{}", result.stdout, result.stderr);
            assert!(result.stderr.contains("sample byte order"), "{}", result.stderr);
            assert!(!output.join("source.pcraft").exists());
        }
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read(&other).unwrap(), foreign);
    }
}

#[test]
fn recorded_order_is_rebound_per_input_without_changing_approval_or_source_actions() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in");
    std::fs::create_dir(&input).unwrap();
    let first = input.join("a.xcf");
    let second = input.join("b.xcf");
    let original = rgb16(11, true);
    for path in [&first, &second] {
        std::fs::write(path, &original).unwrap();
    }
    let steps = json!([["file.openAs", {"path": first, "as": "xcf", "approveHighMemory": false}]]).to_string();
    let actions = dir.path().join("actions.json");
    std::fs::write(&actions, &steps).unwrap();
    let output = dir.path().join("out");
    let result = run_batch(&input, &output, &actions, "little-endian", false);
    assert_eq!(result.code, 1, "{}{}", result.stdout, result.stderr);
    assert!(result.stdout.contains("1 succeeded, 1 failed"), "{}", result.stdout);
    assert_native(&output.join("a.pcraft"), [0x1234, 0x5678, 0x9abc, 0xffff], "little-endian");
    assert!(!output.join("b.pcraft").exists());
    assert_eq!(std::fs::read_to_string(&actions).unwrap(), steps);
    assert_eq!(std::fs::read(&first).unwrap(), original);
    assert_eq!(std::fs::read(&second).unwrap(), original);
}

#[test]
fn nested_batch_uses_its_own_order_and_restores_outer_scope() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in");
    std::fs::create_dir(&input).unwrap();
    let source = input.join("source.xcf");
    let original = rgb16(7, true);
    std::fs::write(&source, &original).unwrap();
    let actions = dir.path().join("actions.json");
    for (label, inner_order) in [("missing", None), ("explicit", Some("big-endian"))] {
        let spec = json!({"version": 1, "actions": [
            {"name": "Outer", "steps": [
                ["file.automate.batch", {"source": "open", "steps": [["file.openAs", {
                    "path": source, "as": "xcf", "historicalXcfSampleOrder": inner_order
                }]]}],
                ["file.openAs", {"path": source, "as": "xcf"}]
            ]}
        ]});
        std::fs::write(&actions, spec.to_string()).unwrap();
        let output = dir.path().join(label);
        let result = run_batch(&input, &output, &actions, "little-endian", false);
        if inner_order.is_some() {
            assert_eq!(result.code, 0, "{}{}", result.stdout, result.stderr);
            assert_native(&output.join("source.pcraft"), [0x1234, 0x5678, 0x9abc, 0xffff], "little-endian");
        } else {
            assert_eq!(result.code, 1, "{}{}", result.stdout, result.stderr);
            assert!(result.stderr.contains("sample byte order"), "{}", result.stderr);
            assert!(!output.join("source.pcraft").exists());
        }
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&actions).unwrap(), spec.to_string());
    }
}

#[test]
fn missing_order_and_invalid_flag_produce_no_documents() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("source.xcf"), rgb16(7, true)).unwrap();
    let actions = dir.path().join("actions.json");
    std::fs::write(&actions, "[]").unwrap();
    let missing_output = dir.path().join("missing-choice");
    let missing = invoke(&[
        "batch",
        "--in",
        input.to_str().unwrap(),
        "--out",
        missing_output.to_str().unwrap(),
        "--actions",
        actions.to_str().unwrap(),
        "--format",
        "pcraft",
        "--no-gpu",
    ]);
    assert_eq!(missing.code, 1, "{}{}", missing.stdout, missing.stderr);
    assert!(missing.stderr.contains("sample byte order"), "{}", missing.stderr);
    assert!(!missing_output.join("source.pcraft").exists());
    let output = dir.path().join("invalid-out");
    let result = run_batch(&input, &output, &actions, "native", false);
    assert_eq!(result.code, 2, "{}", result.stderr);
    assert!(!output.exists());
}
