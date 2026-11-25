import test from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import { createRecoverHandler } from "../src/recover.ts";
import { loadConfig } from "../src/config.ts";
import { stubRoute } from "../src/route.ts";
import { CONFIG } from "./helpers.js";

// A recovery request as the executor builds it (synthetic). Values masked, no password value.
const request = (over = {}) => ({
  kind: "recover", classification: "official-open", app: "com.example.standin", attempt: 1, max_attempts: 3,
  step: { id: "s2", say: "Type the name", action: "set_value", target: { role: "AXTextField", label: "Name" }, consequential: "no", input: "name" },
  divergence: "Name field not found", values: "masked",
  tree: { ref: "n1", role: "AXWindow", label: "Stand-in", children: [{ ref: "n2", role: "AXButton", label: "Close" }] },
  ...over,
});

async function start({ route = stubRoute(), config = CONFIG, options } = {}) {
  const cfg = loadConfig(config);
  const calls = [];
  const spy = { name: route.name, stream: route.stream, recover: route.recover && (async (r) => { calls.push(r); return route.recover(r); }) };
  const events = [];
  const handler = createRecoverHandler(cfg, spy, (e) => events.push(e), options);
  const server = http.createServer((req, res) => { handler(req, res); });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  server.unref();
  const url = `http://127.0.0.1:${server.address().port}/recover`;
  async function post(body, { raw = false, method = "POST" } = {}) {
    const res = await fetch(url, { method, body: method === "GET" ? undefined : raw ? body : JSON.stringify(body) });
    return { status: res.status, json: await res.json() };
  }
  return { post, calls, events, close: () => new Promise((r) => { server.closeAllConnections(); server.close(r); }) };
}

const routeAnswering = (answer) => ({ name: "fake", async *stream() {}, async recover() { return answer; } });

test("the stub route answers ask_officer: no model is connected", async () => {
  const s = await start();
  const r = await s.post(request());
  assert.equal(r.status, 200);
  assert.equal(r.json.answer, "ask_officer");
  assert.match(r.json.reason, /stub/i);
  assert.equal(s.calls.length, 1);
  assert.equal(s.events[0].outcome, "ok");
  await s.close();
});

test("above the ceiling, unknown or missing classification is refused with 403 and the route is never called", async () => {
  const s = await start();
  for (const classification of ["restricted", "secret", "top-secret", "", null, undefined, 3]) {
    const r = await s.post(request({ classification }));
    assert.equal(r.status, 403, JSON.stringify(classification));
    assert.equal(r.json.error, "above_ceiling");
  }
  assert.equal(s.calls.length, 0);
  await s.close();
});

test("above the ceiling wins over a malformed body: refused, never downgraded", async () => {
  const s = await start();
  const r = await s.post({ classification: "secret" });
  assert.equal(r.status, 403);
  assert.equal(s.calls.length, 0);
  await s.close();
});

test("a request that is not a recovery request is a 400", async () => {
  const s = await start();
  for (const bad of [request({ kind: "chat" }), request({ step: "s2" }), request({ tree: [] }), request({ values: "maybe" }), request({ app: 5 }), request({ screenshot: "AAAA" })]) {
    const r = await s.post(bad);
    assert.equal(r.status, 400, JSON.stringify(bad).slice(0, 80));
  }
  assert.equal((await s.post("{not json", { raw: true })).status, 400);
  assert.equal(s.calls.length, 0);
  await s.close();
});

test("only POST", async () => {
  const s = await start();
  assert.equal((await s.post(null, { method: "GET" })).status, 405);
  await s.close();
});

test("a valid answer from the route is passed through as is", async () => {
  for (const answer of [
    { answer: "act", action: "set_value", ref: "n3", value: { input: "name" } },
    { answer: "retry" },
    { answer: "ask_officer", reason: "The page looks different." },
  ]) {
    const s = await start({ route: routeAnswering(answer) });
    const r = await s.post(request());
    assert.equal(r.status, 200, JSON.stringify(answer));
    assert.deepEqual(r.json, answer);
    await s.close();
  }
});

test("a malformed answer from the route fails closed: 502, nothing passed on", async () => {
  for (const answer of [
    null, "press n2", [], { answer: "click", ref: "n2" }, { answer: "act", action: "type", ref: "n2" },
    { answer: "act", action: "press" }, { answer: "act", action: "press", ref: "n2" }, { answer: "act", action: "set_value", ref: "n3", value: "Aiden Tan" },
    { answer: "ask_officer" }, { answer: "retry", extra: 1 },
  ]) {
    const s = await start({ route: routeAnswering(answer) });
    const r = await s.post(request());
    assert.equal(r.status, 502, JSON.stringify(answer));
    assert.equal(r.json.error, "route_error");
    assert.equal(s.events[0].outcome, "route_error");
    await s.close();
  }
});

test("a route that throws is a 502 and its message never reaches the log", async () => {
  const route = { name: "fake", async *stream() {}, async recover() { throw new Error("SEEDED-SECRET-LEAK"); } };
  const s = await start({ route });
  const r = await s.post(request());
  assert.equal(r.status, 502);
  assert.doesNotMatch(JSON.stringify(s.events) + JSON.stringify(r.json), /SEEDED-SECRET-LEAK/);
  await s.close();
});

test("a route with no recover is a 501, never a made-up answer", async () => {
  const s = await start({ route: { name: "chat-only", async *stream() {} } });
  const r = await s.post(request());
  assert.equal(r.status, 501);
  await s.close();
});

test("logs hold metadata only: no label, divergence or say text", async () => {
  const s = await start();
  await s.post(request({ divergence: "SEEDED-DIVERGENCE-TEXT" }));
  const logged = JSON.stringify(s.events);
  assert.doesNotMatch(logged, /SEEDED-DIVERGENCE-TEXT|Type the name|Stand-in|Close/);
  assert.equal(s.events[0].classification, "official-open");
  await s.close();
});

test("a route that never answers hits the timeout: 502, never a hang, and the route is told to stop", async () => {
  let stopped = false;
  const route = { name: "fake", async *stream() {}, recover: ({ signal }) => new Promise(() => { signal.addEventListener("abort", () => { stopped = true; }); }) };
  const s = await start({ route, options: { routeTimeoutMs: 50 } });
  const r = await s.post(request());
  assert.equal(r.status, 502);
  assert.equal(s.events[0].errorClass, "TimeoutError");
  assert.ok(stopped);
  await s.close();
});
