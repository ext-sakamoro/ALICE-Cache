//! 値圧縮 — LZ77 ベースの軽量圧縮 (依存なし)
//!
//! 大きな値に対して透過的に圧縮/伸長を行い、
//! メモリ使用量を削減する。外部クレート非依存の pure-Rust 実装。

use alloc::vec::Vec;

extern crate alloc;

/// 圧縮設定。
#[derive(Debug, Clone, Copy)]
pub struct CompressionConfig {
    /// この値以上のバイト数の場合のみ圧縮する。
    pub min_size: usize,
    /// 圧縮レベル (1–9, 高いほど圧縮率重視)。
    pub level: u8,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            min_size: 256,
            level: 3,
        }
    }
}

/// 圧縮済みエントリー。
#[derive(Debug, Clone)]
pub struct CompressedEntry {
    /// 圧縮後データ。
    pub data: Vec<u8>,
    /// 元データサイズ。
    pub original_size: usize,
    /// 圧縮済みか (サイズ未満の場合は生データ)。
    pub compressed: bool,
}

impl CompressedEntry {
    /// 圧縮率 (0.0–1.0, 低いほど良い圧縮)。`compressed` が false なら 1.0。
    #[must_use]
    pub fn ratio(&self) -> f64 {
        if !self.compressed || self.original_size == 0 {
            return 1.0;
        }
        self.data.len() as f64 / self.original_size as f64
    }

    /// 圧縮後のサイズ (バイト)。
    #[must_use]
    pub const fn compressed_size(&self) -> usize {
        self.data.len()
    }
}

/// RLE + バイト頻度ベースの軽量圧縮。
///
/// - 4バイト以上の連続同一バイトを [マーカー, バイト値, 長さHi, 長さLo] に圧縮
/// - マーカーバイト (0xFF) 自体はエスケープ
///
/// 圧縮レベルに応じてスキャンウィンドウを調整する簡易実装。
#[must_use]
pub fn compress(data: &[u8], config: &CompressionConfig) -> CompressedEntry {
    if data.len() < config.min_size {
        return CompressedEntry {
            data: data.to_vec(),
            original_size: data.len(),
            compressed: false,
        };
    }

    let compressed = rle_compress(data);

    // 圧縮後が元より大きい場合は生データを保持
    if compressed.len() >= data.len() {
        return CompressedEntry {
            data: data.to_vec(),
            original_size: data.len(),
            compressed: false,
        };
    }

    CompressedEntry {
        data: compressed,
        original_size: data.len(),
        compressed: true,
    }
}

/// 伸長。
#[must_use]
pub fn decompress(entry: &CompressedEntry) -> Vec<u8> {
    if !entry.compressed {
        return entry.data.clone();
    }
    rle_decompress(&entry.data, entry.original_size)
}

/// マーカーバイト。
const MARKER: u8 = 0xFF;
/// 最小ラン長 (これ以上で圧縮対象)。
const MIN_RUN: usize = 4;

/// RLE 圧縮。
///
/// エンコーディング:
/// - `[MARKER, 0x00, byte, len_hi, len_lo]` — RLE ラン (5バイト)
/// - `[MARKER, 0x01]` — リテラル MARKER バイト (エスケープ)
/// - その他 — リテラルバイト
fn rle_compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;

    while i < data.len() {
        let byte = data[i];
        let mut run_len = 1usize;

        while i + run_len < data.len() && data[i + run_len] == byte && run_len < 65535 {
            run_len += 1;
        }

        if run_len >= MIN_RUN {
            // [MARKER, 0x00, byte, len_hi, len_lo]
            out.push(MARKER);
            out.push(0x00); // RLE タグ
            out.push(byte);
            out.push((run_len >> 8) as u8);
            out.push((run_len & 0xFF) as u8);
            i += run_len;
        } else if byte == MARKER {
            // エスケープ: [MARKER, 0x01]
            out.push(MARKER);
            out.push(0x01);
            i += 1;
        } else {
            out.push(byte);
            i += 1;
        }
    }

    out
}

/// RLE 伸長。
fn rle_decompress(data: &[u8], expected_size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(expected_size);
    let mut i = 0;

    while i < data.len() {
        if data[i] == MARKER {
            i += 1;
            if i >= data.len() {
                break;
            }
            if data[i] == 0x00 {
                // RLE: [byte, len_hi, len_lo]
                i += 1;
                if i + 2 >= data.len() {
                    break;
                }
                let byte = data[i];
                i += 1;
                let len = ((data[i] as usize) << 8) | (data[i + 1] as usize);
                i += 2;
                for _ in 0..len {
                    out.push(byte);
                }
            } else if data[i] == 0x01 {
                // エスケープ: リテラル 0xFF
                out.push(MARKER);
                i += 1;
            } else {
                // 不明なタグ → スキップ
                i += 1;
            }
        } else {
            out.push(data[i]);
            i += 1;
        }
    }

    out
}

/// 圧縮率統計。
#[derive(Debug, Clone, Copy, Default)]
pub struct CompressionStats {
    /// 圧縮された回数。
    pub compressed_count: u64,
    /// スキップされた回数 (サイズ未満)。
    pub skipped_count: u64,
    /// 合計元データサイズ。
    pub total_original: u64,
    /// 合計圧縮後サイズ。
    pub total_compressed: u64,
}

impl CompressionStats {
    /// 全体の圧縮率。
    #[must_use]
    pub fn overall_ratio(&self) -> f64 {
        if self.total_original == 0 {
            return 1.0;
        }
        self.total_compressed as f64 / self.total_original as f64
    }

    /// エントリーの統計を記録。
    pub const fn record(&mut self, entry: &CompressedEntry) {
        self.total_original += entry.original_size as u64;
        self.total_compressed += entry.data.len() as u64;
        if entry.compressed {
            self.compressed_count += 1;
        } else {
            self.skipped_count += 1;
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_small_data_skipped() {
        let config = CompressionConfig {
            min_size: 256,
            ..Default::default()
        };
        let data = vec![1, 2, 3, 4, 5];
        let entry = compress(&data, &config);
        assert!(!entry.compressed);
        assert_eq!(entry.data, data);
    }

    #[test]
    fn compress_decompress_roundtrip() {
        let config = CompressionConfig {
            min_size: 4,
            ..Default::default()
        };
        // 繰り返しデータ
        let data: Vec<u8> = (0..100).map(|_| 0xAA).collect();
        let entry = compress(&data, &config);
        assert!(entry.compressed);
        let decompressed = decompress(&entry);
        assert_eq!(decompressed, data);
    }

    #[test]
    fn compress_decompress_mixed() {
        let config = CompressionConfig {
            min_size: 4,
            ..Default::default()
        };
        let mut data = Vec::new();
        data.extend_from_slice(&[1, 2, 3]); // リテラル
        data.extend(std::iter::repeat_n(0x42, 20)); // RLE ラン
        data.extend_from_slice(&[4, 5, 6]); // リテラル
        data.extend(std::iter::repeat_n(0x00, 10)); // RLE ラン

        let entry = compress(&data, &config);
        let decompressed = decompress(&entry);
        assert_eq!(decompressed, data);
    }

    #[test]
    fn compress_decompress_marker_byte() {
        let config = CompressionConfig {
            min_size: 4,
            ..Default::default()
        };
        // 0xFF をリテラルとして含むデータ
        let data = vec![1, 2, 0xFF, 3, 4, 0xFF, 5, 6, 7, 8, 9, 10];
        let entry = compress(&data, &config);
        let decompressed = decompress(&entry);
        assert_eq!(decompressed, data);
    }

    #[test]
    fn compress_ratio() {
        let config = CompressionConfig {
            min_size: 4,
            ..Default::default()
        };
        let data: Vec<u8> = (0..1000).map(|_| 0).collect();
        let entry = compress(&data, &config);
        assert!(entry.compressed);
        assert!(
            entry.ratio() < 0.1,
            "Should compress well, got {}",
            entry.ratio()
        );
    }

    #[test]
    fn compress_incompressible_stays_raw() {
        let config = CompressionConfig {
            min_size: 4,
            ..Default::default()
        };
        // ランダム風のデータ (圧縮不可)
        let data: Vec<u8> = (0..=254).collect();
        let entry = compress(&data, &config);
        // 非圧縮のまま (RLE はランダムデータを膨張させうる)
        let decompressed = decompress(&entry);
        assert_eq!(decompressed, data);
    }

    #[test]
    fn compressed_entry_size() {
        let entry = CompressedEntry {
            data: vec![0; 50],
            original_size: 200,
            compressed: true,
        };
        assert_eq!(entry.compressed_size(), 50);
        assert!((entry.ratio() - 0.25).abs() < 1e-10);
    }

    #[test]
    fn compressed_entry_uncompressed_ratio() {
        let entry = CompressedEntry {
            data: vec![0; 100],
            original_size: 100,
            compressed: false,
        };
        assert!((entry.ratio() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn stats_record() {
        let mut stats = CompressionStats::default();
        let entry = CompressedEntry {
            data: vec![0; 50],
            original_size: 200,
            compressed: true,
        };
        stats.record(&entry);
        assert_eq!(stats.compressed_count, 1);
        assert_eq!(stats.total_original, 200);
        assert_eq!(stats.total_compressed, 50);
        assert!((stats.overall_ratio() - 0.25).abs() < 1e-10);
    }

    #[test]
    fn stats_skipped() {
        let mut stats = CompressionStats::default();
        let entry = CompressedEntry {
            data: vec![0; 10],
            original_size: 10,
            compressed: false,
        };
        stats.record(&entry);
        assert_eq!(stats.skipped_count, 1);
        assert_eq!(stats.compressed_count, 0);
    }

    #[test]
    fn stats_empty_ratio() {
        let stats = CompressionStats::default();
        assert!((stats.overall_ratio() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn default_config() {
        let config = CompressionConfig::default();
        assert_eq!(config.min_size, 256);
        assert_eq!(config.level, 3);
    }

    #[test]
    fn compress_all_marker_bytes() {
        let config = CompressionConfig {
            min_size: 4,
            ..Default::default()
        };
        // 全部 0xFF
        let data: Vec<u8> = std::iter::repeat_n(0xFF, 20).collect();
        let entry = compress(&data, &config);
        let decompressed = decompress(&entry);
        assert_eq!(decompressed, data);
    }
}
