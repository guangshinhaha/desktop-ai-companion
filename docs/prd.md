<!-- Source of truth. docs/prd.docx is generated from this file. -->

# Desktop AI Companion: Product Requirements Document

Giant Robot Lab, GovTech Singapore

A companion that learns an officer's tasks, remembers them and does them, so officers spend their time on judgment, not on operating software.

| Field | Detail |
|---|---|
| Working title | Desktop AI Companion. The product name is decided separately. |
| Author | Shin, Pilot, Giant Robot Lab (GRL) |
| Date | 24 November 2026 |
| Version | Draft v2. Replaces v1 of the same date. |
| Status | Draft for the GRL Year End Review and Planning on 7 December 2026 |
| Quadrant | Better Mousetrap: high impact, established technology. Judged on the complete workflow, not on model novelty. |
| Technical design | `docs/td.md` |
| Decision sought on 7 Dec | Approve Phase 0 and Phase 1 under the interim-build approach in this draft. Confirm the relationship to the Legacy Exoskeleton bet. Name owners for the model route, the pilot officer and the Desk consolidation questions in Section 13. |

## What changed from v1

v1 assumed the companion would reach Desk, the GovTech Harness, GovBrain and Muninn through integration from the start. Those integration paths are uncertain, and waiting for them would delay the evidence this product exists to collect. v2 changes the approach:

1. **The companion builds the drafting and summarising jobs itself, as an interim.** They give the officer a reason to open the companion every day. They are consolidated with Desk or a shared Harness agent once the product has proven value.
2. **Shared services are reused, not waited for.** The companion uses inner-source code from Harness, GovBrain or Muninn where access is granted. Where it is not, the companion builds the thinnest version it needs behind an adapter, so moving to the shared service later is a configuration change.
3. **The one integration it cannot skip is a model route cleared for the data's classification**, for example an AWS Bedrock model in the Government Commercial Cloud (GCC) cleared to Restricted / Sensitive Normal, or the Harness route.
4. **Phase 0 and the Phase 1 build need no partner.** They use workflow shapes from the author's own work at MOE DXD, rebuilt as stand-in apps with synthetic data. A real officer is still required before the Phase 1 pilot window starts.
5. **The executor is a native desktop app** (macOS first, Windows next) that reads the operating system's accessibility tree, not a browser extension. Officer workflows cross apps (a spreadsheet into a web form, an email into a case system), and only a native executor sees all of them.
6. **Replay over a list is in Phase 1.** One recorded task runs over many rows from a spreadsheet, with approval for each item. Approving a whole batch at once comes later, capped, after replay meets the reliability bar.
7. **The permissions path for officers' managed laptops moves to Phase 1.** Phase 0 runs on a SEED MacBook where the tester grants permissions.
8. **Memory starts with tasks.** Phase 2 attaches notes and decisions to recorded tasks. Wider memory of the officer comes later. "Watches" is dropped from the tagline.
9. **Only replay returns count toward the adoption kill criterion.** Chat returns are tracked separately.

## Executive summary

Officers do judgment work, but they spend their days operating software. They copy data between systems that do not talk to each other and re-key the same information across legacy applications that have no application programming interface (API). Pair and Desk have made drafting faster. They have not touched the operating work, and they start every session with no memory of the officer's tasks.

The Desktop AI Companion is a thin desktop app. It sits in the menu bar, stays dormant when idle, and opens to a blank canvas with one input. It does two jobs:

1. **Drafting and summarising**, built into the companion as an interim, so the officer has a reason to open it daily.
2. **Operating applications.** The officer records a task once by doing it normally. Next time, the companion replays it by driving the real application through its user interface, over one case or a list of cases. It verifies each step and pauses for the named officer's approval before anything consequential.

GRL builds the recorder, the replay executor, the task library, the approval experience and a thin orchestrator. For the drafting jobs, memory and governance, the companion builds the thinnest interim version behind adapters and consolidates with the shared GovTech services once value is proven.

The test is deliberately small. Phase 0 is one recorded task replayed on a SEED MacBook against a stand-in app, captured on video. Phase 1 is one real workflow with one real officer, run concierge-style. The product continues only if that officer returns unprompted to replay tasks and the replay holds up on the real workflow, not because the build is progressing. Every design decision targets the slow managed laptop officers actually use, even though testing starts on SEED hardware.

Positioning is "do what Desk can't". The drafting jobs are there so officers do not need two tools during the pilot. They are not a bid to replace Desk.

## Contents

1. Problem statement
2. Target users
3. Jobs to be done
4. Goals and non-goals
5. Product principles
6. User stories
7. Functional requirements
8. Architecture and integrations
9. Constraints
10. Success metrics and kill criteria
11. Phased rollout
12. Risks
13. Open questions and decisions needed

Appendix A: Relationship to Legacy Exoskeleton and the GRL portfolio
Appendix B: Market and reference notes

## 1. Problem statement

### Officers operate software instead of exercising judgment

The core problem is operating burden. A large share of an officer's day goes to moving information by hand: copying values from one system into another, re-keying the same case details across modules of a legacy application, checking several screens to assemble one answer, and repeating the same clicks for every case. The systems involved often have no API and no integration path, so the officer is the integration.

Chat assistants do not solve this. Pair, GovTech's chat assistant, and Desk, GovTech's AI workspace, help an officer draft, summarise and reason. The officer still has to go and do the work in the applications, by hand. The draft got faster. The thirty clicks after it did not.

### Work has no memory

Work also restarts from zero. Assistants start each session without knowing the officer's live cases, decisions or prior tasks. When an officer leaves, their successor rebuilds context from threads, documents and handover notes. Know-how about how a task is done in a specific agency system lives in people, not in a shared, reusable form.

### What is verified and what is still assumed

This PRD separates what the author verified first-hand at the Ministry of Education's Digital Experience Division (DXD) from what remains assumed until Phase 1. GRL's rule applies: stated intent is not adoption.

| Claim | Basis | Status |
|---|---|---|
| Officers re-key and copy data across legacy systems with no APIs | Author's direct work on school-facing systems at MOE DXD, including School Cockpit workflows and role and account provisioning | Verified first-hand |
| A good draft does not remove the follow-on operating work | Same experience: drafting help still left the in-application steps to the officer | Verified first-hand |
| Handovers lose context and successors rebuild from scratch | Author's own handover portfolio at DXD in November 2026 | Verified first-hand |
| Operating workflows in other agencies have the same shapes as at MOE (lookup across screens, re-keying from a sheet, account provisioning) | Single-agency experience; common government software patterns | Assumed. Phase 0 builds on it; Phase 1 tests it with a real officer |
| Officers prefer consumer AI tools over sovereign tools on experience grounds | Observed behaviour reported in team discussion; no survey or usage data | Assumed. Tested in Phase 1 interviews |
| Officers will record tasks and reuse each other's recordings | No test yet. This is the central behavioural bet | Unproven. Phase 1 and Phase 2 test it |

No claim in this document should be read as a whole-of-government finding.

## 2. Target users

The primary user is a Singapore public officer whose work involves repeated, structured tasks inside one or more agency systems: case processing, records lookup, data entry, status checking and routine submissions. They also draft and summarise, today with Pair or Desk.

### Who first

**Phase 0: the author.** The first tasks are workflow shapes the author ran at MOE DXD, rebuilt as stand-in apps with synthetic data on a SEED MacBook:

- re-keying rows from a spreadsheet into a web form (account and role provisioning);
- looking up one record across several screens to assemble an answer (School Cockpit-style lookup);
- copying details from an email into a case form.

No partner and no real data are needed. The stand-in apps also give a repeatable way to measure replay reliability before any officer is involved.

**Phase 1: one agency, one workflow, one officer.** The Phase 1 build starts without a partner. The Phase 1 pilot window cannot start without one. The pilot workflow must pass four tests: it repeats often enough that recording it pays back quickly; it runs in a real agency application, ideally one without an API; its steps are stable enough to replay; and a named officer owns the outcome and can approve consequential steps. The agency, workflow and officer are an open question (Section 13). MOE candidates, including School Cockpit workflows, are illustrative only.

The pilot officer is not a test subject for a demo. In Phase 1 they get a concierge service: the team does the work with them, watches where the value appears, and only then hardens the product.

### Who later

- the pilot officer's immediate team, who gain the shared task library without recording anything themselves;
- other teams in the same agency working in the same applications, where recorded tasks transfer with little change;
- other agencies, only where a recorded task or the pattern behind it transfers. Build deeply for one agency, abstract the core, then customise for the next.

Out of scope as first users: officers whose work is mostly unstructured deliberation with no repeated application tasks, and teams whose systems already have complete APIs and automation.

## 3. Jobs to be done

The companion owns the operating jobs no current GovTech assistant does. It also covers the drafting jobs, built in as an interim, so the officer does not need two tools during the pilot.

| Job | How it is served | When |
|---|---|---|
| Operate a legacy application end to end | Replay executor driving the real user interface | Phase 0 (one task), Phase 1 (real workflow) |
| Do the same task for many cases | Replay over a list of rows, approval for each item | Phase 1 |
| Draft a document, email or submission | Built in the companion, model reached through a cleared route. Consolidated with Desk later | Phase 1 |
| Summarise a long document or thread | Same as drafting | Phase 1 |
| Build a helper for a team task | Record and share a task. The recording is the helper | Phase 2 |
| Carry context across sessions and handovers | Task library with notes and decisions attached to tasks | Phase 2 |
| Research an answer across institutional knowledge | GovBrain if reachable. Decided at the Phase 2 gate | Phase 2 or later |
| Call systems that have APIs | MCP gateway tools, alongside replays | Phase 3 |

A job counts as served only if the officer's total effort falls, including review, correction and support. A faster first draft with longer correction time does not count.

## 4. Goals and non-goals

### Goals

- Remove the operating burden for repeated tasks in agency systems, starting with one workflow where the gain is measurable end to end.
- Prove or disprove the behavioural bet cheaply: that officers record once and return to replay, before committing to a full build.
- Turn individual know-how into a team asset through a shared, versioned task library.
- Keep governance complete and out of the officer's way: approvals only at consequential steps, with the audit trail underneath.
- Give the officer's work memory, starting with the tasks themselves.

### Non-goals

- Not a permanent second Desk. The drafting jobs are an interim build, consolidated with Desk or a shared Harness agent once value is proven.
- Not a general chatbot or a destination workspace with dashboards and panels. The interface is one input and a quiet presence.
- Not a new model. Models are reached through a cleared route.
- Not a rebuild of Harness, GovBrain or Muninn. The companion builds the thinnest interim version it needs behind an adapter, reuses inner-source code where access is granted, and moves to the shared service when it is reachable.
- Not robotic process automation (RPA) rebuilt. Fixed scripts that break when the interface changes are the failure this design avoids (Section 12).
- Not autonomous action without accountability. Consequential steps need the named officer's approval: the agent prepares and the named officer decides.
- Not continuous screen watching. Capture happens only while the officer records, and is event-based.
- Not a whole-of-government launch. No scale claim is made until Phase 1 evidence exists.

## 5. Product principles

The principles are ordered. In a direct conflict, an earlier principle overrides a later one.

1. **Companion, not destination.** The companion lives in the menu bar or system tray, is dormant when idle, and opens to a blank canvas with one input. No dashboard, no panels, no workspace to learn. If it ever looks like enterprise software, it has failed.
2. **Experience first; sovereignty underneath.** Officers adopt tools because the work disappears, not because the tool is sovereign. The companion runs on approved government infrastructure, but sovereignty is never the pitch and never the first screen.
3. **Governance stays out of the officer's way.** Rules and audit are always on. The officer sees an approval only at a consequential step and never manages governance to get work done.
4. **Designed for the slow laptop officers actually have.** Event capture instead of video, reasoning in the cloud, waits on verified state instead of fixed delays, and near-zero cost when idle. Testing starts on a fast SEED MacBook. The design target is the slower machine.
5. **Prove value before integrating.** When a shared service is not reachable in time, build the thinnest version behind an adapter and plan the consolidation. Never let an integration queue delay the evidence.
6. **Record once, replay many times.** The unit of value is a recording that replays reliably. Until Phase 2 is complete, a feature is in scope only if it makes recording easier, replay more reliable or the library more useful. The one exception is drafting and summarising, which give the officer a reason to open the companion daily.
7. **The officer teaches it.** The companion does not arrive knowing the agency's work. The officer teaches it by doing the task once, normally. Teaching takes no longer than doing the task, plus starting and stopping the recording.

## 6. User stories

Each story ends in an outcome the team can check in Phase 1.

**First open.** As a new officer, I open the companion and see one input and nothing else, so that I can start without training or setup.
Done when: the officer can say what the companion does after one use, and completes a first request within two minutes of first opening it. Target confirmed at the Phase 1 gate.

**Record a task.** As an officer, I press record, do a task the normal way, and press stop, so that the companion learns the task without me writing instructions or prompts.
Done when: recording adds no steps inside the task. On stop, the officer reviews a readable list of the captured steps, marks which typed values change each time, and saves.

**Replay a task.** As an officer, I ask for a task I recorded before and watch it run in the real application, so that the clicks happen without me.
Done when: the replay checks each step before acting, reports progress in plain language, and completes or stops safely. It never clicks through an unexpected screen.

**Run a task over a list.** As an officer, I point a recorded task at a spreadsheet of cases and let it work through them, so that forty cases take minutes of my attention instead of an afternoon.
Done when: each row runs as one replay; the officer approves each consequential step; the run stops at the first row that diverges; and after a stop or crash, no row is submitted twice.

**Approve a consequential step.** As an officer, when a replay reaches a step that submits, sends, deletes or otherwise commits something, the companion pauses and shows me exactly what it will do, so that I remain the decision-maker.
Done when: nothing consequential runs without my explicit approval. My approval, and what I approved, appear in the audit trail.

**Draft or summarise.** As an officer, I describe what I need or drop in a document, and get a draft or a summary, so that I do not open Desk or Pair for the same job.
Done when: the officer completes the job in the companion, and the content goes only to the cleared model route.

**Share a recorded task with a team.** As an officer, I share a recorded task with my team, so that colleagues can replay it without recording it themselves.
Done when: a teammate can find, review and replay the shared task. The library shows who recorded it, when it last passed verification, and which application version it targets.

**Hand over to a successor.** As a departing officer, I hand over my task library and the notes attached to it, so that my successor starts with my recorded tasks and context instead of rebuilding from zero.
Done when: the successor replays the handed-over tasks in their first week and can see the notes and decisions attached to them. This is the Phase 2 test, not a Phase 1 promise.

## 7. Functional requirements

"Must" means required for the phase noted. "Should" means required before scale beyond the pilot team. `docs/td.md` describes how each component is built.

### 7.1 Companion client

The thin desktop client is what the officer uses. It runs on macOS first (SEED test hardware) and is built to port to the managed Windows environment without an architecture change.

- **C-1 Must (Phase 1).** Menu-bar or tray presence, dormant when idle: no screen capture, polling loop or background model work while idle.
- **C-2 Must (Phase 1).** One input on a blank canvas when opened. Chat history and the officer's tasks are reachable from that one view, not from separate modules.
- **C-3 Must (Phase 1).** A live progress view while a replay runs, so the officer can watch, pause or stop at any point.
- **C-4 Must (Phase 1).** One-action stop. Stopping a replay leaves the target application in a safe, inspectable state and reports what was and was not completed.
- **C-5 Should.** Keyboard invocation and return, so the companion is faster to reach than a browser tab.
- **C-6 Should (Phase 2).** A mobile companion for approvals and notifications during long list runs. It does not drive applications: mobile operating systems do not allow one app to control another.

### 7.2 Recorder

The recorder turns doing into instructions. Capture is event-based (clicks, keystrokes, field changes and application state changes), never continuous video.

- **R-1 Must (Phase 0).** Start and stop recording from the companion, with a visible recording indicator for the whole duration. No silent capture.
- **R-2 Must (Phase 0).** Capture an ordered, readable step list: the target element, the action, the input the value fills, and the state that confirms the step landed. Typed values become named inputs. The recording stores the input name, not the value.
- **R-3 Must (Phase 1).** Exclude password fields and flagged sensitive fields at capture time, before anything is written or leaves the device.
- **R-4 Must (Phase 1).** On stop, show a review screen: the steps in plain language, the inputs each run needs, any step the recorder is unsure about, and which steps commit something. The officer corrects, removes or marks steps.
- **R-5 Must (Phase 1).** Save the reviewed recording as a versioned task with author, date, target application and preconditions.
- **R-6 Must (Phase 1).** Pre-flag likely consequential steps (submit, send, delete, approve). A step the recorder cannot classify is treated as consequential until the officer marks it otherwise.

### 7.3 Replay executor

The executor is a native desktop component. It runs locally, acts only through the target application's user interface, and asks the cloud model for help only when the screen diverges from the recording.

- **E-1 Must (Phase 0).** Execute a recorded task step by step, verifying that the expected element or state exists before acting on it. Fixed delays are not a substitute for verification.
- **E-2 Must (Phase 1).** Recover from expected variation (loading delays, reordered lists, extra confirmation dialogs) by re-reading the screen and asking the model for the next action, within the bounds of the recording.
- **E-3 Must (Phase 1).** Stop and ask the officer when the screen diverges materially from the recording, rather than improvising a consequential action.
- **E-4 Must (Phase 0 minimal, Phase 1 full).** Respect the approval gate: consequential steps never execute without the approval flow in Section 7.6.
- **E-5 Must (Phase 1).** Run a task over a list of rows from a spreadsheet: one replay per row, approval for each item, stop at the first divergence, resume from a chosen row, and never re-submit a row whose outcome is unknown.
- **E-6 Should.** Keep a replay log (steps taken, checks passed, divergences handled) attached to the task's audit record.
- **E-7 Should (after the reliability bar is met).** Approve a batch preview once instead of each item, with a capped batch size and a stop at the first divergence.

### 7.4 Orchestrator

The orchestrator receives each request and chooses how to serve it. It starts thin and grows.

- **O-1 Must (Phase 1).** Route each request to one path, chat or a recorded task, and say which path it took when the officer asks.
- **O-2 Must (Phase 1).** Ask a clarifying question when a replay's required inputs are missing, instead of guessing values.
- **O-3 Should.** Suggest a recording when the officer describes a manual task the companion cannot yet do: "Do it once and I will learn it."
- **O-4 Must (Phase 3).** Assemble each turn from instructions, the officer's tasks and notes, MCP tools and the conversation, following the Harness pattern (Appendix B), and combine knowledge, API calls and replays in one task.

### 7.5 Task library

The library is where value builds over time.

- **L-1 Must (Phase 1 single-user on the device; Phase 2 shared).** Store tasks with owner, team, target application, version, last-verified date and replay history.
- **L-2 Must (Phase 2).** Share a task with a named team. Recipients review the step list before they can replay it.
- **L-3 Must (Phase 2).** Version tasks. When a target interface changes, mark affected tasks as needing verification and route re-recording to the owner or a nominated teammate (Section 13).
- **L-4 Should.** Show reuse: how many officers have replayed a task and when it last succeeded.

### 7.6 Approval experience and audit trail

Approval is the trust mechanism: the agent prepares and the named officer decides.

- **A-1 Must (Phase 1).** Before a consequential step, show what will happen, in which system, with what data, and what cannot be undone. Then wait for explicit approval or rejection. In a list run, approval is for each item.
- **A-2 Must (Phase 1).** Write every approval, rejection, replay start and replay outcome to an append-only audit trail linked to the named officer and the task version.
- **A-3 Must (Phase 1).** Keep the companion's own audit trail and agent registration in a shape that moves to Assistant Desk governance when it is reachable. Do not build a second governance product.
- **A-4 Must (Phase 1).** Make the audit trail retrievable by the owning team and by auditors without showing it to the officer as day-to-day workflow.

### 7.7 Memory

- **M-1 Must (Phase 2).** Attach notes and decisions to tasks, so context travels with the task.
- **M-2 Must (Phase 2).** Handover: an officer transfers their tasks and attached notes to a named successor, and the transfer is recorded in the audit trail.
- **M-3 Must (Phase 2).** Memory is scoped to the officer and their team, and never shared across agencies by default.
- **M-4 Should (Phase 3).** Wider memory of the officer (preferences, live cases, conversation context), on Muninn if it is reachable and covers human work.

### 7.8 Model access

- **P-1 Must (Phase 1).** Every model call goes through one adapter, so the route (Bedrock in GCC, the Harness or another cleared route) changes by configuration.
- **P-2 Must (Phase 1).** The route is cleared for the classification of the data sent. Nothing above the route's clearance is sent.
- **P-3 Should.** Move to the Harness route when it is accessible.

Phase 0 needs no model: replay of a recorded task on the stand-in app is deterministic.

### 7.9 Knowledge

- **K-1 Must (Phase 2 gate decision).** Decide whether research over institutional knowledge is in scope, through GovBrain if reachable. The companion does not build its own index of agency knowledge before that decision.

## 8. Architecture and integrations

The architecture follows a pattern proven inside GovTech. In the Employee Digital HR Concierge, the officer talks through Desk, the agent runs on the GovTech Harness, and downstream systems are reached through an MCP gateway. The companion uses the same shape with a desktop client in front and a second kind of doing beside the gateway.

### Three things the companion does

1. **Chat**: drafting and summarising through the cleared model route.
2. **Recorded replays**: for systems with no API. The recording is the plan, the local executor does the clicking, and the model helps only when the screen diverges.
3. **MCP tools** (Phase 3): for systems with APIs, called the way other Harness agents call them.

```
officer ── one input ──┬── chat ─────────────── model adapter ── cleared route (Bedrock in GCC or Harness)
                       │
                       └── task ── executor (local, accessibility tree, checks each step)
                                     │   └── on divergence only ── model adapter
                                     ├── approval gate ── audit trail (append-only)
                                     └── task library (device first, shared store in Phase 2)
```

### How a recorded task becomes a tool

1. The officer records the task (Section 7.2) and reviews the step list.
2. On save, the companion stores the recording as a task: name, description, named inputs, steps, checks for each step, and which steps are consequential. The format is specified in `docs/td.md`.
3. The orchestrator can offer the task alongside chat (Phase 1) and MCP tools (Phase 3). Running it passes the inputs to the local executor, because only that machine can see and drive the target application.
4. The executor streams progress and check results, triggers the approval gate where needed, and ends with a summary and an audit record.

Authoring a tool no longer needs an engineer or an API. Doing the task once is the authoring. That lets the library grow at the speed of the team's work rather than at the speed of an integration backlog.

### Build now, consolidate later

| Component | Phase 0 to 2 | Consolidation target |
|---|---|---|
| Recorder | Build (GRL) | Stays GRL |
| Replay executor | Build (GRL) | Stays GRL |
| Task library | Build (GRL), on the device, then a companion-owned shared store | Muninn, if it fits |
| Approval experience | Build (GRL) | Stays GRL; governance moves underneath |
| Orchestrator | Build (GRL), thin | Harness agent |
| Drafting and summarising | Build (GRL), interim | Desk or a shared Harness agent |
| Model access | Integrate: a cleared route behind one adapter | Harness routing |
| Audit trail and agent registration | Build (GRL), minimal and append-only | Assistant Desk governance |
| Memory beyond tasks | Not built before Phase 3 | Muninn |
| Institutional knowledge | Not built before the Phase 2 decision | GovBrain |
| MCP gateway | Not used before Phase 3 | MCP gateway |

Where inner-source code for Harness, GovBrain or Muninn is accessible, the companion reuses it instead of writing its own. No plan depends on that access until it is granted.

## 9. Constraints

- **SEED first.** Testing starts on a SEED MacBook. The managed COMET laptop needs security clearance that is not yet in hand, so no Phase 0 milestone and no part of the Phase 1 build depends on COMET.
- **Permissions are a gate, in Phase 1.** Recording and replay need accessibility permission (and input monitoring for recording) on macOS. On SEED the tester grants them for Phase 0. The pilot officer's device needs an approved path before the pilot window starts. The request starts once the Phase 0 tape exists, and uses the tape as evidence.
- **Slow-laptop performance rules.** Event-based capture only, never continuous video. Reasoning runs in the cloud; the client executes and reports state. Steps wait for verified state, never for fixed durations. The client is dormant when idle. Breaking a rule needs a written exception in `docs/decisions/`.
- **Data.** Recordings store steps and named inputs, not values. Phase 0 uses synthetic data only. During a live run, data goes only to a model route cleared for its classification. Screenshots are never stored. The companion acts only with the officer's own permissions in the target application and does not widen what the officer can reach.
- **No heavy approval lifecycle to show value.** The first evidence, a replay tape and one officer's return behaviour, exists before any whole-of-government process is engaged.
- **Capacity.** GRL capacity is one primary product plus up to two side products, and the team is at that cap. Adopting this companion as a primary product requires the bet decision in Appendix A. The build is sized for one pilot, possibly with a second.

## 10. Success metrics and kill criteria

Metrics are set on partner access, approvals and adoption, not on build progress. Lines of code, features shipped and demo count are not success measures. Numerical bars marked as targets are set at the gate noted.

| Signal | What counts as success | Measured how |
|---|---|---|
| Replay reliability on stand-in apps | Replay succeeds at or above the agreed bar across perturbations (delays, reordered lists, extra dialogs), failed runs included | Reliability harness in Phase 0. Bar set at the Phase 0 gate |
| Partner access | A named agency team gives one officer and one real workflow | Signed-up pilot before the Phase 1 pilot window starts |
| Permissions | Recording and replay permissions approved for the pilot officer's device | Written approval, or a dated decision with conditions |
| Replay reliability on the real workflow | Replay succeeds at or above the agreed bar, failed runs included | Replay log over an agreed run count |
| Complete-workflow gain | The officer's total time (doing, reviewing, correcting) falls versus doing the task by hand | Timed comparison on the same workflow, including correction time |
| Adoption | The pilot officer returns unprompted to replay tasks, without being chased. Then a teammate replays a shared task | Replay usage log plus weekly check-in. Chat use is tracked separately and does not count. Stated enthusiasm does not count |
| Library reuse (Phase 2) | Tasks recorded by one officer are replayed by another | Library reuse count. Target set at the Phase 2 gate |

### Kill and narrowing criteria

- The pilot officer does not return unprompted to replay within the agreed Phase 1 window, despite the concierge service removing setup friction. Returning only to chat does not count.
- Replay success on the real workflow stays below the agreed bar after the Phase 1 recovery work, or correction time erases the time saved.
- Recording or accessibility permissions are refused for the pilot officer's device, with no workable alternative.
- No agency provides a named officer and a real workflow by the end of Phase 1 build. Partner access failing is itself the finding.
- The interim builds (drafting, audit, library store) take so much capacity that replay work stalls. Narrow to recorder plus replay and re-decide.

Stopping is a valid outcome. What survives a stop: a tested account of where replay breaks on real government applications, the recorded-task format, the approval pattern for consequential steps, and evidence on whether officers will teach an assistant by doing. Each transfers to the Legacy Exoskeleton work and to any future Harness agent that must act in systems without APIs.

## 11. Phased rollout

| Phase | Scope | Exit gate |
|---|---|---|
| Phase 0: prove the hands | One task, recorded and replayed on a SEED MacBook against a stand-in app built from a DXD workflow shape, with synthetic data. A minimal approval stop before the submit step. Video tape as the artefact. Reliability harness on the stand-in apps. No model, no server. | Tape shows a verified replay. Replay bar for Phase 1 agreed. Task format v1 written down |
| Phase 1: prove someone cares | Build, in order: (1) record, review and replay one item, with the approval gate and audit trail; (2) chat with drafting and summarising on the cleared model route; (3) replay over a list with approval for each item; (4) recovery from expected variation. Then the pilot window: one real workflow, one officer, concierge-style. | Officer returns unprompted to replay. Complete-workflow gain measured. Permissions path for the officer's device approved. Kill criteria checked |
| Phase 2: prove it builds up | Shared, versioned task library. Notes attached to tasks. Handover. Mobile approvals. Decision on institutional-knowledge research. | A teammate replays another officer's task. Handover test passed with a real successor |
| Phase 3: the full product, consolidated | Orchestrator combining chat, replays, MCP tools and knowledge. Drafting consolidated with Desk or a Harness agent, memory on Muninn if it fits, governance on Assistant Desk. | Transfer to an enduring owner (GovTech central product team or an agency team), or scale agency by agency |

Each phase ends in a decision, not just a delivery: continue, narrow or stop, judged against Section 10. Phase 3 is the GRL transfer point. GRL's mandate ends with a complete product experiment handed to an enduring owner.

## 12. Risks

| Risk | If it happens | Mitigation in this design |
|---|---|---|
| Recording brittleness: the RPA graveyard | Interfaces change, popups appear, data differs. Replays break and trust collapses | Elements found by role and label, not screen position. Checks on every step. Model recovery within the recording's bounds. Stop and ask on material divergence. Versioning and re-verification in the library |
| Permissions refused or delayed on officers' laptops | The product works on SEED but never reaches officers' real laptops | Request starts once the Phase 0 tape exists. The thin client needs no screen-recording permission unless the vision fallback is used. Gate before the pilot window, not after |
| The interim build becomes permanent | Drafting, audit and the library store never consolidate. GRL ends up running a second Desk | Every interim component sits behind an adapter. Consolidation is part of the Phase 3 exit |
| Desk turf | The companion is read as a Desk replacement and cooperation from the Desk team is lost | Brief the Desk team before 7 Dec. Position the drafting jobs as interim, with consolidation into Desk or a shared Harness agent as the stated end |
| Owning governance | A companion-built audit trail or identity check falls short in security review | Keep it minimal: an append-only audit trail and the officer's existing sign-in. Line-by-line human review. Move to Assistant Desk when reachable |
| Sensitive data in recordings or model calls | A recording keeps data it should not, or a model call sends data above the route's clearance | Recordings store named inputs, not values. Password fields excluded at capture. Live data goes only to a cleared route. Screenshots never stored |
| Stand-in apps flatter replay | Replay works on the stand-in apps and fails on the real workflow | The real-workflow reliability bar is the Phase 1 gate. Stand-in results never count as Phase 1 evidence |
| Solo capacity | One pilot cannot carry the client, executor, recorder, chat, library and audit | Strict build order. Chat stays thin. Research and wider memory are deferred. Narrowing criterion in Section 10 |
| Model and Harness dependency | Route limits or roadmap changes constrain the product | One model adapter. Phase 0 needs no model |

## 13. Open questions and decisions needed

Owners and dates are assigned on 7 December.

1. **First officer and workflow.** Which agency, workflow and officer for the Phase 1 pilot window? The workflow must pass the four tests in Section 2. This blocks the pilot window, not the build. A delay here is partner-access evidence, not a scheduling detail.
2. **Model route.** Which route is cleared for the data the pilot workflow touches: Bedrock in GCC, the Harness, or another? Who owns the clearance, and up to which classification? This blocks the Phase 1 chat and recovery work.
3. **Inner-source access.** Can GRL read and reuse the Harness, GovBrain and Muninn code? Does Muninn's memory model cover human work (decisions, tasks) or only agent work? Until access is granted, the plan assumes none.
4. **Permissions path for the pilot officer's device.** What is the approval path for accessibility and input-monitoring permissions on COMET or the officer's managed device? Who owns it, and what evidence does it need? The Phase 0 tape is meant to serve as that evidence.
5. **Desk consolidation.** Who on the Desk team owns the conversation about consolidating the companion's drafting jobs, and what is the target: Desk itself, or a shared Harness agent both products call?
6. **Who re-records when the interface changes.** When a target interface changes, affected tasks are flagged (L-3). Who re-records: the original author, a teammate or the product team? What happens to a task whose author has left? The pilot team's working agreement must name it.

A seventh decision, the relationship to the Legacy Exoskeleton bet, sits in Appendix A with a recommendation.

## Appendix A: Relationship to Legacy Exoskeleton and the GRL portfolio

### The companion is the Exoskeleton's officer-facing product

The Legacy Exoskeleton bet holds that agents should drive legacy systems through the user interface and expose the result cleanly, with School Cockpit named as a first target. The companion does not compete with that bet. It is the product an officer adopts: the officer talks to the companion, and through recorded replays it acts inside the officer's real applications. Exoskeleton describes the capability. The companion describes the product.

Recommendation for 7 December: fold the two into one bet, with the companion as the product and replay as its core capability. Running them as parallel tracks would split the same capacity and confuse the same partners. Keeping Exoskeleton as a separate infrastructure bet is defensible only if a second team owns it.

### Adjacent products

| Existing product | Relationship |
|---|---|
| Desk and Pair | Desk remains the workspace and Pair the chat assistant. The companion's drafting jobs are interim and consolidate with Desk or a shared Harness agent |
| GovTech Harness | Target runtime for the orchestrator and model routing. Reused as inner-source where accessible |
| Muninn | Candidate store for the task library and wider memory, subject to Section 13 question 3 |
| GovBrain | Candidate for institutional-knowledge research, decided at the Phase 2 gate |
| Transcribe X | Meeting decisions could feed task notes later. Not a Phase 1 dependency |
| Agentic AOR and Prelude | No functional overlap. Shared Harness and observability patterns only |
| Agent observability | Replay logs and audit records use the portfolio's observability tooling where it fits |

## Appendix B: Market and reference notes

The capability this PRD depends on is no longer speculative. Tencent's WorkBuddy entered public beta for Computer Use on 20 November 2026: a workplace agent that reads an application's screen and clicks, types and scrolls inside it, with no API or connector required. Alibaba (QwenWork) and ByteDance (Doubao Work) made similar moves. Tencent's caveats match this design: recognition quality varies by application, so reliability for each application is the real work; and for sensitive actions the user is told to stay present and verify the result, which is the approval pattern in Section 7.6.

A Katonic white paper dated 21 November 2026 makes the matching governance point for government settings: agents work when the agent prepares and a named officer decides. That sentence is the companion's accountability model.

Inside GovTech, the Employee Digital HR Concierge is the reference implementation for the Harness pattern: an officer-facing interface, an agent on the GovTech Harness, downstream systems reached through an MCP gateway, and each turn assembled from instructions, user profile, available tools and conversation history. The companion adds the desktop client and the recorded replay.

For readers new to the products: Desk is GovTech's AI workspace for drafting, deciding and delivering, with model routing by task and workflow automation, available to every officer. Pair is the chat assistant, with about 98,500 monthly users and clearance for data up to Restricted / Sensitive Normal. The Assistant Desk suite adds governance around agents: a registry of who owns each agent and what it does, customisable rules, sanctioned tools, hygiene checks, and controls such as blocking file deletion and external email.
