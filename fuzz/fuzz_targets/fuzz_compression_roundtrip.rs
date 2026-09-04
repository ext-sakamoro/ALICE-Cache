//! Fuzz target: RLE compress/decompress の bit-exact roundtrip
//!
//! ALICE-Cache は独自 RLE + マーカーエスケープ圧縮を提供
//! 任意 bytes 入力に対し:
//! - compress → decompress で bit-exact に復元
//! - min_size 未満は compressed = false で生データ保持
//! - CompressedEntry::ratio() は finite かつ [0, +inf) 範囲
//!
//! canonical CI template [[reference_alice_ci_canonical_template]] 準拠

#![no_main]

use alice_cache::{compress, decompress, CompressionConfig};
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
struct FuzzInput {
    // 入力サイズは 64KB で cap (fuzz 実行時間の暴走防止)
    data: Vec<u8>,
    min_size_seed: u16, // 0..=65535 → min_size に mapping
    level: u8,
}

fuzz_target!(|input: FuzzInput| {
    let data: &[u8] = if input.data.len() > 65536 {
        &input.data[..65536]
    } else {
        &input.data
    };

    let config = CompressionConfig {
        min_size: (input.min_size_seed as usize) % 1024, // 0..=1023 で現実的な閾値
        level: (input.level % 9) + 1,                    // 1..=9
    };

    // compress は panic せず
    let entry = compress(data, &config);

    // ratio は finite かつ非負
    let ratio = entry.ratio();
    assert!(
        ratio.is_finite(),
        "compression ratio must be finite, got {}",
        ratio
    );
    assert!(
        ratio >= 0.0,
        "compression ratio must be non-negative, got {}",
        ratio
    );

    // roundtrip: compress → decompress で bit-exact
    let restored = decompress(&entry);
    assert_eq!(
        restored, data,
        "compression roundtrip must be bit-exact (compressed={})",
        entry.compressed
    );

    // original_size 検証
    assert_eq!(
        entry.original_size,
        data.len(),
        "original_size must match input"
    );

    // min_size 未満は生データ保持
    if data.len() < config.min_size {
        assert!(
            !entry.compressed,
            "data smaller than min_size must not be marked as compressed"
        );
    }
});
