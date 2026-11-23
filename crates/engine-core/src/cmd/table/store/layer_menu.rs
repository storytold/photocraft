use crate::cmd::{Color, CommandSpec, Fill, Layer, LayerContent, Value, color_param, has_doc, json, layer_panel_option, mask_new_layer};

/// Layer menu: the gradient fill layer.
pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "layer.newFillLayer.gradient",
        "Gradient…",
        ["Layer", "New Fill Layer"],
        None,
        r##"{"from":"#rrggbb","to":"#rrggbb","angle":deg=90,"style":"linear|radial|angle|reflected|diamond","reverse":bool,"path":"work"|saved path name|{…path}? (the active path: becomes the vector mask; else the selection is the layer mask),"stops"?,"transparency"?,"midpoints"?,"scale"?,"offset"?,"dither"?,"align"? (as gradient.fill.set; stops replace from/to)}"##,
        has_doc,
        |s, p| {
            let a = color_param("layer.newFillLayer.gradient", p, "from", s.tools.foreground)?;
            let b = color_param("layer.newFillLayer.gradient", p, "to", s.tools.background)?;
            let angle = crate::gradient_fill_cmds::angle_param("layer.newFillLayer.gradient", p, 90.0)?;
            let style = crate::layer_style::gradient_style(p.get("style").and_then(Value::as_str).unwrap_or("linear"));
            let reverse = p.get("reverse").and_then(Value::as_bool).unwrap_or(false);
            let default_mask = layer_panel_option(s, "useDefaultMasksOnFillLayers", true);
            // The rest of a recorded Gradient Fill (a loaded action's Make): applied as
            // `gradient.fill.set` would, in the same step.
            let rest: serde_json::Map<String, Value> = ["stops", "transparency", "midpoints", "scale", "offset", "dither", "align"]
                .iter()
                .filter_map(|k| p.get(*k).map(|v| ((*k).to_string(), v.clone())))
                .collect();
            let (fg, bg) = (s.tools.foreground, s.tools.background);
            let id = s.edit("New Gradient Fill Layer", |doc, active| {
                let mut fill =
                    Fill::gradient(vec![(0.0, Color::rgba(a[0], a[1], a[2], a[3])), (1.0, Color::rgba(b[0], b[1], b[2], b[3]))], angle, 1.0, style, reverse);
                if !rest.is_empty() {
                    let l = Layer::new(String::new(), LayerContent::Fill(fill.clone()));
                    fill = crate::gradient_fill_cmds::apply_set(&l, &fill, doc.bounds(), &Value::Object(rest.clone()), fg, bg)?;
                }
                let fill = fill.in_mode(doc.mode);
                let mut l = Layer::new(doc.next_layer_name("Gradient Fill"), LayerContent::Fill(fill));
                mask_new_layer(doc, &mut l, p, "layer.newFillLayer.gradient", default_mask)?;
                let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::Auto);
                *active = Some(id);
                Ok(id)
            })?;
            Ok(json!({ "layer": id.0 }))
        }
    )]
}
