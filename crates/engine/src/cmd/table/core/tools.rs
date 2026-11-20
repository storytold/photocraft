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
                if p.get("foreground").is_some_and(|v| !v.is_null()) || fg_cmyk.is_some() {
                    s.tools.clear_role_cmyk(0);
                    if let Some(inks) = fg_cmyk {
                        s.tools.remember_role_cmyk(0, inks);
                        s.tools.remember_cmyk([foreground[0], foreground[1], foreground[2]], inks);
                    }
                }
                if p.get("background").is_some_and(|v| !v.is_null()) || bg_cmyk.is_some() {
                    s.tools.clear_role_cmyk(1);
                    if let Some(inks) = bg_cmyk {
                        s.tools.remember_role_cmyk(1, inks);
                        s.tools.remember_cmyk([background[0], background[1], background[2]], inks);
                    }
                }
                Ok(Value::Null)
            }
        ),
        cmd!("tools.swapColors", "Switch Foreground and Background Colors", [], Some("X"), "{}", always, |s, _| {
            std::mem::swap(&mut s.tools.foreground, &mut s.tools.background);
            s.tools.cmyk_roles[0].swap(0, 1);
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
            s.tools.clear_role_cmyk(0);
            s.tools.clear_role_cmyk(1);
            Ok(Value::Null)
        }),
    ]
}
