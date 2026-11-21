//! `cargo xtask stats`: tests and lines of Rust per crate.

use std::path::Path;

#[derive(Default, Clone, Copy)]
struct Counts {
    files: usize,
    lines: usize,
    tests: usize,
    proptests: usize,
}

fn walk(dir: &Path, c: &mut Counts) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        // Do not count linked source trees twice or follow a directory link back to an ancestor.
        let Ok(kind) = e.file_type() else { continue };
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if kind.is_dir() {
            // Nested fuzz workspaces and build output are not part of the crate.
            if name != "target" && name != "fuzz" && !name.starts_with('.') {
                walk(&p, c);
            }
        } else if kind.is_file()
            && name.ends_with(".rs")
            && let Ok(s) = std::fs::read_to_string(&p)
        {
            c.files += 1;
            c.lines += s.lines().count();
            let (t, pt) = count_tests(&s);
            c.tests += t;
            c.proptests += pt;
        }
    }
}

/// `#[test]` attributes and `proptest!` blocks (each block may hold many
/// property tests, and macro-generated tests are only counted once — use
/// `--exact` for harness-accurate numbers).
fn count_tests(src: &str) -> (usize, usize) {
    let mut tests = 0;
    let mut props = 0;
    for line in src.lines() {
        let t = line.trim_start();
        if t.starts_with("//") {
            continue;
        }
        tests += t.matches("#[test]").count();
        if t.starts_with("proptest!") {
            props += 1;
        }
    }
    (tests, props)
}

fn exact_tests(pkg: &str) -> Option<usize> {
    let out = crate::cargo().args(["test", "-q", "-p", pkg, "--", "--list"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.ends_with(": test")).count())
}

pub fn run(_root: &Path, exact: bool) -> Result<(), String> {
    let meta = crate::metadata()?;
    let mut rows = Vec::new();
    for p in meta["packages"].as_array().into_iter().flatten() {
        let name = p["name"].as_str().unwrap_or("?").to_owned();
        let Some(dir) = p["manifest_path"].as_str().and_then(|m| Path::new(m).parent().map(Path::to_path_buf)) else {
            continue;
        };
        let mut c = Counts::default();
        walk(&dir, &mut c);
        let ex = if exact { exact_tests(&name) } else { None };
        rows.push((name, c, ex));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let exact_col = if exact { format!(" {:>8}", "harness") } else { String::new() };
    println!("{:<26} {:>6} {:>8} {:>7} {:>10}{exact_col}", "crate", "files", "lines", "#[test]", "proptest!");
    println!("{}", "-".repeat(62 + if exact { 9 } else { 0 }));
    let mut total = Counts::default();
    let mut total_exact = 0;
    for (name, c, ex) in &rows {
        let ex_s = match (exact, ex) {
            (false, _) => String::new(),
            (true, Some(n)) => {
                total_exact += n;
                format!(" {n:>8}")
            }
            (true, None) => format!(" {:>8}", "error"),
        };
        println!("{name:<26} {:>6} {:>8} {:>7} {:>10}{ex_s}", c.files, c.lines, c.tests, c.proptests);
        total.files += c.files;
        total.lines += c.lines;
        total.tests += c.tests;
        total.proptests += c.proptests;
    }
    println!("{}", "-".repeat(62 + if exact { 9 } else { 0 }));
    let ex_s = if exact { format!(" {total_exact:>8}") } else { String::new() };
    println!("{:<26} {:>6} {:>8} {:>7} {:>10}{ex_s}", "TOTAL", total.files, total.lines, total.tests, total.proptests);
    if !exact {
        println!("\n(#[test] counts source attributes; macro-generated tests count once. Use --exact for harness counts.)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Counts, count_tests, walk};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!("photocraft-stats-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Remove the fixture's known links before recursively removing its own directory.
            let _ = std::fs::remove_file(self.0.join("project/linked.rs"));
            #[cfg(unix)]
            let _ = std::fs::remove_file(self.0.join("project/linked-dir"));
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn walk_counts_sources_and_skips_excluded_directories() {
        let fixture = Fixture::new();
        for dir in ["src/nested", "target", "fuzz", ".hidden"] {
            std::fs::create_dir_all(fixture.0.join(dir)).unwrap();
        }
        std::fs::write(fixture.0.join("src/a.rs"), "#[test]\nfn a() {}\n").unwrap();
        std::fs::write(fixture.0.join("src/nested/b.rs"), "proptest! {}\n").unwrap();
        for path in ["target/a.rs", "fuzz/a.rs", ".hidden/a.rs", "src/not-rust.txt"] {
            std::fs::write(fixture.0.join(path), "#[test]\n").unwrap();
        }

        let mut counts = Counts::default();
        walk(&fixture.0, &mut counts);
        assert_eq!((counts.files, counts.lines, counts.tests, counts.proptests), (2, 3, 1, 1));
    }

    #[cfg(unix)]
    #[test]
    fn walk_ignores_linked_sources() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.0.join("project")).unwrap();
        std::fs::create_dir(fixture.0.join("external")).unwrap();
        std::fs::write(fixture.0.join("project/real.rs"), "#[test]\nfn real() {}\n").unwrap();
        std::fs::write(fixture.0.join("external/extra.rs"), "#[test]\nfn extra() {}\n").unwrap();
        std::os::unix::fs::symlink(fixture.0.join("external"), fixture.0.join("project/linked-dir")).unwrap();
        std::os::unix::fs::symlink(fixture.0.join("external/extra.rs"), fixture.0.join("project/linked.rs")).unwrap();

        let mut counts = Counts::default();
        walk(&fixture.0.join("project"), &mut counts);
        assert_eq!((counts.files, counts.lines, counts.tests, counts.proptests), (1, 2, 1, 0));
    }

    #[test]
    fn counts_attributes_and_proptest_blocks() {
        let src = "#[test]\nfn a() {}\n// #[test] commented\n    #[test]\nfn b() {}\nproptest! {\n}\n";
        assert_eq!(count_tests(src), (2, 1));
    }
}
