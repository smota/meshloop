# meshloop-engine

Meshloop orchestration engine: plan, route, verify, recover. Consumes ports;
does not construct adapters.

This crate is an implementation crate. The supported product interface is the
`meshloop` binary from `meshloop-cli`, not a stable Rust API.

```text
cargo install meshloop-cli --locked
meshloop bundle
```
