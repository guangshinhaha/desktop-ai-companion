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
