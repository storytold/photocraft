//! Tests that run paint commands, moved from the engine-core `preset_store` tests: 15 test(s).

/// Mirrors `preset_store::tip_file`: a tip's file name inside the store.
fn tip_file(hash: &str) -> String {
    format!("{}/{hash}.{}", photocraft_engine::preset_store::TIPS_DIR, photocraft_engine::preset_store::TIP_EXT)
}

use photocraft_engine::Session;
use photocraft_engine::preset_store::*;
use photocraft_paint::tile::PREVIEW_SIDE;
use photocraft_paint::{BrushPreset, BrushSettings, GrayTile, PresetCaptures, TipShape};

use photocraft_engine::preset_store::PresetStoreExt;

use craft_psd::abr::{AbrSample, LegacyBrush, LegacyTip, write_v12};

use serde_json::json;

use std::path::PathBuf;

/// A fresh, empty directory under the system temp dir (tests never write anywhere else).
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pc-preset-store-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        TempDir(d)
    }
    fn files(&self) -> Vec<String> {
        let mut v: Vec<String> = DirBackend::new(&self.0).list().unwrap().into_iter().map(|(n, _)| n).collect();
        v.sort();
        v
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A session with the store in `dir` attached.
fn session(dir: &TempDir) -> (Session, Vec<String>) {
    let mut s = Session::new();
    let w = s.attach_preset_store(open_dir(&dir.0));
    (s, w)
}

fn tip8(w: u32, h: u32, seed: u32) -> GrayTile {
    GrayTile::from_fn(w, h, |x, y| ((x * 7 + y * 13 + seed) % 256) as f32 / 255.0)
}

fn tip16(w: u32, h: u32, seed: u32) -> GrayTile {
    let data = (0..w * h).map(|i| (i.wrapping_mul(2654435761).wrapping_add(seed) >> 16) as u16).collect();
    GrayTile { width: w, height: h, data }
}

fn sampled(name: &str, group: &str, tip: GrayTile) -> BrushPreset {
    let brush = BrushSettings { size: 42.0, spacing: 0.1, tip: TipShape::Sampled(tip).into(), ..Default::default() };
    BrushPreset { name: name.into(), brush, builtin: false, group: group.into(), folder: Vec::new(), captures: Default::default() }
}

fn user(s: &Session) -> Vec<BrushPreset> {
    s.tools.presets.iter().filter(|p| !p.builtin).cloned().collect()
}

#[test]
fn empty_stores_preserve_shared_builtin_presets() {
    let first_dir = TempDir::new("shared-first");
    let second_dir = TempDir::new("shared-second");
    let (mut first, warnings) = session(&first_dir);
    assert!(warnings.is_empty(), "{warnings:?}");
    let (second, warnings) = session(&second_dir);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(std::sync::Arc::ptr_eq(&first.tools.presets, &second.tools.presets));
    first.execute("brush.presets.save", json!({"name": "Only first"})).unwrap();
    assert!(second.tools.presets.iter().all(|p| p.name != "Only first"));
    assert!(!std::sync::Arc::ptr_eq(&first.tools.presets, &second.tools.presets));
}

#[test]
fn saved_airbrush_group_keeps_legacy_and_explicit_rates_across_reopen() {
    let mem = MemBackend::default();
    let group = json!({"format": "photocraft-brush-group", "version": 1, "group": "Airbrush", "presets": [
        {"name": "Legacy Airbrush", "brush": {"buildUp": true}},
        {"name": "Custom Airbrush", "brush": {"buildUp": true, "buildUpRate": 35}},
        {"name": "Regular Brush", "brush": {}},
    ]});
    mem.write(&group_file_name("Airbrush"), &serde_json::to_vec(&group).unwrap()).unwrap();

    let mut s = Session::new();
    let warnings = s.attach_preset_store(open(Box::new(mem.clone())));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(s.tools.brush.build_up_rate, 10.0);
    for (name, build_up, rate) in [("Regular Brush", false, 10.0), ("Custom Airbrush", true, 35.0), ("Legacy Airbrush", true, 20.0)] {
        s.execute("tools.setBrush", json!({"preset": name})).unwrap();
        assert_eq!(s.tools.brush.build_up, build_up);
        assert_eq!(s.tools.brush.build_up_rate, rate);
    }

    s.execute("brush.presets.save", json!({"name": "Resaved Airbrush"})).unwrap();
    let mut reopened = Session::new();
    assert!(reopened.attach_preset_store(open(Box::new(mem))).is_empty());
    reopened.execute("tools.setBrush", json!({"preset": "Resaved Airbrush"})).unwrap();
    assert_eq!(reopened.tools.brush.build_up_rate, 20.0);
}

#[test]
fn sessions_have_no_store_by_default() {
    let mut s = Session::new();
    assert!(s.preset_store().is_none(), "headless sessions must stay hermetic");
    s.execute("brush.presets.save", json!({"name": "Mine"})).unwrap();
    assert!(s.preset_store().is_none());
}

#[test]
fn commands_persist_and_delete_removes_from_disk() {
    let dir = TempDir::new("commands");
    let (mut s, _) = session(&dir);
    s.tools.brush.tip = TipShape::Sampled(tip16(20, 20, 4)).into();
    s.execute("brush.presets.save", json!({"name": "Scribble"})).unwrap();
    assert_eq!(dir.files().iter().filter(|f| f.starts_with("tips/")).count(), 2, "the tip and its preview");
    // The tool's brush now is the saved preset's stored tip, still loaded.
    assert!(matches!(&*s.tools.brush.tip, TipShape::Stored(r) if r.is_loaded()));
    let (t, _) = session(&dir);
    let p = photocraft_paint::presets::find(&t.tools.presets, "Scribble").expect("saved preset reloads");
    assert_eq!(p.brush.tip, s.tools.brush.tip);

    // Rename through the Preset Manager, then delete.
    s.execute("edit.presets.presetManager", json!({"action": "rename", "kind": "brushes", "name": "Scribble", "newName": "Doodle"})).unwrap();
    let (t, _) = session(&dir);
    assert!(photocraft_paint::presets::find(&t.tools.presets, "Doodle").is_some());
    assert!(photocraft_paint::presets::find(&t.tools.presets, "Scribble").is_none());
    s.execute("brush.presets.delete", json!({"name": "Doodle"})).unwrap();
    assert_eq!(dir.files(), [INDEX_FILE], "the group file and its tip are gone");
    let (t, _) = session(&dir);
    assert!(photocraft_paint::presets::find(&t.tools.presets, "Doodle").is_none());
}

/// The New Brush Preset dialog's boxes (#4490) are saved with the preset and come back with it;
/// a preset saved without them reloads with the default captures and writes no field for them.
#[test]
fn preset_captures_persist() {
    let dir = TempDir::new("captures");
    let (mut s, _) = session(&dir);
    s.execute("brush.presets.save", json!({"name": "Bare", "captureSize": false, "toolSettings": false, "color": true})).unwrap();
    s.execute("brush.presets.save", json!({"name": "Plain"})).unwrap();
    let (t, _) = session(&dir);
    let find = |n: &str| photocraft_paint::presets::find(&t.tools.presets, n).expect("reloads").captures.clone();
    assert_eq!(find("Bare"), PresetCaptures { size: false, tool_settings: false, color: true });
    assert!(find("Plain").is_default());
    let written: String = dir.files().iter().filter(|f| !f.starts_with("tips/")).map(|f| std::fs::read_to_string(dir.0.join(f)).unwrap_or_default()).collect();
    assert_eq!(written.matches("\"captures\"").count(), 1, "only the preset with boxes records them");
}

#[test]
fn imported_abr_group_persists() {
    let dir = TempDir::new("abr");
    let (mut s, _) = session(&dir);
    let tip = AbrSample { id: String::new(), width: 6, height: 4, depth: 8, data: (0..24).map(|i| i * 10).collect() };
    let abr = write_v12(
        2,
        &[
            LegacyBrush { name: "Grit".into(), spacing: 25, anti_alias: true, tip: LegacyTip::Sampled(tip) },
            LegacyBrush {
                name: "Round 9".into(),
                spacing: 10,
                anti_alias: true,
                tip: LegacyTip::Computed { diameter: 9, hardness: 100, angle: 0, roundness: 100 },
            },
        ],
        true,
    )
    .unwrap();
    let data = photocraft_paint::tile::b64_encode(&abr);
    s.execute("brush.presets.importAbr", json!({"data": data, "group": "Legacy Set"})).unwrap();
    let (t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    let grp: Vec<&BrushPreset> = t.tools.presets.iter().filter(|p| p.group == "Legacy Set").collect();
    assert_eq!(grp.len(), 2);
    assert_eq!(user(&t), user(&s));
    // Deleting every preset of the group removes its file and tips.
    let names: Vec<String> = grp.iter().map(|p| p.name.clone()).collect();
    for n in names {
        s.execute("brush.presets.delete", json!({"name": n})).unwrap();
    }
    assert!(!dir.files().iter().any(|f| f.ends_with(".pcbrushes") || f.starts_with("tips/")), "{:?}", dir.files());
}

#[test]
fn deleted_builtins_stay_deleted_and_overrides_replace_them() {
    let dir = TempDir::new("builtins");
    let (mut s, _) = session(&dir);
    let first = s.tools.presets[0].name.clone();
    let second = s.tools.presets[1].name.clone();
    s.execute("brush.presets.delete", json!({"name": first})).unwrap();
    s.tools.brush.size = 77.0;
    s.execute("brush.presets.save", json!({"name": second})).unwrap();
    let (t, _) = session(&dir);
    assert!(photocraft_paint::presets::find(&t.tools.presets, &first).is_none());
    let p = photocraft_paint::presets::find(&t.tools.presets, &second).unwrap();
    assert_eq!((p.builtin, p.brush.size), (false, 77.0));
    assert_eq!(t.tools.presets.iter().filter(|p| p.name.eq_ignore_ascii_case(&second)).count(), 1);
    // No built-in is ever written.
    let n = dir.files().iter().filter(|f| f.ends_with(".pcbrushes")).count();
    assert_eq!(n, 1);
}

#[test]
fn reordering_presets_and_groups_persists() {
    let names = |s: &Session| s.tools.presets.iter().map(|p| (p.name.clone(), p.group.clone())).collect::<Vec<_>>();
    let dir = TempDir::new("reorder");
    let (mut s, _) = session(&dir);
    for (n, g) in [("Ink 1", "Inks"), ("Ink 2", "Inks"), ("Wash", "Washes")] {
        s.execute("brush.presets.save", json!({"name": n})).unwrap();
        std::sync::Arc::make_mut(&mut *s.tools.presets).iter_mut().find(|p| p.name == n).unwrap().group = g.into();
    }
    s.brush_presets_changed();
    s.sync_preset_store();
    // Untouched order: no order list is written (the default order is implied).
    let (t, _) = session(&dir);
    assert_eq!(names(&t), names(&s));
    // Reorder a built-in within its group, a user preset across groups, and whole groups.
    let builtin_group = s.tools.presets[0].group.clone();
    let last_builtin = s.tools.presets.iter().rfind(|p| p.builtin && p.group == builtin_group).unwrap().name.clone();
    let first = s.tools.presets[0].name.clone();
    s.execute("brush.presets.move", json!({"name": last_builtin, "before": first})).unwrap();
    s.execute("brush.presets.move", json!({"name": "Ink 2", "group": "Washes", "index": 0})).unwrap();
    s.execute("brush.presets.moveGroup", json!({"group": "Washes", "index": 0})).unwrap();
    s.execute("brush.presets.moveGroup", json!({"group": "Inks", "before": builtin_group})).unwrap();
    assert_eq!(s.tools.presets[0].name, "Ink 2");
    let (t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(names(&t), names(&s), "the panel order survives a restart");
    assert!(t.tools.presets.iter().find(|p| p.name == last_builtin).unwrap().builtin, "reordering keeps a built-in built-in");
    // Renaming and deleting a group persist too.
    s.execute("brush.presets.renameGroup", json!({"group": "Inks", "newName": "Pens"})).unwrap();
    s.execute("brush.presets.deleteGroup", json!({"group": "Washes"})).unwrap();
    let (t, _) = session(&dir);
    assert_eq!(names(&t), names(&s));
    assert!(t.tools.presets.iter().all(|p| p.group != "Washes" && p.group != "Inks"));
}

#[test]
fn write_failures_are_warnings() {
    let dir = TempDir::new("readonly");
    // The store "directory" is a file: every write fails.
    std::fs::write(&dir.0, b"not a dir").unwrap();
    let mut s = Session::new();
    s.attach_preset_store(open_dir(&dir.0));
    s.tools.brush.tip = TipShape::Sampled(tip8(4, 4, 0)).into();
    s.execute("brush.presets.save", json!({"name": "Nope"})).unwrap();
    let w = s.preset_store_mut().unwrap().take_warnings();
    assert!(!w.is_empty());
    let _ = std::fs::remove_file(&dir.0);
}

// ------------------------------------------------------------------ on-demand tips (#1843)

/// A session on an in-memory store holding `n` presets with distinct `side`-px 8-bit tips (one
/// group), synced; plus the backend to reopen it.
fn library(n: u32, side: u32) -> (Session, MemBackend) {
    let mem = MemBackend::default();
    let mut s = Session::new();
    s.attach_preset_store(open(Box::new(mem.clone())));
    for i in 0..n {
        std::sync::Arc::make_mut(&mut *s.tools.presets).push(sampled(&format!("Big {i}"), "Big Set", tip8(side, side, i)));
    }
    s.brush_presets_changed();
    s.sync_preset_store();
    (s, mem)
}

/// Bitmap samples a preset library holds (full tips, or previews of stored ones not loaded).
fn library_samples(s: &Session) -> usize {
    let b = |p: &BrushPreset| {
        [
            p.brush.tip.bitmap().map(|g| g.data.len()),
            p.brush.dual_brush.tip.bitmap().map(|g| g.data.len()),
            p.brush.texture.pattern.bitmap().map(|g| g.data.len()),
        ]
    };
    s.tools.presets.iter().flat_map(b).flatten().sum()
}

fn cache(s: &Session) -> TipCacheStats {
    s.preset_store().unwrap().tip_cache()
}

#[test]
fn reopening_a_library_decodes_no_tips_and_using_a_brush_decodes_only_its_own() {
    let (n, side) = (20u32, 512u32);
    let (s, mem) = library(n, side);
    // Once written, the imported presets keep previews only (built-ins keep their small tips).
    let budget = (n as usize + 40) * (PREVIEW_SIDE * PREVIEW_SIDE) as usize;
    assert!(library_samples(&s) < budget, "{}", library_samples(&s));
    assert_eq!(cache(&s).decodes, 0);
    let files = mem.list().unwrap();
    drop(s);

    // A restart: nothing is decoded at full size.
    let mut t = Session::new();
    let w = t.attach_preset_store(open(Box::new(mem.clone())));
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(mem.list().unwrap(), files, "attaching an unchanged store rewrites nothing");
    let user_presets = user(&t);
    assert_eq!(user_presets.len(), n as usize);
    for p in &user_presets {
        let TipShape::Stored(r) = &*p.brush.tip else { panic!("{} is not a stored tip", p.name) };
        assert!(!r.is_loaded() && (r.width, r.height) == (side, side));
        assert!(r.preview.width <= PREVIEW_SIDE && r.preview.height <= PREVIEW_SIDE);
    }
    assert!(library_samples(&t) < budget);
    assert_eq!(cache(&t), TipCacheStats { tips: 0, bytes: 0, decodes: 0, limit: TIP_CACHE_BYTES });

    // Using one brush decodes exactly its tip; painting and picking it again reuse it.
    t.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    t.execute("tools.setBrush", json!({"preset": "Big 3"})).unwrap();
    let TipShape::Stored(r) = &*t.tools.brush.tip else { panic!("the brush keeps its stored tip") };
    assert_eq!(r.full.as_deref(), Some(&tip8(side, side, 3)), "the tool paints with the full tip");
    assert_eq!(cache(&t).decodes, 1);
    t.execute("paint.stroke", json!({"points": [[10, 10], [50, 50]], "brush": {"size": 30}})).unwrap();
    t.execute("paint.stroke", json!({"points": [[10, 50], [50, 10]], "preset": "Big 3"})).unwrap();
    t.execute("tools.setBrush", json!({"preset": "Big 3"})).unwrap();
    assert_eq!((cache(&t).decodes, cache(&t).tips), (1, 1));
    // Picking a preset never loads the library's other tips, and the library stays light.
    assert!(user(&t).iter().all(|p| matches!(&*p.brush.tip, TipShape::Stored(r) if !r.is_loaded())));
    // The panel's "current brush" match still works on a stored tip.
    let picked = photocraft_paint::presets::find(&t.tools.presets, "Big 3").unwrap();
    assert_eq!(picked.brush.tip, t.tools.brush.tip);
}

#[test]
fn the_decoded_tip_cache_stays_within_its_budget() {
    let (s, mem) = library(6, 256);
    drop(s);
    let mut t = Session::new();
    t.attach_preset_store(open(Box::new(mem)));
    let one = 256 * 256 * 2;
    t.preset_store().unwrap().set_tip_cache_limit(one * 2 + 1);
    for i in 0..6 {
        t.execute("tools.setBrush", json!({"preset": format!("Big {i}")})).unwrap();
        let c = cache(&t);
        assert!(c.bytes <= c.limit && c.tips <= 2, "{c:?}");
    }
    assert_eq!(cache(&t).decodes, 6);
    // The two most recently used stay: picking them again decodes nothing.
    t.execute("tools.setBrush", json!({"preset": "Big 4"})).unwrap();
    t.execute("tools.setBrush", json!({"preset": "Big 5"})).unwrap();
    assert_eq!(cache(&t).decodes, 6);
    t.execute("tools.setBrush", json!({"preset": "Big 0"})).unwrap();
    assert_eq!(cache(&t).decodes, 7);
    // A budget below one tip still loads it (the brush in use holds its own reference).
    t.preset_store().unwrap().set_tip_cache_limit(0);
    assert_eq!(cache(&t).tips, 0);
    t.execute("tools.setBrush", json!({"preset": "Big 1"})).unwrap();
    assert!(matches!(&*t.tools.brush.tip, TipShape::Stored(r) if r.is_loaded()));
}

/// A stored tip the brush holds unloaded (its file failed to load at startup and is back) loads on the next brush
/// edit, and the brush keeps the loaded bitmap instead of the unloaded tip it compares equal to (#16252).
#[test]
fn a_brush_edit_keeps_a_stored_tip_that_loads_only_now() {
    let (s, mem) = library(1, 64);
    drop(s);
    let mut t = Session::new();
    t.attach_preset_store(open(Box::new(mem)));
    t.execute("tools.setBrush", json!({"preset": "Big 0"})).unwrap();
    if let TipShape::Stored(r) = &mut *t.tools.brush.tip {
        r.full = None;
    }
    assert!(matches!(&*t.tools.brush.tip, TipShape::Stored(r) if !r.is_loaded()), "the tip starts unloaded");
    t.execute("tools.setBrush", json!({"brush": {"size": 30}})).unwrap();
    assert!(matches!(&*t.tools.brush.tip, TipShape::Stored(r) if r.is_loaded()), "the edit keeps the tip it loaded");
    // An edit of a brush whose tip is loaded already keeps the tip and its revision.
    let revision = t.tools.brush.tip.revision();
    t.execute("tools.setBrush", json!({"brush": {"size": 40}})).unwrap();
    assert_eq!(t.tools.brush.tip.revision(), revision, "an unchanged loaded tip keeps its revision");
}

#[test]
fn missing_corrupt_or_unknown_tips_fail_when_used_without_panicking() {
    let (s, mem) = library(3, 64);
    drop(s);
    let mut t = Session::new();
    t.attach_preset_store(open(Box::new(mem.clone())));
    t.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    let key = |t: &Session, name: &str| match &*photocraft_paint::presets::find(&t.tools.presets, name).unwrap().brush.tip {
        TipShape::Stored(r) => r.key.clone(),
        other => panic!("{other:?}"),
    };
    // Deleted behind the store's back.
    let k0 = key(&t, "Big 0");
    mem.remove(&tip_file(&k0)).unwrap();
    t.execute("tools.setBrush", json!({"preset": "Hard Round"})).unwrap();
    let broken = std::sync::Arc::make_mut(&mut *t.tools.presets).iter_mut().find(|p| p.name == "Big 0").unwrap();
    broken.captures.color = true;
    broken.brush.color = [1.0, 0.0, 0.0, 1.0];
    t.tools.foreground = [0.0, 0.0, 1.0, 1.0];
    let before = (*t.tools.brush).clone();
    let preset_before = t.tools.current_preset.clone();
    let foreground_before = t.tools.foreground;
    let revision_before = t.tools.brush_rev;
    assert!(t.execute("tools.setBrush", json!({"preset": "Big 0"})).is_err());
    assert!(t.execute("paint.stroke", json!({"points": [[1, 1], [9, 9]], "preset": "Big 0"})).is_err());
    assert_eq!(*t.tools.brush, before);
    assert_eq!(t.tools.current_preset, preset_before);
    assert_eq!(t.tools.foreground, foreground_before);
    assert_eq!(t.tools.brush_rev, revision_before);
    // Corrupt, or a different bitmap under the same name.
    let k1 = key(&t, "Big 1");
    mem.write(&tip_file(&k1), b"PCTIP1\x08garbage").unwrap();
    assert!(t.execute("tools.setBrush", json!({"preset": "Big 1"})).is_err());
    let k2 = key(&t, "Big 2");
    mem.write(&tip_file(&k2), &encode_tip(&tip8(10, 10, 0))).unwrap();
    let e = t.execute("tools.setBrush", json!({"preset": "Big 2"})).unwrap_err().to_string();
    assert!(e.contains("expected 64×64"), "{e}");
    // Exporting a library whose tip is gone is an error too, not a file with a hole.
    assert!(t.execute("edit.presets.exportImportPresets", json!({"action": "export"})).is_err());
    // A stored reference the store doesn't know, or with no store at all.
    let stray = json!({"tip": {"stored": {"key": "../../etc/passwd", "width": 4, "height": 4,
        "preview": {"width": 1, "height": 1, "data": [1.0]}}}});
    assert!(t.execute("tools.setBrush", json!({"brush": stray})).is_err());
    let mut headless = Session::new();
    assert!(headless.execute("tools.setBrush", json!({"brush": stray})).is_err());
    assert!(headless.execute("brush.presets.list", json!({"full": true})).is_ok());
}

#[test]
fn a_group_file_that_failed_to_load_is_not_overwritten() {
    // e.g. written by a newer PhotoCraft: skipped on load, so a save must not replace it.
    let mem = MemBackend::default();
    let file = group_file_name("");
    let newer = br#"{"format":"photocraft-brush-group","version":2,"group":"","presets":[]}"#.to_vec();
    mem.files.lock().unwrap().insert(file.clone(), newer.clone());
    let mut s = Session::new();
    s.attach_preset_store(open(Box::new(mem.clone())));
    s.tools.brush.tip = TipShape::Sampled(tip8(4, 4, 0)).into();
    s.execute("brush.presets.save", json!({"name": "New"})).unwrap();
    assert_eq!(mem.files.lock().unwrap().get(&file), Some(&newer), "a file the store could not read was overwritten");
    assert!(!s.preset_store_mut().unwrap().take_warnings().is_empty());
}

/// #9317: a defined or imported pattern survives a restart, as in Photoshop; the built-in
/// patterns are never written, and deleting the last user pattern removes the file.
#[test]
fn user_patterns_survive_a_restart() {
    let dir = TempDir::new("patterns");
    let builtin = Session::new().patterns.items.len();
    let (mut s, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    s.execute("file.new", json!({"width": 12, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#1ea05a"})).unwrap();
    let id = s.execute("edit.definePattern", json!({"name": "Green Tile"})).unwrap()["pattern"].as_str().unwrap().to_string();
    s.sync_preset_store();
    assert!(dir.files().contains(&PATTERNS_FILE.to_string()), "{:?}", dir.files());
    // Only the user's pattern is in the file.
    let bytes = std::fs::read(dir.0.join(PATTERNS_FILE)).unwrap();
    let index: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let saved: Vec<photocraft_doc::Pattern> = index["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| photocraft_io::pattern_map::read_pat(&std::fs::read(dir.0.join(e["file"].as_str().unwrap())).unwrap()).unwrap().remove(0))
        .collect();
    assert_eq!(saved.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Green Tile"]);
    drop(s);

    let (mut t, w) = session(&dir);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(t.patterns.items.len(), builtin + 1);
    let back = t.patterns.items.iter().find(|p| p.id == id).expect("the defined pattern is back after a restart");
    assert_eq!((back.name.as_str(), back.width, back.height), ("Green Tile", 12, 10));
    assert_eq!(back.surface, saved[0].surface, "with its pixels");
    // Renames are kept; deleting the last user pattern removes the file.
    t.execute("pattern.rename", json!({"pattern": id, "name": "Moss"})).unwrap();
    t.sync_preset_store();
    let (u, _) = session(&dir);
    assert!(u.patterns.items.iter().any(|p| p.id == id && p.name == "Moss"));
    t.execute("pattern.delete", json!({"pattern": id})).unwrap();
    t.sync_preset_store();
    assert!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(dir.0.join(PATTERNS_FILE)).unwrap()).unwrap()["patterns"].as_array().unwrap().is_empty(),
        "empty authoritative index prevents resurrection"
    );
}
