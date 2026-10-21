# Role collaboration and acceptance

Every role transition is a bilateral contract. The sending role defines what the receiving role
must produce and how that delivery will be accepted. The receiver remains accountable for its own
phase; the sender accepts only conformance to the contract it issued.

## Protocol

```text
Role A -> RoleHandoff + AcceptanceContract
Role B -> DeliveryReceipt
Verifier -> deterministic report
Council -> optional structured advice and synthesis
Role A -> AcceptanceDecision or ReworkRequest
```

`RoleHandoff` is immutable and uses `state: issued`. Acceptance is a separate digest-bound record;
changing the candidate, handover, contract, delivery, or council synthesis invalidates downstream
decisions.

## Initial phase

Phase 0 uses the same bilateral handoff as every later phase. `agentflow:requester` is a bootstrap
role, not a lifecycle phase and not the exploratory QA sidecar. That role is the acceptance owner
and sends the opening handoff to `agentflow:product-manager`, who remains the delivery role for
phase 0. The catalog declares the edge. Phase 0 does not skip the transition check, and the product
manager does not hand off to itself. The outgoing product-manager-to-analyst handoff is unchanged.

Lifecycle phases stay 0 through 8. A copied product-manager self-handoff is not a phase-0
contract: the acceptance owner is `agentflow:requester` and the delivery role stays
`agentflow:product-manager`. The exploratory QA sidecar is not that sender. Consuming projects
adopt this catalog with the framework; they do not add their own inbound edge.

## Complexity routing

| Class         | Trigger                                                                   | Required path                         |
| ------------- | ------------------------------------------------------------------------- | ------------------------------------- |
| `linear`      | bounded, low-risk, complete evidence                                      | deterministic checks                  |
| `bilateral`   | medium risk, uncertainty, or multiple domains                             | checks plus sender acceptance         |
| `council`     | public contract, migration, high risk/uncertainty, or broad domain impact | targeted council plus sender decision |
| `human-gated` | high assurance, sensitive surface, or irreversible action                 | council evidence plus human authority |

Routing uses explicit rules rather than an opaque numeric score. Projects may tune sensitive
surfaces, domain thresholds, public-contract/migration triggers, and structured council seats under
`agent-workflow.config.json#collaboration`. They may not weaken single-writer, action-boundary,
review-independence, or human-approval controls. This separates the stable protocol taxonomy from
the engineering or business-analysis methods chosen by each adopter.

```bash
agentflow-sdlc collaboration classify \
  --profile standard --risk high --domains api,data,ui --json
```

## Deterministic-first acceptance

Code validates record integrity, digests, candidate identity, required criterion results,
ArtifactRefs, handover provenance, and required council completion. Execution receipts separately
record writer leases and execution provenance; these are not authenticated by content digests.
Semantic review receives only the
contract, current delta, deterministic report, and unresolved questions.

An acceptance criterion declares `verification: deterministic|semantic` and optional
`severity: blocking|advisory` (default `blocking`). A required deterministic criterion passes only with
a `pass` result and evidence. Semantic criteria remain owned by the handover sender and cannot replace
specialist verdicts: planners check plan conformance, architects check design conformance, testers own
test verdicts, and reviewers own independent review. Blocking semantic criteria require passing findings
(`status: pass`) with a reason; advisory criteria allow non-blocking observations (`status: observation`
or `status: unverified`) without failing acceptance or blocking phase progression.

```bash
agentflow-sdlc collaboration verify \
  --handoff role-handoff.json \
  --delivery delivery-receipt.json \
  --council-request council-request.json \
  --council-advice council-advice.json \
  --council-synthesis council-synthesis.json \
  --json
```

## Councils

`CollaborationIntent.mode` selects an execution pattern (advisory, discovery, spike, and so on).
`AcceptanceContract.collaborationClass` selects the assurance path (linear, bilateral, council,
human-gated). They are distinct: a council assurance path can run sequentially or in parallel.

A council is an advisory pattern inside the current role, not a lifecycle phase or majority vote.
The request freezes one evidence digest, question, accountable owner, and role-based seats. Each
seat returns structured advice, evidence, objections, confidence, and out-of-scope notes. The owner
must disposition every blocking objection and remains the final decision owner.

`council-advice.json` is an array with one record per participating seat. Verification checks the
delivery digest, owner, required seats, exact advice digests, and every blocking objection. A
deferred blocking objection does not permit acceptance. Semantic acceptance requires one passing
finding with a reason for every required semantic criterion. Human-gated acceptance additionally
requires delivery-bound human approval evidence. Digests detect stale content; they do not
authenticate the human or replace the host platform's authorization controls.

Harnesses may execute seats in parallel, sequentially, in isolated sessions, or through humans.
The record format and accountability do not change.

## Records

- `AcceptanceContract`: criteria, complexity class, candidate digest, controls, and council policy.
- `DeliveryReceipt`: criterion results, evidence, deviations, execution reference, and provenance.
- `AcceptanceDecision`: deterministic report, semantic findings, conditions, and final state.
- `ReworkRequest`: failed criteria, concrete required changes, and rework round (circuit breaker capped at `maxReworkRounds`, default 2).
- `CouncilRequest`, `CouncilAdvice`, `CouncilSynthesis`: bounded advisory exchange.

Validate any record through `agentflow-sdlc collaboration validate --path <record.json> --json` or
the general `sdlc validate-evidence` command.

## Advancement gate

Record validation checks integrity; it does not authorize advancement. `createAcceptanceDecision`
recomputes verification from the source handoff, contract, delivery, and council records rather than
trusting a supplied report. Before advancing, run the source-bound gate with the current durable
rework ledger (`[]` explicitly means no open rework):

```bash
agentflow-sdlc collaboration advance \
  --handoff role-handoff.json --delivery delivery-receipt.json \
  --decision acceptance-decision.json --rework open-rework.json --json
```

Supply the council request, advice array, and synthesis flags above when required. Rejection,
rework, stale evidence, or conditional acceptance blocks advancement. When open rework requests
exceed `maxReworkRounds` (default 2, configurable in `agent-workflow.config.json#collaboration`),
advancement is blocked with an explicit escalation requirement, preventing unbounded peer rework loops.
Resolve conditions and issue a new unconditional `accepted` decision before continuing.
The orchestrator owns obtaining the complete current ledger from the authoritative workflow source; a local
file is only its input projection, not an independent source of truth.

## Epoch fencing and circuit breaker

Peer rework between delivery roles is bounded deterministically:

1. Every rework request monotonically updates rework tracking, derived via `deriveReworkRound`.
2. When rework iterations exceed the circuit breaker limit (`DEFAULT_MAX_REWORK_ROUNDS = 2`), the advancement gate fails closed.
3. The breaker trips via `tripLedgerBreaker`: sets `ledger.frozen = true`, increments integer `fencingToken`, and emits a sealed `reverse-seam-packet` record (`{ triggerCondition: 'rework-exhausted', fencingToken, round, targetIdentityDigest }`).
4. Subsequent commits or transitions carrying an older fencing token (`authored.fencingToken < ledger.fencingToken`) are rejected on the frozen branch.

## Reverse-seam unfreeze protocol

Unfreezing a deadlocked worktree is an external steering responsibility under repository policy (`steer.lead` or human authority).

1. The external steerer records a sealed `unfreeze-resolution` record (`createUnfreezeResolution`).
2. The record must declare:
   - `resolution`: must be one of `re-scope`, `slice-split`, `person-waiver`, or `abandonment`.
   - `actor`: declared authority string (policy restricts to `steer.lead` or human authority; validated structurally as a non-empty string).
   - `rationale`: substantive rationale explaining the intervention.
   - `fencingToken`: the next integer epoch token (`ledger.fencingToken + 1`).
3. `unfreezeLedger` verifies the resolution record, clears `ledger.frozen = false`, advances `ledger.fencingToken = nextToken`, and records `unfreezeResolutionDigest`. The candidate branch resumes execution at the target phase under the new epoch.

## Pragmatic defect triage (`accept_and_register`)

When a delivery candidate exhibits minor, non-blocking discrepancies or auxiliary telemetry anomalies that do not compromise primary functionality:

1. The external steerer may invoke `accept_and_register`.
2. This transitions the decision to `accepted-with-conditions` and mandates the immediate, idempotent registration of a tracked GitHub follow-up issue URL on every condition.
3. **Boundaries and Guardrails**:
   - Rejected while the ledger circuit breaker is tripped (`ledger.frozen === true`).
   - Rejected if any finding was initially logged as `[B] Blocker`.
   - Rejected on sensitive surfaces (`ACCEPT_AND_REGISTER_SURFACES`): `auth`, `crypto`, `migration`, `billing`.
