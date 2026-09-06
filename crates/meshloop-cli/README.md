# meshloop-cli

The Meshloop engine binary (`meshloop`). Skills and local MCP are the operator
surface; this crate is the saga (ML-014).

```text
cargo install meshloop-cli --locked
meshloop bundle
```

`meshloop bundle` writes the version-locked `meshloop:` skill pack and MCP
catalog next to your current directory (`dist/meshloop-session-bundle` by
default). Copy `skills/` into the harness skill directory you already use.
Run `meshloop mcp` for stdio MCP. Do not use unprefixed `plan` / `reviewer`
tools.

Verified R1 path: native Windows. See the
[repository README](https://github.com/smota/meshloop) and
[Getting started](https://github.com/smota/meshloop/blob/main/docs/start.md).
