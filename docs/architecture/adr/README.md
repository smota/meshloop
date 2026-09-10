# Architecture decision records

Decision lifecycle: Draft -> Proposed -> Accepted; Proposed may become Rejected or
Withdrawn. Accepted may become Superseded (link the accepted replacement) or Deprecated
(document retirement implications). Preserve rejected and withdrawn reasoning.

Implementation status is independent: not-started -> in-progress -> implemented -> verified.
Acceptance records a human decision against concrete content, not proof of implementation.
Verification requires evidence. Substantive changes to accepted decisions need a successor;
editorial corrections may preserve meaning. IDs are stable and never reused.

Each record includes context, alternatives, decision/proposal, consequences, validation,
authors/reviewers, approval evidence, implementation links/status, and supersession links.
Use template.md. Architecture overview documents and runtime-design.md describe the
current baseline; ADRs explain why and stay short — mechanism detail belongs in
runtime-design.md, not in an ADR body. Update implementation status with evidence rather
than rewriting history.

A 2026-09-05 cleanup pass consolidated related decisions to keep this index short: five
proposed ADRs now each cover what used to be two or three. Consolidation is not deletion —
every prior ID is preserved below with a pointer, per "IDs are stable and never reused."

## Active

| ID | Topic | Decision status |
|---|---|---|
| 0001 | Product scope, platforms, and operating surface | Accepted |
| 0002 | Workspace boundaries | Accepted |
| 0003 | Harness capability and agent dispatch contract | Accepted |
| 0005 | Execution, transport, isolation, and recovery | Accepted |
| 0007 | Verification and persistence | Accepted |
| 0009 | Planning and routing | Accepted |
| 0011 | Apache-2.0 licensing | Accepted |
| 0012 | ADR and AI engineering governance | Accepted |
| 0016 | Release 1: closed-loop orchestration with live Herdr workers | Accepted |
| 0017 | Session control plane, Herdr live transport, meshloop: namespace | Accepted |
| 0018 | Publishing: crates.io source-install and version-locked session pack | Accepted |
| 0019 | Origin cockpit and Meshloop-owned Herdr space | Accepted |
| 0020 | Live Herdr pane is the wait authority | Accepted |
| 0021 | Restart an accepted plan without replanning | Accepted |

## Withdrawn (consolidated into an active ADR above)

| ID | Topic | Superseded by |
|---|---|---|
| 0004 | Herdr transport | 0005 |
| 0006 | Isolation and integration | 0005 |
| 0008 | Persistence and privacy | 0007 |
| 0010 | Toolchain and distribution | 0001 |
| 0013 | Agent dispatch and prompt contract | 0003 |
| 0014 | Planning and objective-to-task-graph decomposition | 0009 |
| 0015 | Operating surface | 0001 |
