# AgentFlow SDLC Definition

AgentFlow SDLC Definition is the product authority for agentic delivery. It defines the shared model used by roles, agents, skills, validators, migrations, audits, and Cockpit.

Machine-readable vocabulary lives in `sdlc.config.json`. Portable evidence and boundary contracts
are defined in [evidence-contracts.md](evidence-contracts.md) and
[lifecycle-boundaries.md](lifecycle-boundaries.md). Executable behavioral checks and derived,
non-authoritative outcome projections are defined in [agent-evals.md](agent-evals.md) and
[outcome-metrics.md](outcome-metrics.md).

## Authority model

- Human authority: `docs/sdlc-definition.md`.
- Machine authority: `sdlc.config.json`.
- Schema contract: `schemas/sdlc-config.schema.json`.
- Execution adapter: `agent-workflow.config.json` owns branch strategy, validation commands, routing,
  provider/source bindings, enabled extensions, and adoption preferences. It does not own domain
  vocabulary.

Harness-specific directories such as `.pi`, `.claude`, `.agy`, and `.codex` are generated adapter surfaces only. They are never canonical product source.

## Core concepts

| Concept                | Meaning                                                                | Durable sources                             |
| ---------------------- | ---------------------------------------------------------------------- | ------------------------------------------- |
| Workspace              | Configured repository/project boundary                                 | config, Cockpit query state                 |
| Goal Group             | Parent objective/epic                                                  | issue body, relationships                   |
| Goal                   | Outcome a person accepts before breakdown (see Work altitudes)         | issue body, labels, comments                |
| Capability             | Specification agents write under an accepted goal                      | issue body, comments                        |
| Spec                   | Implementation unit where phases 0-8 run                               | issue body, role passes, PR                 |
| Delivery               | Implementation and PR activity                                         | PR, commits, checks                         |
| Role Flow              | Ordered role contributions and returns                                 | role-pass, workflow-status, handover        |
| Readiness              | Path-aware applicable quality state                                    | issue/PR evidence, checks                   |
| Release                | Target, impact, assignment, released/unreleased state                  | issue fields, milestone, PR/release records |
| Human approval gate    | Explicit human decision required by high-assurance work                | PR review or gate record                    |
| Follow-up              | Deferred work tracked as issue                                         | follow-up issue links                       |
| Source                 | External durable record link                                           | `SourceAdapter` artifact references         |
| Guided workflow action | Preview-first safe update to durable workflow records                  | Cockpit/action audit                        |
| Cockpit                | Optional first-class Goal Command Center projecting durable SDLC state | source adapter and CLI records              |

GitHub is the first and default `SourceAdapter`. AgentFlow core remains source-neutral, and product
language leads with AgentFlow concepts.

Cockpit is an official optional projection of this model. It may visualize goals, readiness, role flow, release state, replay, approvals, and follow-ups, but it must not own unique SDLC state or be required by adoption, validators, skills, plugins, or settings merge.

## Work altitudes

Work is held at three altitudes. Each one is a different record, with a `kind` of `goal`,
`capability`, or `spec`. One record is never all three.

| Kind       | Holds                                    | Parent                        | Person                                    |
| ---------- | ---------------------------------------- | ----------------------------- | ----------------------------------------- |
| Goal       | the outcome or job to be done            | none                          | accepts it before any capability opens    |
| Capability | the specification of how the goal is met | a goal that a person accepted | reviews it only when it is high-assurance |
| Spec       | one implementation unit                  | a capability                  | none at this altitude                     |

How a GitHub issue shows that kind and its parent is defined in [Issue Hierarchy](issue-standards.md#issue-hierarchy).

Phases 0-8 are the lifecycle inside a spec. They are not extra records, and they are not seats.
A follow-up is a capability or a spec under an existing goal. It is not a second goal for the same
intent.

`lib/core/work-altitude.mjs` checks these rules when a record opens:

- A capability is refused unless a person admitted its parent goal's current revision at the
  capability's change class.
- A spec is refused unless its parent capability carries that admission, and the admission still
  matches the change class stored on the capability now.
- A standard spec does not open on a stored admission. It opens when the person accepts that capability's revision.
  The admission lives in the record it vouches for, so anyone who can edit that record can replace
  the change class and the admission together with a freshly sealed pair. A spec under any
  capability that is not high-assurance is refused, even when its stored admission matches.
- A spec under a high-assurance capability opens only when a person agreed to that capability's
  current revision in a separate review passed by the caller. A person can only review a capability
  that exists, so the review gates the specs under it. The stored admission passed as that review
  is refused.
- The kinds cannot stand in for each other.
- A record with no `kind` is legacy. It stays legal, and it is never read as a goal.

Consent is never a field on a record. It is a sealed `adequacy-of-intent` gate plus a person's
attestation, checked by `satisfyGate` in `lib/core/gate.mjs`. Only decision `agree` from a
`human-gate` reviewer on the registered human platform counts. `blocked` and `changes-requested`
are refusals. A gate bound to another subject, of another gate class, or altered after sealing is
refused.

The subject is what the person agreed to. For a capability, it is the digest of the goal subject and
the change class together (`admissionDigest`). The goal subject is the goal a person reads, not the
medium revision, so saving the answer does not move it. The class decides whether its specs need a
review. The capability keeps that gate and attestation as its admission. When a spec opens, the
admission is checked again against the change class stored at that moment. Downgrading the class,
on disk or in an issue body, no longer matches, and the spec is refused. Replacing the class and the
admission together with a newly sealed pair does match, which is why a passing check never opens a
spec on its own (#344). There is no witness outside the record yet, so no stored value can. For a
high-assurance
review, the subject is the capability's current revision. Editing a record after the person agreed
changes what it digests to, so the consent no longer applies.

`agentflow-sdlc phase append` applies these rules when it creates a record with `--kind`. It reads
`--parent` through the same medium, and takes consent as `--gate-file` and `--attestation-file`.
A medium stores the kind, the parent, the change class, and a capability's admission. The admission
is evidence that is checked again every time, never a stored yes.

The high-assurance capability review uses the existing `adequacy-of-intent` gate class over the
capability. These altitudes add no new gate class. An admission is the same class with subject kind
`admission`, not a second class. The person gates are the gate classes in `lib/core/gate.mjs`,
resolved by `lib/core/posture.mjs`:

- **Intent freeze** (`adequacy-of-intent`): a person accepts the goal. The same class on subject kind
  `admission` admits capabilities under that goal at one change class. Agreeing to the goal does not
  admit a class. This is required at every posture.
- **Agent escalation** (`agent-escalation`): an agent stops and asks. This is required at every
  posture.
- **Release or merge** (`release-of-candidate`): required when the posture or the change class asks
  for it. Merge and other external actions stay with the person.

A role acknowledgement, a seat choice, and a phase transition inside a spec are not person gates.

The altitudes are harness-neutral. One agent with no rig applies the same rules as a squad. A
harness may run them; it does not define them (see
[ADR 004](adr/004-separate-sdlc-policy-from-harness-execution.md)).

## Paths

Canonical workflow profiles:

- `bounded`: low-risk, narrow work; self-review allowed; optional architecture/product/docs roles may be skipped with reason.
- `standard`: default delivery path; architecture, implementation, validation, review, and PR readiness required.
- `high-assurance`: security/production/data-risk path; all roles required; self-review forbidden; human approval gate required.
- `exploratory`: research/QA/discovery path; implementation roles may be optional until delivery begins.

Rules:

- Path is selected before implementation.
- Skipped-by-path roles are excluded from readiness denominator.
- Skips require reason.
- Escalate to `high-assurance` when security, auth, data migration, production deployment, customer data, or explicit policy demands it.

### Orthogonal dimensions: profiles, postures, and adapter topology rungs

AgentFlow strictly separates delivery governance from harness topology:

1. **Workflow Profiles** (`bounded`, `standard`, `high-assurance`, `exploratory`):
   Govern phase requirements, role-pass constraints, and verification rigor. Profiles are canonical AgentFlow vocabulary.
2. **Postures** (`advisory`, `assisted`, `delegated`, `autonomous`):
   Govern agent autonomy, human-gate intervention points, and attestation requirements (`lib/core/posture.mjs`).
3. **Adapter Topology Rungs** (e.g., OpenRig P0-P4):
   Govern execution squad seat assignment and multi-agent collaboration classes (`linear`, `bilateral`, `council`, `human-gated`).

These three dimensions are orthogonal:

- An adapter topology rung chooses builder seats and collaboration topology; it never alters canonical phases, deletes roles, or bypasses gate classes.
- A task at any rung may run under any workflow profile based on risk.
- High-assurance governance remains enforced regardless of the executing harness or squad rung.

## Role flow

The product catalog maps the workflow slugs to qualified identities such as
`agentflow:product-manager`, `agentflow:implementation-planner`, `agentflow:reviewer`, and
`agentflow:technical-writer`. See
[`roles/index.md`](roles/index.md). Roles are accountability contracts; actors execute them, skills
supply capabilities, and method plays customize how they operate.

Phase 0 is entered by a catalog handoff from `agentflow:requester`. That bootstrap role is not a
numbered lifecycle phase. The product manager remains phase 0 and owns delivery; the requester owns
acceptance.

Canonical sequence:

0. Product manager
1. Analyst
2. Architect
3. Implementation planner
4. Developer
5. Tester
6. Reviewer
7. Technical writer
8. PR readiness

Allowed returns:

- Developer -> Implementation planner for planning defect.
- Reviewer -> Developer for review findings.
- Technical writer -> Developer for docs remediation.
- PR readiness → Developer for merge blocker.

Each role pass records issue, branch, role, profile, owner/executor/provenance, inputs read, decisions, uncertainty, validation, next-role contract, status, and signature.

## Product and delivery seam

Product work ends at phase 1. Delivery starts at phase 2. The phase says which side a piece of
work is on, so a reader needs no rig, seat, or team name to tell them apart.

- **Product.** The product manager (phase 0) proposes the problem and the outcome. The analyst
  (phase 1) owns the acceptance criteria. Those criteria are the spec delivery receives. Product
  work does not edit product code, commit, open a production pull request, or publish.
- **Delivery.** Phases 2 and 3 choose the technical path, and phases 4 onward build and prove it.
  Phase 3 may contain an executable plan. That plan is delivery's own work, not a product handoff.
  Delivery does not rewrite the outcome to fit the code.

The phase 1 handoff names the outcome, the public journey, the constraints, the anti-goals, the
references to read, and the unknowns. Verification steps are allowed. File-by-file edit
instructions are not a product handoff. `phase append` does not check this prose; its machine check
is the transition envelope. Whether the fields are present, and whether the body prescribes files,
is a reviewer judgment.

When the built candidate and the spec disagree:

- If the candidate violates frozen acceptance, delivery reworks it through the allowed returns, and
  the tester checks the remedy.
- If the outcome, a constraint, or an anti-goal was wrong or has changed, that is an intent defect,
  not delivery's to fix. Delivery stops, and the person accepts revised intent before delivery
  continues. The phase graph has no edge from delivery back to phase 1, so this stop is not recorded
  as a phase 1 pass.
- If the spec is silent, delivery decides and records the decision in its role pass.

Four acts stay separate. None stands in for another.

| Act              | Who                           | What it decides                                             |
| ---------------- | ----------------------------- | ----------------------------------------------------------- |
| Tester evidence  | Tester, phase 5               | The candidate meets the acceptance criteria                 |
| Reviewer verdict | Reviewer, phase 6             | The change is right against the spec                        |
| Door judgment    | The requester, a person       | The public journey on the named candidate meets the outcome |
| Merge or publish | The person, as posture allows | The candidate leaves the delivery branch                    |

The door journey is written into the phase 1 acceptance criteria before the build. A phase 5 run
of that journey is tester evidence only. It does not show the goal was the right goal, and it does
not authorize publish. Phase 6 does not perform the door test and does not read a door note as
approval. The door judgment is not a numbered phase and adds no gate class and no record type. It
is made through the existing `adequacy-of-intent` gate, and it names the goal revision, the spec revision, the
candidate SHA, the journey, and what was observed. A bare "looks good" is not a door judgment.

One actor may hold more than one role. The same actor must not be the developer, the reviewer, and
the door-test witness.

Publish follows the posture in `lib/core/posture.mjs`. Not every posture requires a person to
release. A project or team may add that rule; the core does not teach that it is universal.

A harness permission policy neither grants nor proves the propose-only boundary of product work.
Seat ladders and team topologies belong to a harness adapter. An adapter may map its own seat
names onto workflow profile and collaboration class, but it cannot change phases, gates, or
acceptance (see [ADR 004](adr/004-separate-sdlc-policy-from-harness-execution.md)).

## Role ownership registry

| Role                   | Owns                                           | Reads                          | Writes                                 | Handoff                                     |
| ---------------------- | ---------------------------------------------- | ------------------------------ | -------------------------------------- | ------------------------------------------- |
| Requester              | opening request                                | user request                   | request record                         | opening handoff to Product manager          |
| Product manager        | goal purpose, user/job framing, release intent | opening request, product docs  | goal/epic framing                      | clear job/problem to Analyst                |
| Analyst                | requirements, acceptance, scope boundary       | goal framing, comments         | acceptance criteria, open questions    | testable scope to Architect                 |
| Architect              | path selection, technical design, risk         | requirements, constraints      | design, risk, profile                  | plan-ready design to Implementation planner |
| Implementation planner | implementation and validation plan             | design, repo context           | file/test/doc plan                     | executable plan to Developer                |
| Developer              | code/docs implementation                       | plan, design, tests            | commits, implementation evidence       | changed implementation to Tester            |
| Tester                 | validation evidence                            | acceptance, implementation     | test results, coverage notes           | pass/fail evidence to Review                |
| Reviewer               | findings, independence, approval request       | diff, evidence, role passes    | review findings, gate decision request | accepted/returned work to next role         |
| Technical writer       | docs, release notes, language consistency      | implementation, release impact | docs/release note evidence             | docs-ready state to PR readiness            |
| PR readiness           | manifest, issue closure, merge readiness       | all evidence                   | PR body, follow-up status              | merge-ready PR or return reason             |

## Labels and lifecycle

Label groups:

- Type/domain: `epic`, `feature`, `bug`, `dx`, `tooling`, `documentation`, `qa`, `exploratory`.
- Routing: `for-implementation:<agent>`.
- Lifecycle: `drafted-by:<agent>`, `implemented-by:<agent>`, `for-review:<agent>`, `reviewed-by:<agent>`.
- Process state: exactly one of `state:backlog`, `state:readiness`, `state:wip`, `state:delivered`,
  projected from the phase record and the integration fact. The four are fixed; see
  [Issue standards](issue-standards.md#process-state).
- Integration/release: `integrated:<branch>`, `awaiting-release`.
- Test debt: `needs-test`.

Rules:

- New issues need one primary type/domain label.
- Agent provenance labels are factual audit metadata.
- Deprecated `agent:*` labels are forbidden for new work.
- Labels do not replace role-pass evidence.
- The process-state label is projected, never set by hand. An assisted run and a one-agent run
  get the same label from the same record. It is not a second state machine: the phase transitions
  remain the record.

## Gateways

| Gate                | Owner                         | Required evidence                        |
| ------------------- | ----------------------------- | ---------------------------------------- |
| Issue readiness     | PM/Analyst                    | title, labels, acceptance, scope         |
| Phase transition    | current role                  | role-pass + allowed transition           |
| Validation          | Tester                        | command/manual validation evidence       |
| Review              | Reviewer                      | findings, independence boundary          |
| Human approval      | human/reviewer                | reviewer, scope, decision, notes         |
| PR readiness        | PR readiness                  | manifest, validation, review, follow-ups |
| Release readiness   | Technical writer/PR readiness | release assignment, notes, blockers      |
| Guided action write | action gateway                | preview, auth, CSRF, confirmation, audit |

Human approval gate record:

```md
## Human approval gate

Reviewer: @login
Scope: security | acceptance | release | other
Decision: approved | changes-requested | blocked
Notes: ...
```

## Release model

Release candidate sources:

1. Explicit issue fields: `Release: vX.Y.Z`, `Target release: vX.Y.Z`, `Version: vX.Y.Z`.
2. Milestone title exactly matching release version.

Incidental semver in package versions, logs, runtime output, or prose is ignored.

Release assignment states:

- `assigned`: release-impact goal has target release.
- `needs-assignment`: release-impact goal lacks target release.
- `released`: delivered/closed and not awaiting release.
- `no-release-impact`: explicitly excluded.

Release filters:

- `unreleased`
- `released`
- `all`
- `needs-assignment`
- future specific version filters

## Evidence envelopes

Cross-harness state should use deterministic markdown envelopes when recording structured state.

````md
<!-- [AGENTFLOW-ROLE-PASS-v1] -->

```json
{
  "issue": 123,
  "role": "developer",
  "profile": "standard",
  "executor": "agy-cli",
  "transport": "local-cli",
  "delegationBoundary": "current-session",
  "status": "pass"
}
```

<!-- [/AGENTFLOW-ROLE-PASS-v1] -->
````

Rules:

- Envelopes supplement human prose; they do not hide decisions.
- Parsers must ignore malformed envelopes and report audit findings.
- Do not include raw prompts, transcripts, secrets, full logs, or tool payloads.

## Extension policy

Adopting projects may extend:

- roles
- paths
- labels
- gates
- validators
- harness adapters
- release policies

Extensions must:

- declare owner and compatibility.
- preserve role-pass provenance.
- preserve high-assurance human approval.
- preserve readiness denominator rules.
- provide validator or explicit non-automatable rationale.
- avoid hidden TODOs.
- keep product source out of harness-specific generated directories.

Extensions must not:

- weaken high-assurance gates silently.
- claim multi-agent work without attribution.
- treat source systems as primary product concepts.
- derive release candidate from incidental semver.
- store secrets/raw transcripts/full logs as durable evidence.

## Harness contract

Every harness follows:

```text
hydrate -> act -> validate -> flush
```

- Hydrate latest durable state.
- Act within selected path and role ownership.
- Validate with deterministic commands.
- Flush structured evidence to durable surfaces.

Agent slug is not execution target. Record launcher, executor, transport, delegation boundary, context boundary, model/runtime, and independence boundary where applicable.

## Production validators

Required command surface:

- `validate-sdlc-config`
- `validate-sdlc-issue`
- `validate-sdlc-role-pass`
- `validate-sdlc-pr`
- `validate-sdlc-release`
- `validate-sdlc-skill`
- `validate-sdlc-agent`

Validators emit human-readable output and `--json` where useful.

## Verifiable delivery and recovery

New run acceptance uses a frozen v2 evidence policy. Deterministic criteria require source-resolved observations for the current candidate and check definition, in addition to the accountable role decision. Unknown writer status, unresolved external effects and open rework block unsafe advancement or transfer. Runtime policy remains `sdlc.config.json.deliveryPolicy`; operational commands, inputs and source bindings live in `agent-workflow.config.json.delivery`. See [ADR 007](adr/007-verifiable-recoverable-delivery.md) and [reliable delivery](reliable-delivery.md).
