//! Native tile storage configuration, kept out of the preference data model.

use crate::prefs::Preferences;

/// A web session has no filesystem or memory-supervision thread.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(crate) struct Runtime;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::Runtime;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    // Weak ownership: the process-wide cache cannot keep a session's worker alive.
    pub(crate) struct Runtime {
        _worker: Arc<Worker>,
    }

    impl Default for Runtime {
        fn default() -> Self {
            static SHARED: OnceLock<Mutex<Weak<Worker>>> = OnceLock::new();
            let mut shared = SHARED.get_or_init(Mutex::default).lock().unwrap_or_else(PoisonError::into_inner);
            let worker = shared.upgrade().unwrap_or_else(|| {
                let worker = Arc::new(Worker::start());
                *shared = Arc::downgrade(&worker);
                worker
            });
            Self { _worker: worker }
        }
    }

    struct Worker {
        stop: Arc<(Mutex<bool>, Condvar)>,
        thread: Option<JoinHandle<()>>,
    }

    impl Worker {
        fn start() -> Self {
            let stop = Arc::new((Mutex::new(false), Condvar::new()));
            let flag = stop.clone();
            let thread = std::thread::Builder::new()
                .name("pc-memory-residency".into())
                .spawn(move || {
                    let _maintenance = photocraft_raster::spill::background_maintenance();
                    let mut measured = Instant::now();
                    loop {
                        let (lock, wake) = &*flag;
                        let stopped = lock.lock().unwrap_or_else(PoisonError::into_inner);
                        let (stopped, _) = wake.wait_timeout_while(stopped, Duration::from_millis(100), |stop| !*stop).unwrap_or_else(PoisonError::into_inner);
                        if *stopped {
                            break;
                        }
                        drop(stopped);
                        if measured.elapsed() >= Duration::from_secs(2) {
                            photocraft_raster::memory::refresh();
                            measured = Instant::now();
                        }
                        photocraft_raster::spill::maintain_budget();
                    }
                })
                .ok();
            // If spawning fails, raster's synchronous fallback still enforces the budget.
            Self { stop, thread }
        }
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            let (lock, wake) = &*self.stop;
            *lock.lock().unwrap_or_else(PoisonError::into_inner) = true;
            wake.notify_all();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn workers_stop_join_and_can_restart() {
            for _ in 0..3 {
                let worker = Worker::start();
                assert!(worker.thread.is_some());
                let stop = worker.stop.clone();
                let began = Instant::now();
                drop(worker);
                assert!(*stop.0.lock().unwrap());
                assert!(began.elapsed() < Duration::from_secs(2));
            }
        }

        #[test]
        fn sessions_share_supervision_without_static_ownership() {
            let a = Runtime::default();
            let b = Runtime::default();
            assert!(Arc::ptr_eq(&a._worker, &b._worker));
            let weak = Arc::downgrade(&a._worker);
            drop((a, b));
            // Other engine tests can hold the shared runtime concurrently; a private worker
            // proves the same ownership contract without depending on their timing.
            let worker = Arc::new(Worker::start());
            let private = Arc::downgrade(&worker);
            drop(worker);
            assert!(private.upgrade().is_none());
            drop(weak);
        }

        #[test]
        fn preferences_use_fixed_scratch_capacity_in_an_isolated_process() {
            const CHILD: &str = "PHOTOCRAFT_SCRATCH_CAPACITY_TEST_CHILD";
            if std::env::var_os(CHILD).is_none() {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "scratch::native::tests::preferences_use_fixed_scratch_capacity_in_an_isolated_process"])
                    .env(CHILD, "1")
                    .status()
                    .unwrap();
                assert!(status.success(), "isolated scratch test failed: {status}");
                return;
            }

            // If configure uses this query even once, it pauses for lack of room. This catches an
            // override that is missing or is installed after configuring the scratch directory.
            photocraft_raster::spill::set_volume_space_for_tests(|_| Some((0, 64 << 30)));
            super::super::configure(&crate::prefs::Preferences::default());
            assert!(!photocraft_raster::spill::paused());
            let room = (32_u64 << 30) - (64_u64 << 30) / 20;
            assert_eq!(photocraft_raster::spill::scratch_capacity(), Some(room - room / 64));
        }
    }
}

pub(crate) fn configure(prefs: &Preferences) {
    #[cfg(all(test, not(target_arch = "wasm32")))]
    photocraft_raster::spill::use_test_volume_space();
    let budget = photocraft_raster::memory::configure(photocraft_raster::memory::Policy {
        percent: prefs.performance.memory_usage_percent,
        // The web build's cap applies here too (effective_memory_usage_mb).
        absolute_bytes: u64::from(prefs.performance.effective_memory_usage_mb()).saturating_mul(1 << 20),
    });
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = prefs
            .scratch_disks
            .disks
            .iter()
            .find(|d| d.enabled && !d.path.trim().is_empty())
            .map(|d| if d.path.trim() == "(system temp)" { system_temp_dir() } else { std::path::PathBuf::from(d.path.trim()) });
        // An unusable or full directory pauses eviction without dropping resident pixels and
        // is retried with backoff; History's byte limit applies meanwhile (sync_scratch_state).
        // The reason is exposed in ui.inspect and the application's status bar.
        let tiles = if std::env::var_os("PHOTOCRAFT_TILE_BUDGET_MB").is_some() {
            prefs.performance.tile_budget_bytes()
        } else {
            usize::try_from(budget.tile_bytes).unwrap_or(usize::MAX)
        };
        let limit = prefs.scratch_disks.size_limit_mb;
        photocraft_raster::spill::set_size_cap((limit > 0).then(|| u64::from(limit).saturating_mul(1 << 20)));
        let _ = photocraft_raster::spill::configure(dir.as_deref(), tiles);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (prefs, budget);
}

/// The folder the `(system temp)` scratch disk names: the system's temporary folder, except on
/// Linux when that is kept in memory (a `tmpfs` /tmp), where spilling would free nothing. Then it
/// is `/var/tmp` when that is writable and on disk, else `$XDG_CACHE_HOME/photocraft/scratch`.
/// Worked out once per process.
#[cfg(not(target_arch = "wasm32"))]
pub fn system_temp_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let temp = std::env::temp_dir();
        #[cfg(target_os = "linux")]
        {
            let real = |path: &std::path::Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
            let var_tmp = real(std::path::Path::new("/var/tmp"));
            let writable = || tempfile::Builder::new().prefix(".photocraft-").tempfile_in(&var_tmp).is_ok();
            if let Some(dir) = linux::disk_backed_temp(&real(&temp), &var_tmp, &mountinfo, writable, linux::cache_home()) {
                return dir;
            }
        }
        temp
    })
    .clone()
}

#[cfg(target_os = "linux")]
mod linux {
    use photocraft_raster::memory::system::memory_backed_in;
    use std::path::{Path, PathBuf};

    /// The disk-backed folder to spill to instead of the memory-backed `temp`, or `None` when
    /// `temp` is on disk (or nothing better is known). `var_tmp` must be canonical, since a
    /// `/var/tmp` symlinked into a `tmpfs` /tmp is memory-backed too.
    pub(super) fn disk_backed_temp(
        temp: &Path,
        var_tmp: &Path,
        mountinfo: &str,
        var_tmp_writable: impl FnOnce() -> bool,
        cache_home: Option<PathBuf>,
    ) -> Option<PathBuf> {
        if !memory_backed_in(mountinfo, temp) {
            return None;
        }
        if !memory_backed_in(mountinfo, var_tmp) && var_tmp_writable() {
            return Some(var_tmp.to_path_buf());
        }
        cache_home.map(|cache| cache.join("photocraft").join("scratch"))
    }

    /// `$XDG_CACHE_HOME`, else `~/.cache`; the XDG spec ignores a relative value.
    pub(super) fn cache_home() -> Option<PathBuf> {
        let absolute = |key: &str| std::env::var_os(key).map(PathBuf::from).filter(|path| path.is_absolute());
        absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|home| home.join(".cache")))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A mount table with `/` on ext4, plus each `(mount point, filesystem)` in order.
        fn table(mounts: &[(&str, &str)]) -> String {
            let mut text = String::from("22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw\n");
            for (i, (point, fstype)) in mounts.iter().enumerate() {
                text.push_str(&format!("{} 22 0:{} / {point} rw,nosuid,nodev shared:{} - {fstype} {fstype} rw\n", 30 + i, 40 + i, 10 + i));
            }
            text
        }

        fn resolve(mountinfo: &str, writable: bool) -> Option<PathBuf> {
            disk_backed_temp(Path::new("/tmp"), Path::new("/var/tmp"), mountinfo, || writable, Some(PathBuf::from("/home/u/.cache")))
        }

        #[test]
        fn a_tmpfs_tmp_moves_to_a_disk_backed_folder() {
            let cache = PathBuf::from("/home/u/.cache/photocraft/scratch");
            assert_eq!(resolve(&table(&[("/tmp", "tmpfs")]), true), Some(PathBuf::from("/var/tmp")), "tmpfs /tmp, disk-backed /var/tmp");
            assert_eq!(resolve(&table(&[("/tmp", "ramfs")]), true), Some(PathBuf::from("/var/tmp")), "ramfs counts as memory too");
            assert_eq!(resolve(&table(&[("/tmp", "tmpfs")]), false), Some(cache.clone()), "a /var/tmp that can't be written is skipped");
            assert_eq!(resolve(&table(&[("/tmp", "tmpfs"), ("/var/tmp", "tmpfs")]), true), Some(cache.clone()), "memory-backed /var/tmp");
            assert_eq!(
                disk_backed_temp(Path::new("/tmp"), Path::new("/tmp"), &table(&[("/tmp", "tmpfs")]), || true, None),
                None,
                "a /var/tmp linked into /tmp and no cache folder: nothing better than /tmp"
            );
        }

        #[test]
        fn a_disk_backed_tmp_is_kept() {
            assert_eq!(resolve(&table(&[]), true), None, "/tmp on the root disk");
            assert_eq!(resolve(&table(&[("/tmp", "ext4")]), true), None, "/tmp on its own disk");
            assert_eq!(resolve(&table(&[("/tmp", "tmpfs"), ("/tmp", "xfs")]), true), None, "a later mount over a tmpfs /tmp covers it");
            assert_eq!(resolve(&table(&[("/tmpfs", "tmpfs"), ("/run", "tmpfs")]), true), None, "only the mount holding /tmp counts");
        }
    }
}
