//! `bench-learning --offline` (#140): the learning-plane comparison with no socket, no
//! nodecode and no model, run in CI.
//!
//! Meshloop's fixture harness stands in for every (harness, model) candidate, with
//! seeded outcomes that differ by model (`benches/learning-plane/offline/`). The steps
//! are the live comparison's: a warm-up pins each candidate on each task in one shared
//! store; the fold (`priors_fold`, the cell's fold restated, checked against a proposal
//! the real cell wrote for this fixture) turns it into a priors file; `priors check`
//! validates it; then a baseline arm and a priors-on arm run the same task set with a
//! new store per task x repetition, the priors-on arm after `priors accept --as bench`.
//!
//! The gates (`offline/thresholds.toml`) assert the mechanism only: priors applied, the
//! ±0.25 bound respected, the candidate set unchanged, `changed_top` recorded. Each
//! arm's outcome metrics are reported, never gated: whether learning wins is the live
//! comparison's question.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    Ctx, PriorsFile, RunPlan, RunRecord, TaskMeta, TaskSet, absolute, config_max_retries, doctor,
    fold_input, git, path_str, percentile, read_json, round2, run_cmd, run_one, summarize,
    task_config, tier_coverage, unix_now, utc_string, warmup_schedule,
};
use crate::priors_fold;

const IDENTITY: &str = "bench";
const GENERATOR: (&str, &str) = ("xtask-bench-learning", "1");

#[derive(Debug, Deserialize)]
struct Spec {
    version: String,
    reps: u32,
    floor: u32,
    tasks: Vec<SpecTask>,
}

#[derive(Debug, Deserialize)]
struct SpecTask {
    id: String,
    tier: String,
    warmup: bool,
    verify: Vec<String>,
}

impl SpecTask {
    fn meta(&self) -> TaskMeta {
        TaskMeta {
            id: self.id.clone(),
            language: "fixture".into(),
            difficulty: "fixture".into(),
            kind: "fixture".into(),
            description: format!("Offline fixture task {}.", self.id),
            test_command: vec![],
            editable: vec![],
            expected_tests: 0,
            verify: self.verify.clone(),
        }
    }
}

struct Arm {
    name: &'static str,
    runs: Vec<RunRecord>,
}

pub fn offline(root: &Path, lp: &Path, keep: Option<&Path>) -> ExitCode {
    match run(root, lp, keep) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("bench-learning --offline: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(root: &Path, lp: &Path, keep: Option<&Path>) -> Result<bool, String> {
    let clock = Instant::now();
    if !crate::cargo(
        root,
        &[
            "build",
            "-p",
            "meshloop-cli",
            "-p",
            "meshloop-adapters",
            "--bins",
            "--locked",
            "--offline",
        ],
    ) {
        return Err("cargo build failed".into());
    }
    let bin = absolute(&crate::meshloop_bin(root));
    let fixture = absolute(&crate::fixture_bin(root));
    let dir = lp.join("offline");
    let spec: Spec = read_json(&dir.join("tasks.json"))?;
    let template = |name: &str| -> Result<String, String> {
        let text = fs::read_to_string(dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
        let path = path_str(&fixture).replace('\\', "\\\\");
        Ok(text.replace("{FIXTURE}", &path))
    };
    let measured_template = template("meshloop.toml")?;
    let warmup_template = template("warmup.toml")?;

    let scratch = match keep {
        Some(k) => absolute(k),
        None => std::env::temp_dir().join(format!("ml-lp-offline-{}", std::process::id())),
    };
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let outcome = compare_arms(
        root,
        &dir,
        &spec,
        &bin,
        &scratch,
        &measured_template,
        &warmup_template,
    );
    if keep.is_none() {
        let _ = fs::remove_dir_all(&scratch);
    } else {
        println!("bench-learning --offline: kept {}", scratch.display());
    }
    println!(
        "bench-learning --offline: {:.1} s",
        clock.elapsed().as_secs_f64()
    );
    outcome
}

fn ctx<'a>(dir: &'a Path, spec: &Spec, bin: &Path, out: PathBuf, template: &str) -> Ctx<'a> {
    Ctx {
        lp: dir,
        bin: bin.to_path_buf(),
        out,
        config_template: template.to_string(),
        version: spec.version.clone(),
        run_timeout: Duration::from_secs(120),
        accept_as: IDENTITY.into(),
        max_retries: config_max_retries(template).unwrap_or(1),
        offline: true,
        priors: None,
    }
}

#[allow(clippy::too_many_lines)]
fn compare_arms(
    root: &Path,
    dir: &Path,
    spec: &Spec,
    bin: &Path,
    scratch: &Path,
    measured_template: &str,
    warmup_template: &str,
) -> Result<bool, String> {
    let all: Vec<TaskMeta> = spec.tasks.iter().map(SpecTask::meta).collect();
    let warm: Vec<TaskMeta> = spec
        .tasks
        .iter()
        .filter(|t| t.warmup)
        .map(SpecTask::meta)
        .collect();
    let tiers: BTreeMap<String, String> = spec
        .tasks
        .iter()
        .map(|t| (t.id.clone(), t.tier.clone()))
        .collect();
    let mut tier_names: Vec<&str> = tiers.values().map(String::as_str).collect();
    tier_names.sort_unstable();
    tier_names.dedup();
    let set = |tasks: &[TaskMeta]| TaskSet {
        version: spec.version.clone(),
        tasks: tasks.iter().map(|t| t.id.clone()).collect(),
        plan_tiers: tiers.clone(),
    };

    // 1. Warm-up: each candidate pinned on each warm-up task, one shared store.
    let warm_ctx = ctx(dir, spec, bin, scratch.join("warmup"), warmup_template);
    fs::create_dir_all(warm_ctx.out.join("runs")).map_err(|e| e.to_string())?;
    let warm_candidates = tier_coverage(&doctor(&warm_ctx, &warm[0])?, &tier_names)?;
    let schedule = warmup_schedule(&set(&warm), &warm, &warm_candidates, spec.floor);
    let store = warm_ctx.out.join("store");
    let mut warm_runs = Vec::new();
    for (rep, i, h, m) in &schedule {
        let task = &warm[*i];
        let graph_id = format!(
            "wu-{}-r{rep}-{}-{}",
            task.id,
            super::id_part(h),
            super::id_part(m)
        );
        let plan = RunPlan {
            dir_name: graph_id.clone(),
            graph_id,
            pin: Some((h.as_str(), m.as_str())),
            shared_repo: Some(&store),
        };
        warm_runs.push(run_one(
            &warm_ctx,
            task,
            *rep,
            tiers.get(&task.id).map(String::as_str),
            &plan,
        ));
    }

    // 2. Fold, as /meshloop learn does, and compare with the cell's own proposal.
    let store_cfg = warm_ctx.out.join("store-meshloop.toml");
    fs::write(&store_cfg, task_config(&warm_ctx, &warm[0])).map_err(|e| e.to_string())?;
    let input = fold_input(
        bin,
        &store,
        &store_cfg,
        warm_runs.iter().map(|r| r.graph_id.as_str()),
    )?;
    // The cell folds in a session at the store with no --config (recording cell-fold.json).
    let _ = fs::copy(&store_cfg, store.join("meshloop.toml"));
    let fold = priors_fold::fold(&input)?;
    let proposal = priors_fold::to_json(&fold, &utc_string(unix_now()), GENERATOR);
    let proposal_path = scratch.join("priors.json");
    fs::write(&proposal_path, &proposal).map_err(|e| e.to_string())?;
    let sha = meshloop_domain::digest::sha256_hex(proposal.as_bytes());
    let cell = fs::read_to_string(dir.join("cell-fold.json")).ok();
    let fold_matches = cell
        .as_deref()
        .is_some_and(|c| priors_fold::comparable(c) == priors_fold::comparable(&proposal));
    if !fold_matches {
        eprintln!(
            "bench-learning --offline: the Rust fold differs from the cell's recorded fold (benches/learning-plane/offline/cell-fold.json{}); this run's proposal: {}",
            if cell.is_none() { " is missing" } else { "" },
            proposal_path.display()
        );
    }
    let mut explored: Vec<(String, String, String)> = schedule
        .iter()
        .map(|(_, i, h, m)| {
            (
                h.clone(),
                m.clone(),
                tiers.get(&warm[*i].id).cloned().unwrap_or_default(),
            )
        })
        .collect();
    explored.sort();
    explored.dedup();
    let below_floor = explored
        .iter()
        .filter(|(h, m, t)| {
            !fold.priors.iter().any(|e| {
                &e.harness == h
                    && &e.model_ref == m
                    && &e.tier == t
                    && e.attempts >= spec.floor as usize
            })
        })
        .count();

    // 3. priors check against the measured configuration (read-only, no store).
    let measured_ctx = ctx(dir, spec, bin, scratch.join("check"), measured_template);
    let check_repo = measured_ctx.out.join("repo");
    fs::create_dir_all(&check_repo).map_err(|e| e.to_string())?;
    git(&check_repo, &["init", "-q", "-b", "main"])?;
    let check_cfg = measured_ctx.out.join("meshloop.toml");
    fs::write(&check_cfg, task_config(&measured_ctx, &all[0])).map_err(|e| e.to_string())?;
    let check = run_cmd(
        bin,
        &[
            "priors",
            "check",
            "--file",
            &path_str(&proposal_path),
            "--config",
            &path_str(&check_cfg),
            "--json",
        ],
        &check_repo,
        &measured_ctx.out.join("priors-check.json"),
        &measured_ctx.out.join("priors-check.err"),
        Duration::from_secs(60),
    );
    let check_v: Value = serde_json::from_str(&check.stdout).unwrap_or(Value::Null);
    let check_d = &check_v["data"];
    let not_a_candidate = check_d["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["code"] == "not_a_candidate")
        .count();
    let measured_candidates = tier_coverage(&doctor(&measured_ctx, &all[0])?, &tier_names)?;

    // 4. The two arms, a new store per task x repetition.
    let mut arms = Vec::new();
    for (name, priors) in [("baseline", false), ("priors-on", true)] {
        let mut c = ctx(dir, spec, bin, scratch.join(name), measured_template);
        fs::create_dir_all(c.out.join("runs")).map_err(|e| e.to_string())?;
        if priors {
            c.priors = Some(PriorsFile {
                path: proposal_path.clone(),
                sha256: sha.clone(),
                identity: IDENTITY.into(),
            });
        }
        let mut runs = Vec::new();
        for rep in 1..=spec.reps {
            for task in &all {
                runs.push(run_one(
                    &c,
                    task,
                    rep,
                    tiers.get(&task.id).map(String::as_str),
                    &RunPlan::measured(&task.id, rep),
                ));
            }
        }
        arms.push(Arm { name, runs });
    }

    // 5. Mechanism measurements.
    let (base, on) = (&arms[0].runs, &arms[1].runs);
    let key = |a: &super::AttemptRecord| {
        format!(
            "{}/{}",
            a.harness.as_deref().unwrap_or("?"),
            a.model_ref.as_deref().unwrap_or("?")
        )
    };
    let first = |r: &RunRecord| r.attempts.first().map(key);
    let twin = |r: &RunRecord| {
        base.iter()
            .find(|b| b.task_id == r.task_id && b.rep == r.rep)
    };
    let fits = |r: &RunRecord, k: &str| {
        let tier = r.plan_tier.as_deref().unwrap_or("Tier2");
        measured_candidates[tier]
            .as_array()
            .is_some_and(|c| c.iter().any(|x| x == k))
    };
    let outside = arms
        .iter()
        .flat_map(|a| &a.runs)
        .flat_map(|r| r.attempts.iter().map(move |a| (r, key(a))))
        .filter(|(r, k)| !fits(r, k))
        .count();
    let mut tier1_sorted: Vec<String> = measured_candidates["Tier1"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    let attempted = |r: &RunRecord| {
        let mut v: Vec<String> = r.attempts.iter().map(key).collect();
        v.sort();
        v
    };
    tier1_sorted.sort();
    // The measured-only task (t1-never) fails for every candidate, so the retry ladder
    // walks the whole Tier1 set.
    let probe = |r: &RunRecord| spec.tasks.iter().any(|t| !t.warmup && t.id == r.task_id);
    let set_changes = on
        .iter()
        .filter(|r| probe(r))
        .filter(|r| {
            twin(r).is_none_or(|b| attempted(b) != attempted(r)) || attempted(r) != tier1_sorted
        })
        .count()
        + base
            .iter()
            .filter(|r| probe(r) && attempted(r) != tier1_sorted)
            .count();
    let bound_violations = on
        .iter()
        .filter(|r| r.plan_tier.as_deref() == Some("Tier2"))
        .filter(|r| twin(r).is_none_or(|b| first(b) != first(r)))
        .count();
    let changed_top_mismatches = on
        .iter()
        .filter(|r| {
            let differs = twin(r).is_none_or(|b| first(b) != first(r));
            r.attempts.first().and_then(|a| a.changed_top) != Some(differs)
        })
        .count();
    let changed_first = on
        .iter()
        .filter(|r| {
            r.attempts
                .first()
                .is_some_and(|a| a.changed_top == Some(true))
        })
        .count();
    let not_in_effect = on
        .iter()
        .filter(|r| r.priors_in_effect != Some(true) || r.priors_sha256.as_deref() != Some(&sha))
        .count();
    let without_row = on
        .iter()
        .flat_map(|r| &r.attempts)
        .filter(|a| a.changed_top.is_none())
        .count();
    let other_digest = on
        .iter()
        .flat_map(|r| &r.attempts)
        .filter(|a| a.priors_sha256.as_deref().is_some_and(|s| s != sha))
        .count();
    let base_with_priors = base
        .iter()
        .filter(|r| {
            r.priors_in_effect != Some(false) || r.attempts.iter().any(|a| a.changed_top.is_some())
        })
        .count();
    let broken = arms
        .iter()
        .flat_map(|a| &a.runs)
        .chain(&warm_runs)
        .filter(|r| {
            r.run_timed_out
                || r.review_exit != Some(0)
                || r.attempts.is_empty()
                || (r.priors_accept_exit.is_some() && r.priors_accept_exit != Some(0))
        })
        .count();

    let measured: BTreeMap<&str, f64> = [
        ("lp.offline.warmup_keys_below_floor", below_floor as f64),
        (
            "lp.offline.fold_matches_cell",
            f64::from(u8::from(fold_matches)),
        ),
        (
            "lp.offline.priors_check_file_fault",
            f64::from(u8::from(
                !check_d["file_fault"].is_null() || check_v["ok"] != true,
            )),
        ),
        (
            "lp.offline.priors_check_ignored",
            check_d["ignored"]
                .as_array()
                .map_or(-1.0, |a| a.len() as f64),
        ),
        (
            "lp.offline.priors_check_not_a_candidate",
            not_a_candidate as f64,
        ),
        ("lp.offline.priors_runs_not_in_effect", not_in_effect as f64),
        (
            "lp.offline.priors_attempts_without_routing_row",
            without_row as f64,
        ),
        (
            "lp.offline.routing_rows_with_other_digest",
            other_digest as f64,
        ),
        (
            "lp.offline.baseline_runs_with_priors",
            base_with_priors as f64,
        ),
        ("lp.offline.dispatch_outside_candidates", outside as f64),
        ("lp.offline.candidate_set_changes", set_changes as f64),
        ("lp.offline.bound_violations", bound_violations as f64),
        (
            "lp.offline.changed_top_mismatches",
            changed_top_mismatches as f64,
        ),
        (
            "lp.offline.changed_top_first_attempts_min1",
            f64::from(u8::from(changed_first >= 1)),
        ),
        ("lp.offline.timeouts_or_aborted", broken as f64),
    ]
    .into_iter()
    .collect();

    let rules = load_rules(&dir.join("thresholds.toml"))?;
    let mut ok = true;
    let mut gates = Vec::new();
    println!("\nlearning-plane offline comparison ({})", spec.version);
    for (name, op, value) in &rules {
        let observed = measured.get(name.as_str()).copied();
        let pass = observed.is_some_and(|v| match op.as_str() {
            "eq" => (v - value).abs() < 1e-9,
            "lte" => v <= *value,
            "gte" => v >= *value,
            _ => false,
        });
        ok &= pass;
        println!(
            "  [{}] {name} {op} {value}: observed {}",
            if pass { "pass" } else { "FAIL" },
            observed.map_or("not measured".into(), |v| v.to_string())
        );
        gates.push(
            json!({"name": name, "op": op, "value": value, "observed": observed,
                          "status": if pass { "pass" } else { "fail" }}),
        );
    }
    if rules.len() != measured.len() {
        eprintln!(
            "bench-learning --offline: thresholds.toml gates {} of {} measured invariants",
            rules.len(),
            measured.len()
        );
        ok = false;
    }

    // 6. Every metric per arm (reported, not gated).
    let mut arm_reports = serde_json::Map::new();
    println!(
        "\n  arm         runs  success  first-attempt  retries  repair rounds  first dispatch (Tier1 | Tier2)  changed_top  wall p50/p95 s"
    );
    for arm in &arms {
        let report = arm_metrics(&arm.runs);
        println!(
            "  {:<10} {:>5}  {:>7}  {:>13}  {:>7}  {:>13}  {} | {}  {:>11}  {}/{}",
            arm.name,
            report["runs"],
            report["success"],
            report["first_attempt_success"],
            report["retries"],
            report["repair_rounds"],
            report["first_dispatch"]["Tier1"],
            report["first_dispatch"]["Tier2"],
            report["changed_top_attempts"],
            report["dispatch_wall_seconds_p50"],
            report["dispatch_wall_seconds_p95"],
        );
        arm_reports.insert(arm.name.into(), report);
    }
    println!(
        "  warm-up: {} pinned runs, {} keys:",
        warm_runs.len(),
        fold.priors.len()
    );
    for e in &fold.priors {
        println!(
            "    {}/{} {}: {} attempts, {} passed, pi = {:.3}",
            e.harness,
            e.model_ref,
            e.tier,
            e.attempts,
            e.successes,
            (e.successes as f64 + 1.0) / (e.attempts as f64 + 2.0)
        );
    }
    println!("  promoted priors sha256 {sha}");

    let record = json!({
        "schema": "meshloop.bench.learning-plane.offline.v1",
        "task_set_version": spec.version,
        "gates": gates,
        "status": if ok { "pass" } else { "fail" },
        "priors_sha256": sha,
        "warmup": {
            "runs": warm_runs.len(),
            "floor": spec.floor,
            "keys": fold.priors.iter().map(|e| json!({
                "harness": e.harness, "model_ref": e.model_ref, "tier": e.tier,
                "attempts": e.attempts, "successes": e.successes,
            })).collect::<Vec<_>>(),
            "excluded": fold.excluded.iter().map(|(r, n)| json!({"reason": r, "count": n})).collect::<Vec<_>>(),
        },
        "priors_check": check_d,
        "arms": arm_reports,
        "note": "Outcome metrics are reported, not gated: the gates assert the mechanism only. Tokens: the fixture harness reports none. Adjust/Decline: 0 by protocol.",
    });
    let out = root.join("artifacts").join("bench");
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    fs::write(
        out.join("learning_offline.json"),
        serde_json::to_string_pretty(&record).unwrap_or_default() + "\n",
    )
    .map_err(|e| format!("artifacts/bench/learning_offline.json: {e}"))?;
    println!(
        "bench-learning --offline: {} artifacts/bench/learning_offline.json",
        if ok { "pass" } else { "FAIL" }
    );
    Ok(ok)
}

fn arm_metrics(runs: &[RunRecord]) -> Value {
    let s = summarize(runs);
    let mut first_dispatch = serde_json::Map::new();
    for tier in ["Tier1", "Tier2"] {
        let mut counts: BTreeMap<String, u64> = BTreeMap::new();
        for r in runs.iter().filter(|r| r.plan_tier.as_deref() == Some(tier)) {
            if let Some(a) = r.attempts.first() {
                *counts
                    .entry(format!(
                        "{}/{}",
                        a.harness.as_deref().unwrap_or("?"),
                        a.model_ref.as_deref().unwrap_or("?")
                    ))
                    .or_default() += 1;
            }
        }
        first_dispatch.insert(tier.into(), json!(counts));
    }
    let wall: Vec<f64> = runs.iter().map(|r| r.dispatch_wall_seconds).collect();
    json!({
        "runs": runs.len(),
        "success": format!("{}/{}", runs.iter().filter(|r| r.success).count(), runs.len()),
        "first_attempt_success": format!("{}/{}", runs.iter().filter(|r| r.first_attempt_success).count(), runs.len()),
        "retries": runs.iter().map(|r| r.retries).sum::<u64>(),
        "repair_rounds": runs.iter().flat_map(|r| &r.attempts).filter_map(|a| a.repair_rounds).sum::<u64>(),
        "tokens": "not reported (fixture harness)",
        "adjust_decline": "0 / 0 by protocol",
        "first_dispatch": first_dispatch,
        "changed_top_attempts": runs.iter().flat_map(|r| &r.attempts).filter(|a| a.changed_top == Some(true)).count(),
        "dispatch_wall_seconds_p50": percentile(&wall, 0.5).map(round2),
        "dispatch_wall_seconds_p95": percentile(&wall, 0.95).map(round2),
        "summary": s,
    })
}

/// `"name" = { op = "eq", value = N }` lines of the thresholds file.
fn load_rules(path: &Path) -> Result<Vec<(String, String, f64)>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') || line.starts_with('[') || line.is_empty() {
            continue;
        }
        let parsed = line.split_once('=').and_then(|(name, rest)| {
            let op = rest.split("op = \"").nth(1)?.split('"').next()?;
            let value = rest
                .split("value = ")
                .nth(1)?
                .trim_end_matches(['}', ' '])
                .trim()
                .parse()
                .ok()?;
            Some((
                name.trim().trim_matches('"').to_string(),
                op.to_string(),
                value,
            ))
        });
        out.push(parsed.ok_or_else(|| format!("{}: cannot read `{line}`", path.display()))?);
    }
    if out.is_empty() {
        return Err(format!("{}: no gates", path.display()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_file_reads() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let rules =
            load_rules(&root.join("benches/learning-plane/offline/thresholds.toml")).unwrap();
        assert!(rules.iter().all(|(_, op, _)| op == "eq"));
        assert!(
            rules
                .iter()
                .any(|(n, _, v)| n == "lp.offline.fold_matches_cell" && *v == 1.0)
        );
    }
}
