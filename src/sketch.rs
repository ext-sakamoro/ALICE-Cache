//! Count-Min Sketch (Optimized)
//!
//! Probabilistic frequency counter using sub-linear space.
//! Core component of TinyLFU admission policy.
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
    pub const fn new() -> Self {
        Self {
            table: [[0; W]; D],
            total: 0,
        }
    }

    /// Add an item (increment its count)
    #[inline]
    pub fn add(&mut self, key_hash: u64) {
        self.add_count(key_hash, 1);
    }

    /// Add an item with a specific count
    #[inline]
    pub fn add_count(&mut self, key_hash: u64, count: u8) {
        for i in 0..D {
            let idx = self.index(key_hash, i);
            self.table[i][idx] = self.table[i][idx].saturating_add(count);
        }
        self.total = self.total.saturating_add(count as u64);
    }

    /// Estimate the frequency of an item
    ///
    /// Returns the minimum count across all hash functions.
    /// This is always >= true count (no underestimate).
    #[inline]
    pub fn estimate(&self, key_hash: u64) -> u8 {
        let mut min_count = u8::MAX;
        for i in 0..D {
            let idx = self.index(key_hash, i);
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

    /// Halve all counters (aging mechanism for TinyLFU)
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
    #[inline]
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Calculate index for a given hash and row
    #[inline(always)]
    fn index(&self, key_hash: u64, row: usize) -> usize {
        // Different hash for each row using golden ratio mixing
        const GOLDEN: u64 = 0x9E3779B97F4A7C15;
        let mixed = key_hash.wrapping_add((row as u64).wrapping_mul(GOLDEN));
        // Fast modulo for power-of-2 widths, otherwise use remainder
        if W.is_power_of_two() {
            (mixed as usize) & (W - 1)
        } else {
            (mixed as usize) % W
        }
    }

    /// Memory usage in bytes
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
            for _ in 0..(i % 10 + 1) {
                sketch.add(i);
            }
        }

        // Estimate should never be less than true count
        for i in 0..1000u64 {
            let true_count = (i % 10 + 1) as u8;
            let estimate = sketch.estimate(i);
            assert!(
                estimate >= true_count,
                "Key {} has true count {} but estimate {}",
                i, true_count, estimate
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
}
