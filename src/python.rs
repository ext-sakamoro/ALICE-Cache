//! PyO3 Python Bindings for ALICE-Cache
//!
//! Predictive distributed caching for Python web frameworks.
//! String keys, bytes values. Thread-safe.

use pyo3::prelude::*;
use pyo3::exceptions::PyValueError;
use numpy::{IntoPyArray, PyArray1, PyReadonlyArray1};

use crate::cache::{AliceCache, CacheConfig};
use crate::jump_hash;
use crate::oracle::SharedOracle;
use crate::sketch::Sketch4K;

// ============================================================================
// AliceCache (String keys, bytes values)
// ============================================================================

/// High-performance predictive cache with Markov prefetching.
///
/// 256-shard lock-free architecture. Thread-safe.
#[pyclass(name = "AliceCache")]
pub struct PyAliceCache {
    inner: AliceCache<String, Vec<u8>>,
}

#[pymethods]
impl PyAliceCache {
    /// Create cache with given capacity.
    #[new]
    #[pyo3(signature = (capacity=10000))]
    fn new(capacity: usize) -> Self {
        Self {
            inner: AliceCache::new(capacity),
        }
    }

    /// Create with full configuration.
    #[staticmethod]
    #[pyo3(signature = (capacity=10000, num_nodes=1, node_id=0, enable_oracle=true))]
    fn with_config(
        capacity: usize,
        num_nodes: i32,
        node_id: u32,
        enable_oracle: bool,
    ) -> Self {
        let config = CacheConfig {
            capacity,
            num_nodes,
            node_id,
            enable_oracle,
            ..Default::default()
        };
        Self {
            inner: AliceCache::with_config(config),
        }
    }

    /// Get value by key. Returns None if not found.
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.inner.get(&key.to_string())
    }

    /// Insert or update a key-value pair.
    fn put(&self, key: &str, value: Vec<u8>) {
        self.inner.put(key.to_string(), value);
    }

    /// Remove a key. Returns the old value if present.
    fn remove(&self, key: &str) -> Option<Vec<u8>> {
        self.inner.remove(&key.to_string())
    }

    /// Check if key exists.
    fn contains(&self, key: &str) -> bool {
        self.inner.contains(&key.to_string())
    }

    /// Predict whether `candidate` should be prefetched after `current`.
    fn should_prefetch(&self, current: &str, candidate: &str) -> bool {
        self.inner
            .should_prefetch(&current.to_string(), &candidate.to_string())
    }

    /// Get owning node ID for distributed routing.
    fn owner_node(&self, key: &str) -> u32 {
        self.inner.owner_node(&key.to_string())
    }

    /// Check if this node owns the key.
    fn is_local_owner(&self, key: &str) -> bool {
        self.inner.is_local_owner(&key.to_string())
    }

    #[getter]
    fn len(&self) -> usize {
        self.inner.len()
    }

    #[getter]
    fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    #[getter]
    fn hit_rate(&self) -> f64 {
        self.inner.hit_rate()
    }

    #[getter]
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get cache statistics.
    fn stats(&self) -> PyResult<(u64, u64, u64, u64)> {
        let s = self.inner.stats();
        Ok((
            s.hits.load(std::sync::atomic::Ordering::Relaxed),
            s.misses.load(std::sync::atomic::Ordering::Relaxed),
            s.inserts.load(std::sync::atomic::Ordering::Relaxed),
            s.evictions.load(std::sync::atomic::Ordering::Relaxed),
        ))
    }

    fn __repr__(&self) -> String {
        format!(
            "AliceCache(len={}, cap={}, hit_rate={:.1}%)",
            self.inner.len(),
            self.inner.capacity(),
            self.inner.hit_rate() * 100.0
        )
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __contains__(&self, key: &str) -> bool {
        self.inner.contains(&key.to_string())
    }
}

// ============================================================================
// Count-Min Sketch (standalone)
// ============================================================================

/// Standalone frequency counter (4KB, never underestimates).
#[pyclass(name = "CountMinSketch")]
pub struct PyCountMinSketch {
    inner: Sketch4K,
}

#[pymethods]
impl PyCountMinSketch {
    #[new]
    fn new() -> Self {
        Self {
            inner: Sketch4K::new(),
        }
    }

    fn add(&mut self, key_hash: u64) {
        self.inner.add(key_hash);
    }

    fn estimate(&self, key_hash: u64) -> u8 {
        self.inner.estimate(key_hash)
    }

    fn total(&self) -> u64 {
        self.inner.total()
    }

    fn halve(&mut self) {
        self.inner.halve();
    }

    fn clear(&mut self) {
        self.inner.clear();
    }
}

// ============================================================================
// SharedOracle (Markov prediction)
// ============================================================================

/// Lock-free Markov oracle for access pattern prediction.
#[pyclass(name = "SharedOracle")]
pub struct PySharedOracle {
    inner: SharedOracle,
}

#[pymethods]
impl PySharedOracle {
    #[new]
    fn new() -> Self {
        Self {
            inner: SharedOracle::new(),
        }
    }

    fn record(&self, current_hash: u64) {
        self.inner.record(current_hash);
    }

    fn should_prefetch(&self, current: u64, candidate: u64) -> bool {
        self.inner.should_prefetch(current, candidate)
    }

    fn reset(&self) {
        self.inner.reset();
    }
}

// ============================================================================
// Jump Hash Functions
// ============================================================================

/// Jump consistent hash: O(ln n), minimal key movement.
#[pyfunction]
fn jump_consistent_hash(key: u64, num_buckets: i32) -> i32 {
    jump_hash::jump_hash(key, num_buckets)
}

/// Jump hash for byte keys.
#[pyfunction]
fn jump_hash_bytes(key: &[u8], num_buckets: i32) -> i32 {
    jump_hash::jump_hash_bytes(key, num_buckets)
}

/// Batch jump hash for NumPy u64 array (GIL released).
#[pyfunction]
fn jump_hash_batch<'py>(
    py: Python<'py>,
    keys: PyReadonlyArray1<'py, u64>,
    num_buckets: i32,
) -> PyResult<Bound<'py, PyArray1<i32>>> {
    let slice = keys.as_slice().map_err(|e| PyValueError::new_err(e.to_string()))?;
    let result = py.detach(|| {
        slice
            .iter()
            .map(|&k| jump_hash::jump_hash(k, num_buckets))
            .collect::<Vec<i32>>()
    });
    Ok(result.into_pyarray(py))
}

// ============================================================================
// Module
// ============================================================================

#[pymodule]
pub fn alice_cache(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyAliceCache>()?;
    m.add_class::<PyCountMinSketch>()?;
    m.add_class::<PySharedOracle>()?;

    m.add_function(wrap_pyfunction!(jump_consistent_hash, m)?)?;
    m.add_function(wrap_pyfunction!(jump_hash_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(jump_hash_batch, m)?)?;

    Ok(())
}
