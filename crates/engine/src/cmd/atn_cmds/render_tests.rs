use super::*;

/// Both recorded event IDs map empty Clouds descriptors without inventing frozen colors.
#[test]
fn clouds_events_map_and_refuse_unrepresentable_native_settings() {
    for (code, name, id) in [(*b"Clds", "clouds", "filter.render.clouds"), (*b"DfrC", "differenceClouds", "filter.render.differenceClouds")] {
        for event in [AtnEvent::Code(code), AtnEvent::name(name)] {
            let file = AtnFile {
                actions: vec![AtnAction {
                    name: UnicodeString::new_nul("Render"),
                    items: vec![AtnItem { event, descriptor: Some(desc("null")), ..Default::default() }],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let loaded = import(&atn::write(&file).unwrap()).unwrap();
            assert!(loaded.unsupported.is_empty());
            assert_eq!(loaded.actions[0].steps, vec![(id.into(), json!({}))]);
            assert_eq!(export_item(id, &json!({}), "Render").unwrap().descriptor, Some(desc("null")));
        }
        for p in [json!({"seed": 7}), json!({"foreground": "#ff0000"}), json!({"background": "#0000ff"}), json!({"layer": 7}), json!({"target": "mask"})] {
            assert!(export_item(id, &p, "Render").is_none(), "{id}: {p}");
        }
    }
}
