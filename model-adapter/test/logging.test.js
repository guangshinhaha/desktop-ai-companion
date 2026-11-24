import test from "node:test";
import assert from "node:assert/strict";
import { CONFIG, start } from "./helpers.js";
import { consoleLogger } from "../src/log.ts";

const SECRET = "ZEBRA-7731-synthetic-nric-S0000000A";

// Capture every channel a log could leave through.
function capture() {
  const out = [];
  const orig = { log: console.log, error: console.error, warn: console.warn, info: console.info, debug: console.debug, so: process.stdout.write, se: process.stderr.write };
  for (const k of ["log", "error", "warn", "info", "debug"]) console[k] = (...a) => out.push(a.map(String).join(" "));
  process.stdout.write = (c) => { out.push(String(c)); return true; };
  process.stderr.write = (c) => { out.push(String(c)); return true; };
  return { out, restore() { Object.assign(console, { log: orig.log, error: orig.error, warn: orig.warn, info: orig.info, debug: orig.debug }); process.stdout.write = orig.so; process.stderr.write = orig.se; } };
}

test("a known string sent through every path appears in no log", async () => {
  const route = { name: "r", async *stream(req) { if (req.messages[0].content.startsWith("FAIL")) throw new Error(`upstream said ${SECRET}`); yield `echo ${SECRET}`; } };
  const s = await start({ route });
  const cap = capture();
  const results = [];
  try {
    // events from the handler are also fed to the console logger, as the service will wire it
    const push = s.events.push.bind(s.events);
    s.events.push = (e) => { consoleLogger(e); return push(e); };
    results.push(await s.post({ classification: "official-open", messages: [{ role: "user", content: SECRET }] }));
    results.push(await s.post({ classification: "official-open", messages: [{ role: "user", content: "FAIL " + SECRET }] }));
    results.push(await s.post({ classification: SECRET, messages: [{ role: "user", content: SECRET }] }));
    results.push(await s.post({ classification: "official-open", model: SECRET, messages: [{ role: "user", content: SECRET }] }));
    results.push(await s.post(`{"x":"${SECRET}"`, { raw: true }));
    results.push(await s.post({ classification: "secret", messages: [{ role: "user", content: SECRET }] }));
  } finally {
    cap.restore();
  }
  assert.deepEqual(results.map((r) => r.status), [200, 200, 403, 400, 400, 403]);
  assert.ok(cap.out.length >= 6, "events were logged");
  assert.ok(!cap.out.join("\n").includes(SECRET), "console/stdout/stderr");
  assert.ok(!JSON.stringify(s.events).includes(SECRET), "captured events");
  for (const r of results.slice(2)) assert.ok(!r.text.includes(SECRET), "refusals do not echo input");
  await s.close();
});

test("log events carry metadata only", async () => {
  const s = await start();
  await s.post({ classification: "official-open", messages: [{ role: "user", content: "hello" }] });
  const e = s.events[0];
  assert.deepEqual(Object.keys(e).sort(), ["classification", "ms", "messages", "model", "outcome", "replyBytes", "requestBytes", "route", "status"].sort());
  assert.equal(e.classification, "official-open");
  assert.equal(e.model, CONFIG.models[0]);
  await s.close();
});
