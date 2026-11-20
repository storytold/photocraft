use crate::cmd::{Adjustment, CommandSpec, Value, destructive_adjust};

/// Image › Adjustments › Desaturate: Hue/Saturation with saturation at -100.
pub(crate) fn desaturate_adjustment() -> Adjustment {
    Adjustment::HueSaturation { hue: 0.0, saturation: -100.0, lightness: 0.0, colorize: false, ranges: photocraft_doc::adjust::HueRange::defaults() }
}

/// Image menu: Desaturate.
pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "image.adjustments.desaturate",
        "Desaturate",
        ["Image", "Adjustments"],
        Some("Cmd+Shift+U"),
        r##"{"target":"pixels"|"mask"|{"color":i}|{"colors":[i,…]}?=Channels panel target,"layer":id|name?=active}"##,
        crate::adjust_cmds::has_adjustable,
        |s, p| {
            if crate::adjust_cmds::adjust_as_smart_filter(s, "desaturate", "Desaturate", p)? {
                return Ok(Value::Null);
            }
            destructive_adjust(s, "Desaturate", desaturate_adjustment(), p)
        }
    )]
}
