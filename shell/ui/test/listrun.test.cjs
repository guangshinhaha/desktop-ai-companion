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
  assert.deepEqual(calls[0], ["listrun_preview", { path: "/tmp/staff.csv", taskId: "tsk_x", version: 2 }]);
  assert.deepEqual(ui.state.preview.columns, ["Name"]);
});

test("a backend refusal is shown to the officer", async () => {
  const mount = new Node("main");
  const ui = createListRun(fakeDoc, async () => { throw "fix the file first: row 2 full_name is empty"; }, mount);
  await ui.choose();
  assert.match(role(mount, "error")[0].textContent, /row 2/);
});
