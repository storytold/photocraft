
use photocraft_ui_egui::native_menu::{
    self, Backend, Event, ItemRole, MenuBar, Node,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::zvariant::{OwnedValue, StructureBuilder, Value};

const MENU_PATH: &str = "/com/canonical/photocraft/Menu";
const REGISTRAR_SERVICE: &str = "com.canonical.AppMenu.Registrar";
const REGISTRAR_PATH: &str = "/com/canonical/AppMenu/Registrar";
const REGISTRAR_INTERFACE: &str = "com.canonical.AppMenu.Registrar";

#[derive(Clone, Debug, PartialEq, Eq)]
struct MenuNode {
    id: i32,
    label: String,
    enabled: bool,
    checked: Option<bool>,
    role: Option<ItemRole>,
    separator: bool,
    children: Vec<MenuNode>,
    command_id: Option<String>,
}

#[derive(Default)]
struct MenuState {
    tree: Vec<MenuNode>,
    events: Vec<Event>,
    revision: u32,
    pending_property_names: HashMap<i32, Vec<String>>,
}

type SharedState = Arc<Mutex<MenuState>>;
type LayoutItem = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

fn lock_state(state: &SharedState) -> std::sync::MutexGuard<'_, MenuState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn convert_nodes(nodes: &[Node], next_id: &mut i32) -> Vec<MenuNode> {
    let mut result = Vec::new();

    for node in nodes {
        let id = *next_id;
        *next_id += 1;

        let converted = match node {
            Node::Item(item) => MenuNode {
                id,
                label: item.label.clone(),
                enabled: item.enabled,
                checked: item.checked,
                role: item.role,
                separator: false,
                children: Vec::new(),
                command_id: Some(item.id.clone()),
            },
            Node::Submenu { label, children, .. } => MenuNode {
                id,
                label: label.clone(),
                enabled: true,
                checked: None,
                role: None,
                separator: false,
                children: convert_nodes(children, next_id),
                command_id: None,
            },
            Node::Separator => MenuNode {
                id,
                label: String::new(),
                enabled: false,
                checked: None,
                role: None,
                separator: true,
                children: Vec::new(),
                command_id: None,
            },
            Node::Standard(_) => continue,
        };

        result.push(converted);
    }

    result
}

fn build_menu_tree(bar: &MenuBar) -> Vec<MenuNode> {
    let mut next_id = 1;

    bar.menus
        .iter()
        .map(|menu| {
            let id = next_id;
            next_id += 1;

            MenuNode {
                id,
                label: menu.title.clone(),
                enabled: true,
                checked: None,
                role: None,
                separator: false,
                children: convert_nodes(&menu.children, &mut next_id),
                command_id: None,
            }
        })
        .collect()
}

/// 레이블, 활성화, 체크 상태는 레이아웃 구조에 포함하지 않는다.
fn same_layout(old: &[MenuNode], new: &[MenuNode]) -> bool {
    old.len() == new.len()
        && old.iter().zip(new).all(|(a, b)| {
            a.id == b.id
                && a.separator == b.separator
                && a.command_id == b.command_id
                && same_layout(&a.children, &b.children)
        })
}

fn record_property(
    changed: &mut HashMap<i32, Vec<String>>,
    id: i32,
    property: &str,
) {
    let names = changed.entry(id).or_default();

    if !names.iter().any(|name| name == property) {
        names.push(property.to_owned());
    }
}

fn collect_changed_properties(
    old: &[MenuNode],
    new: &[MenuNode],
    changed: &mut HashMap<i32, Vec<String>>,
) {
    for (a, b) in old.iter().zip(new) {
        if a.label != b.label {
            record_property(changed, b.id, "label");
        }

        if a.enabled != b.enabled {
            record_property(changed, b.id, "enabled");
        }

        if a.checked != b.checked {
            record_property(changed, b.id, "toggle-type");
            record_property(changed, b.id, "toggle-state");
        }

        collect_changed_properties(&a.children, &b.children, changed);
    }
}

fn find_node(nodes: &[MenuNode], id: i32) -> Option<&MenuNode> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }

        if let Some(found) = find_node(&node.children, id) {
            return Some(found);
        }
    }

    None
}

fn owned_value(value: Value<'_>) -> zbus::fdo::Result<OwnedValue> {
    OwnedValue::try_from(value)
        .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
}

fn menu_properties(
    node: &MenuNode,
    requested: &[String],
) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
    let mut properties = HashMap::new();

    let wants = |name: &str| {
        requested.is_empty() || requested.iter().any(|property| property == name)
    };

    if wants("label") {
        properties.insert(
            "label".to_owned(),
            owned_value(Value::from(node.label.clone()))?,
        );
    }

    if wants("enabled") {
        properties.insert(
            "enabled".to_owned(),
            owned_value(Value::from(node.enabled))?,
        );
    }

    if wants("visible") {
        properties.insert(
            "visible".to_owned(),
            owned_value(Value::from(true))?,
        );
    }

    if node.separator && wants("type") {
        properties.insert(
            "type".to_owned(),
            owned_value(Value::from("separator"))?,
        );
    }

    if !node.children.is_empty() && wants("children-display") {
        properties.insert(
            "children-display".to_owned(),
            owned_value(Value::from("submenu"))?,
        );
    }

    if let Some(checked) = node.checked {
        if wants("toggle-type") {
            properties.insert(
                "toggle-type".to_owned(),
                owned_value(Value::from("checkmark"))?,
            );
        }

        if wants("toggle-state") {
            let value: i32 = if checked { 1 } else { 0 };

            properties.insert(
                "toggle-state".to_owned(),
                owned_value(Value::from(value))?,
            );
        }
    }

    Ok(properties)
}

fn layout_node(
    node: &MenuNode,
    depth: i32,
    property_names: &[String],
) -> zbus::fdo::Result<LayoutItem> {
    let properties = menu_properties(node, property_names)?;
    let mut children = Vec::new();

    if depth != 0 {
        let child_depth = if depth < 0 { -1 } else { depth - 1 };

        for child in &node.children {
            let (id, properties, child_items) =
                layout_node(child, child_depth, property_names)?;

            let structure = StructureBuilder::new()
                .add_field(id)
                .add_field(properties)
                .add_field(child_items)
                .build()
                .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;

            children.push(owned_value(Value::from(structure))?);
        }
    }

    Ok((node.id, properties, children))
}

struct DbusMenu {
    state: SharedState,
    ctx: egui::Context,
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl DbusMenu {
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, LayoutItem)> {
        let state = lock_state(&self.state);

        let root = MenuNode {
            id: 0,
            label: String::new(),
            enabled: true,
            checked: None,
            role: None,
            separator: false,
            children: state.tree.clone(),
            command_id: None,
        };

        let parent = if parent_id == 0 {
            &root
        } else {
            find_node(&state.tree, parent_id).ok_or_else(|| {
                zbus::fdo::Error::InvalidArgs(format!(
                    "Unknown menu item ID: {parent_id}"
                ))
            })?
        };

        Ok((
            state.revision,
            layout_node(parent, recursion_depth, &property_names)?,
        ))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> zbus::fdo::Result<Vec<(i32, HashMap<String, OwnedValue>)>> {
        let state = lock_state(&self.state);

        ids.into_iter()
            .filter_map(|id| {
                find_node(&state.tree, id).map(|node| (id, node))
            })
            .map(|(id, node)| {
                menu_properties(node, &property_names)
                    .map(|properties| (id, properties))
            })
            .collect()
    }

    fn get_property(
        &self,
        id: i32,
        name: &str,
    ) -> zbus::fdo::Result<OwnedValue> {
        let state = lock_state(&self.state);

        let node = find_node(&state.tree, id).ok_or_else(|| {
            zbus::fdo::Error::InvalidArgs(format!(
                "Unknown menu item ID: {id}"
            ))
        })?;

        menu_properties(node, &[name.to_owned()])?
            .remove(name)
            .ok_or_else(|| zbus::fdo::Error::UnknownProperty(name.to_owned()))
    }

    fn event(
        &self,
        id: i32,
        event_id: &str,
        _data: OwnedValue,
        _timestamp: u32,
    ) -> zbus::fdo::Result<()> {
        if event_id != "clicked" {
            return Ok(());
        }

        let command_id = {
            let state = lock_state(&self.state);

            find_node(&state.tree, id)
                .filter(|node| node.enabled)
                .and_then(|node| node.command_id.clone())
        };

        if let Some(command_id) = command_id {
            lock_state(&self.state)
                .events
                .push(Event::Click(command_id));

            self.ctx.request_repaint();
        }

        Ok(())
    }

    fn about_to_show(&self, _id: i32) -> zbus::fdo::Result<bool> {
        Ok(false)
    }

    #[zbus(signal)]
    async fn layout_updated(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn items_properties_updated(
        signal_ctxt: &zbus::object_server::SignalEmitter<'_>,
        updated_properties: Vec<(i32, HashMap<String, OwnedValue>)>,
        removed_properties: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "normal".to_owned()
    }
}

pub struct LinuxMenu {
    state: SharedState,
}

impl Backend for LinuxMenu {
    fn sync(&mut self, bar: &MenuBar) {
        let tree = build_menu_tree(bar);
        let mut state = lock_state(&self.state);

        if same_layout(&state.tree, &tree) {
            let old_tree = state.tree.clone();

            collect_changed_properties(
                &old_tree,
                &tree,
                &mut state.pending_property_names,
            );

            state.tree = tree;
        } else {
            eprintln!("[global-menu] layout changed");

            state.tree = tree;
            state.revision = state.revision.wrapping_add(1);

            // 레이아웃 전체를 다시 읽게 되므로 속성 알림은 합친다.
            state.pending_property_names.clear();
        }
    }

    fn drain(&mut self) -> Vec<Event> {
        std::mem::take(&mut lock_state(&self.state).events)
    }
}

fn find_x11_window(pid: u32) -> Result<u32, String> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, Window};
    use x11rb::rust_connection::RustConnection;

    fn search(
        conn: &RustConnection,
        parent: Window,
        pid_atom: u32,
        pid: u32,
        depth: usize,
    ) -> Option<Window> {
        let tree = conn.query_tree(parent).ok()?.reply().ok()?;

        for window in &tree.children {
            let property = conn
                .get_property(
                    false,
                    *window,
                    pid_atom,
                    AtomEnum::CARDINAL,
                    0,
                    1,
                )
                .ok()
                .and_then(|cookie| cookie.reply().ok());

            if property
                .as_ref()
                .and_then(|reply| {
                    reply.value32().and_then(|mut values| values.next())
                })
                == Some(pid)
            {
                return Some(*window);
            }
        }

        if depth > 0 {
            for window in tree.children {
                if let Some(found) =
                    search(conn, window, pid_atom, pid, depth - 1)
                {
                    return Some(found);
                }
            }
        }

        None
    }

    let (conn, screen_num) =
        RustConnection::connect(None).map_err(|error| error.to_string())?;

    let screen = conn
        .setup()
        .roots
        .get(screen_num)
        .ok_or_else(|| "X11 screen not found".to_owned())?;

    let pid_atom = conn
        .intern_atom(false, b"_NET_WM_PID")
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| error.to_string())?
        .atom;

    search(&conn, screen.root, pid_atom, pid, 4)
        .map(|window| window as u32)
        .ok_or_else(|| "Could not find PhotoCraft's X11 window".to_owned())
}

/// 대기 중인 속성 변경을 현재 트리의 최신 값으로 변환한다.
fn take_property_updates(
    state: &mut MenuState,
) -> zbus::fdo::Result<(
    Vec<(i32, HashMap<String, OwnedValue>)>,
    Vec<(i32, Vec<String>)>,
)> {
    let pending = std::mem::take(&mut state.pending_property_names);
    let mut updated = Vec::new();
    let mut removed = Vec::new();

    for (id, names) in pending {
        let Some(node) = find_node(&state.tree, id) else {
            continue;
        };

        let properties = menu_properties(node, &names)?;

        let removed_names: Vec<String> = names
            .into_iter()
            .filter(|name| !properties.contains_key(name))
            .collect();

        if !properties.is_empty() {
            updated.push((id, properties));
        }

        if !removed_names.is_empty() {
            removed.push((id, removed_names));
        }
    }

    Ok((updated, removed))
}

fn start_dbus(
    state: SharedState,
    ctx: egui::Context,
    ready: std::sync::mpsc::SyncSender<Result<(), String>>,
) {
    let window_id = match find_x11_window(std::process::id()) {
        Ok(id) => id,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };

    let watch_state = Arc::clone(&state);

    let connection = async_io::block_on(async {
        let builder = zbus::connection::Builder::session()
            .map_err(|error| error.to_string())?
            .serve_at(
                MENU_PATH,
                DbusMenu {
                    state,
                    ctx,
                },
            )
            .map_err(|error| error.to_string())?;

        let connection = builder
            .build()
            .await
            .map_err(|error| error.to_string())?;

        let proxy = zbus::Proxy::new(
            &connection,
            REGISTRAR_SERVICE,
            REGISTRAR_PATH,
            REGISTRAR_INTERFACE,
        )
        .await
        .map_err(|error| error.to_string())?;

        let menu_path = zbus::zvariant::ObjectPath::try_from(MENU_PATH)
            .map_err(|error| error.to_string())?;

        let _: () = proxy
            .call("RegisterWindow", &(window_id, menu_path))
            .await
            .map_err(|error| error.to_string())?;

        Ok::<_, String>(connection)
    });

    match connection {
        Ok(connection) => {
            let _ = ready.send(Ok(()));

            let signal_ctxt =
                match zbus::object_server::SignalEmitter::new(
                    &connection,
                    MENU_PATH,
                ) {
                    Ok(ctxt) => ctxt,
                    Err(error) => {
                        log::warn!(
                            "could not create global menu signal context: {error}"
                        );
                        return;
                    }
                };

            let mut last_revision = lock_state(&watch_state).revision;

            async_io::block_on(async {
                loop {
                    async_io::Timer::after(Duration::from_millis(50)).await;

                    let (revision, property_updates) = {
                        let mut state = lock_state(&watch_state);
                        let revision = state.revision;

                        let updates = match take_property_updates(&mut state) {
                            Ok(updates) => updates,
                            Err(error) => {
                                log::warn!(
                                    "could not prepare global menu property updates: {error}"
                                );
                                continue;
                            }
                        };

                        (revision, updates)
                    };

                    // 구조 변경은 레이아웃 신호로 전달한다.
                    if revision != last_revision {
                        match DbusMenu::layout_updated(
                            &signal_ctxt,
                            revision,
                            0,
                        )
                        .await
                        {
                            Ok(()) => {
                                last_revision = revision;
                            }
                            Err(error) => {
                                log::warn!(
                                    "could not emit global menu LayoutUpdated signal: {error}"
                                );
                                break;
                            }
                        }

                        continue;
                    }

                    let (updated, removed) = property_updates;

                    if updated.is_empty() && removed.is_empty() {
                        continue;
                    }

                    if let Err(error) =
                        DbusMenu::items_properties_updated(
                            &signal_ctxt,
                            updated,
                            removed,
                        )
                        .await
                    {
                        log::warn!(
                            "could not emit global menu ItemsPropertiesUpdated signal: {error}"
                        );
                    }
                }
            });
        }
        Err(error) => {
            let _ = ready.send(Err(error));
        }
    }
}

pub fn install(
    ctx: &egui::Context,
    app: &photocraft_ui_egui::PhotocraftApp,
) -> Option<native_menu::NativeMenu> {
    let items = photocraft_ui_egui::menus::menu_items(app);

    let lang = photocraft_ui_egui::i18n::Lang::from_pref(
        &app.session.prefs().interface.language,
    );

    let bar = native_menu::localized_bar(&items, lang);

    let state = Arc::new(Mutex::new(MenuState {
        tree: build_menu_tree(&bar),
        events: Vec::new(),
        revision: 1,
        pending_property_names: HashMap::new(),
    }));

    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let thread_state = Arc::clone(&state);
    let thread_ctx = ctx.clone();

    let spawn = std::thread::Builder::new()
        .name("photocraft-global-menu".to_owned())
        .spawn(move || start_dbus(thread_state, thread_ctx, ready_tx));

    if let Err(error) = spawn {
        log::warn!("could not start Linux global menu thread: {error}");
        return None;
    }

    match ready_rx.recv_timeout(Duration::from_secs(4)) {
        Ok(Ok(())) => Some(native_menu::NativeMenu::new(Box::new(LinuxMenu {
            state,
        }))),
        Ok(Err(error)) => {
            log::warn!("Linux global menu unavailable: {error}");
            None
        }
        Err(error) => {
            log::warn!("Linux global menu registration timed out: {error}");
            None
        }
    }
}
