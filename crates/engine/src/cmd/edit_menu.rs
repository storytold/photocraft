use super::{CommandSpec, Value, can_redo, can_undo, has_pixel_or_channel, is_mask_target, json, layer_param};

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
            r##"{"contents":"foreground|background|color|contentAware|pattern|history|black|gray|white"="color","color":"#rrggbb|[r,g,b,a]"=foreground (contents=color),"pattern":id|name (contents=pattern),"scale":%=100,"angle":deg,"state":row?,"snapshot":id? (contents=history: a History panel row as history.setSource, redo rows last, or a snapshot; default the History Brush source),"colorAdaptation":bool=true,"scaleAdaptation":bool=false,"mirror":bool=false,"rotationAdaptation":bool=false (contents=contentAware: enable quarter-turn source candidates within the sampling budget),"mode":"normal|multiply|…"="normal","opacity":0..100=100,"preserveTransparency":bool=false,"target":"pixels"|"mask"|"filterMask"|"quickMask"|{"channel":i}?}"##,
            has_pixel_or_channel,
            crate::fill_cmds::fill
        ),
        cmd!(
            "edit.clear",
            "Clear",
            ["Edit"],
            Some("Delete"),
            r##"{"target":"mask"|{"color":i}|{"colors":[i,…]}?=Channels panel target,"layer":id|name?=active} (a targeted mask, channel or Quick Mask is cleared alone)"##,
            has_pixel_or_channel,
            |s, p| {
                let bg = s.tools.background;
                // A targeted alpha channel or Quick Mask is filled instead (#2734).
                let channel = if crate::channel_cmds::is_channel_target(p) {
                    Some(p.clone())
                } else if p.get("target").is_none() {
                    crate::channel_cmds::targeted_channel(s)
                } else {
                    None
                };
                if let Some(t) = channel {
                    s.edit("Clear", |doc, _| crate::channel_cmds::clear_channel(doc, &t, bg))?;
                    return Ok(Value::Null);
                }
                let id = layer_param(s, p)?;
                if is_mask_target(p) {
                    s.edit("Clear Mask", |doc, _| crate::edit_cmds::clear_mask(doc, id, bg, p))?;
                    return Ok(Value::Null);
                }
                // A targeted colour channel clears that channel only (#2720).
                if let Some(colors) = crate::color_channels::target(s, p)? {
                    s.edit("Clear", |doc, _| {
                        for k in colors.indices() {
                            crate::channel_cmds::clear_color_channel(doc, id, k, bg)?;
                        }
                        Ok(())
                    })?;
                    return Ok(Value::Null);
                }
                s.edit("Clear", |doc, _| {
                    let sel = doc.selection.clone();
                    let area = sel.as_ref().map(|m| m.content_bounds()).unwrap_or(doc.bounds());
                    crate::edit_cmds::clear_area(doc, id, area, sel.as_ref(), bg)
                })?;
                Ok(Value::Null)
            }
        ),
    ]
}
