# 0003: One model route, one classification ceiling

Status: proposed, 24 Nov 2026. The route itself is open until #19 is answered. Refs #23, #19, TD sections 5.4 and 9, PRD section 13 question 2.

## Context

Chat (#30) and recovery (#34) need a model. Officers work with student, staff and citizen data (PDPA), so a request may only go to a route cleared for the classification of the data it carries. The client must never hold model credentials.

## Decision

1. **One adapter, one route.** The companion service calls the model through a single adapter. The client talks only to the service. No other code path reaches a model.
2. **The route is not named yet.** #19 (hands-on) records the route, its owner, the highest classification it is cleared for, the models on it, and how the service authenticates. Until #19 closes, #30 builds against a stub and nothing real leaves the device. This note gets a "Route" section filled in from #19 and its status changes to accepted.
3. **Ceiling.** The adapter config holds one value, the route's classification ceiling. Every request carries a classification: set by the task for a run, by the officer for chat. The service compares the two before sending.
4. **Above the ceiling is refused.** The service returns an error to the client. It never downgrades, strips or masks a request to make it fit. The officer sees the refusal and the reason.
5. **Unknown classification counts as above the ceiling.**

## What a recovery request may carry (TD 5.4)

| Field | Sent | Condition |
|---|---|---|
| The step's `say` line, action and descriptor | Yes | Always. Holds no values. |
| Pruned accessibility tree of the focused window: roles, labels, structure | Yes | Always. |
| Field values in that tree | Masked | Sent only if the route is cleared for the data's classification. |
| Password fields | Never | Never read, never sent. |
| Screenshot | No | Only if the vision fallback is on and the route is cleared. Held in memory, never written to disk. |
| Recorded input values and list rows | No | Not part of a recovery request. |
| Credentials of any kind | Never | The client holds none. |

A recovery request is built by one function, so this table is one place to test. The test asserts that a request built from a tree with a seeded value and a password field contains neither unless the ceiling allows the value.

## Consequences

- #30 and #34 can start against a stub, so Phase 1 builds are not blocked on the clearance. Neither ships to real data before #19 closes.
- Changing route later changes the adapter config and this note, not callers.
- Phase 0 and all tests use synthetic data only.

## Open

- The route, owner, ceiling, models and authentication (#19).
- Whether chat documents need a lower ceiling than recovery requests. Decide when #30 handles dropped documents.
