# Adopter path

Use this route to evaluate or add AgentFlow to an existing repository without surrendering local
policy or accepting an opaque write.

Adoption, and bringing an existing installation forward, is one journey:
[Get started](../get-started.md). In short:

1. [Inspect](../get-started.md#inspect) the project; it writes nothing.
2. Choose an [install profile](profiles.md) if `standard` is not the right one.
3. [Preview](../get-started.md#preview) every file the journey would write.
4. [Make the choices](../get-started.md#make-the-choices-the-preview-asks-for) it asks for, and
   confirm only that exact preview.
5. Read the [readiness report](../get-started.md#read-the-readiness-report); keep its undo command.
6. Add a source adapter or provider only when the project needs it.
7. Run a first issue through the documented workflow.

The core works with manual execution or any single coding CLI (**Substrate `S0`**). A missing Claude, Codex, Agy, Pi, or Grok
binary does not block adoption unless project policy explicitly requires that provider capability. As your team's needs grow, you can scale incrementally from **Substrate `S0`** (Single-CLI + native L2 subagents) to **Substrate `S1`** (Multi-CLI orthogonal builder/reviewer routing without a daemon) or **Substrate `S2`** ([OpenRig Lean 4-Seat Squad](../../OPENRIG.md)); see [Project setup](../project-setup.md).

For a legacy or unrecognized installation, the same journey asks for an explicit
[migration](../get-started.md#a-legacy-lock) or [recovery](../get-started.md#an-unknown-or-malformed-lock)
choice. Later setting changes use [Refine your setup](../assisted-configuration.md).
For resuming or transferring active work across agent harnesses, see the [In-Flight Plan Takeover Playbook](plan-takeover-playbook.md).
