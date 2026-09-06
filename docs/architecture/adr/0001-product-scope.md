# 0001 Product scope, platforms, and operating surface

- Status: Proposed
- Implementation: in-progress — CLI entry point (`plan`/`run`) implemented and tested on native Windows only; naming rule followed; WSL2 Linux side unverified (no C toolchain in the WSL distribution here); packaging not started. See docs/engineering/implementation-status.md.
- Date: 2026-09-05
- Author/executor: Claude (Sonnet 5), consolidated from prior Codex/user planning; absorbs former ADRs 0010 (toolchain/distribution) and 0015 (operating surface)
- Reviewer: pending
- Approval evidence: none for this runtime decision
- Supersedes: none
- Superseded by: none

## Context and constraints
Supported OS matrix, toolchain, packaging, and the operating surface all need one coherent
decision rather than three separate ones — they are facets of "what Meshloop is and runs
on/as," not independent tradeoffs. Development happens on Windows with Linux available via
WSL2 on that same host, not a separate bare-metal Linux machine. No service installation is
authorized; no skill-pack dependency is included.

## Alternatives
Foreground CLI versus resident daemon; native Windows plus separate bare-metal Linux versus
Windows-plus-WSL2 as one Tier A platform pairing; CLI-only versus CLI-plus-optional-skill
operating surface.

## Decision
Adopt Meshloop as the product and binary name. Tier A is Windows 10/11 native and Linux via
WSL2 on the same host; macOS is Tier B, best-effort. v1 runs foreground for one task graph
and exits — no daemon. The compiled `meshloop` binary is the sole functional entry point;
an optional, zero-engine-dependency skill wrapper may be added later, named for the
product, not for Herdr. Full detail — platform/WSL constraints, toolchain, packaging
targets, the two named optional skills, naming rules — is in runtime-design.md §1.

## Consequences
Implementation must remain within accepted decisions; unresolved capabilities must be
visible. The bootstrap does not establish runtime readiness. Every Tier A acceptance
scenario must run on both native Windows and Linux-under-WSL2, never treating one as a
stand-in for the other; a run must not cross the Windows/WSL filesystem boundary.

## Verification
See runtime-design.md §1's validation scenarios.
