use super::*;
use photocraft_geom::Rect;
use proptest::prelude::*;

proptest::proptest! {
    #[test]
    fn responsive_rule_solver_is_finite_and_honors_dimension_clamps(
        x in -100_000i32..100_000,
        y in -100_000i32..100_000,
        board_width in 32u32..4096,
        board_height in 32u32..4096,
        layer_width in 1u32..2048,
        layer_height in 1u32..2048,
        target_width in 32u32..4096,
        target_height in 32u32..4096,
        min_width in 1u32..256,
        min_height in 1u32..256,
        max_width in 256u32..4096,
        max_height in 256u32..4096,
    ) {
        let width = layer_width.min(board_width);
        let height = layer_height.min(board_height);
        let board = Rect::new(x, y, x.saturating_add(board_width as i32), y.saturating_add(board_height as i32));
        let bounds = Rect::new(x, y, x.saturating_add(width as i32), y.saturating_add(height as i32));
        let target = Rect::new(x.saturating_sub(500), y.saturating_sub(500), x.saturating_sub(500).saturating_add(target_width as i32), y.saturating_sub(500).saturating_add(target_height as i32));
        let rule = LayerLayoutRule {
            layer: LayerId(1),
            anchor: Anchor { horizontal: HorizontalAnchor::Center, vertical: VerticalAnchor::Center },
            mode: ResizeMode::Stretch,
            min_size: Some(Dimensions { width: min_width, height: min_height }),
            max_size: Some(Dimensions { width: max_width.max(min_width), height: max_height.max(min_height) }),
            text_flow: TextFlow::Preserve,
            min_font_size_pt: None,
            stack: None,
        };
        let first = rule_transform(&rule, bounds, board, target);
        let second = rule_transform(&rule, bounds, board, target);
        prop_assert_eq!(first, second);
        prop_assert!(first.m.iter().all(|value| value.is_finite()));
        let output_width = f64::from(width) * first.m[0].abs();
        let output_height = f64::from(height) * first.m[3].abs();
        prop_assert!(output_width + 1e-6 >= f64::from(min_width));
        prop_assert!(output_width <= f64::from(max_width.max(min_width)) + 1e-6);
        prop_assert!(output_height + 1e-6 >= f64::from(min_height));
        prop_assert!(output_height <= f64::from(max_height.max(min_height)) + 1e-6);
    }

    #[test]
    fn responsive_solver_is_idempotent_for_repeated_target(
        x in -10_000i32..10_000,
        y in -10_000i32..10_000,
        board_width in 64u32..2048,
        board_height in 64u32..2048,
        layer_width in 8u32..48,
        layer_height in 8u32..48,
        target_width in 64u32..2048,
        target_height in 64u32..2048,
        mode_index in 0usize..4,
    ) {
        let board = Rect::from_xywh(x, y, board_width, board_height);
        let bounds = Rect::from_xywh(x + 8, y + 8, layer_width.min(board_width - 16), layer_height.min(board_height - 16));
        let target_width = target_width.max(board_width);
        let target_height = target_height.max(board_height);
        let target = Rect::from_xywh(x.saturating_add(20_000), y.saturating_add(20_000), target_width, target_height);
        let mode = [ResizeMode::Fixed, ResizeMode::Stretch, ResizeMode::Fill, ResizeMode::Fit][mode_index];
        let rule = LayerLayoutRule {
            layer: LayerId(1),
            anchor: Anchor { horizontal: HorizontalAnchor::Center, vertical: VerticalAnchor::Center },
            mode,
            min_size: None,
            max_size: None,
            text_flow: TextFlow::Preserve,
            min_font_size_pt: None,
            stack: None,
        };
        let first = rule_transform(&rule, bounds, board, target);
        let transformed = Rect::new(
            (f64::from(bounds.x0) * first.m[0] + first.m[4]).round() as i32,
            (f64::from(bounds.y0) * first.m[3] + first.m[5]).round() as i32,
            (f64::from(bounds.x1) * first.m[0] + first.m[4]).round() as i32,
            (f64::from(bounds.y1) * first.m[3] + first.m[5]).round() as i32,
        );
        let repeated = rule_transform(&rule, transformed, target, target);
        prop_assert!((repeated.m[0] - 1.0).abs() < 1e-9);
        prop_assert!((repeated.m[3] - 1.0).abs() < 1e-9);
        prop_assert!(repeated.m[4].abs() < 1e-6);
        prop_assert!(repeated.m[5].abs() < 1e-6);
        // Fit/fill intentionally choose a non-reversible aspect-ratio scale when the board shapes differ.
        if matches!(mode, ResizeMode::Fixed | ResizeMode::Stretch) {
            let back = rule_transform(&rule, transformed, target, board);
            let restored = Rect::new(
                (f64::from(transformed.x0) * back.m[0] + back.m[4]).round() as i32,
                (f64::from(transformed.y0) * back.m[3] + back.m[5]).round() as i32,
                (f64::from(transformed.x1) * back.m[0] + back.m[4]).round() as i32,
                (f64::from(transformed.y1) * back.m[3] + back.m[5]).round() as i32,
            );
            for (actual, expected) in [(restored.x0, bounds.x0), (restored.y0, bounds.y0), (restored.x1, bounds.x1), (restored.y1, bounds.y1)] {
                prop_assert!((i64::from(actual) - i64::from(expected)).abs() <= 2);
            }
        }
    }
}

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 80, "depth": depth})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

#[test]
fn responsive_variant_next_and_previous_cycle_master_and_variants() {
    let mut s = session(8);
    let master = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80],"name":"Master"})).unwrap()["layer"].as_u64().unwrap();
    let first =
        s.execute("layer.artboard.createVariant", json!({"layer":master,"width":80,"height":100,"name":"Portrait"})).unwrap()["layer"].as_u64().unwrap();
    let second =
        s.execute("layer.artboard.createVariant", json!({"layer":master,"width":120,"height":80,"name":"Landscape"})).unwrap()["layer"].as_u64().unwrap();

    s.execute("layer.select", json!({"layer":master})).unwrap();
    assert_eq!(s.execute("layer.artboard.variant.next", json!({})).unwrap()["layer"], json!(first));
    assert_eq!(s.active().unwrap().active_layer, Some(LayerId(first)));
    assert_eq!(s.execute("layer.artboard.variant.next", json!({})).unwrap()["layer"], json!(second));
    assert_eq!(s.execute("layer.artboard.variant.next", json!({})).unwrap()["layer"], json!(master));
    assert_eq!(s.execute("layer.artboard.variant.previous", json!({})).unwrap()["layer"], json!(second));
    assert_eq!(s.execute("layer.artboard.variant.previous", json!({})).unwrap()["layer"], json!(first));
    assert_eq!(s.execute("layer.artboard.variant.previous", json!({})).unwrap()["layer"], json!(master));
}

#[test]
fn responsive_variant_cycle_from_a_child_uses_its_containing_artboard() {
    let mut s = session(8);
    let master = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80]})).unwrap()["layer"].as_u64().unwrap();
    let variant = s.execute("layer.artboard.createVariant", json!({"layer":master,"width":80,"height":100})).unwrap()["layer"].as_u64().unwrap();
    let child = s.execute("layer.new.layer", json!({"name":"Local"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.moveTo", json!({"layer":child,"target":variant,"position":"into"})).unwrap();
    s.execute("layer.select", json!({"layer":child})).unwrap();
    assert_eq!(s.execute("layer.artboard.variant.next", json!({})).unwrap()["layer"], json!(master));
}

#[test]
fn responsive_variants_keep_the_master_below_each_variant_in_root_z_order() {
    let mut s = session(8);
    let master = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80]})).unwrap()["layer"].as_u64().unwrap();
    let first = s.execute("layer.artboard.createVariant", json!({"layer":master,"width":80,"height":100})).unwrap()["layer"].as_u64().unwrap();
    let second = s.execute("layer.artboard.createVariant", json!({"layer":master,"width":120,"height":80})).unwrap()["layer"].as_u64().unwrap();
    let root_order: Vec<u64> = doc(&s).layers.iter().map(|layer| layer.id.0).collect();
    let master_index = root_order.iter().position(|id| *id == master).unwrap();
    assert!(root_order.iter().position(|id| *id == first).unwrap() > master_index);
    assert!(root_order.iter().position(|id| *id == second).unwrap() > master_index);
}

fn add_square(s: &mut Session, r: Rect, rgba: [f32; 4]) -> LayerId {
    s.edit("sq", |doc, active| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster(doc.next_layer_name("Layer"), fmt);
        l.surface_mut().unwrap().fill_rect(r, &photocraft_raster::from_rgba(&fmt, rgba));
        let id = doc.insert_above(None, l);
        *active = Some(id);
        Ok(id)
    })
    .unwrap()
}

#[test]
fn new_artboards_place_and_grow_canvas() {
    let mut s = session(8);
    let r = s.execute("layer.new.artboard", json!({"rect": [0, 0, 100, 80]})).unwrap();
    let a1 = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(doc(&s).layer(a1).unwrap().name, "Artboard 1");
    // Default: canvas-sized, right of the last board with a gap; the canvas grows.
    let r = s.execute("layer.new.artboard", json!({"background": "black"})).unwrap();
    assert_eq!(r["rect"], json!([200, 0, 100, 80]));
    assert_eq!(doc(&s).size.width, 300);
    let r = s.execute("layer.new.artboard", json!({"preset": "iPhone 14", "x": 0, "y": 100})).unwrap();
    assert_eq!(r["rect"], json!([0, 100, 390, 844]));
    assert_eq!(doc(&s).artboards().len(), 3);
    assert!(s.execute("layer.new.artboard", json!({"preset": "Nope"})).is_err());
    assert!(s.execute("layer.new.artboard", json!({"rect": [0, 0, 0, 5]})).is_err());
    assert!(s.execute("layer.new.artboard", json!({"background": "plaid"})).is_err());
    // Undo removes the board (and the canvas growth).
    s.undo();
    assert_eq!(doc(&s).artboards().len(), 2);
    assert_eq!(doc(&s).size.height, 80);
}

#[test]
fn responsive_rule_command_validates_and_stores_constraints() {
    let mut s = session(8);
    let board = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80]})).unwrap();
    let board = LayerId(board["layer"].as_u64().unwrap());
    let layer = add_square(&mut s, Rect::new(10, 10, 30, 30), [1.0, 0.0, 0.0, 1.0]);
    s.execute("layer.moveTo", json!({"layer":layer.0,"target":board.0,"position":"into"})).unwrap();
    let result = s
        .execute(
            "layer.artboard.layoutRule.set",
            json!({"layer":layer.0,"horizontal":"center","vertical":"bottom","mode":"fit","minSize":{"width":12,"height":8},"textFlow":"rewrap","stack":{"direction":"vertical","alignment":"center","gap":12}}),
        )
        .unwrap();
    assert_eq!(result["rule"]["mode"], "fit");
    let rule = doc(&s).responsive.rules.iter().find(|r| r.layer == layer).unwrap();
    assert_eq!(rule.anchor.horizontal, HorizontalAnchor::Center);
    assert_eq!(rule.anchor.vertical, VerticalAnchor::Bottom);
    assert_eq!(rule.min_size, Some(Dimensions { width: 12, height: 8 }));
    assert_eq!(rule.stack.as_ref().map(|stack| stack.gap), Some(12.0));
    s.execute("layer.artboard.layoutRule.set", json!({"layer":layer.0,"minSize":null,"maxSize":null})).unwrap();
    assert!(doc(&s).responsive.rules.iter().find(|r| r.layer == layer).is_some_and(|r| r.min_size.is_none() && r.max_size.is_none()));
    assert!(s.execute("layer.artboard.layoutRule.set", json!({"layer":layer.0,"mode":"magic"})).is_err());
    assert!(s.execute("layer.artboard.layoutRule.set", json!({"layer":layer.0,"minFontSizePt":-1})).is_err());
    assert!(s.execute("layer.artboard.layoutRule.set", json!({"layer":layer.0,"horizontal":false})).is_err());
    assert!(s.execute("layer.artboard.layoutRule.set", json!({"layer":"bad","mode":"fit"})).is_err());
}

#[test]
fn responsive_rule_solver_handles_anchors_modes_and_size_clamps() {
    let source = Rect::new(0, 0, 100, 80);
    let target = Rect::new(200, 0, 400, 100);
    let bounds = Rect::new(10, 10, 30, 30);
    let rule = LayerLayoutRule {
        layer: LayerId(1),
        anchor: Anchor { horizontal: HorizontalAnchor::Center, vertical: VerticalAnchor::Center },
        mode: ResizeMode::Fixed,
        min_size: None,
        max_size: None,
        text_flow: TextFlow::Preserve,
        min_font_size_pt: None,
        stack: None,
    };
    let fixed = rule_transform(&rule, bounds, source, target);
    assert_eq!((fixed.m[4], fixed.m[5]), (250.0, 10.0));

    let fit = rule_transform(&LayerLayoutRule { mode: ResizeMode::Fit, ..rule.clone() }, bounds, source, target);
    assert!((fit.m[0] - 1.25).abs() < 1e-9);
    assert!((fit.m[3] - 1.25).abs() < 1e-9);

    let clamped =
        rule_transform(&LayerLayoutRule { mode: ResizeMode::Fit, min_size: Some(Dimensions { width: 40, height: 40 }), ..rule }, bounds, source, target);
    assert!((clamped.m[0] - 2.0).abs() < 1e-9);
    assert!((clamped.m[3] - 2.0).abs() < 1e-9);
}

#[test]
fn artboard_diagnostics_report_overflow_collisions_and_safe_area_violations() {
    let mut s = session(8);
    let board = s.execute("layer.new.artboard", json!({"rect":[0,0,20,20]})).unwrap();
    let board = LayerId(board["layer"].as_u64().unwrap());
    let first = add_square(&mut s, Rect::new(10, 10, 30, 30), [1.0, 0.0, 0.0, 1.0]);
    let second = add_square(&mut s, Rect::new(12, 12, 22, 22), [0.0, 1.0, 0.0, 1.0]);
    s.execute("layer.moveTo", json!({"layer":first.0,"target":board.0,"position":"into"})).unwrap();
    s.execute("layer.moveTo", json!({"layer":second.0,"target":board.0,"position":"into"})).unwrap();
    s.execute("layer.artboard.semanticRole.set", json!({"layer":first.0,"role":"headline"})).unwrap();
    s.execute("layer.artboard.semanticRole.set", json!({"layer":second.0,"role":"cta"})).unwrap();
    s.execute("layer.artboard.safeArea.set", json!({"layer":board.0,"name":"inset","top":0.1,"right":0.1,"bottom":0.1,"left":0.1,"requiredRoles":["cta"]}))
        .unwrap();
    assert!(s.execute("layer.artboard.safeArea.set", json!({"layer":board.0,"name":"invalid","left":0.75})).is_err());
    assert!(s.execute("layer.artboard.safeArea.set", json!({"layer":board.0,"name":"bad type","left":"ten percent"})).is_err());
    let result = s.execute("layer.artboard.diagnose", json!({"layer":board.0})).unwrap();
    assert_eq!(result["overflow"].as_array().unwrap().len(), 2);
    assert_eq!(result["collisions"].as_array().unwrap().len(), 1);
    assert_eq!(result["safeAreas"][0]["violations"].as_array().unwrap().len(), 1);
    s.execute("layer.artboard.safeArea.set", json!({"layer":board.0,"name":"footer","bottom":0.2,"requiredRoles":["disclaimer"]})).unwrap();
    assert_eq!(doc(&s).responsive.safe_areas.iter().filter(|area| area.artboard == board).count(), 2);
    s.execute("layer.artboard.safeArea.delete", json!({"layer":board.0,"name":"footer"})).unwrap();
    assert!(s.execute("layer.artboard.safeArea.delete", json!({"layer":board.0,"name":"missing"})).is_err());
    assert_eq!(doc(&s).responsive.safe_areas.iter().filter(|area| area.artboard == board).count(), 1);
    s.edit("hide collision participant", |document, _| {
        document.layer_mut(second).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    let hidden_result = s.execute("layer.artboard.diagnose", json!({"layer":board.0})).unwrap();
    assert_eq!(hidden_result["overflow"].as_array().unwrap().len(), 1);
    assert!(hidden_result["collisions"].as_array().unwrap().is_empty());
    assert!(hidden_result["safeAreas"][0]["violations"].as_array().unwrap().is_empty());
    s.edit("fill safe-area capacity", |document, _| {
        for index in 0..photocraft_doc::responsive::MAX_RESPONSIVE_SAFE_AREAS - 1 {
            document.responsive.safe_areas.push(photocraft_doc::responsive::ArtboardSafeArea {
                artboard: board,
                name: format!("zone-{index}"),
                insets: photocraft_doc::responsive::SafeInsets::default(),
                required_roles: Vec::new(),
            });
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(doc(&s).responsive.safe_areas.len(), photocraft_doc::responsive::MAX_RESPONSIVE_SAFE_AREAS);
    assert!(s.execute("layer.artboard.safeArea.set", json!({"layer":board.0,"name":"over-limit"})).is_err());
    assert!(s.execute("layer.artboard.diagnose", json!({"layer":999999})).is_err());
    assert!(s.execute("layer.artboard.diagnose", json!({"layer":"bad"})).is_err());
}

#[test]
fn artboard_diagnostics_bound_collision_work_and_report_truncation() {
    let mut s = session(8);
    let board = LayerId(s.execute("layer.new.artboard", json!({"rect":[0,0,20,1000]})).unwrap()["layer"].as_u64().unwrap());
    s.edit("many non-colliding semantic layers", |doc, _| {
        let format = doc.pixel_format();
        let colors = photocraft_raster::from_rgba(&format, [1.0, 0.0, 0.0, 1.0]);
        for index in 0..450i32 {
            let y = 5 + index * 2;
            let mut layer = Layer::raster(format!("Text {index}"), format);
            layer.surface_mut().unwrap().fill_rect(Rect::new(10, y, 11, y + 1), &colors);
            let id = layer.id;
            doc.layer_mut(board).and_then(Layer::children_mut).ok_or(EngineError::NoLayer(board))?.push(layer);
            doc.metadata.text.push((format!("photocraft.artboard-role.{}", id.0), "headline".to_owned()));
        }
        Ok(())
    })
    .unwrap();

    let result = diagnose_document_artboard(doc(&s), board, 0).unwrap();
    assert!(result["collisions"].as_array().unwrap().is_empty());
    assert_eq!(result["truncated"], json!(true));
}

#[test]
fn semantic_role_suggestions_expose_confidence_and_bulk_edits_stay_on_matching_layers() {
    let mut s = session(8);
    let first_board = LayerId(s.execute("layer.new.artboard", json!({"rect":[0,0,100,80],"name":"First"})).unwrap()["layer"].as_u64().unwrap());
    let second_board = LayerId(s.execute("layer.new.artboard", json!({"rect":[120,0,100,80],"name":"Second"})).unwrap()["layer"].as_u64().unwrap());
    let first = add_square(&mut s, Rect::new(5, 5, 25, 25), [1.0, 0.0, 0.0, 1.0]);
    let similar = add_square(&mut s, Rect::new(30, 5, 50, 25), [0.0, 1.0, 0.0, 1.0]);
    let other_board = add_square(&mut s, Rect::new(125, 5, 145, 25), [0.0, 0.0, 1.0, 1.0]);
    for (layer, board) in [(first, first_board), (similar, first_board), (other_board, second_board)] {
        s.execute("layer.moveTo", json!({"layer":layer.0,"target":board.0,"position":"into"})).unwrap();
    }
    let suggestion = semantic_role_suggestion(doc(&s), first);
    assert_eq!(suggestion.role, "free");
    assert!(suggestion.confidence > 0.0 && suggestion.confidence < 1.0);
    assert_eq!(semantic_role_source(doc(&s), first), "suggestion");

    let result = s.execute("layer.artboard.semanticRole.set", json!({"layer":first.0,"role":"headline","applyToSimilar":true})).unwrap();
    assert_eq!(result["updatedCount"], 2);
    assert_eq!(semantic_role(doc(&s), first), "headline");
    assert_eq!(semantic_role(doc(&s), similar), "headline");
    assert_eq!(semantic_role(doc(&s), other_board), "free");
    assert_eq!(semantic_role_source(doc(&s), first), "manual");

    s.execute("layer.artboard.semanticRole.set", json!({"layer":first.0,"role":"auto"})).unwrap();
    assert_eq!(semantic_role_source(doc(&s), first), "suggestion");
    assert!(s.execute("layer.artboard.semanticRole.set", json!({"layer":first.0,"role":"body","applyToSimilar":"yes"})).is_err());
}

#[test]
fn responsive_variant_can_be_deleted_or_promoted_to_master() {
    let mut s = session(8);
    let master = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80],"name":"Master"})).unwrap();
    let master = LayerId(master["layer"].as_u64().unwrap());
    let first = s.execute("layer.artboard.createVariant", json!({"width":80,"height":80,"name":"First"})).unwrap();
    let first = LayerId(first["layer"].as_u64().unwrap());
    assert_eq!(s.execute("layer.artboard.variant.delete", json!({"layer":first.0})).unwrap()["deleted"], first.0);
    assert!(doc(&s).responsive.variants.is_empty());
    let second = s.execute("layer.artboard.createVariant", json!({"width":80,"height":80,"name":"Second"})).unwrap();
    let second = LayerId(second["layer"].as_u64().unwrap());
    assert_eq!(s.execute("layer.artboard.master.set", json!({"layer":second.0})).unwrap()["master"], second.0);
    assert_eq!(doc(&s).responsive.master, Some(second));
    assert!(doc(&s).responsive.variants.is_empty());
    assert!(doc(&s).layer(master).is_some());
}

#[test]
fn master_rules_and_variant_overrides_follow_layer_identity_on_next_variant() {
    let mut s = session(8);
    let master = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80],"name":"Master"})).unwrap();
    let master = LayerId(master["layer"].as_u64().unwrap());
    let layer = add_square(&mut s, Rect::new(10, 10, 30, 30), [1.0, 0.0, 0.0, 1.0]);
    s.execute("layer.moveTo", json!({"layer":layer.0,"target":master.0,"position":"into"})).unwrap();
    s.execute("layer.artboard.layoutRule.set", json!({"layer":layer.0,"horizontal":"left","vertical":"top","mode":"fixed"})).unwrap();
    let first = s.execute("layer.artboard.createVariant", json!({"width":80,"height":80})).unwrap();
    let first = LayerId(first["layer"].as_u64().unwrap());
    let first_child = doc(&s).layer(first).unwrap().children().unwrap()[0].id;
    s.execute("layer.artboard.layoutRule.set", json!({"layer":first_child.0,"horizontal":"right","vertical":"top","mode":"fixed"})).unwrap();
    assert_eq!(doc(&s).responsive.overrides.len(), 1);
    assert!(doc(&s).responsive.variants[0].layers.iter().any(|mapping| mapping.variant_layer == first_child && mapping.master_layer == layer));
    let before = doc(&s);
    let source_board = before.layer(first).unwrap().artboard().unwrap().rect;
    let source_bounds = crate::layer_multi_cmds::layer_bounds(before.layer(first_child).unwrap()).unwrap();
    let second = s.execute("layer.artboard.createVariant", json!({"layer":first.0,"width":140,"height":120})).unwrap();
    let second = LayerId(second["layer"].as_u64().unwrap());
    let after = doc(&s);
    let target_board = after.layer(second).unwrap().artboard().unwrap().rect;
    let target_child = after.layer(second).unwrap().children().unwrap()[0].id;
    let target_bounds = crate::layer_multi_cmds::layer_bounds(after.layer(target_child).unwrap()).unwrap();
    assert_eq!(
        target_bounds.x1 - source_bounds.x1,
        target_board.x1 - source_board.x1,
        "source={source_bounds:?} target={target_bounds:?} boards={source_board:?}->{target_board:?}"
    );
    let relation = after.responsive.variants.iter().find(|variant| variant.artboard == second).unwrap();
    assert_eq!(relation.master, master);
    assert_eq!(relation.layers.iter().find(|mapping| mapping.variant_layer == target_child).unwrap().master_layer, layer);
}

#[test]
fn variant_rule_override_resets_to_the_current_master_rule_and_is_undoable() {
    let mut s = session(8);
    let board = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80]})).unwrap();
    let master = LayerId(board["layer"].as_u64().unwrap());
    let source = add_square(&mut s, Rect::new(10, 10, 30, 30), [1.0, 0.0, 0.0, 1.0]);
    s.execute("layer.moveTo", json!({"layer":source.0,"target":master.0,"position":"into"})).unwrap();
    s.execute("layer.artboard.layoutRule.set", json!({"layer":source.0,"mode":"stretch"})).unwrap();
    let created = s.execute("layer.artboard.createVariant", json!({"layer":master.0,"width":120,"height":100})).unwrap();
    let variant_board = LayerId(created["layer"].as_u64().unwrap());
    let variant_layer = doc(&s)
        .responsive
        .variants
        .iter()
        .find(|variant| variant.artboard == variant_board)
        .unwrap()
        .layers
        .iter()
        .find(|mapping| mapping.master_layer == source)
        .unwrap()
        .variant_layer;
    s.execute("layer.artboard.layoutRule.set", json!({"layer":variant_layer.0,"mode":"fit"})).unwrap();
    assert_eq!(
        doc(&s).responsive.overrides.iter().find(|entry| entry.variant == variant_board && entry.layer == variant_layer).unwrap().rule.mode,
        ResizeMode::Fit
    );

    let reset = s.execute("layer.artboard.layoutRule.reset", json!({"layer":variant_layer.0})).unwrap();
    assert_eq!(reset["masterLayer"], source.0);
    assert!(doc(&s).responsive.overrides.iter().all(|entry| entry.variant != variant_board || entry.layer != variant_layer));
    assert_eq!(doc(&s).responsive.rules.iter().find(|rule| rule.layer == source).unwrap().mode, ResizeMode::Stretch);
    assert!(s.execute("layer.artboard.layoutRule.reset", json!({"layer":variant_layer.0})).is_err());
    s.undo();
    assert!(doc(&s).responsive.overrides.iter().any(|entry| entry.variant == variant_board && entry.layer == variant_layer));
}

#[test]
fn updating_variants_propagates_master_layers_keeps_ids_backgrounds_and_overrides() {
    let mut s = session(8);
    let board = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80]})).unwrap();
    let master = LayerId(board["layer"].as_u64().unwrap());
    let source = add_square(&mut s, Rect::new(10, 10, 30, 30), [1.0, 0.0, 0.0, 1.0]);
    s.execute("layer.moveTo", json!({"layer":source.0,"target":master.0,"position":"into"})).unwrap();
    s.execute("layer.artboard.layoutRule.set", json!({"layer":source.0,"mode":"stretch"})).unwrap();
    let created = s.execute("layer.artboard.createVariant", json!({"layer":master.0,"width":120,"height":100})).unwrap();
    let variant_board = LayerId(created["layer"].as_u64().unwrap());
    let variant_layer = doc(&s)
        .responsive
        .variants
        .iter()
        .find(|variant| variant.artboard == variant_board)
        .unwrap()
        .layers
        .iter()
        .find(|mapping| mapping.master_layer == source)
        .unwrap()
        .variant_layer;
    s.execute("layer.artboard.layoutRule.set", json!({"layer":variant_layer.0,"mode":"fit"})).unwrap();
    s.execute("layer.artboard.set", json!({"layer":variant_board.0,"background":"black"})).unwrap();
    s.execute("layer.renameLayer", json!({"layer":source.0,"name":"Updated master layer"})).unwrap();
    let added_layer = add_square(&mut s, Rect::new(40, 10, 60, 30), [0.0, 1.0, 0.0, 1.0]);
    s.execute("layer.moveTo", json!({"layer":added_layer.0,"target":master.0,"position":"into"})).unwrap();

    let result = s.execute("layer.artboard.variants.update", json!({"layer":master.0})).unwrap();
    assert_eq!(result["updated"], 1);
    assert_eq!(result["addedLayers"], 1);
    let updated_variant = doc(&s).responsive.variants.iter().find(|variant| variant.artboard == variant_board).unwrap();
    assert_eq!(updated_variant.layers.len(), 3); // artboard, renamed child, newly added child
    assert!(updated_variant.layers.iter().any(|mapping| mapping.master_layer == source && mapping.variant_layer == variant_layer));
    assert!(doc(&s).layer(variant_layer).is_some_and(|layer| layer.name == "Updated master layer"));
    assert_eq!(doc(&s).layer(variant_board).and_then(Layer::artboard).map(|board| board.background), Some(ArtboardBackground::Black));
    assert!(
        doc(&s).responsive.overrides.iter().any(|entry| entry.variant == variant_board && entry.layer == variant_layer && entry.rule.mode == ResizeMode::Fit)
    );
}

#[test]
fn updating_variants_preserves_and_reports_variant_only_layers() {
    let mut s = session(8);
    let board = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80]})).unwrap();
    let master = LayerId(board["layer"].as_u64().unwrap());
    let result = s.execute("layer.artboard.createVariant", json!({"layer":master.0,"width":120,"height":100})).unwrap();
    let variant = LayerId(result["layer"].as_u64().unwrap());
    let local = add_square(&mut s, Rect::new(230, 12, 250, 32), [0.0, 0.0, 1.0, 1.0]);
    s.execute("layer.moveTo", json!({"layer":local.0,"target":variant.0,"position":"into"})).unwrap();

    let update = s.execute("layer.artboard.variants.update", json!({"layer":master.0}));
    assert!(update.is_ok());
    assert!(doc(&s).layer(local).is_some(), "local variant content must survive master propagation");
    assert_eq!(doc(&s).artboard_of(local), Some(variant));
    assert!(update.ok().and_then(|value| value.get("conflicts").and_then(Value::as_array).cloned()).is_some_and(|conflicts| {
        conflicts.iter().any(|conflict| {
            conflict.get("kind").and_then(Value::as_str) == Some("preserved_variant_layer") && conflict.get("layer").and_then(Value::as_u64) == Some(local.0)
        })
    }));
}

#[test]
fn updating_variants_surfaces_content_left_by_a_deleted_master_layer() {
    let mut s = session(8);
    let board = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80]})).unwrap();
    let master = LayerId(board["layer"].as_u64().unwrap());
    let source = add_square(&mut s, Rect::new(10, 10, 30, 30), [1.0, 0.0, 0.0, 1.0]);
    s.execute("layer.moveTo", json!({"layer":source.0,"target":master.0,"position":"into"})).unwrap();
    let created = s.execute("layer.artboard.createVariant", json!({"layer":master.0,"width":120,"height":100})).unwrap();
    let variant = LayerId(created["layer"].as_u64().unwrap());
    let copied = doc(&s)
        .responsive
        .variants
        .iter()
        .find(|entry| entry.artboard == variant)
        .and_then(|entry| entry.layers.iter().find(|mapping| mapping.master_layer == source))
        .map(|mapping| mapping.variant_layer);
    let Some(copied) = copied else { panic!("variant mapping should exist before deleting the master layer") };
    assert!(s.execute("layer.delete", json!({"layer":source.0})).is_ok());

    let update = s.execute("layer.artboard.variants.update", json!({"layer":master.0}));
    assert!(update.is_ok());
    assert!(doc(&s).layer(copied).is_some(), "variant pixels remain available for user review");
    assert!(update.ok().and_then(|value| value.get("conflicts").and_then(Value::as_array).cloned()).is_some_and(|conflicts| {
        conflicts.iter().any(|conflict| {
            conflict.get("kind").and_then(Value::as_str) == Some("preserved_variant_layer") && conflict.get("layer").and_then(Value::as_u64) == Some(copied.0)
        })
    }));
}

#[test]
fn responsive_variant_rename_updates_both_names_and_rejects_invalid_names() {
    let mut s = session(8);
    s.execute("file.new", json!({"width":128,"height":96})).unwrap();
    s.execute("layer.new.artboard", json!({"rect":[0,0,64,64],"name":"Master"})).unwrap();
    let master = doc(&s).artboards()[0].0;
    s.execute("layer.artboard.createVariant", json!({"layer":master.0,"width":96,"height":64,"layout":"semantic"})).unwrap();
    let variant = doc(&s).responsive.variants[0].artboard;

    s.execute("layer.artboard.variant.rename", json!({"layer":variant.0,"name":"Wide social"})).unwrap();
    assert_eq!(doc(&s).layer(variant).map(|layer| layer.name.as_str()), Some("Wide social"));
    assert_eq!(doc(&s).responsive.variants[0].name, "Wide social");
    assert!(s.execute("layer.artboard.variant.rename", json!({"layer":variant.0,"name":"  "})).is_err());
    assert!(s.undo());
    assert_ne!(doc(&s).responsive.variants[0].name, "Wide social");
}

#[test]
fn stack_rule_applies_local_linear_spacing_to_group_children() {
    let mut s = session(8);
    let master = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80],"name":"Master"})).unwrap();
    let master = LayerId(master["layer"].as_u64().unwrap());
    let first = add_square(&mut s, Rect::new(10, 10, 20, 20), [1.0, 0.0, 0.0, 1.0]);
    let second = add_square(&mut s, Rect::new(40, 40, 50, 50), [0.0, 1.0, 0.0, 1.0]);
    s.execute("layer.moveTo", json!({"layer":first.0,"target":master.0,"position":"into"})).unwrap();
    s.execute("layer.moveTo", json!({"layer":second.0,"target":master.0,"position":"into"})).unwrap();
    s.execute("layer.artboard.layoutRule.set", json!({"layer":master.0,"stack":{"direction":"vertical","alignment":"start","gap":8}})).unwrap();
    assert_eq!(doc(&s).responsive.rules.iter().find(|rule| rule.layer == master).and_then(|rule| rule.stack.as_ref()).map(|stack| stack.gap), Some(8.0));
    let result = s.execute("layer.artboard.createVariant", json!({"width":120,"height":100})).unwrap();
    let variant = LayerId(result["layer"].as_u64().unwrap());
    let children = doc(&s).layer(variant).unwrap().children().unwrap();
    let a = crate::layer_multi_cmds::layer_bounds(&children[0]).unwrap();
    let b = crate::layer_multi_cmds::layer_bounds(&children[1]).unwrap();
    let (top, bottom) = if a.y0 <= b.y0 { (a, b) } else { (b, a) };
    assert_eq!(bottom.y0 - top.y1, 8);
}

#[test]
fn create_variant_keeps_editable_layers_and_is_one_undo_step() {
    let mut s = session(8);
    let board = s.execute("layer.new.artboard", json!({"rect": [0, 0, 100, 80], "name": "Source"})).unwrap();
    let source = LayerId(board["layer"].as_u64().unwrap());
    let content = add_square(&mut s, Rect::new(10, 10, 30, 30), [1.0, 0.0, 0.0, 1.0]);
    s.execute("layer.moveTo", json!({"layer": content.0, "target": source.0, "position": "into"})).unwrap();
    let past = s.active().unwrap().history.past_len();
    let result = s.execute("layer.artboard.createVariant", json!({"width": 200, "height": 100})).unwrap();
    let variant = LayerId(result["layer"].as_u64().unwrap());
    assert_eq!(result["source"], json!(source.0));
    assert_eq!(s.active().unwrap().history.past_len(), past + 1);
    assert_eq!(doc(&s).artboards().len(), 2);
    assert_eq!(doc(&s).responsive.master, Some(source));
    assert_eq!(doc(&s).responsive.variants.len(), 1);
    assert_eq!(doc(&s).responsive.variants[0].artboard, variant);
    assert_eq!(doc(&s).responsive.variants[0].master, source);
    let variant_board = doc(&s).layer(variant).unwrap();
    let rect = variant_board.artboard().unwrap().rect;
    assert_eq!((rect.width(), rect.height()), (200, 100));
    let copied_child = variant_board.children().unwrap().first().unwrap();
    assert_ne!(copied_child.id, content);
    // 26 since main's measured Photoshop resampling (#2451): the bicubic edge support is one pixel narrower.
    assert_eq!(copied_child.surface().unwrap().content_bounds().width(), 26, "bicubic edge support is retained around the proportionally scaled content");
    assert_eq!(doc(&s).layer(content).unwrap().surface().unwrap().content_bounds(), Rect::new(10, 10, 30, 30));
    s.undo();
    assert_eq!(doc(&s).artboards().len(), 1);
    assert!(doc(&s).responsive.is_empty());
    assert_eq!(doc(&s).layer(content).unwrap().surface().unwrap().content_bounds(), Rect::new(10, 10, 30, 30));
}

#[test]
fn semantic_variants_preserve_type_size_and_keep_role_metadata_undoable() {
    let mut s = session(8);
    let board = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80],"name":"Master"})).unwrap();
    let board = LayerId(board["layer"].as_u64().unwrap());
    let title = s.execute("type.create", json!({"x":10,"y":30,"text":"A title","size":24,"name":"Headline"})).unwrap();
    let title = LayerId(title["layer"].as_u64().unwrap());
    s.execute("layer.moveTo", json!({"layer":title.0,"target":board.0,"position":"into"})).unwrap();
    s.execute("layer.artboard.semanticRole.set", json!({"layer":title.0,"role":"headline"})).unwrap();
    assert_eq!(semantic_role(doc(&s), title), "headline");
    let source_size = match &doc(&s).layer(title).unwrap().content {
        LayerContent::Text(text) => text.size_pt,
        _ => 0.0,
    };

    let result = s.execute("layer.artboard.createVariant", json!({"width":200,"height":160,"layout":"semantic"})).unwrap();
    let variant = LayerId(result["layer"].as_u64().unwrap());
    let duplicate = doc(&s).layer(variant).unwrap().children().unwrap().iter().find(|layer| layer.name == "Headline").unwrap();
    let LayerContent::Text(text) = &duplicate.content else { panic!("duplicate stays editable text") };
    assert_eq!(text.size_pt, source_size);
    assert_eq!(text.transform.m[0], 1.0, "semantic reflow leaves font scale unchanged");
    assert_eq!(semantic_role(doc(&s), duplicate.id), "headline", "role metadata follows copied layers");
    assert_eq!(semantic_role_source(doc(&s), duplicate.id), "manual", "copied roles keep their provenance");

    s.undo();
    assert!(doc(&s).layer(variant).is_none());
    assert_eq!(semantic_role(doc(&s), title), "headline", "the independent role assignment remains in the prior history state");
}

#[test]
fn responsive_reflow_preserves_effectively_position_locked_layers() {
    let mut s = session(8);
    let board = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80],"name":"Master"})).unwrap();
    let board = LayerId(board["layer"].as_u64().unwrap());
    let locked = add_square(&mut s, Rect::new(10, 12, 30, 32), [1.0, 0.0, 0.0, 1.0]);
    s.execute("layer.moveTo", json!({"layer":locked.0,"target":board.0,"position":"into"})).unwrap();
    let lock_set = s.edit("lock layer position", |doc, _| {
        let Some(layer) = doc.layer_mut(locked) else { return Err(EngineError::NoLayer(locked)) };
        layer.locks.position = true;
        Ok(())
    });
    assert!(lock_set.is_ok());

    let result = s.execute("layer.artboard.createVariant", json!({"layer":board.0,"width":200,"height":160,"layout":"semantic"})).unwrap();
    let variant = LayerId(result["layer"].as_u64().unwrap());
    let mapped = doc(&s)
        .responsive
        .variants
        .iter()
        .find(|entry| entry.artboard == variant)
        .and_then(|entry| entry.layers.iter().find(|pair| pair.master_layer == locked))
        .map(|pair| pair.variant_layer);
    let Some(mapped) = mapped else { panic!("the locked layer must remain mapped to its variant copy") };
    let original = doc(&s).layer(locked).and_then(Layer::surface).map(|surface| surface.content_bounds());
    let copied = doc(&s).layer(mapped).and_then(Layer::surface).map(|surface| surface.content_bounds());
    let source_rect = doc(&s).layer(board).and_then(Layer::artboard).map(|artboard| artboard.rect);
    let variant_rect = doc(&s).layer(variant).and_then(Layer::artboard).map(|artboard| artboard.rect);
    assert!(
        matches!((original, copied, source_rect, variant_rect), (Some(source), Some(copy), Some(source_board), Some(copy_board))
        if copy.x0 - copy_board.x0 == source.x0 - source_board.x0
            && copy.y0 - copy_board.y0 == source.y0 - source_board.y0
            && copy.width() == source.width()
            && copy.height() == source.height()),
        "a position lock keeps the layer fixed relative to the copied artboard"
    );
}

#[test]
fn responsive_text_shrink_updates_explicit_style_runs_and_honors_minimum_size() {
    let mut s = session(8);
    let board = s.execute("layer.new.artboard", json!({"rect":[0,0,400,240],"name":"Master"})).unwrap();
    let board = LayerId(board["layer"].as_u64().unwrap());
    let text = s
        .execute(
            "type.create",
            json!({"box":[20,20,340,120],"text":"A long headline that needs to fit in a narrow responsive format","size":30,"name":"Headline"}),
        )
        .unwrap();
    let text = LayerId(text["layer"].as_u64().unwrap());
    s.execute("layer.moveTo", json!({"layer":text.0,"target":board.0,"position":"into"})).unwrap();
    s.execute(
        "layer.artboard.layoutRule.set",
        json!({"layer":text.0,"horizontal":"left","vertical":"top","mode":"fixed","textFlow":"rewrap_and_shrink","minFontSizePt":9}),
    )
    .unwrap();

    let result = s.execute("layer.artboard.createVariant", json!({"layer":board.0,"width":100,"height":30,"layout":"semantic"})).unwrap();
    let variant = LayerId(result["layer"].as_u64().unwrap());
    let mapped = doc(&s)
        .responsive
        .variants
        .iter()
        .find(|entry| entry.artboard == variant)
        .unwrap()
        .layers
        .iter()
        .find(|pair| pair.master_layer == text)
        .unwrap()
        .variant_layer;
    let LayerContent::Text(reflowed) = &doc(&s).layer(mapped).unwrap().content else { panic!("text stays editable") };
    let source_size = match &doc(&s).layer(text).unwrap().content {
        LayerContent::Text(source) => source.runs[0].style.size_pt,
        _ => 0.0,
    };
    let actual_size = reflowed.runs[0].style.size_pt;
    assert!(actual_size < source_size, "actual character style must shrink: {actual_size} >= {source_size}");
    assert!(actual_size >= 9.0, "minimum font-size clamp must hold: {actual_size}");
    assert_eq!(reflowed.size_pt, actual_size, "the compatibility summary follows the first styled run");

    let roomy = s.execute("layer.artboard.createVariant", json!({"layer":board.0,"width":500,"height":300,"layout":"semantic"})).unwrap();
    let roomy_id = LayerId(roomy["layer"].as_u64().unwrap());
    let roomy_layer = doc(&s)
        .responsive
        .variants
        .iter()
        .find(|entry| entry.artboard == roomy_id)
        .unwrap()
        .layers
        .iter()
        .find(|pair| pair.master_layer == text)
        .unwrap()
        .variant_layer;
    let LayerContent::Text(roomy_text) = &doc(&s).layer(roomy_layer).unwrap().content else { panic!("text stays editable") };
    assert_eq!(roomy_text.runs[0].style.size_pt, source_size, "the fallback leaves type size unchanged when wrapped content fits");
}

#[test]
fn responsive_rewrap_auto_height_grows_to_measured_lines() {
    let mut s = session(8);
    let board = s.execute("layer.new.artboard", json!({"rect":[0,0,400,240],"name":"Master"})).unwrap();
    let board = LayerId(board["layer"].as_u64().unwrap());
    let text = s
        .execute(
            "type.create",
            json!({"box":[20,20,340,120],"text":"A long headline that wraps to several lines on a narrow artboard","size":24,"name":"Headline"}),
        )
        .unwrap();
    let text = LayerId(text["layer"].as_u64().unwrap());
    s.execute("layer.moveTo", json!({"layer":text.0,"target":board.0,"position":"into"})).unwrap();
    s.execute("layer.artboard.layoutRule.set", json!({"layer":text.0,"textFlow":"rewrap_auto_height"})).unwrap();

    let result = s.execute("layer.artboard.createVariant", json!({"layer":board.0,"width":100,"height":240,"layout":"semantic"})).unwrap();
    let variant = LayerId(result["layer"].as_u64().unwrap());
    let mapped = doc(&s)
        .responsive
        .variants
        .iter()
        .find(|entry| entry.artboard == variant)
        .unwrap()
        .layers
        .iter()
        .find(|pair| pair.master_layer == text)
        .unwrap()
        .variant_layer;
    let LayerContent::Text(reflowed) = &doc(&s).layer(mapped).unwrap().content else { panic!("text stays editable") };
    let photocraft_doc::text::TextShape::Box { height, width, .. } = reflowed.shape else { panic!("text remains paragraph text") };
    assert!(height > 120.0, "auto-height should fit wrapped lines, got {height}");
    assert!(height <= 300_000.0 && width > 0.0 && width.is_finite());
}

#[test]
fn create_variant_rejects_invalid_dimensions_without_editing() {
    let mut s = session(8);
    s.execute("layer.new.artboard", json!({"rect": [0, 0, 100, 80]})).unwrap();
    for params in [json!({"width": 0, "height": 10}), json!({"width": 1_000_000, "height": 10}), json!({"width": 100_000, "height": 100_000})] {
        assert_rejected(&mut s, "layer.artboard.createVariant", params);
    }
}

#[test]
fn responsive_variant_reflow_runs_as_a_cancellable_job() {
    let mut s = session(8);
    let board = s.execute("layer.new.artboard", json!({"rect":[0,0,100,80]})).unwrap();
    let board = LayerId(board["layer"].as_u64().unwrap());
    let started = s.start("layer.artboard.createVariant", json!({"layer":board.0,"width":80,"height":120}));
    let crate::jobs::Started::Job(job) = started.unwrap() else { panic!("responsive reflow should start a native job") };
    assert_eq!(doc(&s).artboards().len(), 1, "the worker must not mutate the live document");
    let result = s.wait_job(job).unwrap();
    assert_eq!(result["source"], board.0);
    assert_eq!(doc(&s).artboards().len(), 2);
    assert_eq!(s.active().and_then(|state| state.active_layer).map(|id| id.0), result["layer"].as_u64());
}

#[test]
fn from_layers_and_group_then_compose_clips() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let red = add_square(&mut s, Rect::new(10, 10, 30, 30), [1.0, 0.0, 0.0, 1.0]);
        let r = s.execute("layer.new.artboardFromLayers", json!({"name": "Board"})).unwrap();
        assert_eq!(r["rect"], json!([10, 10, 20, 20]));
        let gid = LayerId(r["layer"].as_u64().unwrap());
        assert_eq!(doc(&s).artboard_of(red), Some(gid));
        // Grow the board; the white background shows around the square, nothing outside it.
        s.execute("layer.artboard.set", json!({"width": 40, "height": 40})).unwrap();
        let px = |s: &Session, x, y| photocraft_compose::render(doc(s), Rect::new(x, y, x + 1, y + 1)).px[0];
        assert!(px(&s, 40, 40)[0] > 0.99 && px(&s, 40, 40)[1] > 0.99, "{depth}");
        assert!(px(&s, 15, 15)[1] < 0.01);
        // Moving the board moves its contents.
        s.execute("layer.artboard.set", json!({"x": 50, "y": 20})).unwrap();
        let a = doc(&s).layer(gid).unwrap().artboard().unwrap().clone();
        assert_eq!(a.rect, Rect::from_xywh(50, 20, 40, 40));
        let surf = doc(&s).layer(red).unwrap().surface().unwrap();
        assert_eq!(surf.content_bounds(), Rect::new(50, 20, 70, 40));
        // layer.translate on the board does the same.
        s.execute("layer.translate", json!({"layer": gid.0, "dx": -5, "dy": 0})).unwrap();
        assert_eq!(doc(&s).layer(gid).unwrap().artboard().unwrap().rect.x0, 45);
        assert_eq!(doc(&s).layer(red).unwrap().surface().unwrap().content_bounds().x0, 45);
        // Background and name.
        s.execute("layer.artboard.set", json!({"background": "custom", "color": "#00ff00", "name": "Green"})).unwrap();
        let l = doc(&s).layer(gid).unwrap();
        assert_eq!(l.name, "Green");
        assert!(matches!(l.artboard().unwrap().background, ArtboardBackground::Custom(_)));
        // A plain group becomes an artboard sized to its contents.
        add_square(&mut s, Rect::new(0, 60, 8, 70), [0.0, 0.0, 1.0, 1.0]);
        let g = s.execute("layer.groupLayers", json!({})).unwrap();
        let gid2 = g["layer"].as_u64().unwrap();
        assert!(s.is_enabled("layer.new.artboardFromGroup"));
        let r = s.execute("layer.new.artboardFromGroup", json!({"layer": gid2})).unwrap();
        assert_eq!(r["rect"], json!([0, 60, 8, 10]));
        assert!(!s.is_enabled("layer.new.artboardFromGroup"));
    }
}

#[test]
fn clear_artboard_guides_only_inside() {
    let mut s = session(8);
    s.execute("layer.new.artboard", json!({"rect": [0, 0, 50, 50]})).unwrap();
    s.edit("guides", |d, _| {
        d.guides.vertical = vec![10.0, 70.0];
        d.guides.horizontal = vec![20.0];
        Ok(())
    })
    .unwrap();
    assert_eq!(s.execute("view.clearSelectedArtboardGuides", json!({})).unwrap()["cleared"], 2);
    assert_eq!(doc(&s).guides.vertical, vec![70.0]);
    assert!(doc(&s).guides.horizontal.is_empty());
    // Clear Canvas Guides leaves the artboard's guides.
    s.edit("guides", |d, _| {
        d.guides.vertical = vec![10.0, 70.0];
        Ok(())
    })
    .unwrap();
    assert_eq!(s.execute("view.clearCanvasGuides", json!({})).unwrap()["cleared"], 1);
    assert_eq!(doc(&s).guides.vertical, vec![10.0]);
}

/// Clearing guides drops their colours, so a guide added later at the same place doesn't inherit one.
#[test]
fn clearing_guides_drops_their_colours() {
    let mut s = session(8);
    let coloured = |s: &mut Session, x: f64| s.execute("view.newGuide", json!({"orientation": "vertical", "position": x, "color": "magenta"})).unwrap();
    let colour = |s: &Session, x: f64| doc(s).guides.color(true, x);
    s.execute("layer.new.artboard", json!({"rect": [0, 0, 50, 50]})).unwrap();
    coloured(&mut s, 10.0);
    coloured(&mut s, 70.0);
    s.execute("view.clearSelectedArtboardGuides", json!({})).unwrap();
    s.execute("view.clearCanvasGuides", json!({})).unwrap();
    assert!(doc(&s).guides.colors.is_empty());
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 70})).unwrap();
    assert_eq!((colour(&s, 10.0), colour(&s, 70.0)), (None, None));
    // New Guide Layout's Clear Existing Guides likewise; a guide it keeps keeps its colour.
    s.execute("view.clearGuides", json!({})).unwrap();
    coloured(&mut s, 0.0);
    s.execute("view.newGuideLayout", json!({"columns": 1})).unwrap();
    assert_eq!(colour(&s, 0.0), Some([0xff, 0x00, 0xff]));
    s.execute("view.newGuideLayout", json!({"columns": 1, "clearExisting": true})).unwrap();
    assert!(doc(&s).guides.vertical.contains(&0.0));
    assert_eq!(colour(&s, 0.0), None);
}

#[test]
fn export_to_files_and_pdf() {
    let mut s = session(16);
    add_square(&mut s, Rect::new(0, 0, 10, 10), [1.0, 0.0, 0.0, 1.0]);
    s.execute("layer.new.artboardFromLayers", json!({"name": "Red Board"})).unwrap();
    s.execute("layer.new.artboard", json!({"rect": [20, 0, 30, 15], "name": "Empty/One", "background": "transparent"})).unwrap();
    let dir = std::env::temp_dir().join(format!("photocraft-artboards-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let d = dir.to_string_lossy().into_owned();
    let r = s.execute("file.export.artboardsToFiles", json!({"dir": d, "prefix": "site"})).unwrap();
    let files: Vec<String> = r["files"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(files.len(), 2);
    assert!(files[0].ends_with("site_0000_Empty_One.png"), "{files:?}");
    assert!(files[1].ends_with("site_0001_Red Board.png"), "{files:?}");
    let img = photocraft_io::import("x.png", &std::fs::read(&files[1]).unwrap()).unwrap().document;
    assert_eq!((img.size.width, img.size.height), (10, 10));
    let img2 = photocraft_io::import("x.png", &std::fs::read(&files[0]).unwrap()).unwrap().document;
    assert_eq!((img2.size.width, img2.size.height), (30, 15));
    // Layered PSD keeps the board, moved to the origin.
    let r = s.execute("file.export.artboardsToFiles", json!({"dir": d, "prefix": "", "format": "psd", "artboards": [doc(&s).artboards()[0].0.0]})).unwrap();
    let psd = photocraft_io::import("x.psd", &std::fs::read(r["files"][0].as_str().unwrap()).unwrap()).unwrap().document;
    assert_eq!(psd.artboards()[0].2.rect, Rect::new(0, 0, 10, 10));
    let pdf = join(&d, "boards.pdf");
    let r = s.execute("file.export.artboardsToPdf", json!({"path": pdf})).unwrap();
    assert_eq!(r["pages"], 2);
    let bytes = std::fs::read(&pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert!(bytes.ends_with(b"%%EOF\n"));
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(text.matches("/Type /Page ").count(), 2);
    assert_eq!(text.matches("/DCTDecode").count(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn raster_pdf_xref_offsets_point_at_objects() {
    let pdf = raster_pdf(&[(2, 3, 72.0, vec![0xff, 0xd8, 0xff, 0xd9])]);
    let text = String::from_utf8_lossy(&pdf).into_owned();
    let xref = text.rfind("xref\n").unwrap();
    let offsets: Vec<usize> = text[xref..].lines().skip(3).take(5).map(|l| l[..10].parse().unwrap()).collect();
    for (i, o) in offsets.iter().enumerate() {
        assert!(text[*o..].starts_with(&format!("{} 0 obj", i + 1)));
    }
    assert!(text.contains("/MediaBox [0 0 2.000 3.000]"));
}

#[test]
fn disabled_without_artboards() {
    let mut s = session(8);
    for id in
        ["layer.artboard.set", "view.clearSelectedArtboardGuides", "file.export.artboardsToFiles", "file.export.artboardsToPdf", "layer.new.artboardFromGroup"]
    {
        assert!(!s.is_enabled(id), "{id}");
    }
    assert!(s.is_enabled("layer.new.artboard"));
    assert!(s.is_enabled("layer.new.artboardFromLayers"));
    let _ = s.execute("layer.new.artboard", json!({}));
    assert!(!s.is_enabled("layer.new.artboardFromLayers"));
}

/// Runs `cmd` and asserts it fails with a bad-params error that leaves the document, its
/// revision, its active layer and its history exactly as they were (#931).
fn assert_rejected(s: &mut Session, cmd: &str, p: Value) -> String {
    let d = s.active().unwrap();
    let (before, rev, active, past) = (d.doc.clone(), d.revision, d.active_layer, d.history.past_len());
    let err = s.execute(cmd, p.clone()).expect_err(&format!("{cmd} {p} should fail"));
    assert!(matches!(err, EngineError::BadParams { .. }), "{cmd} {p}: {err}");
    let d = s.active().unwrap();
    assert!(d.doc == before, "{cmd} {p} changed the document");
    assert_eq!(d.revision, rev, "{cmd} {p}");
    assert_eq!(d.active_layer, active, "{cmd} {p}");
    assert_eq!(d.history.past_len(), past, "{cmd} {p} recorded a history step");
    err.to_string()
}

fn board_rect(s: &Session, id: u64) -> Rect {
    doc(s).layer(LayerId(id)).unwrap().artboard().unwrap().rect
}

#[test]
fn new_artboard_rejects_coordinates_past_the_32_bit_range() {
    let mut s = session(8);
    // A board flush with the far right edge: x1 = i32::MAX.
    let r = s.execute("layer.new.artboard", json!({"x": i32::MAX - 50, "y": 0, "width": 50, "height": 50})).unwrap();
    assert_eq!(r["rect"], json!([i32::MAX - 50, 0, 50, 50]));
    // Default placement right of it has no room: an error naming the edge, not a wrapped x.
    let msg = assert_rejected(&mut s, "layer.new.artboard", json!({}));
    assert!(msg.contains(&i32::MAX.to_string()), "{msg}");
    assert_rejected(&mut s, "layer.new.artboard", json!({"preset": "iPhone 14"}));
    // An explicit position still works beside it (the canvas grew to its 300000 px cap).
    let r = s.execute("layer.new.artboard", json!({"x": 0})).unwrap();
    assert_eq!(r["rect"], json!([0, 0, 300_000, 80]));
    let r = s.execute("layer.new.artboard", json!({"preset": "A4", "x": 0, "y": 100})).unwrap();
    assert_eq!(r["rect"], json!([0, 100, 595, 842]));
    assert_eq!(doc(&s).artboards().len(), 3);

    // Default placement whose x fits but whose canvas-sized width runs past the edge.
    let mut s = session(8);
    s.execute("layer.new.artboard", json!({"x": i32::MAX - 250, "width": 50, "height": 50})).unwrap();
    assert_rejected(&mut s, "layer.new.artboard", json!({}));

    // Explicit rects that would end past i32::MAX (or start outside it) are rejected, not clamped.
    let mut s = session(8);
    for p in [
        json!({"x": i32::MAX, "width": 50, "height": 50}),
        json!({"x": i32::MAX - 10, "width": 50, "height": 50}),
        json!({"y": i32::MAX - 10, "width": 50, "height": 50}),
        json!({"rect": [i32::MAX - 10, 0, 50, 50]}),
        json!({"rect": [0, i32::MAX, 50, 50]}),
        json!({"rect": [3e9, 0, 50, 50]}),
        json!({"rect": [-3e9, 0, 50, 50]}),
        json!({"rect": [0, 0, 3e9, 50]}),
        json!({"x": 0, "width": 3_000_000_000_i64, "height": 50}),
    ] {
        assert_rejected(&mut s, "layer.new.artboard", p);
    }
    // Representable extremes are fine: flush with either edge.
    let r = s.execute("layer.new.artboard", json!({"x": i32::MIN, "y": i32::MIN, "width": 50, "height": 50})).unwrap();
    assert_eq!(r["rect"], json!([i32::MIN, i32::MIN, 50, 50]));
    let r = s.execute("layer.new.artboard", json!({"rect": [i32::MAX - 50, i32::MAX - 50, 50, 50]})).unwrap();
    assert_eq!(r["rect"], json!([i32::MAX - 50, i32::MAX - 50, 50, 50]));
}

#[test]
fn edit_artboard_rejects_moves_and_sizes_past_the_32_bit_range() {
    let mut s = session(8);
    let r = s.execute("layer.new.artboard", json!({"x": i32::MIN, "y": i32::MIN, "width": 50, "height": 50})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    // Moving the board with its contents from one edge to the other needs an offset of about
    // 2^32, which no layer can be translated by.
    let msg = assert_rejected(&mut s, "layer.artboard.set", json!({"x": i32::MAX - 50}));
    assert!(msg.contains("4294967245"), "{msg}");
    assert_rejected(&mut s, "layer.artboard.set", json!({"y": i32::MAX - 50}));
    // A rect that would end past the edge is rejected with or without moving the contents.
    assert_rejected(&mut s, "layer.artboard.set", json!({"x": i32::MAX, "moveContents": false}));
    assert_rejected(&mut s, "layer.artboard.set", json!({"x": i32::MAX - 10}));
    assert_rejected(&mut s, "layer.artboard.set", json!({"rect": [i32::MAX - 10, 0, 50, 50], "moveContents": false}));
    // Without the contents only the board's own rect changes, so the far move is fine.
    let r = s.execute("layer.artboard.set", json!({"x": i32::MAX - 50, "y": i32::MAX - 50, "moveContents": false})).unwrap();
    assert_eq!(r["rect"], json!([i32::MAX - 50, i32::MAX - 50, 50, 50]));
    assert_eq!(board_rect(&s, id), Rect::new(i32::MAX - 50, i32::MAX - 50, i32::MAX, i32::MAX));
    // Resizing past the edge is rejected instead of shrinking the board to fit.
    assert_rejected(&mut s, "layer.artboard.set", json!({"width": 51}));
    assert_rejected(&mut s, "layer.artboard.set", json!({"height": 3_000_000_000_i64}));
    assert_rejected(&mut s, "layer.artboard.set", json!({"preset": "iPhone 14"}));
    // A board at the edge can still be shrunk or moved back in.
    let r = s.execute("layer.artboard.set", json!({"width": 20})).unwrap();
    assert_eq!(r["rect"], json!([i32::MAX - 50, i32::MAX - 50, 20, 50]));
    let r = s.execute("layer.artboard.set", json!({"x": 0, "y": 0, "moveContents": false})).unwrap();
    assert_eq!(r["rect"], json!([0, 0, 20, 50]));
}

#[test]
fn artboards_at_ordinary_coordinates_still_place_and_edit() {
    // Default placement leaves a 100 px gap right of the rightmost board.
    let mut s = session(8);
    let r = s.execute("layer.new.artboard", json!({"x": 10, "width": 50, "height": 50})).unwrap();
    let first = r["layer"].as_u64().unwrap();
    let r = s.execute("layer.new.artboard", json!({"width": 50})).unwrap();
    assert_eq!(r["rect"], json!([160, 0, 50, 80]));
    // Editing moves the board (and its contents) and resizes it.
    let red = add_square(&mut s, Rect::new(20, 10, 30, 20), [1.0, 0.0, 0.0, 1.0]);
    s.edit("into board", |doc, _| {
        let l = doc.remove(red).unwrap();
        let LayerContent::Group(g) = &mut doc.layer_mut(LayerId(first)).unwrap().content else { panic!("not a group") };
        g.children.push(l);
        Ok(())
    })
    .unwrap();
    let r = s.execute("layer.artboard.set", json!({"layer": first, "x": 200})).unwrap();
    assert_eq!(r["rect"], json!([200, 0, 50, 50]));
    assert_eq!(doc(&s).layer(red).unwrap().surface().unwrap().content_bounds(), Rect::new(210, 10, 220, 20));
    let r = s.execute("layer.artboard.set", json!({"layer": first, "width": 70, "height": 40})).unwrap();
    assert_eq!(r["rect"], json!([200, 0, 70, 40]));
}

#[test]
fn export_rejects_a_board_too_far_from_the_origin() {
    let mut s = session(8);
    s.execute("layer.new.artboard", json!({"x": i32::MIN, "width": 50, "height": 50, "name": "Far"})).unwrap();
    let dir = std::env::temp_dir().join(format!("photocraft-artboards-931-{}", std::process::id()));
    let d = dir.to_string_lossy().into_owned();
    // Moving the board to the origin needs dx = 2^31, one past i32::MAX: an error, not a panic.
    let err = s.execute("file.export.artboardsToFiles", json!({"dir": d})).unwrap_err().to_string();
    assert!(err.contains(&i32::MIN.to_string()), "{err}");
    let pdf = join(&d, "boards.pdf");
    assert!(s.execute("file.export.artboardsToPdf", json!({"path": pdf})).is_err());
    assert!(!dir.exists(), "nothing was written");
    // One pixel in from the edge is representable and still exports.
    let one = artboard_document(doc(&s), doc(&s).artboards()[0].0);
    assert!(one.is_err());
    s.execute("layer.artboard.set", json!({"x": i32::MIN + 1, "moveContents": false})).unwrap();
    let one = artboard_document(doc(&s), doc(&s).artboards()[0].0).unwrap().unwrap();
    assert_eq!((one.size.width, one.size.height), (50, 50));
    assert_eq!(one.artboards()[0].2.rect, Rect::new(0, 0, 50, 50));
}

/// #1531: a board 40×40 at (10, 10) holding a red square at (10..30)², the artboard active.
fn board_with_square(s: &mut Session) -> LayerId {
    add_square(s, Rect::new(10, 10, 30, 30), [1.0, 0.0, 0.0, 1.0]);
    let gid = LayerId(s.execute("layer.new.artboardFromLayers", json!({"name": "Board"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.artboard.set", json!({"width": 40, "height": 40})).unwrap();
    s.execute("layer.select", json!({"layer": gid.0})).unwrap();
    gid
}

fn red_at(s: &Session, x: i32, y: i32) -> bool {
    let px = photocraft_compose::render(doc(s), Rect::new(x, y, x + 1, y + 1)).px[0];
    px[0] > 0.99 && px[1] < 0.01 && px[3] > 0.99
}

#[test]
fn duplicate_artboard_lands_beside_the_original_with_its_contents() {
    // #1531: the copy sat exactly on the original, so nothing visible happened.
    let mut s = session(8);
    let gid = board_with_square(&mut s);
    let size = doc(&s).size;
    let r = s.execute("layer.duplicate", json!({})).unwrap();
    let copy = r["layer"].as_u64().unwrap();
    assert_eq!(board_rect(&s, gid.0), Rect::new(10, 10, 50, 50), "the original stays");
    // Photoshop: right of the original with the artboard gap, the canvas growing to show it.
    assert_eq!(board_rect(&s, copy), Rect::new(150, 10, 190, 50));
    assert!(doc(&s).size.width >= 190, "{:?}", doc(&s).size);
    assert!(red_at(&s, 155, 15), "the copy's contents moved with it");
    assert!(red_at(&s, 15, 15), "the original's contents stay");
    assert_eq!(doc(&s).layer(LayerId(copy)).unwrap().name, "Board copy");
    // Again from the original: the spot beside it is taken, so the next free one.
    s.execute("layer.select", json!({"layer": gid.0})).unwrap();
    let copy2 = s.execute("layer.duplicate", json!({})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(board_rect(&s, copy2), Rect::new(290, 10, 330, 50));
    // One undo step each: the copy and the canvas growth go together.
    s.undo();
    s.undo();
    assert_eq!(doc(&s).artboards().len(), 1);
    assert_eq!(doc(&s).size, size);
    // `inPlace` (the Move tool's ⌥-drag) keeps the copy on the original.
    s.execute("layer.select", json!({"layer": gid.0})).unwrap();
    let copy3 = s.execute("layer.duplicate", json!({"inPlace": true})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(board_rect(&s, copy3), Rect::new(10, 10, 50, 50));
}

#[test]
fn duplicating_several_artboards_places_each_copy_in_a_free_spot() {
    let mut s = session(8);
    let a = board_with_square(&mut s);
    let b = s.execute("layer.new.artboard", json!({"rect": [10, 100, 40, 40]})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b, "mode": "add"})).unwrap();
    let r = s.execute("layer.duplicate", json!({})).unwrap();
    assert_eq!(r["layers"].as_array().unwrap().len(), 2);
    let boards: Vec<Rect> = doc(&s).artboards().iter().map(|b| b.2.rect).collect();
    assert_eq!(boards.len(), 4);
    for (i, x) in boards.iter().enumerate() {
        for y in &boards[i + 1..] {
            assert!(x.intersect(y).is_empty(), "{x:?} overlaps {y:?}");
        }
    }
    let canvas = doc(&s).bounds();
    assert!(boards.iter().all(|r| r.intersect(&canvas) == *r), "{boards:?} in {canvas:?}");
    // One undo step.
    s.undo();
    assert_eq!(doc(&s).artboards().len(), 2);
}

#[test]
fn moving_an_artboard_past_the_canvas_grows_it() {
    // #1531: a board dragged right of the canvas was cut off at the old edge.
    let mut s = session(8);
    let gid = board_with_square(&mut s);
    s.execute("layer.translate", json!({"dx": 200, "dy": 60})).unwrap();
    assert_eq!(board_rect(&s, gid.0), Rect::new(210, 70, 250, 110));
    assert_eq!((doc(&s).size.width, doc(&s).size.height), (250, 110));
    assert!(red_at(&s, 215, 75));
    s.undo();
    assert_eq!((doc(&s).size.width, doc(&s).size.height), (100, 80));
}

#[test]
fn moving_an_artboard_past_the_canvas_keeps_its_shapes_whole() {
    // #2389: a shape's pixels are cut at the canvas; the board moved before the canvas grew,
    // so its shape rendered against the old edge and stayed missing or clipped.
    let shape_board = |s: &mut Session| {
        s.execute("file.new", json!({"width": 100, "height": 100, "background": "transparent"})).unwrap();
        let sh = s.execute("shape.create", json!({"kind": "rect", "rect": [10, 20, 30, 30], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.new.artboardFromLayers", json!({})).unwrap();
        sh
    };
    for (cmd, p, size, red, not_red) in [
        ("layer.artboard.set", json!({"x": 120}), (150, 100), [(121, 21), (130, 30), (149, 49)], None),
        ("layer.translate", json!({"dx": 110, "dy": 0}), (150, 100), [(121, 21), (130, 30), (149, 49)], None),
        ("layer.artboard.set", json!({"y": 120}), (100, 150), [(11, 121), (20, 130), (39, 149)], None),
        // Partly past the edge: the whole shape, not just the part inside the old canvas.
        ("layer.artboard.set", json!({"x": 90}), (120, 100), [(91, 21), (95, 30), (110, 30)], Some((15, 30))),
    ] {
        let mut s = Session::new();
        let sh = shape_board(&mut s);
        s.execute(cmd, p.clone()).unwrap();
        assert_eq!((doc(&s).size.width, doc(&s).size.height), size, "{cmd} {p}");
        for (x, y) in red {
            assert!(red_at(&s, x, y), "{cmd} {p}: ({x}, {y})");
        }
        if let Some((x, y)) = not_red {
            assert!(!red_at(&s, x, y), "{cmd} {p}: ({x}, {y}) left behind");
        }
        let info = s.execute("shape.info", json!({"layer": sh})).unwrap();
        assert_eq!(info["bounds"], json!([red[0].0 - 1, red[0].1 - 1, 30, 30]), "{cmd} {p}");
        // One undo step back to the original board; redo brings the whole shape back.
        s.undo();
        assert_eq!((doc(&s).size.width, doc(&s).size.height), (100, 100));
        assert!(red_at(&s, 20, 30), "{cmd} {p}: undo");
        s.redo();
        assert_eq!((doc(&s).size.width, doc(&s).size.height), size);
        for (x, y) in red {
            assert!(red_at(&s, x, y), "{cmd} {p}: redo ({x}, {y})");
        }
    }
}

/// A type layer as a PSD import leaves it: Photoshop's rendering as the cache (deliberately not
/// what PhotoCraft would draw, a solid block, so a re-render shows) and Photoshop's `TySh`.
fn add_imported_type(s: &mut Session, block: Rect) -> LayerId {
    s.edit("type", |doc, active| {
        let mut t = photocraft_doc::TextLayer { text: "Hello!".into(), font_family: "No Such Font".into(), size_pt: 26.0, ..Default::default() };
        t.transform = photocraft_geom::Affine::translate(f64::from(block.x0), f64::from(block.y1));
        t.psd_raw = Some(std::sync::Arc::new(photocraft_text::psd::build_tysh(&t, 72.0, None)));
        let fmt = doc.pixel_format();
        let mut cache = photocraft_raster::Surface::new(fmt);
        cache.fill_rect(block, &photocraft_raster::from_rgba(&fmt, [0.4, 0.2, 0.6, 1.0]));
        t.cache = Some(cache);
        let id = doc.insert_above(None, Layer::new("Hello!", LayerContent::Text(t)));
        *active = Some(id);
        Ok(id)
    })
    .unwrap()
}

fn type_layer(doc: &Document, id: LayerId) -> photocraft_doc::TextLayer {
    match &doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.clone(),
        other => panic!("not a type layer: {}", other.kind_name()),
    }
}

#[test]
fn artboard_export_keeps_imported_type_as_rendered() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let text = add_imported_type(&mut s, Rect::new(30, 20, 60, 30));
        s.execute("layer.new.artboard", json!({"rect": [20, 10, 50, 40]})).unwrap();
        let board = doc(&s).artboards()[0].0;
        s.edit("into board", |doc, _| {
            let l = doc.remove(text).unwrap();
            let Some(LayerContent::Group(g)) = doc.layer_mut(board).map(|b| &mut b.content) else { panic!("artboard group") };
            g.children.push(l);
            Ok(())
        })
        .unwrap();
        let one = artboard_document(doc(&s), board).unwrap().unwrap();
        let t = type_layer(&one, text);
        // The same pixels, moved to the board's origin: not re-rendered.
        let c = t.cache.as_ref().unwrap();
        assert_eq!(c.content_bounds(), Rect::new(10, 10, 40, 20), "depth {depth}");
        let px = photocraft_raster::to_rgba(&c.format(), &c.pixel(15, 15));
        assert!((px[0] - 0.4).abs() < 0.01 && (px[2] - 0.6).abs() < 0.01, "depth {depth}: {px:?}");
        let tysh = photocraft_text::psd::parse_tysh(t.psd_raw.as_deref().unwrap()).unwrap();
        assert_eq!((tysh.transform.m[4], tysh.transform.m[5]), (10.0, 20.0), "the PSD position moves with it");
    }
}

#[test]
fn moving_imported_type_shifts_it_without_re_rendering() {
    let mut s = session(8);
    let text = add_imported_type(&mut s, Rect::new(30, 20, 60, 30));
    let before = type_layer(doc(&s), text);
    s.execute("layer.translate", json!({"layer": text.0, "dx": 5, "dy": -3})).unwrap();
    let after = type_layer(doc(&s), text);
    assert_eq!(after.cache.as_ref().unwrap().content_bounds(), Rect::new(35, 17, 65, 27));
    assert_eq!(after.transform.m[4..], [35.0, 27.0]);
    let (old, new) = (before.psd_raw.unwrap(), after.psd_raw.unwrap());
    assert_eq!((&new[..34], &new[50..]), (&old[..34], &old[50..]), "only the TySh translation changes");
    s.undo();
    assert_eq!(type_layer(doc(&s), text).cache, before.cache);
    // Type with nothing rendered yet is rendered after the move, as before.
    s.edit("drop cache", |doc, _| {
        if let Some(LayerContent::Text(t)) = doc.layer_mut(text).map(|l| &mut l.content) {
            t.cache = None;
        }
        Ok(())
    })
    .unwrap();
    s.execute("layer.translate", json!({"layer": text.0, "dx": 1, "dy": 0})).unwrap();
    assert!(type_layer(doc(&s), text).cache.is_some());
}

#[test]
fn moving_an_artboard_left_of_or_above_the_origin_grows_the_canvas_there() {
    // Photoshop grows an artboard document in every direction; the canvas only grew right and
    // down, so a board moved past the origin stayed partly off-canvas. Now everything shifts
    // right/down (boards, contents, guides) and the canvas grows, in the move's undo step.
    for (cmd, p) in [("layer.translate", json!({"dx": -60, "dy": -30})), ("layer.artboard.set", json!({"x": -50, "y": -20}))] {
        let mut s = session(8);
        let gid = board_with_square(&mut s);
        let other = s.execute("layer.new.artboard", json!({"rect": [60, 10, 30, 30]})).unwrap()["layer"].as_u64().unwrap();
        s.edit("guide", |doc, _| {
            doc.guides.vertical.push(70.0);
            Ok(())
        })
        .unwrap();
        s.execute("layer.select", json!({"layer": gid.0})).unwrap();
        s.execute(cmd, p.clone()).unwrap();
        assert_eq!(board_rect(&s, gid.0), Rect::new(0, 0, 40, 40), "{cmd}");
        assert_eq!(board_rect(&s, other), Rect::new(110, 30, 140, 60), "{cmd}: the other board shifts too");
        assert_eq!((doc(&s).size.width, doc(&s).size.height), (150, 100), "{cmd}");
        assert_eq!(doc(&s).guides.vertical, vec![120.0], "{cmd}");
        assert!(red_at(&s, 0, 0) && red_at(&s, 19, 19) && !red_at(&s, 20, 20), "{cmd}: the contents moved with the board");
        // One undo step back.
        s.undo();
        assert_eq!(board_rect(&s, gid.0), Rect::new(10, 10, 50, 50), "{cmd}");
        assert_eq!(board_rect(&s, other), Rect::new(60, 10, 90, 40), "{cmd}");
        assert_eq!((doc(&s).size.width, doc(&s).size.height), (100, 80), "{cmd}");
        assert_eq!(doc(&s).guides.vertical, vec![70.0], "{cmd}");
    }
}

#[test]
fn add_beside_places_a_board_on_each_side_and_alt_duplicates_contents() {
    // #3033: the Artboard tool's "+" buttons. Board 40×40 at (10, 10) holding a red square.
    let mut s = session(8);
    let gid = board_with_square(&mut s);
    let r = s.execute("layer.artboard.addBeside", json!({"side": "right"})).unwrap();
    assert_eq!(r["rect"], json!([150, 10, 40, 40]), "same size, a gap to the right");
    assert!(!red_at(&s, 155, 15), "a plain \"+\" adds an empty board");
    let r = s.execute("layer.artboard.addBeside", json!({"layer": gid.0, "side": "bottom", "duplicate": true})).unwrap();
    assert_eq!(r["rect"], json!([10, 150, 40, 40]));
    assert!(red_at(&s, 15, 155), "⌥: the copy brings the contents");
    // Again on the right: that spot is taken, so the next free one.
    let r = s.execute("layer.artboard.addBeside", json!({"layer": gid.0})).unwrap();
    assert_eq!(r["rect"], json!([290, 10, 40, 40]));
    // Left of x = 10 is past the canvas origin: everything shifts right by 130 to make room.
    let before = doc(&s).size;
    let r = s.execute("layer.artboard.addBeside", json!({"layer": gid.0, "side": "left"})).unwrap();
    assert_eq!(r["rect"], json!([0, 10, 40, 40]));
    assert_eq!(board_rect(&s, gid.0), Rect::new(140, 10, 180, 50));
    assert!(red_at(&s, 145, 15), "the original's contents moved with the shift");
    assert_eq!(doc(&s).size.width, before.width + 130);
    // One undo step.
    s.undo();
    assert_eq!(board_rect(&s, gid.0), Rect::new(10, 10, 50, 50));
    assert_eq!(doc(&s).size, before);
    assert!(s.execute("layer.artboard.addBeside", json!({"side": "diagonal"})).is_err());
}
