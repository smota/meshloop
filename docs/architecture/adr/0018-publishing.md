# 0018 Publishing: crates.io source-install and version-locked session pack

- Status: Accepted
- Implementation: implemented — crates.io 0.1.0 uploaded 2026-09-06 (`meshloop-domain`, `meshloop-engine`, `meshloop-adapters`, `meshloop-cli`). GitHub Release binaries are not done.
- Date: 2026-09-06
- Accepted: 2026-09-06
- Author/executor: Grok, under human product direction
- Decision owner: Samuel
- Approval evidence: user instruction 2026-09-06 to enable publishing in addition to the session-bundle distribution path
- Supersedes: none
- Superseded by: none

## Context and constraints
R1 ships from a git clone (`cargo build -p meshloop-cli` plus `skills/` or `xtask bundle`). Workspace crates had `publish = false`. Operators work inside a harness session; the compiled binary is the engine (ML-014, ADR 0001). A `cargo install` that yields only the binary is an incomplete product: slash skills and the MCP catalog must match that binary's version.

No installer, no Windows service, no global configuration writes, no credential store. Further crates.io versions, GitHub Releases, commit, and push remain separately authorized. This ADR does not replace 0001; it extends the packaging facet. Prebuilt `meshloop-windows-x86_64.exe` / `meshloop-linux-x86_64` artifacts (implementation-plan Phase 9) stay deferred.

## Alternatives
A: Keep clone-only distribution (`publish = false`). Rejected — the user asked to enable publishing.
B: Publish only a renamed `meshloop` crate and hide domain/engine/adapters. Rejected — crates.io requires every path dependency to be published at a matching version.
C: Publish libraries as a stable Rust API. Rejected — the supported contract is the `meshloop` binary and the `meshloop:` session pack, not a library ABI.
D: crates.io for the four product crates, `xtask` unpublished, session pack embedded in the CLI and emitted by `meshloop bundle`, GitHub Release binaries later. Selected.

## Decision
1. Publish **meshloop-domain**, **meshloop-engine**, **meshloop-adapters**, and **meshloop-cli** to crates.io. Do not publish **xtask**.
2. Keep crate name `meshloop-cli` (binary name stays `meshloop`). Install: `cargo install meshloop-cli --locked`.
3. Library crates are implementation crates. Downstream code may depend on them; Meshloop does not promise a stable Rust API.
4. The session pack is version-locked to the binary. `meshloop bundle [--dest DIR]` writes `skills/`, `meshloop-mcp-tools.json`, and a pack README. Canonical git-clone skills remain `skills/` at the repo root; the CLI crate carries an embedded copy so crates.io builds do not need the workspace tree. `xtask check` fails if the copies drift.
5. MCP remains `meshloop mcp` on the same binary — not a third package.
6. Version is the workspace `Cargo.toml` version. Git tags `v<version>` label a release. crates.io upload and GitHub Release attachment are human-gated maintainer actions.
7. Verified install path in R1: native Windows. `cargo install` on other OSes is not an R1 support claim (WSL2 remains unverified).
8. No installer, service, or global skill-dir write. Default bundle destination is `dist/meshloop-session-bundle` under the current directory.

## Consequences
`publish = false` is removed from workspace defaults. Path dependencies carry a version so `cargo publish` can rewrite them to crates.io. `fixture_harness` stays an adapters binary (CI double); product install is `meshloop-cli` only. A tagged maintainer publish can run `xtask publish-dry` then `cargo publish` in dependency order (domain → engine → adapters → cli), waiting for the crates.io index after each crate. `cargo package` of a crate cannot resolve unpublished `meshloop-*` path+version dependencies, so `publish-dry` verifies `meshloop-domain` now and packages the others once their dependencies exist on the index. Trusted publishing / `CARGO_REGISTRY_TOKEN` is configured outside this repository. Forks must not present crates.io or GitHub artifacts as official Meshloop releases (TRADEMARKS.md).

## Verification
`cargo run -p xtask -- check`. `cargo run -p xtask -- smoke` includes `meshloop:bundle`. CLI test: `meshloop bundle --dest <tmp>` writes prefixed skills and a catalog whose version equals `CARGO_PKG_VERSION`. crates.io 0.1.0: https://crates.io/crates/meshloop-cli (and the three implementation crates). Operator install: `docs/install.md`.
