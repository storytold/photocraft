//! Image › DTF Spot Channel, and the built-in Actions preset "DTF".
//!
//! A direct-to-film print file needs a spot channel of the artwork, shrunk by a pixel so the
//! white underbase does not peek past the colour. Photoshop's recorded action loads transparency,
//! contracts by 1px without treating the canvas edge as a selection edge, then makes a spot
//! channel named "Spot Color 1" (RGB red, solidity 60). [`run`] does that as one history step.
//! The Actions panel preset is that same command, filed in the action set "DTF", so the step's
//! numbers and text can be edited. The last successful settings are stored with the other
//! remembered dialogs and become the next dialog's defaults and, while the preset is still that
//! one step, the values Play uses.

use photocraft_algo::selection::{self as sel, SelectionMode};
use photocraft_color::Color;
use photocraft_doc::AlphaChannel;
use serde_json::{Value, json};

use crate::actions_cmds::{Action, ActionStep, ActionsExt};
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Name of the built-in action in the Actions panel.
pub const ACTION_NAME: &str = "DTF";
/// Action set the built-in action is filed under.
pub const SET_NAME: &str = "DTF";
/// `source` value: transparency of the active layer.
pub const SOURCE_LAYER: &str = "layer";
/// `source` value: alpha of the composite of every visible layer.
pub const SOURCE_COMPOSITE: &str = "composite";
/// Contract radius, in pixels. `0` keeps the transparency mask as it is.
pub const DEFAULT_CONTRACT: f32 = 1.0;
pub const DEFAULT_NAME: &str = "Spot Color 1";
pub const DEFAULT_COLOR: &str = "#ff0000";
/// Solidity on Photoshop's 0..100 scale. Stored on the channel as a 0..1 fraction.
pub const DEFAULT_SOLIDITY: f64 = 60.0;

const CMD: &str = "image.dtf";
const MAX_CONTRACT: f32 = 500.0;
const MAX_NAME_CHARS: usize = 256;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn number(p: &Value, key: &str) -> Result<Option<f64>> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::Number(n)) => {
            let v = n.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad(format!("`{key}` is not a usable number")))?;
            Ok(Some(v))
        }
        Some(_) => Err(bad(format!("`{key}` must be a number"))),
    }
}

fn flag(p: &Value, key: &str, default: bool) -> Result<bool> {
    match p.get(key) {
        None => Ok(default),
        Some(Value::Bool(v)) => Ok(*v),
        Some(_) => Err(bad(format!("`{key}` must be true or false"))),
    }
}

/// `#rrggbb`, or a JSON array of three components. Components above 1 are 0..255.
fn color_of(p: &Value) -> Result<Color> {
    match p.get("color") {
        None => Ok(Color::rgb(1.0, 0.0, 0.0)),
        Some(Value::String(s)) => {
            let c = crate::commands::parse_hex(s).ok_or_else(|| bad("color must be #rrggbb"))?;
            Ok(Color::rgb(c[0], c[1], c[2]))
        }
        Some(Value::Array(a)) if a.len() >= 3 => {
            let mut v = [0.0f32; 3];
            for (i, slot) in v.iter_mut().enumerate() {
                let n = a.get(i).and_then(Value::as_f64).filter(|n| n.is_finite()).ok_or_else(|| bad("color components must be numbers"))?;
                *slot = n as f32;
            }
            if v.iter().any(|c| *c > 1.0) {
                if v.iter().any(|c| !(0.0..=255.0).contains(c)) {
                    return Err(bad("color components must be 0..1 or 0..255"));
                }
                v = v.map(|c| c / 255.0);
            } else if v.iter().any(|c| *c < 0.0) {
                return Err(bad("color components must be 0..1 or 0..255"));
            }
            Ok(Color::rgb(v[0], v[1], v[2]))
        }
        Some(_) => Err(bad("color must be #rrggbb or [r, g, b]")),
    }
}

fn name_of(p: &Value) -> Result<String> {
    match p.get("name") {
        None => Ok(DEFAULT_NAME.to_string()),
        Some(Value::String(s)) if s.is_empty() => Err(bad("name must not be empty")),
        Some(Value::String(s)) if s.chars().count() > MAX_NAME_CHARS => Err(bad(format!("name must be at most {MAX_NAME_CHARS} characters"))),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(bad("name must be a string")),
    }
}

fn source_of(p: &Value) -> Result<bool> {
    match p.get("source") {
        None => Ok(false),
        Some(Value::String(s)) if s == SOURCE_LAYER => Ok(false),
        Some(Value::String(s)) if s == SOURCE_COMPOSITE => Ok(true),
        Some(_) => Err(bad("source must be \"layer\" or \"composite\"")),
    }
}

fn hex_color(color: Color) -> String {
    let [r, g, b] = color.to_rgb();
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
}

/// The dialog and the built-in action start from these values.
pub fn default_params() -> Value {
    json!({
        "source": SOURCE_LAYER,
        "contract": DEFAULT_CONTRACT,
        "name": DEFAULT_NAME,
        "color": DEFAULT_COLOR,
        "solidity": DEFAULT_SOLIDITY,
        "applyAtCanvasBounds": false,
    })
}

/// Fields for the Image › DTF Spot Channel dialog, prefilled from the last successful run.
pub fn dialog_fields(saved: Option<&Value>) -> Value {
    let mut fields = default_params();
    if let Some(saved) = saved.and_then(Value::as_object)
        && let Some(fields_obj) = fields.as_object_mut()
    {
        for (key, value) in saved {
            if key.starts_with("__") {
                continue;
            }
            fields_obj.insert(key.clone(), value.clone());
        }
        let source = fields_obj.get("source").and_then(Value::as_str).unwrap_or("");
        if source != SOURCE_LAYER && source != SOURCE_COMPOSITE {
            fields_obj.insert("source".into(), json!(SOURCE_LAYER));
        }
        if !fields_obj.get("applyAtCanvasBounds").is_some_and(Value::is_boolean) {
            fields_obj.insert("applyAtCanvasBounds".into(), json!(false));
        }
    }
    fields
}

/// Active-layer transparency over the canvas: 0 where the layer is clear, 1 where a layer
/// without pixels (an adjustment or fill) covers the canvas.
fn transparency(doc: &photocraft_doc::Document, layer: photocraft_doc::LayerId, n: usize) -> Result<Vec<f32>> {
    let area = doc.bounds();
    let layer = doc.layer(layer).ok_or(EngineError::NoLayer(layer))?;
    match layer.surface() {
        Some(surf) => {
            let mut px = vec![[0.0f32; 4]; n];
            surf.read_rgba_into(area, &mut px);
            Ok(px.into_iter().map(|p| p[3]).collect())
        }
        None => Ok(vec![1.0; n]),
    }
}

/// Alpha of the flattened visible document. Hidden layers are left out.
fn composite_alpha(doc: &photocraft_doc::Document, n: usize) -> Result<Vec<f32>> {
    let buf = photocraft_compose::flatten(doc);
    if buf.px.len() != n {
        return Err(bad("the composite does not cover the canvas"));
    }
    Ok(buf.px.iter().map(|p| p[3]).collect())
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let composite = source_of(p)?;
    let contract = number(p, "contract")?.unwrap_or(f64::from(DEFAULT_CONTRACT)) as f32;
    if !(0.0..=MAX_CONTRACT).contains(&contract) {
        return Err(bad(format!("contract must be a number in 0..={MAX_CONTRACT}")));
    }
    let name = name_of(p)?;
    let color = color_of(p)?;
    let solidity_pct = number(p, "solidity")?.unwrap_or(DEFAULT_SOLIDITY);
    if !(0.0..=100.0).contains(&solidity_pct) {
        return Err(bad("solidity must be a number in 0..=100"));
    }
    let solidity = (solidity_pct as f32) / 100.0;
    let at_bounds = flag(p, "applyAtCanvasBounds", false)?;

    let i = s.edit("DTF Spot Channel", |doc, active| {
        let area = doc.bounds();
        let (w, h) = (area.width() as usize, area.height() as usize);
        let n = w.checked_mul(h).ok_or_else(|| bad("the canvas is too large"))?;
        let mut mask = if composite {
            composite_alpha(doc, n)?
        } else {
            let id = (*active).ok_or_else(|| EngineError::Other("transparency needs a layer".into()))?;
            transparency(doc, id, n)?
        };
        if contract > 0.0 {
            mask = crate::selection_cmds::contract_mask(&mask, w, h, contract, at_bounds).map_err(bad)?;
        }
        if mask.len() != n {
            return Err(bad("the contracted selection does not cover the canvas"));
        }
        doc.selection = sel::combine(None, &mask, area, SelectionMode::Replace);
        let fmt = crate::channel_cmds::channel_format(doc);
        let surface = crate::channel_cmds::plane_surface(&mask, area, fmt);
        doc.channels.push(AlphaChannel { spot: Some((color, solidity)), ..AlphaChannel::new(name.clone(), surface) });
        Ok(doc.channels.len().saturating_sub(1))
    })?;
    if let Some(st) = s.active_mut() {
        st.channel_view.set_alpha_visible(i, true);
        crate::channel_cmds::touch(st);
    }
    let selected = s.active().is_some_and(|st| st.doc.selection.is_some());
    let params = json!({
        "source": if composite { SOURCE_COMPOSITE } else { SOURCE_LAYER },
        "contract": contract,
        "name": name,
        "color": hex_color(color),
        "solidity": solidity_pct,
        "applyAtCanvasBounds": at_bounds,
    });
    remember(s, params);
    Ok(json!({ "channel": i, "name": name, "solidity": solidity_pct, "selected": selected, "source": if composite { SOURCE_COMPOSITE } else { SOURCE_LAYER } }))
}

fn remember(s: &mut Session, params: Value) {
    s.edit_prefs(|prefs| {
        prefs.dialogs.insert(CMD.into(), params.clone());
    });
    let idx =
        s.actions().list.actions().iter().position(|a| a.name == ACTION_NAME && a.steps.len() == 1 && a.steps.first().is_some_and(|step| step.command == CMD));
    if let Some(idx) = idx {
        let _ = s.actions_mut().list.replace_step(idx, 0, ActionStep::new(CMD, params));
    }
}

/// The Actions panel edited the DTF step: the next dialog should open on those values.
pub fn note_edited_step(s: &mut Session, idx: usize) {
    let Some(action) = s.actions().list.action(idx) else { return };
    if action.name != ACTION_NAME || action.steps.len() != 1 {
        return;
    }
    let Some(step) = action.steps.first() else { return };
    if step.command != CMD {
        return;
    }
    let params = step.params.clone();
    s.edit_prefs(|prefs| {
        prefs.dialogs.insert(CMD.into(), params);
    });
}

fn params_for_preset(s: &Session) -> Value {
    dialog_fields(s.prefs().dialogs.get(CMD))
}

/// The one-step action, with the default values, filed in the DTF action set.
pub fn builtin_action() -> Action {
    let mut action = Action::new(ACTION_NAME, vec![ActionStep::new(CMD, default_params())]);
    action.set = Some(SET_NAME.into());
    action
}

/// The previous preset was the three Photoshop commands. Fold those into one `image.dtf` step,
/// keeping the contract, name, colour and solidity they recorded.
fn legacy_params(action: &Action) -> Option<Value> {
    if action.steps.len() != 3 || !action.disabled.is_empty() || !action.dialogs.is_empty() {
        return None;
    }
    let (load, contract, spot) = (&action.steps[0], &action.steps[1], &action.steps[2]);
    if load.command != "select.loadSelection" || contract.command != "select.modify.contract" || spot.command != "channel.newSpot" {
        return None;
    }
    let only_keys = |params: &Value, allowed: &[&str]| params.as_object().is_some_and(|obj| obj.keys().all(|key| allowed.contains(&key.as_str())));
    if !only_keys(&load.params, &["channel", "invert", "operation"])
        || load.params.get("channel").and_then(Value::as_str) != Some("transparency")
        || !matches!(load.params.get("invert"), None | Some(Value::Bool(false)))
        || !load.params.get("operation").is_none_or(|v| matches!(v.as_str(), Some("new" | "replace")))
    {
        return None;
    }
    if !only_keys(&contract.params, &["radius", "applyAtCanvasBounds"])
        || !only_keys(&spot.params, &["name", "color", "solidity", "fromSelection"])
        || !matches!(spot.params.get("fromSelection"), None | Some(Value::Bool(true)))
        || !spot.params.get("color").is_some_and(Value::is_string)
        || ["name", "color", "solidity"].iter().any(|key| spot.params.get(key).is_none())
    {
        return None;
    }
    let mut params = default_params();
    let obj = params.as_object_mut()?;
    if let Some(radius) = contract.params.get("radius") {
        obj.insert("contract".into(), radius.clone());
    }
    if let Some(at) = contract.params.get("applyAtCanvasBounds") {
        obj.insert("applyAtCanvasBounds".into(), at.clone());
    }
    for key in ["name", "color", "solidity"] {
        if let Some(v) = spot.params.get(key) {
            obj.insert(key.into(), v.clone());
        }
    }
    let radius = number(&params, "contract").ok().flatten()?;
    if !(1.0..=f64::from(MAX_CONTRACT)).contains(&radius)
        || flag(&params, "applyAtCanvasBounds", false).is_err()
        || name_of(&params).is_err()
        || color_of(&params).is_err()
        || !number(&params, "solidity").ok().flatten().is_some_and(|n| (0.0..=100.0).contains(&n))
    {
        return None;
    }
    Some(params)
}

/// Replaces the three-command recording at `idx` with one `image.dtf` step in the DTF set.
fn fold_legacy(s: &mut Session, idx: usize, params: Value) {
    let _ = s.actions_mut().list.replace_step(idx, 0, ActionStep::new(CMD, params));
    while s.actions().list.step_count(idx) > 1 {
        let last = s.actions().list.step_count(idx).saturating_sub(1);
        let _ = s.actions_mut().list.delete_step(idx, last);
    }
    if s.actions().list.set_of(idx).is_none() {
        let _ = s.actions_mut().list.assign_set(idx, Some(SET_NAME.into()));
    }
}

/// Install the built-in DTF action when the user asks. A list that already has an action
/// named DTF is left as the user kept it, except the old three-command recording, which
/// becomes the one `image.dtf` step. Startup does not call this.
pub fn ensure_preset(s: &mut Session) {
    if let Some(idx) = s.actions().list.actions().iter().position(|a| a.name == ACTION_NAME) {
        if let Some(params) = s.actions().list.action(idx).and_then(legacy_params) {
            fold_legacy(s, idx, params);
        }
        return;
    }
    let params = params_for_preset(s);
    let mut action = Action::new(ACTION_NAME, vec![ActionStep::new(CMD, params)]);
    action.set = Some(SET_NAME.into());
    let _ = s.actions_mut().list.push(action);
}

fn install_action(s: &mut Session, _p: &Value) -> Result<Value> {
    let had = s.actions().list.actions().iter().any(|a| a.name == ACTION_NAME);
    let legacy = s.actions().list.actions().iter().any(|a| a.name == ACTION_NAME && legacy_params(a).is_some());
    ensure_preset(s);
    Ok(json!({"installed": !had || legacy, "name": ACTION_NAME, "set": SET_NAME}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: CMD,
            label: "DTF Spot Channel…",
            menu: &["Image"],
            shortcut: None,
            params: r##"{"source":"layer"|"composite"="layer","contract":0..500=1,"name":str="Spot Color 1","color":"#rrggbb"|[r,g,b]="#ff0000","solidity":0..100=60,"applyAtCanvasBounds":bool=false} → {channel,name,solidity,selected,source}. source "layer" is the active layer; "composite" is every visible layer. The values used are remembered for the next dialog and, while the DTF preset is still this one step, for Actions playback."##,
            enabled,
            run,
            journal: true,
        },
        CommandSpec {
            id: "image.installDtfAction",
            label: "Install DTF Action",
            menu: &["Image"],
            shortcut: None,
            params: r##"{} → {installed:bool,name,set}. Adds the DTF action to the Actions panel. An action already named DTF is left as the user kept it, except the old three-command recording, which becomes this one step. Loading PhotoCraft does not install it."##,
            enabled: |_| Ok(()),
            run: install_action,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_geom::Rect;

    fn paint_rect(s: &mut Session, rect: Rect) {
        s.edit("paint", |doc, active| {
            let id = (*active).ok_or(EngineError::NoDocument)?;
            let fmt = doc.pixel_format();
            let px = photocraft_raster::from_rgba(&fmt, [0.2, 0.4, 0.6, 1.0]);
            doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.surface_mut().ok_or_else(|| EngineError::Other("no pixels".into()))?.fill_rect(rect, &px);
            Ok(())
        })
        .unwrap();
    }

    fn first(s: &Session) -> &Action {
        &s.actions().list.actions()[0]
    }

    fn channel_at(s: &Session, x: i32, y: i32) -> f32 {
        let doc = &s.active().unwrap().doc;
        let ch = doc.channels.last().unwrap();
        let px = ch.surface.read_region(Rect::new(x, y, x + 1, y + 1));
        px.first().copied().unwrap_or(0.0)
    }

    #[test]
    fn contracts_an_inset_shape_by_one_pixel() {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
        paint_rect(&mut s, Rect::new(2, 2, 6, 6));
        let r = s.execute(CMD, json!({})).unwrap();
        assert_eq!(r["name"], DEFAULT_NAME);
        assert_eq!(r["solidity"], DEFAULT_SOLIDITY);
        assert_eq!(r["selected"], true);
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.channels.len(), 1);
        let ch = &doc.channels[0];
        assert_eq!(ch.name, DEFAULT_NAME);
        let (ink, sol) = ch.spot.unwrap();
        assert_eq!(ink.to_rgb(), [1.0, 0.0, 0.0]);
        assert!((sol - 0.6).abs() < 1e-5);
        // The opaque rect was (2,2)-(6,6). Contracting by 1 drops the outer pixel.
        assert_eq!(channel_at(&s, 2, 3), 0.0);
        assert_eq!(channel_at(&s, 3, 3), 1.0);
        assert_eq!(channel_at(&s, 4, 4), 1.0);
        assert_eq!(channel_at(&s, 5, 4), 0.0);
        assert!(s.active().unwrap().channel_view.alpha_shown(0));
        assert_eq!(s.active().unwrap().history.undo_label(), Some("DTF Spot Channel"));
    }

    #[test]
    fn canvas_bounds_option_matches_select_modify() {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 6, "height": 4, "background": "white"})).unwrap();
        // A locked white background is opaque to the canvas edge.
        s.execute(CMD, json!({"applyAtCanvasBounds": false, "name": "Edge"})).unwrap();
        assert_eq!(channel_at(&s, 0, 0), 1.0);
        assert_eq!(channel_at(&s, 5, 3), 1.0);
        s.execute("channel.delete", json!({"channel": "Edge"})).unwrap();
        s.execute(CMD, json!({"applyAtCanvasBounds": true, "name": "Inset"})).unwrap();
        assert_eq!(channel_at(&s, 0, 0), 0.0);
        assert_eq!(channel_at(&s, 1, 1), 1.0);
    }

    #[test]
    fn custom_color_solidity_and_contract_are_honoured() {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
        paint_rect(&mut s, Rect::new(1, 1, 7, 7));
        s.execute(CMD, json!({"contract": 2, "name": "White", "color": [255, 0, 128], "solidity": 80})).unwrap();
        let ch = &s.active().unwrap().doc.channels[0];
        assert_eq!(ch.name, "White");
        let (ink, sol) = ch.spot.unwrap();
        assert!((ink.to_rgb()[0] - 1.0).abs() < 1e-5);
        assert!((ink.to_rgb()[2] - 128.0 / 255.0).abs() < 1e-5);
        assert!((sol - 0.8).abs() < 1e-5);
        // Two pixels in from the rect's left edge (x = 1).
        assert_eq!(channel_at(&s, 2, 3), 0.0);
        assert_eq!(channel_at(&s, 3, 3), 1.0);
    }

    #[test]
    fn zero_contract_keeps_the_transparency_edge() {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 6, "height": 6, "background": "transparent"})).unwrap();
        paint_rect(&mut s, Rect::new(2, 2, 5, 5));
        s.execute(CMD, json!({"contract": 0})).unwrap();
        assert_eq!(channel_at(&s, 2, 2), 1.0);
        assert_eq!(channel_at(&s, 1, 2), 0.0);
    }

    #[test]
    fn bad_params_do_not_panic() {
        let mut s = crate::session();
        assert!(s.execute(CMD, json!({})).is_err());
        s.execute("file.new", json!({"width": 4, "height": 4, "background": "transparent"})).unwrap();
        for p in [
            json!({"contract": -1}),
            json!({"contract": 501}),
            json!({"contract": "1"}),
            json!({"solidity": 101}),
            json!({"solidity": -0.1}),
            json!({"solidity": false}),
            json!({"color": "red"}),
            json!({"color": 1}),
            json!({"color": [1, 2]}),
            json!({"name": ""}),
            json!({"name": 3}),
            json!({"applyAtCanvasBounds": "no"}),
            json!({"source": "both"}),
            json!({"source": 1}),
        ] {
            assert!(s.execute(CMD, p.clone()).is_err(), "should reject {p}");
        }
        assert!(s.active().unwrap().doc.channels.is_empty());
    }

    #[test]
    fn builtin_action_matches_the_command_defaults() {
        let prep = |s: &mut Session| {
            s.execute("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
            paint_rect(s, Rect::new(2, 2, 6, 6));
        };
        let mut via_cmd = crate::session();
        prep(&mut via_cmd);
        via_cmd.execute(CMD, json!({})).unwrap();
        let mut via_action = crate::session();
        prep(&mut via_action);
        ensure_preset(&mut via_action);
        assert_eq!(first(&via_action).set.as_deref(), Some(SET_NAME));
        assert_eq!(first(&via_action).steps.len(), 1);
        assert_eq!(first(&via_action).steps[0].command, CMD);
        via_action.execute("actions.play", json!({"action": ACTION_NAME})).unwrap();
        let a = &via_cmd.active().unwrap().doc.channels[0];
        let b = &via_action.active().unwrap().doc.channels[0];
        assert_eq!(a.name, b.name);
        assert_eq!(a.spot, b.spot);
        let area = via_cmd.active().unwrap().doc.bounds();
        assert_eq!(a.surface.to_interleaved(area), b.surface.to_interleaved(area));
    }

    #[test]
    fn psd_and_tiff_keep_the_spot_channel() {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
        paint_rect(&mut s, Rect::new(2, 2, 6, 6));
        s.execute(CMD, json!({})).unwrap();
        let doc = s.active().unwrap().doc.clone();
        let psd = photocraft_io::export(&doc, "spot.psd", &photocraft_io::ExportOptions::default()).unwrap();
        let back = photocraft_io::import("spot.psd", &psd.bytes).unwrap().document;
        let ch = &back.channels[0];
        assert_eq!(ch.name, DEFAULT_NAME);
        let (ink, sol) = ch.spot.expect("spot");
        assert_eq!(ink.to_rgb(), [1.0, 0.0, 0.0]);
        assert!((sol - 0.6).abs() < 1e-4);
        let area = doc.bounds();
        assert_eq!(ch.surface.to_interleaved(area), doc.channels[0].surface.to_interleaved(area));

        for (name, layers) in [("flat.tif", false), ("layered.tif", true)] {
            let opts = photocraft_io::ExportOptions { tiff_layers: layers, ..photocraft_io::ExportOptions::default() };
            let tiff = photocraft_io::export(&doc, name, &opts).unwrap();
            assert!(!tiff.warnings.iter().any(|w| w.contains("not written")), "{name}: {:?}", tiff.warnings);
            let back = photocraft_io::import(name, &tiff.bytes).unwrap().document;
            let ch = back
                .channels
                .iter()
                .find(|c| c.name == DEFAULT_NAME)
                .unwrap_or_else(|| panic!("{name}: channels {:?}", back.channels.iter().map(|c| &c.name).collect::<Vec<_>>()));
            let (ink, sol) = ch.spot.expect("spot");
            assert_eq!(ink.to_rgb(), [1.0, 0.0, 0.0], "{name}");
            assert!((sol - 0.6).abs() < 1e-4, "{name}");
            assert_eq!(ch.surface.to_interleaved(area), doc.channels[0].surface.to_interleaved(area), "{name}");
        }
    }

    #[test]
    fn composite_includes_every_visible_layer_and_skips_hidden_ones() {
        let prep = || {
            let mut s = crate::session();
            s.execute("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
            paint_rect(&mut s, Rect::new(1, 1, 4, 4));
            let bottom = s.active().unwrap().doc.layers[0].id;
            s.execute("layer.new.layer", json!({})).unwrap();
            paint_rect(&mut s, Rect::new(4, 4, 7, 7));
            (s, bottom)
        };
        let (mut layer, _) = prep();
        layer.execute(CMD, json!({"contract": 0, "source": "layer", "name": "Layer"})).unwrap();
        assert_eq!(channel_at(&layer, 1, 1), 0.0, "the active layer does not cover the lower layer");
        assert_eq!(channel_at(&layer, 4, 4), 1.0);
        assert_eq!(layer.execute(CMD, json!({})).unwrap()["source"], "layer");

        let (mut both, bottom) = prep();
        both.execute(CMD, json!({"contract": 0, "source": "composite", "name": "Both"})).unwrap();
        assert_eq!(channel_at(&both, 1, 1), 1.0, "the lower layer is part of the composite");
        assert_eq!(channel_at(&both, 5, 5), 1.0);
        assert_eq!(channel_at(&both, 0, 0), 0.0);

        both.execute("channel.delete", json!({"channel": "Both"})).unwrap();
        both.execute("layer.setProps", json!({"layer": bottom.0, "visible": false})).unwrap();
        both.execute(CMD, json!({"contract": 0, "source": "composite", "name": "Top"})).unwrap();
        assert_eq!(channel_at(&both, 1, 1), 0.0, "a hidden layer is not in the composite");
        assert_eq!(channel_at(&both, 4, 4), 1.0);
    }

    #[test]
    fn last_settings_prefill_the_dialog_and_the_action() {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 6, "height": 6, "background": "transparent"})).unwrap();
        paint_rect(&mut s, Rect::new(1, 1, 5, 5));
        ensure_preset(&mut s);
        s.execute(CMD, json!({"contract": 0, "name": "Varnish", "color": "#00ff00", "solidity": 40, "source": "composite", "applyAtCanvasBounds": true}))
            .unwrap();
        let saved = s.prefs().dialogs.get(CMD).expect("remembered");
        assert_eq!(saved["name"], "Varnish");
        assert_eq!(saved["source"], "composite");
        assert_eq!(saved["solidity"], json!(40.0));
        assert_eq!(saved["color"], "#00ff00");
        assert_eq!(saved["applyAtCanvasBounds"], json!(true));
        let fields = dialog_fields(Some(saved));
        assert_eq!(fields["name"], "Varnish");
        assert_eq!(fields["source"], "composite");
        assert_eq!(first(&s).steps[0].params["name"], "Varnish");
        assert_eq!(first(&s).steps[0].params["contract"], json!(0.0));

        s.execute("file.new", json!({"width": 6, "height": 6, "background": "transparent"})).unwrap();
        paint_rect(&mut s, Rect::new(0, 0, 6, 6));
        s.execute("actions.play", json!({"action": ACTION_NAME})).unwrap();
        let ch = s.active().unwrap().doc.channels.last().unwrap();
        assert_eq!(ch.name, "Varnish");
        assert!((ch.spot.unwrap().1 - 0.4).abs() < 1e-5);
        // Contract 0 on a full-canvas opaque layer keeps the edge.
        assert_eq!(channel_at(&s, 0, 0), 1.0);

        // The Actions panel stores a new step, then tells the dialog to follow it.
        s.actions_mut()
            .list
            .replace_step(
                0,
                0,
                ActionStep::new(
                    CMD,
                    json!({"contract": 2, "name": "Gloss", "source": "layer", "solidity": 60, "color": "#ff0000", "applyAtCanvasBounds": false}),
                ),
            )
            .unwrap();
        note_edited_step(&mut s, 0);
        assert_eq!(s.prefs().dialogs.get(CMD).unwrap()["name"], "Gloss");
        assert_eq!(first(&s).steps[0].params["contract"], json!(2));
    }

    #[test]
    fn a_saved_three_step_preset_becomes_one_dtf_step_in_the_set() {
        let mut s = crate::session();
        s.actions_mut()
            .list
            .push(Action::new(
                ACTION_NAME,
                vec![
                    ActionStep::new("select.loadSelection", json!({"channel": "transparency"})),
                    ActionStep::new("select.modify.contract", json!({"radius": 2, "applyAtCanvasBounds": true})),
                    ActionStep::new("channel.newSpot", json!({"name": "White", "color": "#ffffff", "solidity": 80, "fromSelection": true})),
                ],
            ))
            .unwrap();
        ensure_preset(&mut s);
        assert_eq!(s.actions().list.actions().len(), 1);
        assert_eq!(first(&s).set.as_deref(), Some(SET_NAME));
        assert_eq!(first(&s).steps.len(), 1);
        assert_eq!(first(&s).steps[0].command, CMD);
        assert_eq!(first(&s).steps[0].params["name"], "White");
        assert_eq!(first(&s).steps[0].params["contract"], json!(2));
        assert_eq!(first(&s).steps[0].params["applyAtCanvasBounds"], json!(true));
        // A customized action that is no longer those three commands is left in place.
        s.actions_mut().list.insert_step(0, 1, ActionStep::new("layer.new.layer", json!({}))).unwrap();
        s.actions_mut().list.assign_set(0, None).unwrap();
        ensure_preset(&mut s);
        assert_eq!(first(&s).steps.len(), 2);
        assert!(first(&s).set.is_none());
    }

    #[test]
    fn noncanonical_three_step_recordings_are_not_migrated() {
        let load_params = [
            json!({"channel": "transparency", "invert": true}),
            json!({"channel": "transparency", "invert": "false"}),
            json!({"channel": "transparency", "operation": "add"}),
            json!({"channel": "transparency", "layer": "merged"}),
            json!({"channel": "transparency", "document": 1}),
        ];
        for load in load_params {
            let mut s = crate::session();
            let action = Action::new(
                ACTION_NAME,
                vec![
                    ActionStep::new("select.loadSelection", load),
                    ActionStep::new("select.modify.contract", json!({"radius": 2})),
                    ActionStep::new("channel.newSpot", json!({"name": "White", "color": "#ffffff", "solidity": 80})),
                ],
            );
            s.actions_mut().list.push(action.clone()).unwrap();
            let result = s.execute("image.installDtfAction", json!({})).unwrap();
            assert_eq!(result["installed"], false);
            assert_eq!(first(&s), &action);
        }
    }

    #[test]
    fn legacy_recordings_with_different_spot_or_playback_settings_are_not_migrated() {
        let action = Action::new(
            ACTION_NAME,
            vec![
                ActionStep::new("select.loadSelection", json!({"channel": "transparency"})),
                ActionStep::new("select.modify.contract", json!({"radius": 2})),
                ActionStep::new("channel.newSpot", json!({"name": "White", "color": "#ffffff", "solidity": 80})),
            ],
        );
        assert!(legacy_params(&action).is_some());
        let mut different = action.clone();
        different.steps[2].params["fromSelection"] = json!(false);
        assert!(legacy_params(&different).is_none());
        let mut different = action.clone();
        different.steps[2].params["color"] = json!([1, 128, 0]);
        assert!(legacy_params(&different).is_none());
        let mut different = action.clone();
        different.disabled.insert(1);
        assert!(legacy_params(&different).is_none());
        let mut different = action;
        different.dialogs.insert(0);
        assert!(legacy_params(&different).is_none());
    }

    #[test]
    fn the_dtf_action_is_installed_only_when_asked() {
        let mut s = crate::session();
        assert!(s.actions().list.actions().iter().all(|a| a.name != ACTION_NAME), "startup does not install the action");
        let added = s.execute("image.installDtfAction", json!({})).unwrap();
        assert_eq!(added["installed"], json!(true));
        assert_eq!(s.actions().list.actions().len(), 1);
        assert_eq!(first(&s).set.as_deref(), Some(SET_NAME));
        let again = s.execute("image.installDtfAction", json!({})).unwrap();
        assert_eq!(again["installed"], json!(false), "a second install leaves the existing action");
        assert_eq!(s.actions().list.actions().len(), 1);
        let mut params = first(&s).steps[0].params.clone();
        params["contract"] = json!(4);
        s.actions_mut().list.replace_step(0, 0, ActionStep::new(CMD, params)).unwrap();
        s.execute("image.installDtfAction", json!({})).unwrap();
        assert_eq!(first(&s).steps[0].params["contract"], json!(4), "a customized action is not replaced");
    }
}
