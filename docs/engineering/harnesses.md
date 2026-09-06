# Development harness contract

Selected set: codex, claude-code, pi, grok, agy. All five remain required for full
cross-harness acceptance. AGENTS.md is canonical; no skills are installed or required.

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
