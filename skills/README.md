# Meshloop skills (session control plane)

Human map: [docs/README.md](../docs/README.md). This page is the session control
plane (slash / MCP / CLI wrappers). Skills wrap the compiled `meshloop` binary
only (ML-014).

Every skill, slash command, and MCP tool is **prefixed**. Bare names (`plan`,
`reviewer`, `scout`, `orchestrate`) are rejected so they cannot collide with Pi,
Claude Code, Grok, or other harness vocabularies.

| Canonical id | Slash | MCP tool | CLI |
|---|---|---|---|
| `meshloop:plan` | `/meshloop:plan` | `meshloop_plan` | `meshloop plan` |
| `meshloop:run` | `/meshloop:run` | `meshloop_run` | `meshloop run` |
| `meshloop:status` | `/meshloop:status` | `meshloop_status` | `meshloop status` |
| `meshloop:accept` | `/meshloop:accept` | `meshloop_accept` | `meshloop accept` |
| `meshloop:resume` | `/meshloop:resume` | `meshloop_resume` | `meshloop resume` |
| `meshloop:inspect` | `/meshloop:inspect` | `meshloop_inspect` | `meshloop inspect` |
| `meshloop:orchestrate` | `/meshloop:orchestrate` | `meshloop_orchestrate` | `meshloop orchestrate` |
| `meshloop:roles` | `/meshloop:roles` | `meshloop_roles` | `meshloop roles` |
| `meshloop:doctor` | `/meshloop:doctor` | `meshloop_doctor` | `meshloop doctor` |

They contain no saga.

Local MCP: `meshloop mcp` (stdio JSON-RPC).

Origin session is **supervisor-only**. Pass `--origin-harness` and `--origin-session`.
Live workers require Herdr + `--allow-live-harness`.
