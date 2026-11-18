//! A new foreground colour recolours the selected objects: the Color panel, the Swatches panel and
//! the Color Picker set the foreground, and depending on the tool in hand it also goes to what is
//! selected:
//!
//! * characters selected while editing type: those characters (`type_tool::foreground_changed`);
//! * a Type tool, not editing: the colour of every selected type layer (`type.setStyle`);
//! * a shape tool, a Pen or a path selection tool: the fill of every selected shape layer
//!   (`shape.edit` with `fill`);
//! * any other tool (the Move tool, the brushes, the pixel selection tools…): nothing, only the
//!   foreground.
//!
//! A click on a swatch is broader, as in Photoshop: whatever the tool, it recolours every selected
//! type layer and the fill of every selected shape layer (#9238; Photoshop 2025 measured with the
//! Move and Brush tools), while the Color panel keeps to the rules above.
//!
//! One pick is one history step for all the layers it recolours; a drag in the Color panel's field
//! or sliders is one step too.

use photocraft_doc::{Color, Fill, LayerContent};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn hex(c: [f32; 4]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

/// Does a new foreground recolour selected shape layers with `tool`?
fn recolours_shapes(tool: Tool) -> bool {
    crate::vector_ui::is_shape_tool(tool)
        || matches!(
            tool,
            Tool::PathSelection
                | Tool::DirectSelection
                | Tool::Pen
                | Tool::FreeformPen
                | Tool::CurvaturePen
                | Tool::AddAnchorPoint
                | Tool::DeleteAnchorPoint
                | Tool::ConvertPoint
        )
}

/// The foreground colour just changed through the Color panel, the Swatches panel or the Color
/// Picker: recolour what is selected, as the tool in hand says (see the module docs).
/// Picks of one `gesture` (the coalesce key of a Color panel edit gesture, a drag or a typed
/// value) share one history step; `None` (a swatch, the Color Picker) is a step of its own.
pub fn foreground_changed(app: &mut PhotocraftApp, gesture: Option<String>) {
    recolour(app, gesture, false);
}

/// A swatch just set the foreground: recolour the selected characters while editing type, else
/// every selected type layer and shape fill, whatever the tool (see the module docs).
pub fn swatch_picked(app: &mut PhotocraftApp) {
    recolour(app, None, true);
}

fn recolour(app: &mut PhotocraftApp, gesture: Option<String>, any_tool: bool) {
    // Editing type: the selected characters, in the editing session's step; nothing else.
    if app.ui.text_edit.is_some() {
        crate::type_tool::foreground_changed(app);
        return;
    }
    let tool = app.ui.tool;
    // What the tool in hand recolours: type layers or shape layers (a swatch: both).
    let (do_texts, do_shapes) = match tool {
        _ if any_tool => (true, true),
        t if t.is_type() && !t.is_type_mask() => (true, false),
        t if recolours_shapes(t) => (false, true),
        _ => return,
    };
    let Some(st) = app.session.active() else { return };
    let fg = app.session.tools.foreground;
    let (texts, shapes): (Vec<u64>, Vec<(u64, f32)>) = {
        let selected: Vec<_> = st.selected_layers().into_iter().filter_map(|id| st.doc.layer(id)).collect();
        let texts = selected.iter().filter(|l| matches!(l.content, LayerContent::Text(_))).map(|l| l.id.0).collect();
        // A shape keeps its fill's own opacity and takes the float colour as it is (#5568);
        // locked shapes and ones already filled with it are left alone.
        let shapes = selected
            .iter()
            .filter_map(|l| {
                let LayerContent::Shape(sh) = &l.content else { return None };
                let locks = st.doc.effective_locks(l.id);
                if locks.all || locks.pixels {
                    return None;
                }
                let alpha = match &sh.fill {
                    Some(Fill::Solid(c)) => c.alpha,
                    _ => 1.0,
                };
                (sh.fill != Some(Fill::Solid(Color::rgba(fg[0], fg[1], fg[2], alpha)))).then_some((l.id.0, alpha))
            })
            .collect();
        (texts, shapes)
    };
    let texts = if do_texts { texts } else { Vec::new() };
    let shapes = if do_shapes { shapes } else { Vec::new() };
    if texts.is_empty() && shapes.is_empty() {
        return;
    }
    let key = match gesture {
        Some(g) => format!("apply-color-{g}"),
        None => format!("apply-color-{}", app.ui.alloc_id()),
    };
    let result = (|| {
        if !texts.is_empty() {
            app.run("type.setStyle", json!({"layers": texts, "color": hex(fg), "coalesce": key}))?;
        }
        shapes.iter().try_for_each(|(id, alpha)| app.run("shape.edit", json!({"layer": id, "fill": [fg[0], fg[1], fg[2], alpha], "coalesce": key})).map(|_| ()))
    })();
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests;
