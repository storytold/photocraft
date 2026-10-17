//! Import a complete custom-shape set before changing the persistent library.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use super::{CommandSpec, Group, Session, ShapePreset, always, bad, fit};
use crate::Result;

const CMD: &str = "shape.presets.importCsh";

pub(super) fn spec() -> CommandSpec {
    CommandSpec {
        id: CMD,
        label: "Import Shapes",
        menu: &[],
        shortcut: None,
        params: r#"{"path":str (.csh) | "data":str (base64),"group":str?} → {group,count,names}; appends a uniquely named group, never replaces existing presets"#,
        enabled: always,
        run: import,
        journal: true,
    }
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let group = p
        .get("group")
        .map(|v| v.as_str().map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned).ok_or_else(|| bad(CMD, "`group` must be a non-empty string")))
        .transpose()?;
    let params = p.clone();
    crate::jobs::run(
        s,
        "Import Shapes",
        false,
        move |ctx| {
            ctx.check()?;
            let (bytes, stem) = crate::preset_import_cmds::file_bytes_limited(&params, CMD, ".csh file", photocraft_psd::csh::MAX_FILE_BYTES as u64)?;
            let shapes = photocraft_psd::csh::parse(&bytes).map_err(|e| bad(CMD, e.to_string()))?;
            if shapes.is_empty() {
                return Err(bad(CMD, "the CSH set contains no shapes"));
            }
            let mut items = Vec::with_capacity(shapes.len());
            for shape in shapes {
                ctx.check()?;
                let [top, left, bottom, right] = shape.bounds;
                // The parser validates ordering; widen before subtraction for signed bounds.
                let width = u32::try_from(i64::from(right) - i64::from(left)).map_err(|_| bad(CMD, "shape width exceeds supported bounds"))?;
                let height = u32::try_from(i64::from(bottom) - i64::from(top)).map_err(|_| bad(CMD, "shape height exceeds supported bounds"))?;
                let path = photocraft_io::vector_map::path_from_records(&shape.path, width, height);
                if path.is_empty() {
                    return Err(bad(CMD, format!("shape {:?} has no path", shape.name)));
                }
                let path = fit(&path, [0.0, 0.0, 100.0, 100.0], true);
                let name = if shape.name.trim().is_empty() { "Shape".to_owned() } else { shape.name };
                items.push(ShapePreset { name, path });
            }
            let group = group.unwrap_or_else(|| if stem.is_empty() { "Imported Shapes".into() } else { stem });
            Ok((group, items))
        },
        |s, (group, mut items)| {
            let group = super::super::gradients::unique_group(&s.presets.shapes, &group);
            let mut taken: HashSet<String> = s.presets.shapes.iter().flat_map(|g| g.items.iter().map(|i| i.name.clone())).collect();
            taken.extend(s.edit_state.custom_shapes.iter().map(|c| c.name.clone()));
            let mut suffixes: HashMap<String, usize> = HashMap::new();
            for item in &mut items {
                let base = item.name.clone();
                let suffix = suffixes.entry(base.clone()).or_insert(2);
                while !taken.insert(item.name.clone()) {
                    item.name = format!("{base} {suffix}");
                    *suffix += 1;
                }
            }
            let names: Vec<String> = items.iter().map(|i| i.name.clone()).collect();
            let result = json!({"group": group, "count": items.len(), "names": names});
            s.presets.shapes.push(Group::new(&group, items));
            s.presets_changed();
            Ok(result)
        },
    )
}
