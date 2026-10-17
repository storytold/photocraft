//! Independently authored starter actions, composed from PhotoCraft commands. The preset
//! store seeds these once; an explicit command restores missing names without replacing edits.

use serde_json::{Value, json};

use crate::actions_cmds::{Action, ActionState};
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

pub(crate) const VERSION: u32 = 1;
const SET: &str = "PhotoCraft Essentials";

fn defaults() -> Vec<Action> {
    [
        ("Auto Tone", "image.autoTone", json!({})),
        ("Auto Contrast", "image.autoContrast", json!({})),
        ("Auto Color", "image.autoColor", json!({})),
        ("Select Subject", "select.subject", json!({"sampleAllLayers": true, "mode": "replace"})),
        ("Blur Background", "layer.blurBackground", json!({"radius": 8.0})),
    ]
    .into_iter()
    .map(|(name, id, params)| Action { set: Some(SET.into()), ..Action::new(name, vec![(id.into(), params)]) })
    .collect()
}

/// Preserve custom actions, including a user's edited default with the same name.
pub(crate) fn append_missing(state: &mut ActionState) -> Vec<String> {
    let mut added = Vec::new();
    for action in defaults() {
        if !state.list.iter().any(|have| have.name == action.name) {
            added.push(action.name.clone());
            state.list.push(action);
        }
    }
    if !added.is_empty() {
        state.touch();
    }
    added
}

fn enabled(s: &Session) -> std::result::Result<(), String> {
    if s.actions.recording.is_some() || s.actions.playing > 0 {
        return Err("Stop recording or playing an action before loading defaults".into());
    }
    Ok(())
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |msg: String| EngineError::BadParams { cmd: "actions.loadDefaults".into(), msg };
    enabled(s).map_err(bad)?;
    if !p.is_object() {
        return Err(bad("parameters must be an object".into()));
    }
    Ok(json!({"added": append_missing(&mut s.actions), "set": SET}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "actions.loadDefaults",
        label: "Load Default Actions",
        menu: &[],
        shortcut: None,
        params: "{} → {added:[name…], set}. Append missing PhotoCraft Essentials actions; existing names and edits are preserved. No document needed. Unavailable during recording or playback.",
        enabled,
        run: load,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions_cmds::ActionsFile;
    use crate::preset_store::{ACTIONS_FILE, MemBackend, PresetBackend, open};
    use photocraft_geom::Rect;

    fn attached(mem: &MemBackend) -> Session {
        let mut s = Session::new();
        assert!(s.attach_preset_store(open(Box::new(mem.clone()))).is_empty());
        s
    }

    fn saved(mem: &MemBackend) -> ActionsFile {
        serde_json::from_slice(&mem.read(ACTIONS_FILE, 1 << 20).unwrap()).unwrap()
    }

    #[test]
    fn load_defaults_preserves_names_and_edits_and_is_idempotent() {
        let mut s = Session::new();
        assert!(s.actions.list.is_empty(), "headless sessions remain hermetic");
        let custom = Action::new("Auto Tone", vec![("image.flipCanvasHorizontal".into(), json!({}))]);
        s.actions.list.push(custom.clone());
        let loaded = s.execute("actions.loadDefaults", json!({})).unwrap();
        assert_eq!(loaded["added"], json!(["Auto Contrast", "Auto Color", "Select Subject", "Blur Background"]));
        assert_eq!(s.actions.list[0], custom);
        assert!(s.actions.list.iter().skip(1).all(|a| a.set.as_deref() == Some(SET)));
        let rev = s.actions.rev;
        assert_eq!(s.execute("actions.loadDefaults", json!({})).unwrap()["added"], json!([]));
        assert_eq!(s.actions.rev, rev, "unchanged defaults do not cause a store write");
        assert!(s.journal.is_empty());
        assert!(s.active().is_none());

        for bad in [Value::Null, json!([]), json!("defaults"), json!(true), json!(1)] {
            assert!(s.execute("actions.loadDefaults", bad).is_err());
        }
        s.execute("actions.record", json!({"name": "Recording"})).unwrap();
        assert!(!s.is_enabled("actions.loadDefaults"));
        assert!(s.execute("actions.loadDefaults", json!({})).is_err());
        s.execute("actions.stop", json!({})).unwrap();
        s.actions.playing = 1;
        assert!(load(&mut s, &json!({})).is_err());
    }

    #[test]
    fn defaults_seed_once_and_restore_only_when_requested() {
        let mem = MemBackend::default();
        let mut s = attached(&mem);
        assert_eq!(s.actions.list, defaults());
        assert_eq!(saved(&mem).defaults_version, VERSION);
        s.execute("actions.delete", json!({"action": "Auto Tone"})).unwrap();
        s.execute("actions.rename", json!({"action": "Auto Color", "name": "My Color"})).unwrap();
        let customized = s.actions.list.clone();
        let mut restarted = attached(&mem);
        assert_eq!(restarted.actions.list, customized, "deleted and renamed defaults stay customized on restart");
        assert_eq!(restarted.execute("actions.loadDefaults", json!({})).unwrap()["added"], json!(["Auto Tone", "Auto Color"]));
        assert_eq!(attached(&mem).actions.list, restarted.actions.list);
    }

    #[test]
    fn old_store_and_early_actions_win_over_defaults() {
        let mem = MemBackend::default();
        let custom = Action::new("Auto Tone", vec![("select.all".into(), json!({}))]);
        let disk = Action::new("My Workflow", vec![]);
        // Old files have no defaults_version field.
        mem.write(ACTIONS_FILE, &serde_json::to_vec(&json!({"version": 1, "actions": [custom, disk]})).unwrap()).unwrap();
        let mut s = Session::new();
        let early = Action::new("Auto Color", vec![("select.deselect".into(), json!({}))]);
        s.actions.list.push(early.clone());
        s.attach_preset_store(open(Box::new(mem.clone())));
        assert_eq!(&s.actions.list[..3], &[early, custom, disk]);
        assert_eq!(
            s.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["Auto Color", "Auto Tone", "My Workflow", "Auto Contrast", "Select Subject", "Blur Background"]
        );
        assert_eq!(saved(&mem).defaults_version, VERSION);
        assert_eq!(attached(&mem).actions.list, s.actions.list);
    }

    #[test]
    fn unreadable_actions_are_not_seeded_or_overwritten() {
        for bytes in [b"{".as_slice(), br#"{"version":999,"actions":[]}"#.as_slice()] {
            let mem = MemBackend::default();
            mem.write(ACTIONS_FILE, bytes).unwrap();
            let mut s = Session::new();
            assert!(!s.attach_preset_store(open(Box::new(mem.clone()))).is_empty());
            assert!(s.actions.list.is_empty());
            s.execute("actions.loadDefaults", json!({})).unwrap();
            assert_eq!(s.actions.list, defaults(), "explicit recovery is available in memory");
            assert_eq!(mem.read(ACTIONS_FILE, 1 << 20).unwrap(), bytes, "the unreadable original remains recoverable");
        }
    }

    #[test]
    fn auto_defaults_play_on_pixels_and_undo_at_every_depth() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("actions.loadDefaults", json!({})).unwrap();
            s.execute("file.new", json!({"width": 64, "height": 64, "depth": depth})).unwrap();
            s.edit("ramp", |doc, _| {
                let surf = doc.layers[0].surface_mut().unwrap();
                for x in 0..64 {
                    let v = 0.3 + 0.4 * x as f32 / 63.0;
                    surf.fill_rect(Rect::new(x, 0, x + 1, 64), &[v, v * 0.9, v * 0.8, 1.0]);
                }
                Ok(())
            })
            .unwrap();
            let pixel = |s: &Session, x| s.active().unwrap().doc.layers[0].surface().unwrap().pixel(x, 0);
            let before = (pixel(&s, 0), pixel(&s, 63));
            let history = s.active().unwrap().history.past_len();
            for name in ["Auto Tone", "Auto Contrast", "Auto Color"] {
                let played = s.execute("actions.play", json!({"action": name})).unwrap();
                assert_eq!(played["ran"], 1, "{name} at {depth}: {played}");
                assert!(played.get("failed").is_none());
                let (a, b) = (pixel(&s, 0), pixel(&s, 63));
                let (lo, hi) = if name == "Auto Contrast" { (a[2], b[0]) } else { (a[0], b[0]) };
                assert!(lo < 0.05 && hi > 0.95, "{name} at {depth}: {a:?} {b:?}");
                assert_eq!(s.active().unwrap().history.past_len(), history + 1);
                assert!(s.undo());
                assert_eq!((pixel(&s, 0), pixel(&s, 63)), before);
            }
        }
    }

    #[test]
    fn subject_default_samples_visible_layers_and_undo_restores_selection() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("actions.loadDefaults", json!({})).unwrap();
            s.execute("file.new", json!({"width": 200, "height": 160, "depth": depth})).unwrap();
            s.edit("disc", |doc, _| {
                let surf = doc.layers[0].surface_mut().unwrap();
                surf.fill_rect(Rect::new(0, 0, 200, 160), &[0.25, 0.45, 0.7, 1.0]);
                for y in 0..160 {
                    for x in 0..200 {
                        if (x - 100) * (x - 100) + (y - 80) * (y - 80) <= 45 * 45 {
                            surf.fill_rect(Rect::new(x, y, x + 1, y + 1), &[0.9, 0.3, 0.15, 1.0]);
                        }
                    }
                }
                Ok(())
            })
            .unwrap();
            s.execute("layer.new.layer", json!({"name": "Empty active layer"})).unwrap();
            s.execute("select.all", json!({})).unwrap();
            let played = s.execute("actions.play", json!({"action": "Select Subject"})).unwrap();
            assert_eq!(played["ran"], 1, "{depth}: {played}");
            assert!(played.get("failed").is_none());
            let coverage = |s: &Session, x, y| s.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(x, y, 0);
            assert!(coverage(&s, 100, 80) > 0.9, "the subject on the underlying layer is selected");
            assert!(coverage(&s, 0, 0) < 0.1, "replace mode removes the old background selection");
            assert!(s.undo());
            assert!(coverage(&s, 0, 0) > 0.9);
        }
    }
}
