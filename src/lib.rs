//! # ALICE-Cache "Charred" (Optimized)
//!
//! **Predictive Distributed Caching System**
//!
//! > "Don't remember the past. Predict the future."
//!
//! ## Charred Architecture
//!
//! - **256 Shards**: Eliminates lock contention with parking_lot Mutex
//! - **Slab Allocation**: Dense `Vec<Entry>` for true O(1) random sampling
//! - **Sampled Eviction**: Redis-style random sampling, no iteration
//! - **Lock-Free Oracle**: AtomicU8 sketch, zero mutex contention
//! - **Jump Hash**: O(1) distributed key routing
//!
//! ## Performance
//!
//! | Operation | Complexity | Notes |
//! |-----------|------------|-------|
//! | Get | O(1) | HashMap lookup + Vec index |
//! | Put | O(1) | Slab append or swap |
//! | Evict | O(k) | k=5 random Vec probes |
//! | Predict | O(1) | Lock-free atomic sketch |
//!
//! ## Example
//!
//! ```
//! use alice_cache::AliceCache;
//!
//! let cache = AliceCache::<u32, String>::new(10000);
//!
//! // Insert
//! cache.put(1, "hello".to_string());
//!
//! // Retrieve
//! assert_eq!(cache.get(&1), Some("hello".to_string()));
//!
//! // Statistics
//! println!("Hit rate: {:.1}%", cache.hit_rate() * 100.0);
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod cache;
pub mod jump_hash;
pub mod oracle;
pub mod shard;
pub mod sketch;

// Re-exports
pub use cache::{AliceCache, CacheConfig, CacheStats, StandardCache};
pub use jump_hash::{jump_hash, jump_hash_bytes, jump_hash_u128};
pub use oracle::{MarkovOracle, SharedOracle};
pub use shard::CacheShard;
pub use sketch::{CountMinSketch, Sketch1K, Sketch4K, Sketch16K};

/// Version
pub const VERSION: &str = "0.2.0-charred";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_integration() {
        let cache = AliceCache::<&str, i32>::new(100);

        cache.put("a", 1);
        cache.put("b", 2);
        cache.put("c", 3);

        assert_eq!(cache.get(&"a"), Some(1));
        assert_eq!(cache.get(&"b"), Some(2));
        assert_eq!(cache.get(&"d"), None);

        assert!(cache.hit_rate() > 0.0);
    }

    #[test]
    fn test_jump_hash_integration() {
        let config = CacheConfig {
            capacity: 100,
            num_nodes: 5,
            node_id: 2,
            ..Default::default()
        };
        let cache = AliceCache::<u64, u64>::with_config(config);

        // Keys should be distributed
        let mut node_counts = [0u32; 5];
        for key in 0..1000u64 {
            let owner = cache.owner_node(&key);
            node_counts[owner as usize] += 1;
        }

        // Each node should have some keys
        for count in node_counts {
            assert!(count > 100);
            assert!(count < 300);
        }
    }

    #[test]
    fn test_oracle_integration() {
        let cache = AliceCache::<u32, u32>::new(100);

        // Train sequential pattern
        for _ in 0..100 {
            cache.put(1, 10);
            cache.get(&1);
            cache.put(2, 20);
            cache.get(&2);
        }

        // Should predict 2 after 1
        assert!(cache.should_prefetch(&1, &2));
    }

    #[test]
    fn test_sketch_standalone() {
        let mut sketch = Sketch4K::new();

        for _ in 0..100 {
            sketch.add(1);
        }
        for _ in 0..50 {
            sketch.add(2);
        }

        assert!(sketch.estimate(1) > sketch.estimate(2));
        assert!(sketch.estimate(3) == 0);
    }
}
