use super::*;
use photocraft_doc::{Knot, NamedPath, Path, Subpath};

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-print-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn session(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 150, "mode": mode, "depth": depth, "name": "Print Me.psd", "background": "#3366cc"})).unwrap();
    s
}

fn find(h: &[u8], n: &[u8], from: usize) -> usize {
    from + h[from..].windows(n.len()).position(|w| w == n).unwrap()
}

/// The (inflated) image stream of a print PDF: (width, height, colour space, samples).
fn pdf_image(pdf: &[u8]) -> (u32, u32, String, Vec<u8>) {
    let i = find(pdf, b"/Subtype /Image", 0);
    let end = find(pdf, b">>\nstream\n", i);
    let dict = String::from_utf8_lossy(&pdf[i..end]).into_owned();
    let num = |k: &str| dict.split(k).nth(1).unwrap().split_whitespace().next().unwrap().parse::<u32>().unwrap();
    let cs = dict.split("/ColorSpace ").nth(1).unwrap().split(" /BitsPerComponent").next().unwrap().to_string();
    let len = num("/Length ") as usize;
    let start = end + 10;
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&pdf[start..start + len]), &mut out).unwrap();
    (num("/Width "), num("/Height "), cs, out)
}

#[test]
fn print_dry_run_renders_a_pdf_and_reports_lp() {
    let dir = tmp("print");
    for depth in [8, 16, 32] {
        let mut s = session("rgb", depth);
        let out = format!("{dir}/rgb-{depth}.pdf");
        let r = s
            .execute(
                "file.print",
                json!({"output": out, "send": true, "dryRun": true, "printer": "Office", "copies": 2, "paper": "a4", "cornerCropMarks": true, "registrationMarks": true, "labels": true}),
            )
            .unwrap();
        let cmd: Vec<String> = serde_json::from_value(r["command"].clone()).unwrap();
        assert_eq!(&cmd[..5], &["lp", "-d", "Office", "-n", "2"]);
        assert_eq!(cmd.last(), Some(&out));
        assert_eq!(r["sent"], false);
        assert_eq!(r["pdf"], out);
        let pdf = std::fs::read(&out).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox [0 0 595.28 841.89]"));
        let (w, h, cs, data) = pdf_image(&pdf);
        assert_eq!((w, h), (300, 150));
        assert_eq!(cs, "/DeviceRGB");
        assert_eq!(&data[..3], &[0x33, 0x66, 0xcc]);
        // 300 px at 72 ppi = 300 pt, centred.
        let rect: Vec<f64> = serde_json::from_value(r["imageRect"].clone()).unwrap();
        assert!((rect[2] - 300.0).abs() < 1e-6 && (rect[0] - (595.28 - 300.0) / 2.0).abs() < 1e-6);
    }
    // Print to PDF: no spooling by default.
    let mut s = session("gray", 8);
    let out = format!("{dir}/out.pdf");
    let r = s.execute("file.print", json!({"output": out, "scaleToFit": true, "orientation": "landscape"})).unwrap();
    assert_eq!(r["sent"], false);
    assert!(r["command"].is_null());
    let (_, _, cs, _) = pdf_image(&std::fs::read(&out).unwrap());
    assert_eq!(cs, "/DeviceGray");
    let scale = r["scale"].as_f64().unwrap();
    assert!(scale > 200.0, "fit to a landscape letter page: {scale}");
    assert!(s.execute("file.print", json!({"paper": "napkin", "dryRun": true})).is_err());
    assert!(s.execute("file.print", json!({"colorHandling": "photocraftManages", "dryRun": true})).is_err(), "needs a printer profile");
}

#[test]
fn photocraft_manages_colors_converts_to_the_printer_profile() {
    let mut s = session("rgb", 8);
    let out = format!("{}/managed.pdf", tmp("managed"));
    let r = s
        .execute(
            "file.print",
            json!({"output": out, "dryRun": true, "colorHandling": "photocraftManages", "printerProfile": "coated-cmyk", "intent": "perceptual", "bpc": false}),
        )
        .unwrap();
    assert_eq!(r["color"]["intent"], "perceptual");
    let pdf = std::fs::read(&out).unwrap();
    let (_, _, cs, data) = pdf_image(&pdf);
    assert_eq!(cs, "[/ICCBased 7 0 R]");
    assert!(String::from_utf8_lossy(&pdf).contains("/N 4 /Alternate /DeviceCMYK"));
    assert_eq!(data.len(), 300 * 150 * 4);
    // Blue: lots of cyan, little yellow.
    assert!(data[0] > 150 && data[2] < 60, "{:?}", &data[..4]);
    // The document itself is untouched.
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Rgb);
}

#[test]
fn print_one_copy_repeats_the_last_settings() {
    let mut s = session("rgb", 8);
    let first = s.execute("file.print", json!({"dryRun": true, "copies": 3, "paper": "a5", "scale": 50})).unwrap();
    assert!(first["pdf"].is_null());
    let r = s.execute("file.printOneCopy", json!({"dryRun": true})).unwrap();
    assert_eq!(r["copies"], 1);
    assert_eq!(r["paper"], json!([419.53, 595.28]));
    assert_eq!(r["scale"], 50.0);
    let cmd: Vec<String> = serde_json::from_value(r["command"].clone()).unwrap();
    assert!(!cmd.contains(&"-n".to_string()));
    assert_eq!(cmd.last().map(String::as_str), Some("<temporary PDF>"));
    assert!(r["pdf"].is_null());
}

#[test]
fn print_without_output_keeps_private_pdf_only_during_spooling() {
    let mut s = session("rgb", 8);
    let mut temporary_path = None;
    let result = do_print_with_spooler(&mut s, &json!({}), "file.print", |argv| {
        let path = std::path::PathBuf::from(argv.last().unwrap());
        let data = std::fs::read(&path).unwrap();
        assert!(data.starts_with(b"%PDF-1.4"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
        }
        temporary_path = Some(path);
        Ok("request id 1".into())
    })
    .unwrap();
    assert_eq!(result["sent"], true);
    assert!(result["pdf"].is_null(), "the temporary path must not be advertised as a retained PDF");
    assert!(!temporary_path.unwrap().exists(), "spool PDF survives after lp returns");
}

#[test]
fn print_spool_error_removes_private_pdf() {
    let mut s = session("rgb", 8);
    let mut temporary_path = None;
    let result = do_print_with_spooler(&mut s, &json!({}), "file.print", |argv| {
        let path = std::path::PathBuf::from(argv.last().unwrap());
        assert!(path.is_file());
        temporary_path = Some(path);
        Err(other("spooler unavailable"))
    });
    assert!(result.is_err());
    assert!(!temporary_path.unwrap().exists(), "spool PDF survives a failed lp call");
}

#[cfg(unix)]
#[test]
fn private_spool_pdf_does_not_follow_a_guessed_symlink() {
    let dir = tmp("symlink");
    let dir = std::path::Path::new(&dir);
    let victim = dir.join("victim.pdf");
    let link = dir.join("photocraft-print-AAAAAA.pdf");
    std::fs::write(&victim, b"keep this").unwrap();
    std::os::unix::fs::symlink(&victim, &link).unwrap();

    let file = private_spool_pdf_in(b"%PDF-private", dir).unwrap();
    assert_ne!(file.path(), link);
    assert_eq!(std::fs::read(&victim).unwrap(), b"keep this");
    assert_eq!(std::fs::read(file.path()).unwrap(), b"%PDF-private");
    assert!(std::fs::symlink_metadata(file.path()).unwrap().file_type().is_file());
    let path = file.path().to_path_buf();
    drop(file);
    assert!(!path.exists());
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
}

#[test]
fn print_dry_run_without_output_creates_no_spool_pdf() {
    let mut s = session("rgb", 8);
    let result = do_print_with_spooler(&mut s, &json!({"dryRun": true}), "file.print", |_| Err(other("dry run invoked spooler"))).unwrap();
    assert!(result["pdf"].is_null());
    assert_eq!(result["sent"], false);
    assert_eq!(result["command"].as_array().unwrap().last().unwrap(), "<temporary PDF>");
    assert!(result["bytes"].as_u64().unwrap() > 100);
}

#[test]
fn print_without_sending_requires_an_output_unless_dry_run() {
    let mut s = session("rgb", 8);
    let before = s.file_menu.last_print.clone();
    let err = do_print_with_spooler(&mut s, &json!({"send": false}), "file.print", |_| Err(other("unexpected spooler call"))).unwrap_err();
    assert!(err.to_string().contains("needs an output path"), "{err}");
    assert_eq!(s.file_menu.last_print, before, "a rejected print must not become a remembered successful print");
    assert!(do_print_with_spooler(&mut s, &json!({"send": false, "dryRun": true}), "file.print", |_| Err(other("dry run invoked spooler"))).is_ok());
    assert!(s.file_menu.last_print.as_ref().unwrap().get("send").is_none());

    let output = format!("{}/export.pdf", tmp("non-sending-export"));
    do_print_with_spooler(&mut s, &json!({"output": output, "send": false}), "file.print", |_| Err(other("non-sending export invoked spooler"))).unwrap();
    assert!(std::path::Path::new(&output).is_file());
    let mut repeat = s.file_menu.last_print.clone().unwrap();
    assert!(repeat.get("send").is_none(), "a removed output must not leave send: false in remembered settings");
    repeat["copies"] = json!(1);
    let replay = do_print_with_spooler(&mut s, &repeat, "file.printOneCopy", |argv| {
        assert!(std::path::Path::new(argv.last().unwrap()).is_file());
        Ok("request id 2".into())
    })
    .unwrap();
    assert_eq!(replay["sent"], true);
    assert_eq!(replay["copies"], 1);
}

#[test]
fn package_copies_links_and_relinks() {
    let dir = tmp("package");
    let link = format!("{dir}/art.png");
    let mut src = Session::new();
    src.execute("file.new", json!({"width": 20, "height": 20, "background": "#ff0000"})).unwrap();
    crate::file_cmds::save_doc(&src.active().unwrap().doc, &link, None).unwrap();
    let mut s = session("rgb", 8);
    s.execute("file.placeLinked", json!({"path": link})).unwrap();
    let r = s.execute("file.package", json!({"dir": format!("{dir}/pkg")})).unwrap();
    let copied = format!("{dir}/pkg/Print Me/Links/art.png");
    assert_eq!(r["links"], json!([copied]));
    assert!(std::path::Path::new(&copied).is_file());
    let docp = r["document"].as_str().unwrap();
    assert!(docp.ends_with("Print Me/Print Me.pcraft"));
    let back = photocraft_io::import("Print Me.pcraft", &std::fs::read(docp).unwrap()).unwrap().document;
    let linked: Vec<String> = back
        .walk()
        .iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Smart(so) => match &so.source {
                SmartSource::Linked { path } => Some(path.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(linked.len(), 1);
    assert!(linked[0].ends_with("Links/art.png"), "{linked:?}");
    // The open document still points at the original.
    let open = &s.active().unwrap().doc;
    assert!(
        open.walk()
            .iter()
            .any(|(_, _, l)| matches!(&l.content, LayerContent::Smart(so) if matches!(&so.source, SmartSource::Linked { path } if *path == link)))
    );
}

#[test]
fn paths_export_as_illustrator_postscript() {
    let mut s = session("rgb", 8);
    assert!(!s.is_enabled("file.export.pathsToIllustrator"));
    s.edit("paths", |d, _| {
        let tri = Subpath::polygon(&[(10.0, 10.0), (100.0, 10.0), (50.0, 90.0)]);
        let mut curve = Subpath::polyline(&[(0.0, 0.0), (300.0, 150.0)]);
        curve.knots[0] = Knot::smooth(photocraft_geom::Point::new(0.0, 0.0), photocraft_geom::Point::new(0.0, 0.0), photocraft_geom::Point::new(50.0, 0.0));
        d.paths.push(NamedPath { name: "Shapes".into(), path: Path::new(vec![tri, curve]), psd_raw: None });
        Ok(())
    })
    .unwrap();
    let r = s.execute("file.export.pathsToIllustrator", json!({})).unwrap();
    let ai = r["ai"].as_str().unwrap();
    assert!(ai.starts_with("%!PS-Adobe-2.0 EPSF-1.2"));
    assert!(ai.contains("%%BoundingBox: 0 0 300 150"));
    assert!(ai.contains("*u\n10.0000 140.0000 m\n100.0000 140.0000 L\n"), "{ai}");
    assert!(ai.contains("\nn\n") && ai.contains("\nN\n*U\n"));
    assert!(ai.contains(" C\n"), "curve segment");
    let dir = tmp("ai");
    let r = s.execute("file.export.pathsToIllustrator", json!({"path": format!("{dir}/p.ai"), "paths": "Shapes"})).unwrap();
    assert_eq!(r["paths"], 1);
    assert!(s.execute("file.export.pathsToIllustrator", json!({"paths": "Nope"})).is_err());
}
