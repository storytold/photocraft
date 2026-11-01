use super::*;

#[test]
fn parameterless_stylize_and_pixelate_events_use_both_event_id_forms() {
    for (code, name, id) in [
        (*b"FndE", "findEdges", "filter.stylize.findEdges"),
        (*b"Slrz", "solarize", "filter.stylize.solarize"),
        (*b"Fct ", "facet", "filter.pixelate.facet"),
        (*b"Frgm", "fragment", "filter.pixelate.fragment"),
    ] {
        for event in [AtnEvent::Code(code), AtnEvent::name(name)] {
            let file = AtnFile {
                name: UnicodeString::new_nul("Group D"),
                actions: vec![AtnAction {
                    name: UnicodeString::new_nul("A"),
                    items: vec![AtnItem { event, descriptor: Some(desc("null")), ..Default::default() }],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let imported = import(&atn::write(&file).unwrap()).unwrap();
            assert!(imported.unsupported.is_empty());
            assert_eq!(imported.actions[0].steps, vec![(id.into(), json!({}))]);
            assert_eq!(export_item(id, &json!({}), "Group D").unwrap().descriptor, Some(desc("null")));
            assert!(export_item(id, &json!({"layer": 17}), "Group D").is_none());
            assert!(export_item(id, &json!({"target": "mask"}), "Group D").is_none());
        }
    }
}
