# Development harness contract

Selected set: codex, claude-code, pi, grok, agy. All five remain required for full
cross-harness acceptance. AGENTS.md is canonical for **developing Meshloop**.

Product operator skills (`skills/meshloop-*`, slash `/meshloop:…`) are the R1 UX
(ADR 0001 / 0017). They wrap the compiled binary only. They are not AFD discovery
and do not replace AGENTS.md while hacking on this repo.

At initialization, installed AFD 0.6.4 recognizes Codex, Claude Code, and Pi instruction
surfaces. Its audit marks Grok unsupported and Agy discovery generated-only; a filename
does not prove discovery. No complete five-harness apply receipt exists at bootstrap.

AFD workflow: audit -> plan -> external stage -> readiness -> live disposable tests ->
exact-token apply -> verify. Preserve unsupported/unavailable results; never silently
drop a selected harness or manually copy staged files to bypass a failed apply gate.
AFD staging/evidence/receipts stay outside Meshloop. Re-plan after relevant state changes.

Until full verification, launch an agent only with an explicit instruction to read
AGENTS.md and relevant linked documents, and verify that it actually does so before
assigning implementation. This is a manual session practice, not an AFD discovery claim.
Resolve missing AFD contracts upstream as a separate scoped change. Do not equate Agy
with Antigravity or invent a Grok instruction filename.

## Quota windows

Meshloop does not call a vendor quota API. Each selected harness CLI was checked for a
remaining-window command. None expose one, so QACR keeps the local cooldown breaker and
does not invent numeric limits.

| Harness | Vendor quota window |
|---|---|
| codex | absent |
| claude-code | absent |
| pi | absent |
| grok | absent |
| agy | absent |

`grok usage` prints persisted token and cost totals for one session id. That output is
session accounting. Meshloop leaves the command unused and does not treat those totals
as a remaining window.

Routing behavior for the local record:

- A missing quota record stays selectable.
- A present Open breaker inside its cooldown is filtered out
  (`candidate_in_open_cooldown_is_filtered_out`).
- A stale Open breaker whose cooldown has elapsed is selectable again.

## Model selection

Issue #134; ADR 0038 "Model selection". A `[harnesses.<name>]` entry names its models in
one of two forms. Use one form per entry; setting both is a config error.

- **Single model (unchanged):** `model_ref = "..."` with optional `model_tier`
  (`"mid"` when omitted). Configs written before #134 load and route as before.
- **List:** `models = [{ model_ref = "...", model_tier = "top" }, ...]`. `model_tier`
  defaults to `"mid"` per entry. The list must be non-empty and hold at most 16 entries.
  Each `model_ref` must be non-empty, unique within the entry, at most 256 bytes, free
  of whitespace and control characters, and must not start with `-`. Unknown keys in an
  entry are rejected.

Routing expands every (harness, model) pair into its own candidate. The arm key is
(harness, `model_ref`, task tier), so each model has its own `routing_feedback` row and
its own tier filter. Retry and fallback track these arms: a retry may pick another model
of the same harness. Probes, quota cooldown and headroom remain per harness.

At dispatch the chosen `model_ref` replaces `{model_ref}` in `invoke_args_template`.
Arguments are a structured argv with no shell, so the model is one argument and is never
interpreted. The attempt records that same `model_ref` in evidence, and its routing
feedback uses it. `meshloop doctor` lists each configured pair under its harness;
`doctor --json` reports them as `data.harnesses[].models`.

Meshloop ships no model flag (ADR 0003): the shipped templates do not pass
`{model_ref}`, so each CLI runs its own default model and `model_ref` remains a label.
To select a model, the operator adds the CLI's own model flag to the template. The
examples below were checked against each CLI's `--help` on 2026-11-10; they are not
defaults, and live dispatch was not run.

| Harness | Version checked | Help text | Example template |
|---|---|---|---|
| codex | `codex-cli 0.167.0` | `codex exec --help`: `-m, --model <MODEL>` "Model the agent should use" | `["exec", "--model", "{model_ref}", "-"]` with `prompt_via = "stdin"` |
| grok | `grok 1.0.77 (50ca579fa996) [stable]` | `grok --help`: `-m, --model <MODEL>` "Model ID to use"; `grok models` lists IDs | `["--model", "{model_ref}", "--prompt-file", "{prompt_file}"]` |
| claude | `2.1.322 (Claude Code)` | `claude --help`: `--model <model>` "Model for the current session", an alias or a full name | `["--print", "--model", "{model_ref}", "{prompt}"]` with `prompt_via = "arg"` |

For example:

```toml
[harnesses.codex]
kind = "codex"
executable = "codex"
prompt_via = "stdin"
invoke_args_template = ["exec", "--model", "{model_ref}", "-"]
models = [
  { model_ref = "<fast-model-id>", model_tier = "mid" },
  { model_ref = "<strongest-model-id>", model_tier = "top" },
]
```

Model IDs change between CLI releases. Take them from the CLI, not from this page.

## Native dispatch support

Accepted with ADR 0039 (2026-11-10); the table is empty until #152 records live evidence.

This is a mirror of the compiled-in proven set in the composition root (for example
`crates/meshloop-cli/src/native_support.rs`). Native dispatch is honoured only for a
`kind` listed in that table. The table is read-only: it is not configurable through
`meshloop.toml`, and an operator cannot self-certify a harness. It changes only by pull
request with new live evidence from #152. Nodecode also needs an ADR 0038 amendment.
`doctor` warns when a probed version is outside the observed range.

| Kind | Observed version range | Live evidence (#152) | Date |
|---|---|---|---|
| (none proven yet) | — | — | — |
