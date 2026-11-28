//! #13930: spaces around header names and unquoted values are ignored when data sets are
//! imported, so `Title , Badge , Photo` matches the variables; matching stays case-sensitive and
//! quoted values keep their spaces.

use super::*;
use photocraft_doc::{Layer, TextLayer};
use photocraft_geom::Rect;

/// A document with variables `Title` (text), `Badge` (visibility) and `Photo` (pixel replacement).
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    let (photo, badge, title) = s
        .edit("setup", |doc, _| {
            let fmt = doc.pixel_format();
            let mut photo = Layer::raster("photo", fmt);
            photo.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 24), &photocraft_raster::from_rgba(&fmt, [0.5, 0.5, 0.5, 1.0]));
            let badge = Layer::raster("badge", fmt);
            let title = Layer::new("title", LayerContent::Text(TextLayer { text: "Old".into(), ..Default::default() }));
            let ids = (photo.id, badge.id, title.id);
            doc.layers.extend([photo, badge, title]);
            Ok(ids)
        })
        .unwrap();
    s.execute(
        "image.variables.define",
        json!({"defs": [
            {"name": "Title", "layer": title.0, "type": "textReplacement"},
            {"name": "Badge", "layer": badge.0, "type": "visibility"},
            {"name": "Photo", "layer": photo.0, "type": "pixelReplacement", "method": "conform"},
        ]}),
    )
    .unwrap();
    s
}

/// Imports `text` and returns each data set's (variable, value) pairs.
fn import(s: &mut Session, text: &str) -> Vec<Vec<(String, VarValue)>> {
    import_data_sets_text(s, text, &json!({})).unwrap();
    s.active().unwrap().doc.variables.data_sets.iter().map(|set| set.values.iter().map(|v| (v.variable.clone(), v.value.clone())).collect()).collect()
}

#[test]
fn spaces_around_header_names_and_values_are_ignored() {
    let mut s = session();
    let sets = import(&mut s, "Title , Badge , Photo\r\n Sp ,true, /images/a.png \r\n");
    assert_eq!(
        sets,
        [vec![
            ("Title".to_string(), VarValue::Text("Sp".into())),
            ("Badge".to_string(), VarValue::Visibility(true)),
            ("Photo".to_string(), VarValue::Pixels("/images/a.png".into())),
        ]]
    );
}

#[test]
fn header_matching_stays_case_sensitive() {
    let mut s = session();
    assert_eq!(import(&mut s, " title , badge \nSp,true\n"), [Vec::new()]);
}

#[test]
fn quoted_values_keep_their_spaces() {
    assert_eq!(split_csv(r#"" Title ", Badge ," Sp "#, b','), [" Title ", "Badge", " Sp "]);
    assert_eq!(split_csv("\t a \t,b", b','), ["\t a \t", "b"], "only spaces are removed");
}
