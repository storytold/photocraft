//! The document's layer stack with a lazily built id index.

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::OnceLock;

use crate::{Layer, LayerId, LayerPath};

/// Where a layer sits in the stack.
#[derive(Clone, Debug)]
struct Entry {
    path: LayerPath,
    parent: Option<LayerId>,
}

/// Id to position for every layer, nested ones included.
#[derive(Debug, Default)]
pub struct LayerIndex {
    entries: HashMap<LayerId, Entry>,
    serial: u64,
}

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl LayerIndex {
    fn build(layers: &[Layer]) -> Self {
        fn rec(layers: &[Layer], parent: Option<LayerId>, path: &mut LayerPath, out: &mut HashMap<LayerId, Entry>) {
            for (i, l) in layers.iter().enumerate() {
                path.push(i);
                out.entry(l.id).or_insert_with(|| Entry { path: path.clone(), parent });
                if let Some(ch) = l.children() {
                    rec(ch, Some(l.id), path, out);
                }
                path.pop();
            }
        }
        let mut entries = HashMap::with_capacity(layers.len());
        rec(layers, None, &mut Vec::new(), &mut entries);
        Self { entries, serial: SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed) }
    }

    /// The layer's path from the root. Paths order layers bottom-to-top as the depth-first walk does.
    pub fn path(&self, id: LayerId) -> Option<&[usize]> {
        self.entries.get(&id).map(|e| e.path.as_slice())
    }

    /// The group holding the layer; `None` at the root or for an unknown id.
    pub fn parent(&self, id: LayerId) -> Option<LayerId> {
        self.entries.get(&id)?.parent
    }

    /// Nesting depth: 0 at the root.
    pub fn depth(&self, id: LayerId) -> Option<usize> {
        self.entries.get(&id).map(|e| e.path.len() - 1)
    }

    /// The layers of `ids` that no other of them contains, in document order (bottom to top,
    /// depth first); ids not in the stack are left out.
    pub fn roots(&self, ids: &[LayerId]) -> Vec<LayerId> {
        let mut paths: Vec<(&[usize], LayerId)> = ids.iter().filter_map(|&id| Some((self.path(id)?, id))).collect();
        paths.sort_unstable();
        paths.dedup();
        let mut out: Vec<LayerId> = Vec::with_capacity(paths.len());
        let mut last: Option<&[usize]> = None;
        for (path, id) in paths {
            // Sorted by path, a contained layer follows its outermost container directly or after
            // others it contains.
            if last.is_some_and(|l| path.starts_with(l)) {
                continue;
            }
            last = Some(path);
            out.push(id);
        }
        out
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// A number unique to this build: it differs between two builds, so equal serials mean the
    /// same index (tests count rebuilds with it).
    pub fn serial(&self) -> u64 {
        self.serial
    }
}

/// The layer stack, bottom-to-top, with an index from layer id to position built on the first
/// lookup.
///
/// The stack is a public vector that code edits in place, nested groups included, so no mutator
/// method can be the place that drops the index. Every route to a `&mut Layer` or `&mut Vec`
/// goes through `DerefMut`, so the reset lives there: any write drops the index, and the next
/// lookup rebuilds it once. `Clone` starts without an index.
#[derive(Default)]
pub struct LayerTree {
    layers: Vec<Layer>,
    index: OnceLock<LayerIndex>,
}

impl LayerTree {
    /// The id index, built on first use after the last write.
    pub fn index(&self) -> &LayerIndex {
        self.index.get_or_init(|| LayerIndex::build(&self.layers))
    }
}

/// `Vec` methods that callers pass `&document` into (`doc.layers.push(make(&doc))`): as methods
/// on the tree the receiver borrow is two-phase, which it is not through `DerefMut`.
impl LayerTree {
    pub fn push(&mut self, layer: Layer) {
        self.deref_mut().push(layer);
    }

    pub fn insert(&mut self, at: usize, layer: Layer) {
        self.deref_mut().insert(at, layer);
    }
}

impl Deref for LayerTree {
    type Target = Vec<Layer>;
    fn deref(&self) -> &Vec<Layer> {
        &self.layers
    }
}

impl DerefMut for LayerTree {
    fn deref_mut(&mut self) -> &mut Vec<Layer> {
        self.index = OnceLock::new();
        &mut self.layers
    }
}

impl Clone for LayerTree {
    fn clone(&self) -> Self {
        Self { layers: self.layers.clone(), index: OnceLock::new() }
    }
}

impl PartialEq for LayerTree {
    fn eq(&self, other: &Self) -> bool {
        self.layers == other.layers
    }
}

impl std::fmt::Debug for LayerTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.layers.fmt(f)
    }
}

impl From<Vec<Layer>> for LayerTree {
    fn from(layers: Vec<Layer>) -> Self {
        Self { layers, index: OnceLock::new() }
    }
}

impl<'a> IntoIterator for &'a LayerTree {
    type Item = &'a Layer;
    type IntoIter = std::slice::Iter<'a, Layer>;
    fn into_iter(self) -> Self::IntoIter {
        self.layers.iter()
    }
}

impl<'a> IntoIterator for &'a mut LayerTree {
    type Item = &'a mut Layer;
    type IntoIter = std::slice::IterMut<'a, Layer>;
    fn into_iter(self) -> Self::IntoIter {
        self.deref_mut().iter_mut()
    }
}

impl FromIterator<Layer> for LayerTree {
    fn from_iter<I: IntoIterator<Item = Layer>>(iter: I) -> Self {
        iter.into_iter().collect::<Vec<_>>().into()
    }
}

impl IntoIterator for LayerTree {
    type Item = Layer;
    type IntoIter = std::vec::IntoIter<Layer>;
    fn into_iter(self) -> Self::IntoIter {
        self.layers.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PixelFormat;

    fn layers(n: usize) -> LayerTree {
        (0..n).map(|i| Layer::raster(format!("l{i}"), PixelFormat::RGBA8)).collect()
    }

    #[test]
    fn index_builds_once_per_edit() {
        let mut t = layers(200);
        let id = t[150].id;
        let first = t.index().serial();
        for _ in 0..100 {
            assert_eq!(t.index().path(id), Some(&[150][..]));
        }
        assert_eq!(t.index().serial(), first, "lookups reuse the index");
        t.push(Layer::raster("new", PixelFormat::RGBA8));
        let second = t.index().serial();
        assert_ne!(second, first, "an edit rebuilds it");
        assert_eq!(t.index().serial(), second);
        let _ = &mut t[3];
        assert_ne!(t.index().serial(), second, "any mutable access drops it");
    }

    #[test]
    fn clone_after_edit_does_not_reuse_the_index() {
        let mut t = layers(5);
        let old = t.index().serial();
        let new_id = {
            t.insert(0, Layer::raster("below", PixelFormat::RGBA8));
            t[0].id
        };
        let c = t.clone();
        assert_ne!(c.index().serial(), old);
        assert_eq!(c.index().path(new_id), Some(&[0][..]));
        assert_eq!(t.index().path(new_id), Some(&[0][..]));
    }

    #[test]
    fn nested_edits_reset_and_paths_follow_walk_order() {
        let inner = Layer::raster("in", PixelFormat::RGBA8);
        let inner_id = inner.id;
        let group = Layer::group("g", vec![inner]);
        let group_id = group.id;
        let mut t: LayerTree = vec![Layer::raster("a", PixelFormat::RGBA8), group].into();
        assert_eq!(t.index().path(inner_id), Some(&[1, 0][..]));
        assert_eq!(t.index().parent(inner_id), Some(group_id));
        assert_eq!(t.index().depth(inner_id), Some(1));
        let before = t.index().serial();
        t[1].children_mut().unwrap().insert(0, Layer::raster("x", PixelFormat::RGBA8));
        assert_eq!(t.index().path(inner_id), Some(&[1, 1][..]));
        assert_ne!(t.index().serial(), before);
    }
}
