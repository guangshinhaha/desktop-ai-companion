// List run screen, against a small fake DOM and a fake backend (no web view). Synthetic data only.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { createListRun, answersFor, needsOfficer } = require("../listrun.js");

class Node {
  constructor(tag) { this.tag = tag; this.children = []; this.listeners = {}; this.attrs = {}; this.disabled = false; this._text = ""; }
  appendChild(c) { this.children.push(c); return c; }
  replaceChildren(...k) { this.children = k; }
  setAttribute(k, v) { this.attrs[k] = v; }
  addEventListener(ev, fn) { (this.listeners[ev] ||= []).push(fn); }
  get textContent() { return this._text + this.children.map((c) => c.textContent).join(""); }
  async click() { if (!this.disabled) for (const fn of this.listeners.click || []) await fn({ target: this }); }
  all(pred, out = []) { if (pred(this)) out.push(this); this.children.forEach((c) => c.all && c.all(pred, out)); return out; }
}
const fakeDoc = { createElement: (t) => new Node(t), createTextNode: (t) => { const n = new Node("#text"); n._text = t; return n; } };
const role = (root, r) => root.all((n) => n.attrs["data-role"] === r);

function setup(status) {
  const calls = [];
  const invoke = async (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === "listrun_start") return "run_1";
    if (cmd === "listrun_status") return status;
    if (cmd === "listrun_unfinished") return [{ run: "run_9", task: "tsk_x", version: 1, status: "stopped", counts: { done: 2, unknown: 1 } }];
    if (cmd === "listrun_preview") return { columns: ["Name"], rows: 2, inputs: ["full_name"], suggested: {} };
    return null;
  };
  const mount = new Node("main");
  return { ui: createListRun(fakeDoc, invoke, mount), mount, calls };
}

test("an unknown row offers the two answers and a diverged row offers skip or by hand", () => {
  assert.deepEqual(answersFor("unknown").map((a) => a[0]), ["mark_done", "mark_not_done"]);
  assert.deepEqual(answersFor("diverged").map((a) => a[0]), ["skip", "fixed_by_hand"]);
  for (const s of ["pending", "running", "committing", "done", "skipped"]) assert.deepEqual(answersFor(s), []);
});

test("the run cannot be continued from the screen while a row waits for an answer", async () => {
  const { ui, mount } = setup({ run: "run_1", status: "stopped", items: [{ item: 1, state: "done" }, { item: 2, state: "unknown" }] });
  ui.state.run = "run_1";
  await ui.refresh();
  assert.equal(role(mount, "resume")[0].disabled, true);
  assert.equal(role(mount, "blocked").length, 1);
  assert.equal(needsOfficer(ui.state.status.items).length, 1);
});

test("answering sends the row and the officer's name, then reloads the rows", async () => {
  const { ui, mount, calls } = setup({ run: "run_1", status: "stopped", items: [{ item: 2, state: "unknown" }] });
  ui.state.run = "run_1";
  ui.state.officer = "Test Officer";
  await ui.refresh();
  await role(mount, "mark_not_done")[0].click();
  const a = calls.find((c) => c[0] === "listrun_answer");
  assert.deepEqual(a[1], { run: "run_1", item: 2, answer: "mark_not_done", officer: "Test Officer" });
  assert.ok(calls.filter((c) => c[0] === "listrun_status").length >= 2);
});

test("the file is read through the shell and only column names come back to the page", async () => {
  const { ui, calls } = setup({ items: [] });
  ui.state.path = "/tmp/staff.csv"; ui.state.taskId = "tsk_x"; ui.state.version = "2";
  await ui.choose();
  assert.deepEqual(calls[0], ["listrun_preview", { path: "/tmp/staff.csv", taskId: "tsk_x", version: 2, header: false }]);
  assert.deepEqual(ui.state.preview.columns, ["Name"]);
});

test("a backend refusal is shown to the officer", async () => {
  const mount = new Node("main");
  const ui = createListRun(fakeDoc, async () => { throw "fix the file first: row 2 full_name is empty"; }, mount);
  await ui.choose();
  assert.match(role(mount, "error")[0].textContent, /row 2/);
});

test("unfinished runs are listed after a restart and can be opened", async () => {
  const { ui, mount, calls } = setup({ run: "run_9", status: "stopped", items: [{ item: 3, state: "unknown" }] });
  await ui.loadUnfinished();
  assert.equal(role(mount, "unfinished")[0].children.length, 1);
  await role(mount, "open-run")[0].click();
  assert.equal(ui.state.run, "run_9");
  assert.equal(role(mount, "blocked").length, 1);
  assert.deepEqual(calls.find((c) => c[0] === "listrun_status")[1], { run: "run_9" });
});

test("a refused start shows why and lists the unfinished run by id, to open and end", async () => {
  const why = "this task has an unfinished list run: run_9 (version 1). Resume it or end it.";
  const calls = [];
  const mount = new Node("main");
  const ui = createListRun(fakeDoc, async (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === "listrun_start") throw why;
    if (cmd === "listrun_unfinished") return [{ run: "run_9", task: "tsk_x", version: 1, status: "stopped", counts: { done: 1 } }];
    if (cmd === "listrun_status") return { run: "run_9", status: "stopped", items: [{ item: 2, state: "pending" }] };
    return null;
  }, mount);
  await ui.start();
  assert.equal(role(mount, "error")[0].textContent, why);
  assert.match(role(mount, "unfinished")[0].textContent, /run_9: tsk_x v1/);
  await role(mount, "open-run")[0].click();
  await role(mount, "end")[0].click();
  assert.equal(calls.find((c) => c[0] === "listrun_end")[1].run, "run_9");
});

test("a refused continue, answer or end stays on the screen after the rows reload", async () => {
  for (const [button, cmd] of [["resume", "listrun_resume"], ["skip", "listrun_answer"], ["end", "listrun_end"]]) {
    const mount = new Node("main");
    const ui = createListRun(fakeDoc, async (c) => {
      if (c === cmd) throw "refused: " + cmd;
      return { run: "run_1", status: "stopped", items: [{ item: 1, state: button === "skip" ? "diverged" : "pending" }] };
    }, mount);
    ui.state.run = "run_1";
    await ui.refresh();
    await role(mount, button)[0].click();
    await new Promise((r) => setImmediate(r));
    assert.equal(role(mount, "error")[0].textContent, "refused: " + cmd);
  }
});

test("the first row is not taken as column names until the officer says so", async () => {
  const { ui, mount, calls } = setup({ items: [] });
  ui.state.path = "/tmp/a.csv"; ui.state.taskId = "t";
  await ui.choose();
  assert.equal(calls[0][1].header, false);
  ui.state.header = true;
  await ui.choose();
  assert.equal(calls[1][1].header, true);
  await ui.start();
  assert.equal(calls.find((c) => c[0] === "listrun_start")[1].header, true);
  assert.ok(mount);
});

test("a run error and a task with no approval step are shown", async () => {
  const { ui, mount } = setup({ run: "run_1", status: "stopped", error: "the executor ended", active: false, items: [{ item: 1, state: "pending" }] });
  ui.state.run = "run_1";
  await ui.refresh();
  assert.match(role(mount, "run-error")[0].textContent, /executor ended/);
  const b = setup({ items: [] });
  b.ui.state.preview = { columns: ["A"], rows: 1, inputs: ["x"], suggested: {}, approval_steps: 0 };
  b.ui.draw();
  assert.equal(role(b.mount, "no-approval").length, 1);
});

test("identical rows are warned about by row number, and Start stays enabled", async () => {
  const { ui, mount, calls } = setup({ run: "run_1", status: "running", items: [] });
  ui.state.preview = { columns: ["A"], rows: 7, inputs: ["x"], suggested: {}, approval_steps: 1, duplicate_rows: { pairs: [[1, 3], [2, 7]], total: 2 } };
  ui.draw();
  const w = role(mount, "duplicate-rows");
  assert.equal(w.length, 1);
  assert.equal(w[0].attrs.role, "alert");
  assert.match(w[0].textContent, /rows 1 and 3; rows 2 and 7\./);
  assert.doesNotMatch(w[0].textContent, /more/);
  assert.equal(role(mount, "start")[0].disabled, false);
  await role(mount, "start")[0].click();
  assert.equal(calls.filter((c) => c[0] === "listrun_start").length, 1);
});

test("when there are more identical rows than listed, the warning says how many more", () => {
  const { ui, mount } = setup({ items: [] });
  ui.state.preview = { columns: ["A"], rows: 26, inputs: ["x"], suggested: {}, approval_steps: 1, duplicate_rows: { pairs: [[1, 2], [1, 3]], total: 25 } };
  ui.draw();
  assert.match(role(mount, "duplicate-rows")[0].textContent, /rows 1 and 2; rows 1 and 3; and 23 more pairs\./);
});

test("with no identical rows there is no duplicate warning", () => {
  const { ui, mount } = setup({ items: [] });
  ui.state.preview = { columns: ["A"], rows: 2, inputs: ["x"], suggested: {}, approval_steps: 1, duplicate_rows: { pairs: [], total: 0 } };
  ui.draw();
  assert.equal(role(mount, "duplicate-rows").length, 0);
});
