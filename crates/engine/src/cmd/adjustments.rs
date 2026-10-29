use super::{CommandSpec, NEW_LAYER_PATH, Value, adjust_mode, destructive_adjust, doc_mode, gray_tone_params, has_doc, is_mask_target, new_adjustment};

/// The adjustment layers and the Image › Adjustments commands, generated from one table.
pub(super) fn specs() -> Vec<CommandSpec> {
    let mut v = Vec::new();
    // Adjustment layers + destructive adjustments, generated from one list.
    // Levels/Curves channel keys follow the document: red/green/blue (RGB), gray (Grayscale),
    // cyan/magenta/yellow/black (CMYK), lightness/a/b (Lab); see `adjust_params`.
    const ADJ: &[(&str, &str, &str)] = &[
        ("brightnessContrast", "Brightness/Contrast…", r##"{"brightness":-150..150=0,"contrast":-50..100=0,"legacy":bool=false}"##),
        (
            "levels",
            "Levels…",
            r##"{"inBlack":0..253=0,"gamma":0.01..9.99=1,"inWhite":2..255=255,"outBlack":0..255=0,"outWhite":0..255=255,"red":json,"green":json,"blue":json,"eyedropper":json} (top level = composite; per channel {"inBlack","gamma","inWhite","outBlack","outWhite"} under red/green/blue, gray, cyan/magenta/yellow/black or lightness/a/b; eyedropper = the dialog's Set Black/Neutral Gray/White Point picker {"point":"black|gray|white","at":[x,y],"size":1..101=1} or {"point",…,"color":[r,g,b] 0..1}: black/white set each channel's input black/white to the sample, gray each channel's gamma so the sample turns neutral at its luma; RGB and Grayscale (no gray) only, applied over the given levels)"##,
        ),
        (
            "curves",
            "Curves…",
            r##"{"points":json,"red":json,"green":json,"blue":json,"pencil":bool=false,"smooth":0..10=0,"eyedropper":json} (curves as [[in,out],…] in 0..255, 2..19 points: points = composite; red/green/blue, gray, cyan/magenta/yellow/black or lightness/a/b per channel; pencil = drawn curves (Draw to modify the curve): each curve is 256 output levels in 0..255 (point curves given are drawn in, and turning pencil off fits points to the drawing); smooth = Smooth passes over the drawn curves; eyedropper = the dialog's Set Black/Neutral Gray/White Point picker {"point":"black|gray|white","at":[x,y],"size":1..101=1} or {"point",…,"color":[r,g,b] 0..1}, RGB and Grayscale (no gray) only, applied over the given curves)"##,
        ),
        ("exposure", "Exposure…", r##"{"exposure":-20..20=0,"offset":-0.5..0.5=0,"gamma":0.01..9.99=1}"##),
        ("vibrance", "Vibrance…", r##"{"vibrance":-100..100=0,"saturation":-100..100=0}"##),
        (
            "hueSaturation",
            "Hue/Saturation…",
            r##"{"hue":-180..180=0,"saturation":-100..100=0,"lightness":-100..100=0,"colorize":bool=false,"reds":json,"yellows":json,"greens":json,"cyans":json,"blues":json,"magentas":json} (per range {"hue","saturation","lightness","range":[4 hue degrees]}; colorize: hue 0..360, saturation 0..100)"##,
        ),
        (
            "colorBalance",
            "Color Balance…",
            r##"{"shadows":json,"midtones":json,"highlights":json,"preserveLuminosity":bool=true} (each tone [cyan-red, magenta-green, yellow-blue] in -100..100)"##,
        ),
        (
            "blackWhite",
            "Black & White…",
            r##"{"reds":-200..300=40,"yellows":-200..300=60,"greens":-200..300=40,"cyans":-200..300=60,"blues":-200..300=20,"magentas":-200..300=80,"tint":bool=false,"tintColor":"#rrggbb"}"##,
        ),
        (
            "photoFilter",
            "Photo Filter…",
            r##"{"filter":"warming85|warmingLBA|warming81|cooling80|coolingLBB|cooling82|red|orange|yellow|green|cyan|blue|violet|magenta|sepia|deepRed|deepBlue|deepEmerald|deepYellow|underwater","color":"#rrggbb","density":0..100=25,"preserveLuminosity":bool=true}"##,
        ),
        (
            "channelMixer",
            "Channel Mixer…",
            r##"{"red":json,"green":json,"blue":json,"gray":json,"monochrome":bool=false} (per output channel [red %, green %, blue %, constant %] in -200..200; gray = the monochrome mix)"##,
        ),
        ("invert", "Invert", "{}"),
        ("posterize", "Posterize…", r##"{"levels":2..255=4}"##),
        ("threshold", "Threshold…", r##"{"level":1..255=128}"##),
        ("gradientMap", "Gradient Map…", r##"{"stops":json,"reverse":bool=false,"dither":bool=false} (stops [[location 0..1, "#rrggbb"], …], 2..64)"##),
        (
            "selectiveColor",
            "Selective Color…",
            r##"{"method":"relative|absolute"="relative","colors":"reds|yellows|greens|cyans|blues|magentas|whites|neutrals|blacks"="reds","cyan":-100..100=0,"magenta":-100..100=0,"yellow":-100..100=0,"black":-100..100=0,"reds":json} (per-range [c,m,y,k] arrays: reds yellows greens cyans blues magentas whites neutrals blacks)"##,
        ),
        (
            "colorLookup",
            "Color Lookup…",
            r##"{"lut":"none|warm|cool|tealOrange|bleachBypass|fadedFilm|dayForNight|monoContrast|crossProcess"="none","file":text,"interpolation":"trilinear|tetrahedral"="trilinear","dither":bool=false,"data":json} (file: .cube/.3dl/.look path; data: file text + "fileName")"##,
        ),
    ];
    for &(kind, label, params) in ADJ {
        let layer_id: &'static str = Box::leak(format!("layer.newAdjustmentLayer.{kind}").into_boxed_str());
        let image_id: &'static str = Box::leak(format!("image.adjustments.{kind}").into_boxed_str());
        v.push(CommandSpec {
            id: layer_id,
            label,
            menu: &["Layer", "New Adjustment Layer"],
            shortcut: None,
            params: Box::leak(format!("{params} + {{{NEW_LAYER_PATH}}}").into_boxed_str()),
            enabled: has_doc,
            run: |s, p| {
                let kind = p.get("__kind").and_then(Value::as_str).unwrap_or("invert").to_string();
                let eyedropped = if matches!(kind.as_str(), "levels" | "curves") { crate::adjust_params::eyedropper_from_params(s, &kind, p)? } else { None };
                let adj = match eyedropped {
                    Some(adj) => adj,
                    None => crate::adjust_params::from_params(&kind, p, None, doc_mode(s))?,
                };
                new_adjustment(s, adj, p, &format!("layer.newAdjustmentLayer.{kind}"))
            },
            journal: true,
        });
        v.push(CommandSpec {
            id: image_id,
            label,
            menu: &["Image", "Adjustments"],
            shortcut: match kind {
                "levels" => Some("Cmd+L"),
                "curves" => Some("Cmd+M"),
                "hueSaturation" => Some("Cmd+U"),
                "colorBalance" => Some("Cmd+B"),
                "invert" => Some("Cmd+I"),
                "blackWhite" => Some("Cmd+Alt+Shift+B"),
                _ => None,
            },
            params,
            enabled: crate::adjust_cmds::has_adjustable,
            run: |s, p| {
                let kind = p.get("__kind").and_then(Value::as_str).unwrap_or("invert").to_string();
                let eyedropped = if matches!(kind.as_str(), "levels" | "curves") { crate::adjust_params::eyedropper_from_params(s, &kind, p)? } else { None };
                // A smart filter re-runs its params, so it records what an eyedropper resolved
                // to, not the sample (#3127).
                let recorded = eyedropped.as_ref().map(|adj| {
                    let mut q = p.clone();
                    if let (Value::Object(q), Value::Object(resolved)) = (&mut q, crate::adjust_params::to_params(adj)) {
                        q.remove("eyedropper");
                        q.extend(resolved);
                    }
                    q
                });
                let adj = match eyedropped {
                    Some(adj) => adj,
                    None => {
                        // A gray plane target keeps the neutral tone space even over params
                        // carrying Lab or CMYK records (#5335).
                        let tone = if is_mask_target(p) || crate::channel_cmds::is_channel_target(p) {
                            if matches!(kind.as_str(), "levels" | "curves") { gray_tone_params(p) } else { p.clone() }
                        } else {
                            p.clone()
                        };
                        crate::adjust_params::from_params(&kind, &tone, None, adjust_mode(s, p))?
                    }
                };
                let label = adj.label().to_string();
                // A smart object records the adjustment as a smart filter (Photoshop), which
                // re-runs from the source instead of baking into a copy of the pixels.
                if crate::adjust_cmds::adjust_as_smart_filter(s, &kind, &label, recorded.as_ref().unwrap_or(p))? {
                    return Ok(Value::Null);
                }
                destructive_adjust(s, &label, adj, p)
            },
            journal: true,
        });
    }
    v
}
