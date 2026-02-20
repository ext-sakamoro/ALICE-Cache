//! The Predictive Oracle (Atomic / Lock-Free)
//!
//! **Optimized**: Uses `AtomicU8` counters. No Mutex.
//! `Ordering::Relaxed` is sufficient for probabilistic predictions.
//!
//! "Don't remember the past. Predict the future."

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

extern crate alloc;

/// Minimum frequency threshold for prefetch recommendation
const PREFETCH_THRESHOLD: u8 = 3;

/// Lock-Free Count-Min Sketch with AtomicU8 counters
struct AtomicSketch<const W: usize, const D: usize> {
    /// Flattened array: table[row][col] -> table[row * W + col]
    /// Boxed to prevent stack overflow on large W
    table: Box<[AtomicU8]>,
}

impl<const W: usize, const D: usize> AtomicSketch<W, D> {
    fn new() -> Self {
        let len = W * D;
        let mut vec = Vec::with_capacity(len);
        for _ in 0..len {
            vec.push(AtomicU8::new(0));
        }
        Self {
            table: vec.into_boxed_slice(),
        }
    }

    /// Add to frequency count (lock-free, relaxed ordering)
    #[inline(always)]
    fn add(&self, key_hash: u64) {
        for i in 0..D {
            let offset = i * W + self.index(key_hash, i);
            // Relaxed is fine; we don't need synchronization, just statistics
            // Saturating add via load-check-store (racy but OK for prediction)
            let val = self.table[offset].load(Ordering::Relaxed);
            if val < 255 {
                self.table[offset].store(val.saturating_add(1), Ordering::Relaxed);
            }
        }
    }

    /// Estimate frequency (minimum across all rows)
    #[inline(always)]
    fn estimate(&self, key_hash: u64) -> u8 {
        let mut min_count = u8::MAX;
        for i in 0..D {
            let offset = i * W + self.index(key_hash, i);
            let val = self.table[offset].load(Ordering::Relaxed);
            min_count = min_count.min(val);
        }
        min_count
    }

    /// Halve all counters (for aging)
    fn halve(&self) {
        for atomic in self.table.iter() {
            let val = atomic.load(Ordering::Relaxed);
            atomic.store(val >> 1, Ordering::Relaxed);
        }
    }

    /// Clear all counters
    fn clear(&self) {
        for atomic in self.table.iter() {
            atomic.store(0, Ordering::Relaxed);
        }
    }

    /// Hash to index (assumes W is power of 2)
    #[inline(always)]
    fn index(&self, key_hash: u64, row: usize) -> usize {
        const GOLDEN: u64 = 0x9E3779B97F4A7C15;
        let mixed = key_hash.wrapping_add((row as u64).wrapping_mul(GOLDEN));
        (mixed as usize) & (W - 1)
    }
}

/// Markov Oracle for access pattern prediction (internal, not thread-safe alone)
pub struct MarkovOracle {
    /// Transition frequency sketch
    transitions: AtomicSketch<1024, 2>,
    /// Last accessed key hash
    last_key_hash: Option<u64>,
    /// Total transitions (for aging)
    total_transitions: u64,
}

impl MarkovOracle {
    /// Create new oracle
    pub fn new() -> Self {
        Self {
            transitions: AtomicSketch::new(),
            last_key_hash: None,
            total_transitions: 0,
        }
    }

    /// Record an access and update transition probabilities
    #[inline(always)]
    pub fn record(&mut self, current_key_hash: u64) {
        if let Some(prev) = self.last_key_hash {
            let h = Self::mix(prev, current_key_hash);
            self.transitions.add(h);
            self.total_transitions += 1;

            // Age periodically
            if self.total_transitions % 50000 == 0 {
                self.transitions.halve();
            }
        }
        self.last_key_hash = Some(current_key_hash);
    }

    /// Check if we should prefetch a candidate key
    #[inline(always)]
    pub fn should_prefetch(&self, current: u64, candidate: u64) -> bool {
        let h = Self::mix(current, candidate);
        self.transitions.estimate(h) >= PREFETCH_THRESHOLD
    }

    /// Get the estimated transition frequency
    #[inline(always)]
    pub fn transition_freq(&self, from: u64, to: u64) -> u8 {
        let h = Self::mix(from, to);
        self.transitions.estimate(h)
    }

    /// Reset the oracle
    pub fn reset(&mut self) {
        self.transitions.clear();
        self.last_key_hash = None;
        self.total_transitions = 0;
    }

    /// Mix two hashes for transition key
    #[inline(always)]
    fn mix(a: u64, b: u64) -> u64 {
        a ^ b.rotate_left(32)
    }
}

impl Default for MarkovOracle {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe Lock-Free Oracle
///
/// Uses AtomicU64 for last_key tracking and lock-free sketch.
/// Some races on last_key are acceptable for probabilistic prediction.
pub struct SharedOracle {
    /// Lock-free transition sketch
    transitions: AtomicSketch<1024, 2>,
    /// Last accessed key (atomic, races acceptable)
    last_key: AtomicU64,
    /// Counter for aging (atomic)
    counter: AtomicU64,
}

impl SharedOracle {
    pub fn new() -> Self {
        Self {
            transitions: AtomicSketch::new(),
            last_key: AtomicU64::new(0),
            counter: AtomicU64::new(0),
        }
    }

    /// Record access (lock-free)
    #[inline(always)]
    pub fn record(&self, current_hash: u64) {
        // Swap returns the previous value - perfect for chaining
        let prev = self.last_key.swap(current_hash, Ordering::Relaxed);
        if prev != 0 {
            let h = Self::mix(prev, current_hash);
            self.transitions.add(h);

            // Age periodically
            let count = self.counter.fetch_add(1, Ordering::Relaxed);
            if count % 50000 == 0 {
                self.transitions.halve();
            }
        }
    }

    /// Check if we should prefetch (lock-free)
    #[inline(always)]
    pub fn should_prefetch(&self, current: u64, candidate: u64) -> bool {
        let h = Self::mix(current, candidate);
        self.transitions.estimate(h) >= PREFETCH_THRESHOLD
    }

    /// Reset the oracle
    pub fn reset(&self) {
        self.transitions.clear();
        self.last_key.store(0, Ordering::Relaxed);
        self.counter.store(0, Ordering::Relaxed);
    }

    #[inline(always)]
    fn mix(a: u64, b: u64) -> u64 {
        a ^ b.rotate_left(32)
    }
}

impl Default for SharedOracle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oracle_basic() {
        let mut oracle = MarkovOracle::new();

        // Train: A -> B -> C -> A -> B -> C ...
        for _ in 0..100 {
            oracle.record(1); // A
            oracle.record(2); // B
            oracle.record(3); // C
        }

        // Should predict B after A
        assert!(oracle.should_prefetch(1, 2));
        // Should predict C after B
        assert!(oracle.should_prefetch(2, 3));
        // Should predict A after C
        assert!(oracle.should_prefetch(3, 1));

        // Note: Count-Min Sketch is probabilistic - false positives possible
        // We verify trained transitions exceed threshold (true positives work)
        assert!(oracle.transition_freq(1, 2) >= PREFETCH_THRESHOLD);
        assert!(oracle.transition_freq(2, 3) >= PREFETCH_THRESHOLD);
        assert!(oracle.transition_freq(3, 1) >= PREFETCH_THRESHOLD);
    }

    #[test]
    fn test_oracle_cold_start() {
        let oracle = MarkovOracle::new();

        // No training - should not recommend anything
        assert!(!oracle.should_prefetch(1, 2));
        assert!(!oracle.should_prefetch(100, 200));
    }

    #[test]
    fn test_oracle_reset() {
        let mut oracle = MarkovOracle::new();

        // Train
        for _ in 0..100 {
            oracle.record(1);
            oracle.record(2);
        }
        assert!(oracle.should_prefetch(1, 2));

        // Reset
        oracle.reset();
        assert!(!oracle.should_prefetch(1, 2));
    }

    #[test]
    fn test_shared_oracle() {
        let oracle = SharedOracle::new();

        // Train from multiple "threads" (simulated)
        for _ in 0..50 {
            oracle.record(10);
            oracle.record(20);
        }

        assert!(oracle.should_prefetch(10, 20));
    }

    #[test]
    fn test_shared_oracle_concurrent() {
        use std::sync::Arc;
        use std::thread;

        let oracle = Arc::new(SharedOracle::new());
        let mut handles = vec![];

        // Spawn multiple threads recording patterns
        for t in 0..4 {
            let oracle = Arc::clone(&oracle);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    oracle.record(t * 10 + 1);
                    oracle.record(t * 10 + 2);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Should have learned some patterns (concurrent, so not perfectly deterministic)
        // At least verify no crashes and some learning occurred
        let total_freq: u32 = (0..4)
            .map(|t| oracle.transitions.estimate(SharedOracle::mix(t * 10 + 1, t * 10 + 2)) as u32)
            .sum();
        assert!(total_freq > 0);
    }

    #[test]
    fn test_oracle_asymmetric() {
        let mut oracle = MarkovOracle::new();

        // Train A -> B only
        for _ in 0..100 {
            oracle.record(1);
            oracle.record(2);
            // Reset to prevent B->A transition
            oracle.last_key_hash = None;
        }

        // A->B should be strong
        assert!(oracle.should_prefetch(1, 2));
        // B->A should be weak (hash mixing makes them different)
        let ba_freq = oracle.transition_freq(2, 1);
        let ab_freq = oracle.transition_freq(1, 2);
        assert!(ab_freq > ba_freq);
    }
}
