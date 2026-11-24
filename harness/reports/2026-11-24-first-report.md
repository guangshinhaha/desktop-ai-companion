# Replay reliability report: Provision a staff account (stand-in)

- Date: 2026-11-24
- Task file: `executor-macos/Tests/fixtures/replay/provision-task.json`, replayed against the stand-in provisioning app (synthetic data only)
- Runs per perturbation: 20
- Executor: executor-macos debug build, repo commit ab17d40
- Host: gs-tan-v71f1, macOS 26.7.1, Chrome with a fresh isolated profile per run
- Step ceiling: executor default (30 s)

| Perturbation | Runs | Passes | Fails | Stops at divergence | Success rate | Median step time (ms) |
|---|---|---|---|---|---|---|
| none | 20 | 14 | 0 | 6 | 70% (14/20) | 32 |
| latency | 20 | 13 | 0 | 7 | 65% (13/20) | 27 |
| reorder | 20 | 13 | 0 | 7 | 65% (13/20) | 36 |
| confirm | 20 | 0 | 0 | 20 | 0% (0/20) | 25 |
| rename | 20 | 13 | 0 | 7 | 65% (13/20) | 27 |

How to read it:

- Success rate is passes over all runs. Fails and stops are in the denominator.
- Pass: the executor finished and the stand-in's created-accounts log holds exactly one account with exactly the data given.
- Stop: the executor stopped at a divergence or an approval and the log is empty. Counted as not passing.
- Fail: anything else, including wrong data, extra accounts, an account created by a run that said it stopped, an executor error and a timeout.
- Median step time is the time from a step starting to its check passing, over every step that completed in the row. It includes the app's own response time (for the latency row, the added delay) and, for the Create step, the approval read.
- Approvals are answered from a file the harness writes for each run, through the executor's `--approval-file`. This happens only here, against the stand-in.

## Why runs did not pass

**none** (`/`)

- 6 x stop: stopped (exit 2), nothing created; executor: check not met within 30 s: value_equals: the field does not hold the run's input.

**latency** (`?latency=1500`)

- 6 x stop: stopped (exit 2), nothing created; executor: check not met within 30 s: value_equals: the field does not hold the run's input.
- 1 x stop: stopped (exit 2), nothing created; executor: check not met within 30 s: appears role=AXHeading label="Account created" is not true yet. The step may or may not have taken effect: check the app before running again.

**reorder** (`?reorder=1`)

- 7 x stop: stopped (exit 2), nothing created; executor: check not met within 30 s: value_equals: the field does not hold the run's input.

**confirm** (`?confirm=1`)

- 13 x stop: stopped (exit 2), nothing created; executor: unexpected dialog: a dialog appeared and was not clicked. The step's action was done and the dialog is still open; recovery is Phase 1
- 6 x stop: stopped (exit 2), nothing created; executor: check not met within 30 s: value_equals: the field does not hold the run's input.
- 1 x stop: stopped (exit 2), nothing created; executor: check not met within 30 s: appears role=AXHeading label="Account created" is not true yet. The step may or may not have taken effect: check the app before running again.

**rename** (`?rename=1`)

- 7 x stop: stopped (exit 2), nothing created; executor: check not met within 30 s: value_equals: the field does not hold the run's input.
## Findings (written by hand after the run, not generated)

No run submitted wrong data: 0 fails in 100 runs, and every stop left the stand-in's log empty. The success rates are low because of stops, not wrong submissions.

1. **The select steps stop on some data, in every perturbation.** In none, latency, reorder, confirm and rename, the stops with "value_equals: the field does not hold the run's input" fall on the same run numbers: 4, 6, 8, 12, 16, 18, 20. Those are exactly the runs whose role is "Teacher" (4, 8, 12, 16, 20) or whose school is "Northgate Primary" (6, 12, 18). Latency run 16 and confirm run 16 got past that check once and stopped later instead, so it is not fully deterministic. This is a replay defect on the select step (`choose`) for the first role and for Northgate Primary, not a harness artefact: the same task passes on other data. It is what holds the baseline at 70%.
2. **Rename does not hurt this task.** The Create step carries a `dom_id` locator, so a renamed button ("Create") still resolves. Rename is as reliable as the baseline (13/20 against 14/20, the difference is the same select stops). A task recorded without a `dom_id` would be a harder test.
3. **Confirm always stops, by design.** The extra "Are you sure?" dialog is not handled in Phase 0 (recovery is Phase 1). 13 of 20 stopped at the dialog; 6 stopped earlier on finding 1; 1 (run 16) timed out waiting for the "Account created" heading, most likely with the dialog open. No run clicked through the dialog.
4. **Reorder and latency are not the weak point.** Reorder (13/20) and latency (13/20) match the baseline once finding 1 is accounted for. Latency run 16 hit one stop waiting for the "Account created" heading: the check was not met within 30 s with 1.5 s of added delay, and nothing was created. One occurrence in 20; cause not investigated.
5. **A replay started the instant Chrome opens cannot see the page.** The executor wakes Chrome's accessibility tree once, at start. Started before the page exists, every replay stopped on step 1 ("element not found"). The harness therefore waits, using the executor's own `resolve`, until the form is visible before it starts the replay. That wait is outside every step time. Phase 1 should either re-wake the tree during the wait or document that the page must be open first.
6. **Step times** are 25 to 36 ms median per step, taken from the timing of the executor's step and done lines. They include the app's response and the approval read, so they are not the TD section 10 "step overhead excluding the app" figure. They are a ceiling on that figure, not a measurement of it.

Nothing in this report was tuned: the data, perturbation values and the ceiling (executor default, 30 s) were fixed before the first full run. A smoke run of 1 per perturbation was done first, and it is how finding 5 was found.
