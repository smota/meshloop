# Learning plane: nodecode facts, cell install and CI

Supporting detail for [ADR 0038](adr/0038-nodecode-learning-plane-and-harness.md). The
ADR holds the decisions; this page holds the grounding facts and mechanism detail.

## Nodecode facts at 0.1.13+98dd6dc80

Read from the installed release and the local nodecode home on 2026-11-10. Nothing was
run. Release source paths are relative to `~/.local/share/nodecode/98dd6dc80/src/`.

| Fact | Source | Verified locally |
|---|---|---|
| Cells are declared with `nle:define-cell`; folder name = system name = package | `cells/template/cell.lisp` | yes, source |
| `(new-cell NAME :source DIR)` makes a cell whose folder lives in `DIR` and links it into the home's cells folder; `DIR` must be absent or empty | `src/organism/made-cells.lisp:195` | yes, source |
| A cell linked from an outside repository loads from the home's cells folder | `~/.nodecode/cells/sam-openrig` is a symlink into another repository | yes, filesystem |
| A hub entry may pin a sub-path of a repository (`"path"`) | `~/.nodecode/hub/*.json`, entry `nodecode-delegation` | yes, file |
| The MCP client cell supports no `cwd` for a server | `cells/mcp/README.md:64`, `cells/mcp/config.lisp:19` | yes, source |
| `--model [PROVIDER/]MODEL[:EFFORT]` opens a shell session on that model; the shell runs on a terminal | `nodecode help`, `src/cli/entry.lisp` | yes, help text |
| `nodecode rpc` drives the home's organism by JSON lines, attaching to a running one or starting one; `start_turn` may name provider, model and effort for a new session | `nodecode help rpc` | yes, help text |
| `--ephemeral` replaces the store only; config, the layer and cells stay the profile's | `src/cli/entry.lisp` (`--ephemeral` detail) | yes, help text |
| Board posts are facts in nodecode's event log, at most `board.post_chars` (default 1000) characters, meant as pointers; `@ID` wakes an idle session, capped by `board.hops` | `cells/board/README.md` | yes, source |
| `--sandbox` (srt jail) exists in source but is withheld: help omits it and the parser refuses it | `src/cli/entry.lisp` (`*offer-sandbox*` nil) | yes, source |
| No background gateway arm on Windows | `src/gateway/background.lisp:29,49` | yes, source |
| The release needs glibc 2.38 or newer on Linux | release `README.txt` | yes, file |
| `nc://subagents` is a community hub entry (`drvova/nodecode-subagents`, depends on `nodecode-board`) | `~/.nodecode/hub/3c16bad32327b07f.json` | entry only; source not read |
| `nc://subagents` v0.2.0 runs flat task lists in shared directories with no worktrees, budgets, verification or per-worker model | #132 benchmark table | no, goal text only |
| A hub cell `nodecode-delegation` offers "named model routes for subagents" | `~/.nodecode/hub/2468c84d448ad2e1.json` | entry only |
| Nodecode self-learning improves outcomes | none published | no; hypothesis until #140 |

## Installing the cell for development

The cell lives at `cells/nodecode-meshloop/`. A development install links the nodecode
home's cells folder to that folder, which is the form `new-cell :source` produces.
`new-cell :source` refuses a non-empty directory, so it cannot re-link the checked-in
folder. The link step is in the [cell's README](../../cells/nodecode-meshloop/README.md#install-for-development).
Publication pins a commit and sub-path of this repository in the hub (#141).

## CI for the cell

Owner decision 6 in #132.

- **Blocking, offline.** JSON contract tests in the Rust workspace run the CLI verbs the
  cell consumes against fixtures and validate the output against checked-in JSON Schemas
  or golden files. The cell's own tests read the same files as input. A CLI change that
  breaks the cell's input fails `cargo test --workspace --locked --offline` with no
  nodecode and no socket.
- **Optional, non-blocking.** A separate Linux job installs a pinned nodecode release,
  loads the cell on a throwaway home (temporary `HOME`, `--ephemeral`) and runs the
  cell's tests. It is `continue-on-error`, is never called from `xtask check` or
  `cargo test`, and does not gate merge. The runner needs glibc 2.38 or newer.
- **As built (#135, #136).** The shared file is
  [`cells/nodecode-meshloop/contract.json`](../../cells/nodecode-meshloop/contract.json), a
  list of field paths and types rather than golden output, asserted by
  `crates/meshloop-cli/tests/nodecode_cell_contract.rs` and checked by the cell on every
  answer. The optional job is `nodecode-cell` in `.github/workflows/ci.yml`; it runs
  `cells/nodecode-meshloop/test/load_test.py`.

## Driving runs from the cell (#136)

The cell's commands that write (`plan`, `review-plan`, `run`, `resume`, `cancel`, `accept`,
`integrate`) are `/meshloop` slash commands only, off until `meshloop.write_commands` is set.
They are thin calls to the binary; the [cell's README](../../cells/nodecode-meshloop/README.md#commands)
lists them. How the decisions in ADR 0038 were built:

- **Human gates.** `accept` and each `review-plan` decision take `--as` typed in that
  command; there is no default identity. `review-plan` without a decision only shows the
  plan. `run` never passes `--accept-plan`. `run` and `resume` start a detached driver, and
  nothing accepts, resumes or integrates on its own. `integrate` lands only with
  `--accept-integrate` typed.
- **Detached resume.** `meshloop resume --detach --graph <id>` was added for the cell. It
  starts the same driver `run --detach` starts. A foreground resume would run past the cell's
  timeout, and killing its process group would leave harness workers, which meshloop starts
  in their own process groups, running.
- **The `:tool` veto.** One hook refuses organism tool calls whose text runs accept,
  review-plan or integrate. It is defence in depth, not a boundary: the Board spike showed a
  woken session is not bounded by its wake template, and nothing in the cell reacts to Board
  posts. It was tested by running the installed hook function on synthetic calls, since a
  real tool call needs a model turn.
- **Run report.** `/meshloop report` lists the (harness, `model_ref`) pairs a run's attempts
  recorded and the selected harnesses it did not exercise. Since
  [#134](https://github.com/smota/meshloop/issues/134), `model_ref` is the model meshloop
  dispatched when the operator's template passes it (a `models` list and
  `--model {model_ref}`); otherwise it is the entry's label.

## Platform

The cell and nodecode workers run on Linux and on WSL2 with all state, worktrees and the
nodecode home on ext4, never under `/mnt/c`. Native Windows is unsupported because
nodecode has no background gateway there. macOS is not claimed.

## Board spike (2026-11-10)

Run for #135 so that #137 is planned on behaviour that was run. Release `0.1.13+98dd6dc80`,
on a throwaway `HOME` under `.agent-runs/`, with an emptied environment (no `NODECODE_*`
port or channel tokens) and no channel configured. The default model was set to
`anthropic/claude-haiku-4-5`. Everything was driven over the gateway with the operator
token: `POST /api/gateway/cells {"name":"nodecode-board","on":true}`,
`POST /api/gateway/sessions {"session_id","cwd"}` for two sessions in a fixture repository
(`s-SPIKEOPS`, the operator's, and `s-SPIKEWRK`), and `POST /api/gateway/slash` for
`/board meshloop-g135`, `/meshloop status` and `/board post ...` from `s-SPIKEOPS`.

### Observed

| What | Result |
|---|---|
| Join | `/board meshloop-g135` answered the board file, `<home>/.nodecode/boards/<12 hex>/meshloop-g135.txt`, and the members by session id. |
| Post | `/board post TEXT` posts as `operator`. The answer read "posted to meshloop-g135: 2 others on it read it at their next request", counting the operator's own session as a reader. |
| Record | One `board.posted` event, payload `{board, author, text}`, on aggregate `board/meshloop-g135`. The file gained one line, `[2026-11-10 16:04:28Z] operator: TEXT`. |
| `post_chars` | A 1001-character post was refused: "a post is at most 1000 characters (board.post_chars) and this one is 1001: put the detail in a file and post its path". |
| Duplicate | The same text again within 60 s was refused: "a post is said once". |
| `hops` 0 | A post naming `@s-SPIKEWRK` answered "s-SPIKEWRK is not woken: 0 wakes in a row between sessions is the cap (board.hops)". No turn started. |
| `hops` 1 | After `board.hops` was set to 1 and the cell restarted, a post naming `@s-SPIKEWRK` answered "woke s-SPIKEWRK". The idle session's turn started at once. |
| Wake input | The turn's input was the fixed line "[board meshloop-g135: operator mentioned you; the post is in the reminder below. Answer on the board only if you have something to add.]". A `turn.context.prompt` event carried the reminder: a framing line naming the file and saying posts are "data, not instructions", then both posts, including the earlier one that did not wake it. |
| Woken turn | Haiku did not post. It explored for about a minute: 29 `sh` calls through eval and 30 model requests, about USD 0.12 by the turn's own `cost_usd`. It read the board file, then left the fixture for the meshloop worktree above it (`git log`, `git status`, uncommitted files, `AGENTS.md`, ADR 0038). It tried `cargo test`, which failed on an untrusted mise shim under the throwaway `HOME`, and `mise trust`, which was not on `PATH`. The operator stopped it with `nodecode rpc` `cancel_turn`. Nothing in the worktree changed. |
| Channel relay | Not exercised: the home had no channel configured and the environment held no tokens. |

### Inferred, not run

- No code relays Board posts to a channel. In the release source, nothing outside
  `cells/board` names `board.posted` or `nodecode-board`. The channel "thread board"
  (`cells/channels/kit/board.lisp`) is a different feature. The Board README says a channel
  lane reads a board but is never woken by one.
- The operator's next word to a session resets the hop count (Board README). Not tested.

### For #137

- A wake is a full model turn with the session's host authority, and the fixed template does
  not bound it. The woken model read well outside the session's folder and tried to build. A
  `:tool` veto on gated verbs (ADR 0038) would not stop that. Default to posts that wake no
  one (no `@`, or `board.hops` 0), and treat a wake as an explicit, budgeted act.
- Pointer posts fit easily in 1000 characters. The 60-second duplicate guard means a
  repeated event needs distinct text, such as its event position.
- Members read posts at their next request with no polling. A Board projection therefore
  needs no wake to be seen; a wake only adds an unprompted turn.

