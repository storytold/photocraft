//! #13923: a data file whose header or rows do not fit the document's variables is refused with
//! the measured sentence, and the existing data sets, active set and history stay as they were.

use std::sync::Arc;

use photocraft_doc::{Layer, LayerContent, TextLayer};
use serde_json::json;

use super::{IMPORT_DATA_SETS, PARSE_DATA_SETS, import_data_sets_text};
use crate::{EngineError, Session};

fn template() -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    let (title, badge, photo) = session
        .edit("Create template layers", |doc, active| {
            let photo = Layer::raster("Photo", doc.pixel_format());
            let badge = Layer::raster("Badge", doc.pixel_format());
            let title = Layer::new("Title", LayerContent::Text(TextLayer { text: "Template".into(), ..Default::default() }));
            let ids = (title.id, badge.id, photo.id);
            *active = Some(title.id);
            doc.layers.extend([photo, badge, title]);
            Ok(ids)
        })
        .unwrap();
    session
        .execute(
            "image.variables.define",
            json!({"defs": [
                {"name": "Title", "layer": title.0, "type": "textReplacement"},
                {"name": "Badge", "layer": badge.0, "type": "visibility"},
                {"name": "Photo", "layer": photo.0, "type": "pixelReplacement"}
            ]}),
        )
        .unwrap();
    session
        .execute(
            "image.variables.dataSets",
            json!({"dataSets": [
                {"name": "Keep A", "values": [
                    {"variable": "Title", "kind": "text", "value": "A"},
                    {"variable": "Badge", "kind": "visibility", "value": true},
                    {"variable": "Photo", "kind": "pixels", "value": "a.png"}
                ]},
                {"name": "Keep B", "values": [
                    {"variable": "Title", "kind": "text", "value": "B"},
                    {"variable": "Badge", "kind": "visibility", "value": false},
                    {"variable": "Photo", "kind": "pixels", "value": "b.png"}
                ]}
            ]}),
        )
        .unwrap();
    session
}

fn names(session: &Session) -> Vec<String> {
    session.active().unwrap().doc.variables.data_sets.iter().map(|s| s.name.clone()).collect()
}

fn refused(err: EngineError, sentence: &str) -> String {
    let EngineError::BadParams { cmd, msg } = err else { panic!("expected BadParams, got {err:?}") };
    assert_eq!(cmd, IMPORT_DATA_SETS);
    assert!(msg.starts_with(PARSE_DATA_SETS), "{msg}");
    assert!(msg.contains(sentence), "{msg}");
    msg
}

fn snapshot(session: &Session) -> (Arc<photocraft_doc::Document>, u64, Vec<String>, Vec<String>, Option<usize>) {
    let state = session.active().unwrap();
    (state.doc.clone(), state.revision, state.history.entries().to_vec(), state.history.redo_labels().map(str::to_owned).collect(), state.doc.variables.active)
}

fn assert_unchanged(session: &Session, before: &(Arc<photocraft_doc::Document>, u64, Vec<String>, Vec<String>, Option<usize>)) {
    let state = session.active().unwrap();
    assert!(Arc::ptr_eq(&state.doc, &before.0));
    assert_eq!(state.revision, before.1);
    assert_eq!(state.history.entries(), before.2);
    assert_eq!(state.history.redo_labels().collect::<Vec<_>>(), before.3);
    assert_eq!(state.doc.variables.active, before.4);
    assert_eq!(names(session), ["Keep A", "Keep B"]);
}

fn session_with_dataset_variable() -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    let (dataset, title) = session
        .edit("Create template layers", |doc, active| {
            let dataset = Layer::new("DataSet", LayerContent::Text(TextLayer { text: "D".into(), ..Default::default() }));
            let title = Layer::new("Title", LayerContent::Text(TextLayer { text: "T".into(), ..Default::default() }));
            let ids = (dataset.id, title.id);
            *active = Some(title.id);
            doc.layers.extend([dataset, title]);
            Ok(ids)
        })
        .unwrap();
    session
        .execute(
            "image.variables.define",
            json!({"defs": [
                {"name": "DataSet", "layer": dataset.0, "type": "textReplacement"},
                {"name": "Title", "layer": title.0, "type": "textReplacement"}
            ]}),
        )
        .unwrap();
    session
}

fn imported_value(listed: &serde_json::Value, variable: &str) -> serde_json::Value {
    listed["dataSets"][0]["values"].as_array().unwrap().iter().find(|value| value["variable"] == variable).unwrap()["value"].clone()
}

#[test]
fn extra_header_column_is_refused_and_keeps_existing_sets() {
    let mut session = template();
    let before = snapshot(&session);
    let err = import_data_sets_text(&mut session, "Title,Badge,Photo,Extra\nOne,true,a.png,x\n", &json!({})).unwrap_err();
    refused(err, "There were too many variable names in the first line of the text file.");
    assert_unchanged(&session, &before);
}

#[test]
fn missing_header_column_is_refused() {
    let mut session = template();
    let before = snapshot(&session);
    let err = import_data_sets_text(&mut session, "Title,Badge\nOne,true\n", &json!({})).unwrap_err();
    refused(err, "There were not enough variable names in the first line of the text file.");
    assert_unchanged(&session, &before);
}

#[test]
fn semicolon_file_with_comma_delimiter_is_not_enough_names() {
    let mut session = template();
    let before = snapshot(&session);
    let err = import_data_sets_text(&mut session, "Title;Badge;Photo\nOne;true;a.png\n", &json!({})).unwrap_err();
    refused(err, "There were not enough variable names in the first line of the text file.");
    assert_unchanged(&session, &before);
}

#[test]
fn header_name_is_matched_case_sensitively() {
    let mut session = template();
    let before = snapshot(&session);
    let err = import_data_sets_text(&mut session, "title,badge,photo\nOne,true,a.png\n", &json!({})).unwrap_err();
    refused(err, "The name 'title' is not a variable in the current document.");
    assert_unchanged(&session, &before);
}

#[test]
fn short_row_and_empty_field_are_incomplete() {
    let mut session = template();
    let before = snapshot(&session);
    let short = import_data_sets_text(&mut session, "Title,Badge,Photo\nOne,true\n", &json!({})).unwrap_err();
    refused(short, "Data set 1 was incomplete.");
    assert_unchanged(&session, &before);
    let empty = import_data_sets_text(&mut session, "Title,Badge,Photo\nOne,,a.png\n", &json!({})).unwrap_err();
    refused(empty, "Data set 1 was incomplete.");
    assert_unchanged(&session, &before);
}

#[test]
fn extra_row_field_and_second_row_numbering() {
    let mut session = template();
    let before = snapshot(&session);
    let extra = import_data_sets_text(&mut session, "Title,Badge,Photo\nOne,true,a.png,x\n", &json!({})).unwrap_err();
    refused(extra, "Too many values were listed for data set 1.");
    assert_unchanged(&session, &before);
    let second = import_data_sets_text(&mut session, "Title,Badge,Photo\nOne,true,a.png\nTwo,true\n", &json!({})).unwrap_err();
    refused(second, "Data set 2 was incomplete.");
    assert_unchanged(&session, &before);
}

#[test]
fn valid_files_import_in_any_column_order_with_quotes_and_blank_lines() {
    let mut session = template();
    let imported = import_data_sets_text(&mut session, "Photo,Title,Badge\n\n\"p.png\",\"Hello, world\",true\n \n\"q.png\",Other,false\n", &json!({})).unwrap();
    assert_eq!(imported, json!({"imported": 2}));
    let listed = session.execute("variables.list", json!({})).unwrap();
    assert_eq!(listed["dataSets"][0]["name"], "Data Set 1");
    assert_eq!(listed["dataSets"][0]["values"][1]["value"], "Hello, world");
    assert_eq!(listed["dataSets"][1]["name"], "Data Set 2");
    assert_eq!(listed["dataSets"][1]["values"][0]["value"], "q.png");
}

#[test]
fn header_only_file_imports_zero_sets() {
    let mut session = template();
    let imported = import_data_sets_text(&mut session, "Title,Badge,Photo\n", &json!({})).unwrap();
    assert_eq!(imported, json!({"imported": 0}));
    assert!(session.active().unwrap().doc.variables.data_sets.is_empty());
}

#[test]
fn named_column_and_execute_path_use_the_same_sentences() {
    let mut session = template();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sets.csv");
    std::fs::write(&path, "DataSet,Title,Badge,Photo,Extra\nNew,One,true,a.png,x\n").unwrap();
    let err = session.execute(IMPORT_DATA_SETS, json!({"path": path})).unwrap_err();
    refused(err, "There were too many variable names in the first line of the text file.");
    assert_eq!(names(&session), ["Keep A", "Keep B"]);
}

#[test]
fn duplicate_header_is_refused_and_keeps_existing_sets() {
    let mut session = template();
    let before = snapshot(&session);
    let err = import_data_sets_text(&mut session, "Title,Title,Photo\nOne,Two,a.png\n", &json!({})).unwrap_err();
    refused(err, "There were too many variable names in the first line of the text file.");
    assert_unchanged(&session, &before);
}

#[test]
fn defined_dataset_variable_imports_as_a_column_in_any_order() {
    for csv in ["DataSet,Title\nValue,Text\n", "Title,DataSet\nText,Value\n"] {
        let mut session = session_with_dataset_variable();
        let imported = import_data_sets_text(&mut session, csv, &json!({})).unwrap();
        assert_eq!(imported, json!({"imported": 1}), "{csv}");
        let listed = session.execute("variables.list", json!({})).unwrap();
        assert_eq!(listed["dataSets"][0]["name"], "Data Set 1", "{csv}");
        assert_eq!(imported_value(&listed, "DataSet"), json!("Value"), "{csv}");
        assert_eq!(imported_value(&listed, "Title"), json!("Text"), "{csv}");
    }
}

#[test]
fn named_column_still_works_when_dataset_is_a_variable() {
    let mut session = session_with_dataset_variable();
    let imported = import_data_sets_text(&mut session, "DataSet,DataSet,Title\nRow,Value,Text\n", &json!({})).unwrap();
    assert_eq!(imported, json!({"imported": 1}));
    let listed = session.execute("variables.list", json!({})).unwrap();
    assert_eq!(listed["dataSets"][0]["name"], "Row");
    assert_eq!(imported_value(&listed, "DataSet"), json!("Value"));
    assert_eq!(imported_value(&listed, "Title"), json!("Text"));
}

#[test]
fn empty_and_non_object_params_are_errors_not_panics() {
    let mut session = template();
    assert!(session.execute(IMPORT_DATA_SETS, json!({})).is_err());
    assert!(session.execute(IMPORT_DATA_SETS, json!(["nope"])).is_err());
    assert!(import_data_sets_text(&mut session, "", &json!({})).is_err());
}
