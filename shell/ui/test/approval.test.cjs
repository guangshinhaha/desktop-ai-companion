// DOM tests for the approval card, against a small fake DOM and a fake backend (no web view).
// Synthetic data only.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { createApproval } = require("../approval.js");

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

function card(over = {}) {
  return {
    officer: "Test Officer", run: "run_1", task: "Provision a staff account", app: "com.google.Chrome", step: "s5",
    summary: "Create a staff account for Test Person (test.person@example.test), Teacher at Test School",
    will_do: 'press the AXButton "Create account"', unknown: false, undoable_claim: false,
    inputs: [{ name: "full_name", value: "Test Person" }, { name: "email", value: "test.person@example.test" }],
    constants: [{ name: "Role", value: "Teacher" }], ...over,
  };
}

function setup(c = card(), handlers = {}) {
  const calls = [];
  const invoke = async (cmd, args) => {
    calls.push([cmd, args]);
    if (handlers[cmd]) return handlers[cmd](args);
    if (cmd === "approval_current") return c;
    if (cmd === "approval_answer") return null;
    throw new Error("unexpected " + cmd);
  };
  const mount = new Node("main");
  return { ui: createApproval(fakeDoc, invoke, mount), mount, calls };
}

test("the card shows the filled summary, the app, undo-ability, every input and each constant with its field", async () => {
  const { ui, mount } = setup();
  await ui.load();
  assert.match(role(mount, "summary")[0].textContent, /Create a staff account for Test Person \(test\.person@example\.test\)/);
  assert.match(role(mount, "where")[0].textContent, /App: com\.google\.Chrome/);
  assert.match(role(mount, "who")[0].textContent, /Approving as Test Officer/);
  assert.match(role(mount, "undo")[0].textContent, /can be undone: no/);
  assert.match(role(mount, "will-do")[0].textContent, /press the AXButton "Create account"/);
  assert.match(role(mount, "inputs")[0].textContent, /full_name: Test Person/);
  assert.match(role(mount, "constants")[0].textContent, /Teacher into Role/);
});

test("only the Approve button sends true; nothing is sent before a click", async () => {
  const { ui, mount, calls } = setup();
  await ui.load();
  assert.deepEqual(calls.map((c) => c[0]), ["approval_current"]);
  await role(mount, "approve")[0].click();
  assert.deepEqual(calls.at(-1), ["approval_answer", { approved: true }]);
});

test("Reject sends false, and Escape sends false", async () => {
  let s = setup();
  await s.ui.load();
  await role(s.mount, "reject")[0].click();
  assert.deepEqual(s.calls.at(-1), ["approval_answer", { approved: false }]);
  s = setup();
  await s.ui.load();
  await s.ui.key({ key: "Escape" });
  assert.deepEqual(s.calls.at(-1), ["approval_answer", { approved: false }]);
  await s.ui.key({ key: "Enter" });
  await s.ui.key({ key: "y" });
  assert.equal(s.calls.filter((c) => c[0] === "approval_answer").length, 1, "no other key answers");
});

test("one answer only: a second click or key does nothing, and both buttons are disabled", async () => {
  const { ui, mount, calls } = setup();
  await ui.load();
  const approve = role(mount, "approve")[0];
  await approve.click();
  await ui.key({ key: "Escape" });
  assert.equal(calls.filter((c) => c[0] === "approval_answer").length, 1);
  assert.equal(role(mount, "approve")[0].disabled, true);
  assert.equal(role(mount, "reject")[0].disabled, true);
});

test("no timers: the card never answers on its own", async () => {
  const realSet = globalThis.setTimeout;
  let timers = 0;
  globalThis.setTimeout = (...a) => { timers++; return realSet(...a); };
  try {
    const { ui, calls } = setup();
    await ui.load();
    assert.equal(timers, 0);
    assert.equal(calls.filter((c) => c[0] === "approval_answer").length, 0);
  } finally { globalThis.setTimeout = realSet; }
});

test("text from the task is shown as text, never as markup, and an unknown step is flagged", async () => {
  const { ui, mount } = setup(card({ summary: "<img src=x onerror=alert(1)> Create", unknown: true }));
  await ui.load();
  const s = role(mount, "summary")[0];
  assert.equal(s.children.length, 1);
  assert.equal(s.children[0].tag, "#text");
  assert.match(s.textContent, /<img src=x/);
  assert.equal(role(mount, "unknown").length, 1);
});

test("no pending card shows nothing to approve and no buttons", async () => {
  const { ui, mount } = setup(null);
  await ui.load();
  assert.equal(role(mount, "approve").length, 0);
  assert.match(mount.textContent, /Nothing is waiting/);
});

test("a failed send says so", async () => {
  const { ui, mount } = setup(card(), { approval_answer: () => { throw new Error("gone"); } });
  await ui.load();
  await role(mount, "approve")[0].click();
  assert.match(role(mount, "error")[0].textContent, /The run stops/);
});
