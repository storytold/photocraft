//! The document model scripts see, run against a real session: layers and sets, type,
//! History, the selection, sizes in the ruler unit, saving, custom options and preferences.
// Native only: the tests time runs with `std::time`, which has no clock on wasm32-unknown-unknown.
#![cfg(not(target_arch = "wasm32"))]

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

/// Reading a text's position doesn't switch documents, and setting it lands where asked even
/// when converting the new position moves the text first.
#[test]
fn text_position_reads_in_place_and_sets_from_where_the_text_is() {
    let s = session(200, 100);
    s.borrow_mut().execute("file.new", json!({"width": 50, "height": 50, "name": "Other"})).unwrap();
    let lines = run(
        &s,
        r#"
        var a = app.documents[0];
        app.activeDocument = a;
        var t = a.artLayers.add(); t.kind = LayerKind.TEXT; t.textItem.contents = 'Hi'; t.textItem.position = [10, 50];
        app.activeDocument = app.documents[1];
        var at = t.textItem.position;
        $.writeln(at[0].value, ',', at[1].value, ' ', app.activeDocument.name);
        var x = { valueOf: function () { t.textItem.position = [20, 50]; return 30; } };
        t.textItem.position = [x, 50];
        $.writeln(t.textItem.position[0].value);
        "#,
    );
    assert_eq!(lines, ["10,50 Other", "30"]);
}

/// Adding through a group's collection after that group is gone fails and makes nothing: the
/// layer is made inside the group in one edit, not made and then moved.
#[test]
fn adding_to_a_deleted_groups_collection_changes_nothing() {
    let s = session(20, 20);
    let lines = run(
        &s,
        "var d = app.activeDocument, g = d.layerSets.add(), layers = g.artLayers, sets = g.layerSets;
         g.remove();
         var n = d.layers.length;
         try { layers.add(); $.writeln('added'); } catch (e) { $.writeln('refused'); }
         try { sets.add(); $.writeln('added'); } catch (e) { $.writeln('refused'); }
         $.writeln(d.layers.length === n);",
    );
    assert_eq!(lines, ["refused", "refused", "true"]);
}

/// A layer's locks read and set as the layer's own: Lock All, pixels, position and transparent
/// pixels (which the pixels lock includes, so it stays off while that is on).
#[test]
fn layer_locks_are_read_and_set() {
    let s = session(20, 20);
    let lines = run(
        &s,
        "var l = app.activeDocument.artLayers.add();
         $.writeln(l.allLocked, l.pixelsLocked, l.positionLocked, l.transparentPixelsLocked);
         l.allLocked = true; l.positionLocked = true;
         $.writeln(l.allLocked, l.pixelsLocked, l.positionLocked, l.transparentPixelsLocked);
         l.allLocked = false; l.pixelsLocked = true; l.transparentPixelsLocked = true;
         $.writeln(l.allLocked, l.pixelsLocked, l.positionLocked, l.transparentPixelsLocked);
         var g = app.activeDocument.layerSets.add(); g.allLocked = true; $.writeln(g.allLocked);",
    );
    assert_eq!(lines, ["falsefalsefalsefalse", "truefalsetruefalse", "falsetruetruefalse", "true"]);
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
    assert!(s.borrow().prefs().scripts.custom_options.persistent.is_empty(), "erased");
    let session = s.borrow_mut().execute("scripts.customOptions.get", json!({"key": "my.session"})).unwrap();
    assert!(session.is_object(), "the session's own options are still there");
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

#[test]
fn custom_options_too_large_to_keep_say_so() {
    let s = session(8, 8);
    let lines = run(
        &s,
        r#"
        // Within the limit as text, past it encoded: each quote is escaped to two bytes.
        var d = new ActionDescriptor(), big = Array(30001).join('x'), quotes = Array(30001).join('"');
        for (var i = 0; i < 2; i++) { d.putString(i + 1, quotes); }
        try { app.putCustomOptions('big', d); } catch (e) { $.writeln(e.name, ' ', e.message.indexOf('64 KiB') >= 0); }
        // Texts far past the limit are refused the same way.
        var huge = new ActionDescriptor(); for (var i = 0; i < 1000; i++) { huge.putString(i + 1, big); }
        try { app.putCustomOptions('huge', huge); } catch (e) { $.writeln(e.name, ' ', e.message.indexOf('64 KiB') >= 0); }
        "#,
    );
    assert_eq!(lines, ["RangeError true", "RangeError true"]);
}

#[test]
fn a_history_state_an_edit_discarded_is_gone_not_another_state() {
    let s = session(32, 32);
    s.borrow_mut().execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.borrow_mut().execute("layer.new.layer", json!({"name": "B"})).unwrap();
    let lines = run(
        &s,
        r#"
        var doc = app.activeDocument, h = doc.historyStates, last = h[2];
        doc.activeHistoryState = h[1];
        doc.artLayers.add();
        try { $.writeln(last.name); } catch (e) { $.writeln(e.message); }
        $.writeln(h[1].name, ' ', doc.historyStates.length);
        // The new state took the discarded one's row: selecting it goes by the state, not a row.
        var made = doc.activeHistoryState;
        doc.activeHistoryState = doc.historyStates[0];
        doc.activeHistoryState = made;
        $.writeln(doc.activeHistoryState == made, ' ', doc.layers.length);
        "#,
    );
    assert_eq!(lines, ["the History state no longer exists", "New Layer 3", "true 3"]);
}

#[test]
fn layers_added_to_the_document_go_beside_the_active_layer() {
    let s = session(32, 32);
    let lines = run(
        &s,
        r#"
        var doc = app.activeDocument;
        var set = doc.layerSets.add(); set.name = 'G';
        var inner = set.artLayers.add();
        doc.activeLayer = inner;
        var beside = doc.artLayers.add(); beside.name = 'Beside';
        $.writeln(beside.parent == set, ' ', set.layers.length, ' ', doc.layers.length);
        beside.move(doc, ElementPlacement.PLACEATBEGINNING);
        $.writeln(beside.parent == doc, ' ', set.layers.length, ' ', doc.layers.length);
        "#,
    );
    assert_eq!(lines, ["true 2 2", "true 1 3"]);
}

#[test]
fn making_a_layer_text_keeps_its_settings() {
    let s = session(64, 32);
    let lines = run(
        &s,
        r#"
        var doc = app.activeDocument;
        var t = doc.artLayers.add();
        t.name = 'Caption'; t.opacity = 30; t.blendMode = BlendMode.SCREEN; t.visible = false;
        t.kind = LayerKind.TEXT;
        $.writeln(t.kind, ' ', t.name, ' ', t.opacity, ' ', t.blendMode, ' ', t.visible, ' ', doc.layers.length);
        "#,
    );
    assert_eq!(lines, ["LayerKind.TEXT Caption 30 BlendMode.SCREEN false 2"]);
}

#[test]
fn a_layer_moves_to_the_beginning_or_the_end_of_a_group() {
    let s = session(32, 32);
    let lines = run(
        &s,
        r#"
        var doc = app.activeDocument, set = doc.layerSets.add();
        var a = set.artLayers.add(); a.name = 'A';
        var b = set.artLayers.add(); b.name = 'B';
        var x = doc.artLayers.add(); x.name = 'X';
        x.move(set, ElementPlacement.PLACEATEND);
        var names = []; for (var i = 0; i < set.layers.length; i++) names.push(set.layers[i].name);
        $.writeln(names.join(','));
        x.move(set, ElementPlacement.PLACEATBEGINNING);
        names = []; for (var i = 0; i < set.layers.length; i++) names.push(set.layers[i].name);
        $.writeln(names.join(','));
        "#,
    );
    let first = lines[0].split(',').next_back().map(str::to_owned);
    assert_eq!(first.as_deref(), Some("X"), "{lines:?}");
    assert_eq!(lines[1].split(',').next(), Some("X"), "{lines:?}");
}

/// A stop that lands on a document read inside `try` is not caught: every budget that stops
/// the loop stops it at such a read (nothing else in it is charged), and no `catch` runs.
#[test]
fn a_spent_time_limit_stays_uncatchable_at_a_document_read() {
    let mut reached = 0;
    for charges in 2..16 {
        let s = session(8, 8);
        let host = SessionHost::new(s.clone(), "stop");
        let out = host.output();
        let src = "var doc = app.activeDocument; $.writeln('loop'); for (var i = 0; i < 100000; i++) { try { doc.name; } catch (e) { $.writeln('caught'); } }";
        let mut r = Runner::new(src, Box::new(host), Box::new(crate::run::Countdown(charges)), RunOptions::default()).unwrap();
        let Step::Failed(m) = r.step() else { panic!("{charges}: the run was stopped") };
        assert!(m.contains("time limit"), "{charges}: {m}");
        let out = out.borrow();
        assert!(!out.iter().any(|l| l == "caught"), "{charges}: {out:?}");
        // Past the line, the only charged calls left are the reads inside `try`.
        reached += usize::from(out.iter().any(|l| l == "loop"));
    }
    assert!(reached > 5, "most budgets stop the loop at a read inside try ({reached})");
}

/// A fresh run can name the keys of custom options it reads before it uses those names itself.
#[test]
fn custom_options_keep_their_key_names_between_runs() {
    let s = session(8, 8);
    run(&s, "var d = new ActionDescriptor(); d.putString(stringIDToTypeID('lastName'), 'Ada'); app.putCustomOptions('names', d, true);");
    let lines = run(&s, "var d = app.getCustomOptions('names'); $.writeln(typeIDToStringID(d.getKey(0)));");
    assert_eq!(lines, ["lastName"]);
}

/// Key names are saved only when they fit: a descriptor that fits without them still saves.
#[test]
fn custom_options_that_fit_only_without_key_names_still_save() {
    let s = session(8, 8);
    let lines = run(
        &s,
        "var d = new ActionDescriptor(), v = new Array(32001).join('x');
         d.putString(stringIDToTypeID(new Array(901).join('a')), v);
         d.putString(stringIDToTypeID(new Array(901).join('b')), v);
         app.putCustomOptions('big', d, true);
         var back = app.getCustomOptions('big'); $.writeln(back.count, ' ', back.getString(back.getKey(1)).length);",
    );
    assert_eq!(lines, ["2 32000"]);
}

/// Two strings that hash to one ID are refused, not mixed up as one descriptor key.
#[test]
fn string_ids_that_collide_are_refused() {
    let s = session(8, 8);
    let lines = run(
        &s,
        "var a = stringIDToTypeID('costarring'); try { stringIDToTypeID('liquid'); $.writeln('same key') } catch (e) { $.writeln('refused') } $.writeln(stringIDToTypeID('costarring') == a);",
    );
    assert_eq!(lines, ["refused", "true"]);
}

/// Only names of an identifier's length get an id: each is remembered, so a collision between
/// two of them is always caught; a longer string is a RangeError, not an id no one checks.
#[test]
fn only_identifier_length_string_ids_are_made() {
    let s = session(8, 8);
    let lines = run(
        &s,
        r#"
        var short = stringIDToTypeID('layerName'), long = new Array(5000).join('x');
        var r; try { stringIDToTypeID(long); r = 'made'; } catch (e) { r = e instanceof RangeError ? 'range' : String(e); }
        $.writeln(typeIDToStringID(short), ' ', r, ' ', typeIDToStringID(stringIDToTypeID(new Array(1025).join('y'))).length);
        "#,
    );
    assert_eq!(lines, ["layerName range 1024"]);
}

/// Past the run's count of names a new one gets no id (a RangeError) rather than an id that
/// could collide unnoticed; names already made still work.
#[test]
fn string_ids_past_the_runs_count_of_names_are_refused() {
    let s = session(8, 8);
    let lines = run(
        &s,
        "var made = 0, r = 'none';
         try { for (var i = 0; i < 70000; i++) { stringIDToTypeID('n' + i); made++; } } catch (e) { r = e instanceof RangeError ? 'range' : String(e); }
         $.writeln(made, ' ', r, ' ', stringIDToTypeID('n0') === stringIDToTypeID('n0'));",
    );
    assert_eq!(lines, ["65536 range true"]);
}

/// A saved key name whose ID this run already gave another string is refused on reading, not
/// read back as that other string's key.
#[test]
fn custom_options_whose_saved_name_collides_with_this_runs_are_refused() {
    let s = session(8, 8);
    run(&s, "var d = new ActionDescriptor(); d.putString(stringIDToTypeID('costarring'), 'x'); app.putCustomOptions('c', d, true);");
    let lines = run(&s, "stringIDToTypeID('liquid'); try { app.getCustomOptions('c'); $.writeln('read'); } catch (e) { $.writeln('refused: ' + e.message); }");
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("refused: getCustomOptions: the saved name \"costarring\""), "{lines:?}");
}

/// A collection's methods are its own properties, defined rather than assigned: an accessor a
/// script put on `Array.prototype` neither runs nor stands in for them.
#[test]
fn collection_methods_are_own_properties_whatever_array_prototype_holds() {
    let s = session(8, 8);
    let lines = run(
        &s,
        r#"
        var hits = 0;
        ['add', 'getByName'].forEach(function (k) {
            Object.defineProperty(Array.prototype, k, { get: function () { return null; }, set: function () { hits++; }, configurable: true });
        });
        var docs = app.documents, layers = app.activeDocument.artLayers, all = app.activeDocument.layers;
        $.writeln(hits, ' ', docs.hasOwnProperty('add'), docs.hasOwnProperty('getByName'), layers.hasOwnProperty('add'), all.hasOwnProperty('getByName'));
        $.writeln(docs.getByName('Doc').name, ' ', typeof layers.add);
        "#,
    );
    assert_eq!(lines, ["0 truetruetruetrue", "Doc function"]);
}

/// Records the commands a run asks of its session.
struct Logged(SessionHost, Rc<RefCell<Vec<(String, Value)>>>);

impl photocraft_script_js::Host for Logged {
    fn execute(&mut self, id: &str, params: Value) -> Result<Value, photocraft_script_js::HostError> {
        self.1.borrow_mut().push((id.to_owned(), params.clone()));
        self.0.execute(id, params)
    }

    fn write_line(&mut self, line: &str) {
        self.0.write_line(line);
    }
}

/// A document's own values (name, size, resolution) are read without its layers or History,
/// which a script reading them in a loop would otherwise have built for every read.
#[test]
fn a_documents_own_values_are_read_without_its_layers_or_history() {
    let s = session(30, 20);
    let log = Rc::new(RefCell::new(Vec::new()));
    let host = Logged(SessionHost::new(s.clone(), "values"), log.clone());
    let out = host.0.output();
    let src = "var d = app.activeDocument; $.writeln(d.name, ' ', d.width, ' ', d.height, ' ', d.resolution);";
    let mut r = Runner::new(src, Box::new(host), Box::new(Unlimited), RunOptions::default()).unwrap();
    assert_eq!(r.step(), Step::Done);
    assert_eq!(*out.borrow(), ["Doc 30 px 20 px 72"]);
    let reads: Vec<Value> = log.borrow().iter().filter(|(id, _)| id == "document.inspect").map(|(_, p)| p.clone()).collect();
    assert_eq!(reads.len(), 4, "{reads:?}");
    for p in reads {
        assert_eq!((p["layers"].as_bool(), p["history"].as_bool()), (Some(false), Some(false)), "{p}");
    }
}

/// A size's `valueOf` that resizes the document runs before the side left out is read: that
/// side keeps the document's size, or its proportions, as they are once the size is converted.
#[test]
fn a_side_left_out_is_read_after_the_other_is_converted() {
    let s = session(100, 50);
    let lines = run(
        &s,
        "var d = app.activeDocument;
         d.resizeCanvas({ valueOf: function () { d.resizeCanvas(100, 80, AnchorPosition.TOPLEFT); return 120; } }, undefined, AnchorPosition.TOPLEFT);
         $.writeln(d.width, 'x', d.height);
         d.resizeImage({ valueOf: function () { d.resizeCanvas(200, 50, AnchorPosition.TOPLEFT); return 100; } });
         $.writeln(d.width, 'x', d.height);",
    );
    assert_eq!(lines, ["120 pxx80 px", "100 pxx25 px"]);
}

/// The resample method and the anchor are converted before the side left out is read: their
/// `toString` may itself resize the document.
#[test]
fn a_side_left_out_is_read_after_the_method_or_anchor_is_converted() {
    let s = session(100, 50);
    let lines = run(
        &s,
        "var d = app.activeDocument;
         d.resizeImage(120, undefined, undefined, { toString: function () { d.resizeCanvas(100, 80, AnchorPosition.TOPLEFT); return 'ResampleMethod.BICUBIC'; } });
         $.writeln(d.width, 'x', d.height);
         d.resizeCanvas(200, undefined, { toString: function () { d.resizeCanvas(120, 50, AnchorPosition.TOPLEFT); return 'AnchorPosition.TOPLEFT'; } });
         $.writeln(d.width, 'x', d.height);",
    );
    assert_eq!(lines, ["120 pxx96 px", "200 pxx50 px"]);
}

/// A length in physical units is made pixels at the resolution the document has once every
/// argument is converted: a resample method's or anchor's `toString` that changes it counts.
#[test]
fn physical_lengths_use_the_resolution_after_every_argument_is_converted() {
    let s = session(100, 50);
    let lines = run(
        &s,
        "var d = app.activeDocument;
         var to144 = function (name) { return { toString: function () { d.resizeImage(undefined, undefined, 144); return name; } }; };
         d.resizeImage(new UnitValue(1, 'in'), undefined, undefined, to144('ResampleMethod.BICUBIC'));
         $.writeln(d.width.as('px'), ' ', d.resolution);
         d.resizeImage(undefined, undefined, 72);
         d.resizeCanvas(new UnitValue(1, 'in'), undefined, to144('AnchorPosition.TOPLEFT'));
         $.writeln(d.width.as('px'), ' ', d.resolution);",
    );
    assert_eq!(lines, ["144 144", "144 144"]);
}

/// A percentage `UnitValue` is a share of its base unit, not a count of pixels; one whose base
/// is itself relative is refused.
#[test]
fn a_percentage_length_is_a_share_of_its_base() {
    let s = session(100, 50);
    let lines = run(
        &s,
        "var d = app.activeDocument, q = new UnitValue(25, '%'); q.baseUnit = new UnitValue(4, 'in');
         d.resizeImage(undefined, undefined, 100);
         d.resizeCanvas(q, undefined, AnchorPosition.TOPLEFT);
         $.writeln(d.width.as('px'));
         var r = new UnitValue(25, '%'); r.baseUnit = new UnitValue(10, 'px');
         try { d.resizeCanvas(r, undefined, AnchorPosition.TOPLEFT); $.writeln('resized'); } catch (e) { $.writeln(e.name); }",
    );
    assert_eq!(lines, ["100", "TypeError"]);
}

/// Text goes only where it can be made: a position past the range `type.create` takes is
/// refused, so every supported position stays one move away.
#[test]
fn text_moves_only_within_the_supported_positions() {
    let s = session(64, 64);
    let lines = run(
        &s,
        "var t = app.activeDocument.artLayers.add(); t.kind = LayerKind.TEXT; t.textItem.contents = 'Hi';
         try { t.textItem.position = [400000, 0]; $.writeln('moved'); } catch (e) { $.writeln(e.name); }
         t.textItem.position = [300000, 0]; t.textItem.position = [-300000, 0];
         $.writeln(t.textItem.position[0].as('px'));",
    );
    assert_eq!(lines, ["RangeError", "-300000"]);
}

/// Reading a large document's layers again and again records each unchanged inspection briefly,
/// so a script that only reads does not run into the bound on recorded host calls; an edit
/// between reads is still seen.
#[test]
fn reading_an_unchanged_document_again_records_little() {
    let s = session(16, 16);
    for i in 0..700 {
        s.borrow_mut().execute("layer.new.layer", json!({"name": format!("L{i}")})).unwrap();
    }
    let lines = run(
        &s,
        "var doc = app.activeDocument, n = 0;
         for (var i = 0; i < doc.layers.length; i++) n += doc.layers[i].name.length > 0 ? 1 : 0;
         doc.layers[0].name = 'Renamed';
         $.writeln(n, ' ', doc.layers[0].name);",
    );
    assert_eq!(lines, ["701 Renamed"]);
}

/// Saved key names are checked all together before any is kept: a set refused at the run's
/// count of names leaves none of them taking a place.
#[test]
fn custom_options_refused_at_the_count_of_names_keep_none_of_theirs() {
    let s = session(8, 8);
    run(
        &s,
        "var d = new ActionDescriptor(); d.putString(stringIDToTypeID('first'), 'x'); d.putString(stringIDToTypeID('second'), 'y'); app.putCustomOptions('two', d, true);",
    );
    let lines = run(
        &s,
        "for (var i = 0; i < 65535; i++) stringIDToTypeID('n' + i);
         try { app.getCustomOptions('two'); $.writeln('read'); } catch (e) { $.writeln('refused'); }
         try { stringIDToTypeID('fresh'); $.writeln('named'); } catch (e) { $.writeln(e.message); }",
    );
    assert_eq!(lines, ["refused", "named"]);
}

/// A char ID is four printable ASCII characters: anything else is refused rather than read as
/// `?`, so every ID it gives converts back to the characters it was given.
#[test]
fn char_ids_are_four_printable_ascii_characters() {
    let s = session(8, 8);
    let lines = run(
        &s,
        r#"
        var out = [typeIDToCharID(charIDToTypeID('Nm  '))];
        var bad = ['\u00e9abc', '\u0001abc', 'abc', 'abcde', '\u20acabc'];
        for (var i = 0; i < bad.length; i++) { try { charIDToTypeID(bad[i]); out.push('made'); } catch (e) { out.push('refused'); } }
        $.writeln(out.join(','));
        "#,
    );
    assert_eq!(lines, ["Nm  ,refused,refused,refused,refused,refused"]);
}

#[test]
fn lengths_that_are_not_numbers_or_too_far_are_refused() {
    let s = session(32, 32);
    let lines = run(
        &s,
        r#"
        var doc = app.activeDocument, l = doc.artLayers.add();
        function t(f) { try { f(); return 'ok'; } catch (e) { return e instanceof RangeError ? 'range' : e instanceof TypeError ? 'type' : String(e); } }
        $.writeln([
            t(function () { l.translate(Infinity, 0); }),
            t(function () { l.translate(NaN, 0); }),
            t(function () { doc.selection.select([[0, 0], [1e300, 1e300]]); }),
            t(function () { doc.resizeImage(NaN); }),
            t(function () { doc.resizeImage(1e7, 10); }),
            t(function () { doc.resizeCanvas(10, 1e7); }),
            t(function () { doc.crop([0, 0, NaN, 10]); }),
            t(function () { l.translate(2, 3); })
        ].join(' '));
        "#,
    );
    // A length that is no number is a TypeError, one too far a RangeError.
    assert_eq!(lines, ["type type range type range range type ok"]);
}

#[test]
fn a_duplicate_that_cannot_be_placed_leaves_no_copy() {
    let s = session(32, 32);
    let lines = run(
        &s,
        r#"
        var doc = app.activeDocument, l = doc.artLayers.add(); l.name = 'Only';
        var n = doc.layers.length;
        try { l.duplicate(doc, 'ElementPlacement.NOWHERE'); } catch (e) { $.writeln('refused'); }
        $.writeln(doc.layers.length == n);
        "#,
    );
    assert_eq!(lines, ["refused", "true"]);
}

/// A run stopped while a duplicate's placement is read leaves no copy: the placement is read
/// before anything is copied, and the copy is made where it goes in one command.
#[test]
fn a_run_stopped_while_a_duplicate_is_placed_leaves_no_copy() {
    let s = session(32, 32);
    run(&s, "var l = app.activeDocument.artLayers.add(); l.name = 'Only';");
    let before = layer_names(&s);
    let host = SessionHost::new(s.clone(), "stop");
    let src = "var doc = app.activeDocument; doc.layers[0].duplicate(doc, { toString: function () { for (;;) { doc.name; } } });";
    let budget = photocraft_script_js::WallClock::new(std::time::Duration::from_millis(200));
    let mut r = Runner::new(src, Box::new(host), Box::new(budget), RunOptions::default()).unwrap();
    let Step::Failed(m) = r.step() else { panic!("the run was stopped") };
    assert!(m.contains("time limit"), "{m}");
    assert_eq!(layer_names(&s), before);
}

/// A group copied into itself is made and placed with no budget charge between the two: wherever
/// the run's budget stops it, the copy is inside the group or not made, never left beside it.
#[test]
fn a_run_stopped_while_a_group_is_copied_into_itself_leaves_no_copy_beside_it() {
    let mut landed = false;
    for charges in 1..120 {
        let s = session(32, 32);
        run(&s, "var g = app.activeDocument.layerSets.add(); g.name = 'G';");
        let before = layer_names(&s);
        let host = SessionHost::new(s.clone(), "stop");
        let src = "var g = app.activeDocument.layerSets[0]; g.duplicate(g, ElementPlacement.INSIDE); $.writeln('done');";
        let mut r = Runner::new(src, Box::new(host), Box::new(crate::run::Countdown(charges)), RunOptions::default()).unwrap();
        let step = r.step();
        assert_eq!(layer_names(&s), before, "{charges} charges ({step:?}): nothing beside the group");
        if step == Step::Done {
            let d = s.borrow_mut().execute("document.inspect", json!({})).unwrap();
            assert_eq!(d["layers"][0]["children"].as_array().map(Vec::len), Some(1), "{charges}: the copy is inside the group");
            landed = true;
        }
    }
    assert!(landed, "with enough charges the copy is made");
}

/// A duplicate placed beside another layer, or inside a group, lands there.
#[test]
fn a_duplicate_lands_where_it_is_placed() {
    let s = session(32, 32);
    let lines = run(
        &s,
        r#"
        var doc = app.activeDocument, a = doc.artLayers.add(); a.name = 'A';
        var g = doc.layerSets.add(); g.name = 'G';
        var below = a.duplicate(doc.layers[doc.layers.length - 1], ElementPlacement.PLACEBEFORE);
        var inside = a.duplicate(g, ElementPlacement.INSIDE);
        var names = []; for (var i = 0; i < doc.layers.length; i++) names.push(doc.layers[i].name);
        $.writeln(names.join(','), ' ', g.layers.length, ' ', g.layers[0].name, ' ', inside.parent == g, ' ', below.name);
        try { a.duplicate(a, ElementPlacement.INSIDE); } catch (e) { $.writeln('refused'); }
        $.writeln(doc.layers.length);
        // A group copied into itself: inside it, and at its end.
        var gin = g.duplicate(g, ElementPlacement.INSIDE), gend = g.duplicate(g, ElementPlacement.PLACEATEND);
        $.writeln(g.layers.length, ' ', gin.parent == g, ' ', gend.parent == g, ' ', doc.layers.length);
        "#,
    );
    assert_eq!(lines, ["G,A,A copy,Background 1 A copy 2 true A copy", "refused", "4", "3 true true 4"]);
}

#[test]
fn a_replaced_unit_value_global_runs_no_script_code_in_a_host_call() {
    let s = session(32, 32);
    let lines = run(
        &s,
        r#"
        var ran = false, doc = app.activeDocument;
        UnitValue = { get prototype() { ran = true; return {}; } };
        var w = doc.width;
        $.writeln(ran, ' ', typeof w.as);
        "#,
    );
    assert_eq!(lines, ["false function"]);
}

#[test]
fn a_collection_read_twice_is_the_same_object_and_stays_current() {
    let s = session(32, 32);
    let lines = run(
        &s,
        r#"
        var doc = app.activeDocument, layers = doc.layers, n = layers.length;
        doc.artLayers.add();
        $.writeln(app.documents === app.documents, ' ', doc.layers === layers, ' ', doc.artLayers === doc.artLayers,
            ' ', app.documents[0] === app.documents[0], ' ', doc.layers.length === n + 1, ' ', layers.length === n + 1);
        "#,
    );
    assert_eq!(lines, ["true true true true true true"]);
}

/// What nearly every published script starts with: `app.bringToFront()`, `$.os`, and `app`'s
/// properties named bare (`activeDocument`, `preferences`), as Photoshop resolves them.
#[test]
fn the_globals_published_scripts_start_with_are_there() {
    let s = session(30, 20);
    let lines = run(
        &s,
        r#"
        app.bringToFront();
        $.writeln(typeof $.os, ' ', $.os.length > 0);
        $.writeln(activeDocument == app.activeDocument, ' ', activeDocument.name, ' ', documents.length);
        var was = preferences.rulerUnits;
        preferences.rulerUnits = Units.PIXELS;
        $.writeln(app.preferences.rulerUnits == Units.PIXELS);
        preferences.rulerUnits = was;
        $.writeln(app.stringIDToTypeID('bmpDepth') == stringIDToTypeID('bmpDepth'), ' ', app.typeIDToStringID(app.stringIDToTypeID('bmpDepth')));
        "#,
    );
    assert_eq!(lines, ["string true", "true Doc 1", "true", "true bmpDepth"]);
    // A script's own variable of one of those names is its own.
    let lines = run(
        &s,
        "var documents = ['a', 'b', 'c']; var preferences = { mine: true }; $.writeln(documents.length, ' ', preferences.mine, ' ', app.documents.length, ' ', app.preferences.mine, ' ', app.preferences.rulerUnits !== undefined);",
    );
    assert_eq!(lines, ["3 true 1 undefined true"], "the script's variables are its own and app's stay app's");
}

/// `doc.suspendHistory(name, code)` runs `code` in the global scope; the run is one History step
/// already.
#[test]
fn suspend_history_runs_its_code() {
    let s = session(30, 20);
    let lines = run(
        &s,
        r#"
        var ran = 0;
        function main() { ran++; app.activeDocument.artLayers.add().name = 'Inside'; }
        app.activeDocument.suspendHistory('Step', 'main()');
        $.writeln(ran, ' ', app.activeDocument.activeLayer.name);
        try { app.activeDocument.suspendHistory.call({}, 'x', 'main()'); } catch (e) { $.writeln('refused'); }
        $.writeln(ran);
        "#,
    );
    assert_eq!(lines, ["1 Inside", "refused", "1"]);
    assert!(layer_names(&s).contains(&"Inside".to_owned()));
}
