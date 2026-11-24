//! Verified graph shapes become native Invert chains or one static source-over.
//! Other labels/joins remain inert unless every edge is the prior output.
use super::{ArgumentValue, Failure, Result, SmartFilter, StoredEffect, allowed};
use crate::xcf::graph_lowering::{self, Plan, Step};
use craft_xcf::effects::Budget;
use photocraft_raster::Interrupt;

pub(super) fn map(effect: &StoredEffect<'_>, budget: &mut Budget, ctl: &Interrupt<'_>) -> Result<(Vec<SmartFilter>, &'static str, bool)> {
    ctl.check().map_err(|_| Failure::Cancelled)?;
    if effect.operation != "gegl:gegl" || effect.version != Some("0:0") {
        return Err(Failure::Unsupported("graph identity/version has no verified native adapter"));
    }
    if effect.opacity != 1. || effect.mask_offset != 0 || effect.mode != 28 {
        return Err(Failure::Unsupported("graph stage requires full-strength default Normal and no auxiliary mask"));
    }
    allowed(effect, &["string"], ctl)?;
    let [argument] = effect.arguments.as_slice() else { return Err(Failure::Unsupported("graph requires exactly one source string")) };
    let raw = match argument.value {
        ArgumentValue::String(raw) => raw,
        ArgumentValue::NullString => return Err(Failure::Unsupported("stored graph has no source text")),
        _ => return Err(Failure::Invalid("graph string has a non-string source type")),
    };
    let Plan { steps, output, note } = graph_lowering::prepare(raw, budget, ctl).map_err(|error| match error {
        graph_lowering::Failure::Cancelled => Failure::Cancelled,
        graph_lowering::Failure::Invalid(_) | graph_lowering::Failure::Source(craft_xcf::effects::Error::Source(craft_xcf::Error::Invalid(_))) => {
            Failure::Invalid("invalid stored graph syntax or edges")
        }
        graph_lowering::Failure::Unsupported(_) => Failure::Unsupported("graph contains an unsupported operation or property"),
        graph_lowering::Failure::Storage | graph_lowering::Failure::Source(craft_xcf::effects::Error::Limit(_)) => Failure::Limit,
        graph_lowering::Failure::Source(craft_xcf::effects::Error::Source(craft_xcf::Error::Cancelled)) => Failure::Cancelled,
    })?;
    let branch_steps = steps.len().saturating_sub(1);
    let over_original = branch_steps > 0
        && matches!(steps.last(), Some(Step::Join { below: 0, above, blend: photocraft_color::BlendMode::Normal }) if *above == branch_steps)
        && steps.iter().take(branch_steps).enumerate().all(|(index, step)| matches!(step, Step::Filter { input, .. } if *input == index));
    if over_original && !effect.visible {
        return Err(Failure::Unsupported("disabled binary graph remains unapplied"));
    }
    let filter_count = if over_original { branch_steps } else { steps.len() };
    if filter_count == 0 || filter_count > 8 || output != steps.len() {
        return Err(Failure::Unsupported("graph is not a bounded nonempty final-output filter chain"));
    }
    // Inspect the whole topology before returning any native controls. Only the
    // fixed original/Invert join has a separate materializer; other reused values
    // cannot be flattened into a sequential filter chain.
    for (index, step) in steps.iter().enumerate() {
        ctl.check().map_err(|_| Failure::Cancelled)?;
        if !over_original && !matches!(step, Step::Filter { input, .. } if *input == index) {
            return Err(Failure::Unsupported("graph branches and nonsequential references remain unapplied"));
        }
    }
    let mut filters = Vec::new();
    filters.try_reserve_exact(steps.len()).map_err(|_| Failure::Limit)?;
    for step in steps {
        if let Step::Filter { filter, .. } = step {
            filters.push(filter);
        }
    }
    ctl.check().map_err(|_| Failure::Cancelled)?;
    Ok((filters, note, over_original))
}
