<!-- Plain-text extract of docs/prd.docx (canonical). Tables are tab-separated. -->

GIANT ROBOT LAB  •  GOVTECH SINGAPORE
Desktop AI Companion
Product Requirements Document
A companion that watches, learns, remembers and does — so officers spend their time on judgment, not on operating software.

Field	Detail
Working title	Desktop AI Companion. Product name is to be decided separately — this document does not propose a name.
Author	Shin, Pilot, Giant Robot Lab (GRL)
Date	24 November 2026
Status	Draft for the GRL Year End Review & Planning on 7 December 2026
Quadrant	Better Mousetrap — high impact, established technology. Judged on the complete workflow, not on model novelty.
Decision sought on 7 Dec	Approve Phase 0 and Phase 1, confirm the relationship to the Legacy Exoskeleton bet, and assign owners for the Muninn scope and COMET clearance questions in Section 13.

# Executive summary
Officers do judgment work, but they spend their days operating software: copying data between systems that do not talk to each other and re-keying the same information across legacy applications that have no application programming interface (API). Pair and Desk have made thinking and writing faster. They have not touched the operating burden, and they start every session with no memory of the officer's work.
The Desktop AI Companion is a thin desktop companion — always present but quiet, opened to a blank canvas with a single input — that closes that gap in two ways. First, it covers the jobs Desk already serves (drafting, summarising, research, reasoning and routine workflow help) through the same GovTech Harness and routed models, so the officer does not need two tools. Second, it does the operating work Desk leaves behind: the officer records a task once by doing it normally, and the companion replays it next time by driving the real application through its user interface, verifying each step and pausing for a named officer's approval before anything consequential.
Nothing in this proposal rebuilds what GovTech already has. The brain runs on the GovTech Harness, systems with APIs are reached through the Model Context Protocol (MCP) gateway, context comes from GovBrain, memory and the shared task library sit on Muninn, and governance comes from the Assistant Desk layer. What GRL builds is the gap: the recorder client, the replay executor, the task library, the approval experience, and the orchestrator that chooses between answering, calling a tool, and replaying a recording.
The test is deliberately small. Phase 0 is a single recorded task replayed on a SEED MacBook, captured on video. Phase 1 is one workflow with one real officer, run concierge-style. The product continues only if that officer returns unprompted and the replay holds up on the real workflow — not because the build is progressing. All design decisions optimise for the laggy managed laptop officers actually use, even though testing starts on SEED hardware.
Positioning: Positioning throughout this document is “do what Desk can't”, not “replace Desk”. The companion covers Desk's jobs through the shared brain; it does not rebuild Desk's workspace.
# Contents
1  Problem statement
2  Target users
3  Jobs-to-be-done
4  Goals and non-goals
5  Product principles
6  User stories
7  Functional requirements
8  Architecture and integrations
9  Constraints
10  Success metrics and kill criteria
11  Phased rollout
12  Risks
13  Open questions and decisions needed
Appendix A  Relationship to Legacy Exoskeleton and the GRL portfolio
Appendix B  Market and reference notes
# 1  Problem statement
## Officers operate software instead of exercising judgment
The core problem is operating burden. A large share of an officer's day is spent moving information by hand: copying values from one system into another, re-keying the same case details across modules of a legacy application, checking multiple screens to assemble one answer, and repeating the same sequence of clicks for every case. The systems involved often have no API and no integration path, so the officer is the integration layer.
Chat assistants do not solve this. Pair, GovTech's chat assistant, and Desk, GovTech's AI workspace, help an officer draft, summarise and reason. The officer then still has to go and do everything themselves, in the applications, by hand. The draft got faster; the thirty clicks after it did not change.
## Work has no memory
The second problem is that work restarts from zero. Assistants start each session without knowledge of the officer's live cases, decisions, contacts or prior tasks. When an officer leaves, their successor rebuilds context from threads, documents and handover notes. Recorded know-how — how a task is actually done in a specific agency system — lives in individuals, not in a shared, reusable form.
## What is verified and what is still assumed
This PRD is explicit about its evidence basis, in line with GRL's rule that stated intent is not adoption. The table below separates what the author verified first-hand at the Ministry of Education's Digital Experience Division (DXD) from what remains assumed until Phase 1.
Claim	Basis	Status
Officers re-key and copy data across legacy systems with no APIs	Author's direct work on school-facing systems at MOE DXD, including School Cockpit workflows and role / account provisioning processes	Verified from first-hand experience
A good draft does not remove the follow-on operating work	Same experience: drafting help still left the in-application steps to the officer	Verified from first-hand experience
Handovers lose context and successors rebuild from scratch	Author's own handover portfolio at DXD in November 2026	Verified from first-hand experience
Officers prefer consumer AI tools over sovereign tools on experience grounds	Observed behaviour reported in team discussion; no survey or usage data cited in this draft	Assumed — to be tested in Phase 1 interviews
Operating burden is the top pain across agencies, not only at MOE	Single-agency experience only	Assumed — to be tested with the pilot agency
Officers will record tasks and reuse each other's recordings	No test yet; this is the central behavioural bet	Unproven — Phase 1 exists to test it

Phase 1 is designed to convert the assumed rows into evidence before any scale decision. No claim in this document should be read as a whole-of-government finding.
# 2  Target users
The primary user is a Singapore public officer whose work involves repeated, structured tasks inside one or more agency systems — case processing, records lookup, data entry, status checking, and routine submissions — alongside the drafting and summarising work they already do with Pair or Desk.
## Who first
One agency, one workflow, one officer. The pilot workflow must meet four tests: it is repeated often enough that recording it pays back quickly; it runs in a real agency application, ideally one without an API; the steps are stable enough to replay; and a named officer owns the outcome and can approve consequential steps. The specific agency, workflow and officer are an open decision for 7 December (see Section 13). Candidates from the author's MOE experience, including School Cockpit workflows, are illustrative only and are not committed in this draft.
The pilot officer is not a test subject for a demo. In Phase 1 they receive a concierge service: the team does the work with them, watches where the value actually appears, and only then hardens the product.
## Who later
the pilot officer's immediate team, who gain the shared task library without recording anything themselves;
other teams in the same agency working in the same applications, where recorded tasks transfer with little change;
other agencies, only where a recorded task or the pattern behind it transfers — the GRL thesis applies: build deeply for one agency, abstract the core, then customise for the next.
Out of scope as first users: officers whose work is primarily unstructured deliberation with no repeated application tasks, and teams whose systems already expose complete APIs and automation. The companion still serves their drafting and research jobs through the shared brain, but they are not the adoption wedge.
# 3  Jobs-to-be-done
If the companion is the tool the officer opens instead of Desk, it must cover Desk's jobs. It does so through the shared brain — the same Harness and routed models — not by rebuilding Desk. On top of those, it owns the operating jobs no current GovTech assistant does.
Job	How it is served	New or inherited
Draft a document, email or submission	Harness agent with routed models; GovBrain for source context	Inherited via shared brain
Summarise a long document or thread	Harness agent with routed models	Inherited via shared brain
Research an answer across institutional knowledge	GovBrain retrieval, cited in the reply	Inherited via shared brain
Reason through a decision or options	Harness agent; conversation memory from Muninn	Inherited, deepened by memory
Automate a routine workflow	Recorded replay, or MCP tool call where an API exists	New — this is the wedge
Build a helper for a team task	Record and share a task; the recording is the helper	New form of an inherited job
Operate a legacy application end to end	Replay executor driving the real user interface	New — only this product
Carry context across sessions and handovers	Muninn memory and task library	New — only this product

The test for every job is the complete workflow, including review, correction, support and operating effort — the Better Mousetrap standard. A job counts as served only if the officer's total effort falls, not if the first draft arrives faster while correction time grows.
# 4  Goals and non-goals
## Goals
Remove the operating burden for repeated tasks in agency systems, starting with one workflow where the gain is measurable end to end.
Give the officer's work memory: the companion remembers the officer, their tasks and their decisions across sessions.
Turn individual know-how into a team asset through a shared, versioned task library that compounds with use.
Keep governance complete but invisible: registry, rules and audit run underneath, and the officer experiences them as trust, not friction.
Prove or disprove the behavioural bet cheaply — that officers will record once and return to replay — before committing to a full build.
## Non-goals
Not a Desk replacement — the companion covers Desk's jobs through the shared brain and does not rebuild or replace Desk's workspace.
Not a general chatbot or a new destination workspace with dashboards and panels. The surface is a companion: one input, quiet presence.
Not a new model, a new harness, or a new retrieval system. Models, Harness, GovBrain and Muninn are integrated, not rebuilt.
Not robotic process automation (RPA) rebuilt. Fixed scripts that break when the interface changes are the failure mode this design explicitly avoids (see Section 12).
Not autonomous action without accountability. Consequential steps require a named officer's approval, following the pattern that the agent prepares and the named officer decides.
Not a whole-of-government launch. No scale claim is made until Phase 1 evidence exists.
# 5  Product principles
When trade-offs arise, these principles decide. They are ordered: an earlier principle overrides a later one in a direct conflict.
Companion, not destination. The companion lives in the menu bar or system tray, is dormant when idle, and opens to a blank canvas with one input. No dashboard, no panels, no workspace to learn. If it ever looks like enterprise software, it has lost.
Experience leads; sovereignty rides along. Officers adopt tools because the work disappears, not because the tool is sovereign. The companion runs on approved GovTech infrastructure, so sovereignty is satisfied underneath — but it is never the pitch and never the first screen.
Governance is invisible. Registry, rules, guardrails and audit are always on and never surfaced as workflow. The officer feels the result — safe, reversible, accountable action — without operating the governance. If the officer ever has to manage governance to get work done, the product has failed.
Designed for the laggy laptop officers actually have. Every decision assumes a slow, managed laptop: event capture instead of video, reasoning in the cloud, waits based on verification instead of fixed delays, and near-zero cost when idle. Testing starts on a fast SEED MacBook; the design target is the slower machine.
Record once, replay forever. The unit of value is a recording that can be replayed reliably. A feature that does not make recording easier, replay more reliable, or the library more useful is out of scope until Phase 2 is complete.
The officer teaches it. The assistant does not arrive knowing the agency's work. The officer teaches it by doing the task once, normally. Teaching must take no longer than doing the task itself, plus starting and stopping the recording.
# 6  User stories
Stories are written from the officer's point of view. Each ends in an observable outcome the team can check in Phase 1.
First open.  As a new officer, I open the companion and see one input and nothing else, so that I can start without training or setup.
Done when: The officer can state what the companion does after one use, and completes a first request in under two minutes from first open. Target to be confirmed at the Phase 1 gate.
Ask a question.  As an officer, I ask a question in plain language and get an answer grounded in my agency's documents, so that I do not have to search multiple systems myself.
Done when: The answer cites its GovBrain sources, and the officer does not need to open Desk or Pair for the same job.
Record a task.  As an officer, I press record, do a task the normal way, and press stop, so that the companion learns the task without me writing instructions or prompts.
Done when: Recording adds no steps inside the task itself. On stop, the officer can review a readable summary of what was captured before saving it.
Replay a task.  As an officer, I ask for a task I recorded before and watch it run in the real application, so that the clicks happen without me.
Done when: The replay verifies each step before acting, reports progress in plain language, and completes or stops safely — it never clicks blindly through an unexpected screen.
Approve a consequential step.  As an officer, when a replay reaches a step that submits, sends, deletes or otherwise commits something, the companion pauses and shows me exactly what it will do, so that I remain the decision-maker.
Done when: Nothing consequential runs without my explicit approval. My approval, and what I approved, appear in the audit trail.
Share a recorded task with a team.  As an officer, I share a recorded task with my team, so that colleagues can replay it without recording it themselves.
Done when: A teammate can find, review and replay the shared task. The library shows who recorded it, when it was last verified, and which application version it targets.
Hand over to a successor.  As a departing officer, I hand over my task library and work memory, so that my successor starts with my recorded tasks and context instead of rebuilding from zero.
Done when: The successor can replay the handed-over tasks in their first week and can see the decisions and context attached to them. This story is the Phase 2 test, not a Phase 1 promise.
# 7  Functional requirements
Requirements are grouped by component. “Must” means required for the phase noted; “Should” means required before scale beyond the pilot team. No requirement in this section asks for a new model, harness or retrieval system.
## 7.1  Companion client
The thin desktop client is the product surface. It runs on macOS first (SEED test hardware) and is designed for porting to the managed Windows environment without an architecture change.
C-1 Must (Phase 1).  Menu-bar / tray presence, dormant when idle, with no continuous screen capture, polling loop or background model work.
C-2 Must (Phase 1).  Single-input blank canvas on open. Conversation history and the officer's task library are reachable from that one surface, not from separate modules.
C-3 Must (Phase 1).  Picture-in-picture or equivalent live progress view while a replay runs, so the officer can watch, pause or stop at any point.
C-4 Must (Phase 1).  One-action stop. Stopping a replay leaves the target application in a safe, inspectable state and reports what was and was not completed.
C-5 Should.  Keyboard-first invocation and return, so the companion is faster to reach than opening a browser tab.
## 7.2  Recorder
The recorder turns doing into instructions. Capture is event-based — clicks, keystrokes, field values and application state changes — never continuous video.
R-1 Must (Phase 0).  Start and stop recording from the companion with a visible recording indicator for the full duration. No silent capture.
R-2 Must (Phase 0).  Capture an ordered, human-readable step list: target element, action, value entered, and the state that confirms the step landed.
R-3 Must (Phase 1).  Redact or exclude password fields and flagged sensitive fields at capture time, before anything leaves the device.
R-4 Must (Phase 1).  On stop, present a review screen: the steps in plain language, the data each step needs next time, and any step the recorder is unsure about, for the officer to correct or remove.
R-5 Must (Phase 1).  Save the reviewed recording as a versioned task in the library, with author, date, target application and preconditions.
## 7.3  Replay executor (computer use)
The executor is the hands. It runs locally, acts only through the target application's user interface, and takes its reasoning from the cloud orchestrator.
E-1 Must (Phase 0).  Execute a recorded task step by step, verifying that the expected element or state exists before acting on it. Fixed delays are not an acceptable substitute for verification.
E-2 Must (Phase 1).  Recover from expected variation — loading delays, reordered lists, additional confirmation dialogs — by re-reading the screen and asking the orchestrator for the next action, within the bounds of the recording.
E-3 Must (Phase 1).  Stop and ask the officer when the screen diverges materially from the recording, rather than improvising a consequential action.
E-4 Must (Phase 1).  Respect the approval gate: consequential steps (submit, send, delete, pay, approve) never execute without the approval flow in Section 7.6.
E-5 Should.  Record a replay log — steps taken, verifications passed, divergences handled — attached to the task's audit record.
## 7.4  Orchestrator
The orchestrator is the agent on the GovTech Harness that receives every request and chooses how to serve it.
O-1 Must (Phase 1).  Route each request to exactly one primary path: answer directly, call an MCP tool, or replay a recorded task — and say which path it took, in plain language, when the officer asks.
O-2 Must (Phase 3 for full form; a minimal router is Phase 1).  Assemble each turn from its instructions, the officer's profile and memory, the available MCP tools, the officer's task library, and the conversation, following the proven Harness pattern (see Appendix B).
O-3 Must (Phase 1).  Ask a clarifying question when a replay's required inputs are missing, instead of guessing values.
O-4 Should.  Suggest a recording when the officer describes or repeats a manual task the companion cannot yet do: “Do it once and I will learn it.”
## 7.5  Task library
The library is where value compounds. It is backed by Muninn (see Section 7.7) and is the shared asset a team builds together.
L-1 Must (Phase 2; a single-user store is Phase 1).  Store tasks with owner, team, target application, version, last-verified date and replay history.
L-2 Must (Phase 2).  Share a task with a named team; recipients can review the step list before they can replay it.
L-3 Must (Phase 2).  Version tasks. When a target interface changes, mark affected tasks as needing verification and route re-recording to the owner or a nominated teammate (see Section 13).
L-4 Should.  Show reuse: how many officers have replayed a task and when it last succeeded, so useful tasks surface without curation overhead.
## 7.6  Approval experience and audit trail
Approval is the trust mechanism. It follows the pattern that the agent prepares and the named officer decides.
A-1 Must (Phase 1).  Before a consequential step, present what will happen, in which system, with what data, and what cannot be undone — then wait for explicit approval or rejection.
A-2 Must (Phase 1).  Write every approval, rejection, replay start and replay outcome to an audit trail linked to the named officer and the task version.
A-3 Must (Phase 1).  Plug into Assistant Desk governance — agent registry entry, customisable rules, and hard controls such as blocking file deletion and external email — rather than implementing a parallel governance system.
A-4 Must (Phase 1).  Make the audit trail retrievable by the owning team and by auditors without exposing it as day-to-day workflow to the officer.
## 7.7  Memory (Muninn)
Muninn provides memory from agent work. The companion needs memory of the officer's work as well; the gap between the two is an open question (Section 13), not an assumption in this PRD.
M-1 Must (Phase 2).  Remember the officer across sessions: preferences, live tasks, prior decisions and conversation context, retrievable by the orchestrator at the start of each turn.
M-2 Must (Phase 2).  Store the task library in or through Muninn if its service interface supports it; otherwise store tasks in a companion-owned store with a documented migration path (decision in Section 13).
M-3 Must (Phase 2).  Support handover: an officer can transfer their tasks and relevant memory to a named successor, with the transfer recorded in the audit trail.
M-4 Must (Phase 2).  Respect data boundaries: memory is scoped to the officer and their team, and is never shared across agencies by default.
## 7.8  Knowledge (GovBrain)
GovBrain provides institutional knowledge retrieval. The companion consumes it; it does not build a second index.
K-1 Must (Phase 1).  Retrieve relevant policy and reference material while answering questions and while preparing a replay that depends on current rules, and cite sources in answers.
K-2 Should.  Surface staleness: when retrieved material has been superseded, say so rather than presenting it as current.
K-3 Must (Phase 3).  Make retrieval available to the orchestrator as a standard tool alongside MCP tools and replays, so a single task can combine knowledge, API calls and interface actions.
# 8  Architecture and integrations
The architecture follows a pattern already proven inside GovTech. In the Employee Digital HR Concierge, the officer talks through Desk, the agent runs on the GovTech Harness, reasons with a large language model, and reaches downstream systems through an MCP gateway — assembling each turn from its instructions, the user profile, the available tools and the conversation, then calling tools in a loop until the task is done. The companion uses the same shape, with a desktop client in front of it and a second kind of doing beside the gateway.
## Two kinds of doing
The orchestrator has two ways to act on the world, and the distinction is the product:
MCP tools — for systems that expose tools through the MCP gateway. Reliable, structured, and already governed. The companion calls these exactly as other Harness agents do.
Recorded replays — for systems with no API. The recording is the plan; the local executor is the hands; the orchestrator supervises, verifies and recovers. No other product in the current stack covers this half.
A single officer request can combine both: retrieve context from GovBrain, call an MCP tool for the system that has one, then replay a recording for the system that does not, pausing for approval before the consequential step in either path.
## How a recorded task becomes a tool
1.  The officer records the task (Section 7.2) and reviews the step list.
2.  On save, the companion packages the recording as a tool definition: name, description, required inputs, step plan, verification conditions, and which steps are consequential.
3.  The tool is registered so the orchestrator can discover and invoke it alongside MCP gateway tools. Invocation passes the required inputs; the orchestrator delegates execution to the local replay executor on the officer's machine, because only that machine can see and drive the target application.
4.  The executor streams progress and verification results back. The orchestrator handles divergence, triggers the approval gate where required, and closes the turn with a summary and an audit record.
The practical consequence: authoring a tool no longer requires an engineer or an API. Doing the task once, well, is the authoring act. That is what makes the library able to grow at the speed of the team's work rather than at the speed of an integration backlog.
## Integrate versus build
Component	Integrate or build	Notes
Officer surface (chat entry)	Integrate	Requests can also arrive via Desk where that is the agency's habit; the companion client remains the primary surface for recording and replay
GovTech Harness (agent runtime)	Integrate	The orchestrator runs here; no new runtime
Models (routed via Harness)	Integrate	Same routing and data classifications as existing assistants, including Restricted data handling via approved routes
MCP gateway	Integrate	For systems with APIs; recorded tasks are presented to the orchestrator in the same tool shape
GovBrain	Integrate	Retrieval and citation; no second index
Muninn	Integrate, scope TBC	Memory and task library substrate, subject to the Section 13 scope decision
Assistant Desk governance	Integrate	Registry, rules, guardrails, hygiene checks and audit plumbing
Recorder client	Build — GRL	Thin, event-based desktop capture; the core new client component
Replay executor (computer use)	Build — GRL	Local execution with per-step verification and recovery; the core technical risk
Task library	Build — GRL	Versioning, sharing and verification state, on Muninn where possible
Approval experience	Build — GRL	Per-step, named-officer approval in the companion surface; governance underneath is integrated
Orchestrator	Build — GRL	The routing agent on Harness: answer, tool call, or replay

# 9  Constraints
SEED first. Testing starts on a SEED MacBook. The managed COMET laptop environment requires security clearance that is not yet in hand, so no Phase 0 or Phase 1 milestone may depend on COMET access.
Permissions are a gate. Screen-recording and accessibility permissions are required for both recording and replay. On government devices these permissions may need explicit approval. Confirming the approval path is a Phase 0 exit item, not a Phase 3 surprise. If permissions are refused for the pilot environment, the kill criteria in Section 10 apply.
Laggy-laptop performance rules. Event-based capture only; no continuous video. Reasoning runs in the cloud; the client executes and reports state. Steps wait for verified state, never for fixed durations. The client is dormant when idle. Any design that violates one of these rules needs a written exception approved by the product owner.
No heavy approval lifecycle to show value. GRL products show value through a partner's real workflow, not through a lengthy approval lifecycle. The pilot is structured so the first evidence — a replay tape and one officer's return behaviour — exists before any whole-of-government process is engaged.
Data and access. The companion inherits the data classifications and tool restrictions of the Harness and Assistant Desk layers. It does not expand what data an officer can access, and it acts only with the officer's own permissions in the target application.
Capacity. GRL capacity is one primary product plus up to two side products, and the team is at that cap. Adopting this companion as a primary product requires the bet decision in Appendix A.
# 10  Success metrics and kill criteria
Metrics are set on partner access, approvals and adoption signal — not on build progress. Lines of code, features shipped and demo count are not success measures for this product. Numerical bars marked as targets are to be set at the Phase 0 / Phase 1 gate with the pilot agency; this draft does not invent them.
Signal	What counts as success	Measured how
Partner access	A named agency team gives one officer and one real workflow for Phase 1	Signed-up pilot with named officer and workflow before Phase 1 build starts
Permissions	Recording and replay permissions approved for the pilot environment	Written approval, or a dated decision with conditions
Replay reliability	Replay succeeds on the real workflow at or above the agreed bar, failed cases included in the count	Replay log over an agreed run count; target bar set at the Phase 0 gate
Complete-workflow gain	Officer's total time — doing, reviewing, correcting — falls versus doing the task manually	Timed comparison on the same workflow, manual versus replay, including correction time
Adoption signal	Pilot officer returns unprompted and replays without being chased; then a teammate replays a shared task	Usage log plus weekly check-in; stated enthusiasm does not count
Library compounding (Phase 2)	Tasks recorded by one officer are replayed by another	Library reuse count; target set at the Phase 2 gate

## Kill and narrowing criteria
The pilot officer does not return unprompted within the agreed Phase 1 window, despite the concierge service removing setup friction.
Replay success on the real workflow stays below the agreed bar after the hybrid recovery work in Phase 1, or correction time erases the time saved.
Recording or accessibility permissions are refused for the pilot environment with no viable alternative surface.
No agency will provide a named officer and a real workflow — partner access fails, which is itself the finding.
Muninn cannot support officer-work memory and the adapter cost would consume the product's capacity. In that case narrow to recorder plus replay without cross-session memory, and re-decide.
Stopping is a valid outcome. What survives a stop: a tested account of where replay breaks on real government applications, the recorded-task format and tool-packaging approach, the approval pattern for consequential steps, and evidence about whether officers will teach an assistant by doing. Each of these transfers directly to the Legacy Exoskeleton work and to any future Harness agent that needs to act in systems without APIs.
# 11  Phased rollout
Phase	Scope	Exit gate
Phase 0 — Prove the hands	One task, recorded and replayed on a SEED MacBook. Screen-recorded tape as the artefact. Permissions path investigated in parallel.	Tape shows a verified replay of a real-shaped task; replay bar for Phase 1 agreed; permissions path named
Phase 1 — Prove someone cares	Recorder plus replay for one workflow, one real officer, concierge-style for an agreed window. Minimal orchestrator router. Approval gate live.	Officer returns unprompted; complete-workflow gain measured; kill criteria checked
Phase 2 — Prove it compounds	Muninn memory and the shared, versioned task library. Team sharing and handover stories delivered.	A teammate replays another officer's task; handover test passed with a real successor
Phase 3 — The full product	Full orchestrator combining answers, MCP tools, GovBrain retrieval and replays, with Assistant Desk governance fully integrated and invisible.	Decision to transfer to an enduring owner (GovTech central product team or agency team) or to scale agency by agency

Each phase ends in a decision, not just a delivery: continue, narrow, or stop, judged against Section 10. Phase 3 is also the GRL transfer point — GRL's mandate ends with a complete product experiment handed to an enduring owner, not with GRL operating a whole-of-government service indefinitely.
# 12  Risks
Risk	If it happens	Mitigation in this design
Recording brittleness — the RPA graveyard	Interfaces change, popups appear, data differs; replays break and trust collapses	Hybrid execution: recording is the plan, the agent verifies and recovers per step; versioning and re-verification state in the library; stop-and-ask on material divergence
COMET permissions refused or delayed	Product works on SEED but never reaches officers' real laptops	Permissions as a Phase 0 gate; thin client with minimal footprint to make the security case; SEED evidence used to support the clearance request
Desk turf	Companion read as a Desk replacement; cooperation from the Desk team is lost	Positioning is “do what Desk can't”; Desk jobs served via the shared brain, not a rebuilt workspace; Desk surface integration kept open
Scope creep into rebuilding Desk	Team rebuilds workspace features and never ships the operating wedge	Non-goals in Section 4; principles in Section 5; every feature tested against “does it improve recording, replay or the library?” until Phase 2 ends
Governance felt as friction	Approvals and rules interrupt flow; officers route around the tool with consumer AI	Governance integrated underneath; approval only at consequential steps; friction treated as a defect measured in Phase 1
Model and Harness dependency	Harness routing, limits or roadmap changes constrain the product	Orchestrator kept thin and portable in its logic; dependency owned explicitly with the Harness team from Phase 1
Sensitive data in recordings	A recording captures data it should not retain	Redaction at capture (R-3); recordings store steps and field references, with data handling inherited from Harness classifications; audit of library contents

# 13  Open questions and decisions needed
Five questions block or shape Phase 1. Owners and dates are to be assigned on 7 December.
1.  First workflow and first officer. Which agency, which workflow, which officer? The workflow must pass the four tests in Section 2. Without a named answer, Phase 1 cannot start — and that delay is partner-access evidence, not a scheduling detail.
2.  Muninn's scope. Does Muninn expose a service other products can write to and read from, and does its memory model cover human work — an officer's decisions, threads and tasks — or only agent work such as code and pull requests? If it generalises, the companion is a thin integration. If it does not, the gap is a memory adapter, costed before Phase 2. Owner: Muninn product owner, to be confirmed.
3.  Surface: standalone companion, with Desk as an entry point. This PRD proposes a standalone desktop companion as the primary surface, because recording and replay require a desktop client that a chat surface cannot provide. Requests may also arrive via Desk. Confirming this avoids building two surfaces by accident.
4.  COMET clearance path. What is the approval path for screen-recording and accessibility permissions first on SEED, then on COMET, who owns it, and what evidence does it require? Phase 0's tape is intended to serve as that evidence.
5.  Who re-records when the interface changes. When a target interface changes, affected tasks are flagged as needing verification (L-3). Who re-records — the original author, a teammate, or the product team — and what happens to a task whose author has left? The handover story (Section 6) is the default answer, but the pilot team's working agreement must name it.
See also: A sixth decision — the relationship to the Legacy Exoskeleton bet — is material enough to sit in Appendix A with a recommendation, rather than in this list.
# Appendix A  Relationship to Legacy Exoskeleton and the GRL portfolio
## The companion is the Exoskeleton's interface
The existing Legacy Exoskeleton bet holds that agents should drive legacy systems through the user interface and expose the result cleanly, with School Cockpit named as a first target. This companion does not compete with that bet; it is its officer-facing form. The officer talks to the companion, the companion remembers the officer's work, and — through recorded replays — it acts inside the officer's real applications. Exoskeleton describes the capability; the companion describes the product an officer actually adopts.
Recommendation for 7 December: fold the two into one bet, with the companion as the product and replay as its core capability, rather than running them as parallel tracks that split the same capacity and confuse the same partners. The alternative — keeping Exoskeleton as a separate infrastructure bet — is defensible only if a second team owns it. Running both inside the current cap (one primary plus up to two side products, already full) is not.
## Adjacencies — integrate, do not duplicate
Existing product	Relationship
Muninn	Memory substrate for the companion and its task library; scope decision in Section 13
GovBrain	Retrieval for answers and for rule-dependent replays; no second index
Transcribe X	Meeting capture can feed memory — decisions made in meetings become retrievable context — as a later integration, not a Phase 1 dependency
Agentic AOR and Prelude	No functional overlap; shared Harness and observability patterns only
Agent observability	Replay logs and audit records should use the portfolio's observability tooling where it fits, rather than a companion-only stack
Desk and Pair	Shared brain and models; Desk remains the workspace, Pair the chat assistant. The companion owns presence, memory and operating work

# Appendix B  Market and reference notes
The capability this PRD depends on is no longer speculative. Tencent's WorkBuddy entered public beta for Computer Use on 20 November 2026 — a workplace agent that reads an application's screen and clicks, types and scrolls inside it, with no API or connector required — following similar moves by Alibaba (QwenWork) and ByteDance (Doubao Work). Tencent's own caveats are instructive and are reflected in this design: recognition quality varies by application, so per-application reliability is the real work; and for sensitive actions the user is told to stay present and verify the result, which is the approval pattern in Section 7.6.
A Katonic white paper dated 21 November 2026 makes the complementary governance point for government settings: agents work when the agent prepares and a named officer decides. That sentence is the companion's accountability model.
Inside GovTech, the Employee Digital HR Concierge is the reference implementation for the Harness pattern this PRD builds on: officer-facing surface, agent on the GovTech Harness, downstream systems reached through an MCP gateway, with each turn assembled from instructions, user profile, available tools and conversation history. The companion's contribution on top of that pattern is the desktop client and the recorded replay — the second kind of doing.
Desk and Pair context, for readers new to the landscape: Desk is GovTech's AI workspace, positioned around drafting, deciding and delivering, with model routing by task and workflow automation, available to every officer. Pair is the chat assistant, with approximately 98,500 monthly users and clearance for data up to Restricted / Sensitive Normal. The Assistant Desk suite adds the governance layer around agents: a registry of who owns each agent and what it does, customisable rules, sanctioned tools and hygiene checks, and controls such as blocking file deletion and external email. The companion is designed to sit inside that landscape, not beside it.

—  End of draft  —