//! Pure operations on the workspace split tree (`cmux_protocol::LayoutNode`).
//!
//! Split nodes are addressed by *path*: a sequence of booleans from the root
//! (`false` = first child, `true` = second child). The frontend tracks the
//! same paths while rendering recursively, so a divider can name its split
//! without the tree carrying IDs.

use cmux_protocol::{LayoutNode, PaneId, SplitAxis};

/// All panes in the tree, in-order.
pub fn panes(node: &LayoutNode) -> Vec<PaneId> {
    let mut out = Vec::new();
    collect(node, &mut out);
    out
}

fn collect(node: &LayoutNode, out: &mut Vec<PaneId>) {
    match node {
        LayoutNode::Leaf { pane } => out.push(*pane),
        LayoutNode::Split { first, second, .. } => {
            collect(first, out);
            collect(second, out);
        }
    }
}

/// Replace the leaf holding `target` with a split of `target` + `new_pane`.
/// Returns false if `target` is not in the tree.
pub fn split(node: &mut LayoutNode, target: PaneId, axis: SplitAxis, new_pane: PaneId) -> bool {
    match node {
        LayoutNode::Leaf { pane } if *pane == target => {
            *node = LayoutNode::Split {
                axis,
                ratio: 0.5,
                first: Box::new(LayoutNode::Leaf { pane: target }),
                second: Box::new(LayoutNode::Leaf { pane: new_pane }),
            };
            true
        }
        LayoutNode::Leaf { .. } => false,
        LayoutNode::Split { first, second, .. } => {
            split(first, target, axis, new_pane) || split(second, target, axis, new_pane)
        }
    }
}

/// Remove `target`, collapsing its parent split into the sibling.
/// Returns `None` when the tree becomes empty.
pub fn remove(node: LayoutNode, target: PaneId) -> Option<LayoutNode> {
    match node {
        LayoutNode::Leaf { pane } if pane == target => None,
        leaf @ LayoutNode::Leaf { .. } => Some(leaf),
        LayoutNode::Split { axis, ratio, first, second } => {
            match (remove(*first, target), remove(*second, target)) {
                (Some(f), Some(s)) => Some(LayoutNode::Split {
                    axis,
                    ratio,
                    first: Box::new(f),
                    second: Box::new(s),
                }),
                (Some(f), None) => Some(f),
                (None, Some(s)) => Some(s),
                (None, None) => None, // unreachable: target appears once
            }
        }
    }
}

/// Set the ratio of the split at `path`. Clamped to keep both sides usable.
pub fn set_ratio(node: &mut LayoutNode, path: &[bool], ratio: f32) -> bool {
    match node {
        LayoutNode::Leaf { .. } => false,
        LayoutNode::Split { ratio: r, first, second, .. } => match path.split_first() {
            None => {
                *r = ratio.clamp(0.05, 0.95);
                true
            }
            Some((&step, rest)) => {
                set_ratio(if step { second } else { first }, rest, ratio)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(p: PaneId) -> LayoutNode {
        LayoutNode::Leaf { pane: p }
    }

    #[test]
    fn split_remove_collapses() {
        let (a, b, c) = (PaneId::new(), PaneId::new(), PaneId::new());
        let mut tree = leaf(a);
        assert!(split(&mut tree, a, SplitAxis::Horizontal, b));
        assert!(split(&mut tree, b, SplitAxis::Vertical, c));
        assert_eq!(panes(&tree), vec![a, b, c]);

        // Removing the middle pane collapses its split into the sibling.
        let tree = remove(tree, b).expect("tree not empty");
        assert_eq!(panes(&tree), vec![a, c]);

        let tree = remove(tree, a).expect("tree not empty");
        assert_eq!(panes(&tree), vec![c]);
        assert!(remove(tree, c).is_none());
    }

    #[test]
    fn ratio_by_path() {
        let (a, b, c) = (PaneId::new(), PaneId::new(), PaneId::new());
        let mut tree = leaf(a);
        split(&mut tree, a, SplitAxis::Horizontal, b);
        split(&mut tree, b, SplitAxis::Vertical, c);

        assert!(set_ratio(&mut tree, &[], 0.7));
        assert!(set_ratio(&mut tree, &[true], 0.3));
        assert!(!set_ratio(&mut tree, &[false], 0.5)); // leaf, not a split

        match &tree {
            LayoutNode::Split { ratio, second, .. } => {
                assert!((ratio - 0.7).abs() < 1e-6);
                match &**second {
                    LayoutNode::Split { ratio, .. } => assert!((ratio - 0.3).abs() < 1e-6),
                    _ => panic!("expected nested split"),
                }
            }
            _ => panic!("expected split root"),
        }
    }
}
