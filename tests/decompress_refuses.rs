//! What `decompress` reads and what it refuses.
//!
//! oracle: the entries are written out by hand from the format in `src/compression.rs`: a run is
//! `FF 00 byte len_hi len_lo`, an escaped `FF` is `FF 01`, and anything else is that byte. The bytes
//! `compress` writes are pinned by hand as well, so the format of existing entries stays.

use alice_cache::{compress, decompress, CompressedEntry, CompressionConfig, DecompressError};

fn entry(data: &[u8], original_size: usize) -> CompressedEntry {
    CompressedEntry {
        data: data.to_vec(),
        original_size,
        compressed: true,
    }
}

#[test]
fn the_bytes_of_compress_and_the_round_trip_stay() {
    let config = CompressionConfig {
        min_size: 0,
        level: 3,
    };
    // ten 7s are a run, three 8s are literals, FF is escaped, five FF are a run
    let data = [
        7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 0xFF, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ];
    let written = compress(&data, &config);
    assert!(written.compressed);
    assert_eq!(written.original_size, data.len());
    assert_eq!(
        written.data,
        [0xFF, 0, 7, 0, 10, 8, 8, 8, 0xFF, 1, 1, 0xFF, 0, 0xFF, 0, 5]
    );
    assert_eq!(decompress(&written).as_deref(), Ok(&data[..]));
}

#[test]
fn every_part_of_the_format_reads() {
    assert_eq!(decompress(&entry(&[], 0)), Ok(vec![]));
    assert_eq!(decompress(&entry(&[1, 2], 2)), Ok(vec![1, 2]));
    assert_eq!(decompress(&entry(&[0xFF, 1], 1)), Ok(vec![0xFF]));
    assert_eq!(decompress(&entry(&[0xFF, 0, 9, 0, 2], 2)), Ok(vec![9, 9]));
    // the high byte of the length counts: 0x0102 is 258
    assert_eq!(
        decompress(&entry(&[0xFF, 0, 9, 1, 2], 258)),
        Ok(vec![9; 258])
    );
    // a run that ends exactly at the limit, then the end of the data
    assert_eq!(
        decompress(&entry(&[5, 0xFF, 0, 9, 0, 2, 0xFF, 1, 6], 5)),
        Ok(vec![5, 9, 9, 0xFF, 6])
    );
}

#[test]
fn data_cut_anywhere_inside_a_run_or_an_escape_is_refused() {
    for cut in [&[0xFF][..], &[0xFF, 0], &[0xFF, 0, 9], &[0xFF, 0, 9, 0]] {
        assert_eq!(
            decompress(&entry(cut, 9)),
            Err(DecompressError::Truncated),
            "{cut:?}"
        );
    }
    assert_eq!(
        decompress(&entry(&[1, 0xFF], 9)),
        Err(DecompressError::Truncated)
    );
}

#[test]
fn a_tag_compress_does_not_write_is_refused() {
    for tag in [2, 0x7F, 0xFE, 0xFF] {
        assert_eq!(
            decompress(&entry(&[0xFF, tag, 1, 1, 1], 9)),
            Err(DecompressError::UnknownTag(tag))
        );
    }
}

#[test]
fn every_way_of_going_past_the_size_is_refused() {
    let too_long = |expected| Err(DecompressError::TooLong { expected });
    assert_eq!(decompress(&entry(&[1, 2], 1)), too_long(1));
    assert_eq!(decompress(&entry(&[0xFF, 1, 0xFF, 1], 1)), too_long(1));
    assert_eq!(decompress(&entry(&[0xFF, 0, 9, 0, 3], 2)), too_long(2));
    assert_eq!(decompress(&entry(&[1, 0xFF, 0, 9, 0, 3], 3)), too_long(3));
}

#[test]
fn an_entry_that_is_not_compressed_has_its_length_checked_too() {
    let raw = |original_size| CompressedEntry {
        data: vec![0xFF, 1, 2],
        original_size,
        compressed: false,
    };
    assert_eq!(decompress(&raw(3)), Ok(vec![0xFF, 1, 2]));
    assert_eq!(
        decompress(&raw(2)),
        Err(DecompressError::TooLong { expected: 2 })
    );
    assert_eq!(
        decompress(&raw(4)),
        Err(DecompressError::TooShort {
            expected: 4,
            actual: 3
        })
    );
}

#[test]
fn the_errors_read() {
    assert_eq!(
        DecompressError::Truncated.to_string(),
        "compressed data is cut off"
    );
    assert_eq!(
        DecompressError::UnknownTag(2).to_string(),
        "unknown tag 0x02 in compressed data"
    );
    assert_eq!(
        DecompressError::TooLong { expected: 3 }.to_string(),
        "output is longer than 3 bytes"
    );
    assert_eq!(
        DecompressError::TooShort {
            expected: 3,
            actual: 1
        }
        .to_string(),
        "output is 1 bytes, not 3"
    );
}
