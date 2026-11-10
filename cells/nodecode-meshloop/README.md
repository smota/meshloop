# nodecode-meshloop

A [nodecode](https://nodecode.ai) cell that shows meshloop runs and, when the operator
switches it on, drives them: plan, review-plan, run, resume, cancel, accept and integrate. It
is the first step of the learning plane in
[ADR 0038](../../docs/architecture/adr/0038-nodecode-learning-plane-and-harness.md)
([#135](https://github.com/smota/meshloop/issues/135) read-only,
[#136](https://github.com/smota/meshloop/issues/136) driving,
[#137](https://github.com/smota/meshloop/issues/137) an optional one-way projection onto a
nodecode Board). Background and nodecode facts
are in [learning-plane.md](../../docs/architecture/learning-plane.md).

The cell runs the `meshloop` binary as `meshloop <verb> --json --origin-harness nodecode
--origin-session <session id>`. It passes an argument list, never a shell string. The working
directory is the session's repository root, the nearest folder above the session's folder
that holds `.git`. The cell decodes the JSON envelope and answers a short summary. It never
opens meshloop's SQLite store. Every write goes through a meshloop verb, so the meshloop event
log remains the only source of truth and the human gates stay meshloop's. Every meshloop verb
works the same with the cell off.

Platform: Linux, and WSL2 with the repository and the nodecode home on ext4. Tested against
nodecode `0.1.13+98dd6dc80`. Nodecode is alpha, so a later release can break the cell.

## Install for development

1. Build meshloop from this checkout. An installed `meshloop` may be older than this cell
   (see [agent-workflow.md](../../docs/engineering/agent-workflow.md#execution-with-meshloop)):

   ```bash
   cargo build --locked -p meshloop-cli -p meshloop-adapters --bins
   ```

2. Link this folder into the nodecode home's cells folder. The folder name must stay
   `nodecode-meshloop`: nodecode requires the folder name to match the system and package name.
   `(new-cell NAME :source DIR)` refuses a non-empty directory, so it cannot link a
   checked-in folder. Make the link yourself:

   ```bash
   home=~/.nodecode            # or a profile's home: ~/.nodecode/profiles/<name>
   ln -s "$PWD/cells/nodecode-meshloop" "$home/cells/nodecode-meshloop"
   ```

3. Point the cell at the binary in `$home/config.jsonc`:

   ```jsonc
   { "meshloop": { "binary": "/absolute/path/to/meshloop/target/debug/meshloop" } }
   ```

   The commands that write stay off until you add `"write_commands": true` to that section.

4. Restart the organism. `/meshloop` then appears in the shell. A running organism can
   instead restart this one cell on the current config through its gateway:
   `POST /api/gateway/cells` with `{"name": "nodecode-meshloop"}`, as the load test does.

To switch the cell off, set `"enabled": false` in the `meshloop` section and restart the
cell, or remove the link. Its commands disappear, and meshloop itself is unaffected.

## Commands

Read-only, in the shell and through eval:

| In the shell | Through eval | What it answers |
|---|---|---|
| `/meshloop doctor` | `(meshloop:doctor)` | Readiness, dispatchable harnesses, whether the store is git-ignored, warnings |
| `/meshloop status [GRAPH]` | `(meshloop:status "GRAPH")` | The newest run, or GRAPH: plan state, node counts, attempts, one line per node |
| `/meshloop inspect [GRAPH]` | `(meshloop:inspect "GRAPH")` | The same run with its overall status |
| `/meshloop report [GRAPH]` | `(meshloop:report "GRAPH")` | The run report: harnesses and `model_ref`s dispatched, selected harnesses not exercised, one line per node |
| `/meshloop review-plan [--plan FILE]` | none | The plan file's nodes and where its review stands |
| `/meshloop hook-check FORM` | none | Whether the `:tool` veto refuses an eval call whose form is FORM |
| `/meshloop board-sync [GRAPH]` | none | Puts this session on the repository's nodecode board and posts the runs' new events there, when `board` is on ([The Board projection](#the-board-projection)). It writes nothing to meshloop |

Commands that write. These are slash commands only: they are not eval verbs or tools, and no
hook, timer or Board post calls them. Each refuses in one line while `write_commands` is off,
which is the default.

| In the shell | Runs |
|---|---|
| `/meshloop plan [--out FILE] OBJECTIVE` | `meshloop plan --objective OBJECTIVE --out FILE` (default `meshloop-plan.json`). Starts nothing |
| `/meshloop review-plan [--plan FILE] --accept\|--decline\|--adjust --as <your-name> [--reason TEXT]` | `meshloop review-plan` with that decision and `--as <your-name>`. `--adjust` needs `--reason` |
| `/meshloop run [--plan FILE]` | `meshloop run --plan FILE --detach`, then points to `inspect`. Never `--accept-plan` |
| `/meshloop resume [GRAPH] [--retry\|--restart]` | `meshloop resume --detach --graph GRAPH` (newest run without GRAPH). Refuses while the graph's driver is alive |
| `/meshloop cancel [GRAPH] [--task N]` | `meshloop cancel --graph GRAPH`, then reports live attempts, the driver and the worktrees meshloop keeps |
| `/meshloop accept --task N --as <your-name> [--graph GRAPH]` | `meshloop accept --task N --as <your-name> --graph GRAPH`, then reads the acceptance back from `inspect --attempts`. Without `--graph`: the one run waiting for acceptance, a refusal when several are |
| `/meshloop integrate --graph GRAPH --into BRANCH [--accept-integrate]` | Without `--accept-integrate`: names the integrate worktree to review and lands nothing. With it: `meshloop integrate --graph GRAPH --into BRANCH --accept-integrate` |

The human gates are unchanged:

- **Identity.** `accept` and every `review-plan` decision need `--as IDENTITY` typed in that
  command, and pass it to meshloop as `--as`. There is no default identity, no setting that
  supplies one, and the session id is never used. Without `--as` the command refuses, and so
  does a placeholder (`you`, `me`, `identity`, `name`, `<your-name>`, in any case): type your
  own name. The identity is a claim meshloop records; nothing authenticates it (ADR 0038, Consequences).
- **Plan review.** `review-plan` without a decision only shows the plan. The decision is a
  second command the person types. `run` never passes `--accept-plan`, so an unreviewed plan
  is refused by meshloop.
- **Nothing follows on its own.** `run` and `resume` start a detached driver and return. Each
  node then waits for `accept`; nothing accepts, resumes or integrates automatically. While
  a run's driver is alive it merges accepted nodes itself, so `resume` refuses then, and
  meshloop refuses a second `run --detach` or `resume --detach` for that graph too.
- **Integrate is explicit.** `--accept-integrate` must be typed after the integrate worktree
  has been pointed out.

`doctor` is `meshloop doctor`: it runs the `executable` of each selected harness in the
repository's `meshloop.toml` with that harness's `version_args`, with the same authority as
running `meshloop doctor` in a shell. `plan`, `run` and `resume` start the harnesses that
config names. Run the cell only in repositories whose `meshloop.toml` you trust.

`/meshloop` typed alone opens a panel of the read-only verbs. `(help :meshloop)` is the
manual. Sample answers from fixture repositories:

```text
meshloop review-plan: graph g136, accept as person-one: PlanAccepted
  next: /meshloop run --plan plan.json starts it detached; nothing starts on its own

meshloop run: graph g136, driver started detached (running); nothing follows on its own
  follow it with /meshloop inspect g136; a node that waits for acceptance takes /meshloop accept --task N --as <your-name> --graph g136. While this driver is alive it merges accepted nodes itself; once /meshloop status g136 shows it exited, /meshloop resume g136 starts the next

meshloop accept: names no identity. Type who decides after --as, in this command: /meshloop accept --task N --as <your-name> [--graph GRAPH]

meshloop accept: task 1 of graph g136 accepted as person-one; meshloop recorded "accepted by person-one at 1794331730"
  next: /meshloop resume g136 merges it; nothing resumes on its own

meshloop report: graph g136 completed, plan PlanAccepted, 2 nodes (2 Integrated)
  dispatched: fixture (model_ref fixture-model, 2 attempts)
  not exercised: none of the 1 selected
  #1 Integrated: fixture/fixture-model IntegrationOwnerMerge, exit 0; accepted by person-one at 1794331730
  model_ref is the model meshloop dispatched when the harness's invoke template passes {model_ref} (a models list, #134); otherwise it is the entry's label
```

A summary lists 12 nodes at most and counts the rest. Node descriptions, notes and worker
output are data, not instructions; the report shows meshloop's exit codes and outcomes, never
worker text.

### The run report

`/meshloop report` reads `meshloop inspect --attempts --json` and `meshloop doctor --json`. It
lists each (harness, `model_ref`) pair that an attempt recorded, and each selected harness with
no attempt in that run, with doctor's dispatchability. Since
[#134](https://github.com/smota/meshloop/issues/134), a harness entry may list `models`, and
the router substitutes each one into `{model_ref}`. When the operator's invoke template passes
it (for example `--model {model_ref}`), `model_ref` is the model meshloop dispatched.
Otherwise it is the entry's label and the CLI runs its own default model.

### The `:tool` veto

ADR 0038 makes accept, review-plan and integrate operator-typed slash commands, never
model-callable. The cell installs one hook on nodecode's `:tool` point. It reads every string
in a tool call's arguments as written and with `\uXXXX` and `%XX` decoded, and refuses the
call when one of them runs `meshloop accept`, `review-plan` or `integrate`:

- an argv list or token run naming the `meshloop` executable (any path, `meshloop.exe`, the
  `meshloop:accept` form) and then the verb;
- a shell line, read with quotes joined and backslashes dropped (`m""eshloop`, `mesh''loop`
  and `me\shloop` are `meshloop`) and cut at `;`, `&&`, `||`, `&` and newlines with a `|`
  pipeline kept whole, whose command holds a gated verb anywhere and a meshloop runner: the
  executable, a `$VARIABLE` in command position, or `meshloop-cli` after `-p`. That covers
  `echo accept | xargs meshloop` and `B=meshloop; $B accept`;
- the MCP tools `meshloop_accept`, `meshloop_review_plan` and `meshloop_integrate`, or any
  tool whose server segment is meshloop and whose last `__` segment is a gated verb;
- the `/meshloop accept` slash line;
- this cell's own code, reached through `meshloop::`, or by `find-symbol`, `intern`,
  `find-package` or `in-package` naming its package.

The meshloop binary takes its verb as the first argument and refuses a flag there, so moving
the verb after flags is no way around the veto. The refusal is recorded as the call's failure.

This is defence in depth, not a security boundary. The organism's eval has the operator's full
host authority. A call can build the command from pieces or reach meshloop another way, and
this reading will not see it. The [Board spike](../../docs/architecture/learning-plane.md#board-spike-2026-11-10)
showed that a woken session is not bounded by its wake template, which is why nothing in this
cell reacts to Board posts. `/meshloop hook-check FORM` runs the installed hook function on a
synthetic eval call with FORM as its form, without running FORM, and says whether the hook is
on the organism's `:tool` chain.

### Live run (#136, 2026-11-10)

A three-node graph (each node adds one one-line file; 2 depends on 1, 3 on 2) was driven end
to end from the cell. A nodecode organism ran on a throwaway `HOME` under `.agent-runs/`,
with links to the harnesses' credential folders and node's directory on `PATH` (pi is a node
script). Every step was a `/meshloop` slash command through the gateway, with no model turn
in nodecode. The plan file was hand-written, and `plan` was not run against a live planner.
Plan review and every node acceptance were typed as `--as claude-coordinator`: the
coordinator (Claude), acting under authority the repository owner delegated for goal #132,
not a person. Each node reached `AwaitingReview` with meshloop's checks passed before it
was accepted, and the diffs were checked afterwards: one added file each. The disposable
repository's config added `--permission-mode acceptEdits` to claude, an operator change
(#163).

| Node | Harness | model_ref | Harness exit | Checks | Accepted by | Integrated |
|---|---|---|---|---|---|---|
| 1 (Tier3) | codex (`exec -`) | `codex` | 0 | git-diff 1 file; verify skipped (empty) | claude-coordinator | yes |
| 2 (Tier1) | pi (`-p {prompt}`) | `pi` | 0 | git-diff 1 file; verify skipped (empty) | claude-coordinator | yes |
| 3 (Tier3) | codex | `codex` | 0 | git-diff 1 file; verify skipped (empty) | claude-coordinator | yes |

`integrate --into master --accept-integrate` landed the graph and #142's auto-reclaim
removed every worktree and `meshloop/g136-live/` branch, so `workspace inventory` listed
none. The `model_ref` values are labels: these templates pass no `{model_ref}`, so each CLI
ran its own default model and the models actually run are not recorded.

Not exercised: claude was selected and dispatchable, but the router chose codex for both
Tier3 nodes, so it was not dispatched. agy is not dispatchable because its invoke template is
empty (#164). grok was not selected because its quota is exhausted (#162). Two earlier runs
are recorded too. In the first, pi was not dispatchable because node was not on the
organism's `PATH`, and codex ran all three nodes. In the second, pi answered exit 0 with no
diff. It read the node's "do not create any other file" wording as conflicting with the
task, so the node failed. `/meshloop cancel` then left no process and kept only ledger-owned
worktrees.

### The Board projection

Off by default. With `board` on, the cell turns meshloop's event log into pointer posts on
a [nodecode Board](../../docs/architecture/learning-plane.md#board-spike-2026-11-10).
Sessions on that board read the posts at their next model request, so a supervising
session does not have to poll `status`. The projection is one-way. The meshloop event log
is still the only authority. No Board post changes meshloop state, and nothing in this cell
reads a post.

- **One board per repository.** The board is named `meshloop-` plus 12 hex digits of the
  SHA-256 of the repository root's real path, for example `meshloop-b9a29b1a6412`.
  Nodecode lets a session sit on one board, so this name is the same for every run in
  the repository. Every post names its run (graph) and is posted by the session that
  typed the command (`board:post`).
- **Joining is explicit.** Only `/meshloop board-sync` puts the session that typed it on the
  repository's board (`board:join`), and its answer says "this session is now on
  meshloop-X; it left any board it was on". The cell records that session id in its cursor
  file. A sync after another command posts only for a session recorded there that is still
  on that board. The cell reads which board the session is on from the Board cell's
  join and leave facts, never from posts. Any other session posts nothing and gets one
  line: "run /meshloop board-sync to put this session on meshloop-X (it leaves its
  current board)". A supervising session joins the board itself, with
  `/board meshloop-...`. `/meshloop board-sync` prints the name.
- **The source.** `meshloop events --graph G --after N --json` is a read-only verb (ADR
  0034) added for this projection. It returns the graph's events in log order, each with
  `seq`, the event log's own id. The id is unique and increasing and is never renumbered.
  Each event carries the task, attempt, states, event name and the attempt's worktree. It
  never carries a reason, a note or worker output. `watch` was not enough: it polls
  status and sees only the transitions between two polls. The verb folds the rows status
  folds, the way status does (`replay_tasks`). For an unbound graph those are its visible
  rows. For a bound graph they are only the active generation's. It returns each node's
  folded state (`nodes`) and its newest counted row (`latest`). Every other row is marked
  `superseded`: an earlier generation's row after a restart, or, on a ledger written
  before #167, a cancelled attempt's stale check result after its `Cancel`. The
  cell never posts a superseded row and moves its cursor past it, so the board never
  shows a state that status does not, and never wakes anyone for one. `--limit` (default
  1000; the cell passes 200) pages the rows in SQL, and `more` says that rows remain.
  `max_seq`, the ledger's newest event id, and `ledger_id` let the cell notice a replaced
  ledger.
- **The post.** Each post is one line from a fixed template made of meshloop's own words:

  ```text
  meshloop g137 event 4: node #1 AwaitingReview (from Verifying, DeterministicChecksPassed); evidence .meshloop/worktrees/g137/task-1-attempt-1
  meshloop g137 plan 1: PlanAccepted; evidence none
  ```

  The fields are the graph id, the event's `seq`, the node, its new state, the state it
  left, the event name, and the evidence path: the attempt's worktree relative to the
  repository root, or `outside-repo/` and its last two components for a worktree outside
  it (`\` separates as `/` does), or `none`. A plan line is posted when the run's plan state changes,
  numbered by plan change. No description, note, reason, diff, log or worker output goes
  in a post. States and event names must be ASCII letters and digits or they become `?`.
  An `@` in a path is written `%40`, so a post can never mention a session by accident.
  A post fits the board's `board.post_chars` (1000 by default). The evidence path is
  shortened from the left first, then the rest of the line is clipped. The mention is
  never cut: it is the last word, whole, or absent when `post_chars` cannot hold it.
- **Once, and never a second wake.** A cursor holds, for each graph, the `seq` of the last
  posted event, the last posted plan state, the ledger it was kept on, and the post in
  flight. It is a file of this cell's own under the nodecode home,
  `meshloop/board/<store>/<board>.cursor`, next to the joined sessions, and never in
  meshloop's store. Before each post the cell writes a `pending` marker. After the post it
  moves the cursor and clears the marker, each write one rename. A second sync, a
  `resume`, a restarted cell or a sync after an interrupted run therefore posts nothing
  twice. The `seq` in the text keeps posts distinct for the Board's 60-second same-words
  refusal, and that refusal is read as "already posted". A crash between a post and its
  cursor write leaves the marker. The next sync posts that one event again *without* its
  mention: delivery is at least once, and a crash never wakes the supervisor twice. If the
  cursor was kept on another ledger (another `ledger_id`, or a `seq` past the ledger's
  `max_seq`, as after `.meshloop/` was deleted and runs started again), that graph starts
  over and the answer says "cursor reset". If the cursor file is not one this cell wrote,
  the cell refuses to post. It does not start over silently.
- **When it runs.** Only inside a `/meshloop` command the operator typed. `/meshloop
  board-sync [GRAPH]` runs it. While `board_after_commands` is on, which is the default
  when `board` is on, `run`, `resume`, `accept`, `cancel` and `status` also sync afterwards
  and add a `meshloop board:` line to their answer. The graphs synced are the graph the
  command named, the newest run and every run waiting for acceptance. An id that is not a
  graph id is dropped with a note, never posted and never written to the cursor. There
  is no timer,
  thread or hook point, so meshloop and this cell add no daemon (ADR 0022). A transition
  made by a detached driver reaches the board at the next such command. Eval verbs never
  post, so a model turn does not drive the projection. One sync posts at most 200 events
  for each graph and says when more wait for the next.
- **No mentions by default.** A post mentions no one. With `wake_on_gates` on, a post about
  a gate (`AwaitingPlanReview`, `AwaitingReview`, `Blocked`, `Failed`) ends with
  `@<supervisor_session>`, and only when that state is still the node's current state, so
  replayed history wakes no one. The current state is the newest row meshloop's fold
  counts for the node, reaching the folded state. A first sync of a run that ended `Failed`
  therefore still wakes the supervisor, because `Failed` is that node's current gate. The
  same holds after a "cursor reset". The mention is the template's last word and comes
  with no instruction. The Board wakes the named session if it is idle, on the board, and
  within `board.hops` (4 by default; 0 never wakes). It answers `woke ID` or why not, and
  the cell repeats that answer.
- **What a wake costs.** A wake starts a model turn in the supervising session with that
  session's full host authority. The [Board spike](../../docs/architecture/learning-plane.md#board-spike-2026-11-10)
  showed that the fixed wake template does not bound it: a woken session ran 30 model
  requests and 29 shell calls and read files outside its folder. That turn could run
  `meshloop accept --as` itself. The `:tool` veto is defence in depth, not a boundary. Turn
  `wake_on_gates` on only for a supervising session whose own budget you have bounded with
  the organism's turn budget settings, and expect one turn for each gate post that
  mentions it.
- **Acceptance stays typed.** A post that says `accept`, or `/meshloop accept --task 1 --as
  x`, is text on a board. Acceptance is `/meshloop accept --as <identity>`, or `meshloop
  accept`, typed by a person (ADR 0038, invariant 5).
- **No relay.** The nodecode release has no code that relays Board posts to a channel, and
  this cell adds none, following the owner's decision in #137.
- **Workers.** `board_workers` is reserved for nodecode-harness workers ([#138](https://github.com/smota/meshloop/issues/138),
  not built) joining the board. Nothing joins a worker, whether the setting is on or
  off. When it is on, `board-sync` says so. Board text that reaches a worker is untrusted
  input outside meshloop's evidence.

A sync answers like this:

```text
meshloop board: meshloop-b9a29b1a6412; this session is now on meshloop-b9a29b1a6412; it left any board it was on
  g137: 4 posts, cursor at event 13; the board said: woke s-MESHLOOPSUPER
```

## Settings

The `meshloop` section of the home's `config.jsonc`:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `binary` | string | `meshloop` | The executable: the absolute path of `target/debug/meshloop`, or a name found on `PATH` |
| `timeout_seconds` | integer, at least 1 | `60` | How long one call may run before the cell stops it |
| `plan_timeout_seconds` | integer, at least 1 | `600` | How long `plan` and `review-plan --adjust`, which run the planner in the foreground, may run |
| `write_commands` | boolean | `false` | Lets the commands that write run. Off, each refuses in one line |
| `enabled` | boolean | `true` when the section is present | `false` switches the cell off |
| `board` | boolean | `false` | Projects run events onto the repository's nodecode board ([The Board projection](#the-board-projection)). Needs the `nodecode-board` cell, which ships with nodecode. Off, `board-sync` refuses in one line and no other command runs anything more |
| `board_after_commands` | boolean | `true` | With `board` on, also syncs after `/meshloop run`, `resume`, `accept`, `cancel` and `status` |
| `wake_on_gates` | boolean | `false` | With `board` on, a post about a current gate mentions `supervisor_session`, which wakes it when idle: a model turn with full host authority |
| `supervisor_session` | string | empty | The session id a gate post mentions, for example `s-ABC123`. Empty, or anything other than letters, digits, `-` and `_`, mentions no one |
| `board_workers` | boolean | `false` | Reserved for nodecode-harness workers joining the board (#138, not built). No worker joins either way |

## Failures

Every failure is one line that starts with `meshloop <verb>:`, never a Lisp condition:

- The binary is missing or cannot start: the line names the binary and how to build it.
- The binary exits with no JSON, for example an older binary that rejects a flag: the line
  gives the exit code and the first line of stderr.
- The JSON lacks a field the cell reads, or carries it with another type: the line names the
  field and says the binary may be older than the cell.
- meshloop answers `ok: false`, for example an unknown graph: the line carries meshloop's
  `error`.
- A call runs past `timeout_seconds`: the cell kills the binary's whole process group
  (doctor's harness probes included), or answers more than 8 MiB. A detached driver is in its
  own process group and is not affected. meshloop starts harness workers in their own process
  groups, so stopping `plan` or `review-plan --adjust` at `plan_timeout_seconds` can leave the
  planner harness running until it exits.
- A write command whose verb answers no JSON (accept, integrate, or plan when it fails) and
  exits non-zero: the line gives the exit code and meshloop's last line of output.

## The CLI contract

[`contract.json`](contract.json) is the single list of every `--json` field the cell reads,
with its type, for each case: the envelope (`ok`, `command`), `error` when `ok` is false,
`doctor`, `status` with and without a run, `inspect`, `inspect --attempts` (the report and
accept's read-back), the integrate worktree, `plan`, `review-plan`, `run --detach`,
`resume --detach`, `cancel`, `workspace inventory` and `events` (the Board projection), and
the fields of a plan file. `accept`
and `integrate` answer text, so the cell reads their exit code and then what they recorded. The cell loads it at start and
checks every answer against it. That check produces the "older binary" line.
[`crates/meshloop-cli/tests/nodecode_cell_contract.rs`](../../crates/meshloop-cli/tests/nodecode_cell_contract.rs)
runs the same verbs, with the cell's arguments, on a fixture repository and asserts the same
list. To read a new field, add it to `contract.json` first.

## Tests

- **Blocking, offline:** the Rust contract test above runs in
  `cargo test --workspace --locked --offline`, with no nodecode and no socket.
- **Load test:** [`test/load_test.py`](test/load_test.py) starts `nodecode serve` on a
  throwaway `HOME` with an emptied environment, links this folder in and opens one session in
  a fixture repository. It then runs the commands through the gateway's
  `POST /api/gateway/slash`, which runs a cell command with no model turn. It checks the
  summaries and the failure lines (missing binary, older binary, no JSON, a timeout whose
  grandchild holds the pipe, 300 KB of stderr). It also checks that the read-only commands
  leave the repository's git status, refs, worktrees and `.meshloop/` files (bytes and mtimes)
  unchanged, and that switching the cell off leaves `meshloop status --json` as it was. For
  the commands that write it checks, on fixture repositories of their own: each one refuses
  while `write_commands` is off; `accept` and `review-plan` refuse without `--as` and record
  the typed identity (read back from meshloop's event log); the `:tool` veto refuses accept,
  review-plan and integrate in several spellings and lets read verbs through (through
  `hook-check`, because a real tool call needs a model turn); a fixture graph goes plan,
  review-plan, run, accept, resume, integrate, with auto-reclaim leaving no worktree or
  branch; and cancel leaves no process in the repository and only worktrees meshloop owns.
  For the Board projection (#137), on a fixture repository of its own with the
  `nodecode-board` cell on and a supervising session on the board, it checks the
  following. With `board` off, `board-sync` refuses and `status` posts nothing. With it
  on, `status` from a session `board-sync` never joined posts nothing and says how to
  join, even from a session already on the board. Only `board-sync` joins, and it says
  so. The board's name matches one computed independently from the repository path. The
  board file holds exactly one post per event `meshloop events` lists, in order,
  and one post for the plan state. No post holds `@`, the run's text, or more than
  `post_chars`. A second sync posts nothing. A cursor rewound to before its last post posts
  no duplicate, because the Board refuses the same words. The cursor file records the
  joined session. A joined session moved to another board posts nothing there until
  `board-sync` brings it back. Three posts saying `accept`,
  including `/meshloop accept --task 1 --as x`, leave the event log, the meshloop store and
  the node unchanged and start no process. With `wake_on_gates` on, it kills the run's
  driver and worker during an attempt and resumes, with a retry. It then checks that every
  event of the whole run was posted exactly once and that only gate posts mention the
  supervising session. It checks that the Board woke that session once for each such
  post, read from its timeline. A post left in flight (`pending`) is posted again without
  its mention and wakes no one. A restarted cell posts nothing again, and a cursor kept on
  another ledger id resets and says so. The
  throwaway home has no model, so a woken turn ends at once ("no model is chosen yet")
  with no model call: the wake is delivered and nothing is spent. It never touches the
  operator's nodecode home. `--work` must sit inside the repository's
  `.agent-runs/`, the temp directory or `$RUNNER_TEMP`; it is removed when every check
  passes.

  ```bash
  python3 -I cells/nodecode-meshloop/test/load_test.py \
    --meshloop "$PWD/target/debug/meshloop" \
    --fixture-harness "$PWD/target/debug/fixture_harness" \
    --nodecode "$(command -v nodecode)" \
    --work "$PWD/.agent-runs/nodecode-cell"
  ```

  This test found a meshloop defect, fixed in #167: cancelling a running attempt raced the
  run's detached driver and could leave an illegal `Cancelled + DeterministicChecksFailed`
  sequence that made `meshloop status` refuse the graph. Event appends are now checked
  against the persisted state, and the cancel check passes reliably.

- **Offline Lisp check (optional):** [`test/pure_test.lisp`](test/pure_test.lisp) runs under
  a plain SBCL with no nodecode: `sbcl --script cells/nodecode-meshloop/test/pure_test.lisp`.
  It reads every form of the cell's files, compiles each function against stand-in nodecode
  packages and fails on a full compiler warning, then tests the functions that need nothing
  of nodecode: the `:tool` veto's reading of a call (every spelling above, the pass-throughs,
  and a command assembled at run time, which it cannot see) and the argument checks of the commands that write (identity, task, graph,
  branch, plan path, flags). For the Board projection it tests the board name (its form,
  and that it is the same for a path with or without a trailing `/`), the post template
  (exact text, `none` evidence, the mention as the last word, distinct text per event, no
  `@` or free text from any field, every post within `post_chars` down to 40, a long path
  shortened from the left), when a post mentions (`wake_on_gates` off: never; on: only a
  current gate, only a well-formed supervisor id), the cursor file (round trip, refusal of a
  foreign or broken file, format 1, sessions, ledger, pending), when a cursor counts as kept
  on another ledger, when an event is current (newest counted row reaching the folded
  state, so not a pre-#167 stale row), paths outside the repository and with `\`, the
  mention never cut, a plan post in flight naming its plan state (`pN:STATE`, so a later
  plan change is never taken for it), and which graphs a command names. When the pinned
  nodecode release is installed, it also reads that release's board source for the
  functions this cell calls by name (`member-board`, `join`, `post`, `post_chars`). It also reads the sources to check
  that the cell is one-way: no source names the Board's post facts, its `:round` delivery
  or its post readers, the only Board verbs called are `JOIN` and `POST`, and the
  projection asks meshloop only for `events` and `status`. Neither `cargo test` nor
  `xtask check` runs it.

- **Optional CI:** the `nodecode-cell` job in `.github/workflows/ci.yml` installs a pinned
  nodecode release on a throwaway `HOME` and runs the load test. It checks `install.sh`
  and the installed executable against
  [`test/nodecode-98dd6dc80.sha256`](test/nodecode-98dd6dc80.sha256) before running
  either. It is `continue-on-error`,
  and neither `xtask` nor `cargo test` calls it.
