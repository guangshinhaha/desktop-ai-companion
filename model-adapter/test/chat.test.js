import test from "node:test";
import assert from "node:assert/strict";
import { CONFIG, start } from "./helpers.js";

const msgs = [{ role: "user", content: "Summarise the synthetic memo." }];

test("POST /chat streams a reply from the configured route", async () => {
  const s = await start();
  const r = await s.post({ classification: "official-open", messages: msgs });
  assert.equal(r.status, 200);
  const lines = r.text.trim().split("\n").map((l) => JSON.parse(l));
  assert.ok(lines.length > 2, "more than one chunk");
  assert.deepEqual(lines.at(-1), { done: true });
  assert.match(lines.slice(0, -1).map((l) => l.text).join(""), /Stub reply from stub-small/);
  assert.equal(s.calls.length, 1);
  await s.close();
});

test("changing the route is configuration only: a different route object and models, same handler", async () => {
  const route = { name: "other", async *stream(req) { yield `via other ${req.model}`; } };
  const s = await start({ config: { ...CONFIG, models: ["m-x"] }, route });
  const r = await s.post({ classification: "official-open", messages: msgs });
  assert.match(r.text, /via other m-x/);
  assert.equal(s.events[0].route, "other");
  await s.close();
});

test("a request at the ceiling is sent; above it is refused with 403 and the route is never called", async () => {
  const s = await start();
  assert.equal((await s.post({ classification: "official-closed", messages: msgs })).status, 200);
  for (const classification of ["restricted", "confidential", "secret"]) {
    const r = await s.post({ classification, messages: msgs });
    assert.equal(r.status, 403, classification);
    assert.equal(JSON.parse(r.text).error, "above_ceiling");
  }
  assert.equal(s.calls.length, 1, "only the at-ceiling request reached the route");
  await s.close();
});

test("unknown, missing or malformed classification counts as above the ceiling", async () => {
  const s = await start();
  for (const classification of ["top-secret", "", "OFFICIAL-OPEN", null, 0, ["official-open"], { a: 1 }, undefined]) {
    const r = await s.post({ classification, messages: msgs });
    assert.equal(r.status, 403, JSON.stringify(classification));
  }
  assert.equal(s.calls.length, 0);
  await s.close();
});

test("a refused request is never downgraded: nothing is forwarded, nor retried at a lower level", async () => {
  const s = await start();
  const r = await s.post({ classification: "secret", messages: msgs });
  assert.equal(r.status, 403);
  assert.deepEqual(s.calls, []);
  assert.doesNotMatch(r.text, /Stub reply/);
  await s.close();
});

test("above-ceiling is refused even when the rest of the request is also invalid", async () => {
  const s = await start();
  const r = await s.post({ classification: "secret", model: "nope", messages: [] });
  assert.equal(r.status, 403);
  await s.close();
});

test("bad requests get 4xx", async () => {
  const s = await start();
  assert.equal((await s.post("{nope", { raw: true })).status, 400);
  assert.equal((await s.post("[]", { raw: true })).status, 400);
  assert.equal((await s.post({ classification: "official-open", model: "ghost", messages: msgs })).status, 400);
  assert.equal((await s.post({ classification: "official-open", messages: [] })).status, 400);
  assert.equal((await s.post({ classification: "official-open", messages: [{ role: "root", content: "x" }] })).status, 400);
  assert.equal((await s.post("x".repeat(1_100_000), { raw: true })).status, 413);
  assert.equal(s.calls.length, 0);
  await s.close();
});

test("a route that fails mid-stream ends the reply with an error line, not a hang", async () => {
  const route = { name: "flaky", async *stream() { yield "a"; throw new Error("boom"); } };
  const s = await start({ route });
  const r = await s.post({ classification: "official-open", messages: msgs });
  assert.equal(r.status, 200);
  assert.deepEqual(JSON.parse(r.text.trim().split("\n").at(-1)), { error: "route_error" });
  assert.equal(s.events[0].outcome, "route_error");
  await s.close();
});

// #103: hardening. Synthetic data only.
const M = `"messages":${JSON.stringify(msgs)}`;
// The second spelling writes the "c" of the key as a JSON unicode escape: the same key to JSON.parse.
const ESCAPED = "\\u0063lassification";

test("a repeated top-level key, however it is spelt, gets 400 and the route is never called", async () => {
  const s = await start();
  for (const body of [
    `{"classification":"secret","classification":"official-open",${M}}`,
    `{"classification":"secret","${ESCAPED}":"official-open",${M}}`,
    `{"classification":"official-open","classification":"secret",${M}}`,
    `{"classification":"official-open",${M} , "messages" : []}`,
    `{"classification":"official-open",${M},"__proto__":1,"__proto__":2}`,
  ]) {
    const r = await s.post(body, { raw: true });
    assert.equal(r.status, 400, body);
  }
  assert.equal(s.calls.length, 0);
  assert.ok(s.events.every((e) => e.outcome === "bad_request"));
  await s.close();
});

test("keys repeated only below the top level, or only inside strings, are not top-level repeats", async () => {
  const s = await start();
  const content = 'say "classification": {"classification": "x"} \\ ]}';
  const r = await s.post({ classification: "official-open", note: { classification: 1, a: [{ classification: 2 }] }, other: "classification", messages: [{ role: "user", content }, { role: "user", content }] });
  assert.equal(r.status, 200);
  assert.equal(s.calls[0].messages[0].content, content);
  const big = await s.post({ classification: "official-open", messages: [{ role: "user", content: '"\\'.repeat(200_000) }] });
  assert.equal(big.status, 200, "a long run of escapes is scanned");
  await s.close();
});

test("the route receives messages with only role and content", async () => {
  const s = await start();
  const r = await s.post(`{"classification":"official-open","messages":[{"role":"user","content":"hi","name":"x","tool_calls":[],"__proto__":{"a":1}}]}`, { raw: true });
  assert.equal(r.status, 200);
  assert.deepEqual(Reflect.ownKeys(s.calls[0].messages[0]), ["role", "content"]);
  await s.close();
});

async function send(s, { path = "/chat", headers = { "content-type": "application/json" } } = {}) {
  const res = await fetch(s.url.replace("/chat", path), { method: "POST", headers, body: Buffer.from(`{"classification":"official-open",${M}}`) });
  await res.text();
  return res.status;
}

test("a wrong path gets 404, a non-JSON content type 415 and any Origin 403, none reaching the route", async () => {
  const s = await start();
  const json = { "content-type": "application/json" };
  for (const path of ["/", "/anything", "/chat/", "/chat/x", "/Chat", "/recover"]) assert.equal(await send(s, { path }), 404, path);
  assert.equal(await send(s, { headers: {} }), 415, "no content type");
  for (const type of ["text/plain", "text/plain;charset=UTF-8", "application/x-www-form-urlencoded", "multipart/form-data; boundary=x", "application/jsonp", "text/plain; a=application/json"]) {
    assert.equal(await send(s, { headers: { "content-type": type } }), 415, type);
  }
  for (const origin of ["http://evil.test", "null", "http://localhost", `http://127.0.0.1:${s.port}`, ""]) {
    assert.equal(await send(s, { headers: { ...json, origin } }), 403, `origin ${origin}`);
  }
  assert.equal(s.calls.length, 0);
  assert.ok(s.events.every((e) => e.outcome === "bad_request"));
  assert.equal(await send(s, { path: "/chat?x=1", headers: { "content-type": "Application/JSON ; charset=utf-8" } }), 200);
  await s.close();
});
