//! Basic Cache Operations Example
//!
//! Demonstrates put/get with TinyLFU eviction.
//!
//! ```bash
//! cargo run --example basic_cache
//! ```

use alice_cache::AliceCache;

fn main() {
    println!("=== ALICE-Cache Basic Demo ===\n");

    // Create cache with 1000 entries
    let cache: AliceCache<u64, String> = AliceCache::new(1000);

    // Insert items
    for i in 0..100u64 {
        cache.put(i, format!("value_{}", i));
    }
    println!("Inserted 100 items");

    // Read items (cache hits)
    let mut hits = 0;
    for i in 0..100u64 {
        if cache.get(&i).is_some() {
            hits += 1;
        }
    }
    println!("Read 100 items: {} hits", hits);

    // Simulate hot/cold access pattern
    println!("\n--- Hot/Cold Access Pattern ---\n");

    let cache: AliceCache<u64, u64> = AliceCache::new(100);

    // Insert 200 items (exceeds capacity)
    for i in 0..200u64 {
        cache.put(i, i * 10);
    }

    // Access "hot" keys repeatedly
    for _ in 0..10 {
        for i in 0..20u64 {
            cache.get(&i);
        }
    }

    // Check hot vs cold hit rate
    let mut hot_hits = 0;
    let mut cold_hits = 0;
    for i in 0..20u64 {
        if cache.get(&i).is_some() {
            hot_hits += 1;
        }
    }
    for i in 180..200u64 {
        if cache.get(&i).is_some() {
            cold_hits += 1;
        }
    }

    println!("Hot keys (0-19):    {}/20 hits", hot_hits);
    println!("Cold keys (180-199): {}/20 hits", cold_hits);
    println!("TinyLFU keeps frequently accessed items in cache");
}
