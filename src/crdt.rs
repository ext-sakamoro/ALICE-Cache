//! CRDT キャッシュ無効化 — G-Set ベースの分散キー無効化
//!
//! Grow-Only Set (G-Set) CRDT を用いて、複数ノード間で
//! キャッシュ無効化イベントを収束的に同期する。

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

extern crate alloc;

/// 論理クロック (Hybrid Logical Clock 簡易版)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CrdtClock {
    /// 論理タイムスタンプ。
    pub timestamp: u64,
    /// ノード内カウンター (同一タイムスタンプ内の順序付け)。
    pub counter: u32,
}

impl CrdtClock {
    /// 新しいクロックを作成。
    #[must_use]
    pub const fn new(timestamp: u64, counter: u32) -> Self {
        Self { timestamp, counter }
    }

    /// クロックを進める。
    #[must_use]
    pub const fn tick(self) -> Self {
        Self {
            timestamp: self.timestamp,
            counter: self.counter + 1,
        }
    }

    /// リモートクロックとマージ (最大値を取る)。
    #[must_use]
    pub const fn merge(self, other: Self) -> Self {
        let ts = if self.timestamp > other.timestamp {
            self.timestamp
        } else {
            other.timestamp
        };
        let ctr = if self.timestamp == other.timestamp {
            if self.counter > other.counter {
                self.counter
            } else {
                other.counter
            }
        } else if self.timestamp > other.timestamp {
            self.counter
        } else {
            other.counter
        };
        Self {
            timestamp: ts,
            counter: ctr + 1,
        }
    }
}

impl Default for CrdtClock {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

/// キー無効化イベント。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidationEntry {
    /// 無効化されたキー。
    pub key: String,
    /// 無効化時のクロック。
    pub clock: CrdtClock,
    /// 発行ノードID。
    pub node_id: u32,
}

/// CRDT 無効化ログ — G-Set (Grow-Only Set) ベース。
///
/// キーごとに最新の無効化クロックを保持する。
/// マージは冪等かつ可換 (CRDT 特性)。
#[derive(Debug, Clone, Default)]
pub struct InvalidationLog {
    /// キー → (最新クロック, ノードID)。
    entries: BTreeMap<String, (CrdtClock, u32)>,
    /// ローカルクロック。
    clock: CrdtClock,
    /// ノードID。
    node_id: u32,
}

impl InvalidationLog {
    /// 新しいログを作成。
    #[must_use]
    pub const fn new(node_id: u32) -> Self {
        Self {
            entries: BTreeMap::new(),
            clock: CrdtClock::new(0, 0),
            node_id,
        }
    }

    /// キーを無効化。
    pub fn invalidate(&mut self, key: &str) -> CrdtClock {
        self.clock = self.clock.tick();
        let clock = self.clock;
        self.entries.insert(key.into(), (clock, self.node_id));
        clock
    }

    /// キーが無効化済みか。
    #[must_use]
    pub fn is_invalid(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// キーの無効化クロックを取得。
    #[must_use]
    pub fn invalidation_clock(&self, key: &str) -> Option<CrdtClock> {
        self.entries.get(key).map(|(c, _)| *c)
    }

    /// キーが指定クロック以降に無効化されたか。
    ///
    /// キャッシュエントリーの挿入時クロックと比較して、
    /// 無効化が後発なら `true`。
    #[must_use]
    pub fn is_stale(&self, key: &str, insert_clock: CrdtClock) -> bool {
        if let Some((inv_clock, _)) = self.entries.get(key) {
            *inv_clock > insert_clock
        } else {
            false
        }
    }

    /// リモートログとマージ (CRDT merge)。
    ///
    /// 各キーについて、より新しいクロックを持つ方を採用する。
    pub fn merge(&mut self, remote: &Self) {
        self.clock = self.clock.merge(remote.clock);
        for (key, (remote_clock, remote_node)) in &remote.entries {
            match self.entries.get(key) {
                Some((local_clock, _)) if *local_clock >= *remote_clock => {
                    // ローカルが新しい→維持
                }
                _ => {
                    self.entries
                        .insert(key.clone(), (*remote_clock, *remote_node));
                }
            }
        }
    }

    /// 無効化エントリーのリストを取得。
    #[must_use]
    pub fn entries(&self) -> Vec<InvalidationEntry> {
        self.entries
            .iter()
            .map(|(key, (clock, node_id))| InvalidationEntry {
                key: key.clone(),
                clock: *clock,
                node_id: *node_id,
            })
            .collect()
    }

    /// 無効化エントリー数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 空か。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 指定クロック以前の古いエントリーをパージ。
    pub fn purge_before(&mut self, threshold: CrdtClock) {
        self.entries.retain(|_, (clock, _)| *clock >= threshold);
    }

    /// 現在のクロック。
    #[must_use]
    pub const fn current_clock(&self) -> CrdtClock {
        self.clock
    }

    /// ノードID。
    #[must_use]
    pub const fn node_id(&self) -> u32 {
        self.node_id
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_tick() {
        let c = CrdtClock::new(1, 0);
        let c2 = c.tick();
        assert_eq!(c2.counter, 1);
        assert_eq!(c2.timestamp, 1);
    }

    #[test]
    fn clock_merge_same_ts() {
        let a = CrdtClock::new(5, 3);
        let b = CrdtClock::new(5, 7);
        let m = a.merge(b);
        assert_eq!(m.timestamp, 5);
        assert_eq!(m.counter, 8); // max(3,7) + 1
    }

    #[test]
    fn clock_merge_different_ts() {
        let a = CrdtClock::new(3, 10);
        let b = CrdtClock::new(5, 2);
        let m = a.merge(b);
        assert_eq!(m.timestamp, 5);
        assert_eq!(m.counter, 3); // b.counter + 1
    }

    #[test]
    fn clock_ordering() {
        let a = CrdtClock::new(1, 0);
        let b = CrdtClock::new(2, 0);
        assert!(a < b);
    }

    #[test]
    fn log_invalidate_and_check() {
        let mut log = InvalidationLog::new(1);
        log.invalidate("key1");
        assert!(log.is_invalid("key1"));
        assert!(!log.is_invalid("key2"));
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn log_is_stale() {
        let mut log = InvalidationLog::new(1);
        let insert_clock = CrdtClock::new(0, 0);
        log.invalidate("key1");
        assert!(log.is_stale("key1", insert_clock));
        assert!(!log.is_stale("key2", insert_clock));
    }

    #[test]
    fn log_not_stale_if_inserted_after() {
        let mut log = InvalidationLog::new(1);
        log.invalidate("key1");
        // 挿入が無効化より後のクロック
        let insert_clock = CrdtClock::new(100, 0);
        assert!(!log.is_stale("key1", insert_clock));
    }

    #[test]
    fn log_merge_keeps_newer() {
        let mut log_a = InvalidationLog::new(1);
        let mut log_b = InvalidationLog::new(2);

        log_a.invalidate("key1"); // clock (0,1)
                                  // B で key1 を後で無効化
        log_b.clock = CrdtClock::new(10, 0);
        log_b.invalidate("key1"); // clock (10,1)
        log_b.invalidate("key2"); // clock (10,2)

        log_a.merge(&log_b);
        assert_eq!(log_a.len(), 2);
        // key1 は B のクロックが採用される
        let clock = log_a.invalidation_clock("key1").unwrap();
        assert_eq!(clock.timestamp, 10);
    }

    #[test]
    fn log_merge_is_commutative() {
        let mut log_a = InvalidationLog::new(1);
        let mut log_b = InvalidationLog::new(2);
        log_a.invalidate("x");
        log_b.invalidate("y");

        let mut a_copy = log_a.clone();
        let mut b_copy = log_b.clone();

        a_copy.merge(&log_b);
        b_copy.merge(&log_a);

        assert_eq!(a_copy.len(), b_copy.len());
        assert!(a_copy.is_invalid("x"));
        assert!(a_copy.is_invalid("y"));
        assert!(b_copy.is_invalid("x"));
        assert!(b_copy.is_invalid("y"));
    }

    #[test]
    fn log_merge_is_idempotent() {
        let mut log_a = InvalidationLog::new(1);
        let log_b = {
            let mut l = InvalidationLog::new(2);
            l.invalidate("key1");
            l
        };

        log_a.merge(&log_b);
        let len_after_first = log_a.len();
        log_a.merge(&log_b);
        assert_eq!(log_a.len(), len_after_first);
    }

    #[test]
    fn log_purge_before() {
        let mut log = InvalidationLog::new(1);
        log.invalidate("old");
        log.clock = CrdtClock::new(100, 0);
        log.invalidate("new");

        log.purge_before(CrdtClock::new(50, 0));
        assert_eq!(log.len(), 1);
        assert!(!log.is_invalid("old"));
        assert!(log.is_invalid("new"));
    }

    #[test]
    fn log_entries() {
        let mut log = InvalidationLog::new(1);
        log.invalidate("a");
        log.invalidate("b");
        let entries = log.entries();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn log_empty() {
        let log = InvalidationLog::new(1);
        assert!(log.is_empty());
        assert_eq!(log.node_id(), 1);
    }
}
