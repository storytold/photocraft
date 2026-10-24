//! Admission tests for already converted native construction data.
use super::effects::{NativeMapping, NativeStage, NativeStructure, native_filter, validate_stages};
use super::*;
use serde_json::json;

fn stage(input: usize, value: serde_json::Value) -> NativeStage {
    NativeStage::Apply { input, mapping: Box::new(NativeMapping { filters: vec![native_filter("image.adjustments.exposure", value)], ..Default::default() }) }
}

#[test]
fn native_plan_parameters_are_charged_across_all_actual_inputs() {
    let first = json!({"exposure": 0.75, "offset": 0.125});
    let second = json!({"exposure": -0.5});
    let expected = serde_json::to_vec(&first).unwrap().len() + serde_json::to_vec(&second).unwrap().len();
    let mapping = NativeMapping {
        structure: Some(NativeStructure::Stages {
            steps: vec![stage(0, first), stage(1, second), NativeStage::Join { below: 1, above: 2, blend: BlendMode::Multiply, opacity: 1.0 }],
            output: 3,
        }),
        ..Default::default()
    };
    assert_eq!(native_parameter_bytes(&mapping).unwrap(), expected);
}

#[test]
fn native_plan_rejects_forward_cycles_nested_plans_and_invalid_outputs_before_materialization() {
    assert!(validate_stages(&[stage(1, json!({}))], 1).is_err());
    assert!(validate_stages(&[stage(0, json!({}))], 2).is_err());
    assert!(validate_stages(&[NativeStage::Join { below: 0, above: 1, blend: BlendMode::Normal, opacity: 1.0 }], 1).is_err());
    assert!(validate_stages(&[NativeStage::Join { below: 0, above: 0, blend: BlendMode::PassThrough, opacity: 1.0 }], 1).is_err());
    for opacity in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
        assert!(validate_stages(&[NativeStage::Join { below: 0, above: 0, blend: BlendMode::Normal, opacity }], 1).is_err());
    }
    assert!(validate_stages(&[NativeStage::Join { below: 0, above: 0, blend: BlendMode::Multiply, opacity: 0.375 }], 1).is_ok());
    assert!(validate_stages(&[NativeStage::Knockout { below: 0, above: 1, opacity: 1.0 }], 1).is_err());
    assert!(validate_stages(&[NativeStage::Knockout { below: 0, above: 0, opacity: f32::NAN }], 1).is_err());
    assert!(validate_stages(&[NativeStage::Knockout { below: 0, above: 0, opacity: 0.5 }], 1).is_ok());
    let nested = NativeStage::Apply {
        input: 0,
        mapping: Box::new(NativeMapping { structure: Some(NativeStructure::Stages { steps: vec![], output: 0 }), ..Default::default() }),
    };
    assert!(validate_stages(&[nested], 1).is_err());
    let too_many = (0..65).map(|index| stage(index, json!({}))).collect::<Vec<_>>();
    assert!(validate_stages(&too_many, 65).is_err());
}

#[test]
fn native_plan_cannot_bypass_the_existing_total_parameter_allowance() {
    let mapping = NativeMapping {
        structure: Some(NativeStructure::Stages { steps: vec![stage(0, json!({"untrusted": "x".repeat(MAX_METADATA_BYTES)}))], output: 1 }),
        ..Default::default()
    };
    assert!(native_parameter_bytes(&mapping).is_err());
}

#[test]
fn captured_resource_stages_share_parameter_admission_and_reject_forward_inputs() {
    use super::effects::{NativeMapCoordinates, NativeMaskComponent};
    let displace =
        |limit| NativeStage::Displace { input: 0, map: 0, coordinates: NativeMapCoordinates::Offsets, scale: 1., wrap: false, metadata_limit: limit };
    let mapped =
        NativeMapping { structure: Some(NativeStructure::Stages { steps: vec![displace(20_000), displace(30_000)], output: 2 }), ..Default::default() };
    assert_eq!(native_parameter_bytes(&mapped).unwrap(), 50_000);
    let excessive =
        NativeMapping { structure: Some(NativeStructure::Stages { steps: vec![displace(MAX_METADATA_BYTES), displace(1)], output: 2 }), ..Default::default() };
    assert!(native_parameter_bytes(&excessive).is_err());
    assert!(validate_stages(&[NativeStage::Channel { input: 1, component: NativeMaskComponent::Alpha }], 1).is_err());
    assert!(validate_stages(&[NativeStage::Mask { input: 0, mask: 1, component: NativeMaskComponent::Composite, invert: false, scale: 1. }], 1).is_err());
    assert!(validate_stages(&[NativeStage::Mask { input: 0, mask: 0, component: NativeMaskComponent::Alpha, invert: true, scale: f32::NAN }], 1).is_err());
    assert!(validate_stages(&[displace(0)], 1).is_err());
    assert!(
        validate_stages(&[NativeStage::Align { input: 0, target: 1, horizontal: 0.5, vertical: 0.5, margin_x: 0., margin_y: 0., snap: false }], 1).is_err()
    );
    assert!(
        validate_stages(&[NativeStage::Align { input: 0, target: 0, horizontal: f64::NAN, vertical: 0.5, margin_x: 0., margin_y: 0., snap: false }], 1)
            .is_err()
    );
}
