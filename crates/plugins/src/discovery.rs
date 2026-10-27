//! Bounded desktop folder traversal shared by the WASM and native registries.
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(crate) struct Scan {
    pub files: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
}

const MAX_ROOTS: usize = 64;
const MAX_DEPTH: u8 = 32;
const MAX_ENTRIES: usize = 100_000;
const MAX_FILES: usize = 4096;

/// Breadth-first, in path order. Exceeding a bound is reported as a failure and keeps what was
/// found before it, so one oversized folder cannot hide every other plug-in.
pub(crate) fn scan(roots: &[PathBuf], extension: &str) -> Scan {
    let mut report = Scan::default();
    for skipped in roots.iter().skip(MAX_ROOTS) {
        report.failed.push((skipped.clone(), format!("skipped: at most {MAX_ROOTS} plug-in folders are scanned")));
    }
    let mut queue: VecDeque<_> = roots.iter().take(MAX_ROOTS).map(|p| (p.clone(), 0u8)).collect();
    let mut directories = BTreeSet::new();
    let mut files = BTreeMap::new();
    let mut entries = 0usize;
    'scan: while let Some((path, depth)) = queue.pop_front() {
        let canonical = match std::fs::canonicalize(&path) {
            Ok(path) => path,
            Err(e) => {
                report.failed.push((path, e.to_string()));
                continue;
            }
        };
        if !directories.insert(canonical.clone()) {
            continue;
        }
        if depth > MAX_DEPTH {
            report.failed.push((path, format!("plug-in subfolder nesting exceeds {MAX_DEPTH} levels")));
            continue;
        }
        let children = match std::fs::read_dir(&canonical) {
            Ok(children) => children,
            Err(e) => {
                report.failed.push((path, e.to_string()));
                continue;
            }
        };
        let mut paths = Vec::new();
        for child in children {
            entries += 1;
            if entries > MAX_ENTRIES {
                report.failed.push((path, format!("scan stopped after {MAX_ENTRIES} folder entries; remaining folders were skipped")));
                break 'scan;
            }
            match child {
                Ok(child) => paths.push(child.path()),
                Err(e) => report.failed.push((canonical.clone(), e.to_string())),
            }
        }
        paths.sort();
        for path in paths {
            let metadata = match std::fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(e) => {
                    report.failed.push((path, e.to_string()));
                    continue;
                }
            };
            if metadata.is_dir() {
                queue.push_back((path, depth + 1));
            } else if metadata.is_file() && has_extension(&path, extension) {
                match std::fs::canonicalize(&path) {
                    Ok(identity) if files.contains_key(&identity) => {}
                    Ok(_) if files.len() >= MAX_FILES => {
                        report.failed.push((path, format!("scan stopped after {MAX_FILES} matching files; remaining files were skipped")));
                        break 'scan;
                    }
                    // Deduplicate the target but preserve the matching alias suffix for
                    // registry validation (a .8bf link may point at a .dll target).
                    Ok(identity) => {
                        files.insert(identity, path);
                    }
                    Err(e) => report.failed.push((path, e.to_string())),
                }
            }
        }
    }
    report.files = files.into_values().collect();
    report
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(extension))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlapping_roots_symlink_cycles_and_missing_roots_are_bounded() {
        let base = std::env::temp_dir().join(format!("pc-plugin-scan-{}", std::process::id()));
        let nested = base.join("vendor/presets/filters");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(base.join("one.8bf"), b"metadata fixture").unwrap();
        std::fs::write(nested.join("two.8BF"), b"metadata fixture").unwrap();
        std::fs::write(nested.join("ignored.txt"), b"ignored").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&base, nested.join("loop")).unwrap();
            std::os::unix::fs::symlink(base.join("one.8bf"), nested.join("alias.8bf")).unwrap();
        }
        let report = scan(&[base.clone(), nested, base.join("missing")], "8bf");
        std::fs::remove_dir_all(base).unwrap();
        assert_eq!(report.files.len(), 2);
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].0.ends_with("missing"));
    }
    #[cfg(unix)]
    #[test]
    fn matching_alias_keeps_its_extension_when_target_has_another_suffix() {
        let base = std::env::temp_dir().join(format!("pc-plugin-alias-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("filter.dll"), b"metadata fixture").unwrap();
        std::os::unix::fs::symlink(base.join("filter.dll"), base.join("filter.8bf")).unwrap();
        let report = scan(std::slice::from_ref(&base), "8bf");
        std::fs::remove_dir_all(&base).unwrap();
        assert_eq!(report.files, vec![base.join("filter.8bf")]);
    }
    #[test]
    fn exceeding_the_root_limit_reports_skipped_roots_and_keeps_the_rest() {
        let base = std::env::temp_dir().join(format!("pc-plugin-roots-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("kept.8bf"), b"metadata fixture").unwrap();
        let mut roots = vec![base.clone(); MAX_ROOTS];
        roots.push(base.join("over-the-limit"));
        let report = scan(&roots, "8bf");
        std::fs::remove_dir_all(&base).unwrap();
        assert_eq!(report.files, vec![base.join("kept.8bf")]);
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].0.ends_with("over-the-limit"));
    }
}
