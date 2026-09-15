# Changelog

All notable changes to ALICE-Cache will be documented in this file.

## [Unreleased]

### Fixed
- **`no_std` が bare-metal で偽だった** — 非 optional の `parking_lot` (std 必須) と `ahash::RandomState::new()` (runtime RNG) により `aarch64-unknown-none` では `can't find crate for std` (host の `--no-default-features` は host std を暗黙 link して見かけ green) `parking_lot` を `std` feature 限定 optional に、`no_std` は `spin::Mutex` (`shard.rs` で cfg 切替) + `ahash` `compile-time-rng` `AtomicU64` 統計 counter のため 64-bit atomics を持つ target が前提 (32-bit MCU は対象外、lib doc に明記)

### Changed
- CI: clippy を `--all-targets` (benches / examples) に、`no_std` job (host rlib + bare-metal `aarch64-unknown-none` + clippy-driver wrapper)、`feature-powerset` (cargo-hack、std 固定 depth 2)、sibling stub を `alice-stubs` action に統一、rust-cache
- deps: `parking_lot` optional (`std`)、`spin` 0.9 追加 (`no_std` のみ)、`ahash` `default-features = false` + `compile-time-rng` (`std` で `std` / `runtime-rng` を有効化、std build の挙動は不変)

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
- CI/CD via GitHub Actions (test, clippy, fmt, doc)
- `#[must_use]` on all public value-returning functions
- clippy pedantic: 0 warnings
