//! ALICE-Cache × ALICE-Analytics bridge
//!
//! Cache hit/miss metrics using HyperLogLog, DDSketch, CountMinSketch.
//!
//! Author: Moroya Sakamoto

use alice_analytics::{HyperLogLog, DDSketch, CountMinSketch};

/// Cache performance metrics collector
pub struct CacheMetrics {
    unique_keys: HyperLogLog,
    latency: DDSketch,
    hot_keys: CountMinSketch,
    pub hits: u64,
    pub misses: u64,
}

impl CacheMetrics {
    pub fn new() -> Self {
        Self {
            unique_keys: HyperLogLog::new(),
            latency: DDSketch::new(),
            hot_keys: CountMinSketch::new(),
            hits: 0,
            misses: 0,
        }
    }

    /// Record a cache hit with access latency
    pub fn record_hit(&mut self, key_hash: u64, latency_us: f64) {
        self.hits += 1;
        self.unique_keys.insert(key_hash);
        self.latency.add(latency_us);
        self.hot_keys.increment(key_hash);
    }

    /// Record a cache miss with access latency
    pub fn record_miss(&mut self, key_hash: u64, latency_us: f64) {
        self.misses += 1;
        self.unique_keys.insert(key_hash);
        self.latency.add(latency_us);
        self.hot_keys.increment(key_hash);
    }

    /// Cache hit rate (0.0 - 1.0)
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 { return 0.0; }
        self.hits as f64 / total as f64
    }

    /// Median access latency (microseconds)
    pub fn p50_latency(&self) -> f64 {
        self.latency.quantile(0.50)
    }

    /// 99th percentile access latency (microseconds)
    pub fn p99_latency(&self) -> f64 {
        self.latency.quantile(0.99)
    }

    /// Estimated unique key count
    pub fn unique_keys(&self) -> f64 {
        self.unique_keys.count()
    }

    /// Estimated frequency of a specific key
    pub fn key_frequency(&self, key_hash: u64) -> u64 {
        self.hot_keys.estimate(key_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hit_miss_tracking() {
        let mut m = CacheMetrics::new();
        m.record_hit(1, 10.0);
        m.record_hit(2, 20.0);
        m.record_miss(3, 100.0);
        assert_eq!(m.hits, 2);
        assert_eq!(m.misses, 1);
        assert!((m.hit_rate() - 0.6667).abs() < 0.01);
    }

    #[test]
    fn test_latency_quantiles() {
        let mut m = CacheMetrics::new();
        for i in 0..100 {
            m.record_hit(i, i as f64);
        }
        assert!(m.p50_latency() > 0.0);
        assert!(m.p99_latency() >= m.p50_latency());
    }

    #[test]
    fn test_unique_keys() {
        let mut m = CacheMetrics::new();
        for i in 0..1000 {
            m.record_hit(i, 1.0);
        }
        let est = m.unique_keys();
        assert!(est > 500.0 && est < 1500.0);
    }

    #[test]
    fn test_empty_metrics() {
        let m = CacheMetrics::new();
        assert_eq!(m.hit_rate(), 0.0);
        assert_eq!(m.hits, 0);
    }
}
