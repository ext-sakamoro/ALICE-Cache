# ALICE-Cache "Charred"

**Predictive Distributed Caching System**

> "Don't remember the past. Predict the future."

## Features

| Component | Algorithm | Complexity |
|-----------|-----------|------------|
| Storage | Slab Allocation (dense Vec) | O(1) random access |
| Frequency Counter | Count-Min Sketch | O(1), fixed 4KB |
| Eviction | Sampled TinyLFU | O(k), k=5 samples |
| Prediction | Lock-Free Markov Oracle | O(1), no mutex |
| Distributed Routing | Jump Consistent Hash | O(ln n), no ring |

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    AliceCache (Thread-Safe)                 │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────┐ ┌─────────┐ ┌─────────┐       ┌─────────┐     │
│  │ Shard 0 │ │ Shard 1 │ │ Shard 2 │  ...  │Shard 255│     │
│  │ (Mutex) │ │ (Mutex) │ │ (Mutex) │       │ (Mutex) │     │
│  └────┬────┘ └────┬────┘ └────┬────┘       └────┬────┘     │
│       │           │           │                 │          │
│       v           v           v                 v          │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              Slab: Vec<Entry<K,V>>                  │   │
│  │              Lookup: HashMap<K, u32>                │   │
│  │              Sketch: CountMinSketch                 │   │
│  └─────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────────┐   │
│  │         SharedOracle (Lock-Free AtomicU8)           │   │
│  │         Markov Chain: A -> B transition freq        │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### Charred Optimizations

1. **Slab Allocation**: Data stored in dense `Vec<Entry>`, HashMap only maps `K -> Index`. Random sampling is pure `O(1)` memory offset.

2. **Lock-Free Oracle**: `AtomicU8` counters with `Ordering::Relaxed`. No mutex contention on prediction path.

3. **Swap-Remove**: Eviction uses `swap_remove` to keep slab dense. O(1), no memory fragmentation.

4. **256 Shards**: Fine-grained locking eliminates contention. Each shard is independently locked.

## Installation

```toml
[dependencies]
alice-cache = "0.2"
```

## Usage

### Local Cache

```rust
use alice_cache::AliceCache;

let cache = AliceCache::<u32, String>::new(10000);

// Insert
cache.put(1, "hello".to_string());
cache.put(2, "world".to_string());

// Retrieve
assert_eq!(cache.get(&1), Some("hello".to_string()));

// Miss returns None
assert_eq!(cache.get(&999), None);

// Statistics
println!("Hit rate: {:.1}%", cache.hit_rate() * 100.0);
```

### Thread-Safe Usage

```rust
use alice_cache::AliceCache;
use std::sync::Arc;
use std::thread;

let cache = Arc::new(AliceCache::<u64, u64>::new(10000));
let mut handles = vec![];

for t in 0..4 {
    let cache = Arc::clone(&cache);
    handles.push(thread::spawn(move || {
        for i in 0..1000 {
            cache.put(t * 1000 + i, i * 10);
            cache.get(&(t * 1000 + i));
        }
    }));
}

for h in handles {
    h.join().unwrap();
}
```

### Distributed Cache

```rust
use alice_cache::{AliceCache, CacheConfig};

let config = CacheConfig {
    capacity: 10000,
    num_nodes: 10,
    node_id: 3,
    ..Default::default()
};

let cache = AliceCache::<u64, Vec<u8>>::with_config(config);

// Check which node owns a key
let owner = cache.owner_node(&12345u64);
println!("Key 12345 belongs to node {}", owner);

// Check if this node owns a key
if cache.is_local_owner(&12345u64) {
    println!("This node owns key 12345");
}
```

### Predictive Prefetching

```rust
use alice_cache::AliceCache;

let cache = AliceCache::<u32, u32>::new(1000);

// Train access pattern: 1 -> 2 -> 3 -> 1 -> 2 -> 3 ...
for _ in 0..100 {
    cache.put(1, 10); cache.get(&1);
    cache.put(2, 20); cache.get(&2);
    cache.put(3, 30); cache.get(&3);
}

// Oracle predicts next access
if cache.should_prefetch(&1, &2) {
    println!("Prefetch key 2 after accessing key 1");
}
```

### Count-Min Sketch (Standalone)

```rust
use alice_cache::Sketch4K;

let mut sketch = Sketch4K::new();

// Track frequencies
for _ in 0..100 { sketch.add(12345); }
for _ in 0..10 { sketch.add(67890); }

// Estimate (may overestimate, never underestimates)
assert!(sketch.estimate(12345) >= 100);
assert!(sketch.estimate(67890) >= 10);

// Age counters for changing patterns
sketch.halve();
```

### Jump Consistent Hash (Standalone)

```rust
use alice_cache::jump_hash;

// Map key to one of 10 buckets
let bucket = jump_hash(12345, 10);
assert!(bucket >= 0 && bucket < 10);

// Same key always maps to same bucket
assert_eq!(jump_hash(12345, 10), jump_hash(12345, 10));
```

## API Reference

### `AliceCache<K, V>`

```rust
// Construction
fn new(capacity: usize) -> Self;
fn with_config(config: CacheConfig) -> Self;

// Operations (thread-safe)
fn get(&self, key: &K) -> Option<V>;
fn put(&self, key: K, value: V);
fn remove(&self, key: &K) -> Option<V>;
fn contains(&self, key: &K) -> bool;

// Prediction
fn should_prefetch(&self, current: &K, candidate: &K) -> bool;

// Distributed
fn owner_node(&self, key: &K) -> u32;
fn is_local_owner(&self, key: &K) -> bool;

// Stats
fn len(&self) -> usize;
fn capacity(&self) -> usize;
fn hit_rate(&self) -> f64;
fn stats(&self) -> &CacheStats;
```

### `CountMinSketch<W, D>`

```rust
fn new() -> Self;
fn add(&mut self, key_hash: u64);
fn estimate(&self, key_hash: u64) -> u8;
fn halve(&mut self);  // Aging
fn clear(&mut self);
```

### `jump_hash`

```rust
fn jump_hash(key: u64, num_buckets: i32) -> i32;
fn jump_hash_bytes(key: &[u8], num_buckets: i32) -> i32;
fn jump_hash_u128(key: u128, num_buckets: i32) -> i32;
```

## Performance

### Charred Architecture

| Operation | Complexity | Notes |
|-----------|------------|-------|
| Get | O(1) | HashMap lookup + Vec index |
| Put | O(1) | Slab append or update |
| Evict | O(5) | 5 random Vec probes |
| Predict | O(1) | Lock-free atomic sketch |

### Count-Min Sketch

- **Memory**: 4KB for 1M items with ~1% error
- **Speed**: O(1) add/estimate, ~10ns per operation

### Jump Consistent Hash

- **Memory**: O(1) - no ring storage
- **Speed**: O(ln n) jumps, ~20ns for 1000 nodes
- **Stability**: Only 1/n keys move when adding node n

## SDF Asset Delivery Network Integration

ALICE-Cache serves as the predictive caching layer in the SDF asset delivery pipeline, achieving **200-800x bandwidth reduction** vs glTF by delivering mathematical descriptions instead of polygon meshes.

```
Client Request (asset_id + VivaldiCoord)
    │
    ▼
┌──────────────────────────────────────┐
│  ALICE-CDN (Vivaldi Routing)          │
│  ・VivaldiCoord → nearest node (RTT)  │
└──────────┬───────────────────────────┘
           │
           ▼
┌──────────────────────────────────────┐
│  ALICE-Cache (Markov Prefetch)   ◀── this crate
│  ・256-shard parallel cache           │
│  ・SharedOracle: lock-free prediction │
│  ・TinyLFU sampled eviction           │
└──────────┬───────────────────────────┘
           │ cache miss → origin
           ▼
┌──────────────────────────────────────┐
│  ALICE-SDF (ASDF Binary Format)       │
│  ・80 bytes (sphere) vs 20 KB (glTF)  │
└──────────────────────────────────────┘
```

Related: [ALICE-SDF](https://github.com/ext-sakamoro/ALICE-SDF) | [ALICE-CDN](https://github.com/ext-sakamoro/ALICE-CDN)

## Cross-Crate Bridges

### Sync Bridge (feature: `sync`)

CRDT-based cache invalidation and distributed consistency via [ALICE-Sync](../ALICE-Sync). When a node's sync state changes, the sync bridge automatically invalidates stale cache entries across the distributed cache cluster, ensuring eventual consistency without manual purging.

```toml
[dependencies]
alice-cache = { version = "0.2", features = ["sync"] }
```

### ALICE-Analytics Bridge (feature: `analytics`)

Cache hit/miss metrics with streaming telemetry.

- `CacheMetrics` — HyperLogLog (unique keys), DDSketch (latency percentiles), CountMinSketch (hot key frequency)
- `record_hit()` / `record_miss()` — Record cache access with latency
- `hit_rate()`, `p50_latency()`, `p99_latency()`, `unique_keys()` — Query metrics

Enable: `alice-cache = { features = ["analytics"] }`

## License

**GNU AGPLv3**

## Author

Moroya Sakamoto
