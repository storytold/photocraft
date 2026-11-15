//! #13779: the Paths to Illustrator file follows the reference application's layout: its header
//! comments, prolog, setup and graphics state, the written file's name as title, whole-point
//! boxes, CR line endings and no break after `%%EOF`.

use super::*;
use photocraft_doc::{NamedPath, Path, Subpath};

const CMD: &str = "file.export.pathsToIllustrator";

fn session(w: u32, h: u32, dpi: f32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "name": "Header.psd"})).unwrap();
    s.edit("resolution", |d, _| {
        d.resolution_dpi = dpi;
        Ok(())
    })
    .unwrap();
    s
}

/// Writes the file to `name` in a scratch folder and returns its bytes as text.
fn written(s: &mut Session, name: &str, paths: &str) -> String {
    let dir = std::env::temp_dir().join(format!("pc-ai-header-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    s.execute(CMD, json!({"path": file.to_string_lossy(), "paths": paths})).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    text
}

#[test]
fn document_bounds_is_the_reference_file_with_cr_line_endings() {
    let mut s = session(300, 200, 72.0);
    let want = [
        "%!PS-Adobe-2.0",
        "%%Creator: PhotoCraft",
        "%%Title: (ui-docbounds.ai)",
        "%%DocumentNeededResources: procset Adobe_packedarray 2.0 0",
        "%%+ procset Adobe_IllustratorA_AI3 1.0 1",
        "%%ColorUsage: Black&White",
        "%%BoundingBox: 0 0 300 200",
        "%%HiResBoundingBox: 0 0 300 200",
        "%AI3_Cropmarks: 0 0 300 200",
        "%%DocumentPreview: None",
        "%%EndComments",
        "%%BeginProlog",
        "%%IncludeResource: procset Adobe_packedarray 2.0 0",
        "Adobe_packedarray /initialize get exec",
        "%%IncludeResource: procset Adobe_IllustratorA_AI3 1.0 1",
        "%%EndProlog",
        "%%BeginSetup",
        "Adobe_IllustratorA_AI3 /initialize get exec",
        "n",
        "%%EndSetup",
        "0.0 0.0 0.0 1.0 k",
        "0 i 0 J 0 j 1 w 4 M []0 d",
        "%%Note:",
        "%%Trailer",
        "%%EOF",
    ]
    .join("\r");
    let text = written(&mut s, "ui-docbounds.ai", "documentBounds");
    assert_eq!(text, want);
    assert!(!text.contains('\n') && !text.ends_with('\r'), "CR only, no break after %%EOF");
    assert!(!text.contains("%%DocumentProcessColors") && !text.contains("%%PageTrailer"));
}

#[test]
fn the_boxes_are_the_canvas_in_whole_points() {
    let boxes = |s: &mut Session| {
        let r = s.execute(CMD, json!({"paths": "documentBounds"})).unwrap();
        r["ai"].as_str().unwrap().split('\r').filter(|l| l.contains("BoundingBox") || l.starts_with("%AI3_Cropmarks")).map(str::to_string).collect::<Vec<_>>()
    };
    assert_eq!(boxes(&mut session(300, 200, 300.0)), ["%%BoundingBox: 0 0 72 48", "%%HiResBoundingBox: 0 0 72 48", "%AI3_Cropmarks: 0 0 72 48"]);
    let mut s = session(300, 200, 72.0);
    s.execute("image.canvasSize", json!({"width": 400, "height": 300, "anchor": "center"})).unwrap();
    assert_eq!(boxes(&mut s)[0], "%%BoundingBox: 0 0 400 300");
    // Not measured: a size that isn't whole points keeps today's decimals, the box rounded up.
    assert_eq!(boxes(&mut session(301, 200, 300.0)), ["%%BoundingBox: 0 0 73 48", "%%HiResBoundingBox: 0 0 72.2400 48", "%AI3_Cropmarks: 0 0 72.2400 48"]);
}

#[test]
fn the_title_is_the_written_files_name_and_cannot_break_the_line() {
    let mut s = session(300, 200, 72.0);
    // Portable: the parentheses of a file name cannot end the title early.
    let text = written(&mut s, "a(b)c.ai", "documentBounds");
    assert!(text.contains("\r%%Title: (a_b_c.ai)\r"), "{text}");
    // A backslash is part of a file name only where it is not a separator (Windows splits the path there).
    #[cfg(unix)]
    {
        let text = written(&mut s, "a(b)c\\d.ai", "documentBounds");
        assert!(text.contains("\r%%Title: (a_b_c_d.ai)\r"), "{text}");
    }
    // Without a file the title keeps the document's name.
    let r = s.execute(CMD, json!({"paths": "documentBounds"})).unwrap();
    assert!(r["ai"].as_str().unwrap().contains("\r%%Title: (Header.psd)\r"));
}

#[test]
fn path_blocks_sit_after_the_note_with_unchanged_coordinates() {
    let mut s = session(300, 200, 72.0);
    s.edit("paths", |d, _| {
        d.paths.push(NamedPath { name: "Tri".into(), path: Path::new(vec![Subpath::polygon(&[(10.0, 10.0), (100.0, 10.0), (50.0, 90.0)])]), psd_raw: None });
        Ok(())
    })
    .unwrap();
    let text = written(&mut s, "tri.ai", "Tri");
    let note = text.find("%%Note:\r").unwrap();
    let block = text.find("%Adobe_Photoshop_Path_Begin").unwrap();
    assert!(note < block && block < text.find("%%Trailer").unwrap());
    assert!(text.contains("\r10.0000 190.0000 m\r100.0000 190.0000 L\r"), "points, y up, as before: {text}");
    assert!(text.ends_with("%%Trailer\r%%EOF"));
}
