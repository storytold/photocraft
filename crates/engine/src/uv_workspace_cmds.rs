//! Independent document banks for the 3D project and ordinary PhotoCraft workspaces.
use crate::{DocState, EngineError, Result, Session, commands::CommandSpec};
use serde_json::{Value, json};
#[derive(Default)]
pub struct Documents {
    pub active: bool,
    pub(crate) initialized: bool,
    pub(crate) parked: Vec<DocState>,
    active_index: Option<usize>,
}
impl Documents {
    /// Hidden documents still need a save prompt on application exit.
    pub fn parked(&self) -> &[DocState] {
        &self.parked
    }
}
impl Session {
    pub fn uv_documents(&self) -> &[DocState] {
        if self.uv_workspace.initialized && !self.uv_workspace.active { &self.uv_workspace.parked } else { &self.docs }
    }

    pub(crate) fn uv_documents_mut(&mut self) -> &mut [DocState] {
        if self.uv_workspace.initialized && !self.uv_workspace.active { &mut self.uv_workspace.parked } else { &mut self.docs }
    }
}
fn switch(s: &mut Session, p: &Value) -> Result<Value> {
    let active = p.get("active").and_then(Value::as_bool).ok_or_else(|| EngineError::Other("active must be boolean".into()))?;
    if s.uv_workspace.active == active {
        return Ok(json!({"active":active}));
    }
    // Also admit sessions populated by headless imports before the UI was attached.
    if active && !s.uv_workspace.initialized && s.uv.scene.is_some() {
        let ids: std::collections::HashSet<_> = s.uv.all_bindings().flatten().copied().chain(s.uv.saved_revisions.keys().copied()).collect();
        let selected = s.active().map(|st| st.doc.id);
        let mut uv = Vec::new();
        for st in std::mem::take(&mut s.docs) {
            if ids.contains(&st.doc.id) {
                uv.push(st);
            } else {
                s.uv_workspace.parked.push(st);
            }
        }
        s.uv_workspace.active_index =
            selected.and_then(|id| s.uv_workspace.parked.iter().position(|st| st.doc.id == id)).or_else(|| (!s.uv_workspace.parked.is_empty()).then_some(0));
        s.docs = uv;
        s.active = selected.and_then(|id| s.docs.iter().position(|st| st.doc.id == id)).or_else(|| (!s.docs.is_empty()).then_some(0));
    } else {
        std::mem::swap(&mut s.docs, &mut s.uv_workspace.parked);
        std::mem::swap(&mut s.active, &mut s.uv_workspace.active_index);
    }
    s.uv_workspace.active = active;
    s.uv_workspace.initialized = true;
    s.note_active_change();
    s.sync_tools_mask_target();
    Ok(json!({"active":active,"documents":s.docs.len()}))
}
fn close(s: &mut Session, _: &Value) -> Result<Value> {
    switch(s, &json!({"active":true}))?;
    while !s.docs.is_empty() {
        s.close(s.docs.len() - 1);
    }
    s.uv = Default::default();
    Ok(Value::Null)
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "uv.project.close",
            label: "Close 3D container",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: |_| Ok(()),
            run: close,
            journal: false,
        },
        CommandSpec {
            id: "uv.workspace.activate",
            label: "3D document workspace",
            menu: &[],
            shortcut: None,
            params: r#"{"active":boolean}"#,
            enabled: |_| Ok(()),
            run: switch,
            journal: false,
        },
    ]
}
