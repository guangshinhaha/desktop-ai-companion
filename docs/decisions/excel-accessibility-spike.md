# Spike: reading spreadsheet cells through the accessibility API (#11)

Status: spike note, 24 Nov 2026. Refs TD section 4 (`read` input source) and the Phase 0 task.
Merges no product code. Scripts are in `spike/`.

## Read this first: Excel was not tested

Excel for Mac is not installed on the test Mac. Only Numbers 15 (`com.apple.iWork.Numbers`) was. Nothing below shows that Excel exposes cells the same way. The first Done-when item (Excel for Mac through `AXUIElement`) is **not met**. It needs a Mac with Excel, so it is a `hands-on` follow-up (see "Untested").

## Scope change (Shin, via the coordinator)

Excel is out of the measured set. The measured set is Numbers (through AX) and the on-disk file read. Google Sheets in Chrome was added as a target, because Google Drive and Workspace is the more relevant one for officers. **Google Sheets was not tested**: see "Google Sheets in Chrome" below.

## Method

- Mac: the test MacBook (not a SEED laptop), macOS 26 (Darwin 25.6.0), Accessibility and Screen Recording granted to the terminal (`AXIsProcessTrusted() = true`). Screen unlocked throughout (`CGSSessionScreenIsLocked=0`, checked before each run).
- Data: synthetic `stand-in/data/staff.xlsx` and `staff.csv` only (5 staff rows, 4 columns). Nothing left the machine.
- AX side: Swift scripts, compiled with `swiftc -O`, against Numbers with `staff.xlsx` open. Times are wall-clock inside the process (`DispatchTime`), after the table is exposed.
- File side: `spike/read_file.py` (Python stdlib only: `zipfile` + `xml.etree` for xlsx, `csv` for csv). Times are in-process, 200 warm iterations, plus the whole-process time for one cold read.
- Scripts: `spike/ax_dump.swift` (tree dump), `spike/ax_select.swift` (table, cells, selection, timing), `spike/lock.swift` (lock check), `spike/read_file.py`.

## What AX exposes in Numbers

1. **By default, nothing useful.** Right after launch the window tree has toolbar, sheet tabs and a scroll area, but no table and no cells. Zero `AXCell` elements, on repeated walks.
2. **One attribute write wakes the tree.** `AXUIElementSetAttributeValue(app, "AXEnhancedUserInterface", true)` returns an error (`-25208`, `cannotComplete`), yet the next walk shows the table. A second walk then shows cells; the first walk after the write showed rows but no cells. The `AXManualAccessibility` write also errored. This is lazy-tree behaviour and I have not pinned down which step is required. A product build would need to retry until the cells appear.
3. **Once awake**: `AXLayoutArea > AXTable` (title `Table 1, 10 rows, 5 columns`) `> AXRow > AXCell`. Cell text is in **`AXTitle`**, not `AXValue` (`AXValue` is nil on cells). Numbers pads the table to 10 rows, 5 columns; the fifth column and rows 7 to 10 are empty cells.
4. **Cell address**: there is no `A1` string. Each cell has `AXRowIndexRange` and `AXColumnIndexRange` (zero-based `CFRange`); the executor must build `B3` itself.
5. **Selection**: the table has `AXSelectedCells`, and each cell has `AXSelected`. Reading works. In this run I **set** the selection through AX (`AXSelectedCells = [cell]`, err 0) and read it back (count 1, `bella.example@staff.example.test`, row index 2, column index 1). I did not click with the mouse, so selection changed by an officer was not tested, nor was `AXObserver` notification on selection change.
6. Not found: formulas, number formats, and whether Numbers reports a hidden or merged cell. Not tested.

### Real output

Tree before waking (cells counted by `ax_dump | grep -c AXCell`): `0`, `0`.
Tree after the attribute write, second walk: `50` cells.

```
set AXEnhancedUserInterface: -25208
set AXManualAccessibility: -25205
AXTable t=Table 1, 10 rows, 5 columns
  AXRow t=1 d=Row 1:  full_name | email | school | role | (empty)
  AXRow t=2 d=Row 2:  Aiden Tan Example | aiden.example@staff.example.test | Northgate Primary | Teacher | (empty)
```

`ax_select` (one run):

```
locate table: 1.8 ms  title=Table 1, 10 rows, 5 columns
AXSelectedCells: 0.16 ms, count=0            (nothing selected yet)
read all rows via AX: 4.9 ms
["full_name", "email", "school", "role", ""]
["Aiden Tan Example", "aiden.example@staff.example.test", "Northgate Primary", "Teacher", ""]
["Bella Koh Example", "bella.example@staff.example.test", "Riverside Secondary", "Form Teacher", ""]
cell r3c2: title=bella.example@staff.example.test value=nil rowRange=location:2 length:1 colRange=location:1 length:1
set AXSelectedCells err=0 22.81 ms           (first call; 0.16 to 0.25 ms later)
read AXSelectedCells: 0.39 ms count=1
selected: title=bella.example@staff.example.test ... rowRange=location:2 colRange=location:1
```

(The script's "non-empty rows" counter printed 8, but only 6 rows hold data. I did not chase the counter; the printed rows are correct.)

File read (`read_file.py`):

```
B3 = bella.example@staff.example.test
cells: 24 csv rows: 6
xlsx 0.38 ms, csv 0.23 ms (in-process, first call)
xlsx warm x200: median 0.122 ms, p95 0.244 ms
csv warm x200: median 0.028 ms, p95 0.037 ms
whole process, cold, 3 runs: real 0.06 s each (Python start-up dominates)
```

## Timings

| Read | Time | Notes |
|---|---|---|
| AX: locate table | 1.7 to 3.5 ms | 5 runs |
| AX: read all 10 x 5 cells | 3.9 to 6.2 ms | 5 runs, after the tree is awake |
| AX: set then read selection | 0.16 to 0.25 ms set, 0.29 to 0.45 ms read | warm; first set 22.8 ms |
| AX: full tree walk of Numbers incl. menus | 71 to 148 ms | avoid; menus are 700+ elements |
| AX: waking the tree | one write plus up to two walks, not timed precisely | the real cost and the main risk |
| File: xlsx | 0.12 ms median, 0.38 ms first | any cell, any row |
| File: csv | 0.03 ms median, 0.23 ms first | |

Both are far below the 100 ms step budget. Speed does not decide this. What decides it is below.

## Google Sheets in Chrome: untested, needs Shin sign-in

Not measured. On the test Mac, Chrome was running with no open window, so there was no signed-in Google Sheets session to read. Chrome profiles do hold a Chrome sign-in, but I could not tell which account they are, and creating a new sheet writes to a remote Google account. The repo sets `shinstack-mode.shared: ask`, so I did not open a profile or create a sheet. Shin signs in (and confirms the account) before this can run.

Plan once signed in: create a new sheet from `stand-in/data/staff.csv` only (synthetic), read cells from Chrome's accessibility tree (`AXWebArea`, `AXDOMIdentifier`), and time it like Numbers. Expected risk, not measured: Sheets draws its grid on a canvas, so cells may be absent from the AX tree unless Sheets' accessibility mode is on. Until measured, the pick below says nothing about Sheets.

## The pick

**Read the file for list runs and for any `read` source that names a file. Use AX only for the one thing the file cannot give: the cell the officer has selected in the open app.**

Reasons, from the measurements:

- The file read is two orders of magnitude simpler to make reliable: no app has to be running, no lazy tree to wake, no permission beyond file access, and it works with the app closed. AX needed an attribute write that reports an error, a retry until cells appear, and its cell text is in `AXTitle`, an undocumented use that may differ in Excel.
- Both are fast enough, so the AX cost is reliability and portability, not time.
- AX reflects unsaved edits and the current selection; the file does not. An officer who copies a cell they have just edited, or "the highlighted cell", needs AX. This is an expectation risk, not a speed one.
- AX on Excel is unproven (see below), so the design must not depend on it.

## What this changes

TD section 4, `read` input source (`source: read`, "a value read from the screen"):

- Keep `read` as an accessibility read for **web and native form fields**, as designed.
- For a spreadsheet, split it. `column` (list runs, TD section 7) reads the file directly, as TD already says ("read locally"). Add a spreadsheet `read` of the form "the selected cell of app X" as an AX read, marked **best effort, Phase 1 at the earliest**, with the file read as the fallback when the officer names a file.
- The descriptor needs a spreadsheet locator: sheet name plus cell address (`B3`), built from the AX row and column index ranges, not a screen position. The recorder should store the address and not the value (PDPA, per CLAUDE.md Data).
- Recorder: capturing "the officer copied cell B3" needs AX to be awake. Plan for the wake-and-retry step, and for a state of "tree not ready" that is a divergence, not a failure.

Phase 0 task (stand-in provisioning app, synthetic data):

- Use the file read (`staff.csv` or `.xlsx`) for the spreadsheet half of the task. Do not make Phase 0 depend on Excel being installed or on spreadsheet AX. The SEED MacBook may not have Excel in any case.
- If the Phase 0 video should show the officer copying from an open sheet, that is a Phase 1 item and needs the Excel test below first.

## Later

GovSuite is the government version of Word, PowerPoint and Docs. It is to be integrated later and was not tested now.

## Untested, and what would settle it

- **Google Sheets in Chrome, all of it** (see above; needs Shin's sign-in).
- **Excel for Mac, all of it** (now out of the measured set by Shin's call; Excel was not tested). Whether it exposes `AXTable` or `AXCell` at all, where the text sits (`AXValue` or `AXTitle`), whether it gives the A1 address (Excel has a Name Box, an `AXTextField` that may carry it), selection, and the timing. Needs Excel on a Mac. Hands-on for Shin or a Mac with Office. Rerun `spike/ax_dump.swift` and `spike/ax_select.swift` with the bundle id `com.microsoft.Excel`, which only needs that string changed.
- Whether the tree wake-up is required for Excel, and for Numbers on other macOS versions.
- A real mouse click or keyboard selection in Numbers, and `AXObserver` selection notifications.
- Large sheets: this file has 6 rows. AX time grows with cells read; Excel with 10,000 rows is untested. Use `AXVisibleCells` and the parameterised `AXCellForColumnAndRow` (the call returned success, the value was not inspected).
- A managed-laptop (SEED) policy that blocks Accessibility for non-approved apps.
- Windows (out of scope here).

Issue #11 stays open: its first Done-when item (Excel) is unmet. The PR uses `Refs #11`.
