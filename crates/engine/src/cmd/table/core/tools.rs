use crate::cmd::{CommandSpec, Value, always, cmyk_param, color_param};

/// Tool state: foreground and background colors.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "tools.setColors",
            "Set Colors",
            [],
            None,
            r##"{"foreground":"#rrggbb"|[r,g,b,a?]?,"background":same?,"foregroundCmyk":[c,m,y,k]? (inks 0..1 the foreground was picked as: a CMYK document is filled with exactly these),"backgroundCmyk":same?}"##,
            always,
            |s, p| {
                let cmyk = |key: &str| cmyk_param("tools.setColors", p, key);
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
