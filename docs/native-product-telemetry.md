# Native product telemetry

The Rust product exports best-effort OTLP metrics only when `observability/consumer.env`
exists under `$XDG_CONFIG_HOME` (or `~/.config`). It reads at most 16 KiB as data,
never as shell code. Only `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SDK_DISABLED`, and
`MESHLOOP_TELEMETRY_DISABLED` are interpreted. All other fields are ignored.
The endpoint must be a numeric loopback HTTP socket address, normally
`http://127.0.0.1:24318`. No hostname resolution, remote endpoints, credentials,
headers, resource overrides, redirects, retries or persistent spool are supported.

Set `MESHLOOP_TELEMETRY_DISABLED=1` (or `true`) or `OTEL_SDK_DISABLED=true`
in the process environment or consumer file before starting the process to disable
export. Configuration is read once per process. Missing/invalid configuration disables it.

Requests go to `/v1/metrics` with **Content-Type: application/json**. This intentionally
uses OTLP JSON even when the shared consumer declares the standard `http/protobuf`
protocol. The existing collector must accept OTLP JSON; this does not change its configuration.
No SDK or additional dependency is installed.

`service.name=meshloop`, instrumentation scope `native-product`. Metrics:

- `meshloop.operation.count`: delta monotonic sum, one observed operation.
- `meshloop.operation.duration`: delta histogram in seconds, one actual runtime sample,
  with count/sum/min/max and one unbounded bucket.

Only fixed `operation` and `outcome` labels are exported. No paths, arguments,
task/graph/session names or IDs, prompts, errors, messages, content, models or host
identity are included. Valid version-00 `TRACEPARENT` values attach trace/span IDs
as duration exemplars, never metric labels. Invalid and zero IDs are ignored.
This is correlation with a caller's span, not creation of a new tracing span.

One background thread owns a 32-item queue. Enqueue never waits; overflow drops
observations. Connect/read/write waits have 50 ms socket deadlines and each request has a
150 ms total budget. Request bodies are capped at 8 KiB. HTTP response
inspection is capped at 256 bytes; failures produce no output and cannot change
operation results. Normal CLI shutdown drains for at most 150 ms, then permits loss.
Crashes, forced exits, a slow collector and queue pressure can lose observations.
This is operational monitoring, not an exact accounting ledger or a replacement
for persisted product evidence. Source builds do not activate installed binaries.

`cli.operation` reports CLI success/failure and actual invocation duration.
`graph.run` and `graph.resume` measure an actual native execution-loop invocation,
with outcomes `complete` (all nodes Integrated), `failure`, `cancelled`, `idle`, or
`error`. A pause for review is `idle`, never completion. These are invocation
outcomes; repeated resume invocations are not unique graph-lifetime counts.
Detached launching itself has only CLI timing; the child reports its own loop.

`task.transition` is emitted after successful SQLite commits, for both legacy and
scoped native execution. Outcome is the fixed destination TaskState. `task.dwell`
measures seconds spent in the prior TaskState only when the immediately preceding
committed event observed by this store for the same graph/task/generation entered that state and has a
nonzero, valid timestamp no later than the current event. Ready dwell is queue wait;
AwaitingReview dwell is acceptance wait. Missing, mismatched, reversed and invalid
timestamps omit the duration. The per-store timestamp history holds at most 128 entries;
keys above 256 bytes are not cached. Restart, reopening a store, or eviction omits
dwell until continuity is observed again. No telemetry SQL or disk replay runs on
transition paths. State timestamps currently have second resolution;
zero-second dwell is a real sample. No initial queue time or graph lifetime is invented.

These are native product signals, separate from adopted AgentFlow delivery-runner telemetry.
