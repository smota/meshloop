//! Offline System 1 bench. Reads `benches/system1-thresholds.toml` and does not
//! open a socket or run the World D metric producer.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use meshloop_context::path_pinned;
use meshloop_domain::decision::{Confidence, DecisionOutcome};
use meshloop_domain::diagnostic::{
    DiagnosticAtom, DiagnosticLattice, DiagnosticSeverity, LatticeOrder, compare,
};
use meshloop_domain::evidence::{AttemptId, CandidateRef};
use meshloop_domain::policy::{CouplingPenalty, ModelCapabilityTier};
use meshloop_domain::task_graph::{AcceptanceGate, TaskGraph, TaskId, TaskNode, Tier};
use meshloop_engine::agent::rerank_unpinned_suffix;
use meshloop_engine::converge::micro_repair_eligible;
use meshloop_engine::planner::{DefaultTierAssigner, TierAssigner, apply_risk_ratchet};
use meshloop_engine::ports::{
    BatchNoulQuery, ChoiceQuery, DecisionPort, DecisionPortError, FeedbackKey,
    InMemoryFeedbackStore, NoulQuery, OracleOutcome, OracleQuery, PredictiveSuccessOracle,
    RoutingFeedbackStore, ScoreQuery, ScriptedDecisionPort, SessionSpendLedger, TierKey,
    decision_fingerprint,
};
use meshloop_engine::router::{
    Candidate, HistoricalSuccessSignal, RestlessBanditSignal, RoutingContext, RoutingSignal,
};
use meshloop_engine::verify::{CandidateReview, evaluate_candidate_diff};

const REQUIRED: &[&str] = &[
    "dec.tier.downward_violation_count",
    "dec.tier.freeze_violation_count",
    "dec.review.tier3_bypass_count",
    "dec.repair.lyapunov_monotonic_pct",
    "dec.repair.unallowlisted_dispatch_count",
    "dec.oracle.prior_influence_clip_pct",
    "dec.oracle.exploration_preservation",
    "dec.context.file_drop_count",
    "dec.context.pinned_shift_count",
    "dec.context.fallback_fidelity",
    "dec.tier.ratchet_precision",
    "dec.review.false_accept_rate",
    "dec.review.latency_ms.p95",
    "dec.repair.micro_yield_pct",
    "dec.context.rerank_latency_ms.p95",
    "dec.spend.cost_per_task_usd",
];

struct Rule {
    name: String,
    op: String,
    value: f64,
    invariant: bool,
}

struct TierMetrics {
    downward: f64,
    freeze: f64,
    precision: f64,
}

struct ReviewMetrics {
    tier3_bypass: f64,
    false_accept_rate: f64,
    latency_p95_ms: f64,
}

struct RepairMetrics {
    unallowlisted: f64,
    lyapunov: f64,
    micro_yield: f64,
}

struct ContextMetrics {
    file_drops: f64,
    pinned_shifts: f64,
    fallback_fidelity: f64,
    latency_p95_ms: f64,
}

struct Rated {
    id: u32,
    description: &'static str,
    tier: Option<Tier>,
    depends_on: &'static [u32],
    q: f64,
}

struct ReviewRow {
    tier: Tier,
    checks_passed: bool,
    p: Option<f64>,
}

#[derive(Clone, Copy)]
struct PriorCase {
    success: u32,
    failure: u32,
    prior: f64,
}

#[derive(Default)]
struct RerankProbe {
    drops: u64,
    shifts: u64,
    samples_ms: Vec<f64>,
    fallback_cases: u64,
    fallback_matches: u64,
}

struct CountingPort<'a> {
    inner: &'a ScriptedDecisionPort,
    score_calls: &'a Cell<u64>,
}

struct ListPort {
    batch: Result<Vec<DecisionOutcome>, DecisionPortError>,
}

struct ScriptedPrior {
    prior: f64,
}

type Skeleton = (String, String);

pub fn bench_system1(root: &Path) -> ExitCode {
    let rules = match load_rules(root) {
        Ok(rules) => rules,
        Err(err) => {
            eprintln!("bench-system1: {err}");
            return ExitCode::FAILURE;
        }
    };
    let measured = match measure() {
        Ok(measured) => measured,
        Err(err) => {
            eprintln!("bench-system1: {err}");
            return ExitCode::FAILURE;
        }
    };

    let mut metrics = Vec::with_capacity(rules.len());
    let mut invariants_pass = true;
    let mut thresholds_pass = true;
    for rule in &rules {
        let (value, passed) = match measured.get(&rule.name) {
            Some(value) => (*value, passes(&rule.op, *value, rule.value)),
            None => {
                eprintln!("bench-system1: {} not measured", rule.name);
                (0.0, false)
            }
        };
        if !passed {
            eprintln!(
                "bench-system1: {} {} {} observed {}",
                rule.name, rule.op, rule.value, value
            );
            if rule.invariant {
                invariants_pass = false;
            } else {
                thresholds_pass = false;
            }
        }
        metrics.push(serde_json::json!({
            "name": rule.name,
            "value": value,
            "status": if passed { "pass" } else { "fail" },
        }));
    }

    let overall = if invariants_pass && thresholds_pass {
        "pass"
    } else {
        "fail"
    };
    let record = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "schema_version": "1.0.0",
        "suite": {
            "name": "system1-benchmark-suite",
            "kind": "synthetic-and-polyglot",
            "world": "deterministic-mock",
            "version": "1.0.0"
        },
        "metrics": metrics,
        "status": {
            "overall": overall,
            "invariants": if invariants_pass { "pass" } else { "fail" },
            "thresholds": if thresholds_pass { "pass" } else { "fail" },
        }
    });
    if let Err(err) = write_record(root, &record) {
        eprintln!("bench-system1: {err}");
        return ExitCode::FAILURE;
    }
    if invariants_pass && thresholds_pass {
        println!("bench-system1: pass artifacts/bench/system1_bench.json");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn write_record(root: &Path, record: &serde_json::Value) -> Result<(), String> {
    let dir = root.join("artifacts").join("bench");
    fs::create_dir_all(&dir).map_err(|err| format!("cannot create artifacts/bench: {err}"))?;
    let text = serde_json::to_string_pretty(record)
        .map_err(|err| format!("cannot encode system1 bench record: {err}"))?;
    fs::write(dir.join("system1_bench.json"), text + "\n")
        .map_err(|err| format!("cannot write artifacts/bench/system1_bench.json: {err}"))
}

fn load_rules(root: &Path) -> Result<Vec<Rule>, String> {
    let text = fs::read_to_string(root.join("benches").join("system1-thresholds.toml"))
        .map_err(|err| format!("cannot read benches/system1-thresholds.toml: {err}"))?;
    let mut rules = Vec::new();
    let mut invariant = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("[invariants]") {
            invariant = true;
            continue;
        }
        if trimmed.starts_with("[performance") {
            invariant = false;
            continue;
        }
        if let Some((key, rest)) = trimmed.split_once('=') {
            let name = key.trim().trim_matches('"').to_string();
            if let Some(rule) = parse_rule(&name, rest, invariant) {
                rules.push(rule);
            }
        }
    }
    if rules.is_empty() {
        return Err("benches/system1-thresholds.toml has no system 1 gates".into());
    }
    for key in REQUIRED {
        if !rules.iter().any(|rule| rule.name == *key) {
            return Err(format!("benches/system1-thresholds.toml missing {key}"));
        }
    }
    Ok(rules)
}

fn parse_rule(name: &str, rest: &str, invariant: bool) -> Option<Rule> {
    let op_at = rest.find("op = \"")?;
    let after_op = &rest[op_at + 6..];
    let op_end = after_op.find('"')?;
    let op = after_op[..op_end].to_string();
    let val_at = rest.find("value = ")?;
    let val_str = rest[val_at + 8..].trim_end_matches([' ', '}']).trim();
    let value = val_str.parse::<f64>().ok()?;
    Some(Rule {
        name: name.to_string(),
        op,
        value,
        invariant,
    })
}

fn passes(op: &str, value: f64, target: f64) -> bool {
    match op {
        "eq" => (value - target).abs() < 1e-4,
        "gte" => value >= target - 1e-4,
        "lte" => value <= target + 1e-4,
        _ => false,
    }
}

fn measure() -> Result<BTreeMap<String, f64>, String> {
    let tier = measure_tier()?;
    let review = measure_review()?;
    let repair = measure_repair();
    let context = measure_context()?;
    let mut metrics = BTreeMap::new();
    metrics.insert(
        "dec.tier.downward_violation_count".to_string(),
        tier.downward,
    );
    metrics.insert("dec.tier.freeze_violation_count".to_string(), tier.freeze);
    metrics.insert("dec.tier.ratchet_precision".to_string(), tier.precision);
    metrics.insert(
        "dec.review.tier3_bypass_count".to_string(),
        review.tier3_bypass,
    );
    metrics.insert(
        "dec.review.false_accept_rate".to_string(),
        review.false_accept_rate,
    );
    metrics.insert(
        "dec.review.latency_ms.p95".to_string(),
        review.latency_p95_ms,
    );
    metrics.insert(
        "dec.repair.unallowlisted_dispatch_count".to_string(),
        repair.unallowlisted,
    );
    metrics.insert(
        "dec.repair.lyapunov_monotonic_pct".to_string(),
        repair.lyapunov,
    );
    metrics.insert("dec.repair.micro_yield_pct".to_string(), repair.micro_yield);
    metrics.insert(
        "dec.oracle.prior_influence_clip_pct".to_string(),
        prior_clip_pct(),
    );
    metrics.insert(
        "dec.oracle.exploration_preservation".to_string(),
        exploration_pct(),
    );
    metrics.insert(
        "dec.spend.cost_per_task_usd".to_string(),
        cost_per_task_usd(),
    );
    metrics.insert(
        "dec.context.file_drop_count".to_string(),
        context.file_drops,
    );
    metrics.insert(
        "dec.context.pinned_shift_count".to_string(),
        context.pinned_shifts,
    );
    metrics.insert(
        "dec.context.fallback_fidelity".to_string(),
        context.fallback_fidelity,
    );
    metrics.insert(
        "dec.context.rerank_latency_ms.p95".to_string(),
        context.latency_p95_ms,
    );
    for key in REQUIRED {
        if !metrics.contains_key(*key) {
            return Err(format!("{key} was not measured"));
        }
    }
    Ok(metrics)
}

fn measure_tier() -> Result<TierMetrics, String> {
    let rows = rated_rows();
    let mut graph = graph_from(&rows);
    let bases: Vec<Tier> = graph
        .nodes
        .iter()
        .map(|node| match node.tier {
            Some(tier) => tier,
            None => DefaultTierAssigner.assign_tier(&graph, node.id),
        })
        .collect();
    let mut port = ScriptedDecisionPort::default();
    for row in &rows {
        let subject = format!("risk\n{}", row.description);
        port.insert(
            decision_fingerprint("score", &[subject.as_str()]),
            finite_score(row.q),
        );
    }
    let calls = Cell::new(0);
    let counter = CountingPort {
        inner: &port,
        score_calls: &calls,
    };
    apply_risk_ratchet(&mut graph, &counter)
        .map_err(|err| format!("risk ratchet failed: {err:?}"))?;

    let mut downward = 0u64;
    let mut escalations = 0u64;
    let mut at_threshold = 0u64;
    for (node, base) in graph.nodes.iter().zip(bases) {
        let Some(result) = node.tier else {
            continue;
        };
        if result < base {
            downward += 1;
        }
        if result == Tier::Tier3 && base < Tier::Tier3 {
            escalations += 1;
            if node.risk_q.is_some_and(|q| q >= 0.90) {
                at_threshold += 1;
            }
        }
    }
    calls.set(0);
    apply_risk_ratchet(&mut graph, &counter)
        .map_err(|err| format!("risk ratchet freeze call failed: {err:?}"))?;
    let precision = if escalations == 0 {
        0.0
    } else {
        at_threshold as f64 * 100.0 / escalations as f64
    };
    Ok(TierMetrics {
        downward: downward as f64,
        freeze: calls.get() as f64,
        precision,
    })
}

fn rated_rows() -> Vec<Rated> {
    vec![
        Rated {
            id: 1,
            description: "declared-tier1-low",
            tier: Some(Tier::Tier1),
            depends_on: &[],
            q: 0.10,
        },
        Rated {
            id: 2,
            description: "declared-tier1-below",
            tier: Some(Tier::Tier1),
            depends_on: &[],
            q: 0.89,
        },
        Rated {
            id: 3,
            description: "declared-tier1-boundary",
            tier: Some(Tier::Tier1),
            depends_on: &[],
            q: 0.90,
        },
        Rated {
            id: 4,
            description: "declared-tier1-high",
            tier: Some(Tier::Tier1),
            depends_on: &[],
            q: 0.95,
        },
        Rated {
            id: 5,
            description: "declared-tier1-certain",
            tier: Some(Tier::Tier1),
            depends_on: &[],
            q: 1.0,
        },
        Rated {
            id: 6,
            description: "declared-tier2-mid",
            tier: Some(Tier::Tier2),
            depends_on: &[],
            q: 0.50,
        },
        Rated {
            id: 7,
            description: "declared-tier2-high",
            tier: Some(Tier::Tier2),
            depends_on: &[],
            q: 0.91,
        },
        Rated {
            id: 8,
            description: "declared-tier3-low",
            tier: Some(Tier::Tier3),
            depends_on: &[],
            q: 0.10,
        },
        Rated {
            id: 9,
            description: "omitted-two-deps",
            tier: None,
            depends_on: &[3, 4],
            q: 0.20,
        },
        Rated {
            id: 10,
            description: "omitted-with-dependent",
            tier: None,
            depends_on: &[],
            q: 0.99,
        },
        Rated {
            id: 11,
            description: "declared-child",
            tier: Some(Tier::Tier1),
            depends_on: &[10],
            q: 0.0,
        },
        Rated {
            id: 12,
            description: "omitted-leaf",
            tier: None,
            depends_on: &[],
            q: 0.40,
        },
    ]
}

fn graph_from(rows: &[Rated]) -> TaskGraph {
    TaskGraph {
        graph_id: "system1".into(),
        nodes: rows
            .iter()
            .map(|row| TaskNode {
                id: TaskId(row.id),
                description: row.description.to_string(),
                depends_on: row.depends_on.iter().copied().map(TaskId).collect(),
                tier: row.tier,
                allowed_paths: Vec::new(),
                empty_diff_ok: false,
                deliverable: None,
                gate: AcceptanceGate::Human,
                risk_q: None,
                risk_prompt_hash: None,
            })
            .collect(),
    }
}

fn measure_review() -> Result<ReviewMetrics, String> {
    let tier3 = tier3_rows();
    let ordinary = tier12_rows();
    let mut bypass = 0u64;
    for row in &tier3 {
        if matches!(review_once(row), CandidateReview::Pass(_)) {
            bypass += 1;
        }
    }
    let mut pass_rows = 0u64;
    let mut false_accepts = 0u64;
    for row in &ordinary {
        if matches!(review_once(row), CandidateReview::Pass(_)) {
            pass_rows += 1;
            if !row.checks_passed {
                false_accepts += 1;
            }
        }
    }
    let false_accept_rate = if pass_rows == 0 {
        0.0
    } else {
        false_accepts as f64 * 100.0 / pass_rows as f64
    };
    let mut samples = Vec::new();
    for _ in 0..64 {
        for row in tier3.iter().chain(ordinary.iter()) {
            let started = Instant::now();
            let verdict = review_once(row);
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(verdict);
        }
    }
    Ok(ReviewMetrics {
        tier3_bypass: bypass as f64,
        false_accept_rate,
        latency_p95_ms: p95_ms(&mut samples)?,
    })
}

fn tier3_rows() -> Vec<ReviewRow> {
    vec![
        ReviewRow {
            tier: Tier::Tier3,
            checks_passed: true,
            p: Some(1.0),
        },
        ReviewRow {
            tier: Tier::Tier3,
            checks_passed: true,
            p: Some(0.0),
        },
        ReviewRow {
            tier: Tier::Tier3,
            checks_passed: true,
            p: Some(0.95),
        },
        ReviewRow {
            tier: Tier::Tier3,
            checks_passed: false,
            p: Some(1.0),
        },
        ReviewRow {
            tier: Tier::Tier3,
            checks_passed: true,
            p: None,
        },
    ]
}

fn tier12_rows() -> Vec<ReviewRow> {
    vec![
        ReviewRow {
            tier: Tier::Tier1,
            checks_passed: true,
            p: Some(1.0),
        },
        ReviewRow {
            tier: Tier::Tier1,
            checks_passed: true,
            p: Some(0.95),
        },
        ReviewRow {
            tier: Tier::Tier2,
            checks_passed: true,
            p: Some(0.99),
        },
        ReviewRow {
            tier: Tier::Tier1,
            checks_passed: false,
            p: Some(1.0),
        },
        ReviewRow {
            tier: Tier::Tier2,
            checks_passed: false,
            p: Some(0.99),
        },
        ReviewRow {
            tier: Tier::Tier1,
            checks_passed: false,
            p: Some(0.96),
        },
        ReviewRow {
            tier: Tier::Tier2,
            checks_passed: false,
            p: Some(0.95),
        },
        ReviewRow {
            tier: Tier::Tier1,
            checks_passed: true,
            p: Some(0.05),
        },
        ReviewRow {
            tier: Tier::Tier2,
            checks_passed: true,
            p: Some(0.50),
        },
        ReviewRow {
            tier: Tier::Tier1,
            checks_passed: true,
            p: Some(0.94),
        },
        ReviewRow {
            tier: Tier::Tier2,
            checks_passed: true,
            p: Some(0.0),
        },
        ReviewRow {
            tier: Tier::Tier1,
            checks_passed: false,
            p: Some(0.0),
        },
    ]
}

fn review_once(row: &ReviewRow) -> CandidateReview {
    let acceptability = row.p.map(confidence);
    evaluate_candidate_diff(
        row.tier,
        row.checks_passed,
        acceptability,
        CandidateRef {
            task_id: TaskId(1),
            attempt_id: AttemptId(1),
            revision: "rev".into(),
        },
    )
}

fn measure_repair() -> RepairMetrics {
    let windows = [(0.70, 0.20), (1.0, 0.0), (0.85, 0.10)];
    let blocked = [
        lattice([atom(DiagnosticSeverity::Syntax, "syntax", "lib.rs", 1)]),
        lattice([
            atom(DiagnosticSeverity::Syntax, "syntax", "lib.rs", 1),
            atom(DiagnosticSeverity::Type, "E0425", "lib.rs", 2),
        ]),
        lattice([atom(DiagnosticSeverity::Type, "E0308", "lib.rs", 3)]),
        lattice([
            atom(DiagnosticSeverity::Type, "E0308", "lib.rs", 3),
            atom(DiagnosticSeverity::Type, "E0425", "lib.rs", 4),
        ]),
        lattice([atom(DiagnosticSeverity::Warning, "E0308", "lib.rs", 5)]),
    ];
    let mut unallowlisted = 0u64;
    for lattice in &blocked {
        for (mechanical, semantic) in windows {
            if micro_repair_eligible(lattice, mechanical, semantic) {
                unallowlisted += 1;
            }
        }
    }

    let pairs = lyapunov_pairs();
    let mut kept = 0u64;
    let mut strict = 0u64;
    for (previous, next) in &pairs {
        if compare(previous, next) == LatticeOrder::Improves {
            kept += 1;
            if next.progress() < previous.progress() {
                strict += 1;
            }
        }
    }
    let lyapunov = if kept == 0 {
        0.0
    } else {
        strict as f64 * 100.0 / kept as f64
    };

    let yield_set = yield_lattices();
    let eligible = yield_set
        .iter()
        .filter(|lattice| micro_repair_eligible(lattice, 0.70, 0.20))
        .count();
    let micro_yield = eligible as f64 * 100.0 / yield_set.len() as f64;
    RepairMetrics {
        unallowlisted: unallowlisted as f64,
        lyapunov,
        micro_yield,
    }
}

fn lyapunov_pairs() -> Vec<(DiagnosticLattice, DiagnosticLattice)> {
    let three = lattice([
        atom(DiagnosticSeverity::Type, "E0308", "lib.rs", 1),
        atom(DiagnosticSeverity::Type, "E0308", "lib.rs", 2),
        atom(DiagnosticSeverity::Type, "E0308", "lib.rs", 3),
    ]);
    let two = lattice([
        atom(DiagnosticSeverity::Type, "E0308", "lib.rs", 1),
        atom(DiagnosticSeverity::Type, "E0308", "lib.rs", 2),
    ]);
    let one = lattice([atom(DiagnosticSeverity::Type, "E0308", "lib.rs", 1)]);
    let same_progress = lattice([
        atom(DiagnosticSeverity::Type, "E0308", "lib.rs", 8),
        atom(DiagnosticSeverity::Type, "E0308", "lib.rs", 9),
    ]);
    let syntax = lattice([atom(DiagnosticSeverity::Syntax, "syntax", "lib.rs", 4)]);
    vec![
        (three.clone(), two.clone()),
        (two.clone(), one.clone()),
        (one.clone(), DiagnosticLattice::empty()),
        (syntax, one.clone()),
        (two.clone(), two),
        (one, same_progress),
        (DiagnosticLattice::empty(), three),
    ]
}

fn yield_lattices() -> Vec<DiagnosticLattice> {
    vec![
        lattice([atom(DiagnosticSeverity::Type, "E0425", "a.rs", 1)]),
        lattice([atom(DiagnosticSeverity::Type, "unused_imports", "b.rs", 2)]),
        lattice([
            atom(DiagnosticSeverity::Type, "E0425", "a.rs", 1),
            atom(DiagnosticSeverity::Type, "unused_imports", "b.rs", 2),
        ]),
        lattice([
            atom(DiagnosticSeverity::Type, "E0425", "a.rs", 1),
            atom(DiagnosticSeverity::Warning, "unused", "a.rs", 3),
        ]),
        lattice([
            atom(DiagnosticSeverity::Type, "E0425", "a.rs", 1),
            atom(DiagnosticSeverity::Type, "E0425", "a.rs", 4),
        ]),
        lattice([
            atom(DiagnosticSeverity::Type, "unused_imports", "b.rs", 2),
            atom(DiagnosticSeverity::Type, "unused_imports", "c.rs", 5),
        ]),
        lattice([atom(DiagnosticSeverity::Error, "E0425", "d.rs", 6)]),
        lattice([atom(DiagnosticSeverity::Test, "unused_imports", "e.rs", 7)]),
    ]
}

fn atom(severity: DiagnosticSeverity, code: &str, path: &str, template: u64) -> DiagnosticAtom {
    DiagnosticAtom {
        severity,
        code: code.to_string(),
        path: path.to_string(),
        template,
    }
}

fn lattice(atoms: impl IntoIterator<Item = DiagnosticAtom>) -> DiagnosticLattice {
    DiagnosticLattice::new(atoms)
}

fn prior_clip_pct() -> f64 {
    let cases = [
        PriorCase {
            success: 0,
            failure: 0,
            prior: 0.0,
        },
        PriorCase {
            success: 0,
            failure: 0,
            prior: 1.0,
        },
        PriorCase {
            success: 0,
            failure: 0,
            prior: 0.5,
        },
        PriorCase {
            success: 10,
            failure: 0,
            prior: 0.0,
        },
        PriorCase {
            success: 0,
            failure: 10,
            prior: 1.0,
        },
        PriorCase {
            success: 2,
            failure: 2,
            prior: 1.0,
        },
        PriorCase {
            success: 3,
            failure: 1,
            prior: 0.0,
        },
        PriorCase {
            success: 8,
            failure: 2,
            prior: 0.25,
        },
    ];
    let mut within = 0u64;
    for case in cases {
        let mut feedback = InMemoryFeedbackStore::default();
        record_pulls(&mut feedback, case.success, case.failure);
        let headroom = HashMap::new();
        let candidate = local_candidate();
        let signal = HistoricalSuccessSignal;
        let local = signal
            .score(
                &candidate,
                &route_context(Tier::Tier1, &headroom, &feedback, None, None),
            )
            .0;
        let oracle = ScriptedPrior { prior: case.prior };
        let fused = signal
            .score(
                &candidate,
                &route_context(Tier::Tier1, &headroom, &feedback, Some(&oracle), None),
            )
            .0;
        if (fused - local).abs() <= 0.25 + 1e-9 {
            within += 1;
        }
    }
    within as f64 * 100.0 / cases.len() as f64
}

fn exploration_pct() -> f64 {
    let feedback = InMemoryFeedbackStore::default();
    let headroom = HashMap::new();
    let mut total = 0u64;
    let mut preserved = 0u64;
    for harness in ["arm-a", "arm-b", "arm-c", "arm-d"] {
        for prior in [0.0, 1.0] {
            total += 1;
            let oracle = ScriptedPrior { prior };
            let candidate = Candidate {
                harness: harness.to_string(),
                model_ref: "m".into(),
                model_tier: ModelCapabilityTier::TopTier,
            };
            let score = RestlessBanditSignal::default()
                .score(
                    &candidate,
                    &route_context(Tier::Tier2, &headroom, &feedback, Some(&oracle), None),
                )
                .0;
            if score == 0.7 {
                preserved += 1;
            }
        }
    }
    preserved as f64 * 100.0 / total as f64
}

fn cost_per_task_usd() -> f64 {
    // Scripted ledger only. It is never charged, so the contribution is 0.
    let budget_micro = 1_000_000u64;
    let ledger = SessionSpendLedger::new(budget_micro, 1);
    let charged_micro = budget_micro.saturating_sub(ledger.remaining());
    charged_micro as f64 / 1_000_000.0
}

fn record_pulls(store: &mut InMemoryFeedbackStore, success: u32, failure: u32) {
    let key = FeedbackKey {
        harness: "local".into(),
        model_ref: "m".into(),
        tier: TierKey::Tier1,
    };
    for _ in 0..success {
        let _ = store.record_outcome(&key, true);
    }
    for _ in 0..failure {
        let _ = store.record_outcome(&key, false);
    }
}

fn local_candidate() -> Candidate {
    Candidate {
        harness: "local".into(),
        model_ref: "m".into(),
        model_tier: ModelCapabilityTier::MidTier,
    }
}

fn route_context<'a>(
    tier: Tier,
    headroom: &'a HashMap<String, f64>,
    feedback: &'a dyn RoutingFeedbackStore,
    oracle: Option<&'a dyn PredictiveSuccessOracle>,
    spend: Option<&'a SessionSpendLedger>,
) -> RoutingContext<'a> {
    RoutingContext {
        task_tier: tier,
        coupling_penalty: CouplingPenalty(0),
        preferred_harness: None,
        headroom,
        feedback,
        oracle,
        spend,
    }
}

fn measure_context() -> Result<ContextMetrics, String> {
    let mut probe = RerankProbe::default();
    let prefixes = vec!["pin/".to_string()];
    let reordered = skeletons(&[("u/b.rs", "B"), ("pin/a.rs", "A"), ("u/c.rs", "C")]);
    let reorder_port = ListPort::scores(&[0.1, 0.9]);
    let (got, accepted) = probe.call(&reordered, "rank files", &prefixes, &reorder_port);
    if !accepted || got.len() != reordered.len() {
        return Err("scored rerank was not accepted".into());
    }

    let equal = skeletons(&[("pin/a.rs", "A"), ("u/b.rs", "B"), ("u/c.rs", "C")]);
    let scripted = script_equal_scores(&equal, "rank files", &prefixes, 0.4);
    let (_got, accepted) = probe.call(&equal, "rank files", &prefixes, &scripted);
    if !accepted {
        return Err("scripted rerank was not a score batch".into());
    }

    let pinned = skeletons(&[("pin/a.rs", "A"), ("pin/b.rs", "B")]);
    let _ = probe.call(
        &pinned,
        "rank files",
        &prefixes,
        &ListPort::failure(DecisionPortError::EmptyQuery),
    );

    let mut over = vec![("pin/keep.rs".to_string(), "K".to_string())];
    for index in 0..=24 {
        over.push((format!("u/{index:02}.rs"), format!("b{index}")));
    }
    let _ = probe.call(
        &over,
        "rank files",
        &["pin/keep.rs".to_string()],
        &ListPort::scores(&[1.0]),
    );

    let mixed = skeletons(&[("pin/a.rs", "A"), ("u/b.rs", "B"), ("u/c.rs", "C")]);
    let _ = probe.call(
        &mixed,
        "rank files",
        &prefixes,
        &ListPort {
            batch: Ok(vec![
                DecisionOutcome::Boolean(true),
                DecisionOutcome::Boolean(false),
            ]),
        },
    );

    for err in [
        DecisionPortError::EmptyQuery,
        DecisionPortError::Unscripted {
            fingerprint: "bench".into(),
        },
        DecisionPortError::BatchTooLarge { len: 3 },
    ] {
        let (got, _) = probe.call(&mixed, "rank files", &prefixes, &ListPort::failure(err));
        probe.note_fallback(&got, &mixed);
    }

    for _ in 0..32 {
        let _ = probe.call(&reordered, "rank files", &prefixes, &reorder_port);
    }

    let fallback_fidelity =
        if probe.fallback_cases > 0 && probe.fallback_matches == probe.fallback_cases {
            1.0
        } else {
            0.0
        };
    Ok(ContextMetrics {
        file_drops: probe.drops as f64,
        pinned_shifts: probe.shifts as f64,
        fallback_fidelity,
        latency_p95_ms: p95_ms(&mut probe.samples_ms)?,
    })
}

impl RerankProbe {
    fn call(
        &mut self,
        selected: &[Skeleton],
        description: &str,
        prefixes: &[String],
        port: &dyn DecisionPort,
    ) -> (Vec<Skeleton>, bool) {
        let started = Instant::now();
        let (got, accepted) = rerank_unpinned_suffix(selected, description, prefixes, port);
        self.samples_ms
            .push(started.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(&got);
        if got.len() != selected.len() {
            self.drops += 1;
        }
        self.shifts += pinned_index_changes(selected, &got, prefixes);
        (got, accepted)
    }

    fn note_fallback(&mut self, got: &[Skeleton], selected: &[Skeleton]) {
        self.fallback_cases += 1;
        if got == selected {
            self.fallback_matches += 1;
        }
    }
}

fn pinned_index_changes(before: &[Skeleton], after: &[Skeleton], prefixes: &[String]) -> u64 {
    let mut count = 0u64;
    for (index, (path, _)) in before.iter().enumerate() {
        if !path_pinned(path, prefixes) {
            continue;
        }
        let after_index = after.iter().position(|(candidate, _)| candidate == path);
        if after_index != Some(index) {
            count += 1;
        }
    }
    count
}

fn skeletons(items: &[(&str, &str)]) -> Vec<Skeleton> {
    items
        .iter()
        .map(|(path, body)| (path.to_string(), body.to_string()))
        .collect()
}

fn script_equal_scores(
    selected: &[Skeleton],
    description: &str,
    prefixes: &[String],
    score_value: f64,
) -> ScriptedDecisionPort {
    let propositions: Vec<String> = selected
        .iter()
        .filter(|(path, _)| !path_pinned(path, prefixes))
        .map(|(path, _)| format!("{description}\n{path}"))
        .collect();
    let parts: Vec<&str> = propositions.iter().map(String::as_str).collect();
    let mut port = ScriptedDecisionPort::default();
    port.insert(
        decision_fingerprint("batch-noul", &parts),
        finite_score(score_value),
    );
    port
}

fn p95_ms(samples: &mut [f64]) -> Result<f64, String> {
    if samples.is_empty() {
        return Err("no latency samples".into());
    }
    samples.sort_by(f64::total_cmp);
    let index = ((samples.len() as f64) * 0.95) as usize;
    Ok(samples[index.min(samples.len() - 1)])
}

fn finite_score(value: f64) -> DecisionOutcome {
    DecisionOutcome::try_score(value).expect("finite score")
}

fn confidence(value: f64) -> Confidence {
    Confidence::try_new(value).expect("unit-interval fixture")
}

impl DecisionPort for CountingPort<'_> {
    fn evaluate_choice(&self, query: &ChoiceQuery) -> Result<DecisionOutcome, DecisionPortError> {
        self.inner.evaluate_choice(query)
    }

    fn evaluate_noul(&self, query: &NoulQuery) -> Result<DecisionOutcome, DecisionPortError> {
        self.inner.evaluate_noul(query)
    }

    fn evaluate_score(&self, query: &ScoreQuery) -> Result<DecisionOutcome, DecisionPortError> {
        self.score_calls.set(self.score_calls.get() + 1);
        self.inner.evaluate_score(query)
    }

    fn evaluate_batch_noul(
        &self,
        query: &BatchNoulQuery,
    ) -> Result<Vec<DecisionOutcome>, DecisionPortError> {
        self.inner.evaluate_batch_noul(query)
    }
}

impl ListPort {
    fn scores(values: &[f64]) -> Self {
        Self {
            batch: Ok(values.iter().copied().map(finite_score).collect()),
        }
    }

    fn failure(err: DecisionPortError) -> Self {
        Self { batch: Err(err) }
    }
}

impl DecisionPort for ListPort {
    fn evaluate_choice(&self, _query: &ChoiceQuery) -> Result<DecisionOutcome, DecisionPortError> {
        Err(DecisionPortError::EmptyQuery)
    }

    fn evaluate_noul(&self, _query: &NoulQuery) -> Result<DecisionOutcome, DecisionPortError> {
        Err(DecisionPortError::EmptyQuery)
    }

    fn evaluate_score(&self, _query: &ScoreQuery) -> Result<DecisionOutcome, DecisionPortError> {
        Err(DecisionPortError::EmptyQuery)
    }

    fn evaluate_batch_noul(
        &self,
        _query: &BatchNoulQuery,
    ) -> Result<Vec<DecisionOutcome>, DecisionPortError> {
        self.batch.clone()
    }
}

impl PredictiveSuccessOracle for ScriptedPrior {
    fn predict(&self, query: &OracleQuery) -> Result<OracleOutcome, DecisionPortError> {
        if query.arm.is_empty() {
            return Err(DecisionPortError::EmptyQuery);
        }
        let prior = Confidence::try_new(self.prior).map_err(|_| DecisionPortError::Unscripted {
            fingerprint: "prior".into(),
        })?;
        Ok(OracleOutcome { prior })
    }
}
