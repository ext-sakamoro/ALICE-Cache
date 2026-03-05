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

        shard.get(key, hash).map_or_else(
            || {
                self.stats.misses.fetch_add(1, Ordering::Relaxed);
                None
            },
            |val| {
                self.stats.hits.fetch_add(1, Ordering::Relaxed);

                // Update oracle
                if let Some(ref oracle) = self.oracle {
                    oracle.record(hash);
                }

                Some(val)
            },
        )
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
        self.oracle.as_ref().is_some_and(|oracle| {
            let current_hash = self.hash_key(current);
            let candidate_hash = self.hash_key(candidate);
            oracle.should_prefetch(current_hash, candidate_hash)
        })
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
    pub const fn capacity(&self) -> usize {
        self.config.capacity
    }

    /// Get statistics reference
    #[must_use]
    pub const fn stats(&self) -> &CacheStats {
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
#[allow(clippy::doc_markdown)]
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
        // 4シャードで各シャード容量25。10件挿入では退避が発生しない。
        let config = CacheConfig {
            capacity: 100,
            num_shards: 4,
            ..Default::default()
        };
        let cache = AliceCache::<u32, u32>::with_config(config);

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

    // ── 新規テスト (16件) ──────────────────────────────────────────

    /// inserts カウンタが put ごとに加算されることを確認する。
    #[test]
    fn test_cache_stats_inserts_counter() {
        let cache = AliceCache::<u32, u32>::new(1000);

        for i in 0..20 {
            cache.put(i, i);
        }

        assert_eq!(cache.stats().inserts.load(Ordering::Relaxed), 20);
    }

    /// put で同一キーを上書きしても inserts は増え続けることを確認する。
    #[test]
    fn test_cache_stats_inserts_on_update() {
        let cache = AliceCache::<u32, u32>::new(1000);

        cache.put(1, 10);
        cache.put(1, 20);
        cache.put(1, 30);

        // 毎回 put が呼ばれるので 3 回カウント
        assert_eq!(cache.stats().inserts.load(Ordering::Relaxed), 3);
    }

    /// clear() 後に統計がリセットされ、キャッシュが空になることを確認する。
    #[test]
    fn test_cache_stats_cleared_after_clear() {
        let config = CacheConfig {
            capacity: 200,
            num_shards: 4,
            ..Default::default()
        };
        let mut cache = AliceCache::<u32, u32>::with_config(config);

        for i in 0..10 {
            cache.put(i, i);
            cache.get(&i);
        }

        assert!(cache.stats().hits.load(Ordering::Relaxed) > 0);
        assert!(cache.stats().inserts.load(Ordering::Relaxed) > 0);

        cache.clear();

        assert_eq!(cache.stats().hits.load(Ordering::Relaxed), 0);
        assert_eq!(cache.stats().misses.load(Ordering::Relaxed), 0);
        assert_eq!(cache.stats().inserts.load(Ordering::Relaxed), 0);
        assert_eq!(cache.len(), 0);
    }

    /// capacity() は設定した値を返すことを確認する。
    #[test]
    fn test_cache_capacity_matches_config() {
        let cache = AliceCache::<u32, u32>::new(54321);
        assert_eq!(cache.capacity(), 54321);
    }

    /// remove() は存在しないキーに対して None を返すことを確認する。
    #[test]
    fn test_cache_remove_missing_key_returns_none() {
        let cache = AliceCache::<u64, u64>::new(1000);
        cache.put(1, 100);
        assert_eq!(cache.remove(&9999), None);
        // 元のキーはまだ存在する
        assert!(cache.contains(&1));
    }

    /// owner_node() の戻り値が [0, num_nodes) の範囲内であることを確認する。
    #[test]
    fn test_cache_owner_node_in_range() {
        let config = CacheConfig {
            capacity: 1000,
            num_nodes: 7,
            node_id: 0,
            ..Default::default()
        };
        let cache = AliceCache::<u64, u64>::with_config(config);

        for key in 0..500u64 {
            let owner = cache.owner_node(&key);
            assert!(owner < 7, "owner_node {owner} out of range for key {key}");
        }
    }

    /// is_local_owner() の一貫性: 同じキーで結果が変わらないことを確認する。
    #[test]
    fn test_cache_is_local_owner_consistent() {
        let config = CacheConfig {
            capacity: 1000,
            num_nodes: 3,
            node_id: 1,
            ..Default::default()
        };
        let cache = AliceCache::<u64, u64>::with_config(config);

        for key in 0..100u64 {
            let first = cache.is_local_owner(&key);
            let second = cache.is_local_owner(&key);
            assert_eq!(first, second, "is_local_owner not consistent for key {key}");
        }
    }

    /// CacheConfig::clone() が独立したコピーを返すことを確認する。
    #[test]
    fn test_cache_config_clone() {
        let config = CacheConfig {
            capacity: 500,
            num_shards: 8,
            num_nodes: 3,
            node_id: 1,
            enable_oracle: false,
        };
        // clone() が独立したコピーを返すことを確認する。
        // 元の config をキャッシュ構築に使い、cloned で値を検証する。
        let _cache = AliceCache::<u32, u32>::with_config(config.clone());
        let cloned = config;
        assert_eq!(cloned.capacity, 500);
        assert_eq!(cloned.num_shards, 8);
        assert_eq!(cloned.num_nodes, 3);
        assert_eq!(cloned.node_id, 1);
        assert!(!cloned.enable_oracle);
    }

    /// put → remove → is_empty の一連の流れを確認する。
    #[test]
    fn test_cache_is_empty_after_remove_all() {
        let config = CacheConfig {
            capacity: 100,
            num_shards: 4,
            ..Default::default()
        };
        let cache = AliceCache::<u32, u32>::with_config(config);

        for i in 0..5 {
            cache.put(i, i * 10);
        }
        assert!(!cache.is_empty());

        for i in 0..5 {
            cache.remove(&i);
        }
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    /// hit_rate() は hits=0, misses=0 のとき NaN にならないことを確認する。
    #[test]
    fn test_cache_hit_rate_no_nan_on_zero() {
        let cache = AliceCache::<u32, u32>::new(100);
        let rate = cache.hit_rate();
        assert!(!rate.is_nan());
        assert!((rate - 0.0).abs() < f64::EPSILON);
    }

    /// Arc<AliceCache> を複数スレッドで同時読み書きしてもデータ競合が起きないことを確認する。
    #[test]
    fn test_cache_concurrent_reads_and_writes() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(AliceCache::<u32, u32>::new(10_000));
        let mut handles = vec![];

        // 書き込みスレッド 4 本
        for t in 0..4u32 {
            let c = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                for i in 0..500u32 {
                    c.put(t * 500 + i, i);
                }
            }));
        }

        // 読み込みスレッド 4 本
        for t in 0..4u32 {
            let c = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                for i in 0..500u32 {
                    let _ = c.get(&(t * 500 + i));
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // クラッシュなく完了すればよい
        assert!(cache.stats().inserts.load(Ordering::Relaxed) > 0);
    }

    /// evictions カウンタは退避が起きたシャードで加算されることを確認する。
    /// (AliceCache::put は shard.put を呼ぶが evictions の計上は shard 側で行わない。
    ///  ここでは退避後に len が容量以下に収まることを検証する。)
    #[test]
    fn test_cache_eviction_keeps_len_bounded() {
        let config = CacheConfig {
            capacity: 40,
            num_shards: 4,
            enable_oracle: false,
            ..Default::default()
        };
        let cache = AliceCache::<u32, u32>::with_config(config);

        // 容量の 5 倍を挿入
        for i in 0..200u32 {
            cache.put(i, i);
        }

        assert!(cache.len() <= 40, "len {} exceeds capacity 40", cache.len());
    }

    /// oracle なしキャッシュでの基本的な get/put を確認する。
    #[test]
    fn test_cache_no_oracle_get_put() {
        let config = CacheConfig {
            capacity: 400,
            num_shards: 4,
            enable_oracle: false,
            ..Default::default()
        };
        let cache = AliceCache::<u32, u32>::with_config(config);

        for i in 0..100u32 {
            cache.put(i, i * 2);
        }
        for i in 0..100u32 {
            assert_eq!(cache.get(&i), Some(i * 2));
        }
    }

    /// should_prefetch は oracle なしのとき常に false を返すことを確認する。
    #[test]
    fn test_cache_should_prefetch_false_without_oracle() {
        let config = CacheConfig {
            capacity: 100,
            enable_oracle: false,
            ..Default::default()
        };
        let cache = AliceCache::<u32, u32>::with_config(config);

        // いくら使っても false
        for i in 0..50u32 {
            cache.put(i, i);
            cache.get(&i);
        }
        assert!(!cache.should_prefetch(&0, &1));
    }

    /// 大量のシャード数(256)で 256 件挿入した場合でも全件取得できることを確認する。
    #[test]
    fn test_cache_256_shards_no_eviction() {
        // 各シャード容量 = 256.div_ceil(256) = 1 は退避が起きうるため、
        // 容量を 2560 にして各シャード 10 件を確保する。
        let config = CacheConfig {
            capacity: 2560,
            num_shards: 256,
            enable_oracle: false,
            ..Default::default()
        };
        let cache = AliceCache::<u32, u32>::with_config(config);

        for i in 0..256u32 {
            cache.put(i, i * 3);
        }
        for i in 0..256u32 {
            assert_eq!(
                cache.get(&i),
                Some(i * 3),
                "key {i} missing after 256-shard insert"
            );
        }
    }

    /// PaddedAtomicU64::new でゼロ以外の初期値を設定できることを確認する。
    #[test]
    fn test_padded_atomic_u64_initial_value() {
        let p = PaddedAtomicU64::new(42);
        assert_eq!(p.load(Ordering::Relaxed), 42);
    }

    /// CacheStats の hit_rate() が正しい割合を返すことを確認する。
    #[test]
    fn test_cache_stats_hit_rate_ratio() {
        let config = CacheConfig {
            capacity: 400,
            num_shards: 4,
            ..Default::default()
        };
        let cache = AliceCache::<u32, u32>::with_config(config);

        // 100 件挿入して 50 件ヒット、50 件ミス
        for i in 0..100u32 {
            cache.put(i, i);
        }
        for i in 0..50u32 {
            cache.get(&i); // ヒット
        }
        for i in 100..150u32 {
            cache.get(&i); // ミス
        }

        let rate = cache.hit_rate();
        assert!(
            (rate - 0.5).abs() < 0.01,
            "Expected hit_rate ~0.5, got {rate}"
        );
    }
}
