//! Read-only inspection of historical conversion metadata. Payloads never become commands.

use std::collections::HashMap;
use std::sync::Arc;

use photocraft_doc::import_conversion::{ConversionDisposition as D, ImportConversion};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "document.importReport";

/// Expose disposition counts and schema information without returning retained payload bytes or executing report
/// targets.
pub fn summary(conversion: Option<&Arc<ImportConversion>>) -> Value {
    let conversion = conversion.map(Arc::as_ref);
    match conversion {
        None => Value::Null,
        Some(ImportConversion::Opaque { schema_version, attachments, .. }) => json!({
            "schemaVersion": schema_version, "sourceFormat": null, "sourceVersion": null,
            "entryCount": null, "dispositionCounts": null, "attachmentCount": attachments.len(), "opaque": true,
        }),
        Some(ImportConversion::Report(report)) => {
            let (mut equivalent, mut approximate, mut unapplied, mut skipped) = (0usize, 0usize, 0usize, 0usize);
            for entry in &report.entries {
                match entry.disposition {
                    D::NativeEquivalent => equivalent += 1,
                    D::NativeApproximation => approximate += 1,
                    D::RetainedUnapplied => unapplied += 1,
                    D::SkippedNonessential => skipped += 1,
                }
            }
            json!({
                "schemaVersion": conversion.map(ImportConversion::schema_version), "sourceFormat": report.source_format,
                "sourceVersion": report.source_version, "policyVersion": report.policy_version,
                "entryCount": report.entries.len(), "attachmentCount": report.attachments.len(),
                "dispositionCounts": {"native_equivalent": equivalent, "native_approximation": approximate,
                    "retained_unapplied": unapplied, "skipped_nonessential": skipped},
            })
        }
    }
}

/// Associate malformed report-query parameters with the report command's normal typed parameter error.
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: message.into() }
}

/// Read a nonnegative paging integer or its default, rejecting other JSON types instead of coercing them.
fn integer(params: &Value, key: &str, default: u64) -> Result<u64> {
    match params.get(key) {
        None => Ok(default),
        Some(value) => value.as_u64().ok_or_else(|| bad("offset and limit must be nonnegative integers")),
    }
}

/// Validate bounded pagination and annotate historical targets against current IDs without granting them editing
/// authority.
fn page(session: &mut Session, params: &Value) -> Result<Value> {
    if !params.is_object() {
        return Err(bad("params must be an object"));
    }
    let offset = integer(params, "offset", 0)?;
    let limit = integer(params, "limit", 100)?;
    if !(1..=250).contains(&limit) {
        return Err(bad("limit must be 1-250"));
    }
    let state = session.active().ok_or(EngineError::NoDocument)?;
    let conversion = state.doc.metadata.import_conversion.as_ref();
    let Some(ImportConversion::Report(report)) = conversion.map(Arc::as_ref) else {
        return Ok(json!({"total": 0, "offset": offset, "nextOffset": null, "entries": [],
            "summary": summary(conversion), "opaque": matches!(conversion.map(Arc::as_ref), Some(ImportConversion::Opaque { .. }))}));
    };
    report.validate().map_err(|e| EngineError::Other(e.to_string()))?;
    let start = usize::try_from(offset).unwrap_or(usize::MAX).min(report.entries.len());
    let end = start.saturating_add(limit as usize).min(report.entries.len());
    // Build one current-ID set; never search the layer tree once for each historical target.
    let ids: HashMap<_, _> = state.doc.walk().into_iter().map(|(_, _, l)| (l.id, l.name.as_str())).collect();
    let entries: Vec<_> = report
        .entries
        .get(start..end)
        .unwrap_or(&[])
        .iter()
        .map(|entry| {
            json!({
                "featureCode": entry.feature_code, "source": entry.source,
                "targets": entry.targets.iter().map(|target| json!({
                    "layerId": target.layer_id, "itemPath": target.item_path, "channelIndex": target.channel_index,
                    "pathIndex": target.path_index, "filterIndex": target.filter_index, "label": target.label,
            "historical": true, "present": target.layer_id.map(|id| ids.contains_key(&id)),
            "currentName": target.layer_id.and_then(|id| ids.get(&id).copied()),
                })).collect::<Vec<_>>(),
                "disposition": entry.disposition, "reason": entry.reason,
                "sourceDescription": entry.source_description, "nativeDescription": entry.native_description,
                "attachments": entry.attachments,
            })
        })
        .collect();
    Ok(json!({"total": report.entries.len(), "offset": offset,
        "nextOffset": if end < report.entries.len() { Some(end) } else { None }, "entries": entries, "summary": summary(conversion)}))
}

/// Register the non-journaled report inspection command with bounded page size and normal active-document gating.
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Import Conversion Report",
        menu: &[],
        shortcut: None,
        params: r#"{"offset":nonnegative integer=0,"limit":integer 1-250=100} -> {total,offset,nextOffset,entries,summary}; historical references only, no retained payload bytes"#,
        enabled: |s| s.active().map(|_| ()).ok_or_else(|| "no document open".into()),
        run: page,
        journal: false,
    }]
}
