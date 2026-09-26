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

use serde::{Deserialize, Serialize};

/// An opaque plan correlation token (e.g. 16-character hexadecimal hash).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlanId(pub String);

impl std::fmt::Display for PlanId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for PlanId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for PlanId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for PlanId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Algorithm-tagged cryptographic digest for patches, receipts, and artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactDigest {
    pub digest_algo: String,
    pub digest: String,
}

impl ArtifactDigest {
    /// Computes authentic 64-character SHA-256 digest over byte buffer.
    pub fn sha256(bytes: &[u8]) -> Self {
        Self {
            digest_algo: "sha256".to_string(),
            digest: sha256_hex(bytes),
        }
    }

    pub fn new(algo: impl Into<String>, digest: impl Into<String>) -> Self {
        Self {
            digest_algo: algo.into(),
            digest: digest.into(),
        }
    }
}

/// Computes authentic 64-character lowercase hexadecimal SHA-256 hash using the standard `sha2` crate.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:064x}", hasher.finalize())
}

/// Computes an opaque plan correlation token (16-character hex string).
pub fn compute_plan_id(json: &str) -> PlanId {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    json.hash(&mut h);
    PlanId(format!("{:016x}", h.finish()))
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

    #[test]
    fn authentic_sha256_matches_known_test_vectors() {
        // Standard NIST / RFC test vectors
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"hello world"),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn artifact_digest_sha256_tagged_correctly() {
        let digest = ArtifactDigest::sha256(b"patch payload");
        assert_eq!(digest.digest_algo, "sha256");
        assert_eq!(digest.digest.len(), 64);
        assert_eq!(digest.digest, sha256_hex(b"patch payload"));
    }

    #[test]
    fn plan_id_opaque_correlation() {
        let id1 = compute_plan_id("{\"nodes\":[]}");
        let id2 = compute_plan_id("{\"nodes\":[]}");
        assert_eq!(id1, id2);
        assert_eq!(id1.as_ref().len(), 16);
    }
}
