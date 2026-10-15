//! 固定 seed のハッシュ (乱数を使わない)
//!
//! `ahash::RandomState::new()` は OS の乱数で seed するので、同じ操作列でも
//! 実行ごとに key の shard 配置が変わり、どの key が追い出されずに残るかが変わる
//! また ahash は CPU の AES 命令の有無で別の関数に切り替わるので、seed を固定しても
//! target を跨いで同じ値にならない
//!
//! このモジュールのハッシュは seed と入力 byte だけで決まる
//! - 入力は常に little-endian の byte 列として畳む (`usize` は `u64` に広げる)
//!   ので、32 bit / 64 bit、little / big endian の target で同じ値になる
//! - 時刻・乱数・アドレスを読まない

use core::hash::{BuildHasher, Hasher};

/// 既定の seed (`AliceCache::new` / `AliceCache::with_config` が使う)
pub const DEFAULT_SEED: u64 = 0xA11C_E0CA_C4E5_EED1;

/// shard 内の lookup 表の seed を cache の seed から作る時の定数
///
/// cache は hash の下位 bit で shard を選ぶ 表が同じ関数を使うと shard 内の全 key の
/// 下位 bit が同じになり、hashbrown (下位 bit で bucket を選ぶ) の探索が 1 点に偏る
/// 表の関数が cache の関数と、どの seed でも (`DEFAULT_SEED` でも) 別になるよう
/// seed を splitmix64 に通してこの定数と xor する
pub const TABLE_SEED_SALT: u64 = 0x7AB1_E5EE_D5A1_7ED3;

const MUL: u64 = 0x9E37_79B9_7F4A_7C15;

/// splitmix64 の 1 step (seed から 2 本の鍵を作る)
const fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// murmur3 の最終混合 (下位 bit が shard 番号になるので全 bit を混ぜる)
const fn fmix64(mut k: u64) -> u64 {
    k ^= k >> 33;
    k = k.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    k ^= k >> 33;
    k = k.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    k ^ (k >> 33)
}

/// seed から決まる `BuildHasher`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeededState {
    k0: u64,
    k1: u64,
}

impl SeededState {
    /// seed を指定して作る (同じ seed なら同じ関数)
    #[must_use]
    pub const fn with_seed(seed: u64) -> Self {
        let k0 = splitmix64(seed);
        Self {
            k0,
            k1: splitmix64(k0),
        }
    }
}

impl SeededState {
    /// `seed` の cache の shard 内 lookup 表が使う関数 (cache 自身の
    /// [`SeededState::with_seed`] とは別の関数、[`TABLE_SEED_SALT`] 参照)
    #[must_use]
    pub const fn table_for(seed: u64) -> Self {
        Self::with_seed(splitmix64(seed) ^ TABLE_SEED_SALT)
    }
}

impl Default for SeededState {
    fn default() -> Self {
        Self::with_seed(DEFAULT_SEED)
    }
}

impl BuildHasher for SeededState {
    type Hasher = SeededHasher;

    #[inline]
    fn build_hasher(&self) -> SeededHasher {
        SeededHasher {
            state: self.k0,
            k1: self.k1,
            len: 0,
        }
    }
}

/// `SeededState` が作るハッシャ
#[derive(Debug, Clone)]
pub struct SeededHasher {
    state: u64,
    k1: u64,
    len: u64,
}

impl SeededHasher {
    #[inline(always)]
    fn absorb(&mut self, word: u64) {
        self.state = (self.state ^ word).wrapping_mul(MUL).rotate_left(29);
    }
}

impl Hasher for SeededHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        self.len = self.len.wrapping_add(bytes.len() as u64);
        let mut rest = bytes;
        while rest.len() >= 8 {
            let (head, tail) = rest.split_at(8);
            let mut w = [0u8; 8];
            w.copy_from_slice(head);
            self.absorb(u64::from_le_bytes(w));
            rest = tail;
        }
        if !rest.is_empty() {
            let mut w = [0u8; 8];
            w[..rest.len()].copy_from_slice(rest);
            self.absorb(u64::from_le_bytes(w));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.write(&[i]);
    }
    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.write(&i.to_le_bytes());
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.write(&i.to_le_bytes());
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.len = self.len.wrapping_add(8);
        self.absorb(i);
    }
    #[inline]
    fn write_u128(&mut self, i: u128) {
        self.write(&i.to_le_bytes());
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        fmix64(self.state ^ self.k1 ^ self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::hash::Hash;

    fn h<T: Hash + ?Sized>(s: SeededState, v: &T) -> u64 {
        s.hash_one(v)
    }

    #[test]
    fn same_seed_same_hash() {
        let a = SeededState::with_seed(7);
        let b = SeededState::with_seed(7);
        assert_eq!(h(a, &123u64), h(b, &123u64));
        assert_eq!(h(a, "abc"), h(b, "abc"));
    }

    #[test]
    fn different_seed_different_hash() {
        let a = SeededState::with_seed(1);
        let b = SeededState::with_seed(2);
        assert_ne!(h(a, &123u64), h(b, &123u64));
    }

    #[test]
    fn write_u64_equals_byte_stream() {
        let s = SeededState::default();
        let mut x = s.build_hasher();
        x.write_u64(0x0102_0304_0506_0708);
        let mut y = s.build_hasher();
        y.write(&0x0102_0304_0506_0708u64.to_le_bytes());
        assert_eq!(x.finish(), y.finish());
    }

    #[test]
    fn usize_hashes_as_u64() {
        let s = SeededState::default();
        assert_eq!(h(s, &5usize), h(s, &5u64));
    }

    #[test]
    fn length_is_part_of_the_hash() {
        let s = SeededState::default();
        assert_ne!(h(s, &[0u8; 3][..]), h(s, &[0u8; 4][..]));
    }

    #[test]
    fn low_bits_are_spread() {
        // shard 番号は下位 bit: 連番 key が 16 shard にほぼ均等に散る
        let s = SeededState::default();
        let mut buckets = [0u32; 16];
        for k in 0..16_000u64 {
            buckets[(h(s, &k) & 15) as usize] += 1;
        }
        for b in buckets {
            assert!((800..1200).contains(&b), "bucket {b}");
        }
    }
}
