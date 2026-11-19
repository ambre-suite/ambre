//! The bound of one XML tree a codec builds from a part.
//!
//! The part bound caps the bytes a part inflates to, not what its tree
//! holds. An empty element costs four bytes of XML and some eighty bytes
//! of tree. So a part of 32 MiB of `<a/>` asks for some 640 MiB. Each tree
//! builder of the codecs counts what it builds against one [`TreeBudget`]
//! per part. It refuses the part at the first node or attribute past the
//! bound (`ambre-spec-budgets.md section 6`, the foreign-file tree row).

use ambre_core::{Count, NarrowCount, Octets};

/// The nodes of one tree, counted: an element, or a text node.
pub enum TreeNodes {}

impl NarrowCount for TreeNodes {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("TreeNodeCount");
}

/// The most nodes one tree may hold: 1 Mi. The Word test files hold a node
/// per 28 to 36 bytes of XML. So the largest real body measured, 12 MB,
/// holds about 430 000. At some eighty bytes a node the bound holds about
/// 80 MiB.
pub const MAX_TREE_NODES: Count<TreeNodes> = Count::new(1024 * 1024);

/// The most bytes the attributes of one tree may hold: 32 MiB. Each
/// attribute counts its value and [`ATTR_RECORD`], the record that holds
/// it. So a part of short attributes meets the bound as one of long ones.
pub const MAX_ATTR_BYTES: Count<Octets> = Count::new(32 * 1024 * 1024);

/// The bytes one attribute costs beside its value: its name and its value
/// handles in the tree.
pub const ATTR_RECORD: Count<Octets> = Count::new(32);

/// A part builds a tree past [`MAX_TREE_NODES`] or [`MAX_ATTR_BYTES`]. It
/// quotes nothing of the part: its bytes are the user's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a part builds a tree past its bound")]
pub struct TreeTooLarge;

/// What one tree holds so far. A builder makes one per part, and charges
/// each node and each attribute before it allocates it.
#[derive(Debug, Default)]
pub struct TreeBudget {
    nodes: Count<TreeNodes>,
    attr_bytes: Count<Octets>,
}

impl TreeBudget {
    /// Charge one node: an element, or a text node.
    ///
    /// # Errors
    ///
    /// [`TreeTooLarge`] past [`MAX_TREE_NODES`].
    pub fn node(&mut self) -> Result<(), TreeTooLarge> {
        let nodes = self.nodes.incremented();
        if nodes > MAX_TREE_NODES {
            return Err(TreeTooLarge);
        }
        self.nodes = nodes;
        Ok(())
    }

    /// Charge one attribute whose value is `value` bytes long.
    ///
    /// # Errors
    ///
    /// [`TreeTooLarge`] past [`MAX_ATTR_BYTES`].
    pub fn attr(&mut self, value: Count<Octets>) -> Result<(), TreeTooLarge> {
        let bytes = self.attr_bytes.plus(ATTR_RECORD).plus(value);
        if bytes > MAX_ATTR_BYTES {
            return Err(TreeTooLarge);
        }
        self.attr_bytes = bytes;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_node_past_the_bound_is_refused() {
        let mut budget = TreeBudget::default();
        for _ in 0..MAX_TREE_NODES.get() {
            assert_eq!(budget.node(), Ok(()));
        }
        assert_eq!(budget.node(), Err(TreeTooLarge));
    }

    #[test]
    fn short_attributes_meet_the_bound_by_their_records() {
        let mut budget = TreeBudget::default();
        let records = MAX_ATTR_BYTES.get() / ATTR_RECORD.get();
        for _ in 0..records {
            assert_eq!(budget.attr(Count::ZERO), Ok(()));
        }
        assert_eq!(budget.attr(Count::ZERO), Err(TreeTooLarge));
    }

    #[test]
    fn one_long_value_counts_its_bytes() {
        let mut budget = TreeBudget::default();
        assert_eq!(budget.attr(MAX_ATTR_BYTES), Err(TreeTooLarge));
        assert_eq!(budget.attr(Count::new(1024)), Ok(()));
    }
}
