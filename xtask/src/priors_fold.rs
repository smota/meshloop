//! The routing-prior fold of `/meshloop learn` (`cells/nodecode-meshloop/learn.lisp`,
//! `fold-priors` and `priors-json`), restated in Rust for the offline learning-plane
//! comparison (#140), which runs in CI with no nodecode.
//!
//! The cell is the learning plane; this is a copy of its fold, not a second definition.
//! It must give the same file the cell gives for the same `inspect --attempts` data,
//! except the `generated_at` and `generator` lines. `bench-learning --offline` checks
//! that against a proposal the real cell wrote over the same fixture
//! (`benches/learning-plane/offline/cell-fold.json`), and the unit test below checks it
//! against the cell's proposal of the live warm-up (`results/`). Change both or neither.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// Why an attempt is no evidence, in the order the fold tests them.
pub const EXCLUSION_REASONS: [&str; 5] = [
    "not_subprocess",
    "model_not_passed",
    "model_unverified",
    "cancelled",
    "no_check_result",
];

/// A node with no stored tier is routed as Tier2 (`run_loop.rs`).
const DEFAULT_TIER: &str = "Tier2";
const TIERS: [&str; 3] = ["Tier1", "Tier2", "Tier3"];
const MAX_MODEL_BYTES: usize = 256;
const MAX_HARNESS_CHARS: usize = 64;

/// One counted attempt: (graph, task, attempt, passed).
pub type Reference = (String, u64, u64, bool);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub harness: String,
    pub model_ref: String,
    pub tier: String,
    pub attempts: usize,
    pub successes: usize,
    pub provenance: Vec<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fold {
    pub graphs: Vec<String>,
    pub priors: Vec<Entry>,
    /// Every reason of [`EXCLUSION_REASONS`], in order, with its count.
    pub excluded: Vec<(&'static str, usize)>,
    pub counted: usize,
    pub duplicates: usize,
}

fn blank_or_control(c: char) -> bool {
    let code = c as u32;
    code <= 32
        || (0x7f..=0xa0).contains(&code)
        || matches!(
            code,
            0x1680 | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff
        )
        || (0x2000..=0x200a).contains(&code)
}

/// `model-usable-p`: #134's predicate, and not `unverified`.
fn model_usable(model: Option<&str>) -> bool {
    match model {
        Some(m) => {
            !m.is_empty()
                && m.len() <= MAX_MODEL_BYTES
                && !m.starts_with('-')
                && !m.chars().any(blank_or_control)
                && m != "unverified"
        }
        None => false,
    }
}

/// `attempt-exclusion`: the first reason that holds, or `None` when the attempt counts.
fn exclusion(attempt: &Value, state: Option<&str>) -> Option<&'static str> {
    if attempt["dispatch_mode"].as_str() != Some("subprocess") {
        Some("not_subprocess")
    } else if attempt["model_passed"] != Value::Bool(true) {
        Some("model_not_passed")
    } else if !model_usable(attempt["model_ref"].as_str()) {
        Some("model_unverified")
    } else if state == Some("Cancelled") {
        Some("cancelled")
    } else if !matches!(attempt["check_result"].as_str(), Some("passed" | "failed")) {
        Some("no_check_result")
    } else {
        None
    }
}

fn id_of(object: &Value, key: &str, what: &str, graph: &str) -> Result<u64, String> {
    object[key]
        .as_u64()
        .ok_or_else(|| format!("{what} in {graph} has no {key} meshloop could have written"))
}

fn node_tier(node: &Value, graph: &str) -> Result<String, String> {
    match &node["tier"] {
        Value::Null => Ok(DEFAULT_TIER.into()),
        Value::String(t) if TIERS.contains(&t.as_str()) => Ok(t.clone()),
        other => Err(format!(
            "node {} of {graph} has tier {other}, which a v1 priors file cannot carry",
            node["id"]
        )),
    }
}

/// `fold-priors`: RUNS is `(graph id, data object of inspect --attempts --graph G)`.
/// A graph named twice is folded once, from its first data.
pub fn fold(runs: &[(String, Value)]) -> Result<Fold, String> {
    let mut first: BTreeMap<&str, &Value> = BTreeMap::new();
    for (graph, data) in runs {
        first.entry(graph.as_str()).or_insert(data);
    }
    let mut seen: BTreeSet<(String, u64, u64)> = BTreeSet::new();
    let mut keys: BTreeMap<(String, String, String), Vec<Reference>> = BTreeMap::new();
    let mut excluded: Vec<(&'static str, usize)> =
        EXCLUSION_REASONS.iter().map(|r| (*r, 0)).collect();
    let (mut counted, mut duplicates) = (0, 0);
    for (graph, data) in &first {
        for node in data["nodes"].as_array().into_iter().flatten() {
            let task = id_of(node, "id", "a node", graph)?;
            let state = node["state"].as_str();
            for attempt in node["attempts"].as_array().into_iter().flatten() {
                let what = format!("an attempt of task {task}");
                let id = id_of(attempt, "attempt_id", &what, graph)?;
                if !seen.insert(((*graph).to_string(), task, id)) {
                    duplicates += 1;
                    continue;
                }
                if let Some(reason) = exclusion(attempt, state) {
                    if let Some(slot) = excluded.iter_mut().find(|(r, _)| *r == reason) {
                        slot.1 += 1;
                    }
                    continue;
                }
                let harness = attempt["harness"]
                    .as_str()
                    .filter(|h| (1..=MAX_HARNESS_CHARS).contains(&h.chars().count()))
                    .ok_or_else(|| {
                        format!(
                            "attempt {id} of task {task} in {graph} has no harness a v1 priors file can carry"
                        )
                    })?;
                let model = attempt["model_ref"].as_str().unwrap_or_default();
                let key = (
                    harness.to_string(),
                    model.to_string(),
                    node_tier(node, graph)?,
                );
                counted += 1;
                let passed = attempt["check_result"].as_str() == Some("passed");
                keys.entry(key)
                    .or_default()
                    .push(((*graph).to_string(), task, id, passed));
            }
        }
    }
    // BTreeMap orders the keys field by field and String compares by code point (UTF-8
    // byte order is code point order), as `key<` and `ref<` in learn.lisp do.
    let priors = keys
        .into_iter()
        .map(|((harness, model_ref, tier), mut refs)| {
            refs.sort();
            Entry {
                harness,
                model_ref,
                tier,
                attempts: refs.len(),
                successes: refs.iter().filter(|r| r.3).count(),
                provenance: refs,
            }
        })
        .collect();
    Ok(Fold {
        graphs: first.keys().map(|g| (*g).to_string()).collect(),
        priors,
        excluded,
        counted,
        duplicates,
    })
}

/// `json-quoted`: `"` and `\` escaped, control characters as `\uXXXX` (SBCL prints the
/// hex digits upper case), everything else as it is.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 32 || c as u32 == 127 => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `priors-json`: the v1 file, byte for byte as the cell writes it, but for the
/// generator, which names its writer.
pub fn to_json(fold: &Fold, generated_at: &str, generator: (&str, &str)) -> String {
    let mut out = String::new();
    out.push_str("{\n  \"schema\": \"meshloop/advisory-priors\",\n  \"version\": 1,\n");
    out.push_str(&format!("  \"generated_at\": {},\n", quoted(generated_at)));
    out.push_str(&format!(
        "  \"generator\": {{\"name\": {}, \"version\": {}}},\n",
        quoted(generator.0),
        quoted(generator.1)
    ));
    let graphs: Vec<String> = fold.graphs.iter().map(|g| quoted(g)).collect();
    out.push_str(&format!(
        "  \"source\": {{\"graph_ids\": [{}]}},\n",
        graphs.join(", ")
    ));
    if fold.priors.is_empty() {
        out.push_str("  \"priors\": []\n");
    } else {
        out.push_str("  \"priors\": [\n");
        for (i, e) in fold.priors.iter().enumerate() {
            out.push_str(&format!(
                "    {{\n      \"harness\": {},\n      \"model_ref\": {},\n      \"tier\": {},\n      \"dispatch_mode\": \"subprocess\",\n      \"attempts\": {},\n      \"successes\": {},\n      \"provenance\": [\n",
                quoted(&e.harness),
                quoted(&e.model_ref),
                quoted(&e.tier),
                e.attempts,
                e.successes
            ));
            for (j, (graph, task, attempt, passed)) in e.provenance.iter().enumerate() {
                out.push_str(&format!(
                    "        {{\"graph_id\": {}, \"task_id\": {task}, \"attempt_id\": {attempt}, \"passed\": {passed}}}{}\n",
                    quoted(graph),
                    if j + 1 < e.provenance.len() { "," } else { "" }
                ));
            }
            out.push_str(&format!(
                "      ]\n    }}{}\n",
                if i + 1 < fold.priors.len() { "," } else { "" }
            ));
        }
        out.push_str("  ]\n");
    }
    out.push_str("}\n");
    out
}

/// The file with its `generated_at` and `generator` lines dropped: what must be equal
/// between the cell's proposal and this fold's for the same input.
pub fn comparable(text: &str) -> String {
    text.lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with("\"generated_at\":") && !t.starts_with("\"generator\":")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Only the fields the fold reads, per graph: what a recorded fold input keeps.
pub fn reduce(data: &Value) -> Value {
    let nodes: Vec<Value> = data["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|n| {
            let attempts: Vec<Value> = n["attempts"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|a| {
                    serde_json::json!({
                        "attempt_id": a["attempt_id"],
                        "harness": a["harness"],
                        "model_ref": a["model_ref"],
                        "dispatch_mode": a["dispatch_mode"],
                        "model_passed": a["model_passed"],
                        "check_result": a["check_result"],
                    })
                })
                .collect();
            serde_json::json!({
                "id": n["id"], "state": n["state"], "tier": n["tier"], "attempts": attempts,
            })
        })
        .collect();
    serde_json::json!({ "status": data["status"], "nodes": nodes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn attempt(id: u64, model: &str, check: Value) -> Value {
        json!({"attempt_id": id, "harness": "h", "model_ref": model,
               "dispatch_mode": "subprocess", "model_passed": true, "check_result": check})
    }

    #[test]
    fn exclusions_keys_and_order() {
        let a = json!({"nodes": [
            {"id": 1, "state": "Integrated", "tier": "Tier1", "attempts": [
                attempt(3, "m", json!("failed")), attempt(4, "m", json!("passed")),
                attempt(5, "unverified", json!("passed")),
                attempt(6, "m", Value::Null)]},
            {"id": 2, "state": "Cancelled", "tier": null, "attempts": [
                attempt(7, "m", json!("passed"))]},
            {"id": 3, "state": "Failed", "tier": null, "attempts": [
                attempt(8, "m", json!("passed")),
                {"attempt_id": 9, "harness": "h", "model_ref": "m", "dispatch_mode": "native",
                 "model_passed": true, "check_result": "passed"},
                {"attempt_id": 10, "harness": "h", "model_ref": "m", "dispatch_mode": "subprocess",
                 "model_passed": null, "check_result": "passed"}]}
        ]});
        let b = json!({"nodes": [{"id": 1, "state": "Failed", "tier": "Tier1", "attempts": [
            attempt(1, "m", json!("passed")), attempt(1, "m", json!("passed"))]}]});
        let f = fold(&[("g-b".into(), b.clone()), ("g-a".into(), a.clone())]).unwrap();
        assert_eq!(f.graphs, vec!["g-a", "g-b"]);
        assert_eq!(f.counted, 4);
        assert_eq!(f.duplicates, 1);
        assert_eq!(
            f.excluded,
            vec![
                ("not_subprocess", 1),
                ("model_not_passed", 1),
                ("model_unverified", 1),
                ("cancelled", 1),
                ("no_check_result", 1)
            ]
        );
        assert_eq!(f.priors.len(), 2);
        assert_eq!(
            (
                f.priors[0].tier.as_str(),
                f.priors[0].attempts,
                f.priors[0].successes
            ),
            ("Tier1", 3, 2)
        );
        assert_eq!(f.priors[0].provenance[0], ("g-a".into(), 1, 3, false));
        assert_eq!(f.priors[0].provenance[2], ("g-b".into(), 1, 1, true));
        assert_eq!(f.priors[1].tier, "Tier2");
        // Order of the input never changes the bytes.
        let g = fold(&[("g-a".into(), a), ("g-b".into(), b)]).unwrap();
        assert_eq!(
            to_json(&f, "2026-11-11T00:00:00Z", ("x", "1")),
            to_json(&g, "2026-11-11T00:00:00Z", ("x", "1"))
        );
    }

    /// The cell's own proposal of the #140 live warm-up against this fold of the same
    /// `inspect --attempts` data (kept with only the fields the fold reads).
    #[test]
    fn matches_the_cells_fold_of_the_live_warmup() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let dir = root.join("benches/learning-plane/results");
        let input: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("warmup-lp-4-fold-input.json")).unwrap(),
        )
        .unwrap();
        let runs: Vec<(String, Value)> = input
            .as_object()
            .unwrap()
            .iter()
            .map(|(g, d)| (g.clone(), d.clone()))
            .collect();
        let cell = std::fs::read_to_string(dir.join("priors-lp-4-warmup.json")).unwrap();
        let ours = to_json(&fold(&runs).unwrap(), "x", ("x", "x"));
        assert_eq!(comparable(&cell), comparable(&ours));
    }

    #[test]
    fn bad_tier_or_harness_refuses() {
        let n = |tier: Value, harness: Value| {
            json!({"nodes": [{"id": 1, "state": "Failed", "tier": tier, "attempts": [
                {"attempt_id": 1, "harness": harness, "model_ref": "m",
                 "dispatch_mode": "subprocess", "model_passed": true, "check_result": "failed"}]}]})
        };
        assert!(fold(&[("g".into(), n(json!("Tier9"), json!("h")))]).is_err());
        assert!(fold(&[("g".into(), n(json!("Tier1"), json!("")))]).is_err());
        assert!(fold(&[("g".into(), n(json!("Tier1"), json!("h")))]).is_ok());
    }

    #[test]
    fn json_shape_and_quoting() {
        let f = fold(&[(
            "g".into(),
            json!({"nodes": [{"id": 1, "state": "Failed", "tier": "Tier3",
                   "attempts": [attempt(2, "a\"b", json!("failed"))]}]}),
        )])
        .unwrap();
        let text = to_json(&f, "2026-11-11T09:30:00Z", ("nodecode-meshloop", "0.4.0"));
        assert_eq!(
            text,
            "{\n  \"schema\": \"meshloop/advisory-priors\",\n  \"version\": 1,\n  \"generated_at\": \"2026-11-11T09:30:00Z\",\n  \"generator\": {\"name\": \"nodecode-meshloop\", \"version\": \"0.4.0\"},\n  \"source\": {\"graph_ids\": [\"g\"]},\n  \"priors\": [\n    {\n      \"harness\": \"h\",\n      \"model_ref\": \"a\\\"b\",\n      \"tier\": \"Tier3\",\n      \"dispatch_mode\": \"subprocess\",\n      \"attempts\": 1,\n      \"successes\": 0,\n      \"provenance\": [\n        {\"graph_id\": \"g\", \"task_id\": 1, \"attempt_id\": 2, \"passed\": false}\n      ]\n    }\n  ]\n}\n"
        );
        assert_eq!(quoted("\u{1}"), "\"\\u0001\"");
        assert!(!comparable(&text).contains("generated_at"));
        assert!(comparable(&text).contains("\"tier\": \"Tier3\""));
    }
}
