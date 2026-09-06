# AI engineering workflow

1. Frame scope, acceptance criteria, exclusions, risk, effort, and task ownership.
2. Read relevant architecture; propose an ADR for consequential boundary changes.
3. Obtain acceptance of proposed consequential decisions before dependent implementation.
4. Implement through AI with permitted paths and clear integration ownership.
5. Run deterministic validation and appropriate review against the exact candidate.
6. Update durable architecture evidence and report actual outcomes within authorization.

Separate AI review challenges assumptions; it cannot substitute for required human
acceptance or deterministic validation. Bounded work permits explicitly identified
self-review. High-risk security, unsafe code, destructive data, and releases require
human acceptance of a reviewable result. Do not create external comments without authority.

Delegation is optional and requires authorization. A handoff records task ID, executor,
base revision, owned paths, decisions, changed files, checks, limitations, and next owner.
Use separate worktrees for concurrent writers. Serialize shared-file writes and integration.
Keep local notes in ignored .agent-runs/. Product runtime state is a separate future decision.

ADRs use the lifecycle in ../architecture/adr/README.md. A task can be complete without
publication; local tests, CI, release, and deployed behavior are separate observations.
