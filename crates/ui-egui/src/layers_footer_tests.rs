//! The Layers panel footer's buttons run PhotoCraft's commands (#2561, #153, #4728). New Group: like
//! Photoshop CC 2014 and later, a click with two or more layers selected puts them in a new,
//! collapsed group, and with one layer it adds an empty group; Shift-click groups the selection
//! too, Alt-click asks in the New Group dialog first and Cmd-click adds the empty group below the
//! active layer. New Layer: Cmd-click adds it below the active layer. Delete removes the targeted
//! mask at once, or asks before deleting the layer (Alt deletes without asking). Add a mask
//! targets the new mask; with a path selected it is a vector mask from that path.

use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Layer, LayerContent};
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::layers_panel_nodes::{click, row_drawn, thumb};
use craft::layers::ui::tree::LayerPart;

type H = Harness<'static, PhotocraftApp>;

/// OK in the open dialog `id`.
fn confirm(h: &mut H, id: craft::ui::dialog::DialogId) {
    let ctx = h.ctx.clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
}

/// Cancel in the open dialog `id`.
fn cancel(h: &mut H, id: craft::ui::dialog::DialogId) {
    let ctx = h.ctx.clone();
    crate::dialogs::cancel(h.state_mut(), &ctx, id).unwrap();
}

/// Background, then "A" and "B" over it, "B" active; "A" selected too when `both`.
fn two(both: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let a = s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap();
    let b = s.execute("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap();
    if both {
        s.execute("layer.select", json!({"layer": a})).unwrap();
        s.execute("layer.select", json!({"layer": b, "mode": "add"})).unwrap();
    }
    s
}

/// "Masked" (pixel and vector mask, both linked to it) and the "Badge" shape layer over it.
fn masked(linked: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let masked = s.execute("layer.new.layer", json!({"name": "Masked"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({"layer": masked})).unwrap();
    let path = json!({"subpaths": [{"knots": [[20, 10], [120, 10], [120, 90], [20, 90]]}]});
    s.execute("layer.vectorMask.add", json!({"layer": masked, "path": path})).unwrap();
    s.execute("layer.layerMask.linked", json!({"linked": linked})).unwrap();
    s.execute("layer.vectorMask.linked", json!({"linked": linked})).unwrap();
    s.execute("shape.create", json!({"kind": "rect", "rect": [140, 20, 40, 40], "fill": "#cc3333", "name": "Badge"})).unwrap();
    s.execute("layer.select", json!({"layer": masked})).unwrap();
    s
}

fn harness(s: Session) -> H {
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    h
}

fn doc(h: &H) -> &photocraft_doc::Document {
    &h.state().session.active().unwrap().doc
}

fn top(h: &H) -> Vec<String> {
    doc(h).layers.iter().map(|l| l.name.clone()).collect()
}

fn groups(h: &H) -> Vec<Layer> {
    doc(h).walk().into_iter().map(|(_, _, l)| l).filter(|l| l.is_group()).cloned().collect()
}

fn layer(h: &H, name: &str) -> Option<Layer> {
    doc(h).walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == name).cloned()
}

/// One new group holds A and B in order, is the only selected layer and is collapsed (its layers'
/// rows are not drawn); one undo puts both back.
fn grouped(h: &mut H) {
    let groups = groups(h);
    assert_eq!(groups.len(), 1);
    let children: Vec<&str> = groups[0].children().unwrap().iter().map(|l| l.name.as_str()).collect();
    assert_eq!(children, ["A", "B"]);
    assert_eq!(doc(h).layers.len(), 2, "the Background and the new group stay at the top level");
    assert_eq!(h.state().session.active().unwrap().selected_layers(), vec![groups[0].id], "the new group is selected");
    assert!(matches!(&groups[0].content, LayerContent::Group(g) if !g.expanded), "the new group is collapsed");
    assert!(row_drawn(h, &groups[0].name) && !row_drawn(h, "A") && !row_drawn(h, "B"), "only the group's row shows");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert!(self::groups(h).is_empty());
    assert_eq!(top(h)[1..], ["A", "B"]);
}

/// One empty group, with the other layers' order `order` (`*` the group).
fn empty_group(h: &H, order: [&str; 3]) {
    let groups = groups(h);
    assert_eq!(groups.len(), 1);
    assert!(groups[0].children().unwrap().is_empty());
    let names: Vec<String> = top(h)[1..].iter().map(|n| if *n == groups[0].name { "*".into() } else { n.clone() }).collect();
    assert_eq!(names, order);
}

/// The New Group dialog opened for `command`; nothing changed before OK. OK confirms it.
fn group_dialog(h: &mut H, command: &str) {
    assert!(groups(h).is_empty(), "nothing happens before OK");
    let id = h.state().framework.dialogs.stack().top().expect("the New Group dialog opened");
    // PLAN: assumed `new_group_ui::owns(key: &str)` answers for the hosted New Group dialog's command key.
    assert!(crate::new_group_ui::owns(command));
    confirm(h, id);
    h.run_steps(4);
}

/// Click the mask thumbnail `part` of "Masked", which targets it.
fn target(h: &mut H, part: LayerPart) {
    let at = thumb(h, "Masked", part).center();
    click(h, at, Modifiers::NONE);
}

/// Delete removed only the targeted mask, at once, and one undo brings it back.
fn mask_deleted(h: &mut H, part: LayerPart) {
    let before = masked(true);
    let original = before.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == "Masked").cloned().unwrap();
    let l = layer(h, "Masked").expect("the layer stays");
    assert!(!h.state().dialog_open(), "mask deletion is immediate");
    assert_eq!(l.content, original.content);
    assert!(layer(h, "Badge").is_some());
    if part == LayerPart::LayerMask {
        assert!(l.mask.is_none() && l.vector_mask.is_some());
        assert!(!h.state().ui.mask_target);
    } else {
        assert!(l.vector_mask.is_none() && l.mask.is_some());
        assert!(!h.state().ui.vector_mask_target);
    }
    h.state_mut().run("edit.undo", json!({})).unwrap();
    let back = layer(h, "Masked").unwrap();
    assert!(back.mask.is_some() && back.vector_mask.is_some());
}

struct Case {
    case: &'static str,
    doc: fn() -> Session,
    before: fn(&mut H),
    button: &'static str,
    mods: Modifiers,
    after: fn(&mut H),
}

#[test]
fn footer_buttons_run_photocraft_commands() {
    let none: fn(&mut H) = |_| {};
    let cases = [
        Case { case: "New Group, two selected", doc: || two(true), before: none, button: "Create a new group", mods: Modifiers::NONE, after: grouped },
        Case { case: "Shift New Group, two selected", doc: || two(true), before: none, button: "Create a new group", mods: Modifiers::SHIFT, after: grouped },
        Case {
            case: "New Group, one selected: an empty group above it",
            doc: || two(false),
            before: none,
            button: "Create a new group",
            mods: Modifiers::NONE,
            after: |h| empty_group(h, ["A", "B", "*"]),
        },
        Case {
            case: "Alt New Group, two selected",
            doc: || two(true),
            before: none,
            button: "Create a new group",
            mods: Modifiers::ALT,
            after: |h| {
                group_dialog(h, "layer.new.groupFromLayers");
                grouped(h);
            },
        },
        Case {
            case: "Alt New Group, one selected",
            doc: || two(false),
            before: none,
            button: "Create a new group",
            mods: Modifiers::ALT,
            after: |h| {
                group_dialog(h, "layer.new.group");
                empty_group(h, ["A", "B", "*"]);
            },
        },
        Case {
            case: "Cmd New Group, two selected: an empty group below the active layer",
            doc: || two(true),
            before: none,
            button: "Create a new group",
            mods: Modifiers::COMMAND,
            after: |h| empty_group(h, ["A", "*", "B"]),
        },
        Case {
            case: "Cmd New Group, one selected",
            doc: || two(false),
            before: none,
            button: "Create a new group",
            mods: Modifiers::COMMAND,
            after: |h| empty_group(h, ["A", "*", "B"]),
        },
        Case {
            case: "New Layer: above the active layer",
            doc: || two(false),
            before: none,
            button: "Create a new layer",
            mods: Modifiers::NONE,
            after: |h| {
                let names = top(h);
                assert_eq!((names.len(), &names[1..3]), (4, &["A".to_string(), "B".to_string()][..]));
            },
        },
        Case {
            case: "Cmd New Layer: below the active layer",
            doc: || two(false),
            before: none,
            button: "Create a new layer",
            mods: Modifiers::COMMAND,
            after: |h| {
                let names = top(h);
                assert_eq!((names.len(), names[1].as_str(), names[3].as_str()), (4, "A", "B"));
            },
        },
        Case {
            case: "Delete with the layer mask targeted",
            doc: || masked(true),
            before: |h| target(h, LayerPart::LayerMask),
            button: "Delete Layer Mask",
            mods: Modifiers::NONE,
            after: |h| mask_deleted(h, LayerPart::LayerMask),
        },
        Case {
            case: "Delete with the vector mask targeted",
            doc: || masked(true),
            before: |h| target(h, LayerPart::VectorMask),
            button: "Delete Vector Mask",
            mods: Modifiers::NONE,
            after: |h| mask_deleted(h, LayerPart::VectorMask),
        },
        Case {
            case: "Delete with an unlinked layer mask targeted",
            doc: || masked(false),
            before: |h| target(h, LayerPart::LayerMask),
            button: "Delete Layer Mask",
            mods: Modifiers::NONE,
            after: |h| mask_deleted(h, LayerPart::LayerMask),
        },
        Case {
            case: "Delete with the content targeted asks first",
            doc: || masked(true),
            before: |h| target(h, LayerPart::Pixels),
            button: "Delete layer",
            mods: Modifiers::NONE,
            after: |h| {
                assert!(layer(h, "Masked").is_some());
                assert!(crate::dialogs::alerts::prompt_tagged::<crate::delete_layer_prompt::PendingDelete>(h.state()).is_some());
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete").click();
                h.run_steps(3);
                assert!(layer(h, "Masked").is_none());
            },
        },
        Case {
            case: "Alt Delete deletes without asking",
            doc: || masked(true),
            before: |h| target(h, LayerPart::Pixels),
            button: "Delete layer",
            mods: Modifiers::ALT,
            after: |h| {
                assert!(!h.state().dialog_open(), "no confirmation");
                assert!(layer(h, "Masked").is_none());
            },
        },
        Case {
            case: "Add a mask targets it, so the next Delete keeps the layer",
            doc: || two(false),
            before: none,
            button: "Add a mask  (from the selection; Alt inverts)",
            mods: Modifiers::NONE,
            after: |h| {
                assert!(h.state().ui.mask_target && !h.state().ui.vector_mask_target);
                assert!(layer(h, "B").unwrap().mask.is_some());
                let at = h.get_by_label("Delete Layer Mask").rect().center();
                click(h, at, Modifiers::NONE);
                assert!(layer(h, "B").is_some_and(|l| l.mask.is_none()));
            },
        },
        Case {
            case: "Add vector mask with a path selected uses that path and targets the mask",
            doc: || {
                let mut s = two(false);
                s.execute("layer.layerMask.revealAll", json!({})).unwrap();
                let path = json!({"subpaths": [{"closed": true, "knots": [[30, 20], [90, 20], [90, 80], [30, 80]]}]});
                s.execute("path.set", json!({"name": "work", "path": path})).unwrap();
                s
            },
            before: |h| {
                h.state_mut().ui.selected_path = Some("work".into());
                h.run_steps(3);
            },
            button: "Add vector mask",
            mods: Modifiers::NONE,
            after: |h| {
                let work = doc(h).work_path.clone().unwrap();
                assert_eq!(layer(h, "B").unwrap().vector_mask.map(|v| v.path), Some(work), "the selected path, not Reveal All");
                assert!(h.state().ui.vector_mask_target && !h.state().ui.mask_target, "the new vector mask is targeted");
                assert_eq!(h.state().ui.selected_path.as_deref(), Some("layer"), "the mask's path becomes the active path");
            },
        },
    ];
    for c in cases {
        let mut h = harness((c.doc)());
        (c.before)(&mut h);
        let label = c.button.replace("Alt", &crate::shortcuts::pretty("Alt"));
        let at = h.get_by_label(&label).rect().center();
        click(&mut h, at, c.mods);
        h.run_steps(2);
        eprintln!("{}", c.case);
        (c.after)(&mut h);
    }
}

/// The prompt a drop on the trash opened: the layer mask's apply-or-delete question, or with `vector` the vector
/// mask's delete question. Nothing changed before the answer.
fn trash_prompt(h: &H, vector: bool) -> craft::ui::dialog::DialogId {
    use craft::ui::dialog::{PromptKind, StandardPrompt};
    let id = crate::dialogs::alerts::prompt_tagged::<crate::mask_trash_prompt::MaskTrash>(h.state()).expect("the prompt is open");
    let kind = h.state().framework.dialogs.get::<StandardPrompt>().map(|prompt| prompt.kind().clone());
    let want = if vector { PromptKind::TrashVectorMask } else { PromptKind::TrashMask };
    assert!(kind == Some(want), "the prompt asks the {} question", if vector { "vector mask" } else { "layer mask" });
    let l = layer(h, "Masked").expect("the layer stays");
    assert!(l.mask.is_some() && l.vector_mask.is_some(), "nothing changes before the answer");
    id
}

fn past(h: &H) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

/// A drag onto the trash (#3113, #3119): a row deletes its layer, even with a mask targeted; a
/// layer mask's thumbnail asks to apply or delete it; a vector mask's thumbnail asks to delete it.
#[test]
fn dropping_onto_the_trash() {
    type Case = (&'static str, Option<LayerPart>, &'static str, fn(&mut H));
    let cases: [Case; 3] = [
        ("the row, with the layer mask targeted, deletes the layer", None, "Delete Layer Mask", |h| assert!(layer(h, "Masked").is_none())),
        ("the layer mask asks to apply or delete it", Some(LayerPart::LayerMask), "Delete Layer Mask", |h| {
            let pixels = |h: &H| layer(h, "Masked").unwrap().content;
            let (before, steps) = (pixels(h), past(h));
            let id = trash_prompt(h, false);
            confirm(h, id);
            assert!(layer(h, "Masked").unwrap().mask.is_none());
            assert_ne!(pixels(h), before, "applied, not deleted");
            assert_eq!(past(h), steps + 1);
            h.state_mut().session.undo();
            h.run_steps(2);
            // The undo targets the layer again: the trash reads Delete Layer Mask with the mask targeted.
            target(h, LayerPart::LayerMask);
            let at = thumb(h, "Masked", LayerPart::LayerMask).center();
            let to = h.get_by_label("Delete Layer Mask").rect().center();
            crate::layers_panel_nodes::drag(h, at, to, Modifiers::NONE);
            let id = trash_prompt(h, false);
            let ctx = h.ctx.clone();
            h.state_mut().framework.dialogs.set_json_field(&ctx, id, "choice", json!("alternate")).unwrap();
            confirm(h, id);
            assert!(layer(h, "Masked").unwrap().mask.is_none());
            assert_eq!(pixels(h), before, "deleted, not applied");
        }),
        ("the vector mask asks to delete just the mask", Some(LayerPart::VectorMask), "Delete Vector Mask", |h| {
            let steps = past(h);
            let id = trash_prompt(h, true);
            cancel(h, id);
            assert!(layer(h, "Masked").unwrap().vector_mask.is_some(), "Cancel keeps it");
            assert_eq!(past(h), steps);
            let at = thumb(h, "Masked", LayerPart::VectorMask).center();
            let to = h.get_by_role_and_label(egui::accesskit::Role::Button, "Delete Vector Mask").rect().center();
            crate::layers_panel_nodes::drag(h, at, to, Modifiers::NONE);
            let id = trash_prompt(h, true);
            confirm(h, id);
            let l = layer(h, "Masked").unwrap();
            assert!(l.vector_mask.is_none() && l.mask.is_some(), "only the vector mask goes");
            assert_eq!(past(h), steps + 1);
        }),
    ];
    for (case, from, trash, after) in cases {
        // A mask that hides the left half, so applying it changes the pixels.
        let mut s = masked(true);
        let id = s.active().unwrap().active_layer.unwrap().0;
        s.execute("layer.layerMask.delete", json!({"layer": id})).unwrap();
        s.execute("select.rect", json!({"x": 100, "y": 0, "width": 100, "height": 100})).unwrap();
        s.execute("layer.layerMask.revealSelection", json!({"layer": id})).unwrap();
        s.execute("select.deselect", json!({})).ok();
        let mut h = harness(s);
        let part = from.unwrap_or(LayerPart::LayerMask);
        target(&mut h, part);
        let at = match from {
            Some(part) => thumb(&h, "Masked", part).center(),
            None => crate::layers_panel_nodes::name_point(&h, "Masked"),
        };
        let to = h.get_by_label(trash).rect().center();
        crate::layers_panel_nodes::drag(&mut h, at, to, Modifiers::NONE);
        eprintln!("{case}");
        after(&mut h);
    }
}
