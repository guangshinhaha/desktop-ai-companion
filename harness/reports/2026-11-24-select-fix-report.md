# Replay reliability report: Provision a staff account (stand-in)

- Date: 2026-11-24
- Task file: `executor-macos/Tests/fixtures/replay/provision-task.json`, replayed against the stand-in provisioning app (synthetic data only)
- Runs per perturbation: 20
- Executor: executor-macos debug build, repo commit 3d2967f
- Host: gs-tan-v71f1, macOS 26.7.1, Chrome with a fresh isolated profile per run
- Step ceiling: executor default (30 s)

| Perturbation | Runs | Passes | Fails | Stops at divergence | Success rate | Median step time (ms) |
|---|---|---|---|---|---|---|
| none | 20 | 19 | 0 | 1 | 95% (19/20) | 27 |
| latency | 20 | 20 | 0 | 0 | 100% (20/20) | 28 |
| reorder | 20 | 20 | 0 | 0 | 100% (20/20) | 30 |
| confirm | 20 | 0 | 0 | 20 | 0% (0/20) | 20 |
| rename | 20 | 20 | 0 | 0 | 100% (20/20) | 28 |

How to read it:

- Success rate is passes over all runs. Fails and stops are in the denominator.
- Pass: the executor finished and the stand-in's created-accounts log holds exactly one account with exactly the data given.
- Stop: the executor stopped at a divergence or an approval and the log is empty. Counted as not passing.
- Fail: anything else, including wrong data, extra accounts, an account created by a run that said it stopped, an executor error and a timeout.
- Median step time is the time from a step starting to its check passing, over every step that completed in the row. It includes the app's own response time (for the latency row, the added delay) and, for the Create step, the approval read.
- Approvals are answered from a file the harness writes for each run, through the executor's `--approval-file`. This happens only here, against the stand-in.

## Why runs did not pass

**none** (`/`)

- 1 x stop: stopped (exit 2), nothing created; executor: the form was not visible in the accessibility tree after 40 s, before the replay started; a dialog is open before the step; nothing was clicked

**confirm** (`?confirm=1`)

- 20 x stop: stopped (exit 2), nothing created; executor: unexpected dialog: a dialog appeared and was not clicked. The step's action was done and the dialog is still open; recovery is Phase 1

## Findings (written by hand after the run, not generated)

Run for #70 after the select fix. Same task, data and perturbations as `2026-11-24-first-report.md`; only the executor changed.

1. **Root cause of #70 (replay side, not the recorder or the stand-in).** A select to the option the pop-up already shows (the first option, selected at page load: role "Teacher", school "Northgate Primary") was done by opening the pop-up and pressing that menu item through accessibility. In Chrome that leaves the select with nothing chosen: the field then reads an empty value for the full 30 s ceiling, and the form's `required` select blocks submit. Evidence on this host: `choose` logged the value before the press as "Teacher" (already selected) and `value_equals` then read an empty string on every poll; with the check forced to accept an empty value, the run still created nothing (the required select was empty). The recorded step, its `value_equals` check and the stand-in were correct. Fix: when the field already shows the wanted value, do not press; the `value_equals` check still reads the field and decides.
2. **none: 19/20.** Before the fix: 14/20, all 6 stops on this check. After: the 6 select stops are gone. The one stop (run 6) happened before the first step: "the form was not visible in the accessibility tree after 40 s ... a dialog is open before the step; nothing was clicked". A dialog was open in the fresh Chrome profile, nothing was created and no step ran. That is a harness or browser start-up event, not the select; cause not investigated (run 6 passed in the 20/20 rows of latency, reorder and rename, and in a separate run after the fix).
3. **latency, reorder, rename: 20/20 each**, up from 13, 13 and 13. The earlier latency stop on the "Account created" heading did not recur.
4. **confirm: 0/20 is unchanged and by design.** All 20 stop at the "Are you sure?" dialog (recovery is Phase 1, #34). No run clicked through it.
5. Nothing was tuned: data, perturbations and the ceiling were not changed from the first report.
