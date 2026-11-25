// DOM tests for chat in the canvas, against a small fake DOM and a fake backend (no web view).
// Synthetic data only.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { createChat, SUMMARY_PROMPT } = require("../chat.js");

class Node {
  constructor(tag) { this.tag = tag; this.children = []; this.attrs = {}; this.value = ""; this._text = ""; }
  setAttribute(k, v) { this.attrs[k] = v; }
  replaceChildren(...k) { this.children = k; }
  set textContent(t) { this._text = t; this.children = []; }
  get textContent() { return this._text + this.children.map((c) => c.textContent).join(""); }
}
const fakeDoc = { createElement: (t) => new Node(t) };

function setup({ send, attach } = {}) {
  const ui = { input: new Node("input"), select: new Node("select"), log: new Node("div"), status: new Node("p") };
  const calls = { send: [], expand: 0, attach: [] };
  const api = {
    expand: () => { calls.expand++; },
    send: send ? (c, m, on) => { calls.send.push({ c, m: JSON.parse(JSON.stringify(m)) }); return send(c, m, on); }
      : async (c, m, on) => { calls.send.push({ c, m: JSON.parse(JSON.stringify(m)) }); on("Dear "); on("colleague."); return { ok: true }; },
    attach: async (p) => { calls.attach.push(p); return attach ? attach(p) : { ok: true, name: "minutes.docx", text: "Item 1: budget. Action: Test Person." }; },
  };
  const chat = createChat(fakeDoc, api, ui);
  const lines = () => ui.log.children.map((c) => [c.attrs["data-role"], c.textContent]);
  return { chat, ui, calls, lines };
}

const above = { ok: false, error: { kind: "above_ceiling", message: "This chat is classified higher than the model is cleared for, so it was not sent. Your text is still here." } };

test("typing sends the conversation with the officer's classification and streams the reply into the log", async () => {
  const { chat, ui, calls, lines } = setup();
  ui.select.value = "official-open";
  ui.input.value = "Draft a thank-you note to Test Person";
  assert.equal(await chat.ask(), true);
  assert.deepEqual(calls.send[0], { c: "official-open", m: [{ role: "user", content: "Draft a thank-you note to Test Person" }] });
  assert.deepEqual(lines(), [["user", "Draft a thank-you note to Test Person"], ["assistant", "Dear colleague."]]);
  assert.equal(ui.input.value, "", "cleared once answered");
  assert.equal(calls.expand, 1, "the canvas opens up to show the conversation");
});

test("the reply grows as chunks arrive", async () => {
  const seen = [];
  let ctx;
  const send = async (c, m, on) => { on("a"); seen.push(ctx.lines()[1][1]); on("b"); seen.push(ctx.lines()[1][1]); return { ok: true }; };
  ctx = setup({ send });
  ctx.ui.select.value = "official-open";
  ctx.ui.input.value = "hi";
  await ctx.chat.ask();
  assert.deepEqual(seen, ["a", "ab"]);
});

test("a second message carries the whole conversation", async () => {
  const { chat, ui, calls } = setup();
  ui.select.value = "official-open";
  ui.input.value = "first";
  await chat.ask();
  ui.input.value = "second";
  await chat.ask();
  assert.deepEqual(calls.send[1].m, [
    { role: "user", content: "first" }, { role: "assistant", content: "Dear colleague." }, { role: "user", content: "second" },
  ]);
});

test("no classification: nothing is sent and the officer's text stays in the input", async () => {
  const { chat, ui, calls } = setup();
  ui.input.value = "Summarise the incident for Test Person";
  assert.equal(await chat.ask(), false);
  assert.equal(calls.send.length, 0);
  assert.equal(ui.input.value, "Summarise the incident for Test Person");
  assert.match(ui.status.textContent, /Choose a classification/);
  assert.equal(ui.status.attrs.role, "alert");
});

test("above the ceiling: a plain message shows, the text stays in the input, the turn is taken back, history is unchanged", async () => {
  const { chat, ui, lines } = setup({ send: async () => above });
  ui.select.value = "secret";
  ui.input.value = "Test Person case notes";
  assert.equal(await chat.ask(), false);
  assert.equal(ui.input.value, "Test Person case notes");
  assert.match(ui.status.textContent, /not sent/);
  assert.equal(ui.status.attrs.role, "alert");
  assert.deepEqual(lines(), []);
  assert.deepEqual(chat.state().messages, []);
});

test("after a refusal the officer can lower the classification and send the same text", async () => {
  let n = 0;
  const { chat, ui, calls } = setup({ send: async (c, m, on) => { if (n++ === 0) return above; on("ok"); return { ok: true }; } });
  ui.select.value = "secret";
  ui.input.value = "hello";
  await chat.ask();
  ui.select.value = "official-open";
  await chat.ask();
  assert.equal(calls.send[1].c, "official-open");
  assert.deepEqual(calls.send[1].m, [{ role: "user", content: "hello" }], "the refused turn left no trace in the conversation");
  assert.equal(ui.input.value, "");
  assert.equal(ui.status.textContent, "");
});

test("a send that throws is a plain failure, the text stays", async () => {
  const { chat, ui } = setup({ send: async () => { throw new Error("boom: Test Person"); } });
  ui.select.value = "official-open";
  ui.input.value = "hi";
  assert.equal(await chat.ask(), false);
  assert.equal(ui.input.value, "hi");
  assert.ok(!ui.status.textContent.includes("boom"), "internal errors are not shown");
});

test("an empty input sends nothing", async () => {
  const { chat, ui, calls } = setup();
  ui.select.value = "official-open";
  ui.input.value = "   ";
  assert.equal(await chat.ask(), false);
  assert.equal(calls.send.length, 0);
});

test("dropping a document extracts it, then asks for a summary of it", async () => {
  const { chat, ui, calls, lines } = setup();
  ui.select.value = "official-closed";
  assert.equal(await chat.drop(["/tmp/minutes.docx"]), true);
  assert.deepEqual(calls.attach, ["/tmp/minutes.docx"]);
  assert.equal(calls.send[0].c, "official-closed");
  assert.equal(calls.send[0].m[0].content, SUMMARY_PROMPT("minutes.docx", "Item 1: budget. Action: Test Person."));
  assert.match(calls.send[0].m[0].content, /Summarise/);
  assert.deepEqual(lines()[0], ["user", "Summarise: minutes.docx"], "the document body is not dumped into the log");
});

test("a document dropped before a classification is chosen waits, then goes when it is set", async () => {
  const { chat, ui, calls } = setup();
  assert.equal(await chat.drop(["/tmp/minutes.docx"]), false);
  assert.equal(calls.send.length, 0);
  assert.equal(chat.state().pendingDoc, true);
  assert.match(ui.status.textContent, /Choose a classification/);
  ui.select.value = "official-open";
  assert.equal(await chat.classificationChanged(), true);
  assert.equal(calls.send.length, 1);
  assert.equal(chat.state().pendingDoc, false);
});

test("a document above the ceiling is refused and stays waiting; lowering the classification sends it", async () => {
  let n = 0;
  const { chat, ui, calls } = setup({ send: async (c, m, on) => { if (n++ === 0) return above; on("Summary."); return { ok: true }; } });
  ui.select.value = "secret";
  assert.equal(await chat.drop(["/tmp/minutes.docx"]), false);
  assert.match(ui.status.textContent, /not sent/);
  assert.equal(chat.state().pendingDoc, true);
  ui.select.value = "official-open";
  assert.equal(await chat.classificationChanged(), true);
  assert.equal(calls.send.length, 2);
});

test("a file that cannot be read says so plainly and sends nothing", async () => {
  const { chat, ui, calls } = setup({ attach: async () => ({ ok: false, message: "I can read .docx, .pdf and .txt files." }) });
  ui.select.value = "official-open";
  assert.equal(await chat.drop(["/tmp/a.xlsx"]), false);
  assert.equal(ui.status.textContent, "I can read .docx, .pdf and .txt files.");
  assert.equal(calls.send.length, 0);
  assert.equal(chat.state().pendingDoc, false);
});

test("only one turn at a time", async () => {
  let release;
  const gate = new Promise((r) => { release = r; });
  const { chat, ui, calls } = setup({ send: async () => { await gate; return { ok: true }; } });
  ui.select.value = "official-open";
  ui.input.value = "one";
  const first = chat.ask();
  ui.input.value = "two";
  assert.equal(await chat.ask(), false);
  release();
  await first;
  assert.equal(calls.send.length, 1);
});

test("the page uses no browser storage and no innerHTML (text goes in as text)", () => {
  const src = fs.readFileSync(path.join(__dirname, "..", "chat.js"), "utf8");
  for (const bad of ["localStorage", "sessionStorage", "indexedDB", "document.cookie", "innerHTML", "fetch(", "XMLHttpRequest", "WebSocket"]) {
    assert.ok(!src.includes(bad), bad);
  }
});
