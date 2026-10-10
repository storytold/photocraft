//! Combined PDF export. Retain immutable snapshots until the atomic write succeeds.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::Document;
use photocraft_io::ExportOptions;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct Snapshot {
    docs: Vec<(Arc<Document>, u64)>,
}

impl Snapshot {
    pub fn capture(session: &Session) -> Result<Self> {
        if session.documents().is_empty() {
            return Err(EngineError::NoDocument);
        }
        Ok(Self { docs: session.documents().iter().map(|st| (st.doc.clone(), st.revision)).collect() })
    }

    /// The writer must report success only after the complete file has been committed.
    /// Closing is opt-in. New tabs and tabs edited since capture always remain open.
    pub fn write(self, session: &mut Session, path: &str, close: bool, write: impl FnOnce(&str, &[u8]) -> std::result::Result<(), String>) -> Result<Value> {
        if !std::path::Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            return Err(EngineError::Other("Choose a filename ending in .pdf".into()));
        }
        let docs: Vec<&Document> = self.docs.iter().map(|(d, _)| d.as_ref()).collect();
        let out = photocraft_io::pdf::export_pdf_documents(&docs, &ExportOptions::default()).map_err(|e| EngineError::Other(e.to_string()))?;
        write(path, &out.bytes).map_err(EngineError::Other)?;
        let mut closed = 0;
        if close {
            for i in (0..session.documents().len()).rev() {
                let st = &session.documents()[i];
                if self.docs.iter().any(|(doc, revision)| doc.id == st.doc.id && *revision == st.revision && Arc::ptr_eq(doc, &st.doc)) {
                    session.close(i);
                    closed += 1;
                }
            }
        }
        Ok(json!({"path":path, "tabs":self.docs.len(), "bytes":out.bytes.len(), "closed":closed, "warnings":out.warnings}))
    }
}

fn run(session: &mut Session, params: &Value) -> Result<Value> {
    let bad = || EngineError::BadParams { cmd: "file.export.allTabsPdf".into(), msg: "expected {path: string, closeAfter?: boolean}".into() };
    let path = params.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()).ok_or_else(bad)?;
    let close = match params.get("closeAfter") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(bad)?,
    };
    Snapshot::capture(session)?.write(session, path, close, |path, bytes| crate::file_cmds::write_file(path, bytes).map_err(|e| e.to_string()))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "file.export.allTabsPdf",
        label: "Export All Tabs to PDF…",
        menu: &["File", "Export"],
        shortcut: None,
        params: r#"{"path":str,"closeAfter":bool=false}"#,
        journal: true,
        enabled: crate::file_cmds::native_doc,
        run,
    }]
}
