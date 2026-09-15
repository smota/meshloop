//! Data-oblivious 1-bit / 2-bit quantization of feature-hashed signatures.
//!
//! Implements the TurboQuant *ideas* that fit Meshloop (Zandieh et al., ICLR 2026)
//! without taking `turbovec` as a dependency:
//!
//! 1. Signed-random-projection embed (CountSketch) — no neural model.
//! 2. Normalized Walsh–Hadamard rotation — data-oblivious, no train step.
//! 3. Per-coordinate Lloyd-Max scalar quantizer for N(0, 1/d).
//! 4. Length-renormalized inner-product scoring (RaBitQ-style unbiased estimator).
//! 5. Online ingest: `add` never rebuilds existing codes.
//!
//! Safe Rust only (`forbid(unsafe_code)`). Linear scan is the right complexity for
//! a repository-scale index (10^3–10^5 files), not 10^7 embedding rows.

use std::collections::HashMap;
use std::path::Path;

use meshloop_domain::digest::{fnv1a64, splitmix64};

use crate::signature::{Signature, SignatureKind, extract_signatures, file_signature_blob};
use crate::skeleton::{Language, extract_skeleton};

/// Power of two so the Hadamard rotation is an involution up to scale.
pub const DIM: usize = 64;

/// Default seed. Same source always yields the same codes.
pub const DEFAULT_SEED: u64 = 0x4d455348_4c4f4f50; // "MESHLOOP"

/// Soft cap on AST skeletons injected into a prompt. Ranked, not truncated at random.
pub const DEFAULT_SKELETON_BUDGET: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitWidth {
    One,
    Two,
}

impl BitWidth {
    fn bits_per_dim(self) -> usize {
        match self {
            BitWidth::One => 1,
            BitWidth::Two => 2,
        }
    }

    fn packed_bytes(self) -> usize {
        DIM * self.bits_per_dim() / 8
    }
}

/// 2-bit Lloyd-Max centroids for N(0,1), scaled by 1/sqrt(d) at use.
/// Max (1960) / Lloyd (1982); four reconstruction levels.
const LLOYD_MAX_2BIT: [f32; 4] = [-1.510, -0.4528, 0.4528, 1.510];
const LLOYD_MAX_BOUNDARIES: [f32; 3] = [-0.9816, 0.0, 0.9816];

#[derive(Debug, Clone, PartialEq)]
pub struct PackedCode {
    pub bits: Vec<u8>,
    /// `1 / ⟨u, x̂⟩` so the inner-product estimator is unbiased after quantization shrinkage.
    pub scale: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexedSignature {
    pub path: String,
    pub kind: SignatureKind,
    pub name: String,
    pub code: PackedCode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScoredHit {
    pub path: String,
    pub name: String,
    pub kind: SignatureKind,
    pub score: f32,
}

/// Online quantized index. Adding a vector never rewrites another.
#[derive(Debug, Clone)]
pub struct SignatureIndex {
    bit_width: BitWidth,
    seed: u64,
    entries: Vec<IndexedSignature>,
}

impl SignatureIndex {
    pub fn new(bit_width: BitWidth) -> Self {
        Self::with_seed(bit_width, DEFAULT_SEED)
    }

    pub fn with_seed(bit_width: BitWidth, seed: u64) -> Self {
        Self {
            bit_width,
            seed,
            entries: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn bit_width(&self) -> BitWidth {
        self.bit_width
    }

    pub fn add_signature(&mut self, sig: &Signature) {
        let code = encode(&sig.text, self.seed, self.bit_width);
        self.entries.push(IndexedSignature {
            path: sig.path.clone(),
            kind: sig.kind,
            name: sig.name.clone(),
            code,
        });
    }

    pub fn add_skeleton(&mut self, path: &str, skeleton: &str) {
        let language = Language::from_path(Path::new(path));
        for sig in extract_signatures(path, skeleton, language) {
            self.add_signature(&sig);
        }
        // Always index the file path + skeleton blob so files without extracted
        // declarations remain retrievable (README, config, tests).
        let blob = format!("{path}\n{skeleton}");
        let code = encode(&blob, self.seed, self.bit_width);
        self.entries.push(IndexedSignature {
            path: path.to_string(),
            kind: SignatureKind::Other,
            name: path.to_string(),
            code,
        });
    }

    /// Total heap and inline bytes allocated for the index.
    pub fn memory_bytes(&self) -> usize {
        let mut bytes = std::mem::size_of::<Self>();
        for entry in &self.entries {
            bytes += std::mem::size_of::<IndexedSignature>();
            bytes += entry.path.len();
            bytes += entry.name.len();
            bytes += entry.code.bits.len();
        }
        bytes
    }

    /// Total bytes occupied strictly by the quantized feature vectors.
    pub fn quantized_bytes(&self) -> usize {
        self.entries.iter().map(|e| e.code.bits.len()).sum()
    }

    pub fn search(&self, query: &str, k: usize) -> Vec<ScoredHit> {
        if k == 0 || self.entries.is_empty() {
            return Vec::new();
        }
        let q = rotate_unit(query, self.seed);
        let mut scored: Vec<(f32, usize)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (score_query(&q, &e.code, self.bit_width), i))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
            .into_iter()
            .map(|(score, i)| {
                let e = &self.entries[i];
                ScoredHit {
                    path: e.path.clone(),
                    name: e.name.clone(),
                    kind: e.kind,
                    score,
                }
            })
            .collect()
    }

    /// Best score per file path, descending. Ties broken by path for determinism.
    pub fn rank_files(&self, query: &str, k: usize) -> Vec<(String, f32)> {
        let hits = self.search(query, self.entries.len().max(1));
        let mut best: HashMap<String, f32> = HashMap::new();
        for hit in hits {
            best.entry(hit.path)
                .and_modify(|s| {
                    if hit.score > *s {
                        *s = hit.score;
                    }
                })
                .or_insert(hit.score);
        }
        let mut files: Vec<(String, f32)> = best.into_iter().collect();
        files.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        files.truncate(k);
        files
    }
}

/// Pick up to `budget` skeletons for a prompt. `pin_prefixes` (allowed_paths) stay
/// first, in path order; the rest are filled from the quantized index.
pub fn select_context(
    query: &str,
    skeletons: &[(String, String)],
    pin_prefixes: &[String],
    budget: usize,
) -> Vec<(String, String)> {
    if skeletons.is_empty() || budget == 0 {
        return Vec::new();
    }
    let mut by_path: HashMap<&str, &str> = HashMap::new();
    for (path, body) in skeletons {
        by_path.insert(path.as_str(), body.as_str());
    }
    let mut selected: Vec<(String, String)> = Vec::new();
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut pins: Vec<&str> = by_path
        .keys()
        .copied()
        .filter(|p| path_pinned(p, pin_prefixes))
        .collect();
    pins.sort_unstable();
    for p in pins {
        if selected.len() >= budget {
            break;
        }
        if used.insert(p.to_string()) {
            selected.push((p.to_string(), by_path[p].to_string()));
        }
    }

    if selected.len() >= budget {
        return selected;
    }

    if skeletons.len() <= budget && pin_prefixes.is_empty() {
        let mut all = skeletons.to_vec();
        all.sort_by(|a, b| a.0.cmp(&b.0));
        return all;
    }

    let mut index = SignatureIndex::new(BitWidth::Two);
    for (path, body) in skeletons {
        index.add_skeleton(path, body);
    }
    let remaining = budget - selected.len();
    for (path, _) in index.rank_files(query, remaining + used.len()) {
        if selected.len() >= budget {
            break;
        }
        if used.insert(path.clone())
            && let Some(body) = by_path.get(path.as_str())
        {
            selected.push((path, (*body).to_string()));
        }
    }
    selected
}

fn path_pinned(path: &str, prefixes: &[String]) -> bool {
    if prefixes.is_empty() {
        return false;
    }
    let path = path.replace('\\', "/");
    prefixes.iter().any(|rule| {
        let rule = rule.replace('\\', "/");
        if rule.ends_with('/') {
            path.starts_with(&rule)
        } else {
            path == rule
        }
    })
}

pub fn encode(text: &str, seed: u64, bit_width: BitWidth) -> PackedCode {
    let mut rotated = embed(text, seed);
    fwht(&mut rotated);
    let unit = l2_normalize(rotated);
    quantize(&unit, bit_width)
}

fn embed(text: &str, seed: u64) -> [f32; DIM] {
    let mut v = [0.0f32; DIM];
    for tok in tokenize(text) {
        let mixed = splitmix64(seed ^ fnv1a64(tok.as_bytes()));
        let dim = (mixed as usize) % DIM;
        let sign = if mixed & 1 == 0 { 1.0 } else { -1.0 };
        v[dim] += sign;
    }
    v
}

fn tokenize(text: &str) -> impl Iterator<Item = String> {
    text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_ascii_lowercase())
}

fn fwht(a: &mut [f32; DIM]) {
    let mut h = 1usize;
    while h < DIM {
        let mut i = 0usize;
        while i < DIM {
            for j in i..i + h {
                let x = a[j];
                let y = a[j + h];
                a[j] = x + y;
                a[j + h] = x - y;
            }
            i += h * 2;
        }
        h *= 2;
    }
    let scale = (DIM as f32).sqrt().recip();
    for x in a.iter_mut() {
        *x *= scale;
    }
}

fn l2_normalize(mut v: [f32; DIM]) -> [f32; DIM] {
    let mut ss = 0.0f32;
    for x in v {
        ss += x * x;
    }
    if ss > 0.0 {
        let inv = ss.sqrt().recip();
        for x in &mut v {
            *x *= inv;
        }
    }
    v
}

fn rotate_unit(text: &str, seed: u64) -> [f32; DIM] {
    let mut rotated = embed(text, seed);
    fwht(&mut rotated);
    l2_normalize(rotated)
}

fn quantize(unit: &[f32; DIM], bit_width: BitWidth) -> PackedCode {
    let sigma = (DIM as f32).sqrt().recip();
    let mut bits = vec![0u8; bit_width.packed_bytes()];
    let mut reconstructed = [0.0f32; DIM];
    match bit_width {
        BitWidth::One => {
            for (i, &x) in unit.iter().enumerate() {
                let bit = u8::from(x >= 0.0);
                bits[i / 8] |= bit << (i % 8);
                reconstructed[i] = if bit == 1 { sigma } else { -sigma };
            }
        }
        BitWidth::Two => {
            for (i, &x) in unit.iter().enumerate() {
                let z = x / sigma;
                let level = quantize_2bit(z);
                let byte = i / 4;
                let shift = (i % 4) * 2;
                bits[byte] |= level << shift;
                reconstructed[i] = LLOYD_MAX_2BIT[level as usize] * sigma;
            }
        }
    }
    let mut ip = 0.0f32;
    for (x, r) in unit.iter().zip(reconstructed.iter()) {
        ip += x * r;
    }
    let scale = if ip.abs() > 1e-12 { ip.recip() } else { 1.0 };
    PackedCode { bits, scale }
}

fn quantize_2bit(z: f32) -> u8 {
    if z < LLOYD_MAX_BOUNDARIES[0] {
        0
    } else if z < LLOYD_MAX_BOUNDARIES[1] {
        1
    } else if z < LLOYD_MAX_BOUNDARIES[2] {
        2
    } else {
        3
    }
}

fn score_query(query: &[f32; DIM], code: &PackedCode, bit_width: BitWidth) -> f32 {
    let sigma = (DIM as f32).sqrt().recip();
    let mut ip = 0.0f32;
    match bit_width {
        BitWidth::One => {
            for (i, q) in query.iter().enumerate() {
                let bit = (code.bits[i / 8] >> (i % 8)) & 1;
                let centroid = if bit == 1 { sigma } else { -sigma };
                ip += q * centroid;
            }
        }
        BitWidth::Two => {
            for (i, q) in query.iter().enumerate() {
                let level = (code.bits[i / 4] >> ((i % 4) * 2)) & 0b11;
                ip += q * LLOYD_MAX_2BIT[level as usize] * sigma;
            }
        }
    }
    ip * code.scale
}

/// Hash of concatenated, sorted signatures of one file. Used by syntactic slicing.
pub fn signature_fingerprint(path: &str, source: &str) -> u64 {
    let language = Language::from_path(Path::new(path));
    let skeleton = extract_skeleton(source, language);
    let sigs = extract_signatures(path, &skeleton.skeleton, language);
    fnv1a64(file_signature_blob(&sigs).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_is_deterministic_and_data_oblivious() {
        let a = encode(
            "pub trait Auth { fn verify(&self); }",
            DEFAULT_SEED,
            BitWidth::Two,
        );
        let b = encode(
            "pub trait Auth { fn verify(&self); }",
            DEFAULT_SEED,
            BitWidth::Two,
        );
        assert_eq!(a.bits, b.bits);
        assert!((a.scale - b.scale).abs() < 1e-6);
    }

    #[test]
    fn different_seeds_differ() {
        let a = encode("pub fn login()", 1, BitWidth::One);
        let b = encode("pub fn login()", 2, BitWidth::One);
        assert_ne!(a.bits, b.bits);
    }

    #[test]
    fn one_bit_code_is_eight_bytes() {
        let c = encode("fn foo()", DEFAULT_SEED, BitWidth::One);
        assert_eq!(c.bits.len(), 8);
    }

    #[test]
    fn two_bit_code_is_sixteen_bytes() {
        let c = encode("fn foo()", DEFAULT_SEED, BitWidth::Two);
        assert_eq!(c.bits.len(), 16);
    }

    #[test]
    fn fwht_twice_is_identity() {
        let mut v = embed("hello meshloop", DEFAULT_SEED);
        let original = v;
        fwht(&mut v);
        fwht(&mut v);
        for i in 0..DIM {
            assert!(
                (v[i] - original[i]).abs() < 1e-5,
                "dim {i}: {} vs {}",
                v[i],
                original[i]
            );
        }
    }

    #[test]
    fn online_add_does_not_rewrite_existing_codes() {
        let mut index = SignatureIndex::new(BitWidth::Two);
        index.add_skeleton("src/auth.rs", "pub trait Auth { fn verify(&self); }");
        let before = index.entries[0].code.bits.clone();
        index.add_skeleton("src/db.rs", "pub struct Pool;");
        assert_eq!(index.entries[0].code.bits, before);
        assert!(index.len() >= 2);
    }

    #[test]
    fn search_ranks_the_matching_file_first() {
        let mut index = SignatureIndex::new(BitWidth::Two);
        index.add_skeleton(
            "src/auth.rs",
            "pub trait Auth { fn verify(&self, token: &str) -> bool; }",
        );
        index.add_skeleton("src/db.rs", "pub struct Pool { url: String }");
        index.add_skeleton("src/hash.rs", "pub fn sha256(bytes: &[u8]) -> [u8; 32];");
        let files = index.rank_files("implement Auth trait verify token", 3);
        assert_eq!(files[0].0, "src/auth.rs");
    }

    #[test]
    fn select_context_pins_allowed_paths_then_fills() {
        let mut skeletons = Vec::new();
        for i in 0..30 {
            skeletons.push((
                format!("src/n{i}.rs"),
                format!("pub fn n{i}() {{ /* ... */ }}"),
            ));
        }
        skeletons.push((
            "src/auth.rs".into(),
            "pub trait Auth { fn verify(&self); }".into(),
        ));
        let selected = select_context(
            "implement authentication Auth verify",
            &skeletons,
            &["src/auth.rs".into()],
            5,
        );
        assert_eq!(selected[0].0, "src/auth.rs");
        assert_eq!(selected.len(), 5);
    }
}
