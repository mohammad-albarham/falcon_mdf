//! Cycle-safe traversal of block link chains.
//!
//! MF4 structures files as singly-linked chains of blocks — data groups, channel
//! groups, channels, data lists — each block naming the next by file offset. The
//! format does not forbid a link that points backwards, so a corrupt or hostile
//! file can close a chain into a loop.
//!
//! Walking such a chain naively never terminates, and because each step usually
//! allocates, it exhausts memory rather than merely spinning. [`LinkChain`]
//! records the offsets already visited and turns the second visit into an error.

use crate::error::{Mf4Error, Result};
use std::collections::HashSet;

/// Tracks the offsets seen while following one chain of blocks.
///
/// Create one per chain, then call [`LinkChain::visit`] with each offset before
/// parsing the block there.
#[derive(Debug, Default)]
pub struct LinkChain {
    seen: HashSet<u64>,
}

impl LinkChain {
    /// Starts a new chain walk.
    pub fn new() -> Self {
        Self {
            seen: HashSet::new(),
        }
    }

    /// Records a visit to `offset`, failing if the chain has been here before.
    ///
    /// `chain` names the link being followed, e.g. `"dg_next"`, so the error
    /// says which structure was malformed.
    pub fn visit(&mut self, offset: u64, chain: &str) -> Result<()> {
        if !self.seen.insert(offset) {
            return Err(Mf4Error::CyclicLink {
                chain: chain.to_string(),
                offset,
            });
        }
        Ok(())
    }

    /// Returns how many blocks have been visited.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Returns true if nothing has been visited yet.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

/// Maximum nesting depth for composition (structure and array) channels.
///
/// Compositions nest legitimately — a structure inside a structure — but only a
/// handful of levels deep in practice. A bound stops a composition that
/// references an ancestor from recursing until the stack overflows, which a
/// visited-set alone cannot prevent across sibling branches.
pub const MAX_COMPOSITION_DEPTH: usize = 32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_each_offset_once() {
        let mut chain = LinkChain::new();
        assert!(chain.visit(100, "dg_next").is_ok());
        assert!(chain.visit(200, "dg_next").is_ok());
        assert!(chain.visit(300, "dg_next").is_ok());
        assert_eq!(chain.len(), 3);
    }

    #[test]
    fn rejects_a_self_referential_link() {
        let mut chain = LinkChain::new();
        assert!(chain.visit(100, "dg_next").is_ok());
        let err = chain.visit(100, "dg_next").unwrap_err();
        assert!(matches!(err, Mf4Error::CyclicLink { .. }));
    }

    #[test]
    fn rejects_a_longer_loop() {
        // A -> B -> C -> A is just as fatal as A -> A.
        let mut chain = LinkChain::new();
        for offset in [10, 20, 30] {
            chain.visit(offset, "cg_next").unwrap();
        }
        assert!(chain.visit(10, "cg_next").is_err());
    }

    #[test]
    fn names_the_chain_in_the_error() {
        let mut chain = LinkChain::new();
        chain.visit(0x40, "cn_next").unwrap();
        let msg = chain.visit(0x40, "cn_next").unwrap_err().to_string();
        assert!(msg.contains("cn_next"), "error should name the link: {msg}");
        assert!(msg.contains("40"), "error should give the offset: {msg}");
    }

    #[test]
    fn separate_chains_do_not_interfere() {
        let mut a = LinkChain::new();
        let mut b = LinkChain::new();
        a.visit(100, "dg_next").unwrap();
        assert!(
            b.visit(100, "cg_next").is_ok(),
            "one chain visiting an offset must not block another"
        );
    }

    #[test]
    fn starts_empty() {
        let chain = LinkChain::new();
        assert!(chain.is_empty());
        assert_eq!(chain.len(), 0);
    }
}
