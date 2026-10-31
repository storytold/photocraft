//! The runner against stand-in `gmic` programs (shell scripts, Unix only), and opt-in checks
//! against a real G'MIC: `GMIC_TEST=1 cargo test -p photocraft-gmic -- --ignored`
//! (the binary from PATH, or `PHOTOCRAFT_GMIC=/path/to/gmic`).
#![cfg(not(target_arch = "wasm32"))]

use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_gmic::{Error, Gmic, Image, RunOptions, runner};
use photocraft_raster::Surface;

#[cfg(unix)]
mod fake {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    /// A stand-in `gmic`: prints a version for `echo ${-strver}`, else runs `body` in the run's
    /// folder (`in.cimg` in, `out.cimg` out).
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        install(&p, &format!("#!/bin/sh\nif [ \"$3\" = echo ]; then echo 9.9.9 >&2; exit 0; fi\n{body}\n"));
        p
    }

    /// Writes an executable `text` to `path` from a `cp` child, never from this process: a test
    /// thread that forks while another holds a script open for writing keeps that handle until
    /// its child execs, and running the script then fails with "Text file busy" (ETXTBSY).
    fn install(path: &Path, text: &str) {
        let staged = path.with_extension("staged");
        std::fs::write(&staged, text).unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(std::process::Command::new("cp").arg(&staged).arg(path).status().unwrap().success());
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("photocraft-gmic-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn probe(p: &Path) -> Gmic {
        Gmic::probe(p, &RunOptions::default()).unwrap()
    }

    fn image() -> Image {
        Image { width: 3, height: 2, channels: 1, data: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0] }
    }

    #[test]
    fn probes_the_version() {
        let d = dir("probe");
        let g = probe(&script(&d, "gmic", "exit 0"));
        assert_eq!(g.version(), "9.9.9");
        assert_eq!(runner::locate(Some(g.path().to_str().unwrap())).unwrap(), g.path());
        assert!(matches!(runner::locate(Some("/no/such/gmic")), Err(Error::NotFound(_))));
        let bad = script(&d, "notgmic", "exit 0");
        install(&bad, "#!/bin/sh\nexit 0\n");
        assert!(matches!(Gmic::probe(&bad, &RunOptions::default()), Err(Error::NotFound(_))));
    }

    #[test]
    fn passes_images_and_the_pipeline_through_files() {
        let d = dir("echo");
        // Copies the input back, and records the command file it was given.
        let g = probe(&script(&d, "gmic", &format!("cp in.cimg out.cimg; cp run.gmic {}/seen.gmic", d.display())));
        let out = g.run(&[image()], "mirror x\nblur 2", &RunOptions::default()).unwrap();
        assert_eq!(out, vec![image()]);
        let seen = std::fs::read_to_string(d.join("seen.gmic")).unwrap();
        assert_eq!(seen, "photocraft_run :\n  mirror x\n  blur 2\n");
    }

    #[test]
    fn reports_gmic_errors() {
        let d = dir("fail");
        let g = probe(&script(&d, "gmic", "printf '\\033[31m[gmic] *** Error in ./ *** Unknown command or filename '\"'\"'nope'\"'\"'.\\033[0m\\n' >&2; exit 255"));
        let e = g.run(&[image()], "nope", &RunOptions::default()).unwrap_err();
        assert_eq!(e, Error::Failed("Unknown command or filename 'nope'.".into()));
        // Success without an output file, and garbage output, are errors too.
        let g = probe(&script(&d, "gmic2", "exit 0"));
        assert!(matches!(g.run(&[image()], "x", &RunOptions::default()), Err(Error::Failed(_))));
        let g = probe(&script(&d, "gmic3", "echo junk > out.cimg"));
        assert!(matches!(g.run(&[image()], "x", &RunOptions::default()), Err(Error::Cimg(_))));
        assert!(matches!(g.run(&[image()], "  ", &RunOptions::default()), Err(Error::Params(_))));
        assert!(matches!(g.run(&[image()], "a\0b", &RunOptions::default()), Err(Error::Params(_))));
    }

    #[test]
    fn stops_a_hung_run_and_cleans_up() {
        let d = dir("hang");
        let g = probe(&script(&d, "gmic", &format!("pwd > {}/cwd; sleep 30", d.display())));
        let t = Instant::now();
        let e = g.run(&[image()], "x", &RunOptions::default().with_timeout(Duration::from_millis(300))).unwrap_err();
        assert_eq!(e, Error::Timeout(0));
        assert!(t.elapsed() < Duration::from_secs(10));
        let cancel = || true;
        let e = g.run(&[image()], "x", &RunOptions { cancelled: Some(&cancel), ..Default::default() }).unwrap_err();
        assert_eq!(e, Error::Cancelled);
        let cwd = std::fs::read_to_string(d.join("cwd")).unwrap();
        assert!(!Path::new(cwd.trim()).exists(), "the run's temporary folder is removed");
    }

    #[test]
    fn floods_of_output_dont_block() {
        let d = dir("flood");
        let g = probe(&script(&d, "gmic", "head -c 3000000 /dev/zero; head -c 3000000 /dev/zero >&2; cp in.cimg out.cimg"));
        assert_eq!(g.run(&[image()], "x", &RunOptions::default()).unwrap(), vec![image()]);
    }

    #[test]
    fn apply_respects_the_selection_and_keeps_alpha() {
        let d = dir("apply");
        // Writes a 4×4 RGB (alpha dropped) image of 255s: white everywhere.
        let mut out = b"1 float32 little_endian\n4 4 1 3\n".to_vec();
        for _ in 0..48 {
            out.extend_from_slice(&255f32.to_le_bytes());
        }
        let fixture = d.join("white.cimg");
        std::fs::write(&fixture, out).unwrap();
        let g = probe(&script(&d, "gmic", &format!("cp {} out.cimg", fixture.display())));
        for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 4, 4), &[0.0, 0.0, 0.0, 0.5]);
            let mut sel = Surface::new(PixelFormat::GRAY8);
            sel.fill_rect(Rect::new(0, 0, 2, 4), &[1.0]);
            let r = photocraft_gmic::apply(&g, &s, Rect::new(0, 0, 4, 4), Some(&sel), "x", &RunOptions::default()).unwrap();
            let white = r.pixel(1, 1);
            assert!((white[0] - 1.0).abs() < 1e-3 && (white[3] - 0.5).abs() < 1e-2, "{fmt:?}: {white:?}");
            assert!(r.pixel(3, 1)[0].abs() < 1e-6, "outside the selection is unchanged");
        }
        // A filter that changes the size is refused.
        let g = probe(&script(&d, "gmic2", &format!("cp {} out.cimg", fixture.display())));
        let s = Surface::new(PixelFormat::RGBA8);
        let e = photocraft_gmic::apply(&g, &s, Rect::new(0, 0, 8, 8), None, "x", &RunOptions::default()).unwrap_err();
        assert!(e.to_string().contains("changed the image size"), "{e}");
        let cmyk = Surface::new(PixelFormat::CMYKA8);
        assert!(photocraft_gmic::apply(&g, &cmyk, Rect::new(0, 0, 4, 4), None, "x", &RunOptions::default()).is_err());
    }
}

fn real() -> Option<Gmic> {
    if std::env::var_os("GMIC_TEST").is_none() {
        eprintln!("skipped: set GMIC_TEST=1 (and install G'MIC) to run");
        return None;
    }
    Some(runner::find(None).expect("GMIC_TEST is set but G'MIC wasn't found"))
}

#[test]
#[ignore = "needs G'MIC installed: GMIC_TEST=1"]
fn real_gmic_round_trips_and_filters() {
    let Some(g) = real() else { return };
    eprintln!("G'MIC {} at {}", g.version(), g.path().display());
    let img = Image { width: 3, height: 2, channels: 2, data: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0] };
    let out = g.run(std::slice::from_ref(&img), "mirror x add 100", &RunOptions::default()).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].get(0, 0, 0), Some(103.0));
    assert_eq!(out[0].get(2, 1, 1), Some(110.0));
    // Text values are quoted so they can't break out of their string or substitute.
    let e = g.run(&[], "echo \"a\\\"b\\$x\\{1\\}\" 1,1,1,1", &RunOptions::default());
    assert!(e.is_ok(), "{e:?}");
    let err = g.run(std::slice::from_ref(&img), "no_such_command_xyz", &RunOptions::default()).unwrap_err();
    assert!(err.to_string().contains("no_such_command_xyz"), "{err}");
    // A G'MIC-Qt filter from the stdlib keeps the image size and channels.
    let s = {
        let mut s = Surface::new(PixelFormat::RGBA16);
        s.fill_rect(Rect::new(0, 0, 32, 24), &[0.2, 0.4, 0.6, 1.0]);
        s.fill_rect(Rect::new(8, 8, 16, 16), &[0.9, 0.1, 0.1, 1.0]);
        s
    };
    let r = photocraft_gmic::apply(&g, &s, Rect::new(0, 0, 32, 24), None, "fx_kaleidoscope 50,50,0,0,100,10,3", &RunOptions::default()).unwrap();
    assert_eq!(r.content_bounds(), Rect::new(0, 0, 32, 24));
}

#[test]
#[ignore = "needs G'MIC installed and network for the first `gmic update`: GMIC_TEST=1"]
fn real_gmic_catalogue() {
    let Some(g) = real() else { return };
    let dir = std::env::temp_dir().join(format!("photocraft-gmic-cat-{}", std::process::id()));
    let cat = g.load_catalog(Some(&dir), false, &RunOptions::default()).unwrap();
    eprintln!("{} filters in {} categories", cat.filters.len(), cat.categories().len());
    assert!(cat.filters.len() > 100);
    let f = cat.filters.iter().find(|f| f.command == "fx_kaleidoscope").expect("Kaleidoscope [Polar]");
    assert!(f.command_line(&serde_json::Value::Null).unwrap().starts_with("fx_kaleidoscope 50,50,"));
    // Every filter's default command line is well formed.
    for f in &cat.filters {
        f.command_line(&serde_json::Value::Null).unwrap();
    }
    // Second load comes from the cache.
    let t = std::time::Instant::now();
    assert_eq!(g.load_catalog(Some(&dir), false, &RunOptions::default()).unwrap(), cat);
    assert!(t.elapsed() < std::time::Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(dir);
}
