//! `bench-learning --compare` (#140): evaluates a measured arm against the baseline of
//! record with the thresholds fixed in advance on #144
//! (`benches/learning-plane/thresholds-lp-4.json`), condition by condition, and states
//! the verdict the numbers give: GO, NO-GO, or INCONCLUSIVE, which counts as NO-GO.
//! It reads two results files and dispatches nothing.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use serde_json::{Value, json};

use super::{Results, RunRecord, dispatch_wall, percentile, read_json, summarize};

struct Condition {
    id: &'static str,
    text: String,
    observed: String,
    holds: bool,
}

pub fn compare(
    root: &Path,
    baseline: &Path,
    measured: &Path,
    thresholds: Option<&Path>,
    out: Option<&Path>,
) -> ExitCode {
    let t_path = thresholds.map_or_else(
        || root.join("benches/learning-plane/thresholds-lp-4.json"),
        Path::to_path_buf,
    );
    let loaded = (|| -> Result<(Results, Results, Value), String> {
        Ok((
            read_json(baseline)?,
            read_json(measured)?,
            read_json(&t_path)?,
        ))
    })();
    let (b, m, t) = match loaded {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bench-learning --compare: {e}");
            return ExitCode::FAILURE;
        }
    };
    let report = evaluate(&b, &m, &t);
    println!("{}", report["text"].as_str().unwrap_or_default());
    if let Some(out) = out
        && let Err(e) = fs::write(
            out,
            serde_json::to_string_pretty(&report["json"]).unwrap_or_default() + "\n",
        )
    {
        eprintln!("bench-learning --compare: {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn first(r: &RunRecord) -> Option<String> {
    r.attempts.first().map(|a| {
        format!(
            "{}/{}",
            a.harness.as_deref().unwrap_or("?"),
            a.model_ref.as_deref().unwrap_or("?")
        )
    })
}

fn tier_of(r: &RunRecord) -> &str {
    r.plan_tier.as_deref().unwrap_or("Tier2")
}

fn wall_p50(runs: &[RunRecord], tier: &str) -> Option<f64> {
    let w: Vec<f64> = runs
        .iter()
        .filter(|r| tier_of(r) == tier)
        .map(dispatch_wall)
        .collect();
    percentile(&w, 0.5)
}

fn tokens_mean(summary: &Value, arm: &str) -> Option<f64> {
    summary["by_harness_model"]
        .as_array()?
        .iter()
        .find(|a| {
            format!(
                "{}/{}",
                a["harness"].as_str().unwrap_or_default(),
                a["model_ref"].as_str().unwrap_or_default()
            ) == arm
        })
        .and_then(|a| a["tokens_total_mean"].as_f64())
}

fn failing_tasks(runs: &[RunRecord], min_reps: usize) -> Vec<String> {
    let mut fails: BTreeMap<&str, usize> = BTreeMap::new();
    for r in runs.iter().filter(|r| !r.success) {
        *fails.entry(&r.task_id).or_default() += 1;
    }
    fails
        .into_iter()
        .filter(|(_, n)| *n >= min_reps)
        .map(|(t, n)| format!("{t} ({n})"))
        .collect()
}

fn fmt(x: Option<f64>) -> String {
    x.map_or_else(|| "n/a".into(), |v| format!("{v:.2}"))
}

#[allow(clippy::too_many_lines)]
fn evaluate(b: &Results, m: &Results, t: &Value) -> Value {
    let runs = &m.runs;
    let sm = summarize(runs);
    let n = runs.len();
    let max_retries = t["max_retries"].as_u64().unwrap_or(3) as usize;
    let ok = runs.iter().filter(|r| r.success).count();
    let retry_runs = runs.iter().filter(|r| !r.retry_resumes.is_empty()).count();
    let exhausted = runs
        .iter()
        .filter(|r| !r.success && r.attempts.len() >= max_retries)
        .count();
    let twin = |r: &RunRecord| {
        b.runs
            .iter()
            .find(|x| x.task_id == r.task_id && x.rep == r.rep)
    };
    let mut pre = Vec::new();
    let mut go = Vec::new();
    let mut nogo = Vec::new();

    // Preconditions: the comparison is the one #144 fixed.
    let cfg = t["config_sha256"].as_str().unwrap_or_default();
    pre.push(Condition {
        id: "P1",
        text: "same task set, configuration hash and repetitions as the baseline".into(),
        observed: format!(
            "task set {} / {}; config {} / {}; reps {} / {}",
            b.task_set_version,
            m.task_set_version,
            b.config_sha256
                .as_deref()
                .unwrap_or("?")
                .get(..12)
                .unwrap_or("?"),
            m.config_sha256
                .as_deref()
                .unwrap_or("?")
                .get(..12)
                .unwrap_or("?"),
            b.reps_requested,
            m.reps_requested
        ),
        holds: b.config_sha256.as_deref() == Some(cfg)
            && m.config_sha256.as_deref() == Some(cfg)
            && m.task_set_version == t["task_set_version"].as_str().unwrap_or_default()
            && u64::from(m.reps_requested) == t["reps"].as_u64().unwrap_or(3),
    });
    let sha = m.priors_sha256.clone().unwrap_or_default();
    let applied = runs
        .iter()
        .filter(|r| r.priors_in_effect == Some(true) && r.priors_sha256.as_deref() == Some(&sha))
        .count();
    pre.push(Condition {
        id: "P2",
        text: "the promoted priors file is in effect in every measured store".into(),
        observed: format!("{applied} of {n} runs, sha256 {sha}"),
        holds: !sha.is_empty() && applied == n,
    });

    // GO 1, 2.
    let fail2 = failing_tasks(runs, 2);
    go.push(Condition {
        id: "GO-1",
        text: "success >= 23/24 and no task failing in >= 2 repetitions".into(),
        observed: format!("{ok}/{n}; tasks failing in >= 2 reps: {fail2:?}"),
        holds: ok as u64 >= t["go"]["success_min"].as_u64().unwrap_or(23) && fail2.is_empty(),
    });
    go.push(Condition {
        id: "GO-2",
        text: "<= 2 runs need a retry; none exhausts the budget".into(),
        observed: format!("{retry_runs} runs retried, {exhausted} exhausted"),
        holds: retry_runs as u64 <= t["go"]["retry_runs_max"].as_u64().unwrap_or(2)
            && exhausted == 0,
    });

    // GO 3: one learning effect traceable to the promoted file.
    let changed_any = runs
        .iter()
        .flat_map(|r| &r.attempts)
        .filter(|a| a.changed_top == Some(true))
        .count();
    let differing: Vec<&RunRecord> = runs
        .iter()
        .filter(|r| twin(r).is_none_or(|x| first(x) != first(r)))
        .collect();
    let mut per_task: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for r in &differing {
        *per_task.entry((tier_of(r), &r.task_id)).or_default() += 1;
    }
    let min_reps = t["go"]["first_dispatch_change_reps_min"]
        .as_u64()
        .unwrap_or(2) as usize;
    let changed_tasks: Vec<String> = per_task
        .iter()
        .filter(|(_, n)| **n >= min_reps)
        .map(|((tier, task), n)| format!("{tier} {task} ({n})"))
        .collect();
    let affected_ok = differing.iter().all(|r| r.first_attempt_success);
    let traced = differing.iter().all(|r| {
        r.attempts
            .first()
            .is_some_and(|a| a.changed_top == Some(true))
    });
    let g3a = !changed_tasks.is_empty() && affected_ok && traced;
    let (t1, t2, t3) = (
        wall_p50(runs, "Tier1"),
        wall_p50(runs, "Tier2"),
        wall_p50(runs, "Tier3"),
    );
    let all_wall: Vec<f64> = runs.iter().map(dispatch_wall).collect();
    let p95 = percentile(&all_wall, 0.95);
    let lim = &t["no_go"]["wall_p50_max_s"];
    let past =
        |x: Option<f64>, tier: &str| x.is_some_and(|v| v > lim[tier].as_f64().unwrap_or(f64::MAX));
    let wall_nogo = past(t1, "Tier1")
        || past(t2, "Tier2")
        || past(t3, "Tier3")
        || p95.is_some_and(|v| v > t["no_go"]["overall_p95_max_s"].as_f64().unwrap_or(f64::MAX));
    let tier2_changed = runs
        .iter()
        .filter(|r| tier_of(r) == "Tier2")
        .flat_map(|r| &r.attempts)
        .any(|a| a.changed_top == Some(true));
    let g3b_raw = t2.is_some_and(|v| v <= t["go"]["tier2_wall_p50_max_s"].as_f64().unwrap_or(0.0));
    let g3b = g3b_raw && !wall_nogo && tier2_changed;
    let mut tok_go = Vec::new();
    let mut g3c_raw = false;
    for (arm, max) in t["go"]["tokens_mean_max"].as_object().into_iter().flatten() {
        let v = tokens_mean(&sm, arm);
        let hit = v.is_some_and(|v| v <= max.as_f64().unwrap_or(0.0));
        g3c_raw |= hit;
        tok_go.push(format!("{arm} {} (<= {max}: {hit})", fmt(v)));
    }
    let g3c = g3c_raw && changed_any > 0;
    go.push(Condition {
        id: "GO-3a",
        text: "first dispatch for some tier differs from baseline in >= 2 of 3 repetitions (same task), every affected run succeeds first time, traced by changed_top".into(),
        observed: format!(
            "runs whose first dispatch differs: {}; tasks differing in >= 2 reps: {changed_tasks:?}; affected all first-time: {affected_ok}; all changed_top: {traced}",
            differing.len()
        ),
        holds: g3a,
    });
    go.push(Condition {
        id: "GO-3b",
        text: "Tier2 wall p50 <= 10.5 s with no tier past its no-go, traced by changed_top on a Tier2 attempt".into(),
        observed: format!(
            "Tier2 p50 {} s (threshold met: {g3b_raw}); a tier past no-go: {wall_nogo}; Tier2 changed_top: {tier2_changed}",
            fmt(t2)
        ),
        holds: g3b,
    });
    go.push(Condition {
        id: "GO-3c",
        text: "claude/sonnet mean tokens <= 149,152 or codex/gpt-6.1-sol <= 10,188, traced by changed_top".into(),
        observed: format!(
            "{}; attempts with changed_top: {changed_any}",
            tok_go.join("; ")
        ),
        holds: g3c,
    });
    let go3 = g3a || g3b || g3c;

    // NO-GO.
    nogo.push(Condition {
        id: "NOGO-1",
        text: "success <= 22/24, or a task failing in >= 2 repetitions".into(),
        observed: format!("{ok}/{n}; {fail2:?}"),
        holds: (ok as u64) <= t["no_go"]["success_max"].as_u64().unwrap_or(22) || !fail2.is_empty(),
    });
    nogo.push(Condition {
        id: "NOGO-2",
        text: "> 3 runs need a retry, or any run exhausts the budget".into(),
        observed: format!("{retry_runs} retried, {exhausted} exhausted"),
        holds: retry_runs as u64 > t["no_go"]["retry_runs_max"].as_u64().unwrap_or(3)
            || exhausted > 0,
    });
    nogo.push(Condition {
        id: "NOGO-3",
        text: "wall p50 above Tier1 28.7 s, Tier2 13.8 s or Tier3 75.9 s; overall p95 above 61.3 s"
            .into(),
        observed: format!(
            "p50 Tier1 {} / Tier2 {} / Tier3 {} s; p95 {} s",
            fmt(t1),
            fmt(t2),
            fmt(t3),
            fmt(p95)
        ),
        holds: wall_nogo,
    });
    let mut tok_nogo = Vec::new();
    let mut tok_over = false;
    for (arm, max) in t["no_go"]["tokens_mean_max"]
        .as_object()
        .into_iter()
        .flatten()
    {
        let v = tokens_mean(&sm, arm);
        let over = v.is_some_and(|v| v > max.as_f64().unwrap_or(f64::MAX));
        tok_over |= over;
        tok_nogo.push(format!("{arm} {} (> {max}: {over})", fmt(v)));
    }
    nogo.push(Condition {
        id: "NOGO-4",
        text: "tokens per attempt above claude/sonnet 196,253, codex/gpt-6-luna 23,209 or codex/gpt-6.1-sol 33,958".into(),
        observed: tok_nogo.join("; "),
        holds: tok_over,
    });
    let outside = runs
        .iter()
        .flat_map(|r| r.attempts.iter().map(move |a| (r, a)))
        .filter(|(r, a)| {
            let k = format!(
                "{}/{}",
                a.harness.as_deref().unwrap_or("?"),
                a.model_ref.as_deref().unwrap_or("?")
            );
            !m.tier_candidates[tier_of(r)]
                .as_array()
                .is_some_and(|c| c.iter().any(|x| x == &Value::String(k.clone())))
        })
        .count();
    let set_changed = m.tier_candidates != b.tier_candidates;
    let tiers_changed = m.plan_tiers != b.plan_tiers;
    nogo.push(Condition {
        id: "NOGO-5",
        text: "a first dispatch outside the tier's dispatchable set; a prior that adds or removes a candidate; a tier hint that lowers a tier".into(),
        observed: format!(
            "dispatches outside tier_candidates: {outside}; candidate set differs from baseline: {set_changed}; plan tiers differ: {tiers_changed} (a v1 priors file carries no tier hints)"
        ),
        holds: outside > 0 || set_changed || tiers_changed,
    });
    let timeouts = runs.iter().filter(|r| r.run_timed_out).count();
    let aborted = runs
        .iter()
        .filter(|r| r.review_exit != Some(0) || r.priors_accept_exit.is_some_and(|c| c != 0))
        .count();
    let full = n as u64 == t["runs"].as_u64().unwrap_or(24) && m.stopped_early.is_none();
    nogo.push(Condition {
        id: "NOGO-6",
        text: "any timeout, aborted task, or quota stop before 3 full repetitions".into(),
        observed: format!(
            "{timeouts} timeouts, {aborted} aborted, {n} runs, stopped early: {:?}",
            m.stopped_early
        ),
        holds: timeouts > 0 || aborted > 0 || !full,
    });

    let pre_ok = pre.iter().all(|c| c.holds);
    let any_nogo = nogo.iter().any(|c| c.holds);
    let verdict = if !pre_ok {
        "INCONCLUSIVE (precondition failed) = NO-GO"
    } else if any_nogo {
        "NO-GO"
    } else if go[0].holds && go[1].holds && go3 {
        "GO"
    } else {
        "INCONCLUSIVE = NO-GO"
    };
    let mut text = String::from("learning-plane comparison against the #144 thresholds (final)\n");
    let mut rows = Vec::new();
    for (group, list) in [("precondition", &pre), ("GO", &go), ("NO-GO", &nogo)] {
        for c in list.iter() {
            text.push_str(&format!(
                "  [{}] {} {}\n        observed: {}\n",
                if c.holds { "holds" } else { "fails" },
                c.id,
                c.text,
                c.observed
            ));
            rows.push(json!({"group": group, "id": c.id, "condition": c.text,
                             "observed": c.observed, "holds": c.holds}));
        }
    }
    text.push_str(&format!("verdict: {verdict}\n"));
    json!({
        "text": text,
        "json": {
            "verdict": verdict,
            "conditions": rows,
            "measured": {"priors_sha256": m.priors_sha256, "runs": n, "summary_priors": sm["priors"]},
        }
    })
}

#[cfg(test)]
mod tests {
    use super::super::AttemptRecord;
    use super::*;

    fn run(task: &str, rep: u32, tier: &str, model: &str, changed: bool) -> RunRecord {
        RunRecord {
            task_id: task.into(),
            rep,
            plan_tier: Some(tier.into()),
            success: true,
            first_attempt_success: true,
            review_exit: Some(0),
            dispatch_wall_seconds: 10.0,
            priors_in_effect: Some(true),
            priors_sha256: Some("s".into()),
            attempts: vec![AttemptRecord {
                harness: Some("h".into()),
                model_ref: Some(model.into()),
                checks_passed: true,
                changed_top: Some(changed),
                ..AttemptRecord::default()
            }],
            ..RunRecord::default()
        }
    }

    fn results(runs: Vec<RunRecord>) -> Results {
        Results {
            task_set_version: "lp-4".into(),
            config_sha256: Some("c".into()),
            reps_requested: 3,
            priors_sha256: Some("s".into()),
            tier_candidates: json!({"Tier1": ["h/a", "h/b"]}),
            runs,
            ..Results::default()
        }
    }

    fn thresholds() -> Value {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut t: Value =
            read_json(&root.join("benches/learning-plane/thresholds-lp-4.json")).unwrap();
        t["config_sha256"] = json!("c");
        t
    }

    /// 8 tasks x 3 repetitions, every first dispatch to MODEL.
    fn arm(model: &str, changed: bool) -> Vec<RunRecord> {
        (1..=3)
            .flat_map(|r| (0..8).map(move |i| run(&format!("t{i}"), r, "Tier1", model, changed)))
            .collect()
    }

    #[test]
    fn unchanged_routing_is_inconclusive_and_changed_is_go() {
        let base = results(arm("a", false));
        let v = evaluate(&base, &results(arm("a", false)), &thresholds());
        assert_eq!(
            v["json"]["verdict"], "INCONCLUSIVE = NO-GO",
            "{}",
            v["text"]
        );
        let v = evaluate(&base, &results(arm("b", true)), &thresholds());
        assert_eq!(v["json"]["verdict"], "GO", "{}", v["text"]);
        // A change no advisory row traces is not a learning effect.
        let v = evaluate(&base, &results(arm("b", false)), &thresholds());
        assert_eq!(
            v["json"]["verdict"], "INCONCLUSIVE = NO-GO",
            "{}",
            v["text"]
        );
    }

    #[test]
    fn a_failure_twice_or_outside_dispatch_is_no_go() {
        let base = results(arm("a", false));
        let mut runs = arm("b", true);
        runs[0].success = false;
        runs[8].success = false;
        let v = evaluate(&base, &results(runs), &thresholds());
        assert_eq!(v["json"]["verdict"], "NO-GO", "{}", v["text"]);
        let v = evaluate(&base, &results(arm("z", true)), &thresholds());
        assert_eq!(v["json"]["verdict"], "NO-GO", "{}", v["text"]);
        let mut short = arm("b", true);
        short.pop();
        let v = evaluate(&base, &results(short), &thresholds());
        assert_eq!(v["json"]["verdict"], "NO-GO", "{}", v["text"]);
    }
}
