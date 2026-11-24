# 0002: The executor is native and reads the accessibility tree

Status: accepted, 24 Nov 2026, with the open items below. Refs #17, TD sections 3.2 and 5.2, PRD v2 "What changed" item 5, PRD section 12.

## Context

Officer workflows cross applications: a spreadsheet into a web form, an email into a case system. The executor has to see and act on all of them, find elements by what they are (role and label) and not by screen position, and wait on verified state, not fixed sleeps. It runs on managed laptops that may be slow, so it must be dormant when idle.

## Decision

The executor is one native process per operating system. On macOS (Phase 0) it is Swift and reads the UI through the accessibility API (`AXUIElement`, `AXObserver`). It watches input while recording with a listen-only `CGEventTap`. It acts through accessibility actions first (`AXPress`, set `AXValue`) and uses synthetic `CGEvent` input only as a fallback. Windows (after Phase 1) uses UI Automation, with the same task format and RPC protocol.

Browsers go the same route: Chrome, Edge and Safari expose page content through the tree, including the HTML `id` as `AXDOMIdentifier`. The executor wakes a browser's tree by writing `AXEnhancedUserInterface` and `AXManualAccessibility` (`executor-macos/Sources/ExecutorCore/AXTree.swift`).

Elements are found by the resolver (`executor-macos/Sources/ExecutorCore/Resolver.swift`, TD section 5.2): `dom_id` or platform id, then role plus exact label, then role plus fuzzy label scoped by anchors, then the recorded path. Two or more equal matches are a divergence, never a guess. The vision fallback (locator 5) is Phase 1 and optional.

Spreadsheets are the exception. Per the spike below, a list run reads the file directly. Accessibility is used for "the cell the officer has selected", best effort, Phase 1 at the earliest.

## Evidence

**Spreadsheet spike (#11, `docs/decisions/excel-accessibility-spike.md`, PRs #62 and #64).** Excel was NOT tested. Excel for Mac was not installed on the test Mac, so #11's first Done-when item is unmet and #11 stays open. What was tested:

- Numbers 15: AX exposes `AXTable > AXRow > AXCell`, but only after a wake. The attribute write returns an error (`-25208`) and the first walk after it still shows no cells; a second walk shows 50. Cell text is in `AXTitle`, not `AXValue`. There is no `A1` address; the executor builds it from `AXRowIndexRange` and `AXColumnIndexRange`.
- Google Sheets in Dia: the tree also needs waking (`AXManualAccessibility`), but the cells are not in it. Only the selected cell is readable (Name box gives `B3`, formula bar gives the text). The formula bar can lag the Name box, so reads must wait for the new state. Driving selection by key events once edited a cell by accident (19 of 20 values matched the CSV).
- Speed does not decide it. Numbers: read 10 by 5 cells in 3.9 to 6.2 ms once awake. Sheets in Dia: 12.7 ms per cell on a 5 by 4 read. File read: 0.12 ms median for `.xlsx`, 0.03 ms for `.csv`. The step budget is 100 ms. Reliability and portability decide it.

**Replay reliability (`harness/reports/`).** Stand-in provisioning app, synthetic data, 20 runs per perturbation, host `gs-tan-v71f1` on macOS 26.7.1 with Chrome. The reports do not say whether that host is a SEED MacBook; confirm before quoting it as one.

- `2026-11-24-first-report.md` (commit ab17d40): none 14/20 (70%), latency 13/20, reorder 13/20, rename 13/20, confirm 0/20. 0 fails in 100 runs, so no wrong data was submitted. The stops were a replay defect on the select step when the option was already shown (#70), or the confirm dialog.
- `2026-11-24-select-fix-report.md` (after #71): none 19/20 (95%), latency 20/20, reorder 20/20, rename 20/20, confirm 0/20. The one stop in `none` was before step 1: the form was not visible in the tree after 40 s, with a dialog open in the fresh Chrome profile. Cause not investigated. Confirm stops by design; recovery is Phase 1 (#34).
- Median step time is 20 to 36 ms across both reports. It includes the app's response and the approval read, so it is a ceiling on the TD section 10 "step overhead" figure (under 300 ms on SEED), not a measurement of it.

## What is not proven

- Excel for Mac, all of it. Needs Shin or a Mac with Office.
- Google Sheets in Chrome (only Dia was tested), Sheets screen-reader mode, and `AXObserver` notifications on selection change.
- Any real agency app. The numbers are from a stand-in we built. PRD section 12: stand-in results never count as Phase 1 evidence.
- A SEED or managed-laptop policy that blocks Accessibility or Input Monitoring for our app (PRD section 13, question 4).
- Chrome's CPU cost with the full tree built (TD section 15). Not measured yet.
- A replay started the moment Chrome opens cannot see the page, because the tree is woken once at start. The harness waits for the form first. Phase 1 must re-wake the tree or document it.
- Large sheets, formulas, merged or hidden cells.

## Rejected alternatives

- **Browser extension.** Sees only the browser, so it cannot do a spreadsheet into a web form, or any native app (PRD "What changed" item 5; TD section 3.2).
- **Screen-position clicks, or pixel and vision matching, as the base.** Synthetic clicks fail when a window is partly covered or the scale changes, and vision needs Screen Recording permission. Kept only as the optional Phase 1 fallback, off unless an app needs it.
- **Accessibility reads of spreadsheet cells as the main read.** The spike shows a wake-and-retry step, different behaviour per app, and only the selected cell in Sheets. The file read is simpler and more reliable for lists.

## Consequences

- Two executors in the end (Swift now, C# later), kept in step by one task format and RPC protocol.
- macOS permissions (Accessibility, Input Monitoring) gate officers' laptops. The request starts once the Phase 0 tape exists (PRD section 12).
- Chrome and Edge build their full tree when an assistive technology is active, which costs CPU on heavy pages. Measure in Phase 0 and on the pilot app.
- TD section 4 `read` splits for spreadsheets: `column` reads the file; "selected cell of app X" is an AX read.
- Phase 0 does not depend on Excel or on spreadsheet AX.

## Reopen or retire

Reopen if any of these happens:

- The pilot officer's real apps do not expose a usable tree (TD section 15), or the permissions path for the officer's device is refused (PRD section 13, question 4). Then weigh the vision fallback as the base for those apps, or stop.
- Replay reliability on a real workflow misses the Phase 1 gate for tree reasons, not recording or data reasons.
- The Excel test shows Excel is unusable through AX and Excel is the pilot's spreadsheet. The file read still covers lists; the "selected cell" read would drop.
- Chrome's tree cost is unacceptable on SEED or the pilot laptop.

Retire when the Windows executor (UI Automation) is built and a combined record supersedes this one.
