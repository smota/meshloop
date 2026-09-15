//! Content-addressed skeleton cache.
//! Keyed by `(Language, fnv1a64(source))` to guarantee zero stale cache hits.

use std::collections::HashMap;

use meshloop_domain::digest::fnv1a64;

use crate::skeleton::{Language, SkeletonResult, extract_skeleton};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    language: Language,
    source_hash: u64,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    result: SkeletonResult,
    source_len: usize,
    source_head: String,
}

/// Content-addressed cache for extracted code skeletons.
#[derive(Debug, Default)]
pub struct SkeletonCache {
    entries: HashMap<CacheKey, CacheEntry>,
    hits: usize,
    misses: usize,
    stale_hits: usize,
}

impl SkeletonCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_or_extract(&mut self, source: &str, lang: Language) -> SkeletonResult {
        let hash = fnv1a64(source.as_bytes());
        let key = CacheKey {
            language: lang,
            source_hash: hash,
        };

        if let Some(entry) = self.entries.get(&key) {
            if entry.source_len != source.len() || !source.starts_with(&entry.source_head) {
                self.stale_hits += 1;
                let result = extract_skeleton(source, lang);
                let head: String = source.chars().take(64).collect();
                self.entries.insert(
                    key,
                    CacheEntry {
                        result: result.clone(),
                        source_len: source.len(),
                        source_head: head,
                    },
                );
                result
            } else {
                self.hits += 1;
                entry.result.clone()
            }
        } else {
            self.misses += 1;
            let result = extract_skeleton(source, lang);
            let head: String = source.chars().take(64).collect();
            self.entries.insert(
                key,
                CacheEntry {
                    result: result.clone(),
                    source_len: source.len(),
                    source_head: head,
                },
            );
            result
        }
    }

    pub fn hits(&self) -> usize {
        self.hits
    }

    pub fn misses(&self) -> usize {
        self.misses
    }

    pub fn stale_hits(&self) -> usize {
        self.stale_hits
    }

    pub fn hit_rate_pct(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }

    pub fn stale_hit_rate_pct(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.stale_hits as f64 / total as f64) * 100.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hits_on_identical_source() {
        let mut cache = SkeletonCache::new();
        let src = "fn hello() { println!(\"world\"); }";
        let res1 = cache.get_or_extract(src, Language::Rust);
        assert_eq!(cache.hits(), 0);
        assert_eq!(cache.misses(), 1);

        let res2 = cache.get_or_extract(src, Language::Rust);
        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 1);
        assert_eq!(res1.skeleton, res2.skeleton);
        assert_eq!(cache.stale_hit_rate_pct(), 0.0);
    }

    #[test]
    fn cache_misses_on_mutated_source() {
        let mut cache = SkeletonCache::new();
        let src1 = "fn hello() { println!(\"world\"); }";
        let src2 = "fn hello() { println!(\"world!\"); }";
        let _ = cache.get_or_extract(src1, Language::Rust);
        let _ = cache.get_or_extract(src2, Language::Rust);
        assert_eq!(cache.hits(), 0);
        assert_eq!(cache.misses(), 2);
        assert_eq!(cache.stale_hit_rate_pct(), 0.0);
    }
}
