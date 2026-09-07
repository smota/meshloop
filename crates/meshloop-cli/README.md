# meshloop-cli

The Meshloop engine binary (`meshloop`). Skills and local MCP are the operator
surface; this crate is the saga (ML-014).

```text
cargo install meshloop-cli --locked
meshloop --version
```

Then follow **[Install and setup](https://github.com/smota/meshloop/blob/main/docs/install.md)**
in a throwaway git repo: `meshloop.toml`, `meshloop bundle --dest .`, copy
`skills/meshloop-*` into the origin harness skill folder, `/meshloop:doctor`.

`meshloop bundle` writes the version-locked `meshloop:` skill pack and MCP
catalog (default `dist/meshloop-session-bundle`). Run `meshloop mcp` for stdio
MCP. Do not use unprefixed `plan` / `reviewer` tools.

Verified R1 path: native Windows. Loop:
[Getting started](https://github.com/smota/meshloop/blob/main/docs/start.md).
