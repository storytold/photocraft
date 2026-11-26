//! Pure, import-time native preparation. No source graph is saved or executed.
//! Owner/mask admission and native materialization are deliberately separate.
use craft_xcf::effects::{Budget, graph};
use photocraft_color::BlendMode;
use photocraft_doc::SmartFilter;
use photocraft_raster::Interrupt;

mod tone;

#[cfg(test)]
mod tests;

const MAX_NODES: usize = 64;
const MAX_STEPS: usize = 64;
const BRIGHTNESS: &[graph::Field<'_>] =
    &[graph::Field { name: "brightness", kind: graph::FieldType::Number }, graph::Field { name: "contrast", kind: graph::FieldType::Number }];
const EXPOSURE: &[graph::Field<'_>] =
    &[graph::Field { name: "black-level", kind: graph::FieldType::Number }, graph::Field { name: "exposure", kind: graph::FieldType::Number }];
const LEVELS: &[graph::Field<'_>] = &[
    graph::Field { name: "in-low", kind: graph::FieldType::Number },
    graph::Field { name: "in-high", kind: graph::FieldType::Number },
    graph::Field { name: "out-low", kind: graph::FieldType::Number },
    graph::Field { name: "out-high", kind: graph::FieldType::Number },
];
const AUX: &[graph::Field<'_>] = &[graph::Field { name: "aux", kind: graph::FieldType::Input }];
const DECLARATIONS: &[graph::Operation<'_>] = &[
    graph::Operation { name: "invert-linear", fields: &[] },
    graph::Operation { name: "gegl:invert-linear", fields: &[] },
    graph::Operation { name: "brightness-contrast", fields: BRIGHTNESS },
    graph::Operation { name: "gegl:brightness-contrast", fields: BRIGHTNESS },
    graph::Operation { name: "exposure", fields: EXPOSURE },
    graph::Operation { name: "gegl:exposure", fields: EXPOSURE },
    graph::Operation { name: "levels", fields: LEVELS },
    graph::Operation { name: "gegl:levels", fields: LEVELS },
    graph::Operation { name: "over", fields: AUX },
    graph::Operation { name: "gegl:over", fields: AUX },
    graph::Operation { name: "svg:src-over", fields: AUX },
];

#[derive(Debug, PartialEq, thiserror::Error)]
pub(super) enum Failure {
    #[error("cancelled")]
    Cancelled,
    #[error("invalid graph: {0}")]
    Invalid(&'static str),
    #[error("unsupported native graph: {0}")]
    Unsupported(&'static str),
    #[error("native graph preparation allocation refused")]
    Storage,
    #[error(transparent)]
    Source(craft_xcf::effects::Error),
}
type Result<T> = std::result::Result<T, Failure>;

/// Index zero is the admitted owner input. Each step adds one native result;
/// only earlier results are referenced. These transient indices are not saved.
#[derive(Debug, PartialEq)]
pub(super) enum Step {
    Filter { input: usize, filter: SmartFilter },
    Join { below: usize, above: usize, blend: BlendMode },
}

#[derive(Debug, PartialEq)]
pub(super) struct Plan {
    pub steps: Vec<Step>,
    pub output: usize,
    /// Native encoding/range/compositing policy is an explicit approximation,
    /// matching the earlier adapter; syntax admission is not a fidelity claim.
    pub note: &'static str,
}

fn check(ctl: &Interrupt<'_>) -> Result<()> {
    ctl.check().map_err(|_| Failure::Cancelled)
}

/// Parse with the document's existing shared syntax/work budget, then prepare all
/// nodes privately. Failure never publishes a prefix of the native plan.
pub(super) fn prepare(raw: &str, budget: &mut Budget, ctl: &Interrupt<'_>) -> Result<Plan> {
    check(ctl)?;
    let parsed = graph::parse(raw, DECLARATIONS, budget, &|| ctl.check().is_ok()).map_err(|error| match error {
        craft_xcf::effects::Error::Source(craft_xcf::Error::Cancelled) => Failure::Cancelled,
        error => Failure::Source(error),
    })?;
    lower(&parsed, ctl)
}

fn input(input: graph::Input, before: usize) -> Result<usize> {
    match input {
        graph::Input::Source => Ok(0),
        graph::Input::Node(index) if index < before => index.checked_add(1).ok_or(Failure::Invalid("native input overflow")),
        _ => Err(Failure::Invalid("input is not an earlier graph result")),
    }
}

fn native_input(input: graph::Input, before: usize, results: &[usize]) -> Result<usize> {
    match input {
        graph::Input::Source => Ok(0),
        graph::Input::Node(index) if index < before => results.get(index).copied().ok_or(Failure::Invalid("source result has no native output")),
        _ => Err(Failure::Invalid("input is not an earlier graph result")),
    }
}

/// The shared graph is inert public data. Recheck bounds and all edges here so a
/// manually constructed graph cannot bypass native admission or panic indexing.
pub(super) fn lower(source: &graph::Graph<'_>, ctl: &Interrupt<'_>) -> Result<Plan> {
    check(ctl)?;
    if source.nodes.len() > MAX_NODES || source.labels.len() > MAX_NODES {
        return Err(Failure::Invalid("node or label limit exceeded"));
    }
    if !source.diagnostics.is_empty() {
        return Err(Failure::Unsupported("unknown operations or properties remain"));
    }
    for (index, label) in source.labels.iter().enumerate() {
        check(ctl)?;
        if label.name.is_empty()
            || label.name.len() > 128
            || label.name.chars().any(|ch| ch.is_whitespace() || ch.is_control() || matches!(ch, '=' | '[' | ']'))
            || source.labels.iter().take(index).any(|earlier| earlier.name == label.name)
        {
            return Err(Failure::Invalid("invalid or duplicate label"));
        }
        input(label.input, source.nodes.len())?;
    }
    let mut steps = Vec::new();
    steps.try_reserve_exact(MAX_STEPS).map_err(|_| Failure::Storage)?;
    let mut results = Vec::new();
    results.try_reserve_exact(source.nodes.len()).map_err(|_| Failure::Storage)?;
    let mut ordinals = 0u64;
    for (index, node) in source.nodes.iter().enumerate() {
        check(ctl)?;
        if node.source_ordinal >= source.nodes.len() {
            return Err(Failure::Invalid("source ordinal is outside the graph"));
        }
        let bit = 1u64.checked_shl(node.source_ordinal as u32).ok_or(Failure::Invalid("source ordinal overflow"))?;
        if ordinals & bit != 0 {
            return Err(Failure::Invalid("duplicate source ordinal"));
        }
        ordinals |= bit;
        let from = native_input(node.input.ok_or(Failure::Unsupported("operation has no owner input"))?, index, &results)?;
        if tone::is_operation(&node.operation) {
            let filters = tone::map(node, ctl)?;
            if filters.is_empty() || steps.len().checked_add(filters.len()).is_none_or(|count| count > MAX_STEPS) {
                return Err(Failure::Unsupported("native graph control limit exceeded"));
            }
            let mut previous = from;
            for filter in filters {
                steps.push(Step::Filter { input: previous, filter });
                previous = steps.len();
            }
            results.push(previous);
            continue;
        }
        if steps.len() == MAX_STEPS {
            return Err(Failure::Unsupported("native graph control limit exceeded"));
        }
        let step = match node.operation.as_str() {
            "invert-linear" | "gegl:invert-linear" => {
                if !node.properties.is_empty() {
                    return Err(Failure::Unsupported("Invert properties have no native adapter"));
                }
                Step::Filter {
                    input: from,
                    filter: SmartFilter {
                        command: "image.adjustments.invert".into(),
                        params: serde_json::json!({}),
                        blend: BlendMode::Normal,
                        opacity: 1.,
                        visible: true,
                    },
                }
            }
            "over" | "gegl:over" | "svg:src-over" => {
                let [aux] = node.properties.as_slice() else { return Err(Failure::Unsupported("source-over requires exactly one explicit auxiliary input")) };
                if aux.name != "aux" {
                    return Err(Failure::Unsupported("source-over property has no native adapter"));
                }
                let graph::Value::Input(above) = aux.value else { return Err(Failure::Invalid("auxiliary input is not a resolved graph edge")) };
                Step::Join { below: from, above: native_input(above, index, &results)?, blend: BlendMode::Normal }
            }
            _ => return Err(Failure::Unsupported("operation has no native adapter")),
        };
        steps.push(step);
        results.push(steps.len());
    }
    let output = native_input(source.output, source.nodes.len(), &results)?;
    check(ctl)?;
    Ok(Plan {
        steps,
        output,
        note: "Native tone controls, document encoding, depth, range, per-control quantization and source-over rules apply; source transfer, pivots or clipping may differ",
    })
}
