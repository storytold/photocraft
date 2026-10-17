//! Cartography-audit metric rollup for the PhotoCraft workspace (Module 9.4).
//!
//! Single-file, std-only Rust: build with `rustc -O --edition 2021 metrics.rs -o <bin>`.
//! No Cargo project, so it never joins the PhotoCraft workspace.
//!
//! Usage: metrics <scc-by-file.csv> <repo-root> <out-dir>
//!   scc-by-file.csv  output of `scc --by-file -f csv ...` run from <repo-root>
//!   <repo-root>      used to read Rust sources for McCabe counting
//!   <out-dir>        receives component-rollup.{txt,tsv,json} and category-language.tsv
//!
//! Every file is put in exactly one of three categories:
//!   System       shipped product code (crates/*/src, apps/*)
//!   DevOps/Test  tests, benches, examples, fuzz targets, inline `#[cfg(test)]` modules,
//!                testkit, xtask, build scripts, manifests, CI, packaging, perf/scorecard config
//!   Excluded     non-authored or non-code: docs, data, SVG/JSON/text, generated tables
//!
//! Inline `#[cfg(test)] mod ... { }` blocks inside System files are carved out and
//! credited to the component's "(tests)" row, so the System subtotal is test-free.
//!
//! Cyclomatic complexity (Rust only, extended/CC2-style McCabe):
//!   CC(file) = max(fn_count, 1) + decisions
//!   decisions = if + while + for(loops only) + (match arms - match count) + binary && + binary ||
//!   `?` early returns are counted separately (column `qmarks`), not added to CC.
//!   `=>` arms inside `macro_rules!` are counted too (small overstatement, stated in the report).
//! Non-Rust files use scc's own complexity proxy.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

#[derive(Default, Clone)]
struct Row {
    files: u64,
    loc: u64,
    types: u64,
    fns: u64,
    cc: u64,
    qmarks: u64,
    hottest: String,
    hottest_cc: u64,
}

impl Row {
    fn add(&mut self, loc: u64, m: &Measure, file: &str) {
        self.files += 1;
        self.loc += loc;
        self.types += m.types;
        self.fns += m.fns;
        self.cc += m.cc;
        self.qmarks += m.qmarks;
        if m.cc > self.hottest_cc {
            self.hottest_cc = m.cc;
            self.hottest = file.to_string();
        }
    }
}

#[derive(Default, Clone, Copy)]
struct Measure {
    types: u64,
    fns: u64,
    decisions: u64,
    cc: u64,
    qmarks: u64,
}

const EXCLUDED_LANGS: &[&str] = &[
    "Markdown", "SVG", "Plain Text", "License", "JSON", "CSV", "ReStructuredText", "gitignore",
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(csv), Some(root), Some(out)) = (args.get(1), args.get(2), args.get(3)) else {
        eprintln!("usage: metrics <scc-by-file.csv> <repo-root> <out-dir>");
        std::process::exit(2);
    };
    if let Err(e) = run(Path::new(csv), Path::new(root), Path::new(out)) {
        eprintln!("metrics: {e}");
        std::process::exit(1);
    }
}

fn run(csv: &Path, root: &Path, out: &Path) -> Result<(), String> {
    let text = fs::read_to_string(csv).map_err(|e| format!("read {}: {e}", csv.display()))?;
    let mut rows: BTreeMap<(String, String), Row> = BTreeMap::new();
    let mut cat_lang: BTreeMap<(String, String), u64> = BTreeMap::new();
    let mut scc_total: u64 = 0;
    let mut big_files: Vec<(u64, String, String)> = Vec::new();
    let mut hot_files: Vec<(u64, u64, String)> = Vec::new();
    let mut excluded_why: BTreeMap<String, String> = BTreeMap::new();

    for (n, line) in text.lines().enumerate() {
        if n == 0 || line.trim().is_empty() {
            continue;
        }
        let f = split_csv(line);
        let (Some(lang), Some(path), Some(code), Some(scc_cc)) =
            (f.first(), f.get(1), f.get(4), f.get(7))
        else {
            return Err(format!("bad csv line {}: {line}", n + 1));
        };
        let code: u64 = code.parse().map_err(|_| format!("bad Code on line {}", n + 1))?;
        let scc_cc: u64 = scc_cc.parse().unwrap_or(0);
        let path = path.trim_start_matches("./").to_string();
        scc_total += code;

        let src = if lang == "Rust" { fs::read_to_string(root.join(&path)).ok() } else { None };
        let (component, category, why) = classify(&path, lang, src.as_deref());
        if category == "Excluded" {
            excluded_why.entry(component.clone()).or_insert(why);
        }
        if code > 2000 {
            big_files.push((code, path.clone(), category.to_string()));
        }

        match (lang.as_str(), src.as_deref(), category) {
            ("Rust", Some(src), "System") => {
                let cleaned = clean_rust(src);
                let regions = test_regions(&cleaned);
                let (sys_text, test_text) = split_regions(&cleaned, &regions);
                let test_loc = code_lines(&test_text).min(code);
                let sys_m = measure(&sys_text);
                let test_m = measure(&test_text);
                rows.entry((component.clone(), "System".into()))
                    .or_default()
                    .add(code - test_loc, &sys_m, &path);
                *cat_lang.entry(("System".into(), "Rust".into())).or_default() += code - test_loc;
                if test_loc > 0 || !regions.is_empty() {
                    rows.entry((format!("{component} (tests)"), "DevOps/Test".into()))
                        .or_default()
                        .add(test_loc, &test_m, &path);
                    *cat_lang.entry(("DevOps/Test".into(), "Rust".into())).or_default() += test_loc;
                }
                hot_files.push((sys_m.cc, code - test_loc, path.clone()));
            }
            ("Rust", Some(src), cat) => {
                let m = measure(&clean_rust(src));
                rows.entry((component, cat.into())).or_default().add(code, &m, &path);
                *cat_lang.entry((cat.into(), "Rust".into())).or_default() += code;
                if cat != "Excluded" {
                    hot_files.push((m.cc, code, path.clone()));
                }
            }
            (_, _, cat) => {
                let m = Measure { cc: scc_cc, ..Measure::default() };
                rows.entry((component, cat.into())).or_default().add(code, &m, &path);
                *cat_lang.entry((cat.into(), lang.clone())).or_default() += code;
            }
        }
    }

    fs::create_dir_all(out).map_err(|e| format!("mkdir {}: {e}", out.display()))?;
    let report = render(&rows, &cat_lang, scc_total, &big_files, &mut hot_files, &excluded_why);
    write(out, "component-rollup.txt", &report)?;
    write(out, "component-rollup.tsv", &render_tsv(&rows))?;
    write(out, "component-rollup.json", &render_json(&rows, scc_total))?;
    let mut cl = String::from("category\tlanguage\tloc\n");
    for ((c, l), v) in &cat_lang {
        let _ = writeln!(cl, "{c}\t{l}\t{v}");
    }
    write(out, "category-language.tsv", &cl)?;
    print!("{report}");
    Ok(())
}

fn write(dir: &Path, name: &str, s: &str) -> Result<(), String> {
    fs::write(dir.join(name), s).map_err(|e| format!("write {name}: {e}"))
}

/// Minimal CSV splitter: handles double-quoted fields with "" escapes.
fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        match (c, q) {
            ('"', true) if it.peek() == Some(&'"') => {
                cur.push('"');
                it.next();
            }
            ('"', _) => q = !q,
            (',', false) => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Map a repo-relative path to (component, category, reason).
fn classify(path: &str, lang: &str, src: Option<&str>) -> (String, &'static str, String) {
    let seg: Vec<&str> = path.split('/').collect();
    let top = seg.first().copied().unwrap_or("");
    let file = seg.last().copied().unwrap_or("");

    if EXCLUDED_LANGS.contains(&lang) {
        return (format!("Excluded: {lang}"), "Excluded", "docs/data/assets, not program code".into());
    }
    if matches!(top, "book" | "docs") {
        return (format!("Excluded: {top}/"), "Excluded", "documentation sources".into());
    }
    if let Some(s) = src {
        let head: String = s.lines().take(12).collect::<Vec<_>>().join("\n");
        if head.contains("@generated") || head.contains("DO NOT EDIT") || (head.contains("//") && head.contains("Generated by")) {
            return ("Excluded: generated Rust".into(), "Excluded", "header marks file as generated".into());
        }
    }
    if file == "Cargo.toml" || file == "build.rs" || file == "Cargo.lock" {
        return ("build: Cargo manifests + build.rs".into(), "DevOps/Test", String::new());
    }
    match top {
        "crates" => {
            let krate = seg.get(1).copied().unwrap_or("?");
            let comp = format!("crates/{krate}");
            if krate == "testkit" {
                return (comp, "DevOps/Test", String::new());
            }
            match seg.get(2).copied() {
                Some("src") if !is_test_file(file) => (comp, "System", String::new()),
                Some("src") => (format!("{comp} (tests)"), "DevOps/Test", String::new()),
                Some("tests" | "benches" | "examples" | "fuzz") => {
                    (format!("{comp} (tests)"), "DevOps/Test", String::new())
                }
                _ if lang == "Rust" => (comp, "System", String::new()),
                _ => (format!("{comp} (tests)"), "DevOps/Test", String::new()),
            }
        }
        "apps" => {
            let app = seg.get(1).copied().unwrap_or("?");
            let comp = format!("apps/{app}");
            match seg.get(2).copied() {
                Some("tests" | "benches" | "examples") => (format!("{comp} (tests)"), "DevOps/Test", String::new()),
                Some("src") if is_test_file(file) => (format!("{comp} (tests)"), "DevOps/Test", String::new()),
                _ => (comp, "System", String::new()),
            }
        }
        "xtask" => ("xtask".into(), "DevOps/Test", String::new()),
        _ if seg.len() == 1 => ("build: repo root config".into(), "DevOps/Test", String::new()),
        ".github" => ("ci: .github".into(), "DevOps/Test", String::new()),
        other => (format!("devops: {other}/"), "DevOps/Test", String::new()),
    }
}

fn is_test_file(file: &str) -> bool {
    file == "tests.rs" || file.ends_with("_tests.rs") || file == "test.rs" || file.ends_with("_test.rs")
}

// ---------------------------------------------------------------- Rust lexing

/// Blank out comments and the contents of string/char literals, keeping newlines,
/// so later passes see only code tokens. String literals collapse to `""`.
fn clean_rust(src: &str) -> String {
    let c: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let at = |k: usize| c.get(k).copied().unwrap_or('\0');
    let blank = |out: &mut String, ch: char| out.push(if ch == '\n' { '\n' } else { ' ' });
    while i < c.len() {
        let ch = c[i];
        if ch == '/' && at(i + 1) == '/' {
            while i < c.len() && c[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if ch == '/' && at(i + 1) == '*' {
            let mut depth = 0usize;
            while i < c.len() {
                if c[i] == '/' && at(i + 1) == '*' {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if c[i] == '*' && at(i + 1) == '/' {
                    depth = depth.saturating_sub(1);
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    blank(&mut out, c[i]);
                    i += 1;
                }
            }
            continue;
        }
        // raw strings: r"..", r#".."#, br#".."#, cr".."
        let prev_ident = i > 0 && (c[i - 1].is_alphanumeric() || c[i - 1] == '_');
        if !prev_ident && (ch == 'r' || ((ch == 'b' || ch == 'c') && at(i + 1) == 'r')) {
            let start = if ch == 'r' { i + 1 } else { i + 2 };
            let mut j = start;
            while at(j) == '#' {
                j += 1;
            }
            if at(j) == '"' {
                let hashes = j - start;
                out.push_str("\"\"");
                for _ in i..=j {
                    out.push(' ');
                }
                i = j + 1;
                'raw: while i < c.len() {
                    if c[i] == '"' && (0..hashes).all(|h| at(i + 1 + h) == '#') {
                        for _ in 0..=hashes {
                            out.push(' ');
                        }
                        i += 1 + hashes;
                        break 'raw;
                    }
                    blank(&mut out, c[i]);
                    i += 1;
                }
                continue;
            }
        }
        if ch == '"' {
            out.push_str("\"\"");
            i += 1;
            while i < c.len() && c[i] != '"' {
                if c[i] == '\\' {
                    blank(&mut out, c[i]);
                    i += 1;
                }
                if i < c.len() {
                    blank(&mut out, c[i]);
                    i += 1;
                }
            }
            i += 1; // closing quote (replaced by the "" placeholder)
            continue;
        }
        if ch == '\'' {
            // char literal ('x', '\n', '\u{..}') vs lifetime/label ('a)
            if at(i + 1) == '\\' {
                let mut j = i + 2;
                while j < c.len() && c[j] != '\'' && c[j] != '\n' {
                    j += 1;
                }
                out.push_str("' '");
                for _ in i + 3..=j {
                    out.push(' ');
                }
                i = j + 1;
                continue;
            }
            if at(i + 2) == '\'' {
                out.push_str("' '");
                i += 3;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// Byte ranges of `#[cfg(test)]`-gated `mod name { ... }` blocks in cleaned source.
fn test_regions(s: &str) -> Vec<(usize, usize)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(p) = s.get(from..).and_then(|t| t.find("#[cfg(")) {
        let start = from + p;
        let Some(close) = bracket_end(b, start + 1, b'[', b']') else { break };
        let attr = s.get(start..=close).unwrap_or("");
        from = close + 1;
        let gated = has_word(attr, "test") && !attr.contains("not(test") && !attr.contains("not( test");
        if !gated {
            continue;
        }
        // skip whitespace and further attributes, optional visibility, then `mod ident {`
        let mut j = close + 1;
        loop {
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if b.get(j) == Some(&b'#') && b.get(j + 1) == Some(&b'[') {
                match bracket_end(b, j + 1, b'[', b']') {
                    Some(e) => j = e + 1,
                    None => break,
                }
            } else {
                break;
            }
        }
        let rest = s.get(j..).unwrap_or("");
        let rest_trim = rest.strip_prefix("pub(crate) ").or_else(|| rest.strip_prefix("pub ")).unwrap_or(rest);
        let skipped = rest.len() - rest_trim.len();
        if let Some(after_mod) = rest_trim.strip_prefix("mod ") {
            let off = j + skipped + 4;
            if let Some(brace_rel) = after_mod.find(['{', ';']) {
                if after_mod.as_bytes().get(brace_rel) == Some(&b'{') {
                    if let Some(end) = bracket_end(b, off + brace_rel, b'{', b'}') {
                        out.push((start, end + 1));
                        from = end + 1;
                    }
                }
            }
        }
    }
    out
}

fn bracket_end(b: &[u8], open_at: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0i64;
    for (k, &ch) in b.iter().enumerate().skip(open_at) {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(k);
            }
        }
    }
    None
}

fn has_word(s: &str, w: &str) -> bool {
    s.match_indices(w).any(|(i, _)| {
        let before = s[..i].chars().last().map_or(true, |c| !(c.is_alphanumeric() || c == '_'));
        let after = s[i + w.len()..].chars().next().map_or(true, |c| !(c.is_alphanumeric() || c == '_'));
        before && after
    })
}

/// Split cleaned text into (outside-regions, inside-regions), preserving line structure.
fn split_regions(s: &str, regions: &[(usize, usize)]) -> (String, String) {
    let mut sys = String::with_capacity(s.len());
    let mut test = String::new();
    for (i, ch) in s.char_indices() {
        let inside = regions.iter().any(|&(a, b)| i >= a && i < b);
        let (keep, other) = if inside { (&mut test, &mut sys) } else { (&mut sys, &mut test) };
        keep.push(ch);
        if ch == '\n' {
            other.push('\n');
        }
    }
    (sys, test)
}

fn code_lines(s: &str) -> u64 {
    s.lines().filter(|l| !l.trim().is_empty()).count() as u64
}

fn measure(s: &str) -> Measure {
    let c: Vec<char> = s.chars().collect();
    let mut m = Measure::default();
    let (mut matches, mut arrows) = (0u64, 0u64);
    let mut impl_header = false;
    let mut prev_operand = false; // previous significant token can end an expression
    let mut i = 0;
    let next_sig = |k: usize| -> char {
        let mut k = k;
        while k < c.len() && c[k].is_whitespace() {
            k += 1;
        }
        c.get(k).copied().unwrap_or('\0')
    };
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch.is_alphabetic() || ch == '_' {
            let st = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            let w: String = c[st..i].iter().collect();
            match w.as_str() {
                "fn" if next_sig(i) != '(' => m.fns += 1,
                "if" | "while" => m.decisions += 1,
                "for" if !impl_header && next_sig(i) != '<' => m.decisions += 1,
                "match" => matches += 1,
                "impl" => impl_header = true,
                "struct" | "enum" | "trait" => m.types += 1,
                _ => {}
            }
            prev_operand = !matches!(
                w.as_str(),
                "if" | "while" | "for" | "match" | "return" | "in" | "let" | "else" | "move" | "mut" | "ref" | "break"
            );
            continue;
        }
        if ch.is_ascii_digit() {
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '.') {
                i += 1;
            }
            prev_operand = true;
            continue;
        }
        let nx = c.get(i + 1).copied().unwrap_or('\0');
        match (ch, nx) {
            ('=', '>') => {
                arrows += 1;
                prev_operand = false;
                i += 2;
                continue;
            }
            ('&', '&') | ('|', '|') => {
                if prev_operand {
                    m.decisions += 1;
                }
                prev_operand = false;
                i += 2;
                continue;
            }
            ('?', _) => {
                m.qmarks += 1;
                prev_operand = true;
            }
            ('{' | ';', _) => {
                impl_header = false;
                prev_operand = false;
            }
            (')' | ']' | '"' | '\'' | '}', _) => prev_operand = true,
            _ => prev_operand = false,
        }
        i += 1;
    }
    m.decisions += arrows.saturating_sub(matches);
    m.cc = m.fns.max(1) + m.decisions;
    if m.fns == 0 && m.decisions == 0 && s.trim().is_empty() {
        m.cc = 0;
    }
    m
}

// ---------------------------------------------------------------- output

fn fmt_n(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn ratio(a: u64, b: u64) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

const CATS: [&str; 3] = ["System", "DevOps/Test", "Excluded"];

fn render(
    rows: &BTreeMap<(String, String), Row>,
    cat_lang: &BTreeMap<(String, String), u64>,
    scc_total: u64,
    big: &[(u64, String, String)],
    hot: &mut [(u64, u64, String)],
    excluded_why: &BTreeMap<String, String>,
) -> String {
    let mut s = String::new();
    let mut sums: BTreeMap<&str, Row> = BTreeMap::new();
    let _ = writeln!(s, "## Per-component rollup (Rust CC = extended McCabe; other languages = scc proxy)\n");
    let _ = writeln!(s, "| Component | Category | Files | LOC | Types | Fns | CC | CC/type | CC/fn | Density | `?` | Hottest file (CC) |");
    let _ = writeln!(s, "|---|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|---|");
    for cat in CATS {
        let mut list: Vec<_> = rows.iter().filter(|((_, c), _)| c == cat).collect();
        list.sort_by(|a, b| b.1.loc.cmp(&a.1.loc));
        let sum = sums.entry(cat).or_default();
        for ((comp, _), r) in &list {
            let _ = writeln!(
                s,
                "| {comp} | {cat} | {} | {} | {} | {} | {} | {:.1} | {:.2} | {:.3} | {} | {} ({}) |",
                fmt_n(r.files), fmt_n(r.loc), fmt_n(r.types), fmt_n(r.fns), fmt_n(r.cc),
                ratio(r.cc, r.types), ratio(r.cc, r.fns), ratio(r.cc, r.loc), fmt_n(r.qmarks),
                r.hottest, fmt_n(r.hottest_cc)
            );
            sum.files += r.files;
            sum.loc += r.loc;
            sum.types += r.types;
            sum.fns += r.fns;
            sum.cc += r.cc;
            sum.qmarks += r.qmarks;
        }
        let r = sum.clone();
        let _ = writeln!(
            s,
            "| **{cat} subtotal** | | **{}** | **{}** | **{}** | **{}** | **{}** | {:.1} | {:.2} | {:.3} | {} | |",
            fmt_n(r.files), fmt_n(r.loc), fmt_n(r.types), fmt_n(r.fns), fmt_n(r.cc),
            ratio(r.cc, r.types), ratio(r.cc, r.fns), ratio(r.cc, r.loc), fmt_n(r.qmarks)
        );
    }
    let sys = sums.get("System").map_or(0, |r| r.loc);
    let dev = sums.get("DevOps/Test").map_or(0, |r| r.loc);
    let exc = sums.get("Excluded").map_or(0, |r| r.loc);
    let _ = writeln!(s, "| **GRAND TOTAL (System + DevOps/Test)** | | | **{}** | | | | | | | | |", fmt_n(sys + dev));

    let _ = writeln!(s, "\n## Reconciliation\n");
    for cat in CATS {
        let parts: u64 = rows.iter().filter(|((_, c), _)| c == cat).map(|(_, r)| r.loc).sum();
        let sub = sums.get(cat).map_or(0, |r| r.loc);
        let _ = writeln!(s, "- Σ({cat} components) = {} == subtotal {} -> {}", fmt_n(parts), fmt_n(sub), if parts == sub { "OK" } else { "MISMATCH" });
    }
    let _ = writeln!(
        s,
        "- Σ(System) + Σ(DevOps/Test) + Σ(Excluded) = {} + {} + {} = {} == scc whole-repo code total {} -> {}",
        fmt_n(sys), fmt_n(dev), fmt_n(exc), fmt_n(sys + dev + exc), fmt_n(scc_total),
        if sys + dev + exc == scc_total { "OK" } else { "MISMATCH" }
    );
    let _ = writeln!(s, "- System / DevOps-Test split: {:.1}% / {:.1}%", 100.0 * ratio(sys, sys + dev), 100.0 * ratio(dev, sys + dev));

    let _ = writeln!(s, "\n## Excluded (non-authored / non-code) bucket\n");
    for (comp, why) in excluded_why {
        let loc = rows.get(&(comp.clone(), "Excluded".to_string())).map_or(0, |r| r.loc);
        let _ = writeln!(s, "- {comp}: {} LOC - {why}", fmt_n(loc));
    }

    let _ = writeln!(s, "\n## LOC by category and language (backfiring input)\n");
    let _ = writeln!(s, "| Category | Language | LOC |\n|---|---|--:|");
    for ((c, l), v) in cat_lang {
        let _ = writeln!(s, "| {c} | {l} | {} |", fmt_n(*v));
    }

    let _ = writeln!(s, "\n## Files over 2,000 code LOC (confirm each is authored source)\n");
    let mut big = big.to_vec();
    big.sort_by(|a, b| b.0.cmp(&a.0));
    for (loc, p, c) in &big {
        let _ = writeln!(s, "- {p}: {} LOC ({c})", fmt_n(*loc));
    }

    let _ = writeln!(s, "\n## Top 15 most complex Rust files (System + DevOps/Test; inline tests removed from System files)\n");
    let _ = writeln!(s, "| # | File | CC | LOC | Density |\n|--:|---|--:|--:|--:|");
    hot.sort_by(|a, b| b.0.cmp(&a.0));
    for (k, (cc, loc, p)) in hot.iter().take(15).enumerate() {
        let _ = writeln!(s, "| {} | {p} | {} | {} | {:.3} |", k + 1, fmt_n(*cc), fmt_n(*loc), ratio(*cc, *loc));
    }
    s
}

fn render_tsv(rows: &BTreeMap<(String, String), Row>) -> String {
    let mut s = String::from("component\tcategory\tfiles\tloc\ttypes\tfns\tcc\tqmarks\thottest\thottest_cc\n");
    for ((comp, cat), r) in rows {
        let _ = writeln!(s, "{comp}\t{cat}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", r.files, r.loc, r.types, r.fns, r.cc, r.qmarks, r.hottest, r.hottest_cc);
    }
    s
}

fn json_str(v: &str) -> String {
    let mut o = String::from("\"");
    for ch in v.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn render_json(rows: &BTreeMap<(String, String), Row>, scc_total: u64) -> String {
    let mut s = format!("{{\n  \"scc_code_total\": {scc_total},\n  \"components\": [\n");
    let n = rows.len();
    for (k, ((comp, cat), r)) in rows.iter().enumerate() {
        let _ = writeln!(
            s,
            "    {{\"component\": {}, \"category\": {}, \"files\": {}, \"loc\": {}, \"types\": {}, \"fns\": {}, \"cc\": {}, \"qmarks\": {}, \"hottest\": {}, \"hottest_cc\": {}}}{}",
            json_str(comp), json_str(cat), r.files, r.loc, r.types, r.fns, r.cc, r.qmarks, json_str(&r.hottest), r.hottest_cc,
            if k + 1 < n { "," } else { "" }
        );
    }
    s.push_str("  ]\n}\n");
    s
}
