//! `AliceCache` "Inferno" (Optimized)
//!
//! **Architecture**:
//! - **Storage**: Sharded Flat `HashMap` (No Linked Lists, No Pointers)
//! - **Eviction**: Random Sampled `TinyLFU` (Redis-style, O(1))
//! - **Prediction**: Markov Oracle for prefetching
//!
//! Zero allocation on hot paths. Cache locality maximized.

use alloc::vec::Vec;
use core::hash::Hash;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::jump_hash::jump_hash;
use crate::oracle::SharedOracle;
use crate::shard::CacheShard;

extern crate alloc;

/// Default number of shards (must be power of 2)
const DEFAULT_SHARDS: usize = 256;

/// Cache statistics (lock-free)
///
/// Each counter is placed in its own 64-byte cache line to eliminate false
/// sharing when multiple threads update different counters concurrently.
/// Without padding, `hits`, `misses`, `inserts`, and `evictions` would share
/// one or two cache lines, causing unnecessary cache-coherence traffic.
///
/// `AtomicU64` is 8 bytes, so 56 bytes of padding fill the rest of the line.
#[repr(C)]
#[derive(Default)]
pub struct CacheStats {
    pub hits: PaddedAtomicU64,
    pub misses: PaddedAtomicU64,
    pub inserts: PaddedAtomicU64,
    pub evictions: PaddedAtomicU64,
}

/// An `AtomicU64` padded to a full 64-byte cache line.
///
/// Prevents false sharing between adjacent counters on different CPU cores.
#[repr(C, align(64))]
pub struct PaddedAtomicU64 {
    pub value: AtomicU64,
    _pad: [u8; 56], // 64 - size_of::<AtomicU64>() == 64 - 8 == 56
}

impl PaddedAtomicU64 {
    #[must_use]
    pub const fn new(v: u64) -> Self {
        Self {
            value: AtomicU64::new(v),
            _pad: [0u8; 56],
        }
    }
}

impl Default for PaddedAtomicU64 {
    fn default() -> Self {
        Self::new(0)
    }
}

impl core::ops::Deref for PaddedAtomicU64 {
    type Target = AtomicU64;
    #[inline(always)]
    fn deref(&self) -> &AtomicU64 {
        &self.value
    }
}

impl core::ops::DerefMut for PaddedAtomicU64 {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut AtomicU64 {
        &mut self.value
    }
}

impl CacheStats {
    /// Hit rate (0.0 to 1.0)
    #[inline(always)]
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            let inv_total = 1.0 / total as f64;
            hits as f64 * inv_total
        }
    }

    /// Reset all counters
    pub fn reset(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.inserts.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
    }
}

/// Cache configuration
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Total capacity across all shards
    pub capacity: usize,
    /// Number of shards (must be power of 2)
    pub num_shards: usize,
    /// Number of distributed nodes (for jump hash)
    pub num_nodes: i32,
    /// This node's ID
    pub node_id: u32,
    /// Enable Markov oracle for prediction
    pub enable_oracle: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            capacity: 10000,
            num_shards: DEFAULT_SHARDS,
            num_nodes: 1,
            node_id: 0,
            enable_oracle: true,
        }
    }
}

/// High-performance Sharded Cache with Predictive Oracle
///
/// Features:
/// - 256 shards with `parking_lot` Mutex (minimal contention)
/// - Sampled eviction (Redis-style, O(1))
/// - Markov oracle for access prediction
/// - Jump consistent hash for distributed routing
pub struct AliceCache<K, V> {
    /// Shards (power of 2 for fast modulo)
    shards: Vec<CacheShard<K, V>>,
    /// Shard mask (`num_shards` - 1) for fast modulo
    shard_mask: usize,
    /// Hash builder
    hash_builder: ahash::RandomState,
    /// Statistics (lock-free atomics)
    stats: CacheStats,
    /// Predictive oracle
    oracle: Option<SharedOracle>,
    /// Configuration
    config: CacheConfig,
}

impl<K, V> AliceCache<K, V>
where
    K: Eq + Hash + Clone + Send + Sync,
    V: Clone + Send + Sync,
{
    /// Create new cache with given capacity
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self::with_config(CacheConfig {
            capacity,
            ..Default::default()
        })
    }

    /// Create new cache with custom configuration
    ///
    /// # Panics
    ///
    /// Panics if `num_shards` is not a power of two.
    #[must_use]
    pub fn with_config(config: CacheConfig) -> Self {
        assert!(
            config.num_shards.is_power_of_two(),
            "num_shards must be power of 2"
        );

        let shard_cap = config.capacity.div_ceil(config.num_shards);

        let mut shards = Vec::with_capacity(config.num_shards);
        for _ in 0..config.num_shards {
            shards.push(CacheShard::new(shard_cap));
        }

        let oracle = if config.enable_oracle {
            Some(SharedOracle::new())
        } else {
            None
        };

        Self {
            shards,
            shard_mask: config.num_shards - 1,
            hash_builder: ahash::RandomState::new(),
            stats: CacheStats::default(),
            oracle,
            config,
        }
    }

    /// Get value by key
    ///
    /// Returns None if not found. Updates frequency on access.
    #[inline(always)]
    pub fn get(&self, key: &K) -> Option<V> {
        let (shard_idx, hash) = self.get_shard_idx(key);
        let shard = &self.shards[shard_idx];

        if let Some(val) = shard.get(key, hash) {
            self.stats.hits.fetch_add(1, Ordering::Relaxed);

            // Update oracle
            if let Some(ref oracle) = self.oracle {
                oracle.record(hash);
            }

            Some(val)
        } else {
            self.stats.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Insert key-value pair
    ///
    /// May trigger eviction if shard is at capacity.
    #[inline(always)]
    pub fn put(&self, key: K, value: V) {
        let (shard_idx, hash) = self.get_shard_idx(&key);
        self.shards[shard_idx].put(key, value, hash);
        self.stats.inserts.fetch_add(1, Ordering::Relaxed);

        // Update oracle
        if let Some(ref oracle) = self.oracle {
            oracle.record(hash);
        }
    }

    /// Remove key from cache
    #[inline(always)]
    pub fn remove(&self, key: &K) -> Option<V> {
        let (shard_idx, _) = self.get_shard_idx(key);
        self.shards[shard_idx].remove(key)
    }

    /// Check if key exists
    #[inline(always)]
    #[must_use]
    pub fn contains(&self, key: &K) -> bool {
        let (shard_idx, _) = self.get_shard_idx(key);
        self.shards[shard_idx].contains(key)
    }

    /// Check if oracle recommends prefetching candidate after current
    #[inline(always)]
    #[must_use]
    pub fn should_prefetch(&self, current: &K, candidate: &K) -> bool {
        if let Some(ref oracle) = self.oracle {
            let current_hash = self.hash_key(current);
            let candidate_hash = self.hash_key(candidate);
            oracle.should_prefetch(current_hash, candidate_hash)
        } else {
            false
        }
    }

    /// Get which distributed node owns a key (Jump Consistent Hash)
    #[inline(always)]
    #[must_use]
    pub fn owner_node(&self, key: &K) -> u32 {
        let hash = self.hash_key(key);
        jump_hash(hash, self.config.num_nodes) as u32
    }

    /// Check if this node owns the key
    #[inline(always)]
    #[must_use]
    pub fn is_local_owner(&self, key: &K) -> bool {
        self.owner_node(key) == self.config.node_id
    }

    /// Current number of items (sum of all shards)
    #[must_use]
    pub fn len(&self) -> usize {
        self.shards.iter().map(super::shard::CacheShard::len).sum()
    }

    /// Check if cache is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.shards.iter().all(super::shard::CacheShard::is_empty)
    }

    /// Total capacity
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.config.capacity
    }

    /// Get statistics reference
    #[must_use]
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Hit rate (0.0 to 1.0)
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        self.stats.hit_rate()
    }

    /// Clear all shards
    pub fn clear(&mut self) {
        for shard in &mut self.shards {
            shard.clear();
        }
        self.stats.reset();
        if let Some(ref oracle) = self.oracle {
            oracle.reset();
        }
    }

    /// Get shard index and hash for a key
    #[inline(always)]
    fn get_shard_idx(&self, key: &K) -> (usize, u64) {
        let hash = self.hash_key(key);
        // Fast modulo using bitmask
        ((hash as usize) & self.shard_mask, hash)
    }

    /// Hash a key
    #[inline(always)]
    fn hash_key(&self, key: &K) -> u64 {
        self.hash_builder.hash_one(key)
    }
}

// Safety: `AliceCache` is thread-safe because:
// - Each `CacheShard` is protected by `parking_lot::Mutex`, ensuring exclusive
//   mutable access to the inner `HashMap` + `Vec<Entry>` per shard.
// - `CacheStats` uses `AtomicU64` (no shared mutable state without atomics).
// - `SharedOracle` uses only `AtomicU8`/`AtomicU64` (lock-free, no mutex needed).
// - The `ahash::RandomState` hash builder is read-only after construction.
// All fields are either `Send + Sync` by themselves or guarded by synchronization
// primitives, so `AliceCache<K, V>` is safe to share across threads provided
// `K: Send + Sync` and `V: Send + Sync`.
unsafe impl<K: Send + Sync, V: Send + Sync> Send for AliceCache<K, V> {}
unsafe impl<K: Send + Sync, V: Send + Sync> Sync for AliceCache<K, V> {}

/// Type alias for standard configuration
pub type StandardCache<K, V> = AliceCache<K, V>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic() {
        let cache = AliceCache::<u32, String>::new(100);

        cache.put(1, "one".to_string());
        cache.put(2, "two".to_string());
        cache.put(3, "three".to_string());

        assert_eq!(cache.get(&1), Some("one".to_string()));
        assert_eq!(cache.get(&2), Some("two".to_string()));
        assert_eq!(cache.get(&3), Some("three".to_string()));
        assert_eq!(cache.get(&4), None);
    }

    #[test]
    fn test_cache_stats() {
        let cache = AliceCache::<u32, u32>::new(100);

        cache.put(1, 10);

        // Hit
        cache.get(&1);
        // Miss
        cache.get(&2);

        assert_eq!(cache.stats().hits.load(Ordering::Relaxed), 1);
        assert_eq!(cache.stats().misses.load(Ordering::Relaxed), 1);
        assert!((cache.hit_rate() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_cache_eviction() {
        let config = CacheConfig {
            capacity: 100,
            num_shards: 4, // Fewer shards for predictable testing
            ..Default::default()
        };
        let cache = AliceCache::<u32, u32>::with_config(config);

        // Insert more than capacity
        for i in 0..200 {
            cache.put(i, i * 10);
        }

        // Should have evicted some items
        assert!(cache.len() <= 100);
    }

    #[test]
    fn test_cache_oracle_prediction() {
        let cache = AliceCache::<u32, u32>::new(100);

        // Train pattern: 1 -> 2 -> 3 -> 1 -> 2 -> 3 ...
        for _ in 0..50 {
            cache.put(1, 10);
            cache.get(&1);
            cache.put(2, 20);
            cache.get(&2);
            cache.put(3, 30);
            cache.get(&3);
        }

        // Oracle should learn the pattern
        assert!(cache.should_prefetch(&1, &2));
        assert!(cache.should_prefetch(&2, &3));
    }

    #[test]
    fn test_cache_remove() {
        let cache = AliceCache::<u32, String>::new(100);

        cache.put(1, "one".to_string());
        assert!(cache.contains(&1));

        let removed = cache.remove(&1);
        assert_eq!(removed, Some("one".to_string()));
        assert!(!cache.contains(&1));
    }

    #[test]
    fn test_cache_distributed() {
        let config = CacheConfig {
            capacity: 100,
            num_nodes: 10,
            node_id: 3,
            ..Default::default()
        };
        let cache = AliceCache::<u64, u64>::with_config(config);

        // Verify consistent hashing
        let owner1 = cache.owner_node(&12345);
        let owner2 = cache.owner_node(&12345);
        assert_eq!(owner1, owner2);

        // Check local ownership
        let mut local_count = 0;
        for key in 0..1000u64 {
            if cache.is_local_owner(&key) {
                local_count += 1;
            }
        }
        // Should own ~10% of keys
        assert!(local_count > 50 && local_count < 200);
    }

    #[test]
    fn test_cache_concurrent_safe() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(AliceCache::<u64, u64>::new(1000));
        let mut handles = vec![];

        // Spawn multiple threads
        for t in 0..4 {
            let cache = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                for i in 0..1000 {
                    let key = t * 1000 + i;
                    cache.put(key, key * 10);
                    cache.get(&key);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Should have some items
        assert!(!cache.is_empty());
    }

    // ── Additional tests for quality improvement ──────────────────

    #[test]
    fn test_padded_atomic_u64_alignment() {
        // Verify that PaddedAtomicU64 is exactly 64 bytes (one cache line)
        assert_eq!(core::mem::size_of::<PaddedAtomicU64>(), 64);
        assert_eq!(core::mem::align_of::<PaddedAtomicU64>(), 64);
    }

    #[test]
    fn test_cache_stats_all_misses() {
        let cache = AliceCache::<u32, u32>::new(100);

        // All misses
        for i in 0..10 {
            cache.get(&i);
        }

        assert_eq!(cache.stats().hits.load(Ordering::Relaxed), 0);
        assert_eq!(cache.stats().misses.load(Ordering::Relaxed), 10);
        assert!((cache.hit_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cache_stats_all_hits() {
        let cache = AliceCache::<u32, u32>::new(100);

        // Insert first, then all hits
        for i in 0..10 {
            cache.put(i, i * 10);
        }
        for i in 0..10 {
            assert!(cache.get(&i).is_some());
        }

        assert_eq!(cache.stats().hits.load(Ordering::Relaxed), 10);
        // Misses should be 0 (only the get() calls count, not put())
        assert_eq!(cache.stats().misses.load(Ordering::Relaxed), 0);
        assert!((cache.hit_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cache_stats_no_operations() {
        let cache = AliceCache::<u32, u32>::new(100);

        // No operations - hit rate should be 0.0 (not NaN or panic)
        assert!((cache.hit_rate() - 0.0).abs() < f64::EPSILON);
        assert!(!cache.hit_rate().is_nan());
    }

    #[test]
    fn test_cache_stats_reset() {
        let cache = AliceCache::<u32, u32>::new(100);

        cache.put(1, 10);
        cache.get(&1);
        cache.get(&2);

        assert!(cache.stats().hits.load(Ordering::Relaxed) > 0);

        cache.stats().reset();

        assert_eq!(cache.stats().hits.load(Ordering::Relaxed), 0);
        assert_eq!(cache.stats().misses.load(Ordering::Relaxed), 0);
        assert_eq!(cache.stats().inserts.load(Ordering::Relaxed), 0);
        assert_eq!(cache.stats().evictions.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_cache_new_is_empty() {
        let cache = AliceCache::<u32, u32>::new(100);

        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.capacity(), 100);
    }

    #[test]
    fn test_cache_contains() {
        let cache = AliceCache::<u32, u32>::new(100);

        assert!(!cache.contains(&1));
        cache.put(1, 10);
        assert!(cache.contains(&1));
        cache.remove(&1);
        assert!(!cache.contains(&1));
    }

    #[test]
    fn test_cache_put_updates_existing() {
        let cache = AliceCache::<u32, String>::new(100);

        cache.put(1, "first".to_string());
        assert_eq!(cache.get(&1), Some("first".to_string()));

        cache.put(1, "second".to_string());
        assert_eq!(cache.get(&1), Some("second".to_string()));

        // Length should still be 1 (update, not new insert)
        // Note: len counts across all shards, the key is in exactly one shard
        // After update, the item count should not increase
        let len_after = cache.len();
        cache.put(1, "third".to_string());
        assert_eq!(cache.len(), len_after);
    }

    #[test]
    fn test_cache_remove_nonexistent() {
        let cache = AliceCache::<u32, u32>::new(100);

        assert_eq!(cache.remove(&999), None);
    }

    #[test]
    fn test_cache_without_oracle() {
        let config = CacheConfig {
            capacity: 100,
            enable_oracle: false,
            ..Default::default()
        };
        let cache = AliceCache::<u32, u32>::with_config(config);

        cache.put(1, 10);
        assert_eq!(cache.get(&1), Some(10));

        // should_prefetch should always return false when oracle is disabled
        assert!(!cache.should_prefetch(&1, &2));
    }

    #[test]
    #[should_panic(expected = "num_shards must be power of 2")]
    fn test_cache_non_power_of_two_shards_panics() {
        let config = CacheConfig {
            capacity: 100,
            num_shards: 3, // not a power of 2
            ..Default::default()
        };
        let _cache = AliceCache::<u32, u32>::with_config(config);
    }

    #[test]
    fn test_cache_is_local_owner_single_node() {
        let config = CacheConfig {
            capacity: 100,
            num_nodes: 1,
            node_id: 0,
            ..Default::default()
        };
        let cache = AliceCache::<u64, u64>::with_config(config);

        // With only 1 node, all keys should be locally owned
        for key in 0..100u64 {
            assert!(cache.is_local_owner(&key));
        }
    }

    #[test]
    fn test_cache_string_keys() {
        let cache = AliceCache::<String, Vec<u8>>::new(1000);

        cache.put("hello".to_string(), vec![1, 2, 3]);
        cache.put("world".to_string(), vec![4, 5, 6]);

        assert_eq!(cache.get(&"hello".to_string()), Some(vec![1, 2, 3]));
        assert_eq!(cache.get(&"world".to_string()), Some(vec![4, 5, 6]));
        assert_eq!(cache.get(&"missing".to_string()), None);
    }

    #[test]
    fn test_cache_large_capacity() {
        let cache = AliceCache::<u64, u64>::new(100_000);

        // Insert many items
        for i in 0..10_000u64 {
            cache.put(i, i);
        }

        // Verify some items are retrievable
        let mut found = 0;
        for i in 0..10_000u64 {
            if cache.get(&i).is_some() {
                found += 1;
            }
        }
        // All should be present since we're well under capacity
        assert_eq!(found, 10_000);
    }

    #[test]
    fn test_cache_config_default() {
        let config = CacheConfig::default();

        assert_eq!(config.capacity, 10000);
        assert_eq!(config.num_shards, DEFAULT_SHARDS);
        assert_eq!(config.num_nodes, 1);
        assert_eq!(config.node_id, 0);
        assert!(config.enable_oracle);
    }
}
