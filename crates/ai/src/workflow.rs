//! Explicit API graph bindings; prompts never become executable graph fragments.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AiError, Result, WorkflowId};

const MAX_GRAPH_BYTES: usize = 256 * 1024;
const MAX_NODES: usize = 128;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiNode {
    pub class_type: String,
    pub inputs: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InputKey {
    Prompt,
    NegativePrompt,
    Source,
    EditMask,
    Seed,
    Resolution,
    Width,
    Height,
    OutputPrefix,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputBinding {
    pub key: InputKey,
    pub node: String,
    pub input: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelBinding {
    pub node: String,
    pub input: String,
    pub filename: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowPackage {
    pub schema_version: u32,
    pub id: WorkflowId,
    pub graph: BTreeMap<String, ApiNode>,
    pub required_nodes: BTreeSet<String>,
    pub models: Vec<ModelBinding>,
    pub bindings: Vec<InputBinding>,
    pub outputs: BTreeSet<String>,
}

impl WorkflowPackage {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_GRAPH_BYTES {
            return Err(AiError::InvalidWorkflow("package is too large".into()));
        }
        let package: Self = serde_json::from_slice(bytes).map_err(|_| AiError::InvalidWorkflow("invalid package JSON".into()))?;
        package.validate()?;
        Ok(package)
    }

    pub fn validate(&self) -> Result<()> {
        self.id.validate()?;
        if self.schema_version != 1
            || self.graph.is_empty()
            || self.graph.len() > MAX_NODES
            || self.outputs.is_empty()
            || self.bindings.len() > 64
            || self.models.len() > 32
        {
            return Err(AiError::InvalidWorkflow("unsupported schema or graph limits".into()));
        }
        let classes: BTreeSet<_> = self.graph.values().map(|n| n.class_type.clone()).collect();
        if classes != self.required_nodes {
            return Err(AiError::InvalidWorkflow("required node manifest does not match graph".into()));
        }
        let mut dependencies = BTreeMap::<String, BTreeSet<String>>::new();
        for (id, node) in &self.graph {
            if id.is_empty() || id.len() > 64 || node.inputs.len() > 64 {
                return Err(AiError::InvalidWorkflow("invalid node identity or input count".into()));
            }
            let mut edges = BTreeSet::new();
            for value in node.inputs.values() {
                if let Some(link) = value.as_array() {
                    let source = link.first().and_then(Value::as_str);
                    if let Some(source) = source {
                        if link.len() != 2 || link.get(1).and_then(Value::as_u64).is_none_or(|v| v > 1024) || !self.graph.contains_key(source) {
                            return Err(AiError::InvalidWorkflow("invalid graph link".into()));
                        }
                        edges.insert(source.to_string());
                    }
                }
            }
            dependencies.insert(id.clone(), edges);
        }
        // Bounded, iterative cycle check; no recursive walk over untrusted graphs.
        while !dependencies.is_empty() {
            let ready: Vec<_> = dependencies.iter().filter(|(_, edges)| edges.is_empty()).map(|(id, _)| id.clone()).collect();
            if ready.is_empty() {
                return Err(AiError::InvalidWorkflow("cyclic graph".into()));
            }
            for id in &ready {
                dependencies.remove(id);
            }
            for edges in dependencies.values_mut() {
                for id in &ready {
                    edges.remove(id);
                }
            }
        }
        let mut destinations = BTreeSet::new();
        for binding in &self.bindings {
            if !destinations.insert((&binding.node, &binding.input)) {
                return Err(AiError::InvalidWorkflow("duplicate binding destination".into()));
            }
            let value = self.input(&binding.node, &binding.input)?;
            if value.is_array() || value.is_object() {
                return Err(AiError::InvalidWorkflow("bindings must target scalar inputs".into()));
            }
        }
        for model in &self.models {
            if self.input(&model.node, &model.input)?.as_str() != Some(model.filename.as_str()) || model.filename.is_empty() {
                return Err(AiError::InvalidWorkflow("model binding differs from graph".into()));
            }
            if destinations.contains(&(&model.node, &model.input)) {
                return Err(AiError::InvalidWorkflow("model inputs cannot be dynamically rebound".into()));
            }
        }
        for output in &self.outputs {
            if self.graph.get(output).is_none_or(|n| n.class_type != "SaveImage") {
                return Err(AiError::InvalidWorkflow("output must name a SaveImage node".into()));
            }
        }
        Ok(())
    }

    fn input(&self, node: &str, input: &str) -> Result<&Value> {
        self.graph.get(node).and_then(|n| n.inputs.get(input)).ok_or_else(|| AiError::InvalidWorkflow("binding input does not exist".into()))
    }

    /// Check installed node classes and declared model dropdowns before uploading images.
    pub fn validate_server(&self, object_info: &Value) -> Result<()> {
        self.validate()?;
        for class in &self.required_nodes {
            if !object_info.get(class).is_some_and(Value::is_object) {
                return Err(AiError::InvalidWorkflow(format!("missing node class {class}")));
            }
        }
        for model in &self.models {
            let node = self.graph.get(&model.node).ok_or_else(|| AiError::InvalidWorkflow("model node is missing".into()))?;
            let info = object_info.get(&node.class_type).and_then(|n| n.get("input"));
            let spec = info.and_then(|n| n.get("required").and_then(|r| r.get(&model.input)).or_else(|| n.get("optional").and_then(|r| r.get(&model.input))));
            let choices = spec
                .and_then(Value::as_array)
                .and_then(|a| a.first().and_then(Value::as_array).or_else(|| a.get(1).and_then(|v| v.get("options")).and_then(Value::as_array)));
            if choices.is_none_or(|values| !values.iter().any(|v| v.as_str() == Some(model.filename.as_str()))) {
                return Err(AiError::InvalidWorkflow(format!("missing model for {}", node.class_type)));
            }
        }
        Ok(())
    }

    pub fn bind(&self, values: &BTreeMap<InputKey, Value>) -> Result<BTreeMap<String, ApiNode>> {
        self.validate()?;
        if values.keys().any(|key| !self.bindings.iter().any(|b| b.key == *key)) {
            return Err(AiError::InvalidWorkflow("undeclared binding key".into()));
        }
        let mut graph = self.graph.clone();
        for binding in &self.bindings {
            let value = values.get(&binding.key).ok_or_else(|| AiError::InvalidWorkflow("missing binding value".into()))?;
            let valid = match binding.key {
                InputKey::Seed => value.as_u64().is_some(),
                InputKey::Resolution => value.as_u64().is_some_and(|n| n <= 4096 && n % 32 == 0),
                InputKey::Width | InputKey::Height => value.as_u64().is_some_and(|n| n > 0 && n <= 8192),
                InputKey::Prompt | InputKey::NegativePrompt => value.as_str().is_some_and(|s| s.len() <= crate::MAX_PROMPT_BYTES),
                InputKey::Source | InputKey::EditMask | InputKey::OutputPrefix => value
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s.len() <= 256 && !s.contains(['/', '\\']) && !s.contains("..") && !s.chars().any(char::is_control)),
            };
            if !valid {
                return Err(AiError::InvalidWorkflow("invalid binding value".into()));
            }
            let slot = graph
                .get_mut(&binding.node)
                .and_then(|n| n.inputs.get_mut(&binding.input))
                .ok_or_else(|| AiError::InvalidWorkflow("binding input disappeared".into()))?;
            *slot = value.clone();
        }
        Ok(graph)
    }
}
