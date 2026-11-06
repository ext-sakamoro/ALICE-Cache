//! A size given to the cache is a limit, not an allocation.
//!
//! oracle: a cache holds what was put in it, up to its capacity, so 1000 entries fit a cache of any
//! capacity from 1000 up, and take the memory of 1000 entries. `decompress` makes what its data says:
//! `original_size` is a field the caller fills in, and its output does not depend on it. A failed
//! allocation aborts the process, so each row here would end the test run, not fail one test.

#![cfg(target_pointer_width = "64")]

use alice_cache::{decompress, AliceCache, CacheConfig, CacheShard, CompressedEntry};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Far above any row here: without a limit, this is where the test ends, not the machine's memory.
const REFUSE_ABOVE: usize = 1 << 31;

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if LIVE.load(Ordering::Relaxed).saturating_add(l.size()) > REFUSE_ABOVE {
            return std::ptr::null_mut();
        }
        LIVE.fetch_add(l.size(), Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The peak is one counter: the test that reads it runs alone.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// What a shard takes before it holds anything: for a shard of `u64` keys and values, its room for 64
/// entries and its sketch.
const EMPTY_SHARD_BYTES: usize = 5 << 10;

const SIZES: [usize; 4] = [1 << 40, 1 << 50, 1 << 62, usize::MAX];

fn holds_what_was_put(c: &AliceCache<u64, u64>) {
    for k in 0..1000 {
        c.put(k, k + 1);
    }
    assert_eq!(c.len(), 1000);
    for k in 0..1000 {
        assert_eq!(c.get(&k), Some(k + 1));
    }
}

#[test]
fn a_cache_of_any_capacity_takes_the_memory_of_what_it_holds() {
    let _one = ONE_AT_A_TIME.lock().unwrap();
    for capacity in SIZES {
        holds_what_was_put(&AliceCache::new(capacity));
        holds_what_was_put(&AliceCache::with_seed(capacity, 7));
    }
}

#[test]
fn a_shard_count_of_any_size_is_lowered_to_the_limit() {
    let _one = ONE_AT_A_TIME.lock().unwrap();
    for capacity in SIZES {
        for num_shards in [1usize << 40, 1 << 62] {
            let c: AliceCache<u64, u64> = AliceCache::with_config(CacheConfig {
                capacity,
                num_shards,
                num_nodes: 1,
                node_id: 0,
                enable_oracle: false,
            });
            assert_eq!(c.num_shards(), alice_cache::MAX_SHARDS);
            holds_what_was_put(&c);
        }
    }
}

#[test]
fn the_limit_of_shards_is_kept_when_asked_for_exactly() {
    let _one = ONE_AT_A_TIME.lock().unwrap();
    let c: AliceCache<u64, u64> = AliceCache::with_config(CacheConfig {
        capacity: 1 << 40,
        num_shards: alice_cache::MAX_SHARDS,
        num_nodes: 1,
        node_id: 0,
        enable_oracle: false,
    });
    assert_eq!(c.num_shards(), alice_cache::MAX_SHARDS);
}

#[test]
fn the_most_shards_cost_what_their_limit_says() {
    let _one = ONE_AT_A_TIME.lock().unwrap();
    let before = LIVE.load(Ordering::Relaxed);
    let c: AliceCache<u64, u64> = AliceCache::with_config(CacheConfig {
        capacity: 1 << 40,
        num_shards: alice_cache::MAX_SHARDS,
        num_nodes: 1,
        node_id: 0,
        enable_oracle: false,
    });
    let used = LIVE.load(Ordering::Relaxed).wrapping_sub(before);
    println!("{} shards: {used} bytes", c.num_shards());
    assert!(used < EMPTY_SHARD_BYTES * alice_cache::MAX_SHARDS, "{used}");
}

#[test]
fn decompress_makes_what_the_data_says_for_any_original_size() {
    let _one = ONE_AT_A_TIME.lock().unwrap();
    // 0xFF 0x00 is a run: 7 times the byte 9, then the literal 1
    let data = vec![0xFF, 0x00, 9, 0x00, 7, 1, 2];
    for original_size in [0, 3, 1 << 40, 1 << 50, 1 << 62, usize::MAX] {
        let entry = CompressedEntry {
            data: data.clone(),
            original_size,
            compressed: true,
        };
        assert_eq!(
            decompress(&entry),
            vec![9, 9, 9, 9, 9, 9, 9, 1, 2],
            "{original_size}"
        );
    }
}

#[test]
fn a_shard_with_no_room_holds_nothing() {
    let _one = ONE_AT_A_TIME.lock().unwrap();
    let shard: CacheShard<u64, u64> = CacheShard::new(0);
    shard.put(1, 1, 1);
    assert_eq!(shard.get(&1, 1), None);
    assert_eq!(shard.len(), 0);
}

#[test]
fn a_put_dropped_at_capacity_zero_is_not_counted() {
    let _one = ONE_AT_A_TIME.lock().unwrap();
    let c: AliceCache<u64, u64> = AliceCache::new(0);
    c.put(1, 1);
    assert_eq!(c.stats().inserts.load(Ordering::Relaxed), 0);
}
