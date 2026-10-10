//! LUT library commands (Color Lookup › Install Pack…): see [`crate::lut_library`].
//!
//! - `lut.installPack` copies the valid LUTs below a folder, or inside a `.zip`, into the library
//!   as one pack, leaving out LUTs already installed in another pack. It runs as a background job
//!   when started with [`Session::start`], cancellable between files.
//! - `lut.library` lists the installed packs and LUTs, favourites and recently used (read only;
//!   a LUT's `id` is what Color Lookup's `lut` param takes, which is how agents apply one; its
//!   absolute `location` is for trusted callers that use the `file` param).
//! - `lut.removePack` deletes a pack; `lut.favorite` stars or unstars a LUT; `lut.used` records
//!   one as recently used; `lut.rescan` notices files added or removed by hand.
//!
//! Without a library (headless, tests, the web build) `lut.library` lists nothing and the others
//! fail with an error that says so. The automation interface cannot run `lut.installPack` or
//! `lut.removePack`, because they change files outside the document roots.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::lut_library::{InstallOptions, InstallReport, LutLibrary};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn has_library(s: &Session) -> std::result::Result<(), String> {
    if s.lut_library.is_some() { Ok(()) } else { Err("the LUT library is not available in this session".into()) }
}

fn root_of(s: &Session, cmd: &str) -> Result<PathBuf> {
    s.lut_library.as_ref().map(|l| l.root().to_path_buf()).ok_or_else(|| bad(cmd, "the LUT library is not available in this session"))
}

fn library_mut<'a>(s: &'a mut Session, cmd: &str) -> Result<&'a mut LutLibrary> {
    s.lut_library.as_mut().ok_or_else(|| bad(cmd, "the LUT library is not available in this session"))
}

fn text(p: &Value, key: &str, cmd: &str) -> Result<Option<String>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().map(|s| Some(s.to_string())).ok_or_else(|| bad(cmd, format!("`{key}` must be a string"))),
    }
}

fn flag(p: &Value, key: &str, default: bool, cmd: &str) -> Result<bool> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| bad(cmd, format!("`{key}` must be true or false"))),
    }
}

fn install_pack(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lut.installPack";
    let root = root_of(s, cmd)?;
    let path = text(p, "path", cmd)?.filter(|v| !v.trim().is_empty()).ok_or_else(|| bad(cmd, "give `path`, the folder or .zip of LUTs to install"))?;
    let opts =
        InstallOptions { pack: text(p, "name", cmd)?, replace: flag(p, "replace", true, cmd)?, allow_duplicates: flag(p, "allowDuplicates", false, cmd)? };
    crate::jobs::run(
        s,
        "Install LUT Pack",
        false,
        move |ctx| {
            let lib = LutLibrary::new(root);
            lib.install_with(std::path::Path::new(&path), &opts, &mut |f, msg| {
                ctx.progress(f, msg);
                !ctx.cancelled()
            })
            .map_err(|e| if ctx.cancelled() { EngineError::Cancelled } else { EngineError::Other(e) })
        },
        |s, report: InstallReport| {
            if let Some(lib) = s.lut_library.as_mut() {
                lib.prune_state();
                lib.rev += 1;
            }
            Ok(json!({
                "pack": report.pack,
                "count": report.installed.len(),
                "installed": report.installed,
                "skipped": report.skipped.iter().map(|(f, why)| json!({"file": f, "reason": why})).collect::<Vec<_>>(),
                "duplicates": report.duplicates.iter().map(|(f, of)| json!({"file": f, "of": of})).collect::<Vec<_>>(),
                "replaced": report.replaced,
            }))
        },
    )
}

fn library(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lut.library";
    let only = text(p, "pack", cmd)?;
    let Some(lib) = s.lut_library.as_ref() else {
        return Ok(json!({"available": false, "packs": [], "favorites": [], "recent": []}));
    };
    let packs = lib.list().map_err(EngineError::Other)?;
    let packs: Vec<Value> = packs
        .iter()
        .filter(|pack| only.as_ref().is_none_or(|n| *n == pack.name))
        .map(|pack| {
            let luts: Vec<Value> = pack
                .luts
                .iter()
                .map(|l| {
                    let id = format!("{}/{}", pack.name, l.file);
                    json!({"id": id, "name": l.name, "file": l.file, "location": lib.path_of(&pack.name, &l.file).to_string_lossy(), "bytes": l.bytes, "favorite": lib.is_favorite(&id)})
                })
                .collect();
            json!({"name": pack.name, "count": luts.len(), "bytes": pack.bytes(), "luts": luts})
        })
        .collect();
    Ok(json!({"available": true, "dir": lib.root().to_string_lossy(), "rev": lib.rev, "packs": packs, "favorites": lib.favorites(), "recent": lib.recent()}))
}

fn remove_pack(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lut.removePack";
    let name = text(p, "name", cmd)?.ok_or_else(|| bad(cmd, "give `name`, the pack to remove"))?;
    library_mut(s, cmd)?.remove_pack(&name).map_err(|e| bad(cmd, e))?;
    Ok(json!({"removed": name}))
}

fn favorite(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lut.favorite";
    let id = text(p, "id", cmd)?.ok_or_else(|| bad(cmd, "give `id`, as listed by lut.library (Pack/path.cube)"))?;
    let lib = library_mut(s, cmd)?;
    let on = flag(p, "on", !lib.is_favorite(&id), cmd)?;
    lib.set_favorite(&id, on).map_err(|e| bad(cmd, e))?;
    Ok(json!({"id": id, "favorite": on}))
}

fn used(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lut.used";
    let id = text(p, "id", cmd)?.ok_or_else(|| bad(cmd, "give `id`, as listed by lut.library (Pack/path.cube)"))?;
    library_mut(s, cmd)?.note_used(&id).map_err(|e| bad(cmd, e))?;
    Ok(json!({"id": id}))
}

fn rescan(s: &mut Session, _: &Value) -> Result<Value> {
    let lib = library_mut(s, "lut.rescan")?;
    lib.prune_state();
    lib.rev += 1;
    Ok(json!({"rev": lib.rev}))
}

/// LUT library command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "lut.installPack",
            label: "Install LUT Pack…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":folder of .cube / .3dl / .look files (searched to 6 levels deep) or a .zip of them,"name":string?=folder name,"replace":bool=true (replace an installed pack of the same name),"allowDuplicates":bool=false (install LUTs whose content is already in another pack)} → {pack, count, installed, skipped:[{file, reason}], duplicates:[{file, of}], replaced}"##,
            enabled: has_library,
            run: install_pack,
            journal: false,
        },
        CommandSpec {
            id: "lut.library",
            label: "LUT Library",
            menu: &[],
            shortcut: None,
            params: r##"{"pack":string? (only this pack)} → {available, dir, rev, favorites:[id], recent:[id], packs:[{name, count, bytes, luts:[{id, name, file, location, bytes, favorite}]}]}; apply a LUT with Color Lookup's `lut` param set to its id (agents cannot use `file`); location is the file on disk, for trusted callers"##,
            enabled: always,
            run: library,
            journal: false,
        },
        CommandSpec {
            id: "lut.removePack",
            label: "Remove LUT Pack",
            menu: &[],
            shortcut: None,
            params: r##"{"name":installed pack name}"##,
            enabled: has_library,
            run: remove_pack,
            journal: false,
        },
        CommandSpec {
            id: "lut.favorite",
            label: "Favourite LUT",
            menu: &[],
            shortcut: None,
            params: r##"{"id":"Pack/Name.cube","on":bool? (default: toggle)}"##,
            enabled: has_library,
            run: favorite,
            journal: false,
        },
        CommandSpec {
            id: "lut.used",
            label: "Remember Recent LUT",
            menu: &[],
            shortcut: None,
            params: r##"{"id":"Pack/Name.cube" (as listed by lut.library)}"##,
            enabled: has_library,
            run: used,
            journal: false,
        },
        CommandSpec {
            id: "lut.rescan",
            label: "Rescan LUT Library",
            menu: &[],
            shortcut: None,
            params: r##"{} → {rev}: reread the library after files were added or removed by hand"##,
            enabled: has_library,
            run: rescan,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
