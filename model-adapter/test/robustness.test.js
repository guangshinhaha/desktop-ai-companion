import test from "node:test";
import assert from "node:assert/strict";
import net from "node:net";
import { CONFIG, start } from "./helpers.js";

const S = "SECRETCANARY12345";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const body = { classification: "official-open", messages: [{ role: "user", content: S }] };
const until = async (fn, ms = 3000) => { const t = Date.now(); while (!fn()) { if (Date.now() - t > ms) throw new Error("timed out waiting"); await sleep(20); } };

test("Error.name set from request content never reaches the log", async () => {
  const route = { name: "r", async *stream(r) { const e = new Error("x"); e.name = r.messages[0].content; throw e; } };
  const s = await start({ route });
  await s.post(body);
  assert.equal(s.events[0].errorClass, "Error");
  assert.ok(!JSON.stringify(s.events).includes(S));
  await s.close();
});

test("a route that throws before yielding is a logged 5xx, not a hang", async () => {
  const route = { name: "r", stream() { throw new Error(S); } };
  const s = await start({ route });
  const r = await s.post(body);
  assert.equal(r.status, 500);
  assert.ok(!r.text.includes(S) && !JSON.stringify(s.events).includes(S));
  await s.close();
});

test("client abort on a hung route is logged as aborted and the route is told to stop", async () => {
  let returned = false;
  const route = { name: "r", stream: () => ({ [Symbol.asyncIterator]: () => ({ next: () => new Promise(() => {}), return: async () => { returned = true; return { done: true }; } }) }) };
  const s = await start({ route });
  const ac = new AbortController();
  fetch(s.url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body), signal: ac.signal }).catch(() => {});
  await sleep(150);
  ac.abort();
  await until(() => s.events.length === 1);
  assert.equal(s.events[0].outcome, "aborted");
  assert.ok(returned, "iterator.return() was called");
  await s.close();
});

test("a route that ignores abort cannot log ok for a gone client", async () => {
  const route = { name: "r", async *stream() { for (let i = 0; i < 6; i++) { await sleep(100); yield "x"; } } };
  const s = await start({ route });
  const ac = new AbortController();
  fetch(s.url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body), signal: ac.signal }).catch(() => {});
  await sleep(150);
  ac.abort();
  await until(() => s.events.length === 1);
  assert.equal(s.events[0].outcome, "aborted");
  await s.close();
});

test("a route that never yields hits the idle timeout: 502-style error line, logged, cleaned up", async () => {
  let returned = false;
  const route = { name: "r", stream: () => ({ [Symbol.asyncIterator]: () => ({ next: () => new Promise(() => {}), return: async () => { returned = true; return { done: true }; } }) }) };
  const s = await start({ route, options: { idleTimeoutMs: 100 } });
  const r = await s.post(body);
  assert.deepEqual(JSON.parse(r.text.trim()), { error: "route_error" });
  assert.equal(s.events[0].outcome, "route_error");
  assert.equal(s.events[0].errorClass, "TimeoutError");
  assert.ok(returned);
  await s.close();
});

test("a body that never finishes times out with 408 and is logged", async () => {
  const s = await start({ options: { bodyTimeoutMs: 100 } });
  const sock = net.connect(s.port, "127.0.0.1");
  let got = "";
  sock.on("data", (d) => { got += d; });
  sock.on("error", () => {});
  sock.write("POST /chat HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n{");
  await until(() => s.events.length === 1);
  assert.equal(s.events[0].status, 408);
  assert.match(got, /408/);
  sock.destroy();
  await s.close();
});

test("a client dropping mid-body is logged as aborted, not as a route error", async () => {
  const s = await start();
  const sock = net.connect(s.port, "127.0.0.1");
  sock.on("error", () => {});
  sock.write("POST /chat HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n{");
  await sleep(100);
  sock.destroy();
  await until(() => s.events.length === 1);
  assert.equal(s.events[0].outcome, "aborted");
  assert.equal(s.events[0].errorClass, undefined);
  await s.close();
});

test("a throwing log sink does not break the request", async () => {
  const s = await start({ logger: () => { throw new Error(S); } });
  const rejections = [];
  const h = (e) => rejections.push(e);
  process.on("unhandledRejection", h);
  const r = await s.post(body);
  await sleep(100);
  process.off("unhandledRejection", h);
  assert.equal(r.status, 200);
  assert.deepEqual(rejections, []);
  await s.close();
});

test("config default model is used and metadata names it", async () => {
  const s = await start();
  await s.post(body);
  assert.equal(s.events[0].model, CONFIG.models[0]);
  await s.close();
});
