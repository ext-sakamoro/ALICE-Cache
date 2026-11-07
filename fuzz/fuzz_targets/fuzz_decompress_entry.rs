//! Fuzz target: `decompress` on any entry either makes exactly `original_size` bytes or refuses
//!
//! - any bytes under any `original_size`: no panic, and `Ok` has that length
//! - what `compress` wrote, cut anywhere: refused
//! - what `compress` wrote, under a size one off: refused

#![no_main]

use alice_cache::{compress, decompress, CompressedEntry, CompressionConfig, DecompressError};
use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
struct Input {
    data: Vec<u8>,
    original_size: u32,
    huge: bool,
    compressed: bool,
    cut: u32,
}

fuzz_target!(|input: Input| {
    let data = &input.data[..input.data.len().min(1 << 16)];
    let original_size = if input.huge {
        usize::MAX - input.original_size as usize
    } else {
        // a limit near what short data can make, so runs are written and refused both
        input.original_size as usize % (1 << 20)
    };
    let entry = CompressedEntry {
        data: data.to_vec(),
        original_size,
        compressed: input.compressed,
    };
    if let Ok(out) = decompress(&entry) {
        assert_eq!(out.len(), original_size);
    }

    // what compress wrote
    let config = CompressionConfig {
        min_size: 0,
        level: 3,
    };
    let written = compress(data, &config);
    if !written.compressed {
        return;
    }
    assert_eq!(decompress(&written).as_deref(), Ok(data));

    let cut = input.cut as usize % written.data.len();
    let shortened = CompressedEntry {
        data: written.data[..cut].to_vec(),
        ..written.clone()
    };
    assert!(decompress(&shortened).is_err(), "cut at {cut}");

    let over = CompressedEntry {
        original_size: written.original_size + 1,
        ..written.clone()
    };
    assert_eq!(
        decompress(&over),
        Err(DecompressError::TooShort {
            expected: written.original_size + 1,
            actual: written.original_size
        })
    );
    let under = CompressedEntry {
        original_size: written.original_size - 1,
        ..written.clone()
    };
    assert_eq!(
        decompress(&under),
        Err(DecompressError::TooLong {
            expected: written.original_size - 1
        })
    );
});
