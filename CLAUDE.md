# Desktop AI Companion (working title)

A thin desktop companion for Singapore public officers, built by GRL (GovTech).
It covers Desk's jobs (draft, summarise, research) through the GovTech Harness,
and does the operating work Desk can't: the officer records a task once, and the
companion replays it by driving the real app's UI, verifying each step and
pausing for a named officer's approval before consequential steps. Spec:
`docs/prd.docx` (canonical), text copy in `docs/prd.md`.

shinstack-mode: auto
shinstack-mode.deploy: ask
shinstack-mode.shared: ask

## Factory mode

Sessions here follow shinstack (the plugin must be installed) like a factory line. Use its skills; this section adds only what they don't say.

- Build an issue: `/shinstack:feature`. PR and review depth by tier: `/shinstack:opening-a-pr`.
- High risk: `/shinstack:blast-radius`, then `/shinstack:interrogate`.
- Independent verifier and merge: `/shinstack:ship`. Merge only with `--match-head-commit <sha>`; if gh refuses, stop and report.
- Several `ready` issues queued: `/shinstack:coordinate` runs them. It still takes them oldest first and one at a time.

Companion rules:

1. Work only issues labelled `ready`, oldest first, one at a time.
2. Intake gate: if the issue has no testable "Done when" list, comment what's missing and stop.
3. Tier comes from the issue's `risk:none|low|medium|high` label. If it has no `risk:` label, pick a tier, say which and why, and add the label.
4. Anything touching recording capture, redaction, stored recordings, the approval gate, the audit trail, or OS permissions (screen recording, accessibility) is at least `risk:high`.
5. `risk:none` (docs, typos, copy): no independent verifier. Tests and CI are the gate.
6. For `risk:low` and above, the verifier is a fresh subagent that hasn't seen the build reasoning. Its verdict counts only at the head commit it ran at.
7. No Codex or external review. Claude only.
8. Finish with a comment on the issue: the PR link and one line on what you learned. Friction in this repo goes in a new issue here; friction in shinstack goes to the shinstack repo (router: Friction).
9. Blocked on a product call while Shin is away: use `/shinstack:consult-shin`, log the pick on the issue, keep going. A PR that carries the pick merges on the normal gate for its tier; it does not wait for Shin to review the pick. Merge, deploy, credentials, irreversible actions, and anything in PRD Section 13 (open questions for 7 Dec) never go through it.
10. Each PRD phase is a milestone. A milestone ends with `/shinstack:uat` after its issues merge. It is not done until that has run. Issues whose Done when ends in a Live check stay open until it passes.
11. A phase's exit is a continue / narrow / stop call against PRD Section 10. That call is Shin's: report the evidence and stop.
12. A milestone's order (in its description) wins over oldest-first.

## Commands

TODO: no code yet. Add real scripts here once the stack is chosen.

## Scope (from the PRD)

- GRL builds: recorder client, replay executor, task library, approval
  experience, orchestrator. Everything else integrates (Harness, MCP gateway,
  GovBrain, Muninn, Assistant Desk governance). Don't rebuild any of those.
- Current phase: Phase 0, one task recorded and replayed on a SEED MacBook,
  captured on video. Nothing may depend on COMET access.
- Every feature must improve recording, replay or the library until Phase 2 ends.

## Performance rules (laggy managed laptops)

Event-based capture only, never continuous video. Reasoning in the cloud; the
client executes and reports state. Steps wait for verified state, never fixed
sleeps. Client is dormant when idle. Breaking a rule needs a written exception
in `docs/decisions/`.

## Data

Recordings may capture student, staff or citizen data (PDPA). Redact at
capture; store steps and field references, not values. Never send recordings
or captured data to external services, gists, pastes or public repos.

## Gates

Hooks and CI come from gantry (`lefthook.yml`, `.github/workflows/`).
A task is done only when the pre-push checks pass. Never use `--no-verify`.
CI runs every check even when an earlier one fails, so read the whole run.

## Review rules

- Auth, permissions, personal data, recording/redaction and deploy scripts: a
  human reviews line by line.
- UI: review by screenshot.

## Knowledge

- Decisions: docs/decisions/
- Backlog and status: GitHub issues, not .md files.
