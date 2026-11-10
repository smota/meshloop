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
folder. The link step is documented in the cell's README by #135. Publication pins a
commit and sub-path of this repository in the hub (#141).

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

## Platform

The cell and nodecode workers run on Linux and on WSL2 with all state, worktrees and the
nodecode home on ext4, never under `/mnt/c`. Native Windows is unsupported because
nodecode has no background gateway there. macOS is not claimed.
