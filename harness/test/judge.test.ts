import { test } from "node:test";
import assert from "node:assert/strict";
import { inputsFor, judge, median } from "../src/judge.ts";
import { renderReport, summarise, type Row } from "../src/report.ts";

const want = inputsFor(3);

test("pass needs exit 0 and exactly the data given", () => {
  assert.equal(judge({ exitCode: 0, timedOut: false, accounts: [want] }, want).outcome, "pass");
});

test("wrong data is a fail even when the executor says finished", () => {
  const v = judge({ exitCode: 0, timedOut: false, accounts: [{ ...want, role: "Teacher" }] }, inputsFor(3));
  assert.equal(v.outcome, "fail");
  assert.match(v.reason, /wrong data/);
});

test("a stop with nothing created is a stop; with an account created it is a fail", () => {
  assert.equal(judge({ exitCode: 2, timedOut: false, accounts: [] }, want).outcome, "stop");
  assert.equal(judge({ exitCode: 4, timedOut: false, accounts: [] }, want).outcome, "stop");
  assert.equal(judge({ exitCode: 2, timedOut: false, accounts: [want] }, want).outcome, "fail");
});

test("errors, timeouts, empty finishes and duplicates are fails", () => {
  assert.equal(judge({ exitCode: 1, timedOut: false, accounts: [] }, want).outcome, "fail");
  assert.equal(judge({ exitCode: null, timedOut: true, accounts: [] }, want).outcome, "fail");
  assert.equal(judge({ exitCode: 0, timedOut: false, accounts: [] }, want).outcome, "fail");
  assert.equal(judge({ exitCode: 0, timedOut: false, accounts: [want, want] }, want).outcome, "fail");
  assert.equal(judge({ exitCode: null, timedOut: true, accounts: [want] }, want).outcome, "fail");
});

test("median", () => {
  assert.equal(median([]), null);
  assert.equal(median([5, 1, 3]), 3);
  assert.equal(median([4, 1, 3, 2]), 2.5);
});

test("inputs differ run to run and are synthetic", () => {
  assert.notDeepEqual(inputsFor(1), inputsFor(2));
  assert.match(inputsFor(7).email, /@example\.test$/);
});

test("report counts fails and stops in the denominator", () => {
  const rows: Row[] = [
    {
      name: "none", query: "",
      runs: [
        { outcome: "pass", reason: "ok", stepMs: [100, 200] },
        { outcome: "fail", reason: "wrong data submitted", stepMs: [] },
        { outcome: "stop", reason: "stopped (exit 2), nothing created", stepMs: [300] },
        { outcome: "pass", reason: "ok", stepMs: [] },
      ],
    },
  ];
  const s = summarise(rows[0]!);
  assert.deepEqual([s.runs, s.passes, s.fails, s.stops, s.median], [4, 2, 1, 1, 200]);
  const md = renderReport(rows, { date: "d", taskName: "t", taskFile: "f", runsPerPerturbation: 4, executorCommit: "c", host: "h", ceiling: "x" });
  assert.match(md, /\| none \| 4 \| 2 \| 1 \| 1 \| 50% \(2\/4\) \| 200 \|/);
  assert.match(md, /1 x fail: wrong data submitted/);
});
