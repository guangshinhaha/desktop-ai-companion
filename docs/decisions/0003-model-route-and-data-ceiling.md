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
| The step's action and descriptor | Yes | Always. The descriptor follows the tree's rules: role always; label, glob and dom id only for a control-chrome role and masked if they hold an input; path as roles only; anchors only as a count. |
| The step's `say` line, id and checks | No | Task text that can quote page content or values (narrowed in #34). |
| Pruned accessibility tree of the focused window: roles, structure | Yes | Always. Labels and dom ids only for control chrome (buttons, toggles, form fields, the window, a sheet or dialog, a heading inside a dialog); other text is omitted (narrowed in #34). Any sent label holding a run input is masked. |
| Field values in that tree | Masked | Sent only if the route is cleared for the data's classification. |
| Password fields | Never | Never read, never sent. |
| Screenshot | No | Only if the vision fallback is on and the route is cleared. Held in memory, never written to disk. |
| Recorded input values and list rows | No | Not part of a recovery request. |
| Credentials of any kind | Never | The client holds none. |

Also sent (#34), none of it page text: the classification, the app's bundle id, the attempt number and maximum, why the step diverged as a code from a closed set (`not_found`, `dialog`, `no_window`, `find_failed`, `check_failed`, `check_failed:<kind>`, `refused`), the step's consequence flag and index, and the input's name (an identifier only). Dom ids go only on control chrome.

Recovery never presses anything (#34): the only act a model's answer can cause is typing the step's own input into the step's own field. So no answer, from any route, can press a commit.

A recovery request is built by one function, so this table is one place to test. The test asserts that a request built from a tree with a seeded value and a password field contains neither unless the ceiling allows the value.

## Consequences

- #30 and #34 can start against a stub, so Phase 1 builds are not blocked on the clearance. Neither ships to real data before #19 closes.
- Changing route later changes the adapter config and this note, not callers.
- Phase 0 and all tests use synthetic data only.

## Open

- The route, owner, ceiling, models and authentication (#19).
- Whether chat documents need a lower ceiling than recovery requests. Decide when #30 handles dropped documents.
