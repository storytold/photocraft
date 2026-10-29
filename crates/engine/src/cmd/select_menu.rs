use super::{CommandSpec, Rect, Surface, Value, bad, has_doc, has_selection, int_i32, selection_cmds};

/// Select menu commands: All, Deselect, Inverse and rectangular selections.
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("select.all", "All", ["Select"], Some("Cmd+A"), "{}", has_doc, |s, _| {
            s.edit("Select All", |doc, _| {
                let mut m = Surface::new(photocraft_color::PixelFormat::GRAY8);
                m.fill_rect(doc.bounds(), &[1.0]);
                doc.selection = Some(m);
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!("select.deselect", "Deselect", ["Select"], Some("Cmd+D"), "{}", has_selection, |s, _| {
            // Nothing selected: nothing to do and no history step, as in Photoshop (#6744).
            if s.active().is_none_or(|d| d.doc.selection.is_none()) {
                return Ok(Value::Null);
            }
            s.edit("Deselect", |doc, _| {
                doc.selection = None;
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!("select.inverse", "Inverse", ["Select"], Some("Cmd+Shift+I"), "{}", has_selection, |s, _| {
            s.edit("Inverse", |doc, _| {
                let old = doc.selection.take().unwrap_or_else(|| Surface::new(photocraft_color::PixelFormat::GRAY8));
                let b = doc.bounds();
                let data: Vec<f32> = old.read_region(b).into_iter().map(|v| 1.0 - v).collect();
                let mut m = Surface::new(photocraft_color::PixelFormat::GRAY8);
                m.write_region(b, &data);
                m.prune();
                doc.selection = Some(m);
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!(
            "select.rect",
            "Rectangular Selection",
            [],
            None,
            r##"{"x":i32,"y":i32,"width":u32,"height":u32,"mode":"replace|add|subtract|intersect"="replace","ellipse":bool=false,"line":"row|column"?,"antiAlias":bool=true,"feather":px=0}"##,
            has_doc,
            |s, p| {
                let get = |k: &str| int_i32("select.rect", p, k).and_then(|v| v.ok_or_else(|| bad("select.rect", format!("missing `{k}`"))));
                let r = Rect::from_xywh(get("x")?, get("y")?, get("width")?.max(0) as u32, get("height")?.max(0) as u32);
                let mode = p.get("mode").and_then(Value::as_str).unwrap_or("replace").to_string();
                let ellipse = p.get("ellipse").and_then(Value::as_bool).unwrap_or(false);
                // The History step is named after the tool: the Single Row / Single Column
                // Marquee send `line` (Photoshop).
                let step = match p.get("line").and_then(Value::as_str) {
                    Some("row") => "Single Row Marquee",
                    Some("column") => "Single Column Marquee",
                    Some(other) => return Err(bad("select.rect", format!("unknown `line` {other:?} (row|column)"))),
                    None if ellipse => "Elliptical Marquee",
                    None => "Rectangular Marquee",
                };
                // Options bar: anti-aliased ellipse edges (4x4 supersampled) and Feather (applied to the new shape only).
                let aa = p.get("antiAlias").and_then(Value::as_bool).unwrap_or(true);
                let feather = p.get("feather").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 1000.0) as f32;
                s.edit(step, |doc, _| {
                    let area = doc.bounds();
                    // A marquee dragged past the canvas stops at its edge (Photoshop); the ellipse
                    // keeps the dragged shape and is only cut there.
                    use photocraft_algo::selection as sel;
                    let shape = if ellipse {
                        sel::ellipse_surface(r, area, aa)
                    } else {
                        let mut s = Surface::new(photocraft_color::PixelFormat::GRAY8);
                        s.fill_rect(r.intersect(&area), &[1.0]);
                        s
                    };
                    if feather > 0.0 {
                        let window = selection_cmds::feather_window(r, area, feather);
                        let m = sel::feather(&sel::mask_from_surface(Some(&shape), window), window.width() as usize, window.height() as usize, feather);
                        doc.selection = selection_cmds::combine_band(doc.selection.as_ref(), &m, window, sel::SelectionMode::parse(&mode));
                        return Ok(());
                    }
                    let old = doc.selection.take();
                    let combined = match (mode.as_str(), old) {
                        ("add", Some(o)) => sel::combine_surfaces(&o, &shape, area, |a, b| a.max(b)),
                        ("subtract", Some(o)) => sel::combine_surfaces(&o, &shape, area, |a, b| a * (1.0 - b)),
                        ("intersect", Some(o)) => sel::combine_surfaces(&o, &shape, area, |a, b| a.min(b)),
                        // With no existing selection, neither operation can select new pixels.
                        ("subtract" | "intersect", None) => Surface::new(photocraft_color::PixelFormat::GRAY8),
                        _ => shape,
                    };
                    // Nothing selected on the canvas (e.g. dragged entirely outside it) deselects.
                    doc.selection = (!combined.content_bounds().is_empty()).then_some(combined);
                    Ok(())
                })?;
                Ok(Value::Null)
            }
        ),
    ]
}
