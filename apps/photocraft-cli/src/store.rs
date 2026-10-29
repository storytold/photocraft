//! Storage backends on the command line (wordcraft#518): the `store-*` subcommands, and the
//! staging that lets `convert`, `info`, `run` and `batch` take storage paths. A storage path is
//! downloaded to a scratch folder, processed as a local file, and the result uploaded.

use std::io::Write;
use std::path::{Path, PathBuf};

use photocraft_storage::{StorageBackend, WriteOptions};
use serde_json::{Value, json};

use crate::{
    CliError, Result,
    args::{StoreArgs, StoreListArgs, StoreOnlyArgs, StorePathArgs, StoreReadArgs, StoreWriteArgs, StoreWriteFlags},
    output::{JsonStyle, print_json},
};

fn store_err(e: impl std::fmt::Display) -> CliError {
    CliError::Store(e.to_string())
}

/// The backend the storage flags name. The token comes from `--store-token`, else
/// `$CRAFT_DAV_TOKEN`, so it need not be on the command line; it never combines with Basic auth.
pub(crate) fn open_backend(cmd: &str, a: &StoreArgs) -> Result<Box<dyn StorageBackend>> {
    // Without --store, a --store-url names a WebDAV collection; otherwise it's a local folder.
    let default_kind = if a.store_url.is_some() { "webdav" } else { "local" };
    match a.store.as_deref().unwrap_or(default_kind) {
        "local" => {
            let root = a.store_root.as_deref().ok_or_else(|| store_err(format!("{cmd}: --store local needs --store-root DIR")))?;
            if a.store_url.is_some() {
                return Err(store_err(format!("{cmd}: --store-url is for --store webdav, not local")));
            }
            photocraft_storage::local::LocalBackend::open(root).map(|b| Box::new(b) as Box<dyn StorageBackend>).map_err(|e| store_err(format!("{cmd}: {e}")))
        }
        _ => {
            let url = a.store_url.as_deref().ok_or_else(|| store_err(format!("{cmd}: --store webdav needs --store-url URL")))?;
            if a.store_root.is_some() {
                return Err(store_err(format!("{cmd}: --store-root is for --store local, not webdav")));
            }
            let token = a.store_token.clone().or_else(|| std::env::var("CRAFT_DAV_TOKEN").ok().filter(|t| !t.is_empty()));
            let backend = match (token, a.store_user.as_deref(), a.store_password.as_deref()) {
                (Some(t), None, None) => photocraft_storage::webdav::WebDavBackend::with_bearer(url, &t),
                (None, user, password) => photocraft_storage::webdav::WebDavBackend::new(url, user, password),
                _ => {
                    return Err(store_err(format!("{cmd}: --store-token cannot be combined with --store-user/--store-password (pick one auth scheme)")));
                }
            };
            // Credentials go only to a loopback host over plain HTTP unless the network is trusted.
            backend
                .map(|b| if a.store_insecure_http { b.allow_cleartext_credentials() } else { b })
                .map(|b| Box::new(b) as Box<dyn StorageBackend>)
                .map_err(|e| store_err(format!("{cmd}: {e}")))
        }
    }
}

pub(crate) fn write_options(w: &StoreWriteFlags) -> WriteOptions {
    WriteOptions { if_match: w.etag.clone(), atomic: w.atomic }
}

/// `--etag` and `--atomic` mean nothing without a backend to upload to: refuse rather than ignore.
pub(crate) fn check_write_flags(store: &StoreArgs, w: &StoreWriteFlags) -> Result {
    if !store.requested() && (w.etag.is_some() || w.atomic) {
        return Err(CliError::WriteFlagsWithoutStore);
    }
    Ok(())
}

/// A storage path as given on the command line: storage keys are text.
pub(crate) fn key(p: &Path) -> Result<&str> {
    p.to_str().ok_or_else(|| store_err(format!("{}: a storage path must be valid UTF-8", p.display())))
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or("doc")
}

/// A backend plus a scratch folder that storage paths are staged through. The folder is removed
/// when this is dropped.
pub(crate) struct Staging {
    pub(crate) backend: Box<dyn StorageBackend>,
    dir: PathBuf,
}

impl Staging {
    /// `None` when no storage flag was given: the command works on local files.
    pub(crate) fn open(cmd: &str, a: &StoreArgs) -> Result<Option<Staging>> {
        if !a.requested() {
            return Ok(None);
        }
        let backend = open_backend(cmd, a)?;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("photocraft-cli-store-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(Some(Staging { backend, dir }))
    }

    /// Download `path` into the scratch folder `sub`, keeping its file name (and so its extension,
    /// which picks the decoder).
    pub(crate) fn fetch(&self, path: &str, sub: &str) -> Result<PathBuf> {
        let bytes = self.backend.read(path).map_err(|e| store_err(format!("{path}: {e}")))?;
        let d = self.dir.join(sub);
        std::fs::create_dir_all(&d)?;
        let local = d.join(basename(path));
        std::fs::write(&local, &bytes)?;
        Ok(local)
    }

    /// Where to save locally the result that goes to `path`: `name` in the scratch `out` folder,
    /// with `path`'s file name unless `name` is given.
    pub(crate) fn local_out(&self, path: &str, name: Option<&str>) -> Result<PathBuf> {
        let d = self.dir.join("out");
        std::fs::create_dir_all(&d)?;
        Ok(d.join(name.unwrap_or_else(|| basename(path))))
    }

    pub(crate) fn deposit(&self, local: &Path, path: &str, opts: &WriteOptions) -> Result {
        let bytes = std::fs::read(local)?;
        self.backend.write(path, &bytes, opts).map_err(|e| store_err(format!("{path}: {e}")))?;
        Ok(())
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Whether a storage path names a file `batch` can open, by its extension.
pub(crate) fn is_input_key(path: &str) -> bool {
    Path::new(basename(path)).extension().is_some_and(|e| crate::batch::known_extension(&e.to_string_lossy().to_ascii_lowercase()))
}

pub(crate) fn write(a: &StoreWriteArgs, out: &mut dyn Write, err: &mut dyn Write) -> Result {
    let backend = open_backend("store-write", &a.store)?;
    let bytes = std::fs::read(&a.file).map_err(|source| CliError::ReadDir { path: a.file.clone(), source })?;
    let meta = backend.write(&a.path, &bytes, &write_options(&a.write)).map_err(|e| store_err(format!("{}: {e}", a.path)))?;
    print_json(out, &json!({"path": a.path, "size": meta.size, "etag": meta.etag}), JsonStyle::Pretty)?;
    let _ = writeln!(err, "wrote {} ({} bytes)", a.path, bytes.len());
    Ok(())
}

pub(crate) fn read(a: &StoreReadArgs, err: &mut dyn Write) -> Result {
    let backend = open_backend("store-read", &a.store)?;
    let bytes = backend.read(&a.path).map_err(|e| store_err(format!("{}: {e}", a.path)))?;
    std::fs::write(&a.out, &bytes)?;
    let _ = writeln!(err, "wrote {} ({} bytes)", a.out.display(), bytes.len());
    Ok(())
}

pub(crate) fn stat(a: &StorePathArgs, out: &mut dyn Write) -> Result {
    let backend = open_backend("store-stat", &a.store)?;
    let meta = backend.stat(&a.path).map_err(|e| store_err(format!("{}: {e}", a.path)))?;
    print_json(out, &json!({"path": a.path, "size": meta.size, "etag": meta.etag, "modified": meta.modified, "isDir": meta.is_dir}), JsonStyle::Pretty)
}

pub(crate) fn list(a: &StoreListArgs, out: &mut dyn Write) -> Result {
    let backend = open_backend("store-list", &a.store)?;
    let prefix = a.prefix.clone().unwrap_or_default();
    let entries = backend.list(&prefix).map_err(|e| store_err(format!("{}: {e}", if prefix.is_empty() { "(root)" } else { &prefix })))?;
    let list: Vec<Value> = entries.iter().map(|e| json!({"path": e.path, "size": e.meta.size, "etag": e.meta.etag, "isDir": e.meta.is_dir})).collect();
    print_json(out, &Value::Array(list), JsonStyle::Pretty)
}

pub(crate) fn remove(a: &StorePathArgs, err: &mut dyn Write) -> Result {
    let backend = open_backend("store-remove", &a.store)?;
    backend.remove(&a.path).map_err(|e| store_err(format!("{}: {e}", a.path)))?;
    let _ = writeln!(err, "removed {}", a.path);
    Ok(())
}

pub(crate) fn probe(a: &StoreOnlyArgs, out: &mut dyn Write) -> Result {
    let backend = open_backend("store-probe", &a.store)?;
    let probe = backend.probe().map_err(store_err)?;
    // What the server said, not what the backend assumed before asking.
    let caps = &probe.capabilities;
    print_json(
        out,
        &json!({
            "atomicWrite": caps.atomic_write, "conditionalWrite": caps.conditional_write, "list": caps.list,
            "etag": caps.etag, "persistent": caps.persistent,
            "davClasses": probe.dav_classes, "notes": probe.notes,
        }),
        JsonStyle::Pretty,
    )
}

/// Finish or drop interrupted writes; safe to run at every start.
pub(crate) fn recover(a: &StoreOnlyArgs, out: &mut dyn Write) -> Result {
    let backend = open_backend("store-recover", &a.store)?;
    let report = backend.recover().map_err(store_err)?;
    print_json(out, &json!({"completed": report.completed, "dropped": report.dropped, "removedTemp": report.removed_temp}), JsonStyle::Pretty)
}

#[cfg(test)]
mod store_cli_tests {
    use super::*;
    use crate::test_support::{Invoke, invoke};

    fn scratch(name: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("photocraft-cli-store-test-{name}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `store-*` subcommands round-trip bytes through a local backend, with the same JSON shapes
    /// as the other craft apps (`path`/`size`/`etag`, `isDir`, probe capabilities, recover counts).
    #[test]
    fn store_commands_round_trip_local() {
        let dir = scratch("cmds");
        let root = dir.join("store").to_string_lossy().to_string();
        let src = dir.join("hello.bin");
        std::fs::write(&src, b"8BPS-stage").unwrap();
        let src = src.to_string_lossy().to_string();
        let s = ["--store", "local", "--store-root", root.as_str()];
        let with = |args: &[&str]| -> Vec<String> { args.iter().chain(s.iter()).map(|a| a.to_string()).collect() };
        let call = |args: &[&str]| {
            let v = with(args);
            invoke(&v.iter().map(String::as_str).collect::<Vec<_>>())
        };

        let Invoke { code, stdout, stderr } = call(&["store-write", "docs/memo.psd", &src]);
        assert_eq!(code, 0, "{stderr}");
        let written: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!((written["path"].as_str(), written["size"].as_u64()), (Some("docs/memo.psd"), Some(10)));
        assert!(written["etag"].as_str().is_some_and(|t| !t.is_empty()));

        let Invoke { code, stdout, stderr } = call(&["store-stat", "docs/memo.psd"]);
        assert_eq!(code, 0, "{stderr}");
        let stat: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(stat["size"].as_u64(), Some(10));
        assert_eq!(stat["etag"], written["etag"]);
        assert_eq!(stat["isDir"], json!(false));

        let Invoke { code, stdout, stderr } = call(&["store-list", "docs"]);
        assert_eq!(code, 0, "{stderr}");
        let list: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(list.as_array().map(Vec::len), Some(1));
        assert_eq!(list[0]["path"].as_str(), Some("docs/memo.psd"));

        let dst = dir.join("back.bin").to_string_lossy().to_string();
        let Invoke { code, stderr, .. } = call(&["store-read", "docs/memo.psd", &dst]);
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(std::fs::read(&dst).unwrap(), b"8BPS-stage");

        let Invoke { code, stdout, stderr } = call(&["store-probe"]);
        assert_eq!(code, 0, "{stderr}");
        let probe: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(probe["atomicWrite"], json!(true));

        let Invoke { code, stdout, stderr } = call(&["store-recover"]);
        assert_eq!(code, 0, "{stderr}");
        let recover: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(recover["removedTemp"], json!(0));

        let Invoke { code, stderr, .. } = call(&["store-remove", "docs/memo.psd"]);
        assert_eq!(code, 0, "{stderr}");
        // Removing again succeeds (idempotent).
        let Invoke { code, stderr, .. } = call(&["store-remove", "docs/memo.psd"]);
        assert_eq!(code, 0, "{stderr}");
        let Invoke { code, stderr, .. } = call(&["store-stat", "docs/memo.psd"]);
        assert_eq!(code, 1, "removed file stats as missing: {stderr}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Conditional writes: a stale `--etag` loses, a fresh one wins.
    #[test]
    fn store_write_etag_guards_replacement() {
        let dir = scratch("etag");
        let root = dir.join("store").to_string_lossy().to_string();
        let src = |tag: &str, data: &[u8]| {
            let p = dir.join(format!("v-{tag}.bin"));
            std::fs::write(&p, data).unwrap();
            p.to_string_lossy().to_string()
        };
        let (v1, v2) = (src("a", b"one"), src("b", b"two"));
        let Invoke { code, stdout, stderr } = invoke(&["store-write", "f.psd", &v1, "--store", "local", "--store-root", &root]);
        assert_eq!(code, 0, "{stderr}");
        let tag = serde_json::from_str::<Value>(&stdout).unwrap()["etag"].as_str().unwrap().to_string();

        let Invoke { code, stderr, .. } = invoke(&["store-write", "f.psd", &v2, "--store", "local", "--store-root", &root, "--etag", "stale"]);
        assert_eq!(code, 1, "{stderr}");
        let Invoke { code, stderr, .. } = invoke(&["store-write", "f.psd", &v2, "--store", "local", "--store-root", &root, "--etag", &tag, "--atomic"]);
        assert_eq!(code, 0, "{stderr}");
        let Invoke { code, stdout, .. } = invoke(&["store-list", "--store", "local", "--store-root", &root]);
        assert_eq!(code, 0);
        assert!(stdout.contains("f.psd"), "{stdout}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `convert`, `info` and `run` take storage paths in and out when given the storage flags.
    #[test]
    fn convert_info_run_compose_with_local_store() {
        let dir = scratch("compose");
        let root = dir.join("store").to_string_lossy().to_string();
        let seed = dir.join("seed.png").to_string_lossy().to_string();
        let Invoke { code, stderr, .. } = invoke(&["run", "--new", r#"{"width":16,"height":12}"#, "--out", &seed]);
        assert_eq!(code, 0, "{stderr}");
        let Invoke { code, stderr, .. } = invoke(&["store-write", "in/seed.png", &seed, "--store", "local", "--store-root", &root]);
        assert_eq!(code, 0, "{stderr}");

        let Invoke { code, stderr, .. } = invoke(&["convert", "in/seed.png", "out/back.png", "--store", "local", "--store-root", &root]);
        assert_eq!(code, 0, "{stderr}");
        // The same storage path in and out is refused without --in-place, as for local files.
        let Invoke { code, stderr, .. } = invoke(&["convert", "in/seed.png", "in/seed.png", "--store", "local", "--store-root", &root]);
        assert_eq!(code, 1, "{stderr}");

        let Invoke { code, stdout, stderr } = invoke(&["info", "out/back.png", "--compact", "--store", "local", "--store-root", &root]);
        assert_eq!(code, 0, "{stderr}");
        let info: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!((info["width"].as_u64(), info["height"].as_u64()), (Some(16), Some(12)));
        assert_eq!(info["file"].as_str(), Some("out/back.png"));

        let Invoke { code, stdout, stderr } =
            invoke(&["run", "in/seed.png", "--cmd", "image.imageRotation.90cw", "--out", "out/rot.png", "--store", "local", "--store-root", &root]);
        assert_eq!(code, 0, "{stdout}{stderr}");
        let Invoke { code, stdout, stderr } = invoke(&["info", "out/rot.png", "--compact", "--store", "local", "--store-root", &root]);
        assert_eq!(code, 0, "{stderr}");
        let info: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!((info["width"].as_u64(), info["height"].as_u64()), (Some(12), Some(16)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `batch` with the storage flags treats `--in`/`--out` as storage folders.
    #[test]
    fn batch_composes_with_local_store() {
        let dir = scratch("batch");
        let root = dir.join("store").to_string_lossy().to_string();
        let seed = dir.join("a.png").to_string_lossy().to_string();
        let Invoke { code, stderr, .. } = invoke(&["run", "--new", r#"{"width":8,"height":6}"#, "--out", &seed]);
        assert_eq!(code, 0, "{stderr}");
        let Invoke { code, stderr, .. } = invoke(&["store-write", "bin/a.png", &seed, "--store", "local", "--store-root", &root]);
        assert_eq!(code, 0, "{stderr}");
        let actions = dir.join("actions.json");
        std::fs::write(&actions, r#"[["image.imageRotation.90cw", {}]]"#).unwrap();
        let actions = actions.to_string_lossy().to_string();

        let Invoke { code, stdout, stderr } =
            invoke(&["batch", "--actions", &actions, "--in", "bin", "--out", "bout", "--store", "local", "--store-root", &root]);
        assert_eq!(code, 0, "{stdout}{stderr}");
        assert!(stdout.contains("1 succeeded"), "{stdout}");

        let Invoke { code, stdout, stderr } = invoke(&["store-list", "bout", "--store", "local", "--store-root", &root]);
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.contains("bout/a.png"), "{stdout}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Flag misuse fails before any network I/O (every host here is unreachable).
    #[test]
    fn store_flag_misuse_is_an_error() {
        let dir = scratch("misuse");
        let root = dir.join("store").to_string_lossy().to_string();

        // An unknown backend is a usage error from clap.
        let Invoke { code, stderr, .. } = invoke(&["store-probe", "--store", "vault"]);
        assert_eq!(code, 2, "{stderr}");
        assert!(stderr.contains("vault"), "{stderr}");

        let Invoke { code, stderr, .. } = invoke(&["store-probe"]);
        assert_eq!(code, 1, "{stderr}");
        assert!(stderr.contains("--store-root"), "{stderr}");

        let Invoke { code, stderr, .. } = invoke(&["store-probe", "--store", "webdav"]);
        assert_eq!(code, 1, "{stderr}");
        assert!(stderr.contains("--store-url"), "{stderr}");

        // A --store-url alone picks WebDAV: it fails at the (absent) server, not for want of a root.
        let Invoke { code, stderr, .. } = invoke(&["store-probe", "--store-url", "http://127.0.0.1:9/unused/"]);
        assert_eq!(code, 1, "{stderr}");
        assert!(!stderr.contains("--store-root"), "{stderr}");

        // Token plus Basic auth never reaches the network.
        let Invoke { code, stderr, .. } =
            invoke(&["store-probe", "--store", "webdav", "--store-url", "http://127.0.0.1:9/unused/", "--store-token", "tok", "--store-user", "u"]);
        assert_eq!(code, 1, "{stderr}");
        assert!(stderr.contains("pick one auth scheme"), "{stderr}");

        // Flags of the other backend are refused without connecting.
        let Invoke { code, stderr, .. } = invoke(&["store-probe", "--store", "local", "--store-root", &root, "--store-url", "http://127.0.0.1:9/x/"]);
        assert_eq!(code, 1, "{stderr}");
        assert!(stderr.contains("--store-url is for --store webdav"), "{stderr}");

        // https is refused when the backend is built, not after a dial.
        let Invoke { code, stderr, .. } = invoke(&["store-probe", "--store", "webdav", "--store-url", "https://cloud.example/dav/"]);
        assert_eq!(code, 1, "{stderr}");
        assert!(stderr.contains("http"), "{stderr}");

        // Upload flags without a backend are refused, not ignored.
        let Invoke { code, stderr, .. } = invoke(&["convert", "a.png", "b.png", "--etag", "x"]);
        assert_eq!(code, 1, "{stderr}");
        assert!(stderr.contains("--etag and --atomic are for uploads"), "{stderr}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
