//! Same-revision view/save changes may move COW pointers without editing pixels.
use photocraft_doc::{Document, Layer, LayerContent};

pub(super) fn same_content(original: &Document, current: &Document) -> bool {
    // Bound the recursive clone/equality work before entering it. A large/deep
    // document conservatively stays stale rather than allocating an unbounded copy.
    for doc in [original, current] {
        if doc.max_group_depth() > 64 {
            return false;
        }
        let mut stack = vec![doc.layers.as_slice()];
        let mut count = 0usize;
        while let Some(layers) = stack.pop() {
            count = count.saturating_add(layers.len());
            if count > 32_768 {
                return false;
            }
            for layer in layers {
                if let Some(children) = layer.children() {
                    stack.push(children);
                }
            }
        }
    }
    let mut candidate = current.clone();
    candidate.name.clone_from(&original.name);
    if !normalize_disclosure(&original.layers, &mut candidate.layers, 0) {
        return false;
    }
    original == &candidate
}

fn normalize_disclosure(original: &[Layer], current: &mut [Layer], depth: usize) -> bool {
    if depth > 64 || original.len() != current.len() {
        return false;
    }
    for (before, after) in original.iter().zip(current) {
        if before.id != after.id {
            return false;
        }
        if let (LayerContent::Group(before), LayerContent::Group(after)) = (&before.content, &mut after.content) {
            after.expanded = before.expanded;
            if !normalize_disclosure(&before.children, &mut after.children, depth.saturating_add(1)) {
                return false;
            }
        }
    }
    true
}
