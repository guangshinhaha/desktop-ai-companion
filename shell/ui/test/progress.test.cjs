// DOM tests for the progress view, against a small fake DOM and a fake backend (no web view).
// Synthetic data only.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { createProgress } = require("../progress.js");

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

function view(over = {}) {
  return {
    run: "run_1", task: "Provision a staff account", state: "running", over: false,
    current: { step: "s2", say: "Press Create account", index: 2, total: 3 },
    steps: [
      { id: "s1", say: "Type into Full name", status: "done" },
      { id: "s2", say: "Press Create account", status: "current" },
      { id: "s3", say: "Check the confirmation", status: "todo" },
    ],
    report: null, ...over,
  };
}

function setup(v = view()) {
  const calls = [];
  const invoke = async (cmd) => {
    calls.push(cmd);
    if (cmd === "progress_current") return v;
    return null;
  };
  const mount = new Node("main");
  return { ui: createProgress(fakeDoc, invoke, mount), mount, calls };
}

test("it shows the current say line, and which steps are done and remaining", async () => {
  const { ui, mount } = setup();
  await ui.load();
  assert.equal(role(mount, "say")[0].textContent, "Press Create account");
  assert.match(role(mount, "count")[0].textContent, /1 of 3 steps done, 2 remaining/);
  const marks = role(mount, "steps")[0].children.map((li) => li.attrs["data-status"]);
  assert.deepEqual(marks, ["done", "current", "todo"]);
  assert.match(role(mount, "steps")[0].children[0].textContent, /\[x\] Type into Full name/);
});

test("Stop and Pause are one click each", async () => {
  const { ui, mount, calls } = setup();
  await ui.load();
  await role(mount, "stop")[0].click();
  assert.equal(calls.at(-1), "run_stop");
  await role(mount, "pause")[0].click();
  assert.equal(calls.at(-1), "run_pause");
});

test("a paused run offers Resume in place of Pause", async () => {
  const { ui, mount, calls } = setup(view({ state: "paused" }));
  await ui.load();
  assert.equal(role(mount, "pause").length, 0);
  await role(mount, "resume")[0].click();
  assert.equal(calls.at(-1), "run_resume");
});

test("Escape stops; other keys do nothing; nothing is sent once the run is over", async () => {
  const { ui, calls } = setup();
  await ui.load();
  ui.key({ key: "a" });
  assert.deepEqual(calls, ["progress_current"]);
  await ui.key({ key: "Escape" });
  assert.equal(calls.at(-1), "run_stop");
  ui.update(view({ state: "stopped", over: true, current: null }));
  const n = calls.length;
  ui.key({ key: "Escape" });
  assert.equal(calls.length, n);
});

test("while stopping, the buttons are disabled so a second click does nothing new", async () => {
  const { ui, mount } = setup(view({ state: "stopping" }));
  await ui.load();
  assert.equal(role(mount, "stop")[0].disabled, true);
  assert.equal(role(mount, "pause")[0].disabled, true);
});

test("a stopped run reports what completed and what did not, and that the app was left as it is", async () => {
  const { ui, mount } = setup();
  await ui.load();
  ui.update(view({
    state: "stopped", over: true, current: null,
    steps: [
      { id: "s1", say: "Type into Full name", status: "done" },
      { id: "s2", say: "Press Create account", status: "stopped" },
      { id: "s3", say: "Check the confirmation", status: "todo" },
    ],
    report: { outcome: "stopped", completed: ["s1"], not_run: ["s2", "s3"], step: "s2" },
  }));
  assert.match(role(mount, "outcome")[0].textContent, /Stopped/);
  assert.equal(role(mount, "completed")[0].textContent, "Completed: s1");
  assert.equal(role(mount, "not-run")[0].textContent, "Not done: s2, s3");
  assert.match(role(mount, "left-as-is")[0].textContent, /left as it is/);
  assert.equal(role(mount, "stop").length, 0);
});

test("task text is shown as text, never as markup", async () => {
  const { ui, mount } = setup(view({ steps: [{ id: "s1", say: "<img src=x onerror=alert(1)>", status: "todo" }] }));
  await ui.load();
  assert.equal(mount.all((n) => n.tag === "img").length, 0);
  assert.match(mount.textContent, /<img src=x/);
});

test("an error from the shell is shown, not swallowed", async () => {
  const mount = new Node("main");
  const ui = createProgress(fakeDoc, async (c) => { if (c === "progress_current") return view(); throw new Error("gone"); }, mount);
  await ui.load();
  await role(mount, "stop")[0].click();
  assert.match(role(mount, "error")[0].textContent, /gone/);
});
