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
  is `"top"`, `"mid"` (the default) or `"lightweight"`; any other value is a config
  error. (The single-model `model_tier` keeps its old lenient parse, where an unknown
  value means lightweight.) The list must be non-empty and hold at most 16 entries.
  Each `model_ref` must be non-empty, unique within the entry, at most 256 bytes, free
  of whitespace and control characters, and must not start with `-`. Unknown keys in an
  entry are rejected.

Routing expands every (harness, model) pair into its own candidate. The arm key is
(harness, `model_ref`, task tier), so each model has its own `routing_feedback` row and
its own tier filter. Probes, quota cooldown and headroom remain per harness.

Retry and fallback decide what a failed attempt has used up:

- A failure after the harness process started (a timeout, a collection fault, failed
  checks) uses up only that (harness, model). A retry may pick another model of the
  same harness.
- A failure before any process started (an invoke or spawn error, or a worktree that
  could not be created) is not about the model. It uses up every model of that harness.
  The attempt is recorded either way and counts toward `max_retries`.
- A failure that puts the harness into quota cooldown (`CapacityExhausted`) grants a
  retry only if a candidate on another, available harness is left. Otherwise the task
  ends `Failed` instead of waiting on a harness that routing filters out.

At dispatch the chosen `model_ref` replaces `{model_ref}` in `invoke_args_template`.
Arguments are a structured argv with no shell, so the model is one argument and is never
interpreted. The attempt records that same `model_ref` in evidence, and its routing
feedback uses it. `meshloop doctor` lists each configured pair under its harness with
its parsed tier; `doctor --json` reports them as `data.harnesses[].models`, each with
`model_ref`, the configured `model_tier` and the parsed `tier` (for example `TopTier`).

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

## Verification runs worker-written code (#181)

Whatever the harness, `verify_command` runs in the attempt worktree after the worker
exits. By default it runs on the host with the operator's full authority and
environment, so a check that executes repository code (`cargo test`, `npm test`, `make`,
build scripts, test runners that load repository config) executes code the worker
wrote, outside any worker containment. Either use checks that only read the worktree
(`git diff --check`, `grep`, a parse-only linter), or set `[verify] runner` to run every
check under `adapters/sandbox/meshloop-verify-sandbox`: worktree the only writable path,
no credential, no network unless `--allow-network`. How to enable it, toolchain paths and
what is and is not contained: [Contained verification](nodecode-containment.md#contained-verification).
`meshloop doctor` warns when a harness runs under containment but verification does not.

## Nodecode worker (#138)

Nodecode is a worker harness through operator configuration only, and only under the
containment in [nodecode-containment.md](nodecode-containment.md) (ADR 0038). That page
holds the threat model, the operator steps and the test evidence; the example entry is
also commented in `config/meshloop.example.toml`.

### Invocation findings

Spiked on 2026-11-11 against `nodecode 0.1.14+cef1be5a3`, always inside the sandbox, on
a fresh nodecode home, with stdin not a terminal (as meshloop runs a worker).

| Finding | Observed or inferred |
|---|---|
| `nodecode --model M --prompt-file F`, `--model M -- PROMPT`, `--model M @FILE` and a prompt piped on stdin all exit at once with code 2 and `nodecode needs a terminal (no controlling tty); run it in one.` The owner's first choice (the shell) is therefore unusable for a worker | observed |
| `nodecode rpc --ephemeral` with one `start_turn` line on stdin and stdin then closed exits 0 after the replay and organism frames, without running a turn | observed |
| Keeping stdin open until the turn ends works: frames arrive as JSON lines; the turn ends with a `session_stream_event` whose event is `turn.completed`, `turn.failed` or `turn.cancelled`; assistant text is in `turn.assistant_message_completed`, spend in `turn.usage`. A refused `start_turn` answers with `gateway_turn_command_result` status `rejected` and a code | observed |
| `start_turn`'s `model` takes the same `[PROVIDER/]MODEL[:EFFORT]` spec as `--model`, but nodecode splits off the `PROVIDER/` prefix only for a provider whose key comes from config, auth.json, OAuth or the environment (`ridden-provider-p`), not from a secret file. Unsplit, a spec goes whole to the default lane, OpenCode Zen, as a model id: `openai-completions/deepseek-v4-flash-free` with no key did, and failed with HTTP 400. So the driver names the provider separately and `model_ref` must carry one | 400 observed; the secret-file exclusion is from source (`gateway/gateway.lisp`, `target-from-spec`, `ridden-provider-p`) |
| A fresh home refuses a catalog provider with `unknown_provider ... the models.dev list that could name it has not loaded yet` until its first catalog fetch lands (about 1 s here). The driver retries for up to 60 s; `--models-catalog` seeds the cache | observed |
| `nodecode rpc` starts its organism as `setsid -w nodecode serve --store /tmp/nodecode-ephemeral-...`, and the shell tool runs commands under `setsid -w bash -c`. Both leave meshloop's process group, which is why the sandbox's PID namespace, not the group kill alone, is what ends them | observed (process list during the cancel test) |
| The keyless default (OpenCode Zen free tier) answered HTTP 429 "Free usage limit reached" to nodecode; nodecode retries a 429 with 60 s delays, so such a turn runs until meshloop's timeout | observed |
| `update.mode` defaults to `auto` on an installed build (a `current` link beside the release folder) and can move that link | source (`organism/update.lisp`); not triggered here |

### Exit codes of the worker (wrapper and driver)

| Code | Meaning |
|---|---|
| 0 | `turn.completed` |
| 1 | `turn.failed` (for example a provider error after nodecode's own retries) |
| 2 | driver usage error: `model_ref` without a provider, unreadable or empty prompt file |
| 3 | `start_turn` rejected (unknown provider or model, refused effort) |
| 4 | `nodecode rpc` ended or broke before the turn ended, or a driver fault |
| 5 | `turn.cancelled` |
| 125 | wrapper refusal (no bwrap, unsupported filesystem, operator home as worktree, not a linked worktree, bad option) |

Meshloop treats any non-zero exit as a failed attempt. The attempt's stdout evidence holds
the assistant's messages, one `nodecode-turn: usage PROVIDER/MODEL input N output N
cost_usd X` line per model call, and a final `nodecode-turn: <outcome>` line. The exact
value of the bound key is replaced by `[redacted]` in stdout, stderr and the raw log.

The probe goes through the wrapper (`version_args = ["--version"]`, plus `--nodecode`
when the operator names a private copy). It runs `nodecode --version` inside the sandbox
and reports one line that also names the containment, so `meshloop doctor --json` shows,
for example, `nodecode 0.1.14+cef1be5a3 (contained: meshloop-nodecode-sandbox,
bubblewrap 0.11.1)`. A probe without bwrap or user namespaces fails, and the harness is
reported unsupported.
