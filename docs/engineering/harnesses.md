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

## Host-side git

A worker can write anything in its worktree, and an unsandboxed one anything the
operator can. Meshloop runs git on the host in that worktree afterwards (diff, status,
add, commit, merge, cherry-pick, reset, worktree add/remove, cleanup probes), with the
operator's authority. Issue [#180](https://github.com/smota/meshloop/issues/180): every
one of those git processes is built by `meshloop_adapters::host_git` (`command` for
`std::process::Command`, `spec` for owned spawns), so git never runs a program the
worker chose and never follows a `.git` link the worker rewrote.

Neutralised for every host-side git process:

- **System and global config are not read** (`GIT_CONFIG_NOSYSTEM=1`,
  `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_ATTR_NOSYSTEM=1`). Filter, diff and merge drivers
  defined there (git-lfs, for example) therefore never run on worker-chosen content
  selected by a worker's `.gitattributes`. A fixed allowlist of data-only keys is read
  from the operator's system and global config once per process and passed back with
  `-c`: `core.autocrlf`, `core.eol`, `core.safecrlf`, `core.longpaths`,
  `core.symlinks`, `core.fscache` (short plain words only), `core.excludesFile` (a path
  to an ignore file) and `safe.directory` (a protected-config key that git honours from
  `-c`). Identity for Meshloop's own commits
  stays `GIT_AUTHOR_*`/`GIT_COMMITTER_*` = `meshloop`.
- **Hooks**: `core.hooksPath` is a path below a non-directory (`/dev/null` on Unix, the
  Meshloop executable on Windows), so no hook (`pre-commit`, `post-checkout`,
  `reference-transaction`, `fsmonitor-watchman`, ...) can exist.
- **Executable config keys a repository config could set**, overridden on the command
  line: `core.fsmonitor=false`, `core.editor=:` and `sequence.editor=:` (plus
  `GIT_EDITOR`/`GIT_SEQUENCE_EDITOR`), `commit.gpgSign=false`, `tag.gpgSign=false`,
  `merge.verifySignatures=false`, `log.showSignature=false` (no `gpg.program`),
  `protocol.allow=never`, `credential.helper=` and `GIT_NO_LAZY_FETCH=1`,
  `GIT_TERMINAL_PROMPT=0` (no transport, so no `core.sshCommand`, proxy, `uploadpack`,
  credential helper or askpass), `submodule.recurse=false`,
  `diff.ignoreSubmodules=all`, `gc.auto=0` and `maintenance.auto=false`. `--no-pager`
  on every call. Diffs in a worktree also pass `--no-ext-diff --no-textconv
  --ignore-submodules=all`, and status passes `--ignore-submodules=all` (adapter and
  cleanup), so git never recurses into a nested repository and runs its config.
- **Nested repositories**: `commit_all` takes the attempt base. Before every `add -A`
  (the first commit, every repair round and resumed verification), it checks the
  worktree link, then walks the worktree without following symlinks or junctions: any
  entry named `.git` in any case, or the NTFS short name `git~1`, below the top level
  refuses the attempt. It then reads the index (`ls-files --stage`, no helper process)
  and refuses any gitlink (mode 160000) that is not in the attempt base or whose
  `<path>/.git` exists. Staging is `add -A -- . ':(exclude,top,literal)<gitlink>'`
  with one exclude per gitlink in the index: gitlinks carry no content, and `add`
  would otherwise run git inside a populated one under that repository's config.
  After staging it reads the index again and refuses a gitlink staged in between.
  Each of these refusals is `WorkspaceError::NestedGit { path, reason }`, a failing
  `nested-git` evidence row naming the worktree-relative path, and a failed attempt
  with no repair round. A git failure while reading or staging the index, or a
  worktree walk that cannot complete, is `WorkspaceError::StagingFailed`: a failing
  `staging` evidence row (`staging-failed`), a failed attempt, never a failed run. An
  entry that vanishes during the walk is skipped. The refusals include a worker that
  fills an (uninitialised) submodule directory of a repository that uses submodules.
  A change that adds or moves a gitlink also fails the `gitlinks` check. Resumed
  verification uses the attempt's recorded base, else the run's base revision.
- **The worker's process tree ends before verification** (ADR 0025): when a worker
  exits normally, Meshloop ends every process left in its tree before collecting the
  outcome, so nothing the worker started can race host-side git. On Windows it
  terminates the job object and observes it empty; on Unix it kills the process group
  and polls until no member is left: `/proc/<pid>/stat` process-group fields on
  Linux, `kill -0 -- -<pgid>` elsewhere, where only "No such process" counts as empty
  (ADR 0025, amendment of 2026-11-11). If the tree is not observed empty
  within 5 s the attempt fails (`worker-process-tree-survived`) and no host-side git
  runs in its worktree.
- **Inherited environment** that would redirect git or name a program is removed:
  `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_INDEX_FILE`, object-directory
  variables, `GIT_CONFIG*` parameters, `GIT_ATTR_SOURCE`, `GIT_EXTERNAL_DIFF`, `GIT_SSH*`,
  `GIT_PROXY_COMMAND`, askpass and `GIT_PAGER`.
- **The worktree link**: before git runs in an attempt (or integrate/deliverable)
  worktree, `<worktree>/.git` must be a regular file naming an admin dir directly under
  `<common dir>/worktrees/`, whose `gitdir` points back at this `.git` and whose
  `commondir` is the repository's. Otherwise git is not run: the adapter returns
  `WorkspaceError::GitLinkModified { path, reason }`, verification records a failing
  `git-link` evidence row and fails the attempt (`git-link-modified`), removal refuses
  the worktree, and cleanup treats its content as unknown (blocked). A verified
  worktree is then pinned to the canonical admin dir (`--git-dir=<admin>
  --work-tree=.`), so git never reads the link itself. A repair round that hits a
  refused worktree fails the attempt the same way.
- **The prompt file** `.meshloop-prompt-<attempt>` is created with `create_new` and
  `O_NOFOLLOW` (`FILE_FLAG_OPEN_REPARSE_POINT` on Windows). Any existing entry,
  including a dangling symlink, refuses the attempt (`prompt-file-exists`); it is never
  written through. A worker change that touches any `.meshloop-prompt-*` path (any
  case) fails the `reserved-paths` check, whatever `allowed_paths` says.

Not neutralised, by design or for now:

- **Operator global and system config is dropped**, apart from the allowlist above.
  Global git-lfs filters, `core.attributesFile` drivers, global aliases and credential
  helpers do not apply to Meshloop's git. An LFS repository needs the filter in its
  repository config (`git lfs install --local`) for Meshloop's commits to store
  pointers. Landing has the same cause: when LFS is configured only globally,
  Meshloop's checkout and merge into the operator's checkout (`integrate --into`)
  write the pointer text, not the object content, into the working tree; run
  `git lfs pull` (or `git lfs install --local` beforehand). Global `safe.directory`
  exemptions and `core.excludesFile` still apply (imported).
- **Drivers in the repository's own config** (`filter.<x>`, `merge.<x>.driver`) still
  run when a worker's `.gitattributes` selects them: they are the operator's programs,
  chosen for this repository. Their names cannot be enumerated to override.
- **A worker that can write the repository's common dir** (any unsandboxed harness can)
  can change that repository config, `info/attributes` or the admin dir itself; such a
  worker already runs with the operator's authority. The overrides above still stop the
  listed executable keys; config-defined hooks of a future git (`hook.<name>.command`)
  are not covered.
- **Time of check**: the link and the nested-repository checks run before git, and git
  is pinned to the checked admin dir. Processes in the worker's process group or job
  are ended before verification (above). A process that left the group on Unix
  (`setsid`, `daemon`) or was started outside the job is not seen and could still
  change the worktree during that window; staging excludes every gitlink, a gitlink
  it stages is refused by the post-`add` index check, and status/diff never recurse
  into submodules. Native dispatch (ADR 0039) has no process tree, so for native
  workers these checks are the only guard. In a container
  whose PID 1 does not reap orphans, zombies keep the group "alive" and attempts fail
  with `worker-process-tree-survived`.
- `xtask` and test helpers run plain git on fixture repositories and are not host-side
  product paths.

Tests: `crates/meshloop-cli/tests/host_git_hardening.rs` (Unix: a `/bin/sh` worker that
rewrites `.git`, plants hooks, sets executable repository config, selects global-config
drivers with `.gitattributes`, commits symlinks at future prompt names, or creates a
nested repository whose own config defines a filter; no marker file appears across
verify, integrate and cleanup), `git::tests::a_modified_git_link_is_refused_before_git_runs`,
`git::tests::nested_git_entries_are_found_without_following_links`,
`harness::prompt_file_tests` and `verify::tests::prompt_file_paths_are_reserved_anywhere_in_a_change`.

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
