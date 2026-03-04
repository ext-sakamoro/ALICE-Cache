//! Cache Shard (Charred Edition)
//!
//! **Mechanism**:
//! - **Storage**: Slab Allocation (`Vec<Entry>`) for O(1) random sampling.
//! - **Lookup**: `HashMap<K, u32>` mapping Key to Slab Index.
//! - **Eviction**: True O(1) sampling via Slab index.
//!
//! The map iteration scan is gone. We jump directly to memory offsets.

use alloc::vec::Vec;
use core::hash::Hash;
use hashbrown::HashMap;
use parking_lot::Mutex;

use crate::sketch::CountMinSketch;

extern crate alloc;

/// Number of samples for eviction
const EVICTION_SAMPLES: usize = 5;

/// Entry stored in the dense slab
struct Entry<K, V> {
    key: K,
    value: V,
    hash: u64,
}

/// Per-shard internal state
pub struct CacheShard<K, V> {
    inner: Mutex<ShardInner<K, V>>,
    capacity: usize,
}

struct ShardInner<K, V> {
    /// Dense storage for O(1) sampling [Index -> Entry]
    entries: Vec<Entry<K, V>>,
    /// Mapping [Key -> Index]
    lookup: HashMap<K, u32>,
    /// Frequency sketch
    sketch: CountMinSketch<256, 4>,
    /// RNG state
    rng_state: u64,
    /// Sample counter
    sample_count: u64,
}

impl<K, V> CacheShard<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Create new shard with given capacity
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(ShardInner {
                entries: Vec::with_capacity(capacity),
                lookup: HashMap::with_capacity(capacity),
                sketch: CountMinSketch::new(),
                rng_state: 0xDEAD_BEEF_CAFE_BABEu64,
                sample_count: 0,
            }),
            capacity,
        }
    }

    /// Get value by key (updates frequency)
    #[inline(always)]
    pub fn get(&self, key: &K, hash: u64) -> Option<V> {
        let mut guard = self.inner.lock();
        let inner = &mut *guard;

        // Update frequency
        inner.sketch.add(hash);
        inner.sample_count += 1;
        Self::maybe_age_sketch(inner);

        // O(1) lookup -> O(1) index access
        if let Some(&idx) = inner.lookup.get(key) {
            Some(inner.entries[idx as usize].value.clone())
        } else {
            None
        }
    }

    /// Insert key-value pair (may trigger eviction)
    #[inline(always)]
    pub fn put(&self, key: K, value: V, hash: u64) {
        let mut guard = self.inner.lock();
        let inner = &mut *guard;

        inner.sketch.add(hash);
        inner.sample_count += 1;
        Self::maybe_age_sketch(inner);

        // Update existing
        if let Some(&idx) = inner.lookup.get(&key) {
            inner.entries[idx as usize].value = value;
            return;
        }

        // Insert new - evict if at capacity
        if inner.entries.len() >= self.capacity {
            Self::sampled_evict(inner);
        }

        let idx = inner.entries.len() as u32;
        inner.lookup.insert(key.clone(), idx);
        inner.entries.push(Entry { key, value, hash });
    }

    /// Remove key from shard
    #[inline(always)]
    pub fn remove(&self, key: &K) -> Option<V> {
        let mut guard = self.inner.lock();
        let inner = &mut *guard;

        if let Some(idx) = inner.lookup.remove(key) {
            Some(Self::swap_remove(inner, idx as usize))
        } else {
            None
        }
    }

    /// Check if key exists
    #[inline(always)]
    #[must_use]
    pub fn contains(&self, key: &K) -> bool {
        self.inner.lock().lookup.contains_key(key)
    }

    /// Current number of items
    #[inline(always)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().entries.len()
    }

    /// Returns true if the shard contains no items
    #[inline(always)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().entries.is_empty()
    }

    /// Clear all items
    pub fn clear(&mut self) {
        let mut guard = self.inner.lock();
        guard.entries.clear();
        guard.lookup.clear();
        guard.sketch.clear();
        guard.sample_count = 0;
    }

    /// Swap-remove from Slab to keep it dense (O(1))
    /// Moves the last element into the removed hole
    fn swap_remove(inner: &mut ShardInner<K, V>, idx: usize) -> V {
        let last_idx = inner.entries.len() - 1;

        // Remove the entry from slab
        let entry = inner.entries.swap_remove(idx);

        // If we swapped (removed item wasn't the last one), fix the lookup
        if idx != last_idx && !inner.entries.is_empty() {
            let swapped_key = &inner.entries[idx].key;
            inner.lookup.insert(swapped_key.clone(), idx as u32);
        }

        entry.value
    }

    /// Sampled Eviction (Charred O(1))
    ///
    /// Because `entries` is a dense Vec, we can pick a random index in O(1).
    /// No iteration. No skipping. Pure memory offset.
    #[inline(always)]
    fn sampled_evict(inner: &mut ShardInner<K, V>) {
        if inner.entries.is_empty() {
            return;
        }

        let mut victim_idx = 0;
        let mut min_freq = u8::MAX;
        let len = inner.entries.len();

        // 5 random probes into the slab - pure O(1) each
        for _ in 0..EVICTION_SAMPLES {
            inner.rng_state = xorshift64(inner.rng_state);
            let idx = (inner.rng_state as usize) % len;

            let hash = inner.entries[idx].hash;
            let freq = inner.sketch.estimate(hash);

            if freq < min_freq {
                min_freq = freq;
                victim_idx = idx;
            }
        }

        // Remove victim via swap_remove (O(1)).
        // We remove from the lookup map using a reference to avoid cloning.
        // SAFETY: lookup.remove borrows inner.entries[victim_idx].key as &K,
        // which lives long enough since swap_remove is called afterwards.
        // The borrow ends before swap_remove mutates entries.
        inner.lookup.remove(&inner.entries[victim_idx].key);
        let _ = Self::swap_remove(inner, victim_idx);
    }

    /// Age the sketch periodically to adapt to changing patterns
    #[inline(always)]
    fn maybe_age_sketch(inner: &mut ShardInner<K, V>) {
        if inner.sample_count >= 10000 {
            inner.sketch.halve();
            inner.sample_count = 0;
        }
    }
}

/// Xorshift64 PRNG - fast, good enough for sampling
#[inline(always)]
fn xorshift64(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shard_basic() {
        let shard = CacheShard::<u32, String>::new(10);

        shard.put(1, "one".to_string(), 1);
        shard.put(2, "two".to_string(), 2);

        assert_eq!(shard.get(&1, 1), Some("one".to_string()));
        assert_eq!(shard.get(&2, 2), Some("two".to_string()));
        assert_eq!(shard.get(&3, 3), None);
    }

    #[test]
    fn test_shard_update() {
        let shard = CacheShard::<u32, String>::new(10);

        shard.put(1, "one".to_string(), 1);
        assert_eq!(shard.get(&1, 1), Some("one".to_string()));

        // Update existing key
        shard.put(1, "ONE".to_string(), 1);
        assert_eq!(shard.get(&1, 1), Some("ONE".to_string()));
        assert_eq!(shard.len(), 1); // Still only 1 entry
    }

    #[test]
    fn test_shard_eviction() {
        let shard = CacheShard::<u32, u32>::new(5);

        // Fill to capacity
        for i in 0..5 {
            shard.put(i, i * 10, i as u64);
        }
        assert_eq!(shard.len(), 5);

        // Add more - should trigger eviction
        for i in 5..10 {
            shard.put(i, i * 10, i as u64);
        }

        // Should still be at capacity
        assert!(shard.len() <= 5);
    }

    #[test]
    fn test_shard_frequency_bias() {
        let shard = CacheShard::<u32, u32>::new(5);

        // Access key 1 many times to boost frequency
        shard.put(1, 100, 1);
        for _ in 0..50 {
            shard.get(&1, 1);
        }

        // Fill with other keys
        for i in 2..10 {
            shard.put(i, i * 10, i as u64);
        }

        // Key 1 should likely survive due to high frequency
        let has_key1 = shard.contains(&1);
        println!("High-frequency key 1 survived: {has_key1}");
    }

    #[test]
    fn test_shard_remove() {
        let shard = CacheShard::<u32, String>::new(10);

        shard.put(1, "one".to_string(), 1);
        shard.put(2, "two".to_string(), 2);
        shard.put(3, "three".to_string(), 3);
        assert!(shard.contains(&1));
        assert_eq!(shard.len(), 3);

        let removed = shard.remove(&1);
        assert_eq!(removed, Some("one".to_string()));
        assert!(!shard.contains(&1));
        assert_eq!(shard.len(), 2);

        // Other keys should still be accessible
        assert!(shard.contains(&2));
        assert!(shard.contains(&3));
    }

    #[test]
    fn test_shard_swap_remove_consistency() {
        let shard = CacheShard::<u32, u32>::new(10);

        // Insert several items
        for i in 0..5 {
            shard.put(i, i * 100, i as u64);
        }

        // Remove from middle
        shard.remove(&2);

        // All remaining should be accessible with correct values
        assert_eq!(shard.get(&0, 0), Some(0));
        assert_eq!(shard.get(&1, 1), Some(100));
        assert_eq!(shard.get(&2, 2), None); // removed
        assert_eq!(shard.get(&3, 3), Some(300));
        assert_eq!(shard.get(&4, 4), Some(400));
    }

    // ── Additional tests for quality improvement ──────────────────

    #[test]
    fn test_shard_remove_nonexistent() {
        let shard = CacheShard::<u32, String>::new(10);

        shard.put(1, "one".to_string(), 1);

        // Removing a key that does not exist should return None
        assert_eq!(shard.remove(&999), None);
        assert_eq!(shard.len(), 1); // original item still present
    }

    #[test]
    fn test_shard_remove_all_items() {
        let shard = CacheShard::<u32, u32>::new(10);

        for i in 0..5 {
            shard.put(i, i * 10, i as u64);
        }
        assert_eq!(shard.len(), 5);

        for i in 0..5 {
            shard.remove(&i);
        }
        assert_eq!(shard.len(), 0);
    }

    #[test]
    fn test_shard_remove_first_item() {
        let shard = CacheShard::<u32, u32>::new(10);

        shard.put(10, 100, 10);
        shard.put(20, 200, 20);
        shard.put(30, 300, 30);

        // Remove the first inserted item (index 0) - triggers swap_remove
        let removed = shard.remove(&10);
        assert_eq!(removed, Some(100));
        assert_eq!(shard.len(), 2);

        // Remaining items should still be accessible
        assert!(shard.contains(&20));
        assert!(shard.contains(&30));
        assert_eq!(shard.get(&20, 20), Some(200));
        assert_eq!(shard.get(&30, 30), Some(300));
    }

    #[test]
    fn test_shard_remove_last_item() {
        let shard = CacheShard::<u32, u32>::new(10);

        shard.put(10, 100, 10);
        shard.put(20, 200, 20);
        shard.put(30, 300, 30);

        // Remove the last inserted item (no swap needed)
        let removed = shard.remove(&30);
        assert_eq!(removed, Some(300));
        assert_eq!(shard.len(), 2);

        assert!(shard.contains(&10));
        assert!(shard.contains(&20));
    }

    #[test]
    fn test_shard_empty_operations() {
        let shard = CacheShard::<u32, u32>::new(10);

        assert_eq!(shard.len(), 0);
        assert_eq!(shard.get(&1, 1), None);
        assert!(!shard.contains(&1));
        assert_eq!(shard.remove(&1), None);
    }

    #[test]
    fn test_shard_capacity_one() {
        let shard = CacheShard::<u32, u32>::new(1);

        shard.put(1, 10, 1);
        assert_eq!(shard.len(), 1);

        // Inserting another key should trigger eviction
        shard.put(2, 20, 2);
        assert_eq!(shard.len(), 1);

        // One of the two keys should remain
        let has_1 = shard.contains(&1);
        let has_2 = shard.contains(&2);
        assert!(has_1 || has_2);
    }

    #[test]
    fn test_xorshift64_not_zero() {
        // xorshift64 should never produce 0 from a non-zero seed
        let mut state = 0xDEAD_BEEF_CAFE_BABEu64;
        for _ in 0..1000 {
            state = xorshift64(state);
            assert_ne!(state, 0, "xorshift64 produced 0");
        }
    }

    #[test]
    fn test_shard_sequential_remove_and_reinsert() {
        let shard = CacheShard::<u32, u32>::new(10);

        // Insert items
        for i in 0..5 {
            shard.put(i, i * 10, i as u64);
        }

        // Remove all from the middle outward
        shard.remove(&2);
        shard.remove(&1);
        shard.remove(&3);

        assert_eq!(shard.len(), 2);
        assert!(shard.contains(&0));
        assert!(shard.contains(&4));

        // Re-insert removed keys
        shard.put(1, 111, 1);
        shard.put(2, 222, 2);
        shard.put(3, 333, 3);

        assert_eq!(shard.get(&1, 1), Some(111));
        assert_eq!(shard.get(&2, 2), Some(222));
        assert_eq!(shard.get(&3, 3), Some(333));
        assert_eq!(shard.len(), 5);
    }
}
