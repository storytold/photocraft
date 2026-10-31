//! [`S3Store`]: one document's bundle on S3-compatible storage, its objects in a shared pool.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use photocraft_format::{BundleObject, BundleStore, ByteSource, FormatError, Result, Stored};

use crate::client::{Client, Condition, Put, S3Config, io_error};

const MANIFEST: &str = "manifest.json";
/// The previews a bundle can hold (`format`'s preview paths).
const PREVIEWS: [&str; 2] = ["thumb.png", "composite/preview.png"];
/// Default for [`S3Store::set_parallel_uploads`].
pub const DEFAULT_PARALLEL_UPLOADS: usize = 16;

/// A save found the document in storage at another version than the one it started from:
/// someone else saved it since this store read it, or a new document's key was already taken.
/// Nothing was overwritten. [`is_conflict`] recognizes it in a [`FormatError`].
#[derive(Debug)]
pub struct HeadChanged;

impl fmt::Display for HeadChanged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the document in storage changed since it was opened here (or already existed): open it again, or save under another name")
    }
}

impl std::error::Error for HeadChanged {}

/// `true` when a save failed with [`HeadChanged`].
pub fn is_conflict(err: &FormatError) -> bool {
    matches!(err, FormatError::Io(e) if e.get_ref().is_some_and(|inner| inner.is::<HeadChanged>()))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn not_saving() -> FormatError {
    io_error("S3 store used outside a save".into())
}

/// A `.pcraft` bundle on S3-compatible storage (AWS S3, Cloudflare R2, MinIO).
///
/// ```text
/// <pool>tiles/<blake3>.zst         content-addressed objects, shared by every document and
/// <pool>blobs/<blake3>.zst         version that uses the pool
/// <document>manifest.json          this document's head: the version you open
/// <document>thumb.png              previews of that version
/// <document>composite/preview.png
/// ```
///
/// A save uploads the objects the pool lacks (in parallel, `If-None-Match: *`, so an object
/// another save already stored is left alone), then replaces the manifest only if it is still the
/// version this store last read or wrote (`If-Match`; `If-None-Match: *` for a new document).
/// When someone else saved in between, the save fails with [`HeadChanged`] and overwrites nothing.
/// Previews follow the new manifest, best effort.
///
/// Nothing is ever deleted from the pool: objects an old version references stay readable, and
/// removing unreferenced ones (with version history) is left to a later collector that knows
/// every document sharing the pool.
pub struct S3Store {
    client: Client,
    pool: String,
    document: String,
    /// ETag of the manifest this store last read or wrote: the version the next save replaces.
    head: Mutex<Option<String>>,
    /// Pool objects (bundle paths) this store has read or stored, so it doesn't upload them again.
    known: Mutex<HashSet<String>>,
    parallel_uploads: usize,
    save: Option<Save>,
}

#[derive(Default)]
struct Save {
    stored: HashMap<String, Stored>,
    previews: Vec<(String, Vec<u8>)>,
}

impl S3Store {
    /// The document under the key prefix `document`, its objects in the pool under `pool`
    /// (prefixes are used as given: end them with `/` for folder-style keys). Nothing is
    /// requested until the first read or save.
    pub fn new(config: S3Config, pool: impl Into<String>, document: impl Into<String>) -> Result<Self> {
        Ok(S3Store {
            client: Client::new(config)?,
            pool: pool.into(),
            document: document.into(),
            head: Mutex::new(None),
            known: Mutex::new(HashSet::new()),
            parallel_uploads: DEFAULT_PARALLEL_UPLOADS,
            save: None,
        })
    }

    /// ETag of the manifest this store last read or wrote; `None` for a document it hasn't seen.
    pub fn head(&self) -> Option<String> {
        lock(&self.head).clone()
    }

    /// How many objects a save uploads at once (at least 1).
    pub fn set_parallel_uploads(&mut self, n: usize) {
        self.parallel_uploads = n.max(1);
    }

    fn key(&self, path: &str) -> String {
        key(&self.pool, &self.document, path)
    }
}

fn pooled(path: &str) -> bool {
    path.starts_with("tiles/") || path.starts_with("blobs/")
}

fn key(pool: &str, document: &str, path: &str) -> String {
    if pooled(path) { format!("{pool}{path}") } else { format!("{document}{path}") }
}

/// Stores one object unless the pool already has it.
fn upload(client: &Client, pool: &str, object: &BundleObject<'_>) -> Result<Stored> {
    match client.put(&format!("{pool}{}", object.path()), &object.compressed(), Condition::IfAbsent)? {
        Put::Done { .. } => Ok(Stored::Written),
        Put::PreconditionFailed => Ok(Stored::Reused),
        Put::Conflicting => Err(io_error(format!("S3 PUT `{}`: another upload of it was in progress; save again", object.path()))),
    }
}

impl ByteSource for S3Store {
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        let Some((bytes, etag)) = self.client.get(&self.key(path), max)? else {
            return Err(FormatError::Corrupt(format!("missing `{path}`")));
        };
        if path == MANIFEST {
            *lock(&self.head) = etag;
        } else if pooled(path) {
            lock(&self.known).insert(path.to_string());
        }
        Ok(bytes)
    }
}

impl BundleStore for S3Store {
    fn begin(&mut self) -> Result<()> {
        self.save = Some(Save::default());
        Ok(())
    }

    /// Uploads, on [`Self::set_parallel_uploads`] connections, every object this store doesn't
    /// know the pool has. The first failure stops the rest and fails the save.
    fn plan(&mut self, objects: &[BundleObject<'_>]) -> Result<()> {
        let save = self.save.as_mut().ok_or_else(not_saving)?;
        let todo: Vec<&BundleObject<'_>> = {
            let known = lock(&self.known);
            objects.iter().filter(|o| !known.contains(o.path())).collect()
        };
        let (client, pool) = (&self.client, self.pool.as_str());
        let (next, failed) = (AtomicUsize::new(0), AtomicBool::new(false));
        let worker = || -> Result<Vec<(String, Stored)>> {
            let mut done = Vec::new();
            while !failed.load(Ordering::Relaxed) {
                let Some(object) = todo.get(next.fetch_add(1, Ordering::Relaxed)) else { break };
                match upload(client, pool, object) {
                    Ok(stored) => done.push((object.path().to_string(), stored)),
                    Err(e) => {
                        failed.store(true, Ordering::Relaxed);
                        return Err(e);
                    }
                }
            }
            Ok(done)
        };
        let results: Vec<Result<Vec<(String, Stored)>>> = std::thread::scope(|s| {
            let workers: Vec<_> = (0..self.parallel_uploads.min(todo.len())).map(|_| s.spawn(worker)).collect();
            workers.into_iter().map(|w| w.join().unwrap_or_else(|_| Err(io_error("S3 upload worker panicked".into())))).collect()
        });
        let mut known = lock(&self.known);
        let mut first_error = None;
        for result in results {
            match result {
                Ok(done) => {
                    for (path, stored) in done {
                        known.insert(path.clone());
                        save.stored.insert(path, stored);
                    }
                }
                Err(e) => first_error = first_error.or(Some(e)),
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn object(&mut self, object: &BundleObject<'_>) -> Result<Stored> {
        let save = self.save.as_mut().ok_or_else(not_saving)?;
        if let Some(stored) = save.stored.get(object.path()) {
            return Ok(*stored);
        }
        // Not uploaded by `plan`: known already, or a driver that skipped `plan`.
        if lock(&self.known).contains(object.path()) {
            return Ok(Stored::Reused);
        }
        let stored = upload(&self.client, &self.pool, object)?;
        lock(&self.known).insert(object.path().to_string());
        Ok(stored)
    }

    fn preview(&mut self, path: &str, png: Vec<u8>) -> Result<()> {
        let save = self.save.as_mut().ok_or_else(not_saving)?;
        save.previews.push((path.to_string(), png));
        Ok(())
    }

    fn commit(&mut self, manifest: &[u8]) -> Result<usize> {
        let save = self.save.as_mut().ok_or_else(not_saving)?;
        let head = lock(&self.head).clone();
        let condition = head.as_deref().map_or(Condition::IfAbsent, Condition::IfMatch);
        match self.client.put(&key(&self.pool, &self.document, MANIFEST), manifest, condition)? {
            Put::Done { etag } => *lock(&self.head) = etag,
            Put::PreconditionFailed | Put::Conflicting => return Err(FormatError::Io(std::io::Error::other(HeadChanged))),
        }
        // The save has happened. Previews follow the new head, and ones it doesn't have go.
        let previews = std::mem::take(&mut save.previews);
        for name in PREVIEWS {
            let key = key(&self.pool, &self.document, name);
            let result = match previews.iter().find(|(path, _)| path == name) {
                Some((_, png)) => self.client.put(&key, png, Condition::Always).map(drop),
                None => self.client.delete(&key),
            };
            if let Err(e) = result {
                log::warn!("saved, but the preview `{name}` is not up to date: {e}");
            }
        }
        Ok(0)
    }

    fn end(&mut self) {
        self.save = None;
    }
}
