//! Distributed Cache with Jump Hash Example
//!
//! Demonstrates cache sharding across nodes using Jump Consistent Hash.
//!
//! ```bash
//! cargo run --example distributed_cache
//! ```

use alice_cache::{jump_hash, AliceCache, CacheConfig, CountMinSketch, Sketch4K};

fn main() {
    println!("=== Distributed Cache Demo ===\n");

    // --- Jump Consistent Hash ---
    println!("--- Jump Hash Distribution (4 nodes) ---\n");

    let num_nodes = 4u32;
    let mut distribution = [0u32; 4];

    for key in 0..10000u64 {
        let node = jump_hash(key, num_nodes);
        distribution[node as usize] += 1;
    }

    for (i, count) in distribution.iter().enumerate() {
        println!(
            "  Node {}: {} keys ({:.1}%)",
            i,
            count,
            *count as f64 / 100.0
        );
    }

    // --- Count-Min Sketch for frequency estimation ---
    println!("\n--- Count-Min Sketch (Frequency Estimation) ---\n");

    let mut sketch = Sketch4K::new();

    // Simulate traffic: some items are popular
    for _ in 0..1000 {
        sketch.add(42u64);
    }
    for _ in 0..100 {
        sketch.add(99u64);
    }
    sketch.add(7u64);

    println!(
        "  Item 42 frequency: ~{} (actual: 1000)",
        sketch.estimate(42u64)
    );
    println!(
        "  Item 99 frequency: ~{} (actual: 100)",
        sketch.estimate(99u64)
    );
    println!(
        "  Item  7 frequency: ~{} (actual: 1)",
        sketch.estimate(7u64)
    );

    // --- Per-node cache with config ---
    println!("\n--- Per-Node Cache ---\n");

    let config = CacheConfig {
        capacity: 1000,
        num_shards: 16,
        num_nodes: 4,
        node_id: 0,
        enable_oracle: true,
    };

    let node_cache: AliceCache<u64, Vec<u8>> = AliceCache::with_config(config);

    // Only store keys that belong to this node
    let mut stored = 0;
    for key in 0..100u64 {
        let owner = jump_hash(key, 4);
        if owner == 0 {
            node_cache.put(key, vec![0u8; 64]);
            stored += 1;
        }
    }

    println!("  Node 0 stored {} keys (out of 100 total)", stored);
}
