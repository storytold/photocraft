use photocraft_doc::{LayerContent, LayerId, Pattern, PixelFormat};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::json;

fn library_pattern() -> Pattern {
    let mut surface = Surface::new(PixelFormat::RGBA16);
    surface.fill_rect(Rect::new(0, 0, 2, 2), &[0.1, 0.2, 0.9, 1.0]);
    surface.fill_rect(Rect::new(1, 0, 2, 2), &[0.9, 0.7, 0.1, 1.0]);
    Pattern::new("Shape tile", surface, 2, 2)
}

#[test]
fn patterned_shapes_embed_render_and_undo_in_one_command_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":32,"height":32,"depth":depth})).unwrap();
        let p = library_pattern();
        session.patterns.items.push(p.clone());
        let history = session.active().unwrap().history.past_len();
        let f = json!({"pattern":p.name,"id":p.id,"scale":100,"angle":0,"link":true});
        let id = LayerId(
            session.execute("shape.create", json!({"kind":"rect","rect":[8,8,16,16],"fill":f,"stroke":{"width":4,"fill":f}})).unwrap()["layer"]
                .as_u64()
                .unwrap(),
        );
        let doc = session.active().unwrap().doc.clone();
        assert_eq!(session.active().unwrap().history.past_len(), history + 1);
        assert_eq!(doc.patterns, vec![p.clone()]);
        let surface = doc.layer(id).unwrap().surface().unwrap();
        let q = surface.rgba(10, 10);
        assert!(q[2] > 0.85 && q[0] < 0.15 && q[3] > 0.99, "pattern fill is visible: {q:?}");
        let q = surface.rgba(6, 12);
        assert!(q[2] > 0.85 && q[3] > 0.99, "pattern stroke is visible: {q:?}");
        assert!(session.undo());
        assert!(session.active().unwrap().doc.patterns.is_empty());
        assert!(session.active().unwrap().doc.layer(id).is_none());
        assert!(session.redo());
        assert_eq!(session.active().unwrap().doc, doc);

        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let native = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(&native, doc.as_ref());
        let bytes = photocraft_io::export(&doc, "psd", &Default::default()).unwrap().bytes;
        let mut imported = photocraft_io::import("shape-pattern.psd", &bytes).unwrap().document;
        assert_eq!(imported.patterns.len(), 1);
        let imported_id = imported.layers.iter().find(|l| matches!(l.content, LayerContent::Shape(_))).unwrap().id;
        // Prove editable paint survives, rather than relying on the PSD's cached pixels.
        let source = imported.clone();
        if let LayerContent::Shape(sh) = &mut imported.layer_mut(imported_id).unwrap().content {
            photocraft_engine::vector_cmds::refresh_shape(&source, sh);
        }
        let q = imported.layer(imported_id).unwrap().surface().unwrap().rgba(10, 10);
        assert!(q[2] > 0.8 && q[0] < 0.2 && q[3] > 0.99, "PSD pattern remains renderable after refresh: {q:?}");
    }
}

#[test]
fn editing_a_solid_shape_to_pattern_embeds_and_undoes_the_tile() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":32,"height":32})).unwrap();
    let p = library_pattern();
    session.patterns.items.push(p.clone());
    let id = session.execute("shape.create", json!({"kind":"rect","rect":[8,8,16,16],"fill":"#ff0000"})).unwrap()["layer"].as_u64().unwrap();
    let original = session.active().unwrap().doc.clone();
    session.execute("shape.edit", json!({"layer":id,"fill":{"pattern":p.name,"id":p.id}})).unwrap();
    assert_eq!(session.active().unwrap().doc.patterns, vec![p]);
    let edited = session.active().unwrap().doc.clone();
    assert!(session.undo());
    assert_eq!(session.active().unwrap().doc, original);
    assert!(session.redo());
    assert_eq!(session.active().unwrap().doc, edited);
}
