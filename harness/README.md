# harness

Reliability harness (TD section 11). Replays one recorded task N times under each stand-in perturbation and writes a markdown table. Node 24+, no dependencies.

```
cd executor-macos && swift build          # once
harness/run --task executor-macos/Tests/fixtures/replay/provision-task.json --runs 20
```

Flags: `--out <file>` (default `harness/reports/<date>-replay-reliability.md`), `--port` (stand-in, default 3917), `--executor`, `--chrome`, `--ceiling <seconds>` (executor step ceiling, default the executor's 30).

## What a run is

For each perturbation (none, latency=1500, reorder, confirm, rename) and each run: reset the stand-in (`POST /api/reset`), open a fresh Chrome on an isolated temporary profile at the perturbed URL, run `executor-macos replay` against that Chrome's pid, read the stand-in's `/api/accounts`, close Chrome. Each run uses different synthetic data.

- pass: executor finished and the log holds exactly one account with exactly the data given.
- stop: executor stopped at a divergence or approval (exit 2 or 4) and the log is empty.
- fail: anything else (wrong data, extra account, account created by a stopped run, executor error, timeout).

Success rate is passes over all runs; stops and fails stay in the denominator.

## Approvals

The harness answers the executor's approval prompt with a `y` file it writes for each run, through the executor's own `--approval-file`. It is used only here, against the stand-in with synthetic data. It never runs against a real app.

## Needs

A SEED MacBook with the screen unlocked, Accessibility permission for the terminal running it, and Google Chrome. It drives the real screen: do not use the keyboard or mouse while it runs. Do not run it in CI.

## Tests

`npm test` covers judging, the report table and the runner (real stand-in server, fake executor and fake browser). It does not need a screen.
