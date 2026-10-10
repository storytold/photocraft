//! The rows of the LUT list: which sections exist, which are open, and what each row stands for.

use std::collections::HashMap;
use std::sync::Arc;

use photocraft_engine::lut_library::{LutLibrary, PackInfo};

use crate::PhotocraftApp;

/// The built-in looks section key.
pub(super) const BUILTIN: &str = "builtin";
pub(super) const FAVORITES: &str = "favorites";
pub(super) const RECENT: &str = "recent";

/// One folder of a pack: its sub-folders and the LUTs directly inside it (indexes into the pack).
#[derive(Clone, Default)]
pub(super) struct Dir {
    pub name: String,
    pub dirs: Vec<Dir>,
    pub luts: Vec<usize>,
}

#[derive(Clone)]
pub(super) struct Pack {
    pub info: PackInfo,
    pub root: Dir,
}

#[derive(Clone)]
pub(super) struct Listing {
    pub rev: u64,
    pub packs: Arc<Vec<Pack>>,
    /// `Pack/path.cube` → (pack index, LUT index).
    pub ids: Arc<HashMap<String, (usize, usize)>>,
}

pub(super) enum Row {
    None,
    Look(usize),
    Custom,
    Header { key: String, title: String, depth: u8, open: bool, pack: Option<String> },
    Lut { pack: usize, idx: usize, depth: u8 },
}

fn build_tree(info: &PackInfo) -> Dir {
    let mut root = Dir::default();
    for (idx, lut) in info.luts.iter().enumerate() {
        let mut parts: Vec<&str> = lut.file.split('/').collect();
        parts.pop();
        let mut at = &mut root;
        for part in parts {
            let i = match at.dirs.iter().position(|d| d.name == part) {
                Some(i) => i,
                None => {
                    at.dirs.push(Dir { name: part.to_string(), ..Dir::default() });
                    at.dirs.len() - 1
                }
            };
            let Some(next) = at.dirs.get_mut(i) else { break };
            at = next;
        }
        at.luts.push(idx);
    }
    root
}

impl Listing {
    pub(super) fn build(lib: &LutLibrary) -> Listing {
        let infos = lib.list().unwrap_or_default();
        let mut ids = HashMap::new();
        for (pi, info) in infos.iter().enumerate() {
            for (li, lut) in info.luts.iter().enumerate() {
                ids.insert(format!("{}/{}", info.name, lut.file), (pi, li));
            }
        }
        let packs = infos.into_iter().map(|info| Pack { root: build_tree(&info), info }).collect();
        Listing { rev: lib.rev, packs: Arc::new(packs), ids: Arc::new(ids) }
    }
}

/// Everything the row builder reads.
pub(super) struct Source<'a> {
    pub packs: &'a [Pack],
    pub ids: &'a HashMap<String, (usize, usize)>,
    pub builtins: &'a [(&'a str, &'a str)],
    pub custom_shown: bool,
    pub needle: String,
    pub open: &'a HashMap<String, bool>,
    pub favorites: &'a [String],
    pub recent: &'a [String],
    /// `pack → the pack that already holds all of its LUTs`.
    pub redundant: &'a HashMap<String, String>,
}

impl Source<'_> {
    pub(super) fn searching(&self) -> bool {
        !self.needle.is_empty()
    }

    /// Sections start closed, except Favorites and Recent, and the built-in looks while no pack is
    /// installed.
    fn is_open(&self, key: &str) -> bool {
        self.searching() || self.open.get(key).copied().unwrap_or(key == FAVORITES || key == RECENT || (key == BUILTIN && self.packs.is_empty()))
    }

    fn matches(&self, name: &str) -> bool {
        !self.searching() || name.to_lowercase().contains(&self.needle)
    }

    pub(super) fn lut_id(&self, pack: usize, idx: usize) -> Option<String> {
        let p = self.packs.get(pack)?;
        Some(format!("{}/{}", p.info.name, p.info.luts.get(idx)?.file))
    }

    fn dir_rows(&self, pi: usize, pack: &Pack, dir: &Dir, trail: &str, depth: u8) -> Vec<Row> {
        let mut rows = Vec::new();
        for sub in &dir.dirs {
            let key = format!("dir:{}:{trail}{}", pack.info.name, sub.name);
            let open = self.is_open(&key);
            let inner = if open { self.dir_rows(pi, pack, sub, &format!("{trail}{}/", sub.name), depth + 1) } else { Vec::new() };
            if self.searching() && inner.is_empty() {
                continue;
            }
            rows.push(Row::Header { key, title: sub.name.clone(), depth, open, pack: None });
            rows.extend(inner);
        }
        for &idx in &dir.luts {
            if pack.info.luts.get(idx).is_some_and(|l| self.matches(&l.name)) {
                rows.push(Row::Lut { pack: pi, idx, depth });
            }
        }
        rows
    }

    /// A section of LUTs picked by id (Favorites, Recent).
    fn id_section(&self, key: &str, title: &str, ids: &[String], rows: &mut Vec<Row>) {
        let found: Vec<(usize, usize)> = ids
            .iter()
            .filter_map(|id| self.ids.get(id).copied())
            .filter(|&(p, l)| self.packs.get(p).and_then(|p| p.info.luts.get(l)).is_some_and(|l| self.matches(&l.name)))
            .collect();
        if found.is_empty() {
            return;
        }
        let open = self.is_open(key);
        rows.push(Row::Header { key: key.into(), title: format!("{title} ({})", found.len()), depth: 0, open, pack: None });
        if open {
            rows.extend(found.into_iter().map(|(pack, idx)| Row::Lut { pack, idx, depth: 1 }));
        }
    }

    pub(super) fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if !self.searching() {
            rows.push(Row::None);
            if self.custom_shown {
                rows.push(Row::Custom);
            }
        }
        self.id_section(FAVORITES, tl!("Favorites"), self.favorites, &mut rows);
        self.id_section(RECENT, tl!("Recent"), self.recent, &mut rows);
        let looks: Vec<Row> = self.builtins.iter().enumerate().filter(|(_, (_, label))| self.matches(label)).map(|(i, _)| Row::Look(i)).collect();
        if !looks.is_empty() {
            let open = self.is_open(BUILTIN);
            rows.push(Row::Header { key: BUILTIN.into(), title: tl!("Built-in").into(), depth: 0, open, pack: None });
            if open {
                rows.extend(looks);
            }
        }
        for (pi, pack) in self.packs.iter().enumerate() {
            let key = format!("pack:{}", pack.info.name);
            let open = self.is_open(&key);
            let inner = if open { self.dir_rows(pi, pack, &pack.root, "", 1) } else { Vec::new() };
            if self.searching() && inner.is_empty() {
                continue;
            }
            let title = format!("{} ({})", pack.info.name, pack.info.luts.len());
            rows.push(Row::Header { key, title, depth: 0, open, pack: Some(pack.info.name.clone()) });
            rows.extend(inner);
        }
        rows
    }

    /// The identity the keyboard cursor and the highlight use for a row (`None` for headers).
    pub(super) fn key_of(&self, row: &Row) -> Option<String> {
        match row {
            Row::None => Some("none".into()),
            Row::Look(i) => self.builtins.get(*i).map(|(id, _)| format!("look:{id}")),
            Row::Lut { pack, idx, .. } => self.lut_id(*pack, *idx).map(|id| format!("lut:{id}")),
            _ => None,
        }
    }
}

/// The listing for this frame, rebuilt when the library changed.
pub(super) fn listing(app: &PhotocraftApp, ui: &egui::Ui) -> Option<Listing> {
    let lib = app.session.lut_library.as_ref()?;
    let id = egui::Id::new("lutlib-listing");
    if let Some(cached) = ui.data(|d| d.get_temp::<Listing>(id))
        && cached.rev == lib.rev
    {
        return Some(cached);
    }
    let fresh = Listing::build(lib);
    ui.data_mut(|d| d.insert_temp(id, fresh.clone()));
    Some(fresh)
}
