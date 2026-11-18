use super::open::{open, open_smart_filter};
use super::{LayerId, PhotocraftApp, Value, describe, dismiss, take_with};

/// The menu id of the RAW Develop workspace.
const ID: &str = "filter.cameraRaw";

/// Menu / control-channel entry point. `None` when the call isn't for this dialog.
pub fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != ID {
        return None;
    }
    let Some(fields) = params.as_object() else {
        return Some(Err("RAW Develop params must be an object".into()));
    };
    if fields.is_empty() {
        return Some(open(app, ctx).map(|_| describe(app)));
    }
    if let Some(sf) = params.get("smartFilter") {
        let opened = (|| {
            if fields.len() > 1 {
                return Err("smartFilter opens the dialog; send ui requests separately".to_string());
            }
            let layer = sf.get("layer").and_then(Value::as_u64).ok_or("smartFilter.layer must be a layer id")?;
            let index = sf.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or("smartFilter.index must be a filter index")?;
            open_smart_filter(app, ctx, LayerId(layer), index)?;
            Ok(describe(app))
        })();
        return Some(opened);
    }
    let ui = params.get("ui")?;
    app.raw_reply = None;
    let was_closed = app.raw.is_none();
    if was_closed && let Err(e) = open(app, ctx) {
        return Some(Err(e));
    }
    let applied = crate::control::dialogs::workspace_ui(app, ctx, ID, ui);
    // A commit, cancel or done in the request lands now, so its reply is this call's: the confirm or cancel ran the
    // result pump, which left the reply.
    let ended = take_with(app, ctx).or_else(|| app.raw_reply.take());
    if let Err(e) = applied {
        // A request that opened a closed RAW Develop and was refused leaves it closed again.
        if was_closed {
            dismiss(app, ctx);
        }
        return Some(Err(e));
    }
    Some(ended.unwrap_or_else(|| Ok(describe(app))))
}
