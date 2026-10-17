//! Opens a file the way the app's background open does and reports the time. Run it under
//! `/usr/bin/time -l` for the peak memory. `--read` reads the whole file first and imports the
//! bytes, as opens did before `import_file` (to compare).
//!
//! `--preview` only reads the merged-image preview the open card shows (`psd_preview`).
//! `--save <out.psd>` then saves the document as PSD and reopens it, checking that its embedded
//! smart-object files (`lnk2`, left in the source by the open) came through unchanged.
//!
//! `cargo run --release -p photocraft-io --example psd_open_mem -- <file.psd> [--read] [--save <out.psd>]`

fn main() {
    let Some(path) = std::env::args().nth(1).filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: psd_open_mem <file.psd>");
        std::process::exit(2);
    };
    if std::env::args().any(|a| a == "--preview") {
        let t = std::time::Instant::now();
        let p = photocraft_io::psd_preview(std::path::Path::new(&path), 1024);
        println!("{path}: preview {:?} in {:.3}s", p.map(|p| (p.width, p.height)), t.elapsed().as_secs_f64());
        return;
    }
    if std::env::args().any(|a| a == "--read") {
        read_then_import(&path);
        return;
    }
    let t = std::time::Instant::now();
    let res = match photocraft_io::import_file(&path, std::path::Path::new(&path), photocraft_doc::MAX_GROUP_DEPTH, &photocraft_raster::Interrupt::NONE) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{path}: open failed: {e}");
            std::process::exit(1);
        }
    };
    report(&path, &format!("open {:.2}s", t.elapsed().as_secs_f64()), &res);
    let args: Vec<String> = std::env::args().collect();
    if let Some(out) = args.iter().position(|a| a == "--save").and_then(|i| args.get(i + 1)) {
        save_and_check(&res.document, out);
    }
}

fn embedded(doc: &photocraft_doc::Document) -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = doc
        .metadata
        .psd_global_blocks
        .iter()
        .filter(|(_, k, _)| matches!(k, b"lnk2" | b"lnk3" | b"lnkD"))
        .flat_map(|(_, _, d)| photocraft_io::linked::parse_linked_files(&d.bytes().unwrap_or_default()))
        .map(|f| (f.uuid, f.bytes.len() as u64))
        .collect();
    v.sort();
    v
}

fn save_and_check(doc: &photocraft_doc::Document, out: &str) {
    let t = std::time::Instant::now();
    let saved = match photocraft_io::export(doc, out, &Default::default()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{out}: save failed: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = std::fs::write(out, &saved.bytes) {
        eprintln!("{out}: write failed: {e}");
        std::process::exit(1);
    }
    drop(saved);
    println!("saved {out} in {:.2}s", t.elapsed().as_secs_f64());
    let back = match photocraft_io::import_file(out, std::path::Path::new(out), photocraft_doc::MAX_GROUP_DEPTH, &photocraft_raster::Interrupt::NONE) {
        Ok(r) => r.document,
        Err(e) => {
            eprintln!("{out}: reopen failed: {e}");
            std::process::exit(1);
        }
    };
    let (a, b) = (embedded(doc), embedded(&back));
    println!("embedded files: {} before, {} after, same: {}; layers {} / {}", a.len(), b.len(), a == b, doc.walk().len(), back.walk().len());
    if a != b || doc.walk().len() != back.walk().len() {
        std::process::exit(1);
    }
}

fn read_then_import(path: &str) {
    let t = std::time::Instant::now();
    let bytes = match photocraft_format::read_file(std::path::Path::new(path)) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{path}: read failed: {e}");
            std::process::exit(1);
        }
    };
    let read_s = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    let res = match photocraft_io::import_with(path, &bytes, &photocraft_raster::Interrupt::NONE) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{path}: open failed: {e}");
            std::process::exit(1);
        }
    };
    drop(bytes);
    report(path, &format!("read {read_s:.2}s, import {:.2}s", t.elapsed().as_secs_f64()), &res);
}

fn report(path: &str, times: &str, res: &photocraft_io::ImportResult) {
    let doc = &res.document;
    println!("{path}: {times}, {} layers ({} top level)", doc.walk().len(), doc.layers.len());
    for w in &res.warnings {
        println!("  warning: {w}");
    }
}
