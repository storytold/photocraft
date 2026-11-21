//! Artboards to PDF's Reverse Page Order (#13644), on the measured document: 800 x 400 with Red,
//! Blue and Empty Board, bottom to top. The boards have three different sizes, so each page's
//! MediaBox says which board it is.

use super::*;

/// The measured document, and the ids of Red, Blue and Empty Board (bottom to top).
fn session() -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 400, "depth": 8, "resolution": 72})).unwrap();
    let mut board = |rect: [i64; 4], name: &str| {
        let r = s.execute("layer.new.artboard", json!({"rect": rect, "name": name})).unwrap();
        r["layer"].as_u64().unwrap()
    };
    let ids = [board([0, 0, 200, 150], "Red Board"), board([120, 30, 200, 200], "Blue Board"), board([600, 0, 100, 100], "Empty Board")];
    (s, ids)
}

/// Runs the export with `params` (plus a path in a fresh temporary folder) and returns its result
/// and the bytes written (empty when nothing was).
fn export(s: &mut Session, params: Value) -> (Result<Value>, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let path = join(&dir.path().to_string_lossy(), "boards.pdf");
    let mut params = params;
    params["path"] = json!(path);
    let r = s.execute("file.export.artboardsToPdf", params);
    let bytes = std::fs::read(&path).unwrap_or_default();
    (r, bytes)
}

/// Each page's size in points, in page order.
fn page_sizes(s: &mut Session, params: Value) -> Vec<(u32, u32)> {
    let (r, bytes) = export(s, params);
    r.unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    text.split("/MediaBox [0 0 ")
        .skip(1)
        .map(|rest| {
            let mut n = rest.split([' ', ']']).map(|v| v.parse::<f64>().unwrap().round() as u32);
            (n.next().unwrap(), n.next().unwrap())
        })
        .collect()
}

fn reversed(mut v: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    v.reverse();
    v
}

#[test]
fn reverse_page_order_turns_the_default_order_round() {
    let (mut s, _) = session();
    let default = page_sizes(&mut s, json!({}));
    assert_eq!(default.len(), 3);
    let mut sorted = default.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, [(100, 100), (200, 150), (200, 200)], "one page per board");
    assert_eq!(page_sizes(&mut s, json!({"reversePageOrder": true})), reversed(default));
}

#[test]
fn false_or_no_param_keeps_the_default_order() {
    let (mut s, _) = session();
    let default = page_sizes(&mut s, json!({}));
    assert_eq!(page_sizes(&mut s, json!({"reversePageOrder": false})), default);
    assert_eq!(page_sizes(&mut s, json!({"reversePageOrder": null})), default);
}

#[test]
fn chosen_artboards_come_in_the_reversed_order_too() {
    let (mut s, [red, blue, _]) = session();
    let chosen = json!([red, blue]);
    let default = page_sizes(&mut s, json!({"artboards": chosen}));
    let mut sorted = default.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, [(200, 150), (200, 200)], "only Red and Blue Board");
    assert_eq!(page_sizes(&mut s, json!({"artboards": chosen, "reversePageOrder": true})), reversed(default));
}

#[test]
fn a_non_boolean_value_is_an_error_not_a_panic() {
    let (mut s, _) = session();
    for v in [json!("yes"), json!(1), json!([true]), json!({"on": true})] {
        let (r, bytes) = export(&mut s, json!({"reversePageOrder": v}));
        let e = r.unwrap_err().to_string();
        assert!(e.contains("reversePageOrder"), "{e}");
        assert!(bytes.is_empty(), "nothing is written for {v}");
    }
}

#[test]
fn the_params_doc_names_the_option() {
    let spec = specs().into_iter().find(|c| c.id == "file.export.artboardsToPdf").unwrap();
    assert!(spec.params.contains(r#""reversePageOrder":bool=false"#), "{}", spec.params);
}
