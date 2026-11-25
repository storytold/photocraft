use super::*;

fn new_document(app: &mut PhotocraftApp, mode: &str, depth: u8) {
    app.run("file.new", json!({"width": 4, "height": 4, "name": "Sample", "mode": mode, "depth": depth, "background": "transparent"})).unwrap();
}

fn formats(files: &ScriptedFiles) -> Vec<String> {
    last_request(files).filters.into_iter().map(|f| f.extensions[0].clone()).collect()
}

#[test]
fn rgb8_save_copy_omits_only_the_three_other_depth_formats() {
    let (mut app, files, _) = app();
    let ctx = app.ctx().clone();
    new_document(&mut app, "rgb", 8);
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("channel.new", json!({"name": "Alpha"})).unwrap();
    app.run("channel.newSpot", json!({"name": "Spot"})).unwrap();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    let mut expected = formats(&files);
    expected.retain(|ext| !matches!(ext.as_str(), "cin" | "exr" | "hdr"));
    cancel(&files);
    app.poll_file_dialog();

    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    assert_eq!(formats(&files), expected);
}

#[test]
fn rgb32_save_copy_offers_float_formats_and_defaults_to_psd() {
    let (mut app, files, _) = app();
    let ctx = app.ctx().clone();
    new_document(&mut app, "rgb", 32);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    let save_as = formats(&files);
    assert!(save_as.iter().any(|ext| ext == "jpg"));
    assert!(save_as.iter().any(|ext| ext == "png"));
    cancel(&files);
    app.poll_file_dialog();

    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    assert_eq!(asked_save(&files), "Sample.psd");
    assert_eq!(formats(&files), ["psd", "psb", "pcraft", "fits", "pnm", "exr", "hdr", "tif"]);
}

#[test]
fn cmyk8_save_copy_offers_ink_preserving_formats() {
    let (mut app, files, _) = app();
    let ctx = app.ctx().clone();
    new_document(&mut app, "cmyk", 8);
    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    assert_eq!(formats(&files), ["psd", "psb", "pcraft", "jpg", "pdf", "tif"]);
}

#[test]
fn other_modes_and_depths_keep_the_save_as_format_list() {
    let (mut app, files, _) = app();
    let ctx = app.ctx().clone();
    for (mode, depth) in [("rgb", 16), ("cmyk", 16), ("gray", 8), ("lab", 8)] {
        new_document(&mut app, mode, depth);
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
        let expected = formats(&files);
        cancel(&files);
        app.poll_file_dialog();

        menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
        assert_eq!(formats(&files), expected, "{mode}/{depth}");
        cancel(&files);
        app.poll_file_dialog();
    }
}

#[test]
fn explicit_path_save_copy_still_writes_a_converting_format() {
    let dir = tempfile::tempdir().unwrap();
    let jpg = dir.path().join("copy.jpg");
    let (mut app, files, _) = app();
    new_document(&mut app, "rgb", 32);
    app.session.execute("file.saveACopy", json!({"path": jpg})).unwrap();
    let bytes = std::fs::read(jpg).unwrap();
    assert_eq!(photocraft_codecs::detect(&bytes), Some(photocraft_codecs::Format::Jpeg));
    assert!(files.requests().is_empty());
}

#[test]
fn typing_an_unavailable_format_into_save_copy_does_not_write_it() {
    let (mut app, files, written) = app();
    let ctx = app.ctx().clone();
    new_document(&mut app, "rgb", 32);
    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    answer(&files, chose("/pics/copy.JPG"));
    app.poll_file_dialog();
    assert!(app.ui.status_error, "the user gets an error for an unavailable format");
    assert!(!app.dialog_open(), "there is no JPEG Options prompt");
    assert!(written.borrow().is_empty());

    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    answer(&files, chose("/pics/copy.PSB"));
    app.poll_file_dialog();
    assert_eq!(*written.borrow(), ["/pics/copy.PSB"]);
}

#[test]
fn typed_compatible_extensions_reach_the_copy_writer() {
    for (depth, path) in [(32, "/pics/copy.FTS"), (8, "/pics/copy.qoi")] {
        let (mut app, files, written) = app();
        let ctx = app.ctx().clone();
        new_document(&mut app, "rgb", depth);
        menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
        answer(&files, chose(path));
        app.poll_file_dialog();
        assert_eq!(*written.borrow(), [path]);
    }
}

#[test]
fn a_disallowed_suggestion_falls_back_to_psd_in_the_same_folder() {
    let request = save_request("document", "scans/image.JPG", Some(SaveCopyFormats::Rgb32));
    assert_eq!(request.file_name.as_deref(), Some("image.psd"));
    assert_eq!(request.suggested_extension.as_deref(), Some("psd"));
    assert_eq!(request.directory.as_deref(), Some(Path::new("scans")));
    assert!(request.filters[0].extensions.iter().any(|ext| ext == "psd"));
    assert!(request.filters.iter().all(|filter| filter.extensions.iter().all(|ext| ext != "jpg")));
}
