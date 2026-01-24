//! BSP tree for managing pane splits
//!
//! The split tree organizes panes in a binary space partition,
//! allowing horizontal and vertical splits at any level.

use super::pane::PaneId;

/// Direction of a split
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    /// Split horizontally (panes side by side)
    Horizontal,
    /// Split vertically (panes stacked)
    Vertical,
}

/// A node in the BSP tree
#[derive(Debug)]
pub enum SplitNode {
    /// A leaf node containing a pane
    Leaf(PaneId),
    /// An internal node with two children
    Split {
        direction: SplitDirection,
        /// Ratio of first child (0.0 - 1.0)
        ratio: f32,
        /// First child (left or top)
        first: Box<SplitNode>,
        /// Second child (right or bottom)
        second: Box<SplitNode>,
    },
}

impl SplitNode {
    /// Create a new leaf node
    pub fn leaf(id: PaneId) -> Self {
        Self::Leaf(id)
    }

    /// Split this node, replacing the current content
    pub fn split(&mut self, direction: SplitDirection, new_pane: PaneId, after: bool) {
        let old = std::mem::replace(self, SplitNode::Leaf(new_pane));

        let (first, second) = if after {
            (Box::new(old), Box::new(SplitNode::Leaf(new_pane)))
        } else {
            (Box::new(SplitNode::Leaf(new_pane)), Box::new(old))
        };

        *self = SplitNode::Split {
            direction,
            ratio: 0.5,
            first,
            second,
        };
    }

    /// Find a node containing the given pane
    pub fn find(&self, id: PaneId) -> Option<&SplitNode> {
        match self {
            SplitNode::Leaf(leaf_id) if *leaf_id == id => Some(self),
            SplitNode::Leaf(_) => None,
            SplitNode::Split { first, second, .. } => {
                first.find(id).or_else(|| second.find(id))
            }
        }
    }

    /// Find a mutable node containing the given pane
    pub fn find_mut(&mut self, id: PaneId) -> Option<&mut SplitNode> {
        match self {
            SplitNode::Leaf(leaf_id) if *leaf_id == id => Some(self),
            SplitNode::Leaf(_) => None,
            SplitNode::Split { first, second, .. } => {
                if first.find(id).is_some() {
                    first.find_mut(id)
                } else {
                    second.find_mut(id)
                }
            }
        }
    }

    /// Remove a pane, collapsing the tree if needed
    /// Returns the sibling that should take over, or None if this is the only pane
    pub fn remove(&mut self, id: PaneId) -> Option<PaneId> {
        match self {
            SplitNode::Leaf(leaf_id) if *leaf_id == id => None,
            SplitNode::Leaf(_) => None,
            SplitNode::Split { first, second, .. } => {
                // Check if first child is the target
                if let SplitNode::Leaf(leaf_id) = first.as_ref() {
                    if *leaf_id == id {
                        // Replace self with second child
                        let sibling = second.first_pane();
                        *self = std::mem::replace(second.as_mut(), SplitNode::Leaf(PaneId(0)));
                        return sibling;
                    }
                }

                // Check if second child is the target
                if let SplitNode::Leaf(leaf_id) = second.as_ref() {
                    if *leaf_id == id {
                        // Replace self with first child
                        let sibling = first.first_pane();
                        *self = std::mem::replace(first.as_mut(), SplitNode::Leaf(PaneId(0)));
                        return sibling;
                    }
                }

                // Recurse into children
                if first.find(id).is_some() {
                    first.remove(id)
                } else {
                    second.remove(id)
                }
            }
        }
    }

    /// Get the first pane in this subtree (for focus fallback)
    pub fn first_pane(&self) -> Option<PaneId> {
        match self {
            SplitNode::Leaf(id) => Some(*id),
            SplitNode::Split { first, .. } => first.first_pane(),
        }
    }

    /// Get all pane IDs in this subtree
    pub fn all_panes(&self) -> Vec<PaneId> {
        match self {
            SplitNode::Leaf(id) => vec![*id],
            SplitNode::Split { first, second, .. } => {
                let mut panes = first.all_panes();
                panes.extend(second.all_panes());
                panes
            }
        }
    }

    /// Calculate layouts for all panes given a bounding rectangle
    pub fn layout(&self, x: u32, y: u32, width: u32, height: u32) -> Vec<PaneLayout> {
        match self {
            SplitNode::Leaf(id) => {
                vec![PaneLayout {
                    id: *id,
                    x,
                    y,
                    width,
                    height,
                }]
            }
            SplitNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let mut layouts = Vec::new();

                match direction {
                    SplitDirection::Horizontal => {
                        let first_width = ((width as f32) * ratio) as u32;
                        let second_width = width - first_width;

                        layouts.extend(first.layout(x, y, first_width, height));
                        layouts.extend(second.layout(x + first_width, y, second_width, height));
                    }
                    SplitDirection::Vertical => {
                        let first_height = ((height as f32) * ratio) as u32;
                        let second_height = height - first_height;

                        layouts.extend(first.layout(x, y, width, first_height));
                        layouts.extend(second.layout(x, y + first_height, width, second_height));
                    }
                }

                layouts
            }
        }
    }

    /// Find pane in a given direction from the current pane
    pub fn find_neighbor(
        &self,
        from: PaneId,
        direction: Direction,
        layouts: &[PaneLayout],
    ) -> Option<PaneId> {
        let from_layout = layouts.iter().find(|l| l.id == from)?;

        // Find the pane that's closest in the given direction
        let candidates: Vec<_> = layouts
            .iter()
            .filter(|l| l.id != from)
            .filter(|l| match direction {
                Direction::Up => l.y + l.height <= from_layout.y,
                Direction::Down => l.y >= from_layout.y + from_layout.height,
                Direction::Left => l.x + l.width <= from_layout.x,
                Direction::Right => l.x >= from_layout.x + from_layout.width,
            })
            .collect();

        if candidates.is_empty() {
            return None;
        }

        // Find the one with the most overlap on the perpendicular axis
        let best = candidates
            .into_iter()
            .max_by_key(|l| {
                let overlap = match direction {
                    Direction::Up | Direction::Down => {
                        let start = l.x.max(from_layout.x);
                        let end = (l.x + l.width).min(from_layout.x + from_layout.width);
                        end.saturating_sub(start)
                    }
                    Direction::Left | Direction::Right => {
                        let start = l.y.max(from_layout.y);
                        let end = (l.y + l.height).min(from_layout.y + from_layout.height);
                        end.saturating_sub(start)
                    }
                };
                overlap
            })?;

        Some(best.id)
    }

    /// Adjust the split ratio at a given pane's parent
    pub fn resize_pane(&mut self, id: PaneId, delta: f32) -> bool {
        match self {
            SplitNode::Leaf(_) => false,
            SplitNode::Split {
                ratio,
                first,
                second,
                ..
            } => {
                // Check if this split contains the pane as a direct child
                let in_first = matches!(first.as_ref(), SplitNode::Leaf(leaf_id) if *leaf_id == id)
                    || first.find(id).is_some();
                let in_second = matches!(second.as_ref(), SplitNode::Leaf(leaf_id) if *leaf_id == id)
                    || second.find(id).is_some();

                if in_first && !in_second {
                    // Pane is in first child, adjust ratio
                    *ratio = (*ratio + delta).clamp(0.1, 0.9);
                    true
                } else if in_second && !in_first {
                    // Pane is in second child, adjust ratio inversely
                    *ratio = (*ratio - delta).clamp(0.1, 0.9);
                    true
                } else if in_first {
                    first.resize_pane(id, delta)
                } else if in_second {
                    second.resize_pane(id, delta)
                } else {
                    false
                }
            }
        }
    }
}

/// Direction for focus navigation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Layout information for a pane
#[derive(Debug, Clone)]
pub struct PaneLayout {
    pub id: PaneId,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_horizontal() {
        let mut tree = SplitNode::leaf(PaneId(1));
        tree.split(SplitDirection::Horizontal, PaneId(2), true);

        let layouts = tree.layout(0, 0, 100, 50);
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[0].width, 50);
        assert_eq!(layouts[1].width, 50);
    }

    #[test]
    fn test_split_vertical() {
        let mut tree = SplitNode::leaf(PaneId(1));
        tree.split(SplitDirection::Vertical, PaneId(2), true);

        let layouts = tree.layout(0, 0, 100, 100);
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[0].height, 50);
        assert_eq!(layouts[1].height, 50);
    }

    #[test]
    fn test_remove_pane() {
        let mut tree = SplitNode::leaf(PaneId(1));
        tree.split(SplitDirection::Horizontal, PaneId(2), true);

        let sibling = tree.remove(PaneId(2));
        assert_eq!(sibling, Some(PaneId(1)));

        // Tree should now be a single leaf
        assert!(matches!(tree, SplitNode::Leaf(PaneId(1))));
    }
}
