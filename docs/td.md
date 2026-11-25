# Desktop AI Companion: Technical Design

| Field | Detail |
|---|---|
| Author | Shin, Pilot, Giant Robot Lab (GRL) |
| Date | 24 November 2026 |
| Status | Draft v1, for review alongside PRD v2 |
| Product requirements | `docs/prd.md`. Requirement IDs (R-2, E-5 and so on) refer to it |

This document says how the companion is built: the components, the recorded-task format, how replay finds and checks elements, where data goes, and what each phase builds. Choices marked **Recommended** still need a decision record in `docs/decisions/` before code depends on them.

## 1. Design goals

1. **Replay is reliable on slow managed laptops.** Elements are found by what they are (role and label), not where they sit on screen. Every step waits for a verified state.
2. **Stored recordings hold no personal data.** Typed values become named inputs at capture. The recording keeps the name, never the value.
3. **The client costs nothing when idle.** No event taps, observers, timers or model calls while the officer is not recording or replaying.
4. **Every interim component sits behind an adapter.** Model route, task store and audit store each change by configuration, so consolidation with Harness, Muninn or Assistant Desk is a swap, not a rewrite.
5. **One person can build Phase 0 and Phase 1.** Few moving parts, and Phase 0 has no server and no model.

## 2. System overview

```
┌──────────────────────── officer's machine ────────────────────────┐
│                                                                    │
│  shell app (menu bar, one input, progress, approvals)              │
│     │  JSON-RPC over stdio                                         │
│  executor process (native: recorder + replay + resolver)           │
│     │  accessibility API + input events                            │
│  target apps (browser, spreadsheet, mail, agency desktop clients)  │
│                                                                    │
│  local store (SQLite): tasks, run log, audit trail, chat history   │
└────────────────────────────┬───────────────────────────────────────┘
                             │ HTTPS, officer's sign-in (Phase 1)
┌────────────────────────────┴───────────── GCC ─────────────────────┐
│  companion service (thin): model adapter, task sync (Phase 2),     │
│  audit sync, agent registration                                    │
│     │                                                              │
│  cleared model route (Bedrock in GCC, or Harness)                  │
└────────────────────────────────────────────────────────────────────┘
```

The client never holds model credentials. All model calls go through the companion service, which holds the route's credentials and enforces the classification ceiling.

## 3. Components

### 3.1 Shell app

The shell is what the officer sees: the menu-bar icon, the one-input canvas, the replay progress view, the approval card and the review screen.

**Recommended: Tauri v2** (Rust core, web UI). One UI codebase for macOS and Windows. Its idle memory is a fraction of Electron's, which matters on slow laptops. It runs the executor as a sidecar process. Alternatives considered:

- Electron: same cross-platform benefit, but it ships a full Chromium per app. It is too heavy for the target laptop.
- Native SwiftUI, then WinUI later: the best macOS fit, but two UIs to build and maintain for one person.

Phase 0 needs only the recording indicator (R-1) and a terminal approval prompt. The executor draws the indicator itself, so the shell starts in Phase 1.

### 3.2 Executor process

The executor is one native process for each operating system. It contains the recorder, the replay engine and the element resolver, and talks to the shell over JSON-RPC on stdio. The task format and the RPC protocol are the same on both operating systems.

| | macOS (Phase 0) | Windows (after Phase 1) |
|---|---|---|
| Language | Swift | C# (.NET) |
| Read the UI | Accessibility API (`AXUIElement`, `AXObserver`) | UI Automation |
| Watch input while recording | Listen-only `CGEventTap` | Low-level hooks (`SetWindowsHookEx`) |
| Act during replay | Accessibility actions (`AXPress`, set `AXValue`) first; synthetic `CGEvent` input as fallback | UI Automation patterns (`Invoke`, `Value`) first; `SendInput` as fallback |
| Permissions | Accessibility, Input Monitoring. Screen Recording only if the vision fallback is on | Usually none, but managed policy may block hooks |

**Why native and not a browser extension.** Officer workflows cross applications: a spreadsheet into a web form, an email into a case system. A browser extension sees only the browser. The operating system's accessibility tree covers native apps and web pages alike, because Chrome, Edge and Safari expose page content through it, including the HTML `id` as `AXDOMIdentifier`. One executor covers every application the officer uses.

**Why accessibility actions before synthetic clicks.** `AXPress` on a button works wherever the window is, whether it is partly covered, and at any screen scale. A synthetic click at coordinates fails on all three. Synthetic input is the fallback for controls that do not support accessibility actions.

### 3.3 Companion service (Phase 1)

A thin service in GCC. **Recommended:** TypeScript on Node, to match the repo's gantry hooks, deployed as a small container or function.

It has three jobs in Phase 1:

1. **Model adapter**: `POST /chat` (streamed) and `POST /recover`. It holds route credentials, enforces the classification ceiling, and logs each call's metadata, never its content.
2. **Audit sync**: receives the client's audit records and stores them where the owning team and auditors can read them (A-4).
3. **Agent registration**: one record describing the companion, its owner and its tools, shaped to move to the Assistant Desk registry.

Phase 2 adds task sync for the shared library. Sign-in uses the officer's existing government identity. Which provider is an open question (Section 10).

### 3.4 Local store

SQLite on the device, in the user's application-support folder, encrypted at rest with a key in the operating system's keychain.

| Table | Holds |
|---|---|
| `tasks` | Recorded tasks (Section 4), one row per version |
| `runs` | One row per replay or list run: task version, inputs source, status |
| `run_items` | One row per list item: state (Section 6), last step reached |
| `audit` | Append-only, hash-chained records (Section 7) |
| `chat` | Chat history, kept on the device only |

## 4. The recorded-task format

The task format is the product's core asset. Version 1 covers everything up to Phase 2 so later phases add fields without breaking tasks already recorded. It is defined as a JSON Schema in `schema/task.v1.json` (written in Phase 0).

```json
{
  "format": "task/v1",
  "id": "tsk_8f2c",
  "version": 3,
  "name": "Provision a school staff account",
  "description": "Create a staff account and assign a role in the admin portal.",
  "author": "officer-id",
  "created": "2026-12-02T10:14:00+08:00",
  "platform": "macos",
  "apps": [
    { "key": "sheet", "bundle": "com.microsoft.Excel" },
    { "key": "portal", "bundle": "com.google.Chrome", "url": "https://admin.example.gov.sg/*" }
  ],
  "preconditions": [
    { "app": "portal", "check": { "kind": "window_title", "matches": "Admin Portal*" } }
  ],
  "inputs": [
    { "name": "full_name", "type": "text", "source": "ask" },
    { "name": "email", "type": "email", "source": "ask" },
    { "name": "role", "type": "choice", "choices": ["Teacher", "Admin"], "source": "ask" }
  ],
  "steps": [
    {
      "id": "s4",
      "say": "Type the officer's name into Full name",
      "app": "portal",
      "action": "set_value",
      "target": {
        "role": "AXTextField",
        "label": "Full name",
        "dom_id": "fullName",
        "path": ["AXWebArea", "AXGroup:form", "AXTextField:1"],
        "anchors": ["Create staff account"]
      },
      "value": { "input": "full_name" },
      "expect": [{ "kind": "value_equals", "input": "full_name" }],
      "consequential": "no"
    },
    {
      "id": "s9",
      "say": "Press Create account",
      "app": "portal",
      "action": "press",
      "target": { "role": "AXButton", "label": "Create account", "dom_id": "submit" },
      "expect": [{ "kind": "appears", "target": { "role": "AXStaticText", "label_matches": "Account created*" } }],
      "consequential": "yes",
      "approval": { "summary": "Create a staff account for {full_name} ({email}) with role {role}", "undoable": false }
    }
  ],
  "provenance": { "recorded_on_app_versions": { "portal": "Chrome 131" }, "last_verified": "2026-12-05" }
}
```

Rules the format enforces:

- A typed value is always `{ "input": name }` unless the officer marks it as a constant in review. A constant is stored only after the officer confirms it holds no personal data.
- `consequential` is `"yes"`, `"no"` or `"unknown"`. The executor treats `"unknown"` as `"yes"`.
- Every step has at least one `expect` check. A step without one fails schema validation.
- Inputs have a `source`: `ask` (the officer types it at run time), `column` (a list-run column) or `read` (a value read from the screen in an earlier step, for copy-between-systems tasks). A `read` value lives only in memory during the run.

Supported actions in v1: `press`, `set_value`, `select`, `type_keys`, `focus_app`, `open_url`, `read_value`, `wait_for`.

Supported check kinds in v1: `exists`, `appears`, `disappears`, `value_equals`, `window_title`, `focused`.

## 5. Recording and replay

### 5.1 Recording

1. The officer presses record. The executor turns on a listen-only event tap and shows the recording indicator (R-1).
2. For each click or keystroke run, the executor asks the accessibility API which element is under the pointer or focused. It saves an element descriptor: role, label (from the title, description, placeholder or the label element linked to the field), `AXDOMIdentifier` for web content, the path from the window, and nearby label text as anchors.
3. Keystrokes into one field merge into one `set_value` step. The typed text becomes an input slot, named from the field label. The text itself stays in memory only until the officer saves or discards the review.
4. After each action, the executor compares the focused window's accessibility tree before and after, and proposes the most specific stable change as the step's `expect` check. An example is "a static text matching 'Account created' appeared".
5. Password fields (`AXSecureTextField`, or a UI Automation `IsPassword` element) are never read. A step into one becomes a precondition, such as "officer is signed in".
6. The officer presses stop. The event tap turns off. The review screen (R-4) shows each step's `say` line, the inputs, steps flagged `unknown`, and pre-flagged consequential steps (button labels such as Submit, Send, Delete, Approve, Confirm, Create).

### 5.2 Finding an element during replay

The resolver tries locators in order and stops at the first single, confident match:

1. `dom_id` or the platform identifier, if recorded.
2. Role plus exact label, within the expected window.
3. Role plus label, with a fuzzy match, scoped by anchors.
4. The recorded path from the window.
5. Vision fallback (Phase 1, optional): a screenshot of the window and the descriptor are sent to the cleared model route, which returns a point. This needs Screen Recording permission. It stays off unless an app needs it.

Two or more equal matches count as a divergence, never as a guess.

### 5.3 Waiting without fixed delays

The executor waits on events, not timers. Before acting, it subscribes to accessibility notifications on the target app (created, value changed, focus changed, title changed) and re-evaluates the step's checks on each one. Each step has a ceiling, 30 seconds by default and set per step. Reaching the ceiling is a divergence, not a retry.

Some applications do not send notifications reliably. For those apps only, and only while a step is waiting, the executor re-checks with a backoff that starts at 100 ms and stops at 2 s. This is the one place the client polls. It never polls while idle. Record the exception in `docs/decisions/` when the first such app appears.

### 5.4 Divergence and recovery (Phase 1)

When a step's target is not found, or its check fails at the ceiling, the executor builds a recovery request:

- the step's `say` line, action and descriptor;
- a pruned accessibility tree of the focused window: roles, labels and structure, plus field values only if the route is cleared for them (otherwise masked);
- a screenshot, only if the vision fallback is on and the route is cleared. It is never written to disk.

The model returns one of three answers:

- `act`: a non-consequential action on a named element. Examples are dismissing a dialog, scrolling or expanding a section.
- `retry`: wait for the step again.
- `ask_officer`: with a reason.

The executor rejects any `act` that targets a consequential element, types a value other than the step's own input, or leaves the recorded app. After three recovery attempts on one step, it asks the officer.

## 6. List runs (E-5)

A list run executes one task once per row of a spreadsheet (CSV, read locally; Excel is out for now, see decision 0006). The officer maps columns to inputs before the run starts. Each row moves through these states:

```
pending ─▶ running ─▶ awaiting_approval ─▶ committing ─▶ done
              │               │                 │
              ▼               ▼                 ▼
           diverged        rejected          unknown
```

- The executor writes `committing` to `run_items` before it performs a consequential action, and `done` after the action's check passes. This order is what prevents a double submit.
- A crash or stop during `committing` leaves the row `unknown`. On resume, the officer must check that row in the target app and mark it done or not done before the run continues. The executor never re-submits an `unknown` row by itself.
- The run stops at the first `diverged` row by default. The officer can skip the row, fix it by hand, or end the run.
- Built in `shell/src-tauri/src/listrun.rs` (see decision 0006). `skipped` is the officer's answer to a `diverged` row. A row that reached `committing` and then ended any way but `finished` (a divergence, a stop, a dead executor) is `unknown`, not `diverged`: the action may have happened. A rejected row is left as `rejected` and the run goes on.
- A run is held by one lock for as long as it goes (a file next to the store); recovering rows, running and the officer's answers all need it, so a second shell or window cannot act on a live run. A task has one unfinished list run at a time: the same version and file again picks it up, and any other start is refused until the officer resumes or ends it (where a store holds more than one, none starts or continues until all but one are ended). The screen lists unfinished runs after a restart. Continuing from a `diverged` or `rejected` row is recorded as an officer act. Each row has the progress window and Stop (#28). The first row of a file is data until the officer says it holds column names. The run store holds no values; the `shown` text of an `approved` record does (decision 0005).
- Approval is per item in Phase 1. Batch approval (E-7) is a later setting with a capped batch size, and still stops at the first divergence.

## 7. Approval gate and audit trail

### 7.1 Approval

Before a step with `consequential: "yes"` (or `"unknown"`), the executor pauses and the shell shows the approval card: the step's `approval.summary` filled with this run's inputs, the target app, and whether the action can be undone. Only an explicit approve or reject from the officer continues the run. No timeout counts as consent.

### 7.2 Audit records

Each record is appended to the local `audit` table and synced to the companion service when one exists:

```json
{
  "seq": 1042,
  "at": "2026-12-05T15:02:11+08:00",
  "officer": "officer-id",
  "event": "approved",
  "task": "tsk_8f2c@3",
  "run": "run_51",
  "item": 7,
  "step": "s9",
  "shown": "Create a staff account for <full_name> (<email>) with role Teacher",
  "prev_hash": "9c1e…",
  "hash": "4b7a…"
}
```

Events: `run_started`, `approved`, `rejected`, `step_diverged`, `recovered`, `run_stopped`, `run_finished`, `item_marked_by_officer`, `task_saved`, `task_shared`, `task_handed_over`.

Each record carries the hash of the one before it, so a missing or edited record breaks the chain and shows up on verification.

**Decided (decision 0005):** the record stores the values the officer approved, encrypted, in the local audit store only. Retention defaults to 7 years (2555 days), as config. A central copy comes with #29.

How it is built. The shell owns the store and the officer's identity. The executor asks over the stdio JSON-RPC (`audit.append`, `approval.request`) and the shell writes the record through the store's API; the executor cannot set the officer, task, run or time. `approved` is written before the action runs, and a failed write stops the run. `executor-macos verify-audit` reads the shell's JSONL export on stdin and exits 0 when the chain is intact, or 5 naming the sequence number of the first bad or missing record. The `recovered`, `item_marked_by_officer`, `task_shared` and `task_handed_over` events are allowed by the store but have no producer yet.

Progress and stop (#28). The executor sends `run.progress` notifications (`started`, `done`, `paused`, `resumed`, with `step`, `say`, `index`, `total`); the shell draws them in a progress window. The shell sends `replay.pause`, `replay.resume` and `replay.stop` as notifications (no id, no reply), each a whole line written at once. The executor reads them without a second thread: it polls stdin between steps and every 0.1 s while a step waits, so a stop cancels a waiting step within 500 ms and leaves the app as it is. Pause holds between steps; end of input while paused means stop. A stop is written as `run_stopped` (the step it came at, what completed, what did not) and then `run_finished` "stopped", not as a rejection. A stop while an approval card is open answers it no; an approval followed by a stop before the action is not recorded as approved. A stop never cancels an `audit.append` wait (only the officer's question): the record is written, then the run stops. A stop between `approved` and the action is recorded in `run_stopped` as "approval recorded, action not taken". The shell sends a stop pressed before the executor exists only after `replay.start`, and puts up no card once a stop is asked. A pause pressed while a card is open holds the approved step until resume. If the executor has not ended the run 2 s after a stop, the shell ends the process and writes `run_stopped` and `run_finished` itself, saying the executor was ended and naming the step in flight, whose effect is unknown. The stop shortcut (Cmd+Shift+Period) is registered only while a run is on.

Limit: whoever holds the store key can recompute every hash and the control row and forge a whole trail. The central copy (#29) is what closes that.

## 8. Chat (Phase 1)

Chat is deliberately thin: one conversation view, drafting and summarising, and documents dropped in from the officer's machine. The shell sends the conversation to `POST /chat`, and the service streams the model's reply back. History stays on the device. Chat adds no tools, retrieval or memory in Phase 1.

## 9. Data handling

| Data | Where it lives | Leaves the device? |
|---|---|---|
| Recorded task (steps, descriptors, input names) | Local store; shared store in Phase 2 | Yes, to the library store. Holds no values |
| Input values (typed at run time, list rows, `read` values) | Memory during a run | Only to the cleared model route during recovery, and masked unless the route is cleared for them |
| Values the officer approved | Encrypted local audit store (decision 0005), 7 years by default | Central copy later, via #29 |
| Screenshots | Memory only, vision fallback only | Only to the cleared route. Never stored |
| Chat content and dropped documents | Local chat history | To the cleared model route |
| Password fields | Never read | No |

The companion service checks each request against the route's classification ceiling. Each request carries a classification set by the task, or by the officer for chat. A request above the ceiling is refused, never downgraded. Phase 0 uses synthetic data only.

## 10. Performance budget

| State | Allowed | Target |
|---|---|---|
| Idle | Shell process asleep; executor process not running | 0.0% CPU on average over 60 s. Shell memory held at or under 20 MB physical footprint (`footprint <pid>`, canvas closed, no web view process). Set from #24: first Tauri build measured 11 MB on a dev Mac (RSS 61 MB, which counts shared system libraries). Canvas open adds a web content process of about 30 MB RSS and takes the shell to 17 MB footprint. #25 re-measures on a SEED Mac |
| Recording | One listen-only event tap; accessibility queries on each event | No input lag the officer can notice on the target laptop |
| Replay | Accessibility observers on the target app only | Step overhead (resolve plus check, excluding the app's own response time) under 300 ms on SEED. Measured on a slower machine before the pilot |

A known risk: when an assistive technology is active, Chrome and Edge build the full accessibility tree for each page, which costs CPU on heavy pages. Measure it on the stand-in apps in Phase 0, and on the pilot's real application before the pilot window.

## 11. Testing

- **Stand-in apps** (`stand-in/`): small local web apps rebuilt from DXD workflow shapes, with synthetic data. Phase 0 starts with an account-provisioning form fed from a spreadsheet. Each app has perturbation switches: added latency, reordered lists, an extra confirmation dialog, a renamed button.
- **Reliability harness**: replays a task N times for each perturbation and reports the success rate, failed runs included. The Phase 0 replay bar comes from this report.
- **Unit tests**: schema validation, resolver ranking, list-run state transitions (including a crash during `committing`), the audit hash chain, and the classification ceiling.
- **Tape**: the Phase 0 artefact, a screen recording of a verified replay on SEED made with the operating system's own recorder. It is not a feature of the companion.

## 12. What each phase builds

**Phase 0 (no server, no model, no shell):**

1. A stand-in account-provisioning web app and a spreadsheet of 5 synthetic rows.
2. `schema/task.v1.json`, the format in Section 4.
3. A Swift executor command line with `record` and `replay`, the recording indicator, the resolver locators 1 to 4, event-based waits and checks.
4. A minimal approval stop: the executor pauses before a consequential step and asks for `y` or `n` in the terminal.
5. The reliability harness and its first report.
6. The tape.

**Phase 1, in build order:**

1. Shell app; review screen; local store; approval card; audit trail; companion service with audit sync and sign-in.
2. Chat through `POST /chat` on the cleared route.
3. List runs with approval for each item.
4. Recovery through `POST /recover`, and the optional vision fallback.
5. Measure on the pilot's real application, then start the pilot window.

**Phase 2:** task sync and the shared library, versioning and re-verification, task notes, handover, mobile approvals, Windows executor.

## 13. Repository layout

```
executor-macos/     Swift package: recorder, replay, resolver, RPC
shell/              Tauri app (Phase 1)
service/            Companion service, TypeScript (Phase 1)
schema/             task.v1.json and fixtures
stand-in/           Stand-in apps, spreadsheets, perturbation switches
harness/            Reliability harness
docs/               PRD, this TD, decisions/
```

## 14. Decision records to write

1. `0001-interim-drafting-in-companion.md`: build drafting and summarising in the companion; the consolidation target and trigger.
2. `0002-native-accessibility-executor.md`: native executor on the accessibility tree instead of a browser extension.
3. `0003-model-route-and-data-ceiling.md`: the cleared route, the classification ceiling, and what leaves the device during recovery.
4. `0004-tauri-shell.md`: Tauri over Electron and native UIs.
5. `0005-audit-stores-approved-values.md`: the Section 7.2 decision. It needs Shin's review.

## 15. Open technical questions

| Question | How to find out | Blocks |
|---|---|---|
| How good is Excel for Mac's accessibility tree for reading cells? | Spike in week 1 of Phase 0. If it is poor, read the spreadsheet file directly instead of the app | Phase 0 task choice |
| What does Chrome's full accessibility tree cost in CPU on the stand-in apps and the pilot app? | Measure in Phase 0 | Pilot readiness |
| Which identity provider does the companion service use for officer sign-in? | Ask the GCC and identity teams | Phase 1 step 1 |
| Is Bedrock in GCC cleared for the pilot workflow's data, and which models? | PRD Section 13, question 2 | Phase 1 chat and recovery |
| Do the pilot officer's agency apps expose usable accessibility trees? | Inspect them with Accessibility Inspector or Inspect.exe once the workflow is named | Pilot window |
