//! #127: every shortcut the UI displays (menu items, shell commands, tool tooltips) dispatches its
//! command, or the item is visibly disabled with a reason, in the whole app with a realistic layered
//! document. Every shortcut's command is fired into the dispatcher directly; one shortcut of each kind
//! of key and route is pressed from the places focus usually is (#15678).

use photocraft_engine::actions_cmds::ActionsExt;
use std::collections::HashSet;

use craft::keymap::Chord;
use craft::ui::runtime::focus;
use craft::ui::runtime::keys::focus_holds;
use craft::ui::runtime::windows::WindowHost;
use egui::accesskit::Role;
use egui::{Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::LayerContent;
use serde_json::json;

use super::{Outcome, command_rows, set_dry_run, take_log};
use crate::PhotocraftApp;
use crate::shortcuts::parse;
use crate::state::Tool;

/// Background, a filled pixel layer "paint" (active, with a selection), a type layer and a group
/// holding a layer: what a real layout PSD looks like.
fn realistic() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 240, "height": 160})).unwrap();
    s.execute("layer.new.layer", json!({"name": "in-group"})).unwrap();
    s.execute("layer.groupLayers", json!({"name": "group"})).unwrap();
    s.execute("type.create", json!({"x": 20, "y": 60, "text": "Hello"})).unwrap();
    let paint = s.execute("layer.new.layer", json!({"name": "paint"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 80, "height": 60})).unwrap();
    s.execute("edit.fill", json!({"color": "#3070c0"})).unwrap();
    s.execute("layer.select", json!({"layer": paint})).unwrap();
    // Group Layers makes a collapsed group; open it so the panel shows the layer inside. (Opened
    // last: an open active group would take the new layers above inside it, as in Photoshop.)
    s.execute("layer.setExpanded", json!({"all": true, "expanded": true})).unwrap();
    s
}

fn services() -> crate::Services {
    // Saving is available in the app (an exporter is installed); nothing is written here.
    crate::Services { export: Some(Box::new(|_, _, _| Err("tests don't write files".into()))), ..Default::default() }
}

/// The whole app around `app`, run until it has settled. The runtime takes a frame's keys and the app runs the
/// commands they fired in the next frame, so a key needs two frames ([`press`], [`tap`]).
pub(crate) fn app_harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_context(ctx).with_max_steps(64).with_step_dt(1.0 / 60.0).build_eframe(move |_cc| app);
    h.run_steps(8);
    h
}

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    app_harness(PhotocraftApp::new(realistic(), services()))
}

/// The platform's modifiers for a shortcut's: `Cmd` is ⌘ on the Mac and Ctrl elsewhere, as
/// egui-winit reports them (`command` plus the physical key).
fn platform(m: Modifiers) -> Modifiers {
    let mut out = Modifiers { alt: m.alt, shift: m.shift, ctrl: m.ctrl, ..Default::default() };
    if m.command {
        out.command = true;
        if cfg!(target_os = "macos") {
            out.mac_cmd = true;
        } else {
            out.ctrl = true;
        }
    }
    out
}

/// A press and release of `key` with `mods` held, then the frames that take it and run its command.
pub(crate) fn tap(h: &mut Harness<'_, Box<PhotocraftApp>>, key: Key, mods: Modifiers) {
    let m = platform(mods);
    h.event(egui::Event::ModifiersChanged(m));
    h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m });
    h.event(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: m });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

/// A press and release queued in one input frame, then the frames that take it and run its command: two, not
/// [`tap`]'s three - the runtime takes the key in the first and the app runs the command it fired in the second
/// (`quick_shift_press` in `tool_shift_cycle_order_tests` measures the same for the tool letters). The third frame
/// only drew the app again (#15941).
fn press(h: &mut Harness<'_, Box<PhotocraftApp>>, sc: &str) {
    let sc = parse(sc).unwrap_or_else(|| panic!("unparsable shortcut {sc}"));
    let m = platform(sc.modifiers);
    h.input_mut().events.extend([
        egui::Event::ModifiersChanged(m),
        egui::Event::Key { key: sc.logical_key, physical_key: None, pressed: true, repeat: false, modifiers: m },
        egui::Event::Key { key: sc.logical_key, physical_key: None, pressed: false, repeat: false, modifiers: m },
        egui::Event::ModifiersChanged(Modifiers::NONE),
    ]);
    h.run_steps(2);
}

fn click(h: &mut Harness<'_, Box<PhotocraftApp>>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn active_name(h: &Harness<'_, Box<PhotocraftApp>>) -> String {
    let st = h.state().session.active().unwrap();
    st.active_layer.and_then(|id| st.doc.layer(id)).map(|l| l.name.clone()).unwrap_or_default()
}

/// Adds the action `name` with `steps` to the action list.
fn add_action(h: &mut Harness<'_, Box<PhotocraftApp>>, name: &str, steps: &[(&str, serde_json::Value)]) {
    let steps = steps.iter().map(|(command, params)| photocraft_engine::actions_cmds::ActionStep::new(*command, params.clone())).collect();
    h.state_mut().session.actions_mut().list.push(photocraft_engine::actions_cmds::Action::new(name, steps)).unwrap();
}

fn logged(h: &Harness<'_, Box<PhotocraftApp>>) -> Vec<String> {
    take_log(&h.ctx).into_iter().map(|(id, _)| id).collect()
}

fn names(h: &Harness<'_, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().session.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).collect()
}

/// Where focus is when the shortcut is pressed.
#[derive(Clone, Copy, Debug)]
enum Place {
    /// The pointer over the canvas, nothing focused.
    Canvas,
    /// Just after clicking the "paint" row in the Layers panel.
    LayersRow,
    /// Just after clicking into the Layers panel's Opacity field (a text field now).
    OpacityField,
    /// A non-text widget focused (a Layers panel row, through accessibility focus: Tab from
    /// nothing focused is Photoshop's Show/Hide All Panels, #1313).
    FocusedWidget,
}

fn put_focus(h: &mut Harness<'_, Box<PhotocraftApp>>, place: Place) {
    match place {
        Place::Canvas => {
            let c = h.state().last_canvas_rect.center();
            h.hover_at(c);
            h.run_steps(2);
        }
        Place::LayersRow => {
            h.get_by_role_and_label(Role::TreeItem, "paint").scroll_to_me();
            h.run_steps(4);
            let row = crate::layers_panel_nodes::row(h, "paint");
            click(h, row.center() + vec2(row.width() / 4.0, 0.0));
            assert_eq!(active_name(h), "paint");
        }
        Place::OpacityField => {
            // The Layers panel's Opacity field: the DragValue in the right dock reading 100.
            let layers = crate::panel_docking::probe::group_rect(h, "layers").expect("Layers drawn");
            let at = h
                .query_all_by_role(Role::SpinButton)
                .map(|n| n.rect())
                .filter(|r| layers.contains(r.center()))
                .min_by(|a, b| a.center().y.total_cmp(&b.center().y))
                .expect("Layers opacity field")
                .center();
            click(h, at);
            assert!(WindowHost::of(&h.ctx).typing(), "clicking a value field edits it");
        }
        Place::FocusedWidget => {
            h.get_by_role_and_label(Role::TreeItem, "paint").scroll_to_me();
            h.run_steps(4);
            h.get_by_role_and_label(Role::TreeItem, "paint").focus();
            h.run_steps(2);
            assert!(focus::focused_rec(&h.ctx).is_some() && !WindowHost::of(&h.ctx).typing(), "a widget is focused");
        }
    }
    take_log(&h.ctx);
}

/// Is `bound` the same action as the menu item `shown`? (Window › Layers is `window.panel.layers`
/// in the catalogue and `window.toggle.layers` in the shell; Release Clipping Mask shows Create
/// Clipping Mask's key, which releases while it reads so.)
fn same(shown: &str, bound: &str) -> bool {
    shown == bound
        || crate::menus::panel_alias(shown) == Some(bound)
        || crate::menus::panel_alias(bound) == Some(shown)
        || (shown == "layer.releaseClippingMask" && bound == "layer.createClippingMask")
}

/// Every shortcut shown in the menus (and the shell's unlisted ones), with whether it is live.
fn displayed(app: &PhotocraftApp) -> Vec<(String, String, bool)> {
    let mut v: Vec<(String, String, bool)> = crate::menus::menu_rows(app)
        .into_iter()
        .filter(|i| i.command.id != "---")
        .filter_map(|i| Some((i.command.id.clone(), i.shortcut.as_ref()?.portable(), crate::menus::is_live(&i.command.id))))
        .collect();
    for (id, _, menu, sc) in crate::menus::UI_COMMANDS {
        if let Some(sc) = sc
            && menu.is_empty()
        {
            v.push((id.to_string(), sc.to_string(), true));
        }
    }
    v
}

/// The way [`super::run_fired`] takes a fired command `id`: a row of [`super::RUNTIME_COMMANDS`] by its kind, a tool
/// letter, a Shift+letter group step, a recorded action, else an app command through [`super::dispatch_as`].
fn route(id: &str) -> String {
    if let Some((_, bridged)) = super::RUNTIME_COMMANDS.iter().find(|(row, _)| *row == id) {
        let kind = format!("{bridged:?}");
        return kind.split('(').next().unwrap_or(&kind).to_string();
    }
    if crate::tool_shortcuts::tool(id).is_some() {
        "tool".into()
    } else if super::next_in_group_letter(id).is_some() {
        "group".into()
    } else if id.starts_with(crate::actions::SHORTCUT_PREFIX) {
        "action".into()
    } else {
        "app".into()
    }
}

/// What a press of `chord` shares with every other chord of its group: the keyboard path (the runtime's key stage,
/// a focused field's typing rule, a focused component's held keys) depends on the kind of key and the modifiers,
/// not on the command. Letters, digits and function keys are one kind each; every other key is its own kind.
fn key_kind(chord: &Chord) -> String {
    let key = chord.key;
    let kind = if key.is_letter() {
        "letter".to_string()
    } else if key.is_digit() {
        "digit".to_string()
    } else if key.is_function_key() {
        "function".to_string()
    } else {
        format!("{key:?}")
    };
    let m = chord.mods;
    format!("{kind} cmd={} ctrl={} alt={} shift={}", m.cmd, m.ctrl, m.alt, m.shift)
}

/// Every live displayed shortcut (id, text) and its chord.
fn live_chords(app: &PhotocraftApp) -> Vec<(String, String, Chord)> {
    displayed(app)
        .into_iter()
        .filter(|(_, _, live)| *live)
        .map(|(id, sc, _)| {
            let chord = Chord::parse(&sc).unwrap_or_else(|_| panic!("{id}: unparsable {sc}"));
            (id, sc, chord)
        })
        .collect()
}

/// One displayed shortcut per (route, kind of key, modifiers): the shortcuts whose press goes the same way.
fn representatives(app: &PhotocraftApp) -> Vec<(String, String, Chord)> {
    let mut seen = HashSet::new();
    live_chords(app).into_iter().filter(|(id, _, chord)| seen.insert((route(id), key_kind(chord)))).collect()
}

/// The command the runtime fires for `chord`, shown for menu row `id`: the row's own command when the keymap shows
/// the chord for it (no other command wins it where the binding applies), else the key's holder.
fn fired_by(ctx: &egui::Context, id: &str, chord: &Chord) -> Option<craft::ui::runtime::commands::CommandRef> {
    let own = super::keymap_command(id);
    if craft::ui::runtime::commands::chord(ctx, &own) == Some(*chord) {
        return Some(own);
    }
    // `holder` answers with no panel active; it falls back to a panel's binding when no unscoped command holds the key.
    craft::ui::runtime::commands::holder(ctx, chord)
}

/// Every displayed shortcut runs its menu item's command, or the menu shows the item disabled and the shortcut
/// says why: the keymap's command for each chord is fired into [`super::run_fired`] as a key press, with no key
/// events and no frames between them. The keyboard's way to the keymap is the same for every chord of a kind, and
/// [`every_kind_of_shortcut_dispatches_from_where_focus_usually_is`] presses one of each. The view, screen-mode,
/// momentary, Swatches and Workspace rows run in the runtime, the canvas or the shell instead of `run_fired`; that
/// test presses one of each kind of key for them. Every tool letter picks a tool of that letter.
#[test]
fn every_displayed_shortcut_runs_its_menu_item() {
    use craft::ui::runtime::commands::{FirePhase, FireSource, Fired};
    let mut h = harness();
    set_dry_run(&h.ctx, true);
    put_focus(&mut h, Place::Canvas);
    let ctx = h.ctx.clone();
    let items = crate::menus::menu_rows(h.state());
    let chords = live_chords(h.state());
    assert!(chords.len() > 100, "the menus show their shortcuts: {}", chords.len());
    let fire = |h: &mut Harness<'_, Box<PhotocraftApp>>, command| {
        super::run_fired(h.state_mut(), &ctx, &[Fired::new(command, FireSource::Key, FirePhase::Press, 0.0)]);
        take_log(&ctx)
    };
    let mut failures = Vec::new();
    let mut fired = 0;
    for (id, sc, chord) in &chords {
        let Some(command) = fired_by(&ctx, id, chord) else {
            failures.push(format!("{sc} ({id}) runs nothing"));
            continue;
        };
        match route(id).as_str() {
            "View" | "Momentary" | "ScreenMode" | "Swatches" | "Workspace" => {
                if command != super::keymap_command(id) {
                    failures.push(format!("{sc} ({id}) fires {command:?}"));
                }
                continue;
            }
            "tool" | "group" => {
                let want = crate::tool_shortcuts::tool(id).map(Tool::key).or_else(|| super::next_in_group_letter(id));
                fire(&mut h, command);
                if Some(h.state().ui.tool.key()) != want {
                    failures.push(format!("{sc} ({id}) left {:?}", h.state().ui.tool));
                }
                continue;
            }
            _ => {}
        }
        let enabled = items.iter().find(|i| i.command.id == *id).map_or_else(|| crate::menus::is_enabled(h.state(), id), |i| i.enabled);
        fired += 1;
        match fire(&mut h, command).as_slice() {
            [(bound, Outcome::Ran)] if same(id, bound) && enabled => {}
            [(bound, Outcome::Disabled(why))] if same(id, bound) && !enabled && !why.is_empty() => {}
            other => failures.push(format!("{sc} ({id}, menu enabled: {enabled}) -> {other:?}")),
        }
    }
    assert!(fired > 80, "most displayed shortcuts go through run_fired: {fired}");
    for t in Tool::ALL.into_iter().filter(|t| t.key() != '\0') {
        let chord = Chord::parse(&t.key().to_string()).unwrap_or_else(|_| panic!("tool key {}", t.key()));
        let Some(command) = craft::ui::runtime::commands::holder(&ctx, &chord) else {
            failures.push(format!("tool key {} runs nothing", t.key()));
            continue;
        };
        h.state_mut().ui.tool = if t.key() == 'H' { Tool::Move } else { Tool::Hand };
        fire(&mut h, command);
        if h.state().ui.tool.key() != t.key() {
            failures.push(format!("tool key {} left {:?}", t.key(), h.state().ui.tool));
        }
    }
    assert!(failures.is_empty(), "{} shortcut failures:\n{}", failures.len(), failures.join("\n"));
}

/// Press one displayed shortcut of each (route, kind of key, modifiers) from each place focus usually is: each
/// dispatches its command, or the menu shows it disabled and the press reports why; a focused field keeps its typing
/// keys and a focused component the keys it holds. The other shortcuts of a group take the same path to a different
/// row of the table [`every_displayed_shortcut_runs_its_menu_item`] checks. (Dry run: File › Exit, Open, Help
/// links… resolve and check enablement without running.)
#[test]
fn every_kind_of_shortcut_dispatches_from_where_focus_usually_is() {
    let mut failures = Vec::new();
    for place in [Place::Canvas, Place::LayersRow, Place::FocusedWidget, Place::OpacityField] {
        // One app per place: the presses below include the panel toggles the runtime runs itself, so the next
        // place could not count on the Layers panel still being there.
        let mut h = harness();
        set_dry_run(&h.ctx, true);
        put_focus(&mut h, place);
        let typing = WindowHost::of(&h.ctx).typing();
        let items = crate::menus::menu_rows(h.state());
        for (id, sc, chord) in representatives(h.state()) {
            // A press the focused component holds (the Layers rows' stacking and visibility keys)
            // is the component's and reaches no app command.
            let held = focus_holds(&h.ctx, chord);
            press(&mut h, &sc);
            let log = take_log(&h.ctx);
            if typing && !chord.has_command() && !chord.is_function_key() {
                // A focused text field keeps its typing / editing keys: nothing may fire.
                if !log.is_empty() {
                    failures.push(format!("{place:?}: {sc} ({id}) fired {log:?} into a focused field"));
                }
                continue;
            }
            let enabled = items.iter().find(|i| i.command.id == id).map_or_else(|| crate::menus::is_enabled(h.state(), &id), |i| i.enabled);
            match log.as_slice() {
                [] if held || typing => {}
                [(bound, Outcome::Ran)] if same(&id, bound) && enabled => {}
                [(bound, Outcome::Disabled(why))] if same(&id, bound) && !enabled && !why.is_empty() => {}
                other => failures.push(format!("{place:?}: {sc} ({id}, menu enabled: {enabled}) -> {other:?}")),
            }
        }
        // Tool letters from the tools panel tooltips ("Brush Tool  (B)"): a letter of one tool and a letter its group
        // shares take different picks (`pick_tool`); the table checks every letter's tool. In a focused text field the
        // letter is typed and the tool stays.
        let shared = |t: &Tool| Tool::ALL.iter().filter(|o| o.key() == t.key()).count() > 1;
        let one = Tool::ALL.into_iter().find(|t| t.key() != '\0' && !shared(t));
        let many = Tool::ALL.into_iter().find(|t| t.key() != '\0' && shared(t));
        for t in one.into_iter().chain(many) {
            let start = if t.key() == 'H' { Tool::Move } else { Tool::Hand };
            h.state_mut().ui.tool = start;
            press(&mut h, &t.key().to_string());
            let want = if typing { start } else { t };
            if h.state().ui.tool.key() != want.key() {
                failures.push(format!("{place:?}: tool key {} left {:?}, expected a {} tool", t.key(), h.state().ui.tool, want.key()));
            }
        }
    }
    assert!(failures.is_empty(), "{} shortcut failures:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn a_disabled_shortcut_says_why() {
    let mut h = harness();
    put_focus(&mut h, Place::Canvas);
    // Merge Down on the bottom layer: Photoshop greys it; the key press reports the reason.
    let bg = h.state().session.active().unwrap().doc.layers[0].id;
    h.state_mut().session.select_layer(bg).unwrap();
    h.run_steps(2);
    press(&mut h, "Cmd+E");
    let log = take_log(&h.ctx);
    assert!(matches!(log.as_slice(), [(id, Outcome::Disabled(_) | Outcome::Failed(_))] if id == "layer.mergeLayers"), "{log:?}");
    assert!(!h.state().ui.status.is_empty() && h.state().ui.status_error, "the status bar says why");
}

/// The catalog's chords reach PhotoCraft's commands through `run_fired`, whichever of a command's keys is pressed.
/// A dry run logs the command a key fired; the second part runs the keys whose effect is the point.
#[test]
fn keys_run_their_commands() {
    let mut h = harness();
    put_focus(&mut h, Place::Canvas);
    set_dry_run(&h.ctx, true);
    let mut rows: Vec<(&str, &str)> = vec![
        // #3665: Photoshop's second keys reach their command beside its main key.
        ("F2", "edit.cut"),
        ("F3", "edit.copy"),
        ("Cmd+C", "edit.copy"),
        ("F4", "edit.paste"),
        ("Shift+F7", "select.inverse"),
        ("Cmd+Shift+I", "select.inverse"),
        ("Shift+Backspace", "edit.fill"),
        // ⌘⇧↩ adds the path to the selection, ⌘⌥↩ subtracts it.
        ("Cmd+Shift+Enter", "path.addToSelection"),
        ("Cmd+Alt+Enter", "path.subtractFromSelection"),
        ("Cmd+Alt+G", "layer.createClippingMask"),
    ];
    // Ctrl+F4 closes the document beside Ctrl+W, as in Photoshop on Windows.
    if !cfg!(target_os = "macos") {
        rows.push(("Cmd+F4", "file.close"));
    }
    // egui-winit sends Cut for Shift+Delete on Windows; it opens Fill (#530).
    if cfg!(target_os = "windows") {
        rows.push(("Shift+Delete", "edit.fill"));
    }
    for (key, want) in rows {
        press(&mut h, key);
        let log = take_log(&h.ctx);
        assert!(matches!(log.as_slice(), [(id, _)] if id == want), "{key}: {log:?}");
    }
    // Overriding Copy's main key keeps F3; ⌘C no longer copies.
    h.state_mut().session.edit_prefs(|p| {
        p.shortcuts.insert("edit.copy".into(), "Cmd+K".into());
    });
    h.run_steps(2);
    press(&mut h, "F3");
    assert_eq!(logged(&h), ["edit.copy"]);
    press(&mut h, "Cmd+C");
    assert!(logged(&h).is_empty(), "the old main key is free");
    // A removed alternate no longer fires.
    h.state_mut().session.edit_prefs(|p| {
        p.alternate_shortcuts.insert("edit.paste".into(), Vec::new());
    });
    h.run_steps(2);
    press(&mut h, "F4");
    assert!(logged(&h).is_empty());
    set_dry_run(&h.ctx, false);

    // #352: ⌥[ ⌥] ⌥, ⌥. and ⇧⌥[ walk the Layers panel's rows. Rows, top to bottom: paint, Hello (type), group,
    // in-group, Background. The same app: the dry-run presses above ran no command, so the document is still the
    // harness's own (#15941).
    assert_eq!(active_name(&h), "paint");
    for (key, want) in [("Alt+[", "Hello"), ("Alt+[", "group"), ("Alt+[", "in-group"), ("Alt+]", "group"), ("Alt+,", "Background"), ("Alt+.", "paint")] {
        press(&mut h, key);
        assert_eq!(active_name(&h), want, "after {key}");
    }
    press(&mut h, "Alt+Shift+[");
    assert_eq!(active_name(&h), "Hello");
    assert_eq!(h.state().session.active().unwrap().selected_layers().len(), 2, "⇧⌥[ adds to the selection");

    // ⌥⌘G clips the selected layer and pressing it again releases it, as Photoshop's one Create / Release Clipping
    // Mask item does (`clip_key_releases`).
    let mut h = harness();
    put_focus(&mut h, Place::Canvas);
    let paint = h.state().session.active().unwrap().active_layer.unwrap();
    let clipped = |h: &Harness<'_, Box<PhotocraftApp>>| h.state().session.active().unwrap().doc.layer(paint).unwrap().clipped;
    press(&mut h, "Cmd+Alt+G");
    assert_eq!(logged(&h), ["layer.createClippingMask"]);
    assert!(clipped(&h));
    press(&mut h, "Cmd+Alt+G");
    assert_eq!(logged(&h), ["layer.releaseClippingMask"]);
    assert!(!clipped(&h), "the second press releases it");
    press(&mut h, "Cmd+Z");
    assert!(clipped(&h), "one undo puts the clip back");
    assert_eq!(logged(&h), ["edit.undo"]);
    // With several layers selected the key clips every one above the lowest, and releases them once they all are.
    let s = &mut h.state_mut().session;
    let base = s.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.name == "Hello").unwrap().2.id;
    let top = photocraft_doc::LayerId(s.execute("layer.new.layer", json!({"name": "top"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.select", json!({"layer": base.0, "mode": "replace"})).unwrap();
    for id in [paint, top] {
        s.execute("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
    }
    h.run_steps(2);
    let is_clipped = |h: &Harness<'_, Box<PhotocraftApp>>, id: photocraft_doc::LayerId| h.state().session.active().unwrap().doc.layer(id).unwrap().clipped;
    press(&mut h, "Cmd+Alt+G");
    assert_eq!(logged(&h), ["layer.createClippingMask"], "the top layer isn't clipped yet");
    assert!(is_clipped(&h, paint) && is_clipped(&h, top) && !is_clipped(&h, base));
    press(&mut h, "Cmd+Alt+G");
    assert_eq!(logged(&h), ["layer.releaseClippingMask"]);
    assert!(!is_clipped(&h, paint) && !is_clipped(&h, top) && !is_clipped(&h, base));
}

/// Delete (and Backspace) with nothing selected deletes the selected layers whatever their kind (#1077); with the
/// layer mask or a smart object's filter mask targeted they edit the mask (FILE-215-3), unless several layers are
/// selected (#4025); a selection keeps Edit › Clear.
#[test]
fn delete_key_targets() {
    // The selected layer, from the canvas or the Layers panel, in one undo step.
    for place in [Place::Canvas, Place::LayersRow] {
        // One app per place: each key's delete is undone by the end of its case, so the next key starts from the
        // same document without paying for another whole app (#15941).
        let mut h = harness();
        put_focus(&mut h, place);
        for key in ["Delete", "Backspace"] {
            let s = &mut h.state_mut().session;
            // The first case of the app deselects the harness's own selection; the ones after it start without one.
            s.execute("select.deselect", json!({})).ok();
            let adj = photocraft_doc::LayerId(s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap());
            h.run_steps(2);
            let before = names(&h);
            press(&mut h, key);
            assert_eq!(logged(&h), ["layer.delete"], "{place:?} {key}");
            assert!(h.state().session.active().unwrap().doc.layer(adj).is_none(), "{place:?} {key}: the adjustment layer is gone");
            assert_eq!(names(&h).len(), before.len() - 1, "{place:?} {key}: only it");
            press(&mut h, "Cmd+Z");
            assert_eq!(names(&h), before, "{place:?} {key}: one undo brings it back");
            // The undo's own log entry goes here: the next key of this app starts with an empty log, as the
            // app-per-key version did.
            take_log(&h.ctx);
        }
    }

    // The layer mask targeted: a selection fills the mask with the background colour, no selection deletes the mask.
    // One app for both keys: the first case leaves the layer without a mask and without a selection, and the case
    // puts both back before it starts (#15941).
    let mut h = harness();
    put_focus(&mut h, Place::Canvas);
    for key in ["Delete", "Backspace"] {
        let s = &mut h.state_mut().session;
        let layer = s.active().unwrap().active_layer.unwrap();
        s.execute("select.rect", json!({"x": 20, "y": 20, "width": 80, "height": 60})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        let sel = s.active().unwrap().doc.selection.as_ref().expect("the harness keeps a selection").content_bounds();
        let (x, y) = ((sel.x0 + sel.x1) / 2, (sel.y0 + sel.y1) / 2);
        h.state_mut().ui.mask_target = true;
        h.run_steps(2);
        let layer_pixels = |h: &Harness<'_, Box<PhotocraftApp>>| {
            h.state().session.active().unwrap().doc.layer(layer).and_then(|l| l.surface().cloned()).map(|s| s.read_region(sel))
        };
        let mask_at = |h: &Harness<'_, Box<PhotocraftApp>>| {
            h.state().session.active().unwrap().doc.layer(layer).and_then(|l| l.mask.as_ref()).map(|m| m.surface.sample_channel(x, y, 0))
        };
        let before = layer_pixels(&h);
        let bg = h.state().session.tools.background;
        press(&mut h, key);
        assert_eq!(logged(&h), ["edit.fillBackground"], "{key}");
        assert_eq!(mask_at(&h), Some(bg[0]), "{key}: the mask takes the background colour in the selection");
        assert_eq!(layer_pixels(&h), before, "{key}: the layer's pixels are untouched");
        h.state_mut().session.execute("select.deselect", json!({})).unwrap();
        h.run_steps(2);
        press(&mut h, key);
        assert_eq!(logged(&h), ["layer.layerMask.delete"], "{key}");
        assert_eq!(mask_at(&h), None, "{key}: no selection deletes the mask");
        assert!(h.state().session.active().unwrap().doc.layer(layer).is_some(), "{key}: the layer stays");
        assert_eq!(layer_pixels(&h), before, "{key}: the layer's pixels are untouched");
    }
}

/// With a smart object's filter mask targeted, Delete fills it from a selection; with no selection the layer goes.
#[test]
fn delete_key_targets_a_smart_objects_filter_mask() {
    let mut h = harness();
    put_focus(&mut h, Place::Canvas);
    let s = &mut h.state_mut().session;
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    let sel = s.active().unwrap().doc.selection.as_ref().expect("the harness keeps a selection").content_bounds();
    let (x, y) = ((sel.x0 + sel.x1) / 2, (sel.y0 + sel.y1) / 2);
    h.state_mut().select_filter_mask_target(Some(layer.0));
    assert!(h.state().filter_mask_targeted());
    h.run_steps(2);
    let smart = |h: &Harness<'_, Box<PhotocraftApp>>| match h.state().session.active().unwrap().doc.layer(layer).map(|l| &l.content) {
        Some(photocraft_doc::LayerContent::Smart(sm)) => Some(sm.clone()),
        _ => None,
    };
    let filters = smart(&h).unwrap().smart_filters.len();
    let bg = h.state().session.tools.background;
    press(&mut h, "Delete");
    assert_eq!(logged(&h), ["edit.fillBackground"]);
    let sm = smart(&h).expect("the smart object stays");
    assert_eq!(sm.filter_mask.as_ref().map(|m| m.value(x, y)), Some(bg[0]), "the filter mask takes the background colour");
    assert_eq!(sm.smart_filters.len(), filters);
    h.state_mut().session.execute("select.deselect", json!({})).unwrap();
    h.run_steps(2);
    press(&mut h, "Delete");
    assert_eq!(logged(&h), ["layer.delete"]);
    assert!(smart(&h).is_none(), "no selection deletes the layer, as in Photoshop");
}

/// Delete with several layers selected removes every one of them, or edits the targeted mask when a selection exists.
#[test]
fn delete_key_with_several_layers_selected() {
    // Every selected layer goes, pixel layers too.
    let mut h = harness();
    put_focus(&mut h, Place::Canvas);
    let s = &mut h.state_mut().session;
    s.execute("select.deselect", json!({})).unwrap();
    let paint = s.active().unwrap().active_layer.unwrap();
    let adj = photocraft_doc::LayerId(s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    s.execute("layer.select", json!({"layer": adj.0, "mode": "add"})).unwrap();
    h.run_steps(2);
    let n = names(&h).len();
    press(&mut h, "Delete");
    let doc = &h.state().session.active().unwrap().doc;
    assert!(doc.layer(paint).is_none() && doc.layer(adj).is_none(), "both selected layers are gone");
    assert_eq!(names(&h).len(), n - 2);

    // Several layers selected with the mask of the active one targeted: a selection edits the mask, none deletes the layers.
    let mut h = harness();
    put_focus(&mut h, Place::Canvas);
    let s = &mut h.state_mut().session;
    let first = photocraft_doc::LayerId(s.execute("layer.newAdjustmentLayer.curves", json!({})).unwrap()["layer"].as_u64().unwrap());
    let second = photocraft_doc::LayerId(s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.select", json!({"layer": first.0, "mode": "add"})).unwrap();
    // The new layers took the harness's selection as their masks (#8801): select again.
    s.execute("select.all", json!({})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.selected_layers().len(), 2);
    assert!(st.active_layer.and_then(|l| st.doc.layer(l)).is_some_and(|l| l.mask.is_some()), "the active adjustment layer has a mask");
    h.state_mut().ui.mask_target = true;
    h.run_steps(2);
    let n = names(&h).len();
    press(&mut h, "Delete");
    assert_eq!(logged(&h), ["edit.fillBackground"], "a selection edits the mask");
    assert_eq!(names(&h).len(), n);
    h.state_mut().session.execute("select.deselect", json!({})).unwrap();
    h.run_steps(2);
    press(&mut h, "Delete");
    assert_eq!(logged(&h), ["layer.delete"]);
    let doc = &h.state().session.active().unwrap().doc;
    assert!(doc.layer(first).is_none() && doc.layer(second).is_none(), "both selected layers are gone");
    assert_eq!(names(&h).len(), n - 2);

    // A selection keeps Edit › Clear: the selected pixels go, the layer stays.
    let mut h = harness();
    put_focus(&mut h, Place::Canvas);
    let before = names(&h);
    assert!(h.state().session.active().unwrap().doc.selection.is_some());
    press(&mut h, "Delete");
    assert_eq!(logged(&h), ["edit.clear"]);
    assert_eq!(names(&h), before, "no layer was deleted");
}

/// An action key the automation policy refuses leaves the bindings as they were.
#[test]
fn denied_shortcut_assignment_leaves_bindings_unmodified() {
    let mut h = harness();
    add_action(&mut h, "Action", &[]);
    h.state_mut().services.automation_command = Some(Box::new(|_, _| Err("denied shortcut edit".into())));
    h.state_mut().automation_input = true;
    let prefs = h.state().session.prefs().shortcuts.clone();
    assert!(crate::actions::assign_shortcut(h.state_mut(), "Action", "F2").is_err());
    assert_eq!(h.state().session.prefs().shortcuts, prefs);
}

/// A saved key whose action was deleted or renamed says so instead of playing nothing.
#[test]
fn a_key_for_a_missing_action_reports_it() {
    let mut h = harness();
    let (id, ctx) = (crate::actions::shortcut_id("Gone"), h.ctx.clone());
    super::dispatch(h.state_mut(), &ctx, &id);
    assert!(h.state().ui.status_error);
    assert!(matches!(&take_log(&h.ctx)[..], [(logged, Outcome::Disabled(why))] if *logged == id && why == "The Action no longer exists"));
}

#[test]
fn named_action_function_key_replays_and_survives_preferences_roundtrip() {
    let mut h = harness();
    add_action(&mut h, "LivePrint 2R", &[("layer.new.layer", json!({"name": "F6 overlay"}))]);
    crate::actions::assign_shortcut(h.state_mut(), "LivePrint 2R", "F6").unwrap();
    let saved = serde_json::to_value(h.state().session.prefs()).unwrap();
    let restored = serde_json::from_value(saved).unwrap();
    h.state_mut().session.edit_prefs(|p| *p = restored);
    // The action is a run-time catalog row, so the runtime binds its key.
    h.run_steps(2);
    press(&mut h, "F6");
    assert_eq!(active_name(&h), "F6 overlay");
    assert_eq!(h.state().ui.status, "Played 1 step");
    assert!(!h.state().ui.status_error);
    assert_eq!(take_log(&h.ctx), vec![(crate::actions::shortcut_id("LivePrint 2R"), Outcome::Ran)]);
    h.state_mut().run("actions.delete", json!({"action": "LivePrint 2R"})).unwrap();
    h.run_steps(2);
    assert!(command_rows(h.state()).iter().all(|row| !row.command.id.starts_with(crate::actions::SHORTCUT_PREFIX)));
    // F6 goes back to the command it belongs to by default.
    press(&mut h, "F6");
    let log = take_log(&h.ctx);
    assert!(matches!(log.as_slice(), [(id, _)] if !id.starts_with(crate::actions::SHORTCUT_PREFIX)), "{log:?}");
}

#[test]
fn assigning_action_function_key_moves_it_and_reports_failed_steps() {
    let mut h = harness();
    for name in ["first", "second"] {
        add_action(&mut h, name, &[("no.such.command", json!({}))]);
    }
    crate::actions::assign_shortcut(h.state_mut(), "first", "F6").unwrap();
    crate::actions::assign_shortcut(h.state_mut(), "second", "F6").unwrap();
    assert_eq!(h.state().session.prefs().shortcuts[&crate::actions::shortcut_id("first")], "");
    h.run_steps(2);
    press(&mut h, "F6");
    assert!(h.state().ui.status_error);
    assert!(h.state().ui.status.contains("Step 1 (no.such.command) failed"));
    assert!(matches!(&take_log(&h.ctx)[..], [(id, Outcome::Failed(_))] if id == "actions.play:second"));
}

#[test]
fn action_function_key_checks_automation_authorization() {
    let mut h = harness();
    add_action(&mut h, "restricted", &[("layer.new.layer", json!({"name": "should not exist"}))]);
    crate::actions::assign_shortcut(h.state_mut(), "restricted", "F6").unwrap();
    h.state_mut().services.automation_command = Some(Box::new(|id, _| if id == "actions.play" { Err("denied action playback".into()) } else { Ok(()) }));
    h.state_mut().automation_input = true;
    super::dispatch(h.state_mut(), &egui::Context::default(), "actions.play:restricted");
    assert_eq!(h.state().ui.status, "denied action playback");
    assert_eq!(active_name(&h), "paint");
}

#[test]
fn fit_shortcut_records_and_f6_fits_the_resized_canvas() {
    let mut h = harness();
    h.state_mut().run("actions.record", json!({"name": "Resize and fit"})).unwrap();
    h.state_mut().run("image.canvasSize", json!({"width": 1800, "height": 1080})).unwrap();
    press(&mut h, "Cmd+0");
    h.state_mut().run("actions.stop", json!({})).unwrap();
    assert_eq!(
        h.state().session.actions().list.actions()[0].steps.iter().map(|s| s.command.as_str()).collect::<Vec<_>>(),
        ["image.canvasSize", "view.fitOnScreen"]
    );
    let item = crate::menus::menu_rows(h.state()).into_iter().find(|i| i.command.id == "view.fitOnScreen").unwrap();
    assert_eq!(item.shortcut.as_ref().map(|c| c.portable()).as_deref(), Some("Cmd+0"));
    crate::actions::assign_shortcut(h.state_mut(), "Resize and fit", "F6").unwrap();
    h.state_mut().ui.views[0].zoom = 0.1;
    h.state_mut().ui.views[0].center = [0.0, 0.0];
    h.run_steps(2);
    press(&mut h, "F6");
    let view = &h.state().ui.views[0];
    assert!(!view.asks(craft::ui::canvas::ViewCommands::FitOnScreen), "the real canvas frame consumed Fit on Screen");
    assert!(view.zoom > 0.1 && view.zoom < 1.0, "{}", view.zoom);
    assert_eq!(view.center, [900.0, 540.0]);
    assert_eq!(h.state().ui.status, "Played 2 steps");
}

#[test]
fn f6_while_recording_records_a_named_call_not_the_child_steps() {
    let mut h = harness();
    add_action(&mut h, "LivePrint 2R", &[("layer.new.layer", json!({"name": "Nested overlay"})), ("view.fitOnScreen", json!({}))]);
    crate::actions::assign_shortcut(h.state_mut(), "LivePrint 2R", "F6").unwrap();
    h.state_mut().run("actions.record", json!({"name":"Action 2"})).unwrap();
    h.run_steps(2);
    press(&mut h, "F6");
    assert_eq!(active_name(&h), "Nested overlay");
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let recorded: Vec<_> = h.state().session.actions().list.actions()[1].steps.iter().map(|s| (s.command.clone(), s.params.clone())).collect();
    assert_eq!(recorded, [("actions.play".to_string(), json!({"action": "LivePrint 2R"}))]);
}

/// The type size keys act on the selected text while typing, on the whole layer from the canvas. (Which key they are
/// is the keymap's layout matching, not tested here.)
#[test]
fn type_size_keys_follow_the_typed_selection() {
    let mut h = harness();
    let st = h.state().session.active().unwrap();
    let id = st.doc.walk().into_iter().find(|(_, _, l)| matches!(l.content, LayerContent::Text(_))).unwrap().2.id.0;
    h.state_mut().session.execute("layer.select", json!({"layer": id})).unwrap();
    let sizes = |h: &Harness<'_, Box<PhotocraftApp>>| match &h.state().session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
        LayerContent::Text(t) => t.char_runs().iter().map(|r| (r.len, r.style.size_pt)).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let base = sizes(&h)[0].1;
    press(&mut h, "Cmd+Shift+.");
    assert_eq!(logged(&h), ["type.increaseSize"]);
    assert_eq!(sizes(&h), [(5, base + 2.0)], "the whole layer grows from the canvas");

    let ctx = h.ctx.clone();
    crate::type_tool::edit_active(h.state_mut(), &ctx).unwrap();
    if let Some(ed) = h.state_mut().ui.text_edit.as_mut() {
        (ed.anchor, ed.caret) = (0, 2);
    }
    h.run_steps(2);
    press(&mut h, "Cmd+Shift+,");
    assert_eq!(logged(&h), ["type.decreaseSize"]);
    assert_eq!(sizes(&h), [(2, base), (3, base + 2.0)], "only the selected \"He\" shrinks");
}

fn zoom(h: &Harness<'static, Box<PhotocraftApp>>) -> f32 {
    h.state().current_zoom()
}

#[test]
fn ctrl_plus_zooms_the_canvas_not_the_interface() {
    let mut h = harness();
    let z0 = zoom(&h);
    // ⌘+ / Ctrl+'+' as typed with ⇧ on a US layout, and as the numpad / Nordic `+`.
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Plus);
    h.run_steps(2);
    let z1 = zoom(&h);
    assert!(z1 > z0, "⌘+ should zoom the canvas in ({z0} -> {z1})");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Plus);
    h.run_steps(2);
    assert!(zoom(&h) > z1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Minus);
    h.run_steps(2);
    assert!((zoom(&h) - z1).abs() < 1e-4);
    // ⌘1 is 100%, ⌘0 fits on screen again.
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num1);
    h.run_steps(2);
    assert_eq!(zoom(&h), 1.0);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num0);
    h.run_steps(3);
    assert!((zoom(&h) - z0).abs() < 1e-3, "⌘0 fits on screen: {} vs {z0}", zoom(&h));
    assert_eq!(h.ctx.zoom_factor(), 1.0, "the interface must never scale");
}

#[test]
fn keyboard_zoom_works_with_a_dialog_open() {
    let mut h = harness();
    let dialog = crate::dialogs::open_command_dialog(h.state_mut(), "image.adjustments.brightnessContrast", "Brightness/Contrast…").unwrap();
    h.run_steps(3);
    assert_eq!(h.state().dialog_count(), 1);
    let z0 = zoom(&h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Equals);
    h.run_steps(2);
    assert!(zoom(&h) > z0);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Plus);
    h.run_steps(2);
    assert_eq!(h.ctx.zoom_factor(), 1.0);
    assert_eq!(h.state().dialog_count(), 1);
    // Other commands stay blocked while the dialog is open (⌘J would duplicate the layer).
    let layers = h.state().session.active().unwrap().doc.layers.len();
    h.key_press_modifiers(Modifiers::COMMAND, Key::J);
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers);
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.cancel_by_id(&ctx, dialog);
}

/// A frame can run two passes. The fired commands are taken by the first pass that runs them, so a second pass in the
/// same frame runs only what its own input fired: ⌘J duplicates the layer once.
#[test]
fn a_second_pass_in_one_frame_runs_no_command_twice() {
    let mut app = PhotocraftApp::new(realistic(), services());
    let ctx = app.ctx().clone();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let count = |app: &PhotocraftApp| app.session.active().unwrap().doc.walk().len();
    let n = count(&app);
    let press = egui::RawInput {
        events: vec![egui::Event::Key { key: Key::J, physical_key: Some(Key::J), pressed: true, repeat: false, modifiers: Modifiers::COMMAND }],
        ..Default::default()
    };
    crate::app::framework::keys_pass(&mut app, &ctx, press, |_| {});
    crate::app::framework::keys_pass(&mut app, &ctx, egui::RawInput::default(), |_| {});
    assert_eq!(count(&app), n + 1, "one press, one duplicate");
}

/// `bridge_of` answers from the table built once (#15493) exactly as a scan that builds every row's command does:
/// for every row's framework command, and none for a command no row bridges.
#[test]
fn bridge_of_finds_the_row_a_fresh_scan_finds() {
    use super::{CommandRef, RUNTIME_COMMANDS, bridge_of};
    let scan = |command: &CommandRef| RUNTIME_COMMANDS.iter().find(|(_, bridged)| bridged.command() == *command).map(|(id, bridged)| (*id, *bridged));
    for (_, bridged) in RUNTIME_COMMANDS {
        let command = bridged.command();
        assert_eq!(bridge_of(&command), scan(&command), "{command:?}");
        assert!(bridge_of(&command).is_some(), "{command:?} is bridged");
    }
    let unknown = CommandRef::new("photocraft.noSuchCommand");
    assert_eq!(bridge_of(&unknown), None);
}
