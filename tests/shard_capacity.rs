//! The capacity given to a cache is its capacity, whatever the number of shards.
//!
//! oracle: arithmetic on the configuration, not the implementation. The shard capacities have to add
//! up to `capacity`, so a cache filled with far more distinct keys than it holds has exactly
//! `capacity` entries. Rounding every shard up gave up to `capacity + num_shards - 1`.
//!
//! A cache with fewer slots than shards uses fewer shards: a shard with room for nothing would drop
//! every key that hashes to it, while the same key in another shard is kept. Capacity 0 holds nothing.

use alice_cache::{AliceCache, CacheConfig};

const SHARD_COUNTS: [usize; 8] = [1, 2, 4, 8, 16, 32, 64, 128];

fn cache(capacity: usize, num_shards: usize) -> AliceCache<u64, u64> {
    AliceCache::with_config(CacheConfig {
        capacity,
        num_shards,
        num_nodes: 1,
        node_id: 0,
        enable_oracle: false,
    })
}

/// Enough distinct keys that every shard is full, whichever way they hash.
fn fill(c: &AliceCache<u64, u64>, capacity: usize, num_shards: usize) {
    for k in 0..(64 * (capacity + num_shards) as u64) {
        c.put(k, k);
    }
}

fn holds_exactly_its_capacity(capacity: usize) -> usize {
    let mut compared = 0;
    for num_shards in SHARD_COUNTS {
        let c = cache(capacity, num_shards);
        fill(&c, capacity, num_shards);
        assert_eq!(
            c.len(),
            capacity,
            "capacity {capacity}, {num_shards} shards"
        );
        compared += 1;
    }
    compared
}

#[test]
fn a_full_cache_holds_exactly_its_capacity_for_every_size_up_to_64() {
    let mut compared = 0;
    for capacity in 0..=64 {
        compared += holds_exactly_its_capacity(capacity);
    }
    assert_eq!(compared, 65 * SHARD_COUNTS.len());
}

#[test]
fn a_full_cache_holds_exactly_its_capacity_for_larger_sizes() {
    let mut compared = 0;
    for capacity in [
        65, 100, 127, 128, 129, 255, 257, 1000, 1023, 1024, 1025, 4095,
    ] {
        compared += holds_exactly_its_capacity(capacity);
    }
    assert_eq!(compared, 12 * SHARD_COUNTS.len());
}

#[test]
fn the_shard_count_is_the_configured_one_unless_the_capacity_is_smaller() {
    // oracle: the largest power of two that is at most both, and 1 for capacity 0
    let table = [
        (0, 8, 1),
        (1, 8, 1),
        (2, 8, 2),
        (3, 8, 2),
        (4, 8, 4),
        (7, 8, 4),
        (8, 8, 8),
        (9, 8, 8),
        (100, 4, 4),
        (5, 1, 1),
    ];
    for (capacity, num_shards, expected) in table {
        assert_eq!(
            cache(capacity, num_shards).num_shards(),
            expected,
            "{capacity}, {num_shards}"
        );
    }
}

#[test]
fn every_key_can_be_held_when_there_are_fewer_slots_than_shards() {
    for capacity in 1..=8 {
        for num_shards in SHARD_COUNTS {
            for k in 0..256 {
                let c = cache(capacity, num_shards);
                c.put(k, k + 1);
                assert_eq!(
                    c.get(&k),
                    Some(k + 1),
                    "capacity {capacity}, {num_shards} shards, key {k}"
                );
            }
        }
    }
}

#[test]
fn capacity_zero_holds_nothing_and_answers_every_call() {
    for num_shards in SHARD_COUNTS {
        let c = cache(0, num_shards);
        for k in 0..64 {
            c.put(k, k);
            assert_eq!(c.get(&k), None);
            assert!(!c.contains(&k));
            assert_eq!(c.remove(&k), None);
        }
        assert_eq!(c.len(), 0);
        assert!(c.is_empty());
        assert_eq!(c.capacity(), 0);
    }
    let by_size: AliceCache<u64, u64> = AliceCache::new(0);
    by_size.put(1, 1);
    assert_eq!(by_size.get(&1), None);
}

#[test]
fn a_replaced_key_does_not_evict_at_capacity() {
    for num_shards in SHARD_COUNTS {
        let c = cache(1, num_shards);
        c.put(7, 1);
        c.put(7, 2);
        assert_eq!(c.get(&7), Some(2));
        assert_eq!(c.len(), 1);
    }
}
