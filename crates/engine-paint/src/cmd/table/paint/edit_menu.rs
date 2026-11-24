use crate::commands::{CommandSpec, has_pixel_or_channel};

/// Edit menu: Fill.
pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "edit.fill",
        "Fill…",
        ["Edit"],
        Some("Shift+F5"),
        r##"{"contents":"foreground|background|color|contentAware|pattern|history|black|gray|white"="color","color":"#rrggbb|[r,g,b,a]"=foreground (contents=color),"colorCmyk":[c,m,y,k]? (native inks 0..1 for contents=color),"pattern":id|name (contents=pattern),"scale":%=100,"angle":deg,"state":row?,"snapshot":id? (contents=history: a History panel row as history.setSource, redo rows last, or a snapshot; default the History Brush source),"colorAdaptation":bool=true,"scaleAdaptation":bool=false,"mirror":bool=false,"rotationAdaptation":bool=false (contents=contentAware: enable quarter-turn source candidates within the sampling budget),"mode":"normal|multiply|…"="normal","opacity":0..100=100,"preserveTransparency":bool=false,"target":"pixels"|"mask"|"filterMask"|"quickMask"|{"channel":i}?}"##,
        has_pixel_or_channel,
        crate::fill_cmds::fill
    )]
}
