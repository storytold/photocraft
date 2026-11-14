//! Artboards to Files › Include Overlapping Areas / Artboard Content Only (#13629): the boards
//! stacked above an exported board that overlap it show in its file (PNG pixels, PSD structure),
//! boards below never do, and the source document is left alone.

use super::*;
use photocraft_geom::Rect;

const RED: [u8; 3] = [220, 30, 30];
const BLUE: [u8; 3] = [30, 30, 220];
const WHITE: [u8; 3] = [255, 255, 255];

/// The measured test document: 800 x 400, transparent, bottom to top `Red Board` 0,0-200,150
/// (pixel layer `Red` at 20,20-120,100), `Blue Board` 120,30-320,230 (pixel layer `Blue` at
/// 130,40-230,140) and `Empty Board` 600,0-700,100, all with white backgrounds.
fn session(depth: u32) -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 400, "depth": depth, "background": "transparent"})).unwrap();
    let board =
        |s: &mut Session, name: &str, rect: [i64; 4]| s.execute("layer.new.artboard", json!({"rect": rect, "name": name})).unwrap()["layer"].as_u64().unwrap();
    let red = board(&mut s, "Red Board", [0, 0, 200, 150]);
    let blue = board(&mut s, "Blue Board", [120, 30, 200, 200]);
    let empty = board(&mut s, "Empty Board", [600, 0, 100, 100]);
    paint(&mut s, red, "Red", Rect::new(20, 20, 120, 100), RED);
    paint(&mut s, blue, "Blue", Rect::new(130, 40, 230, 140), BLUE);
    (s, [red, blue, empty])
}

/// A pixel layer `name` of `color` over `rect`, on top of board `board`'s layers.
fn paint(s: &mut Session, board: u64, name: &str, rect: Rect, color: [u8; 3]) {
    s.edit(name, |doc, _| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster(name, fmt);
        let [r, g, b] = color.map(|c| f32::from(c) / 255.0);
        l.surface_mut().unwrap().fill_rect(rect, &photocraft_raster::from_rgba(&fmt, [r, g, b, 1.0]));
        doc.layer_mut(LayerId(board)).unwrap().children_mut().unwrap().push(l);
        Ok(())
    })
    .unwrap();
}

struct Out(std::path::PathBuf);

impl Out {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("photocraft-overlap-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Out(dir)
    }
    fn dir(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for Out {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Exports board `board` with `extra` params and reads the one file back.
fn export(s: &mut Session, out: &Out, board: u64, extra: Value) -> Document {
    let mut p = json!({"dir": out.dir(), "prefix": "", "artboards": [board]});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    let format = p.get("format").and_then(Value::as_str).unwrap_or("png").to_string();
    let r = s.execute("file.export.artboardsToFiles", p).unwrap();
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "{r}");
    let path = files[0].as_str().unwrap();
    photocraft_io::import(&format!("x.{format}"), &std::fs::read(path).unwrap()).unwrap().document
}

/// The file's composite pixel at (x, y) as 8-bit RGBA.
fn px(d: &Document, x: i32, y: i32) -> [u8; 4] {
    let flat = photocraft_compose::flatten(d);
    let w = d.size.width as usize;
    flat.px[y as usize * w + x as usize].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn rgb(p: [u8; 4]) -> [u8; 3] {
    [p[0], p[1], p[2]]
}

fn names(layers: &[Layer]) -> Vec<&str> {
    layers.iter().map(|l| l.name.as_str()).collect()
}

#[test]
fn include_overlapping_areas_is_the_default_and_shows_the_board_above() {
    for depth in [8, 16] {
        let (mut s, [red, _, _]) = session(depth);
        let out = Out::new(&format!("default-{depth}"));
        for extra in [json!({}), json!({"content": "overlapping"})] {
            let f = export(&mut s, &out, red, extra.clone());
            assert_eq!((f.size.width, f.size.height), (200, 150), "{depth} bit {extra}");
            assert_eq!(rgb(px(&f, 150, 60)), BLUE, "{depth} bit {extra}: Blue's pixels");
            assert_eq!(rgb(px(&f, 125, 35)), WHITE, "{depth} bit {extra}: Blue Board's background");
            assert_eq!(rgb(px(&f, 125, 50)), WHITE, "{depth} bit {extra}: Blue Board's background");
            assert_eq!(rgb(px(&f, 110, 50)), RED, "{depth} bit {extra}: Red stays red");
        }
    }
}

#[test]
fn artboard_content_only_keeps_the_board_on_its_own() {
    let (mut s, [red, _, _]) = session(8);
    let out = Out::new("content-only");
    let f = export(&mut s, &out, red, json!({"content": "artboard"}));
    assert_eq!(rgb(px(&f, 150, 60)), WHITE, "nothing of Blue Board");
    assert_eq!(rgb(px(&f, 110, 50)), RED);
    let psd = export(&mut s, &out, red, json!({"content": "artboard", "format": "psd"}));
    assert_eq!(names(&psd.layers), ["Red Board"]);
}

#[test]
fn an_overlapping_psd_holds_the_board_above_as_a_plain_group_with_its_background() {
    let (mut s, [red, _, _]) = session(8);
    let out = Out::new("psd");
    let psd = export(&mut s, &out, red, json!({"format": "psd"}));
    assert_eq!((psd.size.width, psd.size.height), (200, 150));
    // Bottom to top: the exported artboard, then the overlapping board as a plain group.
    assert_eq!(names(&psd.layers), ["Red Board", "Blue Board"]);
    let [board, group] = [&psd.layers[0], &psd.layers[1]];
    assert_eq!(board.artboard().map(|a| a.rect), Some(Rect::new(0, 0, 200, 150)));
    assert_eq!(names(board.children().unwrap()), ["Red"]);
    assert!(matches!(&group.content, LayerContent::Group(g) if g.artboard.is_none()), "no longer an artboard");
    let kids = group.children().unwrap();
    assert_eq!(names(kids), ["Background for Blue Board", "Blue"]);
    // Blue (130,40-230,140 in the source) is cut to the file: 130,40-200,140.
    let blue = photocraft_compose::bounds::content_bounds(kids[1].surface().unwrap());
    assert_eq!(blue, Rect::new(130, 40, 200, 140));
    // The background covers where the boards overlap: 120..200 x 30..150.
    let bg = photocraft_compose::bounds::content_bounds(kids[0].surface().unwrap());
    assert_eq!(bg, Rect::new(120, 30, 200, 150));
    assert_eq!(rgb(px(&psd, 150, 60)), BLUE);
    assert_eq!(rgb(px(&psd, 125, 35)), WHITE);
    assert_eq!(rgb(px(&psd, 110, 50)), RED);
}

#[test]
fn boards_below_do_not_show_in_either_mode() {
    let (mut s, [_, blue, _]) = session(8);
    // A transparent Blue Board shows whatever leaks in from Red Board's white background below.
    s.execute("layer.artboard.set", json!({"layer": blue, "background": "transparent"})).unwrap();
    let out = Out::new("below");
    for content in ["overlapping", "artboard"] {
        let f = export(&mut s, &out, blue, json!({"content": content}));
        assert_eq!((f.size.width, f.size.height), (200, 200), "{content}");
        // Source (125,35): Red Board's background, outside Blue's pixels.
        assert_eq!(px(&f, 5, 5)[3], 0, "{content}: nothing of Red Board");
        assert_eq!(rgb(px(&f, 30, 30)), BLUE, "{content}");
        let psd = export(&mut s, &out, blue, json!({"content": content, "format": "psd"}));
        assert_eq!(names(&psd.layers), ["Blue Board"], "{content}");
    }
}

#[test]
fn the_source_document_is_unchanged_and_gets_no_history_entry() {
    let (mut s, [red, blue, _]) = session(8);
    let out = Out::new("source");
    for content in ["overlapping", "artboard"] {
        for board in [red, blue] {
            let d = s.active().unwrap();
            let (before, rev, past) = (d.doc.clone(), d.revision, d.history.past_len());
            export(&mut s, &out, board, json!({"content": content, "format": "psd"}));
            let d = s.active().unwrap();
            assert!(d.doc == before, "{content} changed the document");
            assert_eq!((d.revision, d.history.past_len()), (rev, past), "{content}");
        }
    }
}

/// The artboard clipped its layers to the board; as a plain group in the file they still stop
/// at the board's edge, so Blue pixels outside Blue Board never cover Red Board.
#[test]
fn the_layers_of_the_board_above_stay_inside_their_board() {
    let (mut s, [red, blue, _]) = session(8);
    paint(&mut s, blue, "Spill", Rect::new(100, 45, 125, 55), BLUE);
    let out = Out::new("spill");
    for format in ["png", "psd"] {
        let f = export(&mut s, &out, red, json!({"format": format}));
        assert_eq!(rgb(px(&f, 110, 50)), RED, "{format}: outside Blue Board");
        assert_eq!(rgb(px(&f, 122, 50)), BLUE, "{format}: inside Blue Board");
        assert_eq!(rgb(px(&f, 150, 60)), BLUE, "{format}");
    }
}

/// A layer that covers the whole canvas (a solid colour fill) in the board above is clipped to
/// that board by a mask on the group, so it can't paint over the rest of the exported board.
#[test]
fn a_fill_layer_in_the_board_above_stays_inside_its_board() {
    let (mut s, [red, blue, _]) = session(8);
    s.edit("Fill", |doc, _| {
        let fill = Layer::new("Green", LayerContent::Fill(photocraft_doc::Fill::Solid(Color::rgb(0.0, 1.0, 0.0))));
        doc.layer_mut(LayerId(blue)).unwrap().children_mut().unwrap().push(fill);
        Ok(())
    })
    .unwrap();
    let out = Out::new("fill");
    for format in ["png", "psd"] {
        let f = export(&mut s, &out, red, json!({"format": format}));
        assert_eq!(rgb(px(&f, 110, 50)), RED, "{format}: outside Blue Board");
        assert_eq!(rgb(px(&f, 10, 140)), WHITE, "{format}: outside Blue Board");
        assert_eq!(rgb(px(&f, 150, 60)), [0, 255, 0], "{format}: inside Blue Board");
    }
}

/// A layer of the board above that lies outside the exported board still casts its drop shadow
/// into it, as it does in the document: its pixels are not cut away before the effect is built.
#[test]
fn a_shadow_cast_into_the_exported_board_from_outside_it_still_shows() {
    use photocraft_doc::effects::{Contour, FxCommon, Shadow};
    let (mut s, [red, blue, _]) = session(8);
    // Caster at 210..240 x 160..180 (in Blue Board, outside Red Board); its shadow, 40 px up and
    // to the left, falls on 170..200 x 120..140, inside Red Board.
    paint(&mut s, blue, "Caster", Rect::new(210, 160, 240, 180), [0, 160, 0]);
    s.edit("Shadow", |doc, _| {
        let caster = doc.layer_mut(LayerId(blue)).unwrap().children_mut().unwrap().last_mut().unwrap();
        caster.effects.items.push(photocraft_doc::Effect::DropShadow(Shadow {
            common: FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
            color: Color::rgb(0.0, 0.0, 0.0),
            angle: -45.0,
            use_global_light: false,
            distance: 40.0_f32.hypot(40.0),
            spread: 0.0,
            size: 0.0,
            contour: Contour::Linear,
            anti_alias: false,
            noise: 0.0,
            knocks_out: true,
        }));
        Ok(())
    })
    .unwrap();
    let source = px(&s.active().unwrap().doc, 185, 130);
    assert_eq!(rgb(source), [0, 0, 0], "the shadow lands there in the document");
    let out = Out::new("shadow");
    for format in ["png", "psd"] {
        let f = export(&mut s, &out, red, json!({"format": format}));
        assert_eq!(px(&f, 185, 130), source, "{format}");
        assert_eq!(rgb(px(&f, 110, 50)), RED, "{format}");
    }
}

/// Not measured (keeps today's behaviour): a hidden board above shows nothing.
#[test]
fn a_hidden_board_above_does_not_show() {
    let (mut s, [red, blue, _]) = session(8);
    s.edit("Hide", |doc, _| {
        doc.layer_mut(LayerId(blue)).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    let out = Out::new("hidden");
    let f = export(&mut s, &out, red, json!({}));
    assert_eq!(rgb(px(&f, 150, 60)), WHITE);
    let psd = export(&mut s, &out, red, json!({"format": "psd"}));
    assert_eq!(names(&psd.layers), ["Red Board"]);
}

#[test]
fn a_bad_content_value_is_rejected_without_writing_anything() {
    let (mut s, _) = session(8);
    let out = Out::new("bad");
    for bad in [json!("both"), json!(""), json!(1), json!(true), json!(["overlapping"])] {
        let d = s.active().unwrap();
        let (before, past) = (d.doc.clone(), d.history.past_len());
        let err = s.execute("file.export.artboardsToFiles", json!({"dir": out.dir(), "content": bad})).unwrap_err();
        assert!(matches!(err, EngineError::BadParams { .. }), "{bad}: {err}");
        let d = s.active().unwrap();
        assert!(d.doc == before && d.history.past_len() == past, "{bad}");
        assert_eq!(std::fs::read_dir(&out.0).unwrap().count(), 0, "{bad}: nothing written");
    }
}

#[test]
fn the_params_doc_names_the_option() {
    let spec = specs().into_iter().find(|c| c.id == "file.export.artboardsToFiles").unwrap();
    assert!(spec.params.contains(r#""content":"overlapping|artboard"="overlapping""#), "{}", spec.params);
}
