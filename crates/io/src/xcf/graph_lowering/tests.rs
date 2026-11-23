use super::*;

fn prepare_text(raw: &str) -> Result<Plan> {
    prepare(raw, &mut Budget::default(), &Interrupt::NONE)
}

#[test]
fn ordered_branch_and_reused_labels_lower_to_backward_native_inputs() {
    let plan =
        prepare_text("id=original invert-linear id=changed over aux=[ref=original gegl:invert-linear] ref=changed over aux=[ref=original invert-linear]")
            .unwrap();
    assert_eq!(plan.output, 5);
    assert_eq!(plan.steps.len(), 5);
    assert!(
        matches!(&plan.steps[0], Step::Filter { input: 0, filter } if filter.command == "image.adjustments.invert" && filter.params == serde_json::json!({}) && filter.opacity == 1. && filter.visible)
    );
    assert!(matches!(&plan.steps[1], Step::Filter { input: 0, .. }));
    assert!(matches!(&plan.steps[2], Step::Join { below: 1, above: 2, blend: BlendMode::Normal }));
    assert!(matches!(&plan.steps[3], Step::Filter { input: 0, .. }));
    assert!(matches!(&plan.steps[4], Step::Join { below: 1, above: 4, blend: BlendMode::Normal }));
    assert!(plan.note.contains("may differ"), "native approximation remains explicit");
    let sequential = prepare_text("invert-linear gegl:invert-linear").unwrap();
    assert!(matches!(&sequential.steps[1], Step::Filter { input: 1, .. }));
    assert!(prepare_text("id=original ref=original").unwrap().steps.is_empty());
}

#[test]
fn unsupported_or_unresolved_graphs_refuse_the_whole_native_plan() {
    for raw in [
        "invert-linear unknown-operation",
        "invert-linear arbitrary=1",
        "invert-linear over",
        "id=in over aux=[invert-linear]",
        "id=in over aux=[ref=in invert-linear] srgb=true",
        "ref=future invert-linear id=future",
        "id=in id=in invert-linear",
        "id=in over aux=[ref=missing invert-linear]",
        "invert-linear ]",
        "GEGL:invert-linear",
        "svg:invert-linear",
    ] {
        assert!(prepare_text(raw).is_err(), "{raw}");
    }
    for alias in ["over", "gegl:over", "svg:src-over"] {
        assert!(matches!(&prepare_text(&format!("id=in {alias} aux=[ref=in invert-linear]")).unwrap().steps[1], Step::Join { below: 0, above: 1, .. }));
    }
}

#[test]
fn manually_modified_graph_cannot_supply_forward_edges_or_duplicate_identity() {
    let mut budget = Budget::default();
    let mut parsed = graph::parse("invert-linear invert-linear", DECLARATIONS, &mut budget, &|| true).unwrap();
    parsed.nodes[1].input = Some(graph::Input::Node(1));
    assert!(matches!(lower(&parsed, &Interrupt::NONE), Err(Failure::Invalid(_))));
    parsed.nodes[1].input = Some(graph::Input::Node(0));
    parsed.nodes[1].source_ordinal = 0;
    assert!(matches!(lower(&parsed, &Interrupt::NONE), Err(Failure::Invalid(_))));
    parsed.nodes[1].source_ordinal = 1;
    parsed.output = graph::Input::Node(2);
    assert!(matches!(lower(&parsed, &Interrupt::NONE), Err(Failure::Invalid(_))));
    parsed.output = graph::Input::Node(1);
    parsed.nodes[1].operation = "unadmitted".into();
    assert!(matches!(lower(&parsed, &Interrupt::NONE), Err(Failure::Unsupported(_))));
}

#[test]
fn shared_budget_and_cancellation_refusals_never_publish_a_plan() {
    let mut tight = Budget::new(1 << 20, 1);
    assert!(matches!(prepare("invert-linear", &mut tight, &Interrupt::NONE), Err(Failure::Source(craft_xcf::effects::Error::Limit(_)))));
    assert_eq!(tight.remaining_work(), 0);
    let cancelled = || true;
    let ctl = Interrupt::cancel_only(&cancelled);
    let mut budget = Budget::default();
    let before = budget.remaining_work();
    assert!(matches!(prepare("invert-linear", &mut budget, &ctl), Err(Failure::Cancelled)));
    assert_eq!(budget.remaining_work(), before);
    let parsed = graph::parse("invert-linear", DECLARATIONS, &mut budget, &|| true).unwrap();
    assert!(matches!(lower(&parsed, &ctl), Err(Failure::Cancelled)));
}

#[test]
fn decomposed_tone_nodes_keep_exact_f64_controls_and_resolve_references_to_final_results() {
    let plan =
        prepare_text("brightness-contrast brightness=0.25 contrast=-2 id=adjusted exposure exposure=1 ref=adjusted levels in-low=0.1 in-high=0.9").unwrap();
    assert_eq!(plan.steps.len(), 5);
    assert_eq!(plan.output, 5);
    assert!(matches!(&plan.steps[0], Step::Filter { input: 0, filter } if filter.command == "image.adjustments.invert"));
    assert!(matches!(&plan.steps[3], Step::Filter { input: 3, filter } if filter.command == "image.adjustments.exposure"));
    assert!(matches!(&plan.steps[4], Step::Filter { input: 3, filter } if filter.command == "image.adjustments.levels"));
    let literal = prepare_text("gegl:exposure black-level=0.0123456789012345 exposure=0.1234567890123456").unwrap();
    let Step::Filter { filter, .. } = &literal.steps[0] else { panic!("native exposure") };
    assert_eq!(filter.params["exposure"].as_f64(), Some(0.1234567890123456));
    assert_eq!(filter.params["offset"].as_f64(), Some(-0.0123456789012345 * 0.1234567890123456_f64.exp2()));
    let branch = prepare_text("id=in over aux=[ref=in brightness-contrast brightness=0.25 contrast=-2 exposure exposure=1]").unwrap();
    assert_eq!(branch.output, 5);
    assert!(matches!(&branch.steps[4], Step::Join { below: 0, above: 4, .. }));
}

#[test]
fn graph_tone_domain_and_control_caps_refuse_without_narrowing_or_guessing() {
    for raw in [
        "exposure black-level=0.10000000000000002",
        "brightness-contrast contrast=5.000000000000001",
        "levels in-low=0.5 in-high=0.5",
        "levels out-low=0.9 out-high=0.1",
        "levels gamma=1",
        "exposure exposure=NaN",
        "exposure exposure=1 exposure=2",
        "gimp:exposure exposure=1",
    ] {
        assert!(prepare_text(raw).is_err(), "{raw}");
    }
    let flood = ["brightness-contrast brightness=3 contrast=-5"; 9].join(" ");
    assert!(prepare_text(&flood).is_err(), "native controls remain bounded independently of source node count");
}
