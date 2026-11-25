// Routing tests (#32): deterministic, no model, no network. Synthetic data only.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const R = require("../router.js");
const { createChat } = require("../chat.js");

const TASKS = [
  { id: "tsk_a", version: 2, name: "Add test account", inputs: [{ name: "full_name", type: "text" }, { name: "role", type: "choice", choices: ["Teacher", "Admin"] }] },
  { id: "tsk_b", version: 1, name: "Close test case", inputs: [] },
];

test("naming a saved task or typing run <task> offers that task; anything else is chat", () => {
  assert.equal(R.route("Add test account", TASKS).task.id, "tsk_a");
  assert.equal(R.route("please do the add test account thing", TASKS).task.id, "tsk_a");
  const r = R.route("run Close test case", TASKS);
  assert.deepEqual([r.path, r.task.id, r.explicit], ["task", "tsk_b", true]);
  assert.equal(R.route("Draft a thank-you note to Test Person", TASKS).path, "chat");
  assert.equal(R.route("Summarise this meeting", TASKS).path, "chat");
  assert.equal(R.route("what is the weather", TASKS).path, "chat");
});

test("run <unknown> is answered locally, and two tasks with one name are never guessed between", () => {
  assert.equal(R.route("run Missing task", TASKS).path, "none");
  const twins = [{ ...TASKS[0] }, { ...TASKS[0], id: "tsk_z" }];
  assert.deepEqual([R.route("Add test account", twins).path, R.route("Add test account", twins).reason], ["none", "ambiguous"]);
});

test("the longest matching name wins", () => {
  const t = [{ id: "a", version: 1, name: "Add account", inputs: [] }, { id: "b", version: 1, name: "Add account for contractors", inputs: [] }];
  assert.equal(R.route("add account for contractors", t).task.id, "b");
});

test("a manual task with no recording is a learn offer; writing tasks are not", () => {
  assert.equal(R.route("Update the leave balance for Test Person", TASKS).path, "learn");
  assert.equal(R.route("I need to submit the claim form", TASKS).path, "learn");
  assert.equal(R.route("Draft an update for the team", TASKS).path, "chat");
});

// ---- the canvas flow ----

class Node {
  constructor(tag) { this.tag = tag; this.children = []; this.attrs = {}; this.value = ""; this._text = ""; }
  setAttribute(k, v) { this.attrs[k] = v; }
  replaceChildren(...k) { this.children = k; }
  set textContent(t) { this._text = t; this.children = []; }
  get textContent() { return this._text + this.children.map((c) => c.textContent).join(""); }
}
const fakeDoc = { createElement: (t) => new Node(t) };

function setup(tasks = TASKS) {
  const ui = { input: new Node("input"), select: new Node("select"), log: new Node("div"), status: new Node("p") };
  const calls = { send: [], replay: [], record: [], versions: [] };
  const api = {
    expand() {},
    tasks: async () => tasks,
    send: async (c, m, on) => { calls.send.push(m.at(-1).content); on("Sure."); return { ok: true }; },
    replay: async (task, inputs, officer) => { calls.versions.push(task.version); calls.replay.push({ task: task.id, inputs: { ...inputs }, officer }); return { ok: true, outcome: "finished" }; },
    record: async (bundle) => { calls.record.push(bundle); return { ok: true }; },
  };
  const chat = createChat(fakeDoc, api, ui);
  const say = async (t) => { ui.input.value = t; return chat.ask(); };
  const last = () => ui.log.children.at(-1).textContent;
  const log = () => ui.log.children.map((c) => c.textContent).join("\n");
  ui.select.value = "official-open";
  return { chat, ui, calls, say, last, log };
}

test("a task path never reaches chat, and 'what did you do?' says which path was taken", async () => {
  const { say, last, calls } = setup();
  await say("Draft a thank-you note");
  await say("what did you do?");
  assert.match(last(), /chat path/);
  await say("run Close test case");
  assert.match(last(), /Close test case/);
  await say("what did you do?");
  assert.match(last(), /task path.*offered it, and it has not run/);
  assert.deepEqual(calls.send, ["Draft a thank-you note"], "the question and the task request were not sent to chat");
});

test("starting a task with inputs asks for each one in turn, and runs only when all are given", async () => {
  const { say, last, calls, log } = setup();
  await say("run Add test account");
  await say("yes");
  assert.match(last(), /Who is approving/);
  await say("Test Officer");
  assert.match(last(), /full name/);
  assert.equal(calls.replay.length, 0);
  await say("Test Person");
  assert.match(last(), /role\? Choices: Teacher, Admin/);
  assert.equal(calls.replay.length, 0);
  await say("Admin");
  assert.deepEqual(calls.replay, [{ task: "tsk_a", inputs: { full_name: "Test Person", role: "Admin" }, officer: "Test Officer" }]);
  assert.ok(!log().includes("Test Person"), "typed values are not echoed into the log");
  assert.ok(!calls.send.length, "values never went to chat");
});

const FLAGGED = [{
  id: "tsk_f", version: 1, name: "File test form", status: "needs_verification", flagged_step: "s3", inputs: [{ name: "case_ref", type: "text" }],
  candidate: { version: 2, inputs: [{ name: "case_ref", type: "text" }, { name: "reason", type: "text" }] },
}];

test("a flagged task says it needs verification, and which step, before it is offered to run (#42)", async () => {
  const { say, last, calls } = setup(FLAGGED);
  await say("run File test form");
  assert.match(last(), /needs verification/);
  assert.match(last(), /step s3/);
  assert.match(last(), /new/);
  assert.equal(calls.replay.length, 0);
});

test("'new' runs the new recording as its own version with its own inputs; 'yes' runs the current one (#42)", async () => {
  const a = setup(FLAGGED);
  await a.say("run File test form");
  await a.say("new");
  await a.say("Test Officer");
  assert.match(a.last(), /case ref/);
  await a.say("C-1");
  assert.match(a.last(), /reason/, "the new recording's inputs are asked");
  await a.say("Test reason");
  assert.deepEqual(a.calls.versions, [2]);
  const b = setup(FLAGGED);
  await b.say("run File test form");
  await b.say("yes");
  await b.say("Test Officer");
  await b.say("C-1");
  assert.deepEqual(b.calls.versions, [1]);
});

test("an unverified task says it has not finished a run, never 'ok' (#42)", async () => {
  const { say, last } = setup([{ id: "tsk_u", version: 1, name: "Fresh task", status: "unverified", inputs: [] }]);
  await say("run Fresh task");
  assert.match(last(), /not finished a run/);
});

test("no default or guessed value: a blank or off-list answer asks again and nothing runs", async () => {
  const { say, last, calls } = setup();
  await say("run Add test account");
  await say("yes");
  await say("Test Officer");
  assert.equal(await say("   "), false, "a blank answer is not an answer");
  assert.match(last(), /full name/, "the same question stands");
  await say("Test Person");
  await say("Principal");
  assert.match(last(), /Pick one of: Teacher, Admin/);
  assert.equal(calls.replay.length, 0, "never started with an input missing");
  await say("teacher");
  assert.equal(calls.replay[0].inputs.role, "Teacher", "the saved spelling of the choice, not a guess");
});

test("cancel stops the questions and starts nothing", async () => {
  const { say, last, calls } = setup();
  await say("run Add test account");
  await say("yes");
  await say("Test Officer");
  await say("cancel");
  assert.match(last(), /did not run it/);
  assert.equal(calls.replay.length, 0);
});

test("a task only mentioned, then 'no', goes to chat as typed", async () => {
  const { say, calls } = setup();
  await say("Explain the Close test case process");
  await say("no");
  assert.deepEqual(calls.send, ["Explain the Close test case process"]);
});

test("describing a manual task offers to learn it, and yes then an app starts recording", async () => {
  const { say, last, calls } = setup();
  await say("Update the leave balance for Test Person");
  assert.match(last(), /Do it once and I will learn it/);
  assert.equal(calls.record.length, 0);
  await say("yes");
  assert.match(last(), /bundle id/);
  await say("not an app");
  assert.equal(calls.record.length, 0, "no app is guessed");
  await say("com.example.StandIn");
  assert.deepEqual(calls.record, ["com.example.StandIn"]);
  await say("what did you do?");
  assert.match(last(), /learn path.*started recording/);
  assert.equal(calls.send.length, 0);
});

test("saying no to the learn offer asks chat instead", async () => {
  const { say, calls } = setup();
  await say("Submit the claim form for Test Person");
  await say("no");
  assert.deepEqual(calls.send, ["Submit the claim form for Test Person"]);
});

test("with no saved tasks the shell still routes: chat for chat, and a failed task list does not block chat", async () => {
  const { say, chat, calls, ui } = setup();
  ui.select.value = "official-open";
  await say("hello");
  assert.equal(calls.send.length, 1);
  assert.equal(chat.state().lastRequest.path, "chat");
});
