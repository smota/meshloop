# Provider author path

Providers supply capabilities around AgentFlow; they do not redefine the SDLC.

Start read-only:

```bash
agentflow-sdlc providers list --json
agentflow-sdlc providers inspect manual --json
```

Then use:

- [Provider matrix](provider-matrix.md) for current facets and limitations.
- [Authoring](authoring.md) for the descriptor, inspection, binding, and receipt contracts.
- [Modular architecture](../modular-architecture.md) for ownership rules.

Provider availability is evidence, not configuration truth. Local CLI providers use a bare
executable plus argument array with `shell: false`; arbitrary shell command strings are deprecated
and never executed by routing.

The execution registry remains independent from optional harness providers. The shipped CLI
discovers only the built-in providers. Discovery does not install, configure, or grant execution to
any provider. Library consumers register their own providers with `additionalProviders`.
