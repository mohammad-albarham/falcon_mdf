//! Block caching infrastructure for efficient MF4 parsing.
//!
//! This module provides caching mechanisms inspired by asammdf's approach,
//! but implemented idiomatically in Rust using `Arc<T>` for shared ownership
//! and `HashMap` for O(1) lookups.
//!
//! ## Design Philosophy
//!
//! MDF4 files frequently reference the same blocks multiple times:
//! - Multiple channels may share the same conversion (CC) block
//! - Many blocks reference the same text (TX/MD) blocks
//! - Source information (SI) blocks are often identical across channels
//!
//! By caching these blocks keyed by their file offset (address), we:
//! 1. Avoid re-parsing the same block multiple times
//! 2. Share memory for identical blocks via `Arc<T>`
//! 3. Enable O(1) lookup for previously parsed blocks

use std::collections::HashMap;
use std::sync::Arc;

use crate::blocks::{CcBlock, SiBlock};
use crate::error::Result;
use crate::io::ByteSource;
use crate::parser;

/// Cache for parsed MDF4 blocks, enabling efficient reuse of shared data.
///
/// This cache stores blocks by their file offset, allowing multiple channels
/// or groups that reference the same block to share the parsed representation.
///
/// # Performance Characteristics
///
/// - Lookup: O(1) average case
/// - Insert: O(1) average case
/// - Memory: Shared via `Arc<T>`, so identical blocks use minimal extra memory
///
/// # Example
///
/// ```ignore
/// let mut cache = BlockCache::new();
///
/// // First lookup parses and caches the block
/// let cc1 = cache.get_or_parse_cc(&source, 0x1000)?;
///
/// // Second lookup returns cached version (no parsing)
/// let cc2 = cache.get_or_parse_cc(&source, 0x1000)?;
///
/// // cc1 and cc2 point to the same Arc<CcBlock>
/// assert!(Arc::ptr_eq(&cc1, &cc2));
/// ```
#[derive(Debug, Default)]
pub struct BlockCache {
    /// Cached conversion (CC) blocks, keyed by file offset.
    cc_cache: HashMap<u64, Arc<CcBlock>>,
    
    /// Cached text blocks, keyed by file offset.
    /// Stores the extracted string directly rather than the TX/MD block.
    text_cache: HashMap<u64, Arc<str>>,
    
    /// Cached source information (SI) blocks, keyed by file offset.
    si_cache: HashMap<u64, Arc<SiBlock>>,
    
    /// Statistics for cache performance monitoring.
    stats: CacheStats,
}

/// Statistics about cache usage for performance monitoring.
#[derive(Debug, Default, Clone, Copy)]
pub struct CacheStats {
    /// Number of CC cache hits.
    pub cc_hits: u64,
    /// Number of CC cache misses (new parses).
    pub cc_misses: u64,
    /// Number of text cache hits.
    pub text_hits: u64,
    /// Number of text cache misses.
    pub text_misses: u64,
    /// Number of SI cache hits.
    pub si_hits: u64,
    /// Number of SI cache misses.
    pub si_misses: u64,
}

impl CacheStats {
    /// Returns the total number of cache hits across all caches.
    pub fn total_hits(&self) -> u64 {
        self.cc_hits + self.text_hits + self.si_hits
    }
    
    /// Returns the total number of cache misses across all caches.
    pub fn total_misses(&self) -> u64 {
        self.cc_misses + self.text_misses + self.si_misses
    }
    
    /// Returns the cache hit ratio (0.0 to 1.0).
    pub fn hit_ratio(&self) -> f64 {
        let total = self.total_hits() + self.total_misses();
        if total == 0 {
            0.0
        } else {
            self.total_hits() as f64 / total as f64
        }
    }
}

impl BlockCache {
    /// Creates a new empty block cache.
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Creates a cache with pre-allocated capacity for expected block counts.
    ///
    /// Use this when you know approximately how many unique blocks to expect,
    /// to avoid HashMap reallocations during parsing.
    pub fn with_capacity(cc_count: usize, text_count: usize, si_count: usize) -> Self {
        BlockCache {
            cc_cache: HashMap::with_capacity(cc_count),
            text_cache: HashMap::with_capacity(text_count),
            si_cache: HashMap::with_capacity(si_count),
            stats: CacheStats::default(),
        }
    }
    
    /// Gets or parses a conversion (CC) block at the given offset.
    ///
    /// Returns `None` if the offset is 0 (null link).
    /// Returns a cached `Arc<CcBlock>` if previously parsed.
    /// Otherwise, parses the block, caches it, and returns it.
    pub fn get_or_parse_cc<S: ByteSource>(
        &mut self,
        source: &S,
        offset: u64,
    ) -> Result<Option<Arc<CcBlock>>> {
        if offset == 0 {
            return Ok(None);
        }
        
        if let Some(cached) = self.cc_cache.get(&offset) {
            self.stats.cc_hits += 1;
            return Ok(Some(Arc::clone(cached)));
        }
        
        self.stats.cc_misses += 1;
        let cc_block = parser::parse_cc_block(source, offset)?;
        let arc = Arc::new(cc_block);
        self.cc_cache.insert(offset, Arc::clone(&arc));
        Ok(Some(arc))
    }
    
    /// Gets or parses a text block at the given offset.
    ///
    /// Returns an empty string if the offset is 0 (null link).
    /// Returns a cached `Arc<str>` if previously parsed.
    pub fn get_or_parse_text<S: ByteSource>(
        &mut self,
        source: &S,
        offset: u64,
    ) -> Result<Arc<str>> {
        if offset == 0 {
            return Ok(Arc::from(""));
        }
        
        if let Some(cached) = self.text_cache.get(&offset) {
            self.stats.text_hits += 1;
            return Ok(Arc::clone(cached));
        }
        
        self.stats.text_misses += 1;
        let text = parser::read_text(source, offset)?;
        let arc: Arc<str> = Arc::from(text.as_str());
        self.text_cache.insert(offset, Arc::clone(&arc));
        Ok(arc)
    }
    
    /// Gets or parses a source information (SI) block at the given offset.
    ///
    /// Returns `None` if the offset is 0 (null link).
    pub fn get_or_parse_si<S: ByteSource>(
        &mut self,
        source: &S,
        offset: u64,
    ) -> Result<Option<Arc<SiBlock>>> {
        if offset == 0 {
            return Ok(None);
        }
        
        if let Some(cached) = self.si_cache.get(&offset) {
            self.stats.si_hits += 1;
            return Ok(Some(Arc::clone(cached)));
        }
        
        self.stats.si_misses += 1;
        let si_block = parser::parse_si_block(source, offset)?;
        let arc = Arc::new(si_block);
        self.si_cache.insert(offset, Arc::clone(&arc));
        Ok(Some(arc))
    }
    
    /// Returns cache statistics.
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }
    
    /// Returns the number of cached CC blocks.
    pub fn cc_count(&self) -> usize {
        self.cc_cache.len()
    }
    
    /// Returns the number of cached text strings.
    pub fn text_count(&self) -> usize {
        self.text_cache.len()
    }
    
    /// Returns the number of cached SI blocks.
    pub fn si_count(&self) -> usize {
        self.si_cache.len()
    }
    
    /// Clears all cached blocks.
    ///
    /// This does not free the memory of blocks still referenced elsewhere
    /// (via `Arc`), but removes them from the cache so they won't be
    /// returned on future lookups.
    pub fn clear(&mut self) {
        self.cc_cache.clear();
        self.text_cache.clear();
        self.si_cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_cache_stats_hit_ratio() {
        let stats = CacheStats {
            cc_hits: 8,
            cc_misses: 2,
            text_hits: 0,
            text_misses: 0,
            si_hits: 0,
            si_misses: 0,
        };
        assert!((stats.hit_ratio() - 0.8).abs() < 0.001);
    }
    
    #[test]
    fn test_cache_stats_empty() {
        let stats = CacheStats::default();
        assert_eq!(stats.total_hits(), 0);
        assert_eq!(stats.total_misses(), 0);
        assert_eq!(stats.hit_ratio(), 0.0);
    }
}
