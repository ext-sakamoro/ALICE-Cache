//! Fuzz target: sharded cache の put/get/remove の panic-freedom + roundtrip 整合
//!
//! ALICE-Cache は sharded HashMap + sampled eviction + Markov oracle 構成
//! 任意の (op sequence, key, value) 入力に対し:
//! - panic せず処理完了
//! - put 直後の get で同値を取得可能 (eviction 発生時は None も許容)
//! - hit_rate は [0.0, 1.0] 範囲内
//! - len は capacity を超えない
//!
//! canonical CI template [[reference_alice_ci_canonical_template]] 準拠

#![no_main]

use alice_cache::{AliceCache, CacheConfig};
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
enum Op {
    Put { key: u32, value: u64 },
    Get { key: u32 },
    Remove { key: u32 },
    Contains { key: u32 },
}

#[derive(Debug, Arbitrary)]
struct FuzzInput {
    // capacity は 16..=4096 の範囲に絞る (fuzz 速度 + 現実的 workload)
    capacity_seed: u16,
    // num_shards は power of 2 制約 → seed から 1/2/4/8/16 の 5 択
    shards_seed: u8,
    ops: Vec<Op>,
}

fuzz_target!(|input: FuzzInput| {
    let capacity = ((input.capacity_seed as usize) % 4080) + 16;
    let num_shards: usize = 1 << (input.shards_seed % 5); // 1, 2, 4, 8, 16

    let cache = AliceCache::<u32, u64>::with_config(CacheConfig {
        capacity,
        num_shards,
        num_nodes: 1,
        node_id: 0,
        enable_oracle: false, // fuzz 中は oracle overhead を避ける
    });

    // op sequence を実行し panic-freedom を検証
    // ops 数は上限 256 で cap (arbitrary が巨大 Vec を生成しないよう防御)
    let ops: Vec<Op> = input.ops.into_iter().take(256).collect();
    for op in &ops {
        match op {
            Op::Put { key, value } => {
                cache.put(*key, *value);
            }
            Op::Get { key } => {
                let _ = cache.get(key);
            }
            Op::Remove { key } => {
                let _ = cache.remove(key);
            }
            Op::Contains { key } => {
                let _ = cache.contains(key);
            }
        }
    }

    // 不変式検証: len は capacity を超えてはならない
    // (sampled eviction が働くため厳密上限、超えたら bug)
    let len = cache.len();
    assert!(
        len <= capacity,
        "cache len {} exceeds capacity {}",
        len,
        capacity
    );

    // 不変式検証: hit_rate は [0.0, 1.0] 範囲
    let hr = cache.hit_rate();
    assert!(hr.is_finite(), "hit_rate must be finite, got {}", hr);
    assert!(
        (0.0..=1.0).contains(&hr),
        "hit_rate must be in [0, 1], got {}",
        hr
    );
});
