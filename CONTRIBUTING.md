# Contributing to ALICE-Cache

## Prerequisites

- Rust 1.70+ (stable)
- `cargo fmt`, `cargo clippy` components installed

## Build

```bash
cargo build
```

## Test

```bash
cargo test --lib --tests
```

## Lint

```bash
cargo clippy --lib --tests -- -W clippy::all -W clippy::pedantic
cargo fmt -- --check
cargo doc --no-deps 2>&1 | grep warning
```

## Code Style

- `cargo fmt` must pass with no diff
- `cargo clippy --lib --tests -- -W clippy::pedantic` must pass with 0 warnings
- All public value-returning functions must have `#[must_use]`
- Unsafe code must have `// Safety:` comments explaining invariants

## Optional Features

```bash
# ALICE-Analytics metrics bridge
cargo build --features analytics

# ALICE-Crypto signed entries
cargo build --features crypto

# Python bindings (requires Python environment)
cargo build --features pyo3
```

## Design Constraints

- **256 shards**: `parking_lot::Mutex` per shard eliminates lock contention.
- **Slab allocation**: dense `Vec<Entry>` enables O(1) random sampling for eviction.
- **Sampled eviction**: Redis-style random probes (k=5), no full iteration.
- **Lock-free oracle**: `AtomicU8` sketch for frequency estimation without mutex.
- **Jump hash**: O(1) consistent hashing with minimal key redistribution.
- **`no_std` compatible**: core library works with `alloc` only.
