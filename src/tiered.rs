//! 階層ストレージ — Hot (RAM) → Warm (overflow) 2階層キャッシュ
//!
//! 容量超過時に Hot 層から Warm 層へエントリーをスピルし、
//! アクセス頻度に基づいて昇格 (promote) / 降格 (demote) する。

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

extern crate alloc;

/// 階層ストレージ設定。
#[derive(Debug, Clone, Copy)]
pub struct TieredConfig {
    /// Hot 層の最大エントリー数。
    pub hot_capacity: usize,
    /// Warm 層の最大エントリー数。
    pub warm_capacity: usize,
    /// 昇格閾値 (Warm 層でのアクセス回数)。
    pub promote_threshold: u32,
}

impl Default for TieredConfig {
    fn default() -> Self {
        Self {
            hot_capacity: 1000,
            warm_capacity: 10000,
            promote_threshold: 3,
        }
    }
}

/// 階層キャッシュエントリー。
#[derive(Debug, Clone)]
struct TieredEntry {
    /// 値 (バイト列)。
    value: Vec<u8>,
    /// アクセス回数。
    access_count: u32,
}

/// 階層キャッシュ — Hot + Warm 2層。
///
/// Hot 層: 高速アクセス (RAM)。容量超過時に Warm へスピル。
/// Warm 層: オーバーフロー領域。頻繁にアクセスされれば Hot へ昇格。
#[derive(Debug)]
pub struct TieredCache {
    config: TieredConfig,
    /// Hot 層 (キー → エントリー)。
    hot: BTreeMap<String, TieredEntry>,
    /// Warm 層 (キー → エントリー)。
    warm: BTreeMap<String, TieredEntry>,
    /// 統計: 昇格回数。
    promotions: u64,
    /// 統計: 降格回数。
    demotions: u64,
    /// 統計: 退去回数。
    evictions: u64,
}

impl TieredCache {
    /// 新しい階層キャッシュを作成。
    #[must_use]
    pub const fn new(config: TieredConfig) -> Self {
        Self {
            config,
            hot: BTreeMap::new(),
            warm: BTreeMap::new(),
            promotions: 0,
            demotions: 0,
            evictions: 0,
        }
    }

    /// デフォルト設定で作成。
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(TieredConfig::default())
    }

    /// 値を取得。Hot → Warm の順で探索。
    ///
    /// Warm でヒットした場合、アクセス回数が閾値を超えれば Hot へ昇格。
    /// 値のクローンを返す (昇格時のborrow衝突回避のため)。
    pub fn get(&mut self, key: &str) -> Option<Vec<u8>> {
        // Hot 層チェック
        if let Some(entry) = self.hot.get_mut(key) {
            entry.access_count += 1;
            return Some(entry.value.clone());
        }

        // Warm 層チェック — 昇格判定
        let should_promote = self
            .warm
            .get(key)
            .is_some_and(|e| e.access_count + 1 >= self.config.promote_threshold);

        if should_promote {
            let mut entry = self.warm.remove(key)?;
            entry.access_count += 1;
            let result = entry.value.clone();
            self.promote_to_hot(key, entry);
            return Some(result);
        }

        if let Some(entry) = self.warm.get_mut(key) {
            entry.access_count += 1;
            return Some(entry.value.clone());
        }

        None
    }

    /// 値を挿入 (Hot 層へ)。
    pub fn put(&mut self, key: &str, value: Vec<u8>) {
        // 既存エントリーを削除
        self.warm.remove(key);

        let entry = TieredEntry {
            value,
            access_count: 1,
        };

        // Hot 層が満杯ならスピル
        if self.hot.len() >= self.config.hot_capacity && !self.hot.contains_key(key) {
            self.spill_one();
        }

        self.hot.insert(key.into(), entry);
    }

    /// 値を削除。
    pub fn remove(&mut self, key: &str) -> bool {
        self.hot.remove(key).is_some() || self.warm.remove(key).is_some()
    }

    /// Hot 層から最もアクセスの少ないエントリーを Warm へスピル。
    fn spill_one(&mut self) {
        let victim_key = self
            .hot
            .iter()
            .min_by_key(|(_, e)| e.access_count)
            .map(|(k, _)| k.clone());

        if let Some(key) = victim_key {
            if let Some(mut entry) = self.hot.remove(&key) {
                entry.access_count = 0;
                self.demotions += 1;

                // Warm が満杯なら退去
                if self.warm.len() >= self.config.warm_capacity {
                    self.evict_warm();
                }
                self.warm.insert(key, entry);
            }
        }
    }

    /// Warm 層から最もアクセスの少ないエントリーを退去。
    fn evict_warm(&mut self) {
        let victim_key = self
            .warm
            .iter()
            .min_by_key(|(_, e)| e.access_count)
            .map(|(k, _)| k.clone());

        if let Some(key) = victim_key {
            self.warm.remove(&key);
            self.evictions += 1;
        }
    }

    /// Hot → Warm へのスピルを伴う昇格。
    fn promote_to_hot(&mut self, key: &str, entry: TieredEntry) {
        if self.hot.len() >= self.config.hot_capacity {
            self.spill_one();
        }
        self.hot.insert(key.into(), entry);
        self.promotions += 1;
    }

    /// Hot 層のエントリー数。
    #[must_use]
    pub fn hot_len(&self) -> usize {
        self.hot.len()
    }

    /// Warm 層のエントリー数。
    #[must_use]
    pub fn warm_len(&self) -> usize {
        self.warm.len()
    }

    /// 全エントリー数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.hot.len() + self.warm.len()
    }

    /// 空か。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hot.is_empty() && self.warm.is_empty()
    }

    /// キーが存在するか。
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.hot.contains_key(key) || self.warm.contains_key(key)
    }

    /// 昇格回数。
    #[must_use]
    pub const fn promotions(&self) -> u64 {
        self.promotions
    }

    /// 降格回数。
    #[must_use]
    pub const fn demotions(&self) -> u64 {
        self.demotions
    }

    /// 退去回数。
    #[must_use]
    pub const fn evictions(&self) -> u64 {
        self.evictions
    }

    /// 全エントリーをクリア。
    pub fn clear(&mut self) {
        self.hot.clear();
        self.warm.clear();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn small_config() -> TieredConfig {
        TieredConfig {
            hot_capacity: 3,
            warm_capacity: 5,
            promote_threshold: 2,
        }
    }

    #[test]
    fn put_and_get() {
        let mut cache = TieredCache::new(small_config());
        cache.put("k1", vec![1, 2, 3]);
        assert_eq!(cache.get("k1"), Some(vec![1, 2, 3]));
    }

    #[test]
    fn get_miss() {
        let mut cache = TieredCache::new(small_config());
        assert!(cache.get("nope").is_none());
    }

    #[test]
    fn remove_existing() {
        let mut cache = TieredCache::new(small_config());
        cache.put("k1", vec![1]);
        assert!(cache.remove("k1"));
        assert!(cache.get("k1").is_none());
    }

    #[test]
    fn remove_nonexistent() {
        let mut cache = TieredCache::new(small_config());
        assert!(!cache.remove("nope"));
    }

    #[test]
    fn spill_to_warm() {
        let mut cache = TieredCache::new(small_config());
        cache.put("a", vec![1]);
        cache.put("b", vec![2]);
        cache.put("c", vec![3]);
        // Hot is full, next insert should spill
        cache.put("d", vec![4]);
        assert_eq!(cache.hot_len(), 3);
        assert_eq!(cache.warm_len(), 1);
        assert_eq!(cache.demotions(), 1);
    }

    #[test]
    fn promote_from_warm() {
        let mut cache = TieredCache::new(small_config());
        cache.put("a", vec![1]);
        cache.put("b", vec![2]);
        cache.put("c", vec![3]);
        cache.put("d", vec![4]); // "a" spills to warm

        // "a" は warm にいるはず。2回アクセスで昇格
        let _ = cache.get("a"); // access_count = 1
        let _ = cache.get("a"); // access_count = 2 → 昇格
        assert_eq!(cache.promotions(), 1);
        assert!(cache.hot.contains_key("a"));
    }

    #[test]
    fn evict_warm() {
        let config = TieredConfig {
            hot_capacity: 2,
            warm_capacity: 2,
            promote_threshold: 10,
        };
        let mut cache = TieredCache::new(config);
        // Fill hot
        cache.put("a", vec![1]);
        cache.put("b", vec![2]);
        // Spill to warm
        cache.put("c", vec![3]); // a → warm
        cache.put("d", vec![4]); // b → warm, warm full
        cache.put("e", vec![5]); // c → warm, warm evicts oldest
        assert!(cache.evictions() > 0);
    }

    #[test]
    fn len_and_empty() {
        let mut cache = TieredCache::new(small_config());
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);

        cache.put("k1", vec![1]);
        assert!(!cache.is_empty());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn contains() {
        let mut cache = TieredCache::new(small_config());
        cache.put("k1", vec![1]);
        assert!(cache.contains("k1"));
        assert!(!cache.contains("k2"));
    }

    #[test]
    fn clear() {
        let mut cache = TieredCache::new(small_config());
        cache.put("a", vec![1]);
        cache.put("b", vec![2]);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn overwrite_existing_key() {
        let mut cache = TieredCache::new(small_config());
        cache.put("k1", vec![1]);
        cache.put("k1", vec![2]);
        assert_eq!(cache.get("k1"), Some(vec![2]));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn default_config() {
        let config = TieredConfig::default();
        assert_eq!(config.hot_capacity, 1000);
        assert_eq!(config.warm_capacity, 10000);
        assert_eq!(config.promote_threshold, 3);
    }

    #[test]
    fn with_defaults() {
        let cache = TieredCache::with_defaults();
        assert!(cache.is_empty());
    }
}
