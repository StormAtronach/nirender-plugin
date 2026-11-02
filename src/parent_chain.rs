//! The walk up the parent chain of a scene node, for the readers that test each node above a
//! shape (`instance_stream`, `link_client`). The walk goes to the root. Its cap guards against
//! a corrupt chain (a cycle, a freed node) and no scene reaches it. A walk that ends at the cap
//! says so, so that the reader treats the shape as unknown and not as clear.

use core::sync::atomic::{AtomicBool, Ordering};

/// The most nodes a walk follows. \[I: not read in the engine\] The engine's own recursion down a
/// scene (`NiNode::Display` calls `CullShow` for each child) is far shallower, so a chain this long is not a scene.
pub const GUARD: usize = 4096;

/// The chain has more than the allowed nodes and goes on: no node above the shape was left
/// unread on purpose, and the answer for the nodes beyond is not known.
#[derive(Debug, PartialEq, Eq)]
pub struct ChainTooLong;

/// Reads the nodes from `first` up to the root and returns what the first node for which
/// `answer` gives [`Some`] gave. [`Ok`] of [`None`] means every node was read and none answered.
/// `parent_of` gives the parent of a node, 0 at the root. `limit` is the most nodes to read.
pub fn first_answer<T>(first: usize, limit: usize, parent_of: impl Fn(usize) -> usize, mut answer: impl FnMut(usize) -> Option<T>) -> Result<Option<T>, ChainTooLong> {
    let mut node = first;
    for _ in 0..limit {
        if node == 0 {
            return Ok(None);
        }
        if let Some(found) = answer(node) {
            return Ok(Some(found));
        }
        node = parent_of(node);
    }
    if node == 0 {
        Ok(None)
    } else {
        Err(ChainTooLong)
    }
}

/// Logs the first chain that ended at the cap. `reader` names who walked it, and what it did.
pub fn note_too_long(reader: &str) {
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if !LOGGED.swap(true, Ordering::Relaxed) {
        crate::log!("parent chain: {reader} met more than {GUARD} nodes above a shape (the first; the table's reject test counts the others in the instances report)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nodes 1 to `length`: the parent of each is the next, and the last has none.
    fn chain_of(length: usize) -> impl Fn(usize) -> usize {
        move |node| if node < length { node + 1 } else { 0 }
    }

    /// A kind that the reader rejects, at node 35 of 40: the node that the old cap of 32 never read.
    #[test]
    fn a_node_above_the_old_cap_is_read() {
        let found = first_answer(1, GUARD, chain_of(40), |node| (node == 35).then_some("sort adjust node"));
        assert_eq!(found, Ok(Some("sort adjust node")));
    }

    /// The null: the same chain read with the old cap does not answer clear any more. It says that the
    /// chain was cut, which a reader must not take for "nothing above".
    #[test]
    fn a_cut_chain_is_not_a_clear_chain() {
        let found = first_answer(1, 32, chain_of(40), |node| (node == 35).then_some("sort adjust node"));
        assert_eq!(found, Err(ChainTooLong));
    }

    #[test]
    fn a_chain_of_exactly_the_limit_is_read_whole() {
        let mut seen = 0;
        let found: Result<Option<()>, _> = first_answer(1, 40, chain_of(40), |_| {
            seen += 1;
            None
        });
        assert_eq!(found, Ok(None));
        assert_eq!(seen, 40);
    }

    #[test]
    fn a_shape_with_no_parent_is_clear() {
        assert_eq!(first_answer::<()>(0, GUARD, chain_of(40), |_| Some(())), Ok(None));
    }

    /// A node that is its own parent must end the walk, not hang it.
    #[test]
    fn a_cycle_ends_at_the_guard() {
        let mut seen = 0;
        let found: Result<Option<()>, _> = first_answer(7, GUARD, |node| node, |_| {
            seen += 1;
            None
        });
        assert_eq!(found, Err(ChainTooLong));
        assert_eq!(seen, GUARD);
    }

    /// The first node that answers wins: the nearest to the shape.
    #[test]
    fn the_nearest_answer_wins() {
        let found = first_answer(1, GUARD, chain_of(40), |node| (node >= 3).then_some(node));
        assert_eq!(found, Ok(Some(3)));
    }
}
