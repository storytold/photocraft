//! Explicit, per-source historical XCF sample interpretation shared by command adapters.
use super::{EngineError, Result, Value};

/// Missing and null values leave the order unanswered; no spelling aliases are inferred.
pub fn historical_xcf_sample_order(p: &Value, cmd: &str) -> Result<Option<photocraft_io::HistoricalXcfSampleOrder>> {
    match p.get("historicalXcfSampleOrder") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value == "little-endian" => Ok(Some(photocraft_io::HistoricalXcfSampleOrder::LittleEndian)),
        Some(Value::String(value)) if value == "big-endian" => Ok(Some(photocraft_io::HistoricalXcfSampleOrder::BigEndian)),
        _ => Err(EngineError::BadParams { cmd: cmd.into(), msg: "historicalXcfSampleOrder must be little-endian or big-endian".into() }),
    }
}

/// Resolve fresh-source controls over caller policy, without reusing another source's approval
/// or interpretation. The compatibility byte-level APIs retain their separate default policy.
pub fn import_options_from_params(base: photocraft_io::ImportOptions, p: &Value, cmd: &str) -> Result<photocraft_io::ImportOptions> {
    let approved = match p.get("approveHighMemory") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "expected boolean for \"approveHighMemory\"".into() }),
    };
    let order = historical_xcf_sample_order(p, cmd)?;
    let historical_xcf_order = match order {
        Some(order) => photocraft_io::HistoricalXcfOrder::Explicit(order),
        None => photocraft_io::HistoricalXcfOrder::RequireExplicit,
    };
    Ok(photocraft_io::ImportOptions { psd_high_memory_approved: approved, psd_approved_estimate: None, historical_xcf_order, ..base })
}
