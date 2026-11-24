// Runs the real stand-in server with a fake executor and a fake browser, to prove the harness resets
// between runs and judges from the stand-in's log. No Chrome, no screen, no accessibility.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn, type ChildProcess } from "node:child_process";
import { chmodSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { oneRun, standIn, type Options } from "../src/runner.ts";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const port = "3951";
const base = `http://127.0.0.1:${port}`;
let server: ChildProcess;
let o: Options;

before(async () => {
  server = spawn("node", [join(repo, "stand-in/provisioning/server.ts")], { env: { ...process.env, PORT: port }, stdio: "ignore" });
  for (let i = 0; i < 50; i++) {
    try { await standIn(base, "/healthz"); break; } catch { await new Promise((r) => setTimeout(r, 100)); }
  }
  const dir = mkdtempSync(join(tmpdir(), "fake-exec-"));
  const exe = join(dir, "fake-executor");
  writeFileSync(exe, `#!/usr/bin/env node
import { readFileSync } from "node:fs";
const a = process.argv;
const inp = JSON.parse(readFileSync(a[a.indexOf("--inputs") + 1], "utf8"));
const approvals = readFileSync(a[a.indexOf("--approval-file") + 1], "utf8");
if (!approvals.startsWith("y\\n")) process.exit(4);
const mode = process.env.FAKE_MODE;
console.log("step s1: Type");
console.log("  done: s1");
if (mode === "stop") { console.log("run stopped: step s2 diverged"); console.log("  reason: element not found"); process.exit(2); }
if (mode === "wrong") inp.role = "Admin Clerk";
const body = new URLSearchParams(inp);
const r = await fetch("${base}/accounts", { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body });
process.exit(r.ok ? 0 : 1);
`);
  chmodSync(exe, 0o755);
  o = {
    base, executor: exe, taskPath: "unused", chrome: "unused", runTimeoutMs: 20000, probeReady: false,
    launchBrowser: () => spawn("sleep", ["60"], { stdio: "ignore" }),
  };
});

after(() => { server.kill("SIGTERM"); });

test("a correct run passes and records step times", async () => {
  process.env.FAKE_MODE = "ok";
  const r = await oneRun(o, "", 1);
  assert.equal(r.outcome, "pass", r.reason);
  assert.equal(r.stepMs.length, 1);
});

test("the stand-in is reset between runs: each run sees only its own account", async () => {
  process.env.FAKE_MODE = "ok";
  await oneRun(o, "", 1);
  const r = await oneRun(o, "", 2);
  assert.equal(r.outcome, "pass", r.reason);
  assert.equal((await standIn(base, "/api/accounts")).accounts.length, 1);
});

test("wrong data submitted is a fail", async () => {
  process.env.FAKE_MODE = "wrong";
  // run 1 has role "Form Teacher" (index 1), so Admin Clerk differs
  const r = await oneRun(o, "", 1);
  assert.equal(r.outcome, "fail");
  assert.match(r.reason, /wrong data/);
});

test("a stop at divergence with nothing created is a stop, with the executor's reason", async () => {
  process.env.FAKE_MODE = "stop";
  const r = await oneRun(o, "?rename=1", 1);
  assert.equal(r.outcome, "stop");
  assert.match(r.reason, /element not found/);
});
