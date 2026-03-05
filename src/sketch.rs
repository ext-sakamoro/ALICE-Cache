//! Count-Min Sketch (Optimized)
//!
//! Probabilistic frequency counter using sub-linear space.
//! Core component of `TinyLFU` admission policy.
//!
//! - **Space**: O(w × d) where w = width, d = depth
//! - **Error**: P(overestimate > ε) ≤ δ for w = ⌈e/ε⌉, d = ⌈ln(1/δ)⌉
//! - **No underestimate**: Count-Min only overestimates

/// Count-Min Sketch with compile-time dimensions
///
/// W = width (number of counters per row)
/// D = depth (number of hash functions)
///
/// Recommended: W=1024, D=4 for ~1% error rate
#[derive(Clone)]
pub struct CountMinSketch<const W: usize, const D: usize> {
    table: [[u8; W]; D],
    total: u64,
}

impl<const W: usize, const D: usize> CountMinSketch<W, D> {
    /// Create a new empty sketch
    #[must_use]
    pub const fn new() -> Self {
        Self {
            table: [[0; W]; D],
            total: 0,
        }
    }

    /// Add an item (increment its count)
    #[inline(always)]
    pub fn add(&mut self, key_hash: u64) {
        self.add_count(key_hash, 1);
    }

    /// Add an item with a specific count
    #[inline(always)]
    pub fn add_count(&mut self, key_hash: u64, count: u8) {
        for i in 0..D {
            let idx = Self::index(key_hash, i);
            self.table[i][idx] = self.table[i][idx].saturating_add(count);
        }
        self.total = self.total.saturating_add(count as u64);
    }

    /// Estimate the frequency of an item
    ///
    /// Returns the minimum count across all hash functions.
    /// This is always >= true count (no underestimate).
    #[inline(always)]
    #[must_use]
    pub fn estimate(&self, key_hash: u64) -> u8 {
        let mut min_count = u8::MAX;
        for i in 0..D {
            let idx = Self::index(key_hash, i);
            min_count = min_count.min(self.table[i][idx]);
        }
        min_count
    }

    /// Reset all counters to zero
    pub fn clear(&mut self) {
        for row in &mut self.table {
            row.fill(0);
        }
        self.total = 0;
    }

    /// Halve all counters (aging mechanism for `TinyLFU`)
    ///
    /// Called periodically to prevent counter saturation
    /// and adapt to changing access patterns.
    pub fn halve(&mut self) {
        for row in &mut self.table {
            for counter in row.iter_mut() {
                *counter >>= 1;
            }
        }
        self.total >>= 1;
    }

    /// Total number of items added
    #[inline(always)]
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.total
    }

    /// Calculate index for a given hash and row
    #[inline(always)]
    const fn index(key_hash: u64, row: usize) -> usize {
        // Different hash for each row using golden ratio mixing
        const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;
        let mixed = key_hash.wrapping_add((row as u64).wrapping_mul(GOLDEN));
        // Fast modulo for power-of-2 widths, otherwise use remainder
        if W.is_power_of_two() {
            (mixed as usize) & (W - 1)
        } else {
            (mixed as usize) % W
        }
    }

    /// Memory usage in bytes
    #[must_use]
    pub const fn size_bytes() -> usize {
        W * D + 8 // table + total counter
    }
}

impl<const W: usize, const D: usize> Default for CountMinSketch<W, D> {
    fn default() -> Self {
        Self::new()
    }
}

/// Standard sketch configuration: 4KB, ~1% error for 1M items
pub type Sketch4K = CountMinSketch<1024, 4>;

/// Compact sketch: 1KB, ~4% error
pub type Sketch1K = CountMinSketch<256, 4>;

/// Large sketch: 16KB, ~0.25% error
pub type Sketch16K = CountMinSketch<4096, 4>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sketch_basic() {
        let mut sketch = Sketch4K::new();

        // Add same key multiple times
        for _ in 0..10 {
            sketch.add(12345);
        }

        assert_eq!(sketch.estimate(12345), 10);
        assert_eq!(sketch.estimate(99999), 0); // Never added
    }

    #[test]
    fn test_sketch_no_underestimate() {
        let mut sketch = Sketch4K::new();

        // Add various keys
        for i in 0..1000u64 {
            for _ in 0..=(i % 10) {
                sketch.add(i);
            }
        }

        // Estimate should never be less than true count
        for i in 0..1000u64 {
            let true_count = (i % 10 + 1) as u8;
            let estimate = sketch.estimate(i);
            assert!(
                estimate >= true_count,
                "Key {i} has true count {true_count} but estimate {estimate}",
            );
        }
    }

    #[test]
    fn test_sketch_halve() {
        let mut sketch = Sketch4K::new();

        for _ in 0..100 {
            sketch.add(12345);
        }
        assert_eq!(sketch.estimate(12345), 100);

        sketch.halve();
        assert_eq!(sketch.estimate(12345), 50);

        sketch.halve();
        assert_eq!(sketch.estimate(12345), 25);
    }

    #[test]
    fn test_sketch_saturation() {
        let mut sketch = Sketch4K::new();

        // Add more than u8::MAX times
        for _ in 0..300 {
            sketch.add(12345);
        }

        // Should saturate at 255
        assert_eq!(sketch.estimate(12345), 255);
    }

    #[test]
    fn test_sketch_clear() {
        let mut sketch = Sketch4K::new();

        sketch.add(12345);
        sketch.add(67890);
        assert!(sketch.estimate(12345) > 0);

        sketch.clear();
        assert_eq!(sketch.estimate(12345), 0);
        assert_eq!(sketch.total(), 0);
    }

    #[test]
    fn test_sketch_size() {
        assert_eq!(Sketch4K::size_bytes(), 1024 * 4 + 8);
        assert_eq!(Sketch1K::size_bytes(), 256 * 4 + 8);
    }

    // ── Additional tests for quality improvement ──────────────────

    #[test]
    fn test_sketch_add_count() {
        let mut sketch = Sketch4K::new();

        sketch.add_count(12345, 50);
        assert_eq!(sketch.estimate(12345), 50);
        assert_eq!(sketch.total(), 50);

        sketch.add_count(12345, 30);
        assert_eq!(sketch.estimate(12345), 80);
        assert_eq!(sketch.total(), 80);
    }

    #[test]
    fn test_sketch_add_count_saturating() {
        let mut sketch = Sketch4K::new();

        // Adding counts that would exceed u8::MAX should saturate
        sketch.add_count(12345, 200);
        sketch.add_count(12345, 200);
        assert_eq!(sketch.estimate(12345), 255);
    }

    #[test]
    fn test_sketch_total_tracking() {
        let mut sketch = Sketch4K::new();

        for i in 0..100u64 {
            sketch.add(i);
        }
        assert_eq!(sketch.total(), 100);

        sketch.halve();
        assert_eq!(sketch.total(), 50);

        sketch.clear();
        assert_eq!(sketch.total(), 0);
    }

    #[test]
    fn test_sketch_default_trait() {
        let sketch = Sketch4K::default();
        assert_eq!(sketch.estimate(12345), 0);
        assert_eq!(sketch.total(), 0);
    }

    #[test]
    fn test_sketch_different_sizes() {
        // Test Sketch1K (compact)
        let mut s1k = Sketch1K::new();
        for _ in 0..20 {
            s1k.add(42);
        }
        assert_eq!(s1k.estimate(42), 20);

        // Test Sketch16K (large)
        let mut s16k = Sketch16K::new();
        for _ in 0..20 {
            s16k.add(42);
        }
        assert_eq!(s16k.estimate(42), 20);
    }

    #[test]
    fn test_sketch_halve_rounds_down() {
        let mut sketch = Sketch4K::new();

        // Add odd count
        for _ in 0..7 {
            sketch.add(12345);
        }
        assert_eq!(sketch.estimate(12345), 7);

        sketch.halve();
        // 7 >> 1 = 3
        assert_eq!(sketch.estimate(12345), 3);

        sketch.halve();
        // 3 >> 1 = 1
        assert_eq!(sketch.estimate(12345), 1);

        sketch.halve();
        // 1 >> 1 = 0
        assert_eq!(sketch.estimate(12345), 0);
    }

    #[test]
    fn test_sketch_clone() {
        let mut sketch = Sketch4K::new();

        for _ in 0..30 {
            sketch.add(12345);
        }

        let cloned = sketch.clone();
        assert_eq!(cloned.estimate(12345), 30);
        assert_eq!(cloned.total(), 30);

        // Modifying original should not affect clone
        sketch.clear();
        assert_eq!(sketch.estimate(12345), 0);
        assert_eq!(cloned.estimate(12345), 30);
    }

    #[test]
    fn test_sketch_many_distinct_keys() {
        let mut sketch = Sketch4K::new();

        // Insert 500 distinct keys
        for i in 0..500u64 {
            sketch.add(i);
        }

        // Each key added once should have estimate >= 1 (no underestimate)
        for i in 0..500u64 {
            assert!(sketch.estimate(i) >= 1, "Key {i} underestimated");
        }
    }
}
