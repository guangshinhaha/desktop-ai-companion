# Desktop AI Companion (working title)

A thin desktop companion for Singapore public officers, built by GRL (GovTech).
It does the operating work Desk can't: the officer records a task once, and the
companion replays it by driving the real app's UI, verifying each step and
pausing for a named officer's approval before consequential steps. It also
drafts and summarises, built in as an interim until it consolidates with Desk.
Spec: `docs/prd.md` (source; regenerate `docs/prd.docx` with
`python3 scripts/md2docx.py docs/prd.md docs/prd.docx`). Technical
design: `docs/td.md`.

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

1. Work only issues labelled `ready`, oldest first. Up to 3 builds run at once (Shin, 24 Nov), each in its own worktree, only on issues whose dependencies have merged and that touch different areas of the code. Verifiers and CI don't count toward the 3.
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
13. Only Shin can finish a `hands-on` issue (live check, real accounts or config, a decision). Unattended runs skip it, and a PR for it uses `Refs #N`, never `Closes`.
14. `discovery` means not decided: run the spike or get the product call before building. A spike ends in a note in `docs/decisions/` and merges no product code.
15. Each milestone has one `epic` issue: the goal, what we build, and its sub-issues (GitHub sub-issues). The epic closes when the milestone's UAT has run and Shin has made the phase call.

### Labels

| Label | Means |
|---|---|
| `ready` | Shin's permission to build. Only Phase 0 issues carry it until the next phase is groomed |
| `risk:none` to `risk:high` | Review tier (rules 3 to 6) |
| `hands-on` | Only Shin can finish it (rule 13) |
| `discovery` | Spike or product call first (rule 14) |
| `consult-shin` | An advisor pick made while Shin was away, waiting for Shin's review (rule 9) |
| `epic` | A milestone's parent issue (rule 15) |

### Milestone description

Every milestone description has these parts, so `/shinstack:coordinate` can run it and `/shinstack:uat` can end it:

- **Goal:** one line.
- **In** and **Out**.
- **Order (beats oldest-first):** issue numbers, with why each comes where it does.
- **Hands-on (Shin):** what only Shin can do, and when.
- **Unattended run:** the exit command `gh issue list --milestone "<name>" --state open --search "-label:hands-on -label:epic"` prints nothing once the agent-doable issues merge. Then `/shinstack:uat`; Shin closes the milestone. Escape hatch: `/goal clear`.

## Commands

- Test everything: `scripts/test.sh` (runs every package, exits non-zero if any fails).
- Real-keychain store tests (ignored in CI, run on a signed-in Mac): `cd shell/src-tauri && cargo test --test store -- --ignored`.
- Needs Swift 6 (Xcode), Node 24+ (TypeScript runs via Node type stripping; no install step yet) and Rust (`brew install rustup`, `rustup-init -y`) for `shell/`.
- Shell: `cd shell/src-tauri && cargo build --release`; run `target/release/companion-shell` (menu bar, Cmd+Shift+Space opens the canvas). `--ping-executor` does the sidecar ping headless; set `EXECUTOR_BIN` to the executor binary.
- One package: `cd executor-macos && swift test`, or `cd <stand-in|harness|schema> && npm test`.
- CI: `.github/workflows/test.yml` runs `scripts/test.sh` on a macOS runner for every PR.

## Scope (from the PRD)

- GRL builds: recorder, replay executor, task library, approval experience,
  thin orchestrator. Drafting, audit and the library store are interim builds
  behind adapters, consolidated with Desk, Harness, Muninn and Assistant Desk
  later. Reuse their inner-source code where access is granted; never depend on
  that access until it is.
- Model calls go only through one adapter, to a route cleared for the data's
  classification. The client never holds model credentials.
- Current phase: Phase 0, one task recorded and replayed on a SEED MacBook
  against a stand-in app with synthetic data, captured on video. No model, no
  server. Nothing may depend on COMET access.
- Until Phase 2 ends, every feature must improve recording, replay or the
  library. The one exception is drafting and summarising.

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
