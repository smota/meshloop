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

## Native dispatch support

Proposed with ADR 0039; this section has no effect until that ADR is accepted.

This is a mirror of the compiled-in proven set in the composition root (for example
`crates/meshloop-cli/src/native_support.rs`). Native dispatch is honoured only for a
`kind` listed in that table. The table is read-only: it is not configurable through
`meshloop.toml`, and an operator cannot self-certify a harness. It changes only by pull
request with new live evidence from #152. Nodecode also needs an ADR 0038 amendment.
`doctor` warns when a probed version is outside the observed range.

| Kind | Observed version range | Live evidence (#152) | Date |
|---|---|---|---|
| (none proven yet) | — | — | — |
