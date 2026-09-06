# Security policy

## Supported versions

The supported source is `main` at version **0.1.0**. There is no older release
line and no packaged binary channel.

This file is a reporting path, not a claim that Meshloop is production-hardened.
The [threat model](../docs/architecture/threat-model.md) records design
requirements; it is not a guarantee that the current scaffold enforces them.

## What Meshloop does not do

- It does **not** store credentials. It uses CLI sessions you already
  authenticated. Do not put API keys in config, issues, or reports.
- It does **not** install a Windows service or run as a daemon.
- It does **not** merge onto the current branch unless you pass
  `integrate --accept-integrate`.
- Live workers require `--allow-live-harness`. Completing a live Claude Code,
  Codex, Pi, Grok, or Agy dispatch is **not** an R1 stamp.

## Please report

Private reports are appropriate for:

- Path escape from a worker worktree or `allowed_paths`
- Secrets or personal data written into `.meshloop/state.sqlite`, logs, or
  fixtures
- Unsolicited live harness spawn without `--allow-live-harness`
- Worktree or git operations that touch the operator branch without an
  explicit integrate gate
- Redaction failures in `--json` diagnostics

## Please do not report as a vulnerability

- “The fixture harness is not Claude / Codex”
- Missing WSL2, macOS, Linux, packaging, or concurrency greater than 1
- Model quality or plan quality
- Documented R1 limitations (see [ADR 0016](../docs/architecture/adr/0016-r1-closed-loop.md))

## How to report

Use [GitHub private vulnerability reporting](https://github.com/smota/meshloop/security/advisories/new).

Do **not** open a public issue or pull request with exploit detail, credentials,
session transcripts, or private prompts. Redact secrets from any logs you attach.

There is no published security email and no bug bounty. The maintainer is
[@smota](https://github.com/smota). There is no response SLA.
