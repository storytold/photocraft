//! The Layers panel's Group filter (#10883): every group with its whole subtree; layers in no
//! group are hidden. An artboard row is not a group here, but the groups inside it are.

use std::collections::HashSet;

use photocraft_doc::{Document, Layer, LayerContent};

/// The ids of the layers the Group filter lists.
pub fn listed(doc: &Document) -> HashSet<u64> {
    fn rec(layers: &[Layer], inside: bool, out: &mut HashSet<u64>) {
        for l in layers {
            let artboard = matches!(&l.content, LayerContent::Group(g) if g.artboard.is_some());
            let inside = inside || (l.is_group() && !artboard);
            if inside {
                out.insert(l.id.0);
            }
            if let Some(children) = l.children() {
                rec(children, inside, out);
            }
        }
    }
    let mut out = HashSet::new();
    rec(&doc.layers, false, &mut out);
    out
}