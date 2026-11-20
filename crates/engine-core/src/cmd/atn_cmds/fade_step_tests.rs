use super::*;

/// One action with a single recorded Fade step.
fn fade_file(descriptor: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Fade"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Step"),
            items: vec![AtnItem { event: AtnEvent::Code(*b"Fade"), dictionary: b"Fade".to_vec(), descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

/// Recording still writes both keys, so a step imported without them saves them explicitly.
#[test]
fn fade_step_saves_both_keys() {
    let loaded = import(&fade_file(desc("null"))).unwrap();
    let saved = export("Fade", &[&loaded.actions[0]], &[], false).unwrap().0;
    let d = atn::parse(&saved).unwrap().actions[0].items[0].descriptor.clone().unwrap();
    assert_eq!(d, desc("null").with("Opct", pct(100.0)).with("Md  ", enm("BlnM", "Nrml")));
}
