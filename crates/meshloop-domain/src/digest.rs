//! Portable FNV-1a 64-bit. `DefaultHasher` is not cross-platform-stable; diagnostic
//! fingerprints and signature codes must be identical on Windows and WSL.

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// FNV-1a 64-bit over raw bytes. Deterministic across endianness because it walks `u8`.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// SplitMix64. Used as a data-oblivious stream for signed random projections.
pub fn splitmix64(mut state: u64) -> u64 {
    state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_the_offset_basis() {
        assert_eq!(fnv1a64(b""), FNV_OFFSET);
    }

    #[test]
    fn same_bytes_same_digest_and_codes_differ() {
        assert_eq!(fnv1a64(b"E0308"), fnv1a64(b"E0308"));
        assert_ne!(fnv1a64(b"E0308"), fnv1a64(b"E0309"));
    }

    #[test]
    fn splitmix_is_deterministic() {
        assert_eq!(splitmix64(1), splitmix64(1));
        assert_ne!(splitmix64(1), splitmix64(2));
    }
}
