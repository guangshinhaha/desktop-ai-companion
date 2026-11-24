# Spike: reading spreadsheet cells through the accessibility API (#11)

Status: spike note, 24 Nov 2026. Refs TD section 4 (`read` input source) and the Phase 0 task.
Merges no product code. Scripts are in `spike/`.

## Read this first: Excel was not tested

Excel for Mac is not installed on the test Mac. Only Numbers 15 (`com.apple.iWork.Numbers`) was. Nothing below shows that Excel exposes cells the same way. The first Done-when item (Excel for Mac through `AXUIElement`) is **not met**. It needs a Mac with Excel, so it is a `hands-on` follow-up (see "Untested").

## Scope change (Shin, via the coordinator)

Excel is out of the measured set. The measured set is Numbers (through AX) and the on-disk file read. Google Sheets was added as a target, because Google Drive and Workspace is the more relevant one for officers. Google Sheets was tested in Dia (not Chrome): see "Google Sheets in Dia" below.

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

## Google Sheets in Dia (tested, 24 Nov 2026)

Shin pointed the test at Dia (Dia.app, `company.thebrowser.dia`, Chromium-based), already signed in to Google. Chrome was not used. Account details, the Drive and other tabs were not read. Scripts: `spike/dia_lib.swift` (shared helpers; concatenate with a script to compile), `spike/dia_dump.swift`, `spike/dia_sheets_sub.swift`, `spike/dia_sheets_read.swift`.

What was done: `open -a Dia https://sheets.new` opened one new sheet; it was titled `spike-11-synthetic`, filled with the four columns of `stand-in/data/staff.csv` only (tab-separated text on the clipboard, Edit > Paste pressed by AX), read, then moved to the Drive trash (see "Cleanup"). Screen unlocked (`CGSSessionScreenIsLocked=0`).

### What AX exposes in Sheets

1. **The tree needs waking, like Numbers.** First walk of Dia's window with no write: 57 elements, no `AXWebArea` (only browser chrome). After `AXManualAccessibility = true` on the app (returned success for Dia; `AXEnhancedUserInterface` returned `-25208` again) the page's web area appears with the whole DOM (`AXWebArea` titled "... - Google Sheets", ids such as `docs-chrome`, `waffle-grid-container`). Only the visible tab of a window has a web area, so the sheet tab must be selected first (AXPress on its `AXTabButton` works).
2. **The cells are not in the tree.** The grid under `waffle-grid-container` has about 80 `AXGroup` and 3 empty `AXStaticText`, no `AXCell`, no `AXTable`, no text for any cell. Sheets draws the grid itself. Searching the tree for "Aiden" found nothing. A link "Turn on screen reader support" (`docs-a11y-aria-enable`) is present; I focused it and pressed Return by key event, and the link disappeared, but the grid still had no cell text. I did not confirm that Sheets' screen-reader mode is really on, so "needs screen-reader mode" is **not** established either way.
3. **What is exposed: the active cell.** The Name box is an `AXTextField` (`t-name-box`) whose `AXValue` is the selection in A1 form (`A1:D6` after the paste, `B3`, `D6`). The formula bar is an `AXTextArea` (inside `t-formula-bar-input`) whose `AXValue` is the active cell's text. So the `B3` address is exposed natively and needs no row/column arithmetic, unlike Numbers.
4. **Selection.** Reading it works (Name box plus formula bar). Setting it through pure AX did not work: `AXValue` on the Name box plus `AXFocused` needs a Return key event (`CGEvent.postToPid`) to commit. Sheets' own menu bar items (File, Tools) are in the tree but `AXPress` does not open them (no menu contents appear), so Sheets menus are unreachable by AX alone.
5. **Staleness.** The formula bar lags the Name box after a selection change. In the first 5x4 pass (fixed 20 ms wait) 6 of 20 reads returned the previous cell. With polling until the text changes, one read (D2) still did not match the CSV (see next point). A reader must wait for verified state, not a sleep.
6. **My own side effect.** The sheet ended with D2 holding `bella.example@staff.example.test` and a tall row 2, which differs from the CSV. I believe my Name-box/Return navigation wrote it in the first, unsettled pass (the Return went to a cell rather than the Name box); I did not trace it. So 19 of 20 values matched the CSV, and the mismatch is real data in the sheet, not only a stale read. It is a warning: driving selection with key events can edit cells.
7. Not tested: formulas, number formats, merged or hidden cells, a mouse click selection, `AXObserver` notifications, large sheets, scrolling beyond the viewport, other sheet tabs.

### Timings (Dia, one run each unless stated)

| Step | Time |
|---|---|
| Locate the web area (BFS) | 2.8 to 4.2 ms |
| Locate Name box and formula bar | 25 to 37 ms |
| Read selected cell (Name box + formula bar) | 0.26 ms first, 0.02 ms with handles cached |
| Select one cell (set Name box, Return, wait until it commits) | 7 to 65 ms |
| 5x4 read (A2:D6), navigate and poll per cell | 254 ms total, 12.7 ms per cell |
| 5x4 read, fixed 20 ms wait (unreliable) | 750 ms with stale reads |

The 5x4 read is a 20-step navigate-and-read, not one bulk call. Only the selected cell is readable, so cost scales with cells, and every step can edit the sheet if the focus is wrong.

### Cleanup

The sheet was moved to the Drive trash through Sheets' menu search ("Move to trash"); Sheets showed "File moved to trash". It stays in the trash for 30 days unless Shin empties it. Nothing else was created. I pressed no sign-in or account control. The tab was left on that dialog. The clipboard was overwritten with the synthetic TSV (it held 18 bytes of text before; not restored). A second window holding only the sheet tab appeared in Dia during the run.

### Reading for the pick

Sheets in Dia is workable for exactly one thing: the selected cell, address and text, in well under 100 ms once awake. It is not workable for list runs or any bulk read. That matches the pick below. Dia and Chrome share the engine, so Chrome is expected to behave the same, but that was not tested.

## The pick

**Read the file for list runs and for any `read` source that names a file. Use AX only for the one thing the file cannot give: the cell the officer has selected in the open app.**

Reasons, from the measurements:

- The file read is two orders of magnitude simpler to make reliable: no app has to be running, no lazy tree to wake, no permission beyond file access, and it works with the app closed. AX needed an attribute write that reports an error, a retry until cells appear, and its cell text is in `AXTitle`, an undocumented use that may differ in Excel.
- Both are fast enough, so the AX cost is reliability and portability, not time.
- AX reflects unsaved edits and the current selection; the file does not. An officer who copies a cell they have just edited, or "the highlighted cell", needs AX. This is an expectation risk, not a speed one.
- AX on Excel is unproven (see below), so the design must not depend on it.
- Google Sheets in Dia (tested) agrees: only the selected cell is readable (Name box gives `B3`, formula bar gives the text), the grid cells are not in the tree, and reads can go stale. It confirms "AX for the selected cell only, best effort". Bulk reads of a Sheet need an export or the Sheets API, not AX; that is a later call.

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

- **Google Sheets in Chrome** (Dia was tested, not Chrome), Sheets screen-reader mode, bulk reads, and observer notifications.
- **Excel for Mac, all of it** (now out of the measured set by Shin's call; Excel was not tested). Whether it exposes `AXTable` or `AXCell` at all, where the text sits (`AXValue` or `AXTitle`), whether it gives the A1 address (Excel has a Name Box, an `AXTextField` that may carry it), selection, and the timing. Needs Excel on a Mac. Hands-on for Shin or a Mac with Office. Rerun `spike/ax_dump.swift` and `spike/ax_select.swift` with the bundle id `com.microsoft.Excel`, which only needs that string changed.
- Whether the tree wake-up is required for Excel, and for Numbers on other macOS versions.
- A real mouse click or keyboard selection in Numbers, and `AXObserver` selection notifications.
- Large sheets: this file has 6 rows. AX time grows with cells read; Excel with 10,000 rows is untested. Use `AXVisibleCells` and the parameterised `AXCellForColumnAndRow` (the call returned success, the value was not inspected).
- A managed-laptop (SEED) policy that blocks Accessibility for non-approved apps.
- Windows (out of scope here).

Issue #11 stays open: its first Done-when item (Excel) is unmet. The PR uses `Refs #11`.
