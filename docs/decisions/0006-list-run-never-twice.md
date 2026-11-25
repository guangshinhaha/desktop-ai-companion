# 0006: List runs: CSV only, and how a row is never submitted twice

Status: built for #33 (`risk:high`: approval gate, audit trail, a never-twice guarantee); line-by-line review pending. Refs #33, TD section 6.

## Decisions

- **CSV only.** Excel is out of scope (Shin closed #11 as "Numbers and Sheets only"). An `.xlsx` or `.xls` file is refused with "save the sheet as CSV". The XLSX line of #33's Done when is therefore not met; the PR uses `Refs #33`.
- **Values are not stored.** `runs.inputs_source` holds the file's SHA-256, its row count and the column mapping (column names only). A resume must be given the same file again; a different file is refused, because row numbers would no longer mean the same people.
- **Rows are checked before the run starts.** Every value must be non-empty, at most 256 characters and, for a choice input, one of its choices. A line break inside a value is refused. The message names the row and input, never the value.
- **Never twice.** `committing` is written when the executor is about to record `approved`, and the executor acts only after that record is written. A row found `committing` when a run is picked up, or ended by anything but `finished`, is `unknown`. A run does not continue while any row is `unknown` or `diverged`. Only the officer clears one, and that is an `item_marked_by_officer` record, written before the change. Every state change is a compare-and-set in the store.
- **Rejected rows do not stop the run.** The officer said no to that row; the next row asks again. Closing the card is a no, so closing every card rejects every row, and the run ends with them all `rejected`.

## Limits

- One list run at a time per shell. Two shells on one store are not guarded beyond the per-row compare-and-set.
- The `approved` record and `committing` are two writes. If the shell dies between them the row is `committing` with no record: it becomes `unknown`, never silently retried.
- Whether a row really was created is the officer's judgement when it is `unknown`; the shell cannot check the target app.
