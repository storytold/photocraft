//! Layer › New › Layer: Photoshop's New Layer options (name, Use Previous Layer to Create Clipping
//! Mask, color label, mode, opacity, and Fill with neutral color), applied in one history step.

use photocraft_doc::{BlendMode, LabelColor, Layer};
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

const CMD: &str = "layer.new.layer";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// The color that leaves everything below unchanged in `mode` (Photoshop's "Fill with
/// Multiply-neutral color (white)"), as RGB, and its name. `None` for modes that have none.
pub fn neutral_color(mode: BlendMode) -> Option<([f32; 3], &'static str)> {
    use BlendMode::*;
    match mode {
        Darken | Multiply | ColorBurn | LinearBurn | DarkerColor | Divide => Some(([1.0; 3], "white")),
        Lighten | Screen | ColorDodge | LinearDodge | LighterColor | Difference | Exclusion | Subtract => Some(([0.0; 3], "black")),
        Overlay | SoftLight | HardLight | VividLight | LinearLight | PinLight => Some(([0.5; 3], "50% gray")),
        _ => None,
    }
}

/// A New Layer call's params, checked.
pub(crate) struct NewLayer {
    clip: bool,
    mode: BlendMode,
    opacity: f32,
    label: LabelColor,
    neutral: Option<[f32; 3]>,
    below: bool,
    name: Option<String>,
    group: Option<GroupRef>,
}

/// `layer.new.layer`: a pixel layer where Photoshop puts a new one ([`crate::layer_place`]: above
/// the active layer, inside it when it is an open group, below it with `position: below`, the Layers
/// panel's ⌘/Ctrl-click), joining the clip group it lands in.
pub fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let NewLayer { clip, mode, opacity, label, neutral, below, name, group } = parse(p)?;
    let place = match (&group, below) {
        (Some(_), _) => crate::layer_place::Place::Into,
        (None, true) => crate::layer_place::Place::Below,
        (None, false) => crate::layer_place::Place::Auto,
    };
    let id = s.edit("New Layer", |doc, active| {
        let group = group.as_ref().map(|g| g.in_doc(doc, CMD)).transpose()?;
        let name = name.clone().unwrap_or_else(|| doc.next_layer_name("Layer"));
        let mut layer = Layer::raster(name, doc.pixel_format());
        layer.blend = mode;
        layer.opacity = opacity;
        layer.label = label;
        // Checked inside the edit as well as before it (see [`admit`]), so the group can't go away
        // between the check and the insert; before the fill, which it would waste.
        if let Some(g) = group {
            crate::layer_place::check_into(doc, g, &layer).map_err(bad)?;
        }
        if let Some(rgb) = neutral {
            let bounds = doc.bounds();
            let surface = layer.surface_mut().ok_or_else(|| bad("a new pixel layer has no pixels"))?;
            let px = photocraft_raster::from_rgba(&surface.format(), [rgb[0], rgb[1], rgb[2], 1.0]);
            surface.fill_rect(bounds, &px);
        }
        let id = crate::layer_place::insert_new(doc, group.or(*active), layer, place);
        crate::clip_group::settle(doc, &[id], &Default::default(), clip);
        *active = Some(id);
        Ok(id)
    })?;
    // A name given here is the user's, as one given later is: the layer keeps it if it becomes text.
    if let Some(name) = &name {
        crate::type_cmds::note_typed_name(s, id, name);
    }
    Ok(json!({ "layer": id.0 }))
}

/// Checks New Layer's and New Group's `group` before a floating selection drops (dispatch's
/// admission), so a call into a layer that is no group refuses with nothing changed. The edit
/// checks it again.
pub(crate) fn admit(s: &Session, id: &str, p: &Value) -> Result<()> {
    if !matches!(id, CMD | "layer.new.group") {
        return Ok(());
    }
    let Some(group) = group_param(id, p)? else { return Ok(()) };
    if given_position(p) {
        return Err(EngineError::BadParams { cmd: id.into(), msg: "give `group` or `position`, not both".into() });
    }
    let Some(d) = s.active() else { return Ok(()) };
    let group = group.in_doc(&d.doc, id)?;
    let layer = if id == CMD { Layer::raster(String::new(), d.doc.pixel_format()) } else { Layer::group(String::new(), vec![]) };
    crate::layer_place::check_into(&d.doc, group, &layer).map_err(|msg| EngineError::BadParams { cmd: id.into(), msg })
}

/// Check a New Layer call's params without running it (Free Transform applies its box before a
/// new layer only when the new layer can be made).
pub(crate) fn parse(p: &Value) -> Result<NewLayer> {
    let flag = |k: &str| match p.get(k) {
        None | Some(Value::Null) => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| bad(format!("`{k}` must be true or false"))),
    };
    let clip = flag("clipToPrevious")?;
    let fill_neutral = flag("fillNeutral")?;
    let mode = match p.get("mode") {
        None | Some(Value::Null) => BlendMode::Normal,
        Some(v) => {
            let name = v.as_str().ok_or_else(|| bad("`mode` must be a blend mode name"))?;
            crate::commands::parse_blend(name).filter(|m| *m != BlendMode::PassThrough).ok_or_else(|| bad(format!("unknown layer blend mode `{name}`")))?
        }
    };
    let opacity = match p.get("opacity") {
        None | Some(Value::Null) => 1.0,
        Some(v) => v.as_f64().filter(|o| o.is_finite()).ok_or_else(|| bad("`opacity` must be a number from 0 to 1"))?.clamp(0.0, 1.0) as f32,
    };
    let label = match p.get("color") {
        None | Some(Value::Null) => LabelColor::None,
        Some(v) => v.as_str().and_then(LabelColor::from_id).ok_or_else(|| bad(format!("`color` must be a layer color id, not {v}")))?,
    };
    let neutral =
        if fill_neutral { Some(neutral_color(mode).ok_or_else(|| bad(format!("{} has no neutral color to fill with", mode.label())))?.0) } else { None };
    let below = match p.get("position") {
        None | Some(Value::Null) => false,
        Some(v) => match v.as_str() {
            Some("above") => false,
            Some("below") => true,
            _ => return Err(bad("`position` must be above or below")),
        },
    };
    let name = match p.get("name") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or_else(|| bad("`name` must be a string"))?.to_string()),
    };
    let group = group_param(CMD, p)?;
    if group.is_some() && given_position(p) {
        return Err(bad("give `group` or `position`, not both"));
    }
    Ok(NewLayer { clip, mode, opacity, label, neutral, below, name, group })
}

/// Whether `p` names a `position` (`"above"` too): New Layer and New Group take it or `group`.
pub(crate) fn given_position(p: &Value) -> bool {
    p.get("position").is_some_and(|v| !v.is_null())
}

/// The group a New Layer or New Group goes inside: by id, or by the name a recording keeps when
/// it names one layer only (see `actions_record::portable`).
#[derive(Clone, Debug)]
pub(crate) enum GroupRef {
    Id(photocraft_doc::LayerId),
    Name(String),
}

impl GroupRef {
    /// The layer it names in `doc`: a name must name exactly one layer.
    pub(crate) fn in_doc(&self, doc: &photocraft_doc::Document, cmd: &str) -> Result<photocraft_doc::LayerId> {
        match self {
            Self::Id(id) => Ok(*id),
            Self::Name(name) => {
                let mut named = doc.walk().into_iter().filter(|(_, _, l)| &l.name == name).map(|(_, _, l)| l.id);
                match (named.next(), named.next()) {
                    (Some(id), None) => Ok(id),
                    (None, _) => Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("no layer named \"{name}\"") }),
                    (Some(_), Some(_)) => Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("more than one layer is named \"{name}\"; give its id") }),
                }
            }
        }
    }
}

/// The `group` param of New Layer and New Group: the group the new layer goes inside, as its top
/// child.
pub(crate) fn group_param(cmd: &str, p: &Value) -> Result<Option<GroupRef>> {
    match p.get("group") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(name)) => Ok(Some(GroupRef::Name(name.clone()))),
        Some(v) => v
            .as_u64()
            .map(|g| Some(GroupRef::Id(photocraft_doc::LayerId(g))))
            .ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: "`group` must be a layer id or name".into() }),
    }
}

#[cfg(test)]
mod tests {
    use crate::Session;
    use photocraft_doc::{BlendMode, LabelColor};
    use photocraft_geom::Rect;
    use serde_json::json;

    /// A document with a busy Background: every pixel a different colour.
    fn busy(mode: &str, depth: u32) -> Session {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 32, "height": 32, "mode": mode, "depth": depth})).unwrap();
        s.edit("busy", |doc, active| {
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            let fmt = surf.format();
            for y in 0..32 {
                for x in 0..32 {
                    let rgba = [x as f32 / 31.0, y as f32 / 31.0, ((x * 7 + y * 3) % 32) as f32 / 31.0, 1.0];
                    surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &photocraft_raster::from_rgba(&fmt, rgba));
                }
            }
            Ok(())
        })
        .unwrap();
        s
    }

    #[test]
    fn options_apply_in_one_step() {
        let mut s = busy("rgb", 8);
        let steps = s.active().unwrap().history.past_len();
        let id =
            s.execute("layer.new.layer", json!({"name": "Shade", "mode": "Overlay", "opacity": 0.4, "color": "blue", "fillNeutral": true})).unwrap()["layer"]
                .as_u64()
                .unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.history.past_len(), steps + 1);
        let l = st.doc.layer(photocraft_doc::LayerId(id)).unwrap();
        assert_eq!((l.name.as_str(), l.blend, l.opacity, l.label), ("Shade", BlendMode::Overlay, 0.4, LabelColor::Blue));
        assert_eq!(st.active_layer, Some(l.id));
        // Defaults: what the Layers panel button makes.
        s.execute("layer.new.layer", json!({})).unwrap();
        let st = s.active().unwrap();
        let l = st.doc.layers.last().unwrap();
        assert_eq!((l.blend, l.opacity, l.label, l.clipped), (BlendMode::Normal, 1.0, LabelColor::None, false));
        assert_eq!(l.surface().unwrap().tile_count(), 0);
    }

    /// `group` puts the new layer or group on top inside that group, collapsed or not, in one
    /// step; a group that isn't there (or isn't a group) is an error that changes nothing.
    #[test]
    fn group_puts_the_new_layer_inside_that_group_or_changes_nothing() {
        let mut s = busy("rgb", 8);
        let a = s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap();
        let g = s.execute("layer.groupLayers", json!({"layer": a})).unwrap()["layer"].as_u64().unwrap();
        let top = s.execute("layer.new.layer", json!({"name": "Top"})).unwrap()["layer"].as_u64().unwrap();
        let children = |s: &Session| {
            let doc = &s.active().unwrap().doc;
            doc.layer(photocraft_doc::LayerId(g)).unwrap().children().unwrap().iter().map(|l| l.name.clone()).collect::<Vec<_>>()
        };
        let steps = |s: &Session| s.active().unwrap().history.past_len();
        // The active layer is outside the (collapsed) group; the new layers still go inside it.
        assert_eq!(s.active().unwrap().active_layer, Some(photocraft_doc::LayerId(top)));
        let before = steps(&s);
        s.execute("layer.new.layer", json!({"name": "In", "group": g})).unwrap();
        s.execute("layer.new.group", json!({"name": "Sub", "group": g})).unwrap();
        assert_eq!(children(&s), ["A", "In", "Sub"]);
        assert_eq!(steps(&s), before + 2);
        // Not a group, no such layer, or with any `position`: refused, nothing made.
        let shown = s.active().unwrap().doc.clone();
        for p in [
            json!({"group": top}),
            json!({"group": 999_999}),
            json!({"group": g, "position": "below"}),
            json!({"group": g, "position": "above"}),
            json!({"group": "x"}),
        ] {
            assert!(s.execute("layer.new.layer", p.clone()).is_err(), "{p}");
            assert!(s.execute("layer.new.group", p.clone()).is_err(), "{p}");
        }
        assert_eq!(steps(&s), before + 2);
        assert_eq!(s.active().unwrap().doc.layers.len(), shown.layers.len());
        assert_eq!(children(&s), ["A", "In", "Sub"]);
    }

    /// `position: below` (⌘/Ctrl-click on the panel button) lands under the active layer; the
    /// Background stays at the bottom, so below it the layer lands above it.
    #[test]
    fn position_below_lands_under_the_active_layer() {
        let mut s = busy("rgb", 8);
        let names = |s: &Session| s.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
        s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
        let below = s.execute("layer.new.layer", json!({"name": "Below", "position": "below"})).unwrap()["layer"].as_u64().unwrap();
        assert_eq!(names(&s), ["Background", "Below", "A"]);
        assert_eq!(s.active().unwrap().active_layer, Some(photocraft_doc::LayerId(below)));
        s.execute("layer.new.layer", json!({"name": "Above", "position": "above"})).unwrap();
        assert_eq!(names(&s), ["Background", "Below", "Above", "A"]);
        let bg = s.active().unwrap().doc.layers[0].id.0;
        s.execute("layer.select", json!({"layer": bg})).unwrap();
        s.execute("layer.new.layer", json!({"name": "OverBg", "position": "below"})).unwrap();
        assert_eq!(names(&s), ["Background", "OverBg", "Below", "Above", "A"]);
    }

    /// Bad params are errors, never panics, and change nothing.
    #[test]
    fn bad_params_are_errors() {
        let mut s = busy("rgb", 8);
        let n = s.active().unwrap().doc.layers.len();
        for p in [
            json!({"mode": "Pass Through"}),
            json!({"mode": "Sparkle"}),
            json!({"mode": 3}),
            json!({"opacity": "half"}),
            json!({"color": "plaid"}),
            json!({"color": 1}),
            json!({"fillNeutral": true}),
            json!({"fillNeutral": true, "mode": "Hue"}),
            json!({"fillNeutral": "yes"}),
            json!({"clipToPrevious": 1}),
            json!({"name": 7}),
            json!({"position": "sideways"}),
            json!({"position": 1}),
        ] {
            assert!(s.execute("layer.new.layer", p.clone()).is_err(), "{p}");
        }
        assert_eq!(s.active().unwrap().doc.layers.len(), n);
        // Out-of-range opacity is clamped, as layer.setProps does.
        s.execute("layer.new.layer", json!({"opacity": 7})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.last().unwrap().opacity, 1.0);
    }
}
