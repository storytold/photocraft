use super::{CommandSpec, Value, can_redo, can_undo, has_pixel_or_channel, json, layer_param};

/// Edit menu commands: Undo, Redo, Fill and Clear.
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("edit.undo", "Undo", ["Edit"], Some("Cmd+Z"), "{}", can_undo, |s, _| Ok(json!(s.undo()))),
        cmd!("edit.redo", "Redo", ["Edit"], Some("Cmd+Shift+Z"), "{}", can_redo, |s, _| Ok(json!(s.redo()))),
        cmd!(
            "edit.fill",
            "Fill…",
            ["Edit"],
            Some("Shift+F5"),
            r##"{"contents":"foreground|background|color|contentAware|pattern|history|black|gray|white"="color","color":"#rrggbb|[r,g,b,a]"=foreground (contents=color),"pattern":id|name (contents=pattern),"scale":%=100,"angle":deg,"state":row?,"snapshot":id? (contents=history: a History panel row as history.setSource, redo rows last, or a snapshot; default the History Brush source),"colorAdaptation":bool=true (contents=contentAware),"mode":"normal|multiply|…"="normal","opacity":0..100=100,"preserveTransparency":bool=false,"target":"pixels"|"mask"|"filterMask"|"quickMask"|{"channel":i}?}"##,
            has_pixel_or_channel,
            crate::fill_cmds::fill
        ),
        cmd!("edit.clear", "Clear", ["Edit"], Some("Delete"), "{} (a targeted channel or Quick Mask is cleared alone)", has_pixel_or_channel, |s, p| {
            let bg = s.tools.background;
            // A targeted alpha channel or Quick Mask is filled instead (#2734).
            if let Some(t) = crate::channel_cmds::targeted_channel(s) {
                s.edit("Clear", |doc, _| crate::channel_cmds::clear_channel(doc, &t, bg))?;
                return Ok(Value::Null);
            }
            let id = layer_param(s, p)?;
            // A targeted colour channel clears that channel only (#2720).
            if let Some(k) = crate::channel_cmds::targeted_color(s) {
                s.edit("Clear", |doc, _| crate::channel_cmds::clear_color_channel(doc, id, k, bg))?;
                return Ok(Value::Null);
            }
            s.edit("Clear", |doc, _| {
                let sel = doc.selection.clone();
                let area = sel.as_ref().map(|m| m.content_bounds()).unwrap_or(doc.bounds());
                crate::edit_cmds::clear_area(doc, id, area, sel.as_ref(), bg)
            })?;
            Ok(Value::Null)
        }),
    ]
}
