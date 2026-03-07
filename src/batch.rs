//! Batch KV operations for LLM inference
//!
//! 複数トークンのKVキャッシュを一括処理するAPI。
//! Transformer推論ではprefill (N tokens) と decode (1 token) の両方で
//! KVキャッシュへのバッチアクセスが発生する。
//!
//! - `batch_put`: 複数キー・バリューを一括挿入
//! - `batch_get`: 複数キーを一括取得（ヒット/ミス情報付き）
//! - `batch_remove`: 複数キーを一括削除

use alloc::vec::Vec;
use core::hash::Hash;

use crate::cache::AliceCache;

extern crate alloc;

/// バッチ取得の結果
pub struct BatchGetResult<V> {
    /// 各キーに対応する結果（None = ミス）
    pub values: Vec<Option<V>>,
    /// ヒット数
    pub hits: usize,
    /// ミス数
    pub misses: usize,
}

impl<K, V> AliceCache<K, V>
where
    K: Eq + Hash + Clone + Send + Sync,
    V: Clone + Send + Sync,
{
    /// 複数キー・バリューを一括挿入
    ///
    /// キーとバリューのスライスは同じ長さでなければならない。
    ///
    /// # Panics
    ///
    /// `keys.len() != values.len()` の場合パニック。
    pub fn batch_put(&self, keys: &[K], values: &[V]) {
        assert_eq!(keys.len(), values.len(), "keys and values length mismatch");
        for (k, v) in keys.iter().zip(values.iter()) {
            self.put(k.clone(), v.clone());
        }
    }

    /// 複数キーを一括取得
    ///
    /// 戻り値の `values` は入力 `keys` と同じ順序。
    #[must_use]
    pub fn batch_get(&self, keys: &[K]) -> BatchGetResult<V> {
        let mut values = Vec::with_capacity(keys.len());
        let mut hits = 0_usize;
        let mut misses = 0_usize;

        for k in keys {
            if let Some(v) = self.get(k) {
                values.push(Some(v));
                hits += 1;
            } else {
                values.push(None);
                misses += 1;
            }
        }

        BatchGetResult {
            values,
            hits,
            misses,
        }
    }

    /// 複数キーを一括削除
    ///
    /// 削除されたバリューをVecで返す（存在しないキーはNone）。
    pub fn batch_remove(&self, keys: &[K]) -> Vec<Option<V>> {
        keys.iter().map(|k| self.remove(k)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_put_get() {
        let cache = AliceCache::<u32, u32>::new(100);
        let keys = [1, 2, 3, 4, 5];
        let vals = [10, 20, 30, 40, 50];
        cache.batch_put(&keys, &vals);

        let result = cache.batch_get(&keys);
        assert_eq!(result.hits, 5);
        assert_eq!(result.misses, 0);
        assert_eq!(result.values[0], Some(10));
        assert_eq!(result.values[4], Some(50));
    }

    #[test]
    fn test_batch_get_partial_miss() {
        let cache = AliceCache::<u32, u32>::new(100);
        cache.put(1, 10);
        cache.put(3, 30);

        let result = cache.batch_get(&[1, 2, 3, 4]);
        assert_eq!(result.hits, 2);
        assert_eq!(result.misses, 2);
        assert_eq!(result.values[0], Some(10));
        assert_eq!(result.values[1], None);
        assert_eq!(result.values[2], Some(30));
        assert_eq!(result.values[3], None);
    }

    #[test]
    fn test_batch_remove() {
        let cache = AliceCache::<u32, u32>::new(100);
        cache.batch_put(&[1, 2, 3], &[10, 20, 30]);

        let removed = cache.batch_remove(&[1, 4, 3]);
        assert_eq!(removed[0], Some(10));
        assert_eq!(removed[1], None);
        assert_eq!(removed[2], Some(30));
        assert_eq!(cache.get(&1), None);
    }

    #[test]
    fn test_batch_put_empty() {
        let cache = AliceCache::<u32, u32>::new(100);
        cache.batch_put(&[], &[]);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_batch_get_empty() {
        let cache = AliceCache::<u32, u32>::new(100);
        let result = cache.batch_get(&[]);
        assert_eq!(result.hits, 0);
        assert_eq!(result.misses, 0);
        assert!(result.values.is_empty());
    }

    #[test]
    #[should_panic(expected = "keys and values length mismatch")]
    fn test_batch_put_length_mismatch() {
        let cache = AliceCache::<u32, u32>::new(100);
        cache.batch_put(&[1, 2], &[10]);
    }

    #[test]
    fn test_batch_get_result_order() {
        let cache = AliceCache::<u32, String>::new(100);
        let keys = [10, 20, 30];
        let vals = ["a".into(), "b".into(), "c".into()];
        cache.batch_put(&keys, &vals);

        let result = cache.batch_get(&[30, 10, 20]);
        assert_eq!(result.values[0], Some("c".to_string()));
        assert_eq!(result.values[1], Some("a".to_string()));
        assert_eq!(result.values[2], Some("b".to_string()));
    }

    #[test]
    fn test_batch_large() {
        let cache = AliceCache::<u32, u32>::new(10000);
        let keys: Vec<u32> = (0..1000).collect();
        let vals: Vec<u32> = (0..1000).map(|i| i * 10).collect();
        cache.batch_put(&keys, &vals);

        let result = cache.batch_get(&keys);
        assert_eq!(result.hits, 1000);
        assert_eq!(result.misses, 0);
    }
}
