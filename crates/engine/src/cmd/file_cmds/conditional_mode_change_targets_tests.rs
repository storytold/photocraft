//! #13678: Conditional Mode Change converts to Bitmap, Duotone, Indexed Color and Multichannel
//! through the steps Photoshop takes, each its own history state, on the measured document:
//! 200x100 px RGB, red Background, layer "Top" with a blue 50x50 square.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use serde_json::{Value, json};

use crate::Session;

const CMD: &str = "file.automate.conditionalModeChange";

fn measured(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100, "depth": depth, "background": "#ff0000"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 50, "height": 50})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn history(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

/// Run the command and return the history states it added.
fn run(s: &mut Session, p: Value) -> Vec<String> {
    let before = history(s).len();
    let r = s.execute(CMD, p.clone()).unwrap_or_else(|e| panic!("{p}: {e}"));
    assert_eq!(r["changed"], json!(true), "{p}: {r}");
    history(s)[before..].to_vec()
}

/// What the document looks like: mode, depth and layer names, top first.
fn shape(s: &Session) -> (ColorMode, SampleType, Vec<String>) {
    let d = doc(s);
    (d.mode, d.depth, d.layers.iter().rev().map(|l| l.name.clone()).collect())
}

#[test]
fn bitmap_and_duotone_go_through_grayscale_first() {
    let mut s = measured(8);
    assert_eq!(run(&mut s, json!({"to": "bitmap"})), ["Mode Change", "Bitmap"]);
    assert_eq!(shape(&s), (ColorMode::Bitmap, SampleType::U8, vec!["Background".to_string()]), "flattened to Background");

    let mut s = measured(8);
    assert_eq!(run(&mut s, json!({"from": ["rgb"], "to": "Duotone"})), ["Mode Change", "Duotone"]);
    assert_eq!(shape(&s), (ColorMode::Duotone, SampleType::U8, vec!["Top".to_string(), "Background".to_string()]), "both layers kept");
}

#[test]
fn indexed_flattens_and_goes_to_8_bits_first_from_16_and_32() {
    let mut s = measured(8);
    assert_eq!(run(&mut s, json!({"to": "indexed"})), ["Indexed Color"], "the flatten is not a state of its own");
    assert_eq!(shape(&s), (ColorMode::Indexed, SampleType::U8, vec!["Background".to_string()]));

    let mut s = measured(16);
    assert_eq!(run(&mut s, json!({"to": "indexed"})), ["Bit Depth", "Indexed Color"]);
    assert_eq!(shape(&s), (ColorMode::Indexed, SampleType::U8, vec!["Background".to_string()]));

    let mut s = measured(32);
    s.execute("layer.flattenImage", json!({})).unwrap();
    assert_eq!(run(&mut s, json!({"to": "indexed"})), ["Bit Depth", "Indexed Color"]);
    assert_eq!((doc(&s).mode, doc(&s).depth), (ColorMode::Indexed, SampleType::U8));
    let px = |x, y| doc(&s).layers[0].surface().unwrap().rgba(x, y).map(|v| (v * 255.0).round() as u8);
    assert_eq!((px(30, 30), px(150, 50)), ([0, 0, 255, 255], [255, 0, 0, 255]), "the colours are kept");
}

#[test]
fn multichannel_flattens_into_cyan_magenta_and_yellow() {
    let mut s = measured(8);
    assert_eq!(run(&mut s, json!({"to": "multichannel"})), ["Multichannel"]);
    assert_eq!((doc(&s).mode, doc(&s).depth), (ColorMode::Multichannel, SampleType::U8));
    // A Multichannel document holds the flattened image as its ink channels, with no layers.
    assert!(doc(&s).layers.is_empty(), "both layers flattened into the channels");
    let names: Vec<&str> = doc(&s).channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Cyan", "Magenta", "Yellow"]);
}

/// Grayscale is not in the measured table: 8 bits converts directly, 16 bits goes to Indexed
/// through 8 Bits/Channel, like RGB.
#[test]
fn grayscale_converts_directly_or_through_8_bits() {
    let mut s = measured(8);
    s.execute("image.mode.grayscale", json!({})).unwrap();
    assert_eq!(run(&mut s, json!({"to": "bitmap"})), ["Bitmap"]);
    assert_eq!(doc(&s).mode, ColorMode::Bitmap);

    let mut s = measured(16);
    s.execute("image.mode.grayscale", json!({})).unwrap();
    assert_eq!(run(&mut s, json!({"to": "indexed"})), ["Bit Depth", "Indexed Color"]);
    assert_eq!((doc(&s).mode, doc(&s).depth), (ColorMode::Indexed, SampleType::U8));
}

#[test]
fn bitmap_to_rgb_goes_through_grayscale() {
    let mut s = measured(8);
    run(&mut s, json!({"to": "bitmap"}));
    assert_eq!(run(&mut s, json!({"from": ["bitmap"], "to": "rgb"})), ["Mode Change", "Mode Change"]);
    assert_eq!(shape(&s), (ColorMode::Rgb, SampleType::U8, vec!["Background".to_string()]));
}

/// The other params are a new target's options.
#[test]
fn the_target_takes_its_options_from_the_params() {
    let mut s = measured(8);
    run(&mut s, json!({"to": "bitmap", "method": "threshold", "resolution": 144}));
    assert_eq!((doc(&s).size.width, doc(&s).resolution_dpi), (400, 144.0), "Output resolution 144 ppi from 72");

    let mut s = measured(8);
    run(&mut s, json!({"to": "duotone", "type": "tritone"}));
    assert_eq!(doc(&s).duotone.as_ref().map(|d| d.inks.len()), Some(3));

    let mut s = measured(8);
    run(&mut s, json!({"to": "indexed", "palette": "exact", "dither": "none", "forced": "none"}));
    assert_eq!(doc(&s).color_table.as_ref().map(|t| t.colors.len()), Some(2), "Exact: red and blue");

    // The targets the command converted to before take no options, as before.
    let mut s = measured(8);
    run(&mut s, json!({"to": "lab", "profile": "no such profile"}));
    assert_eq!(doc(&s).mode, ColorMode::Lab);
}

/// A bad option is an error, and the document is as it was: the state before the run is the
/// current one, as when the Flatten prompt or the target's dialog is cancelled in Photoshop. The
/// steps taken are redo states, and a recording action gets none of them.
#[test]
fn bad_target_options_leave_the_document_as_it_was() {
    for (depth, p, want) in [
        (8, json!({"to": "bitmap", "method": "zigzag"}), &["Mode Change"][..]),
        (8, json!({"to": "duotone", "type": "pentatone"}), &["Mode Change"][..]),
        (8, json!({"to": "indexed", "colors": 1}), &[][..]),
        (16, json!({"to": "indexed", "colors": 1}), &["Bit Depth"][..]),
    ] {
        let mut s = measured(depth);
        let before = (history(&s), shape(&s));
        s.execute("actions.record", json!({"name": "Run"})).unwrap();
        assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
        s.execute("actions.stop", json!({})).unwrap();
        assert_eq!((history(&s), shape(&s)), before, "{p}");
        let redo: Vec<&str> = s.active().unwrap().history.redo_labels().collect();
        assert_eq!(redo, want, "{p}: the steps taken are redo states");
        assert_eq!(s.execute("actions.get", json!({"action": "Run"})).unwrap()["steps"], json!([]), "{p}: nothing recorded");
    }
}

/// Routes Photoshop was not measured on are refused, not guessed; an unticked source still
/// leaves the document without an error.
#[test]
fn unmeasured_sources_are_refused() {
    for (setup, to) in [
        ("image.mode.cmyk", "bitmap"),
        ("image.mode.lab", "indexed"),
        ("image.mode.cmyk", "multichannel"),
        ("image.mode.bits16", "bitmap"),
        ("image.mode.bits16", "duotone"),
        ("image.mode.bits32", "multichannel"),
        ("image.mode.bits32", "indexed"),
    ] {
        let mut s = measured(8);
        s.execute(setup, json!({})).unwrap();
        let before = (history(&s), shape(&s));
        let e = s.execute(CMD, json!({"to": to})).unwrap_err();
        assert!(e.to_string().contains("not supported"), "{setup} to {to}: {e}");
        assert_eq!((history(&s), shape(&s)), before, "{setup} to {to}");
    }
    let mut s = measured(8);
    s.execute("image.mode.cmyk", json!({})).unwrap();
    let r = s.execute(CMD, json!({"from": ["rgb"], "to": "bitmap"})).unwrap();
    assert_eq!(r["changed"], json!(false));
}
