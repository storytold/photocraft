use super::*;

#[test]
fn save_for_web_overwrites_invalidate_only_after_a_successful_write() {
    for format in ["png24", "jpeg", "gif", "webp", "wbmp"] {
        for (target, fail) in [("own.png", false), ("other.png", false), ("own.png", true)] {
            let mut s = session(8);
            s.active_mut().unwrap().saved_to("own.png".into());
            let before = s.active().unwrap().saved_revision;
            let params = json!({"path": target, "format": format, "width": 16, "height": 12});
            let r = save_for_web_with_writer(&mut s, &params, &mut |_, _| {
                if fail { Err(EngineError::Other("disk full".into())) } else { Ok(()) }
            });
            assert_eq!(r.is_err(), fail, "{format}: {r:?}");
            let st = s.active().unwrap();
            if !fail && target == "own.png" {
                assert!(st.is_dirty(), "{format}: the 64x48 original no longer exists on disk");
            } else {
                assert_eq!(st.saved_revision, before, "{format}: another destination or a failed write preserves the savepoint");
            }
        }
    }
}

#[test]
fn quick_export_overwrites_invalidate_the_open_file_in_every_format() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["png", "jpg", "gif", "webp", "exr"] {
        let path = dir.path().join(format!("own.{ext}")).to_string_lossy().into_owned();
        let mut s = session(8);
        s.active_mut().unwrap().saved_to(path.clone());
        s.execute("file.export.quickExport", json!({"path": path})).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
        assert!(s.active().unwrap().is_dirty(), "{ext}: export options may change the saved composite");
    }
}
