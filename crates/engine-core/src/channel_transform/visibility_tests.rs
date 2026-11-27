use super::*;
use photocraft_color::SampleType;
use photocraft_doc::{Layer, LayerMask};

fn masked() -> Session {
    let mut s = super::tests::session(ColorMode::Rgb, SampleType::U8);
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s
}

#[test]
fn ordinary_mask_visibility_follows_the_composite_and_layer_eyes() {
    for linked in [false, true] {
        let mut s = masked();
        s.edit("link mask", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap().linked = linked;
            Ok(())
        })
        .unwrap();
        let target = json!({"target":"mask"});
        let eyes = s.active().unwrap().channel_view.clone();
        assert!(visible(&s, &target).unwrap(), "an applied mask is visible without a mask overlay");
        assert_eq!(s.active().unwrap().channel_view, eyes);
        s.active_mut().unwrap().channel_view.color_hidden = vec![true, false, true];
        assert!(visible(&s, &target).unwrap(), "one visible colour still shows the applied mask");
        s.active_mut().unwrap().channel_view.color_hidden = vec![true; 3];
        assert!(!visible(&s, &target).unwrap(), "a composite with every eye off does not show the mask");
        s.active_mut().unwrap().channel_view.color_hidden.clear();
        s.edit("hide mask layer", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().visible = false;
            Ok(())
        })
        .unwrap();
        assert!(!visible(&s, &target).unwrap());
        s.execute("view.layerMask", json!({"mode":"gray"})).unwrap();
        assert!(visible(&s, &target).unwrap(), "a separately viewed mask stays visible on a hidden layer");
    }
}

#[test]
fn mask_visibility_resolves_the_target_layer_and_hidden_ancestors() {
    let mut s = masked();
    let first = s.active().unwrap().active_layer.unwrap();
    s.edit("hidden parent", |doc, _| {
        let mut group = Layer::group("hidden parent", std::mem::take(&mut doc.layers));
        group.visible = false;
        doc.layers.push(group);
        Ok(())
    })
    .unwrap();
    assert!(!visible(&s, &json!({"target":"mask"})).unwrap());
    s.execute("layer.new.layer", json!({"name":"other"})).unwrap();
    s.edit("other mask", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().mask = Some(LayerMask::reveal_all());
        Ok(())
    })
    .unwrap();
    s.execute("view.layerMask", json!({"mode":"gray"})).unwrap();
    assert!(visible(&s, &json!({"target":"mask","layer":"other"})).unwrap());
    assert!(!visible(&s, &json!({"target":"mask","layer":first.0})).unwrap(), "another layer's gray view cannot show the target mask");
    assert!(visible(&s, &json!({"target":"mask","layer":{}})).is_err());
}

#[test]
fn disabled_mask_needs_its_own_view_to_be_visible() {
    let mut s = masked();
    s.execute("layer.layerMask.enabled", json!({"enabled":false})).unwrap();
    assert!(!visible(&s, &json!({"target":"mask"})).unwrap());
    for mode in ["overlay", "gray"] {
        s.execute("view.layerMask", json!({"mode":mode})).unwrap();
        assert!(visible(&s, &json!({"target":"mask"})).unwrap());
    }
}
