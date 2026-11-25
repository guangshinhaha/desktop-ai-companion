// DOM tests for the task notes screen, against a small fake DOM and a fake backend (no web view).
// Synthetic data only.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { createTaskNotes } = require("../task.js");

class Node {
  constructor(tag) { this.tag = tag; this.children = []; this.listeners = {}; this.attrs = {}; this.value = ""; this.disabled = false; this.selected = false; this._text = ""; }
  appendChild(c) { this.children.push(c); return c; }
  replaceChildren(...k) { this.children = k; }
  setAttribute(k, v) { this.attrs[k] = v; }
  addEventListener(ev, fn) { (this.listeners[ev] ||= []).push(fn); }
  set textContent(t) { this._text = t; this.children = []; }
  get textContent() { return this._text + this.children.map((c) => c.textContent).join(""); }
  async fire(ev) { for (const fn of this.listeners[ev] || []) await fn({ target: this }); }
  async click() { if (!this.disabled) await this.fire("click"); }
  async enter(v) { this.value = v; await this.fire("input"); }
  all(pred, out = []) { if (pred(this)) out.push(this); this.children.forEach((c) => c.all && c.all(pred, out)); return out; }
}
const fakeDoc = { createElement: (t) => new Node(t), createTextNode: (t) => { const n = new Node("#text"); n._text = t; return n; } };
const role = (root, r) => root.all((n) => n.attrs["data-role"] === r);

function backend() {
  const state = { notes: [], calls: [] };
  const detail = () => ({ id: "tsk_a", version: 1, name: "Add test account", max_chars: 2000,
    steps: [{ id: "s1", say: "Type into Full name" }, { id: "s2", say: "Press Create account" }], notes: state.notes });
  const invoke = async (cmd, args) => {
    state.calls.push([cmd, args]);
    if (cmd === "task_catalog") return [{ id: "tsk_a", name: "Add test account", reuse: state.reuse ?? { replays: 3, last_success_at: 1700000000, scope: "this_device" } }];
    if (cmd === "task_detail") return detail();
    if (cmd === "task_note_add") { state.notes = [...state.notes, { id: state.notes.length + 1, author: args.author, at: 100, body: args.body }]; return detail(); }
    if (cmd === "task_note_remove") { state.notes = state.notes.filter((n) => n.id !== args.noteId); return detail(); }
    throw new Error("unexpected " + cmd);
  };
  return { state, invoke };
}

async function open(b) {
  const mount = new Node("main");
  const ui = createTaskNotes(fakeDoc, b.invoke, mount);
  await ui.load();
  const pick = role(mount, "task-pick")[0];
  pick.value = "tsk_a";
  await pick.fire("change");
  return mount;
}

test("notes show beside the step list, and adding clears the field", async () => {
  const b = backend();
  const mount = await open(b);
  assert.equal(role(mount, "steps")[0].children.length, 2);
  assert.equal(role(mount, "no-notes").length, 1);
  await role(mount, "author")[0].enter("officer-1");
  const field = role(mount, "note-text")[0];
  field.value = "Decided: use the test org only.";
  await role(mount, "note-add")[0].click();
  assert.equal(field.value, "", "field cleared at once");
  const body = role(mount, "note-body");
  assert.equal(body.length, 1);
  assert.equal(body[0].textContent, "Decided: use the test org only.");
  assert.deepEqual(b.state.calls.at(-1)[1], { taskId: "tsk_a", author: "officer-1", body: "Decided: use the test org only." });
  assert.match(role(mount, "privacy")[0].textContent, /stay on this device/);
});

test("a note can be removed, and a blank note is not sent", async () => {
  const b = backend();
  b.state.notes = [{ id: 1, author: "o", at: 1, body: "old" }];
  const mount = await open(b);
  const before = b.state.calls.length;
  role(mount, "note-text")[0].value = "   ";
  await role(mount, "note-add")[0].click();
  assert.equal(b.state.calls.length, before);
  await role(mount, "note-remove")[0].click();
  assert.equal(role(mount, "note-body").length, 0);
});

test("the task shows how many times it ran on this device and when it last succeeded, with no names", async () => {
  const mount = await open(backend());
  const line = role(mount, "reuse")[0].textContent;
  assert.match(line, /Replayed 3 times on this device/);
  assert.match(line, /last succeeded/);
  assert.doesNotMatch(line, /officer/i);
});

test("a task never replayed says so instead of a date", async () => {
  const b = backend();
  b.state.reuse = { replays: 0, last_success_at: null, scope: "this_device" };
  assert.match(role(await open(b), "reuse")[0].textContent, /Not replayed yet on this device/);
});

test("one replay is singular, and a replay with no success has no date", async () => {
  const b = backend();
  b.state.reuse = { replays: 1, last_success_at: null, scope: "this_device" };
  const t = role(await open(b), "reuse")[0].textContent;
  assert.match(t, /Replayed 1 time on this device/);
  assert.match(t, /no successful run yet/);
});
