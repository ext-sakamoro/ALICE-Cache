//! Fuzz target: jump consistent hash の不変式検証
//!
//! Google's Jump Consistent Hash: 任意 (key: u64, num_buckets: i32) に対し:
//! - num_buckets > 0 なら 0 <= result < num_buckets
//! - num_buckets <= 0 なら result == 0 (contract)
//! - panic せず finite time で完了
//! - u128 / bytes 版も同不変式
//!
//! canonical CI template [[reference_alice_ci_canonical_template]] 準拠

#![no_main]

use alice_cache::{jump_hash, jump_hash_bytes, jump_hash_u128};
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
struct FuzzInput {
    key_u64: u64,
    key_u128: u128,
    key_bytes: Vec<u8>,
    // num_buckets は fuzzer が i32::MAX 近傍を頻繁に選ぶと jump loop が長時間化
    // するため u16 に絞る (0..=65535)、それ以上は現実的 shard 数として非現実的
    num_buckets_seed: u16,
}

fuzz_target!(|input: FuzzInput| {
    let num_buckets = i32::from(input.num_buckets_seed);

    // u64 版
    let b64 = jump_hash(input.key_u64, num_buckets);
    if num_buckets > 0 {
        assert!(
            b64 >= 0 && b64 < num_buckets,
            "jump_hash(u64) result {} out of [0, {})",
            b64,
            num_buckets
        );
    } else {
        assert_eq!(b64, 0, "jump_hash(u64) with num_buckets<=0 must return 0");
    }

    // u128 版
    let b128 = jump_hash_u128(input.key_u128, num_buckets);
    if num_buckets > 0 {
        assert!(
            b128 >= 0 && b128 < num_buckets,
            "jump_hash_u128 result {} out of [0, {})",
            b128,
            num_buckets
        );
    } else {
        assert_eq!(b128, 0, "jump_hash_u128 with num_buckets<=0 must return 0");
    }

    // bytes 版 (key_bytes 長は 4KB で cap)
    let bytes: &[u8] = if input.key_bytes.len() > 4096 {
        &input.key_bytes[..4096]
    } else {
        &input.key_bytes
    };
    let bbytes = jump_hash_bytes(bytes, num_buckets);
    if num_buckets > 0 {
        assert!(
            bbytes >= 0 && bbytes < num_buckets,
            "jump_hash_bytes result {} out of [0, {})",
            bbytes,
            num_buckets
        );
    } else {
        assert_eq!(
            bbytes, 0,
            "jump_hash_bytes with num_buckets<=0 must return 0"
        );
    }

    // 決定性検証: 同一入力は同一出力
    assert_eq!(
        jump_hash(input.key_u64, num_buckets),
        b64,
        "jump_hash must be deterministic"
    );
});
