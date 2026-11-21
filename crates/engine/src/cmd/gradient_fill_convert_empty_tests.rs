//! #12015: a live Gradient drag on an empty pixel layer turns that layer into the Gradient fill
//! layer (same name, id and place, a white mask) instead of adding one above it.

use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use crate::Session;
use crate::gradient_fill_cmds::CREATE;

fn drag() -> serde_json::Value {
    json!({"from": [0, 0], "to": [48, 0]})
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 48, "height": 32, "background": "white"})).unwrap();
    s
}

fn add_layer(s: &mut Session, name: &str) -> LayerId {
    LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap())
}

fn names(s: &Session) -> Vec<String> {
    s.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect()
}

#[test]
fn an_empty_pixel_layer_becomes_the_gradient_fill_layer_in_place() {
    let mut s = session();
    let top = add_layer(&mut s, "Top");
    let steps = s.active().unwrap().history.past_len();
    let r = s.execute(CREATE, drag()).unwrap();
    assert_eq!(r["layer"].as_u64(), Some(top.0));
    assert_eq!(names(&s), ["Background", "Top"], "the layer count and names do not change");
    let d = s.active().unwrap();
    assert_eq!(d.active_layer, Some(top));
    let l = d.doc.layer(top).unwrap();
    assert!(matches!(l.content, LayerContent::Fill(_)), "{}", l.content.kind_name());
    let m = l.mask.as_ref().expect("a white mask");
    assert_eq!((m.value(0, 0), m.value(47, 31)), (1.0, 1.0));
    assert_eq!(d.history.past_len(), steps + 1, "one history state");
    assert_eq!(d.history.entries().last().map(String::as_str), Some("New Gradient Fill Layer"));
    // Undo restores the empty pixel layer.
    s.execute("edit.undo", json!({})).unwrap();
    let l = s.active().unwrap().doc.layer(top).unwrap();
    assert!(matches!(l.content, LayerContent::Raster(_)) && l.mask.is_none());
}

#[test]
fn a_layer_with_pixels_or_the_background_gets_a_new_layer_above() {
    let mut s = session();
    s.execute(CREATE, drag()).unwrap();
    assert_eq!(names(&s), ["Background", "Gradient Fill 1"]);

    let mut s = session();
    let top = add_layer(&mut s, "Top");
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute(CREATE, drag()).unwrap();
    assert_eq!(names(&s), ["Background", "Top", "Gradient Fill 1"]);
    assert!(matches!(s.active().unwrap().doc.layer(top).unwrap().content, LayerContent::Raster(_)));
}

#[test]
fn a_hidden_or_locked_empty_layer_keeps_getting_a_new_layer() {
    for props in [json!({"visible": false}), json!({"locks": {"all": true}})] {
        let mut s = session();
        let top = add_layer(&mut s, "Top");
        let mut p = props.clone();
        p["layer"] = json!(top.0);
        s.execute("layer.setProps", p).unwrap();
        // A hidden layer refuses the live drag outright; a locked one gets a new layer above.
        let _ = s.execute(CREATE, drag());
        assert!(matches!(s.active().unwrap().doc.layer(top).unwrap().content, LayerContent::Raster(_)), "{props}");
    }
}

#[test]
fn the_converted_layer_keeps_its_link_effects_and_fill_opacity() {
    let mut s = session();
    let top = add_layer(&mut s, "Top");
    s.edit("layer properties", |d, _| {
        let l = d.layer_mut(top).unwrap();
        l.link_group = Some(7);
        l.effects.enabled = true;
        l.effects.reference = Some((3.0, 4.0));
        l.fill_opacity = 0.5;
        Ok(())
    })
    .unwrap();
    s.execute(CREATE, drag()).unwrap();
    let l = s.active().unwrap().doc.layer(top).unwrap();
    assert!(matches!(l.content, LayerContent::Fill(_)));
    assert_eq!(l.link_group, Some(7));
    assert!(l.effects.enabled);
    assert_eq!(l.effects.reference, Some((3.0, 4.0)));
    assert_eq!(l.fill_opacity, 0.5);
}

#[test]
fn the_converted_layer_keeps_its_psd_id_and_xmp_packet() {
    let mut s = session();
    let top = add_layer(&mut s, "Top");
    s.edit("layer properties", |d, _| {
        let l = d.layer_mut(top).unwrap();
        l.psd_id = Some(42);
        l.xmp = Some(std::sync::Arc::from("<x:xmpmeta>top</x:xmpmeta>"));
        Ok(())
    })
    .unwrap();
    s.execute(CREATE, drag()).unwrap();
    let l = s.active().unwrap().doc.layer(top).unwrap();
    assert!(matches!(l.content, LayerContent::Fill(_)));
    assert_eq!(l.psd_id, Some(42), "the PSD layer id stays stable across round trips");
    assert_eq!(l.xmp.as_deref(), Some("<x:xmpmeta>top</x:xmpmeta>"));
}

#[test]
fn with_a_selection_the_converted_layers_mask_is_the_selection() {
    let mut s = session();
    let top = add_layer(&mut s, "Top");
    s.execute("select.rect", json!({"x": 4, "y": 4, "width": 10, "height": 8})).unwrap();
    s.execute(CREATE, drag()).unwrap();
    assert_eq!(names(&s), ["Background", "Top"]);
    let l = s.active().unwrap().doc.layer(top).unwrap();
    assert!(matches!(l.content, LayerContent::Fill(_)));
    let m = l.mask.as_ref().unwrap();
    assert_eq!((m.value(5, 5), m.value(30, 20)), (1.0, 0.0));
}

#[test]
fn the_converted_layer_keeps_the_psd_blocks_that_dont_describe_a_content_kind() {
    let mut s = session();
    let top = add_layer(&mut s, "Top");
    s.edit("layer properties", |d, _| {
        let l = d.layer_mut(top).unwrap();
        l.psd_blocks.push((*b"lyvr", std::sync::Arc::new(vec![0, 0, 0, 16])));
        l.psd_blocks.push((*b"lnkE", std::sync::Arc::new(vec![7])));
        // Describing another content kind: kept, they would reopen the layer as that kind.
        l.psd_blocks.push((*b"SoCo", std::sync::Arc::new(vec![1])));
        l.psd_blocks.push((*b"TySh", std::sync::Arc::new(vec![2])));
        l.psd_blocks.push((*b"vmsk", std::sync::Arc::new(vec![3])));
        Ok(())
    })
    .unwrap();
    s.execute(CREATE, drag()).unwrap();
    let l = s.active().unwrap().doc.layer(top).unwrap();
    assert!(matches!(l.content, LayerContent::Fill(_)));
    let keys: Vec<&[u8; 4]> = l.psd_blocks.iter().map(|(k, _)| k).collect();
    assert_eq!(keys, [b"lyvr", b"lnkE"], "the layer version and link block stay; the content-kind blocks go");
}

#[test]
fn a_video_layer_with_an_empty_current_frame_gets_a_new_layer_above() {
    let mut s = session();
    let video = add_layer(&mut s, "Video");
    s.edit("video layer", |d, _| {
        let l = d.layer_mut(video).unwrap();
        let fmt = l.surface().unwrap().format();
        let mut painted = photocraft_raster::Surface::new(fmt);
        painted.fill_rect(photocraft_geom::Rect::new(4, 4, 12, 12), &[1.0, 0.0, 0.0, 1.0]);
        let empty = photocraft_raster::Surface::new(fmt);
        // The current frame (0) is empty; frame 1 holds pixels.
        l.video = Some(photocraft_doc::VideoData::new(vec![empty, painted], 24.0));
        Ok(())
    })
    .unwrap();
    s.execute(CREATE, drag()).unwrap();
    assert_eq!(names(&s).len(), 3, "a new layer above, not a conversion: {:?}", names(&s));
    let d = &s.active().unwrap().doc;
    let l = d.layer(video).unwrap();
    assert!(matches!(l.content, LayerContent::Raster(_)), "the video layer is still a pixel layer");
    assert_eq!(l.video.as_ref().unwrap().frames.len(), 2, "its frames are intact");
}
