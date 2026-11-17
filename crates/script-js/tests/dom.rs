//! The document model scripts see, run against a real session: layers and sets, type,
//! History, the selection, sizes in the ruler unit, saving, custom options and preferences.

use std::cell::RefCell;
use std::rc::Rc;

use photocraft_engine::Session;
use photocraft_script_js::session_host::SessionHost;
use photocraft_script_js::{RunOptions, Runner, Step, Unlimited};
use serde_json::{Value, json};

fn session(width: u32, height: u32) -> Rc<RefCell<Session>> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": width, "height": height, "name": "Doc"})).unwrap();
    Rc::new(RefCell::new(s))
}

/// Run `src` to the end, answering each dialog from `answers`; the lines it wrote.
fn run_answering(s: &Rc<RefCell<Session>>, src: &str, answers: &[Value]) -> Vec<String> {
    let host = SessionHost::new(s.clone(), "dom");
    let out = host.output();
    let mut r = Runner::new(src, Box::new(host), Box::new(Unlimited), RunOptions::default()).unwrap();
    let mut answers = answers.iter();
    loop {
        match r.step() {
            Step::Done => break,
            Step::Paused(_) => r.answer(answers.next().cloned().expect("a dialog the test did not answer")).unwrap(),
            Step::Failed(m) => panic!("{m}\nlines: {:?}", out.borrow()),
        }
    }
    out.borrow().clone()
}

fn run(s: &Rc<RefCell<Session>>, src: &str) -> Vec<String> {
    run_answering(s, src, &[])
}

fn layer_names(s: &Rc<RefCell<Session>>) -> Vec<String> {
    let d = s.borrow_mut().execute("document.inspect", json!({})).unwrap();
    d["layers"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap().to_owned()).collect()
}

#[test]
fn layers_can_be_listed_added_named_and_styled() {
    let s = session(64, 32);
    let lines = run(
        &s,
        r#"
        var doc = app.activeDocument;
        $.writeln(doc.layers.length, ' ', doc.layers[0].name, ' ', doc.layers[0].isBackgroundLayer, ' ', doc.layers[0].typename);
        var l = doc.artLayers.add();
        l.name = 'Shade';
        l.opacity = 40;
        l.fillOpacity = 75;
        l.blendMode = BlendMode.MULTIPLY;
        l.visible = false;
        $.writeln(l.name, ' ', l.opacity, ' ', l.fillOpacity, ' ', l.blendMode, ' ', l.visible, ' ', l.kind == LayerKind.NORMAL, ' ', l.isBackgroundLayer);
        $.writeln(doc.activeLayer == l, ' ', doc.layers[0] == l, ' ', doc.artLayers.getByName('Shade') == l, ' ', l.parent == doc);
        try { doc.artLayers.getByName('nope'); } catch (e) { $.writeln(e.number, ' ', e.message); }
        try { l.opacity = 140; } catch (e) { $.writeln(e.name); }
        try { l.blendMode = 'BlendMode.SPARKLE'; } catch (e) { $.writeln('no sparkle'); }
        "#,
    );
    assert_eq!(
        lines,
        [
            "1 Background true ArtLayer",
            "Shade 40 75 BlendMode.MULTIPLY false true false",
            "true true true true",
            "1302 No such element",
            "RangeError",
            "no sparkle",
        ]
    );
    assert_eq!(layer_names(&s), ["Shade", "Background"]);
}

#[test]
fn layer_sets_hold_layers_and_layers_move_duplicate_merge_and_go() {
    let s = session(64, 32);
    let lines = run(
        &s,
        r#"
        var doc = app.activeDocument;
        var set = doc.layerSets.add(); set.name = 'Group';
        var inner = set.artLayers.add(); inner.name = 'Inner';
        $.writeln(set.typename, ' ', set.kind, ' ', set.layers.length, ' ', inner.parent == set, ' ', doc.layerSets.length, ' ', doc.artLayers.length);
        var top = doc.artLayers.add(); top.name = 'Top';
        top.move(doc.layers[doc.layers.length - 1], ElementPlacement.PLACEBEFORE);
        var copy = top.duplicate(); copy.name = 'Copy';
        $.writeln(doc.layers.length);
        copy.remove();
        var merged = top.merge();
        $.writeln(merged.name);
        try { doc.layers[0].move(doc, 'ElementPlacement.INSIDE'); } catch (e) { $.writeln('placement'); }
        "#,
    );
    assert_eq!(lines, ["LayerSet undefined 1 true 1 1", "4", "Background", "placement"]);
    assert_eq!(layer_names(&s), ["Group", "Background"]);
}

#[test]
fn a_layer_moves_and_reports_its_bounds_in_the_ruler_unit() {
    let s = session(144, 72);
    s.borrow_mut().execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 20})).unwrap();
    s.borrow_mut().execute("layer.new.layer", json!({"name": "Box"})).unwrap();
    s.borrow_mut().execute("edit.fill", json!({"contents": "black"})).unwrap();
    s.borrow_mut().execute("select.deselect", json!({})).unwrap();
    let lines = run(
        &s,
        r#"
        var l = app.activeDocument.activeLayer;
        l.translate(5, 7);
        $.writeln(l.bounds.join(','));
        app.preferences.rulerUnits = Units.INCHES;
        l.translate(new UnitValue(1, 'in'), 0);
        $.writeln(app.preferences.rulerUnits, ' ', app.activeDocument.width, ' ', l.bounds[0].as('px'));
        app.preferences.rulerUnits = Units.PIXELS;
        "#,
    );
    assert_eq!(lines, ["5 px,7 px,15 px,27 px", "Units.INCHES 2 in 77"]);
}

#[test]
fn text_layers_are_made_and_edited() {
    let s = session(200, 100);
    let lines = run(
        &s,
        r#"
        var doc = app.activeDocument;
        var t = doc.artLayers.add();
        t.kind = LayerKind.TEXT;
        t.textItem.contents = 'Hello';
        t.textItem.size = new UnitValue(24, 'pt');
        t.textItem.position = [10, 50];
        $.writeln(t.kind, ' ', t.textItem.contents, ' ', t.textItem.size, ' ', t.textItem.position.join(','), ' ', doc.activeLayer == t);
        try { doc.layers[doc.layers.length - 1].textItem; } catch (e) { $.writeln(e.message); }
        try { doc.layers[doc.layers.length - 1].kind = LayerKind.TEXT; } catch (e) { $.writeln('not empty'); }
        "#,
    );
    assert_eq!(lines, ["LayerKind.TEXT Hello 24 pt 10 px,50 px true", "the layer is not a text layer", "not empty"]);
}

#[test]
fn history_states_list_the_panel_and_a_state_can_be_made_current() {
    let s = session(32, 32);
    s.borrow_mut().execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.borrow_mut().execute("layer.new.layer", json!({"name": "B"})).unwrap();
    let lines = run(
        &s,
        r#"
        var doc = app.activeDocument, h = doc.historyStates;
        $.writeln(h.length, ' ', h[0].name, ' ', doc.activeHistoryState == h[h.length - 1], ' ', h[0].snapshot, ' ', h[0].parent == doc);
        doc.activeHistoryState = h[1];
        $.writeln(doc.layers.length, ' ', doc.historyStates.length, ' ', doc.activeHistoryState == h[1]);
        "#,
    );
    assert_eq!(lines, ["3 New true false true", "2 3 true"]);
}

#[test]
fn the_selection_selects_reports_and_clears() {
    let s = session(100, 50);
    let lines = run(
        &s,
        r#"
        var sel = app.activeDocument.selection;
        try { sel.bounds; } catch (e) { $.writeln(e.message); }
        sel.selectAll(); $.writeln(sel.bounds.join(','));
        sel.select([[10, 5], [40, 5], [40, 25], [10, 25]]); $.writeln(sel.bounds.join(','));
        sel.deselect();
        try { sel.bounds; } catch (e) { $.writeln('none'); }
        $.writeln(sel == app.activeDocument.selection);
        "#,
    );
    assert_eq!(lines, ["there is no selection", "0 px,0 px,100 px,50 px", "10 px,5 px,40 px,25 px", "none", "true"]);
}

#[test]
fn documents_are_made_resized_cropped_duplicated_and_closed() {
    let s = session(100, 50);
    let lines = run(
        &s,
        r#"
        var d = app.documents.add(200, 100, 72, 'Made');
        $.writeln(app.activeDocument == d, ' ', d.name, ' ', app.documents.length, ' ', app.documents.getByName('Doc').width);
        d.resizeImage(100);
        $.writeln(d.width, 'x', d.height);
        d.resizeCanvas(120, 60, AnchorPosition.TOPLEFT);
        $.writeln(d.width, 'x', d.height);
        d.crop([10, 10, 60, 40]);
        $.writeln(d.width, 'x', d.height);
        var c = d.duplicate('Twin');
        $.writeln(c.name, ' ', app.documents.length);
        c.artLayers.add();
        try { c.close(); } catch (e) { $.writeln('kept: ', e.message.indexOf('unsaved') >= 0); }
        c.close(SaveOptions.DONOTSAVECHANGES);
        $.writeln(app.documents.length);
        d.flatten(); d.mergeVisibleLayers === undefined;
        "#,
    );
    assert_eq!(lines, ["true Made 2 100 px", "100 pxx50 px", "120 pxx60 px", "50 pxx30 px", "Twin 3", "kept: true", "2"]);
}

#[test]
fn saving_needs_a_file_or_folder_the_user_chose_in_the_run() {
    let dir = std::env::temp_dir().join(format!("photocraft-dom-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let chosen = dir.join("chosen");
    let s = session(16, 16);
    let src = r#"
        var doc = app.activeDocument;
        try { doc.saveAs(new File('/tmp/made-up.png'), new PNGSaveOptions(), true); } catch (e) { $.writeln('refused'); }
        var f = File.saveDialog('Save');
        var o = new JPEGSaveOptions(); o.quality = 10;
        doc.saveAs(f, o, true);
        var folder = Folder.selectDialog('Export to');
        var w = new ExportOptionsSaveForWeb(); w.format = SaveDocumentType.PNG;
        doc.exportDocument(new File(folder.fsName + '/web.png'), ExportType.SAVEFORWEB, w);
        try { doc.exportDocument(new File(folder.fsName + '/deep/web.png'), ExportType.SAVEFORWEB, w); } catch (e) { $.writeln('only inside'); }
        try { doc.save(); } catch (e) { $.writeln('no save'); }
        "#;
    let answers = [json!(chosen.to_string_lossy()), json!(dir.to_string_lossy())];
    let lines = run_answering(&s, src, &answers);
    assert_eq!(lines, ["refused", "only inside", "no save"]);
    assert!(dir.join("chosen.jpg").is_file(), "the JPEG options add their extension to a name without one");
    assert!(dir.join("web.png").is_file());
    assert!(!dir.join("chosen").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn custom_options_are_kept_between_runs() {
    let s = session(8, 8);
    run(
        &s,
        r#"
        var d = new ActionDescriptor();
        var k = stringIDToTypeID('lastName');
        d.putString(k, 'Ada'); d.putInteger(charIDToTypeID('Cnt '), 3); d.putBoolean(stringIDToTypeID('on'), true);
        d.putUnitDouble(stringIDToTypeID('w'), charIDToTypeID('#Pxl'), 12.5);
        app.putCustomOptions('my.script', d, true);
        app.putCustomOptions('my.session', d, false);
        "#,
    );
    let lines = run(
        &s,
        r#"
        var d = app.getCustomOptions('my.script');
        $.writeln(d.count, ' ', d.getString(stringIDToTypeID('lastName')), ' ', d.getInteger(charIDToTypeID('Cnt ')), ' ', d.getBoolean(stringIDToTypeID('on')));
        $.writeln(d.getUnitDoubleValue(stringIDToTypeID('w')), ' ', typeIDToCharID(d.getUnitDoubleType(stringIDToTypeID('w'))), ' ', d.getType(charIDToTypeID('Cnt ')));
        $.writeln(typeIDToStringID(d.getKey(0)), ' ', d.hasKey(charIDToTypeID('Nope')));
        try { d.getInteger(stringIDToTypeID('lastName')); } catch (e) { $.writeln('typed'); }
        app.eraseCustomOptions('my.script');
        try { app.getCustomOptions('my.script'); } catch (e) { $.writeln(e.number); }
        "#,
    );
    assert_eq!(lines, ["4 Ada 3 true", "12.5 #Pxl DescValueType.INTEGERTYPE", "lastName false", "typed", "8500"]);
    let kept = s.borrow().prefs().scripts.custom_options.clone();
    assert!(kept.persistent.is_empty() && kept.session.contains_key("my.session"));
}

#[test]
fn scripts_may_change_only_the_unit_preferences() {
    let s = session(8, 8);
    let lines = run(
        &s,
        r#"
        app.preferences.typeUnits = TypeUnits.PIXELS;
        $.writeln(app.preferences.typeUnits, ' ', app.preferences.rulerUnits);
        try { app.preferences.rulerUnits = 'Units.PARSECS'; } catch (e) { $.writeln('unknown unit'); }
        app.preferences.typeUnits = TypeUnits.POINTS;
        app.preferences.scratchDisks = '/elsewhere';
        $.writeln(typeof app.displayDialogs);
        "#,
    );
    assert_eq!(lines, ["TypeUnits.PIXELS Units.PIXELS", "unknown unit", "string"]);
    let prefs = serde_json::to_value(s.borrow().prefs()).unwrap();
    assert_eq!(prefs["unitsAndRulers"]["typeUnits"], "points");
}

#[test]
fn a_physical_size_is_resized_at_the_new_resolution() {
    let s = session(100, 50);
    let lines =
        run(&s, "var d = app.activeDocument; d.resizeImage(new UnitValue(2, 'in'), undefined, 144); $.writeln(d.width, 'x', d.height, ' ', d.resolution);");
    assert_eq!(lines, ["288 pxx144 px 144"]);
}
