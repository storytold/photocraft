//! Noticing LUT files added, removed or edited by hand while the list is open. Listing the library
//! walks and stats every file, so the check runs on a worker thread; the UI thread only starts it
//! (at most one at a time), compares nothing, and picks up the answer on a later frame.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::Context;
use photocraft_engine::lut_library::{LutLibrary, PackInfo};

use super::Scans;
use crate::lut_library_ui::rows::Pack;

/// The stale-listing check's state, inside [`Scans`]' shared data.
#[derive(Default)]
pub(super) struct Check {
    /// A worker is running.
    running: bool,
    /// The listing revision the last finished check found out of date, until it is taken.
    found: Option<u64>,
}

/// Whether the packs on disk (`fresh`) differ from the ones the list shows. Sizes and modification
/// times are part of [`PackInfo`], so an edit in place counts too.
fn packs_differ(fresh: &[PackInfo], shown: &[Pack]) -> bool {
    fresh.len() != shown.len() || fresh.iter().zip(shown).any(|(a, b)| *a != b.info)
}

/// The check itself: list the library at `root` and compare it with `shown`. This is the slow part
/// (a walk and a stat per file), so only a worker calls it. A missing or unreadable library lists
/// as empty, the same as when the list was built.
fn differs_on_disk(root: &Path, shown: &[Pack]) -> bool {
    packs_differ(&LutLibrary::new(root).list().unwrap_or_default(), shown)
}

/// Clears `running` when the worker ends, even if it panics, so checks cannot stop for good.
struct Finished(Scans);

impl Drop for Finished {
    fn drop(&mut self) {
        self.0.inner().stale.running = false;
    }
}

impl Scans {
    /// Start a check of the library at `root` against `shown` (the listing of revision `rev`),
    /// unless one is already running. Returns at once; [`take_stale`](Self::take_stale) has the
    /// answer once the worker has finished, and the context repaints when it finds a difference.
    pub(in crate::lut_library_ui) fn start_stale_check(&self, ctx: &Context, root: PathBuf, rev: u64, shown: Arc<Vec<Pack>>) {
        {
            let mut inner = self.inner();
            if inner.stale.running {
                return;
            }
            inner.stale.running = true;
        }
        let (shared, ctx) = (self.clone(), ctx.clone());
        let work = move || {
            let _finished = Finished(shared.clone());
            if differs_on_disk(&root, &shown) {
                shared.inner().stale.found = Some(rev);
                ctx.request_repaint();
            }
        };
        // The web build has no library to check, so this only runs inline there for completeness.
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(work);
        #[cfg(target_arch = "wasm32")]
        work();
    }

    /// Whether a finished check found the listing of revision `rev` out of date. The answer is
    /// used up by asking; one for another revision (the library changed meanwhile) is dropped.
    pub(in crate::lut_library_ui) fn take_stale(&self, rev: u64) -> bool {
        self.inner().stale.found.take() == Some(rev)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use photocraft_cms::lutfile::{LutFile, write_cube};

    use super::*;
    use crate::lut_library_ui::rows::Listing;

    fn library(name: &str) -> LutLibrary {
        let root = std::env::temp_dir().join(format!("photocraft-lutstale-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Pack")).unwrap();
        fs::write(root.join("Pack").join("a.cube"), write_cube(&LutFile::identity(3))).unwrap();
        LutLibrary::new(root)
    }

    fn wait_for(scans: &Scans, rev: u64) -> bool {
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until {
            if scans.take_stale(rev) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    fn settled(scans: &Scans) {
        let until = Instant::now() + Duration::from_secs(10);
        while scans.inner().stale.running && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_comparison_notices_added_removed_and_edited_files() {
        let lib = library("compare");
        let shown = Listing::build(&lib);
        let root = lib.root().to_path_buf();
        assert!(!differs_on_disk(&root, &shown.packs), "nothing changed");

        let new = root.join("Pack").join("b.cube");
        fs::write(&new, write_cube(&LutFile::identity(3))).unwrap();
        assert!(differs_on_disk(&root, &shown.packs), "a file added by hand");
        fs::remove_file(&new).unwrap();
        assert!(!differs_on_disk(&root, &shown.packs), "and removed again");

        fs::write(root.join("Pack").join("a.cube"), write_cube(&LutFile::identity(5))).unwrap();
        assert!(differs_on_disk(&root, &shown.packs), "a file edited in place changes its size");
        fs::write(root.join("Pack").join("a.cube"), write_cube(&LutFile::identity(3))).unwrap();

        fs::create_dir_all(root.join("Other")).unwrap();
        fs::write(root.join("Other").join("c.cube"), write_cube(&LutFile::identity(3))).unwrap();
        assert!(differs_on_disk(&root, &shown.packs), "a new pack");
        fs::remove_dir_all(&root).unwrap();
        assert!(differs_on_disk(&root, &shown.packs), "a library that vanished lists as empty");
    }

    #[test]
    fn the_worker_reports_a_change_once_and_only_for_the_revision_it_checked() {
        let lib = library("worker");
        let shown = Listing::build(&lib);
        let scans = Scans::default();
        let ctx = Context::default();

        scans.start_stale_check(&ctx, lib.root().to_path_buf(), 7, shown.packs.clone());
        settled(&scans);
        assert!(!scans.take_stale(7), "unchanged library: nothing found");

        fs::write(lib.root().join("Pack").join("b.cube"), write_cube(&LutFile::identity(3))).unwrap();
        scans.start_stale_check(&ctx, lib.root().to_path_buf(), 7, shown.packs.clone());
        assert!(wait_for(&scans, 7), "the new file is noticed");
        assert!(!scans.take_stale(7), "an answer is used up by taking it");

        scans.start_stale_check(&ctx, lib.root().to_path_buf(), 7, shown.packs.clone());
        settled(&scans);
        assert!(!scans.take_stale(8), "the library moved on to revision 8, so the old answer is dropped");
        assert!(!scans.take_stale(7), "and it is gone, not kept for later");
    }

    #[test]
    fn only_one_check_runs_at_a_time() {
        let lib = library("single");
        let shown = Listing::build(&lib);
        let scans = Scans::default();
        scans.inner().stale.running = true;
        fs::write(lib.root().join("Pack").join("b.cube"), write_cube(&LutFile::identity(3))).unwrap();
        scans.start_stale_check(&Context::default(), lib.root().to_path_buf(), 1, shown.packs.clone());
        std::thread::sleep(Duration::from_millis(100));
        assert!(!scans.take_stale(1), "a check already in flight means no second worker");
        scans.inner().stale.running = false;
    }
}
