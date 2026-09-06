# Initial threat model

Trust boundaries: user intent, target repository, generated plans, worker processes,
Herdr transport, Git worktrees, verification tools, local storage, and human approvals.

Review prompt injection through repository files/logs; shell argument injection;
path traversal and symlink escapes; unauthorized writes; orphaned workers; stale evidence;
test tampering; secrets in logs; poisoned routing feedback; and partial integration.

Planned controls include typed validated contracts, structured command arguments,
resolved path containment, explicit permissions, owned process trees, isolated worktrees,
candidate hashes/revisions, independent review, redaction, and bounded recovery.
These are requirements for future implementation, not claims that this scaffold enforces them.

Control ownership by component, so these are implementable rather than only named:
structured command arguments and path containment belong to meshloop-adapters (`herdr`,
`git`, `harness` modules; ADR 0005); typed validated contracts and candidate-revision
binding belong to meshloop-domain/meshloop-engine (ADR 0007); owned process trees and
orphan prevention belong to the harness/herdr adapters plus the recovery use case (ADR
0005); redaction and poisoned-feedback resistance belong to the `store` adapter (ADR 0007)
and the router's authorization-preserving feedback rule (ADR 0009). Independent review and
human approval remain outside any single component, enforced at the
`awaiting-review -> accepted` transition (execution-lifecycle.md).
