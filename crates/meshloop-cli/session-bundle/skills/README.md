# Meshloop skills (session control plane)

Operator steps: [Getting started](../docs/start.md). Hub:
[docs/README.md](../docs/README.md).

You stay in this pane. Call, in order:

`/meshloop:doctor` → `/meshloop:plan` → `/meshloop:review-plan` → `/meshloop:run`

Every live command injects `--origin-harness` / `--origin-session` from
`MESHLOOP_ORIGIN_*` or doctor JSON so **this pane is not split**. Live workers
use Herdr. `--fixture-only` is CI.

Skills wrap `meshloop.exe` only (ML-014). **No saga in SKILL.md.**

Bare names (`plan`, `reviewer`, `scout`) are rejected.

## Slash skills (SKILL.md in this pack)

| Canonical id | Slash | MCP | CLI |
|---|---|---|---|
| `meshloop:doctor` | `/meshloop:doctor` | `meshloop_doctor` | `meshloop doctor` |
| `meshloop:plan` | `/meshloop:plan` | `meshloop_plan` | `meshloop plan` |
| `meshloop:review-plan` | `/meshloop:review-plan` | `meshloop_review_plan` | `meshloop review-plan` |
| `meshloop:run` | `/meshloop:run` | `meshloop_run` | `meshloop run` |
| `meshloop:status` | `/meshloop:status` | `meshloop_status` | `meshloop status` |
| `meshloop:accept` | `/meshloop:accept` | `meshloop_accept` | `meshloop accept` |
| `meshloop:orchestrate` | `/meshloop:orchestrate` | `meshloop_orchestrate` | `meshloop orchestrate` |
| `meshloop:roles` | `/meshloop:roles` | `meshloop_roles` | `meshloop roles` |

## Engine-only (no skill file yet)

`resume` · `inspect` · `cancel` · `integrate` · `mcp` · `bundle` — still
`meshloop <verb>` and MCP `meshloop_<verb>` where listed by `meshloop mcp`.
Integrate is the only command that merges onto a branch you name
(`--accept-integrate`). `bundle` writes the version-locked session pack
(`meshloop bundle [--dest <dir>]`).

Local MCP: `meshloop mcp` (stdio JSON-RPC).
