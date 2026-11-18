//! Bounded evidence of the stored commands actually authorized while preparing a source.
use std::cell::RefCell;

use super::{Arc, EngineError, JobCtx, Mutex, PoisonError, Result, Value, Weak};

const MAX_BYTES: usize = 16 << 20;
const MAX_COMMANDS: usize = 65_536;
const MAX_DEPTH: usize = 128;

#[derive(Clone)]
struct Command {
    id: Arc<str>,
    params: Arc<Value>,
}

#[derive(Clone, Default)]
pub(crate) struct Proof {
    commands: Arc<[Command]>,
    bytes: usize,
}

impl Proof {
    /// Recheck exact original and materialized parameters without decoding or retaining a cache lock.
    pub(crate) fn authorize(&self, ctx: &JobCtx) -> Result<()> {
        for command in self.commands.iter() {
            ctx.check()?;
            super::authorize_render_command(&command.id, &command.params)?;
        }
        ctx.check()
    }

    /// Conservative owned parameter and command-metadata charge, in addition to cached tile bytes.
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
}

#[derive(Default)]
struct Recorded {
    commands: Vec<Command>,
    bytes: usize,
    incomplete: bool,
}

thread_local! {
    static RECORDERS: RefCell<Vec<Weak<Mutex<Recorded>>>> = const { RefCell::new(Vec::new()) };
}

pub(crate) struct Recording(Arc<Mutex<Recorded>>);

impl Recording {
    /// A failed or over-budget recording never certifies a reusable source image.
    pub(crate) fn finish(&self) -> Option<Proof> {
        let recorded = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if recorded.incomplete {
            return None;
        }
        Some(Proof { commands: recorded.commands.clone().into(), bytes: recorded.bytes })
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        RECORDERS.with(|recorders| {
            if let Ok(mut recorders) = recorders.try_borrow_mut() {
                let current = Arc::downgrade(&self.0);
                recorders.retain(|recorder| !recorder.ptr_eq(&current) && recorder.strong_count() != 0);
            }
        });
    }
}

/// Nested recordings retain events in every enclosing source's proof as well as their own.
pub(crate) fn start() -> Result<Recording> {
    RECORDERS.with(|recorders| {
        let mut recorders = recorders.try_borrow_mut().map_err(|_| EngineError::Other("render authorization recording is busy".into()))?;
        recorders.retain(|recorder| recorder.strong_count() != 0);
        if recorders.len() >= MAX_DEPTH {
            return Err(EngineError::Other("render authorization recording exceeds its depth limit".into()));
        }
        recorders.try_reserve(1).map_err(|_| EngineError::Other("not enough memory for render authorization recording".into()))?;
        let recording = Recording(Arc::new(Mutex::new(Recorded::default())));
        recorders.push(Arc::downgrade(&recording.0));
        Ok(recording)
    })
}

/// Bound recursive Value cloning and retained memory before copying any input-derived parameters.
fn command_bytes(id: &str, params: &Value) -> Option<usize> {
    let mut bytes = id.len().checked_add(128)?;
    let mut pending = vec![(params, 0usize)];
    while let Some((value, depth)) = pending.pop() {
        bytes = bytes.checked_add(128)?;
        if depth > MAX_DEPTH || bytes > MAX_BYTES {
            return None;
        }
        let children = match value {
            Value::String(value) => {
                bytes = bytes.checked_add(value.len())?;
                0
            }
            Value::Array(values) => values.len(),
            Value::Object(values) => values.len(),
            _ => 0,
        };
        if pending.len().checked_add(children)? > MAX_BYTES / 128 {
            return None;
        }
        pending.try_reserve(children).ok()?;
        match value {
            Value::Array(values) => pending.extend(values.iter().map(|value| (value, depth + 1))),
            Value::Object(values) => {
                for (key, value) in values {
                    bytes = bytes.checked_add(key.len())?;
                    if bytes > MAX_BYTES {
                        return None;
                    }
                    pending.push((value, depth + 1));
                }
            }
            _ => {}
        }
    }
    (bytes <= MAX_BYTES).then_some(bytes)
}

/// Called only after the current gate accepted the command. No recording means no parameter clone.
pub(super) fn record(id: &str, params: &Value) -> Result<()> {
    RECORDERS.with(|recorders| {
        let recorders = recorders.try_borrow().map_err(|_| EngineError::Other("render authorization recording is busy".into()))?;
        if recorders.is_empty() {
            return Ok(());
        }
        // Permission to read a live file does not certify its contents or document-relative
        // resolution. Such sources remain cold, rather than reusing another render's pixels.
        let live_file = id == "layer.smartObjects.updateModifiedContent" && params.get("path").is_some()
            || id == "filter.distort.displace" && crate::filters_ext::map_path(params).is_some();
        let bytes = if live_file { None } else { command_bytes(id, params) };
        let command = bytes.map(|_| Command { id: Arc::from(id), params: Arc::new(params.clone()) });
        for recorder in recorders.iter().filter_map(Weak::upgrade) {
            let mut recorded = recorder.lock().unwrap_or_else(PoisonError::into_inner);
            if recorded.incomplete {
                continue;
            }
            if let Some(total) = bytes.and_then(|bytes| recorded.bytes.checked_add(bytes)).filter(|&bytes| bytes <= MAX_BYTES)
                && recorded.commands.len() < MAX_COMMANDS
                && recorded.commands.try_reserve(1).is_ok()
                && let Some(command) = &command
            {
                recorded.commands.push(command.clone());
                recorded.bytes = total;
            } else {
                recorded.incomplete = true;
                recorded.commands.clear();
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nested_recordings_and_replayed_hits_retain_exact_parameters() {
        let outer = start().unwrap();
        let inner = start().unwrap();
        let mut params = json!({"radius": 1.25, "nested": [true, {"path": "original.map"}]});
        super::super::authorize_render_command("filter.blur.gaussianBlur", &params).unwrap();
        let child = inner.finish().unwrap();
        drop(inner);
        params["radius"] = json!(9);
        child.authorize(&JobCtx::new()).unwrap();
        let parent = outer.finish().unwrap();
        assert_eq!(parent.commands.len(), 2);
        for command in parent.commands.iter() {
            assert_eq!(&*command.id, "filter.blur.gaussianBlur");
            assert_eq!(command.params["radius"], json!(1.25));
            assert_eq!(command.params["nested"], json!([true, {"path": "original.map"}]));
        }
        let cancelled = JobCtx::new();
        cancelled.cancel();
        assert!(matches!(parent.authorize(&cancelled), Err(EngineError::Cancelled)));
    }

    #[test]
    fn oversized_or_deep_parameters_decline_reuse_without_copying_them() {
        let recording = start().unwrap();
        super::super::authorize_render_command("filter.unknown", &json!({"data": "x".repeat(MAX_BYTES)})).unwrap();
        assert!(recording.finish().is_none());
        let mut deep = Value::Null;
        for _ in 0..MAX_DEPTH + 2 {
            deep = Value::Array(vec![deep]);
        }
        assert!(command_bytes("filter.unknown", &deep).is_none());
        let remaining = start().unwrap();
        super::super::authorize_render_command("filter.unknown", &json!({})).unwrap();
        assert_eq!(remaining.finish().unwrap().commands.len(), 1, "an incomplete outer proof cannot poison an inner recording");
    }

    #[test]
    fn live_file_reads_cannot_certify_reusable_pixels() {
        for (id, params) in
            [("layer.smartObjects.updateModifiedContent", json!({"path":"external.pcraft"})), ("filter.distort.displace", json!({"mapPath":"external.psd"}))]
        {
            let recording = start().unwrap();
            super::super::authorize_render_command(id, &params).unwrap();
            assert!(recording.finish().is_none());
        }
    }
}
