# photocraft-appmenu

Publishes an application menu bar over the **Canonical AppMenu protocol** (the
`com.canonical.dbusmenu` D-Bus interface, exported at `/MenuBar`) and registers the app's X11
top-level windows with the menu registrar, so global-menu environments (GNOME with the
AppIndicator/app-menu shell extension, Unity-like shells, Gershwin Menu) can host the menus
natively. Menu clicks come back as events carrying a plain `action` token.

The crate is **standalone** (L0): it depends on no workspace crate and is reusable by every
sibling *craft app. The D-Bus and X11 code is Linux/BSD only; on every other target (macOS,
wasm32) the same API is served by a no-op stub whose `start` fails gracefully — the in-window
menu bar is the only menu there, and callers compile unchanged.

## Backends: the portable boundary

The public surface is deliberately **backend-neutral**, so the same shell code runs
everywhere and a new platform backend slots in without touching the model or any caller:

- *Portable, compiled on every target:* `MenuModel` / `MenuEntry` / `MenuCommand`,
  `flat::FlatItem` (+ `MenuModel::from_flat`), `Shortcut`/`ShortcutMod`, `MenuEvent`
  (an activation carries the app's `action` token and the backend's item id), and the
  `AppMenu` handle contract (`start` / `replace` / `try_event` / `hosted` / `shutdown`).
- *Backend-specific, compiled only where it applies:* the Linux backend's `dbus.rs`
  (`com.canonical.dbusmenu` at `/MenuBar`), `x11.rs` (window discovery) and
  `registrar.rs` — none of them reachable from portable code.
- *Today's backends:* **Linux/BSD** — AppMenu/dbusmenu (this crate's live side);
  **everywhere else** — the no-op stub (the in-window menu bar owns the menus). A
  **macOS** backend over AppKit/`muda` is a planned follow-up: the sibling apps' existing
  `native_menu.rs` adapters (LightCraft's is the most complete — live labels, checked
  items, dynamic rebuilds, accelerators that yield to focused text fields) are the
  behavioural reference, and `Shortcut` ("Cmd" naming) and `MenuEntry` were shaped to
  map onto muda items and accelerators.

`AppMenu::hosted` is part of the portable contract on purpose: its *question* ("should
this app hide its in-window menu bar right now?") is portable — its *answer* is
backend-defined (Linux: a D-Bus menu registrar holds one of our windows; macOS: always
true, the platform bar owns the menus; stub: always false).

## How it works

1. **Model** (`model.rs`): toolkit-independent menu data — `MenuModel::top(children)`,
   `MenuEntry::command(label, action, enabled)` with builder methods (`.submenu`, `.checked`,
   `.shortcut("Ctrl+S")`), and `MenuEntry::Separator`. Submenus nest freely; every leaf
   carries an `action` token the application defines.
2. **Flat layout** (`layout.rs`): builds a depth-limited flat item list with dbusmenu item ids
   assigned in reading order, dbusmenu-style properties (label, type, enabled, children-display,
   toggle-type/state, shortcut as `aas`), a property diff between models, and bounded
   traversal (max depth 32, max items 4096 — a cyclic or huge model never hangs the export).
3. **Wire** (`dbus.rs`): the `com.canonical.dbusmenu` interface — `GetLayout` returning
   `(revision, (ia{sv}av))` where each child row is a *variant* of `(ia{sv}av)` (importers
   demarshall children as variants; a mis-shaped row breaks every decoder), plus
   `GetProperty`, `GetGroupProperties`, `GetPropertyNames`, `Event`/`EventGroup` (clicks are
   recorded; unknown or disabled ids are reported back), `AboutToShow` (always false — menus
   are never lazily fetched). Changes are announced with `LayoutUpdated` (structure change)
   or `ItemsPropertiesUpdated` (property-only), so importers never poll.
4. **Registrar** (`registrar.rs`): keeps the X11 window list registered with
   `com.canonical.AppMenu.Registrar` (falling back to the Ayatana
   `org.ayatana.AppMenu.Registrar`); the first of the two that answers wins and stays sticky.
   Importers find windows through the registrar, so no owned bus name is needed for it.
5. **Live exporter** (`live.rs`): one background thread owns one session-bus connection,
   coalesces bursts of models into a single publish, and re-scans/re-registers windows once a
   second (a registrar can appear or disappear at runtime; windows come and go too). The UI
   thread only calls `replace` (non-blocking) and `try_event`. On an unrecoverable bus error
   the worker retires itself instead of crashing the app.

## Example

```rust
use photocraft_appmenu::{MenuEntry, MenuModel};

let model = MenuModel::top(vec![
    MenuEntry::command("File", "", false).submenu(vec![
        MenuEntry::command("New…", "file.new", true).shortcut("Ctrl+N"),
        MenuEntry::Separator,
        MenuEntry::command("Save", "file.save", true).shortcut("Ctrl+S"),
    ]),
]);

let menu = photocraft_appmenu::AppMenu::start("photocraft")?;
menu.replace(&model);
// in the frame loop:
while let Some(event) = menu.try_event() {
    // dispatch `event`'s action token like a menu click
}
```

## Checklist for a *craft app adopting it

* Call `AppMenu::start(app_name)` once (lazily, on the first frame is fine), publish on a
  duty cycle or on a dirty flag, and drain `try_event` every frame.
* While a host serves the menus the app hides its in-window menu bar — with two app-side
  decisions (ui-egui here): a preference (Interface › Use Global Menu Bar, default on; off,
  the app exports nothing and keeps its in-window bar) and full-screen modes keep the
  in-window titles (the shell's bar may be hidden when the OS window is fullscreen). The
  preference and full-screen state are the app's, not the crate's.
* Map activations to the same dispatch path an in-window menu click uses, so behaviour is
  identical in both hosts.
* Keep the default behaviour on; `PHOTOCRAFT_NO_GLOBAL_MENU=1` is the opt-out convention
  (PhotoCraft's UI shell honours it in `crates/ui-egui/src/appmenu.rs`).

## Tests

Unit tests cover the layout building/diff and the wire shapes (including a byte-level
`GetLayout` reply round trip that decodes nested rows exactly as importers do), and an
integration test exchanges `GetLayout`, `GetProperty` and `Event` clicks against the real
session bus (skipping gracefully when no bus exists). Registry pins are exact (like
`photocraft-tablet`); review diffs before bumping.
