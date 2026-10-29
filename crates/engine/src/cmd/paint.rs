use super::{CommandSpec, Result, Value, always, bad, color_param, has_paintable};

/// Painting commands: strokes and the foreground and background colors.
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "paint.stroke",
            "Brush Stroke",
            [],
            None,
            r##"{"points":[[x,y,pressure?,tiltX?,tiltY?,rotation?,timeMs?,wheel?],…],"brush":{…BrushSettings}?,"preset":name?,"size":0.5..5000 px?,"hardness":0..1?,"opacity":0..1?,"flow":0..1?,"fill":bool=false (fill the region the closed `points` enclose with the brush colour, its edge anti-aliased and softened by hardness, instead of stamping the outline; a closed gesture paints a filled dab, not a ring),"spacing":0..10?,"color":"#rrggbb"?=foreground,"mode":"normal|multiply|screen|…|clear (erases with this brush)"="normal","erase":bool?,"smoothing":0..1?,"zoom":number=1,"seed":u64?,"target":"pixels"|"mask"|"filterMask"|"quickMask"|{"channel":i}=Channels panel target}"##,
            has_paintable,
            crate::brush_cmds::paint_stroke
        ),
        cmd!(
            "tools.setColors",
            "Set Colors",
            [],
            None,
            r##"{"foreground":"#rrggbb"|[r,g,b,a?]?,"background":same?,"foregroundCmyk":[c,m,y,k]? (inks 0..1 the foreground was picked as: a CMYK document is filled with exactly these),"backgroundCmyk":same?}"##,
            always,
            |s, p| {
                let cmyk = |key: &str| -> Result<Option<[f32; 4]>> {
                    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
                    let inks: Option<Vec<f32>> = v
                        .as_array()
                        .filter(|a| a.len() == 4)
                        .and_then(|a| a.iter().map(|x| x.as_f64().filter(|x| x.is_finite()).map(|x| x.clamp(0.0, 1.0) as f32)).collect());
                    match inks.as_deref() {
                        Some(&[c, m, y, k]) => Ok(Some([c, m, y, k])),
                        _ => Err(bad("tools.setColors", format!("`{key}` must be [c, m, y, k] numbers 0..1"))),
                    }
                };
                let (fg_cmyk, bg_cmyk) = (cmyk("foregroundCmyk")?, cmyk("backgroundCmyk")?);
                let foreground = color_param("tools.setColors", p, "foreground", s.tools.foreground)?;
                let background = color_param("tools.setColors", p, "background", s.tools.background)?;
                s.tools.foreground = foreground;
                s.tools.background = background;
                if let Some(k) = fg_cmyk {
                    let [r, g, b, _] = s.tools.foreground;
                    s.tools.remember_cmyk([r, g, b], k);
                }
                if let Some(k) = bg_cmyk {
                    let [r, g, b, _] = s.tools.background;
                    s.tools.remember_cmyk([r, g, b], k);
                }
                Ok(Value::Null)
            }
        ),
        cmd!("tools.swapColors", "Switch Foreground and Background Colors", [], Some("X"), "{}", always, |s, _| {
            std::mem::swap(&mut s.tools.foreground, &mut s.tools.background);
            Ok(Value::Null)
        }),
        cmd!("tools.defaultColors", "Default Foreground and Background Colors", [], Some("D"), "{}", always, |s, _| {
            if s.tools.mask_targeted {
                s.tools.foreground = crate::ToolState::MASK_COLORS[0];
                s.tools.background = crate::ToolState::MASK_COLORS[1];
            } else {
                s.tools.foreground = [0.0, 0.0, 0.0, 1.0];
                s.tools.background = [1.0, 1.0, 1.0, 1.0];
            }
            Ok(Value::Null)
        }),
    ]
}
