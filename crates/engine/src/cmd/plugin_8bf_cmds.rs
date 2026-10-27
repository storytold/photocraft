//! Desktop-only 8BF filter commands. Native libraries execute in an external Windows worker.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LayerContent};
use photocraft_plugins::native_8bf;
use serde_json::{Value, json};
pub const RUN: &str = "plugin.8bf.run";
fn bad(cmd: &str, message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: message.into() }
}
fn error(e: std::io::Error) -> EngineError {
    if e.kind() == std::io::ErrorKind::Interrupted { EngineError::Cancelled } else { EngineError::Other(e.to_string()) }
}
fn document_format(doc: &Document) -> std::result::Result<(u8, u8), String> {
    let channels = match doc.mode {
        ColorMode::Rgb => 3,
        ColorMode::Grayscale => 1,
        _ => return Err("8BF filters require an RGB or Grayscale document".into()),
    };
    let depth = match doc.depth {
        SampleType::U8 => 8,
        SampleType::U16 => 16,
        SampleType::F32 => return Err("8BF filters require an 8-bit or 16-bit document".into()),
    };
    Ok((channels, depth))
}
fn matching_layer(doc: &Document, fmt: photocraft_color::PixelFormat) -> std::result::Result<(), String> {
    if fmt.mode != doc.mode || fmt.sample != doc.depth {
        return Err("8BF pixel layer mode and depth must match the document".into());
    }
    Ok(())
}
fn has_pixel_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    document_format(&d.doc)?;
    if d.channel_view.target != crate::channel_cmds::ChannelTarget::Composite || d.doc.quick_mask.is_some() {
        return Err("8BF filters currently support layer pixels only; select the composite channel".into());
    }
    let layer = d.active_layer.and_then(|id| d.doc.layer(id)).ok_or("no active pixel layer")?;
    let LayerContent::Raster(surface) = &layer.content else { return Err("8BF filters need a raster pixel layer".into()) };
    matching_layer(&d.doc, surface.format())?;
    Ok(())
}
fn install(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p.get("path").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| bad("plugin.8bf.install", "give a .8bf file path"))?;
    let plugin = native_8bf::install(std::path::Path::new(path)).map_err(error)?;
    s.plugin_folders.disown_native(&plugin.descriptor.id);
    serde_json::to_value(&plugin.descriptor).map_err(|e| EngineError::Other(e.to_string()))
}
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("target").is_some_and(|v| v.as_str() != Some("pixels")) || p.get("channel").is_some() {
        return Err(bad(RUN, "8BF bridge currently supports layer pixels only, not masks or individual channels"));
    }
    let id = p.get("id").and_then(Value::as_str).ok_or_else(|| bad(RUN, "missing installed plug-in id"))?;
    let plugin = native_8bf::get(id).ok_or_else(|| bad(RUN, "8BF plug-in is not installed"))?;
    let show_ui = match p.get("show_ui") {
        None => true,
        Some(v) => v.as_bool().ok_or_else(|| bad(RUN, "show_ui must be a boolean"))?,
    };
    let layer = crate::commands::layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let (channels, depth) = document_format(&st.doc).map_err(|message| bad(RUN, message))?;
    let l = st.doc.layer(layer).ok_or(EngineError::NoLayer(layer))?;
    let LayerContent::Raster(surface) = &l.content else { return Err(bad(RUN, "8BF filters need a raster pixel layer")) };
    matching_layer(&st.doc, surface.format()).map_err(|message| bad(RUN, message))?;
    if st.doc.effective_locks(layer).pixels {
        return Err(EngineError::Other("layer pixels are locked".into()));
    }
    let canvas = st.doc.bounds();
    if canvas.is_empty() || canvas.width() > 32767 || canvas.height() > 32767 {
        return Err(bad(RUN, "8BF filter dimensions must be between 1 and 32767 pixels per side"));
    }
    let width = canvas.width();
    let height = canvas.height();
    let request = native_8bf::wire::Request {
        path: native_8bf::client::windows_path(&plugin.descriptor.path).map_err(error)?,
        entry: plugin.descriptor.entry.clone(),
        width,
        height,
        channels,
        depth,
        show_ui,
    };
    let name = plugin.descriptor.name.clone();
    let label = format!("{name} (8BF)");
    let result_id = plugin.descriptor.id.clone();
    crate::jobs::edit_job(
        s,
        &label,
        move |doc, _, ctx| {
            ctx.check()?;
            ctx.progress(0.05, "Running 8BF filter");
            let selection = doc.selection.clone();
            let l = doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?;
            let LayerContent::Raster(surface) = &mut l.content else { return Err(bad(RUN, "8BF filters need a raster pixel layer")) };
            *surface = native_8bf::surface::apply(&plugin.launch, request, surface, canvas, selection.as_ref(), || ctx.cancelled()).map_err(error)?;
            ctx.check()?;
            Ok(())
        },
        move |()| json!({"plugin":result_id,"layer":layer.0}),
    )
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "plugin.8bf.install",
            label: "Install 8BF Plug-in…",
            menu: &["Filter", "Plug-ins"],
            shortcut: None,
            params: r#"{"path":text}"#,
            enabled: crate::commands::always,
            run: install,
            journal: false,
        },
        CommandSpec {
            id: "plugin.8bf.list",
            label: "List 8BF Plug-ins",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: crate::commands::always,
            run: |_, _| Ok(json!({"plugins":native_8bf::list().iter().map(|p|&p.descriptor).collect::<Vec<_>>()})),
            journal: false,
        },
        CommandSpec {
            id: "plugin.8bf.remove",
            label: "Remove 8BF Plug-in",
            menu: &[],
            shortcut: None,
            params: r#"{"id":text}"#,
            enabled: crate::commands::always,
            run: |_, p| {
                let id = p.get("id").and_then(Value::as_str).ok_or_else(|| bad("plugin.8bf.remove", "missing plug-in id"))?;
                if native_8bf::remove(id) { Ok(json!({"removed":id})) } else { Err(bad("plugin.8bf.remove", "8BF plug-in is not installed")) }
            },
            journal: false,
        },
        CommandSpec {
            id: RUN,
            label: "Run 8BF Plug-in",
            menu: &[],
            shortcut: None,
            params: r#"{"id":text,"show_ui":bool=true,"layer":id,"target":text,"channel":text}"#,
            enabled: has_pixel_layer,
            run,
            journal: true,
        },
    ]
}
