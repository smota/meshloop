# 0027 Modular MCP server and client integration

- Status: Proposed
- Implementation: implemented
- Date: 2026-09-15
- Author/executor: Antigravity
- Reviewer: pending human
- Approval evidence: none
- Supersedes: none
- Superseded by: none

## Context and constraints
Modern AI harnesses (such as Claude Code, Codex, Cursor, and IDE extensions) communicate with external agent tools via the Model Context Protocol (MCP). To allow harnesses to inspect plans, review nodes, trigger executions, and query doctor status without ad-hoc CLI wrappers, Meshloop provides an MCP server interface.

Constraints:
- Zero async runtime pollution: the MCP server must not introduce Tokio or async runtimes into `meshloop-engine` or `meshloop-domain`.
- Hexagonal boundary: the MCP server is an interface adapter residing strictly within `crates/meshloop-cli`.
- Subprocess re-entrancy: tool invocations directly invoke the `meshloop` executable with `--json`, ensuring that the MCP server remains completely stateless and daemonless.
- Tool naming: colons (`:`) are forbidden in MCP tool names; canonical IDs like `meshloop:review-plan` must map to `meshloop_review_plan`.
- Protocol standards: support standard MCP specifications (including "2024-11-05", "2025-03-20", and "2026-01-01") with dynamic handshake negotiation.

## Alternatives
1. **Embed heavy async framework (e.g., Tokio + warp/axum/rmcp) in the coordinator**: Rejected: bloats compile times, violates daemonless single-threaded coordinator architecture, and brings high risk of runtime deadlock.
2. **Expose raw CLI tools without MCP schema**: Rejected: prevents IDEs and harnesses from discovering tools dynamically.
3. **Synchronous JSON-RPC 2.0 stdio server with dynamic protocol negotiation**: Chosen. Implemented in `meshloop-cli::mcp` using standard library I/O and `serde_json`, dispatching to child CLI verbs.

## Decision or proposal
1. **Stateless stdio Transport**:
   - `meshloop mcp` reads JSON-RPC 2.0 requests line-by-line from `stdin` and writes formatted JSON-RPC responses to `stdout`.
2. **Protocol Version Negotiation**:
   - In `initialize`, negotiate `protocolVersion` matching client requests for 2024, 2025, and 2026 specifications, defaulting to `2024-11-05`.
   - Advertise tool, resource, and prompt capabilities with `listChanged: false`.
   - Provide standard implementations for `tools/list`, `tools/call`, `resources/list`, `prompts/list`, and `ping`.
3. **Canonical Tool Discovery & Execution**:
   - Dynamically expose bundled CLI commands (`meshloop:status`, `meshloop:run`, `meshloop:review-plan`, etc.) converted to `meshloop_<verb>`.
   - Forward `origin_harness` and `origin_session` arguments transparently to maintain audit provenance.
   - Return structured standard error messages on invalid tool calls or syntax errors.

## Consequences
- Claude Code, Codex, and other MCP-compliant harnesses can natively orchestrate Meshloop via stdio.
- Retains pure zero-dependency, safe Rust execution in the CLI layer.
- Retains zero daemon requirement: the MCP server starts on-demand with the client session and terminates cleanly when stdin closes.

## Verification and implementation evidence
- `crates/meshloop-cli/src/mcp.rs`: unit tests verifying tool naming, JSON-RPC initialization, protocol version negotiation, and empty resource/prompt lists.
- `cargo test -p meshloop-cli --bin meshloop` passes with 100% success.
