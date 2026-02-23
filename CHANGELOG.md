# Changelog

All notable changes to ALICE-Cache will be documented in this file.

## [0.2.0] - 2026-02-23

### Added
- `cache` — `AliceCache` with 256 shards, slab allocation, sampled eviction, `CacheConfig`, `CacheStats`
- `shard` — Per-shard `parking_lot::Mutex`, dense `Vec<Entry>` storage, O(1) random sampling
- `oracle` — Lock-free `AtomicU8` frequency sketch for predictive prefetch
- `sketch` — Count-Min Sketch with halving decay for frequency estimation
- `jump_hash` — O(1) consistent hashing for distributed key routing (`jump_hash`, `jump_hash_bytes`, `jump_hash_u128`)
- `analytics_bridge` — (feature `analytics`) Cache hit/miss metrics and hot key tracking via ALICE-Analytics
- `crypto_bridge` — (feature `crypto`) Signed cache entries for tamper prevention via ALICE-Crypto
- `python` — (feature `pyo3`) Python bindings
- `no_std` support with `alloc` fallback
- 86 unit tests + 1 doc-test
