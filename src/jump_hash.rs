//! Jump Consistent Hash (Optimized)
//!
//! Maps a 64-bit key to a bucket in range [0, `num_buckets`).
//! Google's algorithm for distributed systems.
//!
//! - **Time Complexity**: O(ln n) jumps, but extremely fast
//! - **Space Complexity**: O(1) - No ring structure needed
//! - **Property**: When `num_buckets` changes, only 1/n keys move

/// Jump Consistent Hash
///
/// Maps a key to one of `num_buckets` buckets with minimal disruption
/// when the number of buckets changes.
///
/// # Example
/// ```
/// use alice_cache::jump_hash;
///
/// let bucket = jump_hash(12345, 10);
/// assert!(bucket >= 0 && bucket < 10);
/// ```
/// Precomputed constant 2^31 for jump hash probability calculation
const TWO_POW_31: f64 = 2_147_483_648.0;

#[inline(always)]
#[must_use]
pub fn jump_hash(mut key: u64, num_buckets: i32) -> i32 {
    if num_buckets <= 0 {
        return 0;
    }

    let mut b = -1i64;
    let mut j = 0i64;

    while j < num_buckets as i64 {
        b = j;
        // Linear congruential generator
        key = key.wrapping_mul(2_862_933_555_777_941_757).wrapping_add(1);

        // Convert to float for probability calculation
        let key_float = ((key >> 33) + 1) as f64;

        // Reciprocal multiplication instead of division: (b+1) * (2^31 / key_float)
        let jump_prob = key_float.recip() * TWO_POW_31;
        j = ((b + 1) as f64 * jump_prob) as i64;
    }

    b as i32
}

/// Jump hash with u128 key support (for content-addressed storage)
#[inline]
#[must_use]
pub fn jump_hash_u128(key: u128, num_buckets: i32) -> i32 {
    // Mix high and low bits
    let mixed = (key as u64) ^ ((key >> 64) as u64);
    jump_hash(mixed, num_buckets)
}

/// Jump hash for byte slices (generic key support)
#[inline]
#[must_use]
pub fn jump_hash_bytes(key: &[u8], num_buckets: i32) -> i32 {
    let hash = fnv1a_hash(key);
    jump_hash(hash, num_buckets)
}

/// FNV-1a hash for byte slices
#[inline]
fn fnv1a_hash(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jump_hash_range() {
        for i in 0..1000 {
            let bucket = jump_hash(i, 10);
            assert!((0..10).contains(&bucket));
        }
    }

    #[test]
    fn test_jump_hash_consistency() {
        // Same key should always map to same bucket
        let key = 0xDEAD_BEEF_u64;
        let b1 = jump_hash(key, 100);
        let b2 = jump_hash(key, 100);
        assert_eq!(b1, b2);
    }

    #[test]
    fn test_jump_hash_minimal_movement() {
        // When adding a bucket, most keys should stay in place
        let mut moved = 0;
        let total = 10000;

        for key in 0..total {
            let old_bucket = jump_hash(key, 100);
            let new_bucket = jump_hash(key, 101);
            if old_bucket != new_bucket {
                moved += 1;
            }
        }

        // Approximately 1/101 ≈ 1% should move
        let move_ratio = moved as f64 / total as f64;
        assert!(
            move_ratio < 0.02,
            "Too many keys moved: {:.2}%",
            move_ratio * 100.0
        );
    }

    #[test]
    fn test_jump_hash_distribution() {
        // Check roughly uniform distribution
        let mut buckets = [0u32; 10];
        let total = 100_000;

        for key in 0..total {
            let b = jump_hash(key, 10) as usize;
            buckets[b] += 1;
        }

        // Each bucket should have roughly 10% (±2%)
        let expected = total / 10;
        for (i, &count) in buckets.iter().enumerate() {
            let deviation = (count as i64 - expected as i64).abs() as f64 / expected as f64;
            assert!(
                deviation < 0.1,
                "Bucket {i} has {count}, expected ~{expected} (deviation: {:.1}%)",
                deviation * 100.0
            );
        }
    }

    #[test]
    fn test_jump_hash_edge_cases() {
        assert_eq!(jump_hash(0, 1), 0);
        assert_eq!(jump_hash(u64::MAX, 1), 0);
        assert_eq!(jump_hash(0, 0), 0);
    }

    #[test]
    fn test_jump_hash_bytes() {
        let b1 = jump_hash_bytes(b"hello", 100);
        let b2 = jump_hash_bytes(b"hello", 100);
        assert_eq!(b1, b2);

        let b3 = jump_hash_bytes(b"world", 100);
        // Different keys likely different buckets (not guaranteed)
        assert!((0..100).contains(&b1));
        assert!((0..100).contains(&b3));
    }

    // ── Additional tests for quality improvement ──────────────────

    #[test]
    fn test_jump_hash_negative_buckets() {
        // Negative num_buckets should return 0
        assert_eq!(jump_hash(12345, -1), 0);
        assert_eq!(jump_hash(12345, -100), 0);
    }

    #[test]
    fn test_jump_hash_single_bucket() {
        // With 1 bucket, all keys map to 0
        for key in 0..1000u64 {
            assert_eq!(jump_hash(key, 1), 0);
        }
    }

    #[test]
    fn test_jump_hash_u128() {
        let b1 = jump_hash_u128(0xDEAD_BEEF_CAFE_BABE_1234_5678_9ABC_DEF0u128, 100);
        let b2 = jump_hash_u128(0xDEAD_BEEF_CAFE_BABE_1234_5678_9ABC_DEF0u128, 100);
        assert_eq!(b1, b2);
        assert!((0..100).contains(&b1));
    }

    #[test]
    fn test_jump_hash_u128_range() {
        for i in 0..1000u128 {
            let bucket = jump_hash_u128(i, 10);
            assert!((0..10).contains(&bucket));
        }
    }

    #[test]
    fn test_fnv1a_hash_deterministic() {
        let h1 = fnv1a_hash(b"test data");
        let h2 = fnv1a_hash(b"test data");
        assert_eq!(h1, h2);

        let h3 = fnv1a_hash(b"different data");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_fnv1a_hash_empty_input() {
        // Empty input should return the FNV offset basis
        let h = fnv1a_hash(b"");
        assert_eq!(h, 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn test_jump_hash_bytes_empty_key() {
        // Empty key should still produce a valid bucket
        let bucket = jump_hash_bytes(b"", 10);
        assert!((0..10).contains(&bucket));
    }

    #[test]
    fn test_jump_hash_two_buckets() {
        // With 2 buckets, results should be 0 or 1
        let mut count_0 = 0;
        let mut count_1 = 0;
        for key in 0..10000u64 {
            match jump_hash(key, 2) {
                0 => count_0 += 1,
                1 => count_1 += 1,
                other => panic!("unexpected bucket: {other}"),
            }
        }
        // Both buckets should have some keys
        assert!(count_0 > 3000);
        assert!(count_1 > 3000);
    }
}
