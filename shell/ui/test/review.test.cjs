// DOM tests for the review screen, against a small fake DOM and a fake backend (no web view).
// Synthetic data only.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { createReview } = require("../review.js");

class Node {
  constructor(tag) { this.tag = tag; this.children = []; this.listeners = {}; this.attrs = {}; this.value = ""; this.checked = false; this.disabled = false; this.selected = false; this._text = ""; }
  appendChild(c) { this.children.push(c); return c; }
  replaceChildren(...k) { this.children = k; }
  setAttribute(k, v) { this.attrs[k] = v; }
  addEventListener(ev, fn) { (this.listeners[ev] ||= []).push(fn); }
  set textContent(t) { this._text = t; this.children = []; }
  get textContent() { return this._text + this.children.map((c) => c.textContent).join(""); }
  async fire(ev, extra = {}) { for (const fn of this.listeners[ev] || []) await fn({ target: this, ...extra }); }
  async click() { if (!this.disabled) await this.fire("click"); }
  async enter(v) { this.value = v; await this.fire("input"); }
  async tick(on) { this.checked = on; await this.fire("change"); }
  all(pred, out = []) { if (pred(this)) out.push(this); this.children.forEach((c) => c.all && c.all(pred, out)); return out; }
}
const fakeDoc = { createElement: (t) => new Node(t), createTextNode: (t) => { const n = new Node("#text"); n._text = t; return n; } };

const role = (root, r) => root.all((n) => n.attrs["data-role"] === r);
const stepEl = (root, id) => root.all((n) => n.attrs["data-step"] === id)[0];

function view(over = {}) {
  return {
    id: "tsk_x", name: "Recorded task", author: "officer", apps: [{ key: "chrome", bundle: "com.google.Chrome" }],
    preconditions: ["chrome: exists Password"],
    inputs: [{ name: "full_name", type: "text", source: "ask", steps: ["s1"] }],
    steps: [
      { id: "s1", say: "Type into Full name", action: "set_value", consequential: "no", flag: "none", input: "full_name", is_constant: false, can_be_constant: true },
      { id: "s2", say: "Press Next", action: "press", consequential: "unknown", flag: "unknown", is_constant: false, can_be_constant: false },
      { id: "s3", say: "Press Create account", action: "press", consequential: "yes", flag: "consequential", is_constant: false, can_be_constant: false },
    ],
    unknown_count: 1, consequential_count: 1, problems: [], ...over,
  };
}

function setup(handlers = {}) {
  const calls = [];
  const invoke = async (cmd, args) => {
    calls.push([cmd, args]);
    if (handlers[cmd]) return handlers[cmd](args);
    if (cmd === "review_view") return view();
    throw new Error("unexpected " + cmd);
  };
  const mount = new Node("main");
  const ui = createReview(fakeDoc, invoke, mount);
  return { ui, mount, calls };
}

test("shows each say line, the inputs, the unknown step and the consequential step", async () => {
  const { ui, mount } = setup();
  await ui.load();
  const text = mount.textContent;
  for (const s of ["Type into Full name", "Press Next", "Press Create account", "Works in: com.google.Chrome"]) assert.ok(text.includes(s), s);
  assert.equal(role(mount, "input-name")[0].value, "full_name");
  assert.equal(stepEl(mount, "s2").all((n) => n.attrs["data-flag"] === "unknown").length, 1);
  assert.equal(stepEl(mount, "s3").all((n) => n.attrs["data-flag"] === "consequential").length, 1);
  assert.equal(stepEl(mount, "s1").all((n) => n.attrs["data-flag"]).length, 0);
  assert.match(stepEl(mount, "s2").textContent, /Treated as if it does/);
});

test("rename, delete and mark send the right edits and redraw from the reply", async () => {
  const { ui, mount, calls } = setup({ review_edit: () => view({ name: "after" }) });
  await ui.load();
  role(mount, "input-name")[0].value = "staff_name";
  await role(mount, "rename")[0].click();
  assert.deepEqual(calls.at(-1), ["review_edit", { edit: { op: "rename_input", from: "full_name", to: "staff_name" } }]);
  await role(stepEl(mount, "s2"), "delete")[0].click();
  assert.deepEqual(calls.at(-1)[1].edit, { op: "delete_step", step: "s2" });
  const sel = role(stepEl(mount, "s2"), "commits")[0];
  await sel.fire("change", { target: { value: "no" } });
  assert.deepEqual(calls.at(-1)[1].edit, { op: "set_consequential", step: "s2", value: "no" });
  assert.equal(role(mount, "task-name")[0].value, "after");
});

test("an unknown step offers yes or no, and unknown only as a disabled, selected label", async () => {
  const { ui, mount } = setup();
  await ui.load();
  const opts = role(stepEl(mount, "s2"), "commits")[0].children;
  assert.deepEqual(opts.map((o) => [o.value, o.disabled]), [["unknown", true], ["yes", false], ["no", false]]);
  assert.equal(opts[0].selected, true);
});

test("a fixed value cannot be saved until the no-personal-data box is ticked and a value is typed", async () => {
  const { ui, mount, calls } = setup({ review_edit: () => view() });
  await ui.load();
  assert.equal(role(mount, "constant-form").length, 0);
  await role(stepEl(mount, "s1"), "constant-open")[0].click();
  const form = role(mount, "constant-form")[0];
  const [value, tick, save] = [role(form, "constant-value")[0], role(form, "constant-tick")[0], role(form, "constant-save")[0]];
  assert.equal(save.disabled, true);
  await value.enter("Synthetic-Value-1");
  assert.equal(save.disabled, true, "typed but not ticked");
  await tick.tick(true);
  assert.equal(save.disabled, false);
  await tick.tick(false);
  assert.equal(save.disabled, true);
  await tick.tick(true);
  await save.click();
  assert.deepEqual(calls.at(-1)[1].edit, { op: "mark_constant", step: "s1", value: "Synthetic-Value-1", confirmed_no_personal_data: true });
  assert.equal(value.value, "", "the field is cleared once sent");
});

test("cancelling the fixed-value form clears what was typed and sends nothing", async () => {
  const { ui, mount, calls } = setup();
  await ui.load();
  await role(stepEl(mount, "s1"), "constant-open")[0].click();
  const form = role(mount, "constant-form")[0];
  const value = role(form, "constant-value")[0];
  await value.enter("Synthetic-Value-2");
  const cancel = form.all((n) => n.tag === "button" && n.textContent === "Cancel")[0];
  await cancel.click();
  assert.equal(value.value, "");
  assert.equal(role(mount, "constant-form").length, 0);
  assert.ok(calls.every(([c]) => c === "review_view"));
});

test("a refused edit shows the reason and keeps the screen", async () => {
  const { ui, mount } = setup({ review_edit: () => { throw "confirm that the value contains no personal data first"; } });
  await ui.load();
  await role(stepEl(mount, "s3"), "delete")[0].click();
  assert.match(role(mount, "error")[0].textContent, /no personal data/);
  assert.ok(mount.textContent.includes("Press Create account"));
});

test("save sends the author; discard sends discard; both clear the screen", async () => {
  const saved = setup({ review_save: () => ({ id: "tsk_x", version: 2 }) });
  await saved.ui.load();
  role(saved.mount, "author")[0].value = "officer-1";
  await role(saved.mount, "save")[0].click();
  assert.deepEqual(saved.calls.at(-1), ["review_save", { author: "officer-1" }]);
  assert.match(saved.mount.textContent, /Saved as version 2/);
  assert.ok(!saved.mount.textContent.includes("Press Next"));

  const gone = setup({ review_discard: () => null });
  await gone.ui.load();
  await role(gone.mount, "discard")[0].click();
  assert.equal(gone.calls.at(-1)[0], "review_discard");
  assert.match(gone.mount.textContent, /Discarded/);
  assert.ok(!gone.mount.textContent.includes("Press Next"));
});

test("problems show an alert, and the page never persists anything", async () => {
  const { ui, mount } = setup({ review_view: () => view({ problems: ["/steps/0/expect: every step needs a check"] }) });
  await ui.load();
  assert.match(role(mount, "problems")[0].textContent, /needs a check/);
  const src = require("node:fs").readFileSync(require.resolve("../review.js"), "utf8");
  assert.ok(!/localStorage|sessionStorage|indexedDB|document\.cookie/.test(src));
});

test("the author typed survives a failed save, so the officer can retry", async () => {
  const { ui, mount } = setup({ review_save: () => { throw "store locked"; } });
  await ui.load();
  const a = role(mount, "author")[0];
  a.value = "Officer Tan";
  await a.fire("input");
  await role(mount, "save")[0].click();
  assert.equal(role(mount, "author")[0].value, "Officer Tan");
  assert.match(role(mount, "error")[0].textContent, /store locked/);
});
