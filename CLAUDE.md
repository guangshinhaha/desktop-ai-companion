# Desktop AI Companion (working title)

A thin desktop companion for Singapore public officers, built by GRL (GovTech).
It covers Desk's jobs (draft, summarise, research) through the GovTech Harness,
and does the operating work Desk can't: the officer records a task once, and the
companion replays it by driving the real app's UI, verifying each step and
pausing for a named officer's approval before consequential steps. Spec:
`docs/prd.docx` (canonical), text copy in `docs/prd.md`.

shinstack-mode: supervised

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
